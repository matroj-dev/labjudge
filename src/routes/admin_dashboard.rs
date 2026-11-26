use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use super::access::{
    admin_session, contest_or_error, redirect_admin_login, render_html, require_admin_json,
    student_name,
};
use super::error::AppError;
use crate::app_state::AppState;
use crate::contest::lifecycle::phase;
use crate::model::Phase;
use crate::services::registry::ContestRuntime;
use crate::timefmt::{format_local, now_ms};
use crate::url::segment;
use crate::views::admin::{
    AdminContestItem, AdminContestTemplate, AdminDashboardTemplate, AdminInvalidItem,
    AdminMonitorTemplate, AdminProblemItem, AdminRecentItem, AdminRecentTemplate, AdminRosterItem,
};
use crate::views::fragments::status_label;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/c/{id}", get(contest_detail))
        .route("/admin/c/{id}/_monitor", get(monitor_fragment))
        .route("/admin/c/{id}/_recent", get(recent_fragment))
        .route("/admin/c/{id}/_rank", get(rank_fragment))
}

async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let csrf = state.csrf.for_session(&admin.token);
    let mut contests = Vec::new();
    for runtime in state.registry.list() {
        let contest_state = runtime
            .store
            .state()
            .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
        contests.push(AdminContestItem {
            id: runtime.package.id.clone(),
            title: runtime.package.title.clone(),
            state_label: crate::views::student::phase_label(phase(&contest_state, now_ms()))
                .to_string(),
            duration_minutes: runtime.package.duration_minutes,
            problem_count: runtime.package.problems.len(),
            visibility_label: if contest_state.leaderboard_visible {
                "榜单公开".to_string()
            } else {
                "榜单隐藏".to_string()
            },
        });
    }
    let invalid = state
        .registry
        .invalid()
        .into_iter()
        .map(|item| AdminInvalidItem {
            id: item.id,
            path: item.path.display().to_string(),
            reason: item.reason,
        })
        .collect();

    Ok(render_html(AdminDashboardTemplate {
        contests,
        invalid,
        csrf,
    })?
    .into_response())
}

async fn contest_detail(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(admin) = admin_session(&state, &headers) else {
        return Ok(redirect_admin_login());
    };
    let runtime = contest_or_error(&state, &contest_id, false)?;
    let csrf = state.csrf.for_session(&admin.token);
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let current_phase = phase(&contest_state, now_ms());

    let problems: Vec<AdminProblemItem> = runtime
        .package
        .problems
        .iter()
        .map(|problem| AdminProblemItem {
            key: problem.key.clone(),
            title: problem.title.clone(),
            full_score: problem.full_score,
        })
        .collect();

    // 「已进入」用参赛记录表判断：学生登录进入比赛才会留下记录，
    // 这比「最近 5 分钟有请求」更能说明谁真的到场比赛了。
    let mut activity: HashMap<String, (i64, usize)> = HashMap::new();
    for participant in runtime
        .store
        .participants()
        .map_err(|error| AppError::internal(format!("读取参赛记录失败: {error}")))?
    {
        activity.insert(participant.student_id, (participant.last_seen_at, 0));
    }
    for row in runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?
    {
        activity
            .entry(row.student_id)
            .or_insert((row.created_at, 0))
            .1 += 1;
    }
    let joined_count = activity.len();

    let mut roster: Vec<AdminRosterItem> = runtime
        .package
        .roster
        .students()
        .iter()
        .map(|student| {
            let seen = activity.get(&student.student_id);
            AdminRosterItem {
                name: student.name.clone(),
                school: student.school.clone(),
                grade: student.grade.clone(),
                joined: seen.is_some(),
                last_seen_text: seen
                    .map(|(last_seen_at, _)| format_local(*last_seen_at))
                    .unwrap_or_default(),
                submission_count: seen.map(|(_, count)| *count).unwrap_or(0),
            }
        })
        .collect();
    // 没进入的排在最前，方便老师去催人。
    roster.sort_by(|left, right| {
        left.joined
            .cmp(&right.joined)
            .then_with(|| left.name.cmp(&right.name))
    });
    let roster_count = roster.len();

    let monitor_html = render_judge_status(&state, &runtime)?;
    let contest = segment(&contest_id);
    let monitor_url = format!("/admin/c/{contest}/_monitor");
    let recent_html = render_recent(&runtime)?;
    let recent_url = format!("/admin/c/{contest}/_recent");
    // 教师端始终能看到完整榜单，不受「榜单公开」开关影响。
    let rank_html = super::fragments::rank_html(&runtime, true)?;
    let rank_url = format!("/admin/c/{contest}/_rank");

    Ok(render_html(AdminContestTemplate {
        contest_id,
        title: runtime.package.title.clone(),
        state_label: crate::views::student::phase_label(current_phase).to_string(),
        leaderboard_visible: contest_state.leaderboard_visible,
        duration_minutes: runtime.package.duration_minutes,
        started_text: contest_state
            .started_at
            .map(format_local)
            .unwrap_or_else(|| "—".to_string()),
        ends_text: contest_state
            .ends_at
            .map(format_local)
            .unwrap_or_else(|| "—".to_string()),
        problem_count: problems.len(),
        roster_count,
        joined_count,
        problems,
        roster,
        monitor_url,
        monitor_html,
        recent_url,
        recent_html,
        rank_url,
        rank_html,
        csrf,
        is_not_started: current_phase == Phase::NotStarted,
        is_running: current_phase == Phase::Running,
        is_ended: current_phase == Phase::Ended,
    })?
    .into_response())
}

