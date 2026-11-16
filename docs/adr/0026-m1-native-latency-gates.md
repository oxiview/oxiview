---
status: accepted
---

# M1 native latency gates: one-interval submission and measured presentation regression

Date: 2026-11-16

Partially supersedes [ADR-0001](0001-goals-and-non-goals.md)'s initial absolute
one-frame input-to-pixel gate. Verification remains local under
[ADR-0019](0019-local-only-verification.md).

## Context

M1 measures actual `MTLDrawable.presentedTime`. On the tested macOS compositor,
the optimized windowed path still takes multiple refresh intervals from input
capture to display, even when application submission takes less than one interval.
The owner accepted a floating threshold based on measured optimized behavior and
deferred fullscreen input-to-pixel baselines to M10. Reporting a display-link
prediction or submission time as actual presentation would conceal this gap.

## Decision

- Enforce `input_to_submit_ms` p95 at or below one measured refresh interval.
- Enforce `input_to_pixel_ms` p95 at or below the human-confirmed optimized
  baseline multiplied by 1.15. A missing baseline fails; it is never a pass.
- Record the display interval with latency baselines. Comparisons require the
  same interval within 0.1%; changing displays requires explicit recording after
  human confirmation.
- Use three sequential windowed runs, each with 120 synthetic AppKit pointer
  inputs covering twenty posting phases. Every run must pass the submission
  gate; independently aggregate each metric by the median of its three p95s.
- Keep one input outstanding until its consuming frame reports actual nonzero
  presentation. Injection bypasses HID and WindowServer input delivery; the
  measured clock starts before the in-process command is queued.
- Preserve the idle target: zero submissions, presentations and display-link
  frame events, with process CPU at or below 0.1% over five seconds.
- Keep fullscreen and continuous-input diagnostics separate. Stable presentation
  cadence does not prove one-frame input latency. Record overruns without
  substituting that workload for the phase-swept baseline workload.

## Alternatives and consequences

Keeping the original absolute pixel gate would leave M1 blocked on compositor
and presentation work assigned to M10. Replacing actual display feedback with a
prediction would produce a misleading pass. A compatible measured baseline
instead detects regressions while explicitly retaining the end-to-end latency
limitation; it does not establish that the original one-frame goal has been met.

Native checks need a visible display session and a stable environment. Baseline
recording remains a human decision, never an automatic response to a regression.
The benchmark protocol and diagnostic endpoints are specified in the
[performance conventions](../conventions/testing-performance.md).
