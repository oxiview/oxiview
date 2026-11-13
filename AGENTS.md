# oxiview engineering conventions

oxiview is a self-drawn Rust UI framework exclusively for native macOS / Windows / Linux desktop platforms. It is at the M-1 engineering baseline milestone; M0 follows M-1.

## Hard rules

- Target native platforms only; do not use WebView, HTML/CSS, WASM, or browser targets. Implement only the specified milestone step.
- Use Rust stable 1.97 and edition 2024; framework builds must not use nightly or unstable features.
- `cargo xtask verify` must pass before committing. Run verification locally only ([ADR-0019](docs/adr/0019-local-only-verification.md)).
- Library code must not use unwrap/expect/panic. See the [error conventions](docs/conventions/unsafe-panic-errors.md) for limited exceptions in tests, xtask, examples, and compile-time assertions.
- Restrict unsafe to platform and rendering adapters, except the single native surface handle constructor declaration in `oxiview-base` ([ADR-0025](docs/adr/0025-owned-native-surface-handle.md)). Every unsafe block needs `// SAFETY:` and every unsafe function needs `# Safety`; see [safety boundaries](docs/conventions/unsafe-panic-errors.md).
- Follow [xtask/layers.toml](xtask/layers.toml) for dependency direction. Convert third-party types through an anti-corruption layer instead of exposing them in public APIs.
- Internal crates use `oxiview-*`, version `0.0.0`, and `publish = false`; the existing `xtask` engineering tool and public `oxiview` facade follow the ADR naming decisions.
- Define dependency versions in the root `[workspace.dependencies]`; crates inherit them with `xxx.workspace = true`. Follow the [dependency conventions](docs/conventions/dependencies.md) when adding dependencies.
- Snapshot `--accept` and performance baseline `--record` require prior human confirmation. Never update baselines to hide failures.
- Create canonical skills only in `.agents/skills/`, never directly in `.claude/skills/` ([ADR-0023](docs/adr/0023-agent-conventions-dual-harness.md)).
- The default language is **English (`en`)** for project documentation, comments, descriptions, diagnostics, and commit messages. Use English identifiers and the canonical terms in [GLOSSARY.md](GLOSSARY.md).
- Create or update translations into another language **only when explicitly requested**. Keep English documents at their unsuffixed paths; name translations with an uppercase `_LANG` suffix before the extension, for example `README.md` (English) and `README_ZH.md` (Chinese). Add language navigation at the top of each README with links to available translations.
- Write an ADR and update its index when a decision is hard to reverse, surprising without context, and involves real trade-offs; see the [Git conventions](docs/conventions/git-workflow.md).
- Maintain this file as the sole source of truth; `CLAUDE.md` must be a relative symlink resolving to it.
- Do not push to a remote unless the owner explicitly requests it.
- Do not append attribution, co-author lines, sign-offs, AI/tool/model credits, or other unsolicited commit footers or trailers. Include them only when the user explicitly requests them for that commit, using the identity they supply or confirm.

## Command reference

Prefix every command below with `cargo xtask`. Tools are never installed automatically. The global `--json` option restricts stdout to the JSON report.

| Subcommand | Purpose |
| --- | --- |
| `fmt` / `fmt --check` | Format the workspace / check formatting only |
| `lint` | Run Clippy on all workspace targets with `-D warnings` |
| `test` | Run workspace tests, including pixel snapshot self-tests |
| `deps` | Check dependency direction and run cargo-deny after refreshing the advisory database |
| `snapshot --review` | List pending snapshots for human review of actual / diff images |
| `snapshot --accept <name\|crate/name\|all>` | Update snapshot baselines after human confirmation |
| `bench` / `bench --record` | Compare this machine's performance baseline / record it after human confirmation |
| `new-crate <name> --context <context>` | Create a registered crate with inherited workspace settings; refuse overwrites |
| `install-hooks` / `install-hooks --uninstall` | Optionally install a pre-push verify hook; `--force` backs up before overwriting; uninstall removes only matching hooks |
| `doctor` | Check Rust 1.97 stable and cargo-deny; suggest installation commands for missing tools |
| `verify` / `verify --full` | Run fmt, lint, test, deps, and docs after doctor passes; full adds bench |
| `help [subcommand]` / `--help` | Show help |

Exit codes: `0` for success; `1` for failed checks; `2` for missing environment requirements, invalid arguments, or xtask errors (takes precedence over 1). verify returns early if doctor fails; other failures are aggregated.
Each invocation overwrites `target/xtask/report.json`, recording each check's name, status, duration, failure summary, and artifact paths. Logs are in `target/xtask/<check-name>.log`.
`docs` is an internal verify check with no standalone subcommand. It checks inline relative link targets in this file, Markdown under docs, and skill bodies; it also checks the CLAUDE.md and .claude/skills relative symlinks and skill frontmatter. It does not validate anchors.
Skill frontmatter uses single-line unquoted strings and must contain exactly name and description; name must match the directory name.

## Documentation index

- [ADR index](docs/adr/README.md), [roadmap](docs/roadmap.md), [plans](docs/plans/), [M-1 plan](docs/plans/m-1-engineering-baseline.md)
- [Architecture and context mapping](docs/architecture.md), [glossary](GLOSSARY.md)
- [Unsafe, panic, and errors](docs/conventions/unsafe-panic-errors.md), [API stability](docs/conventions/api-stability.md)
- [Dependency management](docs/conventions/dependencies.md), [Git workflow](docs/conventions/git-workflow.md)
- [Pixel snapshots](docs/conventions/testing-snapshots.md), [Benchmarks and performance baselines](docs/conventions/testing-performance.md)

## Skill index

Canonical skills live in `.agents/skills/`; Claude Code reads them through the `.claude/skills` relative symlink.

| Name | Purpose |
| --- | --- |
| [adr-glossary](.agents/skills/adr-glossary/SKILL.md) | Maintain ADR criteria, numbering, the index, and canonical terminology. |
| [scaffold](.agents/skills/scaffold/SKILL.md) | Create crates under workspace and dependency rules; provide widget and capability checklists. |
| [review-checklist](.agents/skills/review-checklist/SKILL.md) | Review dependencies, safety boundaries, APIs, tests, and observability. |
| [verify](.agents/skills/verify/SKILL.md) | Select verification commands, interpret reports, and follow baseline approval rules. |
