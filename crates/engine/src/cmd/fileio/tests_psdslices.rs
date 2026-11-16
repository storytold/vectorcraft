//! A PSD or PSB file's slices (image resource 1050) opened and placed as user slices
//! (`document.open`'s and `file.place`'s `importSlices`): version 6 records and version 7 and 8
//! descriptors, built from the Photoshop File Formats Specification's "Slices resource format";
//! their options, slices outside the canvas, damaged or unknown slice data, placing scaled and
//! moved in one undo step, linked files, and `document.psdInfo`'s count.

use serde_json::{Value, json};
use vectorcraft_doc::{SliceKind, SliceOptions};
use vectorcraft_geom::Rect;

use crate::Session;

/// A Unicode string: a count of UTF-16 units, then the units.
fn ustr(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut b = (units.len() as u32).to_be_bytes().to_vec();
    b.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    b
}

/// One version 6 slice record.
#[derive(Clone)]
struct S {
    /// 0: generated automatically, 1: follows a layer, 2: made by the user.
    origin: u32,
    name: &'static str,
    /// 0: No Image, 1: Image.
    kind: u32,
    /// Left, top, right, bottom (px).
    rect: [i32; 4],
    url: &'static str,
    target: &'static str,
    message: &'static str,
    alt: &'static str,
    html: bool,
    cell: &'static str,
    /// Alpha, red, green, blue.
    argb: [u8; 4],
}

fn user(name: &'static str, rect: [i32; 4]) -> S {
    S { origin: 2, name, kind: 1, rect, url: "", target: "", message: "", alt: "", html: false, cell: "", argb: [0; 4] }
}

/// Resource 1050 in version 6, each record followed by `trailing` (a Photoshop 7 descriptor, or
/// nothing).
fn v6(slices: &[S], trailing: &[u8]) -> Vec<u8> {
    let mut b = 6u32.to_be_bytes().to_vec();
    b.extend([0u8; 16]);
    b.extend(ustr("group"));
    b.extend((slices.len() as u32).to_be_bytes());
    for (i, s) in slices.iter().enumerate() {
        b.extend((i as u32 + 1).to_be_bytes());
        b.extend(0u32.to_be_bytes());
        b.extend(s.origin.to_be_bytes());
        if s.origin == 1 {
            b.extend(7u32.to_be_bytes());
        }
        b.extend(ustr(s.name));
        b.extend(s.kind.to_be_bytes());
        for v in s.rect {
            b.extend(v.to_be_bytes());
        }
        for t in [s.url, s.target, s.message, s.alt] {
            b.extend(ustr(t));
        }
        b.push(u8::from(s.html));
        b.extend(ustr(s.cell));
        b.extend([0u8; 8]);
        b.extend(s.argb);
        b.extend(trailing);
    }
    b
}

