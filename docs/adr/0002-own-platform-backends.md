---
status: accepted
---

# 平台层：从第一天自研三端后端，不使用 winit

窗口、事件循环、输入与系统服务直接对接各平台：macOS 对接 AppKit，Windows 对接 Win32，Linux 对接 Wayland 与 X11。不依赖 winit 或其他通用窗口库。原因是框架要求输入到像素 ≤ 1 帧、resize 不闪烁、自绘标题栏能完整接入系统行为（Snap Layouts、红绿灯按钮），还要多窗口多线程（ADR-0003）。这些都需要直接控制事件循环和呈现时机，通用窗口库的抽象会成为上限。

Linux 上 **Wayland 优先、X11 兼容**：新能力先在 Wayland 上实现；X11 保证可用，但不强求功能对齐。

系统服务（菜单、托盘、对话框、剪贴板、拖放、通知、光标、窗口材质等）以**能力 trait** 暴露。应用通过 `capabilities()` 在运行时查询当前平台是否支持某项能力，框架不对缺失的能力做静默降级。

## Considered Options

- **先用 winit，留自研出口**：起步快，但多线程窗口、IME 候选框定位、resize 同步呈现在 winit 上都要绕路，迁移成本会在后期集中爆发。
- **长期使用 winit**：上限受上游约束，被否决。

## Consequences

- 要为三套平台后端、两套 Linux 协议维护测试矩阵。
- IME、无障碍桥接（AccessKit 适配器）、DPI 变化都要在自研后端中各自接入。
