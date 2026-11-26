use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection};
use thiserror::Error;

pub mod contest;
pub mod participants;
pub mod sessions;
pub mod submissions;

pub use participants::ParticipantRow;
pub use submissions::{
    InsertedSubmission, NewSubmission, StatusCounts, SubmissionResult, SubmissionRow,
};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("文件错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("数据库锁中毒")]
    Poisoned,
    #[error("数据库内容非法: {0}")]
    InvalidData(String),
}

pub struct ContestStore {
    pub(crate) conn: Mutex<Connection>,
    contest_id: String,
    data_dir: PathBuf,
}

impl ContestStore {
    pub fn open(
        data_dir: &Path,
        contest_id: &str,
        initial_leaderboard_visible: bool,
    ) -> Result<Self, StoreError> {
        fs::create_dir_all(data_dir)?;
        fs::create_dir_all(data_dir.join("submissions"))?;
        fs::create_dir_all(data_dir.join("tmp"))?;

        let conn = Connection::open(data_dir.join("contest.db"))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;

        let now = crate::timefmt::now_ms();
        conn.execute(
            "INSERT OR IGNORE INTO contest_state
             (contest_id, started_at, ends_at, leaderboard_visible, ended_early, updated_at)
             VALUES (?1, NULL, NULL, ?2, 0, ?3)",
            params![
                contest_id,
                if initial_leaderboard_visible {
                    1i64
                } else {
                    0i64
                },
                now
            ],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1')",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            contest_id: contest_id.to_string(),
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn contest_id(&self) -> &str {
        &self.contest_id
    }

    pub fn append_log(&self, line: &str) -> Result<(), StoreError> {
        let path = self.data_dir.join("judge.log");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn log(&self, message: &str) {
        let line = format!(
            "{} {message}",
            crate::timefmt::format_local(crate::timefmt::now_ms())
        );
        if let Err(error) = self.append_log(&line) {
            tracing::error!(contest = %self.contest_id, %error, "写 judge.log 失败");
        }
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS contest_state (
    contest_id TEXT PRIMARY KEY,
    started_at INTEGER,
    ends_at INTEGER,
    leaderboard_visible INTEGER NOT NULL DEFAULT 0,
    ended_early INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS participants (
    contest_id TEXT NOT NULL,
    student_id TEXT NOT NULL,
    name TEXT NOT NULL,
    school TEXT NOT NULL DEFAULT '',
    grade TEXT NOT NULL DEFAULT '',
    last_seen_at INTEGER NOT NULL,
    joined_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (contest_id, student_id)
);

CREATE TABLE IF NOT EXISTS student_sessions (
    contest_id TEXT NOT NULL,
    student_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (contest_id, student_id)
);

CREATE TABLE IF NOT EXISTS submissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contest_id TEXT NOT NULL,
    student_id TEXT NOT NULL,
    problem_key TEXT NOT NULL,
    language TEXT NOT NULL,
    source_path TEXT NOT NULL,
    status TEXT NOT NULL,
    verdict TEXT,
    score INTEGER NOT NULL DEFAULT 0,
    cases_json TEXT,
    compile_output TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    judged_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_submissions_contest_created
    ON submissions (contest_id, created_at);
CREATE INDEX IF NOT EXISTS idx_submissions_student_problem
    ON submissions (contest_id, student_id, problem_key);

CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_and_seeds_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        assert_eq!(store.contest_id(), "c");
        assert!(dir.path().join("contest.db").is_file());
        assert!(dir.path().join("submissions").is_dir());
        assert!(dir.path().join("tmp").is_dir());

        let state = store.state().unwrap();
        assert!(!state.leaderboard_visible);
        assert_eq!(
            store.meta_value("schema_version").unwrap().as_deref(),
            Some("1")
        );
        drop(store);

        let store = ContestStore::open(dir.path(), "c", true).unwrap();
        assert!(!store.state().unwrap().leaderboard_visible);
    }
}
