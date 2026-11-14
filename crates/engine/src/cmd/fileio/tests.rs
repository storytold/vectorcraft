use std::io::Cursor;

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;
use crate::cmd::parse_range;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-fileio-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A `w`×`h` image of one colour in `format`.
fn image_bytes(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let rgba = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 40, 255]));
    let img = if format == image::ImageFormat::Jpeg {
        image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        rgba.into()
    };
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

fn image_of(s: &Session) -> vectorcraft_doc::ImageObject {
    let doc = &s.doc().unwrap().doc;
    match &doc.layers[0].children().unwrap()[0].kind {
        NodeKind::Image(im) => (**im).clone(),
        other => panic!("expected an image, got {other:?}"),
    }
}

/// A path outside a confined request's read roots shows as its file name, after the last `/` or `\`
/// on every platform; one that ends in no name shows as nothing. Without roots it shows whole.
#[test]
fn a_hidden_path_shows_as_its_file_name_on_every_platform() {
    let windows = "C:\\Users\\someone\\art\\red.png";
    crate::file_access::confine(Some(&crate::file_access::AutomationRoots::none()), || {
        for (path, name) in [("/home/someone/art/red.png", "red.png"), (windows, "red.png"), ("/home/someone/..", ""), ("C:\\Users\\someone\\", "")] {
            assert_eq!(shown_path(path), name, "{path}");
        }
    });
    assert_eq!(shown_path(windows), windows);
}

#[test]
fn opens_every_readable_format() {
    let mut s = session(200.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let native = b64(&s.execute("document.serialize", &json!({})).unwrap());
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    let pdf = b64(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap());
    let svgz = vectorcraft_svg::compress(&svg);

    for (name, bytes, format) in [
        ("a.vectorcraft", native.clone(), "vectorcraft"),
        ("a.drawcraft", native, "vectorcraft"),
        ("a.svg", svg.into_bytes(), "svg"),
        ("a.svgz", svgz, "svgz"),
        ("a.pdf", pdf.clone(), "pdf"),
        ("a.ai", pdf, "ai"),
    ] {
        let r = open(&mut s, name, &bytes);
        assert_eq!(r["format"], format, "{name}");
        assert_eq!(r["title"], if format == "vectorcraft" { "Untitled-1" } else { name }, "{name}");
        assert!(s.doc().unwrap().doc.node_count() >= 2, "{name}: the rectangle came through");
    }
    for (name, kind) in [
        ("p.png", image::ImageFormat::Png),
        ("p.jpg", image::ImageFormat::Jpeg),
        ("p.gif", image::ImageFormat::Gif),
        ("p.webp", image::ImageFormat::WebP),
        ("p.tif", image::ImageFormat::Tiff),
        ("p.bmp", image::ImageFormat::Bmp),
    ] {
        let r = open(&mut s, name, &image_bytes(5, 4, kind));
        assert_eq!(r["format"], format(name.rsplit('.').next().unwrap()).unwrap().id, "{name}");
        let im = image_of(&s);
        assert_eq!((im.width, im.height), (5, 4), "{name}");
        let doc = &s.doc().unwrap().doc;
        assert_eq!((doc.artboards[0].rect.width(), doc.artboards[0].rect.height()), (5.0, 4.0));
        // Browser-safe formats keep their bytes; TIFF and BMP are stored as PNG.
        let mime = &doc.images[&im.key].mime;
        assert_eq!(mime, if name.ends_with("tif") || name.ends_with("bmp") { "image/png" } else { format_for_name(name).unwrap().mime });
    }
    assert!(s.execute("document.open", &json!({"name": "x.txt", "dataBase64": "aGVsbG8="})).is_err());
}

#[test]
fn webp_opens_at_its_pixel_size() {
    let mut s = Session::new();
    open(&mut s, "tiny.webp", &image_bytes(3, 2, image::ImageFormat::WebP));
    let im = image_of(&s);
    assert_eq!((im.width, im.height), (3, 2));
    let ab = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (3.0, 2.0));
    let r = raster_image(&image_bytes(3, 2, image::ImageFormat::WebP)).unwrap();
    assert_eq!((r.width, r.height, r.blob.mime.as_str()), (3, 2, "image/webp"));
}

#[test]
fn cmyk_tiffs_keep_their_inks_and_other_tiffs_become_png() {
    let inks = vectorcraft_doc::cmyk::Inks::new(2, 1, vec![0, 102, 255, 0, 10, 20, 30, 40]).unwrap();
    let tiff = vectorcraft_doc::ImageBlob::cmyk_tiff(&inks).unwrap();
    let r = raster_image(&tiff.bytes).unwrap();
    assert_eq!((r.width, r.height, r.blob.mime.as_str()), (2, 1, "image/tiff"));
    assert_eq!(r.blob.cmyk(), Some(inks));
    let rgb = raster_image(&image_bytes(2, 1, image::ImageFormat::Tiff)).unwrap();
    assert_eq!(rgb.blob.mime, "image/png");
}

/// A 3 × 2 TIFF at 150 ppi of `channels` (`extra`: their ExtraSamples values), 8 or 16 `bits` a
/// sample, chunky or `planar`.
fn hand_tiff(photometric: u16, bits: u16, extra: &[u16], planar: bool, channels: &[Vec<u16>]) -> Vec<u8> {
    let t = vectorcraft_testkit::tiff::Tiff { width: 3, height: 2, photometric, bits, extra, planar, ppi: Some(150), channels };
    vectorcraft_testkit::tiff::tiff(&t)
}

