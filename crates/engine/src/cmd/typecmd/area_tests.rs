use super::*;

/// #2413: `text.create` and `text.createInPath` return the new type's `bounds`, the box
/// `document.inspect` reports for it, so a script can place it without a helper object.
#[test]
fn text_create_returns_the_inspect_bounds() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let inspected = |s: &mut Session, id: &Value| {
        let doc = s.execute("document.inspect", &json!({})).unwrap();
        fn find(v: &Value, id: &Value) -> Option<Value> {
            if &v["id"] == id {
                return Some(v["bounds"].clone());
            }
            v.get("children").or_else(|| v.get("layers"))?.as_array()?.iter().find_map(|c| find(c, id))
        }
        find(&doc, id).unwrap()
    };
    let made = s.execute("text.create", &json!({"x": 300, "y": 100, "text": "Hello", "size": 24, "justify": "center"})).unwrap();
    assert!(made["bounds"]["width"].as_f64().unwrap() > 10.0, "{made}");
    assert_eq!(made["bounds"], inspected(&mut s, &made["id"]));
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 200, "width": 100, "height": 50})).unwrap();
    let area = s.execute("text.createInPath", &json!({"path": rect["id"], "mode": "area", "text": "Boxed"})).unwrap();
    assert_eq!(area["bounds"], inspected(&mut s, &area["id"]));
}

/// #1748: `text.create` takes `justify`; point type then lines up on its point (centred on
/// it, or ending there), and `text.setStyle` moves it the same way afterwards.
#[test]
fn text_create_justify_anchors_point_type_on_its_point() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let bounds = |s: &Session, id: &Value| {
        let id = NodeId(id.as_u64().unwrap());
        let st = s.doc().unwrap();
        st.doc.bounds_of(&[id], false).unwrap()
    };
    let make = |s: &mut Session, extra: Value| {
        let mut p = json!({"x": 300, "y": 100, "text": "Centred title", "size": 24});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("text.create", &p).unwrap()["id"].clone()
    };
    let id = make(&mut s, json!({}));
    let left = bounds(&s, &id);
    assert!(left.width() > 10.0, "{left:?}");
    assert!((left.x0 - 300.0).abs() < 1.0, "left (the default) starts at x: {left:?}");
    let id = make(&mut s, json!({"justify": "center"}));
    let centre = bounds(&s, &id);
    assert!((centre.center().x - 300.0).abs() < 0.5, "centred on x: {centre:?}");
    assert!((centre.width() - left.width()).abs() < 0.5);
    let right = make(&mut s, json!({"justify": "right"}));
    let r = bounds(&s, &right);
    assert!((r.x1 - 300.0).abs() < 1.0, "ends at x: {r:?}");
    let got = |s: &Session, id: &Value| match &s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.para.justify,
        _ => panic!("not text"),
    };
    assert_eq!(got(&s, &right), Justify::Right);
    // Changed afterwards, it moves to the same place as made centred.
    s.execute("text.setStyle", &json!({"id": right, "justify": "center"})).unwrap();
    let moved = bounds(&s, &right);
    assert!((moved.center().x - 300.0).abs() < 0.5, "{moved:?}");
    assert!(s.execute("text.create", &json!({"x": 0, "y": 0, "text": "a", "justify": "middle"})).is_err());
    // Type in or on a path takes it too.
    let path = s.execute("path.create", &json!({"d": "M50 300 L550 300"})).unwrap()["id"].clone();
    let on = s.execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "On a path", "justify": "center"})).unwrap();
    assert_eq!(got(&s, &on["id"]), Justify::Center);
}

#[test]
fn kinsoku_set_is_set_per_paragraph_and_saved_only_when_not_hard() {
    use vectorcraft_doc::Kinsoku;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": "あカッ", "area": {"width": 40, "height": 100}})).unwrap()["id"].as_u64().unwrap();
    let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para.clone(),
        _ => panic!("text"),
    };
    // New type: Hard, today's set.
    assert_eq!(para(&s).kinsoku, Kinsoku::Hard);
    assert!(serde_json::to_value(para(&s)).unwrap().get("kinsoku").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"kinsoku": "strict"})).is_err());
    s.execute("text.setFormat", &json!({"kinsoku": "soft"})).unwrap();
    assert_eq!(para(&s).kinsoku, Kinsoku::Soft);
    assert_eq!(serde_json::to_value(para(&s)).unwrap()["kinsoku"], "soft");
    s.execute("text.setFormat", &json!({"kinsoku": "none"})).unwrap();
    assert_eq!(para(&s).kinsoku, Kinsoku::None);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(para(&s).kinsoku, Kinsoku::Hard);
    // Documents from before it read as Hard.
    let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
    assert_eq!(old.kinsoku, Kinsoku::Hard);
}

#[test]
fn burasagari_is_set_per_paragraph_and_saved_only_when_on() {
    use vectorcraft_doc::Burasagari;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "雅楽、笙。", "area": {"width": 200, "height": 100}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para.clone(),
        _ => panic!("text"),
    };
    // New type: Standard.
    assert_eq!(para(&s).burasagari, Burasagari::Standard);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"burasagari": "strong"})).is_err());
    s.execute("text.setFormat", &json!({"burasagari": "forced"})).unwrap();
    assert_eq!(para(&s).burasagari, Burasagari::Forced);
    assert_eq!(serde_json::to_value(para(&s)).unwrap()["burasagari"], "forced");
    s.execute("text.setFormat", &json!({"burasagari": "none"})).unwrap();
    assert_eq!(para(&s).burasagari, Burasagari::None);
    // Saved only when on; documents from before it read as None.
    assert!(serde_json::to_value(para(&s)).unwrap().get("burasagari").is_none());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(para(&s).burasagari, Burasagari::Standard);
    let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
    assert_eq!(old.burasagari, Burasagari::None);
}

/// Type › Optical Margin Alignment: `type.opticalMarginAlignment` sets or toggles the flag on
/// the selected type in one undo step, the opening quotation mark then hangs left of the
/// frame, and the flag is saved only when on and survives a native save.
#[test]
fn optical_margin_alignment_is_set_per_object_and_saved_only_when_on() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s
        .execute("text.create", &json!({"x": 10, "y": 10, "size": 40, "text": "“Quoted” words", "area": {"width": 300, "height": 100}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    // The left edge of the glyphs in text space, where the frame's left edge is at 0.
    let left = |s: &Session| text(s).cached_bounds.unwrap().x0;
    // New type: off, and not saved.
    assert!(!text(&s).optical_margin);
    assert!(serde_json::to_value(text(&s)).unwrap().get("opticalMarginAlignment").is_none());
    let inside = left(&s);
    assert!(inside >= 0.0, "{inside}");
    // Nothing but a rectangle selected: an error.
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    assert!(s.execute("type.opticalMarginAlignment", &json!({})).is_err());
    // Toggled on: the quotation mark hangs outside the frame.
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert_eq!(s.execute("type.opticalMarginAlignment", &json!({})).unwrap()["on"], true);
    assert!(text(&s).optical_margin);
    assert!(left(&s) < inside - 5.0, "{} {inside}", left(&s));
    assert_eq!(serde_json::to_value(text(&s)).unwrap()["opticalMarginAlignment"], true);
    // `on` sets it; without `on` it toggles.
    assert_eq!(s.execute("type.opticalMarginAlignment", &json!({"on": true})).unwrap()["on"], true);
    assert!(text(&s).optical_margin);
    assert_eq!(s.execute("type.opticalMarginAlignment", &json!({})).unwrap()["on"], false);
    assert!(!text(&s).optical_margin);
    assert_eq!(left(&s), inside);
    // One undo step each.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(text(&s).optical_margin);
    // Saved and reopened as it was; a document from before it reads as off.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.optical_margin);
    let mut old = serde_json::to_value(text(&s)).unwrap();
    old.as_object_mut().unwrap().remove("opticalMarginAlignment");
    assert!(!serde_json::from_value::<TextObject>(old).unwrap().optical_margin);
}

/// Character panel menu › System Layout: `type.systemLayout` sets the document and every text
/// object in one undo step. New type inherits it. Off by default and in a document from before
/// the flag, and saved only when on. On, each positive advance is a whole number of points.
#[test]
fn system_layout_sets_the_document_and_every_text_object() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 40, "size": 13, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setStyle", &json!({"id": id, "tracking": 50})).unwrap();
    let other = s.execute("text.create", &json!({"x": 10, "y": 80, "text": "Yo"})).unwrap()["id"].as_u64().unwrap();
    let text = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    let setup = |s: &Session| s.doc().unwrap().doc.setup.clone();
    let advances =
        |s: &Session| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id)).glyphs.iter().map(|g| g.advance).collect::<Vec<_>>();
    assert!(!setup(&s).system_layout);
    assert!(!text(&s, id).system_layout && !text(&s, other).system_layout);
    assert!(serde_json::to_value(setup(&s)).unwrap().get("systemLayout").is_none());
    assert!(serde_json::to_value(text(&s, id)).unwrap().get("systemLayout").is_none());
    let fractional = advances(&s);
    assert!(fractional.iter().any(|a| (a - a.round()).abs() > 1e-6), "{fractional:?}");
    assert_eq!(s.execute("type.systemLayout", &json!({})).unwrap()["on"], true);
    assert!(setup(&s).system_layout);
    assert!(text(&s, id).system_layout && text(&s, other).system_layout);
    let whole = advances(&s);
    assert!(whole.iter().all(|a| *a >= 1.0 && (a - a.round()).abs() < 1e-9), "{whole:?}");
    assert!(fractional.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 0.05), "{fractional:?} {whole:?}");
    assert_eq!(serde_json::to_value(setup(&s)).unwrap()["systemLayout"], true);
    assert_eq!(serde_json::to_value(text(&s, id)).unwrap()["systemLayout"], true);
    // New type inherits the document's setting.
    let made = s.execute("text.create", &json!({"x": 10, "y": 120, "text": "Z"})).unwrap()["id"].as_u64().unwrap();
    assert!(text(&s, made).system_layout);
    // A boolean given as text is an error, and the setting stays on.
    assert!(s.execute("type.systemLayout", &json!({"on": "no"})).is_err());
    assert!(setup(&s).system_layout && text(&s, id).system_layout);
    assert_eq!(s.execute("type.systemLayout", &json!({"on": false})).unwrap()["on"], false);
    assert!(!setup(&s).system_layout && !text(&s, id).system_layout && !text(&s, made).system_layout);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(setup(&s).system_layout && text(&s, id).system_layout);
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert!(back.setup.system_layout);
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.system_layout);
    let mut old = serde_json::to_value(setup(&s)).unwrap();
    old.as_object_mut().unwrap().remove("systemLayout");
    assert!(!serde_json::from_value::<vectorcraft_doc::DocSetup>(old).unwrap().system_layout);
}

