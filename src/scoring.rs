use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

use crate::contest::model::{SubtaskKind, ValidatedProblem};
use crate::model::Verdict;

#[derive(Debug, Clone)]
pub struct RawCase {
    pub name: String,
    pub verdict: Verdict,
    pub time_ms: u64,
    pub memory_kb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoredCase {
    pub name: String,
    pub subtask: String,
    pub verdict: Verdict,
    pub time_ms: u64,
    pub memory_kb: u64,
    pub score: i32,
    pub max_score: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemScore {
    pub cases: Vec<ScoredCase>,
    pub total: i32,
    pub verdict: Verdict,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScoreError {
    #[error("缺少测试点结果: {0}")]
    MissingCase(String),
    #[error("测试点结果重复: {0}")]
    DuplicateCase(String),
    #[error("题目没有测试点")]
    NoCases,
}

pub fn allocate(total: i32, count: usize) -> Vec<i32> {
    if count == 0 {
        return Vec::new();
    }
    let count_i32 = count as i32;
    let base = total / count_i32;
    let remainder = total % count_i32;
    (0..count)
        .map(|index| base + i32::from((index as i32) < remainder))
        .collect()
}

pub fn score_problem(
    problem: &ValidatedProblem,
    raw: &[RawCase],
) -> Result<ProblemScore, ScoreError> {
    if problem.test_cases.is_empty() {
        return Err(ScoreError::NoCases);
    }
    let mut by_name = HashMap::new();
    for case in raw {
        if by_name.insert(case.name.as_str(), case).is_some() {
            return Err(ScoreError::DuplicateCase(case.name.clone()));
        }
    }
    for test_case in &problem.test_cases {
        if !by_name.contains_key(test_case.name.as_str()) {
            return Err(ScoreError::MissingCase(test_case.name.clone()));
        }
    }

    let mut scored = HashMap::new();
    for subtask in &problem.subtasks {
        let mut passed_flags = Vec::with_capacity(subtask.cases.len());
        for case_name in &subtask.cases {
            let raw_case = by_name
                .get(case_name.as_str())
                .ok_or_else(|| ScoreError::MissingCase(case_name.clone()))?;
            passed_flags.push(raw_case.verdict == Verdict::Ac);
        }
        let max_scores = allocate(subtask.score, subtask.cases.len());
        let any_passed = passed_flags.iter().any(|passed| *passed);
        let all_passed = passed_flags.iter().all(|passed| *passed);

        for (index, case_name) in subtask.cases.iter().enumerate() {
            let raw_case = by_name
                .get(case_name.as_str())
                .ok_or_else(|| ScoreError::MissingCase(case_name.clone()))?;
            let max_score = max_scores[index];
            let score = match subtask.kind {
                SubtaskKind::Sum => {
                    if passed_flags[index] {
                        max_score
                    } else {
                        0
                    }
                }
                SubtaskKind::Min => {
                    if all_passed {
                        max_score
                    } else {
                        0
                    }
                }
                SubtaskKind::Max => {
                    if any_passed {
                        max_score
                    } else {
                        0
                    }
                }
            };
            scored.insert(
                case_name.clone(),
                ScoredCase {
                    name: case_name.clone(),
                    subtask: subtask.name.clone(),
                    verdict: raw_case.verdict,
                    time_ms: raw_case.time_ms,
                    memory_kb: raw_case.memory_kb,
                    score,
                    max_score,
                },
            );
        }
    }

    let cases = problem
        .test_cases
        .iter()
        .map(|test_case| {
            scored
                .remove(&test_case.name)
                .ok_or_else(|| ScoreError::MissingCase(test_case.name.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let total = cases.iter().map(|case| case.score).sum();
    let verdict = cases
        .iter()
        .find(|case| case.verdict != Verdict::Ac)
        .map(|case| case.verdict)
        .unwrap_or(Verdict::Ac);

    Ok(ProblemScore {
        cases,
        total,
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{SubtaskKind, TestCase, ValidatedSubtask};
    use std::path::PathBuf;

    fn test_case(name: &str) -> TestCase {
        TestCase {
            name: name.to_string(),
            in_path: PathBuf::from(format!("{name}.in")),
            out_path: PathBuf::from(format!("{name}.out")),
        }
    }

    fn problem(
        full_score: i32,
        subtasks: Vec<ValidatedSubtask>,
        names: &[&str],
    ) -> ValidatedProblem {
        let cases = names.iter().map(|name| test_case(name)).collect();
        ValidatedProblem {
            key: "A".to_string(),
            title: "A".to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score,
            root: PathBuf::from("C:/problem"),
            statement_path: PathBuf::from("C:/problem/statement.md"),
            test_cases: cases,
            subtasks,
        }
    }

    fn subtask(name: &str, score: i32, kind: SubtaskKind, cases: &[&str]) -> ValidatedSubtask {
        ValidatedSubtask {
            name: name.to_string(),
            score,
            kind,
            cases: cases.iter().map(|case| (*case).to_string()).collect(),
        }
    }

    fn raw(values: &[(&str, Verdict)]) -> Vec<RawCase> {
        values
            .iter()
            .map(|(name, verdict)| RawCase {
                name: (*name).to_string(),
                verdict: *verdict,
                time_ms: 10,
                memory_kb: 1024,
            })
            .collect()
    }

    #[test]
    fn allocate_puts_remainder_on_front() {
        assert_eq!(allocate(100, 3), vec![34, 33, 33]);
        assert_eq!(allocate(10, 3), vec![4, 3, 3]);
        assert_eq!(allocate(0, 4), vec![0, 0, 0, 0]);
        assert!(allocate(10, 0).is_empty());
    }

    #[test]
    fn implicit_group_is_sum() {
        let p = problem(
            100,
            vec![subtask("子任务 1", 100, SubtaskKind::Sum, &["1", "2"])],
            &["1", "2"],
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Ac), ("2", Verdict::Wa)])).unwrap();
        assert_eq!(score.total, 50);
        assert_eq!(score.verdict, Verdict::Wa);
        assert_eq!(
            score
                .cases
                .iter()
                .map(|case| case.score)
                .collect::<Vec<_>>(),
            vec![50, 0]
        );
    }

    #[test]
    fn sum_group_scores_by_passed_case() {
        let p = problem(
            30,
            vec![subtask("小数据", 30, SubtaskKind::Sum, &["1", "2", "3"])],
            &["1", "2", "3"],
        );
        let score = score_problem(
            &p,
            &raw(&[("1", Verdict::Ac), ("2", Verdict::Ac), ("3", Verdict::Wa)]),
        )
        .unwrap();
        assert_eq!(score.total, 20);
        assert_eq!(
            score
                .cases
                .iter()
                .map(|case| case.score)
                .collect::<Vec<_>>(),
            vec![10, 10, 0]
        );
    }

    #[test]
    fn min_group_is_all_or_nothing() {
        let p = problem(
            70,
            vec![subtask("大数据", 70, SubtaskKind::Min, &["1", "2"])],
            &["1", "2"],
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Ac), ("2", Verdict::Wa)])).unwrap();
        assert_eq!(score.total, 0);
        assert_eq!(
            score
                .cases
                .iter()
                .map(|case| case.max_score)
                .collect::<Vec<_>>(),
            vec![35, 35]
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Ac), ("2", Verdict::Ac)])).unwrap();
        assert_eq!(score.total, 70);
    }

