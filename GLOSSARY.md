# oxiview

oxiview is a self-drawn Rust UI framework exclusively for native desktop platforms. The framework handles every widget from layout to pixels.

## Language

**Self-drawn**:
The framework requests only a window surface from the operating system. It handles all widget layout and painting without using native system widgets.
_Avoid_: native widgets, self-rendering

**Native platform**:
The macOS, Windows, and Linux desktop operating systems, accessed directly through their system APIs without any Web technology.
_Avoid_: cross-platform (without qualification, it may imply Web or mobile support)

**House style**:
The framework's default visual style, consistent across all three platforms and replaceable or customizable through themes.
_Avoid_: native look and feel, default skin

## Platforms and threads

**Main thread**:
The system thread created at process startup. It only receives system events and calls system APIs that require it; it does not run UI logic.
_Avoid_: UI thread

**Window thread**:
A thread running state, layout, painting, and presentation for one or more windows. The default is one thread per window.
_Avoid_: render thread, UI thread

**Window endpoint**:
The transferable, per-window receiver and command interface used exclusively on its assigned window thread.

**Resize token**:
An identity correlating a window resize transaction with acknowledgement of presentation at its new physical size.

**Shared-thread mode**:
A configuration in which multiple windows run on the same window thread and can share state directly.

**Capability**:
A system service exposed by the platform layer, such as menus, the system tray, or the clipboard. Applications can query its availability at runtime.
_Avoid_: plugin, feature

## Frames and reactivity

**Frame policy**:
The rule determining when a window produces a new frame: on-demand, continuous at window level, or continuous at layer level.
_Avoid_: rendering mode, refresh mode

**On-demand rendering**:
Producing frames only for invalidated state, animation, or system requests, with no frame loop while idle.
_Avoid_: lazy rendering, passive rendering

**Predicted presentation timestamp**:
The backend clock time at which a requested frame is expected to become visible on the display.

**Input capture timestamp**:
The backend clock time carried by an input event. A frame's latency record keeps
the oldest unpresented input it consumes, including when several events coalesce.

**Submission timestamp**:
The endpoint clock time immediately after the presenter reports a submitted
frame. It is not GPU completion or confirmation that the frame reached the display.

**Actual presentation timestamp**:
Native presentation feedback confirming when a frame became visible. On macOS,
this is a nonzero Metal drawable `presentedTime`; zero means it was not displayed.

**Submitted FPS**:
Successful frame submissions per second, counted by submission timestamp within the sample interval.

**Displayed FPS**:
Frames per second confirmed visible by an actual presentation timestamp within the sample interval.

**Signal**:
A reactive value whose dependencies can be tracked. Changes directly notify dependent computations and node properties.
_Avoid_: State, Observable, Atom

**Scope**:
The owner of signals, effects, and local tasks. Destroying it releases the resources it owns.
_Avoid_: Owner (for the owning scope), Context

**Scope handle**:
A non-owning reference to a reactive scope, represented by the `Owner` API.

**Structural reconciliation**:
Updating mounted children when a conditional branch or keyed list changes. Surviving list keys retain their node identities.
_Avoid_: diff, re-render

## Components and nodes

**Component**:
A function that receives props and returns a view, executed only once at mount time.
_Avoid_: Widget (reserved for low-level drawable node types), View function

**Props**:
The input parameter struct of a component.

**View**:
A construction description consumed when mounting UI nodes and their bindings.

**Node**:
An element in the retained tree with style, layout results, and painting content, identified by `NodeId`.
_Avoid_: Element, RenderObject

**Node identity**:
The opaque framework-owned `NodeId` value identifying a UI node within its owning tree.

**Reserved node identity**:
A `NodeId` assigned to a queued creation whose node does not yet exist.

**Root node**:
The unique top-level node of a window's retained UI tree.

**Node mounting**:
The attachment of a UI node to its parent in the retained tree.

