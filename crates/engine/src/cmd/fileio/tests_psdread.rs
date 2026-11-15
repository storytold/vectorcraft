//! Reading Photoshop documents (#918): the merged image of every colour mode, depth and
//! compression, PSB files, transparency, the resolution, the clipping path, and damaged files
//! refused without a panic.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};

use super::psdread::{clipping_path, decode, is_psd, resolution};
use super::*;

/// A Photoshop document built from its parts: `version` 1 (PSD) or 2 (PSB), the header's fields,
/// then the colour mode data, resources, layer and mask section and image data as given.
#[allow(clippy::too_many_arguments)]
fn psd(
    version: u16,
    channels: u16,
    (w, h): (u32, u32),
    depth: u16,
    mode: u16,
    palette: &[u8],
    resources: &[u8],
    layers: &[u8],
    image: &[u8],
) -> Vec<u8> {
    let mut f = b"8BPS".to_vec();
    f.extend(version.to_be_bytes());
    f.extend([0; 6]);
    f.extend(channels.to_be_bytes());
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend(depth.to_be_bytes());
    f.extend(mode.to_be_bytes());
    for part in [palette, resources] {
        f.extend((part.len() as u32).to_be_bytes());
        f.extend(part);
    }
    if version == 2 {
        f.extend((layers.len() as u64).to_be_bytes());
    } else {
        f.extend((layers.len() as u32).to_be_bytes());
    }
    f.extend(layers);
    f.extend(image);
    f
}

/// Raw image data: the compression method (0), then the planes.
fn raw(planes: &[&[u8]]) -> Vec<u8> {
    let mut d = vec![0, 0];
    planes.iter().for_each(|p| d.extend(*p));
    d
}

/// A layer and mask section whose layer info holds just the layer `count` (negative: the merged
/// image's first extra channel is its transparency).
fn layer_count(count: i16, psb: bool) -> Vec<u8> {
    let mut s = if psb { 2u64.to_be_bytes().to_vec() } else { 2u32.to_be_bytes().to_vec() };
    s.extend(count.to_be_bytes());
    s
}

/// Straight sRGB through a naive CMYK conversion (tests don't depend on colour settings).
fn naive(c: [f32; 4]) -> [f32; 3] {
    [0, 1, 2].map(|i| (1.0 - c[i]) * (1.0 - c[3]))
}

fn pixels(f: &[u8]) -> Vec<[u8; 4]> {
    decode(f, 1 << 30, naive).unwrap().pixels().map(|p| p.0).collect()
}

#[test]
fn every_colour_mode_and_depth_reads() {
    // Grayscale, 16 bits.
    let g = psd(1, 1, (2, 1), 16, 1, &[], &[], &[], &raw(&[&[0, 0, 0xff, 0xff]]));
    assert!(is_psd(&g));
    assert_eq!(pixels(&g), [[0, 0, 0, 255], [255, 255, 255, 255]]);
    // Bitmap: a set bit is black.
    let b = psd(1, 1, (9, 1), 1, 0, &[], &[], &[], &raw(&[&[0b1000_0000, 0b1000_0000]]));
    let px = pixels(&b);
    assert_eq!((px[0], px[1], px[8]), ([0, 0, 0, 255], [255, 255, 255, 255], [0, 0, 0, 255]));
    // Indexed: 256 reds, greens, blues.
    let mut palette = vec![0; 768];
    (palette[1], palette[257], palette[513]) = (10, 20, 30);
    let i = psd(1, 1, (1, 1), 8, 2, &palette, &[], &[], &raw(&[&[1]]));
    assert_eq!(pixels(&i), [[10, 20, 30, 255]]);
    // RGB, 32 bits: linear values, shown through sRGB's curve.
    let half = 0.5f32.to_be_bytes();
    let r = psd(1, 3, (1, 1), 32, 3, &[], &[], &[], &raw(&[&half, &half, &1.0f32.to_be_bytes()]));
    assert_eq!(pixels(&r), [[188, 188, 255, 255]]);
    // CMYK, inks stored as 255 − amount: 100 % K is black, no ink white.
    let c = psd(1, 4, (2, 1), 8, 4, &[], &[], &[], &raw(&[&[255, 255], &[255, 255], &[255, 255], &[0, 255]]));
    assert_eq!(pixels(&c), [[0, 0, 0, 255], [255, 255, 255, 255]]);
    // Lab: L 100, a 0, b 0 is white.
    let l = psd(1, 3, (1, 1), 8, 9, &[], &[], &[], &raw(&[&[255], &[128], &[128]]));
    assert!(pixels(&l)[0].iter().all(|&v| v >= 253), "{:?}", pixels(&l));
    // Duotone keeps its grayscale data in the merged image.
    assert_eq!(pixels(&psd(1, 1, (1, 1), 8, 8, &[0; 40], &[], &[], &raw(&[&[64]]))), [[64, 64, 64, 255]]);
}

