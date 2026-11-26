use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use super::access::{admin_session, contest_or_error, redirect_admin_login};
use super::error::AppError;
use crate::app_state::AppState;
use crate::export::csv::build_csv;
use crate::export::zip::build_zip;
use crate::export::{csv_response, zip_response};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/c/{id}/export.csv", get(export_csv))
        .route("/admin/c/{id}/sources.zip", get(export_zip))
}

async fn export_csv(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(_admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let submissions = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?;
    let bytes = build_csv(
        &runtime.package.roster,
        &runtime.package.problems,
        &submissions,
    )
    .map_err(|error| AppError::internal(format!("生成成绩 CSV 失败: {error}")))?;
    Ok(csv_response(bytes, &format!("{contest_id}.csv")))
}

async fn export_zip(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(_admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let submissions = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?;
    let bytes = build_zip(&runtime.package.roster, &submissions)
        .map_err(|error| AppError::internal(format!("生成源码 zip 失败: {error}")))?;
    Ok(zip_response(bytes, &format!("{contest_id}-sources.zip")))
}
