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
        let first = match value {
            Value::Array(mut items) => items.drain(..).next(),
            Value::Object(mut object) => {
                let mut results = object.remove("results");
                results
                    .as_mut()
                    .and_then(Value::as_array_mut)
                    .and_then(|items| items.first().cloned())
            }
            _ => None,
        }
        .ok_or(JudgeTransportError::Empty)?;
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
}

#[derive(Debug, Deserialize)]
struct RawResult {
    status: String,
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
