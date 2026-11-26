use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::CompilerBackend;
use crate::win::run_limited;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(10);
const CHECKER_COMPILE_TIMEOUT: Duration = Duration::from_secs(30);
const COMPILE_MEMORY_LIMIT: u64 = 1024 * 1024 * 1024;
const COMPILE_OUTPUT_LIMIT: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct Compiler {
    path: PathBuf,
    version: String,
}

#[derive(Debug)]
pub enum CompileOutcome {
    Success(PathBuf),
    Ce { output: String },
}

#[derive(Debug, Error)]
pub enum CompilerSetupError {
    #[error("无法执行 `g++ --version`（{0}）；请安装 MinGW 并把其 bin 目录加入 PATH")]
    NotFound(String),
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("编译 IO 失败: {0}")]
    Io(#[from] io::Error),
}

impl Compiler {
    /// 只认 PATH 上的 `g++`：执行不了就视为目标机没有工具链。
    pub fn detect() -> Result<Self, CompilerSetupError> {
        let candidate = PathBuf::from("g++");
        match probe_gpp(&candidate) {
            Ok(version) => Ok(Self {
                path: candidate,
                version,
            }),
            Err(reason) => Err(CompilerSetupError::NotFound(reason)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn compile(&self, source: &str, work_dir: &Path) -> Result<CompileOutcome, CompileError> {
        fs::create_dir_all(work_dir)?;
        fs::write(work_dir.join("main.cpp"), source)?;
        let output_path = work_dir.join("prog.exe");
        let _ = fs::remove_file(&output_path);

        let mut command = Command::new(&self.path);
        command
            .current_dir(work_dir)
            .arg("-std=c++17")
            .arg("-O2")
            .arg("-pipe")
            .arg("-static")
            .arg("-static-libgcc")
            .arg("-static-libstdc++")
            .arg("-o")
            .arg("prog.exe")
            .arg("main.cpp");

        let output = run_limited(&mut command, COMPILE_TIMEOUT, Some(COMPILE_MEMORY_LIMIT))?;
        Ok(compile_result(output, &output_path, "编译超时（10 秒）"))
    }

    /// 编译 checker：固定单文件 `checker.cpp`，只把 testlib 目录加进 include 路径。
    pub fn compile_checker(
        &self,
        source: &Path,
        work_dir: &Path,
        out_name: &str,
        include_dir: &Path,
    ) -> Result<CompileOutcome, CompileError> {
        fs::create_dir_all(work_dir)?;
        // 用绝对路径，避免相对路径被 current_dir(work_dir) 改写。
        let source = fs::canonicalize(source)?;
        let output_path = work_dir.join(out_name);
        let _ = fs::remove_file(&output_path);

        // 直接把题目目录里的绝对路径交给 g++，不往产物目录复制源码。
        let mut command = Command::new(&self.path);
        command
            .current_dir(work_dir)
            .arg("-std=c++17")
            .arg("-O1")
            .arg("-pipe")
            .arg("-static")
            .arg("-static-libgcc")
            .arg("-static-libstdc++")
            .arg("-I")
            .arg(gpp_path(include_dir))
            .arg("-o")
            .arg(out_name)
            .arg(gpp_path(&source));

        let output = run_limited(
            &mut command,
            CHECKER_COMPILE_TIMEOUT,
            Some(COMPILE_MEMORY_LIMIT),
        )?;
        Ok(compile_result(output, &output_path, "编译超时（30 秒）"))
    }
}

/// 把一次编译进程的结果折算成「成功」或「带诊断的 CE」。
fn compile_result(
    output: crate::win::LimitedOutput,
    output_path: &Path,
    timeout_note: &str,
) -> CompileOutcome {
    if output.timed_out {
        return CompileOutcome::Ce {
            output: timeout_note.to_string(),
        };
    }
    let succeeded = output
        .status
        .map(|status| status.success())
        .unwrap_or(false);
    if succeeded && output_path.is_file() {
        return CompileOutcome::Success(output_path.to_path_buf());
    }
    let raw = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    CompileOutcome::Ce {
        output: truncate_output(raw),
    }
}

/// 探测 `g++`：必须能跑 `--version`，且目标平台是 Windows/MinGW
/// （排除 Cygwin/MSYS 的 g++，它们产出的 exe 依赖非系统 DLL，沙箱里跑不起来）。
fn probe_gpp(candidate: &Path) -> Result<String, String> {
    let output = Command::new(candidate)
        .arg("--version")
        .output()
        .map_err(|error| format!("无法执行: {error}"))?;
    if !output.status.success() {
        return Err(format!("--version 退出码 {:?}", output.status.code()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "--version 输出为空".to_string())?;

    if let Some(triple) = target_triple(candidate) {
        if !is_mingw_triple(&triple) {
            return Err(format!("目标平台不是 Windows/MinGW: {triple}"));
        }
    }
    Ok(version.to_string())
}

fn target_triple(candidate: &Path) -> Option<String> {
    let output = Command::new(candidate).arg("-dumpmachine").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let triple = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!triple.is_empty()).then_some(triple)
}

fn is_mingw_triple(triple: &str) -> bool {
    let lower = triple.to_ascii_lowercase();
    lower.contains("mingw") || lower.contains("windows")
}

#[async_trait]
impl CompilerBackend for Compiler {
    async fn compile_source(
        &self,
        source: &str,
        work_dir: &Path,
    ) -> Result<CompileOutcome, CompileError> {
        self.compile(source, work_dir)
    }
}

/// Windows 的 `canonicalize` 会返回 `\\?\C:\...`，MinGW g++ 不认这种形式；
/// 去掉前缀并换成正斜杠，路径里的空格仍作为单个参数传递。
fn gpp_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('\\', "/")
}

fn truncate_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= COMPILE_OUTPUT_LIMIT {
        return text.into_owned();
    }
    let mut end = COMPILE_OUTPUT_LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n（编译输出已截断）");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiler() -> Option<Compiler> {
        Compiler::detect().ok()
    }

    #[test]
    fn probe_rejects_missing_binary_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_gpp(&dir.path().join("g++.exe")).is_err());
        assert!(probe_gpp(dir.path()).is_err());
    }

    #[test]
    fn mingw_triple_filter_rejects_cygwin_and_msys() {
        assert!(is_mingw_triple("x86_64-w64-mingw32"));
        assert!(is_mingw_triple("x86_64-pc-windows-msvc"));
        assert!(!is_mingw_triple("x86_64-pc-cygwin"));
        assert!(!is_mingw_triple("x86_64-pc-msys"));
        assert!(!is_mingw_triple("x86_64-pc-linux-gnu"));
    }

    #[test]
    fn compiles_hello_and_reports_ce() {
        let Some(compiler) = compiler() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let outcome = compiler
            .compile(
                "#include <bits/stdc++.h>\nint main(){std::cout<<42;}\n",
                dir.path(),
            )
            .unwrap();
        match outcome {
            CompileOutcome::Success(path) => assert!(path.is_file()),
            CompileOutcome::Ce { output } => panic!("unexpected CE: {output}"),
        }

        let outcome = compiler.compile("int main( {", dir.path()).unwrap();
        match outcome {
            CompileOutcome::Ce { output } => assert!(!output.is_empty()),
            CompileOutcome::Success(_) => panic!("expected CE"),
        }
    }

    #[test]
    fn checker_compile_leaves_no_source_in_output_dir() {
        let Some(compiler) = compiler() else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("problem");
        let work_dir = root.path().join("checkers/contest");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("checker.cpp");
        fs::write(&source, "int main(){return 0;}\n").unwrap();

        let outcome = compiler
            .compile_checker(&source, &work_dir, "C.exe", &source_dir)
            .unwrap();
        match outcome {
            CompileOutcome::Success(path) => assert!(path.is_file()),
            CompileOutcome::Ce { output } => panic!("unexpected CE: {output}"),
        }
        assert!(!work_dir.join("checker.cpp").exists());
        assert!(source.is_file(), "题目目录里的源文件不能被搬走");
    }

    #[test]
    fn gpp_path_normalizes_windows_verbatim_paths() {
        assert_eq!(
            gpp_path(Path::new(r"\\?\C:\dir\checker.cpp")),
            "C:/dir/checker.cpp"
        );
        assert_eq!(gpp_path(Path::new("include")), "include");
    }

    #[test]
    fn truncation_keeps_utf8_boundary() {
        let mut text = "a".repeat(COMPILE_OUTPUT_LIMIT - 1);
        text.push('中');
        text.push('b');
        let truncated = truncate_output(text.as_bytes());
        assert!(truncated.ends_with("（编译输出已截断）"));
        assert!(truncated.starts_with(&"a".repeat(COMPILE_OUTPUT_LIMIT - 1)));
    }
}