/// Character panel menu › Fractional Widths: `type.fractionalWidths` sets or toggles the flag
/// on the selected type in one undo step. New type and a document from before the flag are on,
/// and the flag is saved only when off. Off, each positive advance is a whole number of points.
#[test]
fn fractional_widths_is_set_per_object_and_saved_only_when_off() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 40, "size": 13, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setStyle", &json!({"id": id, "tracking": 50})).unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    let advances =
        |s: &Session| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s)).glyphs.iter().map(|g| g.advance).collect::<Vec<_>>();
    // New type: on, and not saved.
    assert!(text(&s).fractional_widths);
    assert!(serde_json::to_value(text(&s)).unwrap().get("fractionalWidths").is_none());
    let fractional = advances(&s);
    assert!(fractional.iter().any(|a| (a - a.round()).abs() > 1e-6), "{fractional:?}");
    // Nothing but a rectangle selected: an error.
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    assert!(s.execute("type.fractionalWidths", &json!({})).is_err());
    // Toggled off: whole-point advances, and the key is written.
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert_eq!(s.execute("type.fractionalWidths", &json!({})).unwrap()["on"], false);
    assert!(!text(&s).fractional_widths);
    let whole = advances(&s);
    assert!(whole.iter().all(|a| *a >= 1.0 && (a - a.round()).abs() < 1e-9), "{whole:?}");
    assert!(fractional.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 0.05), "{fractional:?} {whole:?}");
    assert_eq!(serde_json::to_value(text(&s)).unwrap()["fractionalWidths"], false);
    // A boolean given as text is an error, and the flag stays off.
    assert!(s.execute("type.fractionalWidths", &json!({"on": "no"})).is_err());
    assert!(!text(&s).fractional_widths);
    assert_eq!(s.execute("type.fractionalWidths", &json!({})).unwrap()["on"], true);
    assert!(text(&s).fractional_widths);
    assert_eq!(s.execute("type.fractionalWidths", &json!({"on": false})).unwrap()["on"], false);
    assert!(!text(&s).fractional_widths);
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(text(&s).fractional_widths);
    // Saved and reopened as it was; a document from before it reads as on.
    s.execute("type.fractionalWidths", &json!({"on": false})).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(!t.fractional_widths);
    let mut old = serde_json::to_value(text(&s)).unwrap();
    old.as_object_mut().unwrap().remove("fractionalWidths");
    assert!(serde_json::from_value::<TextObject>(old).unwrap().fractional_widths);
}

