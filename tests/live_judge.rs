mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{form_body, TestApp};
use labjudge::contest::model::{SubtaskKind, TestCase, ValidatedProblem, ValidatedSubtask};
use labjudge::judge::{
    CaseRunner, Compiler, CompilerBackend, JudgeEngine, JudgeOutcome, JudgeTask, RealEngine,
    Supervisor,
};
use labjudge::model::{SubmissionStatus, Verdict};
use labjudge::paths::Paths;
use tokio::time::sleep;

fn go_judge_path() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("LABJUDGE_GO_JUDGE") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    // 相对仓库根定位，避免写死开发机的绝对路径。
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        root.join("vendor").join("go-judge").join("go-judge.exe"),
        root.join("dist")
            .join("labjudge")
            .join("bin")
            .join("go-judge.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn problem(
    root: &Path,
    name: &str,
    cases: &[(&str, &str, &str)],
    subtasks: Vec<ValidatedSubtask>,
    time_limit_ms: u32,
) -> ValidatedProblem {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("testdata")).unwrap();
    fs::write(dir.join("statement.md"), name).unwrap();
    let mut test_cases = Vec::new();
    for (case_name, input, output) in cases {
        let in_path = dir.join("testdata").join(format!("{case_name}.in"));
        let out_path = dir.join("testdata").join(format!("{case_name}.out"));
        fs::write(&in_path, input).unwrap();
        fs::write(&out_path, output).unwrap();
        test_cases.push(TestCase {
            name: (*case_name).to_string(),
            in_path,
            out_path,
        });
    }
    ValidatedProblem {
        key: "A".to_string(),
        title: name.to_string(),
        time_limit_ms,
        memory_limit_mb: 256,
        full_score: 100,
        root: dir.clone(),
        statement_path: dir.join("statement.md"),
        test_cases,
        subtasks,
    }
}

fn group(name: &str, score: i32, kind: SubtaskKind, cases: &[&str]) -> ValidatedSubtask {
    ValidatedSubtask {
        name: name.to_string(),
        score,
        kind,
        cases: cases.iter().map(|case| (*case).to_string()).collect(),
    }
}

async fn judge(
    engine: &RealEngine,
    root: &Path,
    problem: ValidatedProblem,
    source: &str,
) -> JudgeOutcome {
    let source_path = root.join(format!("{}.cpp", problem.title));
    fs::write(&source_path, source).unwrap();
    let work_dir = root.join(format!("work-{}", problem.title));
    engine
        .judge(JudgeTask {
            contest_id: "live".to_string(),
            submission_id: 1,
            problem,
            source_path,
            work_dir,
        })
        .await
}