/// What Place keeps of TIFF `bytes`: the ink amounts, when it keeps them, and the pixels any image
/// reader decodes from the stored image, as RGBA.
fn kept(what: &str, bytes: &[u8]) -> (Option<Vec<u8>>, Vec<u8>) {
    let r = raster_image(bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!((r.width, r.height, r.ppi), (3, 2, Some((150.0, 150.0))), "{what}");
    assert_eq!(super::ppi::resolution(&r.blob.bytes), Some((150.0, 150.0)), "{what}: the stored image keeps the resolution");
    let rgba = image::load_from_memory(&r.blob.bytes).unwrap_or_else(|e| panic!("{what}: {e}")).to_rgba8().into_raw();
    (r.blob.cmyk().map(|i| i.data), rgba)
}

/// TIFFs that store one plane per channel (PlanarConfiguration 2) read as their chunky versions do:
/// grayscale and RGB with extra channels, and CMYK with and without them (#1466).
#[test]
fn planar_tiffs_read_as_their_chunky_versions() {
    let ch = |v: [u16; 6]| v.to_vec();
    let (gray, alpha, spot) = (ch([0, 51, 102, 153, 204, 255]), ch([255, 128, 0, 255, 64, 255]), ch([9, 8, 7, 6, 5, 4]));
    let (r, g, b) = (ch([255, 0, 0, 10, 20, 30]), ch([0, 255, 0, 40, 50, 60]), ch([0, 0, 255, 70, 80, 90]));
    let (c, m, y, k) = (ch([255, 0, 0, 0, 12, 200]), ch([0, 255, 0, 0, 34, 100]), ch([0, 0, 255, 0, 56, 50]), ch([0, 0, 0, 255, 78, 0]));
    // Pixel by pixel, the samples of `px`.
    let interleave = |px: &[&Vec<u16>]| -> Vec<u8> { (0..6).flat_map(|i| px.iter().map(move |v| v[i] as u8)).collect() };
    let opaque = ch([255; 6]);
    let inks = interleave(&[&c, &m, &y, &k]);
    // (photometric, ExtraSamples, channels, the RGBA pixels when the color is gray or RGB, the inks when it is CMYK)
    type Case = (u16, Vec<u16>, Vec<Vec<u16>>, Option<Vec<u8>>, Option<Vec<u8>>);
    let cases: Vec<Case> = vec![
        (1, vec![2], vec![gray.clone(), alpha.clone()], Some(interleave(&[&gray, &gray, &gray, &alpha])), None),
        (1, vec![2, 0], vec![gray.clone(), alpha.clone(), spot.clone()], Some(interleave(&[&gray, &gray, &gray, &alpha])), None),
        (2, vec![0], vec![r.clone(), g.clone(), b.clone(), spot.clone()], Some(interleave(&[&r, &g, &b, &opaque])), None),
        (2, vec![2, 0], vec![r.clone(), g.clone(), b.clone(), alpha.clone(), spot.clone()], Some(interleave(&[&r, &g, &b, &alpha])), None),
        (5, vec![], vec![c.clone(), m.clone(), y.clone(), k.clone()], None, Some(inks.clone())),
        (5, vec![0], vec![c.clone(), m.clone(), y.clone(), k.clone(), spot.clone()], None, Some(inks.clone())),
        (5, vec![2], vec![c.clone(), m.clone(), y.clone(), k.clone(), alpha.clone()], None, None),
    ];
    for (photometric, extra, channels, want_rgba, want_inks) in cases {
        for bits in [8, 16] {
            let wide: Vec<Vec<u16>> = channels.iter().map(|ch| ch.iter().map(|v| if bits == 16 { v * 257 } else { *v }).collect()).collect();
            let what = format!("photometric {photometric}, extra samples {extra:?}, {bits} bits");
            let chunky = kept(&format!("{what}, chunky"), &hand_tiff(photometric, bits, &extra, false, &wide));
            let planar = kept(&format!("{what}, planar"), &hand_tiff(photometric, bits, &extra, true, &wide));
            assert_eq!(planar.0, chunky.0, "{what}: planar keeps the inks chunky keeps");
            assert_eq!(planar.0, want_inks, "{what}: the inks");
            // A 16-bit chunky CMYK file is stored as it is, and a planar one is written again with
            // its 8-bit inks: their RGB pixels may be a level apart.
            let close = planar.1.len() == chunky.1.len() && planar.1.iter().zip(&chunky.1).all(|(a, b)| a.abs_diff(*b) <= 1);
            assert!(close, "{what}: planar reads as chunky: {:?} against {:?}", planar.1, chunky.1);
            if let Some(want) = &want_rgba {
                assert_eq!(&planar.1, want, "{what}: the pixels");
            }
        }
    }
}

/// A planar TIFF whose header claims more strips than the decoder can count (here 2^31 + 1 rows
/// a strip each, times 5 planes) is refused as too large to draw, without a panic.
#[test]
fn planar_tiffs_with_more_strips_than_can_be_counted_are_too_large() {
    let plane = |v: u16| vec![v; 6];
    let mut file = hand_tiff(5, 8, &[0], true, &[plane(200), plane(40), plane(10), plane(0), plane(90)]);
    // ImageLength (the directory's second entry, its value at offset 30) and RowsPerStrip (the
    // eighth, at offset 102).
    file[30..34].copy_from_slice(&(1u32 << 31 | 1).to_le_bytes());
    file[102..106].copy_from_slice(&1u32.to_le_bytes());
    let e = raster_image(&file).err().expect("refused");
    assert!(e.to_string().contains("an image of 3 × 2147483649 pixels is too large to draw"), "{e}");
    assert!(load::too_large_note(&file).is_some());
    assert_eq!(vectorcraft_doc::cmyk::oversized_tiff(&file), Some((3, 1 << 31 | 1)));
}

/// A 16-bit RGB TIFF whose samples need more than the allowance (6 bytes a pixel), though its RGBA
/// pixels would fit, is refused as too large to draw before it is decoded.
#[test]
fn tiffs_whose_samples_pass_the_allowance_are_too_large() {
    let plane = |v: u16| vec![v; 6];
    let mut file = hand_tiff(2, 16, &[], false, &[plane(9000), plane(30_000), plane(60_000)]);
    // The loop sets ImageWidth, ImageLength and RowsPerStrip (the first, second and eighth entries) to `side`.
    let side = 20_000u32;
    assert!(u64::from(side * side) * 6 > MAX_RASTER_ALLOC && u64::from(side * side) * 4 <= MAX_RASTER_ALLOC);
    for at in [18, 30, 102] {
        file[at..at + 4].copy_from_slice(&side.to_le_bytes());
    }
    let e = raster_image(&file).err().expect("refused");
    assert_eq!(e.to_string(), "an image of 20000 × 20000 pixels is too large to draw");
}

/// A planar CMYK TIFF opens and places with its ink amounts, stored as a TIFF the image decoder
/// reads (#1466).
#[test]
fn planar_cmyk_tiffs_open_and_place_with_their_inks() {
    let inks = [vec![255u16, 0, 12, 0, 40, 9], vec![0, 255, 34, 0, 50, 8], vec![0, 0, 56, 0, 60, 7], vec![0, 0, 78, 255, 70, 6]];
    let file = hand_tiff(5, 8, &[], true, &inks);
    let want: Vec<u8> = (0..6).flat_map(|i| inks.iter().map(move |v| v[i] as u8)).collect();
    assert!(image::load_from_memory(&file).is_err(), "the image decoder reads planar CMYK now");
    let mut s = Session::new();
    open(&mut s, "planar.tif", &file);
    let blob = &s.doc().unwrap().doc.images[&image_of(&s).key];
    assert_eq!(blob.cmyk().map(|i| i.data), Some(want.clone()), "Open keeps the inks");
    let mut s = session(100.0, 100.0, 1);
    s.execute("file.place", &json!({"name": "planar.tif", "dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
    let blob = &s.doc().unwrap().doc.images[&image_of(&s).key];
    assert_eq!(blob.cmyk().map(|i| i.data), Some(want), "Place keeps the inks");
    assert!(image::load_from_memory(&blob.bytes).is_ok(), "the image decoder reads the stored image");
}

/// A 70 × 7 picture, `true` for black: a white row and a black row (runs past 63 take a make-up
/// code), then runs of several lengths.
fn fax_picture() -> Vec<Vec<bool>> {
    (0..7).map(|y| (0..70).map(|x| if y < 2 { y == 1 } else { (x * (y + 2) / 5 + y) % 3 == 0 }).collect()).collect()
}

/// The first `rows` rows of [`fax_picture`] as a TIFF compressed with CCITT Group 3 in one strip:
/// `compression` 2 or 3 with `t4_options`, white is zero, the highest bit of each byte first.
fn group3_tiff(compression: u16, t4_options: u32, rows: usize) -> Vec<u8> {
    let rows = &fax_picture()[..rows];
    let g =
        vectorcraft_testkit::tiff::Group3 { rows, compression, t4_options, photometric: 0, fill_order: 1, rows_per_strip: 7, tile: None, rtc: false };
    vectorcraft_testkit::tiff::group3(&g)
}

/// 1-bit TIFFs compressed with CCITT Group 3 open and place with their pixels (#1466): Compression
/// 2 (modified Huffman) and Compression 3 (ITU-T T.4) with one-dimensional rows or two-dimensional
/// ones (T4Options bit 0), with and without fill bits (bit 2) and six EOLs at the end, in one strip,
/// three strips or three 32 × 16 tiles, each byte's highest or lowest bit first (FillOrder 1 and 2),
/// white or black as zero.
#[test]
fn ccitt_group3_tiffs_open_and_place_with_their_pixels() {
    let rows = fax_picture();
    let mut s = Session::new();
    for (compression, t4_options, rtc) in [(2, 0, false), (3, 0, false), (3, 4, true), (3, 1, false), (3, 5, true)] {
        for (rows_per_strip, tile) in [(7, None), (3, None), (7, Some((32, 16)))] {
            for fill_order in [1, 2] {
                for photometric in [0, 1] {
                    let what = format!(
                        "Compression {compression}, T4Options {t4_options}, {rows_per_strip} rows a strip, tiles {tile:?}, FillOrder {fill_order}, photometric {photometric}"
                    );
                    let g = vectorcraft_testkit::tiff::Group3 {
                        rows: &rows,
                        compression,
                        t4_options,
                        photometric,
                        fill_order,
                        rows_per_strip,
                        tile,
                        rtc,
                    };
                    let file = vectorcraft_testkit::tiff::group3(&g);
                    assert!(image::load_from_memory(&file).is_err(), "{what}: the image decoder reads Group 3 now");
                    // A black run's pixels are 1 bits, which show as black when white is zero.
                    let want: Vec<u8> = rows.concat().iter().map(|&black| if black == (photometric == 0) { 0 } else { 255 }).collect();
                    let pixels = |s: &Session| {
                        let blob = &s.doc().unwrap().doc.images[&image_of(s).key];
                        image::load_from_memory(&blob.bytes).unwrap_or_else(|e| panic!("{what}: {e}")).to_luma8().into_raw()
                    };
                    open(&mut s, "fax.tif", &file);
                    assert_eq!(pixels(&s), want, "{what}: Open");
                    let mut placed = session(100.0, 100.0, 1);
                    placed.execute("file.place", &json!({"name": "fax.tif", "dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
                    assert_eq!(pixels(&placed), want, "{what}: Place");
                }
            }
        }
    }
}

/// A Group 3 TIFF whose pixels would take more than the decoding allowance as RGBA is refused as
/// too large to draw, without a panic at 2^31 × 2^31 pixels too. One whose data ends inside a row,
/// or after a whole row before the strip's last, is refused as damaged, and so is one whose tiles
/// are wider than the allowance.
#[test]
fn ccitt_group3_tiffs_too_large_or_cut_short_are_refused() {
    // ImageWidth (the directory's first entry, its value at offset 18), ImageLength (the second, at
    // 30) and RowsPerStrip (the eighth, at 102).
    for (width, height) in [(70u32, 1u32 << 30), (1 << 31, 1 << 31)] {
        let mut big = group3_tiff(3, 1, 7);
        big[18..22].copy_from_slice(&width.to_le_bytes());
        big[30..34].copy_from_slice(&height.to_le_bytes());
        big[102..106].copy_from_slice(&height.to_le_bytes());
        let e = raster_image(&big).err().expect("refused");
        assert!(e.to_string().contains(&format!("an image of {width} × {height} pixels is too large to draw")), "{e}");
    }
    let mut short = group3_tiff(3, 0, 7);
    short.truncate(short.len() - 6);
    let e = raster_image(&short).err().expect("refused");
    assert!(e.to_string().contains("could not decode image: unexpected end of input"), "{e}");
    // Compression 2 rows start at a byte, so the strip of the first six rows is where the strip of
    // seven starts. StripByteCounts (the ninth entry, its value at offset 114) gives the six rows'.
    let (mut seven, six) = (group3_tiff(2, 0, 7), group3_tiff(2, 0, 6));
    seven[114..118].copy_from_slice(&six[114..118]);
    assert_eq!(seven[30..34], 7u32.to_le_bytes(), "ImageLength");
    assert_eq!(seven[78..82], six[78..82], "StripOffsets (the sixth entry, at offset 78)");
    let e = raster_image(&seven).err().expect("refused");
    assert!(e.to_string().contains("could not decode image: unexpected end of input"), "{e}");
    // A tile wider than the allowance: TileWidth (the eighth entry, its value at offset 102).
    let rows = fax_picture();
    let g = vectorcraft_testkit::tiff::Group3 {
        rows: &rows,
        compression: 3,
        t4_options: 1,
        photometric: 0,
        fill_order: 1,
        rows_per_strip: 7,
        tile: Some((80, 16)),
        rtc: false,
    };
    let mut wide = vectorcraft_testkit::tiff::group3(&g);
    assert_eq!(wide[102..106], 80u32.to_le_bytes(), "TileWidth");
    wide[102..106].copy_from_slice(&u32::MAX.to_le_bytes());
    let e = raster_image(&wide).err().expect("refused");
    assert!(e.to_string().contains("could not decode image: arithmetic overflow"), "{e}");
}

#[test]
fn content_beats_a_wrong_extension() {
    let png = image_bytes(2, 2, image::ImageFormat::Png);
    assert_eq!(detect("photo.jpg", &png).unwrap().id, "png");
    assert_eq!(detect("noext", b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap().id, "svg");
    assert_eq!(detect("x.ait", b"%PDF-1.7").unwrap().id, "ait");
    assert_eq!(detect("x.bin", b"%PDF-1.7").unwrap().id, "pdf");
    assert!(detect("x.bin", b"nothing").is_none());
}

#[test]
fn templates_open_untitled() {
    let mut s = session(100.0, 100.0, 1);
    let dir = tmp_dir("tpl");
    let path = dir.join("t.vectorcraft").to_string_lossy().to_string();
    // A template under a native file name still opens untitled.
    std::fs::write(&path, b64(&s.execute("file.saveAsTemplate", &json!({})).unwrap())).unwrap();
    let r = s.execute("document.open", &json!({"path": path})).unwrap();
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert!(!s.doc().unwrap().doc.template);

    let pdf = b64(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap());
    let ait = dir.join("t.ait").to_string_lossy().to_string();
    std::fs::write(&ait, pdf).unwrap();
    let r = s.execute("document.open", &json!({"path": ait})).unwrap();
    assert_eq!(r["format"], "ait");
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_exts_cover_every_readable_format() {
    for f in FORMATS.iter().filter(|f| f.read) {
        for e in read_extensions(f) {
            assert!(OPEN_EXTS.contains(e), "OPEN_EXTS lacks .{e} ({})", f.label);
        }
    }
    for e in OPEN_EXTS {
        assert!(readable_format(e).is_some(), ".{e} in OPEN_EXTS is no readable format");
    }
    let filters: Vec<_> = open_filters().collect();
    assert_eq!(filters[0], ("All readable files", OPEN_EXTS));
    assert_eq!(
        filters.len(),
        14 + FORMATS.iter().filter(|f| f.read).count(),
        "and swatch and graphic style libraries, libraries, flattener, PDF, print, perspective grid and export presets, composite \
         fonts, fonts, shortcut sets, scripts, plug-ins"
    );
    let at = |label: &str| filters.iter().position(|f| f.0 == label).unwrap_or_else(|| panic!("File › Open has no {label} filter"));
    let styles: &[&str] = &[vectorcraft_doc::style_libs::STYLES_EXT];
    assert_eq!(filters[at("Graphic Style Libraries")].1, styles, "File › Open opens graphic style libraries");
    assert_eq!(at("Graphic Style Libraries"), at("Swatch libraries") + 1, "next to the swatch libraries");
    assert_eq!(filters[at("Perspective grid presets")].1, crate::cmd::perspgrid::PRESET_EXTS, "File › Open imports perspective grid presets");
    assert_eq!(at("Perspective grid presets"), at("Print presets") + 1, "among the presets");
    assert_eq!(at("Export presets"), at("Perspective grid presets") + 1, "among the presets");
    for e in [vectorcraft_doc::style_libs::STYLES_EXT].iter().chain(crate::cmd::perspgrid::PRESET_EXTS) {
        assert!(!filters[0].1.contains(e), "All readable files lists documents only, not .{e}");
    }
    assert!(filters.contains(&("Export presets", SCREEN_EXPORT_PRESET_EXTS)), "File › Open imports export presets");
    assert!(filters.contains(&("Fonts", crate::cmd::fontfiles::EXTS)), "File › Open adds font files to the session");
    assert!(filters.contains(&("Scripts", crate::cmd::scripts::EXTS)), "File › Open checks scripts");
    assert_eq!(filters.last(), Some(&("Plug-ins", crate::cmd::plugin::EXTS)), "File › Open installs plug-ins");
}

#[test]
fn library_open_filters_list_library_formats_first() {
    let filters: Vec<_> = library_open_filters("Swatch libraries", crate::cmd::swatchlib::LIBRARY_EXTS).collect();
    assert_eq!(filters[0], ("Swatch libraries", crate::cmd::swatchlib::LIBRARY_EXTS));
    assert!(filters[0].1.contains(&"ase"));
    assert_eq!(filters[1], ("All readable files", OPEN_EXTS));
    let styles: &'static [&'static str] = std::slice::from_ref(&vectorcraft_doc::style_libs::STYLES_EXT);
    for (label, exts) in [("Swatch libraries", crate::cmd::swatchlib::LIBRARY_EXTS), ("Graphic Style Libraries", styles)] {
        let filters: Vec<_> = library_open_filters(label, exts).collect();
        assert_eq!(filters.iter().filter(|f| f.0 == label).count(), 1, "{label} is listed once");
    }
}

/// The extensions usage texts list for export: one per writable format, each picking that format
/// (`.png` picks PNG; PNG-8 is asked for by `format`).
#[test]
fn export_extensions_name_every_writable_format() {
    let exts = export_extensions();
    assert_eq!(
        exts,
        [
            "vectorcraft",
            "svg",
            "svgz",
            "pdf",
            "ai",
            "png",
            "jpg",
            "gif",
            "webp",
            "tif",
            "bmp",
            "vctemplate",
            "txt",
            "dxf",
            "eps",
            "emf",
            "wmf",
            "plt",
            "tga",
            "psd"
        ]
    );
    for e in exts {
        assert!(format_for_name(&format!("x.{e}")).is_some_and(|f| f.write), "{e}");
    }
}

#[test]
fn formats_query_lists_readers_writers_and_options() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let ids = |k: &str| r[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(
        ids("writable"),
        [
            "vectorcraft",
            "svg",
            "svgz",
            "pdf",
            "ai",
            "png",
            "jpg",
            "gif",
            "webp",
            "tiff",
            "bmp",
            "template",
            "png8",
            "txt",
            "dxf",
            "eps",
            "emf",
            "wmf",
            "hpgl",
            "tga",
            "psd"
        ]
    );
    assert!(ids("readable").contains(&"tiff".to_string()) && ids("readable").contains(&"ait".to_string()));
    let png = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "png").unwrap();
    assert_eq!(png["options"]["scale"]["default"], Value::Null);
    assert_eq!(r["openExtensions"].as_array().unwrap().len(), OPEN_EXTS.len());
}

/// The format ids `command`'s params text lists for `format` (`a|b (default)|c (…)`, up to the
/// first comma), sorted.
fn listed_formats(command: &str) -> Vec<String> {
    let params = crate::cmd::find_command(command).unwrap().params;
    let (_, rest) = params.split_once("format?: ").unwrap_or_else(|| panic!("{command} lists no formats"));
    let list = rest.split(',').next().unwrap_or_default();
    let mut ids: Vec<String> = list.split('|').filter_map(|f| f.split_whitespace().next()).map(str::to_string).collect();
    ids.sort();
    ids
}

/// The params texts of the commands that take any writable format list exactly the formats each
/// command writes (.ai among them).
#[test]
fn export_params_list_every_format_the_command_writes() {
    let mut s = session(100.0, 80.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    for command in ["document.serialize", "document.export", "document.exportSelection"] {
        let mut refused = vec![];
        let mut written: Vec<String> = FORMATS
            .iter()
            .filter(|f| s.execute(command, &json!({"format": f.id})).map_err(|e| refused.push(format!("{}: {e}", f.id))).is_ok())
            .map(|f| f.id.to_string())
            .collect();
        written.sort();
        assert_eq!(listed_formats(command), written, "{command}; refused: {refused:?}");
    }
}

#[test]
fn ranges_parse_one_based() {
    assert_eq!(parse_range("1-3, 5", 5).unwrap(), [0, 1, 2, 4]);
    assert_eq!(parse_range("3-, 1", 4).unwrap(), [2, 3, 0]);
    assert_eq!(parse_range("-2", 4).unwrap(), [0, 1]);
    assert_eq!(parse_range("2,2,1\u{2013}2", 4).unwrap(), [1, 0]);
    for bad in ["", "0", "6", "3-1", "a", "-", "1-9"] {
        assert!(parse_range(bad, 5).is_err(), "{bad:?}");
    }
}

/// Pages in a PDF, counted by importing it (one artboard per page).
fn pdf_pages(bytes: &[u8]) -> usize {
    vectorcraft_pdf::import(bytes).unwrap().artboards.len()
}

#[test]
fn pdf_export_honours_artboard_and_range() {
    let mut s = session(100.0, 80.0, 3);
    let pages = |s: &mut Session, p: Value| pdf_pages(&b64(&s.execute("document.export", &p).unwrap()));
    assert_eq!(pages(&mut s, json!({"format": "pdf"})), 3);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "artboard": 1})), 1);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "artboards": [0, 2]})), 2);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "range": "2-3"})), 2);
    assert_eq!(pdf_pages(&b64(&s.execute("document.serialize", &json!({"format": "pdf", "artboard": 2})).unwrap())), 1);
    assert!(s.execute("document.export", &json!({"format": "pdf", "artboard": 3})).is_err());
    assert!(s.execute("document.export", &json!({"format": "pdf", "range": "1-4"})).is_err());
    // Single-image formats take one artboard.
    assert!(s.execute("document.export", &json!({"format": "png", "range": "1-2"})).is_err());
    let png = b64(&s.execute("document.export", &json!({"format": "png", "range": "2", "scale": 0.5})).unwrap());
    assert_eq!(image::load_from_memory(&png).unwrap().width(), 50);
    assert!(s.execute("document.export", &json!({"format": "png", "scale": "big"})).is_err(), "typed options reject a bad type");
}

