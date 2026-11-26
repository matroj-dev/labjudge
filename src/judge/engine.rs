use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::{CaseRunner, CompileOutcome, CompilerBackend, JudgeRunRequest};
use crate::compare::outputs_equal;
use crate::contest::model::ValidatedProblem;
use crate::model::Verdict;
use crate::scoring::{score_problem, RawCase, ScoredCase};

#[derive(Debug, Clone)]
pub struct JudgeTask {
    pub contest_id: String,
    pub submission_id: i64,
    pub problem: ValidatedProblem,
    pub source_path: PathBuf,
    pub work_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeOutcomeStatus {
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct JudgeOutcome {
    pub status: JudgeOutcomeStatus,
    pub verdict: Verdict,
    pub score: i32,
    pub cases: Vec<ScoredCase>,
    pub compile_output: Option<String>,
    pub error: Option<String>,
}

impl JudgeOutcome {
    fn failed(error: impl Into<String>) -> Self {
        Self {
            status: JudgeOutcomeStatus::Failed,
            verdict: Verdict::Ie,
            score: 0,
            cases: Vec::new(),
            compile_output: None,
            error: Some(error.into()),
        }
    }
}

#[async_trait]
pub trait JudgeEngine: Send + Sync {
    async fn judge(&self, task: JudgeTask) -> JudgeOutcome;
}

pub struct RealEngine {
    compiler: Arc<dyn CompilerBackend>,
    runner: Arc<dyn CaseRunner>,
}

impl RealEngine {
    pub fn new(compiler: Arc<dyn CompilerBackend>, runner: Arc<dyn CaseRunner>) -> Self {
        Self { compiler, runner }
    }

    async fn judge_inner(&self, task: &JudgeTask) -> JudgeOutcome {
        let source = match fs::read_to_string(&task.source_path) {
            Ok(source) => source,
            Err(error) => return JudgeOutcome::failed(format!("读取源码失败: {error}")),
        };

        let program = match self.compiler.compile_source(&source, &task.work_dir).await {
            Ok(CompileOutcome::Success(program)) => program,
            Ok(CompileOutcome::Ce { output }) => {
                return JudgeOutcome {
                    status: JudgeOutcomeStatus::Done,
                    verdict: Verdict::Ce,
                    score: 0,
                    cases: Vec::new(),
                    compile_output: Some(output),
                    error: None,
                }
            }
            Err(error) => return JudgeOutcome::failed(format!("编译环境错误: {error}")),
        };

        let mut raw_cases = Vec::with_capacity(task.problem.test_cases.len());
        for test_case in &task.problem.test_cases {
            let result = match self
                .runner
                .run_case(&JudgeRunRequest {
                    program: program.clone(),
                    input_path: test_case.in_path.clone(),
                    time_limit_ms: task.problem.time_limit_ms,
                    memory_limit_mb: task.problem.memory_limit_mb,
                })
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    return JudgeOutcome::failed(format!(
                        "测试点 {} 运行失败: {error}",
                        test_case.name
                    ))
                }
            };

            let expected = match fs::read(&test_case.out_path) {
                Ok(expected) => expected,
                Err(error) => {
                    return JudgeOutcome::failed(format!(
                        "读取测试点 {} 期望输出失败: {error}",
                        test_case.name
                    ))
                }
            };
            let verdict = if result.verdict == Verdict::Ac {
                if outputs_equal(&expected, &result.stdout) {
                    Verdict::Ac
                } else {
                    Verdict::Wa
                }
            } else {
                result.verdict
            };

            raw_cases.push(RawCase {
                name: test_case.name.clone(),
                verdict,
                time_ms: result.time_ms,
                memory_kb: result.memory_kb,
            });
        }

        let scored = match score_problem(&task.problem, &raw_cases) {
            Ok(scored) => scored,
            Err(error) => return JudgeOutcome::failed(format!("计分失败: {error}")),
        };

        JudgeOutcome {
            status: JudgeOutcomeStatus::Done,
            verdict: scored.verdict,
            score: scored.total,
            cases: scored.cases,
            compile_output: None,
            error: None,
        }
    }
}

