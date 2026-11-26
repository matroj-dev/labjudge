use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::access::{
    contest_or_error, current_student, redirect_join, remaining_seconds, render_html, student_name,
    with_cookie,
};
use super::error::AppError;
use crate::app_state::AppState;
use crate::auth::session::{
    build_cookie, clear_cookie, read_cookie, student_cookie_name, CSRF_COOKIE, SESSION_SECONDS,
};
use crate::contest::lifecycle::phase;
use crate::contest::validate::is_valid_contest_id;
use crate::markdown::{render_statement, StatementCtx};
use crate::model::Phase;
use crate::services::registry::ContestRuntime;
use crate::store::NewSubmission;
use crate::timefmt::{format_local, now_ms};
use crate::views::student::{
    format_remaining, phase_label, AllProblemsTemplate, ContestListItem, ContestListTemplate,
    HomeTemplate, JoinTemplate, ProblemListItem, ProblemTemplate, StatementView,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(contest_list))
        .route("/c/{id}/join", get(join_get).post(join_post))
        .route("/c/{id}", get(home))
        .route("/c/{id}/all", get(all_problems))
        .route("/c/{id}/p/{key}", get(problem_page))
        .route("/c/{id}/submissions", post(submit))
        .route("/logout", post(logout))
}

async fn contest_list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let mut contests = Vec::new();
    for runtime in state.registry.list() {
        let contest_state = runtime
            .store
            .state()
            .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
        let current_phase = phase(&contest_state, now_ms());
        contests.push(ContestListItem {
            id: runtime.package.id.clone(),
            title: runtime.package.title.clone(),
            state_label: phase_label(current_phase).to_string(),
            duration_minutes: runtime.package.duration_minutes,
            problem_count: runtime.package.problems.len(),
            visibility_label: if contest_state.leaderboard_visible {
                "榜单公开".to_string()
            } else {
                "榜单未公开".to_string()
            },
        });
    }
    render_html(ContestListTemplate { contests })
}

async fn join_get(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let runtime = contest_or_error(&state, &contest_id, false)?;
    if current_student(&state, &headers, &contest_id, false)?.is_some() {
        return Ok(Redirect::to(&format!("/c/{contest_id}")).into_response());
    }

    let (cookie_value, form_token) = state.csrf.issue_anon();
    let html = JoinTemplate {
        contest_id,
        contest_title: runtime.package.title.clone(),
        csrf: form_token,
        student_id: String::new(),
        error: String::new(),
    };
    let response = render_html(html)?.into_response();
    Ok(with_cookie(
        response,
        build_cookie(CSRF_COOKIE, &cookie_value, SESSION_SECONDS),
    ))
}

#[derive(Deserialize)]
struct JoinForm {
    student_id: String,
    password: String,
    csrf: String,
}

async fn join_post(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<JoinForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    let Some(cookie_value) = read_cookie(&headers, CSRF_COOKIE) else {
        return Err(AppError::forbidden("缺少表单校验信息，请刷新页面重试"));
    };
    if !state.csrf.verify_anon(&cookie_value, &form.csrf) {
        return Err(AppError::forbidden("表单校验失败，请刷新页面重试"));
    }

    let runtime = contest_or_error(&state, &contest_id, false)?;
    let student_id = form.student_id.trim();
    let Some(student) = runtime
        .package
        .roster
        .authenticate(student_id, form.password.trim())
    else {
        return join_error_page(
            &runtime,
            &form,
            StatusCode::BAD_REQUEST,
            "账号或密码错误，请联系老师",
        );
    };

    let now = now_ms();
    runtime
        .store
        .upsert_participant(&student, now)
        .map_err(|error| AppError::internal(format!("记录参赛信息失败: {error}")))?;
    let token = state
        .session
        .sign_student(&contest_id, &student.student_id, now);

    let mut response = Redirect::to(&format!("/c/{contest_id}")).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        build_cookie(&student_cookie_name(&contest_id), &token, SESSION_SECONDS),
    );
    Ok(response)
}

