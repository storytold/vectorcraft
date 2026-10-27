//! File → Document Setup (`document.setup`): validation, one undo step, round-trips, and the
//! options other commands read (typographer's quotes, superscript/subscript, SVG text export).

use serde_json::{Value, json};
use vectorcraft_doc::{CharPosition, DocSetup, ExportText, GridSize, NodeId, NodeKind, ScriptMetrics, TextObject, TextStyleDef, Unit};
use vectorcraft_tools::{PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn setup(s: &Session) -> &DocSetup {
    &s.doc().unwrap().doc.setup
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn create_text(s: &mut Session, t: &str) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": t, "size": 20})).unwrap()["id"].as_u64().unwrap())
}

/// Click with the Type tool at (x, y) and type `keys` one at a time; returns the new text object.
fn type_at(s: &mut Session, x: f64, y: f64, keys: &[&str]) -> NodeId {
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    for k in keys {
        s.tool_text(k, v).unwrap();
    }
    s.tool_key(ToolKey::Escape, Default::default(), v).unwrap();
    id
}

#[test]
fn query_reports_the_defaults_and_accepts_them_back() {
    let mut s = session();
    let q = s.execute("document.setup", &json!({})).unwrap();
    assert_eq!(q["units"], "Points");
    assert_eq!(q["bleed"], json!([0.0, 0.0, 0.0, 0.0]));
    assert_eq!(q["gridColors"], json!(["#ffffff", "#cccccc"]));
    assert_eq!(q["gridColorsName"], "Light");
    assert_eq!(q["flattenerPreset"], "Medium Resolution");
    assert_eq!(q["backgroundContents"], "transparent");
    assert_eq!(q["quotes"], json!({"double": "“”", "single": "‘’"}));
    assert_eq!(q["typographersQuotes"], true);
    assert_eq!(q["superscript"], json!({"size": 58.3, "position": 33.3}));
    // Feeding the report back changes nothing (and records no undo step).
    assert_eq!(s.execute("document.setup", &q).unwrap(), q);
    assert!(s.doc().unwrap().history.undo.is_empty());
    assert_eq!(*setup(&s), DocSetup::default());
}

#[test]
fn many_settings_are_one_undo_step() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    let r = s
        .execute(
            "document.setup",
            &json!({
                "units": "Millimeters", "bleed": {"top": 9, "right": 4.5}, "gridSize": "large", "gridColors": "Blue",
                "simulatePaper": true, "flattenerPreset": "high", "discardWhiteOverprint": false,
                "outlineImages": true, "highlightSubstitutedGlyphs": true, "exportText": "appearance",
                "superscript": {"size": 60}, "subscript": {"position": 20}, "smallCapsSize": 75,
            }),
        )
        .unwrap();
    assert_eq!(r["gridColorsName"], "Blue");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.units, Unit::Millimeters);
    let st = &d.setup;
    assert_eq!(st.bleed, [9.0, 0.0, 0.0, 4.5]);
    assert_eq!((st.grid_size, st.simulate_paper, st.outline_images, st.highlight_substituted_glyphs), (GridSize::Large, true, true, true));
    assert_eq!((st.flattener(), st.discard_white_overprint, st.export_text), ("High Resolution", false, ExportText::Appearance));
    assert_eq!((st.superscript, st.subscript), (ScriptMetrics { size: 60.0, position: 33.3 }, ScriptMetrics { size: 58.3, position: 20.0 }));
    assert_eq!(st.small_caps_size, 75.0);
    let undo: Vec<&str> = s.doc().unwrap().history.undo.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(undo, ["Document Setup"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
}

#[test]
fn bad_values_are_rejected_and_change_nothing() {
    let mut s = session();
    for p in [
        json!({"bleed": 73}),
        json!({"bleed": [1, 2, 3]}),
        json!({"bleed": {"middle": 2}}),
        json!({"bleed": -1}),
        json!({"gridSize": "huge"}),
        json!({"gridColors": ["#ffffff"]}),
        json!({"gridColors": "Plaid"}),
        json!({"language": "Klingon"}),
        json!({"quotes": {"double": "“"}}),
        json!({"quotes": {"triple": "“”"}}),
        json!({"superscript": {"size": 0}}),
        json!({"exportText": "pdf"}),
        json!({"flattenerPreset": "Ultra"}),
        json!({"backgroundContents": "grey"}),
        json!({"simulatePaper": "yes"}),
        json!({"units": "furlongs"}),
        json!({"bleed": 9, "nonsense": true}),
        json!([1, 2]),
    ] {
        assert!(s.execute("document.setup", &p).is_err(), "{p}");
    }
    assert_eq!(*setup(&s), DocSetup::default());
    assert!(s.doc().unwrap().history.undo.is_empty());
}

#[test]
fn set_units_is_an_alias() {
    let mut s = session();
    s.execute("document.setUnits", &json!({"units": "inches"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.units, Unit::Inches);
    assert_eq!(s.execute("document.setup", &json!({})).unwrap()["units"], "Inches");
    assert!(s.execute("document.setUnits", &json!({"units": "parsecs"})).is_err());
}

#[test]
fn language_picks_its_quotes_unless_quotes_are_given() {
    let mut s = session();
    s.execute("document.setup", &json!({"language": "german"})).unwrap();
    assert_eq!(setup(&s).language, "German");
    assert_eq!(setup(&s).quotes.double, ['„', '“']);
    s.execute("document.setup", &json!({"language": "French", "quotes": {"double": ["“", "”"]}})).unwrap();
    assert_eq!((setup(&s).quotes.double, setup(&s).quotes.single), (['“', '”'], ['‹', '›']));
}

#[test]
fn typographers_quotes_setting_changes_typed_quotes() {
    let mut s = session();
    let id = type_at(&mut s, 50.0, 50.0, &["\"", "H", "i", "\"", " ", "i", "t", "'", "s"]);
    assert_eq!(text(&s, id).plain_text(), "“Hi” it’s");
    // The document's quote style.
    s.execute("document.setup", &json!({"language": "German"})).unwrap();
    let id = type_at(&mut s, 50.0, 150.0, &["\"", "a", "\""]);
    assert_eq!(text(&s, id).plain_text(), "„a“");
    // Straight quotes with the option off.
    s.execute("document.setup", &json!({"typographersQuotes": false})).unwrap();
    let id = type_at(&mut s, 50.0, 250.0, &["\"", "b", "'"]);
    assert_eq!(text(&s, id).plain_text(), "\"b'");
}

#[test]
fn smart_punctuation_uses_the_document_quotes() {
    let mut s = session();
    s.execute("document.setup", &json!({"language": "French"})).unwrap();
    let id = create_text(&mut s, "\"oui\" 'non'");
    s.execute("type.smartPunctuation", &json!({"scope": "document"})).unwrap();
    assert_eq!(text(&s, id).plain_text(), "«oui» ‹non›");
}

#[test]
fn superscript_and_small_caps_use_and_follow_the_setup_sizes() {
    let mut s = session();
    let id = create_text(&mut s, "E=mc2");
    s.execute("document.setup", &json!({"superscript": {"size": 50, "position": 40}})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 4, "end": 5, "position": "superscript"})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [id.0], "smallCaps": true})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.runs.last().unwrap().style.position, CharPosition::Superscript(ScriptMetrics { size: 50.0, position: 40.0 }));
    assert!(t.runs.iter().all(|r| r.style.small_caps == Some(70.0)));
    let narrow = t.cached_bounds.unwrap();
    // A Document Setup change restyles the text in the same undo step.
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("document.setup", &json!({"superscript": {"size": 100}, "smallCapsSize": 100})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let t = text(&s, id);
    assert_eq!(t.runs.last().unwrap().style.position, CharPosition::Superscript(ScriptMetrics { size: 100.0, position: 40.0 }));
    assert!(t.runs.iter().all(|r| r.style.small_caps == Some(100.0)));
    assert!(t.cached_bounds.unwrap().width() > narrow.width(), "bounds follow the bigger glyphs");
    s.execute("text.setFormat", &json!({"ids": [id.0], "position": "normal", "smallCaps": false})).unwrap();
    assert!(text(&s, id).runs.iter().all(|r| r.style.position == CharPosition::Normal && r.style.small_caps.is_none()));
    assert!(s.execute("text.setFormat", &json!({"ids": [id.0], "position": "sideways"})).is_err());
}