#[async_trait]
impl JudgeEngine for RealEngine {
    async fn judge(&self, task: JudgeTask) -> JudgeOutcome {
        let outcome = self.judge_inner(&task).await;
        let _ = fs::remove_dir_all(&task.work_dir);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{SubtaskKind, TestCase, ValidatedSubtask};
    use crate::judge::{CompileError, JudgeRunResult, JudgeTransportError};
    use std::path::Path;
    use std::sync::Mutex;

    struct FakeCompiler {
        result: Mutex<Option<CompileOutcome>>,
    }

    #[async_trait]
    impl CompilerBackend for FakeCompiler {
        async fn compile_source(
            &self,
            _source: &str,
            work_dir: &Path,
        ) -> Result<CompileOutcome, CompileError> {
            let _ = work_dir;
            Ok(self
                .result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(CompileOutcome::Success(PathBuf::from("prog.exe"))))
        }
    }

    struct FakeRunner {
        results: Mutex<Vec<Result<JudgeRunResult, JudgeTransportError>>>,
    }

    #[async_trait]
    impl CaseRunner for FakeRunner {
        async fn run_case(
            &self,
            _request: &JudgeRunRequest,
        ) -> Result<JudgeRunResult, JudgeTransportError> {
            self.results.lock().unwrap().remove(0)
        }
    }

    fn problem(dir: &Path) -> ValidatedProblem {
        let mut cases = Vec::new();
        for (name, output) in [("1", "7"), ("2", "10")] {
            let in_path = dir.join(format!("{name}.in"));
            let out_path = dir.join(format!("{name}.out"));
            fs::write(&in_path, "in").unwrap();
            fs::write(&out_path, output).unwrap();
            cases.push(TestCase {
                name: name.to_string(),
                in_path,
                out_path,
            });
        }
        ValidatedProblem {
            key: "A".to_string(),
            title: "A".to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score: 100,
            root: dir.to_path_buf(),
            statement_path: dir.join("statement.md"),
            test_cases: cases,
            subtasks: vec![ValidatedSubtask {
                name: "子任务 1".to_string(),
                score: 100,
                kind: SubtaskKind::Sum,
                cases: vec!["1".to_string(), "2".to_string()],
            }],
        }
    }

    fn run_result(verdict: Verdict, stdout: &str) -> JudgeRunResult {
        JudgeRunResult {
            verdict,
            stdout: stdout.as_bytes().to_vec(),
            time_ms: 10,
            memory_kb: 1024,
            error: None,
        }
    }

    fn task(dir: &Path, source: &str) -> JudgeTask {
        let source_path = dir.join("main.cpp");
        fs::write(&source_path, source).unwrap();
        JudgeTask {
            contest_id: "c".to_string(),
            submission_id: 1,
            problem: problem(dir),
            source_path,
            work_dir: dir.join("work"),
        }
    }

    #[tokio::test]
    async fn ac_wa_and_tle_are_derived_from_runner_output() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(None),
            }),
            Arc::new(FakeRunner {
                results: Mutex::new(vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Ac, "wrong\n")),
                ]),
            }),
        );
        let outcome = engine.judge(task(dir.path(), "int main(){}")).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Done);
        assert_eq!(outcome.verdict, Verdict::Wa);
        assert_eq!(outcome.score, 50);
        assert_eq!(outcome.cases.len(), 2);

        let engine = RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(None),
            }),
            Arc::new(FakeRunner {
                results: Mutex::new(vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Tle, "")),
                ]),
            }),
        );
        let outcome = engine.judge(task(dir.path(), "int main(){}")).await;
        assert_eq!(outcome.verdict, Verdict::Tle);
    }

    #[tokio::test]
    async fn ce_and_transport_failures_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(Some(CompileOutcome::Ce {
                    output: "syntax error".to_string(),
                })),
            }),
            Arc::new(FakeRunner {
                results: Mutex::new(vec![]),
            }),
        );
        let outcome = engine.judge(task(dir.path(), "bad")).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Done);
        assert_eq!(outcome.verdict, Verdict::Ce);
        assert_eq!(outcome.compile_output.as_deref(), Some("syntax error"));

        let engine = RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(None),
            }),
            Arc::new(FakeRunner {
                results: Mutex::new(vec![Err(JudgeTransportError::Empty)]),
            }),
        );
        let outcome = engine.judge(task(dir.path(), "int main(){}")).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Failed);
        assert_eq!(outcome.verdict, Verdict::Ie);
        assert!(outcome.error.unwrap().contains("运行失败"));
    }
}
