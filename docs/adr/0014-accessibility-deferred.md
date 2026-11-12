---
status: accepted
---

# 无障碍：推迟实现，但节点树须保留语义扩展点

无障碍（对接 NSAccessibility / UI Automation / AT-SPI）**不在早期里程碑中实现**，前期资源优先投入渲染、文本和输入。

## Consequences

- 风险：无障碍后补的成本通常很高，因为每个控件都要补语义、焦点和动作，已有控件需要返工。为降低返工，节点树（ADR-0007）和控件 trait 从一开始就预留语义字段（role、label、value、可执行的 Action），只是暂不对接平台。
- 届时优先评估 AccessKit。由于平台层是自研的（ADR-0002），需要编写对应的 AccessKit 适配器。
- 在面向外部用户正式发布前，必须重新审视本条决策，并用新的 ADR 替代它。
