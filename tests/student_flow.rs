mod common;

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use common::{body_string, cookie_pair, form_body, header_value, TestApp};

fn anon_join(app: &TestApp) -> (HeaderMap, String) {
    let (cookie, token) = app.anon_csrf();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("lc_csrf={cookie}")).unwrap(),
    );
    (headers, token)
}

async fn join(
    app: &TestApp,
    headers: HeaderMap,
    token: &str,
    student_id: &str,
    password: &str,
) -> axum::response::Response {
    let pairs = [
        ("student_id", student_id),
        ("password", password),
        ("csrf", token),
    ];
    app.post("/c/demo/join", headers, form_body(&pairs)).await
}

#[tokio::test]
async fn contest_list_shows_only_valid_contests() {
    let app = TestApp::new();
    let response = app.get("/", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("M2 测试赛"));
    assert!(body.contains("未开始"));
    assert!(body.contains("题数：2"));
    assert!(!body.contains("bad id"));
    assert!(!body.contains("比赛包校验失败"));
}

#[tokio::test]
async fn join_with_correct_password_sets_session_cookie_and_redirects() {
    let app = TestApp::new();
    let (headers, token) = anon_join(&app);
    let response = join(&app, headers, &token, "2026003", "026003").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/c/demo")
    );
    let set_cookie = header_value(&response, header::SET_COOKIE).unwrap();
    assert!(set_cookie.contains("lc_s_demo="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&cookie_pair(&set_cookie)).unwrap(),
    );
    let response = app.get("/c/demo", headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("M2 测试赛"));
    // 未开赛只显示等待页，不下发题目与题面入口。
    assert!(body.contains("比赛尚未开始"));
    assert!(!body.contains("全部题面"));
    assert!(!body.contains("A. 两数之和"));
}

#[tokio::test]
async fn not_started_contest_hides_problems_until_teacher_starts() {
    let app = TestApp::new();
    let headers = app.student_headers("2026001");

    let response = app.get("/c/demo", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("比赛尚未开始"));
    assert!(body.contains("data-waiting"));
    assert!(!body.contains("A. 两数之和"));
    assert!(!body.contains("/c/demo/p/A"));

    for uri in ["/c/demo/all", "/c/demo/p/A"] {
        let response = app.get(uri, headers.clone()).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "uri={uri}");
        assert_eq!(
            header_value(&response, header::LOCATION).as_deref(),
            Some("/c/demo"),
            "uri={uri}"
        );
    }

    app.start_contest();
    let body = body_string(app.get("/c/demo", headers.clone()).await).await;
    assert!(body.contains("两数之和"));
    assert!(!body.contains("data-waiting"));
    assert_eq!(
        app.get("/c/demo/all", headers.clone()).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.get("/c/demo/p/A", headers).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn join_get_issues_csrf_cookie_and_redirects_logged_in_student() {
    let app = TestApp::new();
    let response = app.get("/c/demo/join", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = header_value(&response, header::SET_COOKIE).unwrap();
    assert!(set_cookie.contains("lc_csrf="));
    let body = body_string(response).await;
    assert!(body.contains("name=\"csrf\""));
    assert!(body.contains("进入比赛"));

    let headers = app.student_headers("2026001");
    let response = app.get("/c/demo/join", headers).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/c/demo")
    );
}

#[tokio::test]
async fn join_rejects_wrong_password_and_unknown_id_alike() {
    let app = TestApp::new();
    let (headers, token) = anon_join(&app);

    let response = join(&app, headers.clone(), &token, "2026001", "nope").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_string(response).await;
    assert!(body.contains("账号或密码错误"));
    // 名单里没有这个人和密码不对必须是同一句话。
    assert!(!body.contains("不在名单"));

    let response = join(&app, headers, &token, "2029999", "026001").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_string(response).await.contains("账号或密码错误"));
}

