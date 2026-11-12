# 架构决策记录

| # | 决策 | 维度 |
|---|------|------|
| [0001](0001-goals-and-non-goals.md) | 目标与非目标：全场景覆盖、按里程碑演进、自有风格、性能硬指标进 CI | 1 目标与非目标 |
| [0002](0002-own-platform-backends.md) | 平台层：自研三端后端，Wayland 优先，能力 trait | 2 平台抽象 |
| [0003](0003-window-threading-model.md) | 线程模型：主线程泵事件，默认每窗口一个 UI 线程 | 2 平台抽象 |
| [0004](0004-frame-scheduling.md) | 帧调度：按需渲染为默认，预留持续渲染，对齐显示器 vsync | 3 帧调度 |
| [0005](0005-fine-grained-reactivity.md) | 响应式：线程局部细粒度信号 + 结构协调，内置本地执行器 | 4 状态与响应式 |
| [0006](0006-builder-api-and-function-components.md) | 组件 API：Builder 为核心，函数组件 + props | 5 组件与 API |
| [0007](0007-single-arena-node-tree.md) | 保留树：单层 arena 节点树，三级脏标记 | 6 保留树 / 失效 |
| [0008](0008-typed-style-with-css-vocabulary.md) | 样式：类型化属性 + CSS 词汇 + 受限主题规则 | 7 样式与主题 |
| [0009](0009-layout-protocol-taffy-default.md) | 布局：自有布局协议，Taffy 为默认实现 | 8 布局 |
| [0010](0010-pure-rust-text-stack.md) | 文本：纯 Rust 文本栈，混合栅格化，自研编辑核心 | 9 文本栈 |
| [0011](0011-ime-and-action-keymap.md) | 输入：IME 内联预编辑 + Action / 分层 Keymap | 10 IME 与输入 |
| [0012](0012-scene-abstraction-wgpu-vello.md) | 渲染：自有 Scene，wgpu + Vello，SDF 快路径，线性色彩 | 11 渲染 |
| [0013](0013-external-content-compositing.md) | 合成：纹理图层 + 原生子窗口嵌入 | 12 合成与外部内容 |
| [0014](0014-accessibility-deferred.md) | 无障碍：推迟实现，节点树保留语义扩展点 | 13 无障碍 |
| [0015](0015-interruptible-animation.md) | 动画：可中断弹簧/缓动，合成层快路径 | 14 动画 |
| [0016](0016-devtools-in-framework.md) | 开发者工具：热重载、Inspector、性能、快照、回放 | 15 开发者体验 |
| [0017](0017-native-system-integration.md) | 原生系统集成：标题栏/材质、偏好、生命周期、分发 | 16 原生系统集成 |
| [0018](0018-macos-first-headless-backend-hedge.md) | 平台顺序：macOS 先行，无头后端对冲，Windows/Linux 最后移植 | 2 平台抽象 |
| [0019](0019-local-only-verification.md) | 验证：只做本地验证，`cargo xtask` 唯一入口，不建 CI | 工程 |
| [0020](0020-cargo-workspace-xtask-stable.md) | 构建：Cargo workspace + xtask，固定 stable，edition 2024 | 工程 |
| [0021](0021-crate-layout-strategic-ddd.md) | 目录与 crate：平铺 + `rui-` 前缀，战略 DDD + 端口/适配器 | 工程 |
| [0022](0022-foundation-libraries.md) | 基础库选型 | 工程 |
| [0023](0023-agent-conventions-dual-harness.md) | Agent 公约：单一正本，Claude Code 与 Codex 共用 | 工程 |
