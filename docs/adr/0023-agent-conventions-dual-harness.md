---
status: accepted
---

# Agent 公约：仓库内单一正本，同时支持 Claude Code 与 Codex

agent 相关的公约和技能都放在仓库内，同一份内容同时服务 Claude Code 和 Codex：

- **公约正本**：`AGENTS.md`（Codex 读取），并建立软链 `CLAUDE.md -> AGENTS.md`（Claude Code 读取）。
- **技能正本**：`.agents/skills/<name>/SKILL.md`（Codex 读取），并建立目录软链 `.claude/skills -> ../.agents/skills`（Claude Code 读取）。
- 技能只写两个工具都支持的通用 frontmatter 字段（`name`、`description`），不依赖某一家的专有字段；工具专有的配置放在各自的 `.claude/`、`.codex/` 目录中，并且不包含公约内容。

只维护一份正本，避免两套规范逐渐漂移；用软链而不是复制，改一处两边同时生效。

## Consequences

- 仓库需要在支持软链的文件系统上检出；将来如有 Windows 贡献者，需要开启 `core.symlinks`。
- 新增技能时只能在 `.agents/skills/` 下创建，不能直接在 `.claude/skills/` 下创建。