#[test]
fn export_without_path_returns_bytes_and_never_retargets() {
    let mut s = session(100.0, 100.0, 1);
    let dir = tmp_dir("retarget");
    let doc_path = dir.join("doc.vectorcraft").to_string_lossy().to_string();
    s.execute("document.save", &json!({"path": doc_path})).unwrap();
    let copy = dir.join("copy.vectorcraft").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": copy})).unwrap();
    assert_eq!(r["format"], "vectorcraft");
    assert!(vectorcraft_format::sniff(&std::fs::read(&copy).unwrap()));
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(doc_path.as_str()), "exporting the native format keeps the document's path");
    let r = s.execute("document.export", &json!({"format": "jpg"})).unwrap();
    assert_eq!(&b64(&r)[..2], [0xFF, 0xD8]);
    assert!(r.get("path").is_none());
    assert!(s.execute("document.export", &json!({"path": dir.join("x.dwg").to_string_lossy()})).is_err(), "DWG can't be written");
    // A never-saved document hands its native bytes back.
    let mut s = session(10.0, 10.0, 1);
    let r = s.execute("document.save", &json!({})).unwrap();
    assert!(vectorcraft_format::sniff(&b64(&r)));
    assert_eq!(s.doc().unwrap().path, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// A red square on a template layer and a blue one on a normal layer above it.
fn template_doc() -> (Session, u64, u64) {
    let mut s = session(100.0, 100.0, 1);
    let template = s.doc().unwrap().doc.layers[0].id.0;
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let red = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let blue = s.execute("shape.rectangle", &json!({"x": 50, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setProps", &json!({"id": template, "template": true})).unwrap();
    (s, red, blue)
}

fn has_red(png: &[u8]) -> bool {
    image::load_from_memory(png).unwrap().to_rgba8().pixels().any(|p| p[0] > 200 && p[2] < 80 && p[3] > 0)
}

#[test]
fn raster_exports_leave_template_layers_out() {
    let (mut s, red, blue) = template_doc();
    let png = b64(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    assert!(!has_red(&png), "no red pixels from the template layer");
    assert!(image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(75, 50)[2] > 200, "the normal layer is exported");
    s.execute("select.set", &json!({"ids": [red, blue]})).unwrap();
    let r = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
    assert!(!has_red(&b64(&r)));
    assert!(r["bounds"][0].as_f64().unwrap() > 40.0, "only the blue square (and its stroke) is exported: {}", r["bounds"]);
}

#[test]
fn export_for_screens_one_page_per_pdf_unique_names() {
    let mut s = session(100.0, 80.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Icon"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Icon"})).unwrap();
    let dir = tmp_dir("screens");
    let folder = dir.to_string_lossy().to_string();
    let r = s
        .execute(
            "document.exportForScreens",
            &json!({"folder": folder, "formats": [{"format": "pdf"}, {"format": "svg", "scale": 2}, {"format": "svg", "scale": 3}, {"format": "png", "scale": 2}]}),
        )
        .unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit('/').next().unwrap()).collect();
    assert_eq!(
        names,
        ["Icon.pdf", "Icon.svg", "Icon@2x.png", "Icon-2.pdf", "Icon-2.svg", "Icon-2@2x.png", "Artboard-3.pdf", "Artboard-3.svg", "Artboard-3@2x.png"]
    );
    for f in files.iter().filter(|f| f.ends_with(".pdf")) {
        assert_eq!(pdf_pages(&std::fs::read(f).unwrap()), 1, "{f}");
    }
    // No folder: the files come back as bytes.
    let r = s.execute("document.exportForScreens", &json!({"range": "3", "formats": [{"format": "jpg", "scale": 0.5}]})).unwrap();
    assert_eq!(r["files"][0]["name"], "Artboard-3@0.5x.jpg");
    assert_eq!(image::load_from_memory(&b64(&r["files"][0])).unwrap().width(), 50);
    // A native row: each artboard as a document of its own to edit.
    let r = s.execute("document.exportForScreens", &json!({"range": "1", "formats": [{"format": "vectorcraft"}]})).unwrap();
    assert!(r["files"][0]["name"].as_str().unwrap().ends_with(".vectorcraft"), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn template_sublayers_are_left_out_too() {
    let mut s = session(100.0, 100.0, 1);
    let sub = s.execute("layer.newSublayer", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setCurrent", &json!({"id": sub})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let red = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.ancestry(vectorcraft_doc::NodeId(red)).unwrap().len(), 3, "the square is on a sublayer");
    assert!(has_red(&b64(&s.execute("document.export", &json!({"format": "png"})).unwrap())));
    s.execute("layer.setProps", &json!({"id": sub, "template": true})).unwrap();
    assert!(!has_red(&b64(&s.execute("document.export", &json!({"format": "png"})).unwrap())), "a template sublayer is left out");
    s.execute("select.set", &json!({"ids": [red]})).unwrap();
    assert!(s.execute("document.exportSelection", &json!({"format": "png"})).is_err(), "nothing but template art is selected");
}

#[test]
fn export_for_screens_vector_rows_drop_scale_suffixes_and_names_ignore_case() {
    let mut s = session(100.0, 80.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Icon"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "icon"})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards[2].name.clear();
    // The dialog's second row starts as PNG @2x; switched to PDF it keeps that suffix text.
    let formats =
        json!([{"format": "png", "scale": 1, "suffix": ""}, {"format": "pdf", "scale": 2, "suffix": "@2x"}, {"format": "svg", "suffix": "-web"}]);
    let r = s.execute("document.exportForScreens", &json!({"formats": formats})).unwrap();
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "Icon.png",
            "Icon.pdf",
            "Icon-web.svg",
            "icon-2.png",
            "icon-2.pdf",
            "icon-2-web.svg",
            "Artboard-3.png",
            "Artboard-3.pdf",
            "Artboard-3-web.svg"
        ]
    );
}

#[test]
fn oversized_raster_exports_are_refused() {
    let mut s = session(2000.0, 2000.0, 1);
    let e = s.execute("document.export", &json!({"format": "webp", "scale": 10})).unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
    assert!(s.execute("document.export", &json!({"format": "png", "scale": 40})).is_err(), "80000 px a side");
}

#[test]
fn new_files_never_replace_a_file() {
    let d = tmp_dir("new-file");
    let path = d.join("Brand.ase").to_string_lossy().to_string();
    let _ = std::fs::remove_file(&path);
    write_new_file(&path, b"first").unwrap();
    let e = write_new_file(&path, b"second").unwrap_err().to_string();
    assert!(e.starts_with(&format!("{path}: ")), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_native_save_streams_the_bytes_the_encoder_makes() {
    let d = tmp_dir("stream");
    let mut s = session(200.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    for (name, compress) in [("a.vectorcraft", false), ("b.vectorcraft", true), ("c.vctemplate", true)] {
        let path = d.join(name).to_string_lossy().to_string();
        // A PDF-compatible file's PDF is dated to the second it is written, so the byte comparison
        // uses files without one.
        let p = json!({"path": path, "compress": compress, "pdfCompatible": false});
        let r = s.execute("file.saveCopy", &p).unwrap();
        let written = std::fs::read(&path).unwrap();
        assert_eq!(r["bytes"], written.len(), "{r}");
        let plan = save_plan(&s, SaveMode::Copy, &p).unwrap();
        let job = save_job(&mut s, plan).unwrap();
        assert!(job.streams());
        assert_eq!(written, job.encode().unwrap().files[0].1, "{name}");
        assert_eq!(vectorcraft_format::load(&written).unwrap().template, name.ends_with("vctemplate"));
    }
    // A save with the default options streams a file that carries its PDF.
    let path = d.join("d.vectorcraft").to_string_lossy().to_string();
    let p = json!({"path": path});
    let r = s.execute("file.saveCopy", &p).unwrap();
    let written = std::fs::read(&path).unwrap();
    assert_eq!(r["bytes"], written.len(), "{r}");
    assert!(vectorcraft_format::pdf_content(&written).is_some());
    let plan = save_plan(&s, SaveMode::Copy, &p).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    assert!(job.streams());
    assert!(vectorcraft_format::pdf_content(&job.encode().unwrap().files[0].1).is_some());
    // A writer that fails: the save fails with its error.
    let plan = save_plan(&s, SaveMode::Copy, &json!({"path": "/x/y.vectorcraft"})).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    let e = job
        .write_streamed(&mut |_, fill| {
            fill(&mut std::io::sink()).unwrap();
            Err(EngineError::Other("disk full".into()))
        })
        .unwrap_err();
    assert_eq!(e.to_string(), "disk full");
    let _ = std::fs::remove_dir_all(&d);
}

/// An option given beside the path (an SVG option flat or in `svg`) and in `options` with different
/// values is refused before anything is written; a native option beside the path of a format that doesn't read it is named
/// in the warnings.
#[test]
fn a_save_refuses_disagreeing_options_and_names_unread_native_ones() {
    let d = tmp_dir("save-options");
    let mut s = session(200.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let path = d.join("both.vectorcraft").to_string_lossy().to_string();
    for cmd in ["document.save", "file.saveAs", "file.saveCopy", "file.saveAsTemplate"] {
        let e = s.execute(cmd, &json!({"path": path, "compress": true, "options": {"compress": false}})).unwrap_err().to_string();
        assert!(e.contains("`compress: true` and `options.compress: false` disagree") && e.contains(&format!("`{cmd}`")), "{cmd}: {e}");
    }
    assert!(!std::path::Path::new(&path).exists(), "nothing is written");
    assert!(!d.join("both.vctemplate").exists(), "nor a template");
    // An SVG option given in `svg: {…}` or in `options.svg` counts as given there.
    let svg = d.join("a.svg").to_string_lossy().to_string();
    for p in [
        json!({"svg": {"hiddenLayers": false}, "options": {"hiddenLayers": true}}),
        json!({"hiddenLayers": false, "options": {"svg": {"hiddenLayers": true}}}),
        json!({"hiddenLayers": true, "svg": {"hiddenLayers": false}, "options": {"hiddenLayers": true}}),
    ] {
        let mut p = p;
        p["path"] = json!(svg);
        let e = s.execute("file.saveCopy", &p).unwrap_err().to_string();
        assert!(e.contains("`hiddenLayers: false` and `options.hiddenLayers: true` disagree") && e.contains("`file.saveCopy`"), "{p}: {e}");
    }
    assert!(!std::path::Path::new(&svg).exists(), "nothing is written");
    // Agreeing values, or one of them null, save as asked; `3` and `3.0` agree.
    for p in [
        json!({"compress": true, "options": {"compress": true}}),
        json!({"compress": null, "options": {"compress": true}}),
        json!({"compress": true, "version": 3, "options": {"compress": true, "version": 3.0}}),
    ] {
        let mut p = p;
        p["path"] = json!(path);
        let r = s.execute("file.saveCopy", &p).unwrap();
        assert_eq!(r["warnings"], json!([]), "{p}: {r}");
        assert_eq!(std::fs::read(&path).unwrap()[..2], [0x1f, 0x8b], "{p}");
    }
    // An SVG file reads none of the native options; a .ai file reads `compress` only.
    let r = s.execute("file.saveCopy", &json!({"path": svg, "compress": true, "version": 3, "preview": null})).unwrap();
    let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(w.contains(&"`compress` was not used: SVG files don't read it"), "{w:?}");
    assert!(w.contains(&"`version` was not used: SVG files don't read it") && !w.iter().any(|w| w.starts_with("`preview`")), "{w:?}");
    assert!(std::fs::read(&svg).unwrap().starts_with(b"<"), "plain SVG");
    let r = s.execute("file.saveCopy", &json!({"path": d.join("a.ai").to_string_lossy(), "compress": false, "version": 3})).unwrap();
    let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(w.iter().any(|w| w.starts_with("`version` was not used")) && !w.iter().any(|w| w.starts_with("`compress`")), "{w:?}");
    // `precision` is read by native files and templates only.
    for name in ["p.svg", "p.pdf", "p.ai", "p.eps"] {
        let r = s.execute("file.saveCopy", &json!({"path": d.join(name).to_string_lossy(), "precision": 2})).unwrap();
        let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(w.iter().any(|w| w.starts_with("`precision` was not used:")), "{name}: {w:?}");
    }
    for name in ["p.vectorcraft", "p.vctemplate"] {
        let r = s.execute("file.saveCopy", &json!({"path": d.join(name).to_string_lossy(), "precision": 2})).unwrap();
        assert_eq!(r["warnings"], json!([]), "{name}: {r}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A native save whose file write fails partway (a full disk) leaves the file as it was, and no
/// temporary file behind.
#[test]
fn a_save_that_fails_while_streaming_leaves_the_file_as_it_was() {
    /// Takes `left` bytes, then fails as a full disk does.
    struct FailAfter<'a> {
        inner: &'a mut dyn std::io::Write,
        left: usize,
    }
    impl std::io::Write for FailAfter<'_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::other("disk full"));
            }
            let n = b.len().min(self.left);
            self.left -= n;
            self.inner.write(&b[..n])
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    let d = tmp_dir("stream-fail");
    let mut s = session(2000.0, 2000.0, 1);
    // Enough objects that the file is written in several pieces, the failure in the middle.
    for i in 0..1500 {
        s.execute("shape.rectangle", &json!({"x": i % 100 * 20, "y": i / 100 * 20, "width": 10, "height": 10})).unwrap();
    }
    for compress in [false, true] {
        let path = d.join("a.vectorcraft").to_string_lossy().to_string();
        s.execute("file.saveCopy", &json!({"path": path, "compress": compress})).unwrap();
        let before = std::fs::read(&path).unwrap();
        s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        let plan = save_plan(&s, SaveMode::Copy, &json!({"path": path, "compress": compress})).unwrap();
        let job = save_job(&mut s, plan).unwrap();
        let e = job.write_streamed(&mut |p, fill| super::write_file_with(p, &mut |w| fill(&mut FailAfter { inner: w, left: 10_000 }))).unwrap_err();
        assert!(e.to_string().contains("disk full") || e.to_string().contains("couldn't be encoded"), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), before, "compress: {compress}");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temporary file is left");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A save that fails leaves File Info's dates and the saved state alone; one that works stamps them.
#[test]
fn a_failed_save_does_not_stamp_the_documents_dates() {
    let mut s = session(200.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let before = s.doc().unwrap().doc.metadata.clone();
    let dirty = s.doc().unwrap().is_dirty();
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/y.vectorcraft", "modified": 12345})).unwrap();
    let e = save_with(&mut s, plan, |_, _| Err(EngineError::Other("disk full".into()))).unwrap_err();
    assert_eq!(e.to_string(), "disk full");
    let st = s.doc().unwrap();
    assert_eq!(st.doc.metadata, before);
    assert_eq!(st.is_dirty(), dirty);
    // Options the encoder refuses: the same.
    assert!(s.execute("file.saveAs", &json!({"path": "/x/y.vectorcraft", "version": 99, "modified": 12345})).is_err());
    let st = s.doc().unwrap();
    assert_eq!(st.doc.metadata, before);
    assert_eq!(st.is_dirty(), dirty);
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/y.vectorcraft", "modified": 12345})).unwrap();
    save_with(&mut s, plan, |_, _| Ok(())).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.metadata.modified, Some(12345));
    assert!(!st.is_dirty());
    // Edited while the file is written (Background Save): the dates go on once it is, and the
    // edit keeps the document modified.
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/z.vectorcraft", "modified": 777})).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    let uid = s.doc().unwrap().uid;
    assert_eq!(s.doc().unwrap().doc.metadata.modified, Some(12345));
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    job.write(|_, _| Ok(())).unwrap();
    job.complete(&mut s, uid);
    let st = s.doc().unwrap();
    assert_eq!(st.doc.metadata.modified, Some(777));
    assert_eq!(st.path.as_deref(), Some("/x/z.vectorcraft"));
    assert!(st.is_dirty());
    // Undoing the edit is back at what the file holds.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
}

/// #4284: a photo whose EXIF orientation says the camera was turned places upright: its pixels
/// turned as the orientation says, its width and height swapped. One without keeps its own bytes.
#[test]
fn a_photo_places_the_way_its_exif_orientation_says() {
    // 16 × 8, red on the left half.
    let mut rgb = image::RgbImage::from_pixel(16, 8, image::Rgb([255, 255, 255]));
    for y in 0..8 {
        for x in 0..8 {
            rgb.put_pixel(x, y, image::Rgb([220, 0, 0]));
        }
    }
    let mut jpg = Vec::new();
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
    // An EXIF segment with Orientation 6: turn a quarter clockwise to show.
    let exif: Vec<u8> =
        [b"Exif\0\0".as_slice(), b"II", &[0x2a, 0, 8, 0, 0, 0], &[1, 0], &[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0], &[0, 0, 0, 0]].concat();
    let mut turned = jpg[..2].to_vec();
    turned.extend([0xff, 0xe1]);
    turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    turned.extend(&exif);
    turned.extend(&jpg[2..]);
    let plain = raster_image(&jpg).unwrap();
    assert_eq!((plain.width, plain.height, plain.blob.bytes.as_slice()), (16, 8, jpg.as_slice()), "a photo as taken keeps its bytes");
    let img = raster_image(&turned).unwrap();
    assert_eq!((img.width, img.height), (8, 16), "turned a quarter");
    let px = image::load_from_memory(&img.blob.bytes).unwrap().to_rgb8();
    let red = |p: &image::Rgb<u8>| p[0] > 150 && p[1] < 80;
    assert!(red(px.get_pixel(4, 3)) && !red(px.get_pixel(4, 12)), "the left half is on top");
}

/// Where the first stored pixel shows once EXIF (or TIFF) `orientation` turns the image, as
/// (right, bottom): orientations 1 and 5 keep it at the top left, 2 and 6 put it at the top right,
/// 3 and 7 at the bottom right, 4 and 8 at the bottom left. Values outside 1 to 8 leave it.
fn first_pixel_at(orientation: u16) -> (bool, bool) {
    match orientation {
        2 | 6 => (true, false),
        3 | 7 => (true, true),
        4 | 8 => (false, true),
        _ => (false, false),
    }
}

/// A little-endian EXIF block (a TIFF header and directory, as a PNG `eXIf` chunk or a WebP
/// `EXIF` chunk holds it) with an Orientation of `orientation` and an XResolution and YResolution
/// of `x` and `y` pixels per inch.
fn exif_block(orientation: u16, (x, y): (u32, u32)) -> Vec<u8> {
    let entry = |tag: u16, kind: u16, value: u32| [&tag.to_le_bytes()[..], &kind.to_le_bytes(), &1u32.to_le_bytes(), &value.to_le_bytes()].concat();
    // Orientation, XResolution, YResolution, ResolutionUnit inches; the two rationals follow.
    [
        b"II".as_slice(),
        &[0x2a, 0, 8, 0, 0, 0],
        &4u16.to_le_bytes(),
        &entry(0x0112, 3, u32::from(orientation)),
        &entry(0x011a, 5, 62),
        &entry(0x011b, 5, 70),
        &entry(0x0128, 3, 2),
        &[0, 0, 0, 0],
        &[x.to_le_bytes(), 1u32.to_le_bytes(), y.to_le_bytes(), 1u32.to_le_bytes()].concat(),
    ]
    .concat()
}

/// A photo turned a quarter by its EXIF orientation keeps its physical size: the resolution's
/// axes swap with the pixels', in what Place reports and in the stored image. Every orientation
/// puts the first stored pixel in its corner once, and the stored image, read again, is not turned
/// a second time. Values outside 1 to 8 leave the photo as stored.
#[test]
fn a_quarter_turn_swaps_the_resolution_axes() {
    // 48 × 32: the top-left 16 × 16 (one whole JPEG MCU) red, the rest white.
    let rgb = image::RgbImage::from_fn(48, 32, |x, y| if x < 16 && y < 16 { image::Rgb([220, 0, 0]) } else { image::Rgb([255, 255, 255]) });
    let mut jpg = Vec::new();
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
    for orientation in [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        let exif = [b"Exif\0\0".as_slice(), &exif_block(orientation, (300, 150))].concat();
        // Without a JFIF density the resolution comes from the EXIF block.
        let mut turned = jpg[..2].to_vec();
        turned.extend([0xff, 0xe1]);
        turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        turned.extend(&exif);
        turned.extend(&jpg[2..]);
        let what = format!("orientation {orientation}");
        assert_eq!(super::ppi::resolution(&turned), Some((300.0, 150.0)), "{what}: as stored");
        let quarter = (5..=8).contains(&orientation);
        let (size, ppi) = if quarter { ((32, 48), (150.0, 300.0)) } else { ((48, 32), (300.0, 150.0)) };
        let img = raster_image(&turned).unwrap();
        assert_eq!(((img.width, img.height), img.ppi), (size, Some(ppi)), "{what}");
        assert_eq!(super::ppi::resolution(&img.blob.bytes), Some(ppi), "{what}: the stored image");
        let px = image::load_from_memory(&img.blob.bytes).unwrap().to_rgb8();
        let red = |x: u32, y: u32| px.get_pixel(x, y)[1] < 80;
        let (w, h) = size;
        let corners = [(false, false), (true, false), (true, true), (false, true)];
        let reds: Vec<_> =
            corners.into_iter().filter(|&(right, bottom)| red(if right { w - 4 } else { 4 }, if bottom { h - 4 } else { 4 })).collect();
        assert_eq!(reds, [first_pixel_at(orientation)], "{what}: the first stored pixel's corner");
        let again = raster_image(&img.blob.bytes).unwrap();
        assert_eq!(((again.width, again.height), again.ppi), (size, Some(ppi)), "{what}: read again, it isn't turned again");
    }
}

/// `tiff` (little-endian) with an Orientation tag (274) of `value`, a SHORT or, when `long`, a
/// LONG, and, when `y_ppi` is given, a YResolution (283, which `tiff` must have) of that many
/// pixels per inch: its first directory copied to the end of the file with the tag added in order,
/// and the header pointing there, so that every other offset in the file still holds.
fn tiff_oriented(tiff: &[u8], value: u16, long: bool, y_ppi: Option<u32>) -> Vec<u8> {
    let at = usize::try_from(u32::from_le_bytes(tiff[4..8].try_into().unwrap())).unwrap();
    let n = usize::from(u16::from_le_bytes([tiff[at], tiff[at + 1]]));
    let mut entries: Vec<Vec<u8>> = tiff[at + 2..at + 2 + 12 * n].chunks(12).map(<[u8]>::to_vec).collect();
    let i = entries.iter().position(|e| u16::from_le_bytes([e[0], e[1]]) > 274).unwrap_or(n);
    let (kind, v) = if long { (4u16, u32::from(value).to_le_bytes()) } else { (3, [value.to_le_bytes(), [0, 0]].concat().try_into().unwrap()) };
    entries.insert(i, [&274u16.to_le_bytes()[..], &kind.to_le_bytes(), &1u32.to_le_bytes(), &v].concat());
    let mut out = tiff.to_vec();
    if out.len() % 2 == 1 {
        out.push(0);
    }
    if let Some(y) = y_ppi {
        let rational = u32::try_from(out.len()).unwrap();
        out.extend([y.to_le_bytes(), 1u32.to_le_bytes()].concat());
        let e = entries.iter_mut().find(|e| u16::from_le_bytes([e[0], e[1]]) == 283).unwrap();
        e[8..12].copy_from_slice(&rational.to_le_bytes());
    }
    let ifd = u32::try_from(out.len()).unwrap();
    out[4..8].copy_from_slice(&ifd.to_le_bytes());
    out.extend(u16::try_from(n + 1).unwrap().to_le_bytes());
    out.extend(entries.concat());
    out.extend([0; 4]);
    out
}

/// TIFFs the image decoder can't read, grayscale with an alpha channel and 1-bit ones compressed
/// with CCITT Group 3, turn and flip as their Orientation tag says, as the others do (#4284),
/// whether the tag is a SHORT or a LONG. A quarter turn swaps the resolution's axes.
#[test]
fn tiffs_read_without_the_image_decoder_follow_their_orientation() {
    // 3 × 2 at 150 × 300 ppi, the first pixel black and the rest white, opaque.
    let gray = hand_tiff(1, 8, &[2], false, &[vec![0, 255, 255, 255, 255, 255], vec![255; 6]]);
    let rows: Vec<Vec<bool>> = (0..2).map(|y| (0..3).map(|x| x == 0 && y == 0).collect()).collect();
    let g = vectorcraft_testkit::tiff::Group3 {
        rows: &rows,
        compression: 2,
        t4_options: 0,
        photometric: 0,
        fill_order: 1,
        rows_per_strip: 2,
        tile: None,
        rtc: false,
    };
    let fax = vectorcraft_testkit::tiff::group3(&g);
    // The Group 3 file declares no resolution.
    for (kind, file, y_ppi) in [("Group 3", fax, None), ("gray with alpha", gray, Some(300))] {
        assert!(image::load_from_memory(&file).is_err(), "{kind}: the image decoder reads it now");
        for (orientation, long) in (1..=8).flat_map(|o| [(o, false), (o, true)]) {
            let what = format!("{kind}, orientation {orientation}, LONG {long}");
            let img = raster_image(&tiff_oriented(&file, orientation, long, y_ppi)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let quarter = orientation >= 5;
            let size = if quarter { (2, 3) } else { (3, 2) };
            let ppi = y_ppi.map(|y| if quarter { (f64::from(y), 150.0) } else { (150.0, f64::from(y)) });
            assert_eq!(((img.width, img.height), img.ppi), (size, ppi), "{what}");
            assert_eq!(super::ppi::resolution(&img.blob.bytes), ppi, "{what}: the stored image");
            let px = image::load_from_memory(&img.blob.bytes).unwrap().to_luma8();
            let (right, bottom) = first_pixel_at(orientation);
            let (x, y) = (if right { size.0 - 1 } else { 0 }, if bottom { size.1 - 1 } else { 0 });
            let black: Vec<_> = px.enumerate_pixels().filter(|p| p.2[0] < 128).map(|p| (p.0, p.1)).collect();
            assert_eq!(black, [(x, y)], "{what}: the first stored pixel's corner");
        }
    }
}

/// A CMYK TIFF that its Orientation tag turns or flips keeps its inks, turned, whether it stores
/// them together or one plane per ink, at 8 or 16 bits; a quarter turn swaps the resolution's axes.
/// Read again, the stored image is not turned a second time.
#[test]
fn cmyk_tiffs_turned_by_their_orientation_keep_their_inks() {
    // 3 × 2 at 150 × 300 ppi: the first pixel full cyan, the others 30% black.
    let first: [u8; 4] = [255, 0, 0, 0];
    let other: [u8; 4] = [0, 0, 0, 77];
    let channel = |i: usize| -> Vec<u16> { (0..6).map(|p| u16::from(if p == 0 { first[i] } else { other[i] })).collect() };
    for (bits, planar) in [(8, false), (8, true), (16, false), (16, true)] {
        let channels: Vec<Vec<u16>> = (0..4).map(|i| channel(i).iter().map(|v| if bits == 16 { v * 257 } else { *v }).collect()).collect();
        let file = hand_tiff(5, bits, &[], planar, &channels);
        for orientation in 1..=8 {
            let what = format!("{bits} bits, planar {planar}, orientation {orientation}");
            let img = raster_image(&tiff_oriented(&file, orientation, false, Some(300))).unwrap_or_else(|e| panic!("{what}: {e}"));
            let quarter = orientation >= 5;
            let size = if quarter { (2, 3) } else { (3, 2) };
            let ppi = if quarter { (300.0, 150.0) } else { (150.0, 300.0) };
            assert_eq!(((img.width, img.height), img.ppi, img.blob.mime.as_str()), (size, Some(ppi), "image/tiff"), "{what}");
            assert_eq!(super::ppi::resolution(&img.blob.bytes), Some(ppi), "{what}: the stored image");
            let inks = img.blob.cmyk().unwrap_or_else(|| panic!("{what}: the inks are lost"));
            assert_eq!((inks.width, inks.height), size, "{what}: the inks' size");
            let (right, bottom) = first_pixel_at(orientation);
            let at = (if right { size.0 - 1 } else { 0 }, if bottom { size.1 - 1 } else { 0 });
            for (i, px) in inks.data.chunks(4).enumerate() {
                let (x, y) = (u32::try_from(i).unwrap() % size.0, u32::try_from(i).unwrap() / size.0);
                assert_eq!(px, if (x, y) == at { first } else { other }, "{what}: the inks of pixel ({x}, {y})");
            }
            let again = raster_image(&img.blob.bytes).unwrap();
            assert_eq!(((again.width, again.height), again.blob.cmyk()), (size, Some(inks)), "{what}: read again, it isn't turned again");
        }
    }
}

/// A CMYK JPEG that its EXIF orientation turns or flips keeps its inks, turned, stored as a CMYK
/// TIFF; a quarter turn swaps the resolution's axes. Orientation 1 keeps the JPEG as it is, and
/// the stored image, read again, is not turned a second time.
#[test]
fn cmyk_jpegs_turned_by_their_orientation_keep_their_inks() {
    use vectorcraft_render::encode::jpeg::{self, ColorModel, JpegOptions};
    // 48 × 32: the top-left 16 × 16 (one whole JPEG MCU) full cyan, the rest 30% black.
    let inks: Vec<u8> =
        (0..32u32).flat_map(|y| (0..48u32).flat_map(move |x| if x < 16 && y < 16 { [255, 0, 0, 0] } else { [0, 0, 0, 77] })).collect();
    let o = JpegOptions { color_model: ColorModel::Cmyk, embed_icc: false, ..Default::default() };
    let jpg = jpeg::encode(&inks, 48, 32, 100, None, &o).unwrap();
    for orientation in 1..=8u16 {
        let exif = [b"Exif\0\0".as_slice(), &exif_block(orientation, (300, 150))].concat();
        let mut file = jpg[..2].to_vec();
        file.extend([0xff, 0xe1]);
        file.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        file.extend(&exif);
        file.extend(&jpg[2..]);
        let what = format!("orientation {orientation}");
        let quarter = orientation >= 5;
        let (size, ppi) = if quarter { ((32, 48), (150.0, 300.0)) } else { ((48, 32), (300.0, 150.0)) };
        let mime = if orientation == 1 { "image/jpeg" } else { "image/tiff" };
        let img = raster_image(&file).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(((img.width, img.height), img.ppi, img.blob.mime.as_str()), (size, Some(ppi), mime), "{what}");
        assert_eq!(super::ppi::resolution(&img.blob.bytes), Some(ppi), "{what}: the stored image");
        let inks = img.blob.cmyk().unwrap_or_else(|| panic!("{what}: the inks are lost"));
        assert_eq!((inks.width, inks.height), size, "{what}: the inks' size");
        let (w, h) = size;
        let at = |x: u32, y: u32| inks.data[usize::try_from((y * w + x) * 4).unwrap()..][..4].to_vec();
        let cyan = |p: &[u8]| p[0] > 200 && p[3] < 40;
        let corners = [(false, false), (true, false), (true, true), (false, true)];
        let cyans: Vec<_> =
            corners.into_iter().filter(|&(right, bottom)| cyan(&at(if right { w - 4 } else { 4 }, if bottom { h - 4 } else { 4 }))).collect();
        assert_eq!(cyans, [first_pixel_at(orientation)], "{what}: the first stored pixel's corner");
        let black = at(w / 2, h / 2);
        assert!(black[0] < 40 && (60..=95).contains(&black[3]), "{what}: the other inks, {black:?}");
        let again = raster_image(&img.blob.bytes).unwrap();
        assert_eq!(((again.width, again.height), again.blob.cmyk()), (size, Some(inks)), "{what}: read again, it isn't turned again");
    }
}

/// A PNG (its `eXIf` chunk and `pHYs` resolution), a WebP (its `EXIF` chunk) and an RGB TIFF read
/// by the image decoder, turned a quarter by their orientation, swap their resolution's axes; a
/// half turn keeps them.
#[test]
fn png_webp_and_rgb_tiff_turned_a_quarter_swap_their_resolution() {
    use image::ImageEncoder as _;
    let rgb = image::RgbImage::from_fn(3, 2, |x, y| image::Rgb(if x == 0 && y == 0 { [0, 0, 0] } else { [255, 255, 255] }));
    for orientation in [3u16, 6, 8] {
        let mut png = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut png);
        enc.set_exif_metadata(exif_block(orientation, (150, 300))).unwrap();
        enc.write_image(rgb.as_raw(), 3, 2, image::ExtendedColorType::Rgb8).unwrap();
        let png = super::ppi::with_png_resolution(&png, (150.0, 300.0));
        let mut webp = Vec::new();
        let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
        enc.set_exif_metadata(exif_block(orientation, (150, 300))).unwrap();
        enc.write_image(rgb.as_raw(), 3, 2, image::ExtendedColorType::Rgb8).unwrap();
        let channel = |c: usize| -> Vec<u16> { rgb.pixels().map(|p| u16::from(p[c])).collect() };
        let tiff = tiff_oriented(&hand_tiff(2, 8, &[], false, &[channel(0), channel(1), channel(2)]), orientation, false, Some(300));
        for (kind, file) in [("PNG", png), ("WebP", webp), ("TIFF", tiff)] {
            let what = format!("{kind}, orientation {orientation}");
            assert_eq!(super::ppi::resolution(&file), Some((150.0, 300.0)), "{what}: as stored");
            let quarter = orientation >= 5;
            let (size, ppi) = if quarter { ((2, 3), (300.0, 150.0)) } else { ((3, 2), (150.0, 300.0)) };
            let img = raster_image(&file).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(((img.width, img.height), img.ppi), (size, Some(ppi)), "{what}");
            assert_eq!(super::ppi::resolution(&img.blob.bytes), Some(ppi), "{what}: the stored image");
            let px = image::load_from_memory(&img.blob.bytes).unwrap().to_luma8();
            let (right, bottom) = first_pixel_at(orientation);
            let at = (if right { size.0 - 1 } else { 0 }, if bottom { size.1 - 1 } else { 0 });
            let black: Vec<_> = px.enumerate_pixels().filter(|p| p.2[0] < 128).map(|p| (p.0, p.1)).collect();
            assert_eq!(black, [at], "{what}: the first stored pixel's corner");
        }
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
/// directory copied to the end of the file with the tag added in order, as [`tiff_oriented`] adds
/// its tag.
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

/// The ICC profile and pixels of a stored image, read by the image decoder for its format.
fn stored_pixels(what: &str, img: &RasterImage) -> (Option<Vec<u8>>, image::ExtendedColorType, image::DynamicImage) {
    use image::ImageDecoder as _;
    let bytes = Cursor::new(img.blob.bytes.as_slice());
    let mut d: Box<dyn image::ImageDecoder> = match img.blob.mime.as_str() {
        "image/png" => Box::new(image::codecs::png::PngDecoder::new(bytes).unwrap()),
        "image/tiff" => Box::new(image::codecs::tiff::TiffDecoder::new(bytes).unwrap()),
        other => panic!("{what}: stored as {other}"),
    };
    let icc = d.icc_profile().unwrap();
    let color = d.original_color_type();
    (icc, color, image::DynamicImage::from_decoder(d).unwrap())
}

/// An image turned by its orientation and stored as PNG keeps the gray or RGB ICC profile its file
/// carries, and its 16-bit samples at 16 bits, from a PNG, a JPEG, a WebP and a TIFF; a TIFF the
/// image decoder reads, stored as PNG without a turn, keeps them too. A profile of another color
/// space than the samples' is left out.
#[test]
fn images_stored_as_png_keep_their_icc_profile_and_16_bit_samples() {
    use image::{DynamicImage, ImageEncoder as _};
    let (rgb_icc, gray_icc, cmyk_icc) = (icc_profile(b"RGB ", 600), icc_profile(b"GRAY", 300), icc_profile(b"CMYK", 400));
    // 3 × 2, every sample's low byte different.
    let rgb16 = image::ImageBuffer::from_fn(3, 2, |x, y| {
        let v = u16::try_from(x * 0x1111 + y * 0x2345).unwrap();
        image::Rgb([0x1000 + v, 0x8001 + v, 0xF0F3 - v])
    });
    let rgb16 = DynamicImage::ImageRgb16(rgb16);
    let gray16 = DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(3, 2, |x, y| image::Luma([u16::try_from(x * 0x3001 + y * 0x0107).unwrap()])));
    let rgb8 = DynamicImage::ImageRgb8(image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([u8::try_from(x * 70 + y * 9).unwrap(), 200, 13])));
    let png = |img: &DynamicImage, icc: &[u8], orientation: u16| {
        let mut out = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut out);
        enc.set_icc_profile(icc.to_vec()).unwrap();
        enc.set_exif_metadata(exif_block(orientation, (150, 150))).unwrap();
        img.write_with_encoder(enc).unwrap();
        out
    };
    let webp = {
        let mut out = Vec::new();
        let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
        enc.set_icc_profile(rgb_icc.clone()).unwrap();
        enc.set_exif_metadata(exif_block(6, (150, 150))).unwrap();
        rgb8.write_with_encoder(enc).unwrap();
        out
    };
    let tiff = |orientation: Option<u16>| {
        let mut out = Vec::new();
        let mut enc = image::codecs::tiff::TiffEncoder::new(Cursor::new(&mut out));
        enc.set_icc_profile(rgb_icc.clone()).unwrap();
        rgb16.write_with_encoder(enc).unwrap();
        match orientation {
            Some(o) => tiff_oriented(&out, o, false, None),
            None => out,
        }
    };
    // (what, file, the ICC profile to keep, the pixels as they show, 16 bits)
    type Case<'a> = (&'a str, Vec<u8>, Option<&'a [u8]>, DynamicImage, bool);
    let cases: Vec<Case> = vec![
        ("16-bit RGB PNG, orientation 6", png(&rgb16, &rgb_icc, 6), Some(&rgb_icc), rgb16.rotate90(), true),
        ("16-bit gray PNG, orientation 8", png(&gray16, &gray_icc, 8), Some(&gray_icc), gray16.rotate270(), true),
        ("8-bit RGB PNG, orientation 3", png(&rgb8, &rgb_icc, 3), Some(&rgb_icc), rgb8.rotate180(), false),
        ("RGB PNG with a CMYK profile, orientation 6", png(&rgb8, &cmyk_icc, 6), None, rgb8.rotate90(), false),
        ("WebP, orientation 6", webp, Some(&rgb_icc), rgb8.rotate90(), false),
        ("16-bit RGB TIFF, not turned", tiff(None), Some(&rgb_icc), rgb16.clone(), true),
        ("16-bit RGB TIFF, orientation 6", tiff(Some(6)), Some(&rgb_icc), rgb16.rotate90(), true),
    ];
    for (what, file, icc, want, wide) in cases {
        let img = raster_image(&file).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!((img.width, img.height, img.blob.mime.as_str()), (want.width(), want.height(), "image/png"), "{what}");
        let (kept, color, px) = stored_pixels(what, &img);
        assert_eq!(kept.as_deref(), icc, "{what}: the ICC profile");
        assert_eq!(color.bits_per_pixel() / u16::from(color.channel_count()), if wide { 16 } else { 8 }, "{what}: the sample depth");
        if wide {
            assert_eq!(px.to_rgb16(), want.to_rgb16(), "{what}: the samples");
        } else {
            assert_eq!(px.to_rgb8(), want.to_rgb8(), "{what}: the samples");
        }
    }
    // A JPEG's samples change in encoding; its profile is kept.
    let mut jpg = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 95);
    enc.set_icc_profile(rgb_icc.clone()).unwrap();
    rgb8.write_with_encoder(enc).unwrap();
    let exif = [b"Exif\0\0".as_slice(), &exif_block(6, (150, 150))].concat();
    let mut turned = jpg[..2].to_vec();
    turned.extend([0xff, 0xe1]);
    turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    turned.extend(&exif);
    turned.extend(&jpg[2..]);
    let img = raster_image(&turned).unwrap();
    assert_eq!((img.width, img.height, img.blob.mime.as_str()), (2, 3, "image/png"), "JPEG");
    assert_eq!(stored_pixels("JPEG", &img).0, Some(rgb_icc.clone()), "JPEG: the ICC profile");
    // Not turned, a PNG keeps its own bytes.
    let plain = png(&rgb16, &rgb_icc, 1);
    assert_eq!(raster_image(&plain).unwrap().blob.bytes.as_slice(), plain.as_slice(), "a PNG as taken keeps its bytes");
}

/// A CMYK TIFF stored again, because it stores one plane per ink or because its orientation turns
/// it, keeps its ICC profile as tag 34675 and its 8 or 16 bits a sample, exactly; one that stores
/// its inks together and isn't turned keeps its own bytes.
#[test]
fn cmyk_tiffs_written_again_keep_their_icc_profile_and_depth() {
    use image::DynamicImage;
    let icc = icc_profile(b"CMYK", 900);
    // 3 × 2, no black, every sample's low byte different.
    let ink = |c: usize, p: usize| -> u16 { u16::try_from(0x0101 * (c * 5 + p + 1) + 0x10 * p + c).unwrap() };
    for bits in [8u16, 16] {
        let sample = |c: usize, p: usize| match (c, bits) {
            (3, _) => 0,
            (_, 16) => ink(c, p),
            _ => ink(c, p) & 0xFF,
        };
        let channels: Vec<Vec<u16>> = (0..4).map(|c| (0..6).map(|p| sample(c, p)).collect()).collect();
        // The image decoder shows a CMYK pixel without black as 1 - C, 1 - M, 1 - Y, exactly.
        let full = if bits == 16 { 0xFFFF } else { 0xFF };
        let shown: Vec<u16> = (0..6).flat_map(|p| (0..3).map(|c| full - channels[c][p]).collect::<Vec<_>>()).collect();
        let shown = if bits == 16 {
            DynamicImage::ImageRgb16(image::ImageBuffer::from_raw(3, 2, shown).unwrap())
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(3, 2, shown.iter().map(|v| u8::try_from(*v).unwrap()).collect()).unwrap())
        };
        for planar in [false, true] {
            let file = tiff_with_icc(&hand_tiff(5, bits, &[], planar, &channels), &icc);
            for (orientation, want) in [(None, shown.clone()), (Some(6), shown.rotate90()), (Some(3), shown.rotate180())] {
                let what = format!("{bits} bits, planar {planar}, orientation {orientation:?}");
                let file = orientation.map_or_else(|| file.clone(), |o| tiff_oriented(&file, o, false, Some(150)));
                let img = raster_image(&file).unwrap_or_else(|e| panic!("{what}: {e}"));
                if orientation.is_none() && !planar {
                    assert_eq!(img.blob.bytes.as_slice(), file.as_slice(), "{what}: kept as it is");
                }
                let (kept, color, px) = stored_pixels(&what, &img);
                assert_eq!(kept.as_deref(), Some(icc.as_slice()), "{what}: the ICC profile");
                let depth = if bits == 16 { image::ExtendedColorType::Cmyk16 } else { image::ExtendedColorType::Cmyk8 };
                assert_eq!(color, depth, "{what}: the sample depth");
                assert_eq!(px, want, "{what}: the samples");
            }
        }
    }
}

/// A CMYK JPEG turned by its orientation and stored as a CMYK TIFF keeps its whole ICC profile,
/// put together from the two APP2 segments a profile larger than one segment is split into.
#[test]
fn a_turned_cmyk_jpeg_keeps_its_icc_profile_from_every_segment() {
    use vectorcraft_render::encode::jpeg::{self, ColorModel, JpegOptions};
    let icc = icc_profile(b"CMYK", 70_000);
    let inks: Vec<u8> = (0..16u32 * 8).flat_map(|i| [u8::try_from(i).unwrap(), 0, 0, 40]).collect();
    let o = JpegOptions { color_model: ColorModel::Cmyk, embed_icc: false, ..Default::default() };
    let jpg = jpeg::encode(&inks, 16, 8, 100, None, &o).unwrap();
    let exif = [b"Exif\0\0".as_slice(), &exif_block(6, (300, 300))].concat();
    let mut file = jpg[..2].to_vec();
    file.extend([0xff, 0xe1]);
    file.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    file.extend(&exif);
    let chunks: Vec<&[u8]> = icc.chunks(65_519).collect();
    assert_eq!(chunks.len(), 2, "the profile takes two segments");
    for (i, chunk) in chunks.iter().enumerate() {
        file.extend([0xff, 0xe2]);
        file.extend(u16::try_from(chunk.len() + 16).unwrap().to_be_bytes());
        file.extend(b"ICC_PROFILE\0");
        file.extend([u8::try_from(i + 1).unwrap(), 2]);
        file.extend(*chunk);
    }
    file.extend(&jpg[2..]);
    let img = raster_image(&file).unwrap();
    assert_eq!((img.width, img.height, img.blob.mime.as_str()), (8, 16, "image/tiff"));
    let (kept, color, _) = stored_pixels("CMYK JPEG", &img);
    assert_eq!(color, image::ExtendedColorType::Cmyk8);
    assert!(kept.as_deref() == Some(icc.as_slice()), "the whole ICC profile: {:?} bytes", kept.map(|k| k.len()));
}

/// With `importer` named, a file up to the importer's limit is read whole and a larger one is an
/// error, whatever its content.
#[test]
fn read_path_for_a_named_importer_stops_at_the_limit() {
    let limit = vectorcraft_plugins::import::MAX_INPUT_BYTES as u64;
    let dir = tmp_dir("read-path-limit");
    let path = dir.join("drawing.tuv");
    let name = path.to_string_lossy().into_owned();
    let named = json!({"importer": "plugin.test.notinstalled"});
    let sized = |len: u64| {
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(len).unwrap();
    };
    for len in [8, 64 << 10, 70_000, limit] {
        sized(len);
        assert_eq!(importer::read_path(&name, &named).unwrap().len() as u64, len);
    }
    sized(limit + 1);
    let e = importer::read_path(&name, &named).unwrap_err();
    assert!(e.to_string().contains("larger than") && e.to_string().contains("drawing.tuv"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #3084, #2096: Export for Screens writes JPGs a strict reader opens, in every colour model and at
/// the qualities and scales the reports used. A four-component (CMYK) file has no JFIF segment,
/// which describes only one- and three-component images (#3140); RGB and gray files keep it.
#[test]
fn export_for_screens_jpgs_open_in_every_colour_model() {
    let mut s = session(100.0, 80.0, 1);
    s.execute("paint.setFill", &json!({"color": "#cc3300"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
    for (model, components) in [("rgb", 3), ("cmyk", 4), ("gray", 1)] {
        for quality in [100, 80, 50] {
            let r = s
                .execute(
                    "document.exportForScreens",
                    &json!({"formats": [{"format": "jpg"}, {"format": "jpg", "scale": 2}], "settings": {"jpg": {"colorModel": model, "quality": quality}}}),
                )
                .unwrap();
            let files = r["files"].as_array().unwrap();
            assert_eq!(files.len(), 2, "{model} {quality}: {r}");
            for (f, width) in files.iter().zip([100, 200]) {
                let bytes = b64(f);
                let what = format!("{model} q{quality} {}", f["name"]);
                assert_eq!(bytes.get(..2), Some(&[0xFF, 0xD8][..]), "{what}: starts with SOI");
                let jfif = bytes.get(2..4) == Some(&[0xFF, 0xE0][..]);
                assert_eq!(jfif, components != 4, "{what}: JFIF only on one- and three-component files");
                let image = image::load_from_memory(&bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(image.width(), width, "{what}");
            }
        }
    }
}
