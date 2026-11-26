use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;

use super::{JudgeEngine, JudgeOutcomeStatus, JudgeTask};
use crate::model::SubmissionStatus;
use crate::services::registry::{ContestRegistry, ContestRuntime};
use crate::store::SubmissionResult;
use crate::timefmt::{format_local, now_ms};

#[derive(Debug, Clone)]
pub struct JudgeJob {
    pub contest_id: String,
    pub submission_id: i64,
}

#[derive(Clone)]
pub struct JudgeQueueHandle {
    tx: mpsc::UnboundedSender<JudgeJob>,
    depth: Arc<AtomicUsize>,
}

impl JudgeQueueHandle {
    pub fn enqueue(&self, contest_id: impl Into<String>, submission_id: i64) -> bool {
        let job = JudgeJob {
            contest_id: contest_id.into(),
            submission_id,
        };
        if self.tx.send(job).is_ok() {
            self.depth.fetch_add(1, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::SeqCst)
    }
}

pub struct JudgeQueue;

impl JudgeQueue {
    pub fn start(
        registry: Arc<ContestRegistry>,
        engine: Arc<dyn JudgeEngine>,
        concurrency: usize,
    ) -> JudgeQueueHandle {
        let (tx, rx) = mpsc::unbounded_channel::<JudgeJob>();
        let rx = Arc::new(AsyncMutex::new(rx));
        let depth = Arc::new(AtomicUsize::new(0));
        let handle = JudgeQueueHandle {
            tx: tx.clone(),
            depth: depth.clone(),
        };

        for _ in 0..concurrency.max(1) {
            let registry = registry.clone();
            let engine = engine.clone();
            let rx = rx.clone();
            let depth = depth.clone();
            tokio::spawn(async move {
                loop {
                    let job = {
                        let mut receiver = rx.lock().await;
                        receiver.recv().await
                    };
                    let Some(job) = job else {
                        break;
                    };
                    depth.fetch_sub(1, Ordering::SeqCst);
                    if let Err(error) = process_job(&registry, engine.as_ref(), job).await {
                        tracing::error!(%error, "判题任务失败");
                    }
                }
            });
        }

        recover_jobs(&registry, &handle);
        handle
    }
}

fn recover_jobs(registry: &ContestRegistry, handle: &JudgeQueueHandle) {
    for runtime in registry.list() {
        if let Err(error) = runtime.store.mark_stale_queued() {
            tracing::error!(contest = %runtime.package.id, %error, "恢复队列失败");
            continue;
        }
        let rows = match runtime.store.recoverable_submissions() {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(contest = %runtime.package.id, %error, "读取待恢复提交失败");
                continue;
            }
        };
        for row in rows {
            if !row.source_path.is_file() {
                let message = "源码文件缺失，无法恢复评测";
                let _ = runtime.store.mark_failed(row.id, message, now_ms());
                log_line(&runtime, &format!("提交 {} 恢复失败: {message}", row.id));
                continue;
            }
            if handle.enqueue(runtime.package.id.clone(), row.id) {
                log_line(&runtime, &format!("提交 {} 重新入队", row.id));
            }
        }
    }
}

async fn process_job(
    registry: &ContestRegistry,
    engine: &dyn JudgeEngine,
    job: JudgeJob,
) -> Result<(), String> {
    let Some(runtime) = registry.get(&job.contest_id) else {
        return Err(format!("比赛 {} 不存在", job.contest_id));
    };
    if !runtime
        .store
        .claim_queued(job.submission_id)
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }

    let Some(row) = runtime
        .store
        .submission(job.submission_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let Some(problem) = runtime.package.find_problem(&row.problem_key).cloned() else {
        let message = format!("题号 {} 已不在当前题包中", row.problem_key);
        runtime
            .store
            .mark_failed(row.id, &message, now_ms())
            .map_err(|error| error.to_string())?;
        log_line(&runtime, &format!("提交 {} 失败: {message}", row.id));
        return Ok(());
    };
    if !row.source_path.is_file() {
        let message = "源码文件缺失";
        runtime
            .store
            .mark_failed(row.id, message, now_ms())
            .map_err(|error| error.to_string())?;
        log_line(&runtime, &format!("提交 {} 失败: {message}", row.id));
        return Ok(());
    }

    runtime
        .store
        .set_status(row.id, SubmissionStatus::Running)
        .map_err(|error| error.to_string())?;
    log_line(&runtime, &format!("提交 {} 开始评测", row.id));

    let work_dir = runtime
        .store
        .data_dir()
        .join("tmp")
        .join(row.id.to_string());
    let _ = fs::remove_dir_all(&work_dir);
    let task = JudgeTask {
        contest_id: row.contest_id.clone(),
        submission_id: row.id,
        problem,
        source_path: row.source_path.clone(),
        work_dir,
    };
    let outcome = engine.judge(task).await;
    let judged_at = now_ms();
    let cases_json = serde_json::to_string(&outcome.cases).ok();

    match outcome.status {
        JudgeOutcomeStatus::Done => {
            runtime
                .store
                .set_result(
                    row.id,
                    &SubmissionResult {
                        verdict: outcome.verdict,
                        score: outcome.score,
                        cases_json: cases_json.as_deref(),
                        compile_output: outcome.compile_output.as_deref(),
                        error: outcome.error.as_deref(),
                        judged_at,
                    },
                )
                .map_err(|error| error.to_string())?;
            log_line(
                &runtime,
                &format!(
                    "提交 {} 完成: {} {} 分",
                    row.id,
                    outcome.verdict.as_str(),
                    outcome.score
                ),
            );
        }
        JudgeOutcomeStatus::Failed => {
            let message = outcome.error.unwrap_or_else(|| "评测内部错误".to_string());
            runtime
                .store
                .mark_failed(row.id, &message, judged_at)
                .map_err(|error| error.to_string())?;
            log_line(&runtime, &format!("提交 {} 失败: {message}", row.id));
        }
    }

    Ok(())
}

fn log_line(runtime: &Arc<ContestRuntime>, message: &str) {
    let line = format!("{} {message}", format_local(now_ms()));
    if let Err(error) = runtime.store.append_log(&line) {
        tracing::error!(contest = %runtime.package.id, %error, "写 judge.log 失败");
    }
}
