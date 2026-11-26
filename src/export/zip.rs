use std::fs;
use std::io::{Cursor, Write};

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};

use super::{manifest_csv, sanitize_component, student_for, ExportError, ManifestRow};
use crate::contest::roster::Roster;
use crate::store::SubmissionRow;
use crate::timefmt::format_local;

pub fn build_zip(roster: &Roster, submissions: &[SubmissionRow]) -> Result<Vec<u8>, ExportError> {
    let cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut manifest = Vec::new();

    for row in submissions {
        let student = student_for(roster, &row.student_id);
        let student_name = student
            .map(|student| student.name.clone())
            .unwrap_or_else(|| row.student_id.clone());
        let folder = sanitize_component(&format!("{}_{}", row.student_id, student_name));
        let file_name = sanitize_component(&format!("{}_{}.cpp", row.problem_key, row.id));
        let entry_name = format!("{folder}/{file_name}");

        let source = fs::read(&row.source_path);
        if let Ok(bytes) = &source {
            zip.start_file(&entry_name, options)?;
            zip.write_all(bytes)?;
        }
        manifest.push(ManifestRow {
            student_id: row.student_id.clone(),
            student_name,
            problem_key: row.problem_key.clone(),
            submission_id: row.id,
            created_at: format_local(row.created_at),
            verdict: row
                .verdict
                .map(|verdict| verdict.as_str().to_string())
                .unwrap_or_else(|| "-".to_string()),
            score: row.score,
            filename: if source.is_ok() {
                entry_name
            } else {
                "<缺失>".to_string()
            },
        });
    }

    let manifest_bytes = manifest_csv(manifest)?;
    zip.start_file("manifest.csv", options)?;
    zip.write_all(&manifest_bytes)?;

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SubmissionStatus, Verdict};
    use std::io::Read;
    use std::path::PathBuf;

    fn roster() -> Roster {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        fs::write(&path, "name,student_id,password\n张三,2026/001,p\n").unwrap();
        Roster::load(&path).unwrap()
    }

    fn row(id: i64, source: PathBuf) -> SubmissionRow {
        SubmissionRow {
            id,
            contest_id: "c".to_string(),
            student_id: "2026/001".to_string(),
            problem_key: "A".to_string(),
            language: "cpp17".to_string(),
            source_path: source,
            status: SubmissionStatus::Done,
            verdict: Some(Verdict::Ac),
            score: 100,
            cases_json: None,
            compile_output: None,
            error: None,
            created_at: 0,
            judged_at: Some(1),
        }
    }

    #[test]
    fn zip_sanitizes_paths_and_includes_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("1.cpp");
        fs::write(&source, "int main(){}").unwrap();
        let bytes = build_zip(
            &roster(),
            &[row(1, source), row(2, dir.path().join("missing.cpp"))],
        )
        .unwrap();

        let mut archive = ::zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"2026_001_张三/A_1.cpp".to_string()));
        assert!(names.contains(&"manifest.csv".to_string()));
        assert!(names.iter().all(|name| !name.contains("..")));

        let mut entry = archive.by_name("2026_001_张三/A_1.cpp").unwrap();
        let mut content = String::new();
        entry.read_to_string(&mut content).unwrap();
        assert_eq!(content, "int main(){}");
        drop(entry);

        let mut manifest = archive.by_name("manifest.csv").unwrap();
        let mut content = Vec::new();
        manifest.read_to_end(&mut content).unwrap();
        assert_eq!(&content[..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8(content[3..].to_vec()).unwrap();
        assert!(text.contains("<缺失>"));
        assert!(text.contains("账号,姓名,题号,提交号,提交时间,判定,分数,文件名"));
    }

    #[test]
    fn sanitizer_replaces_illegal_characters() {
        assert_eq!(sanitize_component("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_component(" . "), "submission");
    }
}
