//! Symbols export as one `<symbol>` def and a `<use>` per instance (instances the def can't stand
//! for get their own art), hidden layers and objects are kept hidden when asked and come back
//! hidden with their `data-*` attributes, clipped layers come in as clipping layers, layers and
//! sublayers keep their names and nesting (a one-layer file's from its root element), and the
//! editing data notices edits made elsewhere.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::pattern::pattern_paint;
use vectorcraft_doc::{Appearance, Document, Node, NodeKind, PatternDef, Symbol};
use vectorcraft_geom::{Affine, Rect, shapes};
use vectorcraft_svg::{ExportOptions, editing, export, export_full, import};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

/// The exported SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options { dpi: 72.0, ..Default::default() }).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

/// A 200×120 document with symbol "My Mark" (a blue square with a thick black stroke, `stroke`
/// wide) and an instance per transform.
fn symbol_doc(stroke: f64, xfs: &[Affine]) -> Document {
    let mut d = Document::new(200.0, 120.0);
    let ap = Appearance::basic(Paint::solid(Color::rgb8(30, 90, 220)), Paint::solid(Color::BLACK), stroke);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)), ap);
    d.symbols.push(Symbol { name: "My Mark".into(), art: Arc::new(art) });
    for xf in xfs {
        instance(&mut d, "My Mark", *xf);
    }
    d
}

fn instance(d: &mut Document, symbol: &str, xf: Affine) -> Node {
    let n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf });
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n.clone()).unwrap();
    n
}

