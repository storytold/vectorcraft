//! Synthetic FreeHand 8–11 files for tests and fuzz seeds. This is not a FreeHand writer: it only
//! emits the few records the reader is tested against. Version 8 uses the raw stream and the
//! layout deltas in the format description; 9–11 use zlib.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

struct Buf(Vec<u8>);

impl Buf {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_be_bytes());
    }
    fn fixed(&mut self, v: f64) {
        let n = (v * 65536.0).round();
        let n = if n.is_finite() { n.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32 } else { 0 };
        self.u32(n as u32);
    }
    fn id(&mut self, v: u32) {
        if v < 0xFFFF {
            self.u16(v as u16);
        } else {
            self.u16(0xFFFF);
            self.u16(0x1_FF00u32.wrapping_sub(v) as u16);
        }
    }
    fn zeros(&mut self, n: usize) {
        self.0.resize(self.0.len() + n, 0);
    }
}

fn mname(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let size = u16::try_from(bytes.len().div_ceil(4).max(1)).unwrap_or(1);
    let mut w = Buf(Vec::new());
    w.u16(size);
    w.u16(u16::try_from(bytes.len()).unwrap_or(0));
    w.0.extend_from_slice(bytes);
    let total = (usize::from(size) + 1) * 4;
    w.0.resize(total, 0);
    w.0
}

fn color6(r: u16, g: u16, b: u16) -> Vec<u8> {
    color6_at(r, g, b, 9)
}

fn color6_at(r: u16, g: u16, b: u16, version: u8) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u16(3);
    w.id(0);
    w.u16(r);
    w.u16(g);
    w.u16(b);
    w.u32(0);
    w.id(0);
    // Variant 3's tail is two bytes shorter before FreeHand 10.
    w.zeros(if version < 10 { 10 } else { 12 });
    w.0
}

fn basic_fill(color: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(color);
    w.u32(0);
    w.0
}

fn line_pat(a: f64, b: f64) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u16(2);
    w.zeros(8);
    w.fixed(a);
    w.fixed(b);
    w.0
}

fn basic_line(color: u32, dashes: u32, width: f64) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(color);
    w.id(dashes);
    w.id(0);
    w.id(0);
    w.fixed(4.0);
    w.fixed(width);
    w.u32(0);
    w.0
}

fn elem(pairs: &[(u32, u32)]) -> Vec<u8> {
    elem_at(pairs, 9)
}

fn elem_at(pairs: &[(u32, u32)], version: u8) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    // Version 8 has no leading skip. The first u16 is the pair count.
    if version >= 9 {
        w.u16(0);
    }
    w.u16(u16::try_from(pairs.len()).unwrap_or(0));
    w.u16(0);
    w.id(0);
    w.id(0);
    for (name, value) in pairs {
        w.id(*name);
        w.id(*value);
    }
    w.0
}

fn node(w: &mut Buf, x: f64, y: f64) {
    w.zeros(3);
    w.fixed(x);
    w.fixed(y);
    w.fixed(x);
    w.fixed(y);
    w.fixed(x);
    w.fixed(y);
}

fn path(style: u32, flags: u8, pts: &[[f64; 2]]) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u16(u16::try_from(pts.len()).unwrap_or(0));
    w.id(style);
    w.id(0);
    w.u32(0);
    w.zeros(9);
    w.u8(flags);
    w.u16(u16::try_from(pts.len()).unwrap_or(0));
    for p in pts {
        node(&mut w, p[0], p[1]);
    }
    w.0
}

/// A translation. Version 9 and later add an identity inverse the reader skips.
fn xform_at(x: f64, y: f64, version: u8) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    if version <= 8 {
        w.zeros(2);
        w.fixed(1.0);
        w.fixed(0.0);
        w.fixed(0.0);
        w.fixed(1.0);
        w.fixed(x);
        w.fixed(y);
        w.zeros(26);
        return w.0;
    }
    w.u8(0x33);
    w.u8(0x00);
    w.fixed(x);
    w.fixed(y);
    w.u8(0x04);
    w.u8(0);
    w.0
}

