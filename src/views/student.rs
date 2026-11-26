use askama::Template;

use crate::model::Phase;

#[derive(Template)]
#[template(path = "student/contest_list.html")]
pub struct ContestListTemplate {
    pub contests: Vec<ContestListItem>,
}

pub struct ContestListItem {
    pub id: String,
    pub title: String,
    pub state_label: String,
    pub duration_minutes: u32,
    pub problem_count: usize,
    pub visibility_label: String,
}

#[derive(Template)]
#[template(path = "student/join.html")]
pub struct JoinTemplate {
    pub contest_id: String,
    pub contest_title: String,
    pub csrf: String,
    /// 出错时回填账号，口令不回填。
    pub student_id: String,
    pub error: String,
    /// 会话被其他设备替换等原因导致的提示。
    pub notice: String,
}

#[derive(Template)]
#[template(path = "student/ended.html")]
pub struct EndedTemplate {
    pub contest_id: String,
    pub contest_title: String,
}

#[derive(Template)]
#[template(path = "student/home.html")]
pub struct HomeTemplate {
    pub contest_id: String,
    pub contest_title: String,
    pub student_name: String,
    pub student_school: String,
    pub student_grade: String,
    pub csrf: String,
    pub state_label: String,
    pub remaining_seconds: u64,
    pub remaining_text: String,
    pub problems: Vec<ProblemListItem>,
    /// 未开赛时只显示等待页，不下发题目列表。
    pub waiting: bool,
    /// 「我的信息」里展示的时间点，缺省为「—」。
    pub joined_text: String,
    pub last_seen_text: String,
    pub started_text: String,
    pub ends_text: String,
    pub mine_url: String,
    pub mine_html: String,
    pub rank_url: String,
    pub rank_html: String,
}

pub struct ProblemListItem {
    pub key: String,
    pub title: String,
    pub full_score: i32,
}

#[derive(Template)]
#[template(path = "student/all.html")]
pub struct AllProblemsTemplate {
    pub contest_id: String,
    pub contest_title: String,
    pub csrf: String,
    pub problems: Vec<StatementView>,
}

pub struct StatementView {
    pub key: String,
    pub title: String,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
    pub full_score: i32,
    pub statement_html: String,
}

#[derive(Template)]
#[template(path = "student/problem.html")]
pub struct ProblemTemplate {
    pub contest_id: String,
    pub contest_title: String,
    pub student_id: String,
    pub csrf: String,
    pub problem_key: String,
    pub problem_title: String,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
    pub full_score: i32,
    pub statement_html: String,
    pub state_label: String,
    pub submit_disabled: bool,
    pub submit_hint: String,
    /// 刚从提交接口跳转回来时提示一次「已提交」。
    pub submitted: bool,
    pub mine_url: String,
}

pub fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::NotStarted => "未开始",
        Phase::Running => "进行中",
        Phase::Ended => "已结束",
    }
}

pub fn format_remaining(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
