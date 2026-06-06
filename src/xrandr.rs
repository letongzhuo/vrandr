//! Parser for `xrandr --current --prop` output.
//!
//! The format of xrandr is line-based. A typical block looks like:
//!
//! ```text
//! eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 309mm x 174mm
//!     EDID:
//!         00ffffffffffff00...
//!     scaling mode: Full
//!     ...
//!     1920x1080     60.02*+  60.01
//!     1680x1050     59.95
//!     ...
//! HDMI-1 disconnected
//! ```
//!
//! The parser walks the lines once, tracks whether it is currently inside a
//! connected-output block, and pushes the appropriate data into the
//! `Output` value it builds. EDID data is collected as raw hex bytes.

use crate::model::{parse_refresh_rate, parse_resolution, Mode, Output, Rotation};
use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// Run `xrandr --current --prop` and parse the output into a list of
/// `Output` entries.
pub fn query_xrandr() -> Result<Vec<Output>> {
    let raw = run_xrandr()?;
    parse_xrandr(&raw)
}

/// Run `xrandr` with arbitrary arguments and return its stdout as a String.
pub fn run_xrandr() -> Result<String> {
    let out = Command::new("xrandr")
        .arg("--current")
        .arg("--prop")
        .output()
        .context("failed to execute `xrandr` – is it installed and on $PATH?")?;
    if !out.status.success() {
        return Err(anyhow!(
            "`xrandr --current --prop` exited with status {}",
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse the textual output of `xrandr --current --prop` into a list of
/// `Output` entries. This is exposed so unit tests can drive it with a fake
/// payload.
pub fn parse_xrandr(raw: &str) -> Result<Vec<Output>> {
    let mut outputs: Vec<Output> = Vec::new();
    let mut current: Option<usize> = None;

    for raw_line in raw.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            continue;
        }

        // A header line is one that does not start with whitespace and does
        // not contain ':'. Mode lines always begin with a resolution so they
        // always have an 'x'.
        let is_header = !line.starts_with(char::is_whitespace) && !line.contains(':');

        if is_header {
            let output = parse_output_header(line)?;
            outputs.push(output);
            current = Some(outputs.len() - 1);
            continue;
        }

        // Indented line: must belong to a header.
        let Some(idx) = current else { continue };
        let out = &mut outputs[idx];
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("EDID:") {
            // EDID line itself is usually just "EDID:"; data follows on the
            // next lines, indented. We just record that the section starts
            // here. Bytes are appended by the hex branch below.
            let _ = rest;
            out.edid = Some(Vec::new());
            continue;
        }

        // Hex lines under EDID: - either "aa bb cc dd" (whitespace
        // separated) or a continuous string of hex digits. We accept both
        // forms and parse 2-character chunks.
        if !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c.is_whitespace())
            && out.edid.is_some()
            && trimmed.len() >= 8
        {
            if let Some(buf) = out.edid.as_mut() {
                // Strip whitespace first, then walk two chars at a time.
                let compact: String = trimmed.split_whitespace().collect();
                let chars: Vec<char> = compact.chars().collect();
                for pair in chars.chunks(2) {
                    if pair.len() == 2 {
                        if let Ok(b) = u8::from_str_radix(&format!("{}{}", pair[0], pair[1]), 16) {
                            buf.push(b);
                        }
                    }
                }
            }
            continue;
        }

        // Skip `scaling mode:`, `Colors:`, `non-desktop:`, etc. – properties
        // we don't model. We identify a mode line by checking that the
        // first whitespace-delimited token is a resolution.
        if let Some((name, rest)) = trimmed.split_once(char::is_whitespace) {
            if parse_resolution(name).is_some() {
                for mode in build_modes(name, rest.trim()) {
                    out.available_modes.push(mode);
                }
                continue;
            }
        }
        // Any other indented property line (e.g. `scaling mode: Full` or
        // `non-desktop: 0`) is ignored.
    }

    // Post-pass: copy the actual current refresh rate from the
    // `available_modes` list (where the `*` marker is recorded) into
    // `current_mode`. The header-line parser only sees the geometry
    // (e.g. "1920x1080+0+0") and cannot tell which of the 165/180/144
    // rates is the one currently in use.
    for out in &mut outputs {
        if let Some(cur) = &out.current_mode {
            if cur.refresh_rate > 0.0 {
                continue;
            }
            if let Some(found) = out
                .available_modes
                .iter()
                .find(|m| m.name == cur.name && m.is_current)
            {
                out.current_mode = Some(found.clone());
            } else if let Some(found) = out
                .available_modes
                .iter()
                .find(|m| m.name == cur.name)
            {
                // Fallback: pick the first mode with the same name. This
                // happens when xrandr forgets to put `*` on the current
                // line (rare, but observed on some drivers).
                out.current_mode = Some(found.clone());
            }
        }
    }

    Ok(outputs)
}

/// Expand a single xrandr mode line into one `Mode` per refresh rate.
///
/// Example input (`name` = "1920x1080", `rest` = "165.00*+  180.00  144.00")
/// produces three `Mode` records:
///   - 1920x1080 @ 165.00Hz  (current, preferred)
///   - 1920x1080 @ 180.00Hz
///   - 1920x1080 @ 144.00Hz
///
/// The `*` marker indicates the *current* mode and the `+` indicates the
/// *preferred* mode. Both may appear on the same token, in which case the
/// resulting `Mode` is flagged as both.
fn build_modes(name: &str, rest: &str) -> Vec<Mode> {
    let mut out: Vec<Mode> = Vec::new();
    let mut pushed_any = false;
    for tok in rest.split_whitespace() {
        // Strip trailing flag chars; we only need the leading number.
        let bare = tok
            .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
            .to_string();
        let rate = match parse_refresh_rate(&bare) {
            Some(r) => r,
            None => continue,
        };
        let is_current = tok.contains('*');
        let is_preferred = tok.contains('+');
        out.push(Mode::new(
            name.to_string(),
            rate,
            is_current,
            is_preferred,
        ));
        pushed_any = true;
    }
    if !pushed_any {
        // Fallback: take the first number we can parse from the whole tail.
        if let Some(rate) = parse_refresh_rate(rest) {
            out.push(Mode::new(name.to_string(), rate, false, false));
        } else {
            // No rate info at all – keep the mode so the user can still see
            // the resolution, but mark rate as 0.0.
            out.push(Mode::new(name.to_string(), 0.0, false, false));
        }
    }
    out
}

fn parse_output_header(line: &str) -> Result<Output> {
    // Examples:
    //   "eDP-1 connected 1920x1080+0+0 (normal left inverted right x axis y axis) 309mm x 174mm"
    //   "HDMI-1 connected primary 1920x1080+0+1920 (normal left inverted right x axis y axis) 553mm x 311mm"
    //   "DP-1 disconnected (normal left inverted right x axis y axis)"
    //   "VGA-1 connected 1280x1024+0+0 left (normal left inverted right x axis y axis)"
    //   "eDP connected primary (normal left inverted right x axis y axis)"  ← no offset → off
    //
    // **Important parsing rules** (these were bugs in earlier revisions):
    //
    // 1. The parenthesised group `(normal left inverted right x axis y axis)`
    //    enumerates *supported* rotations and reflections – it does NOT
    //    describe the current reflection. We therefore only scan for
    //    rotation tokens BEFORE the opening `(` and treat everything after
    //    the closing `)` (size in millimetres, gamma, etc.) as opaque.
    // 2. If the header line has no `+X+Y` offset token, the output is
    //    currently **off** (it is `connected` but no CRTC is assigned).
    //    We must not invent a `current_mode` in that case.
    // 3. The current reflection cannot be recovered from `xrandr --current`
    //    at all – it is the default `normal` unless the program has
    //    staged or applied a change. We deliberately do not touch
    //    `Reflection` here.
    let mut out = Output::default();
    let mut parts = line.split_whitespace();
    let name = parts
        .next()
        .ok_or_else(|| anyhow!("malformed xrandr line: {line}"))?;
    out.name = name.to_string();

    let state = parts.next().unwrap_or("");
    out.connected = state == "connected";
    if !out.connected && state != "disconnected" {
        return Err(anyhow!("unexpected xrandr state token: {state}"));
    }

    // Find the *first* opening paren – everything before it is the
    // current-settings region. We intentionally do not parse anything
    // past the parens: the trailing "309mm x 174mm" contains a literal
    // 'x' that would otherwise be misread as `--reflect x`.
    let paren_open = line.find('(');
    let pre_paren = match paren_open {
        Some(idx) => &line[..idx],
        None => line,
    };

    // Tokenise the pre-paren region. Every whitespace-delimited token is
    // interpreted as either "primary", a geometry ("WxH+X+Y"), or a
    // rotation keyword. We do not honour reflection tokens here.
    let mut rotation = Rotation::Normal;
    let mut current_mode: Option<Mode> = None;
    let mut position: Option<(i32, i32)> = None;

    for tok in pre_paren.split_whitespace() {
        // Geometry tokens look like "1920x1080+0+0" (current mode) or
        // "1920x1080+0+0left" (mode + rotation glued together).
        if strip_geom_token(tok).is_some() {
            let (geom_only, trailing_rot) = split_off_rotation(tok);
            if let Some(rot) = trailing_rot {
                rotation = rot;
            }
            if let Some((res, pos)) = parse_geometry(&geom_only) {
                if let Some((w, h)) = parse_resolution(res) {
                    current_mode = Some(Mode::new(format!("{w}x{h}"), 0.0, true, false));
                    position = Some(pos);
                }
            }
            continue;
        }
        match tok {
            "primary" => out.is_primary = true,
            "left" => rotation = Rotation::Left,
            "right" => rotation = Rotation::Right,
            "inverted" => rotation = Rotation::Inverted,
            "normal" => rotation = Rotation::Normal,
            _ => {
                // A token we don't recognise (and which is not a geometry
                // string) is ignored. The pre-paren region is the right
                // place for current settings; anything else is a bug in
                // xrandr or a future feature.
            }
        }
    }

    out.rotation = rotation;
    // out.reflection is left at its `Default` (Normal) – the parser
    // never sets it. The program treats `normal` as the authoritative
    // baseline; user changes are tracked via `PendingChange` or the
    // loaded configuration.
    if let Some(m) = current_mode {
        out.current_mode = Some(m);
        out.position = position;
    } else {
        // No geometry token found. If the output is connected and not
        // explicitly marked "disconnected", it has no CRTC assigned and
        // is therefore off.
        if out.connected {
            out.off = true;
        }
        out.current_mode = None;
        out.position = None;
    }

    Ok(out)
}

/// Returns `true` if the token *looks like* an xrandr geometry string,
/// i.e. starts with `<digits>x<digits>` and contains a `+` somewhere.
/// We don't fully validate the format here; `parse_geometry` does that.
fn strip_geom_token(tok: &str) -> Option<&str> {
    if !tok.contains('+') {
        return None;
    }
    let first_x = tok.find('x')?;
    // The character before 'x' must be a digit and the character after
    // a digit for this to be a "<W>x<H>" prefix. We don't insist on
    // strict digits; `parse_resolution` will accept the string.
    if first_x == 0 {
        return None;
    }
    let bytes = tok.as_bytes();
    if !bytes[first_x - 1].is_ascii_digit() {
        return None;
    }
    if bytes.get(first_x + 1).copied().map(|b| b.is_ascii_digit()) != Some(true) {
        return None;
    }
    Some(tok)
}

/// Split a geometry token into `(geom, rotation_suffix)`. The geometry
/// is the substring matching `WxH+X+Y` (with no rotation glued on). The
/// rotation suffix, if any, is one of `left`, `right`, `inverted`,
/// `normal` and is glued to the end of the Y offset, e.g.
/// `1920x1080+0+0left`.
fn split_off_rotation(tok: &str) -> (String, Option<Rotation>) {
    // Find the second '+' (end of geometry). After it, skip any digits
    // (the Y offset may have multiple digits) and check whether the
    // remaining suffix is a rotation keyword.
    let mut plus_count = 0;
    let mut cut = tok.len();
    for (i, c) in tok.char_indices() {
        if c == '+' {
            plus_count += 1;
            if plus_count == 2 {
                // Skip the Y-offset digits after the second '+'.
                let rest = &tok[i + 1..];
                let mut j = rest.len();
                for (k, ch) in rest.char_indices() {
                    if ch.is_ascii_digit() {
                        j = k + ch.len_utf8();
                    } else {
                        break;
                    }
                }
                cut = i + 1 + j;
                break;
            }
        }
    }
    let (geom, rest) = tok.split_at(cut);
    let rot = match rest {
        "left" => Some(Rotation::Left),
        "right" => Some(Rotation::Right),
        "inverted" => Some(Rotation::Inverted),
        "normal" => Some(Rotation::Normal),
        _ => None,
    };
    (geom.to_string(), rot)
}

/// Parse the geometry portion of a token. Returns the resolution name
/// (e.g. "1920x1080") and the (x, y) position, if both are well-formed.
fn parse_geometry(geom: &str) -> Option<(&str, (i32, i32))> {
    // Expected form: "<res>+<x>+<y>"  (no trailing rotation here – that
    // is stripped by `split_off_rotation`).
    let mut it = geom.split('+');
    let res = it.next()?;
    let x: i32 = it.next()?.parse().ok()?;
    let y: i32 = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((res, (x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Reflection;

    const SAMPLE: &str = "\
Screen 0: minimum 320 x 200, current 3840 x 1080, maximum 16384 x 16384
eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 309mm x 174mm
	EDID:
		00ffffffffffff0006af2d2000000000
		0014010380201278ea8585a6574a9c25
		125054000000010101010101010101010101010101010101
	scaling mode: Full
		supported: None, Full, Center, Full aspect
	non-desktop: 0
		supported: 0, 1
	1920x1080     165.00*+  180.00   144.00   120.00   100.00   119.88    60.00
	1680x1050     59.95
	1280x1024     60.02
	1280x720      60.00
HDMI-1 connected 1920x1080+1920+0 (normal left inverted right x axis y axis) 553mm x 311mm
	EDID:
		00ffffffffffff0006af2d2000000000
		0014010380201278ea8585a6574a9c25
		125054000000010101010101010101010101010101010101
	1920x1080     60.00*+  50.00
	1920x1080i    30.00
	1280x720      60.00
DP-1 disconnected (normal left inverted right x axis y axis)
";

    #[test]
    fn parse_full_sample() {
        let outs = parse_xrandr(SAMPLE).expect("parse ok");
        assert_eq!(outs.len(), 3);

        let edp = &outs[0];
        assert_eq!(edp.name, "eDP-1");
        assert!(edp.connected);
        assert!(edp.is_primary);
        assert_eq!(edp.rotation, Rotation::Normal);
        assert!(edp.edid.as_ref().unwrap().len() >= 12);

        // Current mode should have been picked up from geometry.
        let cur = edp.current_mode.as_ref().expect("current mode");
        assert_eq!(cur.name, "1920x1080");

        // We should have parsed at least the four resolution lines.
        assert!(edp.available_modes.len() >= 4);

        // The 1920x1080 line carries several refresh rates; the first
        // (165.00*+) must be flagged as both current and preferred.
        let first = &edp.available_modes[0];
        assert_eq!(first.name, "1920x1080");
        assert!((first.refresh_rate - 165.00).abs() < 0.01);
        assert!(first.is_current);
        assert!(first.is_preferred);

        // And the other refresh rates for the same resolution must follow
        // as additional Mode records with the same name.
        let same_res: Vec<&Mode> = edp
            .available_modes
            .iter()
            .filter(|m| m.name == "1920x1080")
            .collect();
        assert!(same_res.len() >= 5, "expected >=5 1920x1080 entries, got {}", same_res.len());
        let rates: Vec<f32> = same_res.iter().map(|m| m.refresh_rate).collect();
        for expected in [180.0_f32, 144.0, 120.0, 100.0, 60.0] {
            assert!(
                rates.iter().any(|r| (*r - expected).abs() < 0.01),
                "missing 1920x1080 @{expected}Hz, got {rates:?}"
            );
        }
        // Only the 165.00 entry should be current+preferred.
        let current_count = same_res.iter().filter(|m| m.is_current).count();
        let preferred_count = same_res.iter().filter(|m| m.is_preferred).count();
        assert_eq!(current_count, 1, "exactly one entry should be current");
        assert_eq!(preferred_count, 1, "exactly one entry should be preferred");

        let hdmi = &outs[1];
        assert_eq!(hdmi.name, "HDMI-1");
        assert!(hdmi.connected);
        assert!(!hdmi.is_primary);
        assert!(hdmi.edid.is_some());

        let dp = &outs[2];
        assert_eq!(dp.name, "DP-1");
        assert!(!dp.connected);
    }

    #[test]
    fn parse_minimal() {
        let s = "X-1 connected 800x600+0+0 (normal x y)\n";
        let outs = parse_xrandr(s).expect("parse ok");
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].name, "X-1");
        assert_eq!(outs[0].current_mode.as_ref().unwrap().name, "800x600");
    }

    #[test]
    fn parse_rotated() {
        // The current rotation appears before the parenthesised list.
        let s = "VGA-1 connected 1280x1024+0+0 left (normal left inverted right x axis y axis)\n";
        let outs = parse_xrandr(s).expect("parse ok");
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].rotation, Rotation::Left);
        assert_eq!(outs[0].reflection, Reflection::Normal);
    }

    #[test]
    fn parse_empty() {
        let outs = parse_xrandr("").expect("parse ok");
        assert!(outs.is_empty());
    }

    #[test]
    fn parse_multi_refresh_rates() {
        // A line with several rates, with `*` on a non-first entry.
        let s = "X-1 connected 1920x1080+0+0 (normal)\n\t1920x1080     165.00+  180.00*  144.00  120.00  60.00\n";
        let outs = parse_xrandr(s).expect("parse ok");
        assert_eq!(outs.len(), 1);
        let modes = &outs[0].available_modes;
        assert_eq!(modes.len(), 5);
        let rates: Vec<f32> = modes.iter().map(|m| m.refresh_rate).collect();
        assert_eq!(rates, vec![165.00, 180.00, 144.00, 120.00, 60.00]);
        // 165.00 is preferred, 180.00 is current.
        let pref = modes.iter().find(|m| m.is_preferred).unwrap();
        let cur = modes.iter().find(|m| m.is_current).unwrap();
        assert!((pref.refresh_rate - 165.00).abs() < 0.01);
        assert!((cur.refresh_rate - 180.00).abs() < 0.01);
    }

    #[test]
    fn parse_single_rate_unchanged() {
        // Single rate per resolution should still yield a single Mode.
        let s = "X-1 connected 1680x1050+0+0 (normal)\n\t1680x1050     59.95\n";
        let outs = parse_xrandr(s).expect("parse ok");
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].available_modes.len(), 1);
        let m = &outs[0].available_modes[0];
        assert_eq!(m.name, "1680x1050");
        assert!((m.refresh_rate - 59.95).abs() < 0.01);
        assert!(!m.is_current);
        assert!(!m.is_preferred);
    }
}
