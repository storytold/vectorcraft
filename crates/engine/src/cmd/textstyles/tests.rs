use super::*;

#[test]
fn captured_character_style_excludes_a_one_character_glyph_choice() {
    let style = CharStyle {
        glyph_alternate: Some(vectorcraft_doc::GlyphChoice { ch: 'a', gid: 42, font: "Test-Regular".into(), version: "1".into() }),
        ..Default::default()
    };
    assert!(!char_attrs(&style).contains_key("glyphAlternate"));
    assert!(style.glyph_alternate.is_some(), "the selected text keeps its direct glyph choice");
}

#[test]
fn character_style_edit_keeps_a_linked_runs_glyph_choice() {
    let (mut s, id) = session_with_text("Styled");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    let choice = vectorcraft_doc::GlyphChoice { ch: 'S', gid: 42, font: "Test-Regular".into(), version: "1".into() };
    if let Some(NodeKind::Text(t)) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(NodeId(id)).map(|n| &mut n.kind) {
        t.runs[0].style.glyph_alternate = Some(choice.clone());
    }
    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {"size": 18}})).unwrap();
    let run = &runs(&s, id)[0];
    assert_eq!((run.style.style_name.as_deref(), run.style.size), (Some("Body"), 18.0));
    assert_eq!(run.style.glyph_alternate, Some(choice), "the style edit keeps the run's own glyph choice");
}

/// A glyph choice given to a character style is an error, not a value the style drops.
#[test]
fn character_style_attributes_refuse_a_glyph_choice() {
    let (mut s, _) = session_with_text("Styled");
    let choice = json!({"ch": "S", "gid": 42, "font": "Test-Regular", "version": "1"});
    let err = s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24, "glyphAlternate": choice}})).unwrap_err();
    assert!(err.to_string().contains("glyphAlternate"), "{err}");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
    let err = s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {"glyphAlternate": choice}})).unwrap_err();
    assert!(err.to_string().contains("glyphAlternate"), "{err}");
}

fn session_with_text(text: &str) -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": text})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    (s, id)
}

fn runs(s: &Session, id: u64) -> Vec<vectorcraft_doc::TextRun> {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!(),
    }
}

fn embedded_style(s: &Session, container: &str) -> (Option<String>, f64, Option<String>) {
    let d = &s.doc().unwrap().doc;
    let brushes = vectorcraft_brush::library(d);
    let roots: Vec<&Node> = match container {
        "symbol" => d.symbols.iter().map(|s| &*s.art).collect(),
        "pattern" => d.patterns.iter().flat_map(|p| p.art.iter().map(|a| &**a)).collect(),
        "graph" => d.graph_designs.iter().map(|g| &*g.art).collect(),
        "brush" => brushes.iter().flat_map(|b| b.art()).collect(),
        _ => panic!("unknown container"),
    };
    let mut found = None;
    for root in roots {
        vectorcraft_doc::swatches::walk_with_masks(root, &mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                found = Some((t.runs[0].style.style_name.clone(), t.runs[0].style.size, t.para.style_name.clone()));
            }
        });
    }
    found.unwrap()
}

fn embedded_style_session(kind: &str, container: &str) -> Session {
    let (mut s, id) = session_with_text("Styled");
    s.execute(&format!("{kind}Style.new"), &json!({"name": "Body", "attrs": if kind == "char" { json!({"size": 24}) } else { json!({}) }})).unwrap();
    s.execute(&format!("{kind}Style.apply"), &json!({"name": "Body", "id": id})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    match container {
        "symbol" => {
            let symbol = s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
            s.execute("edit.clear", &json!({"ids": [symbol["id"]]})).unwrap();
        }
        "pattern" => {
            s.execute("object.pattern.make", &json!({"edit": false})).unwrap();
            if s.doc().unwrap().doc.node(NodeId(id)).is_some() {
                s.execute("edit.clear", &json!({"ids": [id]})).unwrap();
            }
        }
        "graph" => {
            s.execute("graph.design", &json!({"new": "Bar"})).unwrap();
            s.execute("edit.clear", &json!({"ids": [id]})).unwrap();
        }
        "brush" => {
            s.execute("brush.new", &json!({"type": "art", "name": "Lettering"})).unwrap();
            s.execute("edit.clear", &json!({"ids": [id]})).unwrap();
        }
        _ => panic!("unknown container"),
    }
    s
}

#[test]
fn character_style_options_update_embedded_art() {
    for container in ["symbol", "pattern", "graph", "brush"] {
        let mut s = embedded_style_session("char", container);
        assert_eq!(embedded_style(&s, container).0.as_deref(), Some("Body"), "{container}");
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("charStyle.edit", &json!({"name": "Body", "to": "Heading", "attrs": {"size": 18}})).unwrap();
        assert_eq!(embedded_style(&s, container).0.as_deref(), Some("Heading"), "{container}");
        assert_eq!(embedded_style(&s, container).1, 18.0, "{container}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{container}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before, "{container}");
        s.execute("edit.redo", &json!({})).unwrap();
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("charStyle.delete", &json!({"name": "Heading"})).unwrap();
        assert_eq!(embedded_style(&s, container).0, None, "{container}");
        assert_eq!(embedded_style(&s, container).1, 18.0, "{container}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{container}");
    }
}

#[test]
fn character_style_edit_survives_canceling_pattern_edit() {
    let mut s = embedded_style_session("char", "pattern");
    let name = s.doc().unwrap().doc.patterns[0].name.clone();
    s.execute("object.pattern.edit", &json!({"name": name})).unwrap();
    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Heading", "attrs": {"size": 18}})).unwrap();
    s.execute("object.pattern.cancel", &json!({})).unwrap();

    assert_eq!(embedded_style(&s, "pattern").0.as_deref(), Some("Heading"));
    assert_eq!(embedded_style(&s, "pattern").1, 18.0);
}

#[test]
fn paragraph_style_rename_updates_symbol_art() {
    let mut s = embedded_style_session("para", "symbol");
    assert_eq!(embedded_style(&s, "symbol").2.as_deref(), Some("Body"));
    s.execute("paraStyle.rename", &json!({"name": "Body", "to": "Heading"})).unwrap();
    assert_eq!(embedded_style(&s, "symbol").2.as_deref(), Some("Heading"));
}

#[test]
fn character_style_options_keep_text_free_layer_shared() {
    let (mut s, id) = session_with_text("Styled");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    let empty_id = s.execute("layer.new", &json!({"name": "Empty"})).unwrap()["id"].as_u64().unwrap();
    let before = s.doc().unwrap().doc.layers.iter().find(|layer| layer.id == NodeId(empty_id)).unwrap().clone();

    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Heading", "attrs": {"size": 18}})).unwrap();

    let d = &s.doc().unwrap().doc;
    let after = d.layers.iter().find(|layer| layer.id == NodeId(empty_id)).unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, after));
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
    assert_eq!(runs(&s, id)[0].style.size, 18.0);
}

#[test]
fn unchanged_style_options_keep_undo_and_redo_with_and_without_preview() {
    for preview in [false, true] {
        let (mut s, _) = session_with_text("Text");
        s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 18, "leading": null}})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Other", "attrs": {}})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let redo = s.doc().unwrap().history.redo.len();
        assert!(redo > 0);
        let params = json!({"name": "Body", "to": " Body ", "attrs": {"size": 18.0, "leading": null}});
        if preview {
            s.begin_interaction("Character Style Options").unwrap();
            s.preview("charStyle.edit", &params).unwrap();
            s.commit_interaction().unwrap();
        } else {
            s.execute("charStyle.edit", &params).unwrap();
        }
        assert!(std::sync::Arc::ptr_eq(&before, &s.doc().unwrap().doc), "preview={preview}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "preview={preview}");
        assert_eq!(s.doc().unwrap().history.redo.len(), redo, "preview={preview}");
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.char_styles.iter().any(|style| style.name == "Other"));
    }
}

#[test]
fn unchanged_style_attributes_keep_undo_and_redo_for_both_kinds() {
    for (command, para) in [("charStyle", false), ("paraStyle", true)] {
        let (mut s, _) = session_with_text("Text");
        let attrs = if para { json!({"space_before": 18}) } else { json!({"size": 18}) };
        s.execute(&format!("{command}.new"), &json!({"name": "Body", "attrs": attrs})).unwrap();
        s.execute(&format!("{command}.new"), &json!({"name": "Other", "attrs": {}})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let redo = s.doc().unwrap().history.redo.len();
        assert!(redo > 0);
        let attrs = if para { json!({"space_before": 18.0}) } else { json!({"size": 18.0}) };
        s.execute(&format!("{command}.setAttrs"), &json!({"name": "Body", "attrs": attrs})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &s.doc().unwrap().doc), "{command}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{command}");
        assert_eq!(s.doc().unwrap().history.redo.len(), redo, "{command}");
    }
}

