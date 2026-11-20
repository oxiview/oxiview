# Editing fuzz targets

[English](README.md)

The two libFuzzer targets exercise the same public editor operations as the retained view. `editing` varies Unicode insertions, deletion, undo and selection offsets. `input_session` varies marks, replacements, cancellation, invalid ranges, stale callbacks and focus transfers. The stable workspace test runs the checked-in seeds and deterministic generated sequences through the same oracles. [Shared scenarios](scenarios.rs) · [Stable regression test](../examples/markdown-editor-oxi/tests/edit_corpus.rs)

With a compatible nightly and `cargo-fuzz` installed, run these from the repository root. Supply a writable corpus as the first directory so generated inputs stay in the build tree. Preserve minimized failures under `fuzz/seeds/` with their regression test.

```sh
mkdir -p target/xtask/m6-fuzz/editing/corpus target/xtask/m6-fuzz/input_session/corpus
cargo +nightly fuzz run editing target/xtask/m6-fuzz/editing/corpus fuzz/seeds/editing -- -max_total_time=60 -max_len=384
cargo +nightly fuzz run input_session target/xtask/m6-fuzz/input_session/corpus fuzz/seeds/input_session -- -max_total_time=60 -max_len=384
```

The isolated tooling workspace also builds the same libFuzzer driver directly with LLVM coverage on stable Rust. An explicit target keeps the flags on target code. For the Apple silicon acceptance machine, build and run as follows.

```sh
RUSTFLAGS='-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 -Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-pc-table -Cllvm-args=-sanitizer-coverage-trace-compares' \
  cargo build --locked --release --manifest-path fuzz/Cargo.toml --bins --target aarch64-apple-darwin --target-dir target/xtask/m6-fuzz/build
mkdir -p target/xtask/m6-fuzz/editing/corpus target/xtask/m6-fuzz/input_session/corpus
target/xtask/m6-fuzz/build/aarch64-apple-darwin/release/editing target/xtask/m6-fuzz/editing/corpus fuzz/seeds/editing -max_total_time=60 -max_len=384
target/xtask/m6-fuzz/build/aarch64-apple-darwin/release/input_session target/xtask/m6-fuzz/input_session/corpus fuzz/seeds/input_session -max_total_time=60 -max_len=384
```

Record executions, coverage, corpus growth, duration and crash artifacts. The direct build instruments coverage. A sanitizer campaign can use the nightly tooling path. [Toolchain policy](../docs/adr/0020-cargo-workspace-xtask-stable.md)
