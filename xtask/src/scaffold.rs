//! Create crate skeletons only for registered contexts.

use std::{fs, path::Path};

use crate::{check::Check, deps::read_rules, report::Status};

pub(crate) struct NewCrate {
    pub name: String,
    pub context: String,
}

impl Check for NewCrate {
    fn name(&self) -> &str {
        "new-crate"
    }

    fn run(&self, root: &Path) -> (Status, String) {
        match self.generate(root) {
            Ok(()) => (
                Status::Pass,
                format!("Created crates/{}; run cargo xtask verify.\n", self.name),
            ),
            // Argument errors, target conflicts, and filesystem errors prevent execution rather than indicate check violations.
            Err(error) => (Status::Error, error),
        }
    }
}

impl NewCrate {
    fn generate(&self, root: &Path) -> Result<(), String> {
        let name = &self.name;
        if !(name == "oxiview" || (name.starts_with("oxiview-") && name.len() > 8))
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(
                "The name must be oxiview or a lowercase crate name prefixed with oxiview-.".into(),
            );
        }
        let rules = read_rules(root)?;
        let layer = rules
            .crates
            .get(name)
            .ok_or_else(|| format!("{name} is not registered in layers.toml; discuss context and dependency rules first."))?;
        if layer.context != self.context {
            return Err(format!(
                "{name} requires context {:?}, received {:?}",
                layer.context, self.context
            ));
        }
        let directory = root.join("crates").join(name);
        fs::create_dir_all(root.join("crates")).map_err(|error| error.to_string())?;
        // create_dir atomically rejects existing directories, files, and symlinks; do not overwrite with create_dir_all.
        fs::create_dir(&directory).map_err(|error| {
            format!(
                "Refusing to overwrite or unable to create {}: {error}",
                directory.display()
            )
        })?;
        let result = (|| -> std::io::Result<()> {
            fs::create_dir(directory.join("src"))?;
            fs::write(
                directory.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\npublish.workspace = true\n\n[lints]\nworkspace = true\n"
                ),
            )?;
            // Prefix every line as documentation so newlines in rules cannot become source code.
            let documentation = format!(
                "{name}: context {}, role {} (see xtask/layers.toml).",
                layer.context, layer.role
            )
            .lines()
            .map(|line| format!("//! {line}\n"))
            .collect::<String>();
            fs::write(directory.join("src/lib.rs"), documentation)
        })();
        if let Err(error) = result {
            return match fs::remove_dir_all(&directory) {
                Ok(()) => Err(format!(
                    "Generation failed; the new directory was removed: {error}"
                )),
                Err(cleanup) => Err(format!(
                    "Generation failed: {error}; cleanup of {} failed: {cleanup}",
                    directory.display()
                )),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::tests::Fixture;

    fn setup() -> Fixture {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("xtask")).expect("create rules directory");
        fs::write(
            fixture.0.join("xtask/layers.toml"),
            include_str!("../layers.toml"),
        )
        .expect("write rules");
        fixture
    }

    fn task(name: &str, context: &str) -> NewCrate {
        NewCrate {
            name: name.into(),
            context: context.into(),
        }
    }

    #[test]
    fn generates_only_manifest_and_documented_library() {
        let fixture = setup();
        assert_eq!(
            task("oxiview-base", "shared-foundation").run(&fixture.0).0,
            Status::Pass
        );
        let root = fixture.0.join("crates/oxiview-base");
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).expect("read manifest"))
                .expect("parse manifest");
        assert_eq!(manifest["package"]["name"].as_str(), Some("oxiview-base"));
        for field in ["version", "edition", "rust-version", "license", "publish"] {
            assert_eq!(
                manifest["package"][field]["workspace"].as_bool(),
                Some(true)
            );
        }
        assert_eq!(manifest["lints"]["workspace"].as_bool(), Some(true));
        let source = fs::read_to_string(root.join("src/lib.rs")).expect("read library");
        assert!(
            source.starts_with("//! ")
                && source.contains("shared-foundation")
                && source.contains("foundation")
        );
        assert_eq!(fs::read_dir(&root).expect("list directory").count(), 2);
        assert_eq!(
            fs::read_dir(root.join("src"))
                .expect("list source files")
                .count(),
            1
        );
    }

    #[test]
    fn rejects_unknown_name_and_mismatched_context_without_writes() {
        let fixture = setup();
        for (name, context) in [
            ("oxiview-unknown", "shared-foundation"),
            ("oxiview-base", "platform"),
            ("xtask", "tools"),
            ("oxiview-../../escape", "shared-foundation"),
        ] {
            assert_eq!(task(name, context).run(&fixture.0).0, Status::Error);
            assert!(!fixture.0.join("crates").exists());
        }
    }

    #[test]
    fn refuses_existing_directory_without_changing_contents() {
        let fixture = setup();
        let directory = fixture.0.join("crates/oxiview-base");
        fs::create_dir_all(&directory).expect("create target");
        fs::write(directory.join("keep"), "keep").expect("write existing content");
        assert_eq!(
            task("oxiview-base", "shared-foundation").run(&fixture.0).0,
            Status::Error
        );
        assert_eq!(
            fs::read_to_string(directory.join("keep")).expect("read content"),
            "keep"
        );
        assert_eq!(fs::read_dir(directory).expect("list directory").count(), 1);
    }
}