#[test]
fn character_style_options_edit_applies_to_users_and_is_one_saved_undo_step() {
    let (mut s, id) = session_with_text("First second");
    s.execute("charStyle.new", &json!({"name":"Body", "attrs":{"size":24.0,"tracking":20.0,"baseline_shift":3.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name":"Body", "id":id, "start":0, "end":5})).unwrap();
    s.execute("charStyle.apply", &json!({"name":"Body", "id":id, "start":6, "end":12})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id":id,"start":6,"end":12,"size":31.0})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("charStyle.edit", &json!({"name":"Body","to":"Text","attrs":{"size":18.0,"tracking":40.0,"baseline_shift":3.0}})).unwrap();
    let changed = s.doc().unwrap().doc.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(
        runs(&s, id).iter().map(|r| (r.style.style_name.as_deref(), r.style.size, r.style.tracking)).collect::<Vec<_>>(),
        [(Some("Text"), 18.0, 40.0), (None, 12.0, 0.0), (Some("Text"), 31.0, 40.0)]
    );
    let future = s.execute("text.create", &json!({"x":20,"y":90,"text":""})).unwrap()["id"].as_u64().unwrap();
    s.execute("charStyle.apply", &json!({"name":"Text","id":future})).unwrap();
    s.execute("text.editRange", &json!({"id":future,"start":0,"end":0,"insert":"Later"})).unwrap();
    assert_eq!((runs(&s, future)[0].style.size, runs(&s, future)[0].style.tracking), (18.0, 40.0));
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(
        saved.char_styles.iter().find(|d| d.name == "Text").unwrap().attrs,
        s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Text").unwrap().attrs
    );
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, changed);
}

#[test]
fn character_style_options_edit_sets_the_advanced_character_formats() {
    let (mut s, id) = session_with_text("First second");
    s.execute("charStyle.new", &json!({"name":"Body", "attrs":{"h_scale":100.0,"rotation":0.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name":"Body", "id":id})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id":id,"start":6,"end":12,"rotation":30.0})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let attrs = json!({"h_scale":80.0,"v_scale":120.0,"baseline_shift":-2.0,"rotation":15.0,"language":"German"});
    s.execute("charStyle.edit", &json!({"name":"Body","to":"Body","attrs":attrs})).unwrap();
    let changed = s.doc().unwrap().doc.clone();
    let def = changed.char_styles.iter().find(|d| d.name == "Body").unwrap();
    assert_eq!(def.attrs["language"], json!("de"));
    assert_eq!(
        runs(&s, id)
            .iter()
            .map(|r| (r.style.h_scale, r.style.v_scale, r.style.baseline_shift, r.style.rotation, r.style.language.clone()))
            .collect::<Vec<_>>(),
        [(80.0, 120.0, -2.0, 15.0, Some("de".to_string())), (80.0, 120.0, -2.0, 30.0, Some("de".to_string()))]
    );
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&changed, false)).unwrap();
    assert_eq!(saved.char_styles.iter().find(|d| d.name == "Body").unwrap().attrs, def.attrs);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, changed);
}

#[test]
fn character_style_options_edit_keeps_a_language_tag_from_a_file() {
    let (mut s, _) = session_with_text("Text");
    s.execute("charStyle.new", &json!({"name":"Body", "attrs":{}})).unwrap();
    if let Some(def) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.iter_mut().find(|d| d.name == "Body") {
        def.attrs.insert("language".into(), json!("x-file"));
    }
    s.execute("charStyle.edit", &json!({"name":"Body","to":"Body","attrs":{"size":18.0,"language":"x-file"}})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Body").unwrap().attrs["language"], json!("x-file"));
    let error = s.execute("charStyle.edit", &json!({"name":"Body","to":"Body","attrs":{"language":"x-other"}})).unwrap_err().to_string();
    assert!(error.contains("unknown language"), "{error}");
}

#[test]
fn new_character_style_keeps_a_language_tag_from_the_text() {
    let (mut s, id) = session_with_text("Text");
    if let Some(NodeKind::Text(t)) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(NodeId(id)).map(|n| &mut n.kind) {
        t.runs[0].style.language = Some("x-file".into());
    }
    s.execute("charStyle.new", &json!({"name":"Body","id":id,"attrs":{"size":18.0,"language":"x-file"}})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Body").unwrap().attrs["language"], json!("x-file"));
    let error = s.execute("charStyle.new", &json!({"name":"Other","id":id,"attrs":{"language":"x-other"}})).unwrap_err().to_string();
    assert!(error.contains("unknown language"), "{error}");
}

/// The fill of the first run of `id` and its overprint flags, its stroke and stroke weight.
fn run_paints(s: &Session, id: u64) -> (vectorcraft_color::Paint, vectorcraft_color::Paint, f64, bool, bool) {
    let st = runs(s, id)[0].style.clone();
    (st.fill, st.stroke, st.stroke_width, st.overprint_fill, st.overprint_stroke)
}

#[test]
fn character_style_color_and_features_reach_text_and_follow_swatch_edits() {
    let (mut s, id) = session_with_text("Text 1/2");
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#2050a0", "global": true})).unwrap();
    let brand = s.doc().unwrap().doc.tint_paint("Brand", 1.0).unwrap();
    let mut tinted = serde_json::to_value(s.doc().unwrap().doc.tint_paint("Brand", 1.0).unwrap()).unwrap();
    // The command takes the color of the link at the tint, whatever color is sent.
    tinted["tint"] = json!(0.4);
    let attrs = json!({"fill": brand, "stroke": tinted, "stroke_width": 2.0, "overprint_fill": true, "overprint_stroke": false,
        "features": ["dlig", "-liga", "tnum", "lnum", "-onum", "-pnum", "ss02", "-ss01"]});
    s.execute("charStyle.new", &json!({"name": "Ink", "attrs": attrs})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Ink", "id": id})).unwrap();
    let want_stroke = s.doc().unwrap().doc.tint_paint("Brand", 0.4).unwrap();
    assert_eq!(run_paints(&s, id), (brand, want_stroke.clone(), 2.0, true, false));
    assert_eq!(runs(&s, id)[0].style.features, ["dlig", "-liga", "tnum", "lnum", "-onum", "-pnum", "ss02", "-ss01"]);
    let def = |s: &Session| s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Ink").unwrap().attrs.clone();
    assert_eq!(def(&s)["stroke"], serde_json::to_value(&want_stroke).unwrap());
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(saved.char_styles.iter().find(|d| d.name == "Ink").unwrap().attrs, def(&s));

    s.execute("swatch.edit", &json!({"name": "Brand", "color": "#a02050"})).unwrap();
    let (brand, tint40) = (s.doc().unwrap().doc.tint_paint("Brand", 1.0).unwrap(), s.doc().unwrap().doc.tint_paint("Brand", 0.4).unwrap());
    assert_eq!(run_paints(&s, id).0, brand);
    assert_eq!(run_paints(&s, id).1, tint40);
    assert_eq!(def(&s)["fill"], serde_json::to_value(&brand).unwrap());
    assert_eq!(def(&s)["stroke"], serde_json::to_value(&tint40).unwrap());
    let later = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Later"})).unwrap()["id"].as_u64().unwrap();
    s.execute("charStyle.apply", &json!({"name": "Ink", "id": later})).unwrap();
    assert_eq!(run_paints(&s, later).0, brand);
}

#[test]
fn character_style_color_and_features_are_checked() {
    let (mut s, _) = session_with_text("Text");
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#2050a0", "global": true})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {}})).unwrap();
    let mut over = serde_json::to_value(s.doc().unwrap().doc.tint_paint("Brand", 1.0).unwrap()).unwrap();
    over["tint"] = json!(1.5);
    let mut missing = over.clone();
    missing["tint"] = json!(0.5);
    missing["swatch"] = json!("Gone");
    s.execute("swatch.new", &json!({"name": "Local", "color": "#102030"})).unwrap();
    let mut local = missing.clone();
    local["swatch"] = json!("Local");
    let many: Vec<String> = (0..=vectorcraft_text::MAX_STYLE_FEATURES).map(|_| "dlig".to_string()).collect();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for (attrs, message) in [
        (json!({"fill": over}), "`fill.tint` must be from 0 to 1"),
        (json!({"stroke": missing}), "no swatch `Gone`"),
        (json!({"stroke": local}), "`Local` is a process colour"),
        (json!({"stroke_width": -1.0}), "`stroke_width` must be from 0 to 4000000"),
        (json!({"stroke_width": 5.0e6}), "`stroke_width` must be from 0 to 4000000"),
        (json!({"features": ["dlig", "zzzz"]}), "unknown OpenType feature `zzzz`"),
        (json!({"features": ["--liga"]}), "unknown OpenType feature `--liga`"),
        (json!({"features": many}), "features lists at most 256 tags"),
        (json!({"fill": {"type": "solid"}}), "missing field"),
    ] {
        for cmd in ["charStyle.edit", "charStyle.setAttrs", "charStyle.new"] {
            let p = match cmd {
                "charStyle.edit" => json!({"name": "Body", "to": "Body", "attrs": attrs}),
                "charStyle.new" => json!({"name": "Other", "attrs": attrs}),
                _ => json!({"name": "Body", "attrs": attrs}),
            };
            let error = s.execute(cmd, &p).unwrap_err().to_string();
            assert!(error.contains(message), "{cmd} {attrs}: {error}");
        }
    }
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn character_style_dimensions_are_checked_before_a_definition_changes() {
    let (mut s, _) = session_with_text("Text");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for attrs in [
        json!({"size": 0.0}),
        json!({"size": 1296.1}),
        json!({"leading": 0.0}),
        json!({"leading": 5000.1}),
        json!({"h_scale": 0.0}),
        json!({"h_scale": 0.9}),
        json!({"h_scale": 10000.1}),
        json!({"v_scale": 0.0}),
        json!({"v_scale": 0.9}),
        json!({"v_scale": 10000.1}),
        json!({"baseline_shift": -1296.1}),
        json!({"baseline_shift": 1296.1}),
        json!({"tcyUpDown": 1296.1}),
        json!({"tcyLeftRight": -1296.1}),
    ] {
        let key = attrs.as_object().unwrap().keys().next().unwrap();
        for cmd in ["charStyle.edit", "charStyle.setAttrs", "charStyle.new"] {
            let params = match cmd {
                "charStyle.edit" => json!({"name": "Body", "to": "Body", "attrs": attrs}),
                "charStyle.new" => json!({"name": "Other", "attrs": attrs}),
                _ => json!({"name": "Body", "attrs": attrs}),
            };
            let error = s.execute(cmd, &params).unwrap_err().to_string();
            assert!(error.contains(&format!("`{key}`")), "{cmd} {attrs}: {error}");
        }
    }
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);

    s.execute(
        "charStyle.setAttrs",
        &json!({"name": "Body", "attrs": {
            "size": 1296.0, "leading": 5000.0,
            "h_scale": 1.0, "v_scale": 10000.0,
            "baseline_shift": -1296.0,
            "tcyUpDown": 1296.0, "tcyLeftRight": -1296.0
        }}),
    )
    .unwrap();
}

#[test]
fn text_style_rejects_doubly_signed_feature_tags() {
    let (mut s, id) = session_with_text("Text");
    let before = runs(&s, id);
    let undo = s.doc().unwrap().history.undo.len();
    for cmd in ["text.setStyle", "text.setRangeStyle"] {
        for tag in ["--liga", "+-dnom", "-+ss01", "++liga"] {
            let error = s.execute(cmd, &json!({"id": id, "start": 0, "end": 4, "features": [tag]})).unwrap_err().to_string();
            assert!(error.contains(&format!("`{tag}`")), "{cmd}: {error}");
        }
    }
    assert_eq!(runs(&s, id), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn style_paints_count_as_swatch_uses_and_gradient_stops_are_checked() {
    let (mut s, _) = session_with_text("Text");
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#2050a0", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Spare", "color": "#a02050", "global": true})).unwrap();
    let unused = |s: &mut Session| s.execute("swatch.unused", &json!({})).unwrap()["names"].clone();
    assert!(unused(&mut s).as_array().unwrap().contains(&json!("Brand")));
    let brand = s.doc().unwrap().doc.tint_paint("Brand", 0.5).unwrap();
    s.execute("charStyle.new", &json!({"name": "Ink", "attrs": {"stroke": brand}})).unwrap();
    let names = unused(&mut s);
    assert!(!names.as_array().unwrap().contains(&json!("Brand")), "{names}");
    assert!(names.as_array().unwrap().contains(&json!("Spare")), "{names}");

    let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
    g.gradient.stops[0].swatch = Some("Brand".into());
    g.gradient.stops[0].tint = 0.25;
    let fill = serde_json::to_value(vectorcraft_color::Paint::Gradient(Box::new(g.clone()))).unwrap();
    s.execute("charStyle.new", &json!({"name": "Fade", "attrs": {"fill": fill}})).unwrap();
    let def = s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Fade").unwrap().attrs.clone();
    let want = s.doc().unwrap().doc.global_color("Brand").unwrap().tinted(0.25);
    assert_eq!(def["fill"]["gradient"]["stops"][0]["color"], serde_json::to_value(want).unwrap());
    g.gradient.stops[0].swatch = Some("Gone".into());
    let gone = serde_json::to_value(vectorcraft_color::Paint::Gradient(Box::new(g))).unwrap();
    let error = s.execute("charStyle.new", &json!({"name": "Bad", "attrs": {"fill": gone}})).unwrap_err().to_string();
    assert!(error.contains("no swatch `Gone`"), "{error}");
}

#[test]
fn styles_from_files_keep_their_unknown_feature_tags_and_swatch_links() {
    let (mut s, _) = session_with_text("Text");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {}})).unwrap();
    let linked = json!({"type": "solid", "color": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}, "swatch": "Gone"});
    if let Some(def) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.iter_mut().find(|d| d.name == "Body") {
        def.attrs.insert("features".into(), json!(["zzzz", "dlig"]));
        def.attrs.insert("fill".into(), linked.clone());
    }
    let fill = s.doc().unwrap().doc.char_styles[0].attrs["fill"].clone();
    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {"features": ["zzzz", "-dlig"], "fill": fill}})).unwrap();
    let def = s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Body").unwrap().attrs.clone();
    assert_eq!(def["features"], json!(["zzzz", "-dlig"]));
    let mut replaced = def.clone();
    replaced.insert("size".into(), json!(14.0));
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": replaced})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": def})).unwrap();
    s.execute("charStyle.duplicate", &json!({"name": "Body"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Body copy").unwrap().attrs, def);
    // A list longer than a style made here can have stays editable, but does not grow.
    let long: Vec<String> = (0..300).map(|i| if i % 2 == 0 { "dlig".to_string() } else { "-dlig".to_string() }).collect();
    if let Some(def) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.iter_mut().find(|d| d.name == "Body") {
        def.attrs.insert("features".into(), json!(long));
    }
    let mut shorter = long.clone();
    shorter.pop();
    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {"features": shorter}})).unwrap();
    let mut longer = shorter.clone();
    longer.extend(["swsh".to_string(), "salt".to_string()]);
    let error = s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {"features": longer}})).unwrap_err().to_string();
    assert!(error.contains("features lists at most 256 tags"), "{error}");
}

