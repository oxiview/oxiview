---
status: accepted
---

# 组件 API：Builder 链式调用为核心，函数组件 + props 结构体

视图的核心 API 是普通 Rust 的 Builder 链式调用，例如 `v_stack().gap(8.0).child(text(name))`。这样 IDE 补全、类型检查和报错信息都保持原生 Rust 体验，也没有宏带来的编译期成本。宏只作为语法糖，展开结果必须就是 Builder 调用。后续可能借鉴 JSX 的思想提供类 JSX 的 `view!` 宏，但它始终是 Builder 之上的可选层，不能成为必须依赖。

组件就是函数：`fn(props) -> impl View`，只在挂载时执行一次（ADR-0005）。props 是普通结构体，由派生宏生成 Builder；事件回调使用统一的 `Callback` 类型。

## Considered Options

- **宏 DSL 为主**（rsx! / view!）：写法简洁，但宏内的补全和报错体验较差，被降级为可选语法糖。
- **外部 DSL 文件**（Slint 式）：静态优化能力强，但逻辑要在两种语言之间切换，并且需要一套额外的编译器和 LSP，被否决。
- **trait 组件**：更接近 OOP 控件，但与“组件只执行一次 + 信号驱动”的模型不匹配。

## Consequences

- Builder 的类型必须控制泛型嵌套深度（例如在子节点边界做类型擦除），避免像强类型视图树那样拖慢编译。
- 将来的类 JSX 宏必须保证一一对应：每个宏写法都能直接写成等价的 Builder 代码。
