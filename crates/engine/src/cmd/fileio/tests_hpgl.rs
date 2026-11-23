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

/// A plot of the art's bounds is measured on the art it plots: art on a non-printing layer, on a
/// layer at 0% opacity or at 0% opacity in a group, a placed image and a path that paints nothing
/// don't move its origin, a clipping layer is measured as clipped, and a document whose only
/// visible art is such art is refused.
#[test]
fn a_plot_of_the_arts_bounds_leaves_out_the_layers_it_does_not_plot() {
    let mut s = two_boards();
    let plot = |s: &mut Session| s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false})).map(|v| bytes(&v));
    let alone = plot(&mut s).unwrap();
    let printing = s.doc().unwrap().doc.layers[0].id;
    for props in [json!({"printable": false}), json!({})] {
        let layer = s.execute("layer.new", &json!({})).unwrap()["id"].clone();
        if props.as_object().unwrap().is_empty() {
            s.execute("transparency.set", &json!({"ids": [layer], "opacity": 0})).unwrap();
        } else {
            let mut p = props.clone();
            p["id"] = layer.clone();
            s.execute("layer.setProps", &p).unwrap();
        }
        s.execute("shape.rectangle", &json!({"x": -300, "y": 500, "width": 20, "height": 20})).unwrap();
        assert_eq!(plot(&mut s).unwrap(), alone, "{props}");
    }
    // An object at 0% opacity in a group, on a layer that plots.
    s.execute("layer.new", &json!({})).unwrap();
    let far = s.execute("shape.rectangle", &json!({"x": -500, "y": 700, "width": 20, "height": 20})).unwrap()["id"].clone();
    s.execute("object.group", &json!({"ids": [far]})).unwrap();
    s.execute("transparency.set", &json!({"ids": [far], "opacity": 0})).unwrap();
    assert_eq!(plot(&mut s).unwrap(), alone, "0% opacity in a group");
    // A placed image and a path whose fill and stroke paint nothing.
    let mut png = vec![];
    image::DynamicImage::new_rgb8(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.execute("file.place", &json!({"name": "far.png", "dataBase64": b64(&png), "at": [-600, 800]})).unwrap();
    assert_eq!(plot(&mut s).unwrap(), alone, "a placed image");
    let bare = s.execute("shape.rectangle", &json!({"x": 500, "y": -400, "width": 20, "height": 20})).unwrap()["id"].clone();
    s.execute("paint.setStroke", &json!({"ids": [bare], "none": true})).unwrap();
    assert_eq!(plot(&mut s).unwrap(), alone, "a path that paints nothing");

    // A clipping layer's clipping path, which loses its paint, still clips the art measured: a
    // large square clipped to the first square draws nothing more.
    let clipping = s.execute("layer.new", &json!({})).unwrap()["id"].clone();
    s.execute("shape.rectangle", &json!({"x": -300, "y": -300, "width": 800, "height": 800})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("layer.clippingMask.toggle", &json!({"id": clipping})).unwrap();
    assert_eq!(plot(&mut s).unwrap(), alone, "a clipping layer");

    s.execute("layer.setProps", &json!({"id": clipping, "visible": false})).unwrap();
    s.execute("layer.setProps", &json!({"id": printing, "visible": false})).unwrap();
    let e = plot(&mut s).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");
}

/// A placed image in a symbol's art doesn't move a plot of the art's bounds either, and a
/// document whose only other visible art is an instance of that symbol is refused.
#[test]
fn a_plot_of_the_arts_bounds_leaves_out_images_in_symbol_art() {
    let mut s = two_boards();
    let plot = |s: &mut Session| s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false})).map(|v| bytes(&v));
    let alone = plot(&mut s).unwrap();
    let printing = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.new", &json!({})).unwrap();
    let mut png = vec![];
    image::DynamicImage::new_rgb8(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.execute("file.place", &json!({"name": "far.png", "dataBase64": b64(&png), "at": [-600, 800]})).unwrap();
    s.execute("symbol.new", &json!({"name": "Far"})).unwrap();
    assert!(s.doc().unwrap().doc.symbols.iter().any(|sym| sym.name == "Far"));
    assert_eq!(plot(&mut s).unwrap(), alone, "an image in symbol art");

    s.execute("layer.setProps", &json!({"id": printing, "visible": false})).unwrap();
    let e = plot(&mut s).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");

    // A symbol of a square and an image is measured by its square.
    let bounds = |s: &Session| super::export_art_bounds(&s.doc().unwrap().doc, "hpgl").unwrap();
    let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap()["id"].clone();
    let before = bounds(&s);
    let image = s.execute("file.place", &json!({"name": "near.png", "dataBase64": b64(&png), "at": [400, 600]})).unwrap()["ids"][0].clone();
    s.execute("symbol.new", &json!({"name": "Both", "ids": [square, image]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.symbols.len(), 2);
    let after = bounds(&s);
    for (a, b) in [(before.x0, after.x0), (before.y0, after.y0), (before.x1, after.x1), (before.y1, after.y1)] {
        assert!((a - b).abs() < 1e-6, "{before:?} {after:?}");
    }
}

/// In the art of a dynamic symbol a placed image and a path that paints nothing are left out as
/// well, until an instance paints the path, and a document whose only other visible art is an
/// instance of a dynamic symbol of an image is refused.
#[test]
fn a_plot_of_the_arts_bounds_leaves_out_images_in_dynamic_symbol_art() {
    let mut s = two_boards();
    let plot = |s: &mut Session| s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false})).map(|v| bytes(&v));
    let bounds = |s: &Session| super::export_art_bounds(&s.doc().unwrap().doc, "hpgl").unwrap();
    let same = |a: vectorcraft_geom::Rect, b: vectorcraft_geom::Rect| {
        [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(a, b)| (a - b).abs() < 1e-6)
    };
    let printing = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.new", &json!({})).unwrap();
    let mut png = vec![];
    image::DynamicImage::new_rgb8(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let image = s.execute("file.place", &json!({"name": "far.png", "dataBase64": b64(&png), "at": [-600, 800]})).unwrap()["ids"][0].clone();
    let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap()["id"].clone();
    let bare = s.execute("shape.rectangle", &json!({"x": 500, "y": -400, "width": 20, "height": 20})).unwrap()["id"].clone();
    s.execute("paint.setStroke", &json!({"ids": [bare], "none": true})).unwrap();
    let before = bounds(&s);
    let inst = s.execute("symbol.new", &json!({"name": "Mixed", "ids": [image, square, bare], "type": "dynamic"})).unwrap()["id"].clone();
    let after = bounds(&s);
    assert!(same(before, after), "{before:?} {after:?}");

    // A change of the path's blending mode leaves it painting nothing; once the instance paints the
    // path, the path is measured.
    let art = s.doc().unwrap().doc.symbols[0].art.clone();
    let x0 = |c: &std::sync::Arc<vectorcraft_doc::Node>| c.visual_bounds().map_or(f64::MIN, |b| b.x0);
    let at = (0..art.children().unwrap().len()).max_by(|a, b| x0(&art.children().unwrap()[*a]).total_cmp(&x0(&art.children().unwrap()[*b]))).unwrap();
    s.execute("symbol.instance.setAppearance", &json!({"id": inst, "path": [at], "blend": "multiply"})).unwrap();
    let blended = bounds(&s);
    assert!(same(before, blended), "{before:?} {blended:?}");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst, "path": [at], "stroke": "#000000"})).unwrap();
    let painted = bounds(&s);
    assert!(painted.x1 > 500.0 && painted.y0 < -390.0, "{painted:?}");
    // A later change to the same object, as a file can hold, takes the paint off again.
    {
        let id = vectorcraft_doc::NodeId(inst.as_u64().unwrap());
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let n = d.node_mut(id).unwrap();
        let mut overrides = n.extras().symbol_overrides.clone();
        overrides.push(vectorcraft_doc::SymbolOverride { path: vec![at as u32], opacity: Some(0.0), ..Default::default() });
        n.extras_mut().symbol_overrides = overrides.clone();
        let taken_off = bounds(&s);
        assert!(same(before, taken_off), "{before:?} {taken_off:?}");
        overrides.pop();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        d.node_mut(id).unwrap().extras_mut().symbol_overrides = overrides;
    }
    // Another instance, far above, that doesn't paint the path is measured without it, whether or
    // not the first one paints it.
    let other = s.execute("symbol.place", &json!({"name": "Mixed", "x": 0, "y": -5000})).unwrap()["id"].clone();
    let with_paint = bounds(&s);
    s.execute("symbol.instance.reset", &json!({"ids": [inst]})).unwrap();
    let without_paint = bounds(&s);
    assert!((with_paint.y0 - without_paint.y0).abs() < 1e-6, "{with_paint:?} {without_paint:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    let undone = bounds(&s);
    assert!(same(undone, with_paint), "{undone:?} {with_paint:?}");
    s.execute("select.set", &json!({"ids": [other]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();

    s.execute("select.set", &json!({"ids": [inst]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.execute("layer.setProps", &json!({"id": printing, "visible": false})).unwrap();
    let image = s.execute("file.place", &json!({"name": "far.png", "dataBase64": b64(&png), "at": [-600, 800]})).unwrap()["ids"][0].clone();
    s.execute("symbol.new", &json!({"name": "Image", "ids": [image], "type": "dynamic"})).unwrap();
    let e = plot(&mut s).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");

    // An instance that brings a group at 0% opacity back to full opacity still draws nothing when
    // the group holds only an image.
    let image = s.execute("file.place", &json!({"name": "far.png", "dataBase64": b64(&png), "at": [-600, 800]})).unwrap()["ids"][0].clone();
    let group = s.execute("object.group", &json!({"ids": [image]})).unwrap()["id"].clone();
    s.execute("transparency.set", &json!({"ids": [group], "opacity": 0})).unwrap();
    let inst = s.execute("symbol.new", &json!({"name": "Clear", "ids": [group], "type": "dynamic"})).unwrap()["id"].clone();
    let id = vectorcraft_doc::NodeId(inst.as_u64().unwrap());
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    d.node_mut(id).unwrap().extras_mut().symbol_overrides =
        [vec![], vec![0]].into_iter().map(|path| vectorcraft_doc::SymbolOverride { path, opacity: Some(1.0), ..Default::default() }).collect();
    let e = plot(&mut s).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");
}

/// A rectangle filled black, or one that paints nothing.
fn plot_rect(d: &mut vectorcraft_doc::Document, r: vectorcraft_geom::Rect, painted: bool) -> std::sync::Arc<vectorcraft_doc::Node> {
    use vectorcraft_color::{Color, Paint};
    let fill = if painted { Paint::solid(Color::BLACK) } else { Paint::None };
    let appearance = vectorcraft_doc::Appearance::basic(fill, Paint::None, 0.0);
    std::sync::Arc::new(vectorcraft_doc::Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), appearance))
}

/// An instance of symbol `symbol` placed by `xf`.
fn plot_instance(d: &mut vectorcraft_doc::Document, symbol: &str, xf: vectorcraft_geom::Affine) -> vectorcraft_doc::Node {
    vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf })
}

/// Symbol `name` holding `art` in a group.
fn plot_symbol(d: &mut vectorcraft_doc::Document, name: &str, art: Vec<std::sync::Arc<vectorcraft_doc::Node>>, dynamic: bool) {
    let group = vectorcraft_doc::Node::group(d.alloc_id(), art);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic, name: name.into(), art: std::sync::Arc::new(group) });
}

