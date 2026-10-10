//! Stylize › Glowing Edges.

use super::blur_plane;

/// Find Sobel edges, smooth and expand them, then draw coloured outlines on black. Samples outside
/// the raster are transparent.
pub(super) fn glow(px: &mut [[u8; 4]], w: usize, h: usize, width: f64, brightness: f64, smoothness: f64) {
    if w == 0 || h == 0 || w.checked_mul(h) != Some(px.len()) {
        return;
    }
    let src = px.to_vec();
    let pixel = |x: isize, y: isize| -> ([f32; 3], f32) {
        // Beyond the raster's edges is transparency (not the next row's first pixel).
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return ([0.0; 3], 0.0) };
        if x >= w || y >= h {
            return ([0.0; 3], 0.0);
        }
        let Some(p) = src.get(y * w + x) else { return ([0.0; 3], 0.0) };
        let alpha = f32::from(p[3]) / 255.0;
        let straight =
            if alpha > 0.0 { [f32::from(p[0]) / 255.0 / alpha, f32::from(p[1]) / 255.0 / alpha, f32::from(p[2]) / 255.0 / alpha] } else { [0.0; 3] };
        (straight, alpha)
    };
    let signal: Vec<f32> = (0..px.len())
        .map(|i| {
            let (rgb, alpha) = pixel((i % w) as isize, (i / w) as isize);
            (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) * alpha
        })
        .collect();
    let coverage: Vec<f32> = src.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let mut edges: Vec<f32> = sobel(&signal, w, h).iter().zip(sobel(&coverage, w, h)).map(|(s, a)| s.max(a).clamp(0.0, 1.0)).collect();
    let mut colours: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0; px.len()]);
    for (index, edge) in edges.iter().enumerate() {
        let (rgb, alpha) = pixel((index % w) as isize, (index / w) as isize);
        for (colour, v) in colours.iter_mut().zip(rgb) {
            if let Some(c) = colour.get_mut(index) {
                *c = v * alpha * edge;
            }
        }
    }
    blur_plane(&mut edges, w, h, smoothness.max(0.0));
    if width > 0.5 {
        blur_plane(&mut edges, w, h, width * 0.5);
    }
    let peak = edges.iter().copied().fold(0.0f32, f32::max);
    if peak > 0.0 {
        for colour in &mut colours {
            blur_plane(colour, w, h, smoothness.max(0.0));
            if width > 0.5 {
                blur_plane(colour, w, h, width * 0.5);
            }
        }
    }
    for (index, (out, (original, edge))) in px.iter_mut().zip(src.iter().zip(edges)).enumerate() {
        let alpha = f32::from(original[3]) / 255.0;
        let glow = if peak > 0.0 { (edge / peak * brightness as f32 / 6.0).clamp(0.0, 1.0) } else { 0.0 };
        let colour_weight = colours[0][index].max(colours[1][index]).max(colours[2][index]);
        let straight = if colour_weight > 1e-6 { std::array::from_fn(|channel| colours[channel][index] / colour_weight) } else { [0.0; 3] };
        // Premultiplied: the glow is light of the edge's colour, so outside the shape it keeps that
        // colour at the glow's opacity instead of darkening as it fades.
        let a = alpha.max(glow);
        *out = [
            (straight[0] * glow * 255.0).round().clamp(0.0, 255.0) as u8,
            (straight[1] * glow * 255.0).round().clamp(0.0, 255.0) as u8,
            (straight[2] * glow * 255.0).round().clamp(0.0, 255.0) as u8,
            (a * 255.0).round() as u8,
        ];
    }
}

/// The Sobel gradient's length over a `w` × `h` plane of values (zero beyond its edges): 8 × the
/// slope per pixel on a ramp, 4 × the step at a sharp edge. Empty when the sizes don't match.
pub(super) fn sobel(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    if w == 0 || w.checked_mul(h) != Some(v.len()) {
        return Vec::new();
    }
    let at = |x: usize, dx: isize, y: usize, dy: isize| -> f32 {
        match (x.checked_add_signed(dx), y.checked_add_signed(dy)) {
            (Some(x), Some(y)) if x < w && y < h => v.get(y * w + x).copied().unwrap_or(0.0),
            _ => 0.0,
        }
    };
    (0..v.len())
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let gx = at(x, 1, y, -1) - at(x, -1, y, -1) + 2.0 * (at(x, 1, y, 0) - at(x, -1, y, 0)) + at(x, 1, y, 1) - at(x, -1, y, 1);
            let gy = at(x, -1, y, 1) - at(x, -1, y, -1) + 2.0 * (at(x, 0, y, 1) - at(x, 0, y, -1)) + at(x, 1, y, 1) - at(x, 1, y, -1);
            gx.hypot(gy)
        })
        .collect()
}
