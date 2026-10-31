//! TIFF, BMP and Targa export through `document.export`: the options reach the files, impossible
//! ones are refused, and the files open again; Targa files open and place.

use serde_json::{Value, json};

use super::*;

/// A 60 × 40 pt document with a black (100 % K) rectangle on a transparent corner.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 40, "height": 40})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &p).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A TIFF's first directory: tag → (type, count, value or offset), in the file's byte order.
fn tags(f: &[u8]) -> std::collections::HashMap<u16, (u16, u32, u32)> {
    let big = &f[..2] == b"MM";
    let u16_at = |at: usize| {
        let b = [f[at], f[at + 1]];
        if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let u32_at = |at: usize| {
        let b = f[at..at + 4].try_into().unwrap();
        if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    };
    let dir = u32_at(4) as usize;
    (0..u16_at(dir) as usize)
        .map(|i| {
            let e = dir + 2 + i * 12;
            let (ty, count) = (u16_at(e + 2), u32_at(e + 4));
            // A lone SHORT sits in the first half of the value field.
            let value = if ty == 3 && count == 1 { u32::from(u16_at(e + 8)) } else { u32_at(e + 8) };
            (u16_at(e), (ty, count, value))
        })
        .collect()
}

#[test]
fn tiff_options_reach_the_file() {
    let mut s = session();
    let tiff = |s: &mut Session, p: Value| export(s, merge(json!({"format": "tiff"}), p));
    let rgb = tiff(&mut s, json!({}));
    assert_eq!(&rgb[..4], b"II*\0", "little-endian by default");
    let t = tags(&rgb);
    assert_eq!((t[&259].2, t[&262].2, t[&277].2), (5, 2, 4), "LZW, RGB, an alpha channel for the transparent corner");
    assert!(t.contains_key(&34675), "the sRGB profile");
    let img = image::load_from_memory_with_format(&rgb, image::ImageFormat::Tiff).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (60, 40));
    assert_eq!(img.get_pixel(5, 5).0[3], 0, "transparent where nothing is drawn");
    let black = img.get_pixel(40, 20).0;
    assert!(black[3] == 255 && black[0] < 80, "{black:?}");

    let mm = tiff(&mut s, json!({"byteOrder": "big", "lzw": false, "embedIcc": false, "background": "white", "ppi": 300}));
    assert_eq!(&mm[..4], b"MM\0*");
    let t = tags(&mm);
    assert_eq!((t[&259].2, t[&277].2, t[&296].2), (1, 3, 2), "uncompressed, opaque RGB, inches");
    assert!(!t.contains_key(&34675));
    let x_res = t[&282].2 as usize;
    assert_eq!(&mm[x_res..x_res + 8], [300u32.to_be_bytes(), 1u32.to_be_bytes()].concat(), "300/1 ppi");
    assert!(tiff(&mut s, json!({"background": "white"})).len() < mm.len() / 3 * 4, "LZW is smaller");

    // CMYK: four samples of ink; the black rectangle is black ink alone, the rest no ink.
    let cmyk = tiff(&mut s, json!({"colorModel": "cmyk", "lzw": false}));
    let t = tags(&cmyk);
    assert_eq!((t[&262].2, t[&277].2, t[&332].2), (5, 4, 1));
    let strip = t[&273];
    let first = if strip.1 == 1 { strip.2 } else { u32::from_le_bytes(cmyk[strip.2 as usize..][..4].try_into().unwrap()) } as usize;
    let px = |x: usize, y: usize| &cmyk[first + (y * 60 + x) * 4..][..4];
    assert_eq!((px(40, 0), px(5, 0)), (&[0, 0, 0, 255][..], &[0, 0, 0, 0][..]));

    let gray = tiff(&mut s, json!({"colorModel": "gray"}));
    assert_eq!((tags(&gray)[&262].2, tags(&gray)[&277].2), (1, 1), "grey, no alpha");
    assert!(s.execute("document.export", &json!({"format": "tiff", "byteOrder": "middle"})).is_err());
    // The file opens again.
    let r = s.execute("document.open", &json!({"name": "back.tif", "dataBase64": vectorcraft_format::base64_encode(&mm)})).unwrap();
    assert_eq!(r["format"], "tiff");
    for file in [&cmyk, &gray, &rgb] {
        s.execute("document.open", &json!({"name": "back.tif", "dataBase64": vectorcraft_format::base64_encode(file)})).unwrap();
    }
}

