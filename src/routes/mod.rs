use axum::routing::get;
use axum::Router;

use crate::app_state::AppState;

mod access;
mod admin_auth;
mod admin_contest;
mod admin_dashboard;
mod admin_submissions;
mod admin_system;
mod assets;
pub mod error;
mod export;
mod fragments;
mod student;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .merge(assets::router())
        .merge(student::router())
        .merge(fragments::router())
        .merge(admin_auth::router())
        .merge(admin_dashboard::router())
        .merge(admin_contest::router())
        .merge(admin_submissions::router())
        .merge(admin_system::router())
        .merge(export::router())
}

async fn healthz() -> &'static str {
    "ok"
}