#[test]
fn editing_character_style_clears_removed_size_without_losing_local_override() {
    let (mut s, id) = session_with_text("One two");
    s.execute("charStyle.setAttrs", &json!({"name": NORMAL_CHAR, "attrs": {"size": 15.0}})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 4, "end": 7, "size": 31.0})).unwrap();
    s.execute("charStyle.edit", &json!({"name": "Body", "to": "Body", "attrs": {}})).unwrap();
    let r = runs(&s, id);
    assert_eq!((r[0].style.size, r[1].style.size), (15.0, 31.0));
}

#[test]
fn replacing_character_style_attrs_clears_removed_size() {
    let (mut s, id) = session_with_text("Text");
    s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {}})).unwrap();
    assert_eq!(runs(&s, id)[0].style.size, 12.0);
}

#[test]
fn character_style_options_edit_validates_names_and_allows_case_changes() {
    let (mut s, _) = session_with_text("Text");
    s.execute("charStyle.new", &json!({"name":"Body","attrs":{"size":24.0}})).unwrap();
    s.execute("charStyle.new", &json!({"name":"Other","attrs":{}})).unwrap();
    for (name, to, message) in
        [(super::NORMAL_CHAR, "New", "can't be edited this way"), ("Body", "  ", "give the style a name"), ("Body", "Other", "already exists")]
    {
        let error = s.execute("charStyle.edit", &json!({"name":name,"to":to,"attrs":{}})).unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    }
    s.execute("charStyle.edit", &json!({"name":"Body","to":"body","attrs":{"size":18.0}})).unwrap();
    s.execute("charStyle.edit", &json!({"name":"body","to":" body ","attrs":{"size":20.0}})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|style| style.name == "body").unwrap().attrs["size"], json!(20.0));
}

#[test]
fn char_style_new_apply_range_redefine_keeps_overrides() {
    let (mut s, id) = session_with_text("Hello world");
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0, "underline": true}})).unwrap();
    // Style "world" only (a Type tool range).
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11})).unwrap();
    let r = runs(&s, id);
    assert_eq!(r.len(), 2);
    assert_eq!((r[1].text.as_str(), r[1].style.size, r[1].style.underline), ("world", 24.0, true));
    assert_eq!(r[1].style.style_name.as_deref(), Some("Emphasis"));
    assert_eq!(r[0].style.style_name, None);
    // A local override on the styled run survives a redefinition; other attributes update.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 6, "end": 11, "size": 30.0})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Emphasis", "attrs": {"size": 18.0, "underline": false, "tracking": 50.0}})).unwrap();
    let r = runs(&s, id);
    assert_eq!((r[1].style.size, r[1].style.underline, r[1].style.tracking), (30.0, false, 50.0));
    // Clear Overrides: back to exactly the style's attributes.
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11, "clearOverrides": true})).unwrap();
    assert_eq!(runs(&s, id)[1].style.size, 18.0);
    let list = s.execute("charStyle.list", &json!({})).unwrap();
    assert_eq!(list["styles"][0]["name"], NORMAL_CHAR);
    assert_eq!(list["styles"][1]["uses"], 1);
}

/// Applying and redefining a style record a step named from a template with the style's name
/// in it (`HistoryEntry::named`), and undo and redo keep it. The Normal styles' names are
/// marked as names VectorCraft gives. A new style's step has a name of its own.
#[test]
fn style_steps_are_named_from_a_template() {
    let (mut s, id) = session_with_text("Hello");
    let last = |s: &Session| {
        let e = s.doc().unwrap().history.undo.last().unwrap();
        (e.label.clone(), e.named().map(|n| (n.template, n.name.clone(), n.built_in)))
    };
    let named = |template, name: &str, built_in| Some((template, name.to_string(), built_in));
    s.execute("charStyle.new", &json!({"name": "Body"})).unwrap();
    assert_eq!(last(&s), ("New Character Style".into(), None));
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
    s.execute("edit.undo", &json!({})).unwrap();
    let redo = s.doc().unwrap().history.redo.last().unwrap().named().cloned();
    assert_eq!(redo.map(|n| n.label()).as_deref(), Some("Apply Body"));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
    s.execute("charStyle.apply", &json!({"name": NORMAL_CHAR, "id": id})).unwrap();
    assert_eq!(last(&s), (format!("Apply {NORMAL_CHAR}"), named("Apply {name}", NORMAL_CHAR, true)));
    s.execute("paraStyle.new", &json!({"name": "Lead"})).unwrap();
    assert_eq!(last(&s), ("New Paragraph Style".into(), None));
    s.execute("paraStyle.setAttrs", &json!({"name": "Lead", "attrs": {"direction": "rightToLeft"}})).unwrap();
    assert_eq!(last(&s), ("Redefine Lead".into(), named("Redefine {name}", "Lead", false)));
    // A step renamed since it was recorded is no longer named from the template.
    s.doc_mut().unwrap().history.undo.last_mut().unwrap().label = "Typing".into();
    assert_eq!(last(&s), ("Typing".into(), None));
}

