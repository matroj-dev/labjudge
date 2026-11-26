use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub contests: PathBuf,
    pub data: PathBuf,
    pub go_judge: PathBuf,
    pub testlib: PathBuf,
    pub config_file: PathBuf,
}

#[derive(Debug, Error)]
pub enum PathError {
    #[error("路径初始化失败: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum DataLockError {
    #[error("数据目录不可写: {0}")]
    Io(#[from] std::io::Error),
    #[error("程序已在运行：无法锁定 {0}")]
    Locked(PathBuf),
}

pub struct DataLock {
    _file: File,
}

const LOCK_FILE_NAME: &str = "labjudge.lock";
const LOCK_NOTE: &str = "\
# 本文件由 MatrOJ 局域网比赛系统创建，用来防止同一个数据目录被两个进程同时打开。
# 程序运行时独占它；正常退出后可以删除，删除不影响任何数据。
";

impl DataLock {
    pub fn acquire(data_dir: &Path) -> Result<Self, DataLockError> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(LOCK_FILE_NAME);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if FileExt::try_lock_exclusive(&file).is_err() {
            return Err(DataLockError::Locked(path));
        }
        // 空文件才写说明，避免每次启动都重写。
        if file.metadata().map(|meta| meta.len()).unwrap_or(0) == 0 {
            let _ = std::io::Write::write_all(&mut file, LOCK_NOTE.as_bytes());
            let _ = std::io::Write::flush(&mut file);
        }
        Ok(Self { _file: file })
    }
}

impl Paths {
    pub fn discover() -> Result<Self, PathError> {
        let exe = std::env::current_exe()?;
        let cwd = std::env::current_dir()?;
        let env_root = std::env::var_os("LABJUDGE_ROOT").map(PathBuf::from);
        let root = resolve_root(env_root, &cwd, &exe);

        let go_judge = resolve_go_judge(
            std::env::var_os("LABJUDGE_GO_JUDGE").map(PathBuf::from),
            &root,
        );
        let testlib = resolve_testlib(
            std::env::var_os("LABJUDGE_TESTLIB").map(PathBuf::from),
            &root,
        );

        Ok(Self {
            data: root.join("data"),
            contests: root.join("contests"),
            config_file: crate::config::config_path(&root),
            go_judge,
            testlib,
            root,
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(&self.data)?;
        fs::create_dir_all(&self.contests)?;
        Ok(())
    }

    pub fn for_test(root: PathBuf) -> Self {
        Self {
            data: root.join("data"),
            contests: root.join("contests"),
            config_file: crate::config::config_path(&root),
            go_judge: root.join("go-judge.exe"),
            testlib: root.join("vendor").join("testlib").join("testlib.h"),
            root,
        }
    }

    /// checker 编译产物统一放这里：与 `data/<比赛ID>/` 分开，
    /// 免得「重置比赛」清空数据目录时连带删掉 checker。
    pub fn checkers_dir(&self) -> PathBuf {
        self.data.join("checkers")
    }
}

pub fn resolve_root(env_root: Option<PathBuf>, cwd: &Path, exe: &Path) -> PathBuf {
    if let Some(root) = env_root {
        return root;
    }
    if cwd.join("Cargo.toml").is_file() {
        return cwd.to_path_buf();
    }
    let exe_dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.to_path_buf());
    // 发布包把可执行文件放在 bin\ 下，根目录（config.json、data、contests 所在）是上一级。
    if exe_dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
        if let Some(root) = exe_dir.parent() {
            return root.to_path_buf();
        }
    }
    exe_dir
}

pub fn resolve_go_judge(env_path: Option<PathBuf>, root: &Path) -> PathBuf {
    if let Some(path) = env_path {
        return path;
    }
    let in_bin = root.join("bin").join("go-judge.exe");
    if in_bin.is_file() {
        return in_bin;
    }
    root.join("vendor").join("go-judge").join("go-judge.exe")
}

/// 发布包放 `include/testlib.h`（与 `bin/` 同级），开发态用 `vendor/testlib/testlib.h`。
pub fn resolve_testlib(env_path: Option<PathBuf>, root: &Path) -> PathBuf {
    if let Some(path) = env_path {
        return path;
    }
    let in_include = root.join("include").join("testlib.h");
    if in_include.is_file() {
        return in_include;
    }
    root.join("vendor").join("testlib").join("testlib.h")
}

pub fn candidate_ports(start: u16) -> Vec<u16> {
    (0..=10u16)
        .filter_map(|offset| start.checked_add(offset))
        .collect()
}

/// 提交源码的目录：`<data>/<比赛>/submissions/<账号>/<题号>/`。
pub fn submission_dir(data_dir: &Path, student_id: &str, problem_key: &str) -> PathBuf {
    data_dir
        .join("submissions")
        .join(path_component(student_id))
        .join(path_component(problem_key))
}

/// 提交源码：`<data>/<比赛>/submissions/<账号>/<题号>/<第几次提交>.cpp`。
pub fn submission_source_path(
    data_dir: &Path,
    student_id: &str,
    problem_key: &str,
    seq: i64,
) -> PathBuf {
    submission_dir(data_dir, student_id, problem_key).join(format!("{seq}.cpp"))
}

const MAX_COMPONENT_CHARS: usize = 48;

