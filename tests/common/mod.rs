#![allow(dead_code)]

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, HeaderValue, Request};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use labjudge::app::build_router;
use labjudge::app_state::AppState;
use labjudge::config::Config;
use labjudge::contest::lifecycle::{apply_end, apply_start};
use labjudge::judge::{JudgeEngine, JudgeQueue};
use labjudge::model::Verdict;
use labjudge::paths::Paths;
use labjudge::services::registry::{scan_contests, ContestRegistry};
use labjudge::store::{ContestStore, NewSubmission, SubmissionResult};
use labjudge::timefmt::now_ms;
use tempfile::TempDir;
use tower::ServiceExt;

pub struct TestApp {
    _root: TempDir,
    pub state: AppState,
    pub router: Router,
    pub contest_id: String,
}

impl TestApp {
    pub fn new() -> Self {
        Self::build(Config::for_test())
    }

    /// 默认配置有 5 秒提交冷却；需要连着提交同一题时用这个。
    pub fn without_submit_cooldown() -> Self {
        let mut config = Config::for_test();
        config.submit_cooldown_seconds = 0;
        Self::build(config)
    }

    fn build(config: Config) -> Self {
        let root = tempfile::tempdir().unwrap();
        let contests = root.path().join("contests");
        let demo = contests.join("demo");
        write(
            &demo.join("contest.json"),
            r#"{"title":"M2 测试赛","durationMinutes":60,"leaderboardVisible":false}"#,
        );
        write(
            &demo.join("students.csv"),
            "name,student_id,password,school,grade\n张三,2026001,p1,第一中学,七年级\n张三,2026002,p2,第二中学,八年级\n李四,2026003,p3,,,\n",
        );
        write(
            &demo.join("problems/A/problem.json"),
            r#"{"title":"两数之和","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}"#,
        );
        write(
            &demo.join("problems/A/statement.md"),
            "# A 题\n\n公式 $a+b$。\n\n<script>alert(1)</script>\n\n![图](images/example.png)\n",
        );
        write(&demo.join("problems/A/testdata/1.in"), "3 4");
        write(&demo.join("problems/A/testdata/1.out"), "7");
        fs::create_dir_all(demo.join("problems/A/images")).unwrap();
        fs::write(
            demo.join("problems/A/images/example.png"),
            [0x89, b'P', b'N', b'G', 0, 1, 2, 3],
        )
        .unwrap();
        fs::write(demo.join("problems/A/images/bad.svg"), b"<svg></svg>").unwrap();

        write(
            &demo.join("problems/B/problem.json"),
            r#"{"title":"数列","timeLimitMs":2000,"memoryLimitMb":128,"fullScore":100}"#,
        );
        write(
            &demo.join("problems/B/statement.md"),
            "# B 题\n\n普通题面。\n",
        );
        write(&demo.join("problems/B/testdata/1.in"), "1");
        write(&demo.join("problems/B/testdata/1.out"), "1");

        fs::create_dir_all(contests.join("bad id")).unwrap();

        let registry = ContestRegistry::load(scan_contests(&contests), &root.path().join("data"));
        let paths = Arc::new(Paths::for_test(root.path().to_path_buf()));
        let state = AppState::new(Arc::new(config), paths, 0, Arc::new(registry)).unwrap();
        let router = build_router(state.clone());

        Self {
            _root: root,
            state,
            router,
            contest_id: "demo".to_string(),
        }
    }

    pub fn store(&self) -> Arc<ContestStore> {
        self.state
            .registry
            .get(&self.contest_id)
            .expect("demo contest")
            .store
            .clone()
    }

    pub fn contest_dir(&self) -> std::path::PathBuf {
        self.state
            .paths
            .root
            .join("contests")
            .join(&self.contest_id)
    }

    pub async fn get(&self, uri: &str, headers: HeaderMap) -> Response {
        let mut builder = Request::builder().uri(uri).method("GET");
        if !headers.contains_key(header::HOST) {
            builder = builder.header(header::HOST, "127.0.0.1:8080");
        }
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        self.router.clone().oneshot(request).await.unwrap()
    }

