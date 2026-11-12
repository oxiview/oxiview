---
name: scaffold
description: 创建 oxiview crate 或规划控件、平台能力脚手架时使用，核对已登记上下文并调用 new-crate。
---

# 脚手架

## 新建 crate

1. 确认本次里程碑已授权创建；阅读[架构](../../../docs/architecture.md)与 [layers.toml](../../../xtask/layers.toml) 中的 context、role、allow。
2. crate 必须已登记；未登记先讨论限界上下文与依赖规则，不随意登记。名称为 `oxiview-*` 或对外入口 `oxiview`。
3. 从仓库根运行 `cargo xtask new-crate <name> --context '<context>'`，context 使用规则中的精确值。未登记、context 不符、目录已存在均退出 2；命令不修改规则表、不覆盖目录。
4. 核对仅生成 `crates/<name>/Cargo.toml` 和 `src/lib.rs`：继承 workspace package 字段（含 publish）和 lints，crate 级中文文档引用上下文与角色。通配 members 自动收录。
5. 按当前任务需要添加依赖，遵循[依赖公约](../../../docs/conventions/dependencies.md)，版本统一在根 workspace.dependencies，各 crate 继承，不预填未来依赖或功能。
6. 运行 `cargo xtask verify`，审查文件与 Cargo.lock。确需 unsafe 时遵循[安全边界](../../../docs/conventions/unsafe-panic-errors.md)，不预先放开 lint。

## 控件检查清单

此处只规划，代码模板在 M7 补充，当前不生成控件实现。

- 确认所属 crate 与依赖边界。
- 预留节点语义字段与无障碍扩展点。
- 列出 hover、pressed、focused、disabled 等适用状态变体。
- 规划正常与边界状态的像素快照测试，遵循[快照公约](../../../docs/conventions/testing-snapshots.md)。

## 平台能力检查清单

此处只规划，代码模板在 M9 补充，当前不生成能力实现。

- 明确能力 trait 的自有类型边界与错误语义。
- 规划 `capabilities()` 查询及不支持时的行为；这是后续接口要求，当前尚无实现。
- 规划无头平台后端实现及测试。
- 列出 macOS / Windows / Linux / 无头平台后端支持矩阵，区分支持、不支持与待实现。
