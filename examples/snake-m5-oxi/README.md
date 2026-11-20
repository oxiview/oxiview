# Snake M5 in .oxi

[English](README.md)

This fork of [Snake in .oxi](../snake-oxi/README.md) adds keyboard focus, configurable Actions and a live theme to the playable game. Both examples use the same simulation, clock and retained board painter. The M5 fork owns the settings section, the game's Keymaps and the player's configuration.

```sh
cargo run --release -p snake-m5-oxi
```

On macOS 14 or later, Enter starts the game, arrows or the physical WASD positions steer, and Space pauses or resumes while the board has focus. Open **Settings** with the button or Command+comma. Opening settings during play pauses the simulation. Use Tab and Shift+Tab to move anywhere in the window. On Appearance, Left and Right select System, Light or Dark. Done or Escape closes the section. Resume explicitly to return to play.

On **Game action**, click or use Left and Right to choose which Action to rebind. Activate its shortcut control and press the new combination. Escape cancels and Tab returns to ordinary traversal. A conflict identifies the Action already using the stroke. The board and shortcut label update together. A failed save is shown as unsaved and F5 retries while settings are open.

The board also accepts Command+K followed by P to pause. Its incomplete first stroke expires after one second. A rebind that would claim Command+K in that context reports the conflict with Pause.

Native player settings default to `~/Library/Application Support/oxiview/snake-m5-oxi/settings.json`. Use `--settings PATH` to select a separate profile. The included theme is compiled into the executable. Use a development theme file to try live edits in the open window.

```sh
cargo run --release -p snake-m5-oxi -- --theme examples/snake-m5-oxi/theme.toml
```

Edit a color in the chosen palette, then try an invalid token reference. The last valid theme stays visible and the settings area reports the error. The next valid save is applied without remounting the game. The theme format supports typed colors, spacing, radii and font sizes, widget rules and state variants. [Bundled theme](theme.toml) · [Style precedence](../../docs/adr/0040-local-style-over-theme-states.md)

## Acceptance

```sh
cargo run --release -p snake-m5-oxi -- --headless --json
cargo test --locked -p snake-m5-oxi
cargo xtask bench --m5
```

The default output is `target/xtask/snake-m5-oxi/`. The replay writes six real game images and a pass/fail manifest, uses its own temporary player profile, and starts a fresh application instance to verify the saved key. The source-parity test compares the precompiled settings panel with its Builder equivalent, including all rendered image bytes. Run the shared bundle script to produce `target/xtask/snake-m5-oxi/Oxiview Snake M5 Oxi.app`. [Bundle script](package-macos.sh) · [M5 milestone](../../docs/plans/m5-focus-keyboard-and-themes.md)

The [screen](src/screen.oxi), [board](src/board.oxi) and [sidebar](src/sidebar.oxi) compose the copied game. The [inline settings fragment](src/settings/panel.oxi) uses the same Builder operations as its [Rust equivalent](src/settings/view.rs). The [application controller](src/app.rs) connects window input to the [game Keymaps](src/shortcuts.rs) and [player settings](src/settings/storage.rs). The [replay](src/replay.rs) drives those same entry points.
