//! Opening PSD and PSB files with their layers as layers (`document.open`'s `layers` and
//! `hiddenLayers`): names, visibility, opacity, blend modes, groups, layer masks, clipped layers
//! and blend modes VectorCraft lacks merged, 16-bit and PSB files, VectorCraft's own PSD export
//! read back, and damaged files read without a panic. Placing them with their layers as objects
//! (`file.place`'s `layers` and `hiddenLayers`), and `document.psdInfo`.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Node, NodeKind};

use super::*;

/// One layer record of a synthetic file.
#[derive(Clone)]
struct L {
    name: &'static str,
    /// Top, left, bottom, right.
    rect: [i32; 4],
    /// Red, green, blue and alpha, the same for every pixel.
    rgba: [u8; 4],
    /// The alpha of each pixel, in place of `rgba`'s, stored raw.
    alpha: Option<Vec<u8>>,
    blend: &'static [u8; 4],
    opacity: u8,
    clipped: bool,
    hidden: bool,
    divider: Option<u32>,
    /// Rect, default color, flags and the mask's pixels.
    mask: Option<([i32; 4], u8, u8, Vec<u8>)>,
    /// Extra tagged blocks.
    blocks: Vec<([u8; 4], Vec<u8>)>,
    /// Zipped channels: with prediction when `true`.
    zip: Option<bool>,
    /// The record lists no channels, whatever its rect.
    bare: bool,
    /// The Pascal name's bytes, with no Unicode name block.
    legacy: Option<&'static [u8]>,
}

fn layer(name: &'static str, rect: [i32; 4], rgba: [u8; 4]) -> L {
    L {
        name,
        rect,
        rgba,
        alpha: None,
        blend: b"norm",
        opacity: 255,
        clipped: false,
        hidden: false,
        divider: None,
        mask: None,
        blocks: vec![],
        zip: None,
        bare: false,
        legacy: None,
    }
}

fn group_end() -> L {
    L { divider: Some(3), ..layer("</Layer group>", [0; 4], [0; 4]) }
}

fn group(name: &'static str, blend: &'static [u8; 4]) -> L {
    L { divider: Some(1), blend, ..layer(name, [0; 4], [0; 4]) }
}

/// A tagged block, its length padded to even.
fn block(key: &[u8; 4], data: &[u8], psb: bool) -> Vec<u8> {
    let mut data = data.to_vec();
    data.resize(data.len().next_multiple_of(2), 0);
    let mut b = b"8BIM".to_vec();
    b.extend(key);
    if psb && matches!(key, b"Lr16" | b"Lr32") {
        b.extend((data.len() as u64).to_be_bytes());
    } else {
        b.extend((data.len() as u32).to_be_bytes());
    }
    b.extend(data);
    b
}

/// One row of a zipped channel with prediction: each sample stored as the difference from the one
/// before it, and in 32 bits the row's first bytes, then its second bytes and so on, each byte the
/// difference from the byte before it.
fn predict(row: &[u8], depth: u16) -> Vec<u8> {
    let bytewise = |b: &[u8]| -> Vec<u8> { b.iter().scan(0u8, |prev, &v| Some(v.wrapping_sub(std::mem::replace(prev, v)))).collect() };
    match depth {
        16 => {
            let mut prev = 0u16;
            row.chunks(2)
                .flat_map(|s| {
                    let v = u16::from_be_bytes([s[0], s[1]]);
                    v.wrapping_sub(std::mem::replace(&mut prev, v)).to_be_bytes()
                })
                .collect()
        }
        32 => bytewise(&(0..4).flat_map(|k| row.chunks(4).map(move |s| s[k])).collect::<Vec<_>>()),
        _ => bytewise(row),
    }
}

/// One sample of `v` at `depth` (in 32 bits, `v` / 255 as a float).
fn sample_of(v: u8, depth: u16) -> Vec<u8> {
    if depth == 32 { (f32::from(v) / 255.0).to_be_bytes().to_vec() } else { vec![v; usize::from(depth / 8)] }
}

/// A channel of `n` samples of `v` (see [`sample_of`]) at `depth`, raw, zipped (with prediction
/// when `zip` is `Some(true)`) or PackBits rows of `width` when `rle`.
fn channel(v: u8, n: usize, width: usize, depth: u16, rle: bool, psb: bool, zip: Option<bool>) -> Vec<u8> {
    let sample = sample_of(v, depth);
    let row: Vec<u8> = std::iter::repeat_n(sample.as_slice(), width).flatten().copied().collect();
    if let Some(predicted) = zip {
        let rows = n.checked_div(width).unwrap_or(0);
        let one = if predicted { predict(&row, depth) } else { row };
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        for _ in 0..rows {
            std::io::Write::write_all(&mut z, &one).unwrap();
        }
        let mut d = vec![0, if predicted { 3 } else { 2 }];
        d.extend(z.finish().unwrap());
        return d;
    }
    if !rle || n == 0 {
        let mut d = vec![0, 0];
        d.extend(std::iter::repeat_n(sample.as_slice(), n).flatten());
        return d;
    }
    // Each row as literal runs of at most 128 bytes.
    let mut packed = vec![];
    for run in row.chunks(128) {
        packed.push((run.len() - 1) as u8);
        packed.extend(run);
    }
    let rows = n / width;
    let mut d = vec![0, 1];
    for _ in 0..rows {
        if psb {
            d.extend((packed.len() as u32).to_be_bytes());
        } else {
            d.extend((packed.len() as u16).to_be_bytes());
        }
    }
    for _ in 0..rows {
        d.extend(&packed);
    }
    d
}

/// The layer info: the count (negative: the merged image has transparency), the records, then
/// their channels.
fn layer_info(layers: &[L], depth: u16, rle: bool, psb: bool) -> Vec<u8> {
    let mut records: Vec<u8> = vec![];
    let mut data: Vec<u8> = vec![];
    for l in layers {
        let [t, lf, b, r] = l.rect;
        let (w, h) = ((r - lf).max(0) as usize, (b - t).max(0) as usize);
        let mut channels: Vec<(i16, Vec<u8>)> = [(-1, l.rgba[3]), (0, l.rgba[0]), (1, l.rgba[1]), (2, l.rgba[2])]
            .iter()
            .filter(|_| !l.bare)
            .map(|&(id, v)| (id, channel(v, w * h, w, depth, rle, psb, l.zip)))
            .collect();
        if let (Some(pixels), Some((_, d))) = (&l.alpha, channels.first_mut()) {
            *d = vec![0, 0];
            for &p in pixels {
                d.extend(sample_of(p, depth));
            }
        }
        let mut mask = vec![];
        if let Some(([mt, ml, mb, mr], default, flags, pixels)) = &l.mask {
            let mut d = vec![0, 0];
            for &p in pixels {
                d.extend(sample_of(p, depth));
            }
            channels.push((-2, d));
            for v in [mt, ml, mb, mr] {
                mask.extend(v.to_be_bytes());
            }
            mask.extend([*default, *flags, 0, 0]);
        }
        for v in l.rect {
            records.extend(v.to_be_bytes());
        }
        records.extend((channels.len() as u16).to_be_bytes());
        for (id, d) in &channels {
            records.extend(id.to_be_bytes());
            if psb {
                records.extend((d.len() as u64).to_be_bytes());
            } else {
                records.extend((d.len() as u32).to_be_bytes());
            }
            data.extend(d);
        }
        records.extend(b"8BIM");
        records.extend(l.blend);
        records.extend([l.opacity, u8::from(l.clipped), 0x08 | if l.hidden { 2 } else { 0 }, 0]);
        let mut extra = (mask.len() as u32).to_be_bytes().to_vec();
        extra.extend(mask);
        extra.extend(0u32.to_be_bytes());
        let ascii = l.legacy.unwrap_or(l.name.as_bytes());
        extra.push(ascii.len() as u8);
        extra.extend(ascii);
        // The Pascal string padded to a multiple of 4 (what comes before it is too).
        extra.resize(extra.len().next_multiple_of(4), 0);
        if l.legacy.is_none() {
            let units: Vec<u16> = l.name.encode_utf16().collect();
            let mut luni = (units.len() as u32).to_be_bytes().to_vec();
            luni.extend(units.iter().flat_map(|u| u.to_be_bytes()));
            extra.extend(block(b"luni", &luni, psb));
        }
        if let Some(kind) = l.divider {
            extra.extend(block(b"lsct", &[&kind.to_be_bytes()[..], b"8BIM", l.blend].concat(), psb));
        }
        for (key, d) in &l.blocks {
            extra.extend(block(key, d, psb));
        }
        records.extend((extra.len() as u32).to_be_bytes());
        records.extend(extra);
    }
    let mut info = (-(layers.len() as i16)).to_be_bytes().to_vec();
    info.extend(records);
    info.extend(data);
    info.resize(info.len().next_multiple_of(2), 0);
    info
}

/// An RGB document of `w` × `h` pixels at 72 ppi holding `layers` (bottom first). 16- and 32-bit
/// documents keep their layers in an `Lr16` or `Lr32` block.
fn file(w: u32, h: u32, layers: &[L], depth: u16, rle: bool, psb: bool) -> Vec<u8> {
    let info = layer_info(layers, depth, rle, psb);
    let length = |n: usize, out: &mut Vec<u8>| {
        if psb {
            out.extend((n as u64).to_be_bytes());
        } else {
            out.extend((n as u32).to_be_bytes());
        }
    };
    let mut section = vec![];
    if depth == 8 {
        length(info.len(), &mut section);
        section.extend(info);
        section.extend(0u32.to_be_bytes());
    } else {
        length(0, &mut section);
        section.extend(0u32.to_be_bytes());
        section.extend(block(if depth == 32 { b"Lr32" } else { b"Lr16" }, &info, psb));
    }
    let mut f = b"8BPS".to_vec();
    f.extend(if psb { 2u16 } else { 1 }.to_be_bytes());
    f.extend([0; 6]);
    f.extend(4u16.to_be_bytes());
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend(depth.to_be_bytes());
    f.extend(3u16.to_be_bytes());
    f.extend(0u32.to_be_bytes());
    f.extend(0u32.to_be_bytes());
    length(section.len(), &mut f);
    f.extend(section);
    // The merged image: raw, mid gray, opaque.
    f.extend([0, 0]);
    let n = (w * h) as usize * usize::from(depth / 8);
    f.extend(std::iter::repeat_n(128, n * 3));
    f.extend(std::iter::repeat_n(255, n));
    f
}

/// The warning for one hidden layer left out.
const HIDDEN_1: &str = "1 layer was left out (hidden layers and the layers clipped to them)";

fn open(f: &[u8], p: Value) -> (Session, Value) {
    let mut s = Session::new();
    let r = s.execute("document.open", &merge(json!({"name": "layers.psd", "dataBase64": vectorcraft_format::base64_encode(f)}), p)).unwrap();
    (s, r)
}

fn name(n: &Node) -> &str {
    n.name.as_deref().unwrap_or("")
}

/// The layer's one image, named as the layer: its place (points) and its pixels.
fn image_of(s: &Session, l: &Node) -> ((f64, f64), image::RgbaImage) {
    let Some([c]) = l.children().map(Vec::as_slice) else { panic!("{} holds {:?}", name(l), l.children().map(Vec::len)) };
    assert_eq!(name(c), name(l));
    pixels_of(s, c)
}

/// An image's place (points) and its pixels.
fn pixels_of(s: &Session, c: &Node) -> ((f64, f64), image::RgbaImage) {
    let NodeKind::Image(img) = &c.kind else { panic!("{} isn't an image", name(c)) };
    let blob = &s.doc().unwrap().doc.images[&img.key];
    let px = craft_image_codecs::decode(&blob.bytes)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    assert_eq!((px.width(), px.height()), (img.width, img.height));
    let o = img.xf * kurbo::Point::ZERO;
    ((o.x, o.y), px)
}

/// Bottom, a pass-through group of a multiply layer at 50 % and a hidden layer, a screen group, and
/// a layer at 40 % fill opacity.
fn stack() -> Vec<L> {
    let mut fill = layer("Faded", [1, 1, 3, 3], [0, 0, 255, 255]);
    fill.blocks.push((*b"iOpa", vec![102]));
    vec![
        layer("Bottom", [0, 0, 8, 8], [255, 0, 0, 255]),
        group_end(),
        L { blend: b"mul ", opacity: 128, ..layer("Inner A", [0, 0, 4, 4], [0, 255, 0, 255]) },
        L { hidden: true, ..layer("Inner B", [4, 4, 8, 8], [0, 0, 255, 255]) },
        group("Folder", b"pass"),
        group_end(),
        layer("Lit", [2, 2, 6, 6], [10, 20, 30, 128]),
        group("Screens", b"scrn"),
        fill,
    ]
}

#[test]
fn layers_open_as_layers_with_their_names_visibility_opacity_and_blend_modes() {
    let (s, r) = open(&file(8, 8, &stack(), 8, false, false), json!({}));
    assert_eq!((&r["format"], &r["warnings"]), (&json!("psd"), &json!([HIDDEN_1])), "{r}");
    let d = &s.doc().unwrap().doc;
    assert!((d.artboards[0].rect.width() - 8.0).abs() < 1e-9 && (d.artboards[0].rect.height() - 8.0).abs() < 1e-9);
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Bottom", "Folder", "Screens", "Faded"], "bottom first");
    // The bottom layer's image covers the canvas in its color.
    let ((x, y), px) = image_of(&s, &d.layers[0]);
    assert_eq!(((x, y), px.dimensions(), px.get_pixel(3, 3).0), ((0.0, 0.0), (8, 8), [255, 0, 0, 255]));
    // A pass-through group: a layer of sublayers that blend with what is below; its hidden layer
    // is left out.
    let folder = &d.layers[1];
    assert!(matches!(folder.kind, NodeKind::Layer { .. }) && !folder.isolate && folder.blend == BlendMode::Normal);
    let inner: Vec<&str> = folder.children().unwrap().iter().map(|c| name(c)).collect();
    assert_eq!(inner, ["Inner A"]);
    let a = &folder.children().unwrap()[0];
    assert!(a.blend == BlendMode::Multiply && (a.opacity - 128.0 / 255.0).abs() < 1e-6 && a.visible);
    // Another group mode: isolated, in its mode; a layer keeps its transparency and place.
    let screens = &d.layers[2];
    assert!(screens.isolate && screens.blend == BlendMode::Screen);
    let ((x, y), px) = image_of(&s, &screens.children().unwrap()[0]);
    assert_eq!(((x, y), px.dimensions(), px.get_pixel(0, 0).0), ((2.0, 2.0), (4, 4), [10, 20, 30, 128]));
    // Fill opacity multiplies the opacity.
    assert!((d.layers[3].opacity - 102.0 / 255.0).abs() < 1e-6, "{}", d.layers[3].opacity);
    // Hidden layers come in hidden when asked for.
    let (s, r) = open(&file(8, 8, &stack(), 8, false, false), json!({"hiddenLayers": true}));
    assert_eq!(r["warnings"], json!([]), "{r}");
    let folder = &s.doc().unwrap().doc.layers[1];
    let inner: Vec<(&str, bool)> = folder.children().unwrap().iter().map(|c| (name(c), c.visible)).collect();
    assert_eq!(inner, [("Inner A", true), ("Inner B", false)]);
    // `layers: false` opens the merged image, as before.
    let (s, r) = open(&file(8, 8, &stack(), 8, false, false), json!({"layers": false}));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.layers.len(), d.layers[0].children().map(Vec::len)), (1, Some(1)), "{r}");
    assert_eq!(pixels_of(&s, &d.layers[0].children().unwrap()[0]).1.get_pixel(0, 0).0, [128, 128, 128, 255]);
    let bad = Session::new().execute("document.open", &json!({"name": "a.psd", "dataBase64": "AAAA", "hiddenLayers": "yes"}));
    assert!(bad.unwrap_err().to_string().contains("hiddenLayers must be true or false"));
}

#[test]
fn sixteen_bit_packbits_and_psb_layers_read() {
    for (depth, rle, psb) in [(16, false, false), (8, true, false), (8, true, true), (16, true, true)] {
        let (s, r) = open(&file(8, 8, &stack(), depth, rle, psb), json!({}));
        let d = &s.doc().unwrap().doc;
        let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
        assert_eq!(names, ["Bottom", "Folder", "Screens", "Faded"], "{depth} {rle} {psb}: {r}");
        let px = image_of(&s, &d.layers[2].children().unwrap()[0]).1;
        assert_eq!(px.get_pixel(1, 1).0, [10, 20, 30, 128], "{depth} {rle} {psb}");
    }
}

/// Zipped channels, with and without prediction, at 8, 16 and 32 bits (32-bit colors are linear,
/// so the test uses 0 and 1, which sRGB's curve keeps).
#[test]
fn zipped_and_32_bit_layers_read() {
    for depth in [8, 16, 32] {
        for zip in [None, Some(false), Some(true)] {
            if depth != 32 && zip.is_none() {
                continue;
            }
            let layers = [L { zip, ..layer("Back", [0, 0, 4, 4], [0, 0, 255, 255]) }, L { zip, ..layer("Front", [1, 1, 3, 4], [255, 0, 255, 128]) }];
            let (s, r) = open(&file(4, 4, &layers, depth, false, false), json!({}));
            let d = &s.doc().unwrap().doc;
            let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
            assert_eq!(names, ["Back", "Front"], "{depth} {zip:?}: {r}");
            let ((x, y), px) = image_of(&s, &d.layers[1]);
            assert_eq!(((x, y), px.dimensions(), px.get_pixel(2, 1).0), ((1.0, 1.0), (3, 2), [255, 0, 255, 128]), "{depth} {zip:?}");
        }
    }
}

