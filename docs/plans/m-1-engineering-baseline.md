---
status: done
milestone: M-1
---

# M-1 工程规范：详细计划

目标：在写任何框架代码之前，先让仓库具备“一条命令验证一切”的能力，并把公约、技能、选型和目录约束固定下来。完成后，任何人或 agent 从全新检出开始，执行 `cargo xtask verify` 都能得到确定的通过或失败结果，且失败信息可被机器读取。

相关决策：ADR-0001、0019–0024。

## 范围

**做**：workspace 骨架、工具链固定、xtask 验证入口、依赖方向检查、测试 harness（像素快照、基准、性能基线）、公约文档、项目技能包（Claude Code 与 Codex 共用）、本地 git hook。

**不做**：任何框架功能代码；渲染、窗口、示例截图（M0/M1 再接入 harness）；CI（ADR-0019）；发布流程。

## 产出的目录结构

```
oxiview/
├── Cargo.toml                 # 虚拟 manifest：members、workspace.package/dependencies/lints
├── rust-toolchain.toml        # 1.97 + rustfmt + clippy
├── rustfmt.toml  clippy.toml  deny.toml
├── .cargo/config.toml         # alias: xtask = "run -p xtask --"
├── .gitignore  .gitattributes # *.png 二进制、换行规范
├── LICENSE-MIT  LICENSE-APACHE  README.md
├── AGENTS.md                  # 公约入口（正本）
├── CLAUDE.md -> AGENTS.md
├── .agents/skills/            # 技能正本
│   ├── adr-glossary/SKILL.md
│   ├── scaffold/SKILL.md
│   ├── review-checklist/SKILL.md
│   └── verify/SKILL.md
├── .claude/skills -> ../.agents/skills
├── crates/
│   └── oxiview-testing/       # 本里程碑唯一的框架内 crate：快照与基线 harness
├── xtask/
│   ├── src/…
│   └── layers.toml            # 依赖方向规则（单一事实来源）
└── docs/
    ├── architecture.md        # 上下文映射：解释 layers.toml，含依赖图
    ├── conventions/
    │   ├── unsafe-panic-errors.md
    │   ├── api-stability.md
    │   ├── dependencies.md
    │   └── git-workflow.md
    ├── adr/  plans/  roadmap.md
```

## 步骤（从小到大，每步可独立验收）

### S1 workspace 骨架

- 虚拟 manifest；`members = ["crates/*", "xtask"]`；`[workspace.package]` 统一 `edition = "2024"`、`rust-version = "1.97"`、`license = "MIT OR Apache-2.0"`、`version = "0.0.0"`、`publish = false`。
- `[workspace.lints]`：`unsafe_code = "deny"`（需要 unsafe 的 crate 显式放开，见 S6 公约）、`missing_docs = "warn"`、clippy `pedantic` 选择性开启，`unwrap_used` / `expect_used` / `panic` 在库代码中为 `deny`。
- `rust-toolchain.toml`、`rustfmt.toml`、`clippy.toml`、`.gitignore`、`.gitattributes`、许可证、README。

验收：`cargo check --workspace` 通过；`cargo fmt --check` 通过。

### S2 xtask 骨架与统一输出

- 子命令：`fmt`（`--check`）、`lint`、`test`、`verify`、`doctor`。
- `doctor`：检查工具链版本与外部工具（`cargo-deny`，以及后续的 `cargo-fuzz`、`cargo-semver-checks`）是否就绪；缺少时打印安装命令，不自动安装。`verify` 开始前先运行 doctor，缺少工具时以退出码 2 结束。
- `verify` 依次执行各项检查，**不在第一个失败处停止**，最后汇总。
- 每次运行都写入 `target/xtask/report.json`（每项：名称、状态、耗时、失败摘要、产物路径）；终端输出人类可读摘要。`--json` 时只向 stdout 打印 JSON，便于 agent 解析。
- 退出码：0 全部通过；1 有检查失败；2 xtask 自身错误（环境缺失等）。

