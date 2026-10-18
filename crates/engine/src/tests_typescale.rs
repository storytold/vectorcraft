//! #1034: Character sizes and horizontal scales account for the object's affine transform.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextObject};
use vectorcraft_geom::Point;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn create(s: &mut Session, size: f64) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": 40, "y": 60, "text": "Axis label", "size": size})).unwrap()["id"].as_u64().unwrap())
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn em(t: &TextObject, run: usize) -> (f64, f64) {
    let st = &t.runs[run].style;
    let origin = t.xf * Point::ZERO;
    ((t.xf * Point::new(st.size * st.h_scale / 100.0, 0.0) - origin).hypot(), (t.xf * Point::new(0.0, st.size * st.v_scale / 100.0) - origin).hypot())
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn scaled_font_size_setter_sets_document_points_and_undoes() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("object.scale", &json!({"sx": 150})).unwrap();
    let before = text(&s, id);
    near(em(&before, 0).1, 36.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf, "font editing preserves placement and rotation");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(text(&s, id), after);
}

#[test]
fn group_font_size_setter_handles_different_accumulated_scales() {
    let mut s = session();
    let a = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let b = create(&mut s, 18.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    near(em(&text(&s, a), 0).1, 48.0);
    near(em(&text(&s, b), 0).1, 36.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    near(em(&text(&s, a), 0).1, 10.0);
    near(em(&text(&s, b), 0).1, 10.0);
}

#[test]
fn scaled_range_size_and_keyboard_steps_use_document_points() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 4, "size": 10})).unwrap();
    near(em(&text(&s, id), 0).1, 10.0);
    near(em(&text(&s, id), 1).1, 24.0);
    s.prefs.type_size_increment = 2.0;
    s.execute("type.step", &json!({"attribute": "size", "id": id.0, "start": 0, "end": 4})).unwrap();
    near(em(&text(&s, id), 0).1, 12.0);
    near(em(&text(&s, id), 1).1, 24.0);
}

#[test]
fn horizontal_scale_reset_removes_object_stretch_including_groups() {
    for grouped in [false, true] {
        let mut s = session();
        let id = create(&mut s, 12.0);
        if grouped {
            s.execute("object.group", &json!({})).unwrap();
        }
        s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
        let before = text(&s, id);
        s.execute("text.setFormat", &json!({"hScale": 100, "vScale": 100})).unwrap();
        let after = text(&s, id);
        near(em(&after, 0).0, 12.0);
        near(em(&after, 0).1, 12.0);
        assert_eq!(after.xf, before.xf);
    }
}

#[test]
fn rotated_reflected_nonuniform_range_edits_preserve_the_affine() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    // A quarter turn, reflected in the baseline axis: horizontal ×2, vertical ×3.
    s.execute("object.transform", &json!({"matrix": [0, -2, -3, 0, 0, 0]})).unwrap();
    let before = text(&s, id);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 10, "hScale": 100, "vScale": 100})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).0, 10.0);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf);
    // The native document representation retains both the compensation and the affine.
    let bytes = vectorcraft_format::save_file(&s.doc().unwrap().doc);
    let reopened = vectorcraft_format::load(&bytes).unwrap();
    let NodeKind::Text(t) = &reopened.node(id).unwrap().kind else { panic!("not text") };
    near(em(t, 0).0, 10.0);
    near(em(t, 0).1, 10.0);
}

#[test]
fn effective_range_query_leaves_raw_runs_available_for_clipboard() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 300, "sy": 200})).unwrap();
    let query = |s: &mut Session, effective| -> Value {
        s.execute("text.getRange", &json!({"id": id.0, "effective": effective})).unwrap()["runs"][0]["style"].clone()
    };
    let effective = query(&mut s, true);
    assert_eq!(effective["size"], 24.0);
    assert_eq!(effective["h_scale"], 150.0);
    let raw = query(&mut s, false);
    assert_eq!(raw["size"], 12.0);
    assert_eq!(raw["h_scale"], 100.0);
}