/// A layer without a Unicode name takes the name its Pascal string holds, read as Mac Roman.
#[test]
fn names_without_a_unicode_name_read_as_mac_roman() {
    let l = L { legacy: Some(b"Caf\x8E\xCA\xD2Old\xD3\xDB"), ..layer("", [0, 0, 1, 1], [0, 0, 0, 255]) };
    let (s, r) = open(&file(1, 1, &[l], 8, false, false), json!({}));
    assert_eq!(name(&s.doc().unwrap().doc.layers[0]), "Café\u{a0}“Old”€", "{r}");
}

#[test]
fn layer_masks_become_opacity_masks() {
    let mut masked = layer("Masked", [0, 0, 8, 8], [255, 255, 255, 255]);
    masked.mask = Some(([2, 2, 4, 4], 0, 0, vec![255, 128, 0, 255]));
    let mut shown = layer("Shown outside", [0, 0, 8, 8], [0, 0, 0, 255]);
    shown.mask = Some(([0, 0, 1, 2], 255, 0, vec![0, 0]));
    let mut off = layer("Off", [0, 0, 8, 8], [0, 0, 0, 255]);
    off.mask = Some(([0, 0, 1, 1], 0, 2, vec![0]));
    // Flag bit 2: the mask's values and its default color are read inverted.
    let mut inverted = layer("Inverted", [0, 0, 8, 8], [0, 0, 0, 255]);
    inverted.mask = Some(([0, 0, 1, 2], 0, 4, vec![255, 64]));
    let (s, _) = open(&file(8, 8, &[masked, shown, off, inverted], 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let mask = |i: usize| d.layers[i].extras().mask.as_deref().expect("a mask");
    let m = mask(0);
    assert!(m.clip && !m.disabled, "outside a mask with default 0 is hidden");
    let ((x, y), px) = pixels_of(&s, &m.art);
    assert_eq!(((x, y), px.dimensions()), ((2.0, 2.0), (2, 2)));
    assert_eq!([px.get_pixel(0, 0).0, px.get_pixel(1, 0).0, px.get_pixel(0, 1).0], [[255, 255, 255, 255], [128, 128, 128, 255], [0, 0, 0, 255]]);
    assert!(!mask(1).clip, "outside a mask with default 255 shows");
    assert!(mask(2).disabled, "a disabled mask is kept, switched off");
    let m = mask(3);
    assert!(!m.clip, "an inverted default of 0 shows outside the mask");
    let px = pixels_of(&s, &m.art).1;
    assert_eq!([px.get_pixel(0, 0).0, px.get_pixel(1, 0).0], [[0, 0, 0, 255], [191, 191, 191, 255]]);
}

#[test]
fn clipped_layers_merge_into_the_layer_below() {
    let base = L { opacity: 200, ..layer("Base", [4, 4, 8, 8], [255, 255, 0, 255]) };
    let clipped = L { clipped: true, ..layer("Clipped", [0, 0, 8, 6], [0, 0, 255, 255]) };
    let hidden = L { clipped: true, hidden: true, ..layer("Gone", [0, 0, 8, 8], [0, 255, 0, 255]) };
    let (s, r) =
        open(&file(8, 8, &[layer("Under", [0, 0, 2, 2], [1, 2, 3, 255]), base, clipped, hidden], 8, false, false), json!({"hiddenLayers": true}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Under", "Base + Clipped"], "{r}");
    assert_eq!(r["warnings"], json!([HIDDEN_1]), "a hidden clipped layer is left out: {r}");
    // The base keeps its place, transparency and opacity; the clipped layer shows only on it.
    let merged = &d.layers[1];
    assert!((merged.opacity - 200.0 / 255.0).abs() < 1e-6);
    let ((x, y), px) = image_of(&s, merged);
    assert_eq!(((x, y), px.dimensions()), ((4.0, 4.0), (4, 4)));
    assert_eq!((px.get_pixel(0, 0).0, px.get_pixel(3, 3).0), ([0, 0, 255, 255], [255, 255, 0, 255]));
    // A base's layer mask stays the merged layer's opacity mask, over the unmasked pixels.
    let mut masked = layer("Masked", [0, 0, 4, 4], [255, 255, 0, 255]);
    masked.mask = Some(([0, 0, 1, 1], 0, 0, vec![255]));
    let clipped = L { clipped: true, ..layer("Clipped", [0, 0, 4, 4], [0, 0, 255, 255]) };
    let (s, r) = open(&file(4, 4, &[masked, clipped], 8, false, false), json!({}));
    let merged = &s.doc().unwrap().doc.layers[0];
    assert_eq!(name(merged), "Masked + Clipped", "{r}");
    assert!(merged.extras().mask.as_deref().is_some_and(|m| m.clip && !m.disabled), "{r}");
    let px = image_of(&s, merged).1;
    assert_eq!((px.dimensions(), px.get_pixel(3, 3).0), ((4, 4), [0, 0, 255, 255]));
    // A hidden base would need the merge: even with hidden layers asked for, it is left out with
    // the layers clipped to it.
    let hidden_base = L { hidden: true, ..layer("Hidden base", [0, 0, 4, 4], [255, 255, 0, 255]) };
    let clipped = L { clipped: true, ..layer("Clipped", [0, 0, 4, 4], [0, 0, 255, 255]) };
    let layers = [layer("Under", [0, 0, 2, 2], [1, 2, 3, 255]), hidden_base, clipped, layer("Over", [0, 0, 1, 1], [9, 9, 9, 255])];
    for hidden in [false, true] {
        let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": hidden}));
        let names: Vec<&str> = s.doc().unwrap().doc.layers.iter().map(|l| name(l)).collect();
        assert_eq!(names, ["Under", "Over"], "{hidden}: {r}");
        assert_eq!(r["warnings"], json!(["2 layers were left out (hidden layers and the layers clipped to them)"]), "{hidden}: {r}");
    }
}

/// A masked base merged with its clipped layers, then with a layer in a mode VectorCraft lacks,
/// still hides what its mask hides.
#[test]
fn a_merged_base_keeps_its_mask_in_a_later_merge() {
    let mut masked = layer("Masked", [0, 0, 4, 4], [255, 0, 0, 255]);
    masked.mask = Some(([0, 0, 1, 1], 0, 0, vec![255]));
    let clipped = L { clipped: true, ..layer("Clipped", [0, 0, 4, 4], [0, 0, 255, 255]) };
    let dodge = L { blend: b"lddg", ..layer("Dodge", [3, 3, 4, 4], [0, 0, 0, 255]) };
    let layers = [layer("Bottom", [0, 0, 4, 4], [0, 255, 0, 255]), masked, clipped, dodge];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(name(&d.layers[0]), "Bottom + Masked + Clipped + Dodge", "{r}");
    let px = image_of(&s, &d.layers[0]).1;
    // Inside the mask the clipped layer shows; outside it, the layer below.
    assert_eq!((px.get_pixel(0, 0).0, px.get_pixel(2, 2).0), ([0, 0, 255, 255], [0, 255, 0, 255]));
}

#[test]
fn layers_in_modes_vectorcraft_lacks_merge_with_the_layers_below() {
    let dodge = L { blend: b"lddg", ..layer("Dodge", [0, 0, 2, 2], [100, 100, 100, 255]) };
    let hidden = L { hidden: true, ..layer("Hidden", [0, 0, 8, 8], [0, 0, 0, 255]) };
    let layers = [layer("Bottom", [0, 0, 8, 8], [100, 50, 0, 255]), hidden, dodge, layer("Top", [6, 6, 8, 8], [0, 0, 0, 255])];
    let (s, r) = open(&file(8, 8, &layers, 8, false, false), json!({"hiddenLayers": true}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Bottom + Dodge", "Top"], "hidden layers a merge takes in are left out: {r}");
    let (_, px) = image_of(&s, &d.layers[0]);
    // Linear Dodge adds: 100 + 100, 50 + 100, 0 + 100.
    assert_eq!((px.get_pixel(0, 0).0, px.get_pixel(5, 5).0), ([200, 150, 100, 255], [100, 50, 0, 255]));
    assert!(r["warnings"].to_string().contains("Linear Dodge"), "{r}");
    // Darker Color keeps the color whose channels add up to less: (0, 0, 200) over (100, 50, 0)
    // has the larger total but the lower luminance, so the base stays.
    let darker = L { blend: b"dkCl", ..layer("Darker", [0, 0, 2, 2], [0, 0, 200, 255]) };
    let (s, _) = open(&file(8, 8, &[layer("Bottom", [0, 0, 8, 8], [100, 50, 0, 255]), darker], 8, false, false), json!({}));
    let (_, px) = image_of(&s, &s.doc().unwrap().doc.layers[0]);
    assert_eq!(px.get_pixel(0, 0).0, [100, 50, 0, 255]);
    // A hidden layer in such a mode would need the merge: left out, hidden layers asked for or not.
    let hidden_dodge = L { hidden: true, blend: b"lddg", ..layer("Hidden dodge", [0, 0, 2, 2], [100, 100, 100, 255]) };
    let (s, r) = open(&file(8, 8, &[layer("Bottom", [0, 0, 8, 8], [100, 50, 0, 255]), hidden_dodge], 8, false, false), json!({"hiddenLayers": true}));
    let names: Vec<&str> = s.doc().unwrap().doc.layers.iter().map(|l| name(l)).collect();
    assert_eq!((names, &r["warnings"]), (vec!["Bottom"], &json!([HIDDEN_1])), "{r}");
}

/// A merge whose canvases would need more memory than allowed leaves the layer in the Normal mode,
/// with a warning.
#[test]
fn a_merge_past_the_memory_allowed_leaves_the_layer_normal() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [layer("Bottom", [0, 0, 2, 2], [100, 50, 0, 255]), L { blend: b"lddg", ..layer("Dodge", [2, 2, 4, 4], [100, 100, 100, 255]) }];
    let f = file(4, 4, &layers, 8, false, false);
    // Each 2 × 2 layer fits 128 bytes (16 a pixel), and so does the first canvas of the merge with
    // the layer beside it; the 4 × 4 canvas of both doesn't.
    let (doc, warnings) = psdlayers::import(&f, false, 128, cms).unwrap().unwrap();
    let got: Vec<(&str, BlendMode)> = doc.layers.iter().map(|l| (name(l), l.blend)).collect();
    assert_eq!(got, [("Bottom", BlendMode::Normal), ("Dodge", BlendMode::Normal)]);
    assert_eq!(warnings, ["1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed"]);
    // With room for it, the merge is made.
    let (doc, _) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Bottom + Dodge"]);
    // A merge onto a canvas that already covers the layer holds both: two 4 × 4 canvases take 512
    // bytes, past 300, although the canvas of the two 1 × 1 layers below fits (288 bytes).
    let layers = [
        layer("Bottom", [0, 0, 1, 1], [100, 50, 0, 255]),
        layer("Corner", [3, 3, 4, 4], [0, 50, 100, 255]),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 4, 4], [100, 100, 100, 255]) },
    ];
    let f = file(4, 4, &layers, 8, false, false);
    let (doc, warnings) = psdlayers::import(&f, false, 300, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Bottom", "Corner", "Dodge"]);
    assert_eq!(warnings, ["1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed"]);
    let (doc, _) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Bottom + Corner + Dodge"]);
    // A group below whose layers each fit, but whose canvas (4 × 4 with both 1 × 1 layers held, 288
    // bytes) doesn't fit 100: the layers stay, and the layer comes in Normal.
    let layers = [
        group_end(),
        layer("Left", [0, 0, 1, 1], [100, 50, 0, 255]),
        layer("Right", [3, 3, 4, 4], [0, 50, 100, 255]),
        group("Pair", b"norm"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [100, 100, 100, 255]) },
    ];
    let f = file(4, 4, &layers, 8, false, false);
    let (doc, warnings) = psdlayers::import(&f, false, 100, cms).unwrap().unwrap();
    let got: Vec<(&str, BlendMode)> = doc.layers.iter().map(|l| (name(l), l.blend)).collect();
    assert_eq!(got, [("Pair", BlendMode::Normal), ("Dodge", BlendMode::Normal)]);
    assert_eq!(doc.layers[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Left", "Right"]);
    assert_eq!(warnings, ["1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed"]);
    let (doc, _) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Pair + Dodge"]);
    // A layer below whose pixels fit (128 bytes) but whose canvas (16 bytes a pixel, 256) doesn't
    // fit 200: the same.
    let layers = [layer("Big", [0, 0, 4, 4], [100, 50, 0, 255]), L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [100, 100, 100, 255]) }];
    let f = file(4, 4, &layers, 8, false, false);
    let (doc, warnings) = psdlayers::import(&f, false, 200, cms).unwrap().unwrap();
    let got: Vec<(&str, BlendMode)> = doc.layers.iter().map(|l| (name(l), l.blend)).collect();
    assert_eq!(got, [("Big", BlendMode::Normal), ("Dodge", BlendMode::Normal)]);
    assert_eq!(warnings, ["1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed"]);
    // A clipped layer whose merge doesn't fit (the base's canvas and its transparency, 20 bytes a
    // pixel, beside the clipped layer's canvas, 16: 576 bytes, past 300) comes in unclipped.
    let layers = [layer("Base", [0, 0, 4, 4], [255, 0, 0, 255]), L { clipped: true, ..layer("Clipped", [0, 0, 4, 4], [0, 0, 255, 255]) }];
    let f = file(4, 4, &layers, 8, false, false);
    let (doc, warnings) = psdlayers::import(&f, false, 300, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Base", "Clipped"]);
    assert_eq!(warnings, ["1 layer clipped to the layer below came in unclipped: the merge needs more memory than allowed"]);
    let (doc, _) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Base + Clipped"]);
}

/// A merge that can't be made counts nothing that the merges inside it counted: here the merge
/// in a group at half opacity fails inside the outer merge's attempt (the copy of what is below the
/// group takes the room) and succeeds when the group comes in.
#[test]
fn a_merge_that_cant_be_made_counts_only_itself() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [
        layer("Under", [0, 0, 2, 2], [100, 50, 0, 255]),
        group_end(),
        layer("Inner", [0, 0, 2, 2], [10, 10, 10, 255]),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 2, 2], [20, 20, 20, 255]) },
        L { opacity: 128, ..group("Pass", b"pass") },
        L { blend: b"diss", ..layer("Top", [0, 0, 2, 2], [0, 0, 0, 255]) },
    ];
    let (doc, warnings) = psdlayers::import(&file(2, 2, &layers, 8, false, false), false, 150, cms).unwrap().unwrap();
    let got: Vec<(&str, BlendMode)> = doc.layers.iter().map(|l| (name(l), l.blend)).collect();
    assert_eq!(got, [("Under", BlendMode::Normal), ("Pass", BlendMode::Normal), ("Top", BlendMode::Normal)]);
    assert_eq!(doc.layers[1].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Inner + Dodge"]);
    assert_eq!(
        warnings,
        [
            "layers in blend modes VectorCraft doesn't have (Linear Dodge) were merged with the layers below them",
            "1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed",
        ]
    );
}

/// Layers in modes VectorCraft lacks, one after another, merge onto the canvas the first merge made
/// without copying it: 300 of them over a 64 × 64 canvas (64 KiB) open within an allowance that a
/// copy for each would pass. Merges that need a copy of that canvas each, past the allowance, open
/// the merged image instead.
#[test]
fn merges_onto_a_merge_reuse_its_canvas() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let dodge = |rect: [i32; 4]| L { blend: b"lddg", ..layer("d", rect, [1, 1, 1, 255]) };
    let mut layers = vec![layer("Bottom", [0, 0, 1, 1], [0, 0, 0, 255]), dodge([63, 63, 64, 64])];
    layers.extend((0..300).map(|_| dodge([1, 1, 2, 2])));
    let (doc, warnings) = psdlayers::import(&file(64, 64, &layers, 8, false, false), false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!((doc.layers.len(), warnings.len()), (1, 1), "{warnings:?}");
    assert!(name(&doc.layers[0]).starts_with("Bottom + d + d") && name(&doc.layers[0]).matches(" + d").count() == 301);
    // A Normal layer between each pair: every merge copies the 64 KiB canvas, and 150 of them pass
    // eight times the allowance of 1 MiB.
    let mut layers = vec![layer("Bottom", [0, 0, 1, 1], [0, 0, 0, 255]), dodge([63, 63, 64, 64])];
    for _ in 0..150 {
        layers.extend([layer("n", [1, 1, 2, 2], [1, 1, 1, 255]), dodge([2, 2, 3, 3])]);
    }
    let e = psdlayers::import(&file(64, 64, &layers, 8, false, false), false, 1 << 20, cms).err();
    assert!(e.as_deref().is_some_and(|e| e.contains("take more memory to read than allowed")), "{e:?}");
}

