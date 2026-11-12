//! hook 测试只修改 target/xtask 下的独立 Git 仓库。

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
        fs::create_dir_all(&path).expect("创建 fixture");
        let fixture = Self(fs::canonicalize(path).expect("解析 fixture 路径"));
        fixture.git(&["init", "--template="]);
        // 不继承个人 hooksPath；后续测试按需覆盖此本地配置。
        fixture.git(&["config", "core.hooksPath", ".git/hooks"]);
        fixture
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("执行 fixture Git");
        assert!(output.status.success(), "{:?}", output);
    }

    fn apply(&self, force: bool, uninstall: bool) -> (Status, String) {
        InstallHooks { force, uninstall }.run(&self.0)
    }

    fn hook(&self) -> PathBuf {
        hook_path(&self.0).expect("定位 fixture hook")
    }

    fn existing(&self) {
        fs::create_dir_all(self.hook().parent().expect("父目录")).expect("创建 hooks");
        fs::write(self.hook(), "#!/bin/sh\nexit 42\n").expect("写原有 hook");
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
            fs::read(fixture.hook()).expect("读 hook"),
            SCRIPT.as_bytes()
        );
        assert!(!fixture.hook().with_file_name("pre-push.backup").exists());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.hook())
                .expect("权限")
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
            fs::read_to_string(fixture.hook()).expect("读 hook"),
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
        fs::read_to_string(&backup).expect("读备份"),
        "#!/bin/sh\nexit 42\n"
    );
    assert_eq!(fs::read_to_string(fixture.hook()).expect("读 hook"), SCRIPT);
    fs::write(fixture.hook(), "新用户内容").expect("修改 hook");
    assert_eq!(fixture.apply(true, false).0, Status::Error);
    assert_eq!(
        fs::read_to_string(fixture.hook()).expect("读 hook"),
        "新用户内容"
    );
    assert_eq!(
        fs::read_to_string(backup).expect("读备份"),
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
        fixture.git(&["config", "core.hooksPath", path.to_str().expect("路径")]);
        assert_eq!(fixture.apply(false, false).0, Status::Pass);
        assert_eq!(
            fs::read_to_string(fixture.0.join(path).join("pre-push")).expect("读 hook"),
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
        worktree.to_str().expect("路径"),
    ]);
    assert!(worktree.join(".git").is_file());
    let (status, log) = InstallHooks {
        force: false,
        uninstall: false,
    }
    .run(&worktree);
    assert_eq!(status, Status::Pass, "{log}");
    assert_eq!(
        fs::read_to_string(fixture.0.join(".git/hooks/pre-push")).expect("共享 hook"),
        SCRIPT
    );
}

#[cfg(unix)]
#[test]
fn install_hooks_rejects_symlinks_even_with_force() {
    let fixture = Fixture::new();
    let target = fixture.0.join("original");
    fs::write(&target, SCRIPT).expect("写目标");
    fs::create_dir_all(fixture.hook().parent().expect("父目录")).expect("创建 hooks");
    std::os::unix::fs::symlink(&target, fixture.hook()).expect("创建软链");
    for (force, uninstall) in [(false, false), (true, false), (false, true)] {
        assert_eq!(fixture.apply(force, uninstall).0, Status::Error);
        assert!(
            fs::symlink_metadata(fixture.hook())
                .expect("软链")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).expect("读目标"), SCRIPT);
    }
}

#[cfg(unix)]
#[test]
fn install_hooks_script_runs_verify_at_root_and_propagates_failure() {
    let fixture = Fixture::new();
    assert_eq!(fixture.apply(false, false).0, Status::Pass);
    let bin = fixture.0.join("fake bin");
    let nested = fixture.0.join("nested directory");
    fs::create_dir(&bin).expect("创建 bin");
    fs::create_dir(&nested).expect("创建子目录");
    let cargo = bin.join("cargo");
    fs::write(&cargo, "#!/bin/sh\n[ \"$PWD\" = \"$EXPECTED_ROOT\" ] || exit 90\n[ \"$*\" = 'xtask verify' ] || exit 91\nexit \"$VERIFY_EXIT\"\n").expect("写 cargo 替身");
    make_executable(&cargo).expect("执行权限");
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH"),
    )))
    .expect("PATH 合并");
    for code in [0, 1, 2] {
        let result = Command::new(fixture.hook())
            .current_dir(&nested)
            .env("PATH", &path)
            .env("EXPECTED_ROOT", &fixture.0)
            .env("VERIFY_EXIT", code.to_string())
            .status()
            .expect("执行 hook");
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
