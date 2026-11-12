---
status: accepted
---

# 基础库选型

以下基础库直接选定，全 workspace 统一使用：

| 用途 | 选型 | 约束 |
|---|---|---|
| 错误 | `thiserror` | 库 crate 中不使用 `anyhow`；`anyhow` 只用于 xtask 和示例 |
| 可观测性 | `tracing` | 关键路径（帧、布局、绘制、事件分发）必须有 span |
| 几何与画刷 | `kurbo`、`peniko` | 与 Vello 保持一致，在 `oxiview-base` 中重新导出或封装 |
| 结构快照 | `insta` | 像素快照由 `oxiview-testing` 自行实现比对 |
| 基准 | `divan`（微基准）、`criterion`（需要统计报告的场景） | 基线结果由 xtask 管理 |
| 属性测试 | `proptest` | |
| 模糊测试 | `cargo-fuzz` | 唯一允许使用 nightly 的场景之一 |
| macOS 绑定 | `objc2` 系列（`objc2-app-kit`、`objc2-metal`、`objc2-quartz-core` 等） | 只允许出现在 `oxiview-platform-macos` 中 |
| 依赖审查 | `cargo-deny` | 检查许可证、安全公告、重复版本，由 `cargo xtask` 在本地调用 |

新增基础依赖按公约中的依赖管理流程审查，影响范围大的选型要写 ADR。
