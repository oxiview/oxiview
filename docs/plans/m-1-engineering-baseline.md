---
status: done
milestone: M-1
---

# M-1 engineering baseline: detailed plan

Goal: before writing framework code, establish one-command verification and fix the conventions, skills, technology choices, and directory constraints. After completion, any person or agent starting from a fresh checkout can run `cargo xtask verify` for a definite pass/fail result with machine-readable failures.

Related decisions: ADR-0001, 0019–0024.

## Scope

**Included**: workspace skeleton, pinned toolchain, xtask verification entry point, dependency-direction checks, test harness (pixel snapshots, benchmarks, performance baselines), convention documents, shared Claude Code / Codex project skills, local Git hook.

**Excluded**: all framework feature code; rendering, windows, and example screenshots (integrated into the harness in M0/M1); CI (ADR-0019); release workflows.

## Resulting directory structure

```
oxiview/
├── Cargo.toml                 # Virtual manifest: members, workspace.package/dependencies/lints
├── rust-toolchain.toml        # 1.97 + rustfmt + clippy
├── rustfmt.toml  clippy.toml  deny.toml
├── .cargo/config.toml         # alias: xtask = "run -p xtask --"
├── .gitignore  .gitattributes # Binary *.png files and line-ending rules
├── LICENSE-MIT  LICENSE-APACHE  README.md
├── AGENTS.md                  # Canonical conventions entry point
├── CLAUDE.md -> AGENTS.md
├── .agents/skills/            # Canonical skills
│   ├── adr-glossary/SKILL.md
│   ├── scaffold/SKILL.md
│   ├── review-checklist/SKILL.md
│   └── verify/SKILL.md
├── .claude/skills -> ../.agents/skills
├── crates/
│   └── oxiview-testing/       # This milestone's only framework-internal crate: snapshot/baseline harness
├── xtask/
│   ├── src/…
│   └── layers.toml            # Dependency rules: single source of truth
└── docs/
    ├── architecture.md        # Context map explaining layers.toml, with a dependency diagram
    ├── conventions/
    │   ├── unsafe-panic-errors.md
    │   ├── api-stability.md
    │   ├── dependencies.md
    │   └── git-workflow.md
    ├── adr/  plans/  roadmap.md
```

## Steps (small to large, each independently verifiable)

### S1 Workspace skeleton

