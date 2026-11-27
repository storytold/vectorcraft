//! PICT pictures through the commands: `document.open` by extension or by content, File › Place as
//! one group clipped to the picture's frame, the dialogs' filters and the format list, and files
//! that aren't pictures, or a PICT export, refused with a message.

use serde_json::json;
use vectorcraft_doc::NodeKind;

use super::*;

fn b64(bytes: &[u8]) -> String {
    vectorcraft_format::base64_encode(bytes)
}

/// A version 2 PICT file of a 200 × 100 pt frame: a red rectangle and a line.
fn picture() -> Vec<u8> {
    let words = |v: &[i16]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    let mut b = vec![0; 512];
    b.extend(words(&[0, 0, 0, 100, 200]));
    b.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    b.extend(words(&[-1, 0]));
    b.extend([0; 20]);
    b.extend(words(&[0x1A, -1, 0, 0, 0x31, 10, 20, 50, 80, 0x20, 70, 10, 70, 190, 0xFF]));
    b
}

#[test]
fn document_open_reads_a_picture() {
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "logo.pct", "dataBase64": b64(&picture())})).unwrap();
    assert_eq!((r["format"].as_str(), r["title"].as_str()), (Some("pict"), Some("logo.pct")), "{r}");
    assert_eq!(r["warnings"], json!([]));
    let st = s.doc().unwrap();
    assert_eq!(st.doc.artboards[0].rect.size(), (200.0, 100.0).into());
    let mut paths = vec![];
    st.doc.walk(|n| {
        if let NodeKind::Path { .. } = n.kind {
            paths.push(n.clone());
        }
    });
    assert_eq!(paths.len(), 2);
    let b = paths[0].geometric_bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 1e-9 && (b.width() - 60.0).abs() < 1e-9, "{b:?}");
    // Opened as artwork: Save asks for a name.
    assert!(st.path.is_none());

    // A picture opens by its content under a name no format reads, and in each of its names.
    for name in ["logo.PICT", "logo.pic", "logo.bin", "logo"] {
        let r = Session::new().execute("document.open", &json!({"name": name, "dataBase64": b64(&picture())})).unwrap();
        assert_eq!(r["format"].as_str(), Some("pict"), "{name}: {r}");
    }
    assert_eq!(detect("logo.dat", &picture()).map(|f| f.id), Some("pict"));
    assert_eq!(detect("bare.dat", &picture()[512..]).map(|f| f.id), Some("pict"));
}

#[test]
fn file_place_places_a_picture_as_one_clipped_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let p = json!({"name": "logo.pct", "dataBase64": b64(&picture()), "at": [150, 150]});
    let info = s.execute("file.place.info", &p).unwrap();
    assert_eq!(info["format"], "pict", "{info}");
    assert!((info["width"].as_f64().unwrap() - 200.0).abs() < 1e-9, "{info}");
    let r = s.execute("file.place", &p).unwrap();
    assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("pict"), Some(1)), "{r}");
    let doc = &s.doc().unwrap().doc;
    let group = doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    assert_eq!(group.name.as_deref(), Some("logo.pct"));
    let NodeKind::Group { clip: true, .. } = &group.kind else { panic!("a clipping group, got {:?}", group.kind) };
    let b = group.geometric_bounds().unwrap();
    assert!((b.width() - 200.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn files_that_are_not_pictures_are_refused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let e = s.execute("document.open", &json!({"name": "old.pct", "dataBase64": b64(&[0; 600])})).expect_err("refused");
    assert!(e.to_string().contains("not a PICT file"), "{e}");
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), (100.0, 50.0).into());
    assert!(s.execute("file.place", &json!({"name": "old.pct", "dataBase64": b64(&[0; 600])})).is_err());
    // Other formats' files aren't taken for pictures by their content.
    // A file that starts with a PNG signature and holds a picture's version opcodes at byte 522.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(&picture()[8..]);
    assert!(detect("x.png", &png).is_none_or(|f| f.id != "pict"));
    let e = s.execute("document.export", &json!({"format": "pict"})).expect_err("refused").to_string();
    assert!(e.contains("PICT files can be opened but not written"), "{e}");
}

#[test]
fn the_dialogs_and_the_format_list_name_pict() {
    let exts = ["pct", "pic", "pict"];
    for e in exts {
        assert!(OPEN_EXTS.contains(&e) && PLACE_EXTS.contains(&e) && UNASSOCIATED_EXTS.contains(&e), "{e}");
        assert_eq!(format_for_name(&format!("a.{e}")).map(|f| f.id), Some("pict"), "{e}");
    }
    let filter = ("PICT", &exts[..]);
    assert!(open_filters().any(|f| f == filter) && place_filters().any(|f| f == filter));
    let r = Session::new().execute("document.formats", &json!({})).unwrap();
    let f = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "pict").unwrap().clone();
    assert_eq!((f["read"].as_bool(), f["write"].as_bool(), f["raster"].as_bool()), (Some(true), Some(false), Some(false)), "{f}");
    assert_eq!(f["mime"], "image/x-pict");
    assert!(writable_format(None, Some("x/logo.pct")).is_err());
}