/// Paragraph panel menu › Hyphenation…: `text.setFormat` sets each paragraph's hyphenation
/// options, rounded and clamped, in one undo step; they are saved only when not the defaults,
/// and paragraph styles carry them.
#[test]
fn hyphenation_options_are_set_per_paragraph_and_saved_only_when_not_default() {
    use vectorcraft_doc::HyphenationOptions;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "One\nTwo", "area": {"width": 200, "height": 100}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let paras = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (t.para_at(0).clone(), t.para_at(1).clone()),
        _ => panic!("text"),
    };
    assert_eq!(paras(&s).0.hyphenation, HyphenationOptions::default());
    assert!(serde_json::to_value(paras(&s).0).unwrap().get("hyphenation").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute(
        "text.setFormat",
        &json!({"wordsLongerThan": 7.6, "afterFirst": 0, "beforeLast": 40, "hyphenLimit": 2, "hyphenationZone": 18, "hyphenateCapitalized": false, "start": 5, "end": 5}),
    )
    .unwrap();
    let want = HyphenationOptions { words_longer_than: 8, after_first: 1, before_last: 15, hyphen_limit: 2, zone: 18.0, capitalized: false };
    let (first, second) = paras(&s);
    assert_eq!((first.hyphenation, &second.hyphenation), (HyphenationOptions::default(), &want), "only the paragraph the caret is in");
    assert_eq!(serde_json::to_value(&second).unwrap()["hyphenation"]["hyphen_limit"], 2);
    let back: vectorcraft_doc::ParaStyle = serde_json::from_value(serde_json::to_value(&second).unwrap()).unwrap();
    assert_eq!(back.hyphenation, want);
    s.execute("text.setFormat", &json!({"hyphenLimit": -3, "hyphenationZone": 5000})).unwrap();
    assert_eq!((paras(&s).0.hyphenation.hyphen_limit, paras(&s).1.hyphenation.zone), (0, 1296.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paras(&s).1.hyphenation, HyphenationOptions::default());
    // A paragraph style carries them; an older file's paragraph reads the defaults.
    s.execute("paraStyle.new", &json!({"name": "Loose", "attrs": {"hyphenation": {"words_longer_than": 9}}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Loose", "id": id})).unwrap();
    assert_eq!(paras(&s).0.hyphenation, HyphenationOptions { words_longer_than: 9, ..HyphenationOptions::default() });
    let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left", "hyphenate": true})).unwrap();
    assert_eq!(old.hyphenation, HyphenationOptions::default());
}

/// Paragraph panel menu › Roman Hanging Punctuation: `text.setFormat {romanHangingPunctuation}`
/// sets it per paragraph in one undo step, the opening quotation mark then hangs left of the
/// frame, it is saved only when on, and paragraph styles carry it.
#[test]
fn roman_hanging_punctuation_is_set_per_paragraph_and_saved_only_when_on() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s
        .execute("text.create", &json!({"x": 10, "y": 10, "size": 40, "text": "“Quoted”\n“Two”", "area": {"width": 300, "height": 200}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    let left = |s: &Session| text(s).cached_bounds.unwrap().x0;
    assert!(!text(&s).para_at(0).roman_hanging);
    assert!(serde_json::to_value(text(&s).para_at(0)).unwrap().get("roman_hanging").is_none());
    let inside = left(&s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    // Only the first paragraph (the caret's range).
    s.execute("text.setFormat", &json!({"romanHangingPunctuation": true, "start": 0, "end": 0})).unwrap();
    assert!(text(&s).para_at(0).roman_hanging && !text(&s).para_at(1).roman_hanging);
    assert!(left(&s) < inside - 5.0, "{} {inside}", left(&s));
    assert_eq!(serde_json::to_value(text(&s).para_at(0)).unwrap()["roman_hanging"], true);
    let back: vectorcraft_doc::ParaStyle = serde_json::from_value(serde_json::to_value(text(&s).para_at(0)).unwrap()).unwrap();
    assert!(back.roman_hanging);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!text(&s).para_at(0).roman_hanging);
    assert!((left(&s) - inside).abs() < 1e-9);
    // A paragraph style carries it.
    s.execute("paraStyle.new", &json!({"name": "Hung", "attrs": {"roman_hanging": true}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Hung", "id": id})).unwrap();
    assert!(text(&s).para_at(1).roman_hanging);
}

/// Paragraph panel › Insert Kashidas: `text.setFormat {kashida}` sets it per paragraph in one
/// undo step, the paragraph is saved with it only when it is not None and opens with it, an
/// unknown value changes nothing, and paragraph styles carry it.
#[test]
fn insert_kashidas_is_set_per_paragraph_and_saved_only_when_set() {
    use vectorcraft_doc::text::Kashida;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s
        .execute("text.create", &json!({"x": 10, "y": 10, "size": 20, "text": "كتب محمد\nعلي", "area": {"width": 300, "height": 200}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    assert_eq!(text(&s).para_at(0).kashida, Kashida::None);
    assert!(serde_json::to_value(text(&s).para_at(0)).unwrap().get("kashida").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    // Only the first paragraph (the caret's range).
    s.execute("text.setFormat", &json!({"kashida": "medium", "start": 0, "end": 0})).unwrap();
    assert_eq!((text(&s).para_at(0).kashida, text(&s).para_at(1).kashida), (Kashida::Medium, Kashida::None));
    assert_eq!(serde_json::to_value(text(&s).para_at(0)).unwrap()["kashida"], "medium");
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Some(NodeKind::Text(back)) = reopened.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
    assert_eq!((back.para_at(0).kashida, back.para_at(1).kashida), (Kashida::Medium, Kashida::None));
    assert!(s.execute("text.setFormat", &json!({"kashida": "wide"})).is_err());
    assert_eq!(text(&s).para_at(0).kashida, Kashida::Medium, "an unknown value changes nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s).para_at(0).kashida, Kashida::None);
    // A paragraph style carries it.
    s.execute("paraStyle.new", &json!({"name": "Stretched", "attrs": {"kashida": "stylistic"}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Stretched", "id": id})).unwrap();
    assert_eq!(text(&s).para_at(1).kashida, Kashida::Stylistic);
}

/// Paragraph panel › Bullets: `text.setFormat {list, bullet}` makes the paragraphs it touches a
/// bulleted list and refuses unknown values. A paragraph that joins a list with no left or
/// first-line indent gets 18 pt and -18 pt, unless the call sets an indent, and one with indents
/// keeps them. The list is saved only when set and opens with the file, Return continues it,
/// "none" turns it off, and paragraph styles set it and turn it off.
#[test]
fn bulleted_lists_are_set_per_paragraph_saved_and_continued() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "One\nTwo\nThree", "area": {"width": 300, "height": 200}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    let list = |s: &Session, i: usize| text(s).para_at(i).list;
    let indents = |s: &Session, i: usize| (text(s).para_at(i).left_indent, text(s).para_at(i).first_line_indent);
    let dot = Some(ParaList::bulleted(Bullet::Dot));
    assert_eq!(list(&s, 0), None);
    assert!(serde_json::to_value(text(&s).para_at(0)).unwrap().get("list").is_none(), "no list is saved without one");
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"start": 4, "end": 4, "leftIndent": 30, "firstLineIndent": 5})).unwrap();
    // The first two paragraphs: the first gets the hanging indent, the second keeps its indents.
    s.execute("text.setFormat", &json!({"list": "bullet", "start": 0, "end": 4})).unwrap();
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (dot, dot, None));
    assert_eq!((indents(&s, 0), indents(&s, 1), indents(&s, 2)), ((18.0, -18.0), (30.0, 5.0), (0.0, 0.0)));
    // `bullet` picks the bullet, and `list: "bullet"` keeps it; the indents stay as they are.
    s.execute("text.setFormat", &json!({"bullet": "square", "start": 0, "end": 0})).unwrap();
    s.execute("text.setFormat", &json!({"list": "bullet", "start": 0, "end": 0})).unwrap();
    let square = Some(ParaList::bulleted(Bullet::Square));
    assert_eq!((list(&s, 0), indents(&s, 0)), (square, (18.0, -18.0)));
    for bad in [json!({"list": "numbers"}), json!({"bullet": "star"}), json!({"list": 1}), json!({"bullet": true})] {
        assert!(s.execute("text.setFormat", &bad).is_err(), "{bad}");
    }
    assert_eq!((list(&s, 0), list(&s, 2)), (square, None), "an unknown value changes nothing");
    assert_eq!(serde_json::to_value(text(&s).para_at(0)).unwrap()["list"], json!({"bullet": "square"}));
    // A saved list reads the fields it leaves out as their defaults and ignores those it doesn't know.
    let read = |v: serde_json::Value| serde_json::from_value::<vectorcraft_doc::ParaStyle>(json!({ "list": v })).unwrap().list;
    assert_eq!((read(json!({})), read(json!({"bullet": "dash", "level": 2}))), (dot, Some(ParaList::bulleted(Bullet::Dash))));
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Some(NodeKind::Text(back)) = reopened.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
    assert_eq!((back.para_at(0).list, back.para_at(1).list, back.para_at(2).list), (square, dot, None));
    // Return at the end of a list item starts another one.
    s.execute("text.editRange", &json!({"id": id, "start": 3, "end": 3, "insert": "\nOne more"})).unwrap();
    assert_eq!((list(&s, 1), indents(&s, 1)), (square, (18.0, -18.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    // "none" turns the list off and leaves the indents.
    s.execute("text.setFormat", &json!({"bullet": "none", "start": 0, "end": 0})).unwrap();
    assert_eq!((list(&s, 0), indents(&s, 0)), (None, (18.0, -18.0)));
    s.execute("text.setFormat", &json!({"list": "none", "start": 4, "end": 4})).unwrap();
    assert_eq!(list(&s, 1), None);
    // Indents the call sets win over the hanging indent.
    s.execute("text.setFormat", &json!({"list": "bullet", "leftIndent": 0, "firstLineIndent": 0, "start": 8, "end": 8})).unwrap();
    assert_eq!((list(&s, 2), indents(&s, 2)), (dot, (0.0, 0.0)));
    // A style that holds a list sets it; a style captured from a paragraph without one turns it off.
    s.execute("paraStyle.new", &json!({"name": "Listed", "attrs": {"list": {"bullet": "dash"}}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Listed", "id": id})).unwrap();
    let dash = Some(ParaList::bulleted(Bullet::Dash));
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (dash, dash, dash));
    let plain = s.execute("text.create", &json!({"x": 10, "y": 300, "text": "Plain"})).unwrap()["id"].as_u64().unwrap();
    s.execute("paraStyle.new", &json!({"name": "Plain", "id": plain})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Plain", "id": id})).unwrap();
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (None, None, None));
}

/// Numbered lists: `list: "number"` numbers the paragraphs 1, 2, 3 and `number` picks the
/// style; a list switched between bullets and numbers keeps its bullet and its indents; an
/// unknown value is an error; the list is saved, reopened and carried by paragraph styles.
#[test]
fn numbered_lists_are_set_per_paragraph_saved_and_styled() {
    use vectorcraft_doc::text::{Bullet, NumberStyle, ParaList};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "One\nTwo\nThree", "area": {"width": 300, "height": 200}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    let list = |s: &Session, i: usize| text(s).para_at(i).list;
    let numbers = |s: &Session| {
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s)).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"list": "number"})).unwrap();
    let arabic = Some(ParaList::numbered(NumberStyle::Arabic));
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (arabic, arabic, arabic));
    assert_eq!((text(&s).para_at(0).left_indent, text(&s).para_at(0).first_line_indent), (18.0, -18.0));
    assert_eq!(numbers(&s), ["1.", "2.", "3."]);
    // `number` picks the style, `list: "number"` keeps it, and the count runs on through both.
    s.execute("text.setFormat", &json!({"number": "upperRoman", "start": 0, "end": 0})).unwrap();
    s.execute("text.setFormat", &json!({"list": "number", "start": 0, "end": 0})).unwrap();
    let roman = Some(ParaList::numbered(NumberStyle::UpperRoman));
    assert_eq!(list(&s, 0), roman);
    assert_eq!(numbers(&s), ["I.", "2.", "3."]);
    // A bullet makes it bulleted, a number style numbered again, and `list: "bullet"` brings
    // back the bullet it had; the indents stay as they are.
    s.execute("text.setFormat", &json!({"bullet": "square", "start": 0, "end": 0})).unwrap();
    assert_eq!(list(&s, 0), Some(ParaList::bulleted(Bullet::Square)));
    s.execute("text.setFormat", &json!({"number": "lowerLetters", "start": 0, "end": 0})).unwrap();
    let letters = Some(ParaList { bullet: Bullet::Square, number: Some(NumberStyle::LowerLetters) });
    assert_eq!(list(&s, 0), letters);
    assert_eq!(numbers(&s), ["a.", "2.", "3."]);
    assert_eq!((text(&s).para_at(0).left_indent, text(&s).para_at(0).first_line_indent), (18.0, -18.0));
    for bad in [json!({"number": "roman"}), json!({"number": 1}), json!({"list": "numbered"}), json!({"number": true})] {
        assert!(s.execute("text.setFormat", &bad).is_err(), "{bad}");
    }
    assert_eq!((list(&s, 0), list(&s, 1)), (letters, arabic), "an unknown value changes nothing");
    assert_eq!(serde_json::to_value(text(&s).para_at(0)).unwrap()["list"], json!({"bullet": "square", "number": "lowerLetters"}));
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Some(NodeKind::Text(back)) = reopened.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
    assert_eq!((back.para_at(0).list, back.para_at(1).list, back.para_at(2).list), (letters, arabic, arabic));
    // Return at the end of a numbered item starts the next number.
    s.execute("text.editRange", &json!({"id": id, "start": 13, "end": 13, "insert": "\nFour"})).unwrap();
    assert_eq!(numbers(&s), ["a.", "2.", "3.", "4."]);
    s.execute("edit.undo", &json!({})).unwrap();
    // "none" turns the list off; the next numbered paragraph starts again at 1.
    s.execute("text.setFormat", &json!({"number": "none", "start": 4, "end": 4})).unwrap();
    assert_eq!(list(&s, 1), None);
    assert_eq!(numbers(&s), ["a.", "1."]);
    // A style that holds a numbered list sets it, a style captured from a numbered paragraph
    // carries its list, and one captured from a paragraph without a list turns it off.
    s.execute("paraStyle.new", &json!({"name": "Roman", "attrs": {"list": {"number": "upperRoman"}}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Roman", "id": id})).unwrap();
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (roman, roman, roman));
    assert_eq!(numbers(&s), ["I.", "II.", "III."]);
    let plain = s.execute("text.create", &json!({"x": 10, "y": 300, "text": "Plain"})).unwrap()["id"].as_u64().unwrap();
    s.execute("paraStyle.new", &json!({"name": "Plain", "id": plain})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Captured", "id": id})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Captured", "id": plain})).unwrap();
    let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(plain)).map(|n| &n.kind) else { panic!("text") };
    assert_eq!(t.para_at(0).list, roman);
    s.execute("paraStyle.apply", &json!({"name": "Plain", "id": id})).unwrap();
    assert_eq!((list(&s, 0), list(&s, 1), list(&s, 2)), (None, None, None));
}

/// The text object `id`.
fn text_at(s: &Session, id: u64) -> vectorcraft_doc::text::TextObject {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    }
}

/// How many list markers text object `id` draws.
fn marker_count(s: &Session, id: u64) -> usize {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text_at(s, id)).markers.len()
}

/// The tab stops of paragraph `k` of text object `id`.
fn tab_stops(s: &Session, id: u64, k: usize) -> Vec<(f64, vectorcraft_doc::TabAlign, String)> {
    text_at(s, id).para_at(k).tabs.iter().map(|t| (t.position, t.align, t.leader.clone())).collect()
}

/// The glyphs the layout of type `id` draws, each with its glyph, where it is drawn and the
/// transform it is drawn with (in thousandths), from the top line down and left to right.
fn drawn_glyphs(s: &Session, id: u64) -> Vec<(u32, i64, i64, [i64; 6])> {
    let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let milli = |v: f64| (v * 1000.0).round() as i64;
    let mut v: Vec<(u32, i64, i64, [i64; 6])> = lay
        .glyphs
        .iter()
        .filter(|g| !g.outline.elements().is_empty())
        .map(|g| (g.gid, milli(g.origin.x), milli(g.origin.y), g.xf.as_coeffs().map(milli)))
        .collect();
    v.sort_by_key(|&(_, x, y, _)| (y, x));
    v
}

/// Type › Bullets and Numbering › Convert to Text: `text.convertListToText` writes each list
/// paragraph's marker before its text as characters in the marker's style, followed by a tab,
/// turns the list off, and gives the paragraph a plain left tab stop at the left indent where
/// the marker's tab went to it, so every glyph is drawn where it was: bullets in area type (a
/// wrapped item, an empty item, a right-aligned stop at the indent and a stop before it, both
/// with leaders), numbers narrower and wider than the hanging indent (one reaching a stop with
/// a leader past it), and point and vertical type. A paragraph with a tab of its own keeps its
/// stops past the indent. The characters can be styled and deleted, and one undo brings the
/// lists back.
#[test]
fn convert_to_text_writes_the_markers_where_they_were_drawn() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let (text, markers, tabs) = (text_at, marker_count, tab_stops);
    let long = "The quick brown fox jumps over the lazy dog and runs far away into the woods beyond the hill";
    let id = s
        .execute("text.create", &json!({"x": 10, "y": 10, "text": format!("One\n{long}\n\nFour\nFive"), "area": {"width": 200, "height": 400}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let (four, five) = (long.len() + 6, long.len() + 11);
    s.execute("text.setFormat", &json!({"ids": [id], "list": "bullet"})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [id], "bullet": "square", "start": four, "end": four})).unwrap();
    // A right-aligned stop with a leader at the fourth item's left indent, and a stop with a
    // leader before the fifth item's: the bullets' tabs go to the indent and to that stop and
    // draw no leader.
    s.execute("text.tabs.set", &json!({"ids": [id], "start": four, "end": four, "stops": [{"position": 18, "align": "right", "leader": "."}]}))
        .unwrap();
    s.execute("text.tabs.set", &json!({"ids": [id], "start": five, "end": five, "stops": [{"position": 10, "leader": "."}]})).unwrap();
    // The first character larger, raised and underlined: the bullet takes its size alone. The
    // fourth item's first character is larger than the break before it.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 1, "size": 20, "baselineShift": 4, "underline": true})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": four, "end": four + 1, "size": 20})).unwrap();
    let listed = text(&s, id);
    let before = drawn_glyphs(&s, id);
    assert_eq!(markers(&s, id), 4, "the empty item has no bullet");
    let reply = s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
    let inserted = |at: usize| json!({"id": id, "at": at, "length": 4});
    assert_eq!(reply, json!({"ids": [id], "converted": 5, "inserted": [inserted(0), inserted(4), inserted(four), inserted(five)]}));
    assert_eq!(drawn_glyphs(&s, id), before, "every glyph is drawn where it was");
    let t = text(&s, id);
    assert_eq!(t.plain_text(), format!("\u{2022}\tOne\n\u{2022}\t{long}\n\n\u{25AA}\tFour\n\u{2022}\tFive"));
    assert!((0..5).all(|k| t.para_at(k).list.is_none()), "no paragraph is in a list");
    assert_eq!(markers(&s, id), 0);
    let left = vectorcraft_doc::TabAlign::Left;
    let plain_stop = |position| vec![(position, left, String::new())];
    assert_eq!(
        (tabs(&s, id, 0), tabs(&s, id, 1), tabs(&s, id, 2), tabs(&s, id, 3), tabs(&s, id, 4)),
        (plain_stop(18.0), plain_stop(18.0), vec![], plain_stop(18.0), plain_stop(10.0)),
        "a plain left stop at the left indent (in place of the right-aligned one), none in the empty item, and the stop the \
             fifth item's tab went to without its leader"
    );
    let bullet = &t.runs[0];
    assert_eq!(
        (bullet.text.as_str(), bullet.style.size, bullet.style.baseline_shift, bullet.style.underline),
        ("\u{2022}\t", 20.0, 0.0, false),
        "the bullet has the first character's size without its baseline shift and underline"
    );
    // The characters are text: they take a style and can be deleted.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "size": 30})).unwrap();
    assert_eq!(text(&s, id).runs[0].style.size, 30.0);
    s.execute("text.editRange", &json!({"id": id, "start": 0, "end": 4, "insert": ""})).unwrap();
    assert!(text(&s, id).plain_text().starts_with("One\n\u{2022}\t"));
    for _ in 0..3 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    let back = text(&s, id);
    assert_eq!(back.para_at(3).list, Some(ParaList::bulleted(Bullet::Square)));
    assert_eq!((back.runs, back.para, back.paras), (listed.runs, listed.para, listed.paras), "one undo step");
    assert_eq!(drawn_glyphs(&s, id), before);
    // Numbers 1 to 9 at 20 pt are narrower than the hanging indent, 10 is wider and takes the
    // text to the next default stop, and 11 to its stop at 72 pt, whose leader the number's tab
    // draws neither before nor after.
    let items = (1..=11).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let id = s.execute("text.create", &json!({"x": 300, "y": 10, "size": 20, "text": items, "area": {"width": 300, "height": 600}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("text.setFormat", &json!({"ids": [id], "list": "number"})).unwrap();
    let eleven = 9 * 7 + 8;
    s.execute("text.tabs.set", &json!({"ids": [id], "start": eleven, "end": eleven, "stops": [{"position": 72, "leader": "."}]})).unwrap();
    let before = drawn_glyphs(&s, id);
    s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
    assert_eq!(drawn_glyphs(&s, id), before, "numbers");
    let expected = (1..=11).map(|i| format!("{i}.\tItem {i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(text(&s, id).plain_text(), expected);
    assert_eq!((tabs(&s, id, 0), tabs(&s, id, 9), tabs(&s, id, 10)), (plain_stop(18.0), vec![], plain_stop(72.0)));
    // A paragraph with a tab of its own keeps its stops past the left indent, which that tab
    // can reach.
    let id = s.execute("text.create", &json!({"x": 10, "y": 650, "size": 30, "text": "Item\tvalue"})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setFormat", &json!({"ids": [id], "list": "number"})).unwrap();
    s.execute("text.tabs.set", &json!({"ids": [id], "stops": [{"position": 72, "leader": "."}, {"position": 200}]})).unwrap();
    s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
    assert_eq!(text(&s, id).plain_text(), "1.\tItem\tvalue");
    assert_eq!(tabs(&s, id, 0), [(72.0, left, ".".to_string()), (200.0, left, String::new())]);
    // A 30 pt number's tab to a right-aligned stop aligns the whole paragraph there, trailing
    // space included, and a converted tab aligns the rest of its line: a plain left stop where
    // the number's tab ended keeps the text in place. A right-aligned stop before the left stop
    // the number's tab reached, which the wrapped paragraph's width kept the number's tab from,
    // would catch the converted tab, which measures only the first line: it goes. The lines are
    // compared with the Single-line Composer, as a paragraph with a tab is broken line by line.
    let right = vectorcraft_doc::TabAlign::Right;
    for (text_in, width, stops, kept) in [
        ("One ", 300, json!([{"position": 200, "align": "right"}]), vec![(200.0, right, String::new())]),
        ("The quick brown fox jumps over the lazy dog", 230, json!([{"position": 100, "align": "right"}, {"position": 150}]), vec![]),
    ] {
        let id = s.execute("text.create", &json!({"x": 10, "y": 600, "size": 30, "text": text_in, "area": {"width": width, "height": 300}})).unwrap()
            ["id"]
            .as_u64()
            .unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "list": "number", "composer": "singleLine"})).unwrap();
        s.execute("text.tabs.set", &json!({"ids": [id], "stops": stops})).unwrap();
        let before = drawn_glyphs(&s, id);
        s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
        assert_eq!(drawn_glyphs(&s, id), before, "{text_in:?}");
        let after = tabs(&s, id, 0);
        assert_eq!((after.first().map(|t| (t.1, t.2.as_str())), after.get(1..).map(<[_]>::to_vec)), (Some((left, "")), Some(kept)), "{text_in:?}");
    }
    // Overset items, which the layout leaves out, get the stop by the same rule: a 30 pt number is
    // wider than the hanging indent, a 12 pt bullet is not.
    for (size, list, text_after, stop) in
        [(30, "number", "1.\tOne\n2.\tTwo", vec![]), (12, "bullet", "\u{2022}\tOne\n\u{2022}\tTwo", plain_stop(18.0))]
    {
        let id = s
            .execute("text.create", &json!({"x": 10, "y": 500, "size": size, "text": "One\nTwo", "area": {"width": 200, "height": size + 10}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "list": list})).unwrap();
        assert_eq!(markers(&s, id), 1, "the second item is overset");
        s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
        assert_eq!((text(&s, id).plain_text().as_str(), tabs(&s, id, 1)), (text_after, stop), "{size} pt");
    }
    point_and_vertical_type_convert(&mut s);
}

/// Point type and vertical type convert too, and type with no list paragraph has nothing to convert.
fn point_and_vertical_type_convert(s: &mut Session) {
    // Point type and vertical type.
    for vertical in [false, true] {
        let id = s.execute("text.create", &json!({"x": 400, "y": 700, "vertical": vertical, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "list": "number"})).unwrap();
        let before = drawn_glyphs(s, id);
        s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
        assert_eq!(drawn_glyphs(s, id), before, "vertical {vertical}");
        assert_eq!(text_at(s, id).plain_text(), "1.\tOne\n2.\tTwo");
    }
    // Type with no list paragraph: nothing to convert and no undo step.
    let plain = s.execute("text.create", &json!({"x": 10, "y": 700, "text": "Plain"})).unwrap()["id"].as_u64().unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("text.convertListToText", &json!({"ids": [plain]})).unwrap(), json!({"ids": [], "converted": 0, "inserted": []}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
}

/// A right-to-left list paragraph converted to text starts with the number, its period and a
/// tab, which are read first: the line draws its glyphs in the order it drew them with the
/// marker, the number at the right with its period at its left. Letters and roman numerals
/// would make the paragraph run left to right, so the paragraph gets the direction it ran in;
/// digits leave the direction unset.
#[test]
fn a_right_to_left_list_paragraph_converts_to_text_read_first() {
    use vectorcraft_doc::ParaDirection;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    for (number, text, direction) in [
        ("arabic", "1.\tשלום\n2.\tעולם", None),
        ("upperLetters", "A.\tשלום\nB.\tעולם", Some(ParaDirection::RightToLeft)),
        ("upperRoman", "I.\tשלום\nII.\tעולם", Some(ParaDirection::RightToLeft)),
    ] {
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "שלום\nעולם", "area": {"width": 200, "height": 200}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "number": number})).unwrap();
        // `drawn_glyphs` lists the first line first, from left to right.
        let order = |s: &Session| {
            let glyphs = drawn_glyphs(s, id);
            let top = glyphs.first().map_or(0, |g| g.2);
            glyphs.into_iter().filter(|g| g.2 == top).map(|g| g.0).collect::<Vec<_>>()
        };
        let before = order(&s);
        s.execute("text.convertListToText", &json!({"ids": [id]})).unwrap();
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
        assert_eq!(t.plain_text(), text);
        assert!(t.para_at(0).list.is_none() && t.para_at(1).list.is_none());
        let directions = (t.para_at(0).direction, t.para_at(1).direction);
        assert_eq!(order(&s), before, "{number}: the first line's glyphs from left to right");
        assert_eq!(directions, (direction, direction), "{number}");
    }
}

/// Convert to Text in threaded type writes the numbers the story counted through its frames,
/// and both frames draw every glyph where it was. A paragraph that continues into the next
/// frame gets its number once, where it starts, also when only the later frame is named, and
/// empty frames after a converted paragraph leave its list with it.
#[test]
fn threaded_numbers_convert_to_text_as_the_story_counted_them() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let plain = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("text"),
    };
    let numbers = |s: &Session, id: u64| {
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    let thread = |s: &mut Session, text: &str| {
        let a =
            s.execute("text.create", &json!({"x": 10, "y": 10, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
        (a, b)
    };
    // Whole paragraphs: the first frame ends with a paragraph break.
    let (a, b) = thread(&mut s, &(1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n"));
    let first = numbers(&s, a).len();
    assert!((2..10).contains(&first) && plain(&s, a).ends_with('\n'), "both frames hold items: {first}");
    let before = (drawn_glyphs(&s, a), drawn_glyphs(&s, b));
    let reply = s.execute("text.convertListToText", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(reply["converted"], 10, "each paragraph once");
    assert_eq!((drawn_glyphs(&s, a), drawn_glyphs(&s, b)), before, "both frames");
    let story = (1..=10).map(|i| format!("{i}.\tItem {i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(plain(&s, a) + &plain(&s, b), story);
    assert!(plain(&s, b).starts_with(&format!("{n}.\tItem {n}", n = first + 1)));
    // The empty part after the first frame's last break and the second frame's first item are
    // one paragraph: a caret at the end of the first frame converts that item.
    let (a, b) = thread(&mut s, &(1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n"));
    let end = plain(&s, a).len();
    let reply = s.execute("text.convertListToText", &json!({"ids": [a], "start": end, "end": end})).unwrap();
    assert_eq!(reply["converted"], 1);
    assert!(plain(&s, b).starts_with(&format!("{n}.\tItem {n}\nItem", n = first + 1)), "{}", plain(&s, b));
    // A paragraph split between the frames, with only the second frame named.
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(4);
    let (a, b) = thread(&mut s, &format!("One\nTwo\n{long}\nFour"));
    assert!(!plain(&s, a).ends_with('\n'), "the third paragraph goes on in the second frame");
    // A paragraph with a tab is broken line by line, so the lines are compared with the
    // Single-line Composer, which breaks the listed paragraph the same way.
    s.execute("text.setFormat", &json!({"ids": [a, b], "composer": "singleLine"})).unwrap();
    let before = (drawn_glyphs(&s, a), drawn_glyphs(&s, b));
    s.execute("text.convertListToText", &json!({"ids": [b]})).unwrap();
    assert_eq!((drawn_glyphs(&s, a), drawn_glyphs(&s, b)), before, "both frames, with the paragraph split between them");
    assert_eq!(plain(&s, a) + &plain(&s, b), format!("One\nTwo\n3.\t{long}\n4.\tFour"));
    assert_eq!(numbers(&s, a), ["1.", "2."], "the paragraphs of the first frame alone stay in the list");
    let Some(NodeKind::Text(second)) = s.doc().unwrap().doc.node(NodeId(b)).map(|n| &n.kind) else { panic!("text") };
    assert!(second.para_at(0).list.is_none(), "the third paragraph's part in the second frame leaves the list too");
    // One item that fits in the first of three frames: the two empty frames after it leave the
    // list too, so the re-flowed thread draws no number before the converted one.
    let a =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Item", "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 60})).unwrap()["id"].as_u64().unwrap();
    let c = s.execute("shape.rectangle", &json!({"x": 400, "y": 10, "width": 150, "height": 60})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b, c]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b, c], "list": "number"})).unwrap();
    assert_eq!((plain(&s, b), plain(&s, c), numbers(&s, a)), (String::new(), String::new(), vec!["1.".to_string()]));
    assert_eq!(s.execute("text.convertListToText", &json!({"ids": [a]})).unwrap()["converted"], 1);
    assert_eq!(plain(&s, a), "1.\tItem");
    assert_eq!(numbers(&s, a), Vec::<String>::new(), "no number drawn before the converted one");
    for id in [a, b, c] {
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
        assert!(t.para_at(0).list.is_none(), "frame {id} leaves the list");
    }
    // An overset 30 pt number, wider than the hanging indent, in a last frame that starts
    // inside a paragraph: its tab goes to the next stop, so it gets no stop at the left indent.
    let a = s
        .execute("text.create", &json!({"x": 10, "y": 10, "size": 30, "text": format!("{long}\nTwo"), "area": {"width": 150, "height": 80}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 80})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
    let Some(NodeKind::Text(second)) = s.doc().unwrap().doc.node(NodeId(b)).map(|n| &n.kind) else { panic!("text") };
    assert!(second.first_para_continues && second.plain_text().ends_with("\nTwo"), "{:?}", second.plain_text());
    assert_eq!(numbers(&s, b).len(), 0, "the second item is overset");
    s.execute("text.convertListToText", &json!({"ids": [b]})).unwrap();
    let Some(NodeKind::Text(second)) = s.doc().unwrap().doc.node(NodeId(b)).map(|n| &n.kind) else { panic!("text") };
    assert!(second.plain_text().ends_with("\n2.\tTwo"), "{:?}", second.plain_text());
    assert!(second.para_at(1).tabs.is_empty(), "no stop at the left indent: {:?}", second.para_at(1).tabs);
}

/// `text.convertListToText` sent while the Type tool edits (as from MCP or the control channel)
/// moves the tool's anchor and caret past the characters it inserts at or before them, so that
/// neither lands inside a bullet. In threaded type, when the text before the caret moves to the
/// frame before, the caret goes with its character into that frame, which becomes the
/// selection.
#[test]
fn convert_to_text_moves_the_type_tools_selection() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let plain = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("text"),
    };
    let selection = |s: &Session| {
        let o = s.tool_options();
        (o["editing"].as_u64(), o["anchor"].as_u64(), o["caret"].as_u64())
    };
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setFormat", &json!({"ids": [id], "list": "bullet"})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.select_tool("type", Default::default()).unwrap();
    // From after the O of One to after the T of Two.
    s.set_tool_option("textSelection", &json!({"editing": id, "anchor": 1, "caret": 5}));
    s.execute("text.convertListToText", &json!({})).unwrap();
    let text = plain(&s, id);
    assert_eq!(text, "\u{2022}\tOne\n\u{2022}\tTwo");
    assert_eq!(selection(&s), (Some(id), Some(5), Some(13)), "the anchor after the O, the caret after the T");
    assert!(text.is_char_boundary(5) && text.is_char_boundary(13));

    let long = "The quick brown fox jumps over the lazy dog. ".repeat(4);
    let a = s
        .execute("text.create", &json!({"x": 10, "y": 10, "text": format!("One\nTwo\n{long}\nFour"), "area": {"width": 150, "height": 60}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.select_tool("selection", Default::default()).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
    assert!(plain(&s, b).starts_with("dog. "), "the second frame starts with the dog: {:?}", plain(&s, b));
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.select_tool("type", Default::default()).unwrap();
    // The caret after the d of dog, at the start of the second frame.
    s.set_tool_option("textSelection", &json!({"editing": b, "anchor": 1, "caret": 1}));
    s.execute("text.convertListToText", &json!({"ids": [b]})).unwrap();
    let first = plain(&s, a);
    assert!(first.ends_with("lazy dog. "), "the dog moved to the first frame: {first:?}");
    let after_d = (first.len() - "og. ".len()) as u64;
    assert_eq!(selection(&s), (Some(a), Some(after_d), Some(after_d)), "the caret is after the d of dog in the first frame");
    assert_eq!(s.active().unwrap().selection.objects, vec![NodeId(a)], "the first frame is selected");
}

/// Type › Create Outlines makes a path of a list item's bullet as well as of its characters.
#[test]
fn create_outlines_makes_a_path_of_the_bullet() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let outlined = |s: &mut Session, list: bool| {
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "One"})).unwrap()["id"].as_u64().unwrap();
        if list {
            s.execute("text.setFormat", &json!({"ids": [id], "list": "bullet"})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        s.doc().unwrap().doc.node(NodeId(g)).unwrap().children().unwrap().len()
    };
    let plain = outlined(&mut s, false);
    assert_eq!(outlined(&mut s, true), plain + 1, "one more path: the bullet");
}

/// Paragraph panel menu › Justification…: `text.setFormat` sets each paragraph's word and
/// letter spacing, glyph scaling, Auto Leading and Single Word Justification, clamped to their
/// ranges, in one undo step; Minimum ≤ Desired ≤ Maximum must hold or nothing changes. They
/// are saved only when not the defaults, and paragraph styles carry them.
#[test]
fn justification_options_are_set_per_paragraph_and_saved_only_when_not_default() {
    use vectorcraft_doc::{JustificationOptions, SingleWordJustify};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "One\nTwo", "area": {"width": 200, "height": 100}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let paras = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (t.para_at(0).clone(), t.para_at(1).clone()),
        _ => panic!("text"),
    };
    assert_eq!(paras(&s).0.justification, JustificationOptions::default());
    assert!(serde_json::to_value(paras(&s).0).unwrap().get("justification").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute(
        "text.setFormat",
        &json!({
            "wordSpacing": [60, 90, 2000], "letterSpacing": {"max": 10}, "glyphScaling": [95, 100, 105],
            "autoLeading": 150, "singleWordJustification": "alignCenter", "start": 5, "end": 5
        }),
    )
    .unwrap();
    let want = JustificationOptions {
        word_spacing: [60.0, 90.0, 1000.0],
        letter_spacing: [0.0, 0.0, 10.0],
        glyph_scaling: [95.0, 100.0, 105.0],
        auto_leading: 150.0,
        single_word: SingleWordJustify::AlignCenter,
    };
    let (first, second) = paras(&s);
    assert_eq!((first.justification, &second.justification), (JustificationOptions::default(), &want), "only the paragraph the caret is in");
    assert_eq!(serde_json::to_value(&second).unwrap()["justification"]["single_word"], "alignCenter");
    let back: vectorcraft_doc::ParaStyle = serde_json::from_value(serde_json::to_value(&second).unwrap()).unwrap();
    assert_eq!(back.justification, want);
    // Out of order: nothing changes, and the error names the setting.
    let e = s.execute("text.setFormat", &json!({"wordSpacing": {"desired": 85}, "letterSpacing": {"min": 5}})).unwrap_err().to_string();
    assert!(e.contains("letterSpacing"), "{e}");
    assert_eq!(paras(&s).1.justification, want);
    assert!(s.execute("text.setFormat", &json!({"glyphScaling": [100, 100]})).is_err());
    assert!(s.execute("text.setFormat", &json!({"singleWordJustification": "spread"})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paras(&s).1.justification, JustificationOptions::default());
    // A paragraph style carries them; an older file's paragraph reads the defaults.
    s.execute("paraStyle.new", &json!({"name": "Airy", "attrs": {"justification": {"auto_leading": 200.0}}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Airy", "id": id})).unwrap();
    assert_eq!(paras(&s).0.justification, JustificationOptions { auto_leading: 200.0, ..JustificationOptions::default() });
    let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left", "hyphenate": true})).unwrap();
    assert_eq!(old.justification, JustificationOptions::default());
}

/// #432: while the interface is in Japanese, new type (text.create, text.createInPath) starts
/// with em box top-to-top leading and em box centre alignment, and new styles made from nothing
/// carry them; params override; the values are journaled; imported text keeps the Roman
/// baseline.
#[test]
fn new_type_takes_the_japanese_defaults_while_the_interface_is_japanese() {
    use vectorcraft_doc::{CharAlign, LeadingModel};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let made = |s: &mut Session, cmd: &str, p: Value| -> (LeadingModel, CharAlign) {
        let id = s.execute(cmd, &p).unwrap()["id"].as_u64().unwrap();
        match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (t.para.leading_model, t.runs[0].style.char_align),
            _ => panic!("text"),
        }
    };
    let roman = (LeadingModel::RomanBaseline, CharAlign::RomanBaseline);
    let japanese = (LeadingModel::EmBoxTop, CharAlign::EmBoxCenter);
    let point = json!({"x": 10, "y": 50, "text": "雅楽"});
    assert!(!s.japanese_interface());
    assert_eq!(made(&mut s, "text.create", point.clone()), roman, "headless, `auto`: the Roman defaults");
    s.ui_language = Some("ja".into());
    assert_eq!(made(&mut s, "text.create", point.clone()), japanese);
    assert_eq!(made(&mut s, "text.create", json!({"x": 10, "y": 90, "text": "笙", "area": {"width": 100, "height": 50}})), japanese);
    // Journaled: a replay in another language does the same.
    let (_, logged) = s.journal.last().unwrap().clone();
    assert_eq!((logged["leadingModel"].as_str(), logged["charAlign"].as_str()), (Some("emBoxTop"), Some("emBoxCenter")));
    // Params override.
    assert_eq!(
        made(&mut s, "text.create", json!({"x": 10, "y": 120, "text": "a", "leadingModel": "romanBaseline", "charAlign": "romanBaseline"})),
        roman
    );
    assert!(s.execute("text.create", &json!({"x": 0, "y": 0, "text": "a", "leadingModel": "middle"})).is_err());
    // Area type in a path.
    let path = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 100, "height": 80})).unwrap()["id"].clone();
    assert_eq!(made(&mut s, "text.createInPath", json!({"path": path, "mode": "area", "text": "篳篥"})), japanese);
    // A preference set to Japanese counts without a UI; another language doesn't.
    s.ui_language = None;
    s.prefs.interface_language = "ja".into();
    assert_eq!(made(&mut s, "text.create", point.clone()), japanese);
    s.ui_language = Some("en".into());
    assert_eq!(made(&mut s, "text.create", point.clone()), roman);
    // New styles made from nothing.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.ui_language = Some("ja".into());
    let style = |s: &mut Session, kind: &str| {
        let name = s.execute(&format!("{kind}.new"), &json!({})).unwrap()["name"].as_str().unwrap().to_string();
        let list = s.execute(&format!("{kind}.list"), &json!({})).unwrap();
        list["styles"].as_array().unwrap().iter().find(|st| st["name"] == name.as_str()).unwrap()["attrs"].clone()
    };
    assert_eq!(style(&mut s, "paraStyle"), json!({"leading_model": "emBoxTop"}));
    assert_eq!(style(&mut s, "charStyle"), json!({"charAlign": "emBoxCenter"}));
    s.ui_language = Some("en".into());
    assert_eq!(style(&mut s, "paraStyle"), json!({}));
    // Imported text keeps the Roman baseline.
    s.ui_language = Some("ja".into());
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text x="10" y="50">雅楽</text></svg>"#;
    s.execute("document.open", &json!({"name": "t.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let mut found = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            found.push((t.para.leading_model, t.runs[0].style.char_align));
        }
    });
    assert_eq!(found, [roman]);
}

#[test]
fn leading_model_is_set_per_paragraph_and_saved_only_when_top_to_top() {
    use vectorcraft_doc::LeadingModel;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    // Exercise ink bounds with bundled glyphs; Japanese outlines require optional craft-fonts.
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": "A\nB", "area": {"width": 200, "height": 100}})).unwrap()["id"].as_u64().unwrap();
    let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("text"),
    };
    assert_eq!(text(&s).para.leading_model, LeadingModel::RomanBaseline);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"leadingModel": "middle"})).is_err());
    let before = text(&s).cached_bounds;
    s.execute("text.setFormat", &json!({"leadingModel": "emBoxTop"})).unwrap();
    let t = text(&s);
    assert_eq!(t.para.leading_model, LeadingModel::EmBoxTop);
    // The fallback used without craft-fonts has no glyph metrics to move the first line.
    if !vectorcraft_text::CRAFT_FONTS.is_empty() {
        assert_ne!(t.cached_bounds, before, "the first line moves up to the frame's top");
    }
    assert_eq!(serde_json::to_value(&t.para).unwrap()["leading_model"], "emBoxTop");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(serde_json::to_value(&text(&s).para).unwrap().get("leading_model").is_none());
}

#[test]
fn character_alignment_is_a_character_attribute_saved_only_when_set() {
    use vectorcraft_doc::CharAlign;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "雅楽"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    assert_eq!(runs(&s)[0].style.char_align, CharAlign::RomanBaseline);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("charAlign").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"charAlign": "middle"})).is_err());
    s.execute("text.setFormat", &json!({"charAlign": "emBoxCenter"})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.char_align == CharAlign::EmBoxCenter));
    assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["charAlign"], "emBoxCenter");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(runs(&s)[0].style.char_align, CharAlign::RomanBaseline);
    // Characters selected with the Type tool: only the range takes it (雅 is 3 bytes).
    assert!(s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "charAlign": "top"})).is_err());
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "charAlign": "emBoxTop"})).unwrap();
    let aligns: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.char_align)).collect();
    assert_eq!(aligns, [("雅".to_string(), CharAlign::RomanBaseline), ("楽".to_string(), CharAlign::EmBoxTop)]);
    // The ideographic character face's top and bottom.
    for (key, want) in [("icfTop", CharAlign::IcfTop), ("icfBottom", CharAlign::IcfBottom)] {
        s.execute("text.setFormat", &json!({"charAlign": key})).unwrap();
        assert!(runs(&s).iter().all(|r| r.style.char_align == want));
        assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["charAlign"], key);
    }
}

