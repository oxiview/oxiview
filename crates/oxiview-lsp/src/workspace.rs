use crate::document::Document;
use oxiview_syntax::{Declaration, Kind, Source};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
use url::Url;

pub(crate) fn file_path(uri: &str) -> Option<PathBuf> {
    let path = Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(normalize(&path))
}

pub(crate) fn file_uri(path: &Path) -> Option<String> {
    Url::from_file_path(path).ok().map(Into::into)
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

#[derive(Default)]
pub(crate) struct Workspace {
    roots: BTreeMap<PathBuf, bool>,
    packages: BTreeMap<PathBuf, String>,
    cargo: cargo::Cargo,
    pub files: BTreeMap<PathBuf, Entry>,
    sites: BTreeMap<PathBuf, Vec<Site>>,
    exports: BTreeMap<(Context, String), Vec<Slot>>,
    aliases: BTreeMap<(Context, String), Vec<(Context, String)>>,
    externs: BTreeMap<(Context, String), String>,
    members: BTreeMap<String, Vec<Slot>>,
    refreshed: Option<Instant>,
}

pub(crate) struct Entry {
    pub doc: Document,
    pub version: Option<i64>,
    stamp: Option<(SystemTime, u64)>,
}

#[derive(Clone)]
struct Site {
    host: PathBuf,
    module: Vec<String>,
    offset: usize,
    mode: Source,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Context {
    package: PathBuf,
    module: Vec<String>,
    root_len: usize,
}

pub(crate) type Target<'a> = (&'a Path, &'a Document, &'a Declaration);
type Slot = (PathBuf, usize);

mod cargo;
mod navigation;

fn manifest(path: &Path) -> Option<toml::Value> {
    toml::from_str(&fs::read_to_string(path.join("Cargo.toml")).ok()?).ok()
}

impl Workspace {
    pub fn root(&mut self, dir: &Path) {
        let mut package = None;
        for parent in dir.ancestors() {
            if let Some(cargo) = manifest(parent) {
                package.get_or_insert_with(|| parent.to_owned());
                if cargo.get("workspace").is_some() {
                    package = Some(parent.to_owned());
                    break;
                }
            }
        }
        let recursive = package.is_some();
        let root = package.unwrap_or_else(|| dir.to_owned());
        if self.roots.insert(root, recursive).is_none() {
            self.refreshed = None;
        }
    }

    pub fn refresh(&mut self, force: bool) {
        if !force
            && self
                .refreshed
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.refreshed = Some(Instant::now());
        let mut paths = BTreeSet::new();
        let mut packages = BTreeMap::new();
        for (root, recursive) in &self.roots {
            collect(root, *recursive, &mut paths, &mut packages);
        }
        self.cargo = cargo::discover(&mut paths, &mut packages);
        self.packages = packages;
        self.files
            .retain(|path, entry| entry.version.is_some() || paths.contains(path));
        for path in &paths {
            if self.files.get(path).is_some_and(|e| e.version.is_some()) {
                continue;
            }
            let stamp = fs::metadata(path)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len())));
            if self.files.get(path).is_some_and(|e| e.stamp == stamp) {
                continue;
            }
            if let Ok(text) = fs::read_to_string(path) {
                let doc = Document::new(path, text, self.mode(path));
                self.files.insert(
                    path.clone(),
                    Entry {
                        doc,
                        version: None,
                        stamp,
                    },
                );
            }
        }
        self.index_sites();
    }

    fn package<'a>(&'a self, path: &'a Path) -> &'a Path {
        path.ancestors()
            .find(|p| self.packages.contains_key(*p))
            .or_else(|| path.parent())
            .unwrap_or(path)
    }

    fn index_sites(&mut self) {
        let mut sites: BTreeMap<PathBuf, Vec<Site>> = BTreeMap::new();
        for (host, entry) in &self.files {
            for include in &entry.doc.facts.inclusions {
                let target = normalize(&self.package(host).join(&include.path));
                sites.entry(target).or_default().push(Site {
                    host: host.clone(),
                    module: include.module.clone(),
                    offset: include.range.start,
                    mode: include.mode,
                });
            }
        }
        self.sites = sites;
        self.index_declarations();
    }

    fn index_declarations(&mut self) {
        let mut exports = BTreeMap::<_, Vec<_>>::new();
        let mut aliases = BTreeMap::<_, Vec<_>>::new();
        let mut members = BTreeMap::<_, Vec<_>>::new();
        self.externs.clear();
        for (path, entry) in &self.files {
            let externs: Vec<_> = entry
                .doc
                .facts
                .imports
                .iter()
                .filter(|i| i.extern_crate && i.module.is_empty())
                .collect();
            if externs.is_empty() {
                continue;
            }
            for ctx in self.contexts(path, &mut BTreeSet::new()) {
                for i in &externs {
                    if let Some(source) = i.path.first() {
                        self.externs
                            .insert((ctx.clone(), i.name.clone()), source.clone());
                    }
                }
            }
        }
        for (path, entry) in &self.files {
            let contexts = self.contexts(path, &mut BTreeSet::new());
            for (i, decl) in entry.doc.facts.declarations.iter().enumerate() {
                if decl.owner.is_some() {
                    members
                        .entry(decl.name.clone())
                        .or_default()
                        .push((path.clone(), i));
                } else if decl.kind != Kind::Local && decl.scope == (0..entry.doc.text.len()) {
                    for ctx in &contexts {
                        let mut ctx = ctx.clone();
                        ctx.module.extend_from_slice(&decl.module);
                        exports
                            .entry((ctx, decl.name.clone()))
                            .or_default()
                            .push((path.clone(), i));
                    }
                }
            }
            for import in &entry.doc.facts.imports {
                if import.scope != (0..entry.doc.text.len()) || import.name == "*" {
                    continue;
                }
                for ctx in &contexts {
                    let mut ctx = ctx.clone();
                    ctx.module.extend_from_slice(&import.module);
                    if let Some(target) = self.qualify(ctx.clone(), &import.path) {
                        aliases
                            .entry((ctx, import.name.clone()))
                            .or_default()
                            .push(target);
                    }
                }
            }
        }
        self.exports = exports;
        self.aliases = aliases;
        self.members = members;
    }

    fn target(&self, slot: &Slot) -> Option<Target<'_>> {
        let (path, entry) = self.files.get_key_value(&slot.0)?;
        Some((path, &entry.doc, entry.doc.facts.declarations.get(slot.1)?))
    }

    fn mode(&self, path: &Path) -> Option<Source> {
        self.sites.get(path)?.first().map(|site| site.mode)
    }

    pub fn open(&mut self, path: PathBuf, text: String, version: i64) {
        if let Some(parent) = path.parent() {
            self.root(parent);
        }
        self.refresh(false);
        self.update(path, text, version);
    }

    pub fn update(&mut self, path: PathBuf, text: String, version: i64) {
        let doc = Document::new(&path, text, self.mode(&path));
        self.files.insert(
            path,
            Entry {
                doc,
                version: Some(version),
                stamp: None,
            },
        );
        self.index_sites();
    }

    pub fn close(&mut self, path: &Path) {
        self.files.remove(path);
        self.refresh(true);
    }

    fn contexts(&self, path: &Path, visited: &mut BTreeSet<PathBuf>) -> Vec<Context> {
        if !visited.insert(path.to_owned()) {
            return Vec::new();
        }
        let mut result = Vec::new();
        for site in self
            .sites
            .get(path)
            .into_iter()
            .flatten()
            .filter(|s| s.mode == Source::Module)
        {
            for mut context in self.contexts(&site.host, visited) {
                context.module.extend_from_slice(&site.module);
                result.push(context);
            }
        }
        visited.remove(path);
        if result.is_empty() {
            let package = self.package(path);
            let file = path.strip_prefix(package).unwrap_or(path);
            let library = self.cargo.library_roots.get(package);
            let relative = if library.is_some_and(|entry| entry == path) {
                Path::new("")
            } else {
                library
                    .and_then(|entry| path.strip_prefix(entry.parent()?).ok())
                    .unwrap_or_else(|| file.strip_prefix("src").unwrap_or(file))
            };
            let mut module: Vec<_> = relative
                .with_extension("")
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            if module
                .last()
                .is_some_and(|s| matches!(s.as_str(), "lib" | "main" | "mod"))
            {
                module.pop();
            }
            let root_len = if file.starts_with("tests")
                || file.starts_with("examples")
                || file.starts_with("benches")
                || file.starts_with("src/bin")
            {
                2.min(module.len())
            } else {
                0
            };
            result.push(Context {
                package: package.into(),
                module,
                root_len,
            });
        }
        result
    }

    fn local_module(doc: &Document, pos: usize) -> Vec<String> {
        doc.facts
            .declarations
            .iter()
            .filter(|d| d.kind == Kind::Module && d.extent.contains(&pos))
            .max_by_key(|d| d.module.len())
            .map(|d| {
                let mut module = d.module.clone();
                module.push(d.name.clone());
                module
            })
            .unwrap_or_default()
    }

    fn qualify(&self, mut context: Context, path: &[String]) -> Option<(Context, String)> {
        let (name, prefix) = path.split_last()?;
        let mut segments = prefix.iter().peekable();
        if let Some(first) = segments.peek() {
            let mut root = context.clone();
            root.module.truncate(root.root_len);
            let external = self
                .externs
                .get(&(root.clone(), first.to_string()))
                .unwrap_or(first);
            if external == "crate" {
                context = root;
                segments.next();
            } else if let Some(package) = self
                .cargo
                .dependencies
                .get(&context.package)
                .and_then(|deps| deps.get(external))
                .or_else(|| {
                    self.packages
                        .iter()
                        .find(|(_, n)| *n == external)
                        .map(|(p, _)| p)
                })
            {
                context.package = package.clone();
                context.module.clear();
                context.root_len = 0;
                segments.next();
            }
        }
        for segment in segments {
            match segment.as_str() {
                "self" => {}
                "super" => {
                    if context.module.len() == context.root_len {
                        return None;
                    }
                    context.module.pop()?;
                }
                _ => context.module.push(segment.clone()),
            }
        }
        Some((context, name.clone()))
    }

    fn exported<'a>(
        &'a self,
        context: &Context,
        name: &str,
        seen: &mut BTreeSet<(Context, String)>,
    ) -> Vec<Target<'a>> {
        let key = (context.clone(), name.to_owned());
        if !seen.insert(key.clone()) {
            return Vec::new();
        }
        let mut targets: Vec<_> = self
            .exports
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(|s| self.target(s))
            .collect();
        for (next, leaf) in self.aliases.get(&key).into_iter().flatten() {
            targets.extend(self.exported(next, leaf, seen));
        }
        targets
    }

    pub fn resolve(&self, path: &Path, pos: usize, segments: &[String]) -> Vec<Target<'_>> {
        let mut targets = self.resolve_at(path, pos, segments, &mut BTreeSet::new());
        targets.sort_by_key(|(p, _, d)| (*p, d.selection.start));
        targets.dedup_by_key(|(p, _, d)| (*p, d.selection.start));
        targets
    }

    fn resolve_at<'a>(
        &'a self,
        path: &Path,
        pos: usize,
        segments: &[String],
        seen: &mut BTreeSet<(PathBuf, usize)>,
    ) -> Vec<Target<'a>> {
        if !seen.insert((path.to_owned(), pos)) {
            return Vec::new();
        }
        let Some((path, entry)) = self.files.get_key_value(path) else {
            return Vec::new();
        };
        let Some(head) = segments.first() else {
            return Vec::new();
        };
        let doc = &entry.doc;
        if segments.len() == 1 {
            let local = doc
                .facts
                .declarations
                .iter()
                .filter(|d| {
                    d.name == *head
                        && (d.selection.contains(&pos)
                            || (d.owner.is_none()
                                && d.scope != (0..doc.text.len())
                                && d.scope.contains(&pos)))
                })
                .max_by_key(|d| (d.selection.contains(&pos), d.scope.start, d.selection.start));
            if let Some(d) = local {
                return vec![(path, doc, d)];
            }
        }
        let mut targets = Vec::new();
        // Each embedding has its own bindings, even at two sites in the same Rust file.
        for site in self
            .sites
            .get(path)
            .into_iter()
            .flatten()
            .filter(|s| s.mode == Source::Fragment)
        {
            targets.extend(self.resolve_at(&site.host, site.offset, segments, seen));
        }
        if !targets.is_empty() {
            return targets;
        }
        let module = Self::local_module(doc, pos);
        for mut base in self.contexts(path, &mut BTreeSet::new()) {
            base.module.extend_from_slice(&module);
            let mut paths = Vec::new();
            for import in &doc.facts.imports {
                if import.module == module
                    && (import.scope == (0..doc.text.len()) || import.scope.contains(&pos))
                    && (import.name == *head || import.name == "*")
                {
                    let mut expanded = import.path.clone();
                    expanded.extend_from_slice(if import.name == "*" {
                        segments
                    } else {
                        &segments[1..]
                    });
                    paths.push(expanded);
                }
            }
            paths.push(segments.to_vec());
            for candidate in paths {
                if let Some((context, name)) = self.qualify(base.clone(), &candidate) {
                    targets.extend(self.exported(&context, &name, &mut BTreeSet::new()));
                }
                if !targets.is_empty() {
                    break;
                }
            }
        }
        targets
    }

    pub fn visible_names(&self, path: &Path, pos: usize) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let mut sites = vec![(path, pos)];
        for site in self
            .sites
            .get(path)
            .into_iter()
            .flatten()
            .filter(|s| s.mode == Source::Fragment)
        {
            sites.push((&site.host, site.offset));
        }
        for (path, pos) in sites {
            if let Some(entry) = self.files.get(path) {
                let module = Self::local_module(&entry.doc, pos);
                names.extend(
                    entry
                        .doc
                        .facts
                        .declarations
                        .iter()
                        .filter(|d| {
                            d.owner.is_none()
                                && d.module == module
                                && (d.scope == (0..entry.doc.text.len()) || d.scope.contains(&pos))
                        })
                        .map(|d| d.name.clone()),
                );
                names.extend(
                    entry
                        .doc
                        .facts
                        .imports
                        .iter()
                        .filter(|i| {
                            i.module == module
                                && (i.scope == (0..entry.doc.text.len()) || i.scope.contains(&pos))
                                && i.name != "*"
                        })
                        .map(|i| i.name.clone()),
                );
            }
        }
        names
    }

    pub fn included_file(&self, path: &Path, pos: usize) -> Option<PathBuf> {
        let include = self
            .files
            .get(path)?
            .doc
            .facts
            .inclusions
            .iter()
            .find(|i| i.range.contains(&pos))?;
        let target = normalize(&self.package(path).join(&include.path));
        target.is_file().then_some(target)
    }
}

fn collect(
    dir: &Path,
    recursive: bool,
    paths: &mut BTreeSet<PathBuf>,
    packages: &mut BTreeMap<PathBuf, String>,
) {
    if let Some(cargo) = manifest(dir)
        && let Some(name) = cargo::package_name(&cargo)
    {
        packages.insert(dir.to_owned(), name.replace('-', "_"));
    }
    let Ok(children) = dir.read_dir() else { return };
    for file in children.flatten() {
        let Ok(kind) = file.file_type() else { continue };
        let path = file.path();
        let name = file.file_name();
        let name = name.to_string_lossy();
        if kind.is_dir()
            && recursive
            && !name.starts_with('.')
            && !matches!(&*name, "target" | "node_modules" | "vendor")
        {
            collect(&path, true, paths, packages);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "rs" || ext == "oxi")
        {
            paths.insert(path);
        }
    }
}
