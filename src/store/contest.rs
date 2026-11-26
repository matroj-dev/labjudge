use rusqlite::{params, OptionalExtension};

use super::{ContestStore, StoreError};
use crate::contest::lifecycle::ContestState;

impl ContestStore {
    pub fn state(&self) -> Result<ContestState, StoreError> {
        let conn = self.connection()?;
        let state = conn
            .query_row(
                "SELECT started_at, ends_at, leaderboard_visible, ended_early
                 FROM contest_state WHERE contest_id = ?1",
                params![self.contest_id()],
                |row| {
                    Ok(ContestState {
                        started_at: row.get(0)?,
                        ends_at: row.get(1)?,
                        leaderboard_visible: row.get::<_, i64>(2)? != 0,
                        ended_early: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::InvalidData("contest_state 行缺失".to_string()))?;
        Ok(state)
    }

    pub fn set_state(&self, state: &ContestState) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE contest_state
             SET started_at = ?2, ends_at = ?3, leaderboard_visible = ?4,
                 ended_early = ?5, updated_at = ?6
             WHERE contest_id = ?1",
            params![
                self.contest_id(),
                state.started_at,
                state.ends_at,
                if state.leaderboard_visible {
                    1i64
                } else {
                    0i64
                },
                if state.ended_early { 1i64 } else { 0i64 },
                crate::timefmt::now_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn set_leaderboard_visible(&self, visible: bool) -> Result<(), StoreError> {
        let mut state = self.state()?;
        state.leaderboard_visible = visible;
        self.set_state(&state)
    }

    pub fn meta_value(&self, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.connection()?;
        Ok(conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_and_updates_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        let mut state = store.state().unwrap();
        assert_eq!(state.started_at, None);

        state.started_at = Some(1000);
        state.ends_at = Some(2000);
        state.leaderboard_visible = true;
        state.ended_early = true;
        store.set_state(&state).unwrap();
        assert_eq!(store.state().unwrap(), state);

        store.set_leaderboard_visible(false).unwrap();
        assert!(!store.state().unwrap().leaderboard_visible);
    }

    #[test]
    fn state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = ContestStore::open(dir.path(), "c", false).unwrap();
            let mut state = store.state().unwrap();
            state.started_at = Some(1);
            state.ends_at = Some(2);
            store.set_state(&state).unwrap();
        }
        let store = ContestStore::open(dir.path(), "c", false).unwrap();
        assert_eq!(store.state().unwrap().started_at, Some(1));
    }
}