/// Character Direction (Character panel menu, Middle Eastern options): a character attribute
/// that `text.setFormat` and `text.setRangeStyle` set in one undo step each, saved only when
/// set and kept by a native save.
#[test]
fn character_direction_is_a_character_attribute_saved_only_when_set() {
    use vectorcraft_doc::ParaDirection;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "שלום 21 10 2026"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    let set = |s: &Session| runs(s).iter().map(|r| (r.text.clone(), r.style.char_direction)).collect::<Vec<_>>();
    assert_eq!(runs(&s)[0].style.char_direction, None);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("charDirection").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (key, want) in [("rightToLeft", Some(ParaDirection::RightToLeft)), ("leftToRight", Some(ParaDirection::LeftToRight)), ("default", None)] {
        s.execute("text.setFormat", &json!({"charDirection": key})).unwrap();
        assert!(runs(&s).iter().all(|r| r.style.char_direction == want));
        let saved = serde_json::to_value(&runs(&s)[0].style).unwrap();
        assert_eq!(saved.get("charDirection").and_then(|v| v.as_str()), want.is_some().then_some(key));
    }
    let err = s.execute("text.setFormat", &json!({"charDirection": "up"})).unwrap_err().to_string();
    assert!(err.contains("`charDirection` must be"), "{err}");
    // Characters selected with the Type tool: only the range takes it (שלום and the space are 9 bytes).
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 9, "end": 19, "charDirection": "leftToRight"})).unwrap();
    assert_eq!(set(&s), [("שלום ".to_string(), None), ("21 10 2026".to_string(), Some(ParaDirection::LeftToRight))]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.char_direction.is_none()));
    s.execute("edit.redo", &json!({})).unwrap();
    // Saved and reopened as it was.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    let reopened: Vec<_> = t.runs.iter().map(|r| (r.text.clone(), r.style.char_direction)).collect();
    assert_eq!(reopened, set(&s));
    // A character style captured from the characters carries it, Default included.
    assert_eq!(super::super::textstyles::char_attrs(&runs(&s)[1].style)["charDirection"], "leftToRight");
    assert_eq!(super::super::textstyles::char_attrs(&runs(&s)[0].style).get("charDirection"), Some(&serde_json::Value::Null));
}

