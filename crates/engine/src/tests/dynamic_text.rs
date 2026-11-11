//! Type › Dynamic Text (`type.dynamicText`): type on a path scaled by one factor to fill the span
//! between its brackets, fitted again after each edit, and kept at its drawn sizes when it is
//! turned off or threaded.
//! <https://helpx.adobe.com/photoshop/desktop/text-typography/work-with-dynamic-text/dynamic-text-overview.html>

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TextObject};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// 12 pt type on the 400 pt line from (100, 300) to (500, 300), from its start, selected.
fn on_line(s: &mut Session, text: &str) -> NodeId {
    let pid = s.execute("path.create", &json!({ "d": "M100 300 L500 300" })).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": text, "size": 12, "at": [100, 300]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

fn fit(s: &Session, id: NodeId) -> f64 {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id)).fit_scale
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn err(r: Result<Value>) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn dynamic_text_turns_on_and_off_and_toggles() {
    let mut s = session();
    let id = on_line(&mut s, "Grow");
    let before = undo_len(&s);
    let r = s.execute("type.dynamicText", &json!({})).unwrap();
    assert_eq!((r["on"].clone(), r["ids"].clone()), (json!(true), json!([id.0])));
    let f = r["fitScale"].as_f64().unwrap();
    assert!(f > 2.0, "short text on a long path grows: {f}");
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Dynamic Text");
    assert!(text(&s, id).dynamic_text);
    // The bounds the engine keeps are the fitted layout's.
    let t = text(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert_eq!(t.cached_bounds, Some(lay.bounds));
    // Already on: nothing changes and no undo step is recorded.
    let r = s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_eq!(r["on"], json!(true));
    assert_eq!(undo_len(&s), before + 1);
    // Toggled off: the text keeps the sizes it was drawn at, in one undo step.
    let r = s.execute("type.dynamicText", &json!({})).unwrap();
    assert_eq!((r["on"].clone(), r["fitScale"].clone()), (json!(false), json!(1.0)));
    assert_eq!(undo_len(&s), before + 2);
    let off = text(&s, id);
    assert!(!off.dynamic_text);
    assert!((off.runs[0].style.size - 12.0 * f).abs() < 1e-9, "{} vs {}", off.runs[0].style.size, 12.0 * f);
    // Undo brings Dynamic Text back with the stored size.
    s.execute("edit.undo", &json!({})).unwrap();
    let back = text(&s, id);
    assert!(back.dynamic_text && back.runs[0].style.size == 12.0);
    let r = s.execute("type.dynamicText", &json!({"on": false})).unwrap();
    assert_eq!(r["on"], json!(false));
    let r = s.execute("type.dynamicText", &json!({"on": false})).unwrap();
    assert_eq!(r["on"], json!(false));
}

#[test]
fn dynamic_text_refuses_bad_params_and_other_type() {
    let mut s = session();
    let id = on_line(&mut s, "Grow");
    let before = undo_len(&s);
    assert!(err(s.execute("type.dynamicText", &json!({"on": "yes"}))).contains("on"));
    assert!(err(s.execute("type.dynamicText", &json!({"on": 1}))).contains("on"));
    assert_eq!(undo_len(&s), before);
    assert!(!text(&s, id).dynamic_text);
    // Point type and area type alone: refused.
    let point = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    let area =
        s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Area", "area": {"width": 100, "height": 50}})).unwrap()["id"].as_u64().unwrap();
    for other in [point, area] {
        s.execute("select.set", &json!({"ids": [other]})).unwrap();
        assert!(err(s.execute("type.dynamicText", &json!({}))).contains("select type on a path"));
    }
    let empty = s.execute("shape.rectangle", &json!({"x": 300, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [empty]})).unwrap();
    assert!(s.execute("type.dynamicText", &json!({})).is_err());
    // With point type selected too, only the type on a path changes.
    let before = undo_len(&s);
    s.execute("select.set", &json!({"ids": [point, id.0]})).unwrap();
    let r = s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert!(!text(&s, NodeId(point)).dynamic_text);
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn dynamic_text_fits_again_after_each_edit() {
    let mut s = session();
    let id = on_line(&mut s, "Grow");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let start = fit(&s, id);
    // Longer text draws smaller.
    s.execute("text.setText", &json!({"ids": [id.0], "text": "Grow and grow again"})).unwrap();
    let longer = fit(&s, id);
    assert!(longer < start * 0.5, "{longer} vs {start}");
    // An end bracket nearer the start draws it smaller still.
    s.execute("type.pathOptions", &json!({"ids": [id.0], "end": 0.5})).unwrap();
    let half = fit(&s, id);
    assert!(half < longer * 0.6, "{half} vs {longer}");
    s.execute("type.pathOptions", &json!({"ids": [id.0], "end": null})).unwrap();
    // A longer path draws it larger.
    s.execute("text.reshapePath", &json!({"id": id.0, "anchors": [[0, 1]], "dx": 200, "dy": 0})).unwrap();
    let reshaped = fit(&s, id);
    assert!(reshaped > longer * 1.3, "{reshaped} vs {longer}");
    // Each fit keeps the cached bounds in step with the layout.
    let t = text(&s, id);
    assert_eq!(t.cached_bounds, Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds));
    // Scaling the object: the fit is in document points, so the drawn size stays.
    let drawn = |s: &Session| {
        let t = text(s, id);
        t.runs[0].style.size * t.style_scale().unwrap().points * fit(s, id)
    };
    let size = drawn(&s);
    s.execute("object.scale", &json!({"ids": [id.0], "sx": 50, "sy": 50})).unwrap();
    let smaller = drawn(&s);
    assert!(smaller < size * 0.6, "the path is half as long: {smaller} vs {size}");
    assert!(!vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(&s, id)).overflow);
}