/// A descriptor value.
enum D {
    Long(i32),
    Doub(f64),
    Bool(bool),
    Text(&'static str),
    Enum(&'static str, &'static str),
    Obj(Vec<(&'static str, D)>),
    List(Vec<D>),
}

/// A key or class ID: a four-character one with length 0, else its length and characters.
fn key(k: &str) -> Vec<u8> {
    let mut b = if k.len() == 4 { 0u32 } else { k.len() as u32 }.to_be_bytes().to_vec();
    b.extend(k.as_bytes());
    b
}

/// A descriptor: an empty class name, the class ID, the count, then each item.
fn descriptor(items: &[(&str, D)]) -> Vec<u8> {
    let mut b = ustr("");
    b.extend(key("null"));
    b.extend((items.len() as u32).to_be_bytes());
    for (k, v) in items {
        b.extend(key(k));
        b.extend(value(v));
    }
    b
}

/// A value's type, then the value.
fn value(v: &D) -> Vec<u8> {
    let mut b = vec![];
    match v {
        D::Long(n) => {
            b.extend(b"long");
            b.extend(n.to_be_bytes());
        }
        D::Doub(d) => {
            b.extend(b"doub");
            b.extend(d.to_be_bytes());
        }
        D::Bool(on) => {
            b.extend(b"bool");
            b.push(u8::from(*on));
        }
        D::Text(t) => {
            b.extend(b"TEXT");
            b.extend(ustr(t));
        }
        D::Enum(t, e) => {
            b.extend(b"enum");
            b.extend(key(t));
            b.extend(key(e));
        }
        D::Obj(items) => {
            b.extend(b"Objc");
            b.extend(descriptor(items));
        }
        D::List(items) => {
            b.extend(b"VlLs");
            b.extend((items.len() as u32).to_be_bytes());
            for i in items {
                b.extend(value(i));
            }
        }
    }
    b
}

/// A bounds object: top, left, bottom and right.
fn bounds(l: i32, t: i32, r: i32, b: i32) -> D {
    D::Obj(vec![("Top ", D::Long(t)), ("Left", D::Long(l)), ("Btom", D::Long(b)), ("Rght", D::Long(r))])
}

/// Resource 1050 in `version` 7 or 8 holding `slices`.
fn v7(version: u32, slices: Vec<D>) -> Vec<u8> {
    let mut b = version.to_be_bytes().to_vec();
    b.extend(16u32.to_be_bytes());
    b.extend(descriptor(&[("bounds", bounds(0, 0, 20, 20)), ("baseName", D::Text("poster")), ("slices", D::List(slices))]));
    b
}

/// A 20 × 20 px PSD file written by `document.export`, with its layers (two) or flat.
fn psd(layers: bool) -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({ "color": "#ff0000" })).unwrap();
    s.execute("shape.rectangle", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    s.execute("layer.new", &json!({ "name": "Top" })).unwrap();
    s.execute("shape.rectangle", &json!({"x": 8, "y": 8, "width": 10, "height": 10})).unwrap();
    let r = s.execute("document.export", &json!({"format": "psd", "layers": layers})).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
}

/// `psd` with image resource `id` holding `data` added to its image resources.
fn with_resource(psd: &[u8], id: u16, data: &[u8]) -> Vec<u8> {
    let u32_at = |at: usize| u32::from_be_bytes(psd[at..at + 4].try_into().unwrap()) as usize;
    let colors = 26 + 4 + u32_at(26);
    let len = u32_at(colors);
    let mut block = b"8BIM".to_vec();
    block.extend(id.to_be_bytes());
    block.extend([0, 0]);
    block.extend((data.len() as u32).to_be_bytes());
    block.extend(data);
    block.resize(block.len().next_multiple_of(2), 0);
    let mut out = psd[..colors].to_vec();
    out.extend(((len + block.len()) as u32).to_be_bytes());
    out.extend(&psd[colors + 4..colors + 4 + len]);
    out.extend(block);
    out.extend(&psd[colors + 4 + len..]);
    out
}

fn b64(f: &[u8]) -> String {
    vectorcraft_format::base64_encode(f)
}

fn open(f: &[u8], extra: Value) -> (Session, Value) {
    let mut s = Session::new();
    let mut p = json!({"name": "poster.psd", "dataBase64": b64(f)});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute("document.open", &p).unwrap();
    (s, r)
}

/// The document's user slices: rectangle and options.
fn slices(s: &Session) -> Vec<(Rect, SliceOptions)> {
    s.doc().unwrap().doc.slices.iter().map(|x| (x.rect, x.options.clone())).collect()
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"].as_array().map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn version_6_slices_open_as_user_slices_with_their_options() {
    let linked = S { origin: 1, url: "https://example.com/", target: "_blank", message: "Go", alt: "Logo", ..user("logo", [2, 4, 10, 8]) };
    let plain = S { kind: 0, cell: "a < b & c", argb: [0, 0, 0, 0], ..user("note", [10, 10, 20, 20]) };
    let html = S { kind: 0, html: true, cell: "<b>Hi</b>", argb: [255, 0x12, 0x34, 0x56], ..user("", [0, 12, 6, 18]) };
    let matte = S { argb: [0, 1, 2, 3], cell: "ignored", ..user("matte", [12, 0, 18, 6]) };
    let auto = S { origin: 0, ..user("auto", [0, 0, 20, 2]) };
    let data = v6(&[linked, plain, html, matte, auto], &[]);
    for layers in [true, false] {
        let f = with_resource(&psd(layers), 1050, &data);
        // Either conversion brings the slices in; the automatic one stays out.
        for p in [json!({}), json!({"layers": false})] {
            let (s, r) = open(&f, p);
            assert!(warnings(&r).is_empty(), "{r}");
            let got = slices(&s);
            assert_eq!(got.len(), 4, "{got:?}");
            assert_eq!(got[0].0, Rect::new(2.0, 4.0, 10.0, 8.0));
            assert_eq!(
                got[0].1,
                SliceOptions {
                    kind: SliceKind::Image,
                    name: "logo".into(),
                    url: "https://example.com/".into(),
                    target: "_blank".into(),
                    message: "Go".into(),
                    alt: "Logo".into(),
                    ..Default::default()
                }
            );
            // Plain cell text is escaped as the HTML a No Image cell shows; HTML stays as it is.
            assert_eq!((got[1].1.kind, got[1].1.text.as_str(), got[1].1.background.as_str()), (SliceKind::NoImage, "a &lt; b &amp; c", ""));
            assert_eq!((got[2].1.text.as_str(), got[2].1.background.as_str(), got[2].1.name.as_str()), ("<b>Hi</b>", "#123456", ""));
            // An Image slice's cell text isn't kept; a background with no alpha is the matte.
            assert_eq!((got[3].1.text.as_str(), got[3].1.background.as_str()), ("", "matte"));
            // Web output lays them out with auto slices around them.
            let layout = s.doc().unwrap().doc.slice_layout();
            assert_eq!(layout.iter().filter(|a| a.source == vectorcraft_doc::SliceSource::User).count(), 4);
            assert!(layout.iter().any(|a| a.source == vectorcraft_doc::SliceSource::Auto));
        }
    }
}

#[test]
fn photoshop_7_descriptors_after_version_6_records_are_read_past() {
    let trailing = [16u32.to_be_bytes().to_vec(), descriptor(&[("topOutset", D::Long(0))])].concat();
    let f = with_resource(&psd(false), 1050, &v6(&[user("a", [0, 0, 5, 5]), user("b", [5, 5, 10, 10])], &trailing));
    let (s, r) = open(&f, json!({}));
    assert!(warnings(&r).is_empty(), "{r}");
    let names: Vec<String> = slices(&s).into_iter().map(|(_, o)| o.name).collect();
    assert_eq!(names, ["a", "b"]);
}

#[test]
fn a_version_6_record_whose_id_is_16_is_a_slice_not_a_descriptor() {
    // The 16th record has ID 16, group ID 0, origin 2, no name and the Image type: its first 18
    // bytes also read as a descriptor of version 16 with no items.
    let records: Vec<S> = (1..=17).map(|id| if id == 16 { user("", [1, 1, 4, 4]) } else { user("a", [0, 0, 5, 5]) }).collect();
    let descriptor_after = [16u32.to_be_bytes().to_vec(), descriptor(&[("topOutset", D::Long(0))])].concat();
    for trailing in [vec![], descriptor_after] {
        let (s, r) = open(&with_resource(&psd(false), 1050, &v6(&records, &trailing)), json!({}));
        assert!(warnings(&r).is_empty(), "{r}");
        let got = slices(&s);
        assert_eq!(got.len(), 17, "{got:?}");
        assert_eq!((got[15].0, got[15].1.name.as_str()), (Rect::new(1.0, 1.0, 4.0, 4.0), ""));
        assert_eq!(got[16].1.name, "a");
    }
    // A descriptor after each record whose class ID is four zero bytes, with no items, followed by
    // records with no strings: the records don't also read one after another.
    let empty = [ustr(""), 0u32.to_be_bytes().to_vec(), vec![0; 4], 0u32.to_be_bytes().to_vec()].concat();
    let trailing = [16u32.to_be_bytes().to_vec(), empty].concat();
    for rect in [[0, 0, 0, 0], [0, 0, 5, 5]] {
        let (s, r) = open(&with_resource(&psd(false), 1050, &v6(&[user("", rect), user("", rect), user("", rect)], &trailing)), json!({}));
        let w = warnings(&r);
        assert!(!w.iter().any(|w| w.starts_with("the slices can't be read")), "{w:?}");
        assert_eq!(slices(&s).len(), if rect[2] > 0 { 3 } else { 0 }, "{w:?}");
    }
}

#[test]
fn version_6_descriptors_share_one_budget_of_values() {
    // Each record's descriptor holds 2,501 values, well under the 200,000 one descriptor may hold;
    // 100 of them hold 250,100 together.
    let list = D::List((0..2_500).map(D::Long).collect());
    let trailing = [16u32.to_be_bytes().to_vec(), descriptor(&[("many", list)])].concat();
    let records: Vec<S> = (0..100).map(|_| user("a", [0, 0, 5, 5])).collect();
    // Alone, one such descriptor is read past.
    let (s, r) = open(&with_resource(&psd(false), 1050, &v6(&records[..1], &trailing)), json!({}));
    assert!(warnings(&r).is_empty(), "{r}");
    assert_eq!(slices(&s).len(), 1);
    // Together they are refused, and the file opens without slices.
    let (s, r) = open(&with_resource(&psd(false), 1050, &v6(&records, &trailing)), json!({}));
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.starts_with("the slices can't be read (") && w.contains("too many values")), "{w:?}");
    assert!(slices(&s).is_empty());
}