#[test]
fn new_from_selection_rename_delete_and_undo() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"size": 40})).unwrap();
    let name = s.execute("charStyle.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Character Style 1");
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs["size"], 40.0);
    s.execute("charStyle.apply", &json!({"name": name})).unwrap();
    s.execute("charStyle.rename", &json!({"name": name, "to": "Heading"})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
    assert!(s.execute("charStyle.rename", &json!({"name": NORMAL_CHAR, "to": "x"})).is_err());
    s.execute("charStyle.duplicate", &json!({"name": "Heading"})).unwrap();
    s.execute("charStyle.delete", &json!({"name": "Heading"})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name, None);
    assert_eq!(runs(&s, id)[0].style.size, 40.0, "deleting keeps the look");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
    assert!(s.execute("charStyle.new", &json!({"name": "Bad", "attrs": {"nope": 1}})).is_err());
    // Attributes are stored canonically (a hand-written colour matches the text's afterwards).
    s.execute(
        "charStyle.new",
        &json!({"name": "Pink", "attrs": {"fill": {"type": "solid", "color": {"model": "rgb", "r": 0.9, "g": 0.2, "b": 0.4}}}}),
    )
    .unwrap();
    s.execute("charStyle.apply", &json!({"name": "Pink"})).unwrap();
    let fill = serde_json::to_value(&runs(&s, id)[0].style.fill).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Pink").unwrap().attrs["fill"], fill);
}

/// New Character Style and New Paragraph Style trim the name they are given, as a rename
/// trims `to`; the trimmed name is the one compared with the styles there are.
#[test]
fn new_styles_trim_their_names_as_a_rename_does() {
    let (mut s, _) = session_with_text("Body text");
    for p in ["charStyle", "paraStyle"] {
        assert_eq!(s.execute(&format!("{p}.new"), &json!({"name": "  Body \t"})).unwrap()["name"], "Body", "{p}");
        let list = s.execute(&format!("{p}.list"), &json!({})).unwrap();
        let names: Vec<&str> = list["styles"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
        assert_eq!(names[1..], ["Body"], "{p}");
        let e = s.execute(&format!("{p}.new"), &json!({"name": " Body"})).unwrap_err().to_string();
        assert!(e.contains("a style named `Body` already exists"), "{p}: {e}");
        let e = s.execute(&format!("{p}.new"), &json!({"name": " \t "})).unwrap_err().to_string();
        assert!(e.contains("give the style a name"), "{p}: {e}");
        assert_eq!(s.execute(&format!("{p}.duplicate"), &json!({"name": "Body"})).unwrap()["name"], "Body copy", "{p}");
    }
    // A style whose name has spaces around it, as a saved file can hold, is copied under the
    // trimmed name, numbered past the copies there are.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.push(TextStyleDef { name: " Old ".into(), attrs: Map::new() });
    s.execute("charStyle.new", &json!({"name": "Old copy"})).unwrap();
    assert_eq!(s.execute("charStyle.duplicate", &json!({"name": " Old "})).unwrap()["name"], "Old copy 2");
}

fn unused(s: &mut Session, p: &str) -> Vec<String> {
    serde_json::from_value(s.execute(&format!("{p}.unused"), &json!({})).unwrap()["names"].clone()).unwrap()
}

fn listed(s: &mut Session, p: &str) -> Vec<String> {
    let list = s.execute(&format!("{p}.list"), &json!({})).unwrap();
    list["styles"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).map(str::to_string).collect()
}

/// Select All Unused: `charStyle.unused` and `paraStyle.unused` list, in panel order and without an
/// undo step, the styles no text uses, never the Normal style. Type in a symbol, an opacity mask, a
/// graph design, a pattern tile or a brush's art counts, as type in the layers does.
#[test]
fn unused_styles_are_those_no_text_uses_wherever_the_text_is() {
    let (mut s, id) = session_with_text("Used");
    let names = ["Spare", "Used", "Symbol", "Mask", "Design", "Pattern", "Brush", "Also spare"];
    for p in ["charStyle", "paraStyle"] {
        for n in names {
            s.execute(&format!("{p}.new"), &json!({"name": n, "attrs": {}})).unwrap();
        }
        s.execute(&format!("{p}.apply"), &json!({"name": "Used", "id": id})).unwrap();
    }
    // Type in style `name`, selected.
    let styled = |s: &mut Session, name: &str| {
        let t = s.execute("text.create", &json!({"x": 10, "y": 150, "text": name})).unwrap()["id"].as_u64().unwrap();
        for p in ["charStyle", "paraStyle"] {
            s.execute(&format!("{p}.apply"), &json!({"name": name, "id": t})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [t]})).unwrap();
        NodeId(t)
    };
    let clear = |s: &mut Session, id: NodeId| {
        s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    };
    styled(&mut s, "Symbol");
    let sym = s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
    clear(&mut s, NodeId(sym["id"].as_u64().unwrap()));
    let masked = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
    let mask = styled(&mut s, "Mask");
    s.execute("select.set", &json!({"ids": [masked, mask.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let design = styled(&mut s, "Design");
    s.execute("graph.design", &json!({"new": "Bar"})).unwrap();
    clear(&mut s, design);
    let tile = styled(&mut s, "Pattern");
    s.execute("object.pattern.make", &json!({"edit": false})).unwrap();
    if s.doc().unwrap().doc.node(tile).is_some() {
        clear(&mut s, tile);
    }
    let art = styled(&mut s, "Brush");
    s.execute("brush.new", &json!({"type": "art", "name": "Lettering"})).unwrap();
    clear(&mut s, art);
    let undo = s.doc().unwrap().history.undo.len();
    for p in ["charStyle", "paraStyle"] {
        assert_eq!(unused(&mut s, p), ["Spare", "Also spare"], "{p}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    let list = s.execute("charStyle.list", &json!({})).unwrap();
    let uses = |name: &str| list["styles"].as_array().unwrap().iter().find(|x| x["name"] == name).unwrap()["uses"].as_u64();
    for name in ["Symbol", "Mask", "Design", "Pattern", "Brush"] {
        assert_eq!(uses(name), Some(1), "{name}");
    }
}

/// `charStyle.delete` and `paraStyle.delete` take `names` and delete them in one undo step; text
/// that used them keeps its look. A name that is not a style, the Normal style, or `names` that is
/// not an array of strings deletes none.
#[test]
fn several_styles_are_deleted_in_one_step() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"size": 40})).unwrap();
    for p in ["charStyle", "paraStyle"] {
        for n in ["A", "B", "C"] {
            s.execute(&format!("{p}.new"), &json!({"name": n})).unwrap();
        }
        s.execute(&format!("{p}.apply"), &json!({"name": "B", "id": id})).unwrap();
        let before = listed(&mut s, p);
        let undo = s.doc().unwrap().history.undo.len();
        let normal = before[0].clone();
        for bad in [json!(["A", "Nope"]), json!(["A", normal]), json!([]), json!(["A", 7]), json!("A")] {
            assert!(s.execute(&format!("{p}.delete"), &json!({"names": bad})).is_err(), "{p} {bad}");
        }
        assert_eq!((listed(&mut s, p), s.doc().unwrap().history.undo.len()), (before.clone(), undo), "{p}");
        s.execute(&format!("{p}.delete"), &json!({"names": ["A", "B"]})).unwrap();
        assert_eq!(listed(&mut s, p), [normal.as_str(), "C"], "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Delete Styles", "{p}");
        assert_eq!(listed(&mut s, p), before, "{p}");
    }
    s.execute("charStyle.delete", &json!({"names": ["B"]})).unwrap();
    assert_eq!((runs(&s, id)[0].style.style_name.as_deref(), runs(&s, id)[0].style.size), (None, 40.0));
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Delete Style");
}

/// A saved document whose character and paragraph styles are `char` and `para` (name, attrs),
/// stored as they are, as `charStyle.load`'s `dataBase64`.
fn styles_file(char: &[(&str, Value)], para: &[(&str, Value)]) -> String {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let mut d = (*s.doc().unwrap().doc).clone();
    let defs = |v: &[(&str, Value)]| {
        v.iter().map(|(n, a)| TextStyleDef { name: n.to_string(), attrs: a.as_object().cloned().unwrap_or_default() }).collect::<Vec<_>>()
    };
    d.char_styles = defs(char).into();
    d.para_styles = defs(para).into();
    vectorcraft_format::base64_encode(&vectorcraft_format::save(&d, false))
}

/// Load Character Styles and Load Paragraph Styles copy the other document's styles of their kind
/// after the document's own, in one undo step. A style with the same attributes as the style here
/// of its name, or of its name with a number, is not added again; any other gets the first free of
/// those names, and the style here and the type using it keep their attributes. The file's Normal
/// style is not loaded, and a style whose attributes do not read, or whose name is empty, is
/// skipped.
#[test]
fn styles_load_from_another_document_in_one_step() {
    for p in ["charStyle", "paraStyle"] {
        let (mut s, id) = session_with_text("Title");
        let normal = if p == "charStyle" { NORMAL_CHAR } else { NORMAL_PARA };
        let key = if p == "charStyle" { "size" } else { "space_after" };
        let size = |v: f64| Value::Object(Map::from_iter([(key.to_string(), json!(v))]));
        for (n, v) in [("Body", 12.0), ("Accent", 9.0), ("Accent 2", 1.0)] {
            s.execute(&format!("{p}.new"), &json!({"name": n, "attrs": size(v)})).unwrap();
        }
        s.execute(&format!("{p}.apply"), &json!({"name": "Accent", "id": id})).unwrap();
        let file =
            [("Body", size(12.0)), ("Accent", size(20.0)), ("Fresh", size(5.0)), (normal, size(30.0)), ("Bad", json!({"nope": 1})), (" ", size(2.0))];
        let data = styles_file(&file, &file);
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let r = s.execute(&format!("{p}.load"), &json!({"dataBase64": data, "name": "Other.vectorcraft"})).unwrap();
        assert_eq!(r, json!({"added": ["Accent 3", "Fresh"], "existing": ["Body"], "skipped": ["Bad", " "]}), "{p}");
        assert_eq!(listed(&mut s, p), [normal, "Body", "Accent", "Accent 2", "Accent 3", "Fresh"], "{p}");
        let d = s.doc().unwrap().doc.clone();
        let attrs = |name: &str| super::style_attrs(&d, p == "paraStyle", name).map(Value::Object);
        assert_eq!((attrs("Accent"), attrs("Accent 3"), attrs(normal)), (Some(size(9.0)), Some(size(20.0)), Some(json!({}))), "{p}");
        // The other kind and the type are as they were.
        let (other, mine) = if p == "charStyle" { (&d.para_styles, &d.char_styles) } else { (&d.char_styles, &d.para_styles) };
        let (other_before, mine_before) =
            if p == "charStyle" { (&before.para_styles, &before.char_styles) } else { (&before.char_styles, &before.para_styles) };
        assert_eq!((other, &mine[..mine_before.len()]), (other_before, &mine_before[..]), "{p}");
        assert_eq!(d.node(NodeId(id)), before.node(NodeId(id)), "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        // Loading the same file again adds nothing and makes no undo step.
        let again = s.execute(&format!("{p}.load"), &json!({"dataBase64": data, "name": "Other.vectorcraft"})).unwrap();
        assert_eq!(again["added"], json!([]), "{p}");
        assert_eq!(again["existing"], json!(["Body", "Accent 3", "Fresh"]), "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        let undone = s.execute("edit.undo", &json!({})).unwrap()["undone"].clone();
        assert_eq!(undone, if p == "charStyle" { "Load Character Styles" } else { "Load Paragraph Styles" });
        assert_eq!(s.doc().unwrap().doc, before, "{p}");
    }
}

/// Load All Styles loads both kinds in one undo step, from a path too; a file that does not open,
/// or no file, loads nothing.
#[test]
fn load_all_styles_loads_both_kinds_in_one_step() {
    let (mut s, _) = session_with_text("Title");
    let data = styles_file(&[("Lead", json!({"size": 30.0}))], &[("Centered", json!({"justify": "Center"}))]);
    let dir = std::env::temp_dir().join(format!("vc-load-styles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Other.vectorcraft");
    std::fs::write(&path, vectorcraft_format::base64_decode(&data).unwrap()).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for bad in [json!({}), json!({"data": "not a document", "name": "x.vectorcraft"}), json!({"dataBase64": "!"})] {
        assert!(s.execute("textStyle.loadAll", &bad).is_err(), "{bad}");
    }
    let r = s.execute("textStyle.loadAll", &json!({"path": path.to_string_lossy()})).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let one = |name: &str| json!({"added": [name], "existing": [], "skipped": []});
    assert_eq!(r, json!({"charStyles": one("Lead"), "paraStyles": one("Centered")}));
    assert_eq!(listed(&mut s, "charStyle"), [NORMAL_CHAR, "Lead"]);
    assert_eq!(listed(&mut s, "paraStyle"), [NORMAL_PARA, "Centered"]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Load All Styles");
    assert_eq!(s.doc().unwrap().doc, before);
}

/// A loaded character style whose size, scale, leading, shift or stroke width is out of the range
/// `charStyle.new` takes is skipped.
#[test]
fn loaded_styles_out_of_range_are_skipped() {
    let (mut s, _) = session_with_text("Title");
    let file = [
        ("Huge", json!({"size": 2000.0})),
        ("Flat", json!({"h_scale": 0.5})),
        ("Shifted", json!({"baseline_shift": 5000.0})),
        ("Thick", json!({"stroke_width": -2.0})),
        ("Fine", json!({"size": 12.0, "h_scale": 50.0})),
    ];
    let data = styles_file(&file, &[]);
    let r = s.execute("charStyle.load", &json!({"dataBase64": data, "name": "Other.vectorcraft"})).unwrap();
    assert_eq!(r, json!({"added": ["Fine"], "existing": [], "skipped": ["Huge", "Flat", "Shifted", "Thick"]}));
    assert_eq!(listed(&mut s, "charStyle"), [NORMAL_CHAR, "Fine"]);
}

/// A loaded style brings the pattern it paints with, renamed when the document has another pattern
/// of that name, as Paste brings it; a global color whose name the document gives another color
/// is the document's.
#[test]
fn loaded_styles_bring_the_patterns_and_swatches_they_paint_with() {
    // A pattern "Dots" made from a `size` square, a global "Ink", and type painted with each.
    let painted = |size: f64, ink: &str| {
        let (mut s, id) = session_with_text("Dots");
        let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": size, "height": size})).unwrap()["id"].clone();
        s.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square]})).unwrap();
        s.execute("swatch.new", &json!({"name": "Ink", "color": ink, "global": true})).unwrap();
        (s, id)
    };
    let (mut src, id) = painted(10.0, "#3366cc");
    src.execute("select.set", &json!({"ids": [id]})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [id]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Patterned", "id": id})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Ink", "ids": [id]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Inked", "id": id})).unwrap();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&src.doc().unwrap().doc, false));
    let (mut s, id) = painted(30.0, "#ff0000");
    let r = s.execute("charStyle.load", &json!({"dataBase64": data, "name": "Other.vectorcraft"})).unwrap();
    assert_eq!(r["added"], json!(["Patterned", "Inked"]));
    let d = s.doc().unwrap().doc.clone();
    let fill = |name: &str| super::style_attrs(&d, false, name).unwrap()["fill"].to_string();
    assert!(d.pattern("Dots 2").is_some() && fill("Patterned").contains("\"Dots 2\""), "{}", fill("Patterned"));
    assert!(fill("Inked").contains("\"Ink\"") && d.swatch("Ink 2").is_none(), "{}", fill("Inked"));
    s.execute("charStyle.apply", &json!({"name": "Patterned", "id": id})).unwrap();
    assert!(matches!(&runs(&s, id)[0].style.fill, vectorcraft_color::Paint::Pattern { pattern, .. } if pattern == "Dots 2"));
}

/// A pattern whose type is set in a loaded style that was renamed on the way in ("Accent 2") is
/// the same pattern when the file is loaded again, and so is the style that paints with it, also
/// when "Accent" itself paints with another pattern.
#[test]
fn reloaded_patterns_follow_the_renamed_styles_their_type_uses() {
    for patterned in [false, true] {
        let (mut src, id) = session_with_text("Tile");
        if patterned {
            let square = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
            src.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square]})).unwrap();
            src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [id]})).unwrap();
        }
        src.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 4, "kerning": 40})).unwrap();
        src.execute("charStyle.new", &json!({"name": "Accent", "id": id})).unwrap();
        src.execute("charStyle.apply", &json!({"name": "Accent", "id": id})).unwrap();
        if patterned {
            // A solid fill over the style's, so the tile's type itself paints with no pattern.
            src.execute("paint.setFill", &json!({"color": "#000000", "ids": [id]})).unwrap();
        }
        src.execute("object.pattern.make", &json!({"name": "Tiles", "edit": false, "ids": [id]})).unwrap();
        let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
        src.execute("paint.setFill", &json!({"swatch": "Tiles", "ids": [other]})).unwrap();
        src.execute("charStyle.new", &json!({"name": "Tiled", "id": other})).unwrap();
        let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&src.doc().unwrap().doc, false));
        let (mut s, _) = session_with_text("X");
        s.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 10}})).unwrap();
        let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
        s.execute("charStyle.load", &load).unwrap();
        let counts = |s: &Session| {
            let d = &s.doc().unwrap().doc;
            (d.patterns.len(), d.char_styles.iter().map(|c| c.name.clone()).collect::<Vec<_>>())
        };
        let first = counts(&s);
        assert!(first.1.contains(&"Accent 2".to_string()) && first.1.contains(&"Tiled".to_string()), "{patterned}: {first:?}");
        let r = s.execute("charStyle.load", &load).unwrap();
        assert_eq!(r["added"], json!([]), "{patterned}: {r}");
        assert_eq!(counts(&s), first, "{patterned}");
    }
}

