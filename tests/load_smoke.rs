use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use labjudge::app::build_router;
use labjudge::app_state::AppState;
use labjudge::config::Config;
use labjudge::contest::lifecycle::apply_start;
use labjudge::judge::{JudgeEngine, JudgeOutcome, JudgeOutcomeStatus, JudgeQueue, JudgeTask};
use labjudge::model::Verdict;
use labjudge::paths::Paths;
use labjudge::scoring::ScoredCase;
use labjudge::services::registry::{scan_contests, ContestRegistry};
use labjudge::timefmt::now_ms;
use tempfile::TempDir;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tower::ServiceExt;

const CLIENTS: usize = 60;

struct DelayEngine;

#[async_trait]
impl JudgeEngine for DelayEngine {
    async fn judge(&self, _task: JudgeTask) -> JudgeOutcome {
        sleep(Duration::from_millis(50)).await;
        JudgeOutcome {
            status: JudgeOutcomeStatus::Done,
            verdict: Verdict::Ac,
            score: 100,
            cases: vec![ScoredCase {
                name: "1".to_string(),
                subtask: "子任务 1".to_string(),
                verdict: Verdict::Ac,
                time_ms: 10,
                memory_kb: 1024,
                score: 100,
                max_score: 100,
            }],
            compile_output: None,
            error: None,
        }
    }
}

struct LoadApp {
    _root: TempDir,
    state: AppState,
    router: Router,
}

impl LoadApp {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let contest = root.path().join("contests/demo");
        write(
            &contest.join("contest.json"),
            r#"{"title":"压测赛","durationMinutes":60,"problems":[{"key":"A","dir":"problems/A"}]}"#,
        );
        let mut roster = String::from("name,student_id\n");
        for index in 1..=CLIENTS {
            roster.push_str(&format!("学生{index:02},S{index:04}\n"));
        }
        write(&contest.join("students.csv"), &roster);
        write(
            &contest.join("problems/A/problem.json"),
            r#"{"title":"两数之和","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}"#,
        );
        write(
            &contest.join("problems/A/statement.md"),
            "# A\n\n输入两个整数求和。",
        );
        write(&contest.join("problems/A/testdata/1.in"), "3 4\n");
        write(&contest.join("problems/A/testdata/1.out"), "7\n");

        let registry = ContestRegistry::load(
            scan_contests(&root.path().join("contests")),
            &root.path().join("data"),
        );
        let registry = Arc::new(registry);
        let queue = JudgeQueue::start(registry.clone(), Arc::new(DelayEngine), 2);
        let mut state = AppState::new(
            Arc::new(Config::for_test()),
            Arc::new(Paths::for_test(root.path().to_path_buf())),
            0,
            registry,
        )
        .unwrap();
        state.queue = Some(queue);
        let store = state.registry.get("demo").unwrap().store.clone();
        let mut contest_state = store.state().unwrap();
        apply_start(&mut contest_state, now_ms(), 60).unwrap();
        store.set_state(&contest_state).unwrap();
        let router = build_router(state.clone());
        Self {
            _root: root,
            state,
            router,
        }
    }

    async fn call(
        &self,
        method: &str,
        uri: &str,
        headers: HeaderMap,
        body: Option<String>,
    ) -> axum::response::Response {
        let mut builder = Request::builder().uri(uri).method(method);
        if !headers.contains_key(header::HOST) {
            builder = builder.header(header::HOST, "127.0.0.1:8080");
        }
        if method == "POST" {
            builder = builder.header(header::ORIGIN, "http://127.0.0.1:8080");
            builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        self.router
            .clone()
            .oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
            .await
            .unwrap()
    }
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
#[ignore]
async fn sixty_clients_join_and_submit_without_loss() {
    if std::env::var("LABJUDGE_LOADTEST").as_deref() != Ok("1") {
        return;
    }

    let app = Arc::new(LoadApp::new());
    let mut clients = JoinSet::new();
    for index in 1..=CLIENTS {
        let app = app.clone();
        clients.spawn(async move {
            let (csrf_cookie, anon_token) = app.state.csrf.issue_anon();
            let mut headers = HeaderMap::new();
            headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&format!("lc_csrf={csrf_cookie}")).unwrap(),
            );
            let body = format!(
                "student_id=S{index:04}&password=P{index:04}&csrf={}",
                anon_token
            );
            let response = app
                .call("POST", "/c/demo/join", headers, Some(body))
                .await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER);
            let set_cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string();
            let token = set_cookie.split_once('=').unwrap().1.to_string();
            let session_csrf = app.state.csrf.for_session(&token);

            let mut headers = HeaderMap::new();
            headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&set_cookie).unwrap(),
            );
            let source = urlencoding::encode(
                "#include <bits/stdc++.h>\nint main(){long long a,b;std::cin>>a>>b;std::cout<<a+b<<\"\\n\";}\n",
            );
            let body = format!("problem_key=A&source={source}&csrf={session_csrf}");
            let response = app
                .call("POST", "/c/demo/submissions", headers.clone(), Some(body))
                .await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER);

            for _ in 0..6 {
                let status = app.call("GET", "/c/demo/status", headers.clone(), None).await;
                assert_eq!(status.status(), StatusCode::OK);
                let mine = app.call("GET", "/c/demo/_mine", headers.clone(), None).await;
                assert_eq!(mine.status(), StatusCode::OK);
                let _ = body_text(mine).await;
                sleep(Duration::from_millis(40)).await;
            }
        });
    }

    let mut failures = Vec::new();
    while let Some(result) = clients.join_next().await {
        if let Err(error) = result {
            failures.push(error.to_string());
        }
    }
    assert!(failures.is_empty(), "client failures: {failures:?}");

    let store = app.state.registry.get("demo").unwrap().store.clone();
    for _ in 0..400 {
        let counts = store.status_counts().unwrap();
        if counts.done == CLIENTS && store.active_count().unwrap() == 0 {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    let counts = store.status_counts().unwrap();
    assert_eq!(counts.done, CLIENTS);
    assert_eq!(counts.failed, 0);
    assert_eq!(store.active_count().unwrap(), 0);
    assert_eq!(app.state.queue.as_ref().unwrap().depth(), 0);
    assert_eq!(store.submissions_for_contest().unwrap().len(), CLIENTS);
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}
