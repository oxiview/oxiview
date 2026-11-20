---
status: accepted
---

# Give explicit local properties precedence over theme state rules

Themes supply widget defaults by type and state. A component author needs to know whether choosing a background also controls that background while a button is pressed. On 2026-11-20 the owner chose property-by-property precedence. An explicit local value takes precedence over the theme's value for that property in every state. A local state variant can replace the local base value. Properties omitted locally continue to receive their theme values. [Theme rules](0008-typed-style-with-css-vocabulary.md)

Snake's Restart button is the acceptance example. The theme supplies a blue pressed background, control padding and a keyboard focus ring. The component declares a red background.

| Local declaration | Button state | Expected result |
| --- | --- | --- |
| Red background | Pressed and keyboard-focused | The background stays red. The theme supplies padding and the visible focus ring. |
| Red background and a dark red pressed background | Pressed and keyboard-focused | The local pressed variant supplies dark red. The same padding and focus ring apply. |
| Padding only | Pressed | The local padding and the theme's blue pressed background both apply. |

Giving a theme's state rule priority over the local base would automatically supply its pressed feedback. It would also allow an external theme edit to change the color of a control whose author explicitly chose that color. The accepted order lets the author choose pressed feedback for that property. [M5 authoring sketch](../plans/m5-focus-keyboard-and-themes.md#state-variants-tokens-and-hot-reload)
