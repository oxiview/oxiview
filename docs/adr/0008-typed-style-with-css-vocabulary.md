---
status: accepted
---

# Styling: typed properties, CSS vocabulary, and restricted theme rules without implementing CSS

Styles are **typed properties** on nodes, driven by signals and checked at compile time. Property **names and semantics align with CSS** (box model, `padding`, `border_radius`, `box_shadow`, gradients, Flex/Grid properties, transitions), giving designers and frontend developers familiar vocabulary. The framework **does not implement CSS**: no arbitrary selectors, no cascading, and no CSS text parser. This follows ADR-0001's exclusion of Web technologies.

Interaction states use **built-in state variants** by default. The framework maintains hover / pressed / focused / disabled state signals for each node. Styles look like `.bg(t.primary).hover(|s| s.bg(t.primary_hover))`; state transitions can animate. Components may override built-in states with their own signals.

A **theme** consists of design tokens (colors, spacing, corner radii, font sizes, shadows, and others) and **theme rules**. Rules have only two levels: widget type, and widget type plus state, providing widget defaults. Themes can load from external files in an owned format such as RON / TOML and hot reload during development. System dark-mode and accent-color changes arrive as signals.

## Why not implement CSS3

CSS selectors and cascading serve documents with separate structure and styling. In the execute-once, direct-signal-property model (ADR-0005), class or state changes would rematch subtrees, effectively restoring diffing and weakening precise invalidation. Full CSS semantics are extremely costly; the limits of a subset would face repeated challenges. GTK4 illustrates persistent selector-matching costs and difficulty tracing style origins. The valuable parts to borrow are **property vocabulary** and **external style hot reload**, both retained here.

## Considered Options

- **A CSS3 subset** (Blitz, Vizia, GTK4): reusable assets, but conflicts with fine-grained signals and ADR-0001.
- **A custom selector-based style language** (Qt QSS style): flexible, but requires a parser and LSP while retaining matching overhead.
- **Typed properties without CSS vocabulary**: simplest, but increases communication costs with designers.

## Consequences

- Theme rules may expand, but arbitrary selectors and cascading remain prohibited unless a new ADR supersedes this decision.
- Document style units and semantics, such as percentage border radii and shadow spread, against CSS specifications to avoid identical names with different meanings.
