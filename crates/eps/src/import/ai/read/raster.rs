//! The size of an embedded image (`XI`), checked before its samples are decoded: read at full size
//! up to a target number of pixels, averaged down in whole steps past it, and not read at all past
//! the hard limits ([`Limits`]).

use std::io::Write as _;

/// Images up to this many pixels read as they always have: an RGBA PNG, or a CMYK TIFF.
pub(super) const SMALL: u64 = 1 << 25;
/// Most bytes of one image's samples, its alpha channel included.
pub(super) const MAX_BYTES: u64 = 512 << 20;
/// Most pixels of one image.
pub(super) const MAX_PIXELS: u64 = 1 << 31;
/// Longest side of an image as it comes in (pixels).
const MAX_SIDE: u64 = 65_535;

/// How large an image is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Limits {
    /// Most pixels of an image as it comes in; larger ones are reduced.
    pub(super) target: u64,
    /// Most pixels of an image read at all.
    pub(super) pixels: u64,
    /// Most bytes of an image's samples, its alpha channel included.
    pub(super) bytes: u64,
}

#[cfg(test)]
thread_local! {
    static TEST_LIMITS: std::cell::Cell<Option<Limits>> = const { std::cell::Cell::new(None) };
}

impl Limits {
    /// The limits on this machine: [`target`] pixels, [`MAX_PIXELS`] and [`MAX_BYTES`].
    pub(super) fn get() -> Self {
        #[cfg(test)]
        if let Some(l) = TEST_LIMITS.with(std::cell::Cell::get) {
            return l;
        }
        static TARGET: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let target = *TARGET.get_or_init(|| target(vectorcraft_render::installed_memory()));
        Self { target, pixels: MAX_PIXELS, bytes: MAX_BYTES }
    }

    /// The limits for an image of `channels` colour channels: a CMYK image comes in at most as
    /// large as a CMYK TIFF holds ([`vectorcraft_doc::cmyk::MAX_CMYK_PIXELS`]).
    pub(super) fn for_channels(channels: usize) -> Self {
        let mut l = Self::get();
        if channels == 4 {
            l.target = l.target.min(vectorcraft_doc::cmyk::MAX_CMYK_PIXELS);
        }
        l
    }
}

/// `f` run with images read under `target` pixels, `pixels` and `bytes` in place of this machine's
/// limits.
#[cfg(test)]
pub(crate) fn with_image_limits<T>(target: u64, pixels: u64, bytes: u64, f: impl FnOnce() -> T) -> T {
    TEST_LIMITS.with(|l| l.set(Some(Limits { target, pixels, bytes })));
    let out = f();
    TEST_LIMITS.with(|l| l.set(None));
    out
}

/// Most pixels of an image as it comes in, on a machine with `memory` bytes installed (`None`: not
/// known): a thirty-second of it, from 2^26 to 2^28 pixels (an RGBA decode of at most 1 GiB); on the
/// web, a quarter of the largest image the renderer decodes, at most 2^27.
fn target(memory: Option<u64>) -> u64 {
    if cfg!(target_arch = "wasm32") {
        (1u64 << 27).min(vectorcraft_doc::MAX_RASTER_ALLOC / 4)
    } else {
        memory.map_or(1 << 26, |m| (m / 32).clamp(1 << 26, 1 << 28))
    }
}

/// An image's layout from its `XI` operands.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shape {
    pub(super) w: u64,
    pub(super) h: u64,
    /// Colour channels a pixel: 1, 3 or 4.
    pub(super) channels: u64,
    /// Bits a channel: 1 or 8.
    pub(super) bits: u64,
    /// An alpha channel, a byte a pixel, follows the colour samples.
    pub(super) alpha: bool,
}

impl Shape {
    /// Bytes in a row of colour samples (a 1-bit row is padded to a whole byte).
    pub(super) fn row(&self) -> Option<u64> {
        Some(self.w.checked_mul(self.channels)?.checked_mul(self.bits)?.div_ceil(8))
    }

