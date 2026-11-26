use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Form, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::access::{admin_session, redirect_admin_login, render_html, with_cookie};
use super::error::AppError;
use crate::app_state::AppState;
use crate::auth::session::{
    build_cookie, clear_cookie, read_cookie, ADMIN_COOKIE, CSRF_COOKIE, SESSION_SECONDS,
};
use crate::views::admin::AdminLoginTemplate;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/login", get(login_get).post(login_post))
        .route("/admin/logout", post(logout))
        .route("/admin/password", post(password))
}

async fn login_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if admin_session(&state, &headers).is_some() {
        return Ok(Redirect::to("/admin").into_response());
    }
    let (cookie_value, form_token) = state.csrf.issue_anon();
    let html = AdminLoginTemplate {
        csrf: form_token,
        error: String::new(),
    };
    let response = render_html(html)?.into_response();
    Ok(with_cookie(
        response,
        build_cookie(CSRF_COOKIE, &cookie_value, SESSION_SECONDS),
    ))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
    csrf: String,
}

async fn login_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<LoginForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    let Some(csrf_cookie) = read_cookie(&headers, CSRF_COOKIE) else {
        return Err(AppError::forbidden("缺少表单校验信息，请刷新页面重试"));
    };
    if !state.csrf.verify_anon(&csrf_cookie, &form.csrf) {
        return Err(AppError::forbidden("表单校验失败，请刷新页面重试"));
    }

    let expected = state
        .admin_password
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if form.password != expected {
        return login_error(&form, StatusCode::UNAUTHORIZED, "管理口令错误");
    }

    let token = state.session.sign_admin(crate::timefmt::now_ms());
    let mut response = Redirect::to("/admin").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        build_cookie(ADMIN_COOKIE, &token, SESSION_SECONDS),
    );
    Ok(response)
}

fn login_error(form: &LoginForm, status: StatusCode, message: &str) -> Result<Response, AppError> {
    let html = AdminLoginTemplate {
        csrf: form.csrf.clone(),
        error: message.to_string(),
    };
    let html = html
        .render()
        .map_err(|error| AppError::internal(format!("登录页渲染失败: {error}")))?;
    Ok((status, Html(html)).into_response())
}

#[derive(Deserialize)]
struct AdminCsrfForm {
    csrf: String,
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<AdminCsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    if let Some(admin) = admin_session(&state, &headers) {
        if !state.csrf.verify_session(&admin.token, &form.csrf) {
            return Err(AppError::forbidden("表单校验失败"));
        }
    }
    let response = Redirect::to("/admin/login").into_response();
    Ok(with_cookie(response, clear_cookie(ADMIN_COOKIE)))
}

#[derive(Deserialize)]
struct PasswordForm {
    new_password: String,
    csrf: String,
}

async fn password(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<PasswordForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    if !state.csrf.verify_session(&admin.token, &form.csrf) {
        return Err(AppError::forbidden("表单校验失败"));
    }

    let new_password = form.new_password.trim().to_string();
    if new_password.is_empty() || new_password.chars().count() > 64 {
        return Err(AppError::bad_request("新口令长度必须在 1～64 字符之间"));
    }

    let mut config = (*state.config).clone();
    config.admin_password = new_password.clone();
    config
        .save(&state.paths.root)
        .map_err(|error| AppError::internal(format!("保存管理口令失败: {error}")))?;
    *state
        .admin_password
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = new_password;

    Ok(Redirect::to("/admin/system").into_response())
}