#[test]
fn transparency_comes_back_straight_and_extra_channels_without_it_stay_out() {
    // Half-transparent red, stored on white: (255, 127, 127) with alpha 128.
    let planes: [&[u8]; 4] = [&[255], &[127], &[127], &[128]];
    let with = psd(1, 4, (1, 1), 8, 3, &[], &[], &layer_count(-1, false), &raw(&planes));
    let [r, g, b, a] = pixels(&with)[0];
    assert!(r == 255 && g <= 1 && b <= 1 && a == 128, "{:?}", [r, g, b, a]);
    // A flat document's extra channel is a saved selection, not transparency.
    let flat = psd(1, 4, (1, 1), 8, 3, &[], &[], &[], &raw(&planes));
    assert_eq!(pixels(&flat), [[255, 127, 127, 255]]);
    // A 16-bit document says so in its Lr16 block (its layer info is empty).
    let mut layers = 0u32.to_be_bytes().to_vec();
    layers.extend(0u32.to_be_bytes());
    layers.extend(b"8BIMLr16");
    layers.extend(4u32.to_be_bytes());
    layers.extend((-1i16).to_be_bytes());
    layers.extend([0, 0]);
    let wide: [&[u8]; 4] = [&[0xff, 0xff], &[0x7f, 0x7f], &[0x7f, 0x7f], &[0x80, 0x80]];
    let deep = psd(1, 4, (1, 1), 16, 3, &[], &[], &layers, &raw(&wide));
    let [r, g, _, a] = pixels(&deep)[0];
    assert!(r == 255 && g <= 1 && a == 128, "{:?}", pixels(&deep));
}

#[test]
fn packbits_zip_and_psb_files_read() {
    // PackBits in a PSB: 4-byte row counts, transparency from a negative layer count.
    let mut rle = vec![0, 1];
    for _ in 0..4 {
        rle.extend(2u32.to_be_bytes());
    }
    for v in [200u8, 100, 50, 255] {
        rle.extend([0xfe, v]); // a run of three
    }
    let big = psd(2, 4, (3, 1), 8, 3, &[], &[], &layer_count(-1, true), &rle);
    assert_eq!(pixels(&big), [[200, 100, 50, 255]; 3]);
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "big.psb", "dataBase64": vectorcraft_format::base64_encode(&big)})).unwrap();
    assert_eq!(r["format"], "psb");
    // Zipped with prediction, 16 bits: each sample is the difference from the one before.
    let deltas: Vec<u8> = [0x1000u16, 0x1000, 0x1000].iter().flat_map(|d| d.to_be_bytes()).collect();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, &deltas).unwrap();
    let mut zip = vec![0, 3];
    zip.extend(z.finish().unwrap());
    let px = pixels(&psd(1, 1, (3, 1), 16, 1, &[], &[], &[], &zip));
    assert_eq!(px.iter().map(|p| p[0]).collect::<Vec<_>>(), [16, 32, 48]);
    // Zipped with prediction, 32 bits: the row's first bytes, then its second bytes and so on,
    // each byte stored as the difference from the byte before it.
    let samples = [0.0f32, 1.0, 0.0].map(f32::to_be_bytes);
    let planes: Vec<u8> = (0..4).flat_map(|k| samples.iter().map(move |s| s[k])).collect();
    let deltas: Vec<u8> = planes.iter().scan(0u8, |prev, &b| Some(b.wrapping_sub(std::mem::replace(prev, b)))).collect();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, &deltas).unwrap();
    let mut zip = vec![0, 3];
    zip.extend(z.finish().unwrap());
    let px = pixels(&psd(1, 1, (3, 1), 32, 1, &[], &[], &[], &zip));
    assert_eq!(px.iter().map(|p| p[0]).collect::<Vec<_>>(), [0, 255, 0]);
    // The same layout as another implementation writes it: psd-tools 1.26.1 (MIT,
    // <https://github.com/psd-tools/psd-tools>), `compression.compress(data,
    // Compression.ZIP_WITH_PREDICTION, 3, 2, 32)` of the samples 0, 1, 0.25 and 0.5, 0.125, 1,
    // reads as the same samples stored raw.
    let theirs = [
        0x78, 0x9c, 0x63, 0xb0, 0xff, 0x7f, 0xa8, 0x81, 0xa1, 0x81, 0x01, 0x04, 0xec, 0xff, 0x33, 0x1e, 0x64, 0x68, 0x80, 0xb0, 0x01, 0x59, 0xd1,
        0x06, 0x01,
    ];
    let mut zip = vec![0, 3];
    zip.extend(theirs);
    let samples: Vec<u8> = [0.0f32, 1.0, 0.25, 0.5, 0.125, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect();
    let stored = pixels(&psd(1, 1, (3, 2), 32, 1, &[], &[], &[], &raw(&[&samples])));
    assert_eq!(pixels(&psd(1, 1, (3, 2), 32, 1, &[], &[], &[], &zip)), stored);
    assert_eq!(stored.iter().map(|p| p[0]).collect::<Vec<_>>()[..2], [0, 255]);
}

