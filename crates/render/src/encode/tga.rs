//! Targa export: uncompressed true colour at 16 bits per pixel (5-5-5 and a one-bit alpha), 24 or
//! 32 (8-bit alpha), rows bottom-up, with the version 2 footer.

use super::on_white;

/// Bits per pixel a Targa export can have.
pub const DEPTHS: [u8; 3] = [16, 24, 32];

/// The Targa options of a raster export.
#[derive(Clone, Debug, PartialEq)]
pub struct TgaOptions {
    /// Bits per pixel ([`DEPTHS`]).
    pub depth: u8,
}

impl Default for TgaOptions {
    fn default() -> Self {
        Self { depth: 24 }
    }
}

impl TgaOptions {
    /// Why these options can't be written, if they can't.
    pub fn check(&self) -> Result<(), String> {
        if DEPTHS.contains(&self.depth) { Ok(()) } else { Err(format!("Targa depth {}: 16, 24 or 32 bits per pixel", self.depth)) }
    }
}

/// Straight-alpha RGBA pixels as a Targa file. 24 bits blend partly transparent pixels over white;
/// at 16 bits a pixel is opaque from half opacity up, else transparent.
pub fn encode(rgba: &[u8], width: u32, height: u32, o: &TgaOptions) -> Result<Vec<u8>, String> {
    o.check()?;
    let (Ok(w16), Ok(h16)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(format!("{width} × {height} pixels is too large for Targa (at most 65535 pixels a side): lower the resolution"));
    };
    let (w, h) = (usize::from(w16), usize::from(h16));
    if w == 0 || h == 0 || (w * h).checked_mul(4) != Some(rgba.len()) {
        return Err("Targa encoding failed: the pixel buffer doesn't match the image size".into());
    }
    let px = rgba.as_chunks::<4>().0;
    let mut out = Vec::with_capacity((w * h).saturating_mul(usize::from(o.depth / 8)).saturating_add(18 + 26));
    // No image id or colour map, uncompressed true colour, origin (0, 0).
    out.extend([0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend(w16.to_le_bytes());
    out.extend(h16.to_le_bytes());
    // Bits per pixel, then the alpha bits (rows bottom-up, left to right).
    let alpha_bits = match o.depth {
        16 => 1,
        32 => 8,
        _ => 0,
    };
    out.extend([o.depth, alpha_bits]);
    for row in px.chunks(w).rev() {
        for p in row {
            match o.depth {
                16 => {
                    let v = if p[3] >= 128 {
                        let [r, g, b] = [p[0], p[1], p[2]].map(|c| u16::from(c >> 3));
                        0x8000 | r << 10 | g << 5 | b
                    } else {
                        0
                    };
                    out.extend(v.to_le_bytes());
                }
                24 => {
                    let [r, g, b] = on_white(p);
                    out.extend([b, g, r]);
                }
                _ => out.extend([p[2], p[1], p[0], p[3]]),
            }
        }
    }
    // No extension area or developer directory.
    out.extend([0; 8]);
    out.extend(b"TRUEVISION-XFILE.\0");
    Ok(out)
}
