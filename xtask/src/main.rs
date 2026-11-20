//! Local engineering task entry point for oxiview.

// Engineering tools may use expect for internal invariants; library crates do not inherit this exception.
#![allow(clippy::expect_used)]

mod bench;
mod check;
mod deps;
mod docs;
mod hooks;
mod perf_report;
mod report;
mod scaffold;
mod snapshot;

use std::{path::Path, process::ExitCode};

use check::{CommandCheck, execute};
use clap::{Parser, Subcommand};
use report::{CheckResult, Report};

#[derive(Parser)]
#[command(about = "Local engineering verification for oxiview")]
struct Cli {
    /// Print only the JSON report to stdout.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Format code.
    Fmt {
        /// Check without modifying files.
        #[arg(long)]
        check: bool,
    },
    /// Run strict Clippy checks.
    Lint,
    /// Run workspace tests.
    Test,
    /// Check dependency direction, licenses, advisories, and sources.
    Deps,
    /// Review or explicitly accept pixel snapshots (modifies baseline files).
    Snapshot {
        /// List pending snapshots.
        #[arg(long, required_unless_present = "accept", conflicts_with = "accept")]
        review: bool,
        /// Accept a name, crate/name, or all.
        #[arg(long)]
        accept: Option<String>,
    },
    /// Run all checks after environment validation, then summarize results.
    Verify {
        /// Add performance benchmarks after ordinary verification.
        #[arg(long)]
        full: bool,
    },
    /// Run benchmarks and compare this machine's performance baseline.
    Bench {
        /// Run only the eight M2 layout and UI tree workloads without opening a native window.
        #[arg(long)]
        m2: bool,
        /// Record the selected scope in the repository baseline. Run only after human confirmation.
        #[arg(long)]
        record: bool,
    },
    /// Refresh the bilingual README excerpts and complete performance history.
    PerfReport {
        /// Verify generated Markdown without updating files.
        #[arg(long, conflicts_with_all = ["add", "start"])]
        check: bool,
        /// Archive a directory containing run.json and completed measurement artifacts.
        #[arg(long, value_name = "RUN_DIRECTORY")]
        add: Option<std::path::PathBuf>,
        /// Capture the current Git HEAD and working-tree state before measuring.
        #[arg(
            long,
            value_name = "RUN_DIRECTORY",
            conflicts_with = "add",
            requires = "machine"
        )]
        start: Option<std::path::PathBuf>,
        /// Hardware, software and display configuration used for this run.
        #[arg(long, requires = "start")]
        machine: Option<String>,
    },
    /// Create a crate registered in layers.toml without overwriting existing directories.
    NewCrate {
        /// Crate name.
        name: String,
        /// Must exactly match the context in the rules.
        #[arg(long)]
        context: String,
    },
    /// Check the toolchain and external tools without installing them.
    Doctor,
    /// Explicitly install the optional pre-push verification hook.
    InstallHooks {
        /// Back up a different existing hook before replacing it; refuse if a backup already exists.
        #[arg(long, conflicts_with = "uninstall")]
        force: bool,
        /// Remove only a hook whose content exactly matches this command's generated script.
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
        remedy: "Check that the Rust toolchain and rustfmt/clippy components are available.",
    }
}

fn doctor(root: &Path, report: &mut Report) {
    let rustc = CommandCheck {
        name: "doctor-rustc",
        program: "rustc".into(),
        args: vec!["--version"],
        environment: true,
        required_rust: true,
        remedy: "Rust stable 1.97 is required; install with: rustup toolchain install 1.97.0 --component rustfmt --component clippy",
    };
    let deny = CommandCheck {
        environment: true,
        remedy: "cargo-deny is required; install with: cargo install cargo-deny --locked",
        ..cargo_check("doctor-cargo-deny", vec!["deny", "--version"])
    };
    for check in [&rustc, &deny] {
        report.checks.push(execute(check, root));
    }
}

fn run(task: Task, root: &Path, report: &mut Report) {
    if let Task::PerfReport {
        check,
        add,
        start,
        machine,
    } = task
    {
        report.checks.push(execute(
            &perf_report::PerfReport {
                check,
                add,
                start,
                machine,
            },
            root,
        ));
        return;
    }
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
    if let Task::Bench { m2, record } = task {
        let scope = if m2 {
            bench::Scope::M2
        } else {
            bench::Scope::Full
        };
        report
            .checks
            .push(execute(&bench::BenchCheck { scope, record }, root));
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
        | Task::PerfReport { .. }
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
        report.checks.push(execute(
            &bench::BenchCheck {
                scope: bench::Scope::Full,
                record: false,
            },
            root,
        ));
    }
}

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is under the repository root");
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
    if let Some(scope) = report
        .checks
        .iter()
        .find_map(|item| bench::Scope::from_check_name(&item.name))
    {
        report.performance = std::fs::read(root.join("target/xtask").join(scope.results_file()))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    }
    let serialized =
        serde_json::to_string_pretty(&report).expect("report contains only serializable values");
    if let Err(error) = std::fs::write(root.join("target/xtask/report.json"), &serialized) {
        report.checks.push(CheckResult::error(
            "report",
            format!("Cannot write report: {error}"),
        ));
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .expect("report contains only serializable values")
        );
    } else {
        for item in &report.checks {
            println!("{}: {:?} ({} ms)", item.name, item.status, item.duration_ms);
            if matches!(
                item.name.as_str(),
                "snapshot" | "bench" | "bench-m2" | "new-crate" | "install-hooks"
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
            "Summary: {} checks, exit code {}; report: target/xtask/report.json",
            report.checks.len(),
            report.exit_code()
        );
    }
    ExitCode::from(report.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn bench_accepts_each_flag_once_in_either_order() {
        for (flags, m2, record) in [
            (&[][..], false, false),
            (&["--record"][..], false, true),
            (&["--m2"][..], true, false),
            (&["--m2", "--record"][..], true, true),
            (&["--record", "--m2"][..], true, true),
        ] {
            let args = ["xtask", "bench"].into_iter().chain(flags.iter().copied());
            let cli = Cli::try_parse_from(args).expect("valid bench flags");
            assert!(matches!(
                cli.command,
                Task::Bench { m2: actual_m2, record: actual_record }
                    if actual_m2 == m2 && actual_record == record
            ));
        }
        for (flags, kind) in [
            (&["--m2", "--m2"][..], ErrorKind::ArgumentConflict),
            (&["--record", "--record"][..], ErrorKind::ArgumentConflict),
            (&["--m3"][..], ErrorKind::UnknownArgument),
            (&["--m2=true"][..], ErrorKind::TooManyValues),
            (&["layout"][..], ErrorKind::UnknownArgument),
        ] {
            let args = ["xtask", "bench"].into_iter().chain(flags.iter().copied());
            let error = Cli::try_parse_from(args)
                .err()
                .expect("invalid bench flags");
            assert_eq!(error.kind(), kind);
        }
    }
}