#[test]
fn version_7_and_8_descriptors_open_as_user_slices() {
    let slice = |origin: &'static str, kind: &'static str, l: i32, t: i32, r: i32, b: i32| {
        D::Obj(vec![
            ("sliceID", D::Long(1)),
            ("groupID", D::Long(0)),
            ("origin", D::Enum("ESliceOrigin", origin)),
            ("Type", D::Enum("ESliceType", kind)),
            ("bounds", bounds(l, t, r, b)),
            ("Nm  ", D::Text("cell")),
            ("url", D::Text("https://example.com/")),
            ("null", D::Text("_top")),
            ("Msge", D::Text("msg")),
            ("altTag", D::Text("alt")),
            ("cellTextIsHTML", D::Bool(false)),
            ("cellText", D::Text("x>y")),
            ("bgColorType", D::Enum("ESliceBGColorType", "Clr ")),
            ("bgColor", D::Obj(vec![("Rd  ", D::Doub(255.0)), ("Grn ", D::Doub(128.0)), ("Bl  ", D::Long(0))])),
        ])
    };
    for version in [7, 8] {
        let data = v7(
            version,
            vec![slice("userGenerated", "Img ", 1, 2, 9, 10), slice("autoGenerated", "Img ", 0, 0, 20, 1), slice("layer", "noImage", 10, 10, 15, 15)],
        );
        let (s, r) = open(&with_resource(&psd(true), 1050, &data), json!({}));
        assert!(warnings(&r).is_empty(), "{r}");
        let got = slices(&s);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].0, Rect::new(1.0, 2.0, 9.0, 10.0));
        assert_eq!(
            got[0].1,
            SliceOptions {
                name: "cell".into(),
                url: "https://example.com/".into(),
                target: "_top".into(),
                message: "msg".into(),
                alt: "alt".into(),
                background: "#ff8000".into(),
                ..Default::default()
            }
        );
        assert_eq!((got[1].1.kind, got[1].1.text.as_str(), got[1].1.url.as_str()), (SliceKind::NoImage, "x&gt;y", ""));
    }
}

