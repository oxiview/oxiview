use super::{Target, Workspace};
use oxiview_syntax::{Binding, BoundUse, Kind, MemberUse, SourcePath, ValueSource};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

type Visiting = BTreeSet<(PathBuf, usize)>;

impl Workspace {
    fn path_targets(&self, file: &Path, path: &SourcePath) -> Vec<Target<'_>> {
        let named = self.resolve(file, path.at, &path.segments);
        if !named.is_empty() {
            return named;
        }
        let Some((name, owner)) = path.segments.split_last() else {
            return Vec::new();
        };
        if owner.is_empty() {
            return Vec::new();
        }
        self.member_targets(&self.resolve(file, path.at, owner), name, Kind::Method)
    }

    pub(crate) fn binding_at(&self, file: &Path, pos: usize) -> Option<&BoundUse> {
        let doc = &self.files.get(file)?.doc;
        let (_, bound) = doc.facts.bindings.range(..=pos).next_back()?;
        (bound.range.contains(&pos)
            || (pos == bound.range.end && doc.word(pos).is_some_and(|word| word.end == pos)))
        .then_some(bound)
    }

    /// Prefer the operation emitted by the compiler to a same-spelled lexical name.
    pub(crate) fn targets_at(&self, file: &Path, pos: usize) -> Vec<Target<'_>> {
        if let Some(bound) = self.binding_at(file, pos) {
            return self.binding_targets(file, &bound.binding, &mut Visiting::new());
        }
        let Some(doc) = self.files.get(file).map(|e| &e.doc) else {
            return Vec::new();
        };
        let Some(word) = doc.name_at(pos) else {
            return Vec::new();
        };
        self.path_targets(
            file,
            &SourcePath {
                segments: doc.path_at(&word),
                at: pos,
            },
        )
    }

    fn binding_targets(
        &self,
        file: &Path,
        binding: &Binding,
        visiting: &mut Visiting,
    ) -> Vec<Target<'_>> {
        match binding {
            Binding::Item(path) => self.path_targets(file, path),
            Binding::Props(path) => self
                .path_targets(file, path)
                .into_iter()
                .map(|target| first_input(target).unwrap_or(target))
                .collect(),
            Binding::Member {
                receiver,
                name,
                usage,
            } => {
                let owners = self.value_types(file, receiver, visiting);
                self.member_targets(&owners, name, usage.declaration())
            }
        }
    }

    fn member_targets(&self, owners: &[Target<'_>], name: &str, kind: Kind) -> Vec<Target<'_>> {
        if owners.is_empty() {
            return Vec::new();
        }
        self.members
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|slot| self.target(slot))
            .filter(|(path, _, decl)| {
                decl.kind == kind
                    && decl.owner.as_ref().is_some_and(|owner| {
                        self.resolve(path, owner.at, &owner.segments)
                            .iter()
                            .any(|(p, _, d)| {
                                owners
                                    .iter()
                                    .any(|(file, _, ty)| p == file && d.selection == ty.selection)
                            })
                    })
            })
            .collect()
    }

    fn value_types(
        &self,
        file: &Path,
        value: &ValueSource,
        visiting: &mut Visiting,
    ) -> Vec<Target<'_>> {
        let definitions = match value {
            ValueSource::Type(path) => {
                return self
                    .path_targets(file, path)
                    .into_iter()
                    .filter(|(_, _, d)| d.kind == Kind::Type)
                    .collect();
            }
            ValueSource::Name(path) if path.segments == ["self"] => {
                let owner = self
                    .files
                    .get(file)
                    .and_then(|entry| {
                        entry
                            .doc
                            .facts
                            .declarations
                            .iter()
                            .filter(|d| d.kind == Kind::Method && d.extent.contains(&path.at))
                            .min_by_key(|d| d.extent.len())
                    })
                    .and_then(|d| d.owner.as_ref());
                return owner.map_or_else(Vec::new, |owner| self.path_targets(file, owner));
            }
            ValueSource::Name(path) | ValueSource::Call(path) => self.path_targets(file, path),
            ValueSource::Member(offset) => {
                let Some(bound) = self
                    .files
                    .get(file)
                    .and_then(|e| e.doc.facts.bindings.get(offset))
                else {
                    return Vec::new();
                };
                let key = (file.to_owned(), *offset);
                if !visiting.insert(key.clone()) {
                    return Vec::new();
                }
                let targets = self.binding_targets(file, &bound.binding, visiting);
                visiting.remove(&key);
                targets
            }
        };
        definitions
            .into_iter()
            .flat_map(|target| self.declared_type(target, visiting))
            .collect()
    }

    fn declared_type<'a>(&'a self, target: Target<'a>, visiting: &mut Visiting) -> Vec<Target<'a>> {
        let (file, _, decl) = target;
        if decl.kind == Kind::Type {
            return vec![target];
        }
        let Some(value) = &decl.value else {
            return Vec::new();
        };
        if let ValueSource::Type(p) = value
            && p.segments == ["Self"]
            && let Some(owner) = &decl.owner
        {
            return self.path_targets(file, owner);
        }
        let key = (file.to_owned(), decl.selection.start);
        if !visiting.insert(key.clone()) {
            return Vec::new();
        }
        let result = self.value_types(file, value, visiting);
        visiting.remove(&key);
        result
    }

    pub(crate) fn implementations_at(&self, file: &Path, pos: usize) -> Vec<Target<'_>> {
        if let Some(BoundUse {
            binding: Binding::Props(path),
            ..
        }) = self.binding_at(file, pos)
        {
            return self.path_targets(file, path);
        }
        self.targets_at(file, pos)
            .into_iter()
            .filter(|(_, _, d)| {
                matches!(
                    d.kind,
                    Kind::View | Kind::Style | Kind::Function | Kind::Method | Kind::Field
                )
            })
            .collect()
    }

    pub(crate) fn types_at(&self, file: &Path, pos: usize) -> Vec<Target<'_>> {
        let attribute = self.binding_at(file, pos).is_some_and(|bound| {
            matches!(
                bound.binding,
                Binding::Member {
                    usage: MemberUse::Attribute,
                    ..
                }
            )
        });
        self.targets_at(file, pos)
            .into_iter()
            .filter_map(|target| {
                if attribute {
                    first_input(target)
                } else {
                    Some(target)
                }
            })
            .flat_map(|target| self.declared_type(target, &mut Visiting::new()))
            .collect()
    }
}

fn first_input((path, doc, decl): Target<'_>) -> Option<Target<'_>> {
    let input = decl.input.as_ref()?;
    let param = doc
        .facts
        .declarations
        .iter()
        .find(|d| d.kind == Kind::Local && d.selection == *input)?;
    Some((path, doc, param))
}