    /// Bytes of colour samples, and of the alpha channel.
    pub(super) fn lens(&self) -> Option<(u64, u64)> {
        let colour = self.row()?.checked_mul(self.h)?;
        let alpha = if self.alpha { self.w.checked_mul(self.h)? } else { 0 };
        colour.checked_add(alpha)?;
        Some((colour, alpha))
    }

    /// The step `k` the image is reduced by (1: full size): the smallest that brings it within
    /// `limits.target` pixels and [`MAX_SIDE`] a side. `None` past `limits.pixels` or
    /// `limits.bytes`.
    pub(super) fn step(&self, limits: &Limits) -> Option<u64> {
        let pixels = self.w.checked_mul(self.h)?;
        let (colour, alpha) = self.lens()?;
        if pixels > limits.pixels || colour + alpha > limits.bytes {
            return None;
        }
        let fits = |k: u64| {
            let (w, h) = (self.w.div_ceil(k), self.h.div_ceil(k));
            w <= MAX_SIDE && h <= MAX_SIDE && w.saturating_mul(h) <= limits.target.max(1)
        };
        // No step below these brings it within the limits.
        let mut k = ((pixels as f64 / limits.target.max(1) as f64).sqrt().floor() as u64)
            .max(self.w.div_ceil(MAX_SIDE))
            .max(self.h.div_ceil(MAX_SIDE))
            .max(1);
        while !fits(k) {
            k += 1;
        }
        Some(k)
    }
}

/// The samples of the image `s` (rows of [`Shape::row`] bytes; a set bit is the channel at full
/// strength) and its `alpha` channel averaged over blocks of `k` × `k` pixels into a byte a channel,
/// each block at the right and bottom edges over the pixels it has. With `inline`, the alpha follows
/// each pixel's channels; otherwise it comes back on its own.
pub(super) fn reduce(samples: &[u8], alpha: Option<&[u8]>, s: Shape, k: u64, inline: bool) -> (Vec<u8>, Option<Vec<u8>>) {
    let (w, h, k) = (s.w as usize, s.h as usize, k.max(1) as usize);
    let (ch, row) = (s.channels as usize, s.row().unwrap_or(0) as usize);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let n = ch + usize::from(alpha.is_some());
    let mut out = Vec::with_capacity(ow * oh * if inline { n } else { ch });
    let mut alpha_out = (alpha.is_some() && !inline).then(|| Vec::with_capacity(ow * oh));
    let mut sums = vec![0u64; ow * n];
    for oy in 0..oh {
        sums.fill(0);
        let rows = oy * k..((oy + 1) * k).min(h);
        let tall = rows.len() as u64;
        for y in rows {
            let line = samples.get(y * row..(y + 1) * row).unwrap_or_default();
            let opacity = alpha.and_then(|a| a.get(y * w..(y + 1) * w));
            for x in 0..w {
                let at = x / k * n;
                for c in 0..ch {
                    let i = x * ch + c;
                    let v =
                        if s.bits == 1 { line.get(i / 8).map_or(0, |b| ((b >> (7 - i % 8)) & 1) * 255) } else { line.get(i).copied().unwrap_or(0) };
                    sums[at + c] += u64::from(v);
                }
                if alpha.is_some() {
                    sums[at + ch] += u64::from(opacity.and_then(|o| o.get(x)).copied().unwrap_or(255));
                }
            }
        }
        for ox in 0..ow {
            let count = ((((ox + 1) * k).min(w) - ox * k) as u64 * tall).max(1);
            let avg = |sum: u64| ((sum + count / 2) / count) as u8;
            out.extend(sums[ox * n..ox * n + ch].iter().map(|v| avg(*v)));
            if alpha.is_some() {
                let a = avg(sums[ox * n + ch]);
                match alpha_out.as_mut() {
                    Some(o) => o.push(a),
                    None => out.push(a),
                }
            }
        }
    }
    (out, alpha_out)
}