**Reparenting**:
Changing the parent of an existing node together with its subtree.

**Layout boundary**:
A node whose size does not depend on its content, preventing layout changes in its subtree from propagating outward.
_Avoid_: relayout boundary

**Repaint boundary**:
A node with an independent compositing layer, so repainting its subtree does not affect content outside the layer.
_Avoid_: layer (without qualification)

## Styling and layout

**Design token**:
A named basic visual value in a theme, such as a color, spacing, corner radius, or font size.
_Avoid_: variable, theme color

**Theme**:
A set of tokens and theme rules that determines the default appearance of widgets.
_Avoid_: skin, stylesheet

**Theme rule**:
An entry setting default styles by widget type or widget type plus state. It does not support arbitrary selectors or cascading.
_Avoid_: selector, CSS rule

**State variant**:
An additional style override for a node's interaction state, such as hover, pressed, focused, or disabled.
_Avoid_: pseudo-class

**Layout protocol**:
The framework-defined interface for node measurement and positioning, implemented by specific layout algorithms.
_Avoid_: layout engine (reserved for a specific implementation)

## Text and input

**Glyph atlas**:
A GPU texture cache of rasterized glyphs at small font sizes.
_Avoid_: font texture, glyph cache

**Editing core**:
The framework's editable text model, responsible for cursors, selections, IME composition, and undo. All input widgets build on it.
_Avoid_: text box, TextInput (these are widgets)

**Preedit**:
Uncommitted IME composition text displayed inline at the cursor.
_Avoid_: composition, candidate text

**Pointer capture**:
The temporary assignment of a pointer's move and button events and effective hover target to one node, independently of geometric hit testing.
_Avoid_: capture phase, pointer lock

**Event dispatch**:
Delivery of one event to its target and any other recipients selected by its propagation rules.

**Mutation command**:
A queued request to change framework-owned node structure, styles, or pointer capture.

**Command batch report**:
An ordered record of the outcomes of mutation commands in one completed batch.

**Hover path**:
The structural ancestor chain from the window root to the node currently treated as hovered by a pointer.

**Hover notification batch**:
The ordered enter/leave notifications associated with one transition between hover paths.

**Action**:
A named user command independent of keys, triggered by shortcuts, menus, or code.
_Avoid_: Command (to avoid confusion with the platform Cmd key), shortcut

**Keymap**:
A configuration mapping key sequences to Actions, layered by focus context.
_Avoid_: shortcut table, keybinding

**Focus context**:
A region of a window whose Keymap bindings apply when keyboard focus is on its node or a descendant.

## Rendering and compositing

**Scene**:
The set of painting instructions for one frame, serving as the sole interface between the node tree and rendering backends.
_Avoid_: Display List, rendering commands

**SDF fast path**:
A dedicated rendering path that analytically draws common UI primitives, such as rounded rectangles, borders, and shadows, in shaders.

**Stacking context**:
A group of node content with internal painting order, treated as one atomic unit in its containing stacking context.
_Avoid_: compositing layer, repaint boundary

**Compositing layer**:
A painting unit that can update independently and then compose with other layers. External textures also enter as layers.
_Avoid_: Layer (without qualification), surface

**Native surface handle**:
An owned framework description of a native presentation object passed from a platform adapter to a rendering adapter. It owns the representation, not the native object's storage.

**Native child view**:
A native system view embedded in a framework window, always above self-drawn content.
_Avoid_: native widget, overlay

## Engineering

**Headless platform backend**:
A platform implementation that simulates windows and input without connecting to a real window system. It is the platform interface's second implementation, used for testing and event replay.
_Avoid_: mock platform, fake backend

**Milestone**:
A roadmap stage delivering a runnable demo, tests, and metrics, numbered M-1 through M15.
_Avoid_: phase, version

**Composition root**:
The only crate (`oxiview-app`) that can access all ports and concrete adapters and is responsible for wiring them together.
_Avoid_: runtime, main crate
