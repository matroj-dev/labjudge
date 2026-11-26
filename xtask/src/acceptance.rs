use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use anyhow::{bail, Context, Result};

use crate::fsutil;
use crate::http::{self, Session};

const ADMIN_PASSWORD: &str = "242424";

#[derive(Default)]
pub struct Options {
    pub dist: Option<PathBuf>,
    pub no_reset: bool,
}

pub fn run(options: Options) -> Result<()> {
    http::ensure_curl()?;
    let root = fsutil::root();
    let dist = match options.dist {
        Some(path) if path.is_absolute() => path,
        Some(path) => root.join(path),
        None => root.join("dist").join("labjudge"),
    };

    for relative in [
        "start.bat",
        "README.txt",
        "bin/labjudge.exe",
        "bin/go-judge.exe",
        "contests/sample-contest/contest.json",
    ] {
        if !dist.join(relative).is_file() {
            bail!("发布目录缺少 {relative}: {}", dist.display());
        }
    }

    println!("[1/8] 复制发布包到临时目录");
    let run_dir = std::env::temp_dir().join("labjudge-acceptance");
    fsutil::remove_dir_if_exists(&run_dir)?;
    fs::create_dir_all(&run_dir)?;
    fsutil::copy_dir(&dist, &run_dir)?;

    // 与 labjudge 一致：只认 PATH 上的 g++。
    let gpp = PathBuf::from("g++");
    let gpp_version = fsutil::first_line(&gpp, "--version")?;
    let gpp_triple = fsutil::first_line(&gpp, "-dumpmachine")?;
    if !gpp_triple.to_ascii_lowercase().contains("mingw") {
        bail!("需要 PATH 上的 MinGW g++（`g++ -dumpmachine` 当前输出 “{gpp_triple}”）");
    }
    println!("[OK] {gpp_version}（{gpp_triple}）");

    println!("[2/8] 选择端口并写入 config");
    let port = free_port()?;
    write_config(&run_dir, port)?;
    let base = format!("http://127.0.0.1:{port}");

    println!("[3/8] 启动发布版程序");
    let mut app = AppGuard::start(&run_dir)?;
    let probe = Session::new(&base, &run_dir, "probe");
    let result = (|| -> Result<()> {
        app.wait_healthy(&probe)?;
        execute(&run_dir, &base, &probe, options.no_reset)
    })();

    let failed = result.is_err();
    drop(app);
    std::thread::sleep(Duration::from_secs(1));

    if failed {
        print_logs(&run_dir);
        eprintln!("失败现场保留: {}", run_dir.display());
    } else if options.no_reset {
        println!("临时目录保留: {}", run_dir.display());
    } else if let Err(error) = fsutil::remove_dir_with_retry(&run_dir) {
        eprintln!("警告: 无法清理临时目录 {}: {error}", run_dir.display());
    }

    result
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn write_config(run_dir: &Path, port: u16) -> Result<()> {
    let config = serde_json::json!({
        "port": port,
        "adminPassword": ADMIN_PASSWORD,
        "judgeConcurrency": 2,
        "submitCooldownSeconds": 0,
        "maxSourceBytes": 65536,
        "sessionSecret": "ab".repeat(32),
    });
    fs::write(
        run_dir.join("config.json"),
        serde_json::to_vec_pretty(&config)?,
    )?;
    Ok(())
}

fn execute(run_dir: &Path, base: &str, probe: &Session, no_reset: bool) -> Result<()> {
    probe.get("/healthz")?.expect_ok("健康检查")?;
    let asset = probe.get("/static/vendor/mathjax/tex-svg.js")?;
    asset.expect_ok("内嵌 MathJax")?;
    println!("[OK] 健康检查与内嵌静态资源");

    println!("[4/8] 管理员登录并开赛");
    let admin = Session::new(base, run_dir, "admin");
    let login_page = admin.get("/admin/login")?;
    login_page.expect_ok("管理员登录页")?;
    let login_csrf = extract_csrf(&login_page.text()?)?;
    let login = admin.post_form(
        "/admin/login",
        &[("password", ADMIN_PASSWORD), ("csrf", login_csrf.as_str())],
    )?;
    login.expect_ok("管理员登录")?;
    if !login.text()?.contains("name=\"csrf\"") {
        bail!("管理员登录后未进入控制台");
    }

    let detail = admin.get("/admin/c/sample-contest")?;
    detail.expect_ok("比赛详情")?;
    let start_csrf = extract_csrf(&detail.text()?)?;
    admin
        .post_form(
            "/admin/c/sample-contest/start",
            &[("csrf", start_csrf.as_str())],
        )?
        .expect_ok("开赛")?;

    let system = admin.get("/admin/system")?;
    system.expect_ok("系统信息")?;
    // 工具链不再随包分发，只要求启动后确实探测到了 MinGW g++。
    let system_html = system.text()?;
    if system_html.contains("未探测到 MinGW g++") {
        bail!("系统信息显示未探测到 g++，发布包需要能在目标机上自行找到编译器");
    }
    if system_html.contains("<td>不可用</td>") {
        bail!("系统信息中编译器不可用");
    }
    println!("[OK] 管理员登录、开赛与系统信息");

    println!("[5/8] 学生参赛并提交 C++");
    let student = Session::new(base, run_dir, "student");
    let join_page = student.get("/c/sample-contest/join")?;
    join_page.expect_ok("参赛页")?;
    let join_csrf = extract_csrf(&join_page.text()?)?;
    student
        .post_form(
            "/c/sample-contest/join",
            &[
                ("student_id", "110101200001011234"),
                ("password", "011234"),
                ("csrf", join_csrf.as_str()),
            ],
        )?
        .expect_ok("学生参赛")?;

    let problem_page = student.get("/c/sample-contest/p/A")?;
    problem_page.expect_ok("题目页")?;
    let submit_csrf = extract_csrf(&problem_page.text()?)?;
    let source = r#"#include <bits/stdc++.h>
int main(){long long a,b;std::cin>>a>>b;std::cout<<a+b<<"\n";}
"#;
    student
        .post_form(
            "/c/sample-contest/submissions",
            &[
                ("problem_key", "A"),
                ("source", source),
                ("csrf", submit_csrf.as_str()),
            ],
        )?
        .expect_ok("提交")?;

    let mut judged = false;
    for _ in 0..120 {
        std::thread::sleep(Duration::from_millis(500));
        let list = admin.get("/admin/c/sample-contest/submissions")?;
        list.expect_ok("提交列表")?;
        let text = list.text()?;
        if text.contains(">AC<") && text.contains(">100<") {
            judged = true;
            break;
        }
    }
    if !judged {
        bail!("提交未在时限内达到 AC 100");
    }

    let hidden = student.get("/c/sample-contest")?;
    hidden.expect_ok("比赛首页")?;
    if hidden.text()?.contains(">AC<") {
        bail!("隐藏模式泄漏了判定");
    }
    println!("[OK] 真判题 AC，隐藏模式未泄漏");

    println!("[6/8] 公开榜单并验证学生视图");
    let detail = admin.get("/admin/c/sample-contest")?;
    detail.expect_ok("比赛详情")?;
    let toggle_csrf = extract_csrf(&detail.text()?)?;
    admin
        .post_form(
            "/admin/c/sample-contest/leaderboard",
            &[("csrf", toggle_csrf.as_str())],
        )?
        .expect_ok("切换榜单")?;
    let rank = student.get("/c/sample-contest")?;
    rank.expect_ok("学生榜单")?;
    let rank_text = rank.text()?;
    if !rank_text.contains("张三") || !rank_text.contains("100") {
        bail!("学生榜单缺少学生或分数");
    }
    println!("[OK] 榜单开关对学生生效");

    println!("[7/8] 导出");
    let csv = admin.get("/admin/c/sample-contest/export.csv")?;
    csv.expect_ok("CSV 导出")?;
    if csv.body.len() <= 3 {
        bail!("CSV 为空");
    }
    if csv.body[..3] != [0xEF, 0xBB, 0xBF] {
        bail!("CSV 缺少 BOM");
    }
    let csv_text = String::from_utf8_lossy(&csv.body[3..]);
    if !csv_text.contains("110101200001011234") {
        bail!("CSV 缺少学生");
    }

    let sources = admin.get("/admin/c/sample-contest/sources.zip")?;
    sources.expect_ok("源码 zip 导出")?;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(sources.body)).context("源码 zip 无法解析")?;
    let mut has_manifest = false;
    let mut has_source = false;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name();
        if name == "manifest.csv" {
            has_manifest = true;
        }
        if name.ends_with("A_1.cpp") {
            has_source = true;
        }
    }
    if !has_manifest || !has_source {
        bail!("源码 zip 缺少 manifest 或源码");
    }
    println!("[OK] CSV BOM/学生与源码 zip");

    println!("[8/8] 重置并停止");
    if no_reset {
        println!("[SKIP] 按参数保留数据");
    } else {
        let detail = admin.get("/admin/c/sample-contest")?;
        detail.expect_ok("比赛详情")?;
        let reset_csrf = extract_csrf(&detail.text()?)?;
        admin
            .post_form(
                "/admin/c/sample-contest/reset",
                &[("csrf", reset_csrf.as_str()), ("confirm", "yes")],
            )?
            .expect_ok("重置")?;
        let list = admin.get("/admin/c/sample-contest/submissions")?;
        list.expect_ok("提交列表")?;
        if list.text()?.contains("张三") {
            bail!("重置后仍有提交");
        }
        println!("[OK] 重置清除提交");
    }

    println!("ACCEPTANCE PASS");
    Ok(())
}