#[test]
fn the_resolution_resource_sizes_the_placed_image() {
    // ResolutionInfo (1005): 300 ppi across, 118.11 per cm (300 ppi) down.
    let mut res = b"8BIM".to_vec();
    res.extend(0x03edu16.to_be_bytes());
    res.extend([0, 0]); // an empty name, padded
    res.extend(16u32.to_be_bytes());
    res.extend((300u32 << 16).to_be_bytes());
    res.extend([0, 1, 0, 1]);
    res.extend(((118.11f64 * 65536.0) as u32).to_be_bytes());
    res.extend([0, 2, 0, 1]);
    let f = psd(1, 3, (300, 150), 8, 3, &[], &res, &[], &raw(&[&[9; 45_000], &[9; 45_000], &[9; 45_000]]));
    let (x, y) = resolution(&f).unwrap();
    assert!((x - 300.0).abs() < 1e-9 && (y - 300.0).abs() < 0.01, "{x} {y}");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("file.place", &json!({"name": "photo.psd", "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
    assert!(
        (r["width"].as_f64().unwrap() - 72.0).abs() < 0.01 && (r["height"].as_f64().unwrap() - 36.0).abs() < 0.01,
        "an inch by half an inch: {r}"
    );
}

/// A 60 × 40 pt document: a red square on a transparent page, and a half-transparent blue one.
fn art() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 30, "height": 40})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s
}

