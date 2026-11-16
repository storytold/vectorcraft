//! SVG encodings, profiles and embedded fonts through the engine: the options reach the file,
//! files in each encoding open again, and `document.serialize` reads them as text.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{Document, Node, NodeKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Grüße €"})).unwrap();
    s
}

fn serialize(s: &mut Session, svg: serde_json::Value) -> serde_json::Value {
    s.execute("document.serialize", &json!({"format": "svg", "svg": svg})).unwrap()
}

#[test]
fn each_encoding_reopens_and_serializes_as_text() {
    let mut s = session();
    for (encoding, name, bom) in [("utf8", "UTF-8", false), ("utf16", "UTF-16", true), ("latin1", "ISO-8859-1", false)] {
        let r = serialize(&mut s, json!({"encoding": encoding}));
        let text = r["text"].as_str().unwrap();
        assert!(text.contains("Grüße") && text.contains(&format!("encoding=\"{name}\"")), "{text}");
        let bytes = match r.get("dataBase64") {
            Some(b) => vectorcraft_format::base64_decode(b.as_str().unwrap()).unwrap(),
            None => text.as_bytes().to_vec(),
        };
        assert_eq!(bom, bytes.starts_with(&[0xfe, 0xff]), "{encoding}");
        assert_eq!(r.get("dataBase64").is_some(), encoding != "utf8", "{encoding}");
        for (file, data) in [("t.svg", bytes.clone()), ("t.svgz", vectorcraft_svg::compress_bytes(&bytes))] {
            let mut o = Session::new();
            o.execute("document.open", &json!({"name": file, "dataBase64": vectorcraft_format::base64_encode(&data)})).unwrap();
            let plain = o.execute("document.inspect", &json!({})).unwrap().to_string();
            assert!(plain.contains("Grüße €"), "{encoding} {file}: {plain}");
        }
    }
}

#[test]
fn profile_and_embedded_fonts_reach_the_writer() {
    let mut s = session();
    let tiny = serialize(&mut s, json!({"profile": "tiny12", "styling": "css"}));
    let tiny = tiny["text"].as_str().unwrap();
    assert!(tiny.contains("baseProfile=\"tiny\"") && !tiny.contains("<style>"), "{tiny}");
    let fonts = serialize(&mut s, json!({"embedFonts": true}));
    assert!(fonts["text"].as_str().unwrap().contains("@font-face{"));
    assert!(s.execute("document.serialize", &json!({"format": "svg", "svg": {"profile": "svg2"}})).is_err(), "unknown profiles are refused");
    assert!(s.execute("document.serialize", &json!({"format": "svg", "encoding": "ebcdic"})).is_err(), "unknown encodings are refused");
}

/// #550: an SVG of chosen artboards holds the art over each, not every artboard's art; with no
/// artboard named (as Save writes it) the whole document's art is there.
#[test]
fn a_chosen_artboard_s_svg_holds_only_its_own_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let x = |s: &Session, b: usize| s.doc().unwrap().doc.artboards[b].rect.x0;
    let (first, third) = (x(&s, 0), x(&s, 2));
    s.execute("shape.rectangle", &json!({"x": first + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": third + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    // A rectangle has straight sides only; the ellipse is curves.
    let shapes = |svg: &str| (vectorcraft_testkit::svg_shapes(svg), svg.contains(" C") || svg.contains("<ellipse") || svg.contains("<circle"));
    let text = |v: &serde_json::Value| v["text"].as_str().unwrap().to_string();
    let third_only = text(&s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "3"})).unwrap());
    assert_eq!(shapes(&third_only), (1, true), "{third_only}");
    let all = s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "all"})).unwrap();
    let files: Vec<String> = all["files"].as_array().unwrap().iter().map(text).collect();
    assert_eq!(files.iter().map(|f| shapes(f)).collect::<Vec<_>>(), [(1, false), (0, false), (1, true)]);
    // Export for Screens names one artboard at a time the same way.
    let screen = text(&s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap());
    assert_eq!(shapes(&screen), (1, false));
    let whole = text(&s.execute("document.serialize", &json!({"format": "svg"})).unwrap());
    assert_eq!(shapes(&whole).0, 2, "nothing named: the whole document's art");
}

