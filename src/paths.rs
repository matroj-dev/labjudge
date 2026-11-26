use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub contests: PathBuf,
    pub data: PathBuf,
    pub go_judge: PathBuf,
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

impl DataLock {
    pub fn acquire(data_dir: &Path) -> Result<Self, DataLockError> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if FileExt::try_lock_exclusive(&file).is_err() {
            return Err(DataLockError::Locked(path));
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

        Ok(Self {
            data: root.join("data"),
            contests: root.join("contests"),
            config_file: crate::config::config_path(&root),
            go_judge,
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
            root,
        }
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

pub fn candidate_ports(start: u16) -> Vec<u16> {
    (0..=10u16)
        .filter_map(|offset| start.checked_add(offset))
        .collect()
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
    fn data_lock_rejects_second_owner() {
        let dir = tempfile::tempdir().unwrap();
        let _first = DataLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            DataLock::acquire(dir.path()),
            Err(DataLockError::Locked(_))
        ));
    }

    #[test]
    fn candidate_ports_are_eleven_and_overflow_safe() {
        assert_eq!(candidate_ports(8080).len(), 11);
        assert_eq!(candidate_ports(8080)[0], 8080);
        assert_eq!(candidate_ports(8080)[10], 8090);
        assert_eq!(candidate_ports(u16::MAX), vec![u16::MAX]);
    }
}
