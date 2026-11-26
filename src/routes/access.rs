use std::sync::Arc;

use askama::Template;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{Html, IntoResponse, Redirect, Response};

use super::error::AppError;
use crate::app_state::AppState;
use crate::auth::session::{
    read_cookie, student_cookie_name, AdminSession, StudentSession, ADMIN_COOKIE,
};
use crate::contest::lifecycle::{phase, ContestState};
use crate::model::Phase;
use crate::services::registry::ContestRuntime;
use crate::timefmt::now_ms;

pub fn contest_or_error(
    state: &AppState,
    contest_id: &str,
    json: bool,
) -> Result<Arc<ContestRuntime>, AppError> {
    state.registry.get(contest_id).ok_or_else(|| {
        let message = "比赛不存在或未通过校验";
        if json {
            AppError::not_found_json(message)
        } else {
            AppError::not_found(message)
        }
    })
}

pub fn current_student(
    state: &AppState,
    headers: &HeaderMap,
    contest_id: &str,
    json: bool,
) -> Result<Option<(Arc<ContestRuntime>, StudentSession)>, AppError> {
    let runtime = contest_or_error(state, contest_id, json)?;
    let cookie_name = student_cookie_name(contest_id);
    let Some(token) = read_cookie(headers, &cookie_name) else {
        return Ok(None);
    };
    let now = now_ms();
    let Ok(session) = state.session.verify_student(&token, contest_id, now) else {
        return Ok(None);
    };

    let current_session = runtime
        .store
        .session_id(&session.student_id)
        .map_err(|error| AppError::internal(format!("读取会话失败: {error}")))?;
    if current_session.as_deref() != Some(session.session_id.as_str()) {
        return Ok(None);
    }

    if !runtime
        .package
        .roster
        .students()
        .iter()
        .any(|student| student.student_id == session.student_id)
    {
        return Ok(None);
    }

    // 参赛记录在 join 时写入；之后只是刷新「最近活动」，10 秒内不重复写库。
    let touched = runtime
        .store
        .touch_participant_throttled(&session.student_id, now, 10_000)
        .map_err(|error| AppError::internal(format!("更新参赛记录失败: {error}")))?;
    if !touched {
        let existing = runtime
            .store
            .participant(&session.student_id)
            .map_err(|error| AppError::internal(format!("读取参赛记录失败: {error}")))?;
        if existing.is_none() {
            return Ok(None);
        }
    }

    Ok(Some((runtime, session)))
}

pub fn contest_phase(runtime: &ContestRuntime) -> Result<Phase, AppError> {
    runtime
        .store
        .state()
        .map(|state| phase(&state, now_ms()))
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))
}

pub fn student_name(runtime: &ContestRuntime, student_id: &str) -> String {
    runtime
        .package
        .roster
        .students()
        .iter()
        .find(|student| student.student_id == student_id)
        .map(|student| student.name.clone())
        .unwrap_or_else(|| student_id.to_string())
}

pub fn admin_session(state: &AppState, headers: &HeaderMap) -> Option<AdminSession> {
    let token = read_cookie(headers, ADMIN_COOKIE)?;
    state.session.verify_admin(&token, now_ms()).ok()
}

pub fn require_admin_json(state: &AppState, headers: &HeaderMap) -> Result<AdminSession, AppError> {
    admin_session(state, headers).ok_or_else(|| AppError::unauthorized_json("请先登录管理端"))
}

pub fn redirect_admin_login() -> Response {
    Redirect::to("/admin/login").into_response()
}

pub fn remaining_seconds(state: &ContestState, now_ms: i64) -> u64 {
    if phase(state, now_ms) != Phase::Running {
        return 0;
    }
    state
        .ends_at
        .map(|ends_at| ((ends_at - now_ms).max(0) / 1000) as u64)
        .unwrap_or(0)
}

pub fn render_html<T: Template>(template: T) -> Result<Response, AppError> {
    let html = template
        .render()
        .map_err(|error| AppError::internal(format!("页面渲染失败: {error}")))?;
    let mut response = Html(html).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

pub fn no_store_html(html: String) -> Response {
    let mut response = Html(html).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub fn redirect_join(contest_id: &str) -> Response {
    Redirect::to(&format!("/c/{contest_id}/join")).into_response()
}

pub fn with_cookie(mut response: Response, cookie: HeaderValue) -> Response {
    response.headers_mut().append(header::SET_COOKIE, cookie);
    response
}
