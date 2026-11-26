use std::fs;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use axum::Router;
use serde::Deserialize;

use super::access::{admin_session, contest_or_error, redirect_admin_login};
use super::error::AppError;
use crate::app_state::AppState;
use crate::contest::lifecycle::{apply_end, apply_extend, apply_start, phase};
use crate::model::Phase;
use crate::services::registry::{load_contest_package, ContestPackage, ContestRuntime};
use crate::store::ContestStore;
use crate::timefmt::{format_local, now_ms};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/c/{id}/start", post(start))
        .route("/admin/c/{id}/end", post(end))
        .route("/admin/c/{id}/extend", post(extend))
        .route("/admin/c/{id}/leaderboard", post(leaderboard))
        .route("/admin/c/{id}/reset", post(reset))
        .route("/admin/c/{id}/reload", post(reload))
        .route("/admin/c/{id}/rejudge-all", post(rejudge_all))
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
    #[serde(default)]
    confirm: String,
}

#[derive(Deserialize)]
struct ExtendForm {
    csrf: String,
    minutes: u32,
}

fn verify_common(
    state: &AppState,
    admin: &crate::auth::session::AdminSession,
    csrf: &str,
) -> Result<(), AppError> {
    if !state.csrf.verify_session(&admin.token, csrf) {
        return Err(AppError::forbidden("表单校验失败"));
    }
    Ok(())
}

