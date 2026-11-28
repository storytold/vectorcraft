//! Scaling images whose pixels carry straight (not premultiplied) alpha.
//!
//! `image`'s filtered resampling filters each channel on its own, which is correct only for
//! premultiplied pixels, and its thumbnail averages each channel on its own too, so on
//! straight-alpha pixels the color under clear pixels bleeds into the opaque ones next to them
//! (clear black darkens a cut-out's edge). [`resized`], [`fitted`] and [`averaged`] weight each
//! pixel's color by its alpha while scaling, as the streaming shrink in the renderer does.

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};

/// How [`resized`] makes each new pixel from the old ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resample {
    /// `image`'s filtered resampling with this filter.
    Filter(FilterType),
    /// The mean of the old pixels each new one covers ([`averaged`]).
    Average,
}

/// `img` scaled to `w` × `h` pixels as `how` says, each pixel's color weighted by its alpha.
/// Filtered or averaged, an image with clear or partly clear pixels comes back as 8-bit RGBA
/// whatever its own pixel format, and a pixel that ends up wholly clear is transparent black. An
/// image without clear pixels is scaled as `image` scales it, in its own pixel format.
/// Nearest-neighbor picking leaves the pixels as they are, in their own format, and is not
/// weighted. Filtered, the colors keep the precision of premultiplied 8-bit pixels, and an 8-bit
/// RGBA image is premultiplied in place.
pub fn resized(img: DynamicImage, w: u32, h: u32, how: Resample) -> DynamicImage {
    if (w, h) == (img.width(), img.height()) {
        return img;
    }
    let f = match how {
        Resample::Average => return averaged(&img, w, h),
        Resample::Filter(f) => f,
    };
    if f == FilterType::Nearest || opaque(&img) {
        return img.resize_exact(w, h, f);
    }
    let mut rgba = img.into_rgba8();
    for p in rgba.pixels_mut() {
        let [r, g, b, a] = &mut p.0;
        for c in [r, g, b] {
            *c = times_alpha(*c, *a);
        }
    }
    let mut out = image::imageops::resize(&rgba, w, h, f);
    for p in out.pixels_mut() {
        let [r, g, b, a] = &mut p.0;
        for c in [r, g, b] {
            *c = over_alpha(*c, *a);
        }
    }
    DynamicImage::ImageRgba8(out)
}

/// [`resized`] to the largest size that fits `w` × `h` pixels and keeps `img`'s proportions, the
/// size `image`'s own `resize` and `thumbnail` pick.
pub fn fitted(img: DynamicImage, w: u32, h: u32, how: Resample) -> DynamicImage {
    let (nw, nh) = fit_size(img.width(), img.height(), w, h);
    resized(img, nw, nh, how)
}

/// `img` scaled to `w` × `h` pixels, each new pixel the mean of the old pixels it covers, each old
/// pixel counted by the area of it covered and its color weighted by its alpha. `img` is read one
/// row at a time, so besides the result this takes one row of 8-bit RGBA at most. An image
/// without clear pixels is scaled as `image`'s thumbnail scales it.
pub fn averaged(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let (sw, sh) = (img.width(), img.height());
    if opaque(img) || w == 0 || h == 0 || sw == 0 || sh == 0 {
        return img.thumbnail_exact(w, h);
    }
    // Per new pixel of the row being made: red, green and blue times alpha, alpha (all times the
    // area covered), and the area covered.
    let mut sums = vec![([0f64; 4], 0f64); w as usize];
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for ty in 0..h {
        for (y, wy) in covered(ty, sh, h) {
            let row = rgba_row(img, y as u32);
            for ((s, area), tx) in sums.iter_mut().zip(0..w) {
                for (x, wx) in covered(tx, sw, w) {
                    let Some(p) = row.get(x * 4..x * 4 + 4) else { continue };
                    let k = wx * wy;
                    let a = f64::from(p[3]) * k;
                    s[0] += f64::from(p[0]) * a;
                    s[1] += f64::from(p[1]) * a;
                    s[2] += f64::from(p[2]) * a;
                    s[3] += a;
                    *area += k;
                }
            }
        }
        let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
        for (s, area) in &mut sums {
            out.extend(if s[3] <= 0.0 { [0; 4] } else { [byte(s[0] / s[3]), byte(s[1] / s[3]), byte(s[2] / s[3]), byte(s[3] / *area)] });
            (*s, *area) = ([0.0; 4], 0.0);
        }
    }
    RgbaImage::from_raw(w, h, out).map_or_else(|| img.thumbnail_exact(w, h), DynamicImage::ImageRgba8)
}

