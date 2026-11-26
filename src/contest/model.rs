use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContestJson {
    pub title: String,
    pub duration_minutes: u32,
    #[serde(default)]
    pub leaderboard_visible: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemJson {
    pub title: String,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
    #[serde(default)]
    pub full_score: Option<i32>,
    #[serde(default)]
    pub subtasks: Option<Vec<SubtaskJson>>,
    #[serde(default)]
    pub checker: Option<CheckerJson>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckerJson {
    pub enabled: bool,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checker {
    pub source: PathBuf,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubtaskJson {
    #[serde(default)]
    pub name: Option<String>,
    pub score: i32,
    #[serde(default, rename = "type")]
    pub kind: SubtaskKind,
    pub cases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtaskKind {
    #[default]
    Sum,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemRef {
    pub key: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    pub name: String,
    pub in_path: PathBuf,
    pub out_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProblemDraft {
    pub key: String,
    pub title: String,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
    pub full_score: Option<i32>,
    pub subtasks: Option<Vec<SubtaskJson>>,
    pub root: PathBuf,
    pub statement_path: PathBuf,
    pub test_cases: Vec<TestCase>,
    pub checker: Option<Checker>,
}

#[derive(Debug, Clone)]
pub struct ContestDraft {
    pub title: String,
    pub duration_minutes: u32,
    pub leaderboard_visible: bool,
    pub problems: Vec<ProblemDraft>,
}

#[derive(Debug, Clone)]
pub struct ValidatedSubtask {
    pub name: String,
    pub score: i32,
    pub kind: SubtaskKind,
    pub cases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatedProblem {
    pub key: String,
    pub title: String,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
    pub full_score: i32,
    pub root: PathBuf,
    pub statement_path: PathBuf,
    pub test_cases: Vec<TestCase>,
    pub subtasks: Vec<ValidatedSubtask>,
    pub checker: Option<Checker>,
}

#[derive(Debug, Error)]
#[error("{}: {}", path.display(), reason)]
pub struct PackageError {
    pub path: PathBuf,
    pub reason: String,
}

impl PackageError {
    pub fn new(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self {
            path: normalize_display_path(path.into()),
            reason: reason.into(),
        }
    }
}

fn normalize_display_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_error_hides_windows_verbatim_prefix() {
        let error = super::PackageError::new(r"\\?\C:/contests/demo/contest.json", "字段非法");
        assert_eq!(
            error.to_string(),
            r"C:/contests/demo/contest.json: 字段非法"
        );
    }
}
