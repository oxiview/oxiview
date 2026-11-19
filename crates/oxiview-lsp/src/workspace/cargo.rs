use super::{collect, manifest, normalize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use toml::Value;

#[derive(Default)]
pub(super) struct Cargo {
    pub(super) dependencies: BTreeMap<PathBuf, BTreeMap<String, PathBuf>>,
    pub(super) library_roots: BTreeMap<PathBuf, PathBuf>,
}

pub(super) fn package_name(manifest: &Value) -> Option<&str> {
    manifest
        .get("lib")
        .and_then(|lib| lib.get("name"))
        .or_else(|| manifest.get("package")?.get("name"))?
        .as_str()
}

fn library(dir: &Path, manifest: &Value) -> Option<PathBuf> {
    let path = manifest.get("lib")?.get("path")?.as_str()?;
    Some(normalize(&dir.join(path)))
}

/// Discover local SDK sources through Cargo manifests without running a build.
pub(super) fn discover(
    sources: &mut BTreeSet<PathBuf>,
    packages: &mut BTreeMap<PathBuf, String>,
) -> Cargo {
    let mut result = Cargo::default();
    let mut pending: Vec<_> = packages.keys().cloned().collect();
    while let Some(dir) = pending.pop() {
        if result.dependencies.contains_key(&dir) {
            continue;
        }
        let Some(cargo) = manifest(&dir) else {
            continue;
        };
        if let Some(entry) = library(&dir, &cargo) {
            result.library_roots.insert(dir.clone(), entry);
        }
        let deps = dependencies(&dir, &cargo);
        for dep in deps.values() {
            if packages.contains_key(dep) {
                continue;
            }
            let Some(metadata) = manifest(dep) else {
                continue;
            };
            let Some(name) = package_name(&metadata) else {
                continue;
            };
            packages.insert(dep.clone(), name.replace('-', "_"));
            let entry = library(dep, &metadata).unwrap_or_else(|| dep.join("src/lib.rs"));
            if let Some(src) = entry.parent() {
                collect(src, true, sources, packages);
            }
            pending.push(dep.clone());
        }
        result.dependencies.insert(dir, deps);
    }
    result
}

fn dependencies(dir: &Path, cargo: &Value) -> BTreeMap<String, PathBuf> {
    let inherited = workspace_dependencies(dir, cargo);
    let targets = cargo
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|t| t.values());
    let mut deps = BTreeMap::new();
    for table in std::iter::once(cargo).chain(targets) {
        for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
            for (alias, dep) in table
                .get(kind)
                .and_then(Value::as_table)
                .into_iter()
                .flatten()
            {
                let path = if dep.get("workspace").and_then(Value::as_bool) == Some(true) {
                    inherited.as_ref().and_then(|(root, shared)| {
                        Some(root.join(shared.get(alias)?.get("path")?.as_str()?))
                    })
                } else {
                    dep.get("path").and_then(Value::as_str).map(|p| dir.join(p))
                };
                if let Some(path) = path {
                    deps.insert(alias.replace('-', "_"), normalize(&path));
                }
            }
        }
    }
    deps
}

fn workspace_dependencies(dir: &Path, cargo: &Value) -> Option<(PathBuf, toml::Table)> {
    let explicit = cargo
        .get("package")
        .and_then(|p| p.get("workspace"))
        .and_then(Value::as_str)
        .map(|p| dir.join(p));
    for root in explicit.as_deref().into_iter().chain(dir.ancestors()) {
        if let Some(root_manifest) = manifest(root)
            && let Some(workspace) = root_manifest.get("workspace")
        {
            return Some((
                root.to_owned(),
                workspace.get("dependencies")?.as_table()?.clone(),
            ));
        }
    }
    None
}