    pub async fn post(&self, uri: &str, headers: HeaderMap, body: String) -> Response {
        let mut builder = Request::builder().uri(uri).method("POST");
        if !headers.contains_key(header::HOST) {
            builder = builder.header(header::HOST, "127.0.0.1:8080");
        }
        if !headers.contains_key(header::ORIGIN) {
            builder = builder.header(header::ORIGIN, "http://127.0.0.1:8080");
        }
        builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(Body::from(body)).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        self.router.clone().oneshot(request).await.unwrap()
    }

    pub fn student_token(&self, student_id: &str) -> String {
        let runtime = self.state.registry.get(&self.contest_id).unwrap();
        if let Some(student) = runtime
            .package
            .roster
            .students()
            .iter()
            .find(|student| student.student_id == student_id)
        {
            runtime.store.upsert_participant(student, now_ms()).unwrap();
        }
        let session_id = labjudge::auth::session::new_session_id();
        runtime
            .store
            .upsert_session(student_id, &session_id, now_ms())
            .unwrap();
        self.state
            .session
            .sign_student(&self.contest_id, student_id, &session_id, now_ms())
    }

    pub fn student_headers(&self, student_id: &str) -> HeaderMap {
        self.student_auth(student_id).0
    }

    pub fn student_auth(&self, student_id: &str) -> (HeaderMap, String) {
        let token = self.student_token(student_id);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("lc_s_{}={token}", self.contest_id)).unwrap(),
        );
        let csrf = self.state.csrf.for_session(&token);
        (headers, csrf)
    }

    pub fn student_csrf(&self, student_id: &str) -> String {
        self.student_auth(student_id).1
    }

    pub fn admin_auth(&self) -> (HeaderMap, String) {
        let token = self.state.session.sign_admin(now_ms());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("lc_admin={token}")).unwrap(),
        );
        let csrf = self.state.csrf.for_session(&token);
        (headers, csrf)
    }

    pub fn anon_csrf(&self) -> (String, String) {
        self.state.csrf.issue_anon()
    }

    pub fn start_contest(&self) {
        let store = self.store();
        let mut state = store.state().unwrap();
        let _ = apply_start(&mut state, now_ms(), 60);
        store.set_state(&state).unwrap();
    }

    pub fn end_contest(&self) {
        let store = self.store();
        let mut state = store.state().unwrap();
        let _ = apply_end(&mut state, now_ms());
        store.set_state(&state).unwrap();
    }

    pub fn expire_contest(&self) {
        let store = self.store();
        let mut state = store.state().unwrap();
        let now = now_ms();
        state.started_at = Some(now - 60_000);
        state.ends_at = Some(now - 1_000);
        state.ended_early = false;
        store.set_state(&state).unwrap();
    }

    pub fn set_leaderboard_visible(&self, visible: bool) {
        self.store().set_leaderboard_visible(visible).unwrap();
    }

    pub fn with_fake_engine(mut self, engine: Arc<dyn JudgeEngine>) -> Self {
        let queue = JudgeQueue::start(self.state.registry.clone(), engine, 1);
        self.state.queue = Some(queue);
        self.router = build_router(self.state.clone());
        self
    }

    pub fn add_queued_submission(&self, student_id: &str, problem_key: &str) -> i64 {
        self.store()
            .insert_submission(&NewSubmission {
                student_id: student_id.to_string(),
                problem_key: problem_key.to_string(),
                language: "cpp17".to_string(),
                source_path: Path::new("C:/data/submission.cpp").to_path_buf(),
                created_at: now_ms(),
            })
            .unwrap()
            .id
    }

    pub fn add_done_submission(
        &self,
        student_id: &str,
        problem_key: &str,
        verdict: Verdict,
        score: i32,
        compile_output: Option<&str>,
    ) -> i64 {
        let store = self.store();
        let id = self.add_queued_submission(student_id, problem_key);
        let cases = r#"[{"name":"1","subtask":"小数据","verdict":"AC","time_ms":12,"memory_kb":1024,"score":50,"max_score":50}]"#;
        store
            .set_result(
                id,
                &SubmissionResult {
                    verdict,
                    score,
                    cases_json: Some(cases),
                    compile_output,
                    error: None,
                    judged_at: now_ms(),
                },
            )
            .unwrap();
        id
    }
}

pub async fn body_string(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn header_value(response: &Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

pub fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap_or_default().to_string()
}

pub fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

pub fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                urlencoding::encode(name),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