fn group(elements: u32, xform: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(0);
    w.id(0);
    w.u32(0);
    w.u32(0);
    w.id(elements);
    w.id(xform);
    w.0
}

fn list(ids: &[u32]) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    let n = u16::try_from(ids.len()).unwrap_or(0);
    w.u16(n);
    w.u16(n);
    w.zeros(6);
    w.u16(0);
    for id in ids {
        w.id(*id);
    }
    w.0
}

fn rectangle_at(style: u32, b: [f64; 4], version: u8) -> Vec<u8> {
    let mut w = Buf(box_shape(style, b, Some((0.0, 0.0))));
    if version >= 11 {
        // Four extra corners, then the 9-byte tail. Zero keeps one sharp rectangle.
        w.zeros(6 * 4 + 9);
    }
    w.0
}

fn oval_at(style: u32, b: [f64; 4], version: u8) -> Vec<u8> {
    let mut w = Buf(box_shape(style, b, None));
    if version >= 11 {
        w.fixed(0.0);
        w.fixed(0.0);
        w.u8(0);
        w.u8(0);
    }
    w.0
}

fn box_shape(style: u32, b: [f64; 4], radii: Option<(f64, f64)>) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(style);
    w.id(0);
    w.u32(0);
    w.zeros(8);
    w.id(0);
    for v in b {
        w.fixed(v);
    }
    if let Some((rx, ry)) = radii {
        w.fixed(rx);
        w.fixed(ry);
    }
    w.0
}

fn stops(a: u32, b: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u16(2);
    w.u16(0);
    w.id(a);
    w.fixed(0.0);
    w.u32(0);
    w.id(b);
    w.fixed(1.0);
    w.u32(0);
    w.0
}

fn gradient(c1: u32, c2: u32, angle: f64, stops: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(c1);
    w.id(c2);
    w.fixed(angle);
    w.zeros(8);
    w.id(stops);
    w.0
}

fn page(x: f64, y: f64, wdt: f64, hgt: f64) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u32(0);
    w.u16(4);
    w.u16(0);
    for (id, v) in [(0x1C24, x), (0x1C2C, y), (0x1C34, wdt), (0x1C3C, hgt)] {
        w.u16(0);
        w.u16(id);
        w.fixed(v);
    }
    w.0
}

fn prop_list(name: u32, value: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.u16(1);
    w.u16(1);
    w.u32(0);
    w.id(name);
    w.id(value);
    w.0
}

fn layer(elements: u32, name: u32) -> Vec<u8> {
    layer_vis(elements, name, 3)
}

fn layer_vis(elements: u32, name: u32, visibility: u16) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(0);
    w.u32(0);
    w.zeros(6);
    w.id(elements);
    w.id(name);
    w.u16(visibility);
    w.u16(0);
    w.0
}

fn block(layers: u32) -> Vec<u8> {
    block_at(layers, 9)
}

fn block_at(layers: u32, version: u8) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    if version == 10 {
        w.u16(0);
        for i in 0..21 {
            w.id(if i == 5 { layers } else { 0 });
        }
        w.u8(0);
        w.id(0);
        w.id(0);
        return w.0;
    }
    if version <= 8 {
        for i in 0..12 {
            w.id(if i == 5 { layers } else { 0 });
        }
        w.zeros(14);
        return w.0;
    }
    for i in 0..12 {
        w.id(if i == 5 { layers } else { 0 });
    }
    w.zeros(14);
    w.id(0);
    w.id(0);
    w.id(0);
    w.u8(0);
    let trailing = if version >= 11 { 4 } else { 1 };
    for _ in 0..trailing {
        w.id(0);
    }
    w.0
}

fn pack<S: AsRef<str>>(records: &[(S, Vec<u8>)], block_id: u32, props_id: u32) -> Vec<u8> {
    pack_at(records, block_id, props_id, 9)
}