/// A loaded style that paints with a pattern whose type is set in that same style still paints
/// with the pattern when the pattern comes in under a free name ("Dots 2"). The commands refuse to
/// make that loop, so the source is a document edited directly, as a file may hold it.
#[test]
fn a_style_and_the_pattern_that_uses_it_load_together() {
    let (mut src, id) = session_with_text("Tile");
    src.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": id})).unwrap();
    src.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [id]})).unwrap();
    let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": other})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [other]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Painted", "id": other})).unwrap();
    let mut source = (*src.doc().unwrap().doc).clone();
    let painted = source.char_styles.iter().find(|s| s.name == "Painted").unwrap().attrs.clone();
    source.char_styles.retain(|s| s.name != "Painted");
    source.char_styles.iter_mut().find(|s| s.name == "Accent").unwrap().attrs = painted;
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&source, false));
    let fill = |s: &Session, name: &str| super::style_attrs(&s.doc().unwrap().doc, false, name).unwrap()["fill"].to_string();
    let source_fill = super::style_attrs(&source, false, "Accent").unwrap()["fill"].to_string();
    assert!(source_fill.contains("\"Dots\""), "{source_fill}");
    let (mut s, _) = session_with_text("X");
    let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 30, "height": 30})).unwrap()["id"].clone();
    s.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square]})).unwrap();
    let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
    s.execute("charStyle.load", &load).unwrap();
    assert!(s.doc().unwrap().doc.pattern("Dots 2").is_some());
    assert!(fill(&s, "Accent").contains("\"Dots 2\""), "{}", fill(&s, "Accent"));
    // Loaded again, the two are found under those names, and nothing is added.
    let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r["added"], json!([]), "{r}");
    assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len()), (doc, undo));
}

