//! Save PDF › Create PDF layers (an optional content group per top-level layer, with its
//! visibility, print state and lock) and Advanced › Overprint (a graphics state that overprints
//! for the overprinting fills and strokes).

use craft::pdf::syntax::object::{Array, Dict, Name, ObjectIdentifier};
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, LayerColor, Node, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};

use crate::*;

const RED: Color = Color::Rgb { r: 1.0, g: 0.0, b: 0.0 };
const GREEN: Color = Color::Rgb { r: 0.0, g: 1.0, b: 0.0 };
const BLUE: Color = Color::Rgb { r: 0.0, g: 0.0, b: 1.0 };

fn rect(d: &mut Document, x: f64, c: Color) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 10.0, x + 30.0, 40.0)), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

/// Layers, bottom first: "Back" (red), "Hidden" (green; hidden and locked), "Notes" (blue; not
/// printed) and a template layer (grey).
fn layered() -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.layers.clear();
    for (i, (name, c)) in [("Back", RED), ("Hidden", GREEN), ("Notes", BLUE), ("Template", Color::gray(0.5))].into_iter().enumerate() {
        let art = rect(&mut d, 10.0 + 40.0 * i as f64, c);
        let mut l = Node::layer(d.alloc_id(), name, LayerColor::Preset(i as u8));
        (l.visible, l.locked) = (name != "Hidden", name == "Hidden");
        if let NodeKind::Layer { children, printable, template, .. } = &mut l.kind {
            children.push(art.into());
            *printable = name != "Notes";
            *template = name == "Template";
        }
        d.layers.push(l.into());
    }
    d
}

fn export(d: &Document, v: Value) -> ExportReport {
    let settings: PdfSettings = serde_json::from_value(v).unwrap();
    export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap()
}

/// The groups of `refs` (an array of the catalog's optional content properties) by name.
fn names(xref: &craft::pdf::syntax::xref::XRef, refs: Option<Array<'_>>) -> Vec<String> {
    let ids = refs.map(|a| a.raw_iter().filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).collect::<Vec<_>>()).unwrap_or_default();
    ids.into_iter()
        .map(|id| {
            let g = xref.get::<Dict<'_>>(id).unwrap();
            String::from_utf8(g.get::<craft::pdf::syntax::object::String<'_>>(b"Name").unwrap().as_bytes().to_vec()).unwrap()
        })
        .collect()
}

