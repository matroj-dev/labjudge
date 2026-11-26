use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use super::access::{admin_session, redirect_admin_login, render_html};
use super::error::AppError;
use crate::app_state::AppState;
use crate::net::{access_urls, format_bytes};
use crate::views::admin::AdminSystemTemplate;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/system", get(system))
}

async fn system(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let compiler_path = state
        .compiler
        .as_ref()
        .map(|compiler| compiler.path().display().to_string())
        .unwrap_or_else(|| "-".to_string());
    let compiler_version = state
        .compiler
        .as_ref()
        .map(|compiler| compiler.version().to_string())
        .unwrap_or_else(|| "不可用".to_string());
    let judge_version = state
        .supervisor
        .as_ref()
        .map(|supervisor| supervisor.version().to_string())
        .unwrap_or_else(|| "不可用".to_string());
    let judge_alive = state
        .supervisor
        .as_ref()
        .map(|supervisor| supervisor.is_alive())
        .unwrap_or(false);
    let free = fs2::available_space(&state.paths.root).unwrap_or(0);
    let total = fs2::total_space(&state.paths.root).unwrap_or(0);

    Ok(render_html(AdminSystemTemplate {
        compiler_path,
        compiler_version,
        judge_version,
        judge_alive,
        port: state.port,
        addresses: access_urls(state.port),
        disk_free_text: format_bytes(free),
        disk_total_text: format_bytes(total),
        csrf: state.csrf.for_session(&admin.token),
    })?
    .into_response())
}
