# oxiview

[English](README.md) | **简体中文**

oxiview 是一个用 Rust 编写的自绘 UI 框架，面向 macOS、Windows 和 Linux 原生桌面。当前可运行的原生后端是 macOS，另有用于确定性测试的无窗口后端。工具链固定为 Rust stable 1.97.0，使用 edition 2024。

M1 已完成首个原生窗口、独立窗口线程上的按需调度，以及通过 GPU 绘制和呈现圆角矩形。原生视觉验收、全新克隆验证和本机性能基线比较均已通过。[M1 执行记录](docs/plans/m1-first-window.md)

M2 的 Layout Lab 已支持保留式 UI 节点树、Flex / Grid / Block 布局、层叠、裁剪、指针路由和捕获。贪吃蛇用 M3 的 Signal、函数组件和 Rust 源文件中的可选 `view!` 表达式构建界面。M4 负责文字整形，按显示缩放率栅格化字形，再由 GPU 合成。M2–M4 仍有各自的验收项目。[M2 计划](docs/plans/m2-node-tree-and-layout.md) · [M3 计划](docs/plans/m3-reactivity-and-components.md) · [M4 计划](docs/plans/m4-text-and-snake.md)

## 贪吃蛇

在 macOS 14 或更高版本上，使用可见的桌面会话和支持 Metal 的 GPU 运行游戏。

```sh
cargo run --release -p snake
```

Enter 开始新局，空格暂停或继续，方向键或 WASD 控制转向。游戏也提供鼠标方向键、速度选择、得分、本次会话最高分和最近几局记录。在准备、暂停和结束状态下，游戏会取消周期性步进。窗口失去激活状态时游戏暂停，返回后需要手动继续。

追加 `-- --debug-overlay` 可在右上角显示应用 FPS、显示刷新节奏、CPU 耗时、场景和布局计数，以及视口缩放率。Classic 速度起步时每 125 ms 移动一格，因此稳定行进时约为 8 应用 FPS。调试层默认关闭，进入空闲状态后只刷新最后一次。

无窗口回放可以检查交互并生成真实字体渲染图。

```sh
cargo run --release -p snake -- --headless --json
```

回放图像和验收报告写入 `target/xtask/snake/`。运行 `./examples/snake/package-macos.sh` 可在该目录生成能直接打开的 macOS 应用包 `Oxiview Snake.app`。[贪吃蛇示例说明](examples/snake/README.md)

## 连续动画 FPS

运行 `cargo run --release -p animation-fps` 可查看实时面板。比较窗口与全屏时，在同一显示器上依次执行两条命令。

```sh
cargo run --release -p animation-fps -- --objects 128 --measure 5 --json
cargo run --release -p animation-fps -- --objects 128 --measure 5 --fullscreen --json
```

示例分别统计成功提交的帧和 Metal 确认上屏的帧。结合报告中的物理像素尺寸，比较实际显示帧率 `displayed_fps` 和帧间隔 `display_gap_p95_ms`。[测量说明](examples/animation-fps/README.md) · [原生帧调度实测](docs/adr/0036-fifo-drawable-readiness.md)

## Layout Lab

在 macOS 14 或更高版本的可见桌面中运行 M2 布局示例。

```sh
cargo run --release -p window -- --layout-demo
```

**ADD / REMOVE** 增删节点，**WRAP / RESIZE** 调整 Flex 布局，**LAYER** 切换堆叠顺序，**RESET** 复位。悬停或点击色块可观察宽度和透明度变化。按住橙色色块拖出裁剪边界或窗口后松开，可检查指针捕获。调整窗口大小可观察重排。Layout Lab 使用位图诊断文字，贪吃蛇使用真实字体。

运行无窗口检查可复现节点创建与删除、旧 ID 失效、重排、Grid 放置、拖动捕获、层叠，以及输入未变化时的重绘抑制。

```sh
cargo run --release -p window -- --layout-demo --headless
```

CPU 渲染的 `initial.png`、`interacted.png`、`resized.png` 写入 `target/xtask/m2-preview/`，供人工检查。M2 的快照、性能基线和完整里程碑验收仍待完成。[窗口示例说明](examples/window/README.md)

## M1 窗口诊断

运行指针跟随示例并开启调试图。

```sh
cargo run --release -p window -- --debug-overlay
```

移动指针、调整窗口大小，或按 Ctrl+Cmd+F 切换全屏。调试图以一个刷新周期为参照线，显示从绘制到提交、从该帧最早输入到提交的耗时。连续输入可能超过参照线。M1 基线采用另一组覆盖不同刷新相位的输入，将实测上屏延迟与人工记录的基线比较。[M1 延迟验收规则](docs/adr/0026-m1-native-latency-gates.md)

运行 CPU 版本可在无窗口环境中生成 `target/xtask/window.png`。

```sh
cargo run --release -p window -- --headless
```

## 本地验证

从仓库根目录运行常规检查。

```sh
cargo xtask verify
```

`verify` 先检查 Rust 1.97 stable 和 cargo-deny，再执行格式检查、Clippy、测试、依赖检查和文档检查。环境检查通过后，后续失败会汇总报告。其他常用命令包括 `cargo xtask fmt [--check]`、`lint`、`test`、`deps` 和 `doctor`。全局选项 `--json` 将标准输出限制为 JSON 报告。每次运行覆盖 `target/xtask/report.json`，子进程日志位于同一目录。各检查项记录名称、状态、耗时、失败摘要及产物路径。退出码 0 表示成功，1 表示检查失败，2 表示环境或工具错误。`doctor` 会提示缺失工具的安装命令。

`cargo xtask verify --full` 额外运行性能基准，在可见的 macOS 会话中测量真实窗口的输入延迟和空闲 CPU，并与本机基线比较。空闲测量期间请保持鼠标静止。缺少基线时检查会失败，记录基线需要事先获得人工确认。[性能验证约定](docs/conventions/testing-performance.md)

Windows 和 Linux 原生后端、焦点与 Action 映射、可编辑文本和 IME，以及组件库安排在后续里程碑。M1 指针示例和测量模式继续使用独立的测试场景。[路线图](docs/roadmap.md) · [术语表](GLOSSARY.md)

许可证为 MIT 或 Apache-2.0，版权归 The oxiview Authors 所有。