验收：`cargo xtask verify` 在空 workspace 上通过，并产出 report.json；人为制造一个 fmt 错误时退出码为 1 且报告指出文件。

### S3 依赖方向检查

- `xtask/layers.toml` 定义每个 crate 所属的上下文、层级，以及允许依赖的对象；列出全部规划 crate（ADR-0021），不存在的 crate 自动跳过。
- `cargo xtask deps`：读取 `cargo metadata`，检查 workspace 内依赖是否违反规则；检查适配器 crate 只依赖自己的端口；检查只有 `oxiview-app` 可以同时依赖端口与适配器；检查 `objc2*` 只出现在 `oxiview-platform-macos`（ADR-0022）。
- 同一子命令调用 `cargo-deny check`（许可证、安全公告、重复版本、来源）。
- `docs/architecture.md` 解释规则，并附 X6 / 文本依赖图；规则内容只在 layers.toml 中维护。

验收：xtask 自带一组测试 fixture（临时生成的小 workspace），能检测到“下层依赖上层”“适配器依赖其他适配器”“objc2 出现在错误 crate”三类违规。

### S4 像素快照 harness（`oxiview-testing`）

- 比较两张 RGBA 图像：逐像素容差、允许差异像素比例、输出差异图（diff.png）。
- 基准图存放约定：`<crate>/tests/snapshots/<name>.png`，直接提交进仓库，不使用 Git LFS；`.gitattributes` 把 `*.png` 标为二进制；失败时在 `target/xtask/snapshots/` 下生成 actual / diff。
- 更新流程：`cargo xtask snapshot --review` 列出待更新项；`--accept <name|all>` 覆盖基准图。默认测试从不自动覆盖基准。
- 在 M0 有渲染器之前，用程序生成的图像（渐变、棋盘格）做自测。

验收：自测覆盖“完全一致”“容差内”“超出容差”“基准缺失”四种情况，以及 accept 流程。

### S5 基准与性能基线

- 基准：`divan` 作为默认微基准框架，并提供 workspace 级示例基准（对 S4 的图像比较做基准，既有意义又能验证流程）。
- 性能基线文件 `perf/baselines/<machine-id>.json`：记录机器型号、CPU、内存、系统版本、工具链版本，以及每个指标的数值与允许回归幅度。
- `cargo xtask bench`：运行基准，与当前机器的基线比较，超过阈值即失败；`--record` 写入新基线。其他机器的基线只显示，不判定。
- 为 ADR-0001 的三个硬指标预留指标名（`input_to_pixel_ms`、`idle_cpu_percent`、`list_100k_scroll_dropped_frames`），在 M1/M7 接入真实测量。
- `verify` 默认不跑基准（耗时）；`verify --full` 包含基准。

验收：在本机记录一次基线；人为减慢被测函数后 `bench` 报告回归并失败。

### S6 公约文档

- `AGENTS.md`：精简入口（建议不超过 150 行）。包括项目一句话定位、必须遵守的硬规则、命令速查、文档索引；细节链接到 `docs/conventions/`。
- `unsafe-panic-errors.md`：unsafe 只允许出现在平台与渲染适配器 crate，每处都要有 `// SAFETY:` 注释；库代码禁止 `unwrap`/`expect`/`panic`（测试与 xtask 除外）；帧循环路径禁止 panic，错误分为可恢复（返回 `Result`）与致命（上报并安全退出）；错误类型用 `thiserror`。
- `api-stability.md`：内部 crate 一律不稳定；公开 API 的稳定性等级（stable / unstable feature），以及废弃流程；首次发布前建立 `cargo-semver-checks`。
- `dependencies.md`：新增依赖的审查项（许可证、维护状态、是否纯 Rust、传递依赖规模）；版本统一在 `[workspace.dependencies]`；上游补丁队列的管理方式（`[patch]` + 记录原因与上游 issue）。
- `git-workflow.md`：Conventional Commits（类型英文，描述可中文）；单人阶段直接在 `main` 上小步提交，或使用短期分支；何时必须写 ADR（引用 ADR 的三条判断标准）；提交前必须通过 `cargo xtask verify`。
- `CLAUDE.md -> AGENTS.md` 软链。

