//! Common check abstraction and subprocess execution.

use std::{path::Path, process::Command, time::Instant};

use crate::report::{CheckResult, Status};

/// Checks implement this interface; the shared executor records timing, logs, and error categories.
pub(crate) trait Check {
    fn name(&self) -> &str;
    fn run(&self, root: &Path) -> (Status, String);
    fn artifacts(&self) -> &[&str] {
        &[]
    }
}

pub(crate) struct CommandCheck {
    pub name: &'static str,
    pub program: String,
    pub args: Vec<&'static str>,
    pub environment: bool,
    pub required_rust: bool,
    pub remedy: &'static str,
}

impl Check for CommandCheck {
    fn name(&self) -> &str {
        self.name
    }

    fn run(&self, root: &Path) -> (Status, String) {
        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(root)
            .env("CARGO_TERM_COLOR", "never")
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return (
                    Status::Error,
                    format!("Cannot execute {}: {error}\n{}", self.program, self.remedy),
                );
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut log = format!("{stdout}{stderr}");
        let status = if output.status.success() && (!self.required_rust || supported_rust(&stdout))
        {
            Status::Pass
        } else if self.environment || output.status.code().is_none() {
            log.push_str(&format!("\nEnvironment check failed: {}\n", self.remedy));
            Status::Error
        } else {
            Status::Fail
        };
        if status != Status::Pass {
            log.push_str(&format!("\nProcess status: {}\n", output.status));
        }
        (status, log)
    }
}

fn supported_rust(output: &str) -> bool {
    let mut words = output.split_whitespace();
    words.next() == Some("rustc")
        && words.next().is_some_and(|version| {
            let parts: Vec<_> = version.split('.').collect();
            parts.len() == 3
                && parts[0] == "1"
                && parts[1] == "97"
                && parts[2].parse::<u32>().is_ok()
        })
}

pub(crate) fn execute(check: &dyn Check, root: &Path) -> CheckResult {
    let start = Instant::now();
    let (mut status, mut log) = check.run(root);
    if check.name() == "test" && status != Status::Pass && crate::snapshot::has_pending(root) {
        log.push_str("\nPending pixel snapshots; run cargo xtask snapshot --review.\n");
    }
    let path = format!("target/xtask/{}.log", check.name());
    let mut artifacts = Vec::new();
    match std::fs::write(root.join(&path), &log) {
        Ok(()) => artifacts.push(path),
        Err(error) => {
            status = Status::Error;
            log.push_str(&format!("\nCannot write log: {error}"));
        }
    }
    for artifact in check.artifacts() {
        let path = format!("target/xtask/{artifact}");
        if root.join(&path).is_file() {
            artifacts.push(path);
        }
    }
    CheckResult {
        name: check.name().into(),
        status,
        duration_ms: start.elapsed().as_millis(),
        failure_summary: (status != Status::Pass).then_some(log),
        artifacts,
    }
}

#[cfg(test)]
mod tests {
    use super::supported_rust;

    #[test]
    fn accepts_only_required_stable_release() {
        assert!(supported_rust("rustc 1.97.0 (abc 2026-10-01)"));
        for version in [
            "rustc 1.96.0",
            "rustc 1.970.0",
            "rustc 1.97.0-nightly",
            "rustc 1.97.0-beta.1",
            "cargo 1.97.0",
            "",
        ] {
            assert!(!supported_rust(version), "{version}");
        }
    }
}