/// A grayscale PNG of 1-bit rows as they are (`row` bytes each, a set bit white): `w` × `h`
/// pixels without expanding them.
pub(super) fn gray1_png(samples: &[u8], w: u32, h: u32, row: usize) -> Option<Vec<u8>> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    for y in 0..h as usize {
        // Filter type 0 (none), then the row.
        z.write_all(&[0]).ok()?;
        z.write_all(samples.get(y * row..(y + 1) * row)?).ok()?;
    }
    let data = z.finish().ok()?;
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    // Bit depth 1, colour type 0 (gray), deflate, filter method 0, no interlacing.
    let ihdr = [&w.to_be_bytes()[..], &h.to_be_bytes(), &[1, 0, 0, 0, 0]].concat();
    for (ty, body) in [(b"IHDR", ihdr.as_slice()), (b"IDAT", data.as_slice()), (b"IEND", &[])] {
        png.extend_from_slice(&u32::try_from(body.len()).ok()?.to_be_bytes());
        png.extend_from_slice(ty);
        png.extend_from_slice(body);
        let mut crc = flate2::Crc::new();
        crc.update(ty);
        crc.update(body);
        png.extend_from_slice(&crc.sum().to_be_bytes());
    }
    Some(png)
}

/// Decode the hex digits in `text` onto `out`, `half` holding a digit that waits for its pair, up
/// to `max` bytes; other characters are skipped.
pub(super) fn push_hex(out: &mut Vec<u8>, half: &mut Option<u8>, text: &[u8], max: usize) {
    for &b in text {
        if out.len() >= max {
            return;
        }
        let Some(v) = char::from(b).to_digit(16) else { continue };
        match half.take() {
            Some(high) => out.push(high << 4 | v as u8),
            None => *half = Some(v as u8),
        }
    }
}

/// The last digit of [`push_hex`]'s, without its pair, as the byte it starts.
pub(super) fn end_hex(out: &mut Vec<u8>, half: Option<u8>, max: usize) {
    if let Some(high) = half
        && out.len() < max
    {
        out.push(high << 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CMYK image is reduced to the size a CMYK TIFF holds, whatever the target.
    #[test]
    fn cmyk_images_come_in_at_most_as_large_as_a_cmyk_tiff() {
        let shape = |channels| Shape { w: 9000, h: 9000, channels, bits: 8, alpha: false };
        let (rgb, cmyk) = with_image_limits(1 << 28, MAX_PIXELS, MAX_BYTES * 4, || {
            (shape(3).step(&Limits::for_channels(3)), shape(4).step(&Limits::for_channels(4)))
        });
        assert_eq!(rgb, Some(1));
        let k = cmyk.unwrap();
        assert_eq!(k, 2);
        assert!(9000u64.div_ceil(k).pow(2) <= vectorcraft_doc::cmyk::MAX_CMYK_PIXELS);
    }

    /// The step keeps each side within 65,535 pixels, and sizes past the limits or overflowing
    /// aren't read.
    #[test]
    fn steps_keep_sides_within_the_limit() {
        let limits = Limits { target: 1 << 28, pixels: MAX_PIXELS, bytes: MAX_BYTES };
        let long = Shape { w: 200_000, h: 1, channels: 1, bits: 1, alpha: false };
        assert_eq!(long.step(&limits), Some(4));
        let huge = Shape { w: u64::MAX, h: 2, channels: 3, bits: 8, alpha: true };
        assert_eq!(huge.step(&limits), None);
        let many = Shape { w: 1 << 16, h: 1 << 16, channels: 1, bits: 1, alpha: false };
        assert_eq!(many.step(&limits), None);
    }

    /// Hex digits are decoded across calls, other characters skipped, up to the bytes asked for.
    #[test]
    fn hex_is_decoded_up_to_the_bytes_asked_for() {
        let (mut out, mut half) = (vec![], None);
        push_hex(&mut out, &mut half, b"F f0 1", 8);
        push_hex(&mut out, &mut half, b"2a", 8);
        end_hex(&mut out, half, 8);
        assert_eq!(out, [0xff, 0x01, 0x2a]);
        let (mut out, mut half) = (vec![], None);
        push_hex(&mut out, &mut half, b"0102030", 2);
        end_hex(&mut out, half, 2);
        assert_eq!(out, [1, 2]);
    }
}
