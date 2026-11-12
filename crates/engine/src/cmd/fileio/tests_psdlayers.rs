//! Opening PSD and PSB files with their layers as layers (`document.open`'s `layers` and
//! `hiddenLayers`): names, visibility, opacity, blend modes, groups, layer masks, clipped layers
//! and blend modes VectorCraft lacks merged, 16-bit and PSB files, VectorCraft's own PSD export
//! read back, and damaged files read without a panic.

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

/// A channel of `n` samples of `v` (in 32 bits, `v` / 255 as a float) at `depth`, raw, zipped
/// (with prediction when `zip` is `Some(true)`) or PackBits rows of `width` when `rle`.
fn channel(v: u8, n: usize, width: usize, depth: u16, rle: bool, psb: bool, zip: Option<bool>) -> Vec<u8> {
    let sample = if depth == 32 { (f32::from(v) / 255.0).to_be_bytes().to_vec() } else { vec![v; usize::from(depth / 8)] };
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
        let mut mask = vec![];
        if let Some(([mt, ml, mb, mr], default, flags, pixels)) = &l.mask {
            let mut d = vec![0, 0];
            for &p in pixels {
                d.extend(std::iter::repeat_n(p, usize::from(depth / 8)));
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
    let px = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
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
    let l = L { legacy: Some(b"Caf\x8E \xD2Old\xD3"), ..layer("", [0, 0, 1, 1], [0, 0, 0, 255]) };
    let (s, r) = open(&file(1, 1, &[l], 8, false, false), json!({}));
    assert_eq!(name(&s.doc().unwrap().doc.layers[0]), "Café “Old”", "{r}");
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
            }
        }
        for n in 0..good.len() {
            let _ = psdlayers::import(&good[..n], true, 1 << 30, cms);
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
