# vrandr

A lightweight **Vim-style TUI for [`xrandr`](https://www.x.org/wiki/Projects/XRandR/)**,
built with [Ratatui](https://github.com/ratatui-org/ratatui) on top of
[crossterm](https://github.com/crossterm-rs/crossterm).

`vrandr` lets you quickly rearrange your displays: pick an output, stage
the changes you want, press `a` to apply them all in one shot, or `W` to
save the current layout as a named profile and `T` to restore it later.

```
┌────────────────────────────────────────────────────────────┐
│  Outputs                               Modes              │
│  > eDP-1 * 1920x1080 +                 1920x1080  60.00Hz  │
│    HDMI-1  1920x1080 (staged)           1920x1080  59.94Hz  │
│    DP-1    off                         1680x1050  59.95Hz  │
├────────────────────────────────────────────────────────────┤
│  Status: eDP-1 selected, 1 pending op, [?] help            │
│  Preview: xrandr --output HDMI-1 --mode 1920x1080 ...      │
└────────────────────────────────────────────────────────────┘
```

The `+` next to an output's resolution means pending changes have been
staged. The bottom pane always shows the exact `xrandr` command that will
run.

## Requirements

- Linux with X11
- `xrandr` on `$PATH`
- Rust 1.70+ (edition 2021)

## Install

### From source

```sh
cargo build --release
./target/release/vrandr
```

### Static, fully-portable binary

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
./target/x86_64-unknown-linux-musl/release/vrandr
```

The release profile in `Cargo.toml` enables `lto = true`, `opt-level = "z"`,
`strip = true`, and `panic = "abort"` so the resulting binary is small
(typically < 2 MB) and depends only on `libc`.

## Usage

Run `vrandr` in a terminal. The app is fully keyboard-driven; press `?`
inside the app for the full key map grouped by topic.

### Workflow

1. Navigate with `j` / `k` (or arrow keys); use `h` / `l` / `Tab` to
   switch focus between the **outputs** list and the **modes** list.
2. Stage a change for the selected output, e.g. press `Enter` to stage
   the highlighted mode, `R` to cycle rotation, `d` to turn the output
   off, `H` / `L` / `K` / `J` to place it relative to a neighbour, `s` /
   `g` / `b` to enter an exact scale / gamma / brightness, etc.
3. The bottom pane previews the exact `xrandr` command that will run.
4. Press `a` to apply **all** staged changes in a single `xrandr` call.
   Press `r` at any time to discard the staged area.
5. Press `W` to save the current state (with pending changes applied)
   as a named profile under `~/.config/vrandr/profiles/<name>.toml`.
   Press `T` to list, apply, or delete saved profiles.

### Profiles

```
$XDG_CONFIG_HOME/vrandr/profiles/<name>.toml
```

Falls back to `~/.config/vrandr/profiles/<name>.toml` if
`XDG_CONFIG_HOME` is unset. Profile names accept letters, digits, `_`,
`-`, and `.`. The file is plain TOML:

```toml
version = 1

[[outputs]]
name = "eDP-1"
mode = "1920x1080"
rate = 60.0
primary = true
rotation = "normal"
reflection = "normal"
scale = [1.0, 1.0]
position = [0, 0]
off = false

[[outputs]]
name = "HDMI-1"
mode = "1920x1080"
rate = 59.94
relative = { to = "eDP-1", pos = "right-of" }
off = false
```

When applying a profile, outputs that are currently `disconnected` are
silently skipped, so you will not get `xrandr` errors for empty ports.

### License

MIT.
