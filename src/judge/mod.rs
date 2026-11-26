use std::path::Path;

use async_trait::async_trait;

pub mod compile;
pub mod engine;
pub mod go_judge;
pub mod queue;
pub mod supervisor;

pub use compile::{CompileError, CompileOutcome, Compiler, CompilerSetupError};
pub use engine::{JudgeEngine, JudgeOutcome, JudgeOutcomeStatus, JudgeTask, RealEngine};
pub use go_judge::{GoJudgeClient, JudgeRunRequest, JudgeRunResult, JudgeTransportError};
pub use queue::{JudgeJob, JudgeQueue, JudgeQueueHandle};
pub use supervisor::{StartupError, Supervisor};

#[async_trait]
pub trait CaseRunner: Send + Sync {
    async fn run_case(
        &self,
        request: &JudgeRunRequest,
    ) -> Result<JudgeRunResult, JudgeTransportError>;
}

#[async_trait]
pub trait CompilerBackend: Send + Sync {
    async fn compile_source(
        &self,
        source: &str,
        work_dir: &Path,
    ) -> Result<CompileOutcome, CompileError>;
}