#[test]
fn three_instances_give_one_symbol_and_three_uses() {
    let d = symbol_doc(4.0, &[Affine::translate((30.0, 30.0)), Affine::translate((100.0, 40.0)), Affine::translate((160.0, 90.0))]);
    let svg = export(&d, &ExportOptions::default());
    assert_eq!(svg.matches("<symbol ").count(), 1, "{svg}");
    assert_eq!(svg.matches("<use ").count(), 3, "{svg}");
    assert!(svg.contains("<symbol id=\"My_Mark\"") && svg.contains("xlink:href=\"#My_Mark\""), "{svg}");
    // The art is written once.
    assert_eq!(svg.matches("<path").count(), 1, "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 200, 120), 24.0, 0.002);
    // Read back (since SVG import keeps symbols: as the symbol and three instances): the three
    // squares are where the instances are.
    let back = import(&svg).unwrap();
    assert_eq!(back.symbols.len(), 1);
    assert_eq!(back.symbols[0].name, "My Mark", "the name comes back");
    let art = back.symbols[0].art.path_data().unwrap().bounds().unwrap();
    let mut boxes = vec![];
    back.walk(|n| {
        if let NodeKind::SymbolInstance { xf, .. } = &n.kind {
            boxes.push(xf.transform_rect_bbox(art));
        }
    });
    assert_eq!(boxes.len(), 3, "{boxes:?}");
    assert!(boxes.iter().any(|b| (b.x0 - 90.0).abs() < 1e-3 && (b.y0 - 30.0).abs() < 1e-3), "{boxes:?}");
}

#[test]
fn instances_the_def_cant_stand_for_get_their_own_art() {
    // Rotated: shared (a stroke turns with its path). Scaled: stroke weights don't scale on the
    // canvas, so that instance is its own art.
    let rot = Affine::translate((100.0, 60.0)) * Affine::rotate(0.5);
    let scaled = Affine::translate((160.0, 60.0)) * Affine::scale(1.5);
    let mut d = symbol_doc(6.0, &[Affine::translate((40.0, 60.0)), rot, scaled]);
    // Stained: its fill tints the art.
    let mut stained = instance(&mut d, "My Mark", Affine::translate((40.0, 20.0)));
    stained.appearance = Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::None, 0.0);
    stained.appearance.items.retain(|i| i.is_fill());
    if let vectorcraft_doc::AppearanceItem::Fill(f) = &mut stained.appearance.items[0] {
        f.opacity = 0.5;
    }
    let id = stained.id;
    *d.node_mut(id).unwrap() = stained;
    let svg = export(&d, &ExportOptions::default());
    assert_eq!((svg.matches("<symbol ").count(), svg.matches("<use ").count()), (1, 2), "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 200, 120), 24.0, 0.003);

    // Without strokes, scaling keeps the def exact.
    let d = symbol_doc(0.0, &[Affine::translate((40.0, 60.0)), scaled]);
    let svg = export(&d, &ExportOptions::default());
    assert_eq!(svg.matches("<use ").count(), 2, "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 200, 120), 24.0, 0.002);
}

#[test]
fn pattern_paints_in_symbols_stay_on_the_page() {
    let mut d = Document::new(200.0, 120.0);
    let red = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::rgb8(220, 0, 0)), Paint::None, 0.0),
    );
    let mut def = PatternDef::new("Checks", vec![Arc::new(red)]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(def);
    let art =
        Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
    d.symbols.push(Symbol { name: "Tiles".into(), art: Arc::new(art) });
    instance(&mut d, "Tiles", Affine::translate((10.0, 10.0)));
    instance(&mut d, "Tiles", Affine::translate((105.0, 37.0)));
    let svg = export(&d, &ExportOptions::default());
    assert!(!svg.contains("<use "), "{svg}");
    // Both instances paint the one page-fixed tiling.
    assert_eq!(svg.matches("<pattern ").count(), 1, "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 200, 120), 24.0, 0.003);
}

/// `d` with every top-level instance replaced by its art moved as the instance moves it.
fn expanded(d: &Document) -> Document {
    let mut e = d.clone();
    let l = Arc::make_mut(&mut e.layers[0]);
    for c in l.children_mut().unwrap().iter_mut() {
        if let NodeKind::SymbolInstance { symbol, xf } = &c.kind {
            let sym = d.symbols.iter().find(|s| s.name == *symbol).unwrap();
            let mut art = vectorcraft_brush::instance_art(&sym.art, c);
            art.transform(*xf, false);
            *c = Arc::new(art);
        }
    }
    e
}

#[test]
fn masks_in_symbols_move_with_the_art_when_linked() {
    for linked in [true, false] {
        let mut d = Document::new(200.0, 120.0);
        let fill = |c| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
        let mut art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0)), fill(Color::rgb8(200, 0, 0)));
        let shape = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 60.0)), fill(Color::WHITE));
        let mut mask = vectorcraft_doc::OpacityMask::new(shape, true);
        mask.linked = linked;
        art.mask = Some(Box::new(mask));
        d.symbols.push(Symbol { name: "Half".into(), art: Arc::new(art) });
        instance(&mut d, "Half", Affine::translate((10.0, 10.0)));
        instance(&mut d, "Half", Affine::translate((15.0, 50.0)));
        let svg = export(&d, &ExportOptions::default());
        assert_eq!(svg.contains("<use "), linked, "{svg}");
        // As the instances' art moved by the model's own transform.
        let want = export(&expanded(&d), &ExportOptions::default());
        assert_similar(&resvg_render(&want, 200, 120), &resvg_render(&svg, 200, 120), 8.0, 0.001);
    }
}

#[test]
fn symbols_inside_themselves_end() {
    let mut d = symbol_doc(1.0, &[Affine::translate((50.0, 50.0))]);
    let inner = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Loop".into(), xf: Affine::translate((5.0, 0.0)) });
    let mut g = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(inner)], clip: false });
    g.name = Some("g".into());
    d.symbols.push(Symbol { name: "Loop".into(), art: Arc::new(g) });
    instance(&mut d, "Loop", Affine::translate((20.0, 20.0)));
    let svg = export(&d, &ExportOptions::default());
    assert!(import(&svg).is_ok(), "{svg}");
}

/// Two layers, the second hidden, each with a square.
fn layered() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let fill = |c| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
    let a = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), fill(Color::rgb8(255, 0, 0)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, a).unwrap();
    let mut hidden = Node::layer(d.alloc_id(), "Notes", vectorcraft_doc::LayerColor::Preset(1));
    let b = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(50.0, 50.0, 90.0, 90.0)), fill(Color::rgb8(0, 0, 255)));
    hidden.children_mut().unwrap().push(Arc::new(b));
    hidden.visible = false;
    d.layers.push(Arc::new(hidden));
    d
}

