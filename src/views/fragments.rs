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
    pub message: String,
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
    #[serde(default)]
    message: Option<String>,
}

pub fn submission_view(row: &SubmissionRow, visible: bool) -> SubmissionView {
    submission_view_with_compile(row, visible, true)
}

/// 管理端保留完整编译输出，便于排查；学生端只看 main.cpp 的诊断。
pub fn submission_view_raw(row: &SubmissionRow, visible: bool) -> SubmissionView {
    submission_view_with_compile(row, visible, false)
}

fn submission_view_with_compile(
    row: &SubmissionRow,
    visible: bool,
    filter_compile: bool,
) -> SubmissionView {
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
    view.compile_output = match &row.compile_output {
        Some(output) if filter_compile => sanitize_compile_output(output),
        Some(output) => output.clone(),
        None => String::new(),
    };
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
                    message: if filter_compile {
                        String::new()
                    } else {
                        case.message.unwrap_or_default()
                    },
                });
            }
            view.groups = groups;
        }
    }

    view
}

/// 只保留学生自己 main.cpp 的诊断行，避免把被包含文件的内容带回浏览器。
///
/// 链接期错误（`undefined reference`、`collect2/ld returned`）不带 `main.cpp:` 前缀，
/// 但正是学生最需要看的信息，所以单独放行。
fn sanitize_compile_output(raw: &str) -> String {
    let mut kept = Vec::new();
    let mut omitted = false;
    for line in raw.lines() {
        if line.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        if line.contains("In file included from") {
            omitted = true;
            continue;
        }
        if is_student_diagnostic(line) {
            kept.push(line);
        } else if !line.trim().is_empty() {
            omitted = true;
        }
    }
    if kept.is_empty() {
        return if raw.trim().is_empty() {
            String::new()
        } else {
            "编译错误（详细信息位于标准库或其他文件，已省略）".to_string()
        };
    }
    let mut text = kept.join("\n");
    if omitted {
        text.push_str("\n（已省略标准库/其他文件的诊断）");
    }
    text
}

fn is_student_diagnostic(line: &str) -> bool {
    const LINKER_HINTS: [&str; 5] = [
        "undefined reference to",
        "multiple definition of",
        "duplicate symbol",
        "ld returned",
        "cc1plus.exe:",
    ];
    line.contains("main.cpp:") || LINKER_HINTS.iter().any(|hint| line.contains(hint))
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
            compile_output: Some("main.cpp:1:1: error: CE_SECRET_987654".to_string()),
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
        assert_eq!(view.compile_output, "main.cpp:1:1: error: CE_SECRET_987654");
        assert_eq!(view.groups.len(), 1);
        assert_eq!(view.groups[0].name, "小数据");
        assert_eq!(view.groups[0].score, 50);
        assert_eq!(view.groups[0].cases.len(), 1);
    }

    #[test]
    fn checker_message_is_admin_only() {
        let mut row = row();
        row.verdict = Some(Verdict::Wa);
        row.cases_json = Some(
            r#"[{"name":"1","subtask":"小数据","verdict":"WA","time_ms":12,"memory_kb":1024,"score":0,"max_score":50,"message":"wrong answer 答案错误"}]"#
                .to_string(),
        );

        let admin = submission_view_raw(&row, true);
        assert_eq!(
            admin.groups[0].cases[0].message,
            "wrong answer 答案错误".to_string()
        );

        // 学生端能看到自己的 AC/WA，但不该看到 checker 的诊断。
        let student = submission_view(&row, true);
        assert_eq!(student.groups[0].cases[0].message, "");
    }

    #[test]
    fn student_compile_output_drops_included_files() {
        let mut row = row();
        row.compile_output = Some(
            "In file included from main.cpp:1:\n\
             ../../../../config.json:1:1: error: SECRET\n\
             main.cpp:3:5: error: expected ';'\n\
             \x20    int x = 1\n\
             \x20        ^\n"
                .to_string(),
        );
        let view = submission_view(&row, true);
        assert!(view.compile_output.contains("main.cpp:3:5"));
        assert!(!view.compile_output.contains("SECRET"));
        assert!(!view.compile_output.contains("int x = 1"));

        let raw = submission_view_raw(&row, true);
        assert!(raw.compile_output.contains("SECRET"));
    }

    #[test]
    fn student_compile_output_keeps_linker_errors() {
        let mut row = row();
        row.compile_output = Some(
            "C:\\mingw64\\bin\\ld.exe: main.cpp:(.text+0x1a): undefined reference to `foo()'\n\
             collect2.exe: error: ld returned 1 exit status\n"
                .to_string(),
        );
        let view = submission_view(&row, true);
        assert!(view.compile_output.contains("undefined reference to"));
        assert!(view.compile_output.contains("ld returned 1"));
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
