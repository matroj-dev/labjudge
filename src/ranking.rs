use std::collections::HashMap;

use crate::contest::model::ValidatedProblem;
use crate::contest::roster::{Roster, Student};
use crate::model::SubmissionStatus;
use crate::store::SubmissionRow;

#[derive(Debug, Clone)]
pub struct RankRow {
    pub student: Student,
    pub total: i32,
    pub per_problem: Vec<i32>,
    pub last_score_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BestScore {
    score: i32,
    created_at: Option<i64>,
}

pub fn compute_ranking(
    roster: &Roster,
    problems: &[ValidatedProblem],
    submissions: &[SubmissionRow],
) -> Vec<RankRow> {
    let problem_index = problems
        .iter()
        .enumerate()
        .map(|(index, problem)| (problem.key.as_str(), index))
        .collect::<HashMap<_, _>>();

    let mut best: HashMap<&str, Vec<BestScore>> = HashMap::new();
    for student in roster.students() {
        best.insert(
            student.student_id.as_str(),
            vec![BestScore::default(); problems.len()],
        );
    }

    for submission in submissions {
        if submission.status != SubmissionStatus::Done {
            continue;
        }
        let Some(index) = problem_index.get(submission.problem_key.as_str()).copied() else {
            continue;
        };
        let Some(scores) = best.get_mut(submission.student_id.as_str()) else {
            continue;
        };
        let current = &mut scores[index];
        let better = submission.score > current.score
            || (submission.score == current.score
                && current
                    .created_at
                    .map(|created_at| submission.created_at < created_at)
                    .unwrap_or(true));
        if better {
            current.score = submission.score;
            current.created_at = Some(submission.created_at);
        }
    }

    let mut rows = roster
        .students()
        .iter()
        .map(|student| {
            let scores = best
                .get(student.student_id.as_str())
                .cloned()
                .unwrap_or_else(|| vec![BestScore::default(); problems.len()]);
            let per_problem = scores.iter().map(|score| score.score).collect::<Vec<_>>();
            let total = per_problem.iter().sum();
            let last_score_at = scores
                .iter()
                .filter(|score| score.score > 0)
                .filter_map(|score| score.created_at)
                .max();
            RankRow {
                student: student.clone(),
                total,
                per_problem,
                last_score_at,
            }
        })
        .collect::<Vec<_>>();

    rows.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| match (a.last_score_at, b.last_score_at) {
                (Some(left), Some(right)) => left.cmp(&right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.student.student_id.cmp(&b.student.student_id))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{SubtaskKind, TestCase, ValidatedProblem, ValidatedSubtask};
    use crate::model::Verdict;
    use std::path::PathBuf;

    fn roster() -> Roster {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        std::fs::write(
            &path,
            "name,student_id,password\n张三,1,p1\n李四,2,p2\n王五,3,p3\n",
        )
        .unwrap();
        Roster::load(&path).unwrap()
    }

    fn problem(key: &str) -> ValidatedProblem {
        ValidatedProblem {
            key: key.to_string(),
            title: key.to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score: 100,
            root: PathBuf::from("C:/p"),
            statement_path: PathBuf::from("C:/p/statement.md"),
            test_cases: vec![TestCase {
                name: "1".to_string(),
                in_path: PathBuf::from("C:/1.in"),
                out_path: PathBuf::from("C:/1.out"),
            }],
            subtasks: vec![ValidatedSubtask {
                name: "子任务 1".to_string(),
                score: 100,
                kind: SubtaskKind::Sum,
                cases: vec!["1".to_string()],
            }],
            checker: None,
        }
    }

    fn submission(
        id: i64,
        student: &str,
        problem: &str,
        score: i32,
        created_at: i64,
    ) -> SubmissionRow {
        SubmissionRow {
            id,
            contest_id: "c".to_string(),
            student_id: student.to_string(),
            problem_key: problem.to_string(),
            language: "cpp17".to_string(),
            source_path: PathBuf::from("C:/s.cpp"),
            status: SubmissionStatus::Done,
            verdict: Some(if score > 0 { Verdict::Ac } else { Verdict::Wa }),
            score,
            cases_json: None,
            compile_output: None,
            error: None,
            created_at,
            judged_at: Some(created_at),
        }
    }

    #[test]
    fn takes_best_per_problem_and_earliest_best_time() {
        let problems = vec![problem("A"), problem("B")];
        let submissions = vec![
            submission(1, "1", "A", 50, 100),
            submission(2, "1", "A", 50, 50),
            submission(3, "1", "B", 100, 200),
            submission(4, "2", "A", 100, 300),
        ];
        let rows = compute_ranking(&roster(), &problems, &submissions);
        let first = rows
            .iter()
            .find(|row| row.student.student_id == "1")
            .unwrap();
        assert_eq!(first.total, 150);
        assert_eq!(first.per_problem, vec![50, 100]);
        assert_eq!(first.last_score_at, Some(200));

        let second = rows
            .iter()
            .find(|row| row.student.student_id == "2")
            .unwrap();
        assert_eq!(second.total, 100);
        assert_eq!(second.last_score_at, Some(300));
    }

    #[test]
    fn ordering_uses_total_then_time_then_student_id() {
        let problems = vec![problem("A"), problem("B")];
        let submissions = vec![
            submission(1, "1", "A", 50, 200),
            submission(2, "1", "B", 50, 100),
            submission(3, "2", "A", 100, 50),
        ];
        let rows = compute_ranking(&roster(), &problems, &submissions);
        assert_eq!(rows[0].student.student_id, "2");
        assert_eq!(rows[1].student.student_id, "1");
        assert_eq!(rows[2].student.student_id, "3");
        assert_eq!(rows[2].total, 0);
        assert_eq!(rows[2].last_score_at, None);
    }

    #[test]
    fn zero_score_students_are_last_and_sorted_by_id() {
        let problems = vec![problem("A")];
        let rows = compute_ranking(&roster(), &problems, &[]);
        assert_eq!(
            rows.iter()
                .map(|row| row.student.student_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );
        assert!(rows
            .iter()
            .all(|row| row.total == 0 && row.last_score_at.is_none()));
    }
}
