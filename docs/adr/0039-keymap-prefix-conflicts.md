---
status: accepted
---

# Reject ambiguous shortcut prefixes within a focus context

A player can persist a Keymap override in Snake. A complete shortcut should respond as soon as the player enters it. On 2026-11-20 the owner chose to reject a new binding when one complete sequence would be a prefix of another in the same focus context. This refines the layered lookup in [ADR-0011](0011-ime-and-action-keymap.md).

Validate the effective bindings after applying the proposed override. A conflict rejects the edit as a whole, identifies both bindings and preserves the active and saved Keymap. The rule works in both insertion orders. Checking each context independently lets the board and Settings reuse an arrow key for different Actions. [M5 design](../plans/m5-focus-keyboard-and-themes.md#actions-keymaps-and-player-overrides)

| Existing binding | Proposed binding in the same context | Result |
| --- | --- | --- |
| Command+K | Command+K, then T | Report the conflict. Command+K still fires immediately. |
| Command+K, then T | Command+K | Report the conflict. The two-stroke binding still works. |
| Command+K, then T | Command+K, then P | Accept both. The second stroke selects the Action. |

Allowing both a short binding and its longer extension would make more shortcuts available. It would also make a previously immediate Action wait for another stroke or a timeout whenever the player added such an extension. The accepted rule makes that ambiguity visible at configuration time.
