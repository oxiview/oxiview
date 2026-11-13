//! Isolated temporary workspaces verify real Cargo metadata without network access.

use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(crates: &[(&str, &str)]) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let directory = root.join("target/xtask").join(format!(
            "deps-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("create temporary directory");
        let fixture = Self(directory);
        let members: Vec<_> = crates.iter().map(|(name, _)| *name).collect();
        fs::write(
            fixture.0.join("Cargo.toml"),
            format!(
                "[workspace]\nresolver = \"3\"\nmembers = {members:?}\nexclude = [\"external\"]\n"
            ),
        )
        .expect("write isolated workspace");
        for (name, dependencies) in crates {
            fixture.package(name, name, dependencies);
        }
        fixture
    }

    fn package(&self, path: &str, name: &str, dependencies: &str) {
        let directory = self.0.join(path);
        fs::create_dir_all(directory.join("src")).expect("create fixture crate");
        fs::write(directory.join("src/lib.rs"), "//! Test placeholder.\n")
            .expect("write fixture source");
        fs::write(directory.join("Cargo.toml"), format!("[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n{dependencies}\n")).expect("write fixture manifest");
    }

    fn errors(&self) -> Vec<String> {
        let rules = read_rules(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("repository root"),
        )
        .expect("read rules");
        violations(
            &metadata(&self.0).expect("read real Cargo metadata"),
            &rules,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compliant_dependencies_pass() {
    let fixture = Fixture::new(&[
        ("oxiview-base", ""),
        (
            "oxiview-scene",
            "[dependencies]\noxiview-base = { path = '../oxiview-base' }",
        ),
        (
            "oxiview-render-cpu",
            "[dependencies]\noxiview-scene = { path = '../oxiview-scene' }",
        ),
        (
            "oxiview-testing",
            "[dependencies]\noxiview-scene = { path = '../oxiview-scene' }",
        ),
        (
            "oxiview-app",
            "[dependencies]\noxiview-scene = { path = '../oxiview-scene' }\noxiview-render-cpu = { path = '../oxiview-render-cpu' }\n[dev-dependencies]\noxiview-testing = { path = '../oxiview-testing' }",
        ),
    ]);
    assert_eq!(fixture.errors(), Vec::<String>::new());
}

#[test]
fn lower_layer_cannot_depend_on_upper_layer() {
    let fixture = Fixture::new(&[
        (
            "oxiview-base",
            "[dependencies]\nupper = { package = 'oxiview-core', path = '../oxiview-core', optional = true }",
        ),
        ("oxiview-core", ""),
    ]);
    let errors = fixture.errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("oxiview-base -> oxiview-core (normal"));
    assert!(errors[0].contains("allow"));
}

#[test]
fn adapter_cannot_depend_on_another_adapter() {
    let fixture = Fixture::new(&[
        (
            "oxiview-render-cpu",
            "[build-dependencies]\noxiview-render-wgpu = { path = '../oxiview-render-wgpu' }",
        ),
        ("oxiview-render-wgpu", ""),
    ]);
    let errors = fixture.errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("oxiview-render-cpu -> oxiview-render-wgpu (build"));
}

#[test]
fn objc2_is_restricted_even_when_renamed_and_target_specific() {
    for name in ["objc2", "objc2-app-kit"] {
        let fixture = Fixture::new(&[(
            "oxiview-base",
            &format!(
                "[target.'cfg(target_os = \"macos\")'.dependencies]\nbinding = {{ package = '{name}', path = '../external', optional = true }}"
            ),
        )]);
        fixture.package("external", name, "[workspace]");
        let errors = fixture.errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("external"));
        assert!(errors[0].contains(name));
    }
}

#[test]
fn unregistered_workspace_member_is_rejected() {
    let fixture = Fixture::new(&[("oxiview-unknown", "")]);
    assert!(fixture.errors()[0].contains("not registered in layers.toml"));
}

#[test]
fn testing_is_only_a_dev_dependency() {
    for kind in ["dependencies", "build-dependencies"] {
        let fixture = Fixture::new(&[
            (
                "oxiview-base",
                &format!("[{kind}]\noxiview-testing = {{ path = '../oxiview-testing' }}"),
            ),
            ("oxiview-testing", ""),
        ]);
        assert!(fixture.errors()[0].contains("dev-only"));
    }
}

#[test]
fn dev_dependencies_still_obey_direction() {
    let fixture = Fixture::new(&[
        (
            "oxiview-core",
            "[dev-dependencies]\noxiview-platform-headless = { path = '../oxiview-platform-headless' }",
        ),
        ("oxiview-platform-headless", ""),
    ]);
    assert!(fixture.errors()[0].contains("(dev"));
}

#[test]
fn no_crate_can_depend_on_xtask() {
    let fixture = Fixture::new(&[
        (
            "oxiview-base",
            "[dev-dependencies]\nxtask = { path = '../xtask' }",
        ),
        ("xtask", ""),
    ]);
    assert!(fixture.errors()[0].contains("forbidden"));
}

#[test]
fn external_constraints_cover_each_context() {
    for (name, owner) in [
        ("taffy", "oxiview-layout"),
        ("parley", "oxiview-text"),
        ("fontique", "oxiview-text"),
        ("swash", "oxiview-text"),
        ("skrifa", "oxiview-text"),
        ("harfrust", "oxiview-text"),
        ("vello", "oxiview-render-wgpu"),
        ("wgpu", "oxiview-app"),
        ("objc2", "oxiview-platform-macos"),
    ] {
        for (consumer, expected) in [(owner, 0), ("oxiview-base", 1)] {
            let fixture = Fixture::new(&[(
                consumer,
                &format!("[dependencies]\n{name} = {{ path = '../external' }}"),
            )]);
            fixture.package("external", name, "[workspace]");
            assert_eq!(fixture.errors().len(), expected, "{consumer} -> {name}");
        }
    }
}

#[test]
fn database_fetch_failure_is_an_environment_error() {
    assert_eq!(deny_status(false, false, Some(1)), Status::Error);
    assert_eq!(deny_status(false, true, Some(1)), Status::Fail);
    assert_eq!(deny_status(false, true, None), Status::Error);
    assert_eq!(deny_status(true, true, Some(0)), Status::Pass);
}
