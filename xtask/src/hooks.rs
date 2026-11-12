//! 可选的本地 pre-push hook 安装与卸载。

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use crate::{check::Check, report::Status};

const SCRIPT: &str = "#!/bin/sh\n# 由 oxiview cargo xtask install-hooks 安装。\nset -eu\nroot=$(git rev-parse --show-toplevel)\ncd \"$root\"\nexec cargo xtask verify\n";

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
            Err(error) => (Status::Error, format!("hook 操作失败：{error}")),
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
            "无法定位 hooks：{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let path = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let path = path.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        return Err(io::Error::other("Git 返回空 hooks 路径"));
    }
    // join 保留绝对路径；相对 core.hooksPath 按 Git 运行目录解析。
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
    // Windows 没有 POSIX 执行位，Git for Windows 通过 shebang 执行 hook。
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

impl InstallHooks {
    fn apply(&self, root: &Path) -> io::Result<String> {
        let hook = hook_path(root)?;
        let existing = match fs::symlink_metadata(&hook) {
            Ok(metadata) if metadata.file_type().is_file() => Some(fs::read(&hook)?),
            Ok(_) => return Err(io::Error::other("pre-push 不是普通文件，拒绝修改")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if self.uninstall {
            return match existing {
                None => Ok("pre-push 不存在，无需卸载。".into()),
                Some(bytes) if bytes == SCRIPT.as_bytes() => {
                    fs::remove_file(&hook)?;
                    Ok(format!("已卸载 {}；备份保留，需自行恢复。", hook.display()))
                }
                Some(_) => Err(io::Error::other("pre-push 内容不匹配，拒绝卸载")),
            };
        }
        if let Some(bytes) = existing {
            if bytes == SCRIPT.as_bytes() {
                make_executable(&hook)?;
                return Ok(format!("{} 已安装。", hook.display()));
            }
            if !self.force {
                return Err(io::Error::other(
                    "已有不同内容的 pre-push，拒绝覆盖；可用 --force 备份后覆盖",
                ));
            }
            let backup = hook.with_file_name("pre-push.backup");
            // 排他创建，保留前一次备份；写入和权限复制完成后才覆盖 hook。
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(|error| {
                    io::Error::other(format!(
                        "无法创建备份 {}（已有备份时请先处理）：{error}",
                        backup.display()
                    ))
                })?;
            file.write_all(&bytes)?;
            file.set_permissions(fs::metadata(&hook)?.permissions())?;
            fs::write(&hook, SCRIPT)?;
        } else {
            fs::create_dir_all(hook.parent().expect("hook 有父目录"))?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&hook)?
                .write_all(SCRIPT.as_bytes())?;
        }
        make_executable(&hook)?;
        Ok(format!(
            "已安装 {}，推送前执行 cargo xtask verify。",
            hook.display()
        ))
    }
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
