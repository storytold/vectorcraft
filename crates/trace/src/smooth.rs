//! Edge-preserving smoothing of continuous-tone images before they are quantised.
//!
//! A photo's tones run into each other, and its noise is as large as the step from one palette
//! colour to the next: quantised as it is, noise decides pixel by pixel which of two neighbouring
//! colours a pixel gets, and the trace is tens of thousands of specks. Each pixel is averaged with
//! its neighbours instead, each weighted by how alike it is (a bilateral filter: C. Tomasi and
//! R. Manduchi, "Bilateral Filtering for Gray and Color Images", 1998), so that noise within a tone
//! is averaged away while an edge between two tones stays where and as sharp as it was. Three
//! rounds, each along rows and then along columns, three pixels at a time.

use crate::{Raster, parallel_map};

/// Colours this far apart (levels, all channels together) count half as much as equal ones.
const ALIKE: f64 = 20.0;

/// One pass along rows (`dx = 1`) or columns (`dx = 0`).
fn pass(src: &[u8], w: usize, h: usize, along_rows: bool) -> Vec<u8> {
    let rows: Vec<usize> = (0..h).collect();
    let px = |x: usize, y: usize| src.get((y * w + x) * 4..(y * w + x) * 4 + 4);
    let done = parallel_map(&rows, |&y| {
        let mut out = Vec::with_capacity(w * 4);
        for x in 0..w {
            let Some(p) = px(x, y) else { continue };
            if p[3] < 128 {
                out.extend_from_slice(p);
                continue;
            }
            let near = if along_rows {
                [(x > 0).then(|| (x - 1, y)), (x + 1 < w).then(|| (x + 1, y))]
            } else {
                [(y > 0).then(|| (x, y - 1)), (y + 1 < h).then(|| (x, y + 1))]
            };
            let (mut sum, mut weight) = ([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], 1.0);
            for q in near.into_iter().flatten().filter_map(|(x, y)| px(x, y)).filter(|q| q[3] >= 128) {
                let d2: f64 = (0..3).map(|k| (f64::from(q[k]) - f64::from(p[k])).powi(2)).sum();
                let wt = 1.0 / (1.0 + d2 / (ALIKE * ALIKE)).powi(2);
                for k in 0..3 {
                    sum[k] += wt * f64::from(q[k]);
                }
                weight += wt;
            }
            out.extend(sum.map(|v| (v / weight).round() as u8));
            out.push(p[3]);
        }
        out
    });
    done.concat()
}

/// `img` with the noise within its tones smoothed away and its edges kept.
pub(crate) fn smoothed(img: &Raster) -> Raster {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgba = img.rgba.clone();
    if rgba.len() == w * h * 4 {
        for _ in 0..3 {
            rgba = pass(&pass(&rgba, w, h, true), w, h, false);
        }
    }
    Raster { width: img.width, height: img.height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_goes_and_an_edge_stays() {
        // Left half about 60, right half about 160, with noise of ±6 on both.
        let img = Raster::from_fn(40, 20, |x, y| {
            let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
            n ^= n >> 15;
            n = n.wrapping_mul(0x2C1B_3C6D);
            let n = ((n >> 12) % 13) as u8;
            let v = if x < 20 { 54 + n } else { 154 + n };
            [v, v, v, 255]
        });
        let out = smoothed(&img);
        let spread = |r: &Raster, x0: u32, x1: u32| {
            let v: Vec<u8> = (0..20).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| r.pixel(x, y)[0]).collect();
            v.iter().max().unwrap() - v.iter().min().unwrap()
        };
        assert_eq!(spread(&img, 0, 20), 12);
        assert!(spread(&out, 0, 20) <= 6 && spread(&out, 20, 40) <= 6, "{} {}", spread(&out, 0, 20), spread(&out, 20, 40));
        // The edge is still one pixel sharp, where it was.
        assert!(out.pixel(19, 10)[0] < 70 && out.pixel(20, 10)[0] > 150);
    }

    #[test]
    fn transparent_pixels_and_odd_sizes_pass_through() {
        let img = Raster::from_fn(3, 1, |x, _| [10 * x as u8, 0, 0, if x == 1 { 0 } else { 255 }]);
        let out = smoothed(&img);
        assert_eq!(out.pixel(1, 0), [10, 0, 0, 0]);
        assert_eq!((out.width, out.height, out.rgba.len()), (3, 1, 12));
        let broken = Raster { width: 4, height: 4, rgba: vec![0; 7] };
        assert_eq!(smoothed(&broken).rgba.len(), 7);
    }
}