#[test]
fn import_slices_false_leaves_them_out_and_must_be_a_switch() {
    let f = with_resource(&psd(true), 1050, &v6(&[user("a", [0, 0, 5, 5])], &[]));
    let (s, _) = open(&f, json!({"importSlices": false}));
    assert!(slices(&s).is_empty());
    let mut s = Session::new();
    let e = s.execute("document.open", &json!({"name": "poster.psd", "dataBase64": b64(&f), "importSlices": "yes"})).unwrap_err();
    assert!(e.to_string().contains("importSlices must be true or false"), "{e}");
}

#[test]
fn slices_that_cant_be_read_warn_and_the_file_still_opens() {
    let good = v6(&[user("a", [0, 0, 5, 5])], &[]);
    let mut bad_descriptor = v7(7, vec![]);
    bad_descriptor[4..8].copy_from_slice(&15u32.to_be_bytes());
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (good[..good.len() - 3].to_vec(), "cut short"),
        ([9u32.to_be_bytes().to_vec(), good[4..].to_vec()].concat(), "version 9"),
        (bad_descriptor, "version other than 16"),
        // The count follows the version, the bounds and the group's name.
        ([&good[..34], &u32::MAX.to_be_bytes()[..], &good[38..]].concat(), "more than the 10000 read"),
        (v7(8, vec![D::Long(3)]), "other than a slice"),
        // A whole record whose name is longer than a slice's text may be.
        (v6(&[user("a".repeat(65_537).leak(), [0, 0, 5, 5])], &[]), "more than the 65536 characters read"),
    ];
    for (data, why) in cases {
        for layers in [true, false] {
            let (s, r) = open(&with_resource(&psd(layers), 1050, &data), json!({}));
            let w = warnings(&r);
            assert!(w.iter().any(|w| w.starts_with("the slices can't be read (") && w.contains(why)), "{why}: {w:?}");
            assert!(slices(&s).is_empty());
            // The art still came in.
            assert!(s.doc().unwrap().doc.art_bounds().is_some(), "{why}");
        }
    }
    // A name of the longest length read comes in.
    let longest = v6(&[user("a".repeat(65_536).leak(), [0, 0, 5, 5])], &[]);
    let (s, r) = open(&with_resource(&psd(false), 1050, &longest), json!({}));
    assert!(warnings(&r).is_empty(), "{r}");
    assert_eq!(slices(&s).first().map(|(_, o)| o.name.len()), Some(65_536));
    // A file without slices opens without a word about them.
    let (s, r) = open(&psd(true), json!({}));
    assert!(warnings(&r).is_empty() && slices(&s).is_empty(), "{r}");
}

