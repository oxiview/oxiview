---
status: accepted
---

# Build: Cargo workspace and xtask, pinned stable toolchain, edition 2024

Use Cargo alone for the build system, without Bazel, Buck2, just, make, or other tools. Root `Cargo.toml` is a virtual manifest declaring the workspace, with `[workspace.package]`, `[workspace.dependencies]`, and `[workspace.lints]` centralizing edition, versions, dependency versions, and lints for crates to inherit. Implement task automation in the Rust `xtask` crate (ADR-0019).

Pin a specific stable version in `rust-toolchain.toml` (initially 1.97), use edition 2024, and set MSRV to that same version. Nightly is limited to tooling tasks such as miri and cargo-fuzz; it does not enter build artifacts, which use no unstable features.

## Considered Options

- **Cargo + just**: concise task definitions, but adds an external dependency and still needs scripts for complex logic.
- **Bazel / Buck2**: strongest reproducibility and caching, but expensive integration with rust-analyzer and the Cargo ecosystem.
