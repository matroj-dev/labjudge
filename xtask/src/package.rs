use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::Local;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::fsutil;

const GO_JUDGE_SHA: &str = "021BD390DE60212C6875496BB4F1797A5D22693EF2459651856F1937D3309D3A";
/// testlib 0.9.52，来源 MikeMirzayanov/testlib@93214ef6a94e3669f6a8ec452754169775eb5b07。
const TESTLIB_SHA: &str = "70683174E7E4D99A5EE46D47AA4FA2D12FC85631443181A12A2E5023152397F0";
const PROJECT_URL: &str = "https://github.com/matroj-dev/labjudge";

const README_TXT: &str = r#"labjudge —— MatrOJ 局域网比赛系统

单文件、离线运行：把压缩包解压到教师机，双击 start.bat 即可。

运行条件

1. Windows 10 / 11（64 位）。
2. PATH 上必须有 MinGW g++：
   - 命令行执行 g++ --version 能输出版本号；
   - g++ -dumpmachine 的输出包含 mingw。
   缺少编译器时程序会直接报错退出，不会启动服务。

启动

1. 把压缩包完整解压到可写目录，不要在压缩包里直接运行。
2. 双击 start.bat。
3. 首次启动会在控制台打印 12 位管理口令，并自动打开管理页面。
   口令、端口等保存在 config.json，可自行修改。
4. Windows 防火墙询问时，只勾选“专用网络”并允许访问。

使用

- 控制台会打印本机局域网地址，例如 http://192.168.1.10:8080。
- 学生：浏览器打开该地址，输入账号和密码进入，等待教师开赛。
  账号就是比赛包 students.csv 的 student_id 列（老师任意设定，学号或身份证号都行），
  密码是同一份 students.csv 的 password 列，由老师生成并分发（建议 8 位随机串）。
- 教师：打开 http://<本机IP>:8080/admin，用管理口令登录，在比赛详情页
  点击“开始比赛”。比赛结束后学生端会完全关闭，只显示“比赛已结束”；
  可在“比赛信息”页导出成绩 CSV 与源码 zip，成绩由老师线下告知。
- 同一账号只能有一个活动会话；学生换电脑重新登录会顶掉旧会话。
- 比赛结束后学生端关闭；如教师“恢复比赛”，学生需要重新登录。
- 结束后建议让学生关闭浏览器，避免共用电脑上残留上一名学生的页面。
- 安全边界：本工具面向受控教室，默认信任学生。Windows 版不提供代码沙箱，
  学生代码在教师机运行，可能读取教师机文件或联网；学生端的编译输出过滤只影响显示，
  不是隔离。请只在教师可监督、学生可信的教室使用。
- 示例比赛 sample-contest 已放在 contests\ 下，启动后即可看到（C 题是 SPJ 示例）。
  正式比赛把比赛包目录放进 contests\ 后重启程序即可。
- 题目一律扫描 problems\ 下的一级子目录，目录名即题号（1～16 位字母或数字），
  子目录不合规则程序不启动；contest.json 里不需要写题目。
- 比赛包、题目、名单或配置只要有一处校验不过，程序会直接在控制台打印全部原因并退出，
  不会「带病启动」把比赛标成 invalid；请先修好再启动。

SPJ（special judge）

- 题目需要自定义判分时，在题目目录放一个 checker.cpp，并在 problem.json 里写：
      "checker": { "enabled": true, "timeLimitMs": 5000, "memoryLimitMb": 512 }
  三个字段都必填、没有默认值；漏写或拼错会在启动时报出具体字段并退出。
- checker.cpp 必须是单文件，可以直接 #include "testlib.h"（头文件由本程序提供，
  不要自己往题目目录复制一份，复制了也不会被使用）。
- 不想用 SPJ 时把 enabled 改成 false（其余两项照抄），此时即使有 checker.cpp 也会忽略。
- 程序启动时只自动编译所有 checker；任何一道题编译失败，程序都不会启动，
  并打印具体错误——请在开赛前解决。checker 判题逻辑的正确性由教师负责，
  程序不做自检。
- checker 的退出码：0 通过，1/2/4 判错，3 表示题目或数据有问题（整份提交会显示
  「判题失败」，不会记 0 分），5 或 16 以上表示用了部分分（本版本不支持）。

目录

