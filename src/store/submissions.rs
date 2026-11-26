use std::path::{Path, PathBuf};

use rusqlite::types::Type;
use rusqlite::{params, OptionalExtension};

use super::{ContestStore, StoreError};
use crate::model::{SubmissionStatus, Verdict};

#[derive(Debug, Clone)]
pub struct NewSubmission {
    pub student_id: String,
    pub problem_key: String,
    pub language: String,
    pub source_path: PathBuf,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionRow {
    pub id: i64,
    pub contest_id: String,
    pub student_id: String,
    pub problem_key: String,
    pub language: String,
    pub source_path: PathBuf,
    pub status: SubmissionStatus,
    pub verdict: Option<Verdict>,
    pub score: i32,
    pub cases_json: Option<String>,
    pub compile_output: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub judged_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SubmissionResult<'a> {
    pub verdict: Verdict,
    pub score: i32,
    pub cases_json: Option<&'a str>,
    pub compile_output: Option<&'a str>,
    pub error: Option<&'a str>,
    pub judged_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub queued: usize,
    pub compiling: usize,
    pub running: usize,
    pub done: usize,
    pub failed: usize,
}

const SELECT_COLUMNS: &str = "id, contest_id, student_id, problem_key, language, source_path,
    status, verdict, score, cases_json, compile_output, error, created_at, judged_at";

impl ContestStore {
    pub fn insert_submission(&self, new: &NewSubmission) -> Result<i64, StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO submissions
             (contest_id, student_id, problem_key, language, source_path,
              status, verdict, score, cases_json, compile_output, error, created_at, judged_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'queued', NULL, 0, NULL, NULL, NULL, ?6, NULL)",
            params![
                self.contest_id(),
                new.student_id,
                new.problem_key,
                new.language,
                new.source_path.to_string_lossy().into_owned(),
                new.created_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn submission(&self, id: i64) -> Result<Option<SubmissionRow>, StoreError> {
        let conn = self.connection()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {SELECT_COLUMNS} FROM submissions
                     WHERE contest_id = ?1 AND id = ?2"
                ),
                params![self.contest_id(), id],
                row_to_submission,
            )
            .optional()?)
    }

    pub fn submissions_for_contest(&self) -> Result<Vec<SubmissionRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM submissions
             WHERE contest_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = statement.query_map(params![self.contest_id()], row_to_submission)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn submissions_for_student(
        &self,
        student_id: &str,
    ) -> Result<Vec<SubmissionRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM submissions
             WHERE contest_id = ?1 AND student_id = ?2
             ORDER BY created_at DESC, id DESC"
        ))?;
        let rows =
            statement.query_map(params![self.contest_id(), student_id], row_to_submission)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn latest_created_at(
        &self,
        student_id: &str,
        problem_key: &str,
    ) -> Result<Option<i64>, StoreError> {
        let conn = self.connection()?;
        Ok(conn.query_row(
            "SELECT MAX(created_at) FROM submissions
             WHERE contest_id = ?1 AND student_id = ?2 AND problem_key = ?3",
            params![self.contest_id(), student_id, problem_key],
            |row| row.get(0),
        )?)
    }

    pub fn active_count(&self) -> Result<usize, StoreError> {
        let conn = self.connection()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM submissions
             WHERE contest_id = ?1 AND status IN ('queued','compiling','running')",
            params![self.contest_id()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn status_counts(&self) -> Result<StatusCounts, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT status, COUNT(*) FROM submissions
             WHERE contest_id = ?1 GROUP BY status",
        )?;
        let rows = statement.query_map(params![self.contest_id()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut counts = StatusCounts::default();
        for row in rows {
            let (status, count) = row?;
            let count = count as usize;
            match SubmissionStatus::parse(&status) {
                Some(SubmissionStatus::Queued) => counts.queued = count,
                Some(SubmissionStatus::Compiling) => counts.compiling = count,
                Some(SubmissionStatus::Running) => counts.running = count,
                Some(SubmissionStatus::Done) => counts.done = count,
                Some(SubmissionStatus::Failed) => counts.failed = count,
                None => return Err(StoreError::InvalidData(format!("未知提交状态: {status}"))),
            }
        }
        Ok(counts)
    }

    pub fn recoverable_submissions(&self) -> Result<Vec<SubmissionRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM submissions
             WHERE contest_id = ?1 AND status IN ('queued','compiling','running')
             ORDER BY created_at, id"
        ))?;
        let rows = statement.query_map(params![self.contest_id()], row_to_submission)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn mark_stale_queued(&self) -> Result<usize, StoreError> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE submissions SET status = 'queued'
             WHERE contest_id = ?1 AND status IN ('queued','compiling','running')",
            params![self.contest_id()],
        )?;
        Ok(changed)
    }

    pub fn set_status(&self, id: i64, status: SubmissionStatus) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE submissions SET status = ?3 WHERE contest_id = ?1 AND id = ?2",
            params![self.contest_id(), id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn claim_queued(&self, id: i64) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE submissions SET status = 'compiling'
             WHERE contest_id = ?1 AND id = ?2 AND status = 'queued'",
            params![self.contest_id(), id],
        )?;
        Ok(changed > 0)
    }

    pub fn set_result(&self, id: i64, result: &SubmissionResult<'_>) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE submissions
             SET status = 'done', verdict = ?3, score = ?4, cases_json = ?5,
                 compile_output = ?6, error = ?7, judged_at = ?8
             WHERE contest_id = ?1 AND id = ?2",
            params![
                self.contest_id(),
                id,
                result.verdict.as_str(),
                result.score,
                result.cases_json,
                result.compile_output,
                result.error,
                result.judged_at,
            ],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: i64, error: &str, judged_at: i64) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE submissions
             SET status = 'failed', verdict = NULL, cases_json = NULL,
                 compile_output = NULL, error = ?3, judged_at = ?4
             WHERE contest_id = ?1 AND id = ?2",
            params![self.contest_id(), id, error, judged_at],
        )?;
        Ok(())
    }

    pub fn clear_result(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE submissions
             SET status = 'queued', verdict = NULL, score = 0, cases_json = NULL,
                 compile_output = NULL, error = NULL, judged_at = NULL
             WHERE contest_id = ?1 AND id = ?2",
            params![self.contest_id(), id],
        )?;
        Ok(())
    }

    pub fn update_source_path(&self, id: i64, source_path: &Path) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE submissions SET source_path = ?3 WHERE contest_id = ?1 AND id = ?2",
            params![
                self.contest_id(),
                id,
                source_path.to_string_lossy().into_owned()
            ],
        )?;
        Ok(())
    }

    pub fn delete_submission(&self, id: i64) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "DELETE FROM submissions WHERE contest_id = ?1 AND id = ?2",
            params![self.contest_id(), id],
        )?;
        Ok(changed > 0)
    }
}

