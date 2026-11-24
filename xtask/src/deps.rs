//! Data-driven dependency-direction checks and cargo-deny review.

use std::{collections::BTreeMap, fs, path::Path, process::Command};

use serde::Deserialize;

use crate::{check::Check, report::Status};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Rules {
    exempt: Vec<String>,
    forbidden: Vec<String>,
    dev_only: Vec<String>,
    pub(crate) crates: BTreeMap<String, Layer>,
    external: Vec<ExternalRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Layer {
    pub(crate) context: String,
    pub(crate) role: String,
    allow: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalRule {
    packages: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
    allow: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    // Cargo returns real package names so renames cannot bypass rules.
    name: String,
    kind: Option<String>,
    target: Option<String>,
}

fn cargo(root: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(args)
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .output()
}

fn metadata(root: &Path) -> Result<Metadata, String> {
    // No transitive resolution is needed: check all direct declarations, including disabled features and other targets.
    let output = cargo(root, &["metadata", "--format-version", "1", "--no-deps"])
        .map_err(|error| format!("Cannot run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid metadata JSON: {error}"))
}

pub(crate) fn read_rules(root: &Path) -> Result<Rules, String> {
    let source = fs::read_to_string(root.join("xtask/layers.toml"))
        .map_err(|error| format!("Cannot read layers.toml: {error}"))?;
    let rules: Rules =
        toml::from_str(&source).map_err(|error| format!("Invalid layers.toml: {error}"))?;
    for (name, layer) in &rules.crates {
        if layer.context.is_empty() || layer.role.is_empty() {
            return Err(format!("layers.toml: {name} is missing context or role"));
        }
        for dependency in &layer.allow {
            if !rules.crates.contains_key(dependency) {
                return Err(format!(
                    "layers.toml: {name}.allow references unregistered {dependency}"
                ));
            }
        }
    }
    Ok(rules)
}

fn matches_package(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

fn violations(metadata: &Metadata, rules: &Rules) -> Vec<String> {
    let members: Vec<_> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect();
    let mut errors = Vec::new();
    for package in &members {
        let exempt = rules.exempt.contains(&package.name);
        let layer = rules.crates.get(&package.name);
        if layer.is_none() && !exempt {
            errors.push(format!(
                "{}: violates mandatory registration; not registered in layers.toml",
                package.name
            ));
        }
        for dependency in &package.dependencies {
            let kind = dependency.kind.as_deref().unwrap_or("normal");
            let edge = format!(
                "{} -> {} ({kind}, target {})",
                package.name,
                dependency.name,
                dependency.target.as_deref().unwrap_or("all")
            );
            // The ban on engineering-tool dependencies applies even to exempt members.
            if rules.forbidden.contains(&dependency.name) {
                errors.push(format!("{edge}: violates the forbidden rule"));
                continue;
            }
            if exempt {
                continue;
            }
            if rules.dev_only.contains(&dependency.name) {
                if kind != "dev" {
                    errors.push(format!(
                        "{edge}: violates the dev-only rule; only dev-dependencies are allowed"
                    ));
                }
                continue;
            }
            if members.iter().any(|member| member.name == dependency.name)
                && !layer.is_some_and(|layer| layer.allow.contains(&dependency.name))
            {
                errors.push(format!(
                    "{edge}: violates the crates.{}.allow dependency-direction rule",
                    package.name
                ));
            }
            for rule in &rules.external {
                if rule
                    .packages
                    .iter()
                    .any(|pattern| matches_package(pattern, &dependency.name))
                    && !rule.except.contains(&dependency.name)
                    && !rule.allow.contains(&package.name)
                {
                    errors.push(format!(
                        "{edge}: violates external rule {:?}; only {:?} allowed",
                        rule.packages, rule.allow
                    ));
                }
            }
        }
    }
    errors.sort();
    errors
}

fn deny_status(success: bool, fetched: bool, code: Option<i32>) -> Status {
    if success {
        Status::Pass
    } else if !fetched || code.is_none() {
        Status::Error
    } else {
        Status::Fail
    }
}

fn audit(root: &Path) -> (Status, String) {
    // Explicitly refresh the database first to distinguish network/cache permission errors from advisory findings.
    // Then check the same database offline so a second download failure is not mistaken for a violation.
    let mut log = String::new();
    for (args, fetched) in [
        (vec!["deny", "fetch", "db"], false),
        (vec!["deny", "--offline", "check"], true),
    ] {
        log.push_str(&format!("cargo {}\n", args.join(" ")));
        let output = match cargo(root, &args) {
            Ok(output) => output,
            Err(error) => {
                return (
                    Status::Error,
                    format!("{log}Cannot execute cargo-deny: {error}; run cargo xtask doctor."),
                );
            }
        };
        log.push_str(&String::from_utf8_lossy(&output.stdout));
        log.push_str(&String::from_utf8_lossy(&output.stderr));
        let status = deny_status(output.status.success(), fetched, output.status.code());
        if status != Status::Pass {
            if !fetched {
                log.push_str("\nCannot fetch the advisory database: check networking, proxies, and cache permissions, then retry; security review was not skipped.\n");
            }
            return (status, log);
        }
    }
    (Status::Pass, log)
}

pub(crate) struct DepsCheck;

impl Check for DepsCheck {
    fn name(&self) -> &str {
        "deps"
    }

    fn run(&self, root: &Path) -> (Status, String) {
        let direction =
            read_rules(root).and_then(|rules| metadata(root).map(|data| violations(&data, &rules)));
        let (status, mut log) = match direction {
            Ok(errors) if errors.is_empty() => {
                (Status::Pass, "Dependency-direction checks passed.\n".into())
            }
            Ok(errors) => (Status::Fail, format!("{}\n", errors.join("\n"))),
            Err(error) => (Status::Error, format!("{error}\n")),
        };
        // Continue review after direction violations to retain all diagnostics.
        let (audit_status, audit_log) = audit(root);
        log.push_str(&audit_log);
        let combined = if [status, audit_status].contains(&Status::Error) {
            Status::Error
        } else if [status, audit_status].contains(&Status::Fail) {
            Status::Fail
        } else {
            Status::Pass
        };
        (combined, log)
    }
}

#[cfg(test)]
#[path = "deps_tests.rs"]
mod tests;