验收：`AGENTS.md` 中引用的每个路径都存在（由 xtask 检查文档内的相对链接）；软链可解析。

### S7 项目技能包

技能只使用 `name` 与 `description` 两个 frontmatter 字段（ADR-0023）。

| 技能 | 作用 | 依赖的 xtask 能力 |
|---|---|---|
| `verify` | 何时运行哪个验证命令；如何解读 report.json；快照更新与基线记录的审批规则（必须由人确认才能 accept 或 record） | `verify`、`snapshot`、`bench` |
| `adr-glossary` | 何时写 ADR（三条判断标准）、编号规则、README 索引同步、术语表格式与“避免用词” | 文档链接检查 |
| `scaffold` | 新建 crate：`cargo xtask new-crate <name> --context <ctx>`，自动生成目录、继承 workspace 配置、登记到 layers.toml；控件与能力的模板以检查清单形式先写好，在 M7/M9 补全代码模板 | `new-crate` |
| `review-checklist` | 代码审查清单：依赖方向、unsafe 与 SAFETY 注释、panic 路径、公开 API 变化、第三方类型泄漏（防腐层）、测试与快照覆盖、tracing span | `deps`、`lint` |

- `.claude/skills -> ../.agents/skills` 软链。
- `cargo xtask new-crate` 在本步实现，并用它创建一个临时 crate 走一遍流程后删除，以验证脚手架。

验收：两种工具都能列出这 4 个技能（Claude Code 与 Codex 各手动确认一次）；`new-crate` 生成的 crate 能直接通过 `verify`。

### S8 本地 hook 与收尾

- `cargo xtask install-hooks`：安装 `pre-push` hook，执行 `cargo xtask verify`；可选择安装，不自动安装。
- 更新路线图、ADR 索引；本计划状态改为 `done`。
- 从全新检出（`git clone` 到临时目录）执行一次 `cargo xtask verify`，确认无隐藏的本地状态依赖。

验收：全新检出的 `verify` 通过；report.json 内所有项为 pass。

## 完成定义（M-1 Done）

1. 全新检出执行 `cargo xtask verify` 通过，并产出机器可读报告。
2. 依赖方向、快照、基线三类检查都有“能抓住违规 / 回归”的自测。
3. AGENTS.md、4 份公约、4 个技能已就绪，Claude Code 与 Codex 都能读取。
4. `cargo xtask new-crate` 可用，M0 直接用它创建 `oxiview-base`、`oxiview-scene`、`oxiview-render-cpu`。

## 已确认的取舍

| 问题 | 决定 |
|---|---|
| lint 严格度 | 严格：库代码禁止 `unwrap`/`expect`/`panic`；unsafe 默认禁止，按 crate 显式放开；clippy pedantic 选择性开启；`missing_docs = "warn"`；从第一行代码起生效 |
| 快照基准图存储 | 直接入库，不用 Git LFS；仓库变大后再评估迁移 |
| 外部 cargo 工具 | 开始执行前由 agent 安装 `cargo-deny`；`cargo xtask doctor` 负责检查并提示安装命令，不自动安装 |
| 执行方式 | 本计划只定稿，不立即执行；执行时按 S1–S8 顺序，每步验收后单独提交 |

## 执行记录

S1–S7 的实际产出依据提交记录与当前文件核对；S8 由协调者在验收后提交。全程仅实现工程规范与测试 harness，未实现框架功能。

