use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use labjudge::app::build_router;
use labjudge::app_state::AppState;
use labjudge::config::Config;
use labjudge::judge::{CaseRunner, Compiler, CompilerBackend, JudgeQueue, RealEngine, Supervisor};
use labjudge::paths::{candidate_ports, DataLock, Paths};
use labjudge::services::registry::{scan_contests, ContestRegistry};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let paths = Paths::discover().context("初始化程序路径失败")?;
    paths.ensure_dirs().context("创建数据/比赛目录失败")?;
    let _data_lock = DataLock::acquire(&paths.data).context("锁定数据目录失败")?;

    let (config, first_run) =
        Config::load_or_create(&paths.root).context("读取或生成 config.json 失败")?;

    tracing::info!(root = %paths.root.display(), "工具根目录");
    tracing::info!(config = %paths.config_file.display(), "配置文件");

    // 目标机必须自备 MinGW g++（PATH 上可执行）：探测不到就直接退出，不启动服务。
    let compiler = Arc::new(Compiler::detect().context(
        "未探测到可用的 MinGW g++，程序无法启动。\n\
         请安装 MinGW，把其 bin 目录加入 PATH，并确认命令行执行 `g++ --version` 成功。",
    )?);
    tracing::info!(path = %compiler.path().display(), version = compiler.version(), "编译器就绪");

    // go-judge 随包分发，启动失败同样视为环境损坏，直接退出。
    let supervisor = Arc::new(
        Supervisor::start(&paths, config.judge_concurrency)
            .await
            .context(
                "go-judge 启动失败，程序无法启动。\n\
                 请确认 bin 目录下的 go-judge.exe 与 labjudge.exe 未被安全软件拦截或删除。",
            )?,
    );
    tracing::info!(version = supervisor.version(), "go-judge 就绪");

    let registry = Arc::new(ContestRegistry::load(
        scan_contests(&paths.contests),
        &paths.data,
    ));
    for invalid in registry.invalid() {
        tracing::warn!(
            contest = %invalid.id,
            path = %invalid.path.display(),
            reason = %invalid.reason,
            "比赛包校验失败"
        );
    }
    tracing::info!(
        valid = registry.valid_count(),
        invalid = registry.invalid().len(),
        "比赛包扫描完成"
    );

    if first_run {
        println!("首次启动，管理口令：{}", config.admin_password);
    }

    let (listener, port) = bind_available(config.port)
        .await
        .context("绑定 HTTP 端口失败")?;
    tracing::info!(port, "HTTP 服务已启动");

    for url in labjudge::net::access_urls(port) {
        println!("访问地址：{url}");
    }
    let admin_url = labjudge::net::admin_url(port);
    println!("管理地址：{admin_url}");

    let queue = {
        let compiler_backend: Arc<dyn CompilerBackend> = compiler.clone();
        let runner: Arc<dyn CaseRunner> = Arc::new(supervisor.client());
        let engine = Arc::new(RealEngine::new(compiler_backend, runner));
        JudgeQueue::start(registry.clone(), engine, config.judge_concurrency)
    };

    {
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            let mut restarted = false;
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if supervisor.is_alive() {
                    continue;
                }
                if restarted {
                    tracing::error!("go-judge 再次退出，停止自动重启");
                    break;
                }
                restarted = true;
                tracing::warn!("go-judge 退出，尝试自动重启一次");
                match supervisor.restart().await {
                    Ok(()) => tracing::info!("go-judge 已重启"),
                    Err(error) => {
                        tracing::error!(%error, "go-judge 自动重启失败");
                        break;
                    }
                }
            }
        });
    }

    let mut state = AppState::new(Arc::new(config), Arc::new(paths), port, registry)
        .context("会话密钥配置非法")?;
    state.compiler = Some(compiler);
    state.supervisor = Some(supervisor);
    state.queue = Some(queue);
    let app = build_router(state);

    if first_run {
        let admin_url = admin_url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Err(error) = webbrowser::open(&admin_url) {
                tracing::warn!(%error, "自动打开管理端失败");
            }
        });
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("HTTP 服务异常退出")?;
    Ok(())
}

async fn bind_available(start: u16) -> anyhow::Result<(TcpListener, u16)> {
    let mut last_error = None;
    for port in candidate_ports(start) {
        match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(error) => last_error = Some((port, error)),
        }
    }

    match last_error {
        Some((port, error)) => bail!("端口 {port} 及后续 10 个端口均不可用: {error}"),
        None => bail!("没有可用端口"),
    }
}
