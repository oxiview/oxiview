---
status: done
milestone: M5
---

# M5 focus, keyboard, and themes

M5 brings keyboard navigation, configurable Actions and live themes to a running native application. The owner chose a separate fork of the existing precompiled game for acceptance. The M3 and M4 examples still run their original replays. [Snake M5 in .oxi](../../examples/snake-m5-oxi/README.md) · [Roadmap](../roadmap.md) · [M4 acceptance](m4-text-and-snake.md#acceptance-record)

## Owner decisions

The owner selected these behaviors on 2026-11-20.

| Decision | Observable result in Snake |
| --- | --- |
| Add settings inside the existing game window. | The board, game controls and settings are part of the same retained screen. |
| Settings are an ordinary inline section. | Tab and Shift+Tab can move between settings, the board and the other controls in page order while settings are open. |
| Repair focus immediately if the focused control disappears or becomes ineligible. | With no explicit return target, focus moves to the next eligible stop in the original order, or to the preceding stop at the end. An empty sequence leaves focus clear. [Focus recovery](../adr/0041-focus-recovery-in-tab-order.md) |
| Players rebind shortcuts in settings and keep them across launches. | The player chooses an Action, presses a new shortcut and can use it immediately. Starting Snake again restores the binding. Snake owns persistence. The framework validates and applies the Keymap. |
| A complete shortcut responds immediately in its focus context. | If one complete sequence would be a prefix of another, show the conflict and keep the previous binding. Multi-stroke shortcuts can share an incomplete prefix. [Keymap decision](../adr/0039-keymap-prefix-conflicts.md) |
| A component's explicit property overrides the theme across states. | A red Restart button stays red while pressed. The theme can still supply its focus ring and padding. A local pressed background can override the red base. [Style precedence](../adr/0040-local-style-over-theme-states.md) |

The separate demo package is `snake-m5-oxi`. It shares the simulation and board painter with Snake. The acceptance test compares its precompiled settings view with its Rust Builder equivalent.

## A player journey

1. Start a game with the keyboard. The board gains focus and an arrow key turns the snake. Open Settings while playing. Snake pauses the simulation and moves focus to the first setting.
2. On **Appearance**, use Left and Right to choose **System**, **Light** or **Dark**. The window changes appearance and the board keeps its direction and score. Tab through the last setting and continue to other controls. Moving focus alone leaves the game paused.
3. Choose **Rebind** for a game Action and press a new shortcut. The recorder consumes that stroke. A conflict identifies the other Action and offers another attempt. Escape cancels recording. Tab cancels recording and continues traversal.
4. Close Settings. If focus was inside, return it to the opener when eligible. If the player had already moved elsewhere, keep that focus. An explicit Resume returns input to the board and restarts its clock.
5. Quit and launch with the same preferences. The shortcut label and the actual Action agree. Run both settings views with the same seed, input sequence and isolated player profile. [M5 replay](../../examples/snake-m5-oxi/src/replay.rs) · [Source parity test](../../examples/snake-m5-oxi/tests/settings.rs)

## Framework boundaries

```mermaid
flowchart LR
    platform["macOS or headless platform"] -->|owned input and preferences| core["Window thread / UiTree"]
    core -->|focused context| keymap["Keymap and Action handlers"]
    keymap -->|game Action| snake["Snake model and settings"]
    snake -->|validated binding overrides| keymap
    source["Development theme file"] -->|validated theme| style["Typed theme and state variants"]
    style -->|resolved properties| core
    core -->|layout and paint| scene["Scene"]
```

| Owner | Starting point and M5 responsibility |
| --- | --- |
| Platform port, macOS and headless adapters | The key event carries a physical position, a logical key, captured modifiers, press/release, repeat and capture time. The endpoint supplies an initial preference snapshot and ordered changes. Keep native objects on the main thread. [Platform events](../../crates/oxiview-platform/src/interface.rs) · [macOS input](../../crates/oxiview-platform-macos/src/macos.rs) · [Headless driver](../../crates/oxiview-platform-headless/src/lib.rs) |
| `oxiview-core` | The retained tree already owns generational node identities, ordered children, pointer routing, semantic Action names and scoped updates. Put focus, context lookup, Action delivery and state Signals here. Use the existing dispatch boundary for handler-requested changes. [Tree](../../crates/oxiview-core/src/ui/mod.rs) · [Dispatch](../../crates/oxiview-core/src/ui/input.rs) · [Builder](../../crates/oxiview-core/src/ui/view.rs) |
| `oxiview-style`, core and the composition root | Style owns typed tokens, rules and validation. Core binds them to mounted nodes and passes effective changes to layout, paint and text. The composition root wires the optional development file source and window wakeup. [Style](../../crates/oxiview-style/src/lib.rs) · [Text invalidation](../../crates/oxiview-core/src/ui/text.rs) · [Dependency rules](../../xtask/layers.toml) |
| Snake and Snake in `.oxi` | The M5 fork opts into framework Action routing and focus-aware buttons. The earlier Snake demos retain their physical-key controls. Both M5 settings sources share game Actions, user settings and replay. Shortcut labels use the effective Keymap. [Current keyboard switch](../../examples/snake/src/application.rs) · [Pointer controls](../../examples/snake/src/presentation.rs) · [`.oxi` control wiring](../../examples/snake-oxi/src/controls.rs) |

Text editing and IME belong to M6, reusable widgets to M7, animated state transitions to M8, native menus to M9 and the accessibility bridge to M12. The M5 contracts give those consumers enumerable Actions, effective preferences and a focus identity. [Input ADR](../adr/0011-ime-and-action-keymap.md) · [Accessibility ADR](../adr/0014-accessibility-deferred.md)

### S1 event contract

| Boundary | Data and ordering |
| --- | --- |
| Key transition | Extend the existing `WindowEvent::Key` path. Each transition carries the physical position, a logical named key or layout-produced character when available, captured Shift/Control/Alt/Meta modifiers, down/up, repeat and the original capture timestamp. Logical identity can be unidentified. A character here identifies a shortcut key. The editing core will have a separate text-commit path. |
| Preference initialization | Expose supported fields and their initial values on the window endpoint at creation, in time for the first resize-triggered draw. Subscribe and sample on the platform side so changes during initialization reach the window in order. |
| Preference update | Send an owned snapshot on change. The window thread updates the affected Signals together. Inject the same snapshots from the headless driver, including unavailable values, duplicates and a change during startup. |

## Focus and interaction states

| Situation | Behavior |
| --- | --- |
| Traversal | Nodes opt into programmatic focus or into both programmatic focus and Tab traversal. Use preorder over the retained child lists, including keyed reorders. With no focus, Tab starts at the first eligible stop and Shift+Tab at the last. Wrap at the ends of the window. Paint order and `z_index` have no bearing on traversal. |
| Pointer and application requests | A primary press can focus the nearest eligible control in the fixed input route. A programmatic request identifies a live node in this tree. Queue handler-requested focus changes and check eligibility at application time. Deliver blur then focus in one transition, with callback mutations deferred. |
| Eligibility and lifetime | A hidden or disabled subtree is ineligible. Pointer hit participation is independent of Tab eligibility. Reparenting an eligible focused node retains its identity and changes its context path. Removing, hiding or disabling it cancels armed activation and applies the accepted recovery order before the next input. An eligible explicit restoration target takes priority. [Focus recovery](../adr/0041-focus-recovery-in-tab-order.md) |
| Window lifecycle | On deactivation or hiding, remember the logical focus, suppress its active focus indication and cancel pending shortcuts and armed presses. On return, restore the indication only for a still-eligible node. Closing the window disposes all registrations with their scopes. |
| Visual states | Expose hover, pressed, focused and disabled to bindings. Hover follows the effective hover path, including capture. A primary press arms a control and release inside activates it once. For a focused button, use Space down to arm and Space up to activate, or Enter down to activate once. Interruption clears the press. Components can supply Signals for appearance while core owns focus and Action eligibility. |

Preserve the fixed pointer route, normal final-button release and FIFO mutation reports from M2. A focus request or state change during pointer delivery must leave that event's recipients intact. Test focus cleanup while the affected node is still live and rejection of a stale identity after slot reuse. [Input ordering](../adr/0034-pointer-input-processing-order.md) · [Hover and capture cleanup](../adr/0033-hover-lifecycle-cleanup.md) · [Node identity](../adr/0032-owned-generational-ui-node-identity.md)

## Actions, Keymaps and player overrides

- Give each Action a stable application-qualified identifier, a description and a handler on an eligible target. Derive the supported Action names in node semantics from the same registration. Keyboard, pointer activation and programmatic invocation call the same game operation. A handled input fires at most once. [Action and Keymap contract](../adr/0011-ime-and-action-keymap.md)
- Resolve from the focused node's innermost context through its ancestors to window defaults. Apply a player's override before a default in the same context. For example, Right on Appearance selects the next palette, Right on the board turns the snake, and the window can offer Open Settings in both contexts. A claimed but unavailable Action cannot accidentally activate an outer binding.
- Use layout-aware logical keys for ordinary shortcuts and explicit physical bindings for position-based input such as WASD. Preserve modifiers, repeat, transition and capture time in headless replay as well as native delivery. Keep the existing physical-key injection convenience for M1–M4 fixtures. The default shortcut modifier is Command on macOS and Control in the Windows/Linux profiles. Test a changed keyboard layout, an unidentified key and a repeated press. M6 owns text entry and dead-key composition.
- Apply the accepted same-context prefix rule to defaults and player overrides. An incomplete sequence expires after one second on the backend clock. Check it before the next stroke and clear it on focus/context loss or Keymap replacement. A mismatch resolves the latest stroke once as fresh input. A prefix alone has no pending Action or visible result, so expiry needs no frame request. [Prefix conflicts and examples](../adr/0039-keymap-prefix-conflicts.md) · [On-demand scheduler](../../crates/oxiview-core/src/lib.rs)
- For macOS, start with AppKit's normal shortcut dispatch. Apple documents that an unclaimed key equivalent reaches the first responder as a key-down event. The macOS adapter already makes its content view first responder. This route lets native shortcuts run first, then forwards the remaining input once to the window thread, where the current focus and Keymap are available. Test Command shortcuts after a focus change, native shortcut handling, and Tab/backtab with full keyboard access enabled. [Apple event dispatch](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/HandlingKeyEvents/HandlingKeyEvents.html) · [Keyboard values and modifiers](https://developer.apple.com/documentation/appkit/nsevent/charactersignoringmodifiers) · [Threading ADR](../adr/0003-window-threading-model.md)
- Snake persists versioned overrides by Action identifier in its per-user settings. Store the key identity and modifiers independently of the displayed label. Validate before activation and use atomic replacement for saves. A failed write keeps the active override visibly unsaved and retryable. A missing file starts with defaults. Malformed or unrecognized data produces a diagnostic and preserves the original file on load. Replays use an isolated settings path and start a fresh application instance to test persistence.

## State variants, tokens and hot reload

- Resolve each declared property in ascending priority, starting with framework defaults, the widget-type theme rule and its active state rules, followed by local base declarations and local state declarations. Within either state layer, use hover, focused, pressed, then disabled for simultaneous variants. Model the independent focus indicator as a paint property. Rules match one explicit widget type with an optional state. [Styling ADR](../adr/0008-typed-style-with-css-vocabulary.md) · [Local property precedence and examples](../adr/0040-local-style-over-theme-states.md)
- Existing `style(Style)` and `bind_style` supply complete local values, including fields equal to their defaults. Add a sparse typed declaration for themed nodes so a component can set only padding and draw the background from its theme. This also lets authors explicitly set a property back to its default value. Builders, `view!` and precompiled `.oxi` use the same operations. [Builder contract](../../crates/oxiview-core/src/ui/view.rs) · [Source equivalence](../adr/0037-optional-precompiled-oxi.md)
- Cover colors, spacing, corner radii and font sizes with typed design tokens. A changed color repaints consumers. A change to padding or text size remeasures affected nodes. Check layout counters, node identities and screen mounts. Reapplying the same effective values produces zero additional layout work or frames. [M2 invalidation tests](../../crates/oxiview-core/src/ui/layout_tests.rs) · [UI diagnostics](../../crates/oxiview-core/src/ui/diagnostics.rs)
- The file format is version 1 TOML. The workspace already declares its parser. Load and validate a complete candidate off the window thread, including token types, references and rule properties. Apply the result in one window-thread batch only if it belongs to the latest file change. An invalid latest edit keeps the last installed theme and reports the file and offending key. Test a real save-by-rename, a corrected edit and an older read completing last. `.oxi` remains a build-time source. [Workspace dependencies](../../Cargo.toml) · [Hot reload ADR](../adr/0016-devtools-in-framework.md)

For the authoring review, let `restart_fill` declare just the restart button's background and let `button` identify its theme rule. These equivalent spellings use the same two Builder operations.

```rust
node().theme_type(button).style_patch(restart_fill)
```

```text
<Node theme_type={button} style_patch={restart_fill} />
```

In the M5 demo, `--theme <path>` watches a development file. With keyboard focus on a setting, edit a color, introduce an invalid token reference and then correct it. The same mounted control should show both valid edits and keep its focus throughout. [M5 command line](../../examples/snake-m5-oxi/src/cli.rs)

## System preferences

The main-thread context and the window endpoint expose the supported fields through `Capabilities::system_preferences`. The owned initial snapshot supplies the window's available values before its first themed frame. Changes arrive on the endpoint and update window-thread Signals. In **System** mode an appearance change selects the palette. An explicit Light or Dark choice keeps its palette while continuing to receive other preferences, including accent and contrast. Equal effective values keep the window idle. [Platform capabilities](../adr/0002-own-platform-backends.md) · [Native integration ADR](../adr/0017-native-system-integration.md)

| Backend | M5 support |
| --- | --- |
| macOS | Observe effective appearance and accent color, convert the accent into the framework's color space, and read increased contrast and reduced motion on display-option changes. The native adapter reports system text scale as unavailable. [Appearance callback](https://developer.apple.com/documentation/appkit/nsview/viewdidchangeeffectiveappearance%28%29) · [Accent color](https://developer.apple.com/documentation/appkit/nscolor/controlaccentcolor) · [Display-option changes](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayoptionsdidchangenotification) |
| Headless | Inject the initial snapshot, supported and unavailable fields, and changes to appearance, accent, contrast, reduced motion and text scale. Text scale uses logical font sizes independently of the display's pixel scale. |
| Windows and Linux | Native adapters are scheduled for M14 and M15. Exercise their portable shortcut defaults through headless profiles. Each future adapter reports the preferences it can supply. [Platform roadmap](../adr/0018-macos-first-headless-backend-hedge.md) |

The headless driver exercises dynamic logical text scaling. The macOS adapter has no verified source for system reading-size changes. The application's font-size token supplies its fallback. The system setting applies to compatible apps and features. [Apple's text-size settings](https://support.apple.com/en-mt/guide/mac-help/mchld786f2cd/mac)

## Implementation order and acceptance

| Step | Deliverable and evidence |
| --- | --- |
| S1. Keyboard and preference events | Round-trip key identity, modifiers, repeat and capture time through both adapters. Verify initial preference ordering, change delivery, duplicate events and unavailable fields. Check which native Command and Tab events reach the content view. |
| S2. Focus and state | Traverse a headless tree forward and backward, move a keyed subtree, and deactivate during a press. Remove the middle, last and only eligible Tab stop. Check blur/focus order, explicit restoration, final focus and zero activations from releasing an old press. |
| S3. Actions and rebinding | Exercise nested contexts, complete and partial sequences, conflict diagnostics, cancellation and an application-invoked Action. Rebind in Snake, restart with the same settings, and exercise the replacement and former shortcut. Test recovery from an unwritable settings path. |
| S4. Themes and preferences | Exercise combined states, two palettes and a theme reload while focus stays on a mounted control. Check focus indication, paint-only edits, geometry changes and recovery from a malformed file. Use CPU snapshot candidates at 1×, 1.5× and 2×. |
| S5. Snake acceptance and metrics | Replay the player journey with the Rust and `.oxi` settings panels. Compare focus assertions, game and shortcut outcomes, and rendered images, with one screen mount per run. Retain the M4 gameplay checks and zero additional presentations across 1,000 idle frame opportunities. Review keyboard-only play, settings and live system colors on native macOS. |

The window-free `cargo xtask bench --m5` uses fixed workloads for Tab traversal over 256 stops, Action resolution through eight nested contexts, and a color-token change read by 16 of 256 mounted controls. Collect the existing median of three runs and apply the workspace 10% comparison threshold once the owner approves a machine baseline. Ordinary local `cargo xtask verify` covers correctness. Native measurement windows, new pixel baselines and performance recordings follow the existing owner-review procedures. [Benchmark protocol](../conventions/testing-performance.md) · [Snapshot review](../conventions/testing-snapshots.md)

## Acceptance record

On 2026-11-20 the owner accepted the release build of `snake-m5-oxi` after exercising its inline settings, keyboard navigation, appearance selection and shortcut feedback in the native macOS window. The opt-in AppKit probe delivered W down/up, Command+K, Tab and Shift+Tab through the real content view. It read the initial appearance, sRGB accent, contrast and motion preferences, observed a view-local appearance change and restored the original appearance. [Native probe](../../crates/oxiview-platform-macos/examples/m5_input_probe.rs)

The release replay collected 30 points, mounted the screen once, and issued zero frame requests across 1,000 idle opportunities. A fresh instance restored Dark appearance and the new I shortcut. The old Up key stopped steering, and I turned the snake. The Builder and precompiled settings panels produced the same replay results and image bytes. [Replay](../../examples/snake-m5-oxi/src/replay.rs) · [Source parity](../../examples/snake-m5-oxi/tests/settings.rs)

The owner accepted the three focused-and-pressed CPU snapshots at 1×, 1.5× and 2×. Each stored PNG matched its reviewed candidate byte for byte. The snapshots show an independently painted focus ring around a local pressed background. Corrected and unchanged theme-file saves also have deterministic tests. [Snapshot specimen](../../crates/oxiview-app/tests/m5_themed_paint.rs) · [Theme file recovery](../../crates/oxiview-app/tests/m5_theme_file.rs)

A real headless window additionally verified initial and changed preferences, forward/backward Tab delivery, the rendered palette and zero further presentations over 1,000 virtual display opportunities. A separate failure-and-retry test made the final settings rename fail, demonstrated that the new binding worked in memory, and saved it with F5 once the destination was writable. [Window-loop acceptance](../../examples/snake-m5-oxi/src/window_tests.rs)

With the owner's approval, the scoped recording added three metrics to the Mac14,10 baseline. All 25 earlier entries retained their original values. The immediate comparison passed all three M5 workloads at the 10% threshold. The eight M2, four M3 and four M4 workloads also passed their established baselines. M2 initially caught extra work on ordinary nodes. Keeping the cached paint path short and avoiding allocations in structural edits brought clean preparation to 5.592 ns and arena edits to 687.2 ns. [Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json) · [Benchmark protocol](../conventions/testing-performance.md#m5-focus-actions-and-themes)

| M5 workload | Recorded | Immediate comparison |
| --- | ---: | ---: |
| Tab over 256 stops | 1.607 µs | 1.623 µs |
| Resolve an Action through eight contexts | 291.3 ns | 291.3 ns |
| Update a color used by 16 of 256 nodes | 6.368 µs | 6.214 µs |

Ordinary local `cargo xtask verify` passed all seven checks, including workspace tests, Clippy, refreshed dependency advisories and documentation. The bundled release executable also passed the headless replay with a separate seed and development theme file.