/// Row `y` of `img` as 8-bit RGBA: borrowed from an 8-bit RGBA image, converted from others.
fn rgba_row(img: &DynamicImage, y: u32) -> std::borrow::Cow<'_, [u8]> {
    let n = img.width() as usize * 4;
    match img {
        DynamicImage::ImageRgba8(b) => std::borrow::Cow::Borrowed(b.as_raw().get(y as usize * n..(y as usize + 1) * n).unwrap_or_default()),
        other => std::borrow::Cow::Owned(other.crop_imm(0, y, other.width(), 1).into_rgba8().into_raw()),
    }
}

/// The old pixels that new pixel `t` of `tn` covers along a side of `n` old pixels, each with the
/// length of it covered (in old pixels), worked out as they are read so that nothing is stored.
fn covered(t: u32, n: u32, tn: u32) -> impl Iterator<Item = (usize, f64)> {
    let scale = f64::from(n) / f64::from(tn);
    let (a, b) = (f64::from(t) * scale, (f64::from(t) + 1.0) * scale);
    let (first, end) = (a.floor() as u32, (b.ceil() as u32).min(n));
    (first..end.max(first + 1).min(n)).filter_map(move |i| {
        let len = b.min(f64::from(i) + 1.0) - a.max(f64::from(i));
        (len > 0.0).then_some((i as usize, len))
    })
}

/// Each 4-channel pixel of `px` with all four channels multiplied by its value in `alpha` (one
/// sample a pixel, 0 to 255): ink amounts or colors made ready for scaling beside a separate alpha
/// channel. Pixels past the end of `alpha` are left as they are.
pub fn premultiply(px: &mut [u8], alpha: &[u8]) {
    for (p, &a) in px.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
        for c in p {
            *c = times_alpha(*c, a);
        }
    }
}

/// [`premultiply`] undone after scaling: each 4-channel pixel of `px` divided by its value in the
/// scaled `alpha`, and 0 where that is 0.
pub fn unpremultiply(px: &mut [u8], alpha: &[u8]) {
    for (p, &a) in px.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
        for c in p {
            *c = over_alpha(*c, a);
        }
    }
}

/// Does `img` have no alpha channel, or is every pixel of it opaque?
fn opaque(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageRgba8(b) => b.pixels().all(|p| p[3] == u8::MAX),
        DynamicImage::ImageLumaA8(b) => b.pixels().all(|p| p[1] == u8::MAX),
        DynamicImage::ImageRgba16(b) => b.pixels().all(|p| p[3] == u16::MAX),
        DynamicImage::ImageLumaA16(b) => b.pixels().all(|p| p[1] == u16::MAX),
        DynamicImage::ImageRgba32F(b) => b.pixels().all(|p| p[3] >= 1.0),
        other => !other.color().has_alpha(),
    }
}

/// Channel `c` times alpha `a`, both 0 to 255.
fn times_alpha(c: u8, a: u8) -> u8 {
    ((u16::from(c) * u16::from(a) + 127) / 255) as u8
}

/// Premultiplied channel `c` divided by alpha `a` (0 when `a` is 0).
fn over_alpha(c: u8, a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let a = u32::from(a);
    ((u32::from(c) * 255 + a / 2) / a).min(255) as u8
}