fn exported(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &p).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn psd_exports_open_and_place_again() {
    let mut s = art();
    let png = image::load_from_memory(&exported(&mut s, json!({"format": "png"}))).unwrap().to_rgba8();
    for (options, what) in [(json!({}), "layered RGB"), (json!({"layers": false}), "flat RGB"), (json!({"colorModel": "gray"}), "grey")] {
        let f = exported(&mut s, merge(json!({"format": "psd"}), options));
        let back = decode(&f, 1 << 30, naive).unwrap();
        assert_eq!(back.dimensions(), png.dimensions(), "{what}");
        let (red, clear, blue) = (back.get_pixel(10, 20).0, back.get_pixel(35, 20).0, back.get_pixel(50, 20).0);
        if what == "layered RGB" {
            assert_eq!((red, clear[3]), ([255, 0, 0, 255], 0), "{what}");
            assert!(blue[2] >= 250 && blue[0] <= 3 && (blue[3] as i32 - png.get_pixel(50, 20).0[3] as i32).abs() <= 1, "{what}: {blue:?}");
        } else if what == "flat RGB" {
            assert_eq!((red, clear), ([255, 0, 0, 255], [255, 255, 255, 255]), "{what}: flat files are on white");
        } else {
            assert!(red[0] == red[1] && red[3] == 255, "{what}: {red:?}");
        }
        // Open and Place take it by its content, whatever its name.
        let r = s.execute("document.open", &json!({"name": "back.bin", "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
        assert_eq!(r["format"], "psd", "{what}");
        let d = &s.doc().unwrap().doc;
        assert!((d.artboards[0].rect.width() - 60.0).abs() < 1e-6, "{what}");
    }
    let r = s.execute("document.formats", &json!({})).unwrap();
    assert!(r["readable"].as_array().unwrap().iter().any(|v| v == "psd"));
    assert!(PLACE_EXTS.contains(&"psd") && PLACE_EXTS.contains(&"psb") && OPEN_EXTS.contains(&"psb"));
}

/// Photoshop files named `.pdd` open and place as PSD, in any case; the dialogs and a drop on the
/// window take them by their extension, and the desktop packaging doesn't associate them.
#[test]
fn pdd_files_open_and_place() {
    let mut s = art();
    let f = exported(&mut s, json!({"format": "psd"}));
    for name in ["art.pdd", "ART.PDD"] {
        assert_eq!(readable_format_for_name(name).map(|f| f.id), Some("psd"), "{name}");
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
        assert_eq!((&r["format"], &r["warnings"]), (&json!("psd"), &json!([])), "{name}: {r}");
        assert!((s.doc().unwrap().doc.artboards[0].rect.width() - 60.0).abs() < 1e-6, "{name}");
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let r = s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
        assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("psd"), Some(1)), "{name}: {r}");
    }
    assert!(OPEN_EXTS.contains(&"pdd") && PLACE_EXTS.contains(&"pdd") && UNASSOCIATED_EXTS.contains(&"pdd"));
    let photoshop = ("PSD", &["psd", "pdd"][..]);
    assert!(open_filters().any(|f| f == photoshop) && place_filters().any(|f| f == photoshop));
    assert!(!export_extensions().contains(&"pdd"), "exports still name their files .psd");
    assert!(format_for_name("x.pdd").is_none() && writable_format(None, Some("x.pdd")).is_err(), "an export to .pdd is refused as before");
}

/// An image resource block: `8BIM`, `id`, `name` as a Pascal string padded to an even size, then
/// `data` padded to an even size.
fn resource(id: u16, name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut r = b"8BIM".to_vec();
    r.extend(id.to_be_bytes());
    r.push(name.len() as u8);
    r.extend(name);
    if name.len().is_multiple_of(2) {
        r.push(0);
    }
    r.extend((data.len() as u32).to_be_bytes());
    r.extend(data);
    if !data.len().is_multiple_of(2) {
        r.push(0);
    }
    r
}

/// A path point record's 8 bytes: `[x, y]` as fractions of the image's width and height, stored
/// vertical first as signed 8.24 fixed-point numbers.
fn point([x, y]: [f64; 2]) -> Vec<u8> {
    [y, x].iter().flat_map(|v| ((v * f64::from(1 << 24)).round() as i32).to_be_bytes()).collect()
}

/// A saved path's records: the fill rule record, an initial fill record saying whether the fill
/// starts with all pixels, then each subpath (closed or not) of corner knots without handles.
fn saved_path(all_pixels: bool, subpaths: &[(bool, &[[f64; 2]])]) -> Vec<u8> {
    let record = |selector: u16, body: Vec<u8>| {
        let mut r = selector.to_be_bytes().to_vec();
        r.extend(body);
        r.resize(26, 0);
        r
    };
    let mut d = record(6, vec![]);
    d.extend(record(8, vec![0, u8::from(all_pixels)]));
    for (closed, knots) in subpaths {
        d.extend(record(if *closed { 0 } else { 3 }, (knots.len() as u16).to_be_bytes().to_vec()));
        for k in *knots {
            d.extend(record(if *closed { 2 } else { 5 }, [point(*k), point(*k), point(*k)].concat()));
        }
    }
    d
}

/// The clipping path resource (2999) naming the saved path `name`, with a flatness of 0 and the
/// fill rule left as it is.
fn clipping_name(name: &[u8]) -> Vec<u8> {
    let mut d = vec![name.len() as u8];
    d.extend(name);
    d.extend([0; 6]);
    resource(2999, b"", &d)
}

/// A 100 × 50 pixel red RGB document with `resources`, flat or with a layer count of 0.
fn red_psd(resources: &[u8]) -> Vec<u8> {
    psd(1, 3, (100, 50), 8, 3, &[], resources, &[], &raw(&[&[255; 5000], &[0; 5000], &[0; 5000]]))
}

/// The triangle saved as "Outline", another path saved as "Other", and the clipping path naming
/// "Outline".
fn triangle_resources() -> Vec<u8> {
    let other = saved_path(false, &[(true, &[[0.0, 0.0], [0.1, 0.0], [0.1, 0.1]])]);
    let outline = saved_path(false, &[(true, &[[0.5, 0.0], [1.0, 1.0], [0.0, 1.0]])]);
    [resource(2000, b"Other", &other), resource(2001, b"Outline", &outline), clipping_name(b"Outline")].concat()
}

/// The clip group a PSD file comes in as: its clipping path's anchors (pt), whether that path
/// uses the even-odd rule, and the image it clips.
fn clip_group(d: &Document, id: NodeId) -> (Vec<[f64; 2]>, bool, NodeId) {
    let Some(NodeKind::Group { children, clip: true }) = d.node(id).map(|n| &n.kind) else { panic!("not a clip group: {:?}", d.node(id)) };
    let [path, image] = &children[..] else { panic!("{} children", children.len()) };
    let NodeKind::Path { path: p, rule, clipping: true, .. } = &path.kind else { panic!("not a clipping path") };
    assert!(matches!(image.kind, NodeKind::Image(_)));
    let anchors = p.anchors().map(|(_, _, a)| [a.p.x, a.p.y]).collect();
    (anchors, *rule == vectorcraft_geom::FillRule::EvenOdd, image.id)
}

#[test]
fn the_clipping_path_clips_the_merged_image_on_place_and_open() {
    let f = red_psd(&triangle_resources());
    let triangle = vec![[50.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
    let data = vectorcraft_format::base64_encode(&f);
    // Place: one clip group of the named path, not the other, at 100 % (72 ppi: a pixel a point).
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let r = s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": data, "at": [50, 25]})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let (anchors, even_odd, image) = clip_group(&s.doc().unwrap().doc, id);
    assert_eq!((anchors, even_odd), (triangle.clone(), true));
    assert_eq!(r["image"], image.0, "{r}");
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(100.0), Some(50.0)));
    // Open with layers off, and a flat file opened with layers on, as its merged image.
    for params in [json!({"layers": false}), json!({})] {
        s.execute("document.open", &merge(json!({"name": "cut.psd", "dataBase64": data}), params.clone())).unwrap();
        let d = &s.doc().unwrap().doc;
        let top = d.layers[0].children().unwrap();
        assert_eq!(top.len(), 1, "{params}");
        assert_eq!(top[0].name.as_deref(), Some("cut.psd"), "{params}");
        let group = top[0].id;
        let (anchors, even_odd, _) = clip_group(d, group);
        assert_eq!((anchors, even_odd), (triangle.clone(), true), "{params}");
        // The corner outside the triangle draws nothing; its middle draws red.
        let png = image::load_from_memory(&exported(&mut s, json!({"format": "png"}))).unwrap().to_rgba8();
        assert_eq!((png.get_pixel(2, 2).0[3], png.get_pixel(50, 40).0), (0, [255, 0, 0, 255]), "{params}");
        // The clipping path is an ordinary path: releasing the clipping mask leaves the image whole.
        s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
        s.execute("object.clippingMask.release", &json!({})).unwrap();
        let png = image::load_from_memory(&exported(&mut s, json!({"format": "png"}))).unwrap().to_rgba8();
        assert_eq!(png.get_pixel(2, 2).0, [255, 0, 0, 255], "{params}");
    }
}

/// Replace keeps a clipped PSD file's scale, as it keeps an image's.
#[test]
fn replacing_a_clipped_psd_keeps_its_scale() {
    let data = vectorcraft_format::base64_encode(&red_psd(&triangle_resources()));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": data, "at": [200, 200]})).unwrap();
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    let r = s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": data, "replace": true})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(200.0), Some(100.0)), "{r}");
    let (anchors, _, _) = clip_group(&s.doc().unwrap().doc, NodeId(r["ids"][0].as_u64().unwrap()));
    assert_eq!(anchors, [[200.0, 150.0], [300.0, 250.0], [100.0, 250.0]]);
    // A plain image replacing the clip group keeps its scale too.
    let plain = vectorcraft_format::base64_encode(&red_psd(&[]));
    let r = s.execute("file.place", &json!({"name": "plain.psd", "dataBase64": plain, "replace": true})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(200.0), Some(100.0)), "{r}");
}

