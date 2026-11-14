//! Raster images whose orientation (EXIF, or a TIFF's Orientation tag) turns their pixels import
//! as they are shown, linked or as data URLs: the pixels turned, the width and height swapped, and
//! the turned image fitted to the `<image>` box.

use std::path::PathBuf;

use vectorcraft_doc::{Document, ImageObject, LinkInfo, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_svg::{ImportOptions, base64_encode, import_with, import_with_report};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">{body}</svg>"#)
}

fn images(d: &Document) -> Vec<(&Node, &ImageObject)> {
    let mut v = Vec::new();
    d.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            v.push((n, &**im));
        }
    });
    v
}

/// The one image `body` imports to, with its warnings, its link read from `dir` when given.
fn import_one(body: &str, dir: Option<&PathBuf>) -> (Document, ImageObject) {
    let read = |path: &str| std::fs::read(path).ok().map(|b| (b, LinkInfo::new(path)));
    let folder = dir.map(|d| d.to_string_lossy().into_owned());
    let (d, w) = match &folder {
        Some(f) => import_with(&svg(body), &ImportOptions { folder: Some(f), read: Some(&read) }).unwrap(),
        None => import_with_report(&svg(body)).unwrap(),
    };
    assert!(w.is_empty(), "{w:?}");
    let im = match &images(&d)[..] {
        [(_, im)] => (*im).clone(),
        other => panic!("{} images", other.len()),
    };
    (d, im)
}

/// A fresh folder for one test's files.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-svg-orientation-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 16 × 8 JPEG, red on the left half, with an EXIF segment giving Orientation `orientation`
/// when there is one.
fn jpeg(orientation: Option<u16>) -> Vec<u8> {
    let rgb = image::RgbImage::from_fn(16, 8, |x, _| if x < 8 { image::Rgb([220, 0, 0]) } else { image::Rgb([255, 255, 255]) });
    let mut jpg = Vec::new();
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
    let Some(orientation) = orientation else { return jpg };
    let o = orientation.to_le_bytes();
    let exif: Vec<u8> =
        [b"Exif\0\0".as_slice(), b"II", &[0x2a, 0, 8, 0, 0, 0], &[1, 0], &[0x12, 0x01, 3, 0, 1, 0, 0, 0, o[0], o[1], 0, 0], &[0, 0, 0, 0]].concat();
    let mut tagged = jpg[..2].to_vec();
    tagged.extend([0xff, 0xe1]);
    tagged.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    tagged.extend(&exif);
    tagged.extend(&jpg[2..]);
    tagged
}

/// A little-endian, uncompressed TIFF of `w` × `h` pixels of `samples` 8-bit samples each (1 for
/// gray, 2 for gray with an unassociated alpha channel, 4 for CMYK inks), `data` row by row, with
/// an Orientation tag (274) of `orientation`. Its RowsPerStrip (278) value is at byte [`ROWS_AT`]
/// and its PlanarConfiguration (284, 1) value at [`PLANAR_AT`].
fn tiff(w: u32, h: u32, samples: u16, data: &[u8], orientation: u16) -> Vec<u8> {
    let entries: u32 = if samples == 2 { 12 } else { 11 };
    let ifd_end = 8 + 2 + entries * 12 + 4;
    let bits_at = ifd_end;
    let data_at = bits_at + if samples > 2 { 2 * u32::from(samples) } else { 0 };
    let entry = |tag: u16, kind: u16, count: u32, value: u32| {
        [&tag.to_le_bytes()[..], &kind.to_le_bytes(), &count.to_le_bytes(), &value.to_le_bytes()].concat()
    };
    // Two SHORTs fit in the entry; more are written after the directory.
    let bits = match samples {
        1 => 8,
        2 => 8 | 8 << 16,
        _ => bits_at,
    };
    let photometric = if samples == 4 { 5 } else { 1 };
    let mut out = [b"II".as_slice(), &[0x2a, 0], &8u32.to_le_bytes(), &u16::try_from(entries).unwrap().to_le_bytes()].concat();
    // ExtraSamples (338): unassociated alpha.
    let extra = (samples == 2).then(|| entry(338, 3, 1, 2));
    for e in [
        entry(256, 4, 1, w),
        entry(257, 4, 1, h),
        entry(258, 3, u32::from(samples), bits),
        entry(259, 3, 1, 1),
        entry(262, 3, 1, photometric),
        entry(273, 4, 1, data_at),
        entry(274, 3, 1, u32::from(orientation)),
        entry(277, 3, 1, u32::from(samples)),
        entry(278, 4, 1, h),
        entry(279, 4, 1, u32::try_from(data.len()).unwrap()),
        entry(284, 3, 1, 1),
    ]
    .into_iter()
    .chain(extra)
    {
        out.extend(e);
    }
    out.extend(0u32.to_le_bytes());
    if samples > 2 {
        for _ in 0..samples {
            out.extend(8u16.to_le_bytes());
        }
    }
    assert_eq!(out.len(), data_at as usize);
    out.extend(data);
    out
}