#[test]
fn hidden_layers_are_kept_hidden_when_asked() {
    let d = layered();
    let plain = export(&d, &ExportOptions::default());
    assert!(!plain.contains("Notes") && plain.matches("<path").count() == 1, "{plain}");
    // Page isolation wraps the layers in one more group.
    let mut isolated = d.clone();
    isolated.page_isolate = true;
    let styles = [vectorcraft_svg::Styling::PresentationAttributes, vectorcraft_svg::Styling::InlineStyle, vectorcraft_svg::Styling::InternalCss];
    for (d, styling) in styles.iter().map(|s| (&d, *s)).chain([(&isolated, vectorcraft_svg::Styling::InternalCss)]) {
        let kept = export(d, &ExportOptions { hidden_layers: true, styling, ..Default::default() });
        assert!(kept.contains("id=\"Notes\"") && kept.contains("display") && kept.matches("<path").count() == 2, "{kept}");
        // Viewers don't show it.
        assert_eq!(resvg_render(&kept, 100, 100).pixel(70, 70), [255, 255, 255, 255], "{kept}");
        // It reopens as a hidden layer with its art.
        let back = import(&kept).unwrap();
        assert_eq!(back.layers.len(), 2, "{kept}");
        let notes = &back.layers[1];
        assert_eq!((notes.name.as_deref(), notes.visible), (Some("Notes"), false));
        assert_eq!(notes.children().unwrap().len(), 1);
        assert!(back.layers[0].visible);
    }
}

/// An animation rig (#522): hidden alternates (a group and a lone path) and rotation pivots kept as
/// `data-*` attributes.
const RIG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200" width="200" height="200">
  <g id="character" data-pivot="100,180">
    <g id="arm" data-pivot="60,100" data-z-order="2"><path d="M40 100 L60 90 L80 110 Z" fill="#b5835a"/></g>
    <g id="arm-raised" data-pivot="60,100" display="none"><path d="M40 100 L50 60 L70 70 Z" fill="#b5835a"/></g>
    <path id="eye-closed" d="M90 80 L110 80 L100 95 Z" fill="#000" style="display:none" data-blink="1"/>
  </g>
</svg>"##;

/// The `data-*` attributes of the object named `name`.
fn data_of(d: &Document, name: &str) -> Vec<(String, String)> {
    let mut found = vec![];
    d.walk(|n| {
        if n.name.as_deref() == Some(name) {
            found = n.attrs.as_deref().map(|a| a.data.clone()).unwrap_or_default();
        }
    });
    found
}

#[test]
fn hidden_objects_and_data_attributes_round_trip() {
    let d = import(RIG).unwrap();
    let pivot = |v: &str| vec![("pivot".to_string(), v.to_string())];
    assert_eq!(data_of(&d, "character"), pivot("100,180"));
    assert_eq!(data_of(&d, "arm"), [("pivot".to_string(), "60,100".to_string()), ("z-order".to_string(), "2".to_string())]);
    assert_eq!(data_of(&d, "arm-raised"), pivot("60,100"));
    let hidden = |d: &Document| {
        let mut v = vec![];
        d.walk(|n| {
            if !n.visible {
                v.push(n.name.clone().unwrap_or_default());
            }
        });
        v
    };
    assert_eq!(hidden(&d), ["arm-raised", "eye-closed"]);
    // An export leaves the hidden objects out, their data with them; the others keep theirs.
    let plain = export(&d, &ExportOptions::default());
    assert!(!plain.contains("arm-raised") && !plain.contains("eye-closed") && plain.contains("data-pivot=\"60,100\""), "{plain}");
    let styles = [vectorcraft_svg::Styling::PresentationAttributes, vectorcraft_svg::Styling::InlineStyle, vectorcraft_svg::Styling::InternalCss];
    for styling in styles {
        // Kept (Save, or asked), they are written hidden and come back hidden, with every attribute.
        let kept = export(&d, &ExportOptions { hidden_layers: true, styling, ..Default::default() });
        assert!(kept.contains("data-pivot=\"100,180\"") && kept.contains("data-z-order=\"2\"") && kept.contains("data-blink=\"1\""), "{kept}");
        assert_eq!(resvg_render(&kept, 200, 200).pixel(100, 85), [255, 255, 255, 255], "hidden, not drawn: {kept}");
        let back = import(&kept).unwrap();
        assert_eq!(hidden(&back), ["arm-raised", "eye-closed"], "{kept}");
        for name in ["character", "arm", "arm-raised", "eye-closed"] {
            assert_eq!(data_of(&back, name), data_of(&d, name), "{name}: {kept}");
        }
    }
}

