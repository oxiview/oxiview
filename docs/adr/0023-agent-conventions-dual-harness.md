---
status: accepted
---

# Agent conventions: one canonical repository source for Claude Code and Codex

Keep agent conventions and skills in the repository, sharing the same content between Claude Code and Codex:

- **Canonical conventions**: `AGENTS.md` for Codex, with `CLAUDE.md -> AGENTS.md` for Claude Code.
- **Canonical skills**: `.agents/skills/<name>/SKILL.md` for Codex, with the directory symlink `.claude/skills -> ../.agents/skills` for Claude Code.
- Skills use only common frontmatter fields supported by both tools (`name`, `description`), without vendor-specific fields. Tool-specific settings live in their respective `.claude/` or `.codex/` directories and contain no convention content.

Maintain one source of truth to avoid divergence. Symlinks rather than copies ensure that one edit takes effect in both tools.

## Consequences

- Check out the repository on a filesystem supporting symlinks. Future Windows contributors need `core.symlinks` enabled.
- Create new skills only under `.agents/skills/`, never directly under `.claude/skills/`.