/// A plain image replacing a clipped PSD file takes the clipped image's scale and rotation.
#[test]
fn a_plain_image_replacing_a_clipped_psd_keeps_its_rotation() {
    let data = vectorcraft_format::base64_encode(&red_psd(&triangle_resources()));
    let plain = vectorcraft_format::base64_encode(&red_psd(&[]));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let r = s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": data, "at": [200, 200]})).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    s.execute("object.scale", &json!({"sx": 200, "sy": 150})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let xf = |s: &Session, id: NodeId| match s.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Image(im)) => im.xf.as_coeffs(),
        other => panic!("not an image: {other:?}"),
    };
    let (_, _, image) = clip_group(&s.doc().unwrap().doc, group);
    let before = xf(&s, image);
    let r = s.execute("file.place", &json!({"name": "plain.psd", "dataBase64": plain, "replace": true})).unwrap();
    let after = xf(&s, NodeId(r["ids"][0].as_u64().unwrap()));
    assert!(after.iter().zip(before).all(|(a, b)| (a - b).abs() < 1e-9), "{after:?} != {before:?}");
}

/// A square saved as "Square" and the clipping path naming it.
fn square_resources() -> Vec<u8> {
    let square = saved_path(false, &[(true, &[[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]])]);
    [resource(2000, b"Square", &square), clipping_name(b"Square")].concat()
}