/// Art grouped across artboards (one variant per artboard in one group) splits by artboard too.
#[test]
fn a_chosen_artboard_s_svg_leaves_out_its_neighbours_art_in_a_shared_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let x = |s: &Session, b: usize| s.doc().unwrap().doc.artboards[b].rect.x0;
    let (first, third) = (x(&s, 0), x(&s, 2));
    s.execute("shape.rectangle", &json!({"x": first + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": third + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let text = |v: &serde_json::Value| v["text"].as_str().unwrap().to_string();
    let third_only = text(&s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "3"})).unwrap());
    assert_eq!(vectorcraft_testkit::svg_shapes(&third_only), 1, "{third_only}");
    assert!(third_only.contains(" C") || third_only.contains("<ellipse") || third_only.contains("<circle"), "{third_only}");
}

/// An object off a chosen artboard whose drop shadow reaches onto it is in that artboard's SVG.
#[test]
fn a_chosen_artboard_s_svg_keeps_an_object_whose_shadow_reaches_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let x0 = s.doc().unwrap().doc.artboards[0].rect.x0;
    let id = s.execute("shape.rectangle", &json!({"x": x0 - 30.0, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let paths = |s: &mut Session| {
        let svg = s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap();
        let text = svg["text"].as_str().unwrap().to_string();
        vectorcraft_testkit::svg_shapes(&text)
    };
    assert_eq!(paths(&mut s), 0, "the plain rectangle is off the artboard");
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id], "params": {"x": 20, "y": 0, "blur": 0}})).unwrap();
    assert_eq!(paths(&mut s), 1, "its shadow reaches 10 pt onto the artboard");
}

/// A 100 × 100 document whose 3 pt. Round brush paints with a 20 pt nib, and the left edge of its
/// artboard.
fn nib_doc() -> (Document, f64) {
    let mut d = Document::new(100.0, 100.0);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
        panic!("the 3 pt. Round brush")
    };
    c.size = 20.0;
    vectorcraft_brush::store(&mut d, &lib);
    let x0 = d.artboards[0].rect.x0;
    (d, x0)
}

/// A line from (`x`, 40) to (`x`, 60) with a 1 pt stroke in `paint`, painted by the 20 pt nib of
/// [`nib_doc`] when `brushed`: the nib paints 10 pt either side of the line.
fn nib_line(d: &mut Document, x: f64, paint: vectorcraft_color::Paint, brushed: bool) -> vectorcraft_doc::Node {
    let mut look = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, paint, 1.0);
    if brushed {
        look.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    }
    let line = vectorcraft_geom::shapes::line((x, 40.0).into(), (x, 60.0).into());
    vectorcraft_doc::Node::path(d.alloc_id(), line, look)
}

fn black() -> vectorcraft_color::Paint {
    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK)
}