/// The catalog's `/OCProperties` of `bytes` (opened with `password`), if any.
fn oc_properties(bytes: &[u8], password: Option<&str>, f: impl FnOnce(Option<Dict<'_>>, &craft::pdf::syntax::xref::XRef)) {
    let file = crate::pages::open(bytes, password).unwrap();
    let xref = file.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap();
    f(catalog.get::<Dict<'_>>(b"OCProperties"), xref);
}

/// The layers of an import: name, visible, printable, locked and fill colours of their art.
fn layers(d: &Document) -> Vec<(String, bool, bool, bool, Vec<Color>)> {
    d.layers
        .iter()
        .map(|l| {
            let printable = matches!(l.kind, NodeKind::Layer { printable: true, .. });
            let colors = l.children().unwrap().iter().filter_map(|n| n.appearance.fill().and_then(|f| f.paint.color())).collect();
            (l.name.clone().unwrap_or_default(), l.visible, printable, l.locked, colors)
        })
        .collect()
}

#[test]
fn create_layers_writes_a_group_per_layer_with_its_states() {
    let d = layered();
    let r = export(&d, json!({"createLayers": true, "compression": {"compressText": false}}));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    oc_properties(&r.bytes, None, |props, xref| {
        let props = props.expect("/OCProperties");
        // Template layers aren't exported; the groups are in paint order, listed top first.
        assert_eq!(names(xref, props.get(b"OCGs")), ["Back", "Hidden", "Notes"]);
        let config = props.get::<Dict<'_>>(b"D").unwrap();
        assert_eq!(names(xref, config.get(b"Order")), ["Notes", "Hidden", "Back"]);
        assert_eq!(names(xref, config.get(b"OFF")), ["Hidden"], "hidden layers are off");
        assert_eq!(names(xref, config.get(b"Locked")), ["Hidden"]);
        assert!(config.get::<craft::pdf::syntax::object::String<'_>>(b"Name").is_some());
        let auto = config.get::<Array<'_>>(b"AS").unwrap().iter::<Dict<'_>>().next().unwrap();
        assert_eq!(auto.get::<Name<'_>>(b"Event").unwrap().as_ref(), b"Print");
        let print_state = |name: &str| {
            let ids =
                props.get::<Array<'_>>(b"OCGs").unwrap().raw_iter().filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).collect::<Vec<_>>();
            let g = ids
                .into_iter()
                .map(|id| xref.get::<Dict<'_>>(id).unwrap())
                .find(|g| g.get::<craft::pdf::syntax::object::String<'_>>(b"Name").unwrap().as_bytes() == name.as_bytes());
            let usage = g.unwrap().get::<Dict<'_>>(b"Usage").unwrap();
            usage.get::<Dict<'_>>(b"Print").unwrap().get::<Name<'_>>(b"PrintState").unwrap().as_ref().to_vec()
        };
        assert_eq!(print_state("Notes"), b"OFF", "non-printing layers aren't printed");
        assert_eq!(print_state("Back"), b"ON");
    });
    // Each layer's art is its group's optional content.
    let text = String::from_utf8_lossy(&r.bytes);
    assert_eq!(text.matches(" BDC ").count(), 3, "{text}");
    assert!(text.contains("/OC /VCoc0 BDC /x") && text.contains("/Properties<<"));
}

#[test]
fn pdf_layers_import_back_as_layers() {
    let d = layered();
    for password in [None, Some("secret")] {
        let mut set = json!({"createLayers": true});
        if let Some(p) = password {
            set["security"] = json!({"openPassword": p});
        }
        let bytes = export(&d, set).bytes;
        let opts = ImportOptions { password: password.map(str::to_string), ..Default::default() };
        let back = import_with_report(&bytes, &opts).unwrap().document;
        assert_eq!(
            layers(&back),
            [
                ("Back".to_string(), true, true, false, vec![RED]),
                ("Hidden".to_string(), false, true, true, vec![GREEN]),
                ("Notes".to_string(), true, false, false, vec![BLUE]),
            ],
            "password {password:?}"
        );
    }
}

/// A hidden layer written as a PDF layer places the live effects of its instances' symbol art,
/// also in a PDF/A-2b file at PDF 1.4 whose 17,000 pt page has it written as PDF 1.6: the symbol's
/// square from x = 10 to 40, which a Transform effect moves 100 pt right, is written from x = 110
/// to 140.
#[test]
fn a_hidden_layer_places_symbol_art_effects_when_a_long_page_raises_the_version() {
    let mut d = Document::new(17_000.0, 300.0);
    d.layers.clear();
    let mut square = rect(&mut d, 10.0, RED);
    square.appearance.effects.push(vectorcraft_doc::Effect { id: "distort.transform".into(), params: json!({"moveH": 100.0}), visible: true });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Moved".into(), art: square.into() });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Moved".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let mut l = Node::layer(d.alloc_id(), "Hidden", LayerColor::Preset(0));
    l.visible = false;
    if let NodeKind::Layer { children, .. } = &mut l.kind {
        children.push(instance.into());
    }
    d.layers.push(l.into());
    for compatibility in [Compatibility::Pdf14, Compatibility::Pdf16] {
        let settings = PdfSettings { standard: Standard::PdfA2b, compatibility, create_layers: true, ..PdfSettings::default() };
        let r = export_with_report(&d, &PdfOptions { settings, created: Some(1_791_200_000), ..Default::default() }).unwrap();
        assert!(r.bytes.starts_with(b"%PDF-1.6"), "{compatibility:?}");
        let back = import_with_report(&r.bytes, &ImportOptions::default()).unwrap().document;
        let hidden = back.layers.iter().find(|l| l.name.as_deref() == Some("Hidden")).expect("the hidden layer");
        let art = hidden.children().unwrap();
        assert_eq!(art.len(), 1, "{compatibility:?}");
        let b = art[0].geometric_bounds().unwrap();
        assert!((b.x0 - 110.0).abs() < 0.1 && (b.x1 - 140.0).abs() < 0.1, "{compatibility:?}: {b:?}");
    }
}

/// The rows of `nodes` in paint order, depth first: `depth name visible` for layers, `depth fill`
/// for art.
fn tree(nodes: &[std::sync::Arc<Node>], depth: usize, out: &mut Vec<String>) {
    for n in nodes {
        match &n.kind {
            NodeKind::Layer { children, .. } => {
                out.push(format!("{depth} {} {}", n.name.clone().unwrap_or_default(), n.visible));
                tree(children, depth + 1, out);
            }
            _ => out.push(format!("{depth} {:?}", n.appearance.fill().and_then(|f| f.paint.color()))),
        }
    }
}

/// Sublayers are PDF layers listed under their layer's, hidden ones off with their art, and they
/// import back as sublayers (#372: other apps read a `.ai` file's PDF part).
#[test]
fn sublayers_are_nested_pdf_layers_and_import_back() {
    let mut d = Document::new(200.0, 100.0);
    d.layers.clear();
    let layer = |d: &mut Document, name: &str, visible: bool, children: Vec<Node>| {
        let mut l = Node::layer(d.alloc_id(), name, LayerColor::Preset(0));
        l.visible = visible;
        if let NodeKind::Layer { children: c, .. } = &mut l.kind {
            c.extend(children.into_iter().map(std::sync::Arc::new));
        }
        l
    };
    let deep = vec![rect(&mut d, 90.0, GREEN)];
    let deep = layer(&mut d, "Deep", true, deep);
    let sub = vec![rect(&mut d, 50.0, BLUE), deep];
    let sub = layer(&mut d, "Sub", false, sub);
    let main = vec![rect(&mut d, 10.0, RED), sub];
    let main = layer(&mut d, "Main", true, main);
    d.layers.push(main.into());
    let r = export(&d, json!({"createLayers": true, "compression": {"compressText": false}}));
    oc_properties(&r.bytes, None, |props, xref| {
        let props = props.expect("/OCProperties");
        assert_eq!(names(xref, props.get(b"OCGs")), ["Main", "Sub", "Deep"]);
        assert_eq!(names(xref, props.get::<Dict<'_>>(b"D").unwrap().get(b"OFF")), ["Sub"]);
    });
    let text = String::from_utf8_lossy(&r.bytes);
    let order = text.split("/Order").nth(1).and_then(|o| o.split("]/").next()).unwrap().to_string();
    assert_eq!(order.matches('[').count(), 3, "each layer's sublayers in an array after it: {order}");
    let mut got = vec![];
    tree(&import(&r.bytes).unwrap().layers, 0, &mut got);
    // Paint order, as it was.
    let fill = |depth, c: Color| format!("{depth} {:?}", Some(c));
    assert_eq!(got, ["0 Main true".to_string(), fill(1, RED), "1 Sub false".into(), fill(2, BLUE), "2 Deep true".into(), fill(3, GREEN)]);
}

#[test]
fn identical_layers_stay_apart() {
    let mut d = Document::new(200.0, 100.0);
    d.layers.clear();
    for name in ["One", "Two"] {
        let art = rect(&mut d, 10.0, RED);
        let mut l = Node::layer(d.alloc_id(), name, LayerColor::Preset(0));
        if let NodeKind::Layer { children, .. } = &mut l.kind {
            children.push(art.into());
        }
        d.layers.push(l.into());
    }
    let back = import(&export(&d, json!({"createLayers": true})).bytes).unwrap();
    let got: Vec<(String, usize)> = back.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.children().unwrap().len())).collect();
    assert_eq!(got, [("One".to_string(), 1), ("Two".to_string(), 1)]);
}

#[test]
fn without_pdf_layers_the_layers_are_plain_page_content() {
    let mut d = layered();
    for (v, warning) in [
        (json!({}), None),
        (json!({"createLayers": true, "compatibility": "1.4"}), Some("1.5")),
        (json!({"createLayers": true, "compatibility": "1.5"}), None),
    ] {
        let r = export(&d, v.clone());
        let mut has = false;
        oc_properties(&r.bytes, None, |props, _| has = props.is_some());
        assert_eq!(has, v["createLayers"] == json!(true) && warning.is_none(), "{v}");
        match warning {
            Some(w) => assert!(r.setting_notes.iter().any(|x| x.contains(w)), "{v}: {:?}", r.setting_notes),
            None => assert!(r.warnings.is_empty() && r.setting_notes.is_empty(), "{v}: {:?} {:?}", r.warnings, r.setting_notes),
        }
    }
    // A knockout page group draws objects, not layers.
    d.page_knockout = true;
    let r = export(&d, json!({"createLayers": true}));
    oc_properties(&r.bytes, None, |props, _| assert!(props.is_none()));
    assert!(r.setting_notes.iter().any(|w| w.contains("knockout page group")), "{:?}", r.setting_notes);
}

#[test]
fn pdf_a_layers_have_no_automatic_states() {
    let r = export(&layered(), json!({"standard": "pdfA2b", "createLayers": true}));
    oc_properties(&r.bytes, None, |props, _| {
        let config = props.unwrap().get::<Dict<'_>>(b"D").unwrap();
        assert!(config.get::<Array<'_>>(b"AS").is_none() && config.get::<craft::pdf::syntax::object::String<'_>>(b"Name").is_some());
    });
}

#[test]
fn pdf_x4_keeps_pdf_layers_and_x1a_refuses_them() {
    let d = layered();
    let r = export(&d, json!({"standard": "pdfX4", "compatibility": "1.6", "createLayers": true}));
    assert!(r.bytes.starts_with(b"%PDF-1.6"));
    oc_properties(&r.bytes, None, |props, xref| {
        let props = props.expect("/OCProperties");
        assert_eq!(names(xref, props.get(b"OCGs")), ["Back", "Hidden", "Notes"]);
        assert!(props.get::<Dict<'_>>(b"D").unwrap().get::<Array<'_>>(b"AS").is_none());
    });
    let set: PdfSettings = serde_json::from_value(json!({"standard": "pdfX1a", "compatibility": "1.4", "createLayers": true})).unwrap();
    assert!(matches!(export_with_report(&d, &PdfOptions { settings: set, ..Default::default() }), Err(PdfError::BadSetting(_))));
}

/// The painted paths of `d`: their bounds' corner and colour, in order.
fn leaves(d: &Document) -> Vec<(i32, i32, Option<[u8; 4]>)> {
    let mut out = vec![];
    d.walk(|n| {
        if let Some(b) = n.path_data().and_then(|p| p.bounds()) {
            let paint = n.appearance.fill().map(|f| f.paint.clone()).or_else(|| n.appearance.stroke().map(|s| s.paint.clone()));
            out.push((b.x0.round() as i32, b.y0.round() as i32, paint.and_then(|p| p.color()).map(|c| c.to_rgba8(1.0))));
        }
    });
    out.sort_by_key(|l| (l.0, l.1));
    out
}

#[test]
fn layered_art_imports_as_it_was_drawn() {
    let mut d = layered();
    d.layers = d.layers.iter().map(|l| Node { visible: true, ..(**l).clone() }.into()).collect();
    let plain = leaves(&import(&export(&d, json!({"includeNonPrinting": true})).bytes).unwrap());
    assert_eq!(plain.len(), 3);
    assert_eq!(leaves(&import(&export(&d, json!({"createLayers": true})).bytes).unwrap()), plain, "the marks draw nothing");
}

/// A rectangle whose fill overprints, one whose stroke does, a plain one and type whose
/// characters' fill overprints, filled with `fill` (the overprinting rectangle).
fn overprinting(fill: Color) -> Document {
    let mut d = Document::new(300.0, 100.0);
    let l = d.layers[0].id;
    let mut a = rect(&mut d, 10.0, fill);
    if let Some(f) = a.appearance.fill_mut() {
        f.overprint = true;
    }
    let mut b =
        Node::path(d.alloc_id(), shapes::rectangle(Rect::new(50.0, 10.0, 80.0, 40.0)), Appearance::basic(Paint::None, Paint::solid(BLUE), 2.0));
    if let Some(s) = b.appearance.stroke_mut() {
        s.overprint = true;
    }
    let c = rect(&mut d, 90.0, GREEN);
    let style = CharStyle { size: 24.0, fill: Paint::solid(Color::BLACK), overprint_fill: true, ..Default::default() };
    let t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(130.0, 40.0), "Ink", style))));
    for (i, n) in [a, b, c, t].into_iter().enumerate() {
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// The uncompressed export of `d` with `v`: the overprinting graphics states it defines, and the
/// forms that set one.
fn overprints(d: &Document, v: Value) -> (usize, usize) {
    let mut v = v;
    v["compression"] = json!({"compressText": false});
    let r = export(d, v);
    assert!(r.warnings.iter().all(|w| !w.contains("overprint")), "{:?}", r.warnings);
    let text = String::from_utf8_lossy(&r.bytes).into_owned();
    (text.matches("/Type/ExtGState/OP true/op true/OPM 1").count(), text.matches("stream\n/VCop gs\n").count())
}

#[test]
fn overprint_preserve_writes_an_overprinting_graphics_state() {
    let d = overprinting(RED);
    assert_eq!(overprints(&d, json!({})), (1, 3), "the fill, the stroke and the characters' fill");
    assert_eq!(overprints(&d, json!({"advanced": {"overprint": "discard"}})), (0, 0));
    assert_eq!(overprints(&overprinting(RED), json!({"createLayers": true})), (1, 3), "with PDF layers too");
    // Nothing overprints: no graphics state.
    assert_eq!(overprints(&layered(), json!({})), (0, 0));
}

#[test]
fn white_overprint_is_discarded_as_the_document_setup_says() {
    let mut d = overprinting(Color::WHITE);
    assert!(d.setup.discard_white_overprint);
    assert_eq!(overprints(&d, json!({})).1, 2, "the white fill knocks out");
    d.setup.discard_white_overprint = false;
    assert_eq!(overprints(&d, json!({})).1, 3);
}

#[test]
fn a_white_spot_ink_keeps_overprinting_under_discard_white_overprint() {
    let white = Color::cmyk(0.0, 0.0, 0.0, 0.0);
    let mut d = overprinting(white);
    d.swatches.push(vectorcraft_color::Swatch { name: "Opaque White".into(), paint: Paint::solid(white), global: true, spot: true });
    let paint = |d: &mut Document, p: &Paint| {
        for n in std::sync::Arc::make_mut(&mut d.layers[0]).children_mut().into_iter().flatten() {
            let n = std::sync::Arc::make_mut(n);
            if let Some(f) = n.appearance.fill_mut().filter(|f| f.overprint) {
                f.paint = p.clone();
            }
            if let NodeKind::Text(t) = &mut n.kind {
                t.runs.iter_mut().for_each(|r| r.style.fill = p.clone());
            }
        }
    };
    paint(&mut d, &Paint::solid(white));
    assert_eq!(overprints(&d, json!({})).1, 1, "paper white fills and characters knock out");
    paint(&mut d, &Paint::Solid { color: white, swatch: Some("Opaque White".into()), tint: 1.0 });
    assert_eq!(overprints(&d, json!({})).1, 3, "a white spot ink overprints, characters too");
}

#[test]
fn pdf_a_overprint_mode_is_0() {
    let r = export(&overprinting(RED), json!({"standard": "pdfA2b", "compression": {"compressText": false}}));
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(text.contains("/OP true/op true/OPM 0") && !text.contains("/OPM 1"));
}

#[test]
fn overprinting_art_imports_as_it_was_drawn() {
    let d = overprinting(RED);
    // What is drawn (a form's bounds may come back as an unpainted clipping path).
    let painted = |bytes: &[u8]| leaves(&import(bytes).unwrap()).into_iter().filter(|l| l.2.is_some()).collect::<Vec<_>>();
    let plain = painted(&export(&d, json!({"advanced": {"overprint": "discard"}})).bytes);
    for v in [json!({}), json!({"createLayers": true})] {
        assert_eq!(painted(&export(&d, v.clone()).bytes), plain, "{v}: the marks draw nothing");
    }
}

/// The `/BBox` of each form of the uncompressed `bytes`, in the order written.
fn form_boxes(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.split("/Subtype/Form").skip(1).filter_map(|d| Some(d.split("/BBox[").nth(1)?.split(']').next()?.to_string())).collect()
}

/// #2181: a layer's form has a box round the paper and its art, not one 23 m across, and no
/// form (a layer's, an overprinting one or invisible text's) keeps the empty clip far off the
/// page that marked it while the file was written.
#[test]
fn layer_forms_have_real_boxes_and_no_marks() {
    let mut d = overprinting(RED);
    // Invisible text (neither fill nor stroke) and art across the paper's left edge.
    let l = d.layers[0].id;
    let style = CharStyle { size: 24.0, fill: Paint::None, ..Default::default() };
    let t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(200.0, 40.0), "Hid", style))));
    let past = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(-50.0, 10.0, 5.0, 40.0)), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    d.insert(Some(l), 0, t).unwrap();
    d.insert(Some(l), 0, past).unwrap();
    let mut sub = Node::layer(d.alloc_id(), "Sub", LayerColor::Preset(1));
    if let NodeKind::Layer { children, .. } = &mut sub.kind {
        children.push(rect(&mut d, 250.0, BLUE).into());
    }
    d.insert(Some(l), 0, sub).unwrap();
    let r = export(&d, json!({"createLayers": true, "compression": {"compressText": false}}));
    let text = String::from_utf8_lossy(&r.bytes);
    for mark in ["-32000 ", "-32001 ", "-32002 ", "32767"] {
        assert!(!text.contains(mark), "{mark} is in the file: {text}");
    }
    assert!(text.contains("3 Tr") && text.contains("/VCop gs"), "invisible text and overprint are still written: {text}");
    // The layer's form reaches the art across the left edge; its sublayer's covers the paper.
    let boxes = form_boxes(&r.bytes);
    assert!(boxes.contains(&"-50 0 300 100".to_string()) && boxes.contains(&"0 0 300 100".to_string()), "{boxes:?}");
    // It still imports as it was drawn: a layer holding its sublayer, no clipping group.
    let mut got = vec![];
    tree(&import(&r.bytes).unwrap().layers, 0, &mut got);
    assert!(got.contains(&"1 Sub true".to_string()) && got.contains(&format!("2 {:?}", Some(BLUE))), "{got:?}");
}