#[test]
fn character_styles_follow_the_setup_sizes() {
    let mut s = session();
    let attrs = json!({"position": {"kind": "subscript", "size": 58.3, "position": 33.3}, "small_caps": 70.0});
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.push(TextStyleDef { name: "Note".into(), attrs: attrs.as_object().unwrap().clone() });
    s.execute("document.setup", &json!({"subscript": {"size": 40}, "smallCapsSize": 80})).unwrap();
    let def = &s.doc().unwrap().doc.char_styles[0];
    assert_eq!(def.attrs["position"], json!({"kind": "subscript", "size": 40.0, "position": 33.3}));
    assert_eq!(def.attrs["small_caps"], json!(80.0));
}

/// A file whose type, symbols, graph label styles, character styles or pattern definition saved in
/// pattern editing mode hold superscript, subscript or small caps proportions outside the ranges
/// `document.setup` takes opens with each at the nearer end of its range; the other character
/// attributes open as saved.
#[test]
fn a_file_with_script_proportions_out_of_range_opens_with_them_in_range() {
    let mut s = session();
    let id = create_text(&mut s, "E=mc2");
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 4, "end": 5, "position": "superscript"})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [id.0], "smallCaps": true})).unwrap();
    let water = create_text(&mut s, "H2O");
    s.execute("text.setRangeStyle", &json!({"id": water.0, "start": 1, "end": 2, "position": "subscript"})).unwrap();
    let csv = ",2024\nQ1,3\nQ2,5";
    let graph = s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 150, "width": 200, "height": 100, "csv": csv})).unwrap();
    let graph = NodeId(graph["id"].as_u64().unwrap());
    let exponent = create_text(&mut s, "x2");
    s.execute("text.setRangeStyle", &json!({"id": exponent.0, "start": 1, "end": 2, "position": "superscript"})).unwrap();
    s.execute("select.set", &json!({"ids": [exponent.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Exponent", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    // Saved in pattern editing mode: Cancel puts back the definition from before the edit.
    s.execute("object.pattern.edit", &json!({"name": "Exponent"})).unwrap();
    let mut d = (*s.doc().unwrap().doc).clone();
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.runs[0].style.small_caps = Some(-5.0);
        let two = &mut t.runs[1].style;
        two.position = CharPosition::Superscript(ScriptMetrics { size: 0.0, position: f64::MAX });
        two.small_caps = Some(f64::MAX);
    }
    if let Some(NodeKind::Text(t)) = d.node_mut(water).map(|n| &mut n.kind) {
        t.runs[1].style.position = CharPosition::Subscript(ScriptMetrics { size: f64::MAX, position: -1000.0 });
        t.runs[1].style.tracking = -5000.0;
    }
    let mut art = d.node(id).unwrap().clone();
    art.id = NodeId(9000);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Formula".into(), art: Arc::new(art) });
    if let Some(g) = d.node_mut(graph).and_then(|n| n.extras_if_any()).and_then(|e| e.graph.as_deref_mut()) {
        let position = CharPosition::Subscript(ScriptMetrics { size: 0.0, position: 1000.0 });
        g.label_styles.legend = Some(vectorcraft_doc::CharStyle { position, small_caps: Some(0.0), ..vectorcraft_doc::CharStyle::default() });
    }
    let original = d.pattern_edit.as_mut().and_then(|pe| pe.original.as_mut()).and_then(|p| p.art.first_mut());
    if let Some(NodeKind::Text(t)) = original.map(|a| &mut Arc::make_mut(a).kind) {
        t.runs[1].style.position = CharPosition::Superscript(ScriptMetrics { size: 0.0, position: f64::MAX });
    }
    let attrs = json!({"position": {"kind": "superscript", "size": 1000.0, "position": -1000.0}, "small_caps": 0.0, "tracking": -5000.0});
    d.char_styles.push(TextStyleDef { name: "Note".into(), attrs: attrs.as_object().unwrap().clone() });
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "scripts.vectorcraft", "dataBase64": data})).unwrap();
    let raised = CharPosition::Superscript(ScriptMetrics { size: 1.0, position: 500.0 });
    let t = text(&s, id);
    assert_eq!((t.runs[0].style.small_caps, t.runs[1].style.small_caps), (Some(1.0), Some(200.0)));
    assert_eq!(t.runs[1].style.position, raised);
    let t = text(&s, water);
    assert_eq!(t.runs[1].style.position, CharPosition::Subscript(ScriptMetrics { size: 500.0, position: -500.0 }));
    assert_eq!(t.runs[1].style.tracking, -5000.0, "other attributes open as saved");
    let doc = &s.doc().unwrap().doc;
    match &doc.symbols.iter().find(|s| s.name == "Formula").unwrap().art.kind {
        NodeKind::Text(t) => assert_eq!(t.runs[1].style.position, raised),
        _ => panic!("the symbol's art is not text"),
    }
    let legend = doc.node(graph).unwrap().extras().graph.as_ref().unwrap().label_styles.legend.clone().unwrap();
    assert_eq!((legend.position, legend.small_caps), (CharPosition::Subscript(ScriptMetrics { size: 1.0, position: 500.0 }), Some(1.0)));
    let def = doc.char_styles.iter().find(|d| d.name == "Note").unwrap();
    assert_eq!(def.attrs["position"], json!({"kind": "superscript", "size": 500.0, "position": -500.0}));
    assert_eq!((&def.attrs["small_caps"], &def.attrs["tracking"]), (&json!(1.0), &json!(-5000.0)));
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    match &s.doc().unwrap().doc.pattern("Exponent").unwrap().art[0].kind {
        NodeKind::Text(t) => assert_eq!(t.runs[1].style.position, raised),
        _ => panic!("the pattern's art is not text"),
    }
    // With the file's proportions kept as saved, the move failed with "result would exceed the canvas".
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
}