labjudge\
  start.bat               启动脚本
  README.txt              本说明
  config.json             配置（首次启动后生成）
  bin\labjudge.exe     主程序
  bin\go-judge.exe        判题器（不要删除或改名）
  include\testlib.h       SPJ 检查器用的头文件（不要删除）
  contests\               比赛包
  data\                   运行数据（首次启动后生成）
    labjudge.lock         防止误开两份程序（退出后可删）
    <比赛ID>\contest.db  提交与成绩
    <比赛ID>\submissions\<账号>\<题号>\1.cpp 2.cpp ...   学生源码
    <比赛ID>\judge.log   评测日志（每次提交全过程）

数据与备份

- 每场比赛一个数据库：data\<比赛ID>\contest.db，学生源码保存在同目录。
- 备份：停止程序后复制 data\<比赛ID>\。
- 重置比赛：管理端比赛详情页执行，清空该场提交与成绩，保留比赛包。

常见问题

- 窗口一闪而过：缺 g++ 或 go-judge 被杀软拦截。在命令行执行
  bin\labjudge.exe 查看具体错误。
- 学生打不开页面：确认防火墙允许“专用网络”，且两台机器在同一网段。
- 提示「账号或密码错误」：核对 students.csv 的 student_id 列与学生的账号，
  以及同一行的 password 列；密码区分大小写。
- 8080 被占用：程序会自动顺延端口，以控制台打印的地址为准。
- 程序被杀软拦截：把程序目录和 data\ 加入信任列表。
- 启动时提示 checker 编译失败：按提示修好该题的 checker.cpp；想先跑起来，
  把该题 problem.json 的 checker 的 enabled 改成 false 再启动。

项目主页：{{project_url}}
"#;

const START_BAT: &str = r#"@echo off
chcp 65001 >nul
title MatrOJ 局域网比赛系统
cd /d "%~dp0"

if not exist "bin\labjudge.exe" goto missing

echo 正在启动 MatrOJ 局域网比赛系统，关闭此窗口即停止服务。
echo.
"%~dp0bin\labjudge.exe"
if errorlevel 1 goto failed
exit /b 0

:missing
echo 找不到 bin\labjudge.exe。
echo 请确认压缩包已完整解压。
pause
exit /b 1

:failed
echo.
echo 程序异常退出，请查看上方错误信息后重新启动。
pause
exit /b 1
"#;

pub fn run() -> Result<()> {
    let root = fsutil::root();

    println!("[1/5] 正在构建主程序");
    build_release(&root)?;

    println!("[2/5] 正在校验固定判题器与 testlib");
    let go_judge = root.join("vendor").join("go-judge").join("go-judge.exe");
    verify_sha(&go_judge, GO_JUDGE_SHA, "go-judge")?;
    let testlib = root.join("vendor").join("testlib").join("testlib.h");
    verify_sha(&testlib, TESTLIB_SHA, "testlib.h")?;

    println!("[3/5] 生成发布目录");
    let dist_root = root.join("dist");
    let staging = dist_root.join("labjudge.staging");
    build_staging(&root, &staging, &go_judge, &testlib)?;

    println!("[4/5] 正在打包");
    let zip = dist_root.join(format!("labjudge-{}.zip", Local::now().format("%Y%m%d")));
    create_zip(&staging, &zip)?;

    println!("[5/5] 正在切换生成文件");
    let dist = dist_root.join("labjudge");
    swap_dist(&staging, &dist)?;

    println!("[OK] {}", dist.display());
    println!("[OK] {}", zip.display());
    Ok(())
}

fn build_release(root: &Path) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = Command::new(cargo)
        .arg("build")
        .arg("--release")
        .arg("--package")
        .arg("labjudge")
        .current_dir(root)
        .status()
        .context("无法执行 cargo build")?;
    if !status.success() {
        bail!("cargo build --release 失败");
    }
    Ok(())
}

fn verify_sha(path: &Path, expected: &str, label: &str) -> Result<()> {
    if !path.is_file() {
        bail!("{label} 不存在: {}", path.display());
    }
    let actual = fsutil::sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("{label} SHA256 不匹配: {actual}");
    }
    println!("[OK] {label} SHA256");
    Ok(())
}