/// Puts `n` on the first layer of `d`.
fn plot_on_layer(d: &mut vectorcraft_doc::Document, n: vectorcraft_doc::Node) {
    std::sync::Arc::make_mut(&mut d.layers[0]).children_mut().unwrap().push(std::sync::Arc::new(n));
}

/// The art of a symbol with 9-slice guides, static or dynamic, is placed whole and measured
/// without what the plot leaves out: a frame that paints nothing sets where the guides place the
/// painted art inside it, and is not measured itself.
#[test]
fn a_plot_of_the_arts_bounds_places_9_slice_symbol_art_by_all_of_it() {
    use vectorcraft_geom::{Affine, Rect};
    // Static and dynamic, and with the instance's fill staining the art.
    for (dynamic, stained) in [(false, false), (true, false), (false, true)] {
        let mut d = vectorcraft_doc::Document::new(400.0, 400.0);
        let frame = plot_rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), false);
        let square = plot_rect(&mut d, Rect::new(40.0, 40.0, 60.0, 60.0), true);
        plot_symbol(&mut d, "Panel", vec![frame, square], dynamic);
        d.set_symbol_nine_slice("Panel", Some(vectorcraft_doc::SymbolNineSlice::default()));
        let mut wide = plot_instance(&mut d, "Panel", Affine::scale_non_uniform(2.0, 1.0));
        if stained {
            let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0));
            wide.appearance = vectorcraft_doc::Appearance::basic(red, vectorcraft_color::Paint::None, 0.0).into();
        }
        plot_on_layer(&mut d, wide);
        // Scaled 2 times across, the middle column maps x to 3x - 50: the square draws from x 70
        // to 130.
        let b = super::export_art_bounds(&d, "hpgl").unwrap();
        let exact = [(b.x0, 70.0), (b.y0, 40.0), (b.x1, 130.0), (b.y1, 60.0)].iter().all(|(a, e)| (a - e).abs() < 1e-6);
        assert!(exact, "dynamic: {dynamic}, stained: {stained}, {b:?}");
    }
}