| 步骤 | 实际产出 | 偏离计划与原因 |
| --- | --- | --- |
| S1 | `04816f6`：虚拟 workspace、Rust 1.97 stable、edition 2024、统一依赖与 lint、许可证、README 和配置文件。 | 初期 `members` 仅含 xtask，避免 Cargo 拒绝无匹配项的 `crates/*`；S4 创建首个 crate 后加入通配符。S1 与 S2 合并提交，以便骨架具备可运行入口。 |
| S2 | `04816f6`：fmt、lint、test、doctor、verify；统一 Check、日志、JSON 报告与退出码。 | doctor 当前只要求 Rust 与 cargo-deny；cargo-fuzz、cargo-semver-checks 留到对应里程碑，避免强制安装尚未使用的工具。 |
| S3 | `39a6c02`：layers.toml 登记规划 crate，依赖方向与外部依赖归属检查、违规 fixture、cargo-deny 审查、架构文档。 | 公告数据库放在 `target/xtask/advisory-dbs`，避免依赖可写的用户目录；架构图采用文本形式。 |
| S4 | `e26d7fb`：oxiview-testing 的 RGBA8/PNG、容差比较、差异图、快照断言与 pending 元数据；snapshot review/accept、自测与快照公约。 | 接受操作额外支持 `crate/name`，用于消除同名快照歧义；尚无渲染器，使用程序生成图像。 |
| S5 | `7a88908`：Divan 图像比较基准、本机基线、bench/record、verify --full、回归判定与自测、性能公约。 | 采用三轮 median 的中位数降低噪声；Divan 版本固定并严格解析终端输出。macOS 沙箱限制 sysctl 时回退到 system_profiler；三项硬指标只记录 pending。 |
| S6 | `811f960`：AGENTS.md 正本、CLAUDE.md 相对软链、四份公约、verify 内部 docs 检查。 | 链接检查限定内联相对链接目标，不验证锚点；提交由有权限的协调者执行。 |
| S7 | `b75beeb`：四个项目技能、.claude/skills 相对软链、new-crate 脚手架及测试、技能检查；提交记录确认两种工具均能发现四个技能。 | new-crate 只创建已在 layers.toml 登记的 crate，避免猜测依赖规则或提前创建规划 crate。技能 frontmatter 限定为单行无引号字符串，只含 name、description。Codex 沙箱对技能目录与 Git 写入有限制，技能补丁应用与提交由协调者完成。 |
| S8 | install-hooks 显式安装、幂等检查、冲突拒绝、force 备份、匹配卸载；独立 Git fixture 覆盖生命周期、路径配置、worktree 与 hook 执行；补齐命令说明，路线图 M-1 标记完成。 | 当前仓库 .git 只读，安装测试仅在 `target/xtask/` 下独立仓库执行；未在当前仓库安装 hook，未提交或推送。备份已存在时拒绝覆盖，避免丢失原 hook。ADR 索引已完整覆盖 0019–0024，本步未新增决策，核对后保持不变。 |

S8 本地验收：`cargo test -p xtask` 的 43 项测试通过（其中 9 项为 hook 测试）；`cargo xtask verify` 的 7 项检查全部为 pass，包含格式、严格 Clippy、测试、依赖与文档检查。成功克隆到 `target/xtask/fresh-clone` 后，原始 S1–S7 检出及复制 S8 变更文件后的检出分别通过 verify。由于本仓库不能提交，后者是“全新检出 + S8 文件”的验证，不声称 S8 已提交；临时检出在验证后删除，报告保留于 `target/xtask/`。

### 遗留事项

- [ADR-0013](../adr/0013-external-content-compositing.md) 的共享 wgpu 设备进入公开 API，与[ADR-0021](../adr/0021-crate-layout-strategic-ddd.md)及公约“第三方类型不得出现在公开 API”存在张力，需在 M10 前通过 ADR 决策；M-1 不提前修改接口边界。
- Windows / Linux 上的机器 ID 探测与软链检出尚未实测，需要对应原生平台验证。
- [ADR-0001](../adr/0001-goals-and-non-goals.md) 三项硬指标仍为 pending：`input_to_pixel_ms`、`idle_cpu_percent` 在 M1 接入，`list_100k_scroll_dropped_frames` 在 M7 接入；不把预留指标视为已达标。