fn extract_csrf(html: &str) -> Result<String> {
    const NEEDLE: &str = "name=\"csrf\" value=\"";
    let start = html
        .find(NEEDLE)
        .ok_or_else(|| anyhow::anyhow!("页面缺少 csrf 字段"))?
        + NEEDLE.len();
    let rest = &html[start..];
    let end = rest
        .find('"')
        .ok_or_else(|| anyhow::anyhow!("csrf 字段格式非法"))?;
    if end == 0 {
        bail!("csrf 字段为空");
    }
    Ok(rest[..end].to_string())
}

struct AppGuard {
    child: Child,
}

impl AppGuard {
    fn start(run_dir: &Path) -> Result<Self> {
        let stdout = File::create(run_dir.join("acceptance.out.log"))?;
        let stderr = File::create(run_dir.join("acceptance.err.log"))?;
        // 不注入任何路径变量：与教师机双击 start.bat 时走同一套 bin\ 推断逻辑。
        let mut command = Command::new(run_dir.join("bin").join("labjudge.exe"));
        command
            .current_dir(run_dir)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(windows)]
        command.creation_flags(0x08000000);

        let child = command.spawn().context("无法启动发布版程序")?;
        Ok(Self { child })
    }

    fn wait_healthy(&mut self, probe: &Session) -> Result<()> {
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait()? {
                bail!("程序提前退出: {status}");
            }
            if let Ok(response) = probe.get("/healthz") {
                if response.status == 200 {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        bail!("程序未在 20 秒内就绪");
    }
}

impl Drop for AppGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn print_logs(run_dir: &Path) {
    for name in ["acceptance.out.log", "acceptance.err.log"] {
        let path = run_dir.join(name);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        eprintln!("--- {name} ---");
        let lines = text.lines().collect::<Vec<_>>();
        for line in &lines[lines.len().saturating_sub(60)..] {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_csrf_token() {
        let html = r#"<input type="hidden" name="csrf" value="abc123" />"#;
        assert_eq!(extract_csrf(html).unwrap(), "abc123");
    }

    #[test]
    fn rejects_missing_or_empty_csrf() {
        assert!(extract_csrf("<html></html>").is_err());
        assert!(extract_csrf(r#"name="csrf" value="" "#).is_err());
    }
}
