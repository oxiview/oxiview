---
name: review-checklist
description: 审查 oxiview 代码变更或提交前检查依赖、安全边界、公开 API、测试与可观测性时使用。
---

# 代码审查

明确变更范围与获准里程碑，对每项给出发现或适用性判断；M-1 的规则不能作为提前实现框架功能的理由。

- **依赖方向**：运行 `cargo xtask deps`，按 [layers.toml](../../../xtask/layers.toml)核实上下文、端口、适配器与组装根边界；新增依赖核对[依赖公约](../../../docs/conventions/dependencies.md)。
- **unsafe**：按[安全公约](../../../docs/conventions/unsafe-panic-errors.md)核实允许范围，每块中文 `// SAFETY:` 说明不变量与保证来源，unsafe 函数有 `# Safety`；检查 FFI、生命周期与线程亲和性。
- **panic**：库生产路径无 unwrap / expect / panic，也检查索引、assert、todo 等等价路径。追踪帧循环及回调的 panic 路径，错误显式返回或上报并安全退出；例外仅限公约允许的最小作用域。
- **公开 API**：按[稳定性公约](../../../docs/conventions/api-stability.md)检查内部不稳定 API、对外 stable / unstable 等级、feature 与废弃迁移；不宣称运行了未接入的兼容性检查。
- **防腐层**：检查签名、公开字段、类型别名、泛型约束和错误类型的第三方类型泄漏；deps 不能证明类型级边界。
- **测试与快照**：覆盖行为、边界与失败分支；视觉变更有适用快照，基准更新有人工确认，见[快照公约](../../../docs/conventions/testing-snapshots.md)。
- **可观测性**：已有帧、布局、绘制、事件分发关键路径应有 tracing span；M-1 尚无这些路径，不为清单新增框架代码或依赖。
- **文档与术语**：核对 [GLOSSARY.md](../../../GLOSSARY.md)、API 文档和链接，冲突术语指出规范用词。
- **ADR**：同时满足难以逆转、缺少上下文会令人意外、确有权衡时应有 ADR 并同步索引，见[Git 公约](../../../docs/conventions/git-workflow.md)。

运行 `cargo xtask lint`，交付前运行 `cargo xtask verify`；性能相关按[性能公约](../../../docs/conventions/testing-performance.md)追加验证。

按严重程度列出发现，每条包含 `文件:行号`、触发条件或证据、影响与修改建议。区分确定缺陷与待确认问题；无发现时明确说明，并列出实际验证与未覆盖部分。
