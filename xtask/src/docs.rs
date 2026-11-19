//! Check convention entry points, Markdown relative links, and symlinks.

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
                failures.push(format!(
                    "{link}: must be a relative symlink resolving to {target}"
                ));
            }
        }
        if let Err(error) = check_file(root, &root.join("AGENTS.md"), &mut failures)
            .and_then(|()| check_directory(root, &root.join("docs"), &mut failures))
            .and_then(|()| check_skills(root, &mut failures))
        {
            failures.push(format!("Cannot read documentation: {error}"));
            return (Status::Error, failures.join("\n"));
        }
        if root.join("docs/performance").is_dir()
            && let Err(error) = crate::perf_report::check(root)
        {
            failures.push(error);
        }
        if failures.is_empty() {
            (
                Status::Pass,
                "Documentation relative links, skill frontmatter, and convention symlinks passed.\n".into(),
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
        failures.push(".agents/skills: canonical skills directory is missing".into());
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
            failures.push(format!("{}: missing SKILL.md", entry.path().display()));
            continue;
        }
        let source = fs::read_to_string(&file)?;
        if let Err(error) = skill_frontmatter(&source, &entry.file_name().to_string_lossy()) {
            failures.push(format!(
                "{}: {error}",
                file.strip_prefix(root).unwrap_or(&file).display()
            ));
        }
        check_file(root, &file, failures)?;
    }
    Ok(())
}

/// Repository skills use YAML's single-line unquoted string subset, avoiding a full YAML parser.
/// Reject nesting, quotes, block strings, and aliases explicitly instead of guessing field semantics.
fn skill_frontmatter(source: &str, directory: &str) -> Result<(), String> {
    let mut lines = source.lines();
    if lines.next() != Some("---") {
        return Err("frontmatter must start with ---".into());
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
                "frontmatter supports only name: value and description: value (single-line unquoted strings)".into(),
            );
        };
        let slot = match key {
            "name" => &mut name,
            "description" => &mut description,
            _ => {
                return Err(format!(
                    "frontmatter allows only name and description; found {key}"
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
            return Err(format!(
                "frontmatter {key} must be a nonempty single-line unquoted string"
            ));
        }
        if slot.replace(value).is_some() {
            return Err(format!("Duplicate frontmatter field: {key}"));
        }
    }
    if !closed || name != Some(directory) || description.is_none() {
        return Err("frontmatter must be closed, contain name and description, and have a name matching the directory".into());
    }
    Ok(())
}

fn check_directory(root: &Path, directory: &Path, failures: &mut Vec<String>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        // Do not follow directory symlinks to avoid cycles; still read Markdown file symlink contents.
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
    let parent = file.parent().expect("document has a parent directory");
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
                    "{display}:{}: broken link {destination}",
                    line_number + 1
                ));
            }
        }
    }
    Ok(())
}

/// Parse single-line inline links, skipping code spans and supporting balanced parentheses, angle brackets, and optional titles.
/// Do not parse reference links or HTML; validate target existence, not anchors.
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
            fs::create_dir_all(root.join("docs/nested")).expect("create fixture");
            fs::write(
                root.join("AGENTS.md"),
                "[Conventions](docs/nested/rules.md)\n",
            )
            .expect("write entry point");
            fs::write(
                root.join("docs/nested/rules.md"),
                "[Entry](../../AGENTS.md#rules)\n",
            )
            .expect("write documentation");
            fs::create_dir_all(root.join(".agents/skills/verify")).expect("create skill");
            fs::create_dir(root.join(".claude")).expect("create tool directory");
            fs::write(
                root.join(".agents/skills/verify/SKILL.md"),
                "---\nname: verify\ndescription: Use for verification.\n---\nBody\n",
            )
            .expect("write skill");
            let fixture = Self(root);
            fixture.skill_link("../.agents/skills");
            fixture
        }

        fn skill_link(&self, target: &str) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, self.0.join(".claude/skills"))
                .expect("create skill symlink");
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(target, self.0.join(".claude/skills"))
                .expect("creating skill symlinks requires developer mode");
        }

        fn link(&self, target: &str) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, self.0.join("CLAUDE.md")).expect("create symlink");
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(target, self.0.join("CLAUDE.md")).expect(
                "creating symlinks requires Windows developer mode or appropriate permissions",
            );
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
        fs::write(fixture.0.join("docs/example.md"), "[Remote](https://example.invalid/no) [Anchor](#nonexistent)\n`[Code](missing.md)`\n```md\n[Code block](missing.md)\n```\n[title](../AGENTS.md \"Entry\")\n").expect("write link example");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Pass);
    }

    #[test]
    fn broken_link_reports_file_line_and_destination() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        fs::write(
            fixture.0.join("docs/nested/rules.md"),
            "title\n[Broken](missing.md#title)\n",
        )
        .expect("write broken link");
        let (status, log) = DocsCheck.run(&fixture.0);
        assert_eq!(status, Status::Fail);
        assert!(log.contains("docs/nested/rules.md:2"));
        assert!(log.contains("missing.md#title"));
    }

    #[test]
    fn missing_regular_wrong_and_dangling_symlinks_fail() {
        let fixture = Fixture::new();
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::write(fixture.0.join("CLAUDE.md"), "regular file").expect("write regular file");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_file(fixture.0.join("CLAUDE.md")).expect("remove regular file");
        for target in ["docs/nested/rules.md", "missing.md"] {
            fixture.link(target);
            assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
            fs::remove_file(fixture.0.join("CLAUDE.md")).expect("remove incorrect symlink");
        }
    }

    #[test]
    fn skill_frontmatter_rejects_invalid_fields_and_names() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        let file = fixture.0.join(".agents/skills/verify/SKILL.md");
        for source in [
            "name: verify\ndescription: Verification",
            "---\nname: verify\ndescription: Verification\n",
            "---\nname: other\ndescription: Verification\n---",
            "---\nname: verify\n---",
            "---\nname: verify\ndescription: \n---",
            "---\nname: verify\nname: verify\ndescription: Verification\n---",
            "---\nname: verify\ndescription: Verification\nextra: true\n---",
            "---\nname: verify\ndescription: Verification\nmetadata:\n  extra: true\n---",
            "---\nname: verify\ndescription: true\n---",
        ] {
            fs::write(&file, source).expect("write invalid skill");
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
            "---\nname: verify\ndescription: Verification\n---\n[Error](missing.md)",
        )
        .expect("write broken link");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_file(file).expect("remove skill");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
    }

    #[test]
    fn skill_symlink_rejects_missing_regular_wrong_dangling_and_absolute() {
        let fixture = Fixture::new();
        fixture.link("AGENTS.md");
        let link = fixture.0.join(".claude/skills");
        // Remove directory symlinks with remove_dir on Windows.
        let remove_link = || {
            #[cfg(unix)]
            fs::remove_file(&link).expect("remove directory symlink");
            #[cfg(windows)]
            fs::remove_dir(&link).expect("remove directory symlink");
        };
        remove_link();
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::create_dir(&link).expect("create regular directory");
        assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
        fs::remove_dir(&link).expect("remove regular directory");
        for target in [
            "../docs",
            "../missing",
            fixture.0.join(".agents/skills").to_str().expect("path"),
        ] {
            fixture.skill_link(target);
            assert_eq!(DocsCheck.run(&fixture.0).0, Status::Fail);
            remove_link();
        }
    }

    #[test]
    fn parses_parentheses_and_angled_destinations() {
        assert_eq!(
            inline_links("[A](a(b).md) [B](<some file.md> \"title\")"),
            vec!["a(b).md", "some file.md"]
        );
    }
}
