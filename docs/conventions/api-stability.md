# API 稳定性

[ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) 规定内部 crate 一律不稳定，继承 workspace 的 `version = "0.0.0"`、`publish = false`。跨上下文也要遵守自有类型边界；不稳定不等于可以泄漏第三方类型。

## 对外入口

未来对外 facade `oxiview` 分为两个稳定性等级；以下是发布约束，当前不创建 facade 或 feature。

- **stable**：未被 unstable feature 门控且明确纳入稳定承诺的公开 API；按发布版本兼容性规则维护。稳定能力可用普通 feature 控制编译，但不能因此绕过兼容性承诺。
- **unstable**：必须由默认关闭的 `unstable-<领域或能力>` feature 显式启用，名称使用小写英文及连字符（如 `unstable-layout`）；文档标出不稳定原因，可在次版本中改变，不提供兼容性承诺。稳定 API 的签名不得依赖仅在 unstable feature 中存在的类型。

按[路线图原则 2](../roadmap.md)，只有一个实现的抽象一律标为不稳定；第二个实现及测试证明边界成立之后，才评估稳定化，不能以内部实现未来可替换代替验证。crates.io 的 0.0.0 占位包不代表框架已首次发布或承诺稳定 API。

第三方类型须由防腐层封装。[ADR-0013](../adr/0013-external-content-compositing.md) 的共享 wgpu 设备及 [ADR-0022](../adr/0022-foundation-libraries.md) 的重新导出选项，与 S6 的严格公开 API 边界存在张力；本公约保守选择封装，不据此新增公开例外。实现相关能力前须明确决策，历史 ADR 本步不改写。

## 废弃与发布准备

稳定 API 废弃时添加 `#[deprecated(since = "版本", note = "替代入口与迁移方式")]`，在 API 文档及发布迁移说明中提供替代用法，至少完整保留一个后续次版本，之后也只能在允许破坏性变更的版本边界移除。内部不稳定 API 和 unstable API 的变化仍需说明迁移方式，但不承诺相同保留期。

首次实际发布框架前引入 `cargo-semver-checks`，建立与上一发布版本的 stable API 比较，并为 `cargo xtask doctor` 增加安装检查。当前尚未安装或接入该工具，doctor 只检查 Rust 与 cargo-deny；本步不安装、不新增检查，不把未执行的兼容性验证写成已通过。