/// In art placed through 9-slice guides, a group with a fill of its own keeps the path it paints,
/// which paints nothing by itself.
#[test]
fn a_plot_of_the_arts_bounds_keeps_what_a_group_paints_in_9_slice_symbol_art() {
    use vectorcraft_geom::{Affine, Rect};
    let mut d = vectorcraft_doc::Document::new(400.0, 400.0);
    let frame = plot_rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), false);
    let bare = plot_rect(&mut d, Rect::new(40.0, 40.0, 60.0, 60.0), false);
    let mut group = vectorcraft_doc::Node::group(d.alloc_id(), vec![bare]);
    let black = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
    group.appearance = vectorcraft_doc::Appearance::basic(black, vectorcraft_color::Paint::None, 0.0).into();
    plot_symbol(&mut d, "Panel", vec![frame, std::sync::Arc::new(group)], false);
    d.set_symbol_nine_slice("Panel", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let wide = plot_instance(&mut d, "Panel", Affine::scale_non_uniform(2.0, 1.0));
    plot_on_layer(&mut d, wide);
    let b = super::export_art_bounds(&d, "hpgl").expect("the group's path is plotted");
    let exact = [(b.x0, 70.0), (b.y0, 40.0), (b.x1, 130.0), (b.y1, 60.0)].iter().all(|(a, e)| (a - e).abs() < 1e-6);
    assert!(exact, "{b:?}");
}

