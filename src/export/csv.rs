use super::{ranking_submissions, ExportError};
use crate::contest::model::ValidatedProblem;
use crate::contest::roster::Roster;
use crate::store::SubmissionRow;
use crate::timefmt::format_local;

pub fn build_csv(
    roster: &Roster,
    problems: &[ValidatedProblem],
    submissions: &[SubmissionRow],
) -> Result<Vec<u8>, ExportError> {
    let rows = ranking_submissions(roster, problems, submissions);
    let mut output = Vec::new();
    output.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    {
        let mut writer = ::csv::WriterBuilder::new()
            .terminator(::csv::Terminator::CRLF)
            .from_writer(&mut output);
        let mut header = vec![
            "排名".to_string(),
            "姓名".to_string(),
            "账号".to_string(),
            "学校".to_string(),
            "年级".to_string(),
            "总分".to_string(),
        ];
        for problem in problems {
            header.push(problem.key.clone());
        }
        header.push("最后得分时间".to_string());
        writer.write_record(header)?;

        for (index, row) in rows.iter().enumerate() {
            let mut record = vec![
                (index + 1).to_string(),
                row.student.name.clone(),
                row.student.student_id.clone(),
                row.student.school.clone(),
                row.student.grade.clone(),
                row.total.to_string(),
            ];
            for score in &row.per_problem {
                record.push(score.to_string());
            }
            record.push(row.last_score_at.map(format_local).unwrap_or_default());
            writer.write_record(record)?;
        }
        writer.flush()?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{SubtaskKind, TestCase, ValidatedSubtask};
    use crate::model::{SubmissionStatus, Verdict};
    use std::path::PathBuf;

    fn roster() -> Roster {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        std::fs::write(
            &path,
            "name,student_id,school,grade\n张三,1,一中,七年级\n李四,2,,\n",
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
        }
    }

    fn submission(student: &str, score: i32, created_at: i64) -> SubmissionRow {
        SubmissionRow {
            id: 1,
            contest_id: "c".to_string(),
            student_id: student.to_string(),
            problem_key: "A".to_string(),
            language: "cpp17".to_string(),
            source_path: PathBuf::from("C:/s.cpp"),
            status: SubmissionStatus::Done,
            verdict: Some(Verdict::Ac),
            score,
            cases_json: None,
            compile_output: None,
            error: None,
            created_at,
            judged_at: Some(created_at),
        }
    }

    #[test]
    fn csv_has_bom_header_ranking_and_empty_zero_time() {
        let bytes = build_csv(
            &roster(),
            &[problem("A")],
            &[submission("1", 100, 0), submission("2", 0, 1)],
        )
        .unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert!(lines[0].starts_with("排名,姓名,账号,学校,年级,总分,A,最后得分时间"));
        assert!(lines[1].starts_with("1,张三,1,一中,七年级,100,100,"));
        assert!(lines[2].starts_with("2,李四,2,,,0,0,"));
        assert!(!lines[2].ends_with("1970"));
    }
}
