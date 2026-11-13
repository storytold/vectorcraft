//! HP-GL plotter files through the commands: `document.open` by extension or by content, File ›
//! Place as one group of paths, the dialogs' filters and the format list, files that aren't
//! HP-GL refused with a message, and `document.export` of one artboard, each or the art, with its
//! options checked.

use serde_json::{Value, json};
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

/// A plot file named as one opens when its first instruction is one a plot file seldom starts with;
/// under another name it isn't taken for HP-GL.
#[test]
fn a_plot_file_by_its_name_may_start_with_any_instruction() {
    let plot = b"SI0.2,0.3;SP1;PA0,0;PD1016,0;";
    let r = Session::new().execute("document.open", &json!({"name": "part.plt", "dataBase64": b64(plot)})).unwrap();
    assert_eq!(r["format"].as_str(), Some("hpgl"), "{r}");
    assert!(detect("drawing.prn", plot).is_none_or(|f| f.id != "hpgl"));
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
    assert_eq!((f["read"].as_bool(), f["write"].as_bool(), f["raster"].as_bool()), (Some(true), Some(true), Some(false)));
    for o in ["useArtboards", "hpglVersion", "curveTolerance", "pens", "origin", "sortPaths", "insideFirst"] {
        assert!(f["options"].get(o).is_some(), "{o}: {f}");
    }
    assert_eq!(writable_format(None, Some("x/part.plt")).unwrap().id, "hpgl");
    assert_eq!(writable_format(None, Some("x/part.hpgl")).unwrap().id, "hpgl");
}

fn bytes(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// Two 100 pt artboards side by side, a 30 pt square on each, the second one red.
fn two_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 2})).unwrap();
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let red = s.execute("shape.rectangle", &json!({"x": second + 10.0, "y": 10, "width": 30, "height": 30})).unwrap()["id"].clone();
    s.execute("paint.setStroke", &json!({"ids": [red], "color": "#ff0000"})).unwrap();
    s
}

#[test]
fn export_writes_a_plot_of_one_artboard_or_each() {
    let mut s = two_boards();
    let r = s.execute("document.export", &json!({"format": "hpgl"})).unwrap();
    assert_eq!(r["format"], "hpgl");
    // The first artboard's square, 1016 units to the inch from its bottom-left corner.
    assert_eq!(String::from_utf8(bytes(&r)).unwrap(), "IN;PU;SP1;PU141,1270;PD564,1270,564,847,141,847,141,1270;PU;SP0;");
    let second = String::from_utf8(bytes(&s.execute("document.export", &json!({"format": "hpgl", "artboard": 1})).unwrap())).unwrap();
    assert_eq!(second, "IN;PU;SP1;PU141,1270;PD564,1270,564,847,141,847,141,1270;PU;SP0;", "its own square, its own pen 1");
    assert_eq!(bytes(&s.execute("document.serialize", &json!({"format": "hpgl"})).unwrap()), bytes(&r));
    let r = s.execute("document.export", &json!({"format": "hpgl", "useArtboards": true})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[1]["name"].as_str().unwrap().ends_with("-Artboard-2.plt"), "{files:?}");
    // One pen numbering for every file: the red square is pen 2 in the second.
    assert_eq!(String::from_utf8(bytes(&files[1])).unwrap(), "IN;PU;SP2;PU141,1270;PD564,1270,564,847,141,847,141,1270;PU;SP0;");
    // The bounds of the art: both squares, the second pen red, read back as two paths.
    let art = bytes(&s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false, "hpglVersion": "hpgl2"})).unwrap());
    assert!(art.starts_with(b"IN;NP4;PC1,0,0,0;PC2,255,0,0;"), "{:?}", String::from_utf8_lossy(&art));
    let back = Session::new().execute("document.open", &json!({"name": "art.plt", "dataBase64": b64(&art)})).unwrap();
    assert_eq!(back["format"], "hpgl", "{back}");
    // One file holds one artboard.
    assert!(s.execute("document.export", &json!({"format": "hpgl", "artboards": [0, 1]})).is_err());
}