fn pack_at<S: AsRef<str>>(records: &[(S, Vec<u8>)], block_id: u32, props_id: u32, version: u8) -> Vec<u8> {
    let mut names: Vec<&str> = Vec::new();
    for (name, _) in records {
        let name = name.as_ref();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut body = Vec::new();
    for (_, rec) in records {
        body.extend_from_slice(rec);
    }
    let mut tail = vec![0u8; 0x22];
    let block_n = u16::try_from(block_id).unwrap_or(0);
    let props_n = u16::try_from(props_id).unwrap_or(0);
    if let Some(slot) = tail.get_mut(..2) {
        slot.copy_from_slice(&block_n.to_be_bytes());
    }
    if let Some(slot) = tail.get_mut(2..4) {
        slot.copy_from_slice(&props_n.to_be_bytes());
    }
    // A trailer page size that must lose to the real page record.
    let bogus = ((50.0_f64) * 65536.0) as u32;
    tail[0x1A..0x1E].copy_from_slice(&bogus.to_be_bytes());
    tail[0x1E..0x22].copy_from_slice(&bogus.to_be_bytes());
    body.extend_from_slice(&tail);

    let payload = if version <= 8 {
        body
    } else {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        if enc.write_all(&body).is_err() {
            return Vec::new();
        }
        let Ok(compressed) = enc.finish() else { return Vec::new() };
        compressed
    };
    let data_len = u32::try_from(12 + payload.len()).unwrap_or(0);
    let digit = b'0'.saturating_add(version.saturating_sub(5));

    let mut file = Vec::new();
    file.extend_from_slice(&[b'A', b'G', b'D', digit]);
    file.extend(0u32.to_be_bytes());
    file.extend(data_len.to_be_bytes());
    file.extend_from_slice(&payload);

    let mut dict = Buf(Vec::new());
    dict.u16(u16::try_from(names.len()).unwrap_or(0));
    dict.u16(0);
    for (i, name) in names.iter().enumerate() {
        dict.u16(u16::try_from(i + 1).unwrap_or(0));
        if version <= 8 {
            dict.u16(0);
        }
        dict.0.extend_from_slice(name.as_bytes());
        dict.u8(0);
        if version <= 8 {
            dict.u8(0);
            dict.u8(0);
        }
    }
    file.extend_from_slice(&dict.0);
    file.extend(u32::try_from(records.len()).unwrap_or(0).to_be_bytes());
    for (name, _) in records {
        let name = name.as_ref();
        let id = names.iter().position(|n| *n == name).and_then(|i| u16::try_from(i + 1).ok()).unwrap_or(0);
        file.extend(id.to_be_bytes());
    }
    file
}

fn symbol_class(group: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(0);
    w.id(group);
    w.id(0);
    w.id(0);
    w.id(0);
    w.0
}

fn symbol_instance(class: u32) -> Vec<u8> {
    let mut w = Buf(Vec::new());
    w.id(0);
    w.id(0);
    w.zeros(8);
    w.id(class);
    // Identity matrix: the flag bit that means "no components".
    w.u8(0x04);
    w.u8(0);
    w.0
}

/// A page, a layer, and `levels` groups inside one another, the innermost holding one path.
/// `levels` above a few tens of thousands would overflow the trailer's 16-bit block id.
pub fn nested_groups(levels: u32) -> Vec<u8> {
    let levels = levels.clamp(1, 20_000);
    let mut records: Vec<(String, Vec<u8>)> = Vec::new();
    records.push(("MName".into(), mname("pages")));
    records.push(("MName".into(), mname("Ink")));
    records.push(("Path".into(), path(0, 1, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]])));
    let mut child = 3u32;
    for _ in 0..levels {
        let list_id = u32::try_from(records.len()).unwrap_or(0).saturating_add(1);
        records.push(("MList".into(), list(&[child])));
        child = u32::try_from(records.len()).unwrap_or(0).saturating_add(1);
        records.push(("Group".into(), group(list_id, 0)));
    }
    let top = child;
    let layer_list = push(&mut records, "MList", list(&[top]));
    let layer = push(&mut records, "Layer", layer(layer_list, 2));
    let layers = push(&mut records, "MList", list(&[layer]));
    let page_id = push(&mut records, "VMpObj", page(0.0, 0.0, 100.0, 100.0));
    let pages = push(&mut records, "MList", list(&[page_id]));
    let props = push(&mut records, "PropLst", prop_list(1, pages));
    let block_id = push(&mut records, "Block", block(layers));
    pack(&records, block_id, props)
}

