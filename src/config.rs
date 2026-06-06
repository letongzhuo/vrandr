//! Persistent configuration: save and load a layout in TOML.
//!
//! The file lives at `$XDG_CONFIG_HOME/vrandr/layout.toml` (falling back to
//! `~/.config/vrandr/layout.toml`). We persist a snapshot of the *final*
//! state (post-pending) so the file is easy to hand-edit and to reason
//! about – no need to replay a diff.

use crate::model::{CustomMode, Output, Reflection, RelativePosition, Rotation};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Top-level layout metadata. Currently only the schema version.
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub outputs: Vec<PersistedOutput>,
    #[serde(default)]
    pub custom_modes: Vec<CustomMode>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedOutput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default = "default_rotation")]
    pub rotation: Rotation,
    #[serde(default = "default_reflection")]
    pub reflection: Reflection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_from: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamma: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<PersistedRelative>,
    #[serde(default)]
    pub off: bool,
    /// Hex-encoded EDID blob, if we have one. Persisting this is useful for
    /// identifying the same physical monitor across reboots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edid_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedRelative {
    pub to: String,
    pub pos: RelativePosition,
}

fn default_rotation() -> Rotation {
    Rotation::Normal
}
fn default_reflection() -> Reflection {
    Reflection::Normal
}

impl PersistedOutput {
    /// Snapshot a live `Output` into a `PersistedOutput` ready for TOML.
    pub fn from_output(out: &Output) -> Self {
        Self {
            name: out.name.clone(),
            mode: out.current_mode.as_ref().map(|m| m.name.clone()),
            rate: out
                .current_mode
                .as_ref()
                .map(|m| m.refresh_rate)
                .filter(|r| *r > 0.0),
            primary: out.is_primary,
            rotation: out.rotation,
            reflection: out.reflection,
            scale: out.scale.map(|(x, y)| [x, y]),
            scale_from: out.scale_from.map(|(x, y)| [x, y]),
            gamma: out.gamma.map(|(r, g, b)| [r, g, b]),
            position: out.position.map(|(x, y)| [x, y]),
            relative: out.relative_to.as_ref().map(|(to, pos)| PersistedRelative {
                to: to.clone(),
                pos: *pos,
            }),
            off: out.off,
            edid_hex: out.edid.as_deref().map(hex_encode),
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = u8::from_str_radix(std::str::from_utf8(&bytes[i..i + 1]).ok()?, 16).ok()?;
        let lo = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 2]).ok()?, 16).ok()?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

impl LayoutConfig {
    pub fn from_outputs(outputs: &[Output]) -> Self {
        let mut customs: Vec<CustomMode> = Vec::new();
        for o in outputs {
            for cm in &o.custom_modes {
                if !customs.iter().any(|c| c.name == cm.name) {
                    customs.push(cm.clone());
                }
            }
        }
        Self {
            version: 1,
            outputs: outputs.iter().map(PersistedOutput::from_output).collect(),
            custom_modes: customs,
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize config to TOML")
    }

    pub fn from_toml(raw: &str) -> Result<Self> {
        toml::from_str(raw).context("failed to parse config TOML")
    }
}

/// Convert a `LayoutConfig` to a TOML string, optionally prefixing a
/// header that names the profile. The header is a comment, so the file
/// remains valid TOML.
pub fn serialize_with_profile_name(
    cfg: &LayoutConfig,
    profile_name: Option<&str>,
) -> Result<String> {
    let body = toml::to_string_pretty(cfg).context("failed to serialize config to TOML")?;
    let header = match profile_name {
        Some(n) => format!(
            "# vrandr profile: {n}\n\
             # Edit by hand if needed; vrandr will not preserve unrelated fields.\n"
        ),
        None => String::new(),
    };
    Ok(format!("{header}{body}"))
}

/// Return the base configuration directory (`$XDG_CONFIG_HOME/vrandr`).
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("vrandr")
}

/// Return the directory where named profiles are stored.
pub fn profiles_dir() -> PathBuf {
    config_dir().join("profiles")
}