/// 把账号/题号变成安全的目录名。
///
/// 正常情况下（学号、身份证号、`A`/`B` 题号）原样返回，教师打开 `data` 就能按学生找代码。
/// 遇到 Windows 不允许的字符时换成 `_`，并在末尾追加原值的哈希，
/// 这样「`a/b`」和「`a\b`」不会挤进同一个目录、两份源码互相覆盖。
pub fn path_component(raw: &str) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|ch| {
            if is_forbidden_in_file_name(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    while cleaned.ends_with(' ') || cleaned.ends_with('.') {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        cleaned.push('_');
    }
    if is_reserved_name(&cleaned) {
        cleaned.insert(0, '_');
    }
    let truncated: String = cleaned.chars().take(MAX_COMPONENT_CHARS).collect();
    if truncated == raw {
        return truncated;
    }
    let digest = Sha256::digest(raw.as_bytes());
    format!("{truncated}-{}", hex::encode(&digest[..4]))
}

fn is_forbidden_in_file_name(ch: char) -> bool {
    ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Windows 的保留设备名，即使带扩展名也不能用作文件名。
fn is_reserved_name(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    RESERVED
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_root_prefers_env_then_dev_then_bin_parent() {
        let base = tempfile::tempdir().unwrap();
        let cwd = base.path().join("cwd");
        let exe = base.path().join("bin").join("labjudge.exe");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(exe.parent().unwrap()).unwrap();

        let env_root = base.path().join("from-env");
        assert_eq!(resolve_root(Some(env_root.clone()), &cwd, &exe), env_root);

        assert_eq!(resolve_root(None, &cwd, &exe), base.path().to_path_buf());

        fs::write(cwd.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(resolve_root(None, &cwd, &exe), cwd);
    }

    #[test]
    fn resolve_root_uses_exe_parent_outside_bin() {
        let base = tempfile::tempdir().unwrap();
        let cwd = base.path().join("cwd");
        let exe = base.path().join("desktop").join("labjudge.exe");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(exe.parent().unwrap()).unwrap();

        assert_eq!(
            resolve_root(None, &cwd, &exe),
            exe.parent().unwrap().to_path_buf()
        );
    }

    #[test]
    fn go_judge_resolution_order() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let env = base.path().join("custom.exe");
        assert_eq!(resolve_go_judge(Some(env.clone()), root), env);

        let in_bin = root.join("bin").join("go-judge.exe");
        fs::create_dir_all(in_bin.parent().unwrap()).unwrap();
        fs::write(&in_bin, b"x").unwrap();
        assert_eq!(resolve_go_judge(None, root), in_bin);

        fs::remove_file(&in_bin).unwrap();
        assert_eq!(
            resolve_go_judge(None, root),
            root.join("vendor").join("go-judge").join("go-judge.exe")
        );
    }

    #[test]
    fn testlib_resolution_order() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let env = base.path().join("custom-testlib.h");
        assert_eq!(resolve_testlib(Some(env.clone()), root), env);

        assert_eq!(
            resolve_testlib(None, root),
            root.join("vendor").join("testlib").join("testlib.h")
        );

        let in_include = root.join("include").join("testlib.h");
        fs::create_dir_all(in_include.parent().unwrap()).unwrap();
        fs::write(&in_include, b"x").unwrap();
        assert_eq!(resolve_testlib(None, root), in_include);
    }

    #[test]
    fn data_lock_rejects_second_owner() {
        let dir = tempfile::tempdir().unwrap();
        let _first = DataLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            DataLock::acquire(dir.path()),
            Err(DataLockError::Locked(_))
        ));
    }

    #[test]
    fn data_lock_is_a_named_file_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labjudge.lock");
        {
            let _lock = DataLock::acquire(dir.path()).unwrap();
            assert!(path.is_file());
        }
        // Windows 上锁是「锁住字节范围」，运行期间连自己都读不了；退出后再看内容。
        let note = fs::read_to_string(&path).unwrap();
        assert!(note.contains("MatrOJ"));

        // 重启不会重复写说明。
        drop(DataLock::acquire(dir.path()).unwrap());
        let note = fs::read_to_string(&path).unwrap();
        assert_eq!(note.matches("# 本文件由").count(), 1);
    }

    #[test]
    fn path_component_keeps_normal_ids_readable() {
        assert_eq!(path_component("110101200001011234"), "110101200001011234");
        assert_eq!(path_component("2026001"), "2026001");
        assert_eq!(path_component("张三"), "张三");
        assert_eq!(path_component("A"), "A");
    }

    #[test]
    fn path_component_escapes_unsafe_names() {
        // 非法字符换掉，并追加原值哈希，两个只差一个字符的账号不会落到同一目录。
        let a = path_component("a/b");
        let b = path_component("a\\b");
        assert!(!a.contains('/'));
        assert!(!b.contains('\\'));
        assert_ne!(a, b);

        assert!(path_component("  ").starts_with("_-"));
        assert!(path_component("a ").starts_with("a-"));
        assert!(path_component("..").starts_with("_-"));
        assert!(path_component("con").starts_with("_con-"));
        assert!(path_component(&"9".repeat(80)).len() < 80);
    }

    #[test]
    fn submission_paths_are_grouped_by_student_then_problem() {
        let data = Path::new("C:/data/demo");
        let path = submission_source_path(data, "2026001", "B", 3);
        assert_eq!(
            path,
            PathBuf::from("C:/data/demo/submissions/2026001/B/3.cpp")
        );
        assert_eq!(
            submission_dir(data, "2026001", "B"),
            PathBuf::from("C:/data/demo/submissions/2026001/B")
        );
    }

    #[test]
    fn candidate_ports_are_eleven_and_overflow_safe() {
        assert_eq!(candidate_ports(8080).len(), 11);
        assert_eq!(candidate_ports(8080)[0], 8080);
        assert_eq!(candidate_ports(8080)[10], 8090);
        assert_eq!(candidate_ports(u16::MAX), vec![u16::MAX]);
    }
}