#[test]
fn scaled_point_units_and_small_sizes_are_converted_before_local_storage() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("text.setStyle", &json!({"size": 0.1, "leading": 14})).unwrap();
    let t = text(&s, id);
    near(em(&t, 0).1, 0.1);
    near(t.runs[0].style.leading.unwrap() * 2.0, 14.0);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 12, "baselineShift": 4, "leading": 16})).unwrap();
    let t = text(&s, id);
    near(t.runs[0].style.baseline_shift * 2.0, 4.0);
    near(t.runs[0].style.leading.unwrap() * 2.0, 16.0);
}

#[test]
fn singular_text_size_edit_returns_an_error_and_rolls_back_the_group() {
    let mut s = session();
    let a = create(&mut s, 12.0);
    let b = create(&mut s, 18.0);
    s.execute("object.transform", &json!({"matrix": [0, 0, 0, 0, 0, 0]})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    assert!(s.execute("text.setStyle", &json!({"size": 10})).is_err());
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert!(s.execute("text.setRangeStyle", &json!({"id": b.0, "size": 10})).is_err());
    assert!(s.execute("text.setFormat", &json!({"id": b.0, "hScale": 100})).is_err());
    // Non-dimensional edits remain possible, including on imported collapsed text.
    s.execute("text.setStyle", &json!({"fill": "#123456"})).unwrap();
}

#[test]
fn sheared_text_keeps_its_affine_and_accepts_exact_em_dimensions() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [2, 0, 1, 2, 0, 0]})).unwrap();
    let before = text(&s, id);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 10, "hScale": 100})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).0, 10.0);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf);
}

#[test]
fn area_frame_resize_reflows_at_same_size_and_object_scaling_reports_scaled_size() {
    let mut s = session();
    let id = NodeId(
        s.execute("text.create", &json!({"x": 40, "y": 60, "text": "Area label", "size": 12, "area": {"width": 120, "height": 80}})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("object.transform", &json!({"matrix": [2, 0, 0, 2, 0, 0], "typeAreas": true})).unwrap();
    near(em(&text(&s, id), 0).1, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    near(em(&text(&s, id), 0).1, 24.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    near(em(&text(&s, id), 0).1, 10.0);
}

#[test]
fn point_based_keyboard_increments_and_format_fields_follow_document_units() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("text.setStyle", &json!({"leading": 14})).unwrap();
    s.execute("text.setFormat", &json!({"baselineShift": 4})).unwrap();
    s.prefs.type_size_increment = 2.0;
    s.prefs.baseline_shift_increment = 3.0;
    s.execute("type.step", &json!({"attribute": "leading"})).unwrap();
    s.execute("type.step", &json!({"attribute": "baselineShift"})).unwrap();
    let t = text(&s, id);
    near(t.runs[0].style.leading.unwrap() * 2.0, 16.0);
    near(t.runs[0].style.baseline_shift * 2.0, 7.0);
}

#[test]
fn tiny_finite_transform_rejects_unsafe_compensation_without_history_changes() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [1e-306, 0, 0, 1e-306, 0, 0]})).unwrap();
    let before = text(&s, id);
    let history = s.doc().unwrap().history.undo.len();
    let error = s.execute("text.setStyle", &json!({"size": 160})).unwrap_err();
    assert!(matches!(error, EngineError::BadParams { .. }), "reject before attempting unsafe layout: {error}");
    assert_eq!(text(&s, id), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
}

