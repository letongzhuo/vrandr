//! Build and apply xrandr commands from staged `PendingChange`s.

use crate::model::{CustomMode, Output, PendingChange, PendingMap, RelativePosition};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::process::Command;

/// Format a single `PendingChange` as a flat list of `xrandr` arguments
/// (i.e. the tail of `xrandr --output <name> ...`).
///
/// Returns an empty Vec if the change is a no-op (which shouldn't happen if
/// the caller checks `has_visible_change` first).
pub fn change_to_args(change: &PendingChange) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(off) = change.off {
        // `off` is a tri-state: Some(true) -> --off, Some(false) -> --auto,
        // None -> leave the current power state alone. `--auto` and `--off`
        // are mutually exclusive; whichever the user staged most recently
        // wins (the event loop overwrites the field on every keypress).
        if off {
            args.push("--off".into());
            return args;
        }
        args.push("--auto".into());
    }
    if let Some(mode) = &change.mode {
        args.push("--mode".into());
        args.push(mode.name.clone());
        // xrandr also accepts an explicit refresh rate with --rate.
        if mode.refresh_rate > 0.0 {
            args.push("--rate".into());
            args.push(format!("{:.3}", mode.refresh_rate));
        }
    }
    if change.primary == Some(true) {
        args.push("--primary".into());
    }
    if let Some(rot) = change.rotation {
        args.push("--rotate".into());
        args.push(rot.as_str().into());
    }
    if let Some(refl) = change.reflection {
        // Always emit --reflect, including for `normal`. xrandr treats
        // `--reflect normal` as the canonical reset, and users rely on
        // the preview showing it explicitly so they can see that the
        // reflection has actually been cycled back.
        args.push("--reflect".into());
        args.push(refl.as_str().into());
    }
    if let Some((sx, sy)) = change.scale {
        args.push("--scale".into());
        args.push(format!("{sx}x{sy}"));
    }
    if let Some((sx, sy)) = change.scale_from {
        args.push("--scale-from".into());
        args.push(format!("{sx}x{sy}"));
    }
    if let Some((r, g, b)) = change.gamma {
        args.push("--gamma".into());
        args.push(format!("{r}:{g}:{b}"));
    }
    if let Some((x, y)) = change.position {
        args.push("--pos".into());
        args.push(format!("{x}x{y}"));
    }
    if let Some((w, h)) = change.panning {
        args.push("--panning".into());
        args.push(format!("{w}x{h}"));
    }
    if let Some(rel) = &change.relative_to {
        if let Some((to, pos)) = rel {
            args.push(format!("--{}", pos.as_str()));
            args.push(to.clone());
        }
    }
    args
}

/// Build the full xrandr argv for every output with staged changes.
///
/// Custom mode definitions (`--newmode`/`--addmode`) come first so the
/// modes exist before any `--mode <name>` reference.
pub fn build_command(pending: &PendingMap) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Collect custom modes first. We use a BTreeMap to keep the iteration
    // order deterministic (handy for tests and the preview line).
    let mut customs: BTreeMap<String, CustomMode> = BTreeMap::new();
    for change in pending.values() {
        for cm in &change.custom_modes {
            customs.insert(cm.name.clone(), cm.clone());
        }
    }
    for cm in customs.values() {
        args.push("--newmode".into());
        args.push(cm.name.clone());
        args.push(cm.modeline.clone());
    }

    // Now the per-output --output blocks.
    for (name, change) in pending {
        if !change.has_visible_change() && change.custom_modes.is_empty() {
            continue;
        }
        args.push("--output".into());
        args.push(name.clone());
        args.extend(change_to_args(change));
    }

    args
}

/// Build a single human-readable preview line like
/// `xrandr --output HDMI-1 --mode 1920x1080 --rate 60.000 ...`.
pub fn preview_command(pending: &PendingMap) -> String {
    let args = build_command(pending);
    if args.is_empty() {
        return "xrandr  (no pending changes)".to_string();
    }
    let mut s = String::from("xrandr");
    for a in &args {
        if a.starts_with("--") || s.is_empty() {
            s.push(' ');
        } else {
            s.push(' ');
        }
        s.push_str(a);
    }
    s
}

/// Apply the staged changes. On success the pending map is cleared.
pub fn apply(pending: &PendingMap) -> Result<String> {
    if pending.is_empty() {
        return Ok(String::new());
    }
    let args = build_command(pending);
    let out = Command::new("xrandr")
        .args(&args)
        .output()
        .context("failed to execute xrandr")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        return Err(anyhow!(
            "xrandr exited with status {}\nstdout: {}\nstderr: {}",
            out.status,
            stdout,
            stderr
        ));
    }
    Ok(preview_command(pending))
}

/// Convert a `PendingChange` into a fully populated `Output` snapshot. Used
/// when saving the current layout to the configuration file – we persist the
/// *result* of the staged changes, not the diff.
pub fn materialize(base: &Output, change: &PendingChange) -> Output {
    let mut out = base.clone();
    if let Some(m) = &change.mode {
        out.current_mode = Some(m.clone());
    }
    if let Some(off) = change.off {
        out.off = off;
        if off {
            out.current_mode = None;
        }
    }
    if change.primary == Some(true) {
        out.is_primary = true;
    }
    if let Some(r) = change.rotation {
        out.rotation = r;
    }
    if let Some(r) = change.reflection {
        out.reflection = r;
    }
    if let Some(s) = change.scale {
        out.scale = Some(s);
    }
    if let Some(s) = change.scale_from {
        out.scale_from = Some(s);
    }
    if let Some(g) = change.gamma {
        out.gamma = Some(g);
    }
    if let Some(p) = change.position {
        out.position = Some(p);
    }
    if let Some(rel) = &change.relative_to {
        out.relative_to = rel.clone();
    }
    out
}

