//! Original, tiny RIFF fixtures generated from the public record grammar.
//! Available only for tests or when the `synth` feature is enabled.

/// One 100 × 80 point CDR 12 page with a 40 × 20 point rectangle,
/// solid red RGB fill and a one-point black outline. Geometry starts at the
/// page centre (50, 40 points), extending right and down.
pub fn rectangle() -> Vec<u8> {
    rectangle_with_page_size(100.0, 80.0)
}

/// The same editable rectangle, but with zero-by-zero document page metadata.
/// This is an original reproducer for a supplied drawing-only CDR, not its bytes.
pub fn zero_sized_page() -> Vec<u8> {
    rectangle_with_page_size(0.0, 0.0)
}

/// Generate one rectangle and independently selectable document page dimensions.
fn rectangle_with_page_size(width: f64, height: f64) -> Vec<u8> {
    /// Encode little-endian words for generated record fields.
    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    /// Encode a point measurement as a signed CDR coordinate stored in a word.
    fn coord(points: f64) -> u32 {
        (points / 72.0 * 254000.0).round() as i32 as u32
    }
    /// Encode a RIFF chunk and pad odd payload lengths.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if !body.len().is_multiple_of(2) {
            v.push(0);
        }
        v
    }
    /// Wrap generated child chunks in a typed RIFF LIST.
    fn list(kind: &[u8; 4], children: &[u8]) -> Vec<u8> {
        let mut b = kind.to_vec();
        b.extend_from_slice(children);
        chunk(b"LIST", &b)
    }
    let rgb = |r: u8, g: u8, b: u8| {
        let mut v = vec![5, 0, 0, 0, 0, 0, 0, 0];
        v.extend_from_slice(&[b, g, r, 0]);
        v
    };
    let mut fill = vec![1, 0, 0, 0, 1, 0, 0, 0];
    fill.extend(rgb(255, 0, 0));
    let mut outline = vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    outline.extend(u32s(&[coord(1.0)]));
    outline.extend_from_slice(&[100, 0, 0, 0]);
    outline.extend_from_slice(&[0; 56]);
    outline.extend(rgb(0, 0, 0));
    outline.extend_from_slice(&[0; 48]);
    let shape = u32s(&[coord(40.0), coord(-20.0), 0, 0, 0, 0]);
    let mut loda = u32s(&[84, 3, 60, 72, 1]);
    loda.extend_from_slice(&1u32.to_le_bytes());
    loda.extend_from_slice(&2u32.to_le_bytes());
    loda.extend_from_slice(&shape);
    loda.extend_from_slice(&[0; 8]);
    loda.extend(u32s(&[20, 24, 28, 30, 10, 20]));
    let object = list(b"obj ", &chunk(b"loda", &loda));
    let mut page = chunk(b"flgs", &[0, 0, 0, 0x90]);
    page.extend(object);
    let mut body = chunk(b"vrsn", &1200u16.to_le_bytes());
    let mut config = vec![0; 4];
    config.extend(u32s(&[coord(width), coord(height)]));
    body.extend(chunk(b"mcfg", &config));
    body.extend(chunk(b"fild", &fill));
    body.extend(chunk(b"outl", &outline));
    body.extend(list(b"page", &page));
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"CDRC");
    out.extend(body);
    out
}

/// Original 2 × 2 RGB bitmap on a 100 × 80 point CDR 12 page, with a triangular crop and hole.
/// Its stored bottom row is blue/white, its top row red/green; a separate mask gives every
/// pixel alpha 128. The image has no preview.
pub fn clipped_bitmap() -> Vec<u8> {
    /// Encode fixture words without borrowing any producer-authored file bytes.
    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    /// Convert document points to the public CDR integer coordinate units.
    fn coord(points: i32) -> u32 {
        (points as f64 * 254000.0 / 72.0).round() as i32 as u32
    }
    /// Encode one padded RIFF fixture chunk.
    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut result = tag.to_vec();
        result.extend(words(&[body.len() as u32]));
        result.extend(body);
        if !body.len().is_multiple_of(2) {
            result.push(0);
        }
        result
    }
    /// Wrap fixture chunks in a typed RIFF list.
    fn list(kind: &[u8; 4], children: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend(children);
        chunk(b"LIST", &body)
    }
    let mut image = words(&[7, 1, 0x4955, 212, 0, 32, 126, 0, 2, 255]);
    image.extend(b"RI");
    image.extend(words(&[94, 0, 78, 1, 1, 2, 2, 1, 24, 8, 16, 0, 0, 0]));
    image.extend([0; 20]);
    image.extend([255, 0, 0, 255, 255, 255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0]);
    image.extend(b"RI");
    image.extend(words(&[86, 0, 78, 99, 1, 2, 2, 1, 8, 4, 8, 0, 0, 0]));
    image.extend([0; 20]);
    image.extend([128, 128, 0, 0, 128, 128, 0, 0]);
    let corners = [0, 0, coord(40), coord(-20)];
    let mut geometry = words(&corners);
    geometry.extend(words(&corners));
    geometry.extend(words(&[0x0018_0002, 2, 2, 0, 7]));
    geometry.extend([0; 20]);
    geometry.extend(words(&[6, 0, 0, coord(40), 0, 0, coord(-20), coord(4), coord(-2), coord(12), coord(-2), coord(4), coord(-6)]));
    geometry.extend([0, 64, 72, 0, 64, 72]);
    let table = 20 + geometry.len() as u32;
    let mut object = words(&[table + 8, 1, table, table + 4, 5]);
    object.extend(geometry);
    object.extend(words(&[20, 30]));
    let mut page = chunk(b"flgs", &[0, 0, 0, 0x90]);
    page.extend(list(b"obj ", &chunk(b"loda", &object)));
    let mut body = chunk(b"vrsn", &1200u16.to_le_bytes());
    body.extend(chunk(b"mcfg", &words(&[0, coord(100), coord(80)])));
    body.extend(chunk(b"bmp ", &image));
    body.extend(list(b"page", &page));
    let mut result = b"RIFF".to_vec();
    result.extend(words(&[body.len() as u32 + 4]));
    result.extend(b"CDRC");
    result.extend(body);
    result
}