/// Two groups whose child lists point at each other.
pub fn cyclic_groups() -> Vec<u8> {
    // 1 pages, 2 Ink, 3 path, 4 list→group B, 5 list→group A, 6 group A, 7 group B.
    let records: Vec<(&str, Vec<u8>)> = vec![
        ("MName", mname("pages")),
        ("MName", mname("Ink")),
        ("Path", path(0, 1, &[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0]])),
        ("MList", list(&[7])),
        ("MList", list(&[6, 3])),
        ("Group", group(4, 0)),
        ("Group", group(5, 0)),
        ("MList", list(&[6])),
        ("Layer", layer(8, 2)),
        ("MList", list(&[9])),
        ("VMpObj", page(0.0, 0.0, 100.0, 100.0)),
        ("MList", list(&[11])),
        ("PropLst", prop_list(1, 12)),
        ("Block", block(10)),
    ];
    pack(&records, 14, 13)
}

/// A symbol instance whose master group contains that same instance.
pub fn cyclic_symbol() -> Vec<u8> {
    // 3 class→group 6, 4 instance→class 3, 5 list of the instance, 6 the group.
    let records: Vec<(&str, Vec<u8>)> = vec![
        ("MName", mname("pages")),
        ("MName", mname("Ink")),
        ("SymbolClass", symbol_class(6)),
        ("SymbolInstance", symbol_instance(3)),
        ("MList", list(&[4])),
        ("Group", group(5, 0)),
        ("MList", list(&[6])),
        ("Layer", layer(7, 2)),
        ("MList", list(&[8])),
        ("VMpObj", page(0.0, 0.0, 100.0, 100.0)),
        ("MList", list(&[10])),
        ("PropLst", prop_list(1, 11)),
        ("Block", block(9)),
    ];
    pack(&records, 13, 12)
}

/// One empty group beside a path whose style names a content fill (a group) and a zero paint id.
pub fn empty_group_and_content_fill() -> Vec<u8> {
    let records: Vec<(&str, Vec<u8>)> = vec![
        ("MName", mname("pages")),                                    // 1
        ("MName", mname("Ink")),                                      // 2
        ("MName", mname("fill")),                                     // 3
        ("MName", mname("contents")),                                 // 4
        ("Color6", color6(0, 65535, 0)),                              // 5
        ("BasicFill", basic_fill(5)),                                 // 6
        ("MList", list(&[])),                                         // 7 empty
        ("Group", group(7, 0)),                                       // 8
        ("Group", group(7, 0)),                                       // 9 content drawing, referenced as a fill
        ("ElemPropLst", elem(&[(3, 6), (4, 9), (4, 0)])),             // 10 fill, content, and a zero id
        ("Path", path(10, 1, &[[1.0, 1.0], [4.0, 1.0], [4.0, 4.0]])), // 11
        ("MList", list(&[8, 11])),                                    // 12
        ("Layer", layer(12, 2)),                                      // 13
        ("MList", list(&[13])),                                       // 14
        ("VMpObj", page(0.0, 0.0, 80.0, 40.0)),                       // 15
        ("MList", list(&[15])),                                       // 16
        ("PropLst", prop_list(1, 16)),                                // 17
        ("Block", block(14)),                                         // 18
    ];
    pack(&records, 18, 17)
}

