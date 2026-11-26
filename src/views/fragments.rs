use askama::Template;
use serde::Deserialize;

use crate::model::SubmissionStatus;
use crate::store::SubmissionRow;
use crate::timefmt::format_local;

#[derive(Template)]
#[template(path = "fragments/mine_table.html")]
pub struct MineTableTemplate {
    pub visible: bool,
    pub rows: Vec<SubmissionView>,
}

#[derive(Template)]
#[template(path = "fragments/rank_table.html")]
pub struct RankTableTemplate {
    pub problems: Vec<RankProblemView>,
    pub rows: Vec<RankRowView>,
}

#[derive(Template)]
#[template(path = "fragments/hidden_note.html")]
pub struct HiddenNoteTemplate;

pub struct RankProblemView {
    pub key: String,
}

pub struct RankRowView {
    pub rank: usize,
    pub name: String,
    pub school: String,
    pub grade: String,
    pub total: i32,
    pub per_problem: Vec<i32>,
    pub last_score_text: String,
}

#[derive(Debug, Clone)]
pub struct SubmissionView {
    /// 提交编号，前端用它作为折叠卡片的稳定键，轮询刷新后仍能保持展开状态。
    pub id: i64,
    pub created_at_text: String,
    pub problem_key: String,
    pub status_label: String,
    pub verdict_label: String,
    pub score: i32,
    pub compile_output: String,
    pub has_compile_output: bool,
    pub error: String,
    pub groups: Vec<SubtaskGroupView>,
}

#[derive(Debug, Clone)]
pub struct SubtaskGroupView {
    pub name: String,
    pub score: i32,
    pub max_score: i32,
    pub cases: Vec<CaseView>,
}

#[derive(Debug, Clone)]
pub struct CaseView {
    pub name: String,
    pub verdict: String,
    pub time_ms: u64,
    pub memory_kb: u64,
    pub score: i32,
    pub max_score: i32,
}

#[derive(Deserialize)]
struct CaseJson {
    name: String,
    subtask: String,
    verdict: String,
    time_ms: u64,
    memory_kb: u64,
    score: i32,
    max_score: i32,
}

pub fn submission_view(row: &SubmissionRow, visible: bool) -> SubmissionView {
    let status_label = status_label(row.status).to_string();
    let mut view = SubmissionView {
        id: row.id,
        created_at_text: format_local(row.created_at),
        problem_key: row.problem_key.clone(),
        status_label,
        verdict_label: String::new(),
        score: 0,
        compile_output: String::new(),
        has_compile_output: false,
        error: String::new(),
        groups: Vec::new(),
    };
    if !visible {
        return view;
    }

    view.verdict_label = row
        .verdict
        .map(|verdict| verdict.as_str().to_string())
        .unwrap_or_else(|| "-".to_string());
    view.score = row.score;
    view.compile_output = row.compile_output.clone().unwrap_or_default();
    view.has_compile_output = !view.compile_output.is_empty();
    view.error = row.error.clone().unwrap_or_default();

    if let Some(json) = &row.cases_json {
        if let Ok(cases) = serde_json::from_str::<Vec<CaseJson>>(json) {
            let mut groups: Vec<SubtaskGroupView> = Vec::new();
            for case in cases {
                let group = match groups.iter_mut().find(|group| group.name == case.subtask) {
                    Some(group) => group,
                    None => {
                        groups.push(SubtaskGroupView {
                            name: case.subtask.clone(),
                            score: 0,
                            max_score: 0,
                            cases: Vec::new(),
                        });
                        groups.last_mut().expect("just pushed")
                    }
                };
                group.score += case.score;
                group.max_score += case.max_score;
                group.cases.push(CaseView {
                    name: case.name,
                    verdict: case.verdict,
                    time_ms: case.time_ms,
                    memory_kb: case.memory_kb,
                    score: case.score,
                    max_score: case.max_score,
                });
            }
            view.groups = groups;
        }
    }

    view
}

pub fn status_label(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::Queued => "已提交",
        SubmissionStatus::Compiling | SubmissionStatus::Running => "评测中",
        SubmissionStatus::Done => "已评测",
        SubmissionStatus::Failed => "评测异常",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Verdict;
    use std::path::PathBuf;

    fn row() -> SubmissionRow {
        SubmissionRow {
            id: 1,
            contest_id: "demo".to_string(),
            student_id: "2026001".to_string(),
            problem_key: "A".to_string(),
            language: "cpp17".to_string(),
            source_path: PathBuf::from("C:/data/1.cpp"),
            status: SubmissionStatus::Done,
            verdict: Some(Verdict::Ac),
            score: 987654,
            cases_json: Some(
                r#"[{"name":"1","subtask":"小数据","verdict":"AC","time_ms":12,"memory_kb":1024,"score":50,"max_score":50}]"#
                    .to_string(),
            ),
            compile_output: Some("CE_SECRET_987654".to_string()),
            error: None,
            created_at: 0,
            judged_at: Some(1),
        }
    }

    #[test]
    fn hidden_view_drops_verdict_score_and_output() {
        let view = submission_view(&row(), false);
        assert_eq!(view.status_label, "已评测");
        assert!(view.verdict_label.is_empty());
        assert_eq!(view.score, 0);
        assert!(view.compile_output.is_empty());
        assert!(view.error.is_empty());
        assert!(view.groups.is_empty());
    }

    #[test]
    fn public_view_includes_verdict_and_groups() {
        let view = submission_view(&row(), true);
        assert_eq!(view.verdict_label, "AC");
        assert_eq!(view.score, 987654);
        assert_eq!(view.compile_output, "CE_SECRET_987654");
        assert_eq!(view.groups.len(), 1);
        assert_eq!(view.groups[0].name, "小数据");
        assert_eq!(view.groups[0].score, 50);
        assert_eq!(view.groups[0].cases.len(), 1);
    }

    #[test]
    fn status_labels_cover_all_states() {
        assert_eq!(status_label(SubmissionStatus::Queued), "已提交");
        assert_eq!(status_label(SubmissionStatus::Compiling), "评测中");
        assert_eq!(status_label(SubmissionStatus::Running), "评测中");
        assert_eq!(status_label(SubmissionStatus::Done), "已评测");
        assert_eq!(status_label(SubmissionStatus::Failed), "评测异常");
    }
}
