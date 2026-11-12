---
status: accepted
---

# 目录与 crate：平铺 `crates/` + `oxiview-` 前缀，战略 DDD + 端口/适配器，crate 按里程碑创建

**目录**：所有 crate 平铺在 `crates/` 下，目录名与 crate 名完全一致，用 `oxiview-` 前缀加子系统名表达层级（参考 rust-analyzer、Bevy、wgpu）。示例、跨 crate 集成测试、基准、fuzz 目标、xtask 与文档分别放在根目录的 `examples/`、`tests/`、`benches/`、`fuzz/`、`xtask/`、`docs/`。内部 crate 的版本号统一写 `0.0.0` 并设 `publish = false`，直到决定发布为止。

**战略 DDD**：每个子系统是一个**限界上下文**，对应一个或一组 crate，例如响应式、渲染、文本、布局、平台、UI 核心、控件。上下文之间的依赖方向就是**上下文映射**，写在 `docs/architecture.md` 中，由 `cargo xtask` 检查。第三方库（Taffy、Parley、Vello、objc2）的类型不能出现在本项目的公开 API 中，必须经过所在上下文的**防腐层**转换。跨上下文共享的稳定数据类型（如 Scene）放进独立的**公开语言** crate。不引入实体、聚合、仓储等战术 DDD 模式，因为它们服务于业务领域，不适合技术框架。

**端口与适配器**：平台与渲染以端口（trait）加适配器（实现）组织，例如 `oxiview-platform` 是端口，`oxiview-platform-macos` 和 `oxiview-platform-headless` 是适配器。只有**组装根**（`oxiview-app`）能同时看到端口与具体适配器；对外总入口 `oxiview` 按 feature 从组装根重新导出。

**创建时机**：M-1 只建立依赖规则表和检查工具，检查工具会跳过尚不存在的 crate。每个 crate 在对应里程碑首次需要它时才创建，不预先建空壳。

## Considered Options

- **按层嵌套目录**（`crates/platform/macos/`）：层次直观，但会形成与 Cargo 平坦命名空间平行的第二套层级，时间长了容易失配。
- **`libs/` 与 `crates/` 分开**：可发布与内部的边界一目了然。推迟到真正决定发布时再拆。
- **一次建好全部 crate 空壳**：边界最早落地，但会留下大量没有实现的空 crate。

## Consequences

- 上下文增多后，术语表按上下文拆分为 `GLOSSARY-MAP.md` 加各上下文的 `GLOSSARY.md`。