fn push(records: &mut Vec<(String, Vec<u8>)>, name: &str, body: Vec<u8>) -> u32 {
    records.push((name.into(), body));
    u32::try_from(records.len()).unwrap_or(0)
}

/// A one-page FreeHand 9 drawing: a translated closed path, a rectangle, an oval with a gradient.
pub fn demo() -> Vec<u8> {
    demo_at(9)
}

/// The same drawing as [`demo`], stored with the layout of FreeHand `version` (8, 9, 10 or 11).
pub fn demo_at(version: u8) -> Vec<u8> {
    let version = version.clamp(8, 11);
    // Record ids are the 1-based positions in this list.
    let records = vec![
        ("MName", mname("fill")),                                           // 1
        ("MName", mname("stroke")),                                         // 2
        ("MName", mname("pages")),                                          // 3
        ("MName", mname("Ink")),                                            // 4
        ("Color6", color6_at(65535, 0, 0, version)),                        // 5 red
        ("Color6", color6_at(0, 0, 65535, version)),                        // 6 blue
        ("BasicFill", basic_fill(5)),                                       // 7
        ("LinePat", line_pat(8.0, 4.0)),                                    // 8
        ("BasicLine", basic_line(6, 8, 2.0)),                               // 9
        ("ElemPropLst", elem_at(&[(1, 7), (2, 9)], version)),               // 10
        ("Path", path(10, 3, &[[10.0, 10.0], [30.0, 20.0]])),               // 11
        ("Xform", xform_at(5.0, 6.0, version)),                             // 12
        ("MList", list(&[11])),                                             // 13
        ("Group", group(13, 12)),                                           // 14
        ("Rectangle", rectangle_at(10, [40.0, 10.0, 60.0, 30.0], version)), // 15
        ("MultiColorList", stops(5, 6)),                                    // 16
        ("TaperedFillX", gradient(5, 6, 0.0, 16)),                          // 17
        ("ElemPropLst", elem_at(&[(1, 17)], version)),                      // 18
        ("Oval", oval_at(18, [70.0, 10.0, 90.0, 30.0], version)),           // 19
        ("MList", list(&[14, 15, 19])),                                     // 20
        ("Layer", layer(20, 4)),                                            // 21
        ("MList", list(&[21])),                                             // 22
        ("VMpObj", page(0.0, 0.0, 200.0, 100.0)),                           // 23
        ("MList", list(&[23])),                                             // 24
        ("PropLst", prop_list(3, 24)),                                      // 25
        ("Block", block_at(22, version)),                                   // 26
    ];
    pack_at(&records, 26, 25, version)
}

/// A shown layer and a hidden layer that still holds a path.
pub fn hidden_layer() -> Vec<u8> {
    let records: Vec<(&str, Vec<u8>)> = vec![
        ("MName", mname("pages")),                                     // 1
        ("MName", mname("Ink")),                                       // 2
        ("MName", mname("Guides")),                                    // 3
        ("Path", path(0, 1, &[[2.0, 2.0], [12.0, 2.0], [12.0, 8.0]])), // 4
        ("MList", list(&[4])),                                         // 5
        ("Layer", layer_vis(5, 3, 11)),                                // 6 hidden
        ("Path", path(0, 1, &[[0.0, 0.0], [6.0, 0.0], [6.0, 6.0]])),   // 7
        ("MList", list(&[7])),                                         // 8
        ("Layer", layer_vis(8, 2, 3)),                                 // 9 shown
        ("MList", list(&[6, 9])),                                      // 10
        ("VMpObj", page(0.0, 0.0, 40.0, 40.0)),                        // 11
        ("MList", list(&[11])),                                        // 12
        ("PropLst", prop_list(1, 12)),                                 // 13
        ("Block", block(10)),                                          // 14
    ];
    pack(&records, 14, 13)
}

