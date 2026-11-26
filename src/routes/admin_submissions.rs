use std::fs;

use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::access::{admin_session, contest_or_error, redirect_admin_login, render_html};
use super::error::AppError;
use crate::app_state::AppState;
use crate::model::SubmissionStatus;
use crate::services::registry::ContestRuntime;
use crate::timefmt::{format_local, now_ms};
use crate::views::admin::{AdminSubmissionRow, AdminSubmissionTemplate, AdminSubmissionsTemplate};
use crate::views::fragments::{status_label, submission_view};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/c/{id}/submissions", get(list))
        .route("/admin/s/{sid}", get(detail))
        .route("/admin/s/{sid}/rejudge", post(rejudge))
        .route("/admin/s/{sid}/delete", post(delete))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    page: Option<usize>,
}

async fn list(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    Query(query): Query<ListQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let page = query.page.unwrap_or(1).max(1);
    let mut rows = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?;
    rows.reverse();
    let total = rows.len();
    let start = (page - 1) * 50;
    let page_rows = rows
        .into_iter()
        .skip(start)
        .take(50)
        .map(|row| AdminSubmissionRow {
            sid: format!("{}~{}", contest_id, row.id),
            created_text: format_local(row.created_at),
            student_name: student_name(&runtime, &row.student_id),
            problem_key: row.problem_key,
            status_label: status_label(row.status).to_string(),
            verdict_label: row
                .verdict
                .map(|verdict| verdict.as_str().to_string())
                .unwrap_or_else(|| "-".to_string()),
            score: row.score,
        })
        .collect::<Vec<_>>();

    Ok(render_html(AdminSubmissionsTemplate {
        contest_id,
        rows: page_rows,
        page,
        has_next: start + 50 < total,
        csrf: state.csrf.for_session(&admin.token),
    })?
    .into_response())
}

async fn detail(
    State(state): State<AppState>,
    Path(sid): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let (runtime, id) = parse_sid(&state, &sid)?;
    let row = runtime
        .store
        .submission(id)
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?
        .ok_or_else(|| AppError::not_found("提交不存在"))?;
    let source =
        fs::read_to_string(&row.source_path).unwrap_or_else(|_| "<源码文件缺失>".to_string());
    let view = submission_view(&row, true);

    Ok(render_html(AdminSubmissionTemplate {
        sid,
        source,
        submission: view,
        csrf: state.csrf.for_session(&admin.token),
    })?
    .into_response())
}

#[derive(Deserialize)]
struct ConfirmForm {
    csrf: String,
    #[serde(default)]
    confirm: String,
}

fn verify_confirm(
    state: &AppState,
    admin: &crate::auth::session::AdminSession,
    form: &ConfirmForm,
) -> Result<(), AppError> {
    if !state.csrf.verify_session(&admin.token, &form.csrf) {
        return Err(AppError::forbidden("表单校验失败"));
    }
    if form.confirm != "yes" {
        return Err(AppError::bad_request("该操作需要确认"));
    }
    Ok(())
}

async fn rejudge(
    State(state): State<AppState>,
    Path(sid): Path<String>,
    headers: HeaderMap,
    form: Result<Form<ConfirmForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_confirm(&state, &admin, &form)?;
    let (runtime, id) = parse_sid(&state, &sid)?;
    ensure_no_active(&runtime)?;
    let Some(queue) = &state.queue else {
        return Err(AppError::bad_request("判题队列未就绪"));
    };
    let row = runtime
        .store
        .submission(id)
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?
        .ok_or_else(|| AppError::not_found("提交不存在"))?;
    if !matches!(
        row.status,
        SubmissionStatus::Done | SubmissionStatus::Failed
    ) {
        return Err(AppError::bad_request("只能重测已完成或失败的提交"));
    }
    runtime
        .store
        .clear_result(id)
        .map_err(|error| AppError::internal(format!("重置提交失败: {error}")))?;
    if !queue.enqueue(row.contest_id.clone(), id) {
        return Err(AppError::internal("判题队列已关闭"));
    }
    log(&runtime, &format!("教师单条重测提交 {id}"));
    Ok(Redirect::to(&format!("/admin/s/{sid}")).into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(sid): Path<String>,
    headers: HeaderMap,
    form: Result<Form<ConfirmForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_confirm(&state, &admin, &form)?;
    let (runtime, id) = parse_sid(&state, &sid)?;
    ensure_no_active(&runtime)?;
    let row = runtime
        .store
        .submission(id)
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?
        .ok_or_else(|| AppError::not_found("提交不存在"))?;
    if !matches!(
        row.status,
        SubmissionStatus::Done | SubmissionStatus::Failed
    ) {
        return Err(AppError::bad_request("只能删除已完成或失败的提交"));
    }
    runtime
        .store
        .delete_submission(id)
        .map_err(|error| AppError::internal(format!("删除提交失败: {error}")))?;
    let _ = fs::remove_file(&row.source_path);
    log(&runtime, &format!("教师删除提交 {id}"));
    Ok(Redirect::to(&format!("/admin/c/{}/submissions", row.contest_id)).into_response())
}

fn parse_sid(
    state: &AppState,
    sid: &str,
) -> Result<(std::sync::Arc<ContestRuntime>, i64), AppError> {
    let (contest_id, id) = sid
        .split_once('~')
        .ok_or_else(|| AppError::not_found("提交编号非法"))?;
    let id = id
        .parse::<i64>()
        .map_err(|_| AppError::not_found("提交编号非法"))?;
    let runtime = contest_or_error(state, contest_id, false)?;
    Ok((runtime, id))
}

fn ensure_no_active(runtime: &ContestRuntime) -> Result<(), AppError> {
    let active = runtime
        .store
        .active_count()
        .map_err(|error| AppError::internal(format!("读取活动任务失败: {error}")))?;
    if active > 0 {
        return Err(AppError::bad_request(format!(
            "该场仍有 {active} 条排队/评测中的提交，请等待完成"
        )));
    }
    Ok(())
}

fn student_name(runtime: &ContestRuntime, student_id: &str) -> String {
    runtime
        .package
        .roster
        .students()
        .iter()
        .find(|student| student.student_id == student_id)
        .map(|student| student.name.clone())
        .unwrap_or_else(|| "未知学生".to_string())
}

fn log(runtime: &ContestRuntime, message: &str) {
    let line = format!("{} {message}", format_local(now_ms()));
    let _ = runtime.store.append_log(&line);
}
