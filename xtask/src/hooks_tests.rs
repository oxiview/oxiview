//! Hook tests modify only isolated Git repositories under target/xtask.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/xtask")
            .join(format!(
                "hooks-fixture-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).expect("create fixture");
        let fixture = Self(fs::canonicalize(path).expect("resolve fixture path"));
        fixture.git(&["init", "--template="]);
        // Do not inherit personal hooksPath; tests override this local setting as needed.
        fixture.git(&["config", "core.hooksPath", ".git/hooks"]);
        fixture
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("run fixture Git");
        assert!(output.status.success(), "{:?}", output);
    }

    fn apply(&self, force: bool, uninstall: bool) -> (Status, String) {
        InstallHooks { force, uninstall }.run(&self.0)
    }

    fn hook(&self) -> PathBuf {
        hook_path(&self.0).expect("locate fixture hook")
    }

    fn existing(&self) {
        fs::create_dir_all(self.hook().parent().expect("parent directory")).expect("create hooks");
        fs::write(self.hook(), "#!/bin/sh\nexit 42\n").expect("write original hook");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn install_hooks_first_install_and_repeat_are_idempotent() {
    let fixture = Fixture::new();
    assert!(!fixture.hook().exists());
    for _ in 0..2 {
        let (status, log) = fixture.apply(false, false);
        assert_eq!(status, Status::Pass, "{log}");
        assert_eq!(
            fs::read(fixture.hook()).expect("read hook"),
            SCRIPT.as_bytes()
        );
        assert!(!fixture.hook().with_file_name("pre-push.backup").exists());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.hook())
                .expect("permissions")
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
    }
}

#[test]
fn install_hooks_refuses_different_content_and_uninstall_preserves_it() {
    let fixture = Fixture::new();
    fixture.existing();
    for uninstall in [false, true] {
        assert_eq!(fixture.apply(false, uninstall).0, Status::Error);
        assert_eq!(
            fs::read_to_string(fixture.hook()).expect("read hook"),
            "#!/bin/sh\nexit 42\n"
        );
    }
}

#[test]
fn install_hooks_force_backs_up_without_overwriting_previous_backup() {
    let fixture = Fixture::new();
    fixture.existing();
    assert_eq!(fixture.apply(true, false).0, Status::Pass);
    let backup = fixture.hook().with_file_name("pre-push.backup");
    assert_eq!(
        fs::read_to_string(&backup).expect("read backup"),
        "#!/bin/sh\nexit 42\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.hook()).expect("read hook"),
        SCRIPT
    );
    fs::write(fixture.hook(), "new user content").expect("modify hook");
    assert_eq!(fixture.apply(true, false).0, Status::Error);
    assert_eq!(
        fs::read_to_string(fixture.hook()).expect("read hook"),
        "new user content"
    );
    assert_eq!(
        fs::read_to_string(backup).expect("read backup"),
        "#!/bin/sh\nexit 42\n"
    );
}

#[test]
fn install_hooks_uninstall_removes_only_matching_hook_and_keeps_backup() {
    let fixture = Fixture::new();
    fixture.existing();
    assert_eq!(fixture.apply(true, false).0, Status::Pass);
    for _ in 0..2 {
        assert_eq!(fixture.apply(false, true).0, Status::Pass);
        assert!(!fixture.hook().exists());
        assert!(fixture.hook().with_file_name("pre-push.backup").exists());
    }
}

#[test]
fn install_hooks_respects_relative_and_absolute_hooks_path_with_spaces() {
    let fixture = Fixture::new();
    for path in [
        PathBuf::from("custom hooks"),
        fixture.0.join("absolute hooks"),
    ] {
        fixture.git(&["config", "core.hooksPath", path.to_str().expect("path")]);
        assert_eq!(fixture.apply(false, false).0, Status::Pass);
        assert_eq!(
            fs::read_to_string(fixture.0.join(path).join("pre-push")).expect("read hook"),
            SCRIPT
        );
        assert_eq!(fixture.apply(false, true).0, Status::Pass);
    }
    assert!(!fixture.0.join(".git/hooks/pre-push").exists());
}

#[test]
fn install_hooks_respects_linked_worktree() {
    let fixture = Fixture::new();
    fixture.git(&["config", "--unset", "core.hooksPath"]);
    let worktree = fixture.0.join("linked worktree");
    fixture.git(&[
        "worktree",
        "add",
        "--orphan",
        "-b",
        "fixture-worktree",
        worktree.to_str().expect("path"),
    ]);
    assert!(worktree.join(".git").is_file());
    let (status, log) = InstallHooks {
        force: false,
        uninstall: false,
    }
    .run(&worktree);
    assert_eq!(status, Status::Pass, "{log}");
    assert_eq!(
        fs::read_to_string(fixture.0.join(".git/hooks/pre-push")).expect("shared hook"),
        SCRIPT
    );
}

#[cfg(unix)]
#[test]
fn install_hooks_rejects_symlinks_even_with_force() {
    let fixture = Fixture::new();
    let target = fixture.0.join("original");
    fs::write(&target, SCRIPT).expect("write target");
    fs::create_dir_all(fixture.hook().parent().expect("parent directory")).expect("create hooks");
    std::os::unix::fs::symlink(&target, fixture.hook()).expect("create symlink");
    for (force, uninstall) in [(false, false), (true, false), (false, true)] {
        assert_eq!(fixture.apply(force, uninstall).0, Status::Error);
        assert!(
            fs::symlink_metadata(fixture.hook())
                .expect("symlink")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).expect("read target"), SCRIPT);
    }
}

#[cfg(unix)]
#[test]
fn install_hooks_script_runs_verify_at_root_and_propagates_failure() {
    let fixture = Fixture::new();
    assert_eq!(fixture.apply(false, false).0, Status::Pass);
    let bin = fixture.0.join("fake bin");
    let nested = fixture.0.join("nested directory");
    fs::create_dir(&bin).expect("create bin");
    fs::create_dir(&nested).expect("create subdirectory");
    let cargo = bin.join("cargo");
    fs::write(&cargo, "#!/bin/sh\n[ \"$PWD\" = \"$EXPECTED_ROOT\" ] || exit 90\n[ \"$*\" = 'xtask verify' ] || exit 91\nexit \"$VERIFY_EXIT\"\n").expect("write cargo test double");
    make_executable(&cargo).expect("executable permissions");
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH"),
    )))
    .expect("merge PATH");
    for code in [0, 1, 2] {
        let result = Command::new(fixture.hook())
            .current_dir(&nested)
            .env("PATH", &path)
            .env("EXPECTED_ROOT", &fixture.0)
            .env("VERIFY_EXIT", code.to_string())
            .status()
            .expect("run hook");
        assert_eq!(result.code(), Some(code));
    }
}

#[test]
fn install_hooks_cli_rejects_force_with_uninstall() {
    use clap::Parser;
    assert!(
        crate::Cli::try_parse_from(["xtask", "install-hooks", "--force", "--uninstall"]).is_err()
    );
    assert!(crate::Cli::try_parse_from(["xtask", "install-hooks"]).is_ok());
}
