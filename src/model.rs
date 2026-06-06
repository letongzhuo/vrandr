//! Core data structures for vrandr.
//!
//! Defines `Mode`, `Output`, `PendingChange` and a few supporting enums used
//! to model the state of the user's displays plus any modifications the user
//! has staged but not yet applied to `xrandr`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A display mode reported by xrandr.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    /// Mode name as it appears in xrandr output, e.g. "1920x1080".
    pub name: String,
    /// Refresh rate in Hz, e.g. 60.00.
    pub refresh_rate: f32,
    /// Whether this mode is the currently active one for the output.
    pub is_current: bool,
    /// Whether this mode is the preferred (highest-resolution native) mode.
    pub is_preferred: bool,
    /// Width in pixels (parsed from the name, useful for sorting/UI).
    pub width: u32,
    /// Height in pixels (parsed from the name).
    pub height: u32,
}

impl Mode {
    /// Convenience constructor that derives width/height from the name.
    pub fn new(name: String, refresh_rate: f32, is_current: bool, is_preferred: bool) -> Self {
        let (width, height) = parse_resolution(&name).unwrap_or((0, 0));
        Self {
            name,
            refresh_rate,
            is_current,
            is_preferred,
            width,
            height,
        }
    }

    /// Human-readable label, e.g. `1920x1080  60.00`.
    #[allow(dead_code)]
    pub fn label(&self) -> String {
        format!("{:<10}  {:>6.2}Hz", self.name, self.refresh_rate)
    }
}

/// Rotation values accepted by xrandr (`--rotate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rotation {
    Normal,
    Left,
    Right,
    Inverted,
}

impl Rotation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Rotation::Normal => "normal",
            Rotation::Left => "left",
            Rotation::Right => "right",
            Rotation::Inverted => "inverted",
        }
    }

    /// Cycle to the next value, wrapping back to `Normal`.
    pub fn next(self) -> Self {
        match self {
            Rotation::Normal => Rotation::Left,
            Rotation::Left => Rotation::Right,
            Rotation::Right => Rotation::Inverted,
            Rotation::Inverted => Rotation::Normal,
        }
    }
}

impl Default for Rotation {
    fn default() -> Self {
        Rotation::Normal
    }
}

/// Reflection values accepted by xrandr (`--reflect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reflection {
    Normal,
    X,
    Y,
    Xy,
}

impl Reflection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reflection::Normal => "normal",
            Reflection::X => "x",
            Reflection::Y => "y",
            Reflection::Xy => "xy",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Reflection::Normal => Reflection::X,
            Reflection::X => Reflection::Y,
            Reflection::Y => Reflection::Xy,
            Reflection::Xy => Reflection::Normal,
        }
    }
}

impl Default for Reflection {
    fn default() -> Self {
        Reflection::Normal
    }
}

/// Relative position between two outputs, e.g. "right-of eDP-1".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelativePosition {
    LeftOf,
    RightOf,
    Above,
    Below,
    SameAs,
}

impl RelativePosition {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelativePosition::LeftOf => "left-of",
            RelativePosition::RightOf => "right-of",
            RelativePosition::Above => "above",
            RelativePosition::Below => "below",
            RelativePosition::SameAs => "same-as",
        }
    }
}

/// State of a single output/display.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub name: String,
    pub connected: bool,
    pub current_mode: Option<Mode>,
    pub available_modes: Vec<Mode>,
    pub is_primary: bool,
    pub rotation: Rotation,
    pub reflection: Reflection,
    pub scale: Option<(f32, f32)>,
    pub scale_from: Option<(f32, f32)>,
    pub gamma: Option<(f32, f32, f32)>,
    pub position: Option<(i32, i32)>,
    pub relative_to: Option<(String, RelativePosition)>,
    pub off: bool,
    pub edid: Option<Vec<u8>>,
    /// `xrandr --newmode` / `--addmode` definitions loaded from the configuration
    /// file. They are re-applied on startup if the user chose to load the saved
    /// layout.
    pub custom_modes: Vec<CustomMode>,
}

