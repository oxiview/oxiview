---
status: accepted
---

# 输入：IME 内联预编辑 + Action / 分层 Keymap

IME 的组合态（preedit）在输入框内**内联显示**，由编辑核心（ADR-0010）渲染。框架实时向平台上报光标矩形，让系统候选框跟随光标。焦点与 Tab 顺序由框架统一管理。

快捷键不当作原始键盘事件交给控件匹配，而是采用 **Action + 分层 Keymap**：控件声明自己能处理哪些 Action，Keymap 把按键序列映射为 Action，按焦点路径从内到外分层解析（参考 GPUI）。Keymap 可以配置、可以由用户重绑，并按平台惯例提供默认值（macOS 用 Cmd，Windows/Linux 用 Ctrl，以及 Windows 的 Alt 助记键）。应用菜单项也绑定到同一套 Action。

## Consequences

- IME 上报要通过主线程完成（ADR-0003），光标矩形从窗口线程到主线程的跳转不能明显滞后于渲染。
- 菜单、命令面板、快捷键共享 Action 定义，Action 必须可枚举并带描述，供菜单和无障碍使用。
