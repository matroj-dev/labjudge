use thiserror::Error;

use crate::model::Phase;

const MINUTE_MS: i64 = 60_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContestState {
    pub started_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub leaderboard_visible: bool,
    pub ended_early: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("比赛尚未开始")]
    NotStarted,
    #[error("比赛已经开始")]
    AlreadyStarted,
    #[error("时长必须在 1～600 分钟之间")]
    InvalidDuration,
    #[error("延长分钟数必须在 1～600 之间")]
    InvalidExtendMinutes,
}

pub fn phase(state: &ContestState, now_ms: i64) -> Phase {
    match (state.started_at, state.ends_at) {
        (None, _) => Phase::NotStarted,
        (Some(_), Some(ends_at)) if !state.ended_early && now_ms < ends_at => Phase::Running,
        (Some(_), _) => Phase::Ended,
    }
}

pub fn apply_start(
    state: &mut ContestState,
    now_ms: i64,
    duration_minutes: u32,
) -> Result<(), LifecycleError> {
    if state.started_at.is_some() {
        return Err(LifecycleError::AlreadyStarted);
    }
    if !(1..=600).contains(&duration_minutes) {
        return Err(LifecycleError::InvalidDuration);
    }
    state.started_at = Some(now_ms);
    state.ends_at = Some(now_ms + i64::from(duration_minutes) * MINUTE_MS);
    state.ended_early = false;
    Ok(())
}

pub fn apply_end(state: &mut ContestState, now_ms: i64) -> Result<(), LifecycleError> {
    if state.started_at.is_none() {
        return Err(LifecycleError::NotStarted);
    }
    state.ends_at = Some(now_ms);
    state.ended_early = true;
    Ok(())
}

pub fn apply_extend(
    state: &mut ContestState,
    now_ms: i64,
    minutes: u32,
) -> Result<(), LifecycleError> {
    if state.started_at.is_none() {
        return Err(LifecycleError::NotStarted);
    }
    if !(1..=600).contains(&minutes) {
        return Err(LifecycleError::InvalidExtendMinutes);
    }
    let base = if state.ended_early {
        now_ms
    } else {
        now_ms.max(state.ends_at.unwrap_or(now_ms))
    };
    state.ends_at = Some(base + i64::from(minutes) * MINUTE_MS);
    state.ended_early = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_are_derived_from_timestamps() {
        let mut state = ContestState::default();
        assert_eq!(phase(&state, 0), Phase::NotStarted);

        apply_start(&mut state, 1_000, 60).unwrap();
        assert_eq!(state.ends_at, Some(1_000 + 3_600_000));
        assert_eq!(phase(&state, 2_000), Phase::Running);
        assert_eq!(phase(&state, 1_000 + 3_600_000), Phase::Ended);

        state.ended_early = true;
        assert_eq!(phase(&state, 2_000), Phase::Ended);
    }

    #[test]
    fn start_cannot_run_twice_and_validates_duration() {
        let mut state = ContestState::default();
        assert_eq!(
            apply_start(&mut state, 0, 0),
            Err(LifecycleError::InvalidDuration)
        );
        assert_eq!(
            apply_start(&mut state, 0, 601),
            Err(LifecycleError::InvalidDuration)
        );
        apply_start(&mut state, 0, 60).unwrap();
        assert_eq!(
            apply_start(&mut state, 1, 60),
            Err(LifecycleError::AlreadyStarted)
        );
    }

    #[test]
    fn end_requires_start_and_is_immediate() {
        let mut state = ContestState::default();
        assert_eq!(apply_end(&mut state, 0), Err(LifecycleError::NotStarted));

        apply_start(&mut state, 0, 60).unwrap();
        apply_end(&mut state, 5_000).unwrap();
        assert!(state.ended_early);
        assert_eq!(phase(&state, 5_001), Phase::Ended);
    }

    #[test]
    fn extend_from_running_and_ended_resumes() {
        let mut state = ContestState::default();
        apply_start(&mut state, 0, 60).unwrap();
        apply_extend(&mut state, 1_000, 10).unwrap();
        assert_eq!(state.ends_at, Some(3_600_000 + 600_000));
        assert!(!state.ended_early);

        apply_end(&mut state, 10_000).unwrap();
        assert_eq!(state.ends_at, Some(10_000));
        apply_extend(&mut state, 5_000_000, 5).unwrap();
        assert_eq!(state.ends_at, Some(5_000_000 + 300_000));
        assert_eq!(phase(&state, 5_000_001), Phase::Running);
        assert!(!state.ended_early);
    }

    #[test]
    fn resume_before_original_end_uses_now_as_base() {
        let mut state = ContestState::default();
        apply_start(&mut state, 0, 60).unwrap();
        apply_end(&mut state, 600_000).unwrap();
        apply_extend(&mut state, 1_000_000, 5).unwrap();
        assert_eq!(state.ends_at, Some(1_300_000));
        assert_eq!(phase(&state, 1_000_001), Phase::Running);
    }

    #[test]
    fn extend_validates_minutes_and_start() {
        let mut state = ContestState::default();
        assert_eq!(
            apply_extend(&mut state, 0, 5),
            Err(LifecycleError::NotStarted)
        );
        apply_start(&mut state, 0, 60).unwrap();
        assert_eq!(
            apply_extend(&mut state, 0, 0),
            Err(LifecycleError::InvalidExtendMinutes)
        );
        assert_eq!(
            apply_extend(&mut state, 0, 601),
            Err(LifecycleError::InvalidExtendMinutes)
        );
    }
}
