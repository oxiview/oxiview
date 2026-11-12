# rust-ui

只面向原生桌面平台的 Rust 自绘 UI 框架，控件从布局到像素全部由框架自行绘制。

## Language

**自绘（Self-drawn）**：
框架只向操作系统申请窗口表面，所有控件的布局与绘制都由框架完成，不使用系统原生控件。
_Avoid_: 原生控件、自渲染

**原生平台（Native platform）**：
macOS、Windows、Linux 三个桌面操作系统，直接通过各自的系统 API 对接，不经过任何 Web 技术。
_Avoid_: 跨端（不加限定时会被误解为包含 Web/移动端）

**自有风格（House style）**：
框架默认提供、三端外观一致的视觉风格，可通过主题替换或自定义。
_Avoid_: 原生观感、默认皮肤

## 平台与线程

**主线程（Main thread）**：
进程启动时的系统线程，只负责接收系统事件和调用必须在其上执行的系统 API，不运行 UI 逻辑。
_Avoid_: UI 线程

**窗口线程（Window thread）**：
运行一个或多个窗口的状态、布局、绘制与呈现的线程。默认每个窗口一个。
_Avoid_: 渲染线程、UI 线程

**共享线程模式（Shared-thread mode）**：
多个窗口被放在同一个窗口线程上运行的配置，同线程窗口之间可以直接共享状态。

**能力（Capability）**：
平台层暴露的一项系统服务（如菜单、托盘、剪贴板），应用可在运行时查询当前平台是否支持。
_Avoid_: 插件、特性

## 帧与响应式

**帧策略（Frame policy）**：
窗口决定何时产出新帧的规则，分为按需、窗口级持续、图层级持续三种。
_Avoid_: 渲染模式、刷新模式

**按需渲染（On-demand rendering）**：
只在状态失效、动画或系统要求时才产出新帧，空闲时不运行帧循环。
_Avoid_: 懒渲染、被动渲染

**信号（Signal）**：
可被依赖追踪的响应式值，变化时直接通知依赖它的计算和节点属性。
_Avoid_: State、Observable、Atom

**作用域（Scope）**：
信号与副作用的所有者，销毁时连带释放其拥有的全部响应式资源。
_Avoid_: Owner、Context

**结构协调（Structural reconciliation）**：
条件或列表等视图结构变化时，按 key 增删、移动节点的过程，是框架中唯一会改变树形状的更新路径。
_Avoid_: diff、重渲染

## 组件与节点

**组件（Component）**：
接收 props、返回视图的函数，在挂载时只执行一次。
_Avoid_: Widget（专指底层可绘制节点类型时才用）、View 函数

**Props**：
组件的输入参数结构体。

**节点（Node）**：
保留树中的一个元素，带有样式、布局结果和绘制内容，由 `NodeId` 标识。
_Avoid_: Element、RenderObject

**布局边界（Layout boundary）**：
尺寸不依赖其内容的节点，其子树的重排不会向外传播。
_Avoid_: relayout boundary

**重绘边界（Repaint boundary）**：
拥有独立合成图层的节点，其子树的重绘不影响图层之外的内容。
_Avoid_: layer（未明确时）

## 样式与布局

**设计 Token（Design token）**：
主题中命名的基础视觉值，如颜色、间距、圆角、字号。
_Avoid_: 变量、主题色

**主题（Theme）**：
一组 Token 加上主题规则，决定控件的默认外观。
_Avoid_: 皮肤、样式表

**主题规则（Theme rule）**：
按“控件类型”或“控件类型 + 状态”为控件设定默认样式的条目，不支持任意选择器与层叠。
_Avoid_: 选择器、CSS 规则

**状态变体（State variant）**：
节点在 hover、pressed、focused、disabled 等交互状态下附加的样式覆盖。
_Avoid_: 伪类

**布局协议（Layout protocol）**：
框架定义的节点测量与定位接口，具体布局算法作为它的实现插入。
_Avoid_: 布局引擎（专指某个实现时才用）

## 文本与输入

**字形图集（Glyph atlas）**：
存放已栅格化小字号字形的 GPU 纹理缓存。
_Avoid_: 字体纹理、glyph cache

**编辑核心（Editing core）**：
框架提供的可编辑文本模型，负责光标、选区、IME 组合态与撤销，所有输入类控件都建在它上面。
_Avoid_: 文本框、TextInput（那是控件）

**预编辑（Preedit）**：
IME 尚未提交的组合中文本，内联显示在光标处。
_Avoid_: composition、候选文字

**Action**：
命名的、与按键无关的用户命令，可由快捷键、菜单或代码触发。
_Avoid_: Command（避免与平台 Cmd 键混淆）、快捷键

**Keymap**：
按焦点上下文分层，把按键序列映射到 Action 的配置。
_Avoid_: 快捷键表、keybinding

## 渲染与合成

**Scene**：
一帧的绘制指令集合，是节点树与渲染后端之间唯一的接口。
_Avoid_: Display List、渲染命令

**SDF 快路径（SDF fast path）**：
在着色器中解析绘制圆角矩形、边框、阴影等常见 UI 图元的专用渲染路径。

**合成图层（Compositing layer）**：
可独立更新、再与其他图层合成的绘制单元，外部纹理内容也以图层形式接入。
_Avoid_: Layer（未明确时）、surface

**原生子窗口（Native child view）**：
嵌入框架窗口中的系统原生视图，始终位于自绘内容之上。
_Avoid_: 原生控件、overlay

## 工程

**无头平台后端（Headless platform backend）**：
不连接真实窗口系统、模拟窗口与输入的平台实现，是平台接口的第二个实现，用于测试与事件回放。
_Avoid_: mock 平台、假后端

**里程碑（Milestone）**：
路线图中交付一个可运行演示、测试和指标的阶段，编号为 M-1 至 M15。
_Avoid_: 阶段、版本

**组装根（Composition root）**：
唯一能同时看到所有端口与具体适配器、并负责把它们接起来的 crate（`rui-app`）。
_Avoid_: runtime、main crate