/// A file whose type holds a size or scale of 0 or less or a negative leading opens with the
/// default size and scale and Auto leading, and a size, leading, baseline shift or scale past
/// `MAX_COORD` opens with `MAX_COORD`, in the layers, in symbols and in character styles. A
/// leading of 0, a size below 0.1 pt and scales below 1% stay as saved.
#[test]
fn a_file_with_type_sizes_out_of_range_opens_with_them_in_range() {
    let mut s = session();
    let p = json!({"x": 40, "y": 60, "size": 20, "text": "Line one\nLine two\nLine three"});
    let id = NodeId(s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 9, "end": 18, "size": 30})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 18, "end": 28, "size": 40})).unwrap();
    let mut d = (*s.doc().unwrap().doc).clone();
    let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("not text") };
    let [zero, huge, kept] = t.runs.as_mut_slice() else { panic!("not three runs") };
    let (z, h, k) = (&mut zero.style, &mut huge.style, &mut kept.style);
    (z.size, z.h_scale, z.v_scale, z.baseline_shift, z.leading) = (0.0, -100.0, 0.0, -f64::MAX, Some(-50.0));
    (h.size, h.h_scale, h.v_scale, h.baseline_shift, h.leading) = (f64::MAX, f64::MAX, f64::MAX, f64::MAX, Some(f64::MAX));
    (k.size, k.h_scale, k.v_scale, k.baseline_shift, k.leading) = (0.001, 0.5, 0.5, 5000.0, Some(0.0));
    let art = d.node(id).unwrap().clone();
    d.symbols.push(vectorcraft_doc::Symbol { name: "Lines".into(), art: Arc::new(art) });
    let attrs = json!({"size": 0, "h_scale": 1e9, "v_scale": -1, "baseline_shift": -1e9, "leading": -1});
    d.char_styles.push(vectorcraft_doc::TextStyleDef { name: "Broken".into(), attrs: attrs.as_object().unwrap().clone() });
    let attrs = json!({"size": 40, "leading": null});
    d.char_styles.push(vectorcraft_doc::TextStyleDef { name: "Auto".into(), attrs: attrs.as_object().unwrap().clone() });
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "sizes.vectorcraft", "dataBase64": data})).unwrap();
    let dims = |t: &TextObject| {
        t.runs.iter().map(|r| (r.style.size, r.style.h_scale, r.style.v_scale, r.style.baseline_shift, r.style.leading)).collect::<Vec<_>>()
    };
    let max = vectorcraft_doc::MAX_COORD;
    let want = vec![(12.0, 100.0, 100.0, -max, None), (max, max, max, max, Some(max)), (0.001, 0.5, 0.5, 5000.0, Some(0.0))];
    assert_eq!(dims(&text(&s, id)), want);
    let doc = &s.doc().unwrap().doc;
    let NodeKind::Text(art) = &doc.symbols[0].art.kind else { panic!("not text") };
    assert_eq!(dims(art), want);
    let fixed = json!({"size": 12.0, "h_scale": max, "v_scale": 100.0, "baseline_shift": -max, "leading": null});
    assert_eq!(doc.char_styles[0].attrs, *fixed.as_object().unwrap());
    assert_eq!(doc.char_styles[1].attrs, *json!({"size": 40, "leading": null}).as_object().unwrap());
    // With the sizes kept as saved, the type's bounds were infinite.
    let b = doc.bounds_of(&[id], false).unwrap();
    assert!(b.width().is_finite() && b.height().is_finite(), "{b:?}");
}

/// SVG import keeps type sizes outside the Character panel's 0.1 to 1296 pt, and it gives a line
/// aligned differently from the one before it an indent, here past the Paragraph panel's 1296 pt.
/// A native file of the result opens with the same type.
#[test]
fn imported_type_with_sizes_outside_the_panels_range_opens_unchanged() {
    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="3000">
        <text x="10" y="2000" font-size="2000">Big</text>
        <text x="10" y="2400" font-size="0.05">tiny</text>
        <text x="200" y="2600" font-size="12" text-anchor="middle"><tspan x="200" y="2600">One</tspan><tspan x="2900" y="2614.4" text-anchor="start">Two</tspan></text>
    </svg>"#;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "sizes.svg", "dataBase64": vectorcraft_format::base64_encode(SVG.as_bytes())})).unwrap();
    let texts = |s: &Session| {
        let mut out = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                out.push((**t).clone());
            }
        });
        out
    };
    let imported = texts(&s);
    let sizes: Vec<f64> = imported.iter().flat_map(|t| t.runs.iter().map(|r| r.style.size)).collect();
    assert!(sizes.iter().any(|v| *v > 1296.0) && sizes.iter().any(|v| *v < 0.1), "{sizes:?}");
    assert!(imported.iter().flat_map(|t| t.para_styles()).any(|p| p.left_indent > 1296.0), "an indent past the panel's");
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    s.execute("document.open", &json!({"name": "sizes.vectorcraft", "dataBase64": saved})).unwrap();
    assert_eq!(texts(&s), imported);
}
