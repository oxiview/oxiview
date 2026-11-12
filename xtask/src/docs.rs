//! 公约入口、Markdown 相对链接与软链检查。

use std::{fs, io, path::Path};

use crate::{check::Check, report::Status};

pub(crate) struct DocsCheck;

impl Check for DocsCheck {
    fn name(&self) -> &str {
        "docs"
    }

    fn run(&self, root: &Path) -> (Status, String) {
        let mut failures = Vec::new();
        for (link, target) in [
            ("CLAUDE.md", "AGENTS.md"),
            (".claude/skills", ".agents/skills"),
        ] {
            if !valid_symlink(root, link, target) {
                failures.push(format!("{link}：必须是解析到 {target} 的相对路径软链"));
            }
        }
        if let Err(error) = check_file(root, &root.join("AGENTS.md"), &mut failures)
            .and_then(|()| check_directory(root, &root.join("docs"), &mut failures))
            .and_then(|()| check_skills(root, &mut failures))
        {
            failures.push(format!("无法读取文档：{error}"));
            return (Status::Error, failures.join("\n"));
        }
        if failures.is_empty() {
            (
                Status::Pass,
                "文档相对链接、技能 frontmatter 与公约软链检查通过。\n".into(),
            )
        } else {
            (Status::Fail, failures.join("\n"))
        }
    }
}

fn valid_symlink(root: &Path, link: &str, target: &str) -> bool {
    let link = root.join(link);
    fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::read_link(&link).is_ok_and(|target| target.is_relative())
        && fs::canonicalize(link)
            .ok()
            .zip(fs::canonicalize(root.join(target)).ok())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn check_skills(root: &Path, failures: &mut Vec<String>) -> io::Result<()> {
    let directory = root.join(".agents/skills");
    if !directory.is_dir() {
        failures.push(".agents/skills：缺少技能正本目录".into());
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let file = entry.path().join("SKILL.md");
        if !file.is_file() {
            failures.push(format!("{}：缺少 SKILL.md", entry.path().display()));
            continue;
        }
        let source = fs::read_to_string(&file)?;
        if let Err(error) = skill_frontmatter(&source, &entry.file_name().to_string_lossy()) {
            failures.push(format!(
                "{}：{error}",
                file.strip_prefix(root).unwrap_or(&file).display()
            ));
        }
        check_file(root, &file, failures)?;
    }
    Ok(())
}

/// 仓库技能使用 YAML 的单行无引号字符串子集，避免引入完整 YAML 解析器。
/// 不支持嵌套、引号、块字符串或别名；遇到这些写法明确报错，不猜测字段语义。
fn skill_frontmatter(source: &str, directory: &str) -> Result<(), String> {
    let mut lines = source.lines();
    if lines.next() != Some("---") {
        return Err("frontmatter 必须以 --- 开始".into());
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(": ") else {
            return Err(
                "frontmatter 只支持 name: 值 与 description: 值（单行无引号字符串）".into(),
            );
        };
        let slot = match key {
            "name" => &mut name,
            "description" => &mut description,
            _ => {
                return Err(format!(
                    "frontmatter 只允许 name 与 description，发现 {key}"
                ));
            }
        };
        let value = value.trim();
        if value.is_empty()
            || value.starts_with([
                '\'', '"', '|', '>', '&', '*', '!', '{', '[', '#', '@', '`', '%', ',', ']', '}',
            ])
            || value.contains(": ")
            || value.contains(" #")
            || matches!(
                value,
                "null"
                    | "Null"
                    | "NULL"
                    | "~"
                    | "true"
                    | "false"
                    | "True"
                    | "False"
                    | "TRUE"
                    | "FALSE"
            )
            || value.parse::<f64>().is_ok()
        {
            return Err(format!("frontmatter 的 {key} 必须是非空单行无引号字符串"));
        }
        if slot.replace(value).is_some() {
            return Err(format!("frontmatter 字段 {key} 重复"));
        }
    }
    if !closed || name != Some(directory) || description.is_none() {
        return Err("frontmatter 必须闭合，包含 name 与 description，且 name 与目录名一致".into());
    }
    Ok(())
}

fn check_directory(root: &Path, directory: &Path, failures: &mut Vec<String>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        // 不跟随目录软链，避免环；Markdown 文件软链仍读取其内容。
        if entry.file_type()?.is_dir() {
            check_directory(root, &path, failures)?;
        } else if path.extension().is_some_and(|extension| extension == "md") {
            check_file(root, &path, failures)?;
        }
    }
    Ok(())
}

