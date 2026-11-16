//! `Rendered::area_resized`: area averages of premultiplied pixels at any size, edge pixels by
//! their share, a copy at the same size, and errors instead of malformed images.

use crate::*;

fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> Rendered {
    Rendered { width, height, pixels: pixels.concat() }
}

#[test]
fn whole_blocks_average_into_one_pixel() {
    let a = [200, 100, 0, 255];
    let b = [0, 100, 200, 255];
    let img = image(4, 2, &[a, b, a, a, a, b, a, b]);
    let out = img.area_resized(2, 1).unwrap();
    assert_eq!((out.width, out.height), (2, 1));
    assert_eq!(out.pixels, [100, 100, 100, 255, 150, 100, 50, 255]);
}

/// Three pixels into two: each new pixel takes one whole old pixel and half of the middle one.
#[test]
fn edge_pixels_count_by_the_share_they_cover() {
    let img = image(3, 1, &[[0, 0, 0, 255], [255, 255, 255, 255], [90, 90, 90, 255]]);
    let out = img.area_resized(2, 1).unwrap();
    // (0 + 255 / 2) / 1.5 = 85, (255 / 2 + 90) / 1.5 = 145.
    assert_eq!(out.pixels, [85, 85, 85, 255, 145, 145, 145, 255]);
}

/// A transparent pixel carries no color into its opaque neighbor: the average is half red, half
/// clear, still pure red once un-premultiplied.
#[test]
fn transparent_pixels_add_no_color() {
    let img = image(2, 1, &[[0, 0, 0, 0], [255, 0, 0, 255]]);
    let out = img.area_resized(1, 1).unwrap();
    assert_eq!(out.pixels, [128, 0, 0, 128]);
    assert_eq!(out.to_straight(), [255, 0, 0, 128]);
}

#[test]
fn the_same_size_is_a_copy() {
    let px: Vec<[u8; 4]> = (0..12u8).map(|i| [i * 20, 255 - i * 20, i, 255]).collect();
    let img = image(4, 3, &px);
    let out = img.area_resized(4, 3).unwrap();
    assert_eq!(out.pixels, img.pixels);
}

/// A solid image stays exactly its color at any smaller size, with no rounding drift at
/// fractional spans.
#[test]
fn a_solid_color_stays_exact() {
    let px = vec![[12, 34, 56, 255]; 1000 * 7];
    let out = image(1000, 7, &px).area_resized(128, 3).unwrap();
    assert_eq!((out.width, out.height), (128, 3));
    assert!(out.pixels.as_chunks::<4>().0.iter().all(|p| *p == [12, 34, 56, 255]));
}

#[test]
fn bad_sizes_are_errors() {
    let img = image(2, 2, &[[0, 0, 0, 255]; 4]);
    assert!(img.area_resized(0, 1).is_err());
    assert!(img.area_resized(1, 0).is_err());
    let short = Rendered { width: 2, height: 2, pixels: vec![0; 12] };
    assert!(short.area_resized(1, 1).err().is_some_and(|e| e.contains("doesn't match")));
    let empty = Rendered { width: 0, height: 5, pixels: Vec::new() };
    assert!(empty.area_resized(1, 1).is_err());
}