/// The largest size of at most `nw` × `nh` with the proportions of `w` × `h`, at least a pixel a
/// side.
pub fn fit_size(w: u32, h: u32, nw: u32, nh: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (nw.max(1), nh.max(1));
    }
    let ratio = (f64::from(nw) / f64::from(w)).min(f64::from(nh) / f64::from(h));
    let side = |n: u32| ((f64::from(n) * ratio).round() as u32).max(1);
    (side(w), side(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 × 2 image whose left half is opaque red and whose right half is clear black.
    fn half_clear() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) }))
    }

    /// Is the color of `p`, premultiplied by its alpha, within a step of `rgb` premultiplied by the
    /// same alpha (the precision of premultiplied 8-bit pixels)?
    fn close(p: [u8; 4], rgb: [u8; 3]) -> bool {
        p[..3].iter().zip(rgb).all(|(&c, want)| times_alpha(c, p[3]).abs_diff(times_alpha(want, p[3])) <= 1)
    }

    #[test]
    fn averaging_keeps_the_color_of_the_opaque_pixels() {
        let p = resized(half_clear(), 1, 1, Resample::Average).into_rgba8().get_pixel(0, 0).0;
        assert!(close(p, [200, 30, 30]) && p[3].abs_diff(128) <= 1, "{p:?}");
    }

    /// Three pixels averaged into two: each new pixel takes one old pixel whole and half of the
    /// clear one in the middle.
    #[test]
    fn averaging_counts_the_part_of_a_pixel_covered() {
        let img = RgbaImage::from_fn(3, 1, |x, _| if x == 1 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([200, 30, 30, 255]) });
        let out = averaged(&DynamicImage::ImageRgba8(img), 2, 1).into_rgba8();
        assert_eq!(out.into_raw(), [[200, 30, 30, 170], [200, 30, 30, 170]].concat());
        // Gray with alpha is read the same way.
        let gray = image::GrayAlphaImage::from_fn(3, 1, |x, _| image::LumaA(if x == 1 { [0, 0] } else { [200, 255] }));
        let out = averaged(&DynamicImage::ImageLumaA8(gray), 2, 1).into_rgba8();
        assert_eq!(out.into_raw(), [[200, 200, 200, 170], [200, 200, 200, 170]].concat());
    }

    #[test]
    fn filtering_keeps_the_color_of_the_opaque_pixels() {
        for f in [FilterType::Triangle, FilterType::CatmullRom, FilterType::Lanczos3] {
            let out = resized(half_clear(), 2, 1, Resample::Filter(f)).into_rgba8();
            for x in 0..2 {
                let p = out.get_pixel(x, 0).0;
                assert!(p[3] == 0 || close(p, [200, 30, 30]), "{f:?} pixel {x}: {p:?}");
            }
        }
    }

    #[test]
    fn fitted_keeps_proportions() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(400, 100));
        let out = fitted(img, 64, 64, Resample::Average);
        assert_eq!((out.width(), out.height()), (64, 16));
    }

    #[test]
    fn opaque_and_alpha_free_images_scale_as_image_scales_them() {
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::from_fn(5, 3, |x, y| image::Rgb([x as u8 * 40, y as u8 * 60, 7])));
        let ours = resized(rgb.clone(), 2, 2, Resample::Filter(FilterType::Triangle));
        assert_eq!(ours, rgb.resize_exact(2, 2, FilterType::Triangle));
        let opaque = DynamicImage::ImageRgba8(rgb.to_rgba8());
        assert_eq!(resized(opaque.clone(), 2, 2, Resample::Average), opaque.thumbnail_exact(2, 2));
    }

    #[test]
    fn a_wholly_clear_result_is_transparent_black() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 0])));
        assert_eq!(resized(img, 1, 1, Resample::Average).into_rgba8().get_pixel(0, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn premultiply_and_unpremultiply_scale_beside_a_separate_alpha() {
        // Two pixels of inks, the second clear: averaged, the first one's inks remain.
        let mut px = vec![40, 80, 120, 160, 0, 0, 0, 0];
        premultiply(&mut px, &[255, 0]);
        let img = RgbaImage::from_raw(2, 1, px).unwrap();
        let mut out = image::imageops::thumbnail(&img, 1, 1).into_raw();
        let alpha = image::imageops::thumbnail(&image::GrayImage::from_raw(2, 1, vec![255, 0]).unwrap(), 1, 1).into_raw();
        unpremultiply(&mut out, &alpha);
        let want = [40, 80, 120, 160];
        assert!(out.iter().zip(want).all(|(&c, w)| times_alpha(c, alpha[0]).abs_diff(times_alpha(w, alpha[0])) <= 1), "{out:?} {alpha:?}");
    }
}