/// `bytes` written as `name` in `dir`, by its path.
fn write_file(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

/// links.relink and links.update put a linked PSD file's image in the clip group of the clipping
/// path its new file names, take that clip group's clipping path away when the new file names
/// none, and give it back when a later file names one again.
#[test]
fn relink_and_update_follow_the_clipping_path_of_a_linked_psd() {
    let dir = vectorcraft_testkit::TempDir::new("psd-relink-clip");
    let cut = write_file(&dir, "cut.psd", &red_psd(&triangle_resources()));
    let square = write_file(&dir, "square.psd", &red_psd(&square_resources()));
    let plain = write_file(&dir, "plain.psd", &red_psd(&[]));
    let triangle = vec![[50.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
    let squared = vec![[25.0, 12.5], [75.0, 12.5], [75.0, 37.5], [25.0, 37.5]];
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let r = s.execute("file.place", &json!({"path": cut, "at": [50, 25]})).unwrap();
    let (group, image) = (NodeId(r["ids"][0].as_u64().unwrap()), r["image"].as_u64().unwrap());
    assert_eq!(clip_group(&s.doc().unwrap().doc, group), (triangle.clone(), true, NodeId(image)));
    s.execute("links.relink", &json!({"ids": [image], "path": square})).unwrap();
    assert_eq!(clip_group(&s.doc().unwrap().doc, group), (squared.clone(), true, NodeId(image)), "the new file's clipping path");
    s.execute("links.relink", &json!({"ids": [image], "path": plain})).unwrap();
    let d = &s.doc().unwrap().doc;
    match d.node(group).map(|n| &n.kind) {
        Some(NodeKind::Group { children, clip: false }) => assert_eq!(children.iter().map(|c| c.id.0).collect::<Vec<_>>(), [image]),
        other => panic!("a plain group of the image: {other:?}"),
    }
    s.execute("links.relink", &json!({"ids": [image], "path": cut})).unwrap();
    assert_eq!(clip_group(&s.doc().unwrap().doc, group), (triangle.clone(), true, NodeId(image)), "clipped again");
    // The file is saved again with another clipping path.
    std::fs::write(&cut, red_psd(&square_resources())).unwrap();
    s.execute("links.update", &json!({"ids": [image]})).unwrap();
    assert_eq!(clip_group(&s.doc().unwrap().doc, group), (squared, true, NodeId(image)), "links.update reads the clipping path again");
}

/// The PNG a PSD file's merged image is kept as holds the clipping path the file names, and only a
/// text chunk within the size limit that reads as a path gives one back.
#[test]
fn the_kept_png_holds_the_clipping_path_within_bounds() {
    let img = raster_image(&red_psd(&triangle_resources())).unwrap();
    let (path, rule) = img.clip.clone().expect("the file's clipping path");
    assert_eq!(rule, vectorcraft_geom::FillRule::EvenOdd);
    assert_eq!(super::load::png_clip(&img.blob.bytes), Some(path));
    let plain = raster_image(&red_psd(&[])).unwrap();
    assert!(plain.clip.is_none() && super::load::png_clip(&plain.blob.bytes).is_none());
    let with = |text: String| vectorcraft_render::encode::png::with_text(plain.blob.bytes.to_vec(), &[("VectorCraft clip", text.into())]);
    assert_eq!(super::load::png_clip(&with("not a path".into())), None);
    assert_eq!(super::load::png_clip(&with(" ".repeat((16 << 20) + 1))), None, "a chunk past the limit is not read");
}

/// A plain image relinked to a PSD file that names a clipping path goes into a clip group of that
/// path, under the even-odd rule; a clipping mask made in the document stays around it.
#[test]
fn a_plain_image_relinked_to_a_clipped_psd_is_clipped_by_its_path() {
    let dir = vectorcraft_testkit::TempDir::new("psd-relink-plain");
    let plain = write_file(&dir, "plain.psd", &red_psd(&[]));
    let cut = write_file(&dir, "cut.psd", &red_psd(&triangle_resources()));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let r = s.execute("file.place", &json!({"path": plain, "at": [50, 25]})).unwrap();
    let image = NodeId(r["ids"][0].as_u64().unwrap());
    s.execute("links.relink", &json!({"ids": [image.0], "path": cut})).unwrap();
    let d = &s.doc().unwrap().doc;
    let group = d.parent_of(image).expect("a clip group");
    assert_eq!(clip_group(d, group), (vec![[50.0, 0.0], [100.0, 50.0], [0.0, 50.0]], true, image));
    // The clipping path's fill rule changed in the document makes it the document's own mask: a
    // relink keeps it and puts the file's clip group inside it.
    let clip = match d.node(group).map(|n| &n.kind) {
        Some(NodeKind::Group { children, .. }) => children[0].id,
        other => panic!("{other:?}"),
    };
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(vectorcraft_doc::Node { kind: NodeKind::Path { rule, .. }, .. }) = doc.node_mut(clip) {
        *rule = vectorcraft_geom::FillRule::NonZero;
    }
    s.execute("links.relink", &json!({"ids": [image.0], "path": cut})).unwrap();
    let d = &s.doc().unwrap().doc;
    let inner = d.parent_of(image).unwrap();
    assert_eq!(d.parent_of(inner), Some(group), "the document's mask holds the file's clip group");
    assert!(clip_group(d, inner).1, "the file's clip group is even-odd");
}

#[test]
fn a_fill_that_starts_with_all_pixels_puts_the_image_around_the_path() {
    let hole = saved_path(true, &[(true, &[[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]])]);
    let f = red_psd(&[resource(2000, b"Hole", &hole), clipping_name(b"Hole")].concat());
    let path = clipping_path(&f).unwrap();
    let starts: Vec<_> = path.subpaths.iter().map(|s| (s.anchors[0].p.x, s.anchors[0].p.y, s.closed)).collect();
    assert_eq!(starts, [(0.0, 0.0, true), (25.0, 12.5, true)], "the image's edges, then the hole");
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "hole.psd", "dataBase64": vectorcraft_format::base64_encode(&f), "layers": false})).unwrap();
    let png = image::load_from_memory(&exported(&mut s, json!({"format": "png"}))).unwrap().to_rgba8();
    assert_eq!((png.get_pixel(2, 2).0, png.get_pixel(50, 25).0[3]), ([255, 0, 0, 255], 0));
}