/// Proportional Metrics (#966): a character attribute, saved only when on.
#[test]
fn proportional_metrics_is_a_character_attribute_saved_only_when_on() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "雅楽"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    assert!(!runs(&s)[0].style.proportional_metrics);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("proportionalMetrics").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"proportionalMetrics": true})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.proportional_metrics));
    assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["proportionalMetrics"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!runs(&s)[0].style.proportional_metrics);
    // Characters selected with the Type tool: only the range takes it (雅 is 3 bytes).
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "proportionalMetrics": true})).unwrap();
    let on: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.proportional_metrics)).collect();
    assert_eq!(on, [("雅".to_string(), false), ("楽".to_string(), true)]);
    // Styles written without it (documents from before it) read as off.
    let old = serde_json::to_value(&runs(&s)[0].style).unwrap();
    assert!(old.get("proportionalMetrics").is_none());
    assert!(!serde_json::from_value::<vectorcraft_doc::CharStyle>(old).unwrap().proportional_metrics);
}

/// Tate-chu-yoko (Character panel menu, East Asian options): a character attribute, saved only
/// when on. Documents written before it read as off.
#[test]
fn tate_chu_yoko_is_a_character_attribute_saved_only_when_on() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "2026年"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    assert!(!runs(&s)[0].style.tate_chu_yoko);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("tateChuYoko").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"tateChuYoko": true})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.tate_chu_yoko));
    assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["tateChuYoko"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!runs(&s)[0].style.tate_chu_yoko);
    // The year only (2026 is 4 bytes); 年 stays unmarked.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 4, "tateChuYoko": true})).unwrap();
    let on: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.tate_chu_yoko)).collect();
    assert_eq!(on, [("2026".to_string(), true), ("年".to_string(), false)]);
    let old = serde_json::to_value(&runs(&s)[1].style).unwrap();
    assert!(old.get("tateChuYoko").is_none());
    assert!(!serde_json::from_value::<vectorcraft_doc::CharStyle>(old).unwrap().tate_chu_yoko);
    s.execute("text.setFormat", &json!({"tateChuYoko": false})).unwrap();
    assert!(runs(&s).iter().all(|r| !r.style.tate_chu_yoko));
}

