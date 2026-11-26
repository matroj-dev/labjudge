mod common;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{header, HeaderMap, StatusCode};
use common::{body_string, form_body, header_value, TestApp};
use labjudge::judge::{JudgeEngine, JudgeOutcome, JudgeOutcomeStatus, JudgeTask};
use labjudge::model::{SubmissionStatus, Verdict};
use labjudge::scoring::ScoredCase;
use labjudge::store::{NewSubmission, SubmissionRow};
use labjudge::timefmt::now_ms;
use tokio::time::sleep;

#[derive(Default)]
struct FakeEngine {
    seen: Mutex<Vec<i64>>,
}

#[async_trait]
impl JudgeEngine for FakeEngine {
    async fn judge(&self, task: JudgeTask) -> JudgeOutcome {
        self.seen.lock().unwrap().push(task.submission_id);
        JudgeOutcome {
            status: JudgeOutcomeStatus::Done,
            verdict: Verdict::Ac,
            score: 100,
            cases: vec![ScoredCase {
                name: "1".to_string(),
                subtask: "子任务 1".to_string(),
                verdict: Verdict::Ac,
                time_ms: 12,
                memory_kb: 1024,
                score: 100,
                max_score: 100,
            }],
            compile_output: None,
            error: None,
        }
    }
}

async fn submit(
    app: &TestApp,
    student_id: &str,
    problem_key: &str,
    source: &str,
    csrf_override: Option<&str>,
) -> axum::response::Response {
    let (headers, expected_csrf) = app.student_auth(student_id);
    let csrf = csrf_override.unwrap_or(&expected_csrf);
    app.post(
        "/c/demo/submissions",
        headers,
        form_body(&[
            ("problem_key", problem_key),
            ("source", source),
            ("csrf", csrf),
        ]),
    )
    .await
}

async fn latest_row(app: &TestApp, student_id: &str) -> SubmissionRow {
    app.store()
        .submissions_for_student(student_id)
        .unwrap()
        .into_iter()
        .next()
        .expect("submission row")
}

async fn wait_status(app: &TestApp, id: i64) -> SubmissionRow {
    for _ in 0..200 {
        let row = app.store().submission(id).unwrap().unwrap();
        if matches!(
            row.status,
            SubmissionStatus::Done | SubmissionStatus::Failed
        ) {
            return row;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("submission {id} did not finish");
}

#[tokio::test]
async fn submit_is_queued_judged_and_source_saved() {
    let mut app = TestApp::new();
    app.start_contest();
    let engine = Arc::new(FakeEngine::default());
    let engine_seen = engine.clone();
    app = app.with_fake_engine(engine);
    let response = app.get("/c/demo/p/A", app.student_headers("2026001")).await;
    let page = body_string(response).await;
    assert!(!page.contains("disabled"));
    assert!(!page.contains("评测服务未就绪"));

    let response = submit(&app, "2026001", "A", "int main(){return 0;}", None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/c/demo/p/A?submitted=1")
    );

    let row = latest_row(&app, "2026001").await;
    assert_eq!(row.problem_key, "A");
    assert_eq!(row.language, "cpp17");
    assert!(row.source_path.is_file());
    assert_eq!(
        fs::read_to_string(&row.source_path).unwrap(),
        "int main(){return 0;}"
    );
    let finished = wait_status(&app, row.id).await;
    assert_eq!(finished.status, SubmissionStatus::Done);
    assert_eq!(finished.verdict, Some(Verdict::Ac));
    assert_eq!(finished.score, 100);
    assert!(finished.cases_json.unwrap().contains("\"AC\""));
    assert!(engine_seen.seen.lock().unwrap().contains(&row.id));
    sleep(Duration::from_millis(50)).await;
    assert_eq!(app.state.queue.as_ref().unwrap().depth(), 0);

    app.set_leaderboard_visible(true);
    let headers = app.student_headers("2026001");
    let response = app.get("/c/demo/_rank", headers.clone()).await;
    let body = body_string(response).await;
    assert!(body.contains("张三"));
    assert!(body.contains("100"));
    assert!(body.contains("最后得分时间"));

    let response = app.get("/c/demo/_mine", headers).await;
    assert!(body_string(response).await.contains("<td>AC</td>"));
}

#[tokio::test]
async fn submit_enforces_csrf_size_cooldown_and_phase() {
    let mut app = TestApp::new();
    app.start_contest();
    app = app.with_fake_engine(Arc::new(FakeEngine::default()));
    let response = submit(&app, "2026001", "A", "ok", Some("wrong")).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let too_large = "x".repeat(70_000);
    let response = submit(&app, "2026001", "A", &too_large, None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_string(response).await.contains("上限"));

    let response = submit(&app, "2026001", "A", "first", None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = submit(&app, "2026001", "A", "second", None).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(body_string(response).await.contains("提交过快"));

    app.end_contest();
    let response = submit(&app, "2026001", "B", "late", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_string(response).await.contains("未开始或已结束"));
}

#[tokio::test]
async fn submit_without_queue_returns_503_and_page_disables_button() {
    let app = TestApp::new();
    app.start_contest();
    let response = app.get("/c/demo/p/A", app.student_headers("2026001")).await;
    let body = body_string(response).await;
    assert!(body.contains("disabled"));
    assert!(body.contains("评测服务未就绪"));

    let response = submit(&app, "2026001", "A", "int main(){}", None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn queue_recovers_queued_submission_and_fails_missing_source() {
    let app = TestApp::new();
    let store = app.store();
    let source_dir = store.data_dir().join("submissions");
    fs::create_dir_all(&source_dir).unwrap();
    let source_path = source_dir.join("manual.cpp");
    fs::write(&source_path, "int main(){}").unwrap();
    let good_id = store
        .insert_submission(&NewSubmission {
            student_id: "2026001".to_string(),
            problem_key: "A".to_string(),
            language: "cpp17".to_string(),
            source_path,
            created_at: now_ms(),
        })
        .unwrap();
    let missing_id = store
        .insert_submission(&NewSubmission {
            student_id: "2026001".to_string(),
            problem_key: "B".to_string(),
            language: "cpp17".to_string(),
            source_path: store.data_dir().join("submissions/missing.cpp"),
            created_at: now_ms(),
        })
        .unwrap();

    let app = app.with_fake_engine(Arc::new(FakeEngine::default()));
    let good = wait_status(&app, good_id).await;
    assert_eq!(good.status, SubmissionStatus::Done);
    let missing = wait_status(&app, missing_id).await;
    assert_eq!(missing.status, SubmissionStatus::Failed);
    assert!(missing.error.unwrap().contains("源码文件缺失"));
    let log = fs::read_to_string(app.store().data_dir().join("judge.log")).unwrap();
    assert!(log.contains("重新入队") || log.contains("开始评测"));
}

#[tokio::test]
async fn submit_requires_login_redirect() {
    let app = TestApp::new();
    let response = app
        .post(
            "/c/demo/submissions",
            HeaderMap::new(),
            form_body(&[("problem_key", "A"), ("source", "x"), ("csrf", "x")]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/c/demo/join")
    );
}