fn check_file(root: &Path, file: &Path, failures: &mut Vec<String>) -> io::Result<()> {
    let source = fs::read_to_string(file)?;
    let parent = file.parent().expect("文档有父目录");
    let display = file.strip_prefix(root).unwrap_or(file).display();
    let mut fence = None;
    for (line_number, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().unwrap_or(' ');
        let count = trimmed.chars().take_while(|&ch| ch == marker).count();
        if matches!(marker, '`' | '~') && count >= 3 {
            match fence {
                None => fence = Some((marker, count)),
                Some((opening, length)) if marker == opening && count >= length => fence = None,
                _ => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        for destination in inline_links(line) {
            let destination = destination.trim();
            if destination.starts_with('#')
                || destination.starts_with('/')
                || destination.contains(':')
            {
                continue;
            }
            let path = destination.split(['#', '?']).next().unwrap_or("");
            if !path.is_empty() && !parent.join(path).exists() {
                failures.push(format!(
                    "{display}:{}：失效链接 {destination}",
                    line_number + 1
                ));
            }
        }
    }
    Ok(())
}

/// 解析单行内联链接；跳过代码跨度，支持目标中的配对括号、尖括号与可选标题。
/// 不解析引用式链接或 HTML；本检查只保证目标存在，不检查锚点。
fn inline_links(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut links = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'`' {
            let start = index;
            while index < bytes.len() && bytes[index] == b'`' {
                index += 1;
            }
            let delimiter = &line[start..index];
            if let Some(end) = line[index..].find(delimiter) {
                index += end + delimiter.len();
            }
            continue;
        }
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }
        let Some(label_end) = line[index + 1..].find("](") else {
            index += 1;
            continue;
        };
        index += label_end + 3;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let angled = bytes.get(index) == Some(&b'<');
        if angled {
            index += 1;
        }
        let start = index;
        let mut depth = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if angled {
                if byte == b'>' {
                    break;
                }
            } else {
                if (byte == b')' && depth == 0) || byte.is_ascii_whitespace() {
                    break;
                }
                if byte == b'(' {
                    depth += 1;
                } else if byte == b')' {
                    depth -= 1;
                }
            }
            index += 1;
        }
        if index < bytes.len() {
            links.push(&line[start..index]);
        }
    }
    links
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    pub(crate) struct Fixture(pub(crate) PathBuf);

    impl Fixture {
        pub(crate) fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../target/xtask")
                .join(format!(
                    "docs-fixture-{}-{}",
                    std::process::id(),
                    NEXT_ID.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(root.join("docs/nested")).expect("创建 fixture");
            fs::write(root.join("AGENTS.md"), "[公约](docs/nested/rules.md)\n").expect("写入口");
            fs::write(
                root.join("docs/nested/rules.md"),
                "[入口](../../AGENTS.md#规则)\n",
            )
            .expect("写文档");
            fs::create_dir_all(root.join(".agents/skills/verify")).expect("创建技能");
            fs::create_dir(root.join(".claude")).expect("创建工具目录");
            fs::write(
                root.join(".agents/skills/verify/SKILL.md"),
                "---\nname: verify\ndescription: 验证时使用。\n---\n正文\n",
            )
            .expect("写技能");
            let fixture = Self(root);
            fixture.skill_link("../.agents/skills");
            fixture
        }

        fn skill_link(&self, target: &str) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, self.0.join(".claude/skills"))
                .expect("创建技能软链");
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(target, self.0.join(".claude/skills"))
                .expect("创建技能软链需要开发者模式");
        }

        fn link(&self, target: &str) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, self.0.join("CLAUDE.md")).expect("创建软链");
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(target, self.0.join("CLAUDE.md"))
                .expect("创建软链需要 Windows 开发者模式或相应权限");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn valid_links_and_symlink_pass() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        fs::write(fixture.0.join("docs/example.md"), "[远端](https://example.invalid/no) [锚点](#不存在)\n`[代码](missing.md)`\n```md\n[代码块](missing.md)\n```\n[标题](../AGENTS.md \"入口\")\n").expect("写链接示例");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Pass);
    }

    #[test]
    fn broken_link_reports_file_line_and_destination() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        fs::write(
            fixture.0.join("docs/nested/rules.md"),
            "标题\n[失效](missing.md#标题)\n",
        )
        .expect("写失效链接");
        let (status, log) = DocsCheck.run(&fixture.0);
        assert_eq!(status, Status::Fail);
        assert!(log.contains("docs/nested/rules.md:2"));
        assert!(log.contains("missing.md#标题"));
    }

    #[test]
    fn missing_regular_wrong_and_dangling_symlinks_fail() {
        let fixture = Fixture::new();
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::write(fixture.0.join("CLAUDE.md"), "普通文件").expect("写普通文件");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_file(fixture.0.join("CLAUDE.md")).expect("删除普通文件");
        for target in ["docs/nested/rules.md", "missing.md"] {
            fixture.link(target);
            assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
            fs::remove_file(fixture.0.join("CLAUDE.md")).expect("删除错误软链");
        }
    }

    #[test]
    fn skill_frontmatter_rejects_invalid_fields_and_names() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        let file = fixture.0.join(".agents/skills/verify/SKILL.md");
        for source in [
            "name: verify\ndescription: 验证",
            "---\nname: verify\ndescription: 验证\n",
            "---\nname: other\ndescription: 验证\n---",
            "---\nname: verify\n---",
            "---\nname: verify\ndescription: \n---",
            "---\nname: verify\nname: verify\ndescription: 验证\n---",
            "---\nname: verify\ndescription: 验证\nextra: true\n---",
            "---\nname: verify\ndescription: 验证\nmetadata:\n  extra: true\n---",
            "---\nname: verify\ndescription: true\n---",
        ] {
            fs::write(&file, source).expect("写无效技能");
            let (status, log) = DocsCheck.run(&fixture.0);
            assert_eq!(status, Status::Fail, "{source}");
            assert!(log.contains(".agents/skills/verify/SKILL.md"));
        }
    }

    #[test]
    fn skill_links_and_missing_source_are_checked() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        let file = fixture.0.join(".agents/skills/verify/SKILL.md");
        fs::write(
            &file,
            "---\nname: verify\ndescription: 验证\n---\n[错误](missing.md)",
        )
        .expect("写失效链接");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_file(file).expect("移除技能");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
    }

    #[test]
    fn skill_symlink_rejects_missing_regular_wrong_dangling_and_absolute() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        let link = fixture.0.join(".claude/skills");
        // Windows 的目录软链使用 remove_dir 删除。
        let remove_link = || {
            #[cfg(unix)]
            fs::remove_file(&link).expect("删除目录软链");
            #[cfg(windows)]
            fs::remove_dir(&link).expect("删除目录软链");
        };
        remove_link();
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::create_dir(&link).expect("创建普通目录");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_dir(&link).expect("删除普通目录");
        for target in [
            "../docs",
            "../missing",
            fixture.0.join(".agents/skills").to_str().expect("路径"),
        ] {
            fixture.skill_link(target);
            assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
            remove_link();
        }
    }

    #[test]
    fn parses_parentheses_and_angled_destinations() {
        assert_eq!(
            inline_links("[甲](a(b).md) [乙](<some file.md> \"标题\")"),
            vec!["a(b).md", "some file.md"]
        );
    }
}