fn build_staging(root: &Path, staging: &Path, go_judge: &Path, testlib: &Path) -> Result<()> {
    fsutil::remove_dir_if_exists(staging)?;
    fs::create_dir_all(staging.join("bin"))?;
    fs::create_dir_all(staging.join("include"))?;
    fs::create_dir_all(staging.join("contests"))?;

    let exe = root.join("target").join("release").join("labjudge.exe");
    if !exe.is_file() {
        bail!("release exe 不存在: {}", exe.display());
    }
    fsutil::copy_file(&exe, &staging.join("bin").join("labjudge.exe"))?;
    fsutil::copy_file(go_judge, &staging.join("bin").join("go-judge.exe"))?;
    // checker 编译时把这一级目录加进 -I，教师的 checker.cpp 直接 #include "testlib.h"。
    fsutil::copy_file(testlib, &staging.join("include").join("testlib.h"))?;
    fsutil::copy_dir(
        &root.join("sample-contest"),
        &staging.join("contests").join("sample-contest"),
    )?;
    write_text(
        &staging.join("README.txt"),
        &README_TXT.replace("{{project_url}}", PROJECT_URL),
        true,
    )?;
    // 批处理不能带 BOM：cmd 会把 BOM 当成第一行命令的一部分。
    write_text(&staging.join("start.bat"), START_BAT, false)?;

    verify_package(staging)?;
    Ok(())
}

/// 统一写成 CRLF。`bom` 仅给记事本看的 README.txt 用，脚本文件必须为 false。
fn write_text(path: &Path, text: &str, bom: bool) -> Result<()> {
    let body = if bom {
        format!("\u{feff}{}", text.replace('\n', "\r\n"))
    } else {
        text.replace('\n', "\r\n")
    };
    fs::write(path, body).with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

fn verify_package(staging: &Path) -> Result<()> {
    for relative in [
        "bin/labjudge.exe",
        "bin/go-judge.exe",
        "include/testlib.h",
        "contests/sample-contest/contest.json",
        "contests/sample-contest/problems/C/checker.cpp",
        "start.bat",
        "README.txt",
    ] {
        if !staging.join(relative).is_file() {
            bail!("发布目录缺少 {relative}");
        }
    }
    // 旧结构（exe 与 go-judge 散在根目录）不再允许出现。
    for forbidden in [
        "config.json",
        "data",
        "labjudge.exe",
        "go-judge.exe",
        "sample-contest",
    ] {
        if staging.join(forbidden).exists() {
            bail!("发布目录不应包含 {forbidden}");
        }
    }
    Ok(())
}

fn create_zip(staging: &Path, zip_path: &Path) -> Result<()> {
    let part = PathBuf::from(format!("{}.part", zip_path.display()));
    if part.exists() {
        fs::remove_file(&part)?;
    }
    let file = File::create(&part).with_context(|| format!("无法创建 {}", part.display()))?;
    let mut zip = ZipWriter::new(file);
    add_dir(&mut zip, staging, "labjudge")?;
    zip.finish()?;

    verify_zip(&part)?;
    if zip_path.exists() {
        fs::remove_file(zip_path)?;
    }
    fs::rename(&part, zip_path).context("重命名 zip 失败")?;
    Ok(())
}

fn add_dir(zip: &mut ZipWriter<File>, dir: &Path, prefix: &str) -> Result<()> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.add_directory(format!("{prefix}/"), options)?;
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let entry_name = format!("{prefix}/{name}");
        if file_type.is_dir() {
            add_dir(zip, &entry.path(), &entry_name)?;
        } else if file_type.is_file() {
            zip.start_file(entry_name, options)?;
            let mut source = File::open(entry.path())?;
            io::copy(&mut source, zip)?;
        } else {
            bail!("不支持的文件类型: {}", entry.path().display());
        }
    }
    Ok(())
}

fn verify_zip(zip_path: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    for required in [
        "labjudge/start.bat",
        "labjudge/README.txt",
        "labjudge/bin/labjudge.exe",
        "labjudge/bin/go-judge.exe",
        "labjudge/include/testlib.h",
        "labjudge/contests/sample-contest/contest.json",
        "labjudge/contests/sample-contest/problems/C/checker.cpp",
    ] {
        if archive.by_name(required).is_err() {
            bail!("zip 缺少 {required}");
        }
    }
    Ok(())
}

/// 旧 dist 里的比赛包（如 test-contest）已另行备份，直接删除即可。
/// 只有确实被程序或资源管理器占用时才报错，让用户关掉后重试。
fn swap_dist(staging: &Path, dist: &Path) -> Result<()> {
    fsutil::remove_dir_with_retry(dist).context(
        "删除旧 dist\\labjudge 失败：请关闭正在使用它的窗口或程序（包括资源管理器），然后重试",
    )?;
    fs::rename(staging, dist).context("切换到新 dist 失败")?;
    Ok(())
}