/// A style and the pattern it paints with, whose type is set in that style, are both added when
/// the document has a style of that name and attributes but another pattern of that name: the
/// copy of the style paints with the copy of the pattern, and a second load adds nothing.
#[test]
fn a_style_in_a_loop_is_not_matched_to_a_style_that_paints_with_another_pattern() {
    let (mut src, id) = session_with_text("Tile");
    src.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": id})).unwrap();
    src.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [id]})).unwrap();
    let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": other})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [other]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Painted", "id": other})).unwrap();
    let mut source = (*src.doc().unwrap().doc).clone();
    let painted = source.char_styles.iter().find(|s| s.name == "Painted").unwrap().attrs.clone();
    source.char_styles.retain(|s| s.name != "Painted");
    source.char_styles.iter_mut().find(|s| s.name == "Accent").unwrap().attrs = painted.clone();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&source, false));
    let (mut s, _) = session_with_text("X");
    let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 30, "height": 30})).unwrap()["id"].clone();
    s.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square]})).unwrap();
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    doc.char_styles.push(TextStyleDef { name: "Accent".into(), attrs: painted });
    let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r["added"], json!(["Accent 2"]), "{r}");
    let d = s.doc().unwrap().doc.clone();
    let fill = |name: &str| super::style_attrs(&d, false, name).unwrap()["fill"].to_string();
    assert!(fill("Accent").contains("\"Dots\"") && fill("Accent 2").contains("\"Dots 2\""), "{} {}", fill("Accent"), fill("Accent 2"));
    let NodeKind::Text(text) = &d.pattern("Dots 2").unwrap().art[0].kind else { panic!("pattern art should be type") };
    assert_eq!(text.runs[0].style.style_name.as_deref(), Some("Accent 2"));
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r["added"], json!([]), "{r}");
    assert_eq!(s.doc().unwrap().doc, d);
}

/// A brush in a loaded style's pattern whose art sets type in a style renamed on the way in
/// ("Accent 2") is the same brush when the file is loaded again.
#[test]
fn reloaded_brushes_follow_the_renamed_styles_their_art_uses() {
    let (mut src, id) = session_with_text("Ink");
    src.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": id})).unwrap();
    src.execute("brush.new", &json!({"type": "art", "name": "Lettering", "ids": [id]})).unwrap();
    let path = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 40, "height": 10})).unwrap()["id"].clone();
    src.execute("brush.apply", &json!({"name": "Lettering", "ids": [path]})).unwrap();
    src.execute("object.pattern.make", &json!({"name": "Tiles", "edit": false, "ids": [path]})).unwrap();
    let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Tiles", "ids": [other]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Tiled", "id": other})).unwrap();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&src.doc().unwrap().doc, false));
    let (mut s, _) = session_with_text("X");
    s.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 10}})).unwrap();
    let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
    let r = s.execute("charStyle.load", &load).unwrap();
    assert!(r["added"].as_array().unwrap().contains(&json!("Accent 2")), "{r}");
    let d = s.doc().unwrap().doc.clone();
    let lib = vectorcraft_brush::library(&d);
    let brush = lib.iter().find(|b| b.name == "Lettering").unwrap();
    let mut styles = vec![];
    for a in brush.art() {
        a.walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                styles.extend(t.runs.iter().filter_map(|r| r.style.style_name.clone()));
            }
        });
    }
    assert_eq!(styles, ["Accent 2"]);
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r["added"], json!([]), "{r}");
    assert_eq!(s.doc().unwrap().doc, d);
}

/// A style name from a saved document stays the name used by type in imported pattern art.
#[test]
fn loaded_style_keeps_the_name_used_by_pattern_type() {
    let (mut src, id) = session_with_text("Tile");
    src.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    src.execute("charStyle.apply", &json!({"name": "Accent", "id": id})).unwrap();
    src.execute("object.pattern.make", &json!({"name": "Tiles", "edit": false, "ids": [id]})).unwrap();
    let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Tiles", "ids": [other]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Tiled", "id": other})).unwrap();
    let mut source = (*src.doc().unwrap().doc).clone();
    source.char_styles.iter_mut().find(|s| s.name == "Accent").unwrap().name = " Accent ".into();
    let tile = source.patterns.iter_mut().find(|p| p.name == "Tiles").unwrap();
    let art = std::sync::Arc::make_mut(&mut tile.art[0]);
    let NodeKind::Text(text) = &mut art.kind else { panic!("pattern art should be type") };
    text.runs[0].style.style_name = Some(" Accent ".into());
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&source, false));
    let (mut dst, _) = session_with_text("X");
    let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
    dst.execute("charStyle.load", &load).unwrap();
    let doc = &dst.doc().unwrap().doc;
    assert!(doc.char_styles.iter().any(|s| s.name == " Accent "));
    let NodeKind::Text(text) = &doc.pattern("Tiles").unwrap().art[0].kind else { panic!("pattern art should be type") };
    assert_eq!(text.runs[0].style.style_name.as_deref(), Some(" Accent "));
}

/// Load Character Styles brings the paragraph style that type in a loaded style's pattern is set
/// in, renamed when the document has another style of that name, without listing it in the reply.
#[test]
fn loaded_pattern_type_keeps_its_paragraph_style() {
    let (mut src, id) = session_with_text("Tile");
    src.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center"}})).unwrap();
    src.execute("paraStyle.apply", &json!({"name": "Centered", "id": id})).unwrap();
    src.execute("object.pattern.make", &json!({"name": "Tiles", "edit": false, "ids": [id]})).unwrap();
    let other = src.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Tiles", "ids": [other]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Tiled", "id": other})).unwrap();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&src.doc().unwrap().doc, false));
    let (mut s, _) = session_with_text("X");
    s.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Right"}})).unwrap();
    let load = json!({"dataBase64": data, "name": "Other.vectorcraft"});
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r, json!({"added": ["Tiled"], "existing": [], "skipped": []}));
    let d = s.doc().unwrap().doc.clone();
    assert_eq!(listed(&mut s, "paraStyle"), [NORMAL_PARA, "Centered", "Centered 2"]);
    assert_eq!(super::style_attrs(&d, true, "Centered 2").map(Value::Object), Some(json!({"justify": "Center"})));
    let NodeKind::Text(text) = &d.pattern("Tiles").unwrap().art[0].kind else { panic!("pattern art should be type") };
    assert_eq!(text.para_styles().filter_map(|p| p.style_name.as_deref()).collect::<Vec<_>>(), ["Centered 2"]);
    // Loaded again, nothing is added.
    let r = s.execute("charStyle.load", &load).unwrap();
    assert_eq!(r["added"], json!([]), "{r}");
    assert_eq!(s.doc().unwrap().doc, d);
}

/// A pattern whose tile paints with itself is found under its renamed copy on a second load.
#[test]
fn self_referencing_pattern_is_not_added_again() {
    let (mut src, id) = session_with_text("Patterned");
    let square = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
    src.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square]})).unwrap();
    let tile = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
    src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [tile]})).unwrap();
    let art = src.doc().unwrap().doc.node(NodeId(tile.as_u64().unwrap())).unwrap().clone();
    assert!(art.appearance.items.iter().any(|item| matches!(item.paint(), vectorcraft_color::Paint::Pattern { pattern, .. } if pattern == "Dots")));
    let mut source = (*src.doc().unwrap().doc).clone();
    source.patterns.iter_mut().find(|p| p.name == "Dots").unwrap().art = vec![std::sync::Arc::new(art)];
    src.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [id]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Patterned", "id": id})).unwrap();
    source.char_styles = src.doc().unwrap().doc.char_styles.clone();
    assert!(super::style_attrs(&source, false, "Patterned").unwrap()["fill"].to_string().contains("\"Dots\""));
    let (mut dst, _) = session_with_text("X");
    let different = dst.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 30, "height": 30})).unwrap()["id"].clone();
    dst.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [different]})).unwrap();
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    assert!(dst.doc().unwrap().doc.pattern("Dots 2").is_some());
    assert!(
        dst.doc().unwrap().doc.pattern("Dots 2").unwrap().art[0]
            .appearance
            .items
            .iter()
            .any(|item| matches!(item.paint(), vectorcraft_color::Paint::Pattern { pattern, .. } if pattern == "Dots 2"))
    );
    let before = dst.doc().unwrap().doc.clone();
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    let again = super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    assert!(again.iter().all(|s| !s.added));
    assert_eq!(dst.doc().unwrap().doc, before);
}

/// Patterns whose tiles each paint with themselves are matched one loop at a time: nine of them,
/// each in the document under its name with other art and as the copy an earlier load added,
/// are found under those copies, and a second load adds nothing.
#[test]
fn separate_self_referencing_patterns_are_not_added_again() {
    let (mut src, id) = session_with_text("Patterned");
    let mut tiles = vec![];
    for k in 0..9 {
        let name = format!("Dots{k}");
        let square = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
        src.execute("object.pattern.make", &json!({"name": name, "edit": false, "ids": [square]})).unwrap();
        let tile = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
        src.execute("paint.setFill", &json!({"swatch": name, "ids": [tile]})).unwrap();
        tiles.push((name.clone(), src.doc().unwrap().doc.node(NodeId(tile.as_u64().unwrap())).unwrap().clone()));
        src.execute("paint.setFill", &json!({"swatch": name, "ids": [id]})).unwrap();
        src.execute("charStyle.new", &json!({"name": format!("Patterned{k}"), "id": id})).unwrap();
    }
    let mut source = (*src.doc().unwrap().doc).clone();
    for (name, art) in tiles {
        source.patterns.iter_mut().find(|p| p.name == name).unwrap().art = vec![std::sync::Arc::new(art)];
    }
    let (mut dst, _) = session_with_text("X");
    for k in 0..9 {
        let different = dst.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 30, "height": 30})).unwrap()["id"].clone();
        dst.execute("object.pattern.make", &json!({"name": format!("Dots{k}"), "edit": false, "ids": [different]})).unwrap();
    }
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    for k in 0..9 {
        assert!(dst.doc().unwrap().doc.pattern(&format!("Dots{k} 2")).is_some(), "{k}");
    }
    let before = dst.doc().unwrap().doc.clone();
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    let again = super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    assert!(again.iter().all(|s| !s.added));
    assert_eq!(dst.doc().unwrap().doc.patterns.len(), before.patterns.len());
    assert_eq!(dst.doc().unwrap().doc, before);
    // With one loop's pattern gone from the document, the others are still found.
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    target.patterns.retain(|p| !p.name.starts_with("Dots8"));
    let left = target.patterns.len();
    let third = super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    assert_eq!(dst.doc().unwrap().doc.patterns.len(), left + 1);
    assert!(third.iter().filter(|s| s.name != "Patterned8" && !s.name.starts_with("Patterned8 ")).all(|s| !s.added));
}

