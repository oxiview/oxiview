---
status: accepted
---

# Native integration: framework-provided title bars/materials, preferences, lifecycle, and distribution

The framework exposes these integrations as **capabilities** through ADR-0002's `capabilities()`, avoiding per-platform work in applications:

- **Custom title bars and window materials**: report non-client hit testing for custom title bars, retaining Windows 11 Snap Layouts, macOS traffic-light buttons, full-screen animations, and similar behavior. Support Mica / Acrylic / vibrancy materials; rendering surfaces must support transparency.
- **System preference signals**: dark mode, accent colors, high contrast, reduced motion, text scaling, and others feed the theme and animation systems as signals.
- **Application lifecycle**: file associations, URL schemes, single-instance behavior, system notifications, and launch at login.
- **Packaging, signing, and automatic updates**: official tools generate .app / MSI·MSIX / AppImage·Flatpak packages, handle code signing and macOS notarization, and provide update channels.

## Consequences

- These capabilities expand maintenance scope. Each needs tests and documentation for all three platforms, with Wayland/X11 distinctions on Linux. Unsupported capabilities must be queryable instead of failing silently.
- Packaging, signing, and update tools ship as independent CLIs, outside runtime dependencies.
