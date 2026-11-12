//! 数据驱动的依赖方向检查与 cargo-deny 审查。

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
    // Cargo 返回真实包名，避免 rename 绕过规则。
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
    // 无需解析传递依赖：检查所有直接声明，含未启用 feature 和其他目标平台。
    let output = cargo(root, &["metadata", "--format-version", "1", "--no-deps"])
        .map_err(|error| format!("无法运行 cargo metadata：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("metadata JSON 无效：{error}"))
}

pub(crate) fn read_rules(root: &Path) -> Result<Rules, String> {
    let source = fs::read_to_string(root.join("xtask/layers.toml"))
        .map_err(|error| format!("无法读取 layers.toml：{error}"))?;
    let rules: Rules =
        toml::from_str(&source).map_err(|error| format!("layers.toml 无效：{error}"))?;
    for (name, layer) in &rules.crates {
        if layer.context.is_empty() || layer.role.is_empty() {
            return Err(format!("layers.toml：{name} 缺少 context 或 role"));
        }
        for dependency in &layer.allow {
            if !rules.crates.contains_key(dependency) {
                return Err(format!(
                    "layers.toml：{name}.allow 引用了未登记的 {dependency}"
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
                "{}：违反强制登记规则，未在 layers.toml 登记",
                package.name
            ));
        }
        for dependency in &package.dependencies {
            let kind = dependency.kind.as_deref().unwrap_or("normal");
            let edge = format!(
                "{} -> {}（{kind}，目标 {}）",
                package.name,
                dependency.name,
                dependency.target.as_deref().unwrap_or("全部")
            );
            // 禁止依赖工程工具的规则即使对豁免成员也生效。
            if rules.forbidden.contains(&dependency.name) {
                errors.push(format!("{edge}：违反 forbidden 规则"));
                continue;
            }
            if exempt {
                continue;
            }
            if rules.dev_only.contains(&dependency.name) {
                if kind != "dev" {
                    errors.push(format!("{edge}：违反 dev-only 规则，只允许 dev-dependency"));
                }
                continue;
            }
            if members.iter().any(|member| member.name == dependency.name)
                && !layer.is_some_and(|layer| layer.allow.contains(&dependency.name))
            {
                errors.push(format!(
                    "{edge}：违反 crates.{}.allow 依赖方向规则",
                    package.name
                ));
            }
            for rule in &rules.external {
                if rule
                    .packages
                    .iter()
                    .any(|pattern| matches_package(pattern, &dependency.name))
                    && !rule.allow.contains(&package.name)
                {
                    errors.push(format!(
                        "{edge}：违反 external 规则 {:?}，仅允许 {:?}",
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
    // 先显式刷新数据库，将网络/缓存权限错误与发现安全公告区别开。
    // 随后离线检查同一份数据库，避免二次下载失败被误判为违规。
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
                    format!("{log}无法执行 cargo-deny：{error}；请运行 cargo xtask doctor。"),
                );
            }
        };
        log.push_str(&String::from_utf8_lossy(&output.stdout));
        log.push_str(&String::from_utf8_lossy(&output.stderr));
        let status = deny_status(output.status.success(), fetched, output.status.code());
        if status != Status::Pass {
            if !fetched {
                log.push_str("\n无法获取安全公告数据库：请检查网络、代理和数据库缓存权限后重试；未跳过安全审查。\n");
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
            Ok(errors) if errors.is_empty() => (Status::Pass, "依赖方向检查通过。\n".into()),
            Ok(errors) => (Status::Fail, format!("{}\n", errors.join("\n"))),
            Err(error) => (Status::Error, format!("{error}\n")),
        };
        // 方向违规也继续审查，保留全部诊断。
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
