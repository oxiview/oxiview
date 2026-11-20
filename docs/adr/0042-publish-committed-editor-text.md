---
status: accepted
---

# Publish committed editor text separately from inline preedit

On 2026-11-20 the owner chose how inline preedit interacts with document observers, undo and focus changes. The editing core keeps the committed document and its selection before preedit begins. It presents a temporary replacement range, marked text and the input method's selection to the writer. Native surrounding-text and geometry queries see this presentation as well. [Inline IME and Actions](0011-ime-and-action-keymap.md) · [Editing core](0010-pure-rust-text-stack.md)

Starting or updating preedit changes the editor's presentation. The committed-text observer retains its current revision, so Markdown preview, dirty tracking, saving and undo history continue to use the last confirmed document. An input-method commit replaces its target range and publishes the resulting document in one undoable edit. A cancel restores the text and selection that precede the pending replacement. Each edit already committed in the session survives cancellation of later preedit.

On focus transfer, apply native commits already delivered for the outgoing editor in their event order, then cancel any outstanding preedit. Restore the outgoing editor's selection in its own model and give the incoming field focus. Scope later callbacks to their originating session so a delayed candidate cannot insert text into the new field. [Focus recovery](0041-focus-recovery-in-tab-order.md)

For example, a writer selects a heading and types Pinyin. The source shows the marked spelling while the preview shows the current heading. Choosing a candidate changes the heading once. Undo recovers the previous heading and selection. Clicking Find before choosing a candidate leaves the original heading in the document and the query field ready for input. [M6 acceptance session](../plans/m6-text-editing-and-markdown.md#acceptance-session)

Publishing each intermediate spelling would let every live view follow the input method, but document observers would have to distinguish tentative revisions when rendering, saving and recording history. Promoting a pending spelling on blur could preserve the visible letters, but it could also save a phonetic spelling the writer never selected. Keeping preedit in the editor presentation gives observers one document revision per committed change.
