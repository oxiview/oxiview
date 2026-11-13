//! Optional local pre-push hook installation and removal.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use crate::{check::Check, report::Status};

const SCRIPT: &str = "#!/bin/sh\n# Installed by oxiview cargo xtask install-hooks.\nset -eu\nroot=$(git rev-parse --show-toplevel)\ncd \"$root\"\nexec cargo xtask verify\n";

pub(crate) struct InstallHooks {
    pub force: bool,
    pub uninstall: bool,
}

impl Check for InstallHooks {
    fn name(&self) -> &str {
        "install-hooks"
    }

    fn run(&self, root: &Path) -> (Status, String) {
        match self.apply(root) {
            Ok(message) => (Status::Pass, format!("{message}\n")),
            Err(error) => (Status::Error, format!("Hook operation failed: {error}")),
        }
    }
}

fn hook_path(root: &Path) -> io::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Cannot locate hooks: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let path = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let path = path.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        return Err(io::Error::other("Git returned an empty hooks path"));
    }
    // join preserves absolute paths; resolve relative core.hooksPath from Git's working directory.
    Ok(root.join(path).join("pre-push"))
}

fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(path, permissions)?;
    }
    // Windows has no POSIX executable bit; Git for Windows runs hooks through the shebang.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

impl InstallHooks {
    fn apply(&self, root: &Path) -> io::Result<String> {
        let hook = hook_path(root)?;
        let existing = match fs::symlink_metadata(&hook) {
            Ok(metadata) if metadata.file_type().is_file() => Some(fs::read(&hook)?),
            Ok(_) => {
                return Err(io::Error::other(
                    "pre-push is not a regular file; refusing to modify it",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if self.uninstall {
            return match existing {
                None => Ok("pre-push does not exist; nothing to uninstall.".into()),
                Some(bytes) if bytes == SCRIPT.as_bytes() => {
                    fs::remove_file(&hook)?;
                    Ok(format!(
                        "Uninstalled {}; backups remain for manual restoration.",
                        hook.display()
                    ))
                }
                Some(_) => Err(io::Error::other(
                    "pre-push content does not match; refusing to uninstall",
                )),
            };
        }
        if let Some(bytes) = existing {
            if bytes == SCRIPT.as_bytes() {
                make_executable(&hook)?;
                return Ok(format!("{} is already installed.", hook.display()));
            }
            if !self.force {
                return Err(io::Error::other(
                    "A different pre-push already exists; use --force to back it up before replacing it",
                ));
            }
            let backup = hook.with_file_name("pre-push.backup");
            // Create exclusively to preserve previous backups; replace the hook only after writing and copying permissions.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(|error| {
                    io::Error::other(format!(
                        "Cannot create backup {} (handle any existing backup first): {error}",
                        backup.display()
                    ))
                })?;
            file.write_all(&bytes)?;
            file.set_permissions(fs::metadata(&hook)?.permissions())?;
            fs::write(&hook, SCRIPT)?;
        } else {
            fs::create_dir_all(hook.parent().expect("hook has a parent directory"))?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&hook)?
                .write_all(SCRIPT.as_bytes())?;
        }
        make_executable(&hook)?;
        Ok(format!(
            "Installed {}; cargo xtask verify will run before pushes.",
            hook.display()
        ))
    }
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