#[test]
fn files_naming_no_clipping_path_or_a_missing_one_place_a_plain_image() {
    let other = saved_path(false, &[(true, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])]);
    let empty = saved_path(false, &[]);
    for (resources, what) in [
        (resource(2000, b"Other", &other), "saved paths, none named"),
        ([resource(2000, b"Other", &other), clipping_name(b"Gone")].concat(), "a name no path has"),
        ([resource(2000, b"Empty", &empty), clipping_name(b"Empty")].concat(), "a path with no knots"),
        ([resource(3000, b"Other", &other), clipping_name(b"Other")].concat(), "a resource outside the saved paths"),
        ([resource(2000, b"Other", &other), [&b"MeSa"[..], &clipping_name(b"Other")[4..]].concat()].concat(), "a block not signed 8BIM"),
        (vec![], "no resources"),
    ] {
        let f = red_psd(&resources);
        assert!(clipping_path(&f).is_none(), "{what}");
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"name": "a.psd", "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
        let id = NodeId(r["ids"][0].as_u64().unwrap());
        assert!(matches!(s.doc().unwrap().doc.node(id).map(|n| &n.kind), Some(NodeKind::Image(_))), "{what}");
        assert!(r.get("image").is_none(), "{what}: {r}");
    }
}

#[test]
fn damaged_clipping_paths_are_read_without_a_panic() {
    let good = red_psd(&triangle_resources());
    assert!(clipping_path(&good).is_some());
    // The header, the empty colour mode data, then the resources' length and the resources.
    let start = 26 + 4;
    let end = start + 4 + triangle_resources().len();
    for n in 0..end + 2 {
        let _ = clipping_path(&good[..n]);
    }
    for i in start..end {
        for v in [0, 1, 0x7f, 0x80, 0xff] {
            let mut f = good.clone();
            f[i] = v;
            let _ = clipping_path(&f);
        }
    }
    // Knots before any length record, and more knots than a length record counts, are left out.
    let mut extra = saved_path(false, &[(true, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])]);
    extra[26 * 2 + 3] = 2;
    let mut stray = 2u16.to_be_bytes().to_vec();
    stray.resize(26, 0);
    extra.splice(26..26, stray);
    let path = clipping_path(&red_psd(&[resource(2000, b"P", &extra), clipping_name(b"P")].concat())).unwrap();
    assert_eq!(path.subpaths.iter().map(|s| s.anchors.len()).collect::<Vec<_>>(), [2]);
}

