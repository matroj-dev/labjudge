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
    pub problems: Vec<ProblemRefJson>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProblemRefJson {
    pub key: String,
    pub dir: String,
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
            path: path.into(),
            reason: reason.into(),
        }
    }
}
