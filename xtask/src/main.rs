//! oxiview 本地工程任务入口。

// 工程工具允许对内部不变量使用 expect；库 crate 不继承此例外。
#![allow(clippy::expect_used)]

mod bench;
mod check;
mod deps;
mod docs;
mod hooks;
mod report;
mod scaffold;
mod snapshot;

use std::{path::Path, process::ExitCode};

use check::{CommandCheck, execute};
use clap::{Parser, Subcommand};
use report::{CheckResult, Report};

#[derive(Parser)]
#[command(about = "oxiview 本地工程验证")]
struct Cli {
    /// 仅向 stdout 输出 JSON 报告。
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// 格式化代码。
    Fmt {
        /// 只检查，不修改文件。
        #[arg(long)]
        check: bool,
    },
    /// 执行严格 Clippy 检查。
    Lint,
    /// 执行 workspace 测试。
    Test,
    /// 检查依赖方向、许可证、安全公告和来源。
    Deps,
    /// 审阅或显式接受像素快照（会修改基准文件）。
    Snapshot {
        /// 列出待处理快照。
        #[arg(long, required_unless_present = "accept", conflicts_with = "accept")]
        review: bool,
        /// 接受指定名称、crate/name 或 all。
        #[arg(long)]
        accept: Option<String>,
    },
    /// 环境检查通过后执行全部检查，最后汇总。
    Verify {
        /// 普通验证之后追加性能基准。
        #[arg(long)]
        full: bool,
    },
    /// 运行基准并比较本机性能基线。
    Bench {
        /// 创建或更新仓库基线文件；仅在人确认后运行。
        #[arg(long)]
        record: bool,
    },
    /// 创建已在 layers.toml 登记的 crate，不覆盖已有目录。
    NewCrate {
        /// crate 名称。
        name: String,
        /// 必须与规则表中的上下文完全一致。
        #[arg(long)]
        context: String,
    },
    /// 检查工具链和外部工具，不自动安装。
    Doctor,
    /// 显式安装可选的 pre-push 验证 hook。
    InstallHooks {
        /// 备份已有不同内容的 hook 后覆盖；已有备份时拒绝覆盖。
        #[arg(long, conflicts_with = "uninstall")]
        force: bool,
        /// 只删除内容与本命令生成内容完全匹配的 hook。
        #[arg(long)]
        uninstall: bool,
    },
}

fn cargo_check(name: &'static str, args: Vec<&'static str>) -> CommandCheck {
    CommandCheck {
        name,
        program: std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()),
        args,
        environment: false,
        required_rust: false,
        remedy: "请检查 Rust 工具链及 rustfmt、clippy 组件是否可用。",
    }
}

fn doctor(root: &Path, report: &mut Report) {
    let rustc = CommandCheck {
        name: "doctor-rustc",
        program: "rustc".into(),
        args: vec!["--version"],
        environment: true,
        required_rust: true,
        remedy: "需要 Rust stable 1.97；安装命令：rustup toolchain install 1.97.0 --component rustfmt --component clippy",
    };
    let deny = CommandCheck {
        environment: true,
        remedy: "需要 cargo-deny；安装命令：cargo install cargo-deny --locked",
        ..cargo_check("doctor-cargo-deny", vec!["deny", "--version"])
    };
    for check in [&rustc, &deny] {
        report.checks.push(execute(check, root));
    }
}

fn run(task: Task, root: &Path, report: &mut Report) {
    if let Task::InstallHooks { force, uninstall } = task {
        report
            .checks
            .push(execute(&hooks::InstallHooks { force, uninstall }, root));
        return;
    }
    if let Task::NewCrate { name, context } = task {
        report
            .checks
            .push(execute(&scaffold::NewCrate { name, context }, root));
        return;
    }
    if let Task::Bench { record } = task {
        report
            .checks
            .push(execute(&bench::BenchCheck { record }, root));
        return;
    }
    let full = matches!(task, Task::Verify { full: true });
    if let Task::Snapshot { accept, .. } = task {
        report
            .checks
            .push(execute(&snapshot::SnapshotCheck { accept }, root));
        return;
    }
    if matches!(task, Task::Doctor | Task::Verify { .. }) {
        doctor(root, report);
        if matches!(task, Task::Doctor) || report.exit_code() != 0 {
            return;
        }
    }
    let fmt = |check| {
        let mut args = vec!["fmt", "--all"];
        if check {
            args.push("--check");
        }
        cargo_check("fmt", args)
    };
    let lint = || {
        cargo_check(
            "lint",
            vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )
    };
    let test = || cargo_check("test", vec!["test", "--workspace"]);
    let include_deps = matches!(task, Task::Deps | Task::Verify { .. });
    let checks = match task {
        Task::Fmt { check } => vec![fmt(check)],
        Task::Lint => vec![lint()],
        Task::Test => vec![test()],
        Task::Verify { .. } => vec![fmt(true), lint(), test()],
        Task::Doctor
        | Task::Deps
        | Task::Snapshot { .. }
        | Task::Bench { .. }
        | Task::InstallHooks { .. }
        | Task::NewCrate { .. } => Vec::new(),
    };
    for check in checks {
        report.checks.push(execute(&check, root));
    }
    if include_deps {
        report.checks.push(execute(&deps::DepsCheck, root));
    }
    if matches!(task, Task::Verify { .. }) {
        report.checks.push(execute(&docs::DocsCheck, root));
    }
    if full {
        report
            .checks
            .push(execute(&bench::BenchCheck { record: false }, root));
    }
}

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 位于仓库根目录下");
    let mut report = Report::default();
    let mut json = std::env::args_os().any(|arg| arg == "--json");
    if let Err(error) = std::fs::create_dir_all(root.join("target/xtask")) {
        report
            .checks
            .push(CheckResult::error("xtask", error.to_string()));
    } else {
        match Cli::try_parse() {
            Ok(cli) => {
                json = cli.json;
                run(cli.command, root, &mut report);
            }
            Err(error) => {
                if error.exit_code() == 0 {
                    eprintln!("{error}");
                } else {
                    report
                        .checks
                        .push(CheckResult::error("cli", error.to_string()));
                }
            }
        }
    }
    if report.checks.iter().any(|item| item.name == "bench") {
        report.performance = std::fs::read(root.join("target/xtask/bench-results.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    }
    let serialized = serde_json::to_string_pretty(&report).expect("报告只包含可序列化值");
    if let Err(error) = std::fs::write(root.join("target/xtask/report.json"), &serialized) {
        report.checks.push(CheckResult::error(
            "report",
            format!("无法写入报告：{error}"),
        ));
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("报告只包含可序列化值")
        );
    } else {
        for item in &report.checks {
            println!(
                "{}: {:?}（{} 毫秒）",
                item.name, item.status, item.duration_ms
            );
            if matches!(
                item.name.as_str(),
                "snapshot" | "bench" | "new-crate" | "install-hooks"
            ) && item.status == report::Status::Pass
                && let Ok(log) =
                    std::fs::read_to_string(root.join(format!("target/xtask/{}.log", item.name)))
            {
                print!("{log}");
            }
            if let Some(summary) = &item.failure_summary {
                eprintln!("{summary}");
            }
        }
        println!(
            "汇总：{} 项，退出码 {}；报告：target/xtask/report.json",
            report.checks.len(),
            report.exit_code()
        );
    }
    ExitCode::from(report.exit_code())
}