fn join_error_page(
    runtime: &ContestRuntime,
    form: &JoinForm,
    status: StatusCode,
    message: &str,
) -> Result<Response, AppError> {
    let html = JoinTemplate {
        contest_id: runtime.package.id.clone(),
        contest_title: runtime.package.title.clone(),
        csrf: form.csrf.clone(),
        student_id: form.student_id.clone(),
        error: message.to_string(),
    };
    let html = html
        .render()
        .map_err(|error| AppError::internal(format!("参赛页渲染失败: {error}")))?;
    Ok((status, Html(html)).into_response())
}

async fn home(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((runtime, session)) = current_student(&state, &headers, &contest_id, false)? else {
        return Ok(redirect_join(&contest_id));
    };
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let now = now_ms();
    let current_phase = phase(&contest_state, now);
    let remaining = remaining_seconds(&contest_state, now);
    let visible = contest_state.leaderboard_visible;
    // 未开赛不下发题目清单：学生只能看到等待页，也不渲染提交与榜单标签。
    let waiting = current_phase == Phase::NotStarted;
    let (problems, mine_html, rank_html) = if waiting {
        (Vec::new(), String::new(), String::new())
    } else {
        let problems = runtime
            .package
            .problems
            .iter()
            .map(|problem| ProblemListItem {
                key: problem.key.clone(),
                title: problem.title.clone(),
                full_score: problem.full_score,
            })
            .collect();
        (
            problems,
            crate::routes::fragments::mine_html(&runtime, &session, visible)?,
            crate::routes::fragments::rank_html(&runtime, visible)?,
        )
    };

    let html = HomeTemplate {
        contest_id: contest_id.clone(),
        contest_title: runtime.package.title.clone(),
        student_name: student_name(&runtime, &session.student_id),
        student_school: roster_field(&runtime, &session.student_id, |student| &student.school),
        student_grade: roster_field(&runtime, &session.student_id, |student| &student.grade),
        csrf: state.csrf.for_session(&session.token),
        state_label: phase_label(current_phase).to_string(),
        remaining_seconds: remaining,
        remaining_text: format_remaining(remaining),
        problems,
        waiting,
        joined_text: participant_time_text(&runtime, &session.student_id, |row| row.joined_at)?,
        last_seen_text: participant_time_text(&runtime, &session.student_id, |row| {
            row.last_seen_at
        })?,
        started_text: contest_state
            .started_at
            .map(format_local)
            .unwrap_or_else(|| "—".to_string()),
        ends_text: contest_state
            .ends_at
            .map(format_local)
            .unwrap_or_else(|| "—".to_string()),
        mine_url: format!("/c/{contest_id}/_mine"),
        mine_html,
        rank_url: format!("/c/{contest_id}/_rank"),
        rank_html,
    };
    Ok(render_html(html)?.into_response())
}

/// 名单里的学校 / 年级，找不到时留空。
fn roster_field(
    runtime: &ContestRuntime,
    student_id: &str,
    pick: impl Fn(&crate::contest::roster::Student) -> &String,
) -> String {
    runtime
        .package
        .roster
        .students()
        .iter()
        .find(|student| student.student_id == student_id)
        .map(|student| pick(student).clone())
        .unwrap_or_default()
}

fn participant_time_text(
    runtime: &ContestRuntime,
    student_id: &str,
    pick: impl Fn(&crate::store::ParticipantRow) -> i64,
) -> Result<String, AppError> {
    let row = runtime
        .store
        .participant(student_id)
        .map_err(|error| AppError::internal(format!("读取参赛记录失败: {error}")))?;
    Ok(row
        .filter(|row| pick(row) > 0)
        .map(|row| format_local(pick(&row)))
        .unwrap_or_else(|| "—".to_string()))
}

