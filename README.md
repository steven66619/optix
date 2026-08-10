# Optix

A GPU-accelerated terminal emulator written in Rust — kitty-grade power with
modern looks. Optix uses `wgpu` for rendering, `alacritty_terminal` for the
terminal core, and `cosmic-text` for high-quality font shaping.

## Features

- **GPU-accelerated rendering** — `wgpu` (Vulkan/Metal/DX12) with a fallback
  CPU render path.
- **Kitty graphics protocol** — inline images (e.g. `fastfetch`, `chafa`,
  `timg`) are transmitted and placed directly in the terminal.
- **Kitty keyboard protocol** — CSI `u` sequences for modern applications.
- **Split panes** — arbitrary nested splits, tab management, and directional
  focus navigation.
- **Runtime themes** — six built-in palettes (Catppuccin, Gruvbox, Dracula,
  Nord, Solarized, Tokyo Night) plus user themes.
- **Theme switching without leaving the prompt** — type `theme ayu` straight
  at the shell prompt (magic commands), or use `/theme ayu` in the command
  overlay, or `optix-msg theme ayu` from anywhere.
- **`optix-msg` IPC** — drive a running terminal over a Unix socket the way
  `i3-msg` drives i3.
- **Live configuration reload** — edit `config.toml` and the running terminal
  picks up font, theme, and window changes automatically.
- **Picom transparency** — optional ARGB window so a compositor can show the
  wallpaper through the terminal, plus per-window opacity.
- **Background image** — paint a PNG behind the terminal content.
- **Rounded corners & glow** — subtle window/corner styling.
- **Search overlay** — incremental search with next/previous navigation.
- **Scrollback** — scroll up/down, page, and top/bottom actions.
- **256-color and truecolor ANSI** support, bell flash, copy/paste, and
  font scaling.

## Requirements

- Linux (X11 or Wayland)
- Rust toolchain (edition 2021)
- A GPU driver supported by `wgpu`
- `fontconfig`-provided fonts (JetBrains Mono Nerd Font recommended)
- System libraries for `cosmic-text` / `fontconfig`

## Building

```sh
cargo build --release
```

This produces two binaries:

- `target/release/optix` — the terminal
- `target/release/optix-msg` — the IPC client

### Install

```sh
# install to /usr/local/bin (does not require root if PREFIX is writable)
make install

# or build and install system-wide in one step
make deploy
```

## Usage

```sh
optix
```

On first launch optix writes a commented example configuration to
`~/.config/optix/config.toml` if none exists.

### Configuration

Optix reads `~/.config/optix/config.toml` (or
`$XDG_CONFIG_HOME/optix/config.toml`). All fields are optional; defaults are
used for anything omitted. See [`config.toml.example`](./config.toml.example)
for a fully commented reference.

```toml
[font]
family = "JetBrainsMono NF"
size = 12.0

[window]
opacity = 0.8
transparent = true
corner_radius = 12.0

[theme]
background = "#1e1e2e"
foreground = "#cdd6f4"
```

The config file is watched and **reloaded live** while the terminal runs.

### Themes

Built-in themes:

```
catppuccin  gruvbox  dracula  nord  solarized  tokyonight
```

Switch with any of:

```sh
theme ayu                        # magic command: type at the shell prompt
/theme ayu                       # command overlay
optix-msg theme ayu              # from anywhere, via IPC
```

User themes: drop a `<name>.toml` file with the same `[theme]` layout into
`~/.config/optix/themes/` and it takes precedence over built-ins.

### Keybindings

| Shortcut                  | Action                     |
|---------------------------|----------------------------|
| `Ctrl+Shift+E`            | Split right                |
| `Ctrl+Shift+O`            | Split below                |
| `Ctrl+Shift+X`            | Close pane                 |
| `Ctrl+Shift+]` / `[`      | Next / previous pane       |
| `Ctrl+Alt+Arrow`          | Focus pane in direction    |
| `Ctrl+Shift+F`            | Search                     |
| `Ctrl+Shift+Enter` / `G`  | Search next                |
| `Ctrl+Shift+H`            | Search previous            |
| `Ctrl+Shift+P`            | Open `/command` overlay    |
| `Ctrl+Shift+Up/Down`      | Scroll lines               |
| `Ctrl+Shift+PageUp/Down`  | Scroll page                |
| `Ctrl+Shift+Home/End`     | Scroll to top / bottom     |
| `Ctrl+Shift+C` / `V`      | Copy / paste               |
| `Ctrl+Plus` / `Ctrl+Minus`| Increase / decrease font   |
| `Ctrl+0`                  | Reset font size            |
| `Ctrl+Shift+Q`            | Quit                       |

All keybindings are remappable in the `[keybindings]` section of
`config.toml`.

### `optix-msg` IPC

```sh
optix-msg theme ayu     # switch to a theme
optix-msg themes        # list available themes
optix-msg ping          # check that optix is running (replies "pong")
optix-msg quit          # quit the terminal
```

Exit codes: `0` success, `1` command failure (unknown command / no terminal
running), `2` usage error.

The socket lives at `$XDG_RUNTIME_DIR/optix/ipc.sock` (falling back to
`/tmp/optix-$UID/ipc.sock`).

### Magic commands

Lines typed at a shell prompt that optix handles itself instead of sending to
the shell:

```
theme            # list available themes
theme ayu        # switch to the "ayu" theme
```

This works even when the shell's line-start state is ambiguous (after a TUI
app exits or a line is interrupted). Disable it with `[magic] enabled = false`.

## Project layout

```
src/
  app.rs        window/event-loop application logic
  config.rs     configuration loading and the example config
  terminal.rs   per-pane terminal state (alacritty_terminal)
  render.rs     wgpu + CPU rendering
  pty_io.rs     PTY reader/writer, kitty graphics transport
  kitty.rs      kitty graphics protocol state machine
  input.rs      keyboard encoding (incl. kitty protocol)
  fonts.rs      cosmic-text font discovery and shaping
  themes.rs     built-in theme palettes
  layout.rs     tabs and split-pane layout tree
  ipc.rs        Unix-socket IPC server
  magic.rs      shell-level magic command parsing
  bin/optix-msg.rs  the IPC client binary
```

## License

MIT — see [LICENSE](./LICENSE).
