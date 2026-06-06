//! Minimal EDID v1 base-block decoder.
//!
//! The decoder is intentionally tiny – it pulls out the fields that are
//! most useful to a human scanning the EDID popup (vendor, product, serial,
//! physical size, gamma, name string) and ignores the rest.
//!
//! Reference: EDID v1.4 data structure specification, VESA.

/// A handful of decoded EDID fields. `None` means the value is reserved
/// or otherwise unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdidInfo {
    pub manufacturer: Option<String>,
    pub product_code: Option<String>,
    pub serial: Option<String>,
    /// Physical width in centimetres (0 if unspecified).
    pub width_cm: u8,
    /// Physical height in centimetres (0 if unspecified).
    pub height_cm: u8,
    /// EDID version: `(version, revision)`.
    pub version: Option<(u8, u8)>,
    /// Established timings III: 16-byte bitmap. Some implementations don't
    /// include it; this is kept as a raw value.
    pub gamma: Option<u8>,
}

impl EdidInfo {
    /// Parse `bytes` as an EDID v1 base block. Returns `None` if the input
    /// is obviously not an EDID (wrong length or invalid header).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 128 {
            return None;
        }
        if &bytes[0..8] != b"\x00\xff\xff\xff\xff\xff\xff\x00" {
            return None;
        }
        let mut out = EdidInfo::default();
        out.manufacturer = Some(decode_manufacturer(&bytes[8..10]));
        out.product_code = Some(format!("0x{:04x}", u16::from_le_bytes([bytes[10], bytes[11]])));
        let serial = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        if serial != 0 {
            out.serial = Some(format!("{serial}"));
        }
        out.version = Some((bytes[18], bytes[19]));
        if bytes[23] != 0xff {
            out.width_cm = bytes[21];
            out.height_cm = bytes[22];
        }
        if bytes[24] != 0xff {
            // Stored as (value + 100) * 100, so the 8-bit result is the
            // gamma * 100 - 100. We keep the raw 8-bit value for display.
            out.gamma = Some(bytes[24]);
        }
        Some(out)
    }

    /// Format as a few human-readable lines (used by the EDID popup).
    pub fn summary_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((v, r)) = self.version {
            out.push(format!("EDID version:        {v}.{r}"));
        }
        if let Some(m) = &self.manufacturer {
            out.push(format!("Manufacturer:        {m}"));
        }
        if let Some(p) = &self.product_code {
            out.push(format!("Product code:        {p}"));
        }
        if let Some(s) = &self.serial {
            out.push(format!("Serial number:       {s}"));
        }
        if self.width_cm > 0 || self.height_cm > 0 {
            let w_in = self.width_cm as f32 / 2.54;
            let h_in = self.height_cm as f32 / 2.54;
            out.push(format!(
                "Physical size:       {} x {} cm  ({:.1}\" x {:.1}\")",
                self.width_cm, self.height_cm, w_in, h_in
            ));
        }
        if let Some(g) = self.gamma {
            let actual = (f32::from(g) + 100.0) / 100.0;
            out.push(format!("Gamma:               {actual:.2}"));
        }
        out
    }
}