/// Nine patterns whose tiles each paint with the next one, the last with the first, in the
/// document under their names with other art and as the copies an earlier load added, are found
/// under those copies, and a second load adds nothing.
#[test]
fn a_long_loop_of_patterns_is_not_added_again() {
    let (mut src, id) = session_with_text("Patterned");
    for k in 0..9 {
        let square = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
        src.execute("object.pattern.make", &json!({"name": format!("Dots{k}"), "edit": false, "ids": [square]})).unwrap();
    }
    let mut tiles = vec![];
    for k in 0..9 {
        let tile = src.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap()["id"].clone();
        src.execute("paint.setFill", &json!({"swatch": format!("Dots{}", (k + 1) % 9), "ids": [tile]})).unwrap();
        tiles.push(src.doc().unwrap().doc.node(NodeId(tile.as_u64().unwrap())).unwrap().clone());
    }
    src.execute("paint.setFill", &json!({"swatch": "Dots0", "ids": [id]})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Patterned", "id": id})).unwrap();
    let mut source = (*src.doc().unwrap().doc).clone();
    for (k, art) in tiles.into_iter().enumerate() {
        source.patterns.iter_mut().find(|p| p.name == format!("Dots{k}")).unwrap().art = vec![std::sync::Arc::new(art)];
    }
    let (mut dst, _) = session_with_text("X");
    for k in 0..9 {
        let different = dst.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 30, "height": 30})).unwrap()["id"].clone();
        dst.execute("object.pattern.make", &json!({"name": format!("Dots{k}"), "edit": false, "ids": [different]})).unwrap();
    }
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    for k in 0..9 {
        assert!(dst.doc().unwrap().doc.pattern(&format!("Dots{k} 2")).is_some(), "{k}");
    }
    let before = dst.doc().unwrap().doc.clone();
    let target = std::sync::Arc::make_mut(&mut dst.doc_mut().unwrap().doc);
    let again = super::super::clipboard::resources::import_text_styles(&source, target, true, true);
    assert!(again.iter().all(|s| !s.added));
    assert_eq!(dst.doc().unwrap().doc.patterns.len(), before.patterns.len());
    assert_eq!(dst.doc().unwrap().doc, before);
}

/// #1001: attributes their JSON leaves out at the defaults (Auto leading and kerning, no
/// mojikumi) are still attributes a style can set, and a style captured with one applies to
/// text at the default, is duplicated and comes back out of a library.
#[test]
fn styles_take_attributes_left_out_at_their_defaults() {
    let (mut s, id) = session_with_text("A");
    s.execute("charStyle.new", &json!({"name": "Kerned", "attrs": {"kerning": 40, "leading": 30}})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Kerned", "attrs": {"kerning": 20, "small_caps": 70}})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Punctuation", "attrs": {"mojikumi": "lineEndHalf"}})).unwrap();
    s.execute("paraStyle.setAttrs", &json!({"name": "Punctuation", "attrs": {"direction": "rightToLeft"}})).unwrap();
    for bad in [json!({"nope": 1}), json!({"justify_auto": true})] {
        assert!(s.execute("paraStyle.new", &json!({"name": "Bad", "attrs": bad})).is_err(), "{bad}");
    }
    // Captured from text with explicit leading, applied to fresh Auto-leading text.
    s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Spaced"})).unwrap();
    let fresh = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(runs(&s, fresh)[0].style.leading, None);
    for clear in [false, true] {
        s.execute("charStyle.apply", &json!({"name": "Spaced", "id": fresh, "clearOverrides": clear})).unwrap();
        assert_eq!(runs(&s, fresh)[0].style.leading, Some(30.0));
    }
    let copy = s.execute("charStyle.duplicate", &json!({"name": "Spaced"})).unwrap()["name"].as_str().unwrap().to_string();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == copy).unwrap().attrs["leading"], 30.0);
    s.execute("charStyle.apply", &json!({"name": "Kerned", "id": fresh})).unwrap();
    let st = runs(&s, fresh)[0].style.clone();
    assert_eq!((st.kerning, st.small_caps), (Some(20.0), Some(70.0)));
    s.execute("paraStyle.apply", &json!({"name": "Punctuation", "id": id})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
    assert_eq!((t.para.mojikumi, t.para.direction), (vectorcraft_doc::Mojikumi::LineEndHalf, Some(vectorcraft_doc::ParaDirection::RightToLeft)));
    // A library keeps the captured attributes and makes the style again where it's used.
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let lib = s.execute("library.add", &json!({"kind": "charStyle", "name": "Lib Spaced"})).unwrap();
    let other = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "C"})).unwrap()["id"].as_u64().unwrap();
    s.execute("library.use", &json!({"library": lib["library"], "kind": "charStyle", "item": lib["name"]})).unwrap();
    assert_eq!(runs(&s, other)[0].style.leading, Some(30.0));
}

#[test]
fn auto_leading_redefine_keeps_overrides_and_history() {
    for redefine in [true, false] {
        let (mut s, source) = session_with_text("Source");
        let [linked, local] =
            ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
        let ids = [source, linked, local];
        s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"leading": 30}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
        s.execute("text.setStyle", &json!({"ids": [local], "leading": 42})).unwrap();
        s.execute("text.setStyle", &json!({"ids": [source], "leading": "auto"})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        if redefine {
            s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        } else {
            s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": null}})).unwrap();
        }
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [None, None, Some(42.0)]);
        assert!(ids.iter().all(|&id| runs(&s, id)[0].style.style_name.as_deref() == Some("Body")));
        let after = s.doc().unwrap().doc.clone();
        assert_eq!(after.char_styles[0].attrs.get("leading"), Some(&Value::Null), "Auto is explicit, not an unset attribute");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
        let back = vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap();
        assert_eq!(back.char_styles, after.char_styles);
        for id in ids {
            let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("not text") };
            assert_eq!(t.runs, runs(&s, id));
        }
        // Auto users follow the next numeric definition; real numeric and Auto overrides stay.
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 26}})).unwrap();
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(26.0), Some(26.0), Some(42.0)]);
        s.execute("text.setStyle", &json!({"ids": [linked], "leading": "auto"})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 28}})).unwrap();
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(28.0), None, Some(42.0)]);
        // Omitting leading returns linked text to Auto and keeps local overrides.
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"tracking": 20}})).unwrap();
        assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [None, None, Some(42.0)]);
    }
}

/// The attribute names a style of type `T` can set, without the style name.
fn settable<T: DeserializeOwned>() -> std::collections::BTreeSet<String> {
    attr_names::<T>().into_iter().filter(|&n| n != "style_name").map(String::from).collect()
}

/// New and Redefine capture every paragraph attribute. A paragraph style redefined from a
/// paragraph at the defaults gives its paragraphs the defaults back where they had the old
/// definition's values, and `setAttrs` with a default value does too; local overrides, and
/// attributes `setAttrs` leaves out return to the base value.
#[test]
fn para_style_redefined_to_the_defaults_updates_its_paragraphs() {
    use vectorcraft_doc::{Composer, HyphenationOptions};
    let (mut s, source) = session_with_text("Source");
    let [linked, local] =
        ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
    let ids = [source, linked, local];
    let para = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    };
    let limits = |s: &Session| ids.map(|id| para(s, id).hyphenation.hyphen_limit);
    let composers = |s: &Session| ids.map(|id| para(s, id).composer);
    let defined = |s: &Session, i: usize| s.doc().unwrap().doc.para_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    s.execute("text.setFormat", &json!({"ids": ids, "hyphenLimit": 2, "composer": "singleLine"})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [local], "hyphenLimit": 4})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [source], "hyphenLimit": 0, "composer": "everyLine"})).unwrap();
    s.execute("paraStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([0, 0, 4], [Composer::EveryLine; 3]));
    assert_eq!(para(&s, linked).hyphenation, HyphenationOptions::default());
    assert!(ids.iter().all(|&id| para(&s, id).style_name.as_deref() == Some("Body")));
    assert_eq!(defined(&s, 0), settable::<ParaStyle>(), "Redefine captures every attribute");
    s.execute("paraStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
    assert_eq!(defined(&s, 1), settable::<ParaStyle>(), "New captures every attribute");
    // Style options: a default value reaches users, and an attribute left out returns to base.
    s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"hyphenation": {"hyphen_limit": 3}, "composer": "singleLine"}})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([3, 3, 4], [Composer::SingleLine; 3]));
    s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"composer": "everyLine"}})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([0, 0, 4], [Composer::EveryLine; 3]));
    assert_eq!(Value::Object(s.doc().unwrap().doc.para_styles[0].attrs.clone()), json!({"composer": "everyLine"}));
}

/// The same for character styles: Auto kerning and the Roman baseline alignment, which the
/// run's JSON leaves out, are captured and reach the style's users. A glyph choice belongs to
/// one character, so it is not a reusable style attribute.
#[test]
fn char_style_redefined_to_the_defaults_updates_its_text() {
    use vectorcraft_doc::CharAlign;
    let (mut s, source) = session_with_text("Source");
    let [linked, local] =
        ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
    let ids = [source, linked, local];
    let kerning = |s: &Session| ids.map(|id| runs(s, id)[0].style.kerning);
    let aligns = |s: &Session| ids.map(|id| runs(s, id)[0].style.char_align);
    let defined = |s: &Session, i: usize| s.doc().unwrap().doc.char_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    s.execute("text.setFormat", &json!({"ids": ids, "kerning": 40, "charAlign": "emBoxCenter"})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [local], "kerning": 60})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [source], "kerning": "auto", "charAlign": "romanBaseline"})).unwrap();
    s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([None, None, Some(60.0)], [CharAlign::RomanBaseline; 3]));
    let mut reusable = settable::<CharStyle>();
    reusable.remove("glyphAlternate");
    assert_eq!(defined(&s, 0), reusable, "Redefine captures every reusable attribute");
    s.execute("charStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
    assert_eq!(defined(&s, 1), reusable, "New captures every reusable attribute");
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"kerning": 10, "charAlign": "emBoxTop"}})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([Some(10.0), Some(10.0), Some(60.0)], [CharAlign::EmBoxTop; 3]));
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"charAlign": "romanBaseline"}})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([None, None, Some(60.0)], [CharAlign::RomanBaseline; 3]));
    assert_eq!(Value::Object(s.doc().unwrap().doc.char_styles[0].attrs.clone()), json!({"charAlign": "romanBaseline"}));
}

