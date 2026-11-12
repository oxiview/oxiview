---
status: accepted
---

# 构建：纯 Cargo workspace + xtask，固定 stable 工具链，edition 2024

构建系统只用 Cargo，不引入 Bazel、Buck2、just、make 等额外工具。根 `Cargo.toml` 是一个虚拟 manifest，只声明 workspace，并通过 `[workspace.package]`、`[workspace.dependencies]`、`[workspace.lints]` 统一 edition、版本、依赖版本和 lint，各 crate 继承这些配置。任务自动化写成 Rust 的 `xtask` crate（ADR-0019）。

`rust-toolchain.toml` 固定一个具体的 stable 版本（初始为 1.97），edition 2024，并声明 MSRV 与之相同。nightly 只用于工具类任务（miri、cargo-fuzz），不进入构建产物，也不使用任何不稳定特性。

## Considered Options

- **Cargo + just**：任务写法简洁，但多一个外部依赖，复杂逻辑仍要写脚本。
- **Bazel / Buck2**：可复现性和缓存最好，但与 rust-analyzer 和 Cargo 生态的配合成本高。