/// Set the relative position of one output to another.
#[allow(dead_code)]
pub fn make_relative(change: &mut PendingChange, other: &str, pos: RelativePosition) {
    change.relative_to = Some(Some((other.to_string(), pos)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Reflection, Rotation};

    #[test]
    fn change_to_args_mode() {
        let mut c = PendingChange::default();
        c.mode = Some(Mode::new("1920x1080".into(), 60.0, true, false));
        let args = change_to_args(&c);
        assert_eq!(
            args,
            vec!["--mode", "1920x1080", "--rate", "60.000"]
        );
    }

    #[test]
    fn change_to_args_off_short_circuits() {
        let mut c = PendingChange::default();
        c.off = Some(true);
        c.rotation = Some(Rotation::Left); // should be ignored
        let args = change_to_args(&c);
        assert_eq!(args, vec!["--off"]);
    }

    #[test]
    fn change_to_args_auto() {
        // Pressing `p` stages off = Some(false), which must be emitted as
        // --auto so xrandr actually turns the output on.
        let mut c = PendingChange::default();
        c.off = Some(false);
        let args = change_to_args(&c);
        assert_eq!(args, vec!["--auto"]);
    }

    #[test]
    fn auto_overrides_with_mode() {
        // Staging both a mode and --auto should produce both, in that order
        // (xrandr accepts the order --auto --mode <m> --rate <r>).
        let mut c = PendingChange::default();
        c.off = Some(false);
        c.mode = Some(Mode::new("1920x1080".into(), 60.0, true, false));
        let args = change_to_args(&c);
        assert!(args.contains(&"--auto".to_string()));
        assert!(args.contains(&"--mode".to_string()));
        assert!(args.contains(&"1920x1080".to_string()));
    }

    #[test]
    fn pressing_d_then_p_resolves_to_auto() {
        // `d` then `p` should land on --auto (the latest keystroke wins).
        let mut c = PendingChange::default();
        c.off = Some(true);
        c.off = Some(false);
        let args = change_to_args(&c);
        assert_eq!(args, vec!["--auto"]);
    }

    #[test]
    fn change_to_args_full() {
        let mut c = PendingChange::default();
        c.mode = Some(Mode::new("1280x720".into(), 60.0, true, false));
        c.primary = Some(true);
        c.rotation = Some(Rotation::Right);
        c.reflection = Some(Reflection::Xy);
        c.scale = Some((1.25, 1.25));
        c.gamma = Some((1.0, 0.9, 0.8));
        c.position = Some((100, 50));
        c.relative_to = Some(Some(("eDP-1".into(), RelativePosition::RightOf)));
        let args = change_to_args(&c);
        assert!(args.contains(&"--primary".to_string()));
        assert!(args.contains(&"--rotate".to_string()));
        assert!(args.contains(&"right".to_string()));
        assert!(args.contains(&"--reflect".to_string()));
        assert!(args.contains(&"xy".to_string()));
        assert!(args.contains(&"--scale".to_string()));
        assert!(args.contains(&"1.25x1.25".to_string()));
        assert!(args.contains(&"--gamma".to_string()));
        assert!(args.contains(&"1:0.9:0.8".to_string()));
        assert!(args.contains(&"--pos".to_string()));
        assert!(args.contains(&"100x50".to_string()));
        assert!(args.contains(&"--right-of".to_string()));
        assert!(args.contains(&"eDP-1".to_string()));
    }

    #[test]
    fn reflection_normal_is_emitted() {
        // Cycling `x` back to `normal` must still produce --reflect normal
        // so the preview line is accurate and the xrandr invocation is
        // a canonical "reset" call.
        let mut c = PendingChange::default();
        c.reflection = Some(Reflection::Normal);
        let args = change_to_args(&c);
        assert!(args.contains(&"--reflect".to_string()));
        assert!(args.contains(&"normal".to_string()));
    }

    #[test]
    fn build_command_orders_newmode_first() {
        let mut p: PendingMap = PendingMap::new();
        let mut c = PendingChange::default();
        c.mode = Some(Mode::new("CustomMode".into(), 60.0, true, false));
        c.custom_modes.push(CustomMode {
            name: "CustomMode".into(),
            modeline: "83.5 1920 1936 1960 2000 1080 1083 1088 1122 -hsync +vsync".into(),
        });
        p.insert("HDMI-1".into(), c);
        let args = build_command(&p);
        let newmode_idx = args.iter().position(|a| a == "--newmode").unwrap();
        let output_idx = args.iter().position(|a| a == "--output").unwrap();
        assert!(newmode_idx < output_idx);
    }

    #[test]
    fn materialize_merges_into_base() {
        let mut base = Output::default();
        base.name = "HDMI-1".into();
        base.connected = true;
        let mut c = PendingChange::default();
        c.off = Some(true);
        c.rotation = Some(Rotation::Left);
        let merged = materialize(&base, &c);
        assert!(merged.off);
        assert_eq!(merged.rotation, Rotation::Left);
        assert_eq!(merged.name, "HDMI-1");
    }
}