#[test]
fn a_slices_resource_cut_short_warns_and_the_file_still_opens() {
    let good = v6(&[user("a", [0, 0, 5, 5])], &[]);
    // Image resource `id`, the last of the image resources, declares 10 bytes more than they hold.
    let cut = |layers: bool, id: u16, data: &[u8]| {
        let mut f = with_resource(&psd(layers), id, data);
        let sig = [&b"8BIM"[..], &id.to_be_bytes()].concat();
        let at = f.windows(6).rposition(|w| w == sig.as_slice()).unwrap() + 8;
        f[at..at + 4].copy_from_slice(&(data.len() as u32 + 10).to_be_bytes());
        f
    };
    for layers in [true, false] {
        let (s, r) = open(&cut(layers, 1050, &good), json!({}));
        let w = warnings(&r);
        assert!(w.iter().any(|w| w.starts_with("the slices can't be read (") && w.contains("cut short")), "{w:?}");
        assert!(slices(&s).is_empty());
        assert!(s.doc().unwrap().doc.art_bounds().is_some());
        // Another resource cut short says nothing about slices.
        let (s, r) = open(&cut(layers, 1051, &[0, 1]), json!({}));
        assert!(warnings(&r).is_empty() && slices(&s).is_empty(), "{r}");
    }
}

#[test]
fn slices_are_clipped_to_the_canvas_and_those_outside_it_are_left_out() {
    let data = v6(&[user("over", [-5, -5, 5, 30]), user("out", [25, 0, 30, 5]), user("empty", [4, 4, 4, 9]), user("in", [1, 1, 2, 2])], &[]);
    let (s, r) = open(&with_resource(&psd(false), 1050, &data), json!({}));
    assert_eq!(warnings(&r), ["2 slices lie outside the canvas or have no size and were left out"]);
    let got: Vec<(Rect, String)> = slices(&s).into_iter().map(|(r, o)| (r, o.name)).collect();
    assert_eq!(got, [(Rect::new(0.0, 0.0, 5.0, 20.0), "over".to_string()), (Rect::new(1.0, 1.0, 2.0, 2.0), "in".to_string())]);
}