/// Original vector-only PowerClip: a blue rectangle exceeds a red compound frame
/// with a same-winding hole. Its contents live in a referenced resource group.
/// CDR 12 container/style fields keep this small; the reference relationships are
/// independently checked with local Corel-authored format 15/16/27 drawings.
pub fn powerclip() -> Vec<u8> {
    /// Encode fixture words, including signed coordinates represented as words.
    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    /// Convert points to native integer units without producer-authored bytes.
    fn coord(points: i32) -> u32 {
        (points as f64 * 254000.0 / 72.0).round() as i32 as u32
    }
    /// Encode one padded original RIFF record.
    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = tag.to_vec();
        bytes.extend(words(&[body.len() as u32]));
        bytes.extend(body);
        if !body.len().is_multiple_of(2) {
            bytes.push(0);
        }
        bytes
    }
    /// Encode a typed original container.
    fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_vec();
        bytes.extend(body);
        chunk(b"LIST", &bytes)
    }
    /// Put geometry and reference fields ahead of their offset/reversed-type tables.
    fn loda(kind: u32, args: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let count = args.len() as u32;
        let table = 20 + args.iter().map(|(_, data)| data.len() as u32).sum::<u32>();
        let mut bytes = words(&[table + count * 8, count, table, table + count * 4, kind]);
        let mut offsets = Vec::new();
        for (_, data) in args {
            offsets.push(bytes.len() as u32);
            bytes.extend(data);
        }
        bytes.extend(words(&offsets));
        bytes.extend(words(&args.iter().rev().map(|(id, _)| *id).collect::<Vec<_>>()));
        chunk(b"loda", &bytes)
    }
    /// Encode a solid RGB style under an original numeric identifier.
    fn fill(id: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut data = words(&[id, 1, 5, 0]);
        data.extend([rgb[2], rgb[1], rgb[0], 0]);
        chunk(b"fild", &data)
    }
    let mut geometry = words(&[8]);
    for (x, y) in [(0, 0), (40, 0), (40, -30), (0, -30), (10, -10), (30, -10), (30, -20), (10, -20)] {
        geometry.extend(words(&[coord(x), coord(y)]));
    }
    geometry.extend([0, 64, 64, 72, 0, 64, 64, 72]);
    let mut frame = chunk(b"spnd", &words(&[10]));
    frame.extend(loda(3, &[(30, geometry), (20, words(&[1])), (8005, words(&[8, 1, 0, 0]))]));
    let content = loda(1, &[(30, words(&[coord(60), coord(-45), 0, 0, 0, 0])), (20, words(&[2]))]);
    let mut resource = chunk(b"spnd", &words(&[8]));
    resource.extend(loda(0, &[(3500, words(&[10]))]));
    resource.extend(list(b"obj ", &content));
    let resources = list(b"doc ", &list(b"clpt", &list(b"grp ", &resource)));
    let mut page = chunk(b"flgs", &[0, 0, 0, 0x90]);
    page.extend(list(b"obj ", &frame));
    let mut body = chunk(b"vrsn", &1200u16.to_le_bytes());
    body.extend(chunk(b"mcfg", &words(&[0, coord(160), coord(120)])));
    body.extend(fill(1, [220, 40, 70]));
    body.extend(fill(2, [20, 100, 230]));
    body.extend(resources);
    body.extend(list(b"page", &page));
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(words(&[body.len() as u32 + 4]));
    bytes.extend(b"CDRC");
    bytes.extend(body);
    bytes
}