/// Three-letter vendor ID packed into two bytes (5-bit letters, big-endian).
fn decode_manufacturer(b: &[u8]) -> String {
    if b.len() < 2 {
        return String::from("???");
    }
    let hi = b[0];
    let lo = b[1];
    // Bits: c1=hi>>2, c2=((hi&3)<<3)|(lo>>5), c3=lo&0x1f
    let c1 = (hi >> 2) & 0x1f;
    let c2 = ((hi & 0x03) << 3) | ((lo >> 5) & 0x07);
    let c3 = lo & 0x1f;
    let mut s = String::with_capacity(3);
    for c in [c1, c2, c3] {
        // The 5-bit value is a 1-indexed letter (A=1, B=2, ..., Z=26).
        // Anything outside 1..=26 is treated as unknown.
        if (1..=26).contains(&c) {
            s.push((c + b'A' - 1) as char);
        } else {
            s.push('?');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-crafted minimal EDID base block with all 0s except the
    /// mandatory header and version fields. Useful for negative tests.
    fn minimal_edid() -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[0..8].copy_from_slice(b"\x00\xff\xff\xff\xff\xff\xff\x00");
        v[18] = 1;
        v[19] = 4;
        v
    }

    #[test]
    fn decode_manufacturer_known() {
        // "DEL" is the canonical example: bytes 0x10 0xAC.
        assert_eq!(decode_manufacturer(&[0x10, 0xac]), "DEL");
        // "SAM" -> 0x4C 0x2D
        //   S=19, A=1, M=13
        //   hi = (19 << 2) | (1 >> 3) = 76 = 0x4C
        //   lo = ((1 & 7) << 5) | 13   = 45 = 0x2D
        assert_eq!(decode_manufacturer(&[0x4c, 0x2d]), "SAM");
    }

    #[test]
    fn parse_too_short() {
        assert!(EdidInfo::parse(&[0u8; 64]).is_none());
    }

    #[test]
    fn parse_bad_header() {
        let mut v = minimal_edid();
        v[0] = 0x42;
        assert!(EdidInfo::parse(&v).is_none());
    }

    #[test]
    fn parse_minimal_ok() {
        // Build a minimal EDID where the manufacturer is "AAA" (c1=c2=c3=1,
        // so vendor bytes are 0x04 0x21) and version is 1.4.
        let mut v = vec![0u8; 128];
        v[0..8].copy_from_slice(b"\x00\xff\xff\xff\xff\xff\xff\x00");
        v[8] = 0x04; // vendor hi -> c1=1 (A), top-2-bits-of-c2=0
        v[9] = 0x21; // vendor lo -> low-3-bits-of-c2=1 (A), c3=1 (A)
        v[18] = 1;
        v[19] = 4;
        let info = EdidInfo::parse(&v).expect("parses");
        assert_eq!(info.version, Some((1, 4)));
        assert_eq!(info.manufacturer.as_deref(), Some("AAA"));
        // No serial when all zero.
        assert!(info.serial.is_none());
        // Width/height are zero (reserved when == 0xff, which they aren't here).
        // The default is 0, so they are 0.
        assert_eq!(info.width_cm, 0);
        assert_eq!(info.height_cm, 0);
    }

    #[test]
    fn parse_with_dimensions_and_gamma() {
        let mut v = minimal_edid();
        v[21] = 60; // width cm
        v[22] = 34; // height cm
        v[23] = 1;  // not 0xff => "landscape" => use the size
        v[24] = 120; // gamma = (120+100)/100 = 2.20
        let info = EdidInfo::parse(&v).expect("parses");
        assert_eq!(info.width_cm, 60);
        assert_eq!(info.height_cm, 34);
        assert_eq!(info.gamma, Some(120));
        let lines = info.summary_lines();
        assert!(lines.iter().any(|l| l.contains("60 x 34 cm")));
        assert!(lines.iter().any(|l| l.contains("Gamma:               2.20")));
    }

    #[test]
    fn parse_with_serial() {
        let mut v = minimal_edid();
        // Big-endian serial of 0x12345678 in little-endian layout:
        v[12..16].copy_from_slice(&0x12345678u32.to_le_bytes());
        let info = EdidInfo::parse(&v).expect("parses");
        assert_eq!(info.serial.as_deref(), Some("305419896"));
    }

    #[test]
    fn summary_well_formed() {
        let v = minimal_edid();
        let info = EdidInfo::parse(&v).unwrap();
        let mut buf = String::new();
        for l in info.summary_lines() {
            buf.push_str(&l);
            buf.push('\n');
        }
        // Output is non-empty and contains the version line.
        assert!(buf.contains("EDID version:        1.4"));
    }
}