#[test]
fn threading_turns_dynamic_text_off_at_its_drawn_sizes() {
    let mut s = session();
    let id = on_line(&mut s, "Threaded story");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let f = fit(&s, id);
    let frame = s.execute("shape.rectangle", &json!({"x": 100, "y": 400, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id.0, frame]})).unwrap();
    let before = undo_len(&s);
    let r = s.execute("text.thread.create", &json!({})).unwrap();
    assert_eq!(r["dynamicTextOff"], json!([id.0]));
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let t = text(&s, id);
    assert!(!t.dynamic_text);
    assert!((t.runs[0].style.size - 12.0 * f).abs() < 1e-9, "{} vs {}", t.runs[0].style.size, 12.0 * f);
    // Undo restores Dynamic Text and the stored size together.
    s.execute("edit.undo", &json!({})).unwrap();
    let t = text(&s, id);
    assert!(t.dynamic_text && t.runs[0].style.size == 12.0);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(!text(&s, id).dynamic_text);
    // Type on a path in a thread is refused by id, alone or with other type on a path.
    let e = err(s.execute("type.dynamicText", &json!({"on": true, "ids": [id.0]})));
    assert!(e.contains("in a thread") && e.contains(&id.0.to_string()), "{e}");
    let free = on_line(&mut s, "Free");
    let e = err(s.execute("type.dynamicText", &json!({"on": true, "ids": [free.0, id.0]})));
    assert!(e.contains("in a thread") && e.contains(&id.0.to_string()), "{e}");
    assert!(!text(&s, id).dynamic_text && !text(&s, free).dynamic_text);
    // A thread made without any Dynamic Text says nothing about it.
    let a = on_line(&mut s, "Plain");
    let b = s.execute("shape.rectangle", &json!({"x": 400, "y": 400, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b]})).unwrap();
    let r = s.execute("text.thread.create", &json!({})).unwrap();
    assert!(r.get("dynamicTextOff").is_none(), "{r}");
}

#[test]
fn text_thread_add_turns_dynamic_text_off() {
    let mut s = session();
    let id = on_line(&mut s, "Threaded story");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let f = fit(&s, id);
    let r = s.execute("text.thread.add", &json!({"from": id.0, "rect": [100, 400, 200, 100]})).unwrap();
    assert_eq!(r["dynamicTextOff"], json!([id.0]));
    let t = text(&s, id);
    assert!(!t.dynamic_text && (t.runs[0].style.size - 12.0 * f).abs() < 1e-9);
    let added = text(&s, NodeId(r["id"].as_u64().unwrap()));
    assert!(!added.dynamic_text);
}

#[test]
fn dynamic_text_is_saved_only_when_on_and_reads_back() {
    let mut s = session();
    let id = on_line(&mut s, "Saved");
    let data =
        |s: &mut Session| s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let off = data(&mut s);
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let f = fit(&s, id);
    let on = data(&mut s);
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "on.vectorcraft", "dataBase64": on})).unwrap();
    let t = first_text(&o);
    assert!(text(&o, t).dynamic_text);
    assert_eq!(text(&o, t).runs[0].style.size, 12.0);
    assert_eq!(fit(&o, t), f);
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "off.vectorcraft", "dataBase64": off})).unwrap();
    let t = first_text(&o);
    assert!(!text(&o, t).dynamic_text);
    // The document summary reports the setting and the factor.
    let d = s.execute("document.inspect", &json!({})).unwrap().to_string();
    assert!(d.contains("\"dynamicText\":true"), "{d}");
}

#[test]
fn svg_export_writes_the_drawn_size() {
    let mut s = session();
    let id = on_line(&mut s, "Grow");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let size = 12.0 * fit(&s, id);
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains("<textPath"), "{svg}");
    let sizes: Vec<f64> = svg
        .match_indices("font-size")
        .filter_map(|(at, m)| {
            let rest = svg[at + m.len()..].trim_start_matches(['=', ':', '"', ' ']);
            let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
            rest[..end].parse().ok()
        })
        .collect();
    assert!(sizes.iter().any(|v| (v - size).abs() < 1e-3), "{size} in {sizes:?}");
    assert!(!sizes.iter().any(|v| (v - 12.0).abs() < 1e-3), "the stored size is not written: {sizes:?}");
}

#[test]
fn release_type_on_a_path_keeps_the_drawn_size() {
    let mut s = session();
    let id = on_line(&mut s, "Release");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let size = 12.0 * fit(&s, id);
    assert!(size > 24.0, "the text grows on the long line: {size}");
    let r = s.execute("addon.type.releasePath", &json!({"ids": [id.0], "mode": "text"})).unwrap();
    let released = NodeId(r["texts"][0].as_u64().unwrap());
    let t = text(&s, released);
    assert!(!t.dynamic_text, "point type does not keep Dynamic Text");
    assert!((t.runs[0].style.size - size).abs() < 1e-9, "{} vs the drawn {size}", t.runs[0].style.size);
    assert_eq!(fit(&s, released), 1.0);
    // Type on a path without Dynamic Text keeps its stored size.
    let plain = on_line(&mut s, "Plain");
    let r = s.execute("addon.type.releasePath", &json!({"ids": [plain.0], "mode": "text"})).unwrap();
    let t = text(&s, NodeId(r["texts"][0].as_u64().unwrap()));
    assert!(!t.dynamic_text && t.runs[0].style.size == 12.0);
}

/// The id of the first text object in `s`'s document.
fn first_text(s: &Session) -> NodeId {
    let mut found = None;
    s.doc().unwrap().doc.walk(|n| {
        if found.is_none() && matches!(n.kind, NodeKind::Text(_)) {
            found = Some(n.id);
        }
    });
    found.unwrap()
}