/// Where [`tiff`] writes the RowsPerStrip value (the ninth entry) and the PlanarConfiguration value
/// (the eleventh).
const ROWS_AT: usize = 8 + 2 + 8 * 12 + 8;
const PLANAR_AT: usize = 8 + 2 + 10 * 12 + 8;

/// A 4 × 2 gray TIFF, black on the left half, turned a quarter clockwise by Orientation 6.
fn gray_tiff() -> Vec<u8> {
    tiff(4, 2, 1, &[0, 0, 255, 255, 0, 0, 255, 255], 6)
}

/// Where the image's own box goes on the page.
fn placed(im: &ImageObject) -> Rect {
    im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)))
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

/// The stored pixels of `im`, as RGB.
fn pixels(d: &Document, im: &ImageObject) -> image::RgbImage {
    image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgb8()
}

fn red(p: &image::Rgb<u8>) -> bool {
    p[0] > 150 && p[1] < 80
}

/// A JPEG whose EXIF Orientation 6 turns it a quarter clockwise shows upright, linked and as a
/// data URL alike: 8 × 16, the left half on top, fitted (centered) in a square box as the turned
/// image is, and stored without an orientation that would turn it again.
#[test]
fn a_turned_jpeg_imports_upright() {
    let jpg = jpeg(Some(6));
    let dir = folder("jpeg");
    std::fs::write(dir.join("photo.jpg"), &jpg).unwrap();
    let data = format!("data:image/jpeg;base64,{}", base64_encode(&jpg));
    let untyped = format!("data:;base64,{}", base64_encode(&jpg));
    for href in ["photo.jpg", data.as_str(), untyped.as_str()] {
        let (d, im) = import_one(&format!(r#"<image href="{href}" x="10" y="20" width="80" height="80"/>"#), Some(&dir));
        let what = &href[..href.len().min(20)];
        assert_eq!((im.width, im.height), (8, 16), "{what}: turned a quarter");
        assert!(near(placed(&im), Rect::new(30.0, 20.0, 70.0, 100.0)), "{what}: fitted as turned: {:?}", placed(&im));
        let px = pixels(&d, &im);
        assert!(red(px.get_pixel(4, 3)) && !red(px.get_pixel(4, 12)), "{what}: the left half is on top");
        assert_eq!(vectorcraft_doc::cmyk::exif_orientation(&d.images[&im.key].bytes), None, "{what}: not turned again");
        assert_eq!(im.link.is_some(), href == "photo.jpg", "{what}");
    }
}

/// With `preserveAspectRatio="none"` the turned image fills the box.
#[test]
fn a_turned_jpeg_stretches_to_the_box_as_turned() {
    let data = format!("data:image/jpeg;base64,{}", base64_encode(&jpeg(Some(6))));
    let (_, im) = import_one(&format!(r#"<image href="{data}" x="10" y="20" width="80" height="40" preserveAspectRatio="none"/>"#), None);
    assert_eq!((im.width, im.height), (8, 16));
    assert!(near(placed(&im), Rect::new(10.0, 20.0, 90.0, 60.0)), "{:?}", placed(&im));
}

/// A JPEG without an orientation, or with Orientation 1, keeps its own bytes.
#[test]
fn an_upright_jpeg_keeps_its_bytes() {
    for jpg in [jpeg(None), jpeg(Some(1))] {
        let data = format!("data:image/jpeg;base64,{}", base64_encode(&jpg));
        let (d, im) = import_one(&format!(r#"<image href="{data}" width="80" height="80"/>"#), None);
        assert_eq!((im.width, im.height), (16, 8));
        assert_eq!(*d.images[&im.key].bytes, jpg);
        assert_eq!(d.images[&im.key].mime, "image/jpeg");
    }
}

/// A TIFF whose Orientation tag is 6 shows upright, linked, as an `image/tiff` data URL and as a
/// data URL without a type.
#[test]
fn a_turned_tiff_imports_upright() {
    let tif = gray_tiff();
    let dir = folder("tiff");
    std::fs::write(dir.join("scan.tif"), &tif).unwrap();
    let data = format!("data:image/tiff;base64,{}", base64_encode(&tif));
    let untyped = format!("data:;base64,{}", base64_encode(&tif));
    for href in ["scan.tif", data.as_str(), untyped.as_str()] {
        let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), Some(&dir));
        let what = &href[..href.len().min(20)];
        assert_eq!((im.width, im.height), (2, 4), "{what}: turned a quarter");
        assert!(near(placed(&im), Rect::new(0.0, 0.0, 20.0, 40.0)), "{what}: {:?}", placed(&im));
        let px = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_luma8();
        assert_eq!((px.get_pixel(0, 0)[0], px.get_pixel(1, 1)[0], px.get_pixel(0, 3)[0]), (0, 0, 255), "{what}: the left half is on top");
    }
}

/// A CMYK TIFF keeps its inks as a CMYK TIFF, turned when its orientation turns it.
#[test]
fn a_cmyk_tiff_keeps_its_inks_turned_or_not() {
    let (c, none) = ([255u8, 0, 0, 0], [0u8; 4]);
    let data: Vec<u8> = [c, c, none, none, c, c, none, none].concat();
    for (orientation, (w, h), cyan_at, none_at) in [(1, (4usize, 2usize), (1, 1), (2, 0)), (6, (2, 4), (1, 1), (0, 2))] {
        let tif = tiff(4, 2, 4, &data, orientation);
        let href = format!("data:image/tiff;base64,{}", base64_encode(&tif));
        let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
        let what = format!("orientation {orientation}");
        assert_eq!((im.width as usize, im.height as usize), (w, h), "{what}");
        let blob = &d.images[&im.key];
        assert_eq!(blob.mime, "image/tiff", "{what}");
        let inks = blob.cmyk().expect("CMYK inks");
        assert_eq!((inks.width as usize, inks.height as usize), (w, h), "{what}");
        let ink = |(x, y): (usize, usize)| &inks.data[(y * w + x) * 4..][..4];
        assert_eq!((ink((0, 0)), ink(cyan_at), ink(none_at)), (&c[..], &c[..], &none[..]), "{what}: cyan on the left, or on top");
    }
}

/// A gray TIFF with an alpha channel, which the image decoder can't read, imports as Open and
/// Place read it, with its transparency: upright, and turned by Orientation 6.
#[test]
fn a_gray_tiff_with_alpha_imports_turned_or_not() {
    let (black, clear) = ([0u8, 255], [255u8, 0]);
    let data: Vec<u8> = [black, black, clear, clear, black, black, clear, clear].concat();
    for (orientation, size, black_at, clear_at) in [(1, (4, 2), (0, 1), (3, 0)), (6, (2, 4), (1, 1), (0, 3))] {
        let href = format!("data:image/tiff;base64,{}", base64_encode(&tiff(4, 2, 2, &data, orientation)));
        let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
        assert_eq!((im.width, im.height), size, "orientation {orientation}");
        let px = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
        assert_eq!(px.get_pixel(black_at.0, black_at.1).0, [0, 0, 0, 255], "orientation {orientation}: black");
        assert_eq!(px.get_pixel(clear_at.0, clear_at.1)[3], 0, "orientation {orientation}: clear");
    }
}

/// A turned TIFF whose header claims more strips than the decoder can count (2^31 + 1 rows a
/// strip each, times 5 planes) is refused without a panic: linked, it is a placeholder with a
/// warning; as a data URL, it is left out.
#[test]
fn a_tiff_with_more_strips_than_can_be_counted_is_refused() {
    let mut tif = tiff(3, 1 << 31 | 1, 5, &[0; 30], 6);
    tif[ROWS_AT..ROWS_AT + 4].copy_from_slice(&1u32.to_le_bytes());
    tif[PLANAR_AT..PLANAR_AT + 2].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(vectorcraft_doc::cmyk::oversized_tiff(&tif), Some((3, 1 << 31 | 1)));
    let dir = folder("oversized");
    std::fs::write(dir.join("huge.tif"), &tif).unwrap();
    let read = |path: &str| std::fs::read(path).ok().map(|b| (b, LinkInfo::new(path)));
    let f = dir.to_string_lossy();
    let (d, w) =
        import_with(&svg(r#"<image href="huge.tif" width="20" height="40"/>"#), &ImportOptions { folder: Some(&f), read: Some(&read) }).unwrap();
    assert!(w.iter().any(|w| w.contains("huge.tif") && w.contains("is not an image VectorCraft reads")), "{w:?}");
    let [(_, im)] = images(&d)[..] else { panic!("{} images", images(&d).len()) };
    assert!(d.images[&im.key].is_proxy(), "a placeholder");
    let data = format!("data:image/tiff;base64,{}", base64_encode(&tif));
    let (d, _) = import_with_report(&svg(&format!(r#"<image href="{data}" width="20" height="40"/>"#))).unwrap();
    assert!(images(&d).is_empty());
}

/// A turned JPEG inside an SVG shown as an image is turned too.
#[test]
fn a_turned_jpeg_in_an_svg_image_imports_upright() {
    let jpg = format!("data:image/jpeg;base64,{}", base64_encode(&jpeg(Some(6))));
    let inner = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><image href="{jpg}" width="80" height="80"/></svg>"#);
    let nested = format!("data:image/svg+xml;base64,{}", base64_encode(inner.as_bytes()));
    let (d, im) = import_one(&format!(r#"<image href="{nested}" width="80" height="80"/>"#), None);
    assert_eq!((im.width, im.height), (8, 16));
    assert!(near(placed(&im), Rect::new(20.0, 0.0, 60.0, 80.0)), "{:?}", placed(&im));
    let px = pixels(&d, &im);
    assert!(red(px.get_pixel(4, 3)) && !red(px.get_pixel(4, 12)));
}

/// A turned JPEG that a linked SVG file links to is turned too.
#[test]
fn a_turned_jpeg_linked_from_a_linked_svg_file_imports_upright() {
    let dir = folder("nested-file");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("photo.jpg"), jpeg(Some(6))).unwrap();
    let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><image href="photo.jpg" width="80" height="80"/></svg>"#;
    std::fs::write(dir.join("sub").join("inner.svg"), inner).unwrap();
    let (d, im) = import_one(r#"<image href="sub/inner.svg" width="80" height="80"/>"#, Some(&dir));
    assert_eq!((im.width, im.height), (8, 16));
    assert!(near(placed(&im), Rect::new(20.0, 0.0, 60.0, 80.0)), "{:?}", placed(&im));
    let px = pixels(&d, &im);
    assert!(red(px.get_pixel(4, 3)) && !red(px.get_pixel(4, 12)));
}
