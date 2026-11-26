mod common;

use std::fs;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use common::{body_string, cookie_pair, form_body, header_value, TestApp};
use labjudge::contest::lifecycle::phase;
use labjudge::judge::{JudgeEngine, JudgeOutcome, JudgeOutcomeStatus, JudgeTask};
use labjudge::model::{Phase, SubmissionStatus, Verdict};
use labjudge::scoring::ScoredCase;
use labjudge::timefmt::now_ms;
use tokio::time::sleep;

#[derive(Default)]
struct AcceptEngine {
    seen: AtomicUsize,
}

#[async_trait]
impl JudgeEngine for AcceptEngine {
    async fn judge(&self, _task: JudgeTask) -> JudgeOutcome {
        self.seen.fetch_add(1, Ordering::SeqCst);
        JudgeOutcome {
            status: JudgeOutcomeStatus::Done,
            verdict: Verdict::Ac,
            score: 100,
            cases: vec![ScoredCase {
                name: "1".to_string(),
                subtask: "子任务 1".to_string(),
                verdict: Verdict::Ac,
                time_ms: 10,
                memory_kb: 1024,
                score: 100,
                max_score: 100,
            }],
            compile_output: None,
            error: None,
        }
    }
}

fn anon_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("lc_csrf={token}")).unwrap(),
    );
    headers
}

async fn login(app: &TestApp, password: &str) -> axum::response::Response {
    let mut headers = HeaderMap::new();
    let (cookie, token) = app.anon_csrf();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("lc_csrf={cookie}")).unwrap(),
    );
    app.post(
        "/admin/login",
        headers,
        form_body(&[("password", password), ("csrf", &token)]),
    )
    .await
}

async fn admin_post(app: &TestApp, path: &str, extra: &[(&str, &str)]) -> axum::response::Response {
    let (headers, csrf) = app.admin_auth();
    let mut pairs = vec![("csrf", csrf.as_str())];
    pairs.extend_from_slice(extra);
    app.post(path, headers, form_body(&pairs)).await
}

