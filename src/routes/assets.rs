use std::fs;
use std::path::Path as FilePath;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

use super::access::{contest_phase, current_student, redirect_join};
use super::error::AppError;
use crate::app_state::AppState;
use crate::model::Phase;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/static/{*path}", get(static_file))
        .route("/c/{id}/p/{key}/files/{*path}", get(problem_file))
}

async fn static_file(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mut response = Response::new(Body::from(file.data.into_owned()));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&path)),
            );
            // 自己的样式/脚本没有版本号，升级后必须让浏览器重新取，
            // 否则会拿旧 CSS 渲染新页面。第三方 vendor 资源保持不变，继续走浏览器缓存。
            if matches!(path.as_str(), "app.css" | "app.js") {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            response
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn problem_file(
    State(state): State<AppState>,
    Path((contest_id, problem_key, relative_path)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((runtime, _session)) = current_student(&state, &headers, &contest_id, false)? else {
        return Ok(redirect_join(&contest_id));
    };
    if contest_phase(&runtime)? == Phase::Ended {
        return Err(AppError::not_found("比赛已结束"));
    }
    let Some(problem) = runtime.package.find_problem(&problem_key) else {
        return Err(AppError::not_found("题目不存在"));
    };

    let candidate = problem.root.join(&relative_path);
    let path = fs::canonicalize(&candidate).map_err(|_| AppError::not_found("资源不存在"))?;
    if !path.starts_with(&problem.root) || !path.is_file() {
        return Err(AppError::not_found("资源不存在"));
    }
    let Some(mime) = image_content_type(&path) else {
        return Err(AppError::not_found("资源不存在"));
    };
    let data = fs::read(&path).map_err(|_| AppError::not_found("资源不存在"))?;

    let mut response = Response::new(Body::from(data));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn image_content_type(path: &FilePath) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "md" | "license" => "text/plain; charset=utf-8",
        _ if path.ends_with("LICENSE") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
