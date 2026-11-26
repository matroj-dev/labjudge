use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::model::Verdict;

#[derive(Debug, Clone)]
pub struct JudgeRunRequest {
    pub program: PathBuf,
    pub input_path: PathBuf,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
}

#[derive(Debug, Clone)]
pub struct JudgeRunResult {
    pub verdict: Verdict,
    pub stdout: Vec<u8>,
    pub time_ms: u64,
    pub memory_kb: u64,
    pub error: Option<String>,
}

/// checker 一次运行：`checker.exe <input> <output> <answer>`，与 testlib 协议一致。
#[derive(Debug, Clone)]
pub struct CheckerRunRequest {
    pub checker_path: PathBuf,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub answer_path: PathBuf,
    pub time_limit_ms: u32,
    pub memory_limit_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerRunResult {
    pub status: String,
    pub exit_status: i64,
    pub message: Option<String>,
}

/// checker 诊断只留第一条，截到 1 KiB，避免把 cases_json 撑大。
const CHECKER_MESSAGE_LIMIT: usize = 1024;

#[derive(Debug, Error)]
pub enum JudgeTransportError {
    #[error("请求 go-judge 失败: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("go-judge 返回 HTTP {0}: {1}")]
    Http(StatusCode, String),
    #[error("go-judge 返回空结果")]
    Empty,
    #[error("go-judge 响应解析失败: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("go-judge 结果缺少状态")]
    MissingStatus,
}

#[derive(Clone)]
pub struct GoJudgeClient {
    base_url: String,
    token: String,
    client: Client,
}

impl GoJudgeClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            client: Client::new(),
        }
    }

    pub async fn version(&self) -> Result<String, JudgeTransportError> {
        let response = self
            .client
            .get(format!("{}/version", self.base_url))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(2))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(JudgeTransportError::Http(
                status,
                response.text().await.unwrap_or_default(),
            ));
        }
        let value: Value = response.json().await?;
        Ok(value
            .get("buildVersion")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    pub async fn run_case(
        &self,
        request: &JudgeRunRequest,
    ) -> Result<JudgeRunResult, JudgeTransportError> {
        let program_name = request
            .program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(JudgeTransportError::MissingStatus)?
            .to_string();
        let clock_limit_ns = (u64::from(request.time_limit_ms) * 3 + 2000) * 1_000_000;

        let mut copy_in = serde_json::Map::new();
        copy_in.insert(
            program_name.clone(),
            json!({ "src": request.program.to_string_lossy().into_owned() }),
        );

        let body = json!({
            "requestId": format!("run-{}", request.time_limit_ms),
            "cmd": [{
                "args": [program_name],
                "env": ["PATH=C:\\Windows\\System32"],
                "files": [
                    { "src": request.input_path.to_string_lossy().into_owned() },
                    { "name": "stdout", "max": 67_108_864u64 },
                    { "name": "stderr", "max": 65_536u64 }
                ],
                "cpuLimit": u64::from(request.time_limit_ms) * 1_000_000,
                "clockLimit": clock_limit_ns,
                "memoryLimit": u64::from(request.memory_limit_mb) * 1024 * 1024,
                "procLimit": 1,
                "copyIn": serde_json::Value::Object(copy_in),
                "copyOut": ["stdout"],
                "copyOutMax": 67_108_864u64
            }]
        });

        let timeout = Duration::from_millis((u64::from(request.time_limit_ms) * 3 + 2000) + 5000);
        let response = self
            .client
            .post(format!("{}/run", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(timeout)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(JudgeTransportError::Http(
                status,
                response.text().await.unwrap_or_default(),
            ));
        }

        let value: Value = response.json().await?;
        let first = first_result(value).ok_or(JudgeTransportError::Empty)?;
        let raw: RawResult = serde_json::from_value(first)?;

        let mut stdout = raw
            .files
            .as_ref()
            .and_then(|files| files.get("stdout"))
            .cloned()
            .unwrap_or_default()
            .into_bytes();
        if stdout.len() > 67_108_864 {
            stdout.truncate(67_108_864);
        }

        let verdict = map_status(&raw.status);
        Ok(JudgeRunResult {
            verdict,
            stdout,
            time_ms: raw.time / 1_000_000,
            memory_kb: raw.memory / 1024,
            error: raw.error,
        })
    }

    pub async fn run_checker(
        &self,
        request: &CheckerRunRequest,
    ) -> Result<CheckerRunResult, JudgeTransportError> {
        let clock_limit_ns = (u64::from(request.time_limit_ms) * 3 + 2000) * 1_000_000;
        let mut copy_in = serde_json::Map::new();
        for (name, path) in [
            ("checker.exe", &request.checker_path),
            ("input.txt", &request.input_path),
            ("output.txt", &request.output_path),
            ("answer.txt", &request.answer_path),
        ] {
            copy_in.insert(
                name.to_string(),
                json!({ "src": path.to_string_lossy().into_owned() }),
            );
        }

        let body = json!({
            "requestId": "run-checker",
            "cmd": [{
                "args": ["checker.exe", "input.txt", "output.txt", "answer.txt"],
                "env": ["PATH=C:\\Windows\\System32"],
                // Windows 上 go-judge 要求 stdin/stdout/stderr 三个流都声明
                "files": [
                    { "content": "" },
                    { "name": "stdout", "max": 65_536u64 },
                    { "name": "stderr", "max": 65_536u64 }
                ],
                "cpuLimit": u64::from(request.time_limit_ms) * 1_000_000,
                "clockLimit": clock_limit_ns,
                "memoryLimit": u64::from(request.memory_limit_mb) * 1024 * 1024,
                "procLimit": 1,
                "copyIn": serde_json::Value::Object(copy_in),
                "copyOut": ["stderr"],
                "copyOutMax": 65_536u64
            }]
        });

        let timeout = Duration::from_millis((u64::from(request.time_limit_ms) * 3 + 2000) + 5000);
        let response = self
            .client
            .post(format!("{}/run", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(timeout)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(JudgeTransportError::Http(
                status,
                response.text().await.unwrap_or_default(),
            ));
        }

        let first = first_result(response.json().await?).ok_or(JudgeTransportError::Empty)?;
        let raw: RawResult = serde_json::from_value(first)?;
        let message = raw
            .files
            .as_ref()
            .and_then(|files| files.get("stderr"))
            .map(|text| truncate_message(text))
            .filter(|text| !text.is_empty());

        Ok(CheckerRunResult {
            status: raw.status,
            exit_status: raw.exit_status,
            message,
        })
    }
}

/// go-judge 的响应既可能是数组，也可能是 `{"results": [...]}`；统一取第一条。
fn first_result(value: Value) -> Option<Value> {
    match value {
        Value::Array(mut items) => items.drain(..).next(),
        Value::Object(mut object) => object
            .remove("results")
            .as_mut()
            .and_then(Value::as_array_mut)
            .and_then(|items| items.first().cloned()),
        _ => None,
    }
}

fn truncate_message(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= CHECKER_MESSAGE_LIMIT {
        return trimmed.to_string();
    }
    let mut end = CHECKER_MESSAGE_LIMIT;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &trimmed[..end])
}

#[derive(Debug, Deserialize)]
struct RawResult {
    status: String,
    #[serde(default, rename = "exitStatus")]
    exit_status: i64,
    #[serde(default)]
    time: u64,
    #[serde(default)]
    memory: u64,
    #[serde(default)]
    files: Option<HashMap<String, String>>,
    #[serde(default)]
    error: Option<String>,
}

fn map_status(status: &str) -> Verdict {
    match status {
        "Accepted" => Verdict::Ac,
        "Wrong Answer" => Verdict::Wa,
        "Time Limit Exceeded" => Verdict::Tle,
        "Memory Limit Exceeded" => Verdict::Mle,
        "Output Limit Exceeded" => Verdict::Ole,
        "Nonzero Exit Status" | "Signalled" | "Dangerous Syscall" => Verdict::Re,
        _ => Verdict::Ie,
    }
}

#[async_trait::async_trait]
impl super::CaseRunner for GoJudgeClient {
    async fn run_case(
        &self,
        request: &JudgeRunRequest,
    ) -> Result<JudgeRunResult, JudgeTransportError> {
        GoJudgeClient::run_case(self, request).await
    }

    async fn run_checker(
        &self,
        request: &CheckerRunRequest,
    ) -> Result<CheckerRunResult, JudgeTransportError> {
        GoJudgeClient::run_checker(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode as AxumStatus};
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn sends_expected_payload_and_parses_array_response() {
        let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let state = captured.clone();
        let app = Router::new()
            .route(
                "/run",
                post(
                    |State(captured): State<Arc<Mutex<Option<Value>>>>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        assert_eq!(
                            headers.get("authorization").unwrap().to_str().unwrap(),
                            "Bearer token"
                        );
                        *captured.lock().unwrap() = Some(body);
                        (
                            AxumStatus::OK,
                            Json(json!([{
                                "status": "Accepted",
                                "exitStatus": 0,
                                "time": 12_000_000u64,
                                "memory": 2_048_000u64,
                                "files": { "stdout": "7\r\n", "stderr": "" }
                            }])),
                        )
                    },
                ),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = GoJudgeClient::new(format!("http://{address}"), "token");
        let result = client
            .run_case(&JudgeRunRequest {
                program: PathBuf::from("C:/tmp/prog.exe"),
                input_path: PathBuf::from("C:/tmp/1.in"),
                time_limit_ms: 1000,
                memory_limit_mb: 256,
            })
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Ac);
        assert_eq!(result.stdout, b"7\r\n");
        assert_eq!(result.time_ms, 12);
        assert_eq!(result.memory_kb, 2000);

        let body = captured.lock().unwrap().clone().unwrap();
        let cmd = &body["cmd"].as_array().unwrap()[0];
        assert_eq!(cmd["cpuLimit"], 1_000_000_000u64);
        assert_eq!(cmd["clockLimit"], 5_000_000_000u64);
        assert_eq!(cmd["memoryLimit"], 268_435_456u64);
        assert_eq!(cmd["procLimit"], 1);
        assert_eq!(cmd["files"].as_array().unwrap()[0]["src"], "C:/tmp/1.in");
        assert_eq!(cmd["copyIn"]["prog.exe"]["src"], "C:/tmp/prog.exe");
        assert_eq!(cmd["copyOut"].as_array().unwrap()[0], "stdout");

        server.abort();
    }

    #[tokio::test]
    async fn checker_payload_uses_three_files_and_absolute_paths() {
        let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let state = captured.clone();
        let app =
            Router::new()
                .route(
                    "/run",
                    post(
                        |State(captured): State<Arc<Mutex<Option<Value>>>>,
                         Json(body): Json<Value>| async move {
                            *captured.lock().unwrap() = Some(body);
                            (
                                AxumStatus::OK,
                                Json(json!([{
                                    "status": "Nonzero Exit Status",
                                    "exitStatus": 1,
                                    "files": { "stderr": "wrong answer: expected 5, found 3\r\n" }
                                }])),
                            )
                        },
                    ),
                )
                .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = GoJudgeClient::new(format!("http://{address}"), "token");
        let result = client
            .run_checker(&CheckerRunRequest {
                checker_path: PathBuf::from("C:/data/checkers/c/A.exe"),
                input_path: PathBuf::from("C:/contest/problems/A/testdata/1.in"),
                output_path: PathBuf::from("C:/data/tmp/7/case-1.out"),
                answer_path: PathBuf::from("C:/contest/problems/A/testdata/1.out"),
                time_limit_ms: 5000,
                memory_limit_mb: 512,
            })
            .await
            .unwrap();
        assert_eq!(result.status, "Nonzero Exit Status");
        assert_eq!(result.exit_status, 1);
        assert_eq!(
            result.message.as_deref(),
            Some("wrong answer: expected 5, found 3")
        );

        let body = captured.lock().unwrap().clone().unwrap();
        let cmd = &body["cmd"].as_array().unwrap()[0];
        assert_eq!(
            cmd["args"].as_array().unwrap(),
            &vec![
                json!("checker.exe"),
                json!("input.txt"),
                json!("output.txt"),
                json!("answer.txt")
            ]
        );
        // Windows 上少声明 stdout 会被 go-judge 拒绝，这里锁定三个流都在。
        assert_eq!(cmd["files"].as_array().unwrap().len(), 3);
        assert_eq!(
            cmd["copyIn"]["checker.exe"]["src"],
            "C:/data/checkers/c/A.exe"
        );
        assert_eq!(
            cmd["copyIn"]["output.txt"]["src"],
            "C:/data/tmp/7/case-1.out"
        );
        assert_eq!(cmd["cpuLimit"], 5_000_000_000u64);
        assert_eq!(cmd["memoryLimit"], 536_870_912u64);
        assert_eq!(cmd["copyOut"].as_array().unwrap()[0], "stderr");

        server.abort();
    }

    #[test]
    fn checker_message_is_truncated_on_char_boundary() {
        let long = "错".repeat(1024);
        let truncated = truncate_message(&long);
        assert!(truncated.ends_with('…'));
        assert!(truncated.len() <= CHECKER_MESSAGE_LIMIT + '…'.len_utf8());
        assert_eq!(truncate_message("  ok  \r\n"), "ok");
    }

    #[test]
    fn status_mapping_covers_runner_codes() {
        assert_eq!(map_status("Accepted"), Verdict::Ac);
        assert_eq!(map_status("Time Limit Exceeded"), Verdict::Tle);
        assert_eq!(map_status("Memory Limit Exceeded"), Verdict::Mle);
        assert_eq!(map_status("Output Limit Exceeded"), Verdict::Ole);
        assert_eq!(map_status("Nonzero Exit Status"), Verdict::Re);
        assert_eq!(map_status("Signalled"), Verdict::Re);
        assert_eq!(map_status("File Error"), Verdict::Ie);
        assert_eq!(map_status("Internal Error"), Verdict::Ie);
    }
}
