---
status: accepted
---

# Input: inline IME preedit, Actions, and layered Keymaps

Display IME composition (**preedit**) **inline** inside the input, rendered by the editing core (ADR-0010). Report the cursor rectangle to the platform in real time so the system candidate window follows it. The framework centrally manages focus and Tab order.

Rather than sending raw key events to widgets for shortcut matching, use **Actions and layered Keymaps**. Widgets declare supported Actions; Keymaps map key sequences to Actions, resolving from the innermost focus context outward (inspired by GPUI). Keymaps are configurable and user-rebindable, with platform-appropriate defaults: Cmd on macOS, Ctrl on Windows/Linux, and Alt mnemonics on Windows. Application menu items bind to the same Actions.

## Consequences

- IME reporting runs through the main thread (ADR-0003); the cursor-rectangle hop from window thread to main thread must not visibly lag rendering.
- Menus, command palettes, and shortcuts share Action definitions. Actions must be enumerable and carry descriptions for menus and accessibility.