async fn wait_status(app: &TestApp, id: i64) -> labjudge::store::SubmissionRow {
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
async fn admin_login_requires_csrf_and_checks_password() {
    let app = TestApp::new();

    let response = app.get("/admin", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/admin/login")
    );

    let response = app.get("/admin/login", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_value(&response, header::SET_COOKIE)
        .unwrap()
        .contains("lc_csrf="));
    assert!(body_string(response).await.contains("name=\"csrf\""));

    let (cookie, token) = app.anon_csrf();
    let headers = anon_headers(&cookie);

    // 缺 CSRF cookie：拒绝，即使口令是对的。
    let response = app
        .post(
            "/admin/login",
            HeaderMap::new(),
            form_body(&[("password", "000000"), ("csrf", &token)]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // CSRF 对不上：同样拒绝。
    let response = app
        .post(
            "/admin/login",
            headers.clone(),
            form_body(&[("password", "000000"), ("csrf", "nope")]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .post(
            "/admin/login",
            headers,
            form_body(&[("password", "bad"), ("csrf", &token)]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(body_string(response).await.contains("管理口令错误"));

    let fresh = TestApp::new();
    let response = login(&fresh, "000000").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/admin")
    );
    let set_cookie = header_value(&response, header::SET_COOKIE).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&cookie_pair(&set_cookie)).unwrap(),
    );
    let response = fresh.get("/admin", headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("管理端"));
    assert!(body.contains("bad id"));
    assert!(body.contains("ID 非法"));
}

#[tokio::test]
async fn contest_page_tabs_and_export_only_after_end() {
    let app = TestApp::new();
    let (headers, _csrf) = app.admin_auth();

    let body = body_string(app.get("/admin/c/demo", headers.clone()).await).await;
    for marker in [
        "data-tabs",
        "data-tab-panel=\"info\"",
        "data-tab-panel=\"monitor\"",
        "data-tab-panel=\"roster\"",
        "data-tab-panel=\"rank\"",
        "参赛者名单",
        "实时监控",
    ] {
        assert!(body.contains(marker), "missing {marker}");
    }
    assert!(!body.contains("只看 CE"), "CE-only link should be gone");
    // 教师始终看得到排行榜，不受隐藏开关影响。
    assert!(body.contains("排名"));
    // 比赛未结束不提供导出入口。
    assert!(!body.contains("/admin/c/demo/export.csv"));

    app.start_contest();
    app.end_contest();
    let body = body_string(app.get("/admin/c/demo", headers).await).await;
    assert!(body.contains("/admin/c/demo/export.csv"));
    assert!(body.contains("/admin/c/demo/sources.zip"));
}

#[tokio::test]
async fn admin_start_end_extend_leaderboard_reload_and_reset() {
    let no_queue = TestApp::new();
    let response = admin_post(&no_queue, "/admin/c/demo/start", &[]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let app = TestApp::new().with_fake_engine(Arc::new(AcceptEngine::default()));
    let (headers, _csrf) = app.admin_auth();

    let response = admin_post(&app, "/admin/c/demo/start", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        phase(&app.store().state().unwrap(), now_ms()),
        Phase::Running
    );

    let response = admin_post(&app, "/admin/c/demo/leaderboard", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(app.store().state().unwrap().leaderboard_visible);

    let response = admin_post(&app, "/admin/c/demo/end", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(phase(&app.store().state().unwrap(), now_ms()), Phase::Ended);

    let response = admin_post(&app, "/admin/c/demo/extend", &[("minutes", "5")]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        phase(&app.store().state().unwrap(), now_ms()),
        Phase::Running
    );

    let response = app.get("/admin/c/demo", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("M2 测试赛"));
    assert!(body.contains("张三"));
    assert!(body.contains("全部重测"));
    // 名单只展示姓名，学生的 ID 不上页面。
    assert!(body.contains("<th>姓名</th>"));
    assert!(!body.contains("2026001"));

    let response = admin_post(&app, "/admin/c/demo/reload", &[]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = admin_post(&app, "/admin/c/demo/start", &[]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = admin_post(&app, "/admin/c/demo/reset", &[("confirm", "yes")]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        phase(&app.store().state().unwrap(), now_ms()),
        Phase::NotStarted
    );
    assert!(app.store().submissions_for_contest().unwrap().is_empty());
}

#[tokio::test]
async fn admin_reload_updates_roster_only_before_start_and_blocks_active_reset() {
    let app = TestApp::new().with_fake_engine(Arc::new(AcceptEngine::default()));
    let students = app.contest_dir().join("students.csv");
    fs::write(
        &students,
        "name,student_id\n张三,2026001\n李四,2026003\n王五,2026004\n",
    )
    .unwrap();
    let response = admin_post(&app, "/admin/c/demo/reload", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        app.state
            .registry
            .get("demo")
            .unwrap()
            .package
            .roster
            .students()
            .len(),
        3
    );

    app.store()
        .insert_submission(&labjudge::store::NewSubmission {
            student_id: "2026001".to_string(),
            problem_key: "A".to_string(),
            language: "cpp17".to_string(),
            source_path: std::path::PathBuf::from("C:/missing.cpp"),
            created_at: now_ms(),
        })
        .unwrap();
    let response = admin_post(&app, "/admin/c/demo/reset", &[("confirm", "yes")]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = admin_post(&app, "/admin/c/demo/rejudge-all", &[("confirm", "yes")]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_monitor_and_system_pages() {
    let app = TestApp::new().with_fake_engine(Arc::new(AcceptEngine::default()));
    let (headers, _csrf) = app.admin_auth();
    let _ = admin_post(&app, "/admin/c/demo/start", &[]).await;

    let response = app.get("/c/demo", app.student_headers("2026001")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.get("/admin/c/demo/_monitor", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("排队"));
    assert!(body.contains("评测器"));

    let response = app.get("/admin/c/demo/_recent", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("暂无提交"));

    let response = app.get("/admin/c/demo/_monitor", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 学生进入过比赛后，名单里应标记为「已进入」。
    let body = body_string(app.get("/admin/c/demo", headers.clone()).await).await;
    assert!(body.contains("已进入"));
    assert!(body.contains("data-fragment-url=\"/admin/c/demo/_recent\""));

    let response = app.get("/admin/system", headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("系统信息"));
    assert!(body.contains("go-judge"));
}

#[tokio::test]
async fn admin_full_flow_submit_rejudge_delete_export_reset() {
    let engine = Arc::new(AcceptEngine::default());
    let engine_seen = engine.clone();
    let app = TestApp::new().with_fake_engine(engine);
    assert_eq!(
        admin_post(&app, "/admin/c/demo/start", &[]).await.status(),
        StatusCode::SEE_OTHER
    );

    let (student_headers, student_csrf) = app.student_auth("2026001");
    let response = app
        .post(
            "/c/demo/submissions",
            student_headers,
            form_body(&[
                ("problem_key", "A"),
                ("source", "int main(){return 0;}"),
                ("csrf", &student_csrf),
            ]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let id = app
        .store()
        .submissions_for_student("2026001")
        .unwrap()
        .first()
        .unwrap()
        .id;
    let done = wait_status(&app, id).await;
    assert_eq!(done.status, SubmissionStatus::Done);
    assert_eq!(done.verdict, Some(Verdict::Ac));
    let sid = format!("demo~{id}");

    let (headers, _) = app.admin_auth();
    let response = app.get("/admin/c/demo/submissions", headers.clone()).await;
    let body = body_string(response).await;
    assert!(body.contains(&sid));
    assert!(body.contains("张三"));

    let response = app.get(&format!("/admin/s/{sid}"), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("int main(){return 0;}"));
    assert!(body.contains("重测"));

    let response = app.get("/admin/c/demo/export.csv", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &body_string(response).await.as_bytes()[..3],
        &[0xEF, 0xBB, 0xBF]
    );
    let response = app.get("/admin/c/demo/sources.zip", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    assert!(archive.by_name("2026001_张三/A_1.cpp").is_ok());

    let response = admin_post(&app, "/admin/c/demo/rejudge-all", &[("confirm", "yes")]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let _ = wait_status(&app, id).await;
    assert!(engine_seen.seen.load(Ordering::SeqCst) >= 2);

    let response = admin_post(
        &app,
        &format!("/admin/s/{sid}/rejudge"),
        &[("confirm", "yes")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let _ = wait_status(&app, id).await;

    let response = admin_post(
        &app,
        &format!("/admin/s/{sid}/delete"),
        &[("confirm", "yes")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(app.store().submission(id).unwrap().is_none());

    let response = admin_post(&app, "/admin/c/demo/reset", &[("confirm", "yes")]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        phase(&app.store().state().unwrap(), now_ms()),
        Phase::NotStarted
    );
}

#[tokio::test]
async fn admin_password_change_updates_login() {
    let app = TestApp::new();
    let (headers, csrf) = app.admin_auth();
    let response = app
        .post(
            "/admin/password",
            headers,
            form_body(&[("new_password", "654321"), ("csrf", &csrf)]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        *app.state
            .admin_password
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        "654321"
    );
    assert!(app.state.paths.root.join("config.json").is_file());

    let response = login(&app, "000000").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = login(&app, "654321").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}
