use askama::Template;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use super::access::{current_student, remaining_seconds};
use super::error::AppError;
use crate::app_state::AppState;
use crate::auth::session::StudentSession;
use crate::contest::lifecycle::phase;
use crate::ranking::compute_ranking;
use crate::services::registry::ContestRuntime;
use crate::timefmt::{format_local, now_ms};
use crate::views::fragments::{
    submission_view, HiddenNoteTemplate, MineTableTemplate, RankProblemView, RankRowView,
    RankTableTemplate, SubmissionView,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/c/{id}/status", get(status))
        .route("/c/{id}/_mine", get(mine_fragment))
        .route("/c/{id}/_rank", get(rank_fragment))
}

async fn status(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((runtime, _session)) = current_student(&state, &headers, &contest_id, true)? else {
        return Err(AppError::unauthorized_json("请先登录参赛"));
    };
    let contest_state = runtime
        .store
        .state()
        .map_err(|error| AppError::internal(format!("读取比赛状态失败: {error}")))?;
    let now = now_ms();
    let remaining = remaining_seconds(&contest_state, now);
    Ok(Json(json!({
        "state": phase(&contest_state, now).as_str(),
        "remainingSeconds": remaining,
    }))
    .into_response())
}

async fn mine_fragment(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    let Some((runtime, session)) = current_student(&state, &headers, &contest_id, true)? else {
        return Err(AppError::unauthorized_json("请先登录参赛"));
    };
    let visible = leaderboard_visible(&runtime)?;
    let html = mine_html(&runtime, &session, visible)?;
    Ok(Html(html))
}

async fn rank_fragment(
    State(state): State<AppState>,
    Path(contest_id): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    let Some((runtime, _session)) = current_student(&state, &headers, &contest_id, true)? else {
        return Err(AppError::unauthorized_json("请先登录参赛"));
    };
    let visible = leaderboard_visible(&runtime)?;
    let html = rank_html(&runtime, visible)?;
    Ok(Html(html))
}

pub(crate) fn leaderboard_visible(runtime: &ContestRuntime) -> Result<bool, AppError> {
    runtime
        .store
        .state()
        .map(|state| state.leaderboard_visible)
        .map_err(|error| AppError::internal(format!("读取榜单开关失败: {error}")))
}

pub(crate) fn mine_html(
    runtime: &ContestRuntime,
    session: &StudentSession,
    visible: bool,
) -> Result<String, AppError> {
    let rows = runtime
        .store
        .submissions_for_student(&session.student_id)
        .map_err(|error| AppError::internal(format!("读取提交失败: {error}")))?;

    let rows = rows
        .iter()
        .map(|row| submission_view(row, visible))
        .collect::<Vec<SubmissionView>>();
    MineTableTemplate { visible, rows }
        .render()
        .map_err(|error| AppError::internal(format!("提交片段渲染失败: {error}")))
}

pub(crate) fn rank_html(runtime: &ContestRuntime, visible: bool) -> Result<String, AppError> {
    if !visible {
        return HiddenNoteTemplate
            .render()
            .map_err(|error| AppError::internal(format!("榜单提示渲染失败: {error}")));
    }
    let submissions = runtime
        .store
        .submissions_for_contest()
        .map_err(|error| AppError::internal(format!("读取榜单提交失败: {error}")))?;
    let ranked = compute_ranking(
        &runtime.package.roster,
        &runtime.package.problems,
        &submissions,
    );
    let problems = runtime
        .package
        .problems
        .iter()
        .map(|problem| RankProblemView {
            key: problem.key.clone(),
        })
        .collect();
    let rows = ranked
        .iter()
        .enumerate()
        .map(|(index, row)| RankRowView {
            rank: index + 1,
            name: row.student.name.clone(),
            school: row.student.school.clone(),
            grade: row.student.grade.clone(),
            total: row.total,
            per_problem: row.per_problem.clone(),
            last_score_text: row
                .last_score_at
                .map(format_local)
                .unwrap_or_else(|| "—".to_string()),
        })
        .collect();
    RankTableTemplate { problems, rows }
        .render()
        .map_err(|error| AppError::internal(format!("榜单渲染失败: {error}")))
}