/// No Break is a character attribute, saved only when on.
#[test]
fn no_break_is_a_character_attribute_saved_only_when_on() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    assert!(!runs(&s)[0].style.no_break);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("noBreak").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"noBreak": true})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.no_break));
    assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["noBreak"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!runs(&s)[0].style.no_break);
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 1, "noBreak": true})).unwrap();
    let on: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.no_break)).collect();
    assert_eq!(on, [("A".to_string(), true), ("b".to_string(), false)]);
    let old = serde_json::to_value(&runs(&s)[1].style).unwrap();
    assert!(old.get("noBreak").is_none());
    assert!(!serde_json::from_value::<vectorcraft_doc::CharStyle>(old).unwrap().no_break);
    s.execute("text.setFormat", &json!({"noBreak": false})).unwrap();
    assert!(runs(&s).iter().all(|r| !r.style.no_break));
}

/// Digits (Character panel menu, Middle Eastern options): a character attribute, saved only
/// when set; the text keeps the digits that were typed.
#[test]
fn digits_are_a_character_attribute_saved_only_when_set() {
    use vectorcraft_doc::Digits;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "صفحة 12"})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    assert_eq!(runs(&s)[0].style.digits, Digits::Default);
    assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("digits").is_none());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (key, want) in [("hindi", Digits::Hindi), ("farsi", Digits::Farsi), ("arabic", Digits::Arabic), ("default", Digits::Default)] {
        s.execute("text.setFormat", &json!({"digits": key})).unwrap();
        assert!(runs(&s).iter().all(|r| r.style.digits == want));
        let saved = serde_json::to_value(&runs(&s)[0].style).unwrap();
        assert_eq!(saved.get("digits").and_then(|v| v.as_str()), (want != Digits::Default).then_some(key));
        // The text is as typed: only the glyphs change.
        assert_eq!(runs(&s).iter().map(|r| r.text.as_str()).collect::<String>(), "صفحة 12");
    }
    s.execute("text.setFormat", &json!({"digits": "hindi"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(runs(&s)[0].style.digits, Digits::Default);
    let err = s.execute("text.setFormat", &json!({"digits": "roman"})).unwrap_err().to_string();
    assert!(err.contains("`digits` must be"), "{err}");
    // Characters selected with the Type tool: only the range takes it (صفحة is 8 bytes).
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 9, "end": 11, "digits": "farsi"})).unwrap();
    let set: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.digits)).collect();
    assert_eq!(set, [("صفحة ".to_string(), Digits::Default), ("12".to_string(), Digits::Farsi)]);
}

