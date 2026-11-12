# oxiview 工程公约

oxiview 是只面向 macOS / Windows / Linux 原生桌面平台的 Rust 自绘 UI 框架；当前处于 M-1 工程规范里程碑，M-1 完成后进入 M0。

## 硬规则

- 只做原生平台，不使用 WebView、HTML/CSS、WASM 或浏览器目标；只实现本次指定的里程碑步骤。
- 使用 Rust stable 1.97、edition 2024，不在框架构建中使用 nightly 或不稳定特性。
- 提交前必须通过 `cargo xtask verify`；验证只在本地执行（[ADR-0019](docs/adr/0019-local-only-verification.md)）。
- 库代码禁止 unwrap/expect/panic；测试、xtask、示例及编译期断言的有限例外见[错误公约](docs/conventions/unsafe-panic-errors.md)。
- unsafe 仅限平台与渲染适配器；每个 unsafe 块写 `// SAFETY:`，unsafe 函数写 `# Safety`，详见[安全边界](docs/conventions/unsafe-panic-errors.md)。
- 依赖方向以 [xtask/layers.toml](xtask/layers.toml) 为准；第三方类型不得出现在公开 API，须经防腐层转换。
- 内部 crate 使用 `oxiview-*`、版本 `0.0.0`、`publish = false`；既有工程工具 `xtask` 与对外入口 `oxiview` 按 ADR 命名。
- 依赖版本统一在根 `[workspace.dependencies]`，crate 用 `xxx.workspace = true` 继承；新增依赖遵循[依赖公约](docs/conventions/dependencies.md)。
- 快照 `--accept` 与性能基线 `--record` 必须先经人确认；不得用更新基线掩盖失败。
- 技能正本只在 `.agents/skills/` 创建，不得在 `.claude/skills/` 下直接新增（[ADR-0023](docs/adr/0023-agent-conventions-dual-harness.md)）。
- 注释和文档用中文，代码标识符用英文；术语使用 [GLOSSARY.md](GLOSSARY.md) 的规范用词。
- 决策同时满足“难以逆转、缺少上下文会令人意外、确有权衡”时必须写 ADR，并同步索引，见[Git 公约](docs/conventions/git-workflow.md)。
- 只维护本文件正本；`CLAUDE.md` 必须是解析到本文件的相对路径软链。
- 未经所有者明确要求不得推送远端。

## 命令速查

以下命令均以 `cargo xtask` 开头；不自动安装工具。全局 `--json` 只向 stdout 输出 JSON 报告。

| 子命令 | 用途 |
| --- | --- |
| `fmt` / `fmt --check` | 格式化 workspace / 只检查格式 |
| `lint` | workspace 全部 target 的 Clippy，`-D warnings` |
| `test` | workspace 测试，包含像素快照自测 |
| `deps` | 依赖方向与 cargo-deny 审查；先刷新安全公告数据库 |
| `snapshot --review` | 列出待处理快照，人工查看 actual / diff |
| `snapshot --accept <name\|crate/name\|all>` | 经人确认后更新快照基准 |
| `bench` / `bench --record` | 比较本机性能基线 / 经人确认后记录基线 |
| `doctor` | 检查 Rust 1.97 stable 与 cargo-deny；缺失时提示安装命令 |
| `verify` / `verify --full` | doctor 通过后执行 fmt、lint、test、deps、docs；full 追加 bench |
| `help [子命令]` / `--help` | 查看帮助 |

退出码：`0` 全部通过；`1` 检查失败；`2` 环境缺失、参数或 xtask 自身错误（优先于 1）。doctor 失败时 verify 提前返回，其他检查失败仍继续汇总。
每次调用覆盖 `target/xtask/report.json`，各项含名称、状态、耗时、失败摘要与产物路径；日志位于 `target/xtask/<检查名>.log`。
`docs` 是 verify 内部检查项，无独立子命令：检查本文件和 docs 下 Markdown 的内联相对链接目标及 CLAUDE.md 软链，不验证锚点内容。

## 文档索引

- [ADR 索引](docs/adr/README.md)、[路线图](docs/roadmap.md)、[计划目录](docs/plans/)、[M-1 计划](docs/plans/m-1-engineering-baseline.md)
- [架构与上下文映射](docs/architecture.md)、[术语表](GLOSSARY.md)
- [unsafe、panic 与错误](docs/conventions/unsafe-panic-errors.md)、[API 稳定性](docs/conventions/api-stability.md)
- [依赖管理](docs/conventions/dependencies.md)、[Git 工作流](docs/conventions/git-workflow.md)
- [像素快照](docs/conventions/testing-snapshots.md)、[基准与性能基线](docs/conventions/testing-performance.md)

## 技能索引（由 S7 提供）

以下技能将位于 `.agents/skills/`；本步仅预告，不创建技能或相关命令。

| 名称 | 用途 |
| --- | --- |
| `adr-glossary` | 维护 ADR 判断、编号、索引与规范术语；由 S7 提供。 |
| `scaffold` | 按 workspace 与依赖规则创建 crate，提供控件与能力检查清单；由 S7 提供。 |
| `review-checklist` | 审查依赖、安全边界、API、测试和可观测性；由 S7 提供。 |
| `verify` | 选择验证命令、解读报告并遵守基线审批规则；由 S7 提供。 |
