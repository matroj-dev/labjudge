use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use rand::RngCore;
use thiserror::Error;
use tokio::time::sleep;

use super::GoJudgeClient;
use crate::paths::Paths;
use crate::win::JobHandle;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("go-judge 不存在: {0}")]
    Missing(PathBuf),
    #[error("启动 go-judge 失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("go-judge 未就绪: {0}")]
    NotReady(String),
}

pub struct Supervisor {
    path: PathBuf,
    concurrency: usize,
    port: u16,
    token: String,
    child: Mutex<Child>,
    job: Mutex<Option<JobHandle>>,
    client: GoJudgeClient,
    version: String,
}

impl Supervisor {
    pub async fn start(paths: &Paths, concurrency: usize) -> Result<Self, StartupError> {
        if !paths.go_judge.is_file() {
            return Err(StartupError::Missing(paths.go_judge.clone()));
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let mut token_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token_bytes);
        let token = hex::encode(token_bytes);
        let client = GoJudgeClient::new(format!("http://127.0.0.1:{port}"), token.clone());

        let (child, job) = spawn_go_judge(&paths.go_judge, port, &token, concurrency)?;
        let version = wait_ready(&client).await?;

        Ok(Self {
            path: paths.go_judge.clone(),
            concurrency,
            port,
            token,
            child: Mutex::new(child),
            job: Mutex::new(Some(job)),
            client,
            version,
        })
    }

    pub fn client(&self) -> GoJudgeClient {
        self.client.clone()
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn is_alive(&self) -> bool {
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(child.try_wait(), Ok(None))
    }

    pub async fn restart(&self) -> Result<(), StartupError> {
        {
            let mut child = self
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }
        self.job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        let (child, job) = spawn_go_judge(&self.path, self.port, &self.token, self.concurrency)?;
        *self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = child;
        *self
            .job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(job);
        if let Err(error) = wait_ready(&self.client).await {
            let mut child = self
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = child.kill();
            let _ = child.wait();
            self.job
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_go_judge(
    path: &Path,
    port: u16,
    token: &str,
    concurrency: usize,
) -> Result<(Child, JobHandle), StartupError> {
    let mut command = Command::new(path);
    command
        .arg("-http-addr")
        .arg(format!("127.0.0.1:{port}"))
        .arg("-auth-token")
        .arg(token)
        .arg("-parallelism")
        .arg(concurrency.to_string())
        .arg("-output-limit")
        .arg("64m")
        .arg("-copy-out-limit")
        .arg("64m")
        .arg("-silent")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }

    let mut child = command.spawn()?;
    let job = JobHandle::new(None)?;
    if let Err(error) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(StartupError::Io(error));
    }
    Ok((child, job))
}

async fn wait_ready(client: &GoJudgeClient) -> Result<String, StartupError> {
    let mut last_error = String::new();
    for _ in 0..50 {
        match client.version().await {
            Ok(version) => return Ok(version),
            Err(error) => {
                last_error = error.to_string();
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(StartupError::NotReady(last_error))
}
