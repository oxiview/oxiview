# unsafe、panic 与错误处理

## unsafe 边界

workspace 默认 `unsafe_code = "deny"`。依据 [ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) 和[依赖规则](../../xtask/layers.toml)，当前规划中仅以下平台与渲染适配器可因必要的系统调用或资源交接使用 unsafe：

- `oxiview-platform-macos`
- `oxiview-platform-headless`
- `oxiview-render-cpu`
- `oxiview-render-wgpu`

这是允许范围，不是创建 crate 或使用 unsafe 的要求；无头平台后端和 CPU 实现应优先使用安全 Rust。Windows / Linux 适配器尚未登记，待对应里程碑明确名称并更新规则与本公约；不得据此提前建空壳。其他 crate（包括端口和组装根）保持 workspace 的 deny。

选择 **crate 级 `#![allow(unsafe_code)]`** 显式放开：仅在确有必要的上述适配器 `src/lib.rs` 顶部添加，并写明需要 unsafe 的边界；继续保留 Cargo.toml 的 `[lints] workspace = true`，不覆盖整套 Cargo lints。不得在其他 crate 的模块或函数上局部 allow 绕过规则。

每个 unsafe 块紧邻的 `// SAFETY:` 注释必须用中文说明为何当前调用满足不变量：指针有效性、长度与对齐、初始化、别名与生命周期、资源所有权、线程亲和性中与该处相关的条件及保证来源。unsafe 函数文档必须有 `# Safety` 段，说明调用方义务；其函数体中的不安全操作仍放进带 SAFETY 注释的显式 unsafe 块。安全 API 不得将这些义务隐含转嫁给调用方。

## FFI 边界

FFI 限于适配器内部：核实 ABI、参数布局、空指针、错误码、回调存活期及资源释放配对；把系统对象转换为自有类型后再跨上下文传递。遵守主线程 / 窗口线程约束，不以裸指针或无根据的 Send/Sync 实现绕过线程亲和性。

Rust panic 不得跨 FFI 边界展开，外部异常也不得越过未经定义的 ABI。对进入 Rust 的系统回调，在可展开的配置下于边界捕获意外 panic，转换为宿主认可的失败结果并请求安全退出；不能证明可恢复时不继续使用受影响状态。捕获与上报自身不能再次 panic。`catch_unwind` 不能捕获 abort、进程终止或任意外部异常，不是安全保证，也不能代替输入检查。

## panic 的范围与例外

库的生产路径禁止 `unwrap`、`expect`、`panic!`，也禁止以 `assert!`、越界索引、`todo!`、`unreachable!` 等制造等价的运行时 panic。对外输入、不支持的能力与设备丢失都走显式错误处理。帧循环路径（事件分发、布局、绘制、呈现及其回调）不得主动 panic。

有限例外为测试（含测试专用模块和基准）、xtask、示例，以及 **保证在编译期求值** 的 const 断言。例外只在最小作用域显式 allow 对应 Clippy lint，并说明用途；不能在库根放开。const fn 仍可能运行时调用，不能因标记 const 就允许 panic。示例的简写不能被复制成生产错误处理；测试中的断言不属于运行时恢复策略。

未来运行时在窗口线程入口与 FFI 回调等明确的外层边界捕获可展开的意外 panic，记录上下文并通知应用终止受影响窗口或进程，释放能确定安全释放的资源；不得捕获后继续帧循环，也不得靠 AssertUnwindSafe 掩盖不变量问题。无法维持安全状态时以非零状态终止进程，优先避免再次展开或死锁。这些是后续实现的约束，M-1 尚无运行时捕获器。

## 错误分类与类型

可恢复错误返回 `Result<T, Error>`，由调用方选择重试、替代方案或显示错误；库使用 `thiserror` 定义结构化错误。致命错误同样显式传递到组装根，由应用边界通过 `tracing::error!` 上报原因、窗口/操作上下文并安全退出；没有日志设施时由应用边界提供 stderr 兜底，不静默吞错，不把 panic 当错误通道。可观测性设施随对应里程碑实现，本步不引入依赖或运行时代码。

有错误返回的 crate 以一个顶层 `Error` 枚举作为统一入口，可包裹私有分类；变体按本上下文语义命名。通过防腐层把上游错误映射为自有错误，公开变体字段、类型别名、泛型约束与返回类型不得泄漏第三方错误类型。需要保留原因时用私有包装或标准库错误链，不让其他上下文依赖具体上游错误。无错误返回的 crate 不为满足形式而创建空枚举。

既有 S4 harness 的 `SnapshotError` 等具名错误保持兼容，本步不重构；后续变更统一入口时单独处理迁移。`anyhow` 仅用于工程工具、测试与示例，不进入库的生产 API（[ADR-0022](../adr/0022-foundation-libraries.md)）。