#[test]
fn pdf_x1a_keeps_overprint() {
    let r = export(&overprinting(RED), json!({"standard": "pdfX1a", "compatibility": "1.4", "compression": {"compressText": false}}));
    assert!(r.bytes.starts_with(b"%PDF-1.3") && String::from_utf8_lossy(&r.bytes).contains("/OP true/op true/OPM 1"));
}

/// A PDF with a correct cross-reference table from `bodies` (each an object's contents, numbered
/// from 1), its trailer naming object 1 as the catalog.
fn handmade(bodies: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for (i, b) in bodies.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{b}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).bytes());
    out
}

/// #6299: a layer's form whose mark gives no paper (its mark rewritten without the paper corners
/// [`layer_form`] writes) still gets a real box — its art's box, ordered — never the page-max
/// box that inflated the document bounds on import.
#[test]
fn a_layer_form_without_paper_gets_a_real_box() {
    // The mark without its paper corners: the layer index reads, the paper doesn't.
    let art = "q -32000 0 m h W n Q 0 0 1 rg 10 10 90 90 re f";
    let page = "/Fm0 Do";
    let pdf = handmade(&[
        "<< /Type/Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type/Pages /Kids[3 0 R] /Count 1 >>".to_string(),
        "<< /Type/Page /Parent 2 0 R /MediaBox[0 0 612 792] /Contents 4 0 R /Resources<</XObject<</Fm0 5 0 R>>>> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{page}\nendstream", page.len()),
        format!("<< /Type/XObject /Subtype/Form /BBox[10 10 100 100] /Length {} >>\nstream\n{art}\nendstream", art.len()),
    ]);
    let doc = Document::new(612.0, 792.0);
    let set: PdfSettings = serde_json::from_value(json!({})).unwrap();
    let out = crate::forms::finish(pdf, &doc, &set, true, false, false).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("32767"), "no page-max box: {text}");
    assert_eq!(form_boxes(&out), ["10 10 100 100"], "the art's box: {text}");
}

