use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::checker::checker_verdict;
use super::{CaseRunner, CheckerRunRequest, CompileOutcome, CompilerBackend, JudgeRunRequest};
use crate::compare::outputs_equal;
use crate::contest::model::ValidatedProblem;
use crate::model::Verdict;
use crate::scoring::{score_problem, RawCase, ScoredCase};

#[derive(Debug, Clone)]
pub struct JudgeTask {
    pub contest_id: String,
    pub submission_id: i64,
    pub problem: ValidatedProblem,
    pub source_path: PathBuf,
    pub work_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeOutcomeStatus {
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct JudgeOutcome {
    pub status: JudgeOutcomeStatus,
    pub verdict: Verdict,
    pub score: i32,
    pub cases: Vec<ScoredCase>,
    pub compile_output: Option<String>,
    pub compile_ms: u64,
    pub error: Option<String>,
}

impl JudgeOutcome {
    fn failed(error: impl Into<String>) -> Self {
        Self {
            status: JudgeOutcomeStatus::Failed,
            verdict: Verdict::Ie,
            score: 0,
            cases: Vec::new(),
            compile_output: None,
            compile_ms: 0,
            error: Some(error.into()),
        }
    }
}

#[async_trait]
pub trait JudgeEngine: Send + Sync {
    async fn judge(&self, task: JudgeTask) -> JudgeOutcome;
}

pub struct RealEngine {
    compiler: Arc<dyn CompilerBackend>,
    runner: Arc<dyn CaseRunner>,
    checker_dir: PathBuf,
}

impl RealEngine {
    pub fn new(
        compiler: Arc<dyn CompilerBackend>,
        runner: Arc<dyn CaseRunner>,
        checker_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            compiler,
            runner,
            checker_dir: checker_dir.into(),
        }
    }

    /// checker 产物路径由「比赛 ID + 题号」唯一决定，启动时编译好后直接取用。
    fn checker_exe(&self, contest_id: &str, problem_key: &str) -> PathBuf {
        self.checker_dir
            .join(contest_id)
            .join(format!("{problem_key}.exe"))
    }

    async fn judge_inner(&self, task: &JudgeTask) -> JudgeOutcome {
        let source = match fs::read_to_string(&task.source_path) {
            Ok(source) => source,
            Err(error) => return JudgeOutcome::failed(format!("读取源码失败: {error}")),
        };

        let compile_started = std::time::Instant::now();
        let compiled = self.compiler.compile_source(&source, &task.work_dir).await;
        let compile_ms = compile_started.elapsed().as_millis() as u64;
        let program = match compiled {
            Ok(CompileOutcome::Success(program)) => program,
            Ok(CompileOutcome::Ce { output }) => {
                return JudgeOutcome {
                    status: JudgeOutcomeStatus::Done,
                    verdict: Verdict::Ce,
                    score: 0,
                    cases: Vec::new(),
                    compile_output: Some(output),
                    compile_ms,
                    error: None,
                }
            }
            Err(error) => return JudgeOutcome::failed(format!("编译环境错误: {error}")),
        };

        let mut raw_cases = Vec::with_capacity(task.problem.test_cases.len());
        for test_case in &task.problem.test_cases {
            let result = match self
                .runner
                .run_case(&JudgeRunRequest {
                    program: program.clone(),
                    input_path: test_case.in_path.clone(),
                    time_limit_ms: task.problem.time_limit_ms,
                    memory_limit_mb: task.problem.memory_limit_mb,
                })
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    return JudgeOutcome::failed(format!(
                        "测试点 {} 运行失败: {error}",
                        test_case.name
                    ))
                }
            };

            let expected = match fs::read(&test_case.out_path) {
                Ok(expected) => expected,
                Err(error) => {
                    return JudgeOutcome::failed(format!(
                        "读取测试点 {} 期望输出失败: {error}",
                        test_case.name
                    ))
                }
            };
            let mut message = None;
            let verdict = if result.verdict == Verdict::Ac {
                match &task.problem.checker {
                    Some(checker) => {
                        let output_path =
                            task.work_dir.join(format!("case-{}.out", test_case.name));
                        if let Err(error) = fs::create_dir_all(&task.work_dir)
                            .and_then(|()| fs::write(&output_path, &result.stdout))
                        {
                            return JudgeOutcome::failed(format!(
                                "写入测试点 {} 的选手输出失败: {error}",
                                test_case.name
                            ));
                        }
                        let run = match self
                            .runner
                            .run_checker(&CheckerRunRequest {
                                checker_path: self.checker_exe(&task.contest_id, &task.problem.key),
                                input_path: test_case.in_path.clone(),
                                output_path,
                                answer_path: test_case.out_path.clone(),
                                time_limit_ms: checker.time_limit_ms,
                                memory_limit_mb: checker.memory_limit_mb,
                            })
                            .await
                        {
                            Ok(run) => run,
                            Err(error) => {
                                return JudgeOutcome::failed(format!(
                                    "测试点 {} 运行 checker 失败: {error}",
                                    test_case.name
                                ))
                            }
                        };
                        match checker_verdict(&run) {
                            Ok(verdict) => {
                                message = run.message.clone();
                                verdict
                            }
                            Err(reason) => {
                                return JudgeOutcome::failed(format!(
                                    "测试点 {} 判题失败：{reason}",
                                    test_case.name
                                ))
                            }
                        }
                    }
                    None => {
                        if outputs_equal(&expected, &result.stdout) {
                            Verdict::Ac
                        } else {
                            Verdict::Wa
                        }
                    }
                }
            } else {
                result.verdict
            };

            raw_cases.push(RawCase {
                name: test_case.name.clone(),
                verdict,
                time_ms: result.time_ms,
                memory_kb: result.memory_kb,
                message,
            });
        }

