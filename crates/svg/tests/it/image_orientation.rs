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
    let mut rgb = Vec::with_capacity(16 * 8 * 3);
    for _ in 0..8 {
        for x in 0..16 {
            rgb.extend_from_slice(&if x < 8 { [220, 0, 0] } else { [255, 255, 255] });
        }
    }
    let raw = craft_image_codecs::Image::from_raw(16, 8, craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, rgb).unwrap();
    let jpg = craft_image_codecs::encode(&raw, craft_image_codecs::Format::Jpeg, &craft_image_codecs::EncodeOptions::default()).unwrap();
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
    tiff_at(w, h, samples, data, orientation, None)
}

/// [`tiff`], declaring `ppi` (horizontal, vertical pixels per inch) when given, with XResolution
/// (282), YResolution (283) and ResolutionUnit (296, inches).
fn tiff_at(w: u32, h: u32, samples: u16, data: &[u8], orientation: u16, ppi: Option<(u32, u32)>) -> Vec<u8> {
    let entries = 11 + u32::from(samples == 2) + 3 * u32::from(ppi.is_some());
    let ifd_end = 8 + 2 + entries * 12 + 4;
    let bits_at = ifd_end;
    let ppi_at = bits_at + if samples > 2 { 2 * u32::from(samples) } else { 0 };
    let data_at = ppi_at + if ppi.is_some() { 16 } else { 0 };
    let entry = |tag: u16, kind: u16, count: u32, value: u32| {
        [&tag.to_le_bytes()[..], &kind.to_le_bytes(), &count.to_le_bytes(), &value.to_le_bytes()].concat()
    };
    let resolution = |tags: [(u16, u16, u32); 2]| ppi.map(|_| tags.map(|(tag, kind, value)| entry(tag, kind, 1, value))).into_iter().flatten();
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
    ]
    .into_iter()
    .chain(resolution([(282, 5, ppi_at), (283, 5, ppi_at + 8)]))
    .chain([entry(284, 3, 1, 1)])
    .chain(ppi.map(|_| entry(296, 3, 1, 2)))
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
    if let Some((x, y)) = ppi {
        for v in [x, 1, y, 1] {
            out.extend(v.to_le_bytes());
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
    let decoded = craft_image_codecs::decode(&d.images[&im.key].bytes).unwrap();
    let rgba = image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap();
    image::DynamicImage::ImageRgba8(rgba).into_rgb8()
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
        let decoded = craft_image_codecs::decode(&d.images[&im.key].bytes).unwrap();
        let px =
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap()).into_luma8();
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
        let decoded = craft_image_codecs::decode(&d.images[&im.key].bytes).unwrap();
        let px = image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap();
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

/// [`jpeg`] with its JFIF segment declaring `x` × `y` pixels per inch.
fn jpeg_at(orientation: Option<u16>, (x, y): (u16, u16)) -> Vec<u8> {
    let mut jpg = jpeg(orientation);
    let jfif = jpg.windows(5).position(|w| w == b"JFIF\0").unwrap();
    jpg[jfif + 7] = 1;
    jpg[jfif + 8..jfif + 10].copy_from_slice(&x.to_be_bytes());
    jpg[jfif + 10..jfif + 12].copy_from_slice(&y.to_be_bytes());
    jpg
}

/// The resolution the stored image of `im` declares.
fn stored_ppi(d: &Document, im: &ImageObject) -> Option<(f64, f64)> {
    vectorcraft_doc::ppi::resolution(&d.images[&im.key].bytes)
}

/// A raster that import writes again as PNG keeps the resolution its file declares, linked, as a
/// data URL and inside an SVG shown as an image: its axes swapped when the image turns a quarter,
/// in their order when it turns a half or is flipped.
#[test]
fn a_turned_raster_keeps_its_resolution() {
    let dir = folder("ppi");
    for (orientation, ppi) in [(6, (150.0, 300.0)), (8, (150.0, 300.0)), (3, (300.0, 150.0)), (2, (300.0, 150.0))] {
        let jpg = jpeg_at(Some(orientation), (300, 150));
        std::fs::write(dir.join("photo.jpg"), &jpg).unwrap();
        let data = format!("data:image/jpeg;base64,{}", base64_encode(&jpg));
        let inner = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><image href="{data}" width="80" height="80"/></svg>"#);
        let nested = format!("data:image/svg+xml;base64,{}", base64_encode(inner.as_bytes()));
        for href in ["photo.jpg", data.as_str(), nested.as_str()] {
            let (d, im) = import_one(&format!(r#"<image href="{href}" width="80" height="80"/>"#), Some(&dir));
            let what = format!("orientation {orientation}, {}", &href[..href.len().min(20)]);
            assert_eq!(d.images[&im.key].mime, "image/png", "{what}");
            assert_eq!(stored_ppi(&d, &im), Some(ppi), "{what}");
        }
    }
    let href = format!("data:image/tiff;base64,{}", base64_encode(&tiff_at(4, 2, 1, &[0, 0, 255, 255, 0, 0, 255, 255], 6, Some((300, 150)))));
    let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
    assert_eq!((im.width, im.height), (2, 4));
    assert_eq!(stored_ppi(&d, &im), Some((150.0, 300.0)), "a turned TIFF");
}

/// A TIFF that is not turned is written as PNG with the resolution it declares; a raster that
/// declares none is stored without one.
#[test]
fn an_upright_tiff_keeps_its_resolution() {
    let gray = [0, 0, 255, 255, 0, 0, 255, 255];
    let href = format!("data:image/tiff;base64,{}", base64_encode(&tiff_at(4, 2, 1, &gray, 1, Some((300, 150)))));
    let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
    assert_eq!(d.images[&im.key].mime, "image/png");
    assert_eq!(stored_ppi(&d, &im), Some((300.0, 150.0)));
    for bytes in [tiff(4, 2, 1, &gray, 1), jpeg(Some(6))] {
        let href = format!("data:;base64,{}", base64_encode(&bytes));
        let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
        assert_eq!(stored_ppi(&d, &im), None, "no resolution is made up");
    }
}

/// A CMYK TIFF stored with its inks keeps its resolution, swapped when it turns a quarter.
#[test]
fn a_cmyk_tiff_keeps_its_resolution_turned_or_not() {
    let (c, none) = ([255u8, 0, 0, 0], [0u8; 4]);
    let data: Vec<u8> = [c, c, none, none, c, c, none, none].concat();
    for (orientation, ppi) in [(1, (300.0, 150.0)), (6, (150.0, 300.0))] {
        let href = format!("data:image/tiff;base64,{}", base64_encode(&tiff_at(4, 2, 4, &data, orientation, Some((300, 150)))));
        let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), None);
        assert_eq!(d.images[&im.key].mime, "image/tiff", "orientation {orientation}");
        assert!(d.images[&im.key].cmyk().is_some(), "orientation {orientation}: CMYK inks");
        assert_eq!(stored_ppi(&d, &im), Some(ppi), "orientation {orientation}");
    }
}

/// A made-up ICC profile of `len` bytes (at least a header's 128) whose header names the color
/// space `space`; the bytes after the header differ from place to place.
fn icc_profile(space: &[u8; 4], len: usize) -> Vec<u8> {
    let mut icc: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    icc[..4].copy_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(space);
    icc[36..40].copy_from_slice(b"acsp");
    icc
}

/// `tiff` (little-endian) with an ICCProfile tag (34675, of type UNDEFINED) holding `icc`: its first
/// directory copied to the end of the file with the tag added in order.
fn tiff_with_icc(tiff: &[u8], icc: &[u8]) -> Vec<u8> {
    let at = usize::try_from(u32::from_le_bytes(tiff[4..8].try_into().unwrap())).unwrap();
    let n = usize::from(u16::from_le_bytes([tiff[at], tiff[at + 1]]));
    let mut entries: Vec<Vec<u8>> = tiff[at + 2..at + 2 + 12 * n].chunks(12).map(<[u8]>::to_vec).collect();
    let mut out = tiff.to_vec();
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let data = u32::try_from(out.len()).unwrap();
    out.extend(icc);
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let i = entries.iter().position(|e| u16::from_le_bytes([e[0], e[1]]) > 34675).unwrap_or(n);
    let count = u32::try_from(icc.len()).unwrap();
    entries.insert(i, [&34675u16.to_le_bytes()[..], &7u16.to_le_bytes(), &count.to_le_bytes(), &data.to_le_bytes()].concat());
    let ifd = u32::try_from(out.len()).unwrap();
    out[4..8].copy_from_slice(&ifd.to_le_bytes());
    out.extend(u16::try_from(n + 1).unwrap().to_le_bytes());
    out.extend(entries.concat());
    out.extend([0; 4]);
    out
}

/// A little-endian EXIF block (a TIFF header and one directory) giving Orientation `orientation`.
fn exif_block(orientation: u16) -> Vec<u8> {
    let o = orientation.to_le_bytes();
    [b"II".as_slice(), &[0x2a, 0, 8, 0, 0, 0], &[1, 0], &[0x12, 0x01, 3, 0, 1, 0, 0, 0, o[0], o[1], 0, 0], &[0, 0, 0, 0]].concat()
}

/// The ICC profile and the bits per sample of the PNG `im` is stored as.
fn stored_icc(d: &Document, im: &ImageObject) -> (Option<Vec<u8>>, u16) {
    use image::ImageDecoder as _;
    let blob = &d.images[&im.key];
    assert_eq!(blob.mime, "image/png");
    let mut dec = image::codecs::png::PngDecoder::new(std::io::Cursor::new(blob.bytes.as_slice())).unwrap();
    let color = dec.original_color_type();
    (dec.icc_profile().unwrap(), color.bits_per_pixel() / u16::from(color.channel_count()))
}

/// A raster that import writes again as PNG keeps the gray or RGB ICC profile its file carries,
/// byte for byte, linked and as a data URL: a TIFF, turned or not, and a turned PNG and JPEG, with
/// 16-bit samples kept at 16 bits and the resolution kept beside the profile. A profile of another
/// color space than the written samples', one too short to be a profile, and one on a TIFF whose
/// pixels are read as RGBA by its own reader are left out, and the image still imports.
#[test]
fn a_raster_written_again_as_png_keeps_its_icc_profile() {
    use image::{DynamicImage, ImageEncoder as _};
    let (rgb_icc, gray_icc, cmyk_icc) = (icc_profile(b"RGB ", 600), icc_profile(b"GRAY", 300), icc_profile(b"CMYK", 400));
    let rgb8 = DynamicImage::ImageRgb8(image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([u8::try_from(x * 70 + y * 9).unwrap(), 200, 13])));
    let gray8 = DynamicImage::ImageLuma8(image::GrayImage::from_fn(3, 2, |x, y| image::Luma([u8::try_from(x * 80 + y * 7).unwrap()])));
    let rgb16 = DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(3, 2, |x, y| {
        let v = u16::try_from(x * 0x1111 + y * 0x2345).unwrap();
        image::Rgb([0x1000 + v, 0x8001 + v, 0xF0F3 - v])
    }));
    let tiff_of = |img: &DynamicImage, icc: &[u8]| {
        let mut out = Vec::new();
        let mut enc = image::codecs::tiff::TiffEncoder::new(std::io::Cursor::new(&mut out));
        enc.set_icc_profile(icc.to_vec()).unwrap();
        img.write_with_encoder(enc).unwrap();
        out
    };
    let png = {
        let mut out = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut out);
        enc.set_icc_profile(rgb_icc.clone()).unwrap();
        enc.set_exif_metadata(exif_block(6)).unwrap();
        rgb16.write_with_encoder(enc).unwrap();
        out
    };
    let jpg = {
        let mut out = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
        enc.set_icc_profile(rgb_icc.clone()).unwrap();
        rgb8.write_with_encoder(enc).unwrap();
        let exif = [b"Exif\0\0".as_slice(), &exif_block(6)].concat();
        let mut turned = out[..2].to_vec();
        turned.extend([0xff, 0xe1]);
        turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        turned.extend(&exif);
        turned.extend(&out[2..]);
        turned
    };
    let gray = [0, 0, 255, 255, 0, 0, 255, 255];
    let turned_gray = tiff_with_icc(&tiff_at(4, 2, 1, &gray, 6, Some((300, 150))), &gray_icc);
    let gray_alpha = [0, 255, 0, 255, 255, 255, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255];
    // (what, file, the ICC profile to keep, bits per sample, resolution)
    type Case<'a> = (&'a str, Vec<u8>, Option<&'a [u8]>, u16, Option<(f64, f64)>);
    let cases: Vec<Case> = vec![
        ("RGB TIFF", tiff_of(&rgb8, &rgb_icc), Some(&rgb_icc), 8, None),
        ("gray TIFF", tiff_of(&gray8, &gray_icc), Some(&gray_icc), 8, None),
        ("16-bit RGB TIFF", tiff_of(&rgb16, &rgb_icc), Some(&rgb_icc), 16, None),
        ("gray TIFF, orientation 6, with a resolution", turned_gray, Some(&gray_icc), 8, Some((150.0, 300.0))),
        ("16-bit RGB PNG, orientation 6", png, Some(&rgb_icc), 16, None),
        ("JPEG, orientation 6", jpg, Some(&rgb_icc), 8, None),
        ("RGB TIFF with a gray profile", tiff_of(&rgb8, &gray_icc), None, 8, None),
        ("gray TIFF with an RGB profile", tiff_of(&gray8, &rgb_icc), None, 8, None),
        ("RGB TIFF with a CMYK profile", tiff_of(&rgb8, &cmyk_icc), None, 8, None),
        ("RGB TIFF with a profile too short to be one", tiff_of(&rgb8, &rgb_icc[..40]), None, 8, None),
        ("gray TIFF with alpha, read as RGBA", tiff_with_icc(&tiff(4, 2, 2, &gray_alpha, 1), &rgb_icc), None, 8, None),
    ];
    let dir = folder("icc");
    for (what, file, icc, bits, ppi) in cases {
        std::fs::write(dir.join("photo.img"), &file).unwrap();
        let data = format!("data:;base64,{}", base64_encode(&file));
        for href in ["photo.img", data.as_str()] {
            let what = format!("{what}, {}", &href[..href.len().min(12)]);
            let (d, im) = import_one(&format!(r#"<image href="{href}" width="20" height="40"/>"#), Some(&dir));
            assert_eq!(stored_icc(&d, &im), (icc.map(<[u8]>::to_vec), bits), "{what}");
            assert_eq!(stored_ppi(&d, &im), ppi, "{what}");
        }
    }
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
