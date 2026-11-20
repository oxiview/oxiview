# Headless platform backend

Unstable platform-context adapter for M1 S2. Created with
`cargo xtask new-crate oxiview-platform-headless --context platform`.
It implements the platform port without a native surface or a renderer.

## Driver

Construct `HeadlessPlatform::new(refresh_interval)` on the thread that will call
`Platform::run`. Move endpoints to window threads in the initialization callback.
Clone the returned `Driver` onto controlling threads as needed.
`HeadlessPlatform::new_with_preferences(refresh_interval, initial_preferences)`
configures the snapshot returned by each new window endpoint.

- `now`: read virtual time with explicit poison reporting.
- `advance_to`: move to an absolute virtual instant, never backwards.
- `advance_by`: advance by a duration, rejecting overflow.
- `pointer_moved`: inject logical position and an optional capture timestamp.
- `pointer_button`: inject button, press/release, position, and capture timestamp.
- `pointer_left` reports an ordinary pointer exit. `pointer_interrupted` reports loss of pointer continuity.
- `activation_changed` and `visibility_changed` inject window activation and visibility transitions with their capture timestamps.
- `key` injects a key with its unshifted US logical identity and empty modifiers.
- `key_transition` injects an explicit physical position, logical identity,
  modifiers, press/release, repeat, and capture timestamp.
- `preferences_changed` queues an owned preference snapshot, including duplicates
  and unavailable fields. Changes queued during initialization precede the first resize.
- `resize`: change physical size and scale, returning a fresh resize token.
- `resize_outcome`: inspect a pending or completed resize transaction.
- `wait_resize`: block the caller until acknowledgement, supersession, or timeout.
- `request_redraw`: enqueue system repaint intent without requesting a frame.
- `request_user_close`: enqueue close intent without destroying the window.
- `counters`: read per-window frame deliveries, resize acknowledgements, and timeouts.
- `wait_until_blocked`: handshake with a blocked event or resize wait.

An omitted input timestamp means the current virtual time. Explicit capture
timestamps are preserved, even when replay injection order differs from capture
order. Events follow injection order. Driver operations serialize on one mutex;
tests needing a specific order between producers must coordinate those producers.

Initial resize publication occurs after initialization returns. Driver resize
requests are rejected before that publication. A scale change queues
ScaleFactorChanged immediately before Resized. Every resize supersedes any
pending predecessor. Destroying a window also supersedes a pending resize.
Completed outcomes and counters remain queryable until the driver is dropped.

## Virtual time and blocking

The shared state wraps oxiview-base's ManualClock in a mutex. Every deadline,
resize transaction, frame request, and event queue uses this same state.
Condition-variable waits release the mutex, allowing acknowledgement, frame
requests, input, wake, destruction, and clock advancement from other threads.
There is no sleep, system clock, entropy, async runtime, or application executor.
ManualScheduler and randomness services are not needed by this adapter.

Vsync boundaries are positive multiples of the configured nonzero interval.
A request arms the first boundary strictly after the current virtual time.
Crossing it enqueues exactly one Frame and disarms the request. Its predicted
presentation timestamp is the boundary itself: presentation latency is zero.
Additional elapsed boundaries do not generate frames without another request.
Counters count enqueued Frame events. Destruction may discard queued events.

Every Resized event, including the initial resize after initialization, carries
the first virtual vsync boundary strictly after its publication time as its
predicted presentation timestamp, plus the configured refresh interval.
The immediate resize presentation does not arm or consume a Frame event.

Resize deadlines are measured from publication using the window's resize_timeout.
Advancement expires pending tokens at or beyond the deadline, counts each timeout
once, and traces its window, token, and timeout. A blocked wait never advances
time itself. Event waits prefer queued events to elapsed deadlines, and terminal
destruction takes precedence over a timeout. Application wakes coalesce until
consumed. A presenter's readiness notification queues a separate
`PresentationReady` event and wakes the window thread. This allows tests to
deliver input while a presenter is waiting for a drawable.

A controlling thread can call resize followed by wait_resize to simulate the
blocking main-thread portion of live resize. Another thread must acknowledge or
advance time; calling both the blocking wait and its release sequentially on one
thread cannot make progress. wait_until_blocked provides a deterministic
handshake, with no wall-clock timeout.

## S1 interface observations

- Platform::run requires the process main thread, but safe portable std cannot
  identify it. This backend enforces creation/run thread affinity and treats
  that thread as the simulated main thread. Recommend defining this requirement
  per backend: process main thread for native backends, designated event-loop
  thread for headless. No S1 interface was changed.
- WindowEndpoint::now cannot report poisoned synchronization state because its
  return type is Duration. It reads the last clock value through the poisoned
  guard; fallible operations report DriverError::Poisoned (mapped to
  OperationFailed at the port). Recommend a fallible clock accessor if the port
  requires uniform poison reporting.
- Platform::run exposes only an initialization callback, so test code cannot
  execute a post-initialization resize wait on its calling thread through the
  port. The separate driver exposes wait_resize to a controlling thread that
  models the main-thread resize wait. Recommend retaining a backend-specific
  driver hook rather than adding test orchestration to the platform port.

There are no extra dependencies beyond existing workspace oxiview-base,
oxiview-platform, tracing, and thiserror. The adapter inherits the workspace
unsafe prohibition. The [Snake window-loop tests](../../examples/snake/tests/window_loop.rs)
use the driver to verify timed steps, pause/resume and zero idle frames.
