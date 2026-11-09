//! Reading Pixar picture files: every channel set, storage mode, packet kind and alpha mode
//! decodes to the pixels it stores, tiled pictures assemble with their border and null tiles, the
//! files open, place and paste, and damaged files are refused with a message.

use serde_json::json;

use super::pxrread::{decode, is_pxr};
use super::*;

/// The header fields a test sets: the picture's (width, height), the tile's (width, height), the
/// channel bits (R 8, G 4, B 2, A 1), the storage mode, the blocking factor and the alpha mode.
#[derive(Clone, Copy)]
struct Head {
    size: (u16, u16),
    tile: (u16, u16),
    format: u16,
    storage: u16,
    block: u16,
    alpha: u16,
}

/// A one-tile picture of `w` × `h` pixels.
fn head(w: u16, h: u16, format: u16, storage: u16) -> Head {
    Head { size: (w, h), tile: (w, h), format, storage, block: 0, alpha: 1 }
}

/// A Pixar picture file of `version` with header `h`, whose tiles' data `tiles` (`None` for a
/// null tile) follow one another from byte `start` (at least the end of the tile pointer table).
fn pxr(version: u16, h: Head, start: usize, tiles: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut f = vec![0x80, 0xe8, 0, 0];
    f.extend(version.to_le_bytes());
    f.extend(b"a test picture");
    f.resize(416, 0);
    for v in [h.size.1, h.size.0, h.tile.1, h.tile.0, h.format, h.storage, h.block, h.alpha, 7, 9] {
        f.extend(v.to_le_bytes());
    }
    f.resize(512, 0);
    let mut at = start;
    let mut data: Vec<u8> = vec![];
    for tile in tiles {
        match tile {
            Some(bytes) => {
                f.extend((at as u32).to_le_bytes());
                f.extend((bytes.len() as i32).to_le_bytes());
                data.extend(bytes);
                at += bytes.len();
            }
            None => f.extend([0; 8]),
        }
    }
    assert!(f.len() <= start, "the tile data starts inside the table");
    f.resize(start, 0);
    f.extend(data);
    f
}

/// A one-tile picture whose tile data follows the table.
fn one(h: Head, data: &[u8]) -> Vec<u8> {
    pxr(0, h, 520, &[Some(data.to_vec())])
}

/// A packet header: `kind` in the top four bits, `count` less one in the low twelve.
fn packet(kind: u16, count: u16) -> [u8; 2] {
    ((kind << 12) | (count - 1)).to_le_bytes()
}