- Virtual manifest; `members = ["crates/*", "xtask"]`; `[workspace.package]` centralizes `edition = "2024"`, `rust-version = "1.97"`, `license = "MIT OR Apache-2.0"`, `version = "0.0.0"`, and `publish = false`.
- `[workspace.lints]`: `unsafe_code = "deny"` (crates needing unsafe opt in explicitly under S6), `missing_docs = "warn"`, selective Clippy pedantic checks, and `deny` for `unwrap_used` / `expect_used` / `panic` in library code.
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.gitignore`, `.gitattributes`, licenses, README.

Acceptance: `cargo check --workspace` and `cargo fmt --check` pass.

### S2 xtask skeleton and consistent output

- Subcommands: `fmt` (`--check`), `lint`, `test`, `verify`, `doctor`.
- `doctor`: check the toolchain and external tools (`cargo-deny`, later `cargo-fuzz` and `cargo-semver-checks`). Print installation commands for missing tools without installing automatically. verify starts with doctor and exits with 2 if tools are missing.
- `verify` runs checks sequentially and **does not stop at the first failure**, aggregating results at the end.
- Every run writes `target/xtask/report.json` with each check's name, status, duration, failure summary, and artifact paths. The terminal shows a human-readable summary; `--json` restricts stdout to JSON for agents.
- Exit codes: 0 for success; 1 for failed checks; 2 for xtask errors such as missing environment requirements.

Acceptance: `cargo xtask verify` passes on the empty workspace and produces report.json. An intentional formatting error produces exit code 1 and identifies the file in the report.

### S3 Dependency-direction checks

- `xtask/layers.toml` defines each crate's context, layer, and allowed dependencies. List all planned crates (ADR-0021), automatically skipping those not yet created.
- `cargo xtask deps` reads `cargo metadata` to detect workspace dependency violations: adapters depend only on their own ports; only `oxiview-app` may depend on both ports and adapters; `objc2*` appears only in `oxiview-platform-macos` (ADR-0022).
- The same subcommand invokes `cargo-deny check` for licenses, advisories, duplicate versions, and sources.
- `docs/architecture.md` explains the rules with an X6 / text dependency diagram. Maintain the rules themselves only in layers.toml.

Acceptance: xtask fixtures (small temporary workspaces) detect lower-to-upper dependencies, cross-adapter dependencies, and objc2 in the wrong crate.

### S4 Pixel snapshot harness (oxiview-testing)

- Compare two RGBA images with per-pixel tolerance and an allowed differing-pixel ratio, producing diff.png.
- Store baselines at `<crate>/tests/snapshots/<name>.png`, committed directly without Git LFS. Mark `*.png` binary in `.gitattributes`; generate actual / diff under `target/xtask/snapshots/` on failure.
- Update workflow: `cargo xtask snapshot --review` lists pending items; `--accept <name|all>` replaces baselines. Default tests never overwrite them automatically.
- Before the M0 renderer exists, self-test with generated images such as gradients and checkerboards.

Acceptance: self-tests cover exact matches, within tolerance, beyond tolerance, missing baselines, and acceptance.

### S5 Benchmarks and performance baselines

- Use `divan` as the default microbenchmark framework. Provide a workspace example benchmarking S4 image comparison, both useful in itself and validating the workflow.
- `perf/baselines/<machine-id>.json` records model, CPU, memory, OS, toolchain, metric values, and permitted regression thresholds.
- `cargo xtask bench` runs benchmarks and compares against this machine's baseline, failing above thresholds; `--record` writes a baseline. Other machines' baselines are shown without affecting results.
- Reserve names for ADR-0001's three hard targets (`input_to_pixel_ms`, `idle_cpu_percent`, `list_100k_scroll_dropped_frames`), with real measurement in M1/M7.
- Ordinary verify skips benchmarks for speed; `verify --full` includes them.

Acceptance: record a baseline on this machine; intentionally slow the measured function and confirm bench reports a regression and fails.

### S6 Convention documents

- `AGENTS.md`: concise entry point (preferably at most 150 lines) with project positioning, hard rules, command reference, and documentation index. Link details in `docs/conventions/`.
- `unsafe-panic-errors.md`: unsafe only in platform/rendering adapters with `// SAFETY:` at each use; no library `unwrap`/`expect`/`panic` except tests and xtask. No panics in frame loops. Recoverable errors return `Result`; fatal errors are reported before safe shutdown. Use `thiserror`.
- `api-stability.md`: all internal crates unstable; public stable / unstable-feature tiers and deprecation process; establish `cargo-semver-checks` before first release.
- `dependencies.md`: review licenses, maintenance, pure Rust status, and transitive footprint; centralize versions in `[workspace.dependencies]`; manage upstream patches with `[patch]`, rationale, and upstream issues.
- `git-workflow.md`: Conventional Commits with English types and descriptions by default; small main commits or short-lived branches during single-maintainer development; three ADR criteria; mandatory `cargo xtask verify` before commits.
- `CLAUDE.md -> AGENTS.md` symlink.

Acceptance: every path referenced from AGENTS.md exists, checked by xtask's relative-link validation; the symlink resolves.

### S7 Project skills

Skills use only the `name` and `description` frontmatter fields (ADR-0023).

| Skill | Purpose | Required xtask capabilities |
|---|---|---|
| `verify` | Command selection, report.json interpretation, snapshot and baseline approval rules (human confirmation before accept or record) | `verify`, `snapshot`, `bench` |
| `adr-glossary` | Three ADR criteria, numbering, README index synchronization, glossary format, and discouraged terms | Documentation link checks |
| `scaffold` | `cargo xtask new-crate <name> --context <ctx>` generates directories, inherits workspace settings, and registers in layers.toml; widget/capability checklists precede M7/M9 code templates | `new-crate` |
| `review-checklist` | Dependency direction, unsafe/SAFETY, panic paths, public API changes, third-party type leakage, test/snapshot coverage, tracing spans | `deps`, `lint` |

- `.claude/skills -> ../.agents/skills` symlink.
- Implement `cargo xtask new-crate` in this step; exercise it by creating and then removing a temporary crate.

Acceptance: Claude Code and Codex each manually confirm discovery of all four skills; generated crates pass verify immediately.

### S8 Local hook and completion

- `cargo xtask install-hooks`: optional explicit installation of a pre-push hook running verify; no automatic installation.
- Update the roadmap and ADR index; set this plan's status to done.
- Run verify from a fresh checkout cloned into a temporary directory to detect hidden local-state dependencies.

Acceptance: fresh-checkout verify passes, with every report.json check marked pass.

## Definition of done (M-1)

1. A fresh checkout passes `cargo xtask verify` and produces a machine-readable report.
2. Dependency-direction, snapshot, and baseline checks each have self-tests proving they detect violations or regressions.
3. AGENTS.md, four conventions, and four skills are ready and readable by Claude Code and Codex.
4. `cargo xtask new-crate` is ready for M0 to create `oxiview-base`, `oxiview-scene`, and `oxiview-render-cpu`.