#[test]
fn damaged_and_unsupported_files_are_refused_without_a_panic() {
    let good = psd(2, 4, (3, 2), 8, 3, &[], &[], &layer_count(-1, true), &raw(&[&[1; 6], &[2; 6], &[3; 6], &[4; 6]]));
    assert!(decode(&good, 1 << 30, naive).is_ok());
    for n in 0..good.len() {
        assert!(decode(&good[..n], 1 << 30, naive).is_err(), "cut at {n}");
        let _ = resolution(&good[..n]);
    }
    // Every byte changed in turn: an error or an image, never a panic.
    for i in 0..good.len() {
        for v in [0, 1, 0x7f, 0x80, 0xff] {
            let mut f = good.clone();
            f[i] = v;
            let _ = decode(&f, 1 << 30, naive);
            let _ = resolution(&f);
        }
    }
    // An image too large for the memory allowed is refused before anything is allocated.
    let huge = psd(2, 3, (300_000, 300_000), 16, 3, &[], &[], &[], &[0, 0]);
    assert!(decode(&huge, 1 << 30, naive).unwrap_err().contains("too large"));
    // Multichannel, and depths a mode doesn't have.
    assert!(decode(&psd(1, 2, (1, 1), 8, 7, &[], &[], &[], &raw(&[&[0], &[0]])), 1 << 30, naive).unwrap_err().contains("Multichannel"));
    assert!(decode(&psd(1, 4, (1, 1), 32, 4, &[], &[], &[], &[0, 0]), 1 << 30, naive).is_err());
    // A placed file with a damaged image reports it.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let cut = &good[..good.len() - 3];
    assert!(s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": vectorcraft_format::base64_encode(cut)})).is_err());
}