#[tokio::test]
#[ignore]
async fn live_judge_ac_wa_tle_re_ce_and_subtasks() {
    if std::env::var("LABJUDGE_REAL_JUDGE").as_deref() != Ok("1") {
        return;
    }
    let Some(go_judge) = go_judge_path() else {
        eprintln!("skip live judge: g++ or go-judge missing");
        return;
    };
    let Ok(compiler) = Compiler::detect() else {
        eprintln!("skip live judge: g++ missing on PATH");
        return;
    };

    let root = tempfile::tempdir().unwrap();
    let mut paths = Paths::for_test(root.path().to_path_buf());
    paths.go_judge = go_judge;
    let compiler = Arc::new(compiler);
    let supervisor = Supervisor::start(&paths, 2).await.unwrap();
    let runner: Arc<dyn CaseRunner> = Arc::new(supervisor.client());
    let compiler_backend: Arc<dyn CompilerBackend> = compiler;
    let engine = RealEngine::new(compiler_backend, runner);

    let ac = problem(
        root.path(),
        "ac",
        &[("1", "3 4\n", "7\n")],
        vec![group("组", 100, SubtaskKind::Sum, &["1"])],
        1000,
    );
    let outcome = judge(
        &engine,
        root.path(),
        ac,
        "#include <bits/stdc++.h>\nint main(){long long a,b;std::cin>>a>>b;std::cout<<a+b<<\"\\n\";}\n",
    )
    .await;
    assert_eq!(outcome.verdict, Verdict::Ac);
    assert_eq!(outcome.score, 100);

    let wa = problem(
        root.path(),
        "wa",
        &[("1", "3 4\n", "7\n")],
        vec![group("组", 100, SubtaskKind::Sum, &["1"])],
        1000,
    );
    let outcome = judge(
        &engine,
        root.path(),
        wa,
        "#include <bits/stdc++.h>\nint main(){std::cout<<0<<\"\\n\";}\n",
    )
    .await;
    assert_eq!(outcome.verdict, Verdict::Wa);

    let tle = problem(
        root.path(),
        "tle",
        &[("1", "1\n", "1\n")],
        vec![group("组", 100, SubtaskKind::Sum, &["1"])],
        300,
    );
    let outcome = judge(
        &engine,
        root.path(),
        tle,
        "#include <bits/stdc++.h>\nint main(){volatile unsigned long long x=0;for(;;)x++;}\n",
    )
    .await;
    assert_eq!(outcome.verdict, Verdict::Tle);

    let re = problem(
        root.path(),
        "re",
        &[("1", "1\n", "1\n")],
        vec![group("组", 100, SubtaskKind::Sum, &["1"])],
        1000,
    );
    let outcome = judge(
        &engine,
        root.path(),
        re,
        "#include <bits/stdc++.h>\nint main(){return 3;}\n",
    )
    .await;
    assert_eq!(outcome.verdict, Verdict::Re);

    let ce = problem(
        root.path(),
        "ce",
        &[("1", "1\n", "1\n")],
        vec![group("组", 100, SubtaskKind::Sum, &["1"])],
        1000,
    );
    let outcome = judge(&engine, root.path(), ce, "int main( {").await;
    assert_eq!(outcome.verdict, Verdict::Ce);
    assert!(outcome.compile_output.unwrap().contains("error"));

    let subtasks = problem(
        root.path(),
        "subtasks",
        &[
            ("1", "1 1\n", "2\n"),
            ("2", "2 2\n", "4\n"),
            ("3", "3 3\n", "6\n"),
        ],
        vec![
            group("小数据", 30, SubtaskKind::Sum, &["1", "2"]),
            group("大数据", 70, SubtaskKind::Min, &["3"]),
        ],
        1000,
    );
    let outcome = judge(
        &engine,
        root.path(),
        subtasks,
        "#include <bits/stdc++.h>\nint main(){int a,b;std::cin>>a>>b;if(a==3)std::cout<<0<<\"\\n\";else std::cout<<a+b<<\"\\n\";}\n",
    )
    .await;
    assert_eq!(outcome.score, 30);
    assert_eq!(outcome.verdict, Verdict::Wa);

    let max_group = problem(
        root.path(),
        "maxgroup",
        &[("1", "1 1\n", "2\n"), ("2", "2 2\n", "4\n")],
        vec![group("任一点", 100, SubtaskKind::Max, &["1", "2"])],
        1000,
    );
    let outcome = judge(
        &engine,
        root.path(),
        max_group,
        "#include <bits/stdc++.h>\nint main(){int a,b;std::cin>>a>>b;if(a==2)std::cout<<0<<\"\\n\";else std::cout<<a+b<<\"\\n\";}\n",
    )
    .await;
    assert_eq!(outcome.score, 100);
}

#[tokio::test]
#[ignore]
async fn live_app_submit_reaches_ac_through_queue() {
    if std::env::var("LABJUDGE_REAL_JUDGE").as_deref() != Ok("1") {
        return;
    }
    let Some(go_judge) = go_judge_path() else {
        return;
    };
    let Ok(compiler) = Compiler::detect() else {
        return;
    };

    let mut app = TestApp::new();
    app.start_contest();
    let mut paths = Paths::for_test(app.state.paths.root.clone());
    paths.go_judge = go_judge;
    let compiler = Arc::new(compiler);
    let supervisor = Supervisor::start(&paths, 1).await.unwrap();
    let compiler_backend: Arc<dyn CompilerBackend> = compiler;
    let runner: Arc<dyn CaseRunner> = Arc::new(supervisor.client());
    let engine: Arc<dyn JudgeEngine> = Arc::new(RealEngine::new(compiler_backend, runner));
    app = app.with_fake_engine(engine);

    let (headers, csrf) = app.student_auth("2026001");
    let response = app
        .post(
            "/c/demo/submissions",
            headers,
            form_body(&[
                (
                    "problem_key",
                    "A",
                ),
                (
                    "source",
                    "#include <bits/stdc++.h>\nint main(){long long a,b;std::cin>>a>>b;std::cout<<a+b<<\"\\n\";}\n",
                ),
                ("csrf", &csrf),
            ]),
        )
        .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let id = app
        .store()
        .submissions_for_student("2026001")
        .unwrap()
        .first()
        .unwrap()
        .id;
    let mut row = None;
    for _ in 0..300 {
        let current = app.store().submission(id).unwrap().unwrap();
        if matches!(
            current.status,
            SubmissionStatus::Done | SubmissionStatus::Failed
        ) {
            row = Some(current);
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let row = row.expect("submission finished");
    assert_eq!(row.status, SubmissionStatus::Done);
    assert_eq!(row.verdict, Some(Verdict::Ac));
    assert_eq!(row.score, 100);
}