#[test]
fn bmp_options_reach_the_file() {
    let mut s = session();
    let bmp = |s: &mut Session, p: Value| export(s, merge(json!({"format": "bmp"}), p));
    let bits = |f: &[u8]| u16::from_le_bytes([f[28], f[29]]);
    let plain = bmp(&mut s, json!({}));
    assert_eq!((&plain[..2], bits(&plain)), (&b"BM"[..], 24));
    for depth in [1, 4, 8, 16, 32] {
        assert_eq!(bits(&bmp(&mut s, json!({"depth": depth}))), depth);
    }
    let rle = bmp(&mut s, json!({"depth": 8, "rle": true}));
    assert_eq!(u32::from_le_bytes(rle[30..34].try_into().unwrap()), 1, "RLE8");
    let flipped = bmp(&mut s, json!({"flipRows": true}));
    assert_eq!(i32::from_le_bytes(flipped[22..26].try_into().unwrap()), -40, "top-down rows");
    let os2 = bmp(&mut s, json!({"fileFormat": "os2", "depth": 4}));
    assert_eq!(u32::from_le_bytes(os2[14..18].try_into().unwrap()), 12, "OS/2 core header");
    let gray = image::load_from_memory_with_format(&bmp(&mut s, json!({"colorModel": "gray", "depth": 8})), image::ImageFormat::Bmp).unwrap();
    assert!(gray.to_rgb8().pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
    for bad in [
        json!({"depth": 2}),
        json!({"depth": 24, "rle": true}),
        json!({"depth": 8, "rle": true, "flipRows": true}),
        json!({"fileFormat": "os2", "depth": 32}),
        json!({"fileFormat": "amiga"}),
        json!({"colorModel": "cmyk"}),
    ] {
        let e = s.execute("document.export", &merge(json!({"format": "bmp"}), bad.clone())).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{bad}: {e}");
    }
    let r = s.execute("document.open", &json!({"name": "back.bmp", "dataBase64": vectorcraft_format::base64_encode(&rle)})).unwrap();
    assert_eq!(r["format"], "bmp");
}

#[test]
fn targa_depth_reaches_the_file() {
    let mut s = session();
    for (depth, alpha) in [(16, 1), (24, 0), (32, 8)] {
        let tga = export(&mut s, json!({"format": "tga", "depth": depth}));
        assert_eq!((tga[16], tga[17]), (depth, alpha));
        assert_eq!(u16::from_le_bytes([tga[12], tga[13]]), 60);
    }
    let tga = export(&mut s, json!({"format": "tga"}));
    assert_eq!(tga[16], 24, "24 bits by default");
    for bad in [json!({"depth": 8}), json!({"colorModel": "gray"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "tga"}), bad.clone())).is_err(), "{bad}");
    }
}

/// The RGBA pixels of the image `document.open` or `file.place` stored for the first object of the
/// active document's first layer.
fn stored_pixels(s: &Session) -> image::RgbaImage {
    let doc = &s.doc().unwrap().doc;
    let im = match &doc.layers[0].children().unwrap()[0].kind {
        vectorcraft_doc::NodeKind::Image(im) => im.clone(),
        other => panic!("expected an image, got {other:?}"),
    };
    image::load_from_memory(&doc.images[&im.key].bytes).unwrap().to_rgba8()
}

