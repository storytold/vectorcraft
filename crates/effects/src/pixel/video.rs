//! Effect › Video: De-Interlace and NTSC Colors.

use vectorcraft_geom::Point;

use super::PixelSpace;

/// The tallest field line De-Interlace reads across (document units): one pixel at the lowest
/// document raster resolution, 1 ppi.
pub(super) const MAX_LINE: f64 = 72.0;

/// De-Interlace: the odd (or, with `even`, the even) fields of the image are made again from the
/// others. The fields are the lines of the document's raster grid (each [`PixelSpace::line`]
/// tall, numbered from 1 at the top of the page): a line taken out becomes a copy of the line
/// above it (the one below at the top), or with `interpolate` the average of the two.
pub(super) fn deinterlace(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, even: bool, interpolate: bool) {
    let line = space.line;
    let to_px = space.to_doc.inverse();
    if !(line.is_finite() && line > 0.0 && to_px.is_finite() && space.to_doc.is_finite()) {
        return;
    }
    let src = px.to_vec();
    // The pixel holding document point `q`.
    let at = |q: Point| {
        let p = to_px * q;
        (p.x >= 0.0 && p.y >= 0.0 && p.x < w as f64 && p.y < h as f64).then(|| src.get(p.y as usize * w + p.x as usize).copied()).flatten()
    };
    for (i, out) in px.iter_mut().enumerate().take(w.saturating_mul(h)) {
        let q = space.to_doc * Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        let k = (q.y / line).floor();
        // Line k + 1: odd lines have an even k.
        if !k.is_finite() || ((k as i64).rem_euclid(2) == 1) != even {
            continue;
        }
        let (above, below) = (at(Point::new(q.x, q.y - line)), at(Point::new(q.x, q.y + line)));
        let made = match (interpolate, above, below) {
            (true, Some(a), Some(b)) => Some([0, 1, 2, 3].map(|c| (u16::from(a[c]) + u16::from(b[c])).div_ceil(2) as u8)),
            (_, a, b) => a.or(b),
        };
        if let Some(v) = made {
            *out = v;
        }
    }
}

/// NTSC Colors: colours a television's composite signal can't carry are made less saturated,
/// keeping their brightness. In YIQ, the luma plus the chroma's amplitude must stay within 110 %
/// of white and the luma minus it above −20 %; a colour past either has its chroma cut to fit.
pub(super) fn ntsc(px: &mut [[u8; 4]]) {
    for p in px.iter_mut() {
        let a = f32::from(p[3]) / 255.0;
        if a <= 0.0 {
            continue;
        }
        // Straight colour (the pixels are premultiplied).
        let [r, g, b] = [p[0], p[1], p[2]].map(|v| (f32::from(v) / 255.0 / a).min(1.0));
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        let (i, q) = (0.596 * r - 0.274 * g - 0.322 * b, 0.211 * r - 0.523 * g + 0.312 * b);
        let c = i.hypot(q);
        let mut keep = 1.0f32;
        if y + c > 1.1 {
            keep = keep.min((1.1 - y) / c);
        }
        if y - c < -0.2 {
            keep = keep.min((y + 0.2) / c);
        }
        if keep >= 1.0 || !keep.is_finite() {
            continue;
        }
        let (i, q) = (i * keep.max(0.0), q * keep.max(0.0));
        let rgb = [y + 0.956 * i + 0.621 * q, y - 0.272 * i - 0.647 * q, y - 1.106 * i + 1.703 * q];
        let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * a * 255.0).round() as u8);
        *p = [r, g, b, p[3]];
    }
}