#[test]
fn setup_survives_save_and_load() {
    let mut s = session();
    s.execute(
        "document.setup",
        &json!({"bleed": [9, 9, 18, 18], "gridColors": ["#ff0000", "#00ff00"], "language": "Japanese", "exportText": "appearance"}),
    )
    .unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(d)).unwrap();
    assert_eq!(back.setup, d.setup);
    assert_eq!(back.setup.grid_colors_name(), "Custom");
}

/// Highlight Substituted Fonts is a preference (Preferences › Type): `document.setup` does not
/// report it, and ignores it with a warning that names `prefs.set`, so an older build's reply
/// passed back still applies its other keys. The document's stored value and the preference stay.
#[test]
fn highlight_substituted_fonts_is_ignored_with_a_pointer_to_the_preference() {
    let mut s = session();
    let q = s.execute("document.setup", &json!({})).unwrap();
    assert!(q.get("highlightSubstitutedFonts").is_none() && q.get("warnings").is_none(), "{q}");
    let r = s.execute("document.setup", &json!({"highlightSubstitutedFonts": false, "bleed": 9})).unwrap();
    let w = r["warnings"][0].as_str().unwrap_or_default();
    assert!(w.contains("prefs.set") && w.contains("highlightSubstitutedFonts"), "{r}");
    assert_eq!(setup(&s).bleed, [9.0; 4]);
    assert!(setup(&s).highlight_substituted_fonts, "the stored value is not changed");
    assert!(s.prefs.highlight_substituted_fonts, "nor is the preference");
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    // The reply, warning included, can be passed back; alone, the key changes nothing.
    let mut plain = r.clone();
    plain.as_object_mut().unwrap().remove("warnings");
    assert_eq!(s.execute("document.setup", &r).unwrap(), plain);
    assert_eq!(s.execute("document.setup", &json!({"highlightSubstitutedFonts": true})).unwrap()["warnings"], r["warnings"]);
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    let e = s.execute("document.setup", &json!({"highlightSubstitutedFonts": 1})).unwrap_err().to_string();
    assert!(e.contains("true or false"), "{e}");
}