/// A symbol without guides in the art of a symbol with 9-slice guides is measured without what the
/// plot leaves out, in the placed art and in an instance of its own.
#[test]
fn a_plot_of_the_arts_bounds_leaves_out_unplotted_art_of_symbols_in_9_slice_art() {
    use vectorcraft_geom::{Affine, Rect};
    let mut d = vectorcraft_doc::Document::new(400.0, 400.0);
    let far = plot_rect(&mut d, Rect::new(45.0, -500.0, 55.0, -480.0), false);
    let dot = plot_rect(&mut d, Rect::new(45.0, 45.0, 55.0, 55.0), true);
    plot_symbol(&mut d, "Dot", vec![far, dot], false);
    let frame = plot_rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), false);
    let inner = std::sync::Arc::new(plot_instance(&mut d, "Dot", Affine::IDENTITY));
    plot_symbol(&mut d, "Panel", vec![frame, inner], false);
    d.set_symbol_nine_slice("Panel", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let wide = plot_instance(&mut d, "Panel", Affine::scale_non_uniform(2.0, 1.0));
    plot_on_layer(&mut d, wide);
    let near = |b: Rect, e: [f64; 4]| [b.x0, b.y0, b.x1, b.y1].iter().zip(e).all(|(a, e)| (a - e).abs() < 1e-6);
    // The dot's box is in the middle column, which maps x to 3x - 50.
    let b = super::export_art_bounds(&d, "hpgl").unwrap();
    assert!(near(b, [85.0, 45.0, 115.0, 55.0]), "{b:?}");
    let own = plot_instance(&mut d, "Dot", Affine::translate((200.0, 0.0)));
    plot_on_layer(&mut d, own);
    let b = super::export_art_bounds(&d, "hpgl").unwrap();
    assert!(near(b, [85.0, 45.0, 255.0, 55.0]), "{b:?}");
}

