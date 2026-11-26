use rusqlite::{params, OptionalExtension};

use super::{ContestStore, StoreError};

impl ContestStore {
    /// 每次登录覆盖旧值：同一账号只保留一个活动会话。
    pub fn upsert_session(
        &self,
        student_id: &str,
        session_id: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO student_sessions (contest_id, student_id, session_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(contest_id, student_id) DO UPDATE SET
                 session_id = excluded.session_id,
                 created_at = excluded.created_at",
            params![self.contest_id(), student_id, session_id, now_ms],
        )?;
        Ok(())
    }

    pub fn session_id(&self, student_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.connection()?;
        Ok(conn
            .query_row(
                "SELECT session_id FROM student_sessions
                 WHERE contest_id = ?1 AND student_id = ?2",
                params![self.contest_id(), student_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn delete_session(&self, student_id: &str) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM student_sessions WHERE contest_id = ?1 AND student_id = ?2",
            params![self.contest_id(), student_id],
        )?;
        Ok(())
    }

    pub fn clear_sessions(&self) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM student_sessions WHERE contest_id = ?1",
            params![self.contest_id()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_replaces_previous_session_and_clear_removes_all() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();

        store.upsert_session("s1", "a", 100).unwrap();
        assert_eq!(store.session_id("s1").unwrap().as_deref(), Some("a"));
        store.upsert_session("s2", "b", 200).unwrap();

        store.upsert_session("s1", "c", 300).unwrap();
        assert_eq!(store.session_id("s1").unwrap().as_deref(), Some("c"));
        assert_eq!(store.session_id("s2").unwrap().as_deref(), Some("b"));

        store.delete_session("s1").unwrap();
        assert!(store.session_id("s1").unwrap().is_none());
        assert_eq!(store.session_id("s2").unwrap().as_deref(), Some("b"));

        store.clear_sessions().unwrap();
        assert!(store.session_id("s2").unwrap().is_none());
    }
}