/// Validate a profile name. Allowed: ASCII letters, digits, underscore,
/// dash, dot. Anything else is rejected so a stray path separator or
/// shell metacharacter cannot escape the profile directory.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("profile name is empty");
    }
    if name == "." || name == ".." {
        anyhow::bail!("profile name not allowed");
    }
    for c in name.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
        if !ok {
            anyhow::bail!("profile name has disallowed character: {c:?}");
        }
    }
    Ok(())
}

/// Return the canonical on-disk path for a given profile.
pub fn profile_path(name: &str) -> PathBuf {
    profiles_dir().join(format!("{name}.toml"))
}

/// Save the configuration to a named profile. Creates intermediate
/// directories as needed.
pub fn save_profile(name: &str, cfg: &LayoutConfig) -> Result<PathBuf> {
    validate_profile_name(name)?;
    let path = profile_path(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
    }
    let body = serialize_with_profile_name(cfg, Some(name))?;
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// Load a named profile. Returns `Ok(None)` if the file does not exist.
pub fn load_profile_by_name(name: &str) -> Result<Option<(PathBuf, LayoutConfig)>> {
    let path = profile_path(name);
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let cfg = LayoutConfig::from_toml(&body)?;
    Ok(Some((path, cfg)))
}

/// List the names of all saved profiles, sorted alphabetically. A
/// missing profile directory is treated as an empty list, not an error.
pub fn list_profiles() -> Result<Vec<String>> {
    let dir = profiles_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Backwards-compat: read the legacy `layout.toml` if present, so the
/// caller can offer to migrate it to a named profile.
#[allow(dead_code)]
pub fn load_legacy_layout() -> Result<Option<(PathBuf, LayoutConfig)>> {
    let path = config_dir().join("layout.toml");
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let cfg = LayoutConfig::from_toml(&body)?;
    Ok(Some((path, cfg)))
}

/// Return the canonical configuration file path (legacy `layout.toml`).
#[allow(dead_code)]
pub fn config_path() -> PathBuf {
    config_dir().join("layout.toml")
}

/// Persist the configuration to the legacy `layout.toml`. Kept for the
/// `--save` style escape hatch and tests; the in-app `W` key uses
/// `save_profile` instead.
#[allow(dead_code)]
pub fn save(cfg: &LayoutConfig) -> Result<PathBuf> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    let body = cfg.to_toml()?;
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// Load the configuration from the legacy `layout.toml`, if it exists.
#[allow(dead_code)]
pub fn load() -> Result<Option<(PathBuf, LayoutConfig)>> {
    load_legacy_layout()
}

/// Convert a loaded `PersistedOutput` back into a `PendingChange` so that
/// the user can apply the saved layout with `a`.
pub fn to_pending(p: &PersistedOutput) -> crate::model::PendingChange {
    use crate::model::PendingChange;
    let mut c = PendingChange::default();
    if p.off {
        c.off = Some(true);
    }
    if let Some(mode) = &p.mode {
        let rate = p.rate.unwrap_or(0.0);
        c.mode = Some(crate::model::Mode::new(mode.clone(), rate, true, false));
    }
    if p.primary {
        c.primary = Some(true);
    }
    // Always stage rotation and reflection, even when the persisted value
    // is `normal`. Combined with the always-emit behaviour in
    // `change_to_args`, this makes the load -> apply cycle deterministic:
    // a saved layout is applied verbatim, no live state leaks through.
    c.rotation = Some(p.rotation);
    c.reflection = Some(p.reflection);
    if let Some([sx, sy]) = p.scale {
        c.scale = Some((sx, sy));
    }
    if let Some([sx, sy]) = p.scale_from {
        c.scale_from = Some((sx, sy));
    }
    if let Some([r, g, b]) = p.gamma {
        c.gamma = Some((r, g, b));
    }
    if let Some([x, y]) = p.position {
        c.position = Some((x, y));
    }
    if let Some(rel) = &p.relative {
        c.relative_to = Some(Some((rel.to.clone(), rel.pos)));
    }
    c
}

/// Return the EDID bytes for a saved output, if any.
#[allow(dead_code)]
pub fn edid_of(p: &PersistedOutput) -> Option<Vec<u8>> {
    p.edid_hex.as_ref().and_then(|s| hex_decode(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Mode;

    #[test]
    fn hex_round_trip() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        let s = hex_encode(&bytes);
        let back = hex_decode(&s).unwrap();
        assert_eq!(bytes, back);
    }

    #[test]
    fn round_trip_outputs() {
        let mut o = Output::default();
        o.name = "HDMI-1".into();
        o.connected = true;
        o.current_mode = Some(Mode::new("1920x1080".into(), 60.0, true, false));
        o.is_primary = true;
        o.rotation = Rotation::Left;
        o.scale = Some((1.25, 1.25));
        o.position = Some((1920, 0));
        o.relative_to = Some(("eDP-1".into(), RelativePosition::RightOf));
        o.edid = Some(vec![0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0xaa]);

        let cfg = LayoutConfig::from_outputs(&[o]);
        let body = cfg.to_toml().unwrap();
        let back = LayoutConfig::from_toml(&body).unwrap();
        assert_eq!(back.outputs.len(), 1);
        let p = &back.outputs[0];
        assert_eq!(p.name, "HDMI-1");
        assert_eq!(p.mode.as_deref(), Some("1920x1080"));
        assert_eq!(p.primary, true);
        assert_eq!(p.rotation, Rotation::Left);
        assert_eq!(p.scale, Some([1.25, 1.25]));
        assert_eq!(p.relative.as_ref().unwrap().to, "eDP-1");
        assert_eq!(p.relative.as_ref().unwrap().pos, RelativePosition::RightOf);
        let c = to_pending(p);
        assert_eq!(c.rotation, Some(Rotation::Left));
        assert_eq!(c.scale, Some((1.25, 1.25)));
    }

    #[test]
    fn enum_values_are_lowercase_in_toml() {
        // This guards against the original bug where Reflection::X was
        // serialized as "X" (uppercase) and broke xrandr round-trips.
        let mut o = Output::default();
        o.name = "HDMI-1".into();
        o.connected = true;
        o.reflection = Reflection::X;
        o.rotation = Rotation::Left;
        o.relative_to = Some(("eDP-1".into(), RelativePosition::RightOf));
        let cfg = LayoutConfig::from_outputs(&[o]);
        let body = cfg.to_toml().unwrap();
        assert!(
            body.contains("reflection = \"x\""),
            "reflection should serialize as lowercase \"x\", got: {body}"
        );
        assert!(
            body.contains("rotation = \"left\""),
            "rotation should serialize as lowercase \"left\", got: {body}"
        );
        assert!(
            body.contains("pos = \"right-of\""),
            "relative pos should serialize as kebab-case \"right-of\", got: {body}"
        );
    }

    #[test]
    fn profile_name_validation() {
        assert!(validate_profile_name("home_dual").is_ok());
        assert!(validate_profile_name("work.toml").is_ok());
        assert!(validate_profile_name("a-b.c").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name(".").is_err());
        assert!(validate_profile_name("..").is_err());
        assert!(validate_profile_name("../escape").is_err());
        assert!(validate_profile_name("name with space").is_err());
        assert!(validate_profile_name("name/with/slash").is_err());
        assert!(validate_profile_name("name$bad").is_err());
    }

    #[test]
    fn save_and_load_profile_round_trip() {
        // Build a config, save it under a profile name, then load it back.
        // We don't depend on $HOME; we only assert the public functions
        // succeed when given a sanitised name.
        let mut o = Output::default();
        o.name = "HDMI-1".into();
        o.connected = true;
        o.reflection = Reflection::Y;
        o.rotation = Rotation::Right;
        let cfg = LayoutConfig::from_outputs(&[o]);
        let body = serialize_with_profile_name(&cfg, Some("unit_test_profile"))
            .expect("serialise");
        assert!(body.contains("# vrandr profile: unit_test_profile"));
        // Round-trip via the public parser so the test stays independent
        // of the file-system location.
        let parsed = LayoutConfig::from_toml(&body).expect("parse");
        assert_eq!(parsed.outputs.len(), 1);
        assert_eq!(parsed.outputs[0].reflection, Reflection::Y);
        assert_eq!(parsed.outputs[0].rotation, Rotation::Right);
    }
}
