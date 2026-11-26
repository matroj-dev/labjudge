use askama::Template;

use crate::views::fragments::SubmissionView;

#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct AdminLoginTemplate {
    pub csrf: String,
    pub error: String,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct AdminDashboardTemplate {
    pub contests: Vec<AdminContestItem>,
    pub invalid: Vec<AdminInvalidItem>,
    pub csrf: String,
}

pub struct AdminContestItem {
    pub id: String,
    pub title: String,
    pub state_label: String,
    pub duration_minutes: u32,
    pub problem_count: usize,
    pub visibility_label: String,
}

pub struct AdminInvalidItem {
    pub id: String,
    pub path: String,
    pub reason: String,
}

#[derive(Template)]
#[template(path = "admin/contest.html")]
pub struct AdminContestTemplate {
    pub contest_id: String,
    pub title: String,
    pub state_label: String,
    pub leaderboard_visible: bool,
    pub duration_minutes: u32,
    pub started_text: String,
    pub ends_text: String,
    pub problems: Vec<AdminProblemItem>,
    pub roster: Vec<AdminRosterItem>,
    pub roster_count: usize,
    pub joined_count: usize,
    pub problem_count: usize,
    pub monitor_url: String,
    pub monitor_html: String,
    pub recent_url: String,
    pub recent_html: String,
    pub rank_url: String,
    pub rank_html: String,
    pub csrf: String,
    pub is_not_started: bool,
    pub is_running: bool,
    pub is_ended: bool,
}

pub struct AdminProblemItem {
    pub key: String,
    pub title: String,
    pub full_score: i32,
}

pub struct AdminRosterItem {
    pub name: String,
    pub school: String,
    pub grade: String,
    /// 是否真的登录进入过比赛（participants 表里有记录）。
    pub joined: bool,
    pub last_seen_text: String,
    pub submission_count: usize,
}

#[derive(Template)]
#[template(path = "admin/monitor.html")]
pub struct AdminMonitorTemplate {
    pub judge_alive: bool,
    pub queued: usize,
    pub compiling: usize,
    pub running: usize,
    pub done: usize,
    pub failed: usize,
    pub active: usize,
    pub queue_depth: usize,
    pub concurrency: usize,
}

#[derive(Template)]
#[template(path = "admin/recent.html")]
pub struct AdminRecentTemplate {
    pub rows: Vec<AdminRecentItem>,
}

pub struct AdminRecentItem {
    pub sid: String,
    pub created_text: String,
    pub student_name: String,
    pub problem_key: String,
    pub status_label: String,
    pub verdict_label: String,
    pub score: i32,
}

#[derive(Template)]
#[template(path = "admin/submissions.html")]
pub struct AdminSubmissionsTemplate {
    pub contest_id: String,
    pub rows: Vec<AdminSubmissionRow>,
    pub page: usize,
    pub has_next: bool,
    pub csrf: String,
}

pub struct AdminSubmissionRow {
    pub sid: String,
    pub created_text: String,
    pub student_name: String,
    pub problem_key: String,
    pub status_label: String,
    pub verdict_label: String,
    pub score: i32,
}

#[derive(Template)]
#[template(path = "admin/submission.html")]
pub struct AdminSubmissionTemplate {
    pub sid: String,
    pub source: String,
    pub submission: SubmissionView,
    pub csrf: String,
}

#[derive(Template)]
#[template(path = "admin/system.html")]
pub struct AdminSystemTemplate {
    pub compiler_path: String,
    pub compiler_version: String,
    pub judge_version: String,
    pub judge_alive: bool,
    pub port: u16,
    pub addresses: Vec<String>,
    pub disk_free_text: String,
    pub disk_total_text: String,
    pub csrf: String,
}