        let scored = match score_problem(&task.problem, &raw_cases) {
            Ok(scored) => scored,
            Err(error) => return JudgeOutcome::failed(format!("计分失败: {error}")),
        };

        JudgeOutcome {
            status: JudgeOutcomeStatus::Done,
            verdict: scored.verdict,
            score: scored.total,
            cases: scored.cases,
            compile_output: None,
            compile_ms,
            error: None,
        }
    }
}

#[async_trait]
impl JudgeEngine for RealEngine {
    async fn judge(&self, task: JudgeTask) -> JudgeOutcome {
        let outcome = self.judge_inner(&task).await;
        let _ = fs::remove_dir_all(&task.work_dir);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{Checker, SubtaskKind, TestCase, ValidatedSubtask};
    use crate::judge::{CheckerRunResult, CompileError, JudgeRunResult, JudgeTransportError};
    use std::path::Path;
    use std::sync::Mutex;

    struct FakeCompiler {
        result: Mutex<Option<CompileOutcome>>,
    }

    #[async_trait]
    impl CompilerBackend for FakeCompiler {
        async fn compile_source(
            &self,
            _source: &str,
            work_dir: &Path,
        ) -> Result<CompileOutcome, CompileError> {
            let _ = work_dir;
            Ok(self
                .result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(CompileOutcome::Success(PathBuf::from("prog.exe"))))
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        results: Mutex<Vec<Result<JudgeRunResult, JudgeTransportError>>>,
        checker_results: Mutex<Vec<Result<CheckerRunResult, JudgeTransportError>>>,
    }

    #[async_trait]
    impl CaseRunner for FakeRunner {
        async fn run_case(
            &self,
            _request: &JudgeRunRequest,
        ) -> Result<JudgeRunResult, JudgeTransportError> {
            self.results.lock().unwrap().remove(0)
        }

        async fn run_checker(
            &self,
            _request: &CheckerRunRequest,
        ) -> Result<CheckerRunResult, JudgeTransportError> {
            let mut queue = self.checker_results.lock().unwrap();
            assert!(!queue.is_empty(), "checker 不应被调用");
            queue.remove(0)
        }
    }

    fn problem(dir: &Path, with_checker: bool) -> ValidatedProblem {
        let mut cases = Vec::new();
        for (name, output) in [("1", "7"), ("2", "10")] {
            let in_path = dir.join(format!("{name}.in"));
            let out_path = dir.join(format!("{name}.out"));
            fs::write(&in_path, "in").unwrap();
            fs::write(&out_path, output).unwrap();
            cases.push(TestCase {
                name: name.to_string(),
                in_path,
                out_path,
            });
        }
        ValidatedProblem {
            key: "A".to_string(),
            title: "A".to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score: 100,
            root: dir.to_path_buf(),
            statement_path: dir.join("statement.md"),
            test_cases: cases,
            subtasks: vec![ValidatedSubtask {
                name: "子任务 1".to_string(),
                score: 100,
                kind: SubtaskKind::Sum,
                cases: vec!["1".to_string(), "2".to_string()],
            }],
            checker: with_checker.then(|| Checker {
                source: dir.join("checker.cpp"),
                time_limit_ms: 5000,
                memory_limit_mb: 512,
            }),
        }
    }

    fn run_result(verdict: Verdict, stdout: &str) -> JudgeRunResult {
        JudgeRunResult {
            verdict,
            stdout: stdout.as_bytes().to_vec(),
            time_ms: 10,
            memory_kb: 1024,
            error: None,
        }
    }

    fn checker_result(status: &str, exit_status: i64, message: Option<&str>) -> CheckerRunResult {
        CheckerRunResult {
            status: status.to_string(),
            exit_status,
            message: message.map(str::to_string),
        }
    }

    fn task(dir: &Path, with_checker: bool) -> JudgeTask {
        let source_path = dir.join("main.cpp");
        fs::write(&source_path, "int main(){}").unwrap();
        JudgeTask {
            contest_id: "c".to_string(),
            submission_id: 1,
            problem: problem(dir, with_checker),
            source_path,
            work_dir: dir.join("work"),
        }
    }

    fn engine(dir: &Path, runner: FakeRunner) -> RealEngine {
        RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(None),
            }),
            Arc::new(runner),
            dir.join("checkers"),
        )
    }

    fn runner(
        runs: Vec<Result<JudgeRunResult, JudgeTransportError>>,
        checkers: Vec<Result<CheckerRunResult, JudgeTransportError>>,
    ) -> FakeRunner {
        FakeRunner {
            results: Mutex::new(runs),
            checker_results: Mutex::new(checkers),
        }
    }

    #[tokio::test]
    async fn ac_wa_and_tle_are_derived_from_runner_output() {
        let dir = tempfile::tempdir().unwrap();
        let judge = engine(
            dir.path(),
            runner(
                vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Ac, "wrong\n")),
                ],
                vec![],
            ),
        );
        let outcome = judge.judge(task(dir.path(), false)).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Done);
        assert_eq!(outcome.verdict, Verdict::Wa);
        assert_eq!(outcome.score, 50);
        assert_eq!(outcome.cases.len(), 2);
        assert!(outcome.cases.iter().all(|case| case.message.is_none()));

        let judge = engine(
            dir.path(),
            runner(
                vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Tle, "")),
                ],
                vec![],
            ),
        );
        let outcome = judge.judge(task(dir.path(), false)).await;
        assert_eq!(outcome.verdict, Verdict::Tle);
    }

    #[tokio::test]
    async fn checker_decides_instead_of_string_equality() {
        let dir = tempfile::tempdir().unwrap();
        // 第二点输出与 .out 不同，但 checker 判通过；说明比对确实交给了 checker。
        let judge = engine(
            dir.path(),
            runner(
                vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Ac, "10 10\n")),
                ],
                vec![
                    Ok(checker_result("Accepted", 0, None)),
                    Ok(checker_result("Accepted", 0, None)),
                ],
            ),
        );
        let outcome = judge.judge(task(dir.path(), true)).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Done);
        assert_eq!(outcome.verdict, Verdict::Ac);
        assert_eq!(outcome.score, 100);

        // checker 判 WA 时带上 stderr 诊断。
        let judge = engine(
            dir.path(),
            runner(
                vec![
                    Ok(run_result(Verdict::Ac, "7\n")),
                    Ok(run_result(Verdict::Ac, "10 10\n")),
                ],
                vec![
                    Ok(checker_result(
                        "Nonzero Exit Status",
                        1,
                        Some("wrong answer 1st numbers differ"),
                    )),
                    Ok(checker_result("Accepted", 0, None)),
                ],
            ),
        );
        let outcome = judge.judge(task(dir.path(), true)).await;
        assert_eq!(outcome.verdict, Verdict::Wa);
        assert_eq!(outcome.score, 50);
        assert_eq!(
            outcome.cases[0].message.as_deref(),
            Some("wrong answer 1st numbers differ")
        );
    }

    #[tokio::test]
    async fn checker_fault_marks_submission_failed() {
        for code in [3, 5, 20] {
            let dir = tempfile::tempdir().unwrap();
            let judge = engine(
                dir.path(),
                runner(
                    vec![Ok(run_result(Verdict::Ac, "7\n"))],
                    vec![Ok(checker_result(
                        "Nonzero Exit Status",
                        code,
                        Some("boom"),
                    ))],
                ),
            );
            let outcome = judge.judge(task(dir.path(), true)).await;
            assert_eq!(outcome.status, JudgeOutcomeStatus::Failed, "exit {code}");
            assert!(outcome.cases.is_empty());
            assert!(outcome.error.unwrap().contains("判题失败"));
        }
    }

    #[tokio::test]
    async fn checker_is_skipped_when_the_program_does_not_pass() {
        let dir = tempfile::tempdir().unwrap();
        // checker 队列为空：只要被调用就会 panic。
        let judge = engine(
            dir.path(),
            runner(
                vec![
                    Ok(run_result(Verdict::Tle, "")),
                    Ok(run_result(Verdict::Tle, "")),
                ],
                vec![],
            ),
        );
        let outcome = judge.judge(task(dir.path(), true)).await;
        assert_eq!(outcome.verdict, Verdict::Tle);
    }

    #[tokio::test]
    async fn ce_and_transport_failures_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let judge = RealEngine::new(
            Arc::new(FakeCompiler {
                result: Mutex::new(Some(CompileOutcome::Ce {
                    output: "syntax error".to_string(),
                })),
            }),
            Arc::new(runner(vec![], vec![])),
            dir.path().join("checkers"),
        );
        let outcome = judge.judge(task(dir.path(), false)).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Done);
        assert_eq!(outcome.verdict, Verdict::Ce);
        assert_eq!(outcome.compile_output.as_deref(), Some("syntax error"));

        let judge = engine(
            dir.path(),
            runner(vec![Err(JudgeTransportError::Empty)], vec![]),
        );
        let outcome = judge.judge(task(dir.path(), false)).await;
        assert_eq!(outcome.status, JudgeOutcomeStatus::Failed);
        assert_eq!(outcome.verdict, Verdict::Ie);
        assert!(outcome.error.unwrap().contains("运行失败"));
    }
}