/// 12-bit samples, two bytes each.
fn wide(samples: &[u16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// The pixels `raster_image` stores for `f`, row by row.
fn pixels(f: &[u8]) -> Vec<[u8; 4]> {
    let img = raster_image(f).unwrap();
    image::load_from_memory(&img.blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
}

const RGBA: u16 = 15;
const RGB: u16 = 14;
const R: u16 = 8;

#[test]
fn every_channel_set_and_dumped_storage_reads() {
    // 8 bits dumped, RGBA unassociated: the samples as they are.
    let f = one(head(2, 1, RGBA, 2), &[10, 20, 30, 40, 50, 60, 70, 255]);
    assert_eq!(pixels(&f), [[10, 20, 30, 40], [50, 60, 70, 255]]);
    // RGB is opaque; R alone is gray; G and B alone leave the other colors at 0.
    assert_eq!(pixels(&one(head(2, 1, RGB, 2), &[1, 2, 3, 4, 5, 6])), [[1, 2, 3, 255], [4, 5, 6, 255]]);
    assert_eq!(pixels(&one(head(3, 1, R, 2), &[0, 128, 255])), [[0, 0, 0, 255], [128, 128, 128, 255], [255; 4]]);
    assert_eq!(pixels(&one(head(1, 1, 0b0110, 2), &[9, 8])), [[0, 9, 8, 255]]);
    // A alone: black at the stored coverage.
    assert_eq!(pixels(&one(head(1, 1, 1, 2), &[77])), [[0, 0, 0, 77]]);

    // 12 bits dumped: 0 is black, 2048 white, 1024 half (127.5, rounded up); values past 1.0
    // (2049 to 3071) are white and values below 0.0 (3072 to 4095) black; the top four bits of
    // each pair aren't part of the sample.
    let f = one(head(2, 1, RGB, 3), &wide(&[0, 1024, 2048, 3000, 3072, 0xf800]));
    assert_eq!(pixels(&f), [[0, 128, 255, 255], [255, 0, 255, 255]]);
    let f = one(head(1, 1, R, 3), &wide(&[512]));
    assert_eq!(pixels(&f), [[64, 64, 64, 255]], "512 of 2048 is 63.75 of 255");
}

#[test]
fn alpha_matted_to_black_becomes_straight_color() {
    // Alpha mode 0: color is multiplied by A. 50 and 100 at A 128 are 99.6 and 199.2 straight;
    // color past its A is white; A 0 is transparent black.
    let mut h = head(3, 1, RGBA, 2);
    h.alpha = 0;
    let f = one(h, &[50, 100, 0, 128, 200, 0, 0, 100, 30, 30, 30, 0]);
    assert_eq!(pixels(&f), [[100, 199, 0, 128], [255, 0, 0, 100], [0, 0, 0, 0]]);
    // Alpha mode 1 keeps the color as it is.
    h.alpha = 1;
    let f = one(h, &[50, 100, 0, 128, 200, 0, 0, 100, 30, 30, 30, 0]);
    assert_eq!(pixels(&f), [[50, 100, 0, 128], [200, 0, 0, 100], [30, 30, 30, 0]]);
    // Without A, the alpha mode changes nothing.
    let mut h = head(1, 1, RGB, 2);
    h.alpha = 0;
    assert_eq!(pixels(&one(h, &[50, 100, 0])), [[50, 100, 0, 255]]);
}

#[test]
fn every_packet_kind_decodes() {
    // Kind 1 dumps two full pixels; kind 2 holds one run of two pixels (length byte 1).
    let mut data = packet(1, 2).to_vec();
    data.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    data.extend(packet(2, 1));
    data.extend([1, 9, 10, 11, 12]);
    assert_eq!(pixels(&one(head(4, 1, RGBA, 0), &data)), [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [9, 10, 11, 12]]);

    // Kind 3 dumps one pixel's RGB after A 200; kind 4 holds runs of one and two pixels after A 100.
    let mut data = packet(3, 1).to_vec();
    data.extend([200, 1, 2, 3]);
    data.extend(packet(4, 2));
    data.extend([100, 0, 4, 5, 6, 1, 7, 8, 9]);
    assert_eq!(pixels(&one(head(4, 1, RGBA, 0), &data)), [[1, 2, 3, 200], [4, 5, 6, 100], [7, 8, 9, 100], [7, 8, 9, 100]]);

    // Kind 2 in a gray picture: three runs over five pixels.
    let mut data = packet(2, 3).to_vec();
    data.extend([0, 10, 2, 20, 0, 30]);
    assert_eq!(
        pixels(&one(head(5, 1, R, 0), &data)),
        [[10, 10, 10, 255], [20, 20, 20, 255], [20, 20, 20, 255], [20, 20, 20, 255], [30, 30, 30, 255]]
    );

    // 12 bits encoded: samples take two bytes; run lengths and the packet header don't change.
    let mut data = packet(1, 1).to_vec();
    data.extend(wide(&[2048, 1024, 0]));
    data.extend(packet(2, 1));
    data.push(0);
    data.extend(wide(&[0, 0, 2048]));
    assert_eq!(pixels(&one(head(2, 1, RGB, 1), &data)), [[255, 128, 0, 255], [0, 0, 255, 255]]);
    let mut data = packet(3, 1).to_vec();
    data.extend(wide(&[1024, 2048, 0, 0]));
    assert_eq!(pixels(&one(head(1, 1, RGBA, 1), &data)), [[255, 0, 0, 128]]);

    // Several rows, one packet each, and a block end: with a blocking factor of 16, kind 0 sends
    // reading to byte 32 of the file, where the second row starts. Packets of both rows mix kinds.
    let mut h = head(2, 2, RGB, 0);
    h.block = 16;
    let mut data = packet(1, 2).to_vec();
    data.extend([1, 1, 1, 2, 2, 2]);
    data.extend(packet(0, 1));
    data.resize(16, 0);
    data.extend(packet(2, 1));
    data.extend([1, 3, 3, 3]);
    let f = pxr(0, h, 528, &[Some(data)]);
    assert_eq!(528 % 16, 0);
    assert_eq!(pixels(&f), [[1, 1, 1, 255], [2, 2, 2, 255], [3, 3, 3, 255], [3, 3, 3, 255]]);
}

#[test]
fn tiles_assemble_with_border_and_null_tiles() {
    // A 3 × 3 picture in 2 × 2 tiles: two across and two down, numbered across. The border tiles'
    // pixels past the picture are stored and dropped. Tile 2 (bottom left) is null: black, since
    // the picture has no A.
    let mut h = head(3, 3, R, 2);
    h.tile = (2, 2);
    let tiles = [Some(vec![1, 2, 4, 5]), Some(vec![3, 99, 6, 99]), None, Some(vec![9, 99, 99, 99])];
    let f = pxr(0, h, 600, &tiles);
    let gray = |v: u8| [v, v, v, 255];
    assert_eq!(pixels(&f), [gray(1), gray(2), gray(3), gray(4), gray(5), gray(6), gray(0), gray(0), gray(9)]);

    // With A, a null tile is transparent.
    let mut h = head(2, 1, RGBA, 2);
    h.tile = (1, 1);
    let f = pxr(0, h, 600, &[None, Some(vec![1, 2, 3, 4])]);
    assert_eq!(pixels(&f), [[0, 0, 0, 0], [1, 2, 3, 4]]);

    // A picture smaller than its tile is one border tile: a 1 × 1 picture in a 2 × 2 tile keeps its
    // one pixel, dumped and encoded.
    let mut h = head(1, 1, RGB, 2);
    h.tile = (2, 2);
    assert_eq!(pixels(&one(h, &[1, 2, 3, 99, 99, 99, 99, 99, 99, 99, 99, 99])), [[1, 2, 3, 255]]);
    h.storage = 0;
    let mut data = packet(2, 1).to_vec();
    data.extend([1, 4, 5, 6]);
    data.extend(packet(2, 1));
    data.extend([1, 99, 99, 99]);
    assert_eq!(pixels(&one(h, &data)), [[4, 5, 6, 255]]);

    // Tile data may lie anywhere after the table, in any order, and a length of 0 is read like any
    // other complete tile.
    let mut f = pxr(0, head(1, 1, RGB, 2), 1000, &[Some(vec![7, 8, 9])]);
    f[516..520].copy_from_slice(&0i32.to_le_bytes());
    assert_eq!(pixels(&f), [[7, 8, 9, 255]]);
}

#[test]
fn version_2_headers_read_like_version_0() {
    let data = [10, 20, 30, 40];
    let (v0, v2) = (pxr(0, head(1, 1, RGBA, 2), 520, &[Some(data.to_vec())]), pxr(2, head(1, 1, RGBA, 2), 520, &[Some(data.to_vec())]));
    assert_eq!(v2[4..6], [2, 0]);
    assert_eq!(pixels(&v2), pixels(&v0));
}

#[test]
fn pxr_files_open_place_and_paste() {
    use crate::cmd::clipboard::{PASTE_ORDER, file_flavour};
    let b64 = vectorcraft_format::base64_encode;
    let f = one(head(2, 1, RGB, 2), &[10, 20, 30, 40, 50, 60]);
    // The header has no resolution: the image takes the default for images without one.
    assert_eq!(ppi::resolution(&f), None);
    assert_eq!(detect("picture.dat", &f).map(|f| f.id), Some("pxr"), "by its magic number");

    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "art.pxr", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("pxr"), &json!([])), "{r}");
    let size = s.doc().unwrap().doc.artboards[0].rect.size();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.place", &json!({"name": "ART.PXR", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("pxr"), Some(1)), "{r}");
    // Placed from a path, linked and embedded: both show the file's pixels at its pixel size.
    let dir = std::env::temp_dir().join(format!("vectorcraft-pxr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.pxr");
    std::fs::write(&path, &f).unwrap();
    for link in [true, false] {
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": link})).unwrap();
        assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("pxr"), Some(1)), "link {link}: {r}");
        let doc = &s.doc().unwrap().doc;
        let node = doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
        let vectorcraft_doc::NodeKind::Image(im) = &node.kind else { panic!("link {link}: an image, got {:?}", node.kind) };
        assert_eq!((im.link.is_some(), im.width, im.height), (link, 2, 1), "link {link}");
        let shown = image::load_from_memory(&doc.images[&im.key].bytes).unwrap().to_rgba8();
        assert_eq!(shown.pixels().map(|p| p.0).collect::<Vec<_>>(), pixels(&f), "link {link}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    // Open sizes it as a PNG of 2 × 1 pixels without a resolution.
    let mut png = vec![];
    image::RgbaImage::new(2, 1).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut t = Session::new();
    t.execute("document.open", &json!({"name": "same.png", "dataBase64": b64(&png)})).unwrap();
    assert_eq!(t.doc().unwrap().doc.artboards[0].rect.size(), size);

    // Copied in a file manager, a Pixar file pastes as a bitmap.
    assert_eq!(file_flavour("art.pxr", &f, &PASTE_ORDER), Some("image/x-pixar"));
    let r = s.execute("clipboard.importImage", &json!({"mime": "image/x-pixar", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["count"], &r["pixelWidth"], &r["pixelHeight"]), (&json!(1), &json!(2), &json!(1)), "{r}");
    // Glass's Load Texture… reads it too.
    assert!(texture_image(&f).is_ok());

    // The dialogs list Pixar; the desktop packaging doesn't associate it; nothing writes it.
    assert!(OPEN_EXTS.contains(&"pxr") && PLACE_EXTS.contains(&"pxr") && UNASSOCIATED_EXTS.contains(&"pxr"));
    let filter = ("Pixar", &["pxr"][..]);
    assert!(open_filters().any(|f| f == filter) && place_filters().any(|f| f == filter));
    let r = s.execute("document.formats", &json!({})).unwrap();
    let pxr_format = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "pxr").unwrap().clone();
    assert_eq!((pxr_format["read"].as_bool(), pxr_format["write"].as_bool()), (Some(true), Some(false)));
    assert_eq!(format_for_name("a.pxr").map(|f| f.id), Some("pxr"));
}

#[test]
fn damaged_files_are_refused_with_a_message() {
    let message = |f: &[u8]| raster_image(f).err().expect("refused").to_string();
    let rgb = one(head(2, 1, RGB, 2), &[1, 2, 3, 4, 5, 6]);
    let (short, header, data) = ("the Pixar file is cut short", "the Pixar file's header is damaged", "the Pixar file's image data is damaged");

    // A header, tile table or dumped tile cut short, and a tile whose writing wasn't finished.
    assert_eq!(message(&rgb[..300]), short);
    assert_eq!(message(&rgb[..516]), short);
    assert_eq!(message(&rgb[..rgb.len() - 1]), short);
    let mut unfinished = rgb.clone();
    unfinished[516..520].copy_from_slice(&(-1i32).to_le_bytes());
    assert_eq!(message(&unfinished), short);
    let mut far = rgb.clone();
    far[512..516].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(message(&far), short, "a pointer past the end of the file");
    // A refused open leaves the active document as it was.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let before = s.doc().unwrap().doc.artboards[0].rect.size();
    let e = s.execute("document.open", &json!({"name": "a.pxr", "dataBase64": vectorcraft_format::base64_encode(&rgb[..516])}));
    assert!(e.expect_err("refused").to_string().contains(short));
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), before);

    // Encoded data cut short: in a packet header, a sample and a run length.
    let mut cut = packet(1, 2).to_vec();
    cut.extend([1, 2, 3, 4]);
    assert_eq!(message(&one(head(2, 1, RGB, 0), &cut)), short);
    assert_eq!(message(&one(head(2, 1, RGB, 0), &[0x01])), short);
    assert_eq!(message(&one(head(2, 1, RGB, 0), &packet(2, 1))), short);
    // A block end that moves past the end of the file.
    let mut h = head(1, 1, RGB, 0);
    h.block = 4096;
    assert_eq!(message(&one(h, &packet(0, 1))), short);

    // Header values the format doesn't define.
    let with = |at: usize, v: u16| {
        let mut f = rgb.clone();
        f[at..at + 2].copy_from_slice(&v.to_le_bytes());
        message(&f)
    };
    for (at, v, what) in [
        (416, 0, "no height"),
        (418, 0, "no width"),
        (420, 0, "no tile height"),
        (422, 0, "no tile width"),
        (424, 0, "no channels"),
        (424, 16, "a channel bit past A"),
        (426, 4, "storage mode 4"),
        (430, 2, "alpha mode 2"),
    ] {
        assert_eq!(with(at, v), header, "{what}");
    }

    // Packets the format doesn't define, packets past the end of the scan line, packets with a
    // shared A in a picture without A, and a block end without a blocking factor.
    let encoded = |format: u16, data: &[u8]| message(&one(head(2, 1, format, 0), data));
    assert_eq!(encoded(RGB, &[&packet(5, 1)[..], &[0; 8]].concat()), data, "kind 5");
    assert_eq!(encoded(RGB, &[&packet(1, 3)[..], &[0; 9]].concat()), data, "three pixels in a row of two");
    assert_eq!(encoded(RGB, &[&packet(2, 1)[..], &[2, 0, 0, 0]].concat()), data, "a run of three in a row of two");
    assert_eq!(encoded(RGB, &[&packet(3, 1)[..], &[0; 4]].concat()), data, "a shared A without A");
    assert_eq!(encoded(1, &[&packet(4, 1)[..], &[0; 4]].concat()), data, "a shared A without color");
    assert_eq!(encoded(RGB, &packet(0, 1)), data, "a block end without a blocking factor");
    // A block end right after another one: an empty block.
    let mut h = head(2, 1, RGB, 0);
    h.block = 2;
    assert_eq!(message(&one(h, &[&packet(0, 1)[..], &packet(0, 1), &packet(1, 2), &[0; 6]].concat())), data, "two block ends in a row");
    // Tiles that share their encoded data read more than the file holds: a scan line of 2000
    // one-pixel packets, shown one pixel wide, is read once per tile.
    let row: Vec<u8> = (0..2000).flat_map(|_| [&packet(1, 1)[..], &[5, 6, 7]].concat()).collect();
    let mut h = head(1, 1, RGB, 0);
    h.tile = (2000, 1);
    assert_eq!(pixels(&pxr(0, h, 520, &[Some(row.clone())])), [[5, 6, 7, 255]], "the one tile alone");
    h.size = (1, 3);
    let mut shared = pxr(0, h, 600, &[Some(row.clone()), None, None]);
    let first = shared[512..520].to_vec();
    shared[520..528].copy_from_slice(&first);
    shared[528..536].copy_from_slice(&first);
    assert_eq!(message(&shared), data, "three tiles on the same data");

    // Past the allowance, from the header, before any tile is read.
    let huge = pxr(0, head(65535, 65535, RGBA, 2), 520, &[None]);
    assert_eq!(message(&huge), "an image of 65535 × 65535 pixels is too large to draw");
    assert!(decode(&rgb, 7).is_err(), "2 × 1 RGBA is 8 bytes");
    assert!(decode(&rgb, 8).is_ok());

    // Only the magic number makes a Pixar file.
    assert!(is_pxr(&rgb));
    for bad in [[0x80, 0xe8, 0, 1], [0xe8, 0x80, 0, 0], [0, 0, 0xe8, 0x80]] {
        assert!(!is_pxr(&bad), "{bad:?}");
    }
}

#[test]
fn tile_pointers_into_the_header_or_table_are_refused() {
    // Tile data follows the tile pointer table, which ends at byte 520 in a one-tile picture.
    let rgb = one(head(2, 1, RGB, 2), &[1, 2, 3, 4, 5, 6]);
    for pointer in [4u32, 511, 512, 519] {
        let mut inside = rgb.clone();
        inside[512..516].copy_from_slice(&pointer.to_le_bytes());
        let e = raster_image(&inside).err().expect("refused").to_string();
        assert_eq!(e, "the Pixar file's image data is damaged", "a pointer to byte {pointer}");
    }
    // A pointer at the table's end reads.
    assert_eq!(pixels(&rgb), [[1, 2, 3, 255], [4, 5, 6, 255]]);
}