    #[test]
    fn max_group_scores_if_any_case_passes() {
        let p = problem(
            70,
            vec![subtask("大数据", 70, SubtaskKind::Max, &["1", "2"])],
            &["1", "2"],
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Ac), ("2", Verdict::Tle)])).unwrap();
        assert_eq!(score.total, 70);
        assert_eq!(
            score
                .cases
                .iter()
                .map(|case| case.score)
                .collect::<Vec<_>>(),
            vec![35, 35]
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Wa), ("2", Verdict::Tle)])).unwrap();
        assert_eq!(score.total, 0);
    }

    #[test]
    fn case_scores_sum_to_total_in_all_group_types() {
        for kind in [SubtaskKind::Sum, SubtaskKind::Min, SubtaskKind::Max] {
            let p = problem(
                100,
                vec![subtask("组", 100, kind, &["1", "2", "3"])],
                &["1", "2", "3"],
            );
            let score = score_problem(
                &p,
                &raw(&[("1", Verdict::Ac), ("2", Verdict::Wa), ("3", Verdict::Ac)]),
            )
            .unwrap();
            assert_eq!(
                score.cases.iter().map(|case| case.score).sum::<i32>(),
                score.total
            );
        }
    }

    #[test]
    fn output_cases_follow_natural_order() {
        let p = problem(
            100,
            vec![subtask("组", 100, SubtaskKind::Sum, &["2", "1"])],
            &["1", "2"],
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Ac), ("2", Verdict::Ac)])).unwrap();
        assert_eq!(
            score
                .cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }

    #[test]
    fn first_non_ac_case_determines_verdict() {
        let p = problem(
            100,
            vec![subtask("组", 100, SubtaskKind::Sum, &["1", "2"])],
            &["1", "2"],
        );
        let score = score_problem(&p, &raw(&[("1", Verdict::Tle), ("2", Verdict::Wa)])).unwrap();
        assert_eq!(score.verdict, Verdict::Tle);
    }

    #[test]
    fn missing_and_duplicate_cases_are_errors() {
        let p = problem(
            100,
            vec![subtask("组", 100, SubtaskKind::Sum, &["1", "2"])],
            &["1", "2"],
        );
        assert!(matches!(
            score_problem(&p, &raw(&[("1", Verdict::Ac)])),
            Err(ScoreError::MissingCase(_))
        ));
        assert!(matches!(
            score_problem(
                &p,
                &raw(&[("1", Verdict::Ac), ("1", Verdict::Ac), ("2", Verdict::Ac)])
            ),
            Err(ScoreError::DuplicateCase(_))
        ));
    }
}