/// A brushed path just left of a chosen artboard, whose brush art paints onto it, is in that
/// artboard's SVG and SVGZ; a brushed path too far off for its brush art to reach it is not.
#[test]
fn a_chosen_artboard_s_svg_keeps_brush_art_reaching_in() {
    let (mut d, x0) = nib_doc();
    let layer = d.layers[0].id;
    let near = nib_line(&mut d, x0 - 5.0, black(), true);
    let far = nib_line(&mut d, x0 - 200.0, black(), true);
    let (near, far) = (d.insert(Some(layer), 0, near).unwrap(), d.insert(Some(layer), 0, far).unwrap());
    let kept = export::art_over(&d, d.artboards[0].rect, &mut vectorcraft_render::Renderer::new());
    assert!(kept.node(near).is_some(), "the nib paints 5 pt onto the artboard");
    assert!(kept.node(far).is_none(), "the nib paints 190 pt short of the artboard");
    let mut s = Session::new();
    s.add_document(d, None);
    let svg = s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap();
    let svg = svg["text"].as_str().unwrap().to_string();
    assert!(vectorcraft_testkit::svg_shapes(&svg) > 0, "{svg}");
    let svgz = s.execute("document.serialize", &json!({"format": "svgz", "artboard": 0})).unwrap();
    let svgz = vectorcraft_format::base64_decode(svgz["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(vectorcraft_svg::text_of(&svgz).unwrap(), svg, "the SVGZ is the SVG gzipped");
}

/// The room for brush art also keeps a brushed path in a plain group on a sublayer. A brushed
/// stroke without a paint draws no brush art, a clipping group's clip keeps its members' brush art
/// inside it, and a compound path is painted with its own appearance, not its paths': those stay
/// out when only brush art could reach the artboard.
#[test]
fn a_chosen_artboard_s_svg_keeps_nested_brush_art_but_not_unpainted_or_clipped_art() {
    use vectorcraft_doc::{LayerColor, Node, NodeKind};
    let (mut d, x0) = nib_doc();
    let layer = d.layers[0].id;
    let x = x0 - 5.0;
    // A brushed line in a plain group on a sublayer.
    let line = nib_line(&mut d, x, black(), true);
    let nested = line.id;
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(line)], clip: false });
    let mut sublayer = Node::layer(d.alloc_id(), "Sublayer", LayerColor::Preset(1));
    sublayer.children_mut().unwrap().push(Arc::new(group));
    d.insert(Some(layer), 0, sublayer).unwrap();
    // A brushed stroke without a paint.
    let unpainted = nib_line(&mut d, x, vectorcraft_color::Paint::None, true);
    let unpainted = d.insert(Some(layer), 0, unpainted).unwrap();
    // A clipping group whose clip, far left of the artboard, holds a brushed line just off it.
    let mut clip = Node::path(
        d.alloc_id(),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x0 - 100.0, 40.0, x0 - 90.0, 60.0)),
        vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0),
    );
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let member = nib_line(&mut d, x, black(), true);
    let clipped = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(member)], clip: true });
    let clipped = d.insert(Some(layer), 0, clipped).unwrap();
    // Compound paths of a brushed line: plain itself, then brushed itself.
    let mut compounds = vec![];
    for own in [false, true] {
        let member = nib_line(&mut d, x, black(), true);
        let mut compound =
            Node::new(d.alloc_id(), NodeKind::Compound { children: vec![Arc::new(member)], rule: vectorcraft_geom::FillRule::NonZero });
        compound.appearance = nib_line(&mut d, x, black(), own).appearance;
        compounds.push(d.insert(Some(layer), 0, compound).unwrap());
    }
    let kept = export::art_over(&d, d.artboards[0].rect, &mut vectorcraft_render::Renderer::new());
    assert!(kept.node(nested).is_some(), "the brushed line in the group on the sublayer");
    assert!(kept.node(unpainted).is_none(), "a brushed stroke without a paint");
    assert!(kept.node(clipped).is_none(), "a clipping group whose clip misses the artboard");
    assert!(kept.node(compounds[0]).is_none(), "a compound path that is not brushed itself");
    assert!(kept.node(compounds[1]).is_some(), "a compound path that is brushed itself");
}