/// A plot read back opens with the same squares: as wide as the art drew them, a point at most
/// apart from the 30 pt sides plus the 0.35 mm pen.
#[test]
fn a_plot_reads_back_as_its_art() {
    let mut s = two_boards();
    let plot = bytes(&s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false})).unwrap());
    let mut back = Session::new();
    back.execute("document.open", &json!({"name": "art.plt", "dataBase64": b64(&plot)})).unwrap();
    let doc = &back.doc().unwrap().doc;
    let mut paths = 0;
    doc.walk(|n| paths += usize::from(matches!(n.kind, NodeKind::Path { .. })));
    assert_eq!(paths, 2);
    let board = doc.artboards[0].rect;
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    let pen = 0.35 * 72.0 / 25.4;
    assert!((board.width() - (second + 30.0 + pen)).abs() < 0.1, "{board:?}");
    assert!((board.height() - (30.0 + pen)).abs() < 0.1, "{board:?}");
}

#[test]
fn export_options_are_checked() {
    let mut s = two_boards();
    let refused = |s: &mut Session, p: Value| s.execute("document.export", &p).expect_err("refused").to_string();
    let cases = [
        (json!({"format": "hpgl", "hpglVersion": "hpgl3"}), "`hpglVersion` must be one of hpgl, hpgl2"),
        (json!({"format": "hpgl", "hpglVersion": 2}), "`hpglVersion` must be a string"),
        (json!({"format": "hpgl", "curveTolerance": 0}), "`curveTolerance` must be between 0.01 and 10 (got 0)"),
        (json!({"format": "hpgl", "curveTolerance": "fine"}), "`curveTolerance` must be a number"),
        (json!({"format": "hpgl", "pens": []}), "`pens` must be an array of 1 to 255 #rrggbb colors"),
        (json!({"format": "hpgl", "pens": ["#ff0000", "red"]}), "`pens` must be an array of 1 to 255 #rrggbb colors"),
        (json!({"format": "hpgl", "pens": "#ff0000"}), "`pens` must be an array of 1 to 255 #rrggbb colors"),
        (json!({"format": "hpgl", "origin": "top"}), "`origin` must be one of bottomLeft, center"),
        (json!({"format": "hpgl", "sortPaths": "yes"}), "`sortPaths` must be true or false"),
        (json!({"format": "hpgl", "insideFirst": 1}), "`insideFirst` must be true or false"),
    ];
    for (p, want) in cases {
        let e = refused(&mut s, p.clone());
        assert!(e.contains(want), "{p}: {e}");
    }
    let many: Vec<String> = (0..256).map(|i| format!("#{:02x}0000", i % 256)).collect();
    assert!(refused(&mut s, json!({"format": "hpgl", "pens": many})).contains("`pens`"));
    // Given options take effect: the center as the origin, a pen table (black is pen 2), and the
    // square starting at its corner nearest the origin.
    let p = json!({"format": "hpgl", "origin": "CENTER", "pens": ["#ff0000", "#000000"], "curveTolerance": 10, "sortPaths": true});
    let plot = String::from_utf8(bytes(&s.execute("document.export", &p).unwrap())).unwrap();
    assert_eq!(plot, "IN;PU;SP2;PU-141,141;PD-564,141,-564,564,-141,564,-141,141;PU;SP0;");
    // The check a dialog runs refuses the same options, and passes the given ones.
    let doc = s.doc().unwrap().doc.clone();
    assert!(hpgl::check(&doc, &json!({"pens": ["#ff0000", "red"]})).unwrap_err().contains("`pens`"));
    assert!(hpgl::check(&doc, &json!({"curveTolerance": 11})).unwrap_err().contains("`curveTolerance`"));
    assert_eq!(hpgl::check(&doc, &p), Ok(()));
    // It checks the artboard choice as the export does: an artboard or range past the document's,
    // a `useArtboards` that isn't true or false, and the art's bounds when no art shows.
    let mut blank = Session::new();
    blank.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let blank_doc = blank.doc().unwrap().doc.clone();
    for (on_blank, p) in [
        (false, json!({"format": "hpgl", "artboard": 2})),
        (false, json!({"format": "hpgl", "useArtboards": true, "range": "1-3"})),
        (false, json!({"format": "hpgl", "artboards": [0, 1]})),
        (false, json!({"format": "hpgl", "useArtboards": "yes"})),
        (true, json!({"format": "hpgl", "useArtboards": false})),
    ] {
        let (session, d) = if on_blank { (&mut blank, &blank_doc) } else { (&mut s, &doc) };
        assert!(hpgl::check(d, &p).is_err(), "{p}");
        assert!(session.execute("document.export", &p).is_err(), "{p}");
    }
    // An artboard with no width is refused before the writer refuses it.
    let mut flat = vectorcraft_doc::Document::clone(&doc);
    flat.artboards[1].rect = vectorcraft_geom::Rect::new(200.0, 0.0, 200.0, 100.0);
    assert_eq!(hpgl::check(&flat, &json!({"artboard": 1})).unwrap_err(), hpgl::check(&flat, &json!({"useArtboards": true})).unwrap_err());
    assert!(hpgl::check(&flat, &json!({"artboard": 1})).unwrap_err().contains("the export region is empty"));
    assert_eq!(hpgl::check(&flat, &json!({})), Ok(()));
    // A document with no artboard has nothing to plot on one, or on each.
    flat.artboards.clear();
    for p in [json!({}), json!({"useArtboards": true})] {
        assert!(hpgl::check(&flat, &p).unwrap_err().contains("the document has no artboard"), "{p}");
    }
    // An area of the page is a raster export's option: the export refuses it, and so does the check.
    let rect = json!({"format": "hpgl", "rect": [0, 0, 10, 10]});
    assert!(s.execute("document.export", &rect).is_err());
    assert!(hpgl::check(&doc, &rect).unwrap_err().contains("rect applies to raster formats"));
    assert_eq!(hpgl::check(&doc, &json!({"useArtboards": true, "range": "2"})), Ok(()));
    assert_eq!(hpgl::check(&doc, &json!({"useArtboards": false})), Ok(()));
}