/// A Dissolve layer at half opacity merges as pixels of its own color or of the color below,
/// about half of each, and none in between.
#[test]
fn dissolve_layers_merge_as_opaque_or_transparent_pixels() {
    let dissolve = L { blend: b"diss", opacity: 128, ..layer("Dissolve", [0, 0, 16, 16], [255, 0, 0, 255]) };
    let layers = [layer("Bottom", [0, 0, 16, 16], [255, 255, 255, 255]), dissolve];
    let (s, r) = open(&file(16, 16, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Bottom + Dissolve"], "{r}");
    let (_, px) = image_of(&s, &d.layers[0]);
    let red = px.pixels().filter(|p| p.0 == [255, 0, 0, 255]).count();
    let white = px.pixels().filter(|p| p.0 == [255, 255, 255, 255]).count();
    assert_eq!(red + white, 256, "every pixel is the layer's color or the color below");
    assert!((96..=160).contains(&red), "{red} of 256 pixels are the layer's");
    // A second Dissolve layer at half opacity over the first has its own pattern: about a quarter
    // of the pixels still show the first.
    let blue = L { blend: b"diss", opacity: 128, ..layer("Blue", [0, 0, 16, 16], [0, 0, 255, 255]) };
    let layers = [layers[0].clone(), layers[1].clone(), blue];
    let (s, r) = open(&file(16, 16, &layers, 8, false, false), json!({}));
    let (_, px) = image_of(&s, &s.doc().unwrap().doc.layers[0]);
    let red = px.pixels().filter(|p| p.0 == [255, 0, 0, 255]).count();
    assert!((32..=96).contains(&red), "{red} of 256 pixels show the lower layer: {r}");
}

/// A layer with a Knockout value of `knko` and a fill opacity of `fill`.
fn knockout(l: L, knko: u8, fill: u8) -> L {
    let mut l = l;
    l.blocks.push((*b"knko", vec![knko, 0, 0, 0]));
    l.blocks.push((*b"iOpa", vec![fill]));
    l
}

/// A green layer under a group of `group_blend` holding a red layer and `cut` above it.
fn knockout_stack(group_blend: &'static [u8; 4], cut: L) -> Vec<L> {
    vec![
        layer("Outside", [0, 0, 4, 4], [0, 255, 0, 255]),
        group_end(),
        layer("Lower", [0, 0, 4, 4], [255, 0, 0, 255]),
        cut,
        group("Group", group_blend),
    ]
}

/// The names of the document's layers, and of the layers in its second one.
fn knockout_names(s: &Session) -> (Vec<String>, Vec<String>) {
    let d = &s.doc().unwrap().doc;
    let names = |ls: &[std::sync::Arc<Node>]| ls.iter().map(|l| name(l).to_string()).collect::<Vec<_>>();
    (names(&d.layers), d.layers.get(1).and_then(|g| g.children()).map_or(vec![], |c| names(c)))
}

/// A layer with a shallow knockout in a group merges with the layers below it in the group: its
/// shape clears them to transparency, it draws there at its fill opacity, and its opacity fades
/// that. What is outside the group stays a layer of its own and shows through.
#[test]
fn knockout_layers_merge_with_the_layers_below_them_in_their_group() {
    let cut = |fill: u8, opacity: u8| knockout(L { opacity, ..layer("Cut", [0, 0, 2, 2], [0, 0, 255, 255]) }, 1, fill);
    for (fill, opacity, at_cut) in [(0, 255, [0, 0, 0, 0]), (128, 255, [0, 0, 255, 128]), (0, 128, [255, 0, 0, 127]), (255, 255, [0, 0, 255, 255])] {
        for group_blend in [b"norm", b"pass"] {
            let (s, r) = open(&file(4, 4, &knockout_stack(group_blend, cut(fill, opacity)), 8, false, false), json!({}));
            assert_eq!(r["warnings"], json!([]), "{r}");
            let (top, inner) = knockout_names(&s);
            assert_eq!((top, inner), (vec!["Outside".to_string(), "Group".into()], vec!["Lower + Cut".to_string()]), "{fill} {opacity}");
            let d = &s.doc().unwrap().doc;
            assert_eq!(d.layers[1].isolate, group_blend == b"norm");
            let ((x, y), px) = image_of(&s, &d.layers[1].children().unwrap()[0]);
            assert_eq!(((x, y), px.dimensions()), ((0.0, 0.0), (4, 4)));
            assert_eq!((px.get_pixel(1, 1).0, px.get_pixel(3, 3).0), (at_cut, [255, 0, 0, 255]), "{fill} {opacity}");
        }
    }
    // A layer mask shapes the cut: where it hides the layer, nothing below is cleared.
    let mut masked = cut(0, 255);
    masked.mask = Some(([0, 0, 1, 1], 255, 0, vec![0]));
    let (s, r) = open(&file(4, 4, &knockout_stack(b"norm", masked), 8, false, false), json!({}));
    let px = image_of(&s, &s.doc().unwrap().doc.layers[1].children().unwrap()[0]).1;
    assert_eq!((px.get_pixel(0, 0).0, px.get_pixel(1, 1).0), ([255, 0, 0, 255], [0, 0, 0, 0]), "{r}");
    // Without a knockout (0), the layers stay apart.
    let (s, r) =
        open(&file(4, 4, &knockout_stack(b"norm", knockout(layer("Cut", [0, 0, 2, 2], [0, 0, 255, 255]), 0, 0)), 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower".to_string(), "Cut".into()], &json!([])), "{r}");
}

/// The knockouts this import doesn't apply come in as layers, counted in a warning: at the top level,
/// on a clipped layer, and with a Knockout value other than 1. Hidden ones that would need the merge
/// are left out; one that draws nothing, or nothing below it, merges nothing.
#[test]
fn knockouts_that_arent_applied_are_said() {
    const UNAPPLIED: &str = "the knockout of 1 layer wasn't applied";
    let cut = |l: L, knko: u8| knockout(l, knko, 0);
    let blue = || layer("Cut", [0, 0, 2, 2], [0, 0, 255, 255]);
    // At the top level.
    let layers = [layer("Lower", [0, 0, 4, 4], [255, 0, 0, 255]), cut(blue(), 1)];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).0, &r["warnings"]), (vec!["Lower".to_string(), "Cut".into()], &json!([UNAPPLIED])), "{r}");
    // Another Knockout value, and a clipped layer, in a group.
    let (s, r) = open(&file(4, 4, &knockout_stack(b"norm", cut(blue(), 2)), 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower".to_string(), "Cut".into()], &json!([UNAPPLIED])), "{r}");
    let (s, r) = open(&file(4, 4, &knockout_stack(b"norm", cut(L { clipped: true, ..blue() }, 1)), 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower + Cut".to_string()], &json!([UNAPPLIED])), "{r}");
    // In a pass-through group, a layer below it, or the knockout itself, in another mode blends with
    // what is outside the group: the layers stay apart, and the lower one still multiplies the green
    // outside. In an isolated group they merge.
    let mut multiply = knockout_stack(b"pass", cut(blue(), 1));
    multiply[2].blend = b"mul ";
    let (s, r) = open(&file(4, 4, &multiply, 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower".to_string(), "Cut".into()], &json!([UNAPPLIED])), "{r}");
    assert_eq!(s.doc().unwrap().doc.layers[1].children().unwrap()[0].blend, BlendMode::Multiply);
    let (s, r) = open(&file(4, 4, &knockout_stack(b"pass", cut(L { blend: b"mul ", ..blue() }, 1)), 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower".to_string(), "Cut".into()], &json!([UNAPPLIED])), "{r}");
    multiply[4] = group("Group", b"norm");
    let (s, r) = open(&file(4, 4, &multiply, 8, false, false), json!({}));
    assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower + Cut".to_string()], &json!([])), "{r}");
    // A hidden knockout would need the merge: left out, hidden layers asked for or not.
    for hidden in [false, true] {
        let (s, r) =
            open(&file(4, 4, &knockout_stack(b"norm", cut(L { hidden: true, ..blue() }, 1)), 8, false, false), json!({"hiddenLayers": hidden}));
        assert_eq!((knockout_names(&s).1, &r["warnings"]), (vec!["Lower".to_string()], &json!([HIDDEN_1])), "{hidden}: {r}");
    }
    // One that stores no pixels or only transparent ones cuts nothing, and merges nothing.
    for (l, inner) in [(L { bare: true, ..blue() }, vec!["Lower"]), (layer("Cut", [0, 0, 2, 2], [0, 0, 255, 0]), vec!["Lower", "Cut"])] {
        let (s, r) = open(&file(4, 4, &knockout_stack(b"norm", cut(l, 1)), 8, false, false), json!({}));
        assert_eq!((knockout_names(&s).1, &r["warnings"]), (inner.iter().map(|n| n.to_string()).collect(), &json!([])), "{r}");
    }
    // Nothing below it in its group: it comes in as a layer.
    let layers = [group_end(), cut(blue(), 1), group("Group", b"norm"), layer("Over", [0, 0, 1, 1], [9, 9, 9, 255])];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Cut"], "{r}");
    assert_eq!(r["warnings"], json!([]), "{r}");
}

/// A knockout that changes nothing below it leaves the layers apart, hidden ones included: one at 0
/// opacity, one at 0 fill over layers that are transparent or beside their painted pixels, and one
/// whose layer mask hides all of it. At 0 fill over painted layers, and
/// at a fill over transparent ones, it still merges. A knockout at 0 fill over a transparent layer
/// stays apart without a warning when their merge would need more memory than allowed.
#[test]
fn knockouts_that_change_nothing_keep_the_layers_apart() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [
        group_end(),
        layer("Lower", [0, 0, 2, 2], [255, 0, 0, 0]),
        knockout(layer("Cut", [2, 2, 4, 4], [0, 0, 255, 255]), 1, 0),
        group("Group", b"norm"),
    ];
    // The two 2 × 2 layers drawn together fit 256 bytes, but not the 4 × 4 canvas covering both.
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 256, cms).unwrap().unwrap();
    assert_eq!(doc.layers[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Lower", "Cut"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let lower = |alpha: u8| layer("Lower", [0, 0, 4, 4], [255, 0, 0, alpha]);
    let hidden = L { hidden: true, ..layer("Hidden", [0, 0, 4, 4], [0, 255, 0, 255]) };
    let cut = |fill: u8, opacity: u8| knockout(L { opacity, ..layer("Cut", [0, 0, 2, 2], [0, 0, 255, 255]) }, 1, fill);
    let inner = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(vec![], |c| c.iter().map(|l| name(l).to_string()).collect::<Vec<_>>());
    for (alpha, fill, opacity, apart) in [(255, 0, 0, true), (0, 0, 255, true), (255, 0, 255, false), (0, 128, 255, false)] {
        let layers = [group_end(), lower(alpha), hidden.clone(), cut(fill, opacity), group("Group", b"norm")];
        let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": true}));
        let (names, warnings) = if apart { (vec!["Lower", "Hidden", "Cut"], json!([])) } else { (vec!["Lower + Cut"], json!([HIDDEN_1])) };
        assert_eq!((inner(&s), &r["warnings"]), (names.iter().map(|n| n.to_string()).collect(), &warnings), "{alpha} {fill} {opacity}: {r}");
    }
    // A hidden knockout that needs no merge comes in with hidden layers: at 0 opacity, with nothing
    // below it in its group, at 0 fill beside the painted pixels below it, and with a layer mask that
    // hides all of it.
    let hidden_cut = |opacity: u8| L { hidden: true, ..cut(0, opacity) };
    let beside = L { hidden: true, ..knockout(layer("Cut", [0, 0, 2, 2], [0, 0, 255, 255]), 1, 0) };
    let masked = L { hidden: true, mask: Some(([0, 0, 2, 2], 0, 0, vec![0; 4])), ..cut(255, 255) };
    for (layers, names) in [
        (vec![group_end(), lower(255), hidden_cut(0), group("Group", b"norm")], vec!["Lower", "Cut"]),
        (vec![group_end(), hidden_cut(255), group("Group", b"norm")], vec!["Cut"]),
        (vec![group_end(), layer("Lower", [2, 2, 4, 4], [255, 0, 0, 255]), beside, group("Group", b"norm")], vec!["Lower", "Cut"]),
        (vec![group_end(), lower(255), masked.clone(), group("Group", b"norm")], vec!["Lower", "Cut"]),
    ] {
        let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": true}));
        assert_eq!((inner(&s), &r["warnings"]), (names.iter().map(|n| n.to_string()).collect(), &json!([])), "{r}");
    }
    // With the mask switched off it would need the merge, and is left out.
    let unmasked = L { mask: Some(([0, 0, 2, 2], 0, 2, vec![0; 4])), ..masked };
    let (s, r) = open(&file(4, 4, &[group_end(), lower(255), unmasked, group("Group", b"norm")], 8, false, false), json!({"hiddenLayers": true}));
    assert_eq!((inner(&s), &r["warnings"]), (vec!["Lower".to_string()], &json!([HIDDEN_1])), "{r}");
    // A hidden knockout group that would need the merge is counted once, as a hidden layer left out:
    // its type layer and its own hidden layer aren't counted.
    let mut words = layer("Words", [0, 0, 2, 2], [0, 0, 0, 255]);
    words.blocks.push((*b"TySh", vec![0; 4]));
    let layers = [
        group_end(),
        lower(255),
        group_end(),
        words.clone(),
        L { hidden: true, ..layer("Inner", [0, 0, 2, 2], [0, 0, 255, 255]) },
        L { hidden: true, ..knockout(group("Cut", b"norm"), 1, 255) },
        group("Group", b"norm"),
    ];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": true}));
    assert_eq!((inner(&s), &r["warnings"]), (vec!["Lower".to_string()], &json!([HIDDEN_1])), "{r}");
    // One whose layer mask hides all of it needs no merge, but a knockout above it merges the layers
    // and leaves it out: it is counted once, without its type layer.
    let layers = [
        group_end(),
        lower(255),
        group_end(),
        words,
        L { hidden: true, mask: Some(([0, 0, 2, 2], 0, 0, vec![0; 4])), ..knockout(group("Masked", b"norm"), 1, 255) },
        cut(0, 255),
        group("Group", b"norm"),
    ];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": true}));
    assert_eq!((inner(&s), &r["warnings"]), (vec!["Lower + Cut".to_string()], &json!([HIDDEN_1])), "{r}");
}

/// With hidden layers asked for, the hidden layers of a group drawn for a knockout or a clip that
/// isn't made come in, and aren't counted as left out: a knockout group over a layer that stores no
/// pixels, and a clipped group whose merge needs more memory than allowed.
#[test]
fn hidden_layers_of_a_group_drawn_for_a_merge_not_made_come_in() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let children = |l: &Node| l.children().map_or(vec![], |c| c.iter().map(|l| (name(l).to_string(), l.visible)).collect::<Vec<_>>());
    let shown_and_hidden =
        || [group_end(), layer("Shown", [0, 0, 2, 2], [0, 0, 255, 255]), L { hidden: true, ..layer("Hidden", [2, 2, 4, 4], [0, 255, 0, 255]) }];
    let both = vec![("Shown".to_string(), true), ("Hidden".to_string(), false)];
    // A knockout group over a layer that stores no pixels.
    let mut layers = vec![group_end(), L { bare: true, ..layer("Lower", [0, 0, 2, 2], [0; 4]) }];
    layers.extend(shown_and_hidden());
    layers.extend([knockout(group("Cut", b"norm"), 1, 255), group("Group", b"norm")]);
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({"hiddenLayers": true}));
    let d = &s.doc().unwrap().doc;
    let cut = &d.layers[0].children().unwrap()[0];
    assert_eq!((name(cut), children(cut), &r["warnings"]), ("Cut", both.clone(), &json!([])), "{r}");
    // A clipped group whose merge with its base doesn't fit 320 bytes.
    let mut layers = vec![layer("Base", [0, 0, 2, 2], [255, 0, 0, 255])];
    layers.extend(shown_and_hidden());
    layers[2].rect = [0, 0, 4, 4];
    layers.push(L { clipped: true, ..group("Clipped", b"norm") });
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), true, 320, cms).unwrap().unwrap();
    let clipped = &doc.layers[1];
    assert_eq!((name(clipped), children(clipped)), ("Clipped", both), "{warnings:?}");
    assert_eq!(warnings, ["1 layer clipped to the layer below came in unclipped: the merge needs more memory than allowed"]);
}

/// A knockout's merge whose canvases would need more memory than allowed leaves the layers apart,
/// with a warning. One at 0 fill beside the painted pixels below it cuts nothing, and stays apart
/// whatever the memory.
#[test]
fn a_knockout_merge_past_the_memory_allowed_leaves_the_layers_apart() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let stack = |cut: [i32; 4]| {
        let layers = [
            group_end(),
            layer("Lower", [0, 0, 2, 2], [255, 0, 0, 255]),
            knockout(layer("Cut", cut, [0, 0, 255, 255]), 1, 0),
            group("Group", b"norm"),
        ];
        file(4, 4, &layers, 8, false, false)
    };
    let names = |doc: &vectorcraft_doc::Document| doc.layers[0].children().unwrap().iter().map(|c| name(c).to_string()).collect::<Vec<_>>();
    let f = stack([1, 1, 3, 3]);
    // Each 2 × 2 layer fits 128 bytes (16 a pixel), but not the canvas of the layer below with the
    // knockout's canvas held beside it.
    let (doc, warnings) = psdlayers::import(&f, false, 128, cms).unwrap().unwrap();
    assert_eq!(names(&doc), ["Lower", "Cut"]);
    assert_eq!(warnings, ["the knockout of 1 layer wasn't applied"]);
    let (doc, warnings) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(names(&doc), ["Lower + Cut"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    for max_alloc in [128, 1 << 20] {
        let (doc, warnings) = psdlayers::import(&stack([2, 2, 4, 4]), false, max_alloc, cms).unwrap().unwrap();
        assert_eq!(names(&doc), ["Lower", "Cut"], "{max_alloc}");
        assert!(warnings.is_empty(), "{max_alloc}: {warnings:?}");
    }
}

/// A knockout that cuts nothing gives back what drawing the layers below it counted: here a
/// knockout at 0 fill beside a group, whose own knockout doesn't fit 192 bytes while the outer
/// knockout's canvas is held, but merges when the group comes in.
#[test]
fn a_knockout_that_cuts_nothing_doesnt_keep_what_drawing_below_it_counted() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [
        group_end(),
        group_end(),
        layer("Opaque", [0, 0, 2, 2], [255, 0, 0, 255]),
        knockout(layer("Inner cut", [0, 0, 1, 2], [0, 0, 255, 255]), 1, 0),
        group("Inner", b"norm"),
        knockout(layer("Outer cut", [2, 2, 4, 4], [0, 0, 255, 255]), 1, 0),
        group("Outer", b"norm"),
    ];
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 192, cms).unwrap().unwrap();
    let outer = doc.layers[0].children().unwrap();
    assert_eq!(outer.iter().map(|c| name(c)).collect::<Vec<_>>(), ["Inner", "Outer cut"], "{warnings:?}");
    assert_eq!(outer[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Opaque + Inner cut"], "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A knockout merge that can't be made leaves the import the whole allowance for reading the layers
/// apart: here the merge decodes 22 layers over one another before the layer beside them makes its
/// canvas too large, which used up nearly all of eight times 64 KiB. The merge has that allowance
/// also after five layers below the group were read, and a knockout at 0 fill over 22 transparent
/// layers, which cuts no paint after they are drawn, leaves the same room.
#[test]
fn a_knockout_merge_not_made_leaves_room_to_read_the_layers() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let mut layers = vec![group_end()];
    layers.extend((0..22).map(|_| layer("Lower", [0, 0, 32, 32], [255, 0, 0, 255])));
    layers.push(layer("Beside", [0, 64, 32, 96], [0, 255, 0, 255]));
    layers.extend([knockout(layer("Cut", [0, 0, 32, 32], [0, 0, 255, 255]), 1, 0), group("Group", b"norm")]);
    let (doc, warnings) = psdlayers::import(&file(96, 32, &layers, 8, false, false), false, 64 << 10, cms).unwrap().unwrap();
    assert_eq!(doc.layers[0].children().unwrap().len(), 24, "{warnings:?}");
    assert_eq!(warnings, ["the knockout of 1 layer wasn't applied"]);
    let mut after_five = vec![layer("Outside", [0, 0, 32, 32], [0, 0, 0, 255]); 5];
    after_five.extend(layers);
    let (doc, warnings) = psdlayers::import(&file(96, 32, &after_five, 8, false, false), false, 64 << 10, cms).unwrap().unwrap();
    assert_eq!((doc.layers.len(), doc.layers[5].children().unwrap().len()), (6, 24), "{warnings:?}");
    assert_eq!(warnings, ["the knockout of 1 layer wasn't applied"]);
    let mut layers = vec![group_end()];
    layers.extend((0..22).map(|_| layer("Lower", [0, 0, 32, 32], [255, 0, 0, 0])));
    layers.extend([knockout(layer("Cut", [0, 0, 32, 32], [0, 0, 255, 255]), 1, 0), group("Group", b"norm")]);
    let (doc, warnings) = psdlayers::import(&file(32, 32, &layers, 8, false, false), false, 64 << 10, cms).unwrap().unwrap();
    assert_eq!(doc.layers[0].children().unwrap().len(), 23, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A hidden knockout in a mode VectorCraft lacks is left out without being drawn, hidden layers
/// asked for: here its pixels are larger than the memory allowed for one image.
#[test]
fn a_hidden_knockout_in_a_mode_vectorcraft_lacks_is_left_out_unread() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [
        group_end(),
        layer("Lower", [0, 0, 2, 2], [255, 0, 0, 255]),
        L { hidden: true, blend: b"diss", ..knockout(layer("Cut", [0, 0, 8, 8], [0, 0, 255, 255]), 1, 255) },
        group("Group", b"norm"),
    ];
    let (doc, warnings) = psdlayers::import(&file(8, 8, &layers, 8, false, false), true, 128, cms).unwrap().unwrap();
    assert_eq!(doc.layers[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Lower"]);
    assert_eq!(warnings, [HIDDEN_1]);
}

/// A Dissolve knockout at part opacity clears each pixel below it fully or not at all, at the share
/// its opacity gives, as a Dissolve layer merges.
#[test]
fn a_dissolve_knockout_at_part_opacity_clears_whole_pixels() {
    let cut = knockout(L { blend: b"diss", opacity: 128, ..layer("Cut", [0, 0, 16, 16], [0, 0, 255, 255]) }, 1, 0);
    let layers = [group_end(), layer("Lower", [0, 0, 16, 16], [255, 0, 0, 255]), cut, group("Group", b"norm")];
    let (s, r) = open(&file(16, 16, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let (_, px) = image_of(&s, &d.layers[0].children().unwrap()[0]);
    assert!(px.pixels().all(|p| p.0 == [255, 0, 0, 255] || p.0[3] == 0), "{r}");
    let red = px.pixels().filter(|p| p.0 == [255, 0, 0, 255]).count();
    assert!((32..=224).contains(&red), "{red} of 256 pixels keep the lower layer: {r}");
}

/// The knockout of a layer that another is clipped to isn't applied, also when the clipped layer's
/// merge with it needs more memory than allowed and the two come in apart.
#[test]
fn a_clipping_base_knockout_isnt_applied_when_its_clip_doesnt_fit() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    // The 4 × 4 clipped layer and its 2 × 2 base don't fit 320 bytes together for the clip; the
    // base's knockout merge with the 2 × 2 layer below would.
    let layers = [
        group_end(),
        layer("Lower", [0, 0, 2, 2], [255, 0, 0, 255]),
        knockout(layer("Base", [0, 0, 2, 2], [0, 0, 255, 255]), 1, 0),
        L { clipped: true, ..layer("Clipped", [0, 0, 4, 4], [0, 255, 0, 255]) },
        group("Group", b"norm"),
    ];
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 320, cms).unwrap().unwrap();
    assert_eq!(doc.layers[0].children().unwrap().iter().map(|c| name(c)).collect::<Vec<_>>(), ["Lower", "Base", "Clipped"], "{warnings:?}");
    assert_eq!(
        warnings,
        ["the knockout of 1 layer wasn't applied", "1 layer clipped to the layer below came in unclipped: the merge needs more memory than allowed"]
    );
}

/// A knockout group that comes in as a layer counts each layer clipped in it once in the warnings,
/// though its layers are also drawn for the knockout: one clipped to a fill that isn't drawn, and
/// one that a merge past the memory allowed leaves unclipped.
#[test]
fn a_knockout_group_that_comes_in_as_a_layer_counts_its_clipped_layers_once() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let empty = L { bare: true, ..layer("Empty", [0, 0, 2, 2], [0, 0, 0, 0]) };
    // A fill whose shape is a vector mask, with a layer clipped to it: neither is drawn.
    let mut soco = vec![0; 8];
    for (key, v) in [(b"Rd  doub", 255.0f64), (b"Grn doub", 0.0), (b"Bl  doub", 51.0)] {
        soco.extend(key);
        soco.extend(v.to_be_bytes());
    }
    let mut outline = layer("Outline", [0, 0, 0, 0], [0; 4]);
    outline.blocks.push((*b"SoCo", soco));
    outline.blocks.push((*b"vsms", vec![0; 8]));
    let on_outline = L { clipped: true, ..layer("On outline", [0, 0, 2, 2], [1, 1, 1, 255]) };
    // Each 2 × 2 layer fits 128 bytes, and so do the two drawn together, but not the merge of the
    // clipped one with the one below.
    let base = layer("Base", [0, 0, 2, 2], [255, 0, 0, 255]);
    let clipped = L { clipped: true, ..layer("Clipped", [0, 0, 2, 2], [0, 0, 255, 255]) };
    for (inner, said) in [
        (vec![outline, on_outline], "1 layer clipped to a fill layer that wasn't drawn was left out"),
        (vec![base, clipped], "1 layer clipped to the layer below came in unclipped: the merge needs more memory than allowed"),
    ] {
        let mut layers = vec![layer("Outside", [0, 0, 4, 4], [0, 255, 0, 255]), group_end(), empty.clone(), group_end()];
        layers.extend(inner);
        layers.extend([knockout(group("Cut", b"norm"), 1, 255), group("Group", b"norm")]);
        let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 128, cms).unwrap().unwrap();
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Outside", "Group"]);
        assert!(warnings.iter().any(|w| w == said), "{said}: {warnings:?}");
    }
}

/// A pass-through group that draws nothing still counts what its layers count in it, also when it
/// is drawn again inside a group that merges: a knockout above a Multiply layer isn't applied in a
/// pass-through group, though drawn alone it would be.
#[test]
fn a_pass_through_group_that_draws_nothing_still_counts_its_unapplied_knockouts() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let empty = |name: &'static str, blend: &'static [u8; 4]| L { bare: true, blend, ..layer(name, [0, 0, 2, 2], [0, 0, 0, 0]) };
    let layers = [
        layer("Lower", [0, 0, 4, 4], [255, 0, 0, 255]),
        group_end(),
        empty("Empty", b"norm"),
        group_end(),
        empty("Multiplied", b"mul "),
        knockout(empty("Cut", b"norm"), 1, 255),
        knockout(group("Through", b"pass"), 1, 255),
        group("Dissolved", b"diss"),
    ];
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Lower + Dissolved"], "{warnings:?}");
    assert_eq!(warnings, ["the knockout of 1 layer wasn't applied".to_string(), psdlayers::MERGED_WARNING.replace("{modes}", "Dissolve")]);
}

/// A group that draws nothing is drawn once, however deep the groups like it nest: here each holds
/// a layer that stores no pixels and, above it, the next group, with a knockout. The import walks
/// at most as many layer lists as the file has records, with isolated groups, with groups that
/// alternate with pass-through ones, with pass-through groups inside the outermost one (whose
/// knockouts aren't applied), and with a layer in a mode VectorCraft lacks in each (a merge of
/// nothing, a canvas without pixels).
#[test]
fn groups_that_draw_nothing_are_drawn_once_however_deep_they_nest() {
    const LEVELS: usize = 48;
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let empty = |name: &'static str, blend: &'static [u8; 4]| L { bare: true, blend, ..layer(name, [0, 0, 2, 2], [0, 0, 0, 0]) };
    // Which levels are pass-through groups.
    let none: fn(usize) -> bool = |_| false;
    let odd: fn(usize) -> bool = |k| k % 2 == 1;
    let all: fn(usize) -> bool = |k| k > 0;
    for (case, pass, dissolve, warned) in [
        ("isolated", none, false, vec![]),
        ("alternating", odd, false, vec![]),
        ("pass-through", all, false, vec![format!("the knockout of {} layers wasn't applied", LEVELS - 1)]),
        ("dissolved", none, true, vec![psdlayers::MERGED_WARNING.replace("{modes}", "Dissolve")]),
    ] {
        let mut layers = vec![layer("Outside", [0, 0, 4, 4], [0, 255, 0, 255])];
        for _ in 0..=LEVELS {
            layers.push(group_end());
            if dissolve {
                layers.push(empty("Dissolved", b"diss"));
            }
            layers.push(empty("Empty", b"norm"));
        }
        for k in (0..=LEVELS).rev() {
            let g = group("Group", if pass(k) { b"pass" } else { b"norm" });
            layers.push(if k == 0 { g } else { knockout(g, 1, 255) });
        }
        psdlayers::WALKS.set((0, layers.len()));
        let imported = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 1 << 20, cms);
        let (walked, _) = psdlayers::WALKS.replace((0, usize::MAX));
        let (doc, warnings) = imported.unwrap().unwrap();
        assert!(walked <= layers.len(), "{case}: {walked} walks for {} records", layers.len());
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Outside", "Group"], "{case}");
        let mut expected = warned;
        expected.push(vectorcraft_doc::depth::merged_note());
        assert_eq!(warnings, expected, "{case}");
    }
}

#[test]
fn what_doesnt_come_in_is_said() {
    let mut text = layer("Words", [0, 0, 2, 2], [0, 0, 0, 255]);
    text.blocks.push((*b"TySh", vec![0; 4]));
    let mut adjust = layer("Levels", [0, 0, 0, 0], [0; 4]);
    adjust.blocks.push((*b"levl", vec![0; 4]));
    let mut fx = layer("Shadowed", [0, 0, 2, 2], [0, 0, 0, 255]);
    fx.blocks.push((*b"lfx2", effects(true, true)));
    fx.blocks.push((*b"knko", vec![1, 0, 0, 0]));
    // Effects with the master switch off, or none switched on, aren't counted.
    let mut fx_off = layer("Switched off", [0, 0, 1, 1], [0, 0, 0, 255]);
    fx_off.blocks.push((*b"lfx2", effects(false, true)));
    let mut fx_none = layer("None on", [0, 0, 1, 1], [0, 0, 0, 255]);
    fx_none.blocks.push((*b"lfx2", effects(true, false)));
    // An adjustment layer clipped to the layer below draws nothing, and is still counted.
    let mut clipped_adjust = L { clipped: true, ..layer("Curves", [0, 0, 0, 0], [0; 4]) };
    clipped_adjust.blocks.push((*b"curv", vec![0; 4]));
    let mut solid = layer("Fill", [0, 0, 0, 0], [0; 4]);
    let mut soco = vec![0; 8];
    for (key, v) in [(b"Rd  doub", 255.0f64), (b"Grn doub", 0.0), (b"Bl  doub", 51.0)] {
        soco.extend(key);
        soco.extend(v.to_be_bytes());
    }
    solid.blocks.push((*b"SoCo", soco.clone()));
    // A shape layer: a solid color fill with a vector mask and the shape's pixels stored.
    let mut shape = layer("Shape", [1, 1, 3, 2], [255, 0, 51, 255]);
    shape.blocks.push((*b"SoCo", soco.clone()));
    shape.blocks.push((*b"vmsk", vec![0; 8]));
    // One with its vector mask only, no pixels: not drawn, rather than its color over the canvas,
    // and the layer clipped to it isn't either.
    let mut outline = layer("Outline", [0, 0, 0, 0], [0; 4]);
    outline.blocks.push((*b"SoCo", soco));
    outline.blocks.push((*b"vsms", vec![0; 8]));
    let on_outline = L { clipped: true, ..layer("On outline", [0, 0, 2, 2], [1, 1, 1, 255]) };
    let layers = [solid, text, clipped_adjust, adjust, fx, fx_off, fx_none, shape, outline, on_outline];
    let (s, r) = open(&file(4, 4, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Fill", "Words", "Shadowed", "Switched off", "None on", "Shape"], "an adjustment layer draws nothing");
    assert_eq!(image_of(&s, &d.layers[0]).1.get_pixel(3, 3).0, [255, 0, 51, 255], "a solid color fill covers the canvas");
    let ((x, y), px) = image_of(&s, &d.layers[5]);
    assert_eq!(((x, y), px.dimensions()), ((1.0, 1.0), (1, 2)), "a shape layer comes in as the pixels it stores");
    let w = r["warnings"].to_string();
    for said in [
        "1 type layer came in as pixels",
        "2 adjustment layers were not applied",
        "1 fill layer was not drawn",
        "the layer effects of 1 layer weren't drawn",
        "the vector masks of 2 layers",
        "the knockout of 1 layer wasn't applied",
        "1 layer clipped to a fill layer that wasn't drawn was left out",
    ] {
        assert!(w.contains(said), "{said}: {w}");
    }
}

/// An effects descriptor (`lfx2`) with its master switch and one effect's `enab` set: each key after
/// its length (0 for a 4-character key), then `bool` and a byte.
fn effects(master: bool, enabled: bool) -> Vec<u8> {
    let mut d = vec![0; 8];
    d.extend(14u32.to_be_bytes());
    d.extend(b"masterFXSwitchbool");
    d.push(u8::from(master));
    d.extend(0u32.to_be_bytes());
    d.extend(b"enabbool");
    d.push(u8::from(enabled));
    d
}

/// A layer's fill opacity fades only its own pixels when layers are clipped to it: they still show
/// in its shape, and the merged layer keeps the layer's opacity.
#[test]
fn a_clipping_base_fill_opacity_fades_only_its_own_pixels() {
    for (fill, size, corner) in [(0u8, (4, 2), None), (128, (4, 4), Some([255, 0, 0, 128]))] {
        let mut base = L { opacity: 200, ..layer("Base", [0, 0, 4, 4], [255, 0, 0, 255]) };
        base.blocks.push((*b"iOpa", vec![fill]));
        let photo = L { clipped: true, ..layer("Photo", [0, 0, 2, 4], [0, 0, 255, 255]) };
        let (s, r) = open(&file(4, 4, &[base, photo], 8, false, false), json!({}));
        let merged = &s.doc().unwrap().doc.layers[0];
        assert_eq!(name(merged), "Base + Photo", "{r}");
        assert!((merged.opacity - 200.0 / 255.0).abs() < 1e-6, "{fill}: {}", merged.opacity);
        let ((x, y), px) = image_of(&s, merged);
        assert_eq!(((x, y), px.dimensions(), px.get_pixel(0, 0).0), ((0.0, 0.0), size, [0, 0, 255, 255]), "{fill}");
        if let Some(c) = corner {
            assert!(px.get_pixel(3, 3).0.iter().zip(c).all(|(&a, b)| a.abs_diff(b) <= 1), "{fill}: {:?}", px.get_pixel(3, 3));
        }
    }
}

/// Layers clipped to an adjustment layer show over the whole canvas of the file, through the
/// adjustment layer's mask.
#[test]
fn layers_clipped_to_an_adjustment_layer_show_over_the_canvas() {
    let mut curves = layer("Curves", [0, 0, 0, 0], [0; 4]);
    curves.blocks.push((*b"curv", vec![0; 4]));
    curves.mask = Some(([0, 0, 1, 1], 255, 0, vec![0]));
    let photo = L { clipped: true, ..layer("Photo", [-1, -1, 2, 5], [0, 0, 255, 255]) };
    let (s, r) = open(&file(4, 4, &[layer("Under", [0, 0, 4, 4], [255, 0, 0, 255]), curves, photo], 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Under", "Curves + Photo"], "{r}");
    // Cut to the canvas: 4 × 2 at its corner.
    let ((x, y), px) = image_of(&s, &d.layers[1]);
    assert_eq!(((x, y), px.dimensions(), px.get_pixel(1, 1).0), ((0.0, 0.0), (4, 2), [0, 0, 255, 255]));
    assert!(d.layers[1].extras().mask.is_some(), "the adjustment layer's mask stays");
    assert!(r["warnings"].to_string().contains("1 adjustment layer was not applied"), "{r}");
}

/// An adjustment layer of key `key` with `data`, storing no pixels.
fn adjustment(name: &'static str, key: &[u8; 4], data: &[u8]) -> L {
    L { blocks: vec![(*key, data.to_vec())], ..layer(name, [0, 0, 0, 0], [0; 4]) }
}

/// The colors of a 1-pixel-high layer's image, left to right.
fn row_of(s: &Session, l: &Node) -> Vec<[u8; 4]> {
    let px = image_of(s, l).1;
    (0..px.width()).map(|x| px.get_pixel(x, 0).0).collect()
}

/// Invert, Threshold and Posterize adjustment layers merge with the visible layers below them, which
/// they change; the layers above stay apart. Invert turns each value `v` into `255 − v`; Threshold
/// at 128 makes a pixel white when its luminance is above 128 and black at or below it; Posterize at
/// 4 levels maps each channel to the nearest of 0, 85, 170 and 255.
#[test]
fn invert_threshold_and_posterize_merge_with_the_layers_below() {
    let top = layer("Top", [0, 0, 1, 1], [1, 2, 3, 255]);
    for (depth, psb) in [(8, false), (16, false), (8, true), (16, true)] {
        let layers = [
            layer("Dark", [0, 0, 1, 1], [10, 20, 30, 255]),
            layer("Light", [0, 1, 1, 2], [200, 150, 100, 128]),
            adjustment("Invert", b"nvrt", &[]),
            top.clone(),
        ];
        let (s, r) = open(&file(2, 1, &layers, depth, false, psb), json!({}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Dark + Light + Invert", "Top"], "{depth} {psb}: {r}");
        assert_eq!(r["warnings"], json!([]), "{depth} {psb}");
        assert_eq!(row_of(&s, &d.layers[0]), [[245, 235, 225, 255], [55, 105, 155, 128]], "{depth} {psb}: transparency stays");
        assert_eq!(row_of(&s, &d.layers[1]), [[1, 2, 3, 255]], "{depth} {psb}: the layer above isn't changed");
    }
    let grays = [
        layer("128", [0, 0, 1, 1], [128, 128, 128, 255]),
        layer("129", [0, 1, 1, 2], [129, 129, 129, 255]),
        layer("Light", [0, 2, 1, 3], [200, 150, 100, 255]),
        layer("Dark", [0, 3, 1, 4], [10, 20, 30, 255]),
    ];
    let layers = [&grays[..], &[adjustment("Threshold", b"thrs", &[0, 128, 0, 0])][..]].concat();
    let (s, r) = open(&file(4, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["128 + 129 + Light + Dark + Threshold"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[0, 0, 0, 255], [255, 255, 255, 255], [255, 255, 255, 255], [0, 0, 0, 255]]);
    let layers =
        [layer("A", [0, 0, 1, 1], [42, 43, 128, 255]), layer("B", [0, 1, 1, 2], [212, 213, 0, 255]), adjustment("Posterize", b"post", &[0, 4, 0, 0])];
    let (s, r) = open(&file(2, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["A + B + Posterize"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[0, 85, 170, 255], [170, 255, 0, 255]]);
}

/// An adjustment layer's opacity, layer mask and blend mode decide how much of the adjusted color
/// replaces the color below.
#[test]
fn an_adjustment_layer_takes_its_opacity_mask_and_blend_mode() {
    let black = || layer("Black", [0, 0, 1, 2], [0, 0, 0, 255]);
    let half = L { opacity: 128, ..adjustment("Invert", b"nvrt", &[]) };
    let (s, r) = open(&file(2, 1, &[black(), half], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[128, 128, 128, 255]; 2], "{r}");
    // Black in the mask leaves the first pixel as it was; outside, the mask's default (white) applies it.
    let masked = L { mask: Some(([0, 0, 1, 1], 255, 0, vec![0])), ..adjustment("Invert", b"nvrt", &[]) };
    let (s, r) = open(&file(2, 1, &[black(), masked], 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(row_of(&s, &d.layers[0]), [[0, 0, 0, 255], [255, 255, 255, 255]], "{r}");
    assert!(d.layers[0].extras().mask.is_none(), "the mask is spent on the merge");
    // A disabled mask applies it everywhere.
    let off = L { mask: Some(([0, 0, 1, 1], 255, 2, vec![0])), ..adjustment("Invert", b"nvrt", &[]) };
    let (s, _) = open(&file(2, 1, &[black(), off], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[255, 255, 255, 255]; 2]);
    // Multiply: the inverted color (55, 155, 205) multiplies the color below.
    let multiply = L { blend: b"mul ", ..adjustment("Invert", b"nvrt", &[]) };
    let (s, _) = open(&file(1, 1, &[layer("Under", [0, 0, 1, 1], [200, 100, 50, 255]), multiply], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[43, 61, 40, 255]]);
    // In a mode VectorCraft lacks, the warnings name the mode, as for other layers merged for it.
    let dodge = L { blend: b"lddg", ..adjustment("Invert", b"nvrt", &[]) };
    let (s, r) = open(&file(1, 1, &[layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), dodge], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[255, 255, 255, 255]], "{r}");
    let merged = "layers in blend modes VectorCraft doesn't have (Linear Dodge) were merged with the layers below them";
    assert_eq!(r["warnings"], json!([merged]));
}

/// Clipped to a layer, an adjustment layer changes only that layer and the layers clipped to it.
#[test]
fn a_clipped_adjustment_layer_changes_only_its_base() {
    let layers = [
        layer("Other", [0, 1, 1, 2], [10, 10, 10, 255]),
        layer("Base", [0, 0, 1, 1], [200, 0, 0, 255]),
        L { clipped: true, ..adjustment("Invert", b"nvrt", &[]) },
    ];
    let (s, r) = open(&file(2, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Other", "Base + Invert"], "{r}");
    assert_eq!(r["warnings"], json!([]));
    assert_eq!((row_of(&s, &d.layers[0]), row_of(&s, &d.layers[1])), (vec![[10, 10, 10, 255]], vec![[55, 255, 255, 255]]));
    // Over a layer clipped to the same base, and only inside the base's shape.
    let layers = [
        layer("Base", [0, 0, 1, 1], [200, 0, 0, 255]),
        L { clipped: true, ..layer("Photo", [0, 0, 1, 2], [0, 0, 255, 255]) },
        L { clipped: true, ..adjustment("Invert", b"nvrt", &[]) },
    ];
    let (s, r) = open(&file(2, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Base + Photo + Invert"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[255, 255, 0, 255]]);
    // Clipped to another adjustment layer, it isn't applied: the layers keep their colors.
    let layers = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        adjustment("Curves", b"curv", &[0; 4]),
        L { clipped: true, ..layer("Photo", [0, 0, 1, 1], [0, 0, 255, 255]) },
        L { clipped: true, ..adjustment("Invert", b"nvrt", &[]) },
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Curves + Photo"], "{r}");
    assert_eq!((row_of(&s, &d.layers[0]), row_of(&s, &d.layers[1])), (vec![[10, 20, 30, 255]], vec![[0, 0, 255, 255]]));
    assert_eq!(r["warnings"], json!(["2 adjustment layers were not applied"]));
    // Its layer mask isn't read then: one too large to read doesn't stop the file opening.
    let mut masked = layers;
    masked[3].mask = Some(([0, 0, 1, 40_000], 0, 0, vec![200]));
    let (s, r) = open(&file(1, 1, &masked, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Curves + Photo"], "{r}");
    assert_eq!(r["warnings"], json!(["2 adjustment layers were not applied"]));
    // An Invert layer that a layer drawing nothing is clipped to isn't applied either.
    let layers = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        adjustment("Invert", b"nvrt", &[]),
        L { clipped: true, ..layer("Empty", [0, 0, 0, 0], [0; 4]) },
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[10, 20, 30, 255]]);
    assert_eq!(r["warnings"], json!(["1 adjustment layer was not applied"]));
}

/// A hidden adjustment layer changes nothing and is left out, with or without the hidden layers; one
/// with nothing below it, or in an isolated group with nothing below it there, changes nothing.
#[test]
fn hidden_and_lone_adjustment_layers_change_nothing() {
    for hidden in [false, true] {
        let layers = [layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), L { hidden: true, ..adjustment("Invert", b"nvrt", &[]) }];
        let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({"hiddenLayers": hidden}));
        let d = &s.doc().unwrap().doc;
        assert_eq!((d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), &r["warnings"]), (vec!["Under"], &json!([HIDDEN_1])), "{hidden}");
        assert_eq!(row_of(&s, &d.layers[0]), [[10, 20, 30, 255]]);
    }
    let (s, r) = open(&file(1, 1, &[adjustment("Invert", b"nvrt", &[]), layer("Top", [0, 0, 1, 1], [10, 20, 30, 255])], 8, false, false), json!({}));
    assert_eq!((s.doc().unwrap().doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), &r["warnings"]), (vec!["Top"], &json!([])));
    let layers = [layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), group_end(), adjustment("Invert", b"nvrt", &[]), group("Isolated", b"norm")];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), &r["warnings"]), (vec!["Under", "Isolated"], &json!([])));
    assert_eq!(row_of(&s, &d.layers[0]), [[10, 20, 30, 255]]);
}

/// In a pass-through group with nothing below it in the group, an adjustment layer changes what is
/// below the group when the group is merged; when the group comes in as layers, it isn't applied.
#[test]
fn an_adjustment_layer_in_a_pass_through_group_changes_what_is_below_the_group() {
    let layers = |dodge: bool| {
        let mut l = vec![layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), group_end(), adjustment("Invert", b"nvrt", &[]), group("Folder", b"pass")];
        if dodge {
            l.push(L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) });
        }
        l
    };
    let (s, r) = open(&file(1, 1, &layers(true), 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[245, 235, 225, 255]]);
    let (s, r) = open(&file(1, 1, &layers(false), 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Folder"], "{r}");
    assert_eq!(r["warnings"], json!(["1 adjustment layer was not applied"]));
    // Two at the bottom of the group both change what is below it, in order: inverted, then
    // posterized to 4 levels (without the Invert the color would posterize to black).
    let stacked = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(1, 1, &stacked, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!((row_of(&s, &d.layers[0]), r["warnings"].to_string().contains("adjustment")), (vec![[255, 255, 255, 255]], false), "{r}");
    // One above a layer of the group, with one kept below that layer, is kept too: the group draws
    // them in order over what is below it.
    let between = [
        layer("Under", [0, 0, 1, 2], [10, 20, 30, 255]),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        layer("Top", [0, 1, 1, 2], [100, 100, 100, 255]),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(2, 1, &between, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[255, 255, 255, 255], [85, 85, 85, 255]], "{r}");
    // Another adjustment layer below it in the group draws nothing, so it still changes what is
    // below the group.
    let over_curves = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        adjustment("Curves", b"curv", &[0; 4]),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(1, 1, &over_curves, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[245, 235, 225, 255]], "{r}");
    // One kept in a pass-through group inside the group keeps the ones above it too: inverted, then
    // posterized to 4 levels.
    let nested = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        group("Inner", b"pass"),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(1, 1, &nested, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!((row_of(&s, &d.layers[0]), r["warnings"].to_string().contains("adjustment")), (vec![[255, 255, 255, 255]], false), "{r}");
    // Without the merge, the group comes in as layers and both are counted as not applied.
    let (s, r) = open(&file(1, 1, &nested[..7], 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Folder"], "{r}");
    assert_eq!(r["warnings"], json!(["2 adjustment layers were not applied"]));
    // Merged in the group with a layer in a mode VectorCraft lacks, it has nothing to change.
    let inside = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
        group("Folder", b"pass"),
    ];
    let (s, r) = open(&file(1, 1, &inside, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    let merged = "layers in blend modes VectorCraft doesn't have (Linear Dodge) were merged with the layers below them";
    assert_eq!((names, &r["warnings"]), (vec!["Under", "Folder"], &json!(["1 adjustment layer was not applied", merged])), "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[10, 20, 30, 255]]);
}

/// The layer of a pass-through group's sublayers whose name ends with `end`.
fn sublayer<'d>(folder: &'d Node, end: &str) -> &'d Node {
    let children = folder.children().map(Vec::as_slice).unwrap_or_default();
    let names: Vec<&str> = children.iter().map(|c| name(c)).collect();
    children.iter().find(|c| name(c).ends_with(end)).unwrap_or_else(|| panic!("no sublayer ending with {end} in {names:?}"))
}

/// An adjustment layer that changes nothing, in a hidden pass-through group or at 0 opacity or 0
/// fill, keeps no adjustment layer above it in the group from merging with the layers below it:
/// the gray 100 posterizes to 85.
#[test]
fn adjustment_layers_that_change_nothing_hold_none_back() {
    let invert = |opacity: u8, fill: u8| {
        let mut l = L { opacity, ..adjustment("Invert", b"nvrt", &[]) };
        l.blocks.push((*b"iOpa", vec![fill]));
        l
    };
    let cases = [("hidden group", true, invert(255, 255)), ("0 opacity", false, invert(0, 255)), ("0 fill", false, invert(255, 0))];
    for (what, hidden, invert) in cases {
        let layers = [
            group_end(),
            group_end(),
            invert,
            L { hidden, ..group("Inner", b"pass") },
            layer("Photo", [0, 0, 1, 1], [100, 100, 100, 255]),
            adjustment("Posterize", b"post", &[0, 4, 0, 0]),
            group("Folder", b"pass"),
        ];
        let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({"hiddenLayers": hidden}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Folder"], "{what}: {r}");
        assert_eq!(row_of(&s, sublayer(&d.layers[0], "Photo + Posterize")), [[85, 85, 85, 255]], "{what}: {r}");
    }
}

/// A layer that stores no pixels draws nothing: an adjustment layer above it in a pass-through group,
/// directly or in a pass-through group inside it, still changes what is below the group.
#[test]
fn an_adjustment_layer_over_layers_that_draw_nothing_changes_what_is_below_the_group() {
    let empty = || layer("Empty", [0, 0, 0, 0], [0; 4]);
    let direct = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        empty(),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(1, 1, &direct, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[245, 235, 225, 255]], "{r}");
    assert!(!r["warnings"].to_string().contains("adjustment"), "{r}");
    // Inverted in the inner group, then posterized to 4 levels.
    let nested = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        group_end(),
        empty(),
        adjustment("Invert", b"nvrt", &[]),
        group("Inner", b"pass"),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(1, 1, &nested, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{r}");
    assert_eq!(row_of(&s, &d.layers[0]), [[255, 255, 255, 255]], "{r}");
    assert!(!r["warnings"].to_string().contains("adjustment"), "{r}");
    // Nor does a group of such layers, in any mode, or a layer at 0 opacity: below them, directly
    // or in the inner group, Invert and Posterize still change what is below the outer group.
    let mut no_fill = layer("Clear", [0, 0, 1, 1], [0, 0, 0, 255]);
    no_fill.blocks.push((*b"iOpa", vec![0]));
    let photo = || layer("Photo", [0, 0, 1, 1], [0, 0, 255, 255]);
    let below: [(&str, Vec<L>); 5] = [
        ("empty group", vec![group_end(), empty(), group("Empty", b"norm")]),
        ("empty pass-through group", vec![group_end(), empty(), group("Empty", b"pass")]),
        ("0 opacity", vec![L { opacity: 0, ..layer("Clear", [0, 0, 1, 1], [0, 0, 0, 255]) }]),
        ("0 fill in a group", vec![group_end(), no_fill, group("Empty", b"norm")]),
        ("group clipped to an empty layer", vec![empty(), group_end(), photo(), L { clipped: true, ..group("Clipped", b"norm") }]),
    ];
    for (what, nothing) in below {
        let direct = [
            &[layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), group_end()][..],
            &nothing,
            &[adjustment("Invert", b"nvrt", &[]), group("Folder", b"pass"), L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) }],
        ]
        .concat();
        let (s, r) = open(&file(1, 1, &direct, 8, false, false), json!({}));
        assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[245, 235, 225, 255]], "{what}: {r}");
        let nested = [
            &[layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), group_end(), group_end()][..],
            &nothing,
            &[
                adjustment("Invert", b"nvrt", &[]),
                group("Inner", b"pass"),
                adjustment("Posterize", b"post", &[0, 4, 0, 0]),
                group("Folder", b"pass"),
                L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
            ],
        ]
        .concat();
        let (s, r) = open(&file(1, 1, &nested, 8, false, false), json!({}));
        assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[255, 255, 255, 255]], "{what}: {r}");
    }
}

/// Layers clipped to a layer at 0 fill, or to an adjustment layer, draw: an adjustment layer above
/// them in a pass-through group merges with them and is applied.
#[test]
fn an_adjustment_layer_over_clipped_layers_merges_with_them() {
    let mut clear = layer("Base", [0, 0, 1, 1], [0, 0, 0, 255]);
    clear.blocks.push((*b"iOpa", vec![0]));
    let photo = || L { clipped: true, ..layer("Photo", [0, 0, 1, 1], [0, 0, 255, 255]) };
    let layers = [
        group_end(),
        group_end(),
        clear,
        photo(),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
        group("Outer", b"pass"),
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let merged = "layers in blend modes VectorCraft doesn't have (Linear Dodge) were merged with the layers below them";
    assert_eq!(r["warnings"], json!([merged]), "{r}");
    assert_eq!(row_of(&s, sublayer(&d.layers[0], "Dodge")), [[255, 255, 0, 255]], "{r}");
    // The same with a group at 0 fill as the base.
    let mut clear_group = group("Base", b"norm");
    clear_group.blocks.push((*b"iOpa", vec![0]));
    let layers = [
        group_end(),
        group_end(),
        group_end(),
        layer("Black", [0, 0, 1, 1], [0, 0, 0, 255]),
        clear_group,
        photo(),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
        group("Outer", b"pass"),
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(r["warnings"], json!([merged]), "{r}");
    assert_eq!(row_of(&s, sublayer(&d.layers[0], "Dodge")), [[255, 255, 0, 255]], "{r}");
    // Clipped to an Invert layer, which isn't applied then, the photo shows over the canvas, and
    // Posterize merges with it: the gray 100 posterizes to 85.
    let layers = [
        group_end(),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        L { clipped: true, ..layer("Photo", [0, 0, 1, 1], [100, 100, 100, 255]) },
        group("Folder", b"pass"),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Outer", b"pass"),
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(row_of(&s, sublayer(&d.layers[0], "Posterize")), [[85, 85, 85, 255]], "{r}");
}

/// Invert, Threshold and Posterize data that can't be read is not applied, with a warning: a level
/// out of range, data too short, or a second adjustment block on the layer.
#[test]
fn adjustment_data_that_cant_be_read_isnt_applied() {
    let mut twice = adjustment("Twice", b"nvrt", &[]);
    twice.blocks.push((*b"post", vec![0, 4, 0, 0]));
    let mut apart = adjustment("Apart", b"nvrt", &[]);
    apart.blocks.extend([(*b"SoCo", vec![]), (*b"post", vec![0, 4, 0, 0])]);
    let cases = [
        adjustment("Level 0", b"thrs", &[0, 0, 0, 0]),
        adjustment("Level 256", b"thrs", &[1, 0, 0, 0]),
        adjustment("No level", b"thrs", &[]),
        adjustment("No padding", b"thrs", &[0, 128]),
        adjustment("No padding", b"post", &[0, 4]),
        adjustment("1 level", b"post", &[0, 1, 0, 0]),
        adjustment("256 levels", b"post", &[1, 0, 0, 0]),
        twice,
        apart,
    ];
    for bad in cases {
        let what = bad.name;
        let (s, r) = open(&file(1, 1, &[layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), bad], 8, false, false), json!({}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under"], "{what}: {r}");
        assert_eq!(row_of(&s, &d.layers[0]), [[10, 20, 30, 255]], "{what}");
        assert_eq!(r["warnings"], json!(["1 adjustment layer was not applied"]), "{what}");
    }
    // The highest level and count are read.
    let under = |c: u8| layer("Under", [0, 0, 1, 1], [c, c, c, 255]);
    let (s, r) = open(&file(1, 1, &[under(254), adjustment("Max", b"thrs", &[0, 255, 0, 0])], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[0, 0, 0, 255]], "{r}");
    let (s, r) = open(&file(1, 1, &[under(7), adjustment("Max", b"post", &[0, 255, 0, 0])], 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[7, 7, 7, 255]], "{r}");
}

/// An adjustment layer whose merge needs more memory than allowed isn't applied: the layers below
/// it stay as they are, with a warning.
#[test]
fn an_adjustment_merge_past_the_memory_allowed_isnt_applied() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let layers = [layer("Under", [0, 0, 2, 2], [10, 20, 30, 255]), layer("Beside", [2, 2, 4, 4], [1, 2, 3, 255]), adjustment("Invert", b"nvrt", &[])];
    let f = file(4, 4, &layers, 8, false, false);
    // Each 2 × 2 layer fits 128 bytes; the 4 × 4 canvas of both doesn't.
    let (doc, warnings) = psdlayers::import(&f, false, 128, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Beside"]);
    assert_eq!(warnings, ["1 adjustment layer was not applied"]);
    let (doc, warnings) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Beside + Invert"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    // Clipped: the base's canvas with its shape doesn't fit 300 bytes, so the adjustment is left
    // out rather than applied to everything below.
    let layers = [layer("Base", [0, 0, 4, 4], [10, 20, 30, 255]), L { clipped: true, ..adjustment("Invert", b"nvrt", &[]) }];
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 300, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Base"]);
    assert_eq!(warnings, ["1 adjustment layer was not applied"]);
    // A layer clipped to an Invert layer whose merge doesn't fit comes in unclipped, and the Invert
    // layer isn't applied to the layers below it.
    let layers = [
        layer("Under", [0, 0, 2, 2], [10, 20, 30, 255]),
        adjustment("Invert", b"nvrt", &[]),
        L { clipped: true, ..layer("Photo", [0, 0, 4, 4], [1, 2, 3, 255]) },
    ];
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 128, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under", "Photo"], "{warnings:?}");
    assert!(warnings.iter().any(|w| w == "1 adjustment layer was not applied"), "{warnings:?}");
}

/// An adjustment layer whose layer mask is too large for the memory allowed isn't applied, merged
/// with the layers below it or clipped to one, and the file still opens with its layers.
#[test]
fn an_adjustment_mask_past_the_memory_allowed_isnt_applied() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    // The 100 × 100 mask takes more than 128 bytes; the 1-pixel layer below it doesn't.
    let masked = |clipped: bool| L { clipped, mask: Some(([0, 0, 100, 100], 255, 0, vec![0; 10_000])), ..adjustment("Invert", b"nvrt", &[]) };
    for clipped in [false, true] {
        let f = file(1, 1, &[layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]), masked(clipped)], 8, false, false);
        let (doc, warnings) = psdlayers::import(&f, false, 128, cms).unwrap_or_else(|e| panic!("{clipped}: {e}")).unwrap();
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under"], "{clipped}: {warnings:?}");
        assert_eq!(warnings, ["1 adjustment layer was not applied"], "{clipped}");
        // With the memory for it, the mask's black leaves the layer as it was.
        let (doc, warnings) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Invert"], "{clipped}: {warnings:?}");
        assert!(warnings.is_empty(), "{clipped}: {warnings:?}");
    }
}

/// An adjustment layer's mask is read beside the canvases it changes: a 100 × 1 mask fits 256 bytes
/// alone, but not beside a 6 × 1 canvas, so the layer isn't applied. With the memory for both, it is.
#[test]
fn an_adjustment_mask_counts_the_canvas_it_changes() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let masked = |clipped: bool| L { clipped, mask: Some(([0, 0, 1, 100], 255, 0, vec![255; 100])), ..adjustment("Invert", b"nvrt", &[]) };
    for clipped in [false, true] {
        let f = file(6, 1, &[layer("Under", [0, 0, 1, 6], [10, 20, 30, 255]), masked(clipped)], 8, false, false);
        let (doc, warnings) = psdlayers::import(&f, false, 256, cms).unwrap_or_else(|e| panic!("{clipped}: {e}")).unwrap();
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under"], "{clipped}: {warnings:?}");
        assert_eq!(warnings, ["1 adjustment layer was not applied"], "{clipped}");
        let (doc, warnings) = psdlayers::import(&f, false, 1 << 20, cms).unwrap().unwrap();
        assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Invert"], "{clipped}: {warnings:?}");
        assert!(warnings.is_empty(), "{clipped}: {warnings:?}");
    }
}

/// An adjustment layer whose mask is too large to read is left out without stopping the clip or the
/// merge around it: a layer clipped above it still clips to the base, and a layer in a mode
/// VectorCraft lacks above a pass-through group holding it still merges in its mode.
#[test]
fn an_adjustment_mask_too_large_leaves_the_clip_and_merge_around_it() {
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let masked = |clipped: bool| L { clipped, mask: Some(([0, 0, 100, 100], 255, 0, vec![255; 10_000])), ..adjustment("Invert", b"nvrt", &[]) };
    let layers = [layer("Base", [0, 0, 1, 1], [200, 0, 0, 255]), masked(true), L { clipped: true, ..layer("Photo", [0, 0, 1, 2], [0, 0, 255, 255]) }];
    let (doc, warnings) = psdlayers::import(&file(2, 1, &layers, 8, false, false), false, 128, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Base + Photo"], "{warnings:?}");
    assert_eq!(warnings, ["1 adjustment layer was not applied"]);
    // Gray 100 under Linear Dodge's gray 20 gives gray 120.
    let layers = [
        layer("Under", [0, 0, 1, 1], [100, 100, 100, 255]),
        group_end(),
        masked(false),
        group("Folder", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [20, 20, 20, 255]) },
    ];
    let (doc, warnings) = psdlayers::import(&file(1, 1, &layers, 8, false, false), false, 128, cms).unwrap().unwrap();
    assert_eq!(doc.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Under + Folder + Dodge"], "{warnings:?}");
    let merged = "layers in blend modes VectorCraft doesn't have (Linear Dodge) were merged with the layers below them";
    assert_eq!(warnings, ["1 adjustment layer was not applied", merged]);
    let Some([c]) = doc.layers[0].children().map(Vec::as_slice) else { panic!("the merged layer holds one image") };
    let NodeKind::Image(img) = &c.kind else { panic!("the merged layer holds an image") };
    let px = craft_image_codecs::decode(&doc.images[&img.key].bytes).unwrap().to_rgba8();
    assert_eq!(px, [120, 120, 120, 255]);
}

/// A layer merged in a pass-through group for its mode or its knockout takes the adjustment layers
/// kept below it with it: an adjustment layer above that group in an outer pass-through group then
/// merges with it, and the gray 100 posterizes to 85.
#[test]
fn an_adjustment_layer_above_a_group_whose_kept_ones_merged_is_applied() {
    let tops = [
        ("Linear Dodge", vec![L { blend: b"lddg", ..layer("Top", [0, 0, 1, 1], [100, 100, 100, 255]) }]),
        ("knockout", vec![layer("Photo", [0, 0, 1, 1], [50, 50, 50, 255]), knockout(layer("Top", [0, 0, 1, 1], [100, 100, 100, 255]), 1, 255)]),
    ];
    for (what, top) in tops {
        let layers = [
            &[group_end(), group_end(), adjustment("Invert", b"nvrt", &[])][..],
            &top,
            &[group("Inner", b"pass"), adjustment("Posterize", b"post", &[0, 4, 0, 0]), group("Folder", b"pass")],
        ]
        .concat();
        let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(row_of(&s, sublayer(&d.layers[0], "Posterize")), [[85, 85, 85, 255]], "{what}: {r}");
        assert!(r["warnings"].to_string().contains("1 adjustment layer was not applied"), "{what}: {r}");
    }
}

/// An adjustment layer in a pass-through group, over a layer drawn below that group in an outer
/// pass-through group, isn't kept to change what is below the outer group: a layer merged above
/// them for its mode merges it applied, and the import doesn't count it as not applied. The gray
/// 100 inverts to 155.
#[test]
fn an_adjustment_layer_in_a_group_over_a_drawn_layer_merges_applied() {
    let layers = [
        group_end(),
        layer("Photo", [0, 0, 1, 1], [100, 100, 100, 255]),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        group("Inner", b"pass"),
        L { blend: b"lddg", ..layer("Top", [0, 0, 1, 1], [0, 0, 0, 255]) },
        group("Folder", b"pass"),
    ];
    let (s, r) = open(&file(1, 1, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(row_of(&s, sublayer(&d.layers[0], "Top")), [[155, 155, 155, 255]], "{r}");
    assert!(!r["warnings"].to_string().contains("adjustment"), "{r}");
}

/// A layer whose pixels are all transparent draws nothing: an adjustment layer above it in a
/// pass-through group, directly or in a pass-through group inside it, still changes what is below
/// the group.
#[test]
fn an_adjustment_layer_over_transparent_pixels_changes_what_is_below_the_group() {
    let clear = || layer("Clear", [0, 0, 1, 1], [50, 50, 50, 0]);
    let dodge = || L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) };
    let direct = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        clear(),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        dodge(),
    ];
    let (s, r) = open(&file(1, 1, &direct, 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[245, 235, 225, 255]], "{r}");
    let nested = [
        layer("Under", [0, 0, 1, 1], [10, 20, 30, 255]),
        group_end(),
        group_end(),
        clear(),
        adjustment("Invert", b"nvrt", &[]),
        group("Inner", b"pass"),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        dodge(),
    ];
    let (s, r) = open(&file(1, 1, &nested, 8, false, false), json!({}));
    assert_eq!(row_of(&s, &s.doc().unwrap().doc.layers[0]), [[255, 255, 255, 255]], "{r}");
}

/// A layer or group whose layer mask hides all of it draws nothing: an adjustment layer above it in
/// a pass-through group still changes what is below the group, and a shallow knockout hidden that
/// way merges nothing, so the adjustment layers below it stay kept. A layer's mask hides all of it
/// when it hides each of its pixels that isn't fully transparent, and a group's when it hides each
/// such pixel of the group's layers, whatever the mask's value elsewhere; a layer hidden that way
/// hides the layers clipped to it too. The layers in a pass-through group hidden that way don't
/// count as drawing below an adjustment layer above the group.
#[test]
fn an_adjustment_layer_over_layers_their_masks_hide_changes_what_is_below_the_group() {
    let black = |l: L| L { mask: Some(([0, 0, 1, 2], 0, 0, vec![0, 0])), ..l };
    let under = || layer("Under", [0, 0, 1, 2], [10, 20, 30, 255]);
    let dodge = || L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 2], [0, 0, 0, 255]) };
    let photo = || layer("Photo", [0, 0, 1, 2], [100, 100, 100, 255]);
    let inverted = [[245, 235, 225, 255], [245, 235, 225, 255]];
    let masked_layer = [under(), group_end(), black(photo()), adjustment("Invert", b"nvrt", &[]), group("Folder", b"pass"), dodge()];
    // A mask over all of the layer's pixels hides it whatever its value outside them.
    let white_outside = L { mask: Some(([0, 0, 1, 2], 255, 0, vec![0, 0])), ..photo() };
    let covered_layer = [under(), group_end(), white_outside, adjustment("Invert", b"nvrt", &[]), group("Folder", b"pass"), dodge()];
    let masked_group = [
        under(),
        group_end(),
        group_end(),
        photo(),
        black(group("Inner", b"norm")),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        dodge(),
    ];
    let knockout_cut = knockout(black(layer("Cut", [0, 0, 1, 1], [0, 0, 255, 255])), 1, 255);
    let masked_knockout = [
        under(),
        group_end(),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        layer("Photo", [0, 0, 1, 1], [100, 100, 100, 255]),
        knockout_cut,
        group("Inner", b"pass"),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        dodge(),
    ];
    // A knockout whose mask hides each pixel it doesn't leave transparent.
    let half = L { alpha: Some(vec![255, 0]), mask: Some(([0, 0, 1, 2], 0, 0, vec![0, 255])), ..layer("Cut", [0, 0, 1, 2], [0, 0, 255, 255]) };
    let mut transparent_knockout = masked_knockout.clone();
    transparent_knockout[5] = knockout(half, 1, 255);
    // A pass-through group its mask hides, below Invert in a pass-through group, with a layer drawn
    // above that group, under Posterize.
    let masked_pass = [
        under(),
        group_end(),
        group_end(),
        group_end(),
        photo(),
        black(group("Hidden", b"pass")),
        adjustment("Invert", b"nvrt", &[]),
        group("Inner", b"pass"),
        layer("Blue", [0, 0, 1, 1], [0, 0, 100, 255]),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        dodge(),
    ];
    // Invert in a pass-through group its mask hides keeps nothing above that group: Posterize
    // merges with the layer below it, as with no such group.
    let masked_invert = [
        under(),
        group_end(),
        group_end(),
        adjustment("Invert", b"nvrt", &[]),
        black(group("Hidden", b"pass")),
        layer("Blue", [0, 0, 1, 1], [0, 0, 100, 255]),
        adjustment("Posterize", b"post", &[0, 4, 0, 0]),
        group("Folder", b"pass"),
        dodge(),
    ];
    // A mask over only the pixels that aren't fully transparent, white outside it.
    let corner = |l: L| L { mask: Some(([0, 0, 1, 1], 255, 0, vec![0])), ..l };
    let half_photo = || L { alpha: Some(vec![255, 0]), ..photo() };
    let corner_layer = [under(), group_end(), corner(half_photo()), adjustment("Invert", b"nvrt", &[]), group("Folder", b"pass"), dodge()];
    let corner_group = [
        under(),
        group_end(),
        group_end(),
        half_photo(),
        corner(group("Inner", b"norm")),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        dodge(),
    ];
    // The same mask over a group with a pixel drawn outside it: the group draws there.
    let drawn_group = [
        under(),
        group_end(),
        group_end(),
        photo(),
        corner(group("Inner", b"norm")),
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        dodge(),
    ];
    // A layer its mask hides, with a layer clipped to it.
    let clipped_base = [
        under(),
        group_end(),
        black(photo()),
        L { clipped: true, ..layer("Red", [0, 0, 1, 2], [200, 0, 0, 255]) },
        adjustment("Invert", b"nvrt", &[]),
        group("Folder", b"pass"),
        dodge(),
    ];
    let cases = [
        ("layer", &masked_layer[..], inverted),
        ("covered layer", &covered_layer[..], inverted),
        ("group", &masked_group[..], inverted),
        ("knockout", &masked_knockout[..], [[85, 85, 85, 255], [255, 255, 255, 255]]),
        ("transparent knockout", &transparent_knockout[..], [[85, 85, 85, 255], [255, 255, 255, 255]]),
        ("pass-through group", &masked_pass[..], [[0, 0, 85, 255], [255, 255, 255, 255]]),
        ("adjustment in a pass-through group", &masked_invert[..], [[0, 0, 85, 255], [10, 20, 30, 255]]),
        ("layer covered where it draws", &corner_layer[..], inverted),
        ("group covered where it draws", &corner_group[..], inverted),
        ("group drawn outside its mask", &drawn_group[..], [[10, 20, 30, 255], [155, 155, 155, 255]]),
        ("clipped base", &clipped_base[..], inverted),
    ];
    let rows = |layers: &[L]| {
        let (s, _) = open(&file(2, 1, layers, 8, false, false), json!({}));
        row_of(&s, &s.doc().unwrap().doc.layers[0])
    };
    let got: Vec<_> = cases.iter().map(|(what, layers, _)| (*what, rows(layers))).collect();
    let want: Vec<_> = cases.iter().map(|(what, _, want)| (*what, want.to_vec())).collect();
    assert_eq!(got, want);
}

/// Damaged Invert, Threshold and Posterize layers, masked and clipped, open without a panic.
#[test]
fn damaged_adjustment_layers_never_panic() {
    let layers = [
        layer("A", [0, 0, 2, 2], [255, 0, 0, 255]),
        L { mask: Some(([0, 0, 1, 1], 0, 0, vec![200])), ..adjustment("Invert", b"nvrt", &[]) },
        L { clipped: true, ..adjustment("Threshold", b"thrs", &[0, 128, 0, 0]) },
        adjustment("Posterize", b"post", &[0, 3, 0, 0]),
    ];
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let good = file(2, 2, &layers, 8, false, false);
    assert!(psdlayers::import(&good, true, 1 << 30, cms).unwrap().is_some());
    for i in 0..good.len() {
        for v in [0, 1, 0x7f, 0x80, 0xff] {
            let mut f = good.clone();
            f[i] = v;
            let _ = psdlayers::import(&f, true, 1 << 30, cms);
            let _ = psdlayers::import(&f, false, 64, cms);
        }
    }
    for n in 0..good.len() {
        let _ = psdlayers::import(&good[..n], true, 1 << 30, cms);
    }
}

/// Each blend mode VectorCraft lacks merges with the layer below by its formula, from the public
/// descriptions: <https://helpx.adobe.com/photoshop/desktop/repair-retouch/adjust-light-tone/blending-mode-descriptions.html>
/// (Vivid Light, Pin Light, Hard Mix, Darker and Lighter Color) and
/// <https://en.wikipedia.org/wiki/Blend_modes> (Linear Burn, Linear Dodge, Linear Light, Subtract,
/// Divide). Color Dodge, Color Burn and Difference keep their mode.
#[test]
fn every_mode_vectorcraft_lacks_merges_by_its_formula() {
    let b = [200u8, 100, 30];
    let s = [60u8, 180, 128];
    let burn = |b: f64, c: f64| if c <= 0.0 { if b >= 1.0 { 1.0 } else { 0.0 } } else { 1.0 - ((1.0 - b) / c).min(1.0) };
    let dodge = |b: f64, c: f64| if c >= 1.0 { if b <= 0.0 { 0.0 } else { 1.0 } } else { (b / (1.0 - c)).min(1.0) };
    let each = |f: &dyn Fn(f64, f64) -> f64| -> [u8; 3] {
        [0, 1, 2].map(|i| (f(f64::from(b[i]) / 255.0, f64::from(s[i]) / 255.0).clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    let total = |c: [u8; 3]| c.iter().map(|&v| u32::from(v)).sum::<u32>();
    let cases: [(&[u8; 4], [u8; 3]); 10] = [
        (b"lbrn", each(&|b, s| b + s - 1.0)),
        (b"lddg", each(&|b, s| b + s)),
        (b"vLit", each(&|b, s| if s <= 0.5 { burn(b, 2.0 * s) } else { dodge(b, 2.0 * s - 1.0) })),
        (b"lLit", each(&|b, s| b + 2.0 * s - 1.0)),
        (b"pLit", each(&|b, s| if s <= 0.5 { b.min(2.0 * s) } else { b.max(2.0 * s - 1.0) })),
        (b"hMix", each(&|b, s| if (b + s) * 255.0 >= 254.999 { 1.0 } else { 0.0 })),
        (b"fsub", each(&|b, s| b - s)),
        (b"fdiv", each(&|b, s| if s <= 0.0 { 1.0 } else { b / s })),
        (b"dkCl", if total(s) < total(b) { s } else { b }),
        (b"lgCl", if total(s) > total(b) { s } else { b }),
    ];
    for (key, want) in cases {
        let top = L { blend: key, ..layer("Top", [0, 0, 1, 1], [s[0], s[1], s[2], 255]) };
        let (st, r) = open(&file(1, 1, &[layer("Bottom", [0, 0, 1, 1], [b[0], b[1], b[2], 255]), top], 8, false, false), json!({}));
        let d = &st.doc().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Bottom + Top"], "{key:?}: {r}");
        let got = image_of(&st, &d.layers[0]).1.get_pixel(0, 0).0;
        assert!(
            got[..3].iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 1) && got[3] == 255,
            "{}: {got:?}, want {want:?}",
            String::from_utf8_lossy(key)
        );
    }
    for (key, mode) in [(b"div ", BlendMode::ColorDodge), (b"idiv", BlendMode::ColorBurn), (b"diff", BlendMode::Difference)] {
        let top = L { blend: key, ..layer("Top", [0, 0, 1, 1], [s[0], s[1], s[2], 255]) };
        let (st, r) = open(&file(1, 1, &[layer("Bottom", [0, 0, 1, 1], [b[0], b[1], b[2], 255]), top], 8, false, false), json!({}));
        let d = &st.doc().unwrap().doc;
        let got: Vec<(&str, BlendMode)> = d.layers.iter().map(|l| (name(l), l.blend)).collect();
        assert_eq!(got, [("Bottom", BlendMode::Normal), ("Top", mode)], "{r}");
    }
}

/// A descriptor's numbers: each key (a 4-character key after a length of 0, a longer one after its
/// length), then `doub` and the value, or for `H   `, `UntF`, `#Ang` and the value.
fn descriptor(values: &[(&[u8], f64)]) -> Vec<u8> {
    let mut d = vec![0; 8];
    for (key, v) in values {
        d.extend(if key.len() == 4 { 0u32 } else { key.len() as u32 }.to_be_bytes());
        d.extend(*key);
        d.extend(if *key == b"H   " { &b"UntF#Ang"[..] } else { b"doub" });
        d.extend(v.to_be_bytes());
    }
    d
}

/// Solid color fill layers that store no pixels come in as their color in every color model a
/// descriptor uses; one whose color can't be read, and a gradient fill that stores no pixels, are
/// counted; a gradient fill that stores pixels comes in as them.
#[test]
fn solid_color_fills_read_every_color_model() {
    let fill = |name: &'static str, values: &[(&[u8], f64)]| {
        let mut l = layer(name, [0, 0, 0, 0], [0; 4]);
        l.blocks.push((*b"SoCo", descriptor(values)));
        l
    };
    let mut gradient = layer("Gradient", [0, 0, 0, 0], [0; 4]);
    gradient.blocks.push((*b"GdFl", vec![0; 8]));
    let mut gradient_shape = layer("Gradient shape", [1, 1, 2, 2], [7, 8, 9, 255]);
    gradient_shape.blocks.push((*b"GdFl", vec![0; 8]));
    // Bounds, but no channels: it stores no pixels either.
    let mut pattern = L { bare: true, ..layer("Pattern", [0, 0, 4, 4], [0; 4]) };
    pattern.blocks.push((*b"PtFl", vec![0; 8]));
    let layers = [
        fill("Float", &[(b"redFloat", 0.0), (b"greenFloat", 0.0), (b"blueFloat", 1.0)]),
        fill("Cmyk", &[(b"Cyn ", 0.0), (b"Mgnt", 100.0), (b"Ylw ", 100.0), (b"Blck", 0.0)]),
        fill("Gray", &[(b"Gry ", 25.0)]),
        fill("Lab", &[(b"Lmnc", 100.0), (b"A   ", 0.0), (b"B   ", 0.0)]),
        fill("Hsb", &[(b"H   ", 120.0), (b"Strt", 100.0), (b"Brgh", 100.0)]),
        fill("Unread", &[(b"Bk  ", 1.0)]),
        gradient,
        gradient_shape,
        pattern,
    ];
    // A naive CMYK conversion, so that the colors don't depend on the color settings.
    let naive = |c: [f32; 4]| [0, 1, 2].map(|i| (1.0 - c[i]) * (1.0 - c[3]));
    let (doc, warnings) = psdlayers::import(&file(4, 4, &layers, 8, false, false), false, 1 << 20, naive).unwrap().unwrap();
    let names: Vec<&str> = doc.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Float", "Cmyk", "Gray", "Lab", "Hsb", "Gradient shape"]);
    let mut s = Session::new();
    s.add_document(doc, None);
    let d = &s.doc().unwrap().doc;
    let colors: Vec<[u8; 4]> = d.layers[..5].iter().map(|l| image_of(&s, l).1.get_pixel(2, 2).0).collect();
    assert_eq!(colors[..3], [[0, 0, 255, 255], [255, 0, 0, 255], [191, 191, 191, 255]]);
    assert!(colors[3].iter().all(|&v| v >= 253), "Lab 100, 0, 0 is white: {:?}", colors[3]);
    assert_eq!(colors[4], [0, 255, 0, 255]);
    assert_eq!(image_of(&s, &d.layers[5]).1.get_pixel(0, 0).0, [7, 8, 9, 255]);
    assert_eq!(warnings, ["3 fill layers were not drawn (a gradient or pattern fill, a color that can't be read, or a shape that stores no pixels)"]);
}

/// VectorCraft's own PSD export opens with the layers it wrote: groups as layers of sublayers,
/// hidden ones hidden, with their opacity and blend mode.
#[test]
fn psd_exports_open_with_their_layers() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 30, "height": 40})).unwrap();
    s.execute("layer.new", &json!({"name": "Blue"})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50, "blend": "Multiply"})).unwrap();
    s.execute("layer.new", &json!({"name": "Off"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
    let off = s.doc().unwrap().doc.layers.last().unwrap().id;
    s.execute("layer.setProps", &json!({"id": off.0, "visible": false})).unwrap();
    for (options, top) in [
        (json!({"hiddenLayers": true}), vec![("Layer 1", true), ("Blue", true), ("Off", false)]),
        (json!({"maxEditability": true}), vec![("Layer 1", true), ("Blue", true)]),
    ] {
        let r = s.execute("document.export", &merge(json!({"format": "psd"}), options.clone())).unwrap();
        let f = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
        let (back, r) = open(&f, json!({"hiddenLayers": true}));
        let d = &back.doc().unwrap().doc;
        let got: Vec<(&str, bool)> = d.layers.iter().map(|l| (name(l), l.visible)).collect();
        assert_eq!(got, top, "{options}: {r}");
        if options["maxEditability"] == true {
            // Each layer a group: its objects become sublayers, the rectangle's opacity and blend mode
            // on its own.
            let blue = d.layers[1].children().unwrap();
            assert_eq!(blue.len(), 1, "{options}");
            assert!(blue[0].blend == BlendMode::Multiply && (blue[0].opacity - 0.5).abs() < 0.01, "{options}");
        }
    }
}

#[test]
fn files_without_layers_or_with_damaged_layers_open_as_their_merged_image() {
    // No layer records: the merged image, without a warning.
    let flat = {
        let mut f = file(2, 2, &[layer("x", [0, 0, 2, 2], [9, 9, 9, 255])], 8, false, false);
        // The layer and mask section emptied: 4 length bytes of 0, after the header and two
        // empty sections.
        let at = 26 + 8;
        let len = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) as usize;
        f.splice(at..at + 4 + len, 0u32.to_be_bytes());
        f
    };
    let (_, r) = open(&flat, json!({}));
    assert_eq!(r["warnings"], json!([]), "{r}");
    // Layer info of just the count (negative: the merged image has transparency) has no layers.
    let count_only = {
        let mut f = flat.clone();
        let at = 26 + 8;
        f.splice(at..at + 4, [&10u32.to_be_bytes()[..], &2u32.to_be_bytes(), &(-1i16).to_be_bytes(), &[0, 0, 0, 0]].concat());
        f
    };
    let (s, r) = open(&count_only, json!({}));
    assert_eq!((&r["warnings"], s.doc().unwrap().doc.layers[0].children().map(Vec::len)), (&json!([]), Some(1)), "{r}");
    // A channel cut short: the merged image, with a warning.
    let mut cut = file(4, 4, &[layer("x", [0, 0, 4, 4], [9, 9, 9, 255])], 8, false, false);
    let at = cut.windows(4).position(|w| w == [0, 0, 0, 18]).unwrap();
    cut[at..at + 4].copy_from_slice(&1000u32.to_be_bytes());
    let (s, r) = open(&cut, json!({}));
    assert!(r["warnings"][0].as_str().unwrap().starts_with("the layers can't be read"), "{r}");
    assert_eq!(s.doc().unwrap().doc.layers[0].children().map(Vec::len), Some(1));
    // Groups nested past the limit.
    let mut deep: Vec<L> = (0..=psdlayers::MAX_DEPTH).map(|_| group_end()).collect();
    deep.push(layer("x", [0, 0, 1, 1], [0, 0, 0, 255]));
    let (_, r) = open(&file(2, 2, &deep, 8, false, false), json!({}));
    assert!(r["warnings"][0].as_str().unwrap().contains("nested too deeply"), "{r}");
    // Groups within the limit but deeper than a document may nest: the deepest merge into the
    // group around them, with the warning.
    let mut deep: Vec<L> = (0..40).map(|_| group_end()).collect();
    deep.push(layer("x", [0, 0, 1, 1], [0, 0, 0, 255]));
    deep.extend((0..40).map(|_| group("g", b"norm")));
    let (s, r) = open(&file(2, 2, &deep, 8, false, false), json!({}));
    assert_eq!(r["warnings"], json!([vectorcraft_doc::depth::merged_note()]), "{r}");
    fn depth(n: &Node) -> usize {
        1 + n.children().map_or(0, |c| c.iter().map(|c| depth(c)).max().unwrap_or(0))
    }
    assert!(s.doc().unwrap().doc.layers.iter().map(|l| depth(l)).max().unwrap() <= vectorcraft_doc::MAX_DEPTH);
}

/// A record whose layer mask or name is cut short opens the merged image with a warning, rather
/// than a layer without its mask or name.
#[test]
fn records_with_a_damaged_mask_or_name_open_as_the_merged_image() {
    let good = file(4, 4, &[layer("x", [0, 0, 4, 4], [9, 9, 9, 255])], 8, false, false);
    // The record's extra data: an empty mask section, empty blending ranges, then the name.
    let at = good.windows(10).position(|w| w == [0, 0, 0, 0, 0, 0, 0, 0, 1, b'x']).unwrap();
    // A mask section of 4 bytes, too short for a mask's place, default color and flags.
    let mut short_mask = good.clone();
    short_mask[at + 3] = 4;
    // A name longer than the record's extra data.
    let mut long_name = good.clone();
    long_name[at + 8] = 0xff;
    for f in [short_mask, long_name] {
        let (s, r) = open(&f, json!({}));
        assert!(r["warnings"][0].as_str().unwrap().starts_with("the layers can't be read"), "{r}");
        assert_eq!(s.doc().unwrap().doc.layers[0].children().map(Vec::len), Some(1), "{r}");
    }
}

#[test]
fn damaged_layers_never_panic() {
    let mut masked = layer("Masked", [0, 0, 3, 3], [1, 2, 3, 255]);
    masked.mask = Some(([1, 1, 2, 2], 0, 0, vec![200]));
    let layers = [
        layer("A", [0, 0, 3, 3], [255, 0, 0, 255]),
        group_end(),
        masked,
        L { clipped: true, ..layer("C", [0, 0, 3, 3], [0, 0, 255, 255]) },
        L { blend: b"lbrn", ..layer("D", [0, 0, 2, 2], [9, 9, 9, 255]) },
        group("G", b"pass"),
    ];
    for (rle, psb) in [(false, false), (true, true)] {
        let good = file(3, 3, &layers, 8, rle, psb);
        let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
        assert!(psdlayers::import(&good, true, 1 << 30, cms).unwrap().is_some());
        for i in 0..good.len() {
            for v in [0, 1, 0x7f, 0x80, 0xff] {
                let mut f = good.clone();
                f[i] = v;
                let _ = psdlayers::import(&f, true, 1 << 30, cms);
                let _ = psdlayers::info(&f);
            }
        }
        for n in 0..good.len() {
            let _ = psdlayers::import(&good[..n], true, 1 << 30, cms);
            let _ = psdlayers::info(&good[..n]);
        }
        // A layer past the memory allowed is refused, not read.
        assert!(psdlayers::import(&good, true, 64, cms).unwrap_err().contains("too large"));
    }
    // Many layers that each fit the memory allowed for one image, but not all together (8 bytes a
    // pixel to read an 8 × 8 layer; 4 a pixel decoded, against 8 times the allowance).
    let cms = |c: [f32; 4]| [c[0], c[1], c[2]];
    let many: Vec<L> = (0..20).map(|_| layer("x", [0, 0, 8, 8], [1, 2, 3, 255])).collect();
    assert!(psdlayers::import(&file(8, 8, &many[..16], 8, false, false), false, 512, cms).unwrap().is_some());
    let e = psdlayers::import(&file(8, 8, &many, 8, false, false), false, 512, cms).err();
    assert!(e.as_deref().is_some_and(|e| e.contains("take more memory to read than allowed")), "{e:?}");
    // Looking through those layers' transparency for pixels that draw, below an adjustment layer,
    // takes nothing from the allowance.
    let mut clear: Vec<L> = (0..16).map(|_| layer("x", [0, 0, 8, 8], [1, 2, 3, 0])).collect();
    clear.push(adjustment("Invert", b"nvrt", &[]));
    let opened = psdlayers::import(&file(8, 8, &clear, 8, false, false), false, 512, cms);
    assert!(opened.as_ref().is_ok_and(Option::is_some), "{:?}", opened.err());
}

/// A layer in a mode VectorCraft lacks above a pass-through group merges with the group's layers
/// blending onto what is below the group, as they do without the merge.
#[test]
fn merges_keep_pass_through_groups_blending_with_what_is_below() {
    let layers = [
        layer("Bottom", [0, 0, 2, 2], [100, 200, 50, 255]),
        group_end(),
        L { blend: b"mul ", ..layer("Multiply", [0, 0, 2, 2], [128, 128, 128, 255]) },
        group("Pass", b"pass"),
        L { blend: b"lddg", ..layer("Dodge", [0, 0, 1, 1], [0, 0, 0, 255]) },
    ];
    let (s, r) = open(&file(2, 2, &layers, 8, false, false), json!({}));
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| name(l)).collect();
    assert_eq!(names, ["Bottom + Pass + Dodge"], "{r}");
    // 100, 200 and 50 times 128/255.
    assert_eq!(image_of(&s, &d.layers[0]).1.get_pixel(1, 1).0, [50, 100, 25, 255]);
    // At half opacity, or with a layer mask, the group's layers still blend with what is below
    // it, and the result fades back toward it: halfway from (100, 200, 50) to (50, 100, 25).
    let mut masked = group("Pass", b"pass");
    masked.mask = Some(([0, 0, 2, 2], 0, 0, vec![128, 128, 128, 128]));
    for top in [L { opacity: 128, ..group("Pass", b"pass") }, masked] {
        let mut layers = layers.clone();
        layers[3] = top;
        let (s, r) = open(&file(2, 2, &layers, 8, false, false), json!({}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| name(l)).collect::<Vec<_>>(), ["Bottom + Pass + Dodge"], "{r}");
        let px = image_of(&s, &d.layers[0]).1.get_pixel(1, 1).0;
        assert!(px.iter().zip([75, 150, 38, 255]).all(|(&a, b)| a.abs_diff(b) <= 1), "{px:?}");
    }
}