/// A style captured from type without Variable Font settings sets none explicitly (`{}`):
/// applying it clears type's settings, and redefining it from type whose settings were reset
/// resets the type that uses it.
#[test]
fn variable_font_settings_are_captured_explicitly() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let (mut s, source) = session_with_text("Source");
    let linked = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Linked"})).unwrap()["id"].as_u64().unwrap();
    let ids = [source, linked];
    s.execute("text.setStyle", &json!({"ids": ids, "font": TWO_AXIS_FAMILY})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs.get("fontAxes"), Some(&json!({})));
    s.execute("text.setFormat", &json!({"ids": [linked], "fontAxes": {"wght": 650}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    let wght = |s: &Session| ids.map(|id| runs(s, id)[0].style.font_axes.get("wght").copied());
    assert_eq!(wght(&s), [None, None], "applying the style clears the settings");
    for (set, want) in [(json!(650), Some(650.0)), (json!(null), None)] {
        s.execute("text.setFormat", &json!({"ids": [source], "fontAxes": {"wght": set}})).unwrap();
        s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        assert_eq!(wght(&s), [want, want], "redefined from {set}");
    }
}

/// A style that changes the font, applied or redefined, leaves none of the old font's
/// version and Variable Font settings, unless the style sets them.
#[test]
fn another_font_from_a_style_clears_the_old_fonts_version_and_settings() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let (mut s, id) = session_with_text("Source");
    let set_variable = |s: &mut Session| {
        s.execute("text.setStyle", &json!({"ids": [id], "font": TWO_AXIS_FAMILY})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "fontAxes": {"wght": 650}})).unwrap();
        s.edit("version", |d, _| {
            if let Some(t) = text_mut(d, NodeId(id)) {
                t.runs.iter_mut().for_each(|r| r.style.font_version = Some("Version 2.000".into()));
            }
            Ok(())
        })
        .unwrap();
    };
    let font = |s: &Session| {
        let st = runs(s, id)[0].style.clone();
        (st.font_family, st.font_version, st.font_axes.get("wght").copied())
    };
    set_variable(&mut s);
    s.execute("charStyle.new", &json!({"name": "Plain", "attrs": {"font_family": "Inter"}})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Same", "attrs": {"size": 20}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Same", "id": id})).unwrap();
    assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), Some("Version 2.000".to_string()), Some(650.0)), "the same font keeps them");
    s.execute("charStyle.apply", &json!({"name": "Plain", "id": id})).unwrap();
    assert_eq!(font(&s), ("Inter".to_string(), None, None));
    // A style that sets the settings keeps its own.
    s.execute("charStyle.new", &json!({"name": "Heavy", "attrs": {"font_family": TWO_AXIS_FAMILY, "fontAxes": {"wght": 600}}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Heavy", "id": id})).unwrap();
    assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), None, Some(600.0)));
    // Redefined to another font: its users' settings and versions go too.
    s.execute("charStyle.new", &json!({"name": "Sans", "attrs": {"font_family": TWO_AXIS_FAMILY}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Sans", "id": id})).unwrap();
    set_variable(&mut s);
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Sans"));
    s.execute("charStyle.setAttrs", &json!({"name": "Sans", "attrs": {"font_family": "Inter"}})).unwrap();
    assert_eq!(font(&s), ("Inter".to_string(), None, None));
}

#[test]
fn auto_leading_new_distinguishes_unset_attributes() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Sparse", "attrs": {"tracking": 20}})).unwrap();
    assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
    s.execute("charStyle.apply", &json!({"name": "Sparse", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, Some(30.0));
    s.execute("charStyle.new", &json!({"name": "Automatic", "attrs": {"leading": null}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Automatic", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, None);
    assert_eq!(s.doc().unwrap().doc.char_styles[1].attrs.get("leading"), Some(&Value::Null));
    // New from Auto text captures that value too, rather than leaving leading unspecified.
    s.execute("charStyle.new", &json!({"name": "Captured", "id": id})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [id], "leading": 36})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Captured", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, None);
    assert_eq!(s.doc().unwrap().doc.char_styles[2].attrs.get("leading"), Some(&Value::Null));
}

#[test]
fn optical_character_style_captures_and_redefines_automatic_mode() {
    let (mut s, id) = session_with_text("III");
    s.execute("text.setRangeStyle", &json!({"id": id, "kerning": "optical"})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Profile", "id": id})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs.get("opticalKerning"), Some(&json!(true)));
    s.execute("text.setRangeStyle", &json!({"id": id, "kerning": "auto"})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Profile", "id": id})).unwrap();
    assert!(runs(&s, id)[0].style.optical_kerning);
    s.execute("text.setRangeStyle", &json!({"id": id, "kerning": "auto"})).unwrap();
    s.execute("charStyle.redefine", &json!({"name": "Profile", "id": id})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs.get("opticalKerning"), Some(&json!(false)));
}

#[test]
fn paragraph_styles_and_survive_save() {
    let (mut s, id) = session_with_text("Body");
    s.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center", "space_after": 6.0}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Centered"})).unwrap();
    let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para.clone(),
        _ => panic!(),
    };
    assert_eq!((para(&s).justify, para(&s).space_after), (vectorcraft_doc::Justify::Center, 6.0));
    // The Normal style can be redefined too (stored once redefined).
    s.execute("paraStyle.redefine", &json!({"name": NORMAL_PARA})).unwrap();
    assert_eq!(s.execute("paraStyle.list", &json!({})).unwrap()["styles"][0]["attrs"]["justify"], "Center");
    let d = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(back.para_styles, d.para_styles);
}

fn capture_boundary_command(redefine: bool, text: &str, start: usize, end: Option<usize>, expected_size: f64) {
    let (mut s, id) = session_with_text(text);
    s.execute("text.setStyle", &json!({"id": id, "size": 18})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 2, "end": text.len(), "size": 34})).unwrap();
    if redefine {
        s.execute("charStyle.new", &json!({"name": "Captured", "attrs": {"size": 24}})).unwrap();
    }
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let mut params = json!({"name": "Captured", "id": id, "start": start});
    if let Some(end) = end {
        params["end"] = json!(end);
    }
    let command = if redefine { "charStyle.redefine" } else { "charStyle.new" };
    s.execute(command, &params).unwrap();
    let st = s.doc().unwrap();
    let def = st.doc.char_styles.iter().find(|d| d.name == "Captured").unwrap();
    assert_eq!(def.attrs.get("size").and_then(Value::as_f64), Some(expected_size), "{command} {text:?} {start}..{end:?}");
    assert!(!def.attrs.contains_key("style_name"));
    assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)), "capture does not apply the definition");
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
    let after = st.doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, after);
    assert_eq!(s.doc().unwrap().selection, selection);
}

fn capture_boundary_ranges(redefine: bool) {
    for (text, start, end, size) in [
        ("ABcdef", 2, 5, 34.0),
        ("é漢字", 2, 5, 34.0),
        ("ABcdef", 5, 2, 34.0), // command ranges are ordered before selected-character sampling
        ("ABcdef", 1, 5, 18.0), // a mixed range samples its first selected character
        ("ABcdef", 2, 2, 18.0), // collapsed range retains insertion affinity
        ("ABcdef", 3, 3, 34.0),
        ("ABcdef", 99, 100, 34.0), // beyond the text retains the final run
    ] {
        capture_boundary_command(redefine, text, start, Some(end), size);
    }
}

#[test]
fn char_style_new_range_capture_boundary() {
    capture_boundary_ranges(false);
}

#[test]
fn char_style_redefine_range_capture_boundary() {
    capture_boundary_ranges(true);
}

#[test]
fn char_style_start_only_capture_affinity_controls() {
    for redefine in [false, true] {
        for text in ["ABcdef", "é漢字"] {
            for (start, size) in [(0, 18.0), (2, 18.0), (3, 34.0), (99, 34.0)] {
                capture_boundary_command(redefine, text, start, None, size);
            }
        }
    }
}

#[test]
fn para_style_range_capture_samples_start_paragraph() {
    for redefine in [false, true] {
        for (start, end, justify) in [(0, 7, "Left"), (4, 7, "Right"), (4, 0, "Right")] {
            let (mut s, id) = session_with_text("One\nTwo");
            s.execute("text.setStyle", &json!({"id": id, "justify": "left"})).unwrap();
            s.execute("text.setStyle", &json!({"id": id, "start": 4, "end": 7, "justify": "right"})).unwrap();
            if redefine {
                s.execute("paraStyle.new", &json!({"name": "Captured", "attrs": {"justify": "Center"}})).unwrap();
            }
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let command = if redefine { "paraStyle.redefine" } else { "paraStyle.new" };
            s.execute(command, &json!({"name": "Captured", "id": id, "start": start, "end": end})).unwrap();
            let st = s.doc().unwrap();
            let def = st.doc.para_styles.iter().find(|d| d.name == "Captured").unwrap();
            assert_eq!(def.attrs["justify"], json!(justify), "paragraph capture still samples start, not the end or a normalized range");
            assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)));
            assert_eq!(st.selection, selection);
            assert_eq!(st.history.undo.len(), undo + 1);
            let after = st.doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, after);
        }
    }
}