/// A custom mode definition loaded from configuration.
///
/// Custom modes are added with `xrandr --newmode <name> <modeline>`. We keep
/// the modeline string here so that it can be replayed when the user loads a
/// saved layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMode {
    pub name: String,
    pub modeline: String,
}

impl Output {
    /// A short status string for the output list column.
    pub fn status_label(&self) -> String {
        if self.off {
            return "off".to_string();
        }
        match &self.current_mode {
            Some(m) => format!("{} @ {:.2}", m.name, m.refresh_rate),
            None => "(no mode)".to_string(),
        }
    }
}

/// Staged, not-yet-applied changes for a single output.
///
/// The pending representation mirrors the fields of `Output` but every field
/// is optional: `Some` means the user has changed that value, `None` means
/// "leave the current value alone".
#[derive(Debug, Clone, Default)]
pub struct PendingChange {
    pub mode: Option<Mode>,
    pub off: Option<bool>,
    pub primary: Option<bool>,
    pub rotation: Option<Rotation>,
    pub reflection: Option<Reflection>,
    pub scale: Option<(f32, f32)>,
    pub scale_from: Option<(f32, f32)>,
    pub gamma: Option<(f32, f32, f32)>,
    pub position: Option<(i32, i32)>,
    /// (width, height) in pixels for `--panning`.
    pub panning: Option<(u32, u32)>,
    pub relative_to: Option<Option<(String, RelativePosition)>>,
    /// Reserved for future EDID-overriding use.
    #[allow(dead_code)]
    pub edid: Option<Option<Vec<u8>>>,
    /// Custom modes to (re)create with `--newmode`/`--addmode`.
    pub custom_modes: Vec<CustomMode>,
}

impl PendingChange {
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.off.is_none()
            && self.primary.is_none()
            && self.rotation.is_none()
            && self.reflection.is_none()
            && self.scale.is_none()
            && self.scale_from.is_none()
            && self.gamma.is_none()
            && self.position.is_none()
            && self.panning.is_none()
            && self.relative_to.is_none()
            && self.edid.is_none()
            && self.custom_modes.is_empty()
    }

    /// Returns true if at least one user-visible field is staged.
    pub fn has_visible_change(&self) -> bool {
        self.mode.is_some()
            || self.off.is_some()
            || self.primary == Some(true)
            || self.rotation.is_some()
            || self.reflection.is_some()
            || self.scale.is_some()
            || self.scale_from.is_some()
            || self.gamma.is_some()
            || self.position.is_some()
            || self.panning.is_some()
            || self.relative_to.is_some()
    }
}

/// The whole pending state, indexed by output name.
pub type PendingMap = HashMap<String, PendingChange>;

/// Parses a resolution string like "1920x1080" into (width, height).
pub fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    let w = w.trim().parse().ok()?;
    let h = h.trim().parse().ok()?;
    Some((w, h))
}

/// Parses a refresh rate like "60.00" or "60.00*".
pub fn parse_refresh_rate(s: &str) -> Option<f32> {
    s.trim()
        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resolution_ok() {
        assert_eq!(parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_resolution(" 800x600 "), Some((800, 600)));
    }

    #[test]
    fn parse_resolution_fail() {
        assert!(parse_resolution("abc").is_none());
        assert!(parse_resolution("1920").is_none());
    }

    #[test]
    fn parse_refresh_rate_ok() {
        assert!((parse_refresh_rate("60.00*").unwrap() - 60.0).abs() < 1e-3);
        assert!((parse_refresh_rate("59.94+").unwrap() - 59.94).abs() < 1e-3);
    }

    #[test]
    fn rotation_cycle() {
        assert_eq!(Rotation::Normal.next(), Rotation::Left);
        assert_eq!(Rotation::Inverted.next(), Rotation::Normal);
    }

    #[test]
    fn reflection_cycle() {
        assert_eq!(Reflection::Normal.next(), Reflection::X);
        assert_eq!(Reflection::Xy.next(), Reflection::Normal);
    }

    #[test]
    fn pending_change_empty() {
        let p = PendingChange::default();
        assert!(p.is_empty());
        assert!(!p.has_visible_change());
    }
}