async fn rank_fragment(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    require_admin_json(&state, &headers)?;
    let runtime = contest_or_error(&state, &contest_id, true)?;
    Ok(Html(super::fragments::rank_html(&runtime, true)?))
}

async fn monitor_fragment(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    require_admin_json(&state, &headers)?;
    let runtime = contest_or_error(&state, &contest_id, true)?;
    Ok(Html(render_judge_status(&state, &runtime)?))
}

async fn recent_fragment(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    require_admin_json(&state, &headers)?;
    let runtime = contest_or_error(&state, &contest_id, true)?;
    Ok(Html(render_recent(&runtime)?))
}

fn render_judge_status(state: &AppState, runtime: &ContestRuntime) -> Result<String, AppError> {
    let counts = runtime
        .store
        .status_counts()
        .map_err(|error| AppError::internal(format!("读取提交统计失败: {error}")))?;
    let active = runtime
        .store
        .active_count()
        .map_err(|error| AppError::internal(format!("读取活动任务失败: {error}")))?;
    let queue_depth = state.queue.as_ref().map(|queue| queue.depth()).unwrap_or(0);
    let judge_alive = state
        .supervisor
        .as_ref()
        .map(|supervisor| supervisor.is_alive())
        .unwrap_or(false);

    AdminMonitorTemplate {
        judge_alive,
        queued: counts.queued,
        compiling: counts.compiling,
        running: counts.running,
        done: counts.done,
        failed: counts.failed,
        active,
        queue_depth,
        concurrency: state.config.judge_concurrency,
    }
    .render()
    .map_err(|error| AppError::internal(format!("监控渲染失败: {error}")))
}

fn render_recent(runtime: &ContestRuntime) -> Result<String, AppError> {
    let all = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取提交流水失败: {error}")))?;
    let rows = all
        .iter()
        .rev()
        .take(20)
        .map(|row| AdminRecentItem {
            sid: format!("{}~{}", runtime.package.id, row.id),
            created_text: format_local(row.created_at),
            student_name: student_name(runtime, &row.student_id),
            problem_key: row.problem_key.clone(),
            status_label: status_label(row.status).to_string(),
            verdict_label: row
                .verdict
                .map(|verdict| verdict.as_str().to_string())
                .unwrap_or_else(|| "-".to_string()),
            score: row.score,
        })
        .collect::<Vec<_>>();

    AdminRecentTemplate { rows }
        .render()
        .map_err(|error| AppError::internal(format!("最近提交渲染失败: {error}")))
}