/// A Targa header: no image id or color map, image type `kind`, `w` × `h` pixels of `depth` bits,
/// image descriptor `descriptor`.
fn targa_header(kind: u8, w: u16, h: u16, depth: u8, descriptor: u8) -> Vec<u8> {
    let mut f = vec![0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    f.extend(w.to_le_bytes());
    f.extend(h.to_le_bytes());
    f.extend([depth, descriptor]);
    f
}

/// Targa files open and place at their pixel size with the transparency they store, by any of the
/// four Targa extensions; copied in a file manager they paste as a bitmap; the dialogs list them.
#[test]
fn targa_files_open_and_place() {
    use crate::cmd::clipboard::{PASTE_ORDER, file_flavour};
    let mut s = session();
    let b64 = vectorcraft_format::base64_encode;
    // VectorCraft's own exports: the transparent corner stays transparent at 16 and 32 bits (24
    // bits are flattened on white) and the black rectangle stays black.
    for depth in [16, 24, 32] {
        let tga = export(&mut s, json!({"format": "tga", "depth": depth}));
        let r = s.execute("document.open", &json!({"name": "art.tga", "dataBase64": b64(&tga)})).unwrap();
        assert_eq!((&r["format"], &r["warnings"]), (&json!("tga"), &json!([])), "{depth} bits: {r}");
        assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(60.0, 40.0));
        let px = stored_pixels(&s);
        assert_eq!(px.dimensions(), (60, 40));
        let corner = if depth == 24 { [255; 4] } else { [0, 0, 0, 0] };
        assert_eq!(px.get_pixel(5, 5).0[3], corner[3], "{depth} bits: the corner's alpha");
        if depth == 24 {
            assert_eq!(px.get_pixel(5, 5).0, corner, "flattened on white");
        }
        let black = px.get_pixel(40, 20).0;
        assert!(black[3] == 255 && black[..3].iter().all(|c| *c < 80), "{depth} bits: {black:?}");
        // The next export starts from the session's document again.
        s = session();
    }
    let tga = export(&mut s, json!({"format": "tga", "depth": 32}));
    for name in ["art.vda", "ART.ICB", "art.vst"] {
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": b64(&tga)})).unwrap();
        assert_eq!(r["format"], "tga", "{name}: {r}");
    }
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let r = s.execute("file.place", &json!({"name": "art.tga", "dataBase64": b64(&tga)})).unwrap();
    assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("tga"), Some(1)), "{r}");
    assert_eq!(stored_pixels(&s).get_pixel(5, 5).0[3], 0, "placed with its transparency");
    // Copied in a file manager, a Targa file pastes as a bitmap.
    assert_eq!(file_flavour("art.tga", &tga, &PASTE_ORDER), Some("image/x-tga"));
    let r = s.execute("clipboard.importImage", &json!({"mime": "image/x-tga", "dataBase64": b64(&tga)})).unwrap();
    assert_eq!((&r["count"], &r["pixelWidth"], &r["pixelHeight"]), (&json!(1), &json!(60), &json!(40)), "{r}");
    // The dialogs list the four extensions; the desktop packaging doesn't associate them.
    for e in ["tga", "vda", "icb", "vst"] {
        assert!(OPEN_EXTS.contains(&e) && PLACE_EXTS.contains(&e) && UNASSOCIATED_EXTS.contains(&e), "{e}");
    }
    let targa = ("Targa", &["tga", "vda", "icb", "vst"][..]);
    assert!(open_filters().any(|f| f == targa) && place_filters().any(|f| f == targa));
}

