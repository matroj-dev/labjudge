mod acceptance;
mod fsutil;
mod http;
mod package;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("package") => {
            if args.next().is_some() {
                bail!("package 不接受参数");
            }
            package::run()
        }
        Some("acceptance") => {
            let mut options = acceptance::Options::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--no-reset" => options.no_reset = true,
                    "--dist" => {
                        let path = args
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("--dist 缺少路径"))?;
                        options.dist = Some(PathBuf::from(path));
                    }
                    "-h" | "--help" => {
                        print_usage();
                        return Ok(());
                    }
                    other => bail!("未知参数: {other}"),
                }
            }
            acceptance::run(options)
        }
        Some("-h" | "--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => bail!("未知命令: {other}"),
    }
}

fn print_usage() {
    println!("用法:");
    println!("  cargo xtask package");
    println!("  cargo xtask acceptance [--no-reset] [--dist <发布目录>]");
}