/// A path whose `contents` paint is a group that holds another path.
pub fn content_fill() -> Vec<u8> {
    let records: Vec<(&str, Vec<u8>)> = vec![
        ("MName", mname("pages")),                                       // 1
        ("MName", mname("contents")),                                    // 2
        ("MName", mname("fill")),                                        // 3
        ("Color6", color6(0, 65535, 0)),                                 // 4
        ("BasicFill", basic_fill(4)),                                    // 5
        ("ElemPropLst", elem(&[(3, 5)])),                                // 6 style of the content path
        ("Path", path(6, 1, &[[2.0, 2.0], [8.0, 2.0], [8.0, 8.0]])),     // 7
        ("MList", list(&[7])),                                           // 8
        ("Group", group(8, 0)),                                          // 9 content drawing
        ("ElemPropLst", elem(&[(2, 9)])),                                // 10 contents → group
        ("Path", path(10, 1, &[[0.0, 0.0], [20.0, 0.0], [20.0, 16.0]])), // 11
        ("MList", list(&[11])),                                          // 12
        ("Layer", layer(12, 0)),                                         // 13
        ("MList", list(&[13])),                                          // 14
        ("VMpObj", page(0.0, 0.0, 40.0, 30.0)),                          // 15
        ("MList", list(&[15])),                                          // 16
        ("PropLst", prop_list(1, 16)),                                   // 17
        ("Block", block(14)),                                            // 18
    ];
    pack(&records, 18, 17)
}

/// An AGD container around `stream` (raw for version 8, zlib otherwise) followed by `after`.
pub fn envelope(version: u8, stream: &[u8], after: &[u8]) -> Vec<u8> {
    let version = version.clamp(8, 11);
    let payload = if version <= 8 {
        stream.to_vec()
    } else {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        if enc.write_all(stream).is_err() {
            return Vec::new();
        }
        match enc.finish() {
            Ok(bytes) => bytes,
            Err(_) => return Vec::new(),
        }
    };
    let data_len = u32::try_from(12 + payload.len()).unwrap_or(0);
    let digit = b'0'.saturating_add(version.saturating_sub(5));
    let mut file = Vec::new();
    file.extend_from_slice(&[b'A', b'G', b'D', digit]);
    file.extend(0u32.to_be_bytes());
    file.extend(data_len.to_be_bytes());
    file.extend_from_slice(&payload);
    file.extend_from_slice(after);
    file
}

/// A small file whose fields are chosen from `input`, then truncated. Counts stay inside the
/// bytes this builder allocates; the reader still has to survive the hostile values written
/// into the count, id and fixed-point slots.
pub fn fuzz_file(input: &[u8]) -> Vec<u8> {
    let b = |i: usize| input.get(i).copied().unwrap_or(0);
    let version = 8 + (b(0) % 4);
    let nest = u32::from(b(1) % 24).max(1);
    let mut file = if b(2) & 1 == 0 { demo_at(version) } else { nested_groups(nest) };
    if file.is_empty() {
        return file;
    }
    let n = file.len();
    // Record-count, id and node-count slots: poke the fuzzer's bytes into a few offsets.
    for i in 0..16 {
        let at = usize::from(b(3 + i)).wrapping_mul(17).wrapping_add(i * 13) % n;
        if let Some(slot) = file.get_mut(at) {
            *slot = b(20 + i);
        }
    }
    // 16.16 bit patterns, including the quiet-NaN and infinity payloads of an IEEE float,
    // written where a fixed-point value would sit.
    let patterns: [u32; 4] = [0x7fc0_0000, 0x7f80_0000, 0xff80_0000, 0x8000_0000];
    let which = patterns[usize::from(b(2) % 4)];
    let at = 12 + (usize::from(b(4)) % n.saturating_sub(16).max(1));
    if let Some(slot) = file.get_mut(at..at + 4) {
        slot.copy_from_slice(&which.to_be_bytes());
    }
    let keep = usize::from(b(5));
    let end = if keep == 0 { n } else { (keep.saturating_mul(n)) / 255 };
    file.truncate(end.max(1).min(n));
    file
}