/// Each layer's art: clipping layer or not, and the kinds of its children.
fn layer_art(d: &Document) -> Vec<(String, bool, Vec<&'static str>)> {
    d.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.clips(), l.children().unwrap().iter().map(|c| c.kind_label()).collect())).collect()
}

/// Two Inkscape layers clipped to the same window (#420), in pixels so resvg draws them at our scale.
const CLIPPED_LAYERS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="200" height="100" viewBox="0 0 200 100">
<defs><clipPath id="window"><rect x="5" y="5" width="190" height="90"/></clipPath></defs>
<g id="CUT" inkscape:groupmode="layer" inkscape:label="Cut lines" clip-path="url(#window)"><rect x="0" y="0" width="100" height="100" fill="#f00"/></g>
<g id="HATCH" inkscape:groupmode="layer" inkscape:label="HATCH" clip-path="url(#window)"><rect x="100" y="0" width="100" height="100" fill="#00f"/></g>
</svg>"##;

#[test]
fn clipped_layers_come_in_as_clipping_layers() {
    let d = import(CLIPPED_LAYERS).unwrap();
    assert_eq!(
        layer_art(&d),
        [("Cut lines".to_string(), true, vec!["Path", "Path"]), ("HATCH".to_string(), true, vec!["Path", "Path"])],
        "{:?}",
        d.layers
    );
    for l in &d.layers {
        let clip = &l.children().unwrap()[0];
        assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }) && clip.name.as_deref() == Some("window"));
        assert_eq!(clip.geometric_bounds().unwrap(), Rect::new(5.0, 5.0, 195.0, 95.0));
    }
    // Drawn as the SVG is: each layer cut to the window.
    assert_similar(&render_artboard(&d), &resvg_render(CLIPPED_LAYERS, 200, 100), 24.0, 0.002);
    // The issue's file (in points): two clipping layers, the names kept.
    let pt = CLIPPED_LAYERS.replace("width=\"200\" height=\"100\"", "width=\"200pt\" height=\"100pt\"");
    let d = import(&pt).unwrap();
    assert_eq!(d.layers.iter().map(|l| (l.name.as_deref().unwrap(), l.clips())).collect::<Vec<_>>(), [("Cut lines", true), ("HATCH", true)]);
}

#[test]
fn clipping_layers_round_trip() {
    let mut d = import(CLIPPED_LAYERS).unwrap();
    // A clip path clipped in turn: the layer's own clip outermost, the other one inside it.
    let nested = CLIPPED_LAYERS.replace(
        "<defs>",
        "<defs><clipPath id=\"left\"><rect x=\"0\" y=\"0\" width=\"60\" height=\"100\"/></clipPath><clipPath id=\"inner\" clip-path=\"url(#left)\"><rect x=\"20\" y=\"20\" width=\"160\" height=\"60\"/></clipPath>",
    );
    let nested = nested.replacen("url(#window)", "url(#inner)", 1);
    let n = import(&nested).unwrap();
    assert_eq!(layer_art(&n)[0], ("Cut lines".to_string(), true, vec!["Path", "Clip Group"]));
    assert_similar(&render_artboard(&n), &resvg_render(&nested, 200, 100), 24.0, 0.002);
    d.layers.push(n.layers[0].clone());
    let svg = export(&d, &ExportOptions::default());
    let back = import(&svg).unwrap();
    assert_eq!(layer_art(&back), layer_art(&d), "{svg}");
    assert_similar(&render_artboard(&back), &render_artboard(&d), 24.0, 0.002);
}

#[test]
fn text_joining_a_clipping_layer_stays_unclipped() {
    // Text beside clipped layers isn't in their clip: the layer keeps its clip as a clip group.
    let svg = CLIPPED_LAYERS
        .replace("<g id=\"CUT\"", "<text x=\"1\" y=\"99\">Before</text><g id=\"CUT\"")
        .replace("</svg>", "<text x=\"150\" y=\"99\">After</text></svg>");
    let d = import(&svg).unwrap();
    assert_eq!(
        layer_art(&d),
        [("Cut lines".to_string(), false, vec!["Type", "Clip Group"]), ("HATCH".to_string(), false, vec!["Clip Group", "Type"])],
        "{:?}",
        d.layers
    );
    for l in &d.layers {
        let g = l.children().unwrap().iter().find(|c| c.clips()).unwrap();
        assert_eq!(g.children().unwrap().iter().map(|c| c.kind_label()).collect::<Vec<_>>(), ["Path", "Path"]);
    }
}