/// `file.place` of `f` with `p` into a new 100 × 100 pt document, centred on (50, 50).
fn place(f: &[u8], p: Value) -> (Session, crate::cmd::Result<Value>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.place", &merge(json!({"name": "layers.psd", "dataBase64": vectorcraft_format::base64_encode(f), "at": [50, 50]}), p));
    (s, r)
}

/// The one object `file.place` placed.
fn placed(s: &Session, r: &Value) -> Node {
    let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

#[test]
fn placing_with_layers_embeds_the_layers_as_one_group() {
    let f = file(8, 8, &stack(), 8, false, false);
    let (mut s, r) = place(&f, json!({"layers": true}));
    let r = r.unwrap();
    assert_eq!((&r["format"], &r["linked"], &r["warnings"]), (&json!("psd"), &json!(false), &json!([HIDDEN_1])), "{r}");
    let g = placed(&s, &r);
    assert!(matches!(g.kind, NodeKind::Group { clip: false, .. }), "one group, unclipped: its layers lie on the canvas");
    assert_eq!(name(&g), "layers.psd");
    let kids = g.children().unwrap();
    assert_eq!(kids.iter().map(|c| name(c)).collect::<Vec<_>>(), ["Bottom", "Folder", "Screens", "Faded"], "bottom first");
    // A layer is its image, where the merged image would put it: the 8 pt canvas centred on (50, 50).
    let ((x, y), px) = pixels_of(&s, &kids[0]);
    assert_eq!(((x, y), px.dimensions(), px.get_pixel(3, 3).0), ((46.0, 46.0), (8, 8), [255, 0, 0, 255]));
    // A group is a group of its layers; a pass-through group isn't isolated, another mode's is.
    let folder = &kids[1];
    assert!(matches!(folder.kind, NodeKind::Group { .. }) && !folder.isolate && folder.blend == BlendMode::Normal);
    let inner = folder.children().unwrap();
    assert_eq!(inner.iter().map(|c| name(c)).collect::<Vec<_>>(), ["Inner A"], "the hidden layer is left out");
    assert!(matches!(inner[0].kind, NodeKind::Image(_)) && inner[0].blend == BlendMode::Multiply && (inner[0].opacity - 128.0 / 255.0).abs() < 1e-6);
    let screens = &kids[2];
    assert!(screens.isolate && screens.blend == BlendMode::Screen);
    let ((x, y), px) = pixels_of(&s, &screens.children().unwrap()[0]);
    assert_eq!(((x, y), px.get_pixel(0, 0).0), ((48.0, 48.0), [10, 20, 30, 128]));
    assert!((kids[3].opacity - 102.0 / 255.0).abs() < 1e-6, "fill opacity: {}", kids[3].opacity);
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.layers.iter().all(|l| l.children().is_none_or(Vec::is_empty)));
    // Hidden layers come in hidden when asked for.
    let (s, r) = place(&f, json!({"layers": true, "hiddenLayers": true}));
    let r = r.unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    let g = placed(&s, &r);
    let inner: Vec<(&str, bool)> = g.children().unwrap()[1].children().unwrap().iter().map(|c| (name(c), c.visible)).collect();
    assert_eq!(inner, [("Inner A", true), ("Inner B", false)]);
    // Without `layers`, the merged image, in the same box.
    let (s, r) = place(&f, json!({}));
    let r = r.unwrap();
    let flat = placed(&s, &r);
    let ((x, y), px) = pixels_of(&s, &flat);
    assert_eq!(((x, y), px.dimensions(), px.get_pixel(0, 0).0), ((46.0, 46.0), (8, 8), [128, 128, 128, 255]), "{r}");
}

