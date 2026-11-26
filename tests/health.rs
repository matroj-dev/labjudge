use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use labjudge::app::build_router;
use labjudge::app_state::AppState;
use tower::ServiceExt;

async fn get(uri: &str) -> axum::response::Response {
    let app = build_router(AppState::default());
    app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn healthz_ok() {
    let response = get("/healthz").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn static_css_is_embedded() {
    let response = get("/static/app.css").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/css; charset=utf-8"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("body"));
}

#[tokio::test]
async fn vendor_scripts_are_embedded() {
    let response = get("/static/vendor/mathjax/tex-svg.js").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("MathJax"));

    let response = get("/static/vendor/codemirror/codemirror.js").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("CodeMirror"));
}

#[tokio::test]
async fn missing_and_traversal_static_files_are_404() {
    assert_eq!(get("/static/nope.js").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get("/static/%2e%2e%2fCargo.toml").await.status(),
        StatusCode::NOT_FOUND
    );
}
