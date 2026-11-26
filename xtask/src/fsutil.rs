use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须位于项目根目录下")
        .to_path_buf()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("无法打开 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)
        .with_context(|| format!("复制失败 {} -> {}", from.display(), to.display()))?;
    Ok(())
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        bail!("目录不存在: {}", from.display());
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        } else {
            bail!("不支持的文件类型: {}", entry.path().display());
        }
    }
    Ok(())
}

pub fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn remove_dir_with_retry(path: &Path) -> Result<()> {
    for attempt in 0..15 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if attempt == 14 => return Err(error.into()),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    Ok(())
}

pub fn first_line(program: &Path, arg: &str) -> Result<String> {
    let output = Command::new(program)
        .arg(arg)
        .output()
        .with_context(|| format!("无法执行 {}", program.display()))?;
    if !output.status.success() {
        bail!("{} {} 执行失败", program.display(), arg);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("{} 没有输出", program.display()))
}