## Confirmed trade-offs

| Question | Decision |
|---|---|
| Lint strictness | Strict: no library unwrap/expect/panic; unsafe denied by default with explicit per-crate opt-in; selective Clippy pedantic; missing_docs warns; enforced from the first line |
| Snapshot storage | Commit directly without Git LFS; reassess migration if repository size grows |
| External Cargo tools | The agent installs cargo-deny before execution begins; doctor checks and suggests installation commands without installing automatically |
| Execution | Finalize this plan without immediate execution; execute S1–S8 in order, verifying and committing each step separately |

## Execution record

S1–S7 deliverables were checked against commits and current files; the coordinator commits S8 after acceptance. Work was limited to engineering conventions and the test harness, without framework features.

| Step | Actual deliverables | Deviations and reasons |
| --- | --- | --- |
| S1 | `90bd449`: virtual workspace, Rust 1.97 stable, edition 2024, shared dependencies/lints, licenses, README, configuration. | Initially members contained only xtask because Cargo rejects unmatched crates/*; S4 added the wildcard after creating the first crate. S1/S2 shared a commit to provide a runnable entry point. |
| S2 | `90bd449`: fmt, lint, test, doctor, verify; common Check, logs, JSON reports, exit codes. | doctor currently requires only Rust and cargo-deny. cargo-fuzz and cargo-semver-checks wait for their milestones instead of requiring unused tools. |
| S3 | `b365ebf`: planned crates in layers.toml, dependency-direction and external-ownership checks, violation fixtures, cargo-deny review, architecture docs. | Advisory databases use target/xtask/advisory-dbs to avoid requiring writable user directories; architecture diagrams use text. |
| S4 | `b41cd21`: oxiview-testing RGBA8/PNG, tolerant comparison, diffs, snapshot assertions, pending metadata; review/accept, self-tests, snapshot conventions. | Acceptance also supports crate/name to disambiguate snapshots; generated images substitute for the nonexistent renderer. |
| S5 | `77c8086`: Divan comparison benchmarks, local baseline, bench/record, verify --full, regression checks and self-tests, performance conventions. | Median of three run medians reduces noise; Divan is pinned with strict terminal-output parsing. macOS falls back to system_profiler when sysctl is sandboxed. Hard targets remain pending. |
| S6 | `4fe53dc`: canonical AGENTS.md, relative CLAUDE.md symlink, four conventions, internal verify docs check. | Link checks validate inline relative targets only, not anchors; an authorized coordinator performed the commit. |
| S7 | `75cde84`: four project skills, relative .claude/skills symlink, new-crate scaffolding/tests, skill checks; commit records confirm both tools discover all four skills. | new-crate creates only registered crates to avoid guessing dependency rules or creating planned crates early. Frontmatter allows only single-line unquoted name/description strings. Codex sandbox restrictions on skill directories and Git writes required coordinator patch application and commit. |
| S8 | Explicit hook installation, idempotence, conflict rejection, force backup, matching uninstall; isolated Git fixtures cover lifecycle, path configuration, worktrees, and execution; command docs completed, M-1 marked done. | The repository .git was read-only; installation tests used isolated target/xtask repositories. No hook was installed in this repository and no commit or push was made. Existing backups block replacement to protect originals. The ADR index already covered 0019–0024 and remained unchanged after review; no new decisions. |

S8 local acceptance: all 43 `cargo test -p xtask` tests passed, including 9 hook tests. All 7 `cargo xtask verify` checks passed, including formatting, strict Clippy, tests, dependencies, and documentation. After cloning into `target/xtask/fresh-clone`, both the original S1–S7 checkout and the checkout with S8 files copied in passed verify. Since the repository could not be committed, the latter was verification of a fresh checkout plus S8 files, not a claim that S8 was committed. The temporary checkout was removed afterward; reports remain under `target/xtask/`.

### Remaining items

- [ADR-0013](../adr/0013-external-content-compositing.md) exposes a shared wgpu device publicly, conflicting with [ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) and the convention against third-party types in public APIs. Resolve through an ADR before M10; M-1 does not change interface boundaries early.
- Machine ID probing and symlink checkout on Windows / Linux still need native-platform testing.
- [ADR-0001](../adr/0001-goals-and-non-goals.md) hard targets remain pending: input_to_pixel_ms and idle_cpu_percent integrate in M1; list_100k_scroll_dropped_frames in M7. Reserved metrics do not imply targets have been met.