/// The art over an artboard, among 300 instances with drop shadows and 2,000 symbols, is found
/// reading the symbol list once, and keeps the instances over it.
#[test]
fn art_over_an_artboard_reads_the_symbol_list_once() {
    let mut d = Document::new(100.0, 100.0);
    let square = |d: &mut Document, side: f64| {
        let r = vectorcraft_geom::Rect::new(-side, -side, side, side);
        let ap =
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0);
        Arc::new(Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), ap))
    };
    for i in 0..2000 {
        let art = square(&mut d, 0.5);
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: format!("Filler {i}"), art });
    }
    let art = square(&mut d, 2.0);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Mark".into(), art });
    let layer = d.layers[0].id;
    // Twenty rows of 15: the first ten over the artboard, the rest below it.
    let ids: Vec<_> = (0..300)
        .map(|i| {
            let xf = vectorcraft_geom::Affine::translate((f64::from(i % 15) * 6.0 + 5.0, f64::from(i / 15) * 10.0 + 5.0));
            let mut n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Mark".into(), xf });
            n.appearance.effects.push(vectorcraft_doc::Effect {
                id: "stylize.dropShadow".into(),
                params: json!({"x": 1, "y": 1, "blur": 0}),
                visible: true,
            });
            d.insert(Some(layer), usize::MAX, n).unwrap()
        })
        .collect();
    let before = vectorcraft_doc::symbols_read();
    let kept = export::art_over(&d, d.artboards[0].rect, &mut vectorcraft_render::Renderer::new());
    let read = vectorcraft_doc::symbols_read() - before;
    assert!(read <= d.symbols.len(), "{read} symbols read, {} in the document", d.symbols.len());
    assert_eq!(ids.iter().filter(|id| kept.node(**id).is_some()).count(), 150);
}

/// #3544: an SVG of one artboard on its own (an `artboard` or `range`, Export for Screens, an asset)
/// has the artboard's (or asset's) name as `<title>`; Save's SVG and Export for Screens' Full
/// Document keep the document's.
#[test]
fn an_svg_of_one_artboard_is_titled_after_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Icons", "width": 24, "height": 24, "artboards": 2})).unwrap();
    for (index, name) in [(0, "arrow-left"), (1, "arrow-right")] {
        s.execute("artboard.setProps", &json!({"index": index, "name": name})).unwrap();
    }
    let square = s.execute("shape.rectangle", &json!({"x": 4, "y": 4, "width": 16, "height": 16})).unwrap()["id"].clone();
    let title = |svg: &str| svg.lines().find_map(|l| l.trim().strip_prefix("<title>")?.strip_suffix("</title>")).map(str::to_string);
    let serialize = |s: &mut Session, p: serde_json::Value| s.execute("document.serialize", &p).unwrap()["text"].as_str().unwrap().to_string();
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg", "artboard": 1}))).as_deref(), Some("arrow-right"));
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg", "artboard": 0}))).as_deref(), Some("arrow-left"));
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg"}))).as_deref(), Some("Icons"), "Save's SVG");
    let all = s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "all"})).unwrap();
    let titles: Vec<_> = all["files"].as_array().unwrap().iter().map(|f| title(f["text"].as_str().unwrap())).collect();
    assert_eq!(titles, [Some("arrow-left".into()), Some("arrow-right".into())]);
    // Export for Screens: each artboard, an asset, and the Full Document.
    let screens = |s: &mut Session, p: serde_json::Value| -> Vec<Option<String>> {
        let r = s.execute("document.exportForScreens", &p).unwrap();
        r["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| title(&String::from_utf8(vectorcraft_format::base64_decode(f["dataBase64"].as_str().unwrap()).unwrap()).unwrap()))
            .collect()
    };
    let svg = json!([{"format": "svg"}]);
    assert_eq!(screens(&mut s, json!({"formats": svg})), [Some("arrow-left".into()), Some("arrow-right".into())]);
    let asset = s.execute("assets.add", &json!({"ids": [square], "name": "Square"})).unwrap()["assets"][0].clone();
    assert_eq!(screens(&mut s, json!({"assets": [asset], "formats": svg})), [Some("Square".into())]);
    assert_eq!(screens(&mut s, json!({"fullDocument": true, "formats": svg})), [Some("Icons".into())]);
}