async fn all_problems(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((runtime, session)) = current_student(&state, &headers, &contest_id, false)? else {
        return Ok(redirect_join(&contest_id));
    };
    if !contest_started(&runtime)? {
        return Ok(Redirect::to(&format!("/c/{contest_id}")).into_response());
    }
    let problems = statement_views(&contest_id, &runtime)?;
    let html = AllProblemsTemplate {
        contest_id,
        contest_title: runtime.package.title.clone(),
        csrf: state.csrf.for_session(&session.token),
        problems,
    };
    Ok(render_html(html)?.into_response())
}

async fn problem_page(
    State(state): State<AppState>,
    Path((contest_id, problem_key)): Path<(String, String)>,
    Query(query): Query<ProblemQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((runtime, session)) = current_student(&state, &headers, &contest_id, false)? else {
        return Ok(redirect_join(&contest_id));
    };
    let Some(problem) = runtime.package.find_problem(&problem_key) else {
        return Err(AppError::not_found("题目不存在"));
    };
    if !contest_started(&runtime)? {
        return Ok(Redirect::to(&format!("/c/{contest_id}")).into_response());
    }
    let statement = read_statement(&contest_id, problem)?;
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let current_phase = phase(&contest_state, now_ms());
    let submit_disabled = current_phase != Phase::Running || state.queue.is_none();
    let submit_hint = if current_phase != Phase::Running {
        "比赛未开始或已结束。".to_string()
    } else if state.queue.is_none() {
        "评测服务未就绪。".to_string()
    } else {
        String::new()
    };

    let html = ProblemTemplate {
        contest_id: contest_id.clone(),
        contest_title: runtime.package.title.clone(),
        student_id: session.student_id.clone(),
        csrf: state.csrf.for_session(&session.token),
        problem_key: problem_key.clone(),
        problem_title: problem.title.clone(),
        time_limit_ms: problem.time_limit_ms,
        memory_limit_mb: problem.memory_limit_mb,
        full_score: problem.full_score,
        statement_html: statement,
        state_label: phase_label(current_phase).to_string(),
        submit_disabled,
        submit_hint,
        submitted: query.submitted.as_deref() == Some("1"),
        mine_url: format!("/c/{contest_id}#me"),
    };
    Ok(render_html(html)?.into_response())
}

#[derive(Deserialize, Default)]
struct ProblemQuery {
    submitted: Option<String>,
}

#[derive(Deserialize)]
struct SubmitForm {
    problem_key: String,
    source: String,
    csrf: String,
}

