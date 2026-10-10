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
    let signal = |x: isize, y: isize| {
        let (rgb, alpha) = pixel(x, y);
        (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) * alpha
    };
    let mut edges = vec![0.0f32; px.len()];
    let mut colours: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0; px.len()]);
    for y in 0..h {
        for x in 0..w {
            let x = x as isize;
            let y = y as isize;
            let gx = -signal(x - 1, y - 1) + signal(x + 1, y - 1) - 2.0 * signal(x - 1, y) + 2.0 * signal(x + 1, y) - signal(x - 1, y + 1)
                + signal(x + 1, y + 1);
            let gy = -signal(x - 1, y - 1) - 2.0 * signal(x, y - 1) - signal(x + 1, y - 1)
                + signal(x - 1, y + 1)
                + 2.0 * signal(x, y + 1)
                + signal(x + 1, y + 1);
            let alpha_signal = |x, y| pixel(x, y).1;
            let agx = -alpha_signal(x - 1, y - 1) + alpha_signal(x + 1, y - 1) - 2.0 * alpha_signal(x - 1, y) + 2.0 * alpha_signal(x + 1, y)
                - alpha_signal(x - 1, y + 1)
                + alpha_signal(x + 1, y + 1);
            let agy = -alpha_signal(x - 1, y - 1) - 2.0 * alpha_signal(x, y - 1) - alpha_signal(x + 1, y - 1)
                + alpha_signal(x - 1, y + 1)
                + 2.0 * alpha_signal(x, y + 1)
                + alpha_signal(x + 1, y + 1);
            let index = y as usize * w + x as usize;
            let edge = gx.hypot(gy).max(agx.hypot(agy)).clamp(0.0, 1.0);
            if let Some(value) = edges.get_mut(index) {
                *value = edge;
                let (rgb, alpha) = pixel(x, y);
                for channel in 0..3 {
                    colours[channel][index] = rgb[channel] * alpha * edge;
                }
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