/// A file saved with Document Setup's old Highlight Substituted Fonts value opens, keeps that
/// value when saved again, and leaves the preference as it was.
#[test]
fn a_file_with_the_old_highlight_substituted_fonts_value_opens_and_keeps_it() {
    let mut s = session();
    let mut d = (*s.doc().unwrap().doc).clone();
    d.setup.highlight_substituted_fonts = false;
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "old.vectorcraft", "dataBase64": data})).unwrap();
    assert!(!setup(&s).highlight_substituted_fonts);
    assert!(s.prefs.highlight_substituted_fonts, "the preference decides the screen");
    let again = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
    assert!(!again.setup.highlight_substituted_fonts);
    let q = s.execute("document.setup", &json!({})).unwrap();
    assert_eq!(s.execute("document.setup", &q).unwrap(), q);
}

/// A file whose bleed, superscript, subscript or small caps size lies outside the range
/// `document.setup` takes opens with it at the nearer end of that range, and the setup it reports
/// can be fed back.
#[test]
fn a_file_with_setup_values_out_of_range_opens_with_them_in_range() {
    let mut s = session();
    let mut d = (*s.doc().unwrap().doc).clone();
    d.setup.bleed = [-5.0, 100.0, f64::MAX, 9.0];
    d.setup.superscript = ScriptMetrics { size: 0.0, position: f64::MAX };
    d.setup.subscript = ScriptMetrics { size: f64::MAX, position: -1000.0 };
    d.setup.small_caps_size = -5.0;
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "setup.vectorcraft", "dataBase64": data})).unwrap();
    let st = setup(&s);
    assert_eq!(st.bleed, [0.0, 72.0, 72.0, 9.0]);
    assert_eq!(st.superscript, ScriptMetrics { size: 1.0, position: 500.0 });
    assert_eq!(st.subscript, ScriptMetrics { size: 500.0, position: -500.0 });
    assert_eq!(st.small_caps_size, 1.0);
    let q = s.execute("document.setup", &json!({})).unwrap();
    assert_eq!(s.execute("document.setup", &q).unwrap(), q);
}