/// Variable Font settings (Character panel › Variable Font): a character attribute, saved only
/// when set, each value clamped to its axis's range, and cleared by another font or style.
#[test]
fn variable_font_settings_are_a_character_attribute_saved_only_when_set() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY as FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "lIn lIn", "font": FAMILY, "size": 40})).unwrap()["id"].as_u64().unwrap();
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("text"),
    };
    let saved = |s: &Session| serde_json::to_value(&runs(s)[0].style).unwrap().get("fontAxes").cloned();
    let width = |s: &Session| s.doc().unwrap().doc.bounds_of(&[NodeId(id)], false).unwrap().width();
    assert_eq!(saved(&s), None);
    let regular = width(&s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": 650}})).unwrap();
    assert_eq!(saved(&s), Some(json!({"wght": 650.0})));
    assert!(width(&s) > regular * 1.1, "the type is set heavier and wider: {regular} -> {}", width(&s));
    // Past the axis's range: its maximum; an axis the font hasn't: left out; others kept.
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": 2000, "opsz": 12, "wdth": 80}})).unwrap();
    assert_eq!(saved(&s), Some(json!({"wdth": 80.0, "wght": 700.0})));
    // Null goes back to the style's own setting, and so does a value equal to it.
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": null}})).unwrap();
    assert_eq!(saved(&s), Some(json!({"wdth": 80.0})));
    s.execute("text.setFormat", &json!({"fontAxes": {"wdth": 100}})).unwrap();
    assert_eq!(saved(&s), None);
    assert!((width(&s) - regular).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(saved(&s), Some(json!({"wdth": 80.0})));
    // Characters selected with the Type tool: only the range takes it.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 4, "end": 7, "fontAxes": {"wght": 550}})).unwrap();
    let set: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.font_axes.clone())).collect();
    let axes = |v: &[(&str, f64)]| v.iter().map(|(t, v)| (t.to_string(), *v)).collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(set, [("lIn ".to_string(), axes(&[("wdth", 80.0)])), ("lIn".to_string(), axes(&[("wdth", 80.0), ("wght", 550.0)]))]);
    // A saved file keeps them.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert_eq!(t.runs, runs(&s));
    // Another style or font clears them: their settings were the old face's.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 4, "end": 7, "style": "Bold"})).unwrap();
    assert_eq!(runs(&s).last().map(|r| (r.style.font_style.as_str(), r.style.font_axes.is_empty())), Some(("Bold", true)));
    s.execute("text.setStyle", &json!({"style": "Bold"})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.font_axes.is_empty()));
    s.execute("text.setFormat", &json!({"fontAxes": {"wdth": 75}})).unwrap();
    s.execute("text.replaceFont", &json!({"from": {"family": FAMILY}, "to": {"family": "Inter"}})).unwrap();
    assert!(runs(&s).iter().all(|r| r.style.font_axes.is_empty()));
    // A static font has no axes to set.
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": 650}})).unwrap();
    assert_eq!(saved(&s), None);
    // A missing font keeps them as given, for when it is installed.
    s.execute("text.setStyle", &json!({"font": "No Such Variable Sans"})).unwrap();
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": 2000}})).unwrap();
    assert_eq!(saved(&s), Some(json!({"wght": 2000.0})));
    for (bad, says) in
        [(json!({"weight": 650}), "not an axis tag"), (json!({"wght": "heavy"}), "must be a number"), (json!(650), "must be an object")]
    {
        let err = s.execute("text.setFormat", &json!({"fontAxes": bad})).unwrap_err().to_string();
        assert!(err.contains(says), "{err}");
    }
    // The axes and the setting of a style, as the sliders show them.
    let list = s.execute("text.fontList", &json!({"family": FAMILY, "style": "Bold"})).unwrap();
    assert_eq!(
        list["axes"],
        json!([
            {"tag": "wght", "name": "Weight", "min": 400.0, "default": 400.0, "max": 700.0, "value": 700.0, "hidden": false},
            {"tag": "wdth", "name": "Width", "min": 75.0, "default": 100.0, "max": 100.0, "value": 100.0, "hidden": false},
        ])
    );
    assert_eq!(s.execute("text.fontList", &json!({"family": "Inter"})).unwrap()["axes"], json!([]));
}

