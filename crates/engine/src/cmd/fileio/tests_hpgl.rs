//! HP-GL plotter files through the commands: `document.open` by extension or by content, File ›
//! Place as one group of paths, the dialogs' filters and the format list, and files that aren't
//! HP-GL refused with a message.

use serde_json::json;
use vectorcraft_doc::NodeKind;

use super::*;

fn b64(bytes: &[u8]) -> String {
    vectorcraft_format::base64_encode(bytes)
}

/// An inch-wide red square and a black diagonal, with the thinnest pen.
const PLOT: &[u8] = b"IN;SP2;PW0;PA0,0;PD1016,0,1016,1016,0,1016,0,0;PU;SP1;PD1016,1016;PU;";

#[test]
fn document_open_reads_a_plot_file() {
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "part.plt", "dataBase64": b64(PLOT)})).unwrap();
    assert_eq!((r["format"].as_str(), r["title"].as_str()), (Some("hpgl"), Some("part.plt")), "{r}");
    let st = s.doc().unwrap();
    let board = st.doc.artboards[0].rect;
    assert!((board.width() - 72.25).abs() < 1e-9 && (board.height() - 72.25).abs() < 1e-9, "{board:?}");
    let mut paths = vec![];
    st.doc.walk(|n| {
        if let NodeKind::Path { .. } = n.kind {
            paths.push(n.clone());
        }
    });
    assert_eq!(paths.len(), 2);
    // Opened as artwork: Save asks for a name.
    assert!(st.path.is_none());

    // A plot file opens by its content in any of its names, and under a name no format reads.
    for name in ["drawing.prn", "PART.HPGL", "part.hgl", "part.hpg"] {
        let r = Session::new().execute("document.open", &json!({"name": name, "dataBase64": b64(PLOT)})).unwrap();
        assert_eq!(r["format"].as_str(), Some("hpgl"), "{name}: {r}");
    }
}

#[test]
fn file_place_places_a_plot_file_as_one_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let p = json!({"name": "part.plt", "dataBase64": b64(PLOT), "thumbnail": 16});
    let info = s.execute("file.place.info", &p).unwrap();
    assert_eq!(info["format"], "hpgl", "{info}");
    assert!((info["width"].as_f64().unwrap() - 72.25).abs() < 1e-9, "{info}");
    let r = s.execute("file.place", &p).unwrap();
    assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("hpgl"), Some(1)), "{r}");
    let doc = &s.doc().unwrap().doc;
    let group = doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    let NodeKind::Group { children, clip, .. } = &group.kind else { panic!("a group, got {:?}", group.kind) };
    assert_eq!((children.len(), *clip), (2, false), "the art's bounds, not clipped");
}

#[test]
fn files_that_are_not_hpgl_are_refused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    let before = s.doc().unwrap().doc.artboards[0].rect.size();
    let script = b"set terminal png\nplot sin(x)\n";
    let e = s.execute("document.open", &json!({"name": "graph.plt", "dataBase64": b64(script)})).expect_err("refused");
    assert!(e.to_string().contains("not an HP-GL file"), "{e}");
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), before);
    assert!(s.execute("file.place", &json!({"name": "graph.plt", "dataBase64": b64(script)})).is_err());
    // Text that isn't a plot opens by its name, not as HP-GL.
    assert!(detect("notes.txt", b"IN THE BEGINNING").is_none_or(|f| f.id != "hpgl"));
    // Only a plot file's own names, and names no other format reads, are read by their content:
    // text, other formats' files and the formats named as unsupported (a DWG drawing starts with
    // `AC` and digits) aren't, even when they start like a plot.
    for name in ["notes.txt", "notes.svg", "picture.tga", "drawing.dxf", "site.dwg", "old.pct"] {
        for bytes in [&b"IN"[..], b"in 5;", b"AC1032\0\0", PLOT] {
            assert!(detect(name, bytes).is_none_or(|f| f.id != "hpgl"), "{name}: {:?}", String::from_utf8_lossy(bytes));
        }
    }
    let text = s.execute("file.place", &json!({"name": "notes.txt", "dataBase64": b64(b"IN 5; PA 10;")})).unwrap();
    assert_eq!(text["format"], "txt", "{text}");
}

#[test]
fn the_dialogs_and_the_format_list_name_hpgl() {
    let exts = ["plt", "hpgl", "hgl", "hpg"];
    for e in exts {
        assert!(OPEN_EXTS.contains(&e) && PLACE_EXTS.contains(&e) && UNASSOCIATED_EXTS.contains(&e), "{e}");
        assert_eq!(format_for_name(&format!("a.{e}")).map(|f| f.id), Some("hpgl"), "{e}");
    }
    let filter = ("HP-GL", &exts[..]);
    assert!(open_filters().any(|f| f == filter) && place_filters().any(|f| f == filter));
    let r = Session::new().execute("document.formats", &json!({})).unwrap();
    let f = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "hpgl").unwrap().clone();
    assert_eq!((f["read"].as_bool(), f["write"].as_bool(), f["raster"].as_bool()), (Some(true), Some(false), Some(false)));
}
