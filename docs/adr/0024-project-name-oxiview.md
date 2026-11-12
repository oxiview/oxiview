---
status: accepted
---

# 项目命名：oxiview

框架正式命名为 **oxiview**：oxide（氧化物，即铁锈，暗指 Rust）+ view（视图）。对外总入口 crate 是 `oxiview`，内部 crate 统一使用 `oxiview-<子系统>` 前缀（ADR-0021）。Rust 代码中的 crate 名写作 `oxiview_*`，类型前缀不强制带项目名。

定名时（2026-11-12）核实的可用性：

- crates.io：`oxiview` 以及规划中的全部 16 个子 crate（`-base`、`-reactive`、`-scene`、`-render-cpu`、`-render-wgpu`、`-text`、`-layout`、`-platform`、`-platform-macos`、`-platform-headless`、`-core`、`-style`、`-widgets`、`-app`、`-macros`、`-testing`）均未被占用；近似名 `oxi-view`、`oxi_view`、`oxiviews`、`oxyview` 也未被占用。
- GitHub：组织名 `oxiview` 未被占用，也没有同名仓库。
- 域名：`oxiview.dev`、`oxiview.rs`、`oxiview.io` 未注册；`oxiview.com` 已被注册。

## Considered Options

- **`rui`**：crates.io 上已有同领域的 Rust UI 库（audulus/rui）。
- **`rsui`**：crates.io 上已有同领域的 Rust 跨平台 UI 库；GitHub 组织名已被占用。
- **`rstyle`**：名称全部可用，但容易被理解为样式库或代码格式化工具，并且与样式子系统 `-style` 重名。
- **`newui`**：名称全部可用，但过于通用，搜索结果会被 JetBrains “New UI” 淹没，名字也会随时间过时。
- **`viewloom`、`heddleui`** 等：可用，最终选择了更短、Rust 意象更明确的 oxiview。

## Consequences

- 建议尽早在 crates.io 占位发布 `oxiview` 的 `0.0.0` 空包，并注册 GitHub 组织 `oxiview` 与域名 `oxiview.dev`。这些都是对外操作，需要单独确认后再执行。
- 仓库目录名 `rust-ui` 不属于代码命名，可以保留，也可以在合适时改名为 `oxiview`。
