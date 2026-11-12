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
        let link = root.join("CLAUDE.md");
        let valid_link = fs::symlink_metadata(&link)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
            && fs::read_link(&link).is_ok_and(|target| target.is_relative())
            && fs::canonicalize(&link)
                .ok()
                .zip(fs::canonicalize(root.join("AGENTS.md")).ok())
                .is_some_and(|(actual, expected)| actual == expected);
        if !valid_link {
            failures.push("CLAUDE.md：必须是解析到 AGENTS.md 的相对路径软链".into());
        }
        if let Err(error) = check_file(root, &root.join("AGENTS.md"), &mut failures)
            .and_then(|()| check_directory(root, &root.join("docs"), &mut failures))
        {
            failures.push(format!("无法读取文档：{error}"));
            return (Status::Error, failures.join("\n"));
        }
        if failures.is_empty() {
            (
                Status::Pass,
                "文档相对链接与 CLAUDE.md 软链检查通过。\n".into(),
            )
        } else {
            (Status::Fail, failures.join("\n"))
        }
    }
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
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
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
            Self(root)
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
    fn parses_parentheses_and_angled_destinations() {
        assert_eq!(
            inline_links("[甲](a(b).md) [乙](<some file.md> \"标题\")"),
            vec!["a(b).md", "some file.md"]
        );
    }
}