#[test]
fn hidden_groups_others_use_stay_hidden() {
    // A display:none group a <use> refers to is a template, not a hidden layer.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">
        <g id="tpl" style="display:none"><rect width="10" height="10" fill="red"/></g>
        <use xlink:href="#tpl" x="50" y="50"/>
    </svg>"##;
    let d = import(svg).unwrap();
    let (mut paths, mut hidden) = (0, 0);
    d.walk(|n| {
        paths += usize::from(n.path_data().is_some());
        hidden += usize::from(!n.visible);
    });
    // Read as before: the template isn't shown (nor shown through the `<use>`).
    assert_eq!((paths, hidden), (0, 0), "{:?}", d.layers);
}

/// The layer tree: each object by kind (layers and groups with their names), `(hidden)` when
/// hidden, its contents indented below it.
fn outline(d: &Document) -> Vec<String> {
    fn walk(n: &Node, depth: usize, out: &mut Vec<String>) {
        let name = n.children().map(|_| format!(" {}", n.name.as_deref().unwrap_or("-"))).unwrap_or_default();
        let hidden = if n.visible { "" } else { " (hidden)" };
        out.push(format!("{}{}{name}{hidden}", "  ".repeat(depth), n.kind_label()));
        for c in n.children().into_iter().flatten() {
            walk(c, depth + 1, out);
        }
    }
    let mut out = vec![];
    for l in &d.layers {
        walk(l, 0, &mut out);
    }
    out
}

/// Layers as other apps export them (#372): named top-level groups, one hidden; a hidden named
/// group and a hidden object inside a layer.
const EXPORTED_LAYERS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <g id="Main">
    <rect id="plainStroke" x="20" y="50" width="120" height="60" style="fill:none;stroke:#000;stroke-width:4"/>
    <rect id="hiddenObject" x="200" y="50" width="60" height="60" style="display:none;fill:#fff;stroke:#000"/>
    <g id="HiddenSub" style="display:none"><circle cx="45" cy="175" r="25" style="fill:#fff;stroke:#000"/></g>
  </g>
  <g id="Second" display="none"><rect x="10" y="10" width="5" height="5"/></g>
</svg>"##;

#[test]
fn top_level_groups_become_named_layers() {
    let expected = ["Layer Main", "  Path", "  Path (hidden)", "  Group HiddenSub (hidden)", "    Path", "Layer Second (hidden)", "  Path"];
    let d = import(EXPORTED_LAYERS).unwrap();
    assert_eq!(outline(&d), expected);
    // A nested group stays a group: nothing in the file says it's a sublayer.
    // `data-name` names the layer when the id can't (spaces, punctuation).
    let named = EXPORTED_LAYERS.replace("<g id=\"Main\">", "<g id=\"Main_Art\" data-name=\"Main Art\">");
    assert_eq!(import(&named).unwrap().layers[0].name.as_deref(), Some("Main Art"));
    // Layers wrapped in more groups: a transform on all the art, or a `<switch>` choosing them.
    let moved = EXPORTED_LAYERS.replace("<g id=\"Main\">", "<g transform=\"translate(5 5)\"><g id=\"Main\">").replace("</svg>", "</g></svg>");
    let d = import(&moved).unwrap();
    assert_eq!(outline(&d), expected, "{moved}");
    let stroke = &d.layers[0].children().unwrap()[0];
    assert_eq!(stroke.geometric_bounds().unwrap(), Rect::new(25.0, 55.0, 145.0, 115.0));
    let switched = EXPORTED_LAYERS
        .replace("<g id=\"Main\">", "<switch><foreignObject requiredExtensions=\"urn:example:none\" width=\"1\" height=\"1\"/><g><g id=\"Main\">")
        .replace("</svg>", "</g></switch></svg>");
    assert_eq!(outline(&import(&switched).unwrap()), expected, "{switched}");
    // Art beside the groups puts everything in one layer, as before.
    let loose = EXPORTED_LAYERS.replace("</svg>", "<rect x=\"1\" y=\"1\" width=\"2\" height=\"2\"/></svg>");
    assert_eq!(import(&loose).unwrap().layers.iter().map(|l| l.name.as_deref()).collect::<Vec<_>>(), [Some("Layer 1")]);
}

