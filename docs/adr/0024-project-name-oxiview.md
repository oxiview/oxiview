---
status: accepted
---

# Project name: oxiview

The framework is officially named **oxiview**: oxide (rust, alluding to Rust) + view. The public facade crate is `oxiview`; internal crates use `oxiview-<subsystem>` (ADR-0021). Rust code refers to crates as `oxiview_*`; types need not carry the project name as a prefix.

Availability checked when naming the project (2026-11-12):

- crates.io: `oxiview` and all 16 planned subcrates (`-base`, `-reactive`, `-scene`, `-render-cpu`, `-render-wgpu`, `-text`, `-layout`, `-platform`, `-platform-macos`, `-platform-headless`, `-core`, `-style`, `-widgets`, `-app`, `-macros`, `-testing`) were available, as were similar names `oxi-view`, `oxi_view`, `oxiviews`, and `oxyview`.
- GitHub: organization name `oxiview` was available, with no repository of the same name.
- Domains: `oxiview.dev`, `oxiview.rs`, and `oxiview.io` were unregistered; `oxiview.com` was registered.

## Considered Options

- **`rui`**: an existing Rust UI library in the same domain on crates.io (audulus/rui).
- **`rsui`**: an existing cross-platform Rust UI library on crates.io; GitHub organization already taken.
- **`rstyle`**: available everywhere, but easily mistaken for a styling library or code formatter and redundant with the `-style` subsystem.
- **`newui`**: available everywhere, but too generic, overshadowed in search by JetBrains New UI, and likely to age poorly.
- **`viewloom`, `heddleui`**, and others: available; oxiview was chosen for brevity and clearer Rust imagery.

## Consequences

- The `oxiview` 0.0.0 placeholder was published on crates.io on 2026-11-12 (owner dangaogit, MIT OR Apache-2.0). The owner handles registration of the GitHub organization `oxiview` and domain `oxiview.dev`. Subcrate names are not reserved separately; register them at first publication.
- The repository directory was renamed from `rust-ui` to `oxiview`.
