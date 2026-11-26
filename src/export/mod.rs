use thiserror::Error;

use crate::contest::model::ValidatedProblem;
use crate::contest::roster::Roster;
use crate::store::SubmissionRow;

pub mod csv;
pub mod zip;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("CSV 生成失败: {0}")]
    Csv(#[from] ::csv::Error),
    #[error("文件读取失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP 生成失败: {0}")]
    Zip(#[from] ::zip::result::ZipError),
}

pub fn csv_response(bytes: Vec<u8>, filename: &str) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    response
}

pub fn zip_response(bytes: Vec<u8>, filename: &str) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    response
}

pub(crate) fn manifest_csv(rows: Vec<ManifestRow>) -> Result<Vec<u8>, ExportError> {
    let mut output = Vec::new();
    output.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    {
        let mut writer = ::csv::WriterBuilder::new()
            .terminator(::csv::Terminator::CRLF)
            .from_writer(&mut output);
        writer.write_record([
            "账号",
            "姓名",
            "题号",
            "提交号",
            "提交时间",
            "判定",
            "分数",
            "文件名",
        ])?;
        for row in rows {
            writer.write_record([
                row.student_id,
                row.student_name,
                row.problem_key,
                row.submission_id.to_string(),
                row.created_at,
                row.verdict,
                row.score.to_string(),
                row.filename,
            ])?;
        }
        writer.flush()?;
    }
    Ok(output)
}

pub(crate) struct ManifestRow {
    pub student_id: String,
    pub student_name: String,
    pub problem_key: String,
    pub submission_id: i64,
    pub created_at: String,
    pub verdict: String,
    pub score: i32,
    pub filename: String,
}

pub(crate) fn sanitize_component(value: &str) -> String {
    let mut result = value
        .chars()
        .map(|ch| match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect::<String>();
    while result.ends_with([' ', '.']) {
        result.pop();
    }
    while result.starts_with([' ', '.']) {
        result.remove(0);
    }
    if result.is_empty() {
        "submission".to_string()
    } else {
        result
    }
}

pub(crate) fn student_for<'a>(
    roster: &'a Roster,
    student_id: &str,
) -> Option<&'a crate::contest::roster::Student> {
    roster
        .students()
        .iter()
        .find(|student| student.student_id == student_id)
}

pub(crate) fn ranking_submissions(
    roster: &Roster,
    problems: &[ValidatedProblem],
    submissions: &[SubmissionRow],
) -> Vec<crate::ranking::RankRow> {
    crate::ranking::compute_ranking(roster, problems, submissions)
}