/// The pens an export without a pen table uses: one per color the plotted art draws in, in the
/// order it first draws in each, over the artboards or the art's bounds the export plots.
#[test]
fn automatic_pens_list_the_plotted_art_s_colors_in_pen_order() {
    let s = two_boards();
    let doc = &s.doc().unwrap().doc;
    let pens = |p: Value| hpgl::automatic_pens(doc, &p);
    assert_eq!(pens(json!({})), ["#000000"], "the first artboard");
    assert_eq!(pens(json!({"artboard": 1})), ["#ff0000"]);
    assert_eq!(pens(json!({"useArtboards": true})), ["#000000", "#ff0000"], "one numbering for every file");
    assert_eq!(pens(json!({"useArtboards": false})), ["#000000", "#ff0000"], "the art's bounds");
    // The pens given and the path order are not the export's own pens.
    assert_eq!(pens(json!({"useArtboards": true, "pens": ["#00ff00"], "sortPaths": true})), ["#000000", "#ff0000"]);
    // A refused artboard choice: no pens.
    assert!(pens(json!({"artboard": 5})).is_empty());

    // Art outside every artboard is plotted only with the art's bounds.
    let mut off = Session::new();
    off.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    off.execute("paint.setFill", &json!({"none": true})).unwrap();
    off.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    off.execute("shape.rectangle", &json!({"x": 500, "y": 500, "width": 30, "height": 30})).unwrap();
    let doc = &off.doc().unwrap().doc;
    assert!(hpgl::automatic_pens(doc, &json!({})).is_empty());
    assert_eq!(hpgl::automatic_pens(doc, &json!({"useArtboards": false})), ["#0000ff"]);
}

/// `insideFirst` draws the square inside the other one first, though it was made last.
#[test]
fn inside_first_draws_the_inner_square_first() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 80})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 30, "y": 30, "width": 20, "height": 20})).unwrap();
    let starts = |s: &mut Session, p: Value| {
        let plot = String::from_utf8(bytes(&s.execute("document.export", &p).unwrap())).unwrap();
        (plot.find("PU141,1270;").unwrap(), plot.find("PU423,988;").unwrap())
    };
    let (outer, inner) = starts(&mut s, json!({"format": "hpgl"}));
    assert!(outer < inner);
    let (outer, inner) = starts(&mut s, json!({"format": "hpgl", "insideFirst": true}));
    assert!(inner < outer);
}

/// Export Selection plots the selected square alone (the red one, made last), from the
/// bottom-left corner of its bounds.
#[test]
fn export_selection_writes_a_plot_of_the_selection() {
    let mut s = two_boards();
    let r = s.execute("document.exportSelection", &json!({"format": "hpgl"})).unwrap();
    assert_eq!(r["format"], "hpgl");
    let plot = String::from_utf8(bytes(&r)).unwrap();
    assert_eq!(plot.matches("PD").count(), 1, "{plot}");
    assert!(plot.starts_with("IN;PU;SP1;PU7,430;PD"), "{plot}");
}