#[test]
fn new_type_is_composed_with_line_end_half_width_punctuation() {
    use vectorcraft_doc::Mojikumi;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "雅楽。"})).unwrap()["id"].as_u64().unwrap();
    let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para.clone(),
        _ => panic!("text"),
    };
    assert_eq!(para(&s).mojikumi, Mojikumi::LineEndHalf);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"mojikumi": "everything"})).is_err());
    s.execute("text.setFormat", &json!({"mojikumi": "none"})).unwrap();
    assert_eq!(para(&s).mojikumi, Mojikumi::None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(para(&s).mojikumi, Mojikumi::LineEndHalf);
    // Saved only when set; documents from before it read as None.
    let json = serde_json::to_value(para(&s)).unwrap();
    assert_eq!(json["mojikumi"], "lineEndHalf");
    let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
    assert_eq!(old.mojikumi, Mojikumi::None);
    assert!(serde_json::to_value(&old).unwrap().get("mojikumi").is_none());
}

#[test]
fn vertical_point_type_keeps_its_anchor_on_the_column_centre_line() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 100, "y": 50, "text": "§§§§ abc"})).unwrap()["id"].as_u64().unwrap();
    let bounds = |s: &Session| s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
    let wide = bounds(&s);
    assert!(wide.width() > wide.height());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("type.orientation.vertical", &json!({})).unwrap();
    let tall = bounds(&s);
    assert!(tall.height() > tall.width(), "{tall:?}");
    assert!((tall.center().x - 100.0).abs() < 6.0, "the anchor stays on the column's centre line: {tall:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(bounds(&s), wide);
}

#[test]
fn area_options_columns_change_layout() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let text = "word ".repeat(80);
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": text, "area": {"width": 300, "height": 120}})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    assert_eq!((q["rows"].as_u64(), q["columns"].as_u64()), (Some(1), Some(1)));
    let lay = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t),
        _ => panic!(),
    };
    assert_eq!(lay(&s).frames.len(), 1);
    let r = s.execute("text.areaOptions", &json!({"columns": 3, "gutter": 12, "inset": 4, "firstBaseline": "capHeight"})).unwrap();
    assert_eq!(r["firstBaseline"], "capHeight");
    let l = lay(&s);
    assert_eq!(l.frames.len(), 3, "three column cells");
    // Glyphs land in more than one column.
    let xs: Vec<f64> = l.glyphs.iter().map(|g| g.origin.x).collect();
    assert!(xs.iter().any(|x| *x > 110.0 + 10.0) && xs.iter().any(|x| *x < 100.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(lay(&s).frames.len(), 1);
    assert!(s.execute("text.areaOptions", &json!({"firstBaseline": "nope"})).is_err());
}