/// Layer "Base" holding a path, a named group, a clipping sublayer and a hidden sublayer "Notes"
/// with a sublayer of its own; layer "Top" above it.
fn sublayered() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let square = |d: &mut Document, x: f64| {
        let ap = Appearance::basic(Paint::solid(Color::rgb8(200, 30, 30)), Paint::None, 0.0);
        Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, x, x + 10.0, x + 10.0)), ap))
    };
    let layer = |children: Vec<Arc<Node>>, d: &mut Document, name: &str| {
        let mut l = Node::layer(d.alloc_id(), name, vectorcraft_doc::LayerColor::Preset(3));
        *l.children_mut().unwrap() = children;
        l
    };
    let deep = layer(vec![square(&mut d, 60.0)], &mut d, "Deep");
    let mut notes = layer(vec![square(&mut d, 50.0), Arc::new(deep)], &mut d, "Notes");
    notes.visible = false;
    let mut clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 30.0, 30.0)), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let mut window = layer(vec![Arc::new(clip), square(&mut d, 25.0)], &mut d, "Window");
    window.set_clips(true);
    let mut group = Node::group(d.alloc_id(), vec![square(&mut d, 30.0), square(&mut d, 40.0)]);
    group.name = Some("Pair".into());
    let art = vec![square(&mut d, 10.0), Arc::new(group), Arc::new(window), Arc::new(notes)];
    let base = Arc::make_mut(&mut d.layers[0]);
    base.name = Some("Base".into());
    *base.children_mut().unwrap() = art;
    let top = layer(vec![square(&mut d, 80.0)], &mut d, "Top");
    d.layers.push(Arc::new(top));
    d
}

#[test]
fn sublayers_round_trip() {
    let d = sublayered();
    let expected = [
        "Layer Base",
        "  Path",
        "  Group Pair",
        "    Path",
        "    Path",
        "  Layer Window",
        "    Path",
        "    Path",
        "  Layer Notes (hidden)",
        "    Path",
        "    Layer Deep",
        "      Path",
        "Layer Top",
        "  Path",
    ];
    assert_eq!(outline(&d), expected);
    let styles = [vectorcraft_svg::Styling::PresentationAttributes, vectorcraft_svg::Styling::InlineStyle, vectorcraft_svg::Styling::InternalCss];
    for styling in styles {
        // Saved with the hidden sublayer: the same tree comes back.
        let kept = export(&d, &ExportOptions { hidden_layers: true, styling, ..Default::default() });
        assert_eq!(kept.matches("data-vc-layer").count(), 3, "sublayers only: {kept}");
        let back = import(&kept).unwrap();
        assert_eq!(outline(&back), expected, "{kept}");
        // A sublayer takes its layer's colour.
        back.walk(|n| {
            if let NodeKind::Layer { color, .. } = &n.kind
                && n.name.as_deref() != Some("Top")
            {
                assert_eq!(*color, vectorcraft_doc::LayerColor::Preset(0), "{:?}", n.name);
            }
        });
        assert!(back.layers[0].children().unwrap()[2].clips());
        assert_similar(&render_artboard(&back), &render_artboard(&d), 24.0, 0.002);
    }
    // Exported without hidden art, the visible sublayer still comes back as one.
    let plain = import(&export(&d, &ExportOptions::default())).unwrap();
    assert_eq!(
        outline(&plain),
        ["Layer Base", "  Path", "  Group Pair", "    Path", "    Path", "  Layer Window", "    Path", "    Path", "Layer Top", "  Path"]
    );
}