/// #6299: every layer form's box stays round the page and the art — with an empty layer, art
/// across the page's edge and a sublayer — so no form's box is page-max or past the page and the
/// art on import.
#[test]
fn layer_forms_stay_round_the_page_and_the_art() {
    let mut d = Document::new(200.0, 100.0);
    d.layers.clear();
    let art = |d: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, c: Color| {
        Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x0, y0, x1, y1)), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
    };
    let layer = |d: &mut Document, name: &str, children: Vec<Node>| {
        let mut l = Node::layer(d.alloc_id(), name, LayerColor::Preset(0));
        if let NodeKind::Layer { children: c, .. } = &mut l.kind {
            c.extend(children.into_iter().map(std::sync::Arc::new));
        }
        l
    };
    let sub = {
        let a = art(&mut d, 90.0, 10.0, 120.0, 40.0, Color::gray(0.5));
        layer(&mut d, "Sub", vec![a])
    };
    let main = {
        let a = art(&mut d, 50.0, 10.0, 80.0, 40.0, BLUE);
        layer(&mut d, "Main", vec![a, sub])
    };
    let back = {
        let a = art(&mut d, 10.0, 10.0, 40.0, 40.0, RED);
        layer(&mut d, "Back", vec![a])
    };
    let empty = layer(&mut d, "Empty", vec![]);
    let off = {
        let a = art(&mut d, 180.0, 80.0, 260.0, 160.0, GREEN);
        layer(&mut d, "Off", vec![a])
    };
    d.layers.push(back.into());
    d.layers.push(empty.into());
    d.layers.push(off.into());
    d.layers.push(main.into());
    let r = export(&d, json!({"createLayers": true, "compression": {"compressText": false}}));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let boxes = form_boxes(&r.bytes);
    assert_eq!(boxes.len(), 5, "one form per layer and sublayer: {boxes:?}");
    // The boxes live in their forms' space (the art drawn through the page's y-flip); mapped
    // back to the page they must stay round the page and the art, in whole units.
    for b in &boxes {
        let v: Vec<f64> = b.split_whitespace().map(|v| v.parse().unwrap()).collect();
        assert_eq!(v.len(), 4, "{b}");
        assert!(v.iter().all(|v| v.is_finite()) && v[0] <= v[2] && v[1] <= v[3], "{b}");
        assert!(v.iter().all(|x| x.abs() < 10_000.0), "no page-max box: {b}");
        let page = [v[0], 100.0 - v[3], v[2], 100.0 - v[1]];
        assert!(page[0] >= -0.5 && page[1] >= -0.5 && page[2] <= 260.5 && page[3] <= 160.5, "round the page and the art: {b}");
    }
    assert!(boxes.contains(&"0 -60 260 100".to_string()), "the art across the edge is covered: {boxes:?}");
}