#[tokio::test]
async fn join_rejects_missing_or_wrong_csrf() {
    let app = TestApp::new();
    let response = app
        .post(
            "/c/demo/join",
            HeaderMap::new(),
            form_body(&[
                ("student_id", "2026003"),
                ("password", "026003"),
                ("csrf", "nope"),
            ]),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let (headers, _token) = anon_join(&app);
    let response = join(&app, headers, "wrong-token", "2026003", "026003").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unauthenticated_pages_redirect_and_fragments_return_401_json() {
    let app = TestApp::new();

    for uri in ["/c/demo", "/c/demo/all"] {
        let response = app.get(uri, HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            header_value(&response, header::LOCATION).as_deref(),
            Some("/c/demo/join")
        );
    }

    let response = app.get("/c/demo/_mine", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(body["error"]["message"].as_str().unwrap().contains("参赛"));
}

#[tokio::test]
async fn status_json_has_only_two_fields_and_tracks_phase() {
    let app = TestApp::new();
    let headers = app.student_headers("2026001");

    let response = app.get("/c/demo/status", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert_eq!(body["state"], "not_started");
    assert_eq!(body["remainingSeconds"], 0);
    assert!(body.get("verdict").is_none());

    app.start_contest();
    let response = app.get("/c/demo/status", headers.clone()).await;
    let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(body["state"], "running");
    assert!(body["remainingSeconds"].as_u64().unwrap() > 0);

    app.end_contest();
    let response = app.get("/c/demo/status", headers).await;
    let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(body["state"], "ended");
    assert_eq!(body["remainingSeconds"], 0);
}

#[tokio::test]
async fn problem_and_all_pages_render_statement_math_and_editor() {
    let app = TestApp::new();
    app.start_contest();
    let headers = app.student_headers("2026001");

    let response = app.get("/c/demo/p/A", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("A. 两数之和"));
    assert!(body.contains("\\(a+b\\)"));
    assert!(body.contains("&lt;script&gt;"));
    assert!(!body.contains("<script>alert(1)</script>"));
    assert!(body.contains("/c/demo/p/A/files/images/example.png"));
    assert!(body.contains("/static/vendor/codemirror/codemirror.js"));
    assert!(body.contains("/static/vendor/mathjax/tex-svg.js"));
    assert!(body.contains("id=\"editor\""));
    assert!(body.contains("disabled"));
    assert!(body.contains("进行中"));
    // 题目页不再展示提交记录，避免和「我的信息」标签里的提交列表重复。
    assert!(!body.contains("我的本题提交"));
    assert!(!body.contains("_subs"));
    assert!(!body.contains("已提交，正在评测"));

    let body = body_string(app.get("/c/demo/p/A?submitted=1", headers.clone()).await).await;
    assert!(body.contains("已提交，正在评测"));
    assert!(body.contains("/c/demo#me"));

    let response = app.get("/c/demo/all", headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let a = body.find("A. 两数之和").unwrap();
    let b = body.find("B. 数列").unwrap();
    assert!(a < b);
    assert!(!body.contains("action=\"/c/demo/submissions\""));
    assert!(body.contains("/static/vendor/mathjax/tex-svg.js"));

    app.end_contest();
    assert_eq!(
        app.get("/c/demo/p/A", headers.clone()).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.get("/c/demo/all", headers).await.status(),
        StatusCode::OK
    );

    assert_eq!(
        app.get("/c/demo/p/Z", app.student_headers("2026001"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn logout_clears_student_cookie_with_csrf() {
    let app = TestApp::new();
    let (headers, csrf) = app.student_auth("2026001");
    let body = form_body(&[("contest_id", "demo"), ("csrf", &csrf)]);
    let response = app.post("/logout", headers, body).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_value(&response, header::LOCATION).as_deref(),
        Some("/")
    );
    let set_cookie = header_value(&response, header::SET_COOKIE).unwrap();
    assert!(set_cookie.contains("lc_s_demo="));
    assert!(set_cookie.contains("Max-Age=0"));
}

#[tokio::test]
async fn problem_files_are_served_with_type_and_traversal_is_blocked() {
    let app = TestApp::new();
    let headers = app.student_headers("2026001");

    let response = app
        .get("/c/demo/p/A/files/images/example.png", headers.clone())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .unwrap()
            .to_str()
            .unwrap(),
        "nosniff"
    );

    assert_eq!(
        app.get("/c/demo/p/A/files/images/bad.svg", headers.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/c/demo/p/A/files/%2e%2e%2fcontest.json", headers.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/c/demo/p/A/files/images/example.png", HeaderMap::new())
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn home_shows_problem_mine_and_rank_tabs() {
    let app = TestApp::new();
    app.start_contest();
    let headers = app.student_headers("2026001");
    let response = app.get("/c/demo", headers).await;
    let body = body_string(response).await;
    assert!(body.contains("两数之和"));
    assert!(body.contains("数列"));
    assert!(body.contains("data-countdown"));
    assert!(body.contains("进行中"));
    assert!(body.contains("data-tab=\"problems\""));
    assert!(body.contains("data-tab=\"me\""));
    assert!(body.contains("我的信息"));
    assert!(body.contains("data-tab=\"rank\""));
    assert!(body.contains("data-fragment-url=\"/c/demo/_mine\""));
    assert!(body.contains("data-fragment-url=\"/c/demo/_rank\""));
    assert!(body.contains("老师暂未公开排行榜"));

    app.set_leaderboard_visible(true);
    let body = body_string(app.get("/c/demo", app.student_headers("2026001")).await).await;
    assert!(body.contains("最后得分时间"));
}

#[tokio::test]
async fn missing_contest_and_problem_render_chinese_error_pages() {
    let app = TestApp::new();
    let response = app.get("/c/missing/join", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_string(response).await;
    assert!(body.contains("比赛不存在"));
    assert!(body.contains("返回比赛列表"));

    let response = app.get("/c/demo/p/Z", app.student_headers("2026001")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(body_string(response).await.contains("题目不存在"));
}

#[tokio::test]
async fn app_js_contains_countdown_fragment_and_draft_hooks() {
    let app = TestApp::new();
    let response = app.get("/static/app.js", HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("initCountdown"));
    assert!(body.contains("initFragments"));
    assert!(body.contains("restoreOpenKeys"));
    assert!(body.contains("initEditor"));
    assert!(body.contains("lc:draft:"));
    assert!(body.contains("replaceSelection"));
}
