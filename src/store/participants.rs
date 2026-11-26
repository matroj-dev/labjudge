use rusqlite::params;

use super::{ContestStore, StoreError};
use crate::contest::roster::Student;

/// 参赛记录：哪名学生登录进入了比赛，第一次进入与最后一次活动的时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantRow {
    pub student_id: String,
    pub joined_at: i64,
    pub last_seen_at: i64,
}

impl ContestStore {
    pub fn upsert_participant(&self, student: &Student, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO participants
             (contest_id, student_id, name, school, grade, last_seen_at, joined_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(contest_id, student_id) DO UPDATE SET
                 name = excluded.name,
                 school = excluded.school,
                 grade = excluded.grade,
                 last_seen_at = excluded.last_seen_at",
            params![
                self.contest_id(),
                student.student_id,
                student.name,
                student.school,
                student.grade,
                now_ms,
            ],
        )?;
        Ok(())
    }

    pub fn touch_participant_throttled(
        &self,
        student_id: &str,
        now_ms: i64,
        min_interval_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE participants SET last_seen_at = ?3
             WHERE contest_id = ?1 AND student_id = ?2 AND last_seen_at <= ?3 - ?4",
            params![self.contest_id(), student_id, now_ms, min_interval_ms],
        )?;
        Ok(changed > 0)
    }

    pub fn participants(&self) -> Result<Vec<ParticipantRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT student_id, joined_at, last_seen_at
             FROM participants WHERE contest_id = ?1
             ORDER BY student_id",
        )?;
        let rows = statement.query_map(params![self.contest_id()], |row| {
            Ok(ParticipantRow {
                student_id: row.get(0)?,
                joined_at: row.get(1)?,
                last_seen_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 单个学生的参赛记录，用于比赛首页的「我的信息」。
    pub fn participant(&self, student_id: &str) -> Result<Option<ParticipantRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT student_id, joined_at, last_seen_at
             FROM participants WHERE contest_id = ?1 AND student_id = ?2",
        )?;
        let mut rows = statement.query_map(params![self.contest_id(), student_id], |row| {
            Ok(ParticipantRow {
                student_id: row.get(0)?,
                joined_at: row.get(1)?,
                last_seen_at: row.get(2)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn clear_participants(&self) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM participants WHERE contest_id = ?1",
            params![self.contest_id()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn student(id: &str, name: &str) -> Student {
        Student {
            student_id: id.to_string(),
            name: name.to_string(),
            school: "第一中学".to_string(),
            grade: "七年级".to_string(),
        }
    }

    #[test]
    fn upsert_updates_and_touch_moves_last_seen() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        store
            .upsert_participant(&student("1", "张三"), 100)
            .unwrap();
        store
            .upsert_participant(&student("1", "张三丰"), 200)
            .unwrap();

        let rows = store.participants().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].student_id, "1");
        assert_eq!(rows[0].last_seen_at, 200);
        // 重复进入只更新活动时间，不会覆盖第一次进入的时间。
        assert_eq!(rows[0].joined_at, 100);
        assert_eq!(store.participant("1").unwrap().unwrap().joined_at, 100);
        assert!(store.participant("missing").unwrap().is_none());

        assert!(!store.touch_participant_throttled("1", 205, 10_000).unwrap());
        assert!(store
            .touch_participant_throttled("1", 10_300, 10_000)
            .unwrap());
        assert_eq!(store.participants().unwrap()[0].last_seen_at, 10_300);
    }
}