fn row_to_submission(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubmissionRow> {
    let status_value: String = row.get(6)?;
    let status = SubmissionStatus::parse(&status_value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("未知提交状态: {status_value}"),
            )),
        )
    })?;
    let verdict_value: Option<String> = row.get(7)?;
    let verdict = match verdict_value {
        Some(value) => Some(Verdict::parse(&value).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("未知判定: {value}"),
                )),
            )
        })?),
        None => None,
    };

    Ok(SubmissionRow {
        id: row.get(0)?,
        contest_id: row.get(1)?,
        student_id: row.get(2)?,
        problem_key: row.get(3)?,
        language: row.get(4)?,
        source_path: PathBuf::from(row.get::<_, String>(5)?),
        status,
        verdict,
        score: row.get(8)?,
        cases_json: row.get(9)?,
        compile_output: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        judged_at: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_submission(student: &str, problem: &str, created_at: i64) -> NewSubmission {
        NewSubmission {
            student_id: student.to_string(),
            problem_key: problem.to_string(),
            language: "cpp17".to_string(),
            source_path: PathBuf::from(format!("C:/data/{student}-{problem}.cpp")),
            created_at,
        }
    }

    #[test]
    fn insert_and_read_queued_submission() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        let id = store
            .insert_submission(&new_submission("s1", "A", 100))
            .unwrap();

        let row = store.submission(id).unwrap().unwrap();
        assert_eq!(row.status, SubmissionStatus::Queued);
        assert_eq!(row.verdict, None);
        assert_eq!(row.score, 0);
        assert_eq!(row.created_at, 100);
        assert_eq!(store.active_count().unwrap(), 1);
        assert_eq!(store.latest_created_at("s1", "A").unwrap(), Some(100));
        assert_eq!(store.latest_created_at("s1", "B").unwrap(), None);
    }

    #[test]
    fn result_transitions_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        let id = store
            .insert_submission(&new_submission("s1", "A", 100))
            .unwrap();

        store
            .set_result(
                id,
                &SubmissionResult {
                    verdict: Verdict::Ac,
                    score: 100,
                    cases_json: Some("[{\"name\":\"1\"}]"),
                    compile_output: None,
                    error: None,
                    judged_at: 200,
                },
            )
            .unwrap();
        let row = store.submission(id).unwrap().unwrap();
        assert_eq!(row.status, SubmissionStatus::Done);
        assert_eq!(row.verdict, Some(Verdict::Ac));
        assert_eq!(row.score, 100);
        assert_eq!(row.judged_at, Some(200));
        assert_eq!(store.active_count().unwrap(), 0);

        let counts = store.status_counts().unwrap();
        assert_eq!(counts.done, 1);
        assert_eq!(counts.queued, 0);

        store.clear_result(id).unwrap();
        let row = store.submission(id).unwrap().unwrap();
        assert_eq!(row.status, SubmissionStatus::Queued);
        assert_eq!(row.verdict, None);
        assert_eq!(row.score, 0);
        assert_eq!(row.judged_at, None);
    }

    #[test]
    fn recovery_requeues_stale_states() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        let a = store
            .insert_submission(&new_submission("s1", "A", 100))
            .unwrap();
        let b = store
            .insert_submission(&new_submission("s1", "B", 200))
            .unwrap();
        let c = store
            .insert_submission(&new_submission("s1", "C", 300))
            .unwrap();
        store.set_status(a, SubmissionStatus::Compiling).unwrap();
        store.set_status(b, SubmissionStatus::Running).unwrap();
        store.set_status(c, SubmissionStatus::Done).unwrap();

        assert_eq!(store.recoverable_submissions().unwrap().len(), 2);
        assert_eq!(store.mark_stale_queued().unwrap(), 2);
        assert_eq!(store.recoverable_submissions().unwrap().len(), 2);
        assert_eq!(store.status_counts().unwrap().queued, 2);
        assert_eq!(store.status_counts().unwrap().done, 1);
        assert!(store.claim_queued(a).unwrap());
        assert!(!store.claim_queued(a).unwrap());
    }

    #[test]
    fn failed_and_delete_and_source_update() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        let id = store
            .insert_submission(&new_submission("s1", "A", 100))
            .unwrap();

        store.mark_failed(id, "源码缺失", 200).unwrap();
        let row = store.submission(id).unwrap().unwrap();
        assert_eq!(row.status, SubmissionStatus::Failed);
        assert_eq!(row.error.as_deref(), Some("源码缺失"));
        assert_eq!(row.verdict, None);

        store
            .update_source_path(id, std::path::Path::new("C:/data/new.cpp"))
            .unwrap();
        assert_eq!(
            store.submission(id).unwrap().unwrap().source_path,
            PathBuf::from("C:/data/new.cpp")
        );

        assert!(store.delete_submission(id).unwrap());
        assert!(store.submission(id).unwrap().is_none());
        assert!(!store.delete_submission(id).unwrap());
    }

    #[test]
    fn filters_by_student() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        store
            .insert_submission(&new_submission("s1", "A", 100))
            .unwrap();
        store
            .insert_submission(&new_submission("s1", "B", 200))
            .unwrap();
        store
            .insert_submission(&new_submission("s2", "A", 300))
            .unwrap();

        assert_eq!(store.submissions_for_contest().unwrap().len(), 3);
        assert_eq!(store.submissions_for_student("s1").unwrap().len(), 2);
        assert_eq!(store.submissions_for_student("s2").unwrap().len(), 1);
    }
}
