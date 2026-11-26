mod common;

use axum::http::{HeaderMap, StatusCode};
use common::{body_string, TestApp};
use labjudge::model::Verdict;

#[tokio::test]
async fn hidden_mode_never_leaks_verdict_score_cases_or_compile_output() {
    let app = TestApp::new();
    app.start_contest();
    app.add_done_submission(
        "2026001",
        "A",
        Verdict::Ac,
        987654,
        Some("CE_SECRET_987654"),
    );
    let headers = app.student_headers("2026001");

    for uri in ["/c/demo", "/c/demo/_mine"] {
        let response = app.get(uri, headers.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("已评测"), "missing status in {uri}");
        assert!(!body.contains("987654"), "leaked score in {uri}");
        assert!(!body.contains("CE_SECRET_987654"), "leaked CE in {uri}");
        assert!(!body.contains("编译输出"), "leaked compile block in {uri}");
        assert!(!body.contains(">AC<"), "leaked verdict in {uri}");
        assert!(!body.contains("小数据"), "leaked subtask in {uri}");
    }

    for uri in ["/c/demo", "/c/demo/_rank"] {
        let response = app.get(uri, headers.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_string(response).await.contains("老师暂未公开"));
    }
}

#[tokio::test]
async fn public_mode_shows_verdict_score_subtasks_and_compile_output() {
    let app = TestApp::new();
    app.start_contest();
    app.add_done_submission(
        "2026001",
        "A",
        Verdict::Ac,
        987654,
        Some("CE_SECRET_987654"),
    );
    app.set_leaderboard_visible(true);
    let headers = app.student_headers("2026001");

    for uri in ["/c/demo/_mine", "/c/demo"] {
        let response = app.get(uri, headers.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains(">AC<"), "missing verdict in {uri}");
        assert!(body.contains("987654"), "missing score in {uri}");
        assert!(body.contains("小数据"), "missing subtask in {uri}");
        assert!(body.contains("CE_SECRET_987654"), "missing CE in {uri}");
    }

    let response = app.get("/c/demo/_rank", headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("排名"));
    assert!(body.contains("最后得分时间"));
    assert!(body.contains("张三"));
    assert!(body.contains("李四"));
    assert!(body.contains("987654"));
}

#[tokio::test]
async fn failed_submission_uses_review_abnormal_label_in_hidden_mode() {
    let app = TestApp::new();
    let id = app.add_queued_submission("2026001", "A");
    app.store().mark_failed(id, "go-judge 内部错误", 1).unwrap();
    let headers = app.student_headers("2026001");

    let response = app.get("/c/demo/_mine", headers.clone()).await;
    let body = body_string(response).await;
    assert!(body.contains("评测异常"));
    assert!(!body.contains("go-judge 内部错误"));
}

#[tokio::test]
async fn fragments_without_session_are_json_401() {
    let app = TestApp::new();
    for uri in ["/c/demo/_mine", "/c/demo/_rank", "/c/demo/status"] {
        let response = app.get(uri, HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        assert_eq!(body["error"]["code"], "unauthorized");
    }
}