#[test]
fn a_root_element_names_the_one_layer() {
    // A one-layer document exported with its art loose under the root, which carries the name.
    let svg = r##"<svg version="1.1" id="Main" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300">
  <rect x="20" y="50" width="120" height="60" style="fill:none;stroke:#000;stroke-width:4"/>
  <g id="HiddenSub" style="display:none;"><circle cx="45" cy="175" r="25"/></g>
</svg>"##;
    assert_eq!(outline(&import(svg).unwrap()), ["Layer Main", "  Path", "  Group HiddenSub (hidden)", "    Path"]);
    // `data-name` when the id can't hold the name.
    let named = svg.replace("id=\"Main\"", "id=\"Main_Art\" data-name=\"Main Art\"");
    assert_eq!(import(&named).unwrap().layers[0].name.as_deref(), Some("Main Art"));
    let names = |svg: &str| import(svg).unwrap().layers.iter().map(|l| l.name.clone().unwrap()).collect::<Vec<_>>();
    // Not when the groups at the top are the layers.
    assert_eq!(names(&svg.replace("<rect", "<g id=\"Art\"><rect").replace("stroke-width:4\"/>", "stroke-width:4\"/></g>")), ["Art", "HiddenSub"]);
    // Nor from the id Inkscape gives its root.
    let inkscape = svg.replace("id=\"Main\"", "id=\"svg8\" xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" inkscape:version=\"1.3\"");
    assert_eq!(names(&inkscape), ["Layer 1"]);
}

#[test]
fn marked_groups_are_sublayers_only_in_layers() {
    // Inkscape marks its layers and sublayers; one marked group in a plain group stays a group.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="100" height="100">
  <g id="layer1" inkscape:groupmode="layer" inkscape:label="Base">
    <rect x="1" y="1" width="5" height="5"/>
    <g id="layer2" inkscape:groupmode="layer" inkscape:label="Sub" style="display:none">
      <rect x="10" y="10" width="5" height="5"/>
      <g id="layer3" inkscape:groupmode="layer" inkscape:label="Deeper"><rect x="20" y="10" width="5" height="5"/></g>
    </g>
    <g id="g1"><rect x="30" y="1" width="5" height="5"/><g id="layer4" data-vc-layer="sublayer"><rect x="40" y="1" width="5" height="5"/></g></g>
  </g>
</svg>"##;
    let d = import(svg).unwrap();
    assert_eq!(
        outline(&d),
        [
            "Layer Base",
            "  Path",
            "  Layer Sub (hidden)",
            "    Path",
            "    Layer Deeper",
            "      Path",
            "  Group g1",
            "    Path",
            "    Group layer4",
            "      Path",
        ]
    );
    // Art outside layers ("Layer 1" holds it): a marked group there is a sublayer of it.
    let loose = svg.replace("</svg>", "<rect x=\"1\" y=\"1\" width=\"2\" height=\"2\"/></svg>");
    let d = import(&loose).unwrap();
    assert_eq!(outline(&d)[..4], ["Layer Layer 1", "  Layer Base", "    Path", "    Layer Sub (hidden)"]);
}

#[test]
fn editing_data_notices_edits_made_elsewhere() {
    let d = symbol_doc(2.0, &[Affine::translate((50.0, 50.0))]);
    let opts = ExportOptions { preserve_editing: true, metadata: true, ..Default::default() };
    let svg = export_full(&d, &opts, Some(b"native bytes")).svg;
    assert!(svg.contains("<![CDATA[") && svg.contains(" hash=\""), "{svg}");
    let e = editing(&svg).unwrap();
    assert!(e.intact && e.data == vectorcraft_svg::base64_encode(b"native bytes"));
    // Re-indented, with other line endings: still the same picture.
    let reformatted = svg.replace("\n  ", "\r\n\t\t");
    assert!(editing(&reformatted).unwrap().intact);
    let minified = export_full(&d, &ExportOptions { minify: true, ..opts.clone() }, Some(b"native bytes")).svg;
    assert!(editing(&minified).unwrap().intact);
    // A colour changed elsewhere.
    let edited = svg.replacen("#1e5adc", "#ff0000", 1);
    assert_ne!(edited, svg);
    assert!(!editing(&edited).unwrap().intact);
    // Metadata edits don't count: only the markup around the <metadata> element does.
    let retitled = svg.replacen("<dc:format>", "<dc:creator>someone</dc:creator><dc:format>", 1);
    assert!(editing(&retitled).unwrap().intact);
    // Unique ids hash the final output.
    let unique = export_full(&d, &ExportOptions { object_ids: vectorcraft_svg::ObjectIds::Unique, ..opts }, Some(b"native bytes")).svg;
    assert!(editing(&unique).unwrap().intact);
}