#[test]
fn svg_text_export_follows_the_export_text_setting() {
    let mut s = session();
    create_text(&mut s, "Hello");
    let svg = |s: &mut Session, p: Value| {
        let mut p = p;
        p["format"] = json!("svg");
        let r = s.execute("document.export", &p).unwrap();
        String::from_utf8(vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()).unwrap()
    };
    assert!(svg(&mut s, json!({})).contains("<text"));
    s.execute("document.setup", &json!({"exportText": "appearance"})).unwrap();
    assert!(!svg(&mut s, json!({})).contains("<text"));
    // An explicit option still wins.
    assert!(svg(&mut s, json!({"outlineText": false})).contains("<text"));
}

#[test]
fn discard_white_overprint_keeps_white_overprints_in_overprint_preview() {
    let mut s = session();
    let rect = |s: &mut Session, color: &str| {
        let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [id], "color": color})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
        id
    };
    rect(&mut s, "#ff0000");
    let white = rect(&mut s, "#ffffff");
    s.execute("object.setOverprint", &json!({"fill": true, "ids": [white]})).unwrap();
    let centre = |s: &Session| {
        let opts = vectorcraft_render::RenderOptions { background: Some([255; 4]), overprint_preview: true, ..Default::default() };
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 40, 40, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(20, 20)
    };
    // On by default: the white knocks out as it will print.
    assert_eq!(centre(&s), [255, 255, 255, 255]);
    s.execute("document.setup", &json!({"discardWhiteOverprint": false})).unwrap();
    let [r, g, b, _] = centre(&s);
    assert!(r > 200 && g < 60 && b < 60, "a white overprint vanishes: {:?}", [r, g, b]);
}

#[test]
fn saved_flattener_presets_and_background_contents() {
    let mut s = session();
    s.execute("flattener.presets.save", &json!({"name": "Proofs", "preset": "high"})).unwrap();
    assert_eq!(s.execute("document.setup", &json!({"flattenerPreset": "proofs"})).unwrap()["flattenerPreset"], "Proofs");
    assert_eq!(s.execute("document.setup", &json!({"flattenerPreset": "Medium Resolution"})).unwrap()["flattenerPreset"], "Medium Resolution");
    assert_eq!(setup(&s).flattener_preset, None, "the default is not stored");
    let r = s.execute("document.setup", &json!({"backgroundContents": "white"})).unwrap();
    assert_eq!(r["backgroundContents"], "white");
    assert_eq!(setup(&s).background, vectorcraft_doc::Background::White);
}
