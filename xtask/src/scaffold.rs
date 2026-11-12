//! 只为已登记的上下文创建 crate 骨架。

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
                format!("已创建 crates/{}，请运行 cargo xtask verify。\n", self.name),
            ),
            // 参数、目标冲突和文件系统错误均是任务无法执行，而非检查发现违规。
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
            return Err("名称必须为 oxiview 或 oxiview- 前缀的小写 crate 名称。".into());
        }
        let rules = read_rules(root)?;
        let layer = rules
            .crates
            .get(name)
            .ok_or_else(|| format!("{name} 未在 layers.toml 登记；请先讨论上下文与依赖规则。"))?;
        if layer.context != self.context {
            return Err(format!(
                "{name} 的 context 必须为 {:?}，收到 {:?}",
                layer.context, self.context
            ));
        }
        let directory = root.join("crates").join(name);
        fs::create_dir_all(root.join("crates")).map_err(|error| error.to_string())?;
        // create_dir 原子拒绝已有目录、文件和软链，不能用 create_dir_all 覆盖目标。
        fs::create_dir(&directory)
            .map_err(|error| format!("拒绝覆盖或无法创建 {}：{error}", directory.display()))?;
        let result = (|| -> std::io::Result<()> {
            fs::create_dir(directory.join("src"))?;
            fs::write(
                directory.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\npublish.workspace = true\n\n[lints]\nworkspace = true\n"
                ),
            )?;
            // 每行都加文档前缀，规则中的换行不能变成源码。
            let documentation = format!(
                "{name}：所属上下文为 {}，角色为 {}（见 xtask/layers.toml）。",
                layer.context, layer.role
            )
            .lines()
            .map(|line| format!("//! {line}\n"))
            .collect::<String>();
            fs::write(directory.join("src/lib.rs"), documentation)
        })();
        if let Err(error) = result {
            return match fs::remove_dir_all(&directory) {
                Ok(()) => Err(format!("生成失败，已清理本次目录：{error}")),
                Err(cleanup) => Err(format!(
                    "生成失败：{error}；清理 {} 失败：{cleanup}",
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
        fs::create_dir(fixture.0.join("xtask")).expect("创建规则目录");
        fs::write(
            fixture.0.join("xtask/layers.toml"),
            include_str!("../layers.toml"),
        )
        .expect("写规则");
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
            task("oxiview-base", "共享基础").run(&fixture.0).0,
            Status::Pass
        );
        let root = fixture.0.join("crates/oxiview-base");
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).expect("读取 manifest"))
                .expect("解析 manifest");
        assert_eq!(manifest["package"]["name"].as_str(), Some("oxiview-base"));
        for field in ["version", "edition", "rust-version", "license", "publish"] {
            assert_eq!(
                manifest["package"][field]["workspace"].as_bool(),
                Some(true)
            );
        }
        assert_eq!(manifest["lints"]["workspace"].as_bool(), Some(true));
        let source = fs::read_to_string(root.join("src/lib.rs")).expect("读取库");
        assert!(
            source.starts_with("//! ")
                && source.contains("共享基础")
                && source.contains("foundation")
        );
        assert_eq!(fs::read_dir(&root).expect("列目录").count(), 2);
        assert_eq!(fs::read_dir(root.join("src")).expect("列源码").count(), 1);
    }

    #[test]
    fn rejects_unknown_name_and_mismatched_context_without_writes() {
        let fixture = setup();
        for (name, context) in [
            ("oxiview-unknown", "共享基础"),
            ("oxiview-base", "平台"),
            ("xtask", "工具"),
            ("oxiview-../../escape", "共享基础"),
        ] {
            assert_eq!(task(name, context).run(&fixture.0).0, Status::Error);
            assert!(!fixture.0.join("crates").exists());
        }
    }

    #[test]
    fn refuses_existing_directory_without_changing_contents() {
        let fixture = setup();
        let directory = fixture.0.join("crates/oxiview-base");
        fs::create_dir_all(&directory).expect("创建目标");
        fs::write(directory.join("keep"), "保留").expect("写已有内容");
        assert_eq!(
            task("oxiview-base", "共享基础").run(&fixture.0).0,
            Status::Error
        );
        assert_eq!(
            fs::read_to_string(directory.join("keep")).expect("读取内容"),
            "保留"
        );
        assert_eq!(fs::read_dir(directory).expect("列目录").count(), 1);
    }
}