fn ensure_confirm(form: &CsrfForm) -> Result<(), AppError> {
    if form.confirm != "yes" {
        return Err(AppError::bad_request("该操作需要确认"));
    }
    Ok(())
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

fn log(runtime: &ContestRuntime, message: &str) {
    let line = format!("{} {message}", format_local(now_ms()));
    let _ = runtime.store.append_log(&line);
}

async fn start(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;

    if state.queue.is_none() {
        return Err(AppError::bad_request("g++ 或 go-judge 未就绪，不能开赛"));
    }
    let mut contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    if phase(&contest_state, now_ms()) != Phase::NotStarted {
        return Err(AppError::bad_request("比赛已经开始或结束"));
    }
    apply_start(
        &mut contest_state,
        now_ms(),
        runtime.package.duration_minutes,
    )
    .map_err(|error| AppError::bad_request(error.to_string()))?;
    runtime
        .store
        .set_state(&contest_state)
        .map_err(|error| AppError::internal(format!("保存比赛状态失败: {error}")))?;
    log(&runtime, "教师开赛");
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

async fn end(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let mut contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    if contest_state.started_at.is_none() {
        return Err(AppError::bad_request("比赛尚未开始"));
    }
    apply_end(&mut contest_state, now_ms())
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    runtime
        .store
        .set_state(&contest_state)
        .map_err(|error| AppError::internal(format!("保存比赛状态失败: {error}")))?;
    runtime
        .store
        .clear_sessions()
        .map_err(|error| AppError::internal(format!("清空学生会话失败: {error}")))?;
    log(&runtime, "教师提前结束");
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

async fn extend(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<ExtendForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let mut contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let was_ended = phase(&contest_state, now_ms()) == Phase::Ended;
    apply_extend(&mut contest_state, now_ms(), form.minutes)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    runtime
        .store
        .set_state(&contest_state)
        .map_err(|error| AppError::internal(format!("保存比赛状态失败: {error}")))?;
    if was_ended {
        runtime
            .store
            .clear_sessions()
            .map_err(|error| AppError::internal(format!("清空学生会话失败: {error}")))?;
    }
    log(&runtime, &format!("教师延长 {} 分钟", form.minutes));
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

async fn leaderboard(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let visible = !contest_state.leaderboard_visible;
    runtime
        .store
        .set_leaderboard_visible(visible)
        .map_err(|error| AppError::internal(format!("保存榜单开关失败: {error}")))?;
    log(
        &runtime,
        if visible {
            "教师公开榜单"
        } else {
            "教师隐藏榜单"
        },
    );
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

async fn reset(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    ensure_confirm(&form)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    ensure_no_active(&runtime)?;

    let package = runtime.package.clone();
    let data_dir = runtime.store.data_dir().to_path_buf();
    drop(runtime);
    let Some(runtime) = state.registry.remove_runtime(&contest_id) else {
        return Err(AppError::not_found("比赛不存在"));
    };
    for _ in 0..100 {
        if Arc::strong_count(&runtime) <= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if Arc::strong_count(&runtime) > 1 {
        state.registry.insert_runtime(runtime);
        return Err(AppError::bad_request(
            "该场仍有页面请求正在进行，请稍后重试",
        ));
    }
    drop(runtime);

    if let Err(error) = fs::remove_dir_all(&data_dir) {
        if let Ok(store) = ContestStore::open(&data_dir, &contest_id, package.leaderboard_visible) {
            state.registry.insert_runtime(Arc::new(ContestRuntime {
                package: package.clone(),
                store: Arc::new(store),
            }));
        }
        return Err(AppError::internal(format!("删除运行数据失败: {error}")));
    }
    let store = ContestStore::open(&data_dir, &contest_id, package.leaderboard_visible)
        .map_err(|error| AppError::internal(format!("重建运行数据库失败: {error}")))?;
    let new_runtime = Arc::new(ContestRuntime {
        package,
        store: Arc::new(store),
    });
    log(&new_runtime, "教师重置比赛");
    state.registry.insert_runtime(new_runtime);
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

/// 重新加载题包时也要重编 checker：加载阶段就是唯一的编译时机。
/// 失败时不替换现有 runtime，把错误原样回显到管理页。
async fn prepare_checkers(state: &AppState, package: &ContestPackage) -> Result<(), AppError> {
    if !package
        .problems
        .iter()
        .any(|problem| problem.checker.is_some())
    {
        return Ok(());
    }
    let compiler = state
        .compiler
        .as_ref()
        .ok_or_else(|| AppError::internal("编译器未就绪"))?;
    crate::judge::prepare_contest(
        package,
        compiler.as_ref(),
        &state.paths.testlib,
        &state.paths.checkers_dir(),
    )
    .await
    .map_err(|error| AppError::bad_request(format!("checker 编译失败: {error}")))?;
    Ok(())
}

async fn reload(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    if phase(&contest_state, now_ms()) != Phase::NotStarted {
        return Err(AppError::bad_request("只能在开赛前重新加载名单/题包"));
    }
    let package = load_contest_package(&runtime.package.dir, &contest_id)
        .map_err(|error| AppError::bad_request(format!("题包/名单校验失败: {error}")))?;
    prepare_checkers(&state, &package).await?;
    runtime
        .store
        .clear_participants()
        .map_err(|error| AppError::internal(format!("清空参赛记录失败: {error}")))?;
    runtime
        .store
        .clear_sessions()
        .map_err(|error| AppError::internal(format!("清空学生会话失败: {error}")))?;
    state.registry.insert_runtime(Arc::new(ContestRuntime {
        package: Arc::new(package),
        store: runtime.store.clone(),
    }));
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}

async fn rejudge_all(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    verify_common(&state, &admin, &form.csrf)?;
    ensure_confirm(&form)?;
    let runtime = contest_or_error(&state, &contest_id, false)?;
    ensure_no_active(&runtime)?;
    let Some(queue) = &state.queue else {
        return Err(AppError::bad_request("判题队列未就绪"));
    };
    let rows = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?;
    for row in &rows {
        runtime
            .store
            .clear_result(row.id)
            .map_err(|error| AppError::internal(format!("重置提交失败: {error}")))?;
        if !queue.enqueue(contest_id.clone(), row.id) {
            return Err(AppError::internal("判题队列已关闭"));
        }
    }
    log(&runtime, &format!("教师全部重测 {} 条", rows.len()));
    Ok(Redirect::to(&format!("/admin/c/{contest_id}")).into_response())
}