#[test]
fn placed_embedded_files_bring_their_slices_with_the_art_in_one_undo_step() {
    let data = v6(&[user("a", [0, 0, 10, 10]), user("b", [10, 10, 20, 20])], &[]);
    for layers in [false, true] {
        let f = with_resource(&psd(layers), 1050, &data);
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        // Fitted into a 40 pt square at (100, 60): twice the size.
        let r = s.execute("file.place", &json!({"name": "poster.psd", "dataBase64": b64(&f), "rect": [100, 60, 40, 40], "layers": layers})).unwrap();
        assert_eq!(r["slices"].as_array().map(Vec::len), Some(2), "{r}");
        let got: Vec<Rect> = slices(&s).into_iter().map(|(r, _)| r).collect();
        assert_eq!(got, [Rect::new(100.0, 60.0, 120.0, 80.0), Rect::new(120.0, 80.0, 140.0, 100.0)]);
        // The placed art stays selected, and one undo takes the art and the slices away.
        assert_eq!(s.doc().unwrap().selection.objects.len(), 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(slices(&s).is_empty());
        assert!(s.doc().unwrap().doc.art_bounds().is_none());
        // Centered on `at` at 100%.
        s.execute("file.place", &json!({"name": "poster.psd", "dataBase64": b64(&f), "at": [100, 100], "layers": layers})).unwrap();
        assert_eq!(slices(&s)[0].0, Rect::new(90.0, 90.0, 100.0, 100.0));
        // `importSlices: false` places the art alone.
        let r = s.execute("file.place", &json!({"name": "poster.psd", "dataBase64": b64(&f), "importSlices": false, "layers": layers})).unwrap();
        assert!(r.get("slices").is_none());
        assert_eq!(slices(&s).len(), 2);
    }
}

#[test]
fn linked_files_are_placed_without_their_slices() {
    let dir = std::env::temp_dir().join(format!("vc-psd-slices-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("poster.psd");
    std::fs::write(&path, with_resource(&psd(false), 1050, &v6(&[user("a", [0, 0, 5, 5])], &[]))).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    assert_eq!((&r["linked"], r.get("slices")), (&json!(true), None));
    assert!(slices(&s).is_empty());
    let e = s.execute("file.place", &json!({ "path": path, "importSlices": true })).unwrap_err();
    assert!(e.to_string().contains("importSlices: a linked PSD file is placed as its flat image"), "{e}");
    // Embedded from its path, they come in.
    let r = s.execute("file.place", &json!({ "path": path, "link": false })).unwrap();
    assert_eq!((&r["linked"], r["slices"].as_array().map(Vec::len)), (&json!(false), Some(1)));
    // file.place.info counts what a place would bring in.
    let info = s.execute("file.place.info", &json!({ "path": path, "link": false })).unwrap();
    assert_eq!(info["slices"], json!(1));
    let info = s.execute("file.place.info", &json!({ "path": path })).unwrap();
    assert_eq!(info["slices"], json!(0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn psd_info_counts_the_slices_that_come_in() {
    let mut s = Session::new();
    let info = |s: &mut Session, f: &[u8]| s.execute("document.psdInfo", &json!({"dataBase64": b64(f)})).unwrap();
    let data = v6(&[user("a", [0, 0, 5, 5]), S { origin: 0, ..user("auto", [5, 5, 20, 20]) }, user("out", [30, 30, 40, 40])], &[]);
    assert_eq!(info(&mut s, &with_resource(&psd(false), 1050, &data))["slices"], json!(1));
    assert_eq!(info(&mut s, &with_resource(&psd(true), 1050, &data[..data.len() - 4]))["slices"], json!(0));
    assert_eq!(info(&mut s, &psd(true))["slices"], json!(0));
}

#[test]
fn damaged_slices_never_panic() {
    let data = v7(8, vec![D::Obj(vec![("origin", D::Enum("ESliceOrigin", "userGenerated")), ("bounds", bounds(0, 0, 5, 5))])]);
    let base = with_resource(&psd(false), 1050, &data);
    let start = base.windows(data.len()).position(|w| w == data).unwrap();
    for at in (start - 12..start + data.len()).step_by(3) {
        for v in [0u8, 0x7f, 0xff] {
            let mut f = base.clone();
            f[at] = v;
            let mut s = Session::new();
            let _ = s.execute("document.open", &json!({"name": "poster.psd", "dataBase64": b64(&f)}));
            let _ = s.execute("document.psdInfo", &json!({"dataBase64": b64(&f)}));
        }
    }
}
