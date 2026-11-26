use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use thiserror::Error;

use super::go_judge::CheckerRunResult;
use super::{CompileOutcome, Compiler};
use crate::model::Verdict;
use crate::services::registry::ContestPackage;

/// 一次成功准备的 checker。
#[derive(Debug, Clone)]
pub struct CheckerPrep {
    pub contest: String,
    pub problem: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Error)]
pub enum CheckerError {
    #[error(
        "缺少 testlib.h（{path}），无法编译 checker。请确认发布包的 include/testlib.h 未被删除。"
    )]
    MissingTestlib { path: PathBuf },
    #[error("准备 checker 目录失败（{path}）: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("比赛 {contest} 的 {problem} 题 checker {stage}失败：\n{detail}")]
    Problem {
        contest: String,
        problem: String,
        stage: &'static str,
        detail: String,
    },
}

pub fn checker_verdict(result: &CheckerRunResult) -> Result<Verdict, String> {
    let fail = |reason: String| Err(with_message(reason, result));

    if result.status == "Accepted" {
        return Ok(Verdict::Ac);
    }
    if result.status == "Wrong Answer" {
        return Ok(Verdict::Wa);
    }
    if result.status != "Nonzero Exit Status" {
        return fail(format!("运行异常（{}）", result.status));
    }

    match result.exit_status {
        0 => Ok(Verdict::Ac),
        1 | 2 | 4 => Ok(Verdict::Wa),
        3 => fail("checker 报告题目或数据有误（exit 3）".to_string()),
        5 => fail("checker 使用了 quitp 部分分，当前版本不支持部分分".to_string()),
        code if code >= 16 => fail(format!(
            "checker 返回部分分（exit {code}），当前版本不支持部分分"
        )),
        code => fail(format!("checker 异常退出（exit {code}）")),
    }
}

fn with_message(reason: String, result: &CheckerRunResult) -> String {
    match &result.message {
        Some(message) if !message.is_empty() => format!("{reason}\n{message}"),
        _ => reason,
    }
}

pub async fn prepare_all(
    packages: &[Arc<ContestPackage>],
    compiler: &Compiler,
    testlib: &Path,
    checkers_dir: &Path,
) -> Result<Vec<CheckerPrep>, CheckerError> {
    let _ = fs::remove_dir_all(checkers_dir);
    fs::create_dir_all(checkers_dir).map_err(|source| CheckerError::Io {
        path: checkers_dir.to_path_buf(),
        source,
    })?;
    // 只有真的有 checker 才要求 testlib.h；没有 SPJ 题的比赛包不受影响。
    if packages.iter().any(|package| has_checker(package)) && !testlib.is_file() {
        return Err(CheckerError::MissingTestlib {
            path: testlib.to_path_buf(),
        });
    }

    let mut prepared = Vec::new();
    for package in packages {
        prepared.extend(prepare_contest(package, compiler, testlib, checkers_dir).await?);
    }
    Ok(prepared)
}

pub async fn prepare_contest(
    package: &ContestPackage,
    compiler: &Compiler,
    testlib: &Path,
    checkers_dir: &Path,
) -> Result<Vec<CheckerPrep>, CheckerError> {
    let contest_dir = checkers_dir.join(&package.id);
    let _ = fs::remove_file(contest_dir.join("checker.cpp"));
    let mut prepared = Vec::new();

    for problem in &package.problems {
        let Some(checker) = &problem.checker else {
            continue;
        };
        if !testlib.is_file() {
            return Err(CheckerError::MissingTestlib {
                path: testlib.to_path_buf(),
            });
        }
        let started = Instant::now();
        tracing::info!(
            contest = %package.id,
            problem = %problem.key,
            source = %checker.source.display(),
            "编译 SPJ 检查器"
        );

        let out_name = format!("{}.exe", problem.key);
        let outcome = compiler
            .compile_checker(
                &checker.source,
                &contest_dir,
                &out_name,
                testlib_dir(testlib),
            )
            .map_err(|error| CheckerError::Problem {
                contest: package.id.clone(),
                problem: problem.key.clone(),
                stage: "编译",
                detail: format!("无法调用编译器: {error}"),
            })?;
        match outcome {
            CompileOutcome::Success(_) => {}
            CompileOutcome::Ce { output } => {
                return Err(CheckerError::Problem {
                    contest: package.id.clone(),
                    problem: problem.key.clone(),
                    stage: "编译",
                    detail: output,
                })
            }
        };

        let elapsed_ms = started.elapsed().as_millis() as u64;
        tracing::info!(
            contest = %package.id,
            problem = %problem.key,
            elapsed_ms,
            "SPJ 检查器就绪"
        );
        prepared.push(CheckerPrep {
            contest: package.id.clone(),
            problem: problem.key.clone(),
            elapsed_ms,
        });
    }

    Ok(prepared)
}

fn has_checker(package: &ContestPackage) -> bool {
    package
        .problems
        .iter()
        .any(|problem| problem.checker.is_some())
}

/// `testlib.h` 所在的目录，用作 `-I`。
fn testlib_dir(testlib: &Path) -> &Path {
    testlib.parent().unwrap_or(testlib)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(status: &str, exit_status: i64, message: Option<&str>) -> CheckerRunResult {
        CheckerRunResult {
            status: status.to_string(),
            exit_status,
            message: message.map(str::to_string),
        }
    }

    #[test]
    fn exit_codes_follow_testlib_semantics() {
        assert_eq!(
            checker_verdict(&result("Accepted", 0, None)),
            Ok(Verdict::Ac)
        );
        assert_eq!(
            checker_verdict(&result("Nonzero Exit Status", 1, None)),
            Ok(Verdict::Wa)
        );
        assert_eq!(
            checker_verdict(&result("Nonzero Exit Status", 2, None)),
            Ok(Verdict::Wa)
        );
        assert_eq!(
            checker_verdict(&result("Nonzero Exit Status", 4, None)),
            Ok(Verdict::Wa)
        );
        assert_eq!(
            checker_verdict(&result("Wrong Answer", 1, None)),
            Ok(Verdict::Wa)
        );
    }

    #[test]
    fn checker_faults_are_errors_not_wrong_answers() {
        for code in [3, 5, 16, 42, 255] {
            let verdict = checker_verdict(&result("Nonzero Exit Status", code, None));
            assert!(verdict.is_err(), "exit {code} 应当判定为系统失败");
        }
        assert!(checker_verdict(&result("Time Limit Exceeded", 0, None)).is_err());
        assert!(checker_verdict(&result("Signalled", 0, None)).is_err());
    }

    #[test]
    fn partial_score_messages_are_explicit() {
        let error = checker_verdict(&result("Nonzero Exit Status", 5, None)).unwrap_err();
        assert!(error.contains("部分分"));
        let error = checker_verdict(&result("Nonzero Exit Status", 20, None)).unwrap_err();
        assert!(error.contains("部分分"));
    }

    #[test]
    fn stderr_message_is_attached() {
        let error = checker_verdict(&result(
            "Nonzero Exit Status",
            3,
            Some("FAIL expected 5, found 3"),
        ))
        .unwrap_err();
        assert!(error.contains("exit 3"));
        assert!(error.contains("FAIL expected 5, found 3"));
    }
}