/// The copy of a dynamic symbol that an instance's changes are measured on keeps the symbol's
/// 9-slice guides.
#[test]
fn a_plot_of_the_arts_bounds_measures_changed_9_slice_instances_by_their_guides() {
    use vectorcraft_geom::{Affine, Rect};
    let mut d = vectorcraft_doc::Document::new(400.0, 400.0);
    let bar = plot_rect(&mut d, Rect::new(10.0, 40.0, 70.0, 60.0), true);
    plot_symbol(&mut d, "Bar", vec![bar], false);
    let frame = plot_rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), false);
    let inner = std::sync::Arc::new(plot_instance(&mut d, "Bar", Affine::IDENTITY));
    plot_symbol(&mut d, "Panel", vec![frame, inner], true);
    d.set_symbol_nine_slice("Panel", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let mut wide = plot_instance(&mut d, "Panel", Affine::scale_non_uniform(2.0, 1.0));
    wide.extras_mut().symbol_overrides =
        vec![vectorcraft_doc::SymbolOverride { path: vec![0], fill: Some(vectorcraft_color::Paint::None), ..Default::default() }];
    plot_on_layer(&mut d, wide);
    // The bar crosses the left guide and is placed by the middle column's map, x to 3x - 50: it
    // draws from x -20 to 160.
    let b = super::export_art_bounds(&d, "hpgl").unwrap();
    let exact = |b: Rect| [(b.x0, -20.0), (b.y0, 40.0), (b.x1, 160.0), (b.y1, 60.0)].iter().all(|(a, e)| (a - e).abs() < 1e-6);
    assert!(exact(b), "{b:?}");
    // The same instance in the art of a symbol without guides.
    let wide = std::sync::Arc::make_mut(&mut d.layers[0]).children_mut().unwrap().pop().unwrap();
    plot_symbol(&mut d, "Holder", vec![wide], false);
    let holder = plot_instance(&mut d, "Holder", Affine::IDENTITY);
    plot_on_layer(&mut d, holder);
    let b = super::export_art_bounds(&d, "hpgl").unwrap();
    assert!(exact(b), "{b:?}");
}

/// An instance in a dynamic symbol's art, in a group at 0% opacity that an instance of the symbol
/// brings back to full opacity, is measured with the paint it gives its own symbol's art.
#[test]
fn a_plot_of_the_arts_bounds_measures_instances_that_an_instance_brings_back() {
    use vectorcraft_geom::{Affine, Rect};
    let mut d = vectorcraft_doc::Document::new(400.0, 400.0);
    let bare = plot_rect(&mut d, Rect::new(300.0, 300.0, 320.0, 320.0), false);
    plot_symbol(&mut d, "Inner", vec![bare], true);
    let mut inner = plot_instance(&mut d, "Inner", Affine::IDENTITY);
    let black = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
    inner.extras_mut().symbol_overrides = vec![vectorcraft_doc::SymbolOverride { path: vec![0], fill: Some(black), ..Default::default() }];
    plot_symbol(&mut d, "Outer", vec![std::sync::Arc::new(inner)], true);
    std::sync::Arc::make_mut(&mut d.symbols[1].art).opacity = 0.0;
    let mut outer = plot_instance(&mut d, "Outer", Affine::IDENTITY);
    outer.extras_mut().symbol_overrides = vec![vectorcraft_doc::SymbolOverride { path: vec![], opacity: Some(1.0), ..Default::default() }];
    plot_on_layer(&mut d, outer);
    let b = super::export_art_bounds(&d, "hpgl").expect("the painted rectangle is plotted");
    assert!((b.x0 - 300.0).abs() < 1e-6 && (b.x1 - 320.0).abs() < 1e-6, "{b:?}");
}