async fn submit(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
    form: Result<Form<SubmitForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    let Some((runtime, session)) = current_student(&state, &headers, &contest_id, false)? else {
        return Ok(redirect_join(&contest_id));
    };
    if !state.csrf.verify_session(&session.token, &form.csrf) {
        return Err(AppError::forbidden("表单校验失败，请刷新页面重试"));
    }

    if runtime.package.find_problem(&form.problem_key).is_none() {
        return Err(AppError::not_found("题目不存在"));
    }
    let now = now_ms();
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    if phase(&contest_state, now) != Phase::Running {
        return Err(AppError::bad_request("比赛未开始或已结束"));
    }

    let source_bytes = form.source.as_bytes();
    if source_bytes.len() > state.config.max_source_bytes {
        return Err(AppError::bad_request(format!(
            "源码超过 {} 字节上限",
            state.config.max_source_bytes
        )));
    }

    let cooldown_ms = (state.config.submit_cooldown_seconds * 1000) as i64;
    if cooldown_ms > 0 {
        if let Some(last) = runtime
            .store
            .latest_created_at(&session.student_id, &form.problem_key)
            .map_err(|error| AppError::internal(format!("读取提交冷却失败: {error}")))?
        {
            let elapsed = now.saturating_sub(last);
            if elapsed < cooldown_ms {
                let wait_seconds = ((cooldown_ms - elapsed + 999) / 1000) as u64;
                return Err(AppError::page(
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("提交过快，请 {wait_seconds} 秒后再试"),
                ));
            }
        }
    }

    let Some(queue) = &state.queue else {
        return Err(AppError::page(
            StatusCode::SERVICE_UNAVAILABLE,
            "评测服务未就绪，请稍后重试",
        ));
    };

    let id = runtime
        .store
        .insert_submission(&NewSubmission {
            student_id: session.student_id.clone(),
            problem_key: form.problem_key.clone(),
            language: "cpp17".to_string(),
            source_path: PathBuf::new(),
            created_at: now,
        })
        .map_err(|error| AppError::internal(format!("记录提交失败: {error}")))?;

    let submissions_dir = runtime.store.data_dir().join("submissions");
    let final_path = submissions_dir.join(format!("{id}.cpp"));
    let temp_path = submissions_dir.join(format!(".tmp-{id}.cpp"));
    let write_result = (|| -> std::io::Result<()> {
        fs::create_dir_all(&submissions_dir)?;
        fs::write(&temp_path, source_bytes)?;
        fs::rename(&temp_path, &final_path)?;
        Ok(())
    })();

    if let Err(error) = write_result {
        fs::remove_file(&temp_path).ok();
        fs::remove_file(&final_path).ok();
        runtime.store.delete_submission(id).ok();
        return Err(AppError::internal(format!("保存源码失败: {error}")));
    }
    if let Err(error) = runtime.store.update_source_path(id, &final_path) {
        fs::remove_file(&final_path).ok();
        runtime.store.delete_submission(id).ok();
        return Err(AppError::internal(format!("更新提交路径失败: {error}")));
    }

    if !queue.enqueue(contest_id.clone(), id) {
        runtime
            .store
            .mark_failed(id, "判题队列已关闭", now_ms())
            .ok();
        return Err(AppError::page(
            StatusCode::SERVICE_UNAVAILABLE,
            "评测队列不可用，请联系老师",
        ));
    }

    Ok(Redirect::to(&format!(
        "/c/{contest_id}/p/{}?submitted=1",
        form.problem_key
    ))
    .into_response())
}

#[derive(Deserialize)]
struct LogoutForm {
    contest_id: String,
    csrf: String,
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<LogoutForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Form(form) = form.map_err(|_| AppError::bad_request("表单格式错误"))?;
    if !is_valid_contest_id(&form.contest_id) {
        return Err(AppError::bad_request("比赛编号非法"));
    }

    if let Some(token) = read_cookie(&headers, &student_cookie_name(&form.contest_id)) {
        if let Ok(session) = state
            .session
            .verify_student(&token, &form.contest_id, now_ms())
        {
            if !state.csrf.verify_session(&session.token, &form.csrf) {
                return Err(AppError::forbidden("表单校验失败"));
            }
        }
    }

    let response = Redirect::to("/").into_response();
    Ok(with_cookie(
        response,
        clear_cookie(&student_cookie_name(&form.contest_id)),
    ))
}

/// 未开赛前，题面与题目列表一律不下发。
fn contest_started(runtime: &ContestRuntime) -> Result<bool, AppError> {
    runtime
        .store
        .state()
        .map(|state| phase(&state, now_ms()) != Phase::NotStarted)
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))
}

fn statement_views(
    contest_id: &str,
    runtime: &Arc<ContestRuntime>,
) -> Result<Vec<StatementView>, AppError> {
    runtime
        .package
        .problems
        .iter()
        .map(|problem| {
            Ok(StatementView {
                key: problem.key.clone(),
                title: problem.title.clone(),
                time_limit_ms: problem.time_limit_ms,
                memory_limit_mb: problem.memory_limit_mb,
                full_score: problem.full_score,
                statement_html: read_statement(contest_id, problem)?,
            })
        })
        .collect()
}

fn read_statement(
    contest_id: &str,
    problem: &crate::contest::model::ValidatedProblem,
) -> Result<String, AppError> {
    let markdown = fs::read_to_string(&problem.statement_path).map_err(|error| {
        AppError::internal(format!(
            "读取题面失败（{}）：{error}",
            problem.statement_path.display()
        ))
    })?;
    Ok(render_statement(
        crate::markdown::strip_leading_title(&markdown),
        &StatementCtx {
            contest_id,
            problem_key: &problem.key,
        },
    ))
}