#[test]
fn placed_layers_keep_their_masks_and_are_clipped_to_the_canvas() {
    let mut masked = layer("Masked", [0, 0, 8, 8], [255, 255, 255, 255]);
    masked.mask = Some(([2, 2, 4, 4], 0, 0, vec![255, 128, 0, 255]));
    let (s, r) = place(&file(8, 8, &[masked], 8, false, false), json!({"layers": true}));
    let g = placed(&s, &r.unwrap());
    let kids = g.children().unwrap();
    assert!(matches!(kids[0].kind, NodeKind::Image(_)), "the layer is its image");
    let m = kids[0].extras().mask.as_deref().expect("the layer mask as the image's opacity mask");
    let ((x, y), px) = pixels_of(&s, &m.art);
    assert_eq!(((x, y), px.dimensions(), m.clip), ((48.0, 48.0), (2, 2), true), "the mask moves with its layer");
    // A layer that reaches past the canvas: the group is clipped to the canvas, as the merged
    // image shows only the canvas.
    let over = layer("Over", [-2, -2, 6, 6], [0, 255, 0, 255]);
    let (s, r) = place(&file(8, 8, &[layer("Bottom", [0, 0, 8, 8], [255, 0, 0, 255]), over], 8, false, false), json!({"layers": true}));
    let r = r.unwrap();
    let g = placed(&s, &r);
    assert!(matches!(g.kind, NodeKind::Group { clip: true, .. }), "{r}");
    let kids = g.children().unwrap();
    let b = kids[0].geometric_bounds_with(&s.doc().unwrap().doc.symbol_boxes()).unwrap();
    assert!(matches!(kids[0].kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (46.0, 46.0, 54.0, 54.0), "the clipping path is the canvas");
    assert_eq!(kids[1..].iter().map(|c| name(c)).collect::<Vec<_>>(), ["Bottom", "Over"]);
    assert_eq!(pixels_of(&s, &kids[2]).0, (44.0, 44.0));
    // A hidden layer that reaches past the canvas clips the group too, so showing it later draws
    // only the canvas. Left out, it doesn't.
    let mut hidden = layer("Hidden", [-2, -2, 6, 6], [0, 255, 0, 255]);
    hidden.hidden = true;
    let f = file(8, 8, &[layer("Bottom", [0, 0, 8, 8], [255, 0, 0, 255]), hidden], 8, false, false);
    let (s, r) = place(&f, json!({"layers": true, "hiddenLayers": true}));
    let r = r.unwrap();
    let g = placed(&s, &r);
    assert!(matches!(g.kind, NodeKind::Group { clip: true, .. }), "{r}");
    assert_eq!(g.children().unwrap()[1..].iter().map(|c| (name(c), c.visible)).collect::<Vec<_>>(), [("Bottom", true), ("Hidden", false)]);
    let (s, r) = place(&f, json!({"layers": true}));
    assert!(matches!(placed(&s, &r.unwrap()).kind, NodeKind::Group { clip: false, .. }));
}

#[test]
fn placing_with_layers_never_links_and_refuses_values_it_cant_use() {
    let f = file(8, 8, &stack(), 8, false, false);
    let dir = std::env::temp_dir().join(format!("vc-psd-place-layers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("layers.psd");
    std::fs::write(&path, &f).unwrap();
    let from_path = |p: Value| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = s.execute("file.place", &merge(json!({"path": path.to_str().unwrap()}), p));
        (s, r)
    };
    // From a path, `layers` embeds: `link` is then false unless given, and `link: true` is refused.
    let (s, r) = from_path(json!({"layers": true}));
    let r = r.unwrap();
    assert_eq!(r["linked"], json!(false), "{r}");
    assert!(matches!(placed(&s, &r).kind, NodeKind::Group { .. }));
    let (s, r) = from_path(json!({"layers": true, "link": true}));
    assert!(r.unwrap_err().to_string().contains("a linked PSD file is placed as its flat image"));
    assert!(s.doc().unwrap().doc.layers.iter().all(|l| l.children().is_none_or(Vec::is_empty)), "nothing is placed");
    // Without `layers` the file is linked as before.
    let (s, r) = from_path(json!({}));
    let r = r.unwrap();
    assert_eq!(r["linked"], json!(true), "{r}");
    assert!(matches!(placed(&s, &r).kind, NodeKind::Image(_)));
    // A file without layers is embedded as its merged image, without a warning.
    let flat = {
        let mut f = file(2, 2, &[layer("x", [0, 0, 2, 2], [9, 9, 9, 255])], 8, false, false);
        let at = 26 + 8;
        let len = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) as usize;
        f.splice(at..at + 4 + len, 0u32.to_be_bytes());
        f
    };
    std::fs::write(&path, &flat).unwrap();
    let (s, r) = from_path(json!({"layers": true}));
    let r = r.unwrap();
    assert_eq!((&r["linked"], &r["warnings"]), (&json!(false), &json!([])), "{r}");
    assert!(matches!(placed(&s, &r).kind, NodeKind::Image(_)));
    // Layers that are all left out: the merged image, with the importer's warnings.
    let mut gone = layer("Gone", [0, 0, 2, 2], [9, 9, 9, 255]);
    gone.hidden = true;
    let (s, r) = place(&file(2, 2, &[gone], 8, false, false), json!({"layers": true}));
    let r = r.unwrap();
    assert_eq!((&r["linked"], r["warnings"].as_array().map(Vec::len)), (&json!(false), Some(1)), "{r}");
    assert!(matches!(placed(&s, &r).kind, NodeKind::Image(_)));
    // Layers that can't be read: the merged image, with a warning.
    let mut cut = file(4, 4, &[layer("x", [0, 0, 4, 4], [9, 9, 9, 255])], 8, false, false);
    let at = cut.windows(4).position(|w| w == [0, 0, 0, 18]).unwrap();
    cut[at..at + 4].copy_from_slice(&1000u32.to_be_bytes());
    let (s, r) = place(&cut, json!({"layers": true}));
    let r = r.unwrap();
    assert!(r["warnings"][0].as_str().unwrap().starts_with("the layers can't be read"), "{r}");
    assert!(matches!(placed(&s, &r).kind, NodeKind::Image(_)));
    // Values the command can't use are errors, and nothing is placed.
    for (p, why) in [
        (json!({"hiddenLayers": true}), "hiddenLayers applies to a PSD file placed with layers: true"),
        (json!({"layers": true, "link": true}), "a linked PSD file is placed as its flat image"),
        (json!({"layers": "yes"}), "layers must be true or false"),
        (json!({"layers": true, "hiddenLayers": 1}), "hiddenLayers must be true or false"),
    ] {
        let (s, r) = place(&f, p.clone());
        assert!(r.unwrap_err().to_string().contains(why), "{p}");
        assert!(s.doc().unwrap().doc.layers.iter().all(|l| l.children().is_none_or(Vec::is_empty)), "{p}");
    }
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let q = s.execute("file.place.queue", &json!({"paths": [path.to_str().unwrap()], "layers": "yes"}));
    assert!(q.unwrap_err().to_string().contains("`layers` must be true or false"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn psd_info_counts_the_layers_without_opening_the_file() {
    let mut s = Session::new();
    let info = |s: &mut Session, f: &[u8]| s.execute("document.psdInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(f)}));
    let r = info(&mut s, &file(8, 8, &stack(), 8, false, false)).unwrap();
    assert_eq!(r, json!({"format": "psd", "width": 8, "height": 8, "layers": 5, "hiddenLayers": 1, "groups": 2, "slices": 0}));
    let r = info(&mut s, &file(4, 2, &[layer("x", [0, 0, 2, 2], [9, 9, 9, 255])], 16, true, true)).unwrap();
    assert_eq!((&r["format"], &r["width"], &r["height"], &r["layers"]), (&json!("psb"), &json!(4), &json!(2), &json!(1)), "{r}");
    // Layer records that can't be read count as no layers: the file opens as its merged image.
    let mut cut = file(4, 4, &[layer("x", [0, 0, 4, 4], [9, 9, 9, 255])], 8, false, false);
    let at = cut.windows(4).position(|w| w == [0, 0, 0, 18]).unwrap();
    cut[at..at + 4].copy_from_slice(&1000u32.to_be_bytes());
    assert_eq!(info(&mut s, &cut).unwrap()["layers"], json!(0));
    assert!(info(&mut s, b"not a psd").unwrap_err().to_string().contains("not a PSD file"));
}