/// A placed image with a Vector Halftone effect is measured when the effect makes dots, and left
/// out when it would take more cells than the effect makes.
#[test]
fn a_plot_of_the_arts_bounds_measures_images_by_their_halftone_dots() {
    let mut s = two_boards();
    let plot = |s: &mut Session| s.execute("document.export", &json!({"format": "hpgl", "useArtboards": false})).map(|v| bytes(&v));
    let printing = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.new", &json!({})).unwrap();
    let mut png = vec![];
    image::DynamicImage::new_rgb8(100, 100).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.execute("file.place", &json!({"name": "dots.png", "dataBase64": b64(&png), "at": [300, 300]})).unwrap();
    s.execute("layer.setProps", &json!({"id": printing, "visible": false})).unwrap();

    // At 300 lines per inch a 100 pt image takes more cells than the effect makes.
    s.execute("effect.apply", &json!({"effect": "addon.color.halftone", "params": {"frequency": 300}})).unwrap();
    let e = plot(&mut s).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");

    s.execute("effect.apply", &json!({"effect": "addon.color.halftone", "params": {"frequency": 20}})).unwrap();
    let out = String::from_utf8(plot(&mut s).unwrap()).unwrap();
    assert!(out.contains("PD"), "{out}");
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
    let pens = |p: Value| hpgl::automatic_pens(doc, &p).unwrap();
    assert_eq!(pens(json!({})), ["#000000"], "the first artboard");
    assert_eq!(pens(json!({"artboard": 1})), ["#ff0000"]);
    assert_eq!(pens(json!({"useArtboards": true})), ["#000000", "#ff0000"], "one numbering for every file");
    assert_eq!(pens(json!({"useArtboards": false})), ["#000000", "#ff0000"], "the art's bounds");
    // The pens given and the path order are not the export's own pens.
    assert_eq!(pens(json!({"useArtboards": true, "pens": ["#00ff00"], "sortPaths": true})), ["#000000", "#ff0000"]);
    // A refused artboard choice or option is an error, not an empty table.
    assert!(hpgl::automatic_pens(doc, &json!({"artboard": 5})).is_err());
    assert!(hpgl::automatic_pens(doc, &json!({"curveTolerance": 20})).unwrap_err().contains("`curveTolerance`"));
    // So is an area of the page, which the export and the check refuse for HP-GL.
    assert!(hpgl::automatic_pens(doc, &json!({"rect": [0, 0, 10, 10]})).unwrap_err().contains("rect applies to raster formats"));

    // Art outside every artboard is plotted only with the art's bounds.
    let mut off = Session::new();
    off.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    off.execute("paint.setFill", &json!({"none": true})).unwrap();
    off.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    off.execute("shape.rectangle", &json!({"x": 500, "y": 500, "width": 30, "height": 30})).unwrap();
    let doc = &off.doc().unwrap().doc;
    assert_eq!(hpgl::automatic_pens(doc, &json!({})), Ok(vec![]));
    assert_eq!(hpgl::automatic_pens(doc, &json!({"useArtboards": false})), Ok(vec!["#0000ff".to_string()]));
}

/// A placed document opened with only its preview is plotted with its file's art (the plot reads
/// the file), so the check and the pens of a dialog measure that art as the export does.
#[test]
fn a_placed_document_s_pens_are_those_of_its_file_s_art() {
    use crate::tests_links::{Folder, open, save, session, write};
    let dir = Folder::new("hpgl-placed");
    let src = dir.file("logo.vectorcraft");
    let mut d = vectorcraft_doc::Document::new(100.0, 50.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0));
    let look = vectorcraft_doc::Appearance::basic(red, vectorcraft_color::Paint::None, 0.0);
    let rect = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 50.0));
    d.insert(Some(layer), 0, vectorcraft_doc::Node::path(id, rect, look)).unwrap();
    write(&src, &vectorcraft_format::save_file(&d));
    let mut s = session();
    s.execute("file.place", &json!({"path": src, "at": [200, 150]})).unwrap();
    let poster = dir.file("poster.vectorcraft");
    save(&mut s, &poster);
    open(&mut s, &poster);
    let doc = &s.doc().unwrap().doc;
    assert!(doc.images.values().any(vectorcraft_doc::ImageBlob::is_proxy), "opened with the preview only");
    for p in [json!({}), json!({"useArtboards": false})] {
        assert_eq!(hpgl::automatic_pens(doc, &p), Ok(vec!["#ff0000".to_string()]), "{p}");
        assert_eq!(hpgl::check(doc, &p), Ok(()), "{p}");
    }
    let plot = String::from_utf8(bytes(&s.execute("document.export", &json!({"format": "hpgl"})).unwrap())).unwrap();
    assert!(plot.contains("SP1;") && plot.contains("PD"), "{plot}");
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