/// Targa pixels in every layout: run-length encoded rows stored top down (as the `image` crate
/// writes them), a 16-bit file whose one attribute bit per pixel is its alpha, with runs, stored
/// right to left and top down, 32-bit files with no attribute bits (opaque), with eight, and with
/// each Attributes Type in an extension area, and color-mapped files whose 16-bit entries carry the
/// attribute bit.
#[test]
fn targa_layouts_and_alpha_read() {
    let src = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 100, y as u8 * 200, 50, if x == 1 { 128 } else { 255 }]));
    let mut rle = Vec::new();
    src.write_to(&mut std::io::Cursor::new(&mut rle), image::ImageFormat::Tga).unwrap();
    assert_eq!((rle[2], rle[17] & 0x20), (10, 0x20), "run-length encoded true color, stored top down");
    assert_eq!(raster_image(&rle).map(|r| image::load_from_memory(&r.blob.bytes).unwrap().to_rgba8()).unwrap(), src);

    // 3 × 2, 5-5-5 color and an attribute bit, right to left (bit 4), top down (bit 5): a raw
    // packet of red, blue and a clear pixel for the first row, then a run of green.
    let mut f = targa_header(10, 3, 2, 16, 0x31);
    f.push(0x02);
    f.extend([0xfc00u16, 0x801f, 0x0000].iter().flat_map(|v| v.to_le_bytes()));
    f.push(0x82);
    f.extend(0x83e0u16.to_le_bytes());
    let img = image::load_from_memory(&raster_image(&f).unwrap().blob.bytes).unwrap().to_rgba8();
    let px = |x, y| img.get_pixel(x, y).0;
    assert_eq!((px(2, 0), px(1, 0), px(0, 0)[3]), ([255, 0, 0, 255], [0, 0, 255, 255], 0), "the first row, right to left");
    assert!((0..3).all(|x| px(x, 1) == [0, 255, 0, 255]), "the run");

    // 2 × 1 at 32 bits with no attribute bits: the fourth byte isn't alpha, whatever it holds.
    for fourth in [[0, 0], [0, 255]] {
        let mut f = targa_header(2, 2, 1, 32, 0x20);
        f.extend([10, 20, 30, fourth[0], 40, 50, 60, fourth[1]]);
        let img = image::load_from_memory(&raster_image(&f).unwrap().blob.bytes).unwrap().to_rgba8();
        assert_eq!((img.get_pixel(0, 0).0, img.get_pixel(1, 0).0), ([30, 20, 10, 255], [60, 50, 40, 255]), "{fourth:?}");
    }
    // The same with eight attribute bits: the fourth byte is alpha, also when it is 0 everywhere.
    let mut f = targa_header(2, 2, 1, 32, 0x28);
    f.extend([10, 20, 30, 0, 40, 50, 60, 0]);
    let img = image::load_from_memory(&raster_image(&f).unwrap().blob.bytes).unwrap().to_rgba8();
    assert!(img.pixels().all(|p| p.0[3] == 0));
    // The extension area's Attributes Type: undefined data (1, 2) isn't alpha, premultiplied colors
    // (4) are divided by their alpha, and alpha (3) or 0, the field left unset, keeps it.
    for (kind, want) in [(0, [50, 50, 50, 128]), (1, [50, 50, 50, 255]), (2, [50, 50, 50, 255]), (3, [50, 50, 50, 128]), (4, [100, 100, 100, 128])] {
        let mut f = targa_header(2, 1, 1, 32, 0x28);
        f.extend([50, 50, 50, 128]);
        let mut area = vec![0; 495];
        area[..2].copy_from_slice(&495u16.to_le_bytes());
        area[494] = kind;
        let at = f.len() as u32;
        f.extend(area);
        f.extend(at.to_le_bytes());
        f.extend([0; 4]);
        f.extend(b"TRUEVISION-XFILE.\0");
        let img = image::load_from_memory(&raster_image(&f).unwrap().blob.bytes).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, want, "Attributes Type {kind}");
    }

    // Color-mapped, one attribute bit, top down: 16-bit entries 5 (clear black) and 6 (red), as a
    // raw row of 8-bit indices and as runs.
    for (kind, data) in [(1, vec![6, 5, 6]), (9, vec![0x80, 6, 0x00, 5, 0x80, 6])] {
        let mut f = vec![0, 1, kind, 5, 0, 2, 0, 16, 0, 0, 0, 0, 3, 0, 1, 0, 8, 0x21];
        f.extend([0x0000u16, 0xfc00].iter().flat_map(|v| v.to_le_bytes()));
        f.extend(data);
        let img = image::load_from_memory(&raster_image(&f).unwrap().blob.bytes).unwrap().to_rgba8();
        let row: Vec<[u8; 4]> = (0..3).map(|x| img.get_pixel(x, 0).0).collect();
        assert_eq!(row, [[255, 0, 0, 255], [0, 0, 0, 0], [255, 0, 0, 255]], "image type {kind}");
    }

    // A grayscale image the decoder could hold whose RGBA pixels pass the allowance is refused from
    // its header.
    let huge = targa_header(11, 32768, 32768, 8, 0);
    let e = raster_image(&huge).err().expect("refused");
    assert_eq!(e.to_string(), "an image of 32768 × 32768 pixels is too large to draw");

    // Headers with values the specification doesn't define aren't Targa files.
    for bad in [targa_header(2, 0, 1, 24, 0), targa_header(1, 1, 1, 8, 0), targa_header(2, 1, 1, 12, 0), targa_header(2, 1, 1, 24, 0xc0)] {
        assert!(!super::tgaread::is_targa(&bad), "{bad:?}");
    }
    assert!(raster_image(b"nothing an image reader knows").is_err());
}

#[test]
fn the_formats_are_listed_and_write_one_file_per_artboard() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 30, "height": 20, "artboards": 2})).unwrap();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let find = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap().clone();
    let (tiff, bmp, tga) = (find("tiff"), find("bmp"), find("tga"));
    assert_eq!((tiff["write"].as_bool(), tiff["read"].as_bool()), (Some(true), Some(true)));
    assert_eq!((bmp["write"].as_bool(), tga["write"].as_bool(), tga["read"].as_bool()), (Some(true), Some(true), Some(true)));
    for (f, keys) in [
        (&tiff, &["colorModel", "lzw", "byteOrder", "embedIcc", "ppi", "antiAlias"][..]),
        (&bmp, &["colorModel", "depth", "fileFormat", "rle", "flipRows", "reduction", "dither"]),
        (&tga, &["depth", "ppi", "antiAlias", "background"]),
    ] {
        for k in keys {
            assert!(f["options"].get(*k).is_some(), "{} takes {k}", f["id"]);
        }
    }
    assert_eq!(format_for_name("a.tga").map(|f| f.id), Some("tga"));
    let r = s.execute("document.export", &json!({"format": "tiff", "useArtboards": true})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(files, ["Untitled-1-Artboard-1.tif", "Untitled-1-Artboard-2.tif"]);
}
