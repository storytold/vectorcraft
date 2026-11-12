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
    // Point type alone: refused (area type takes Block, `typecmd::block_tests`).
    let point = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [point]})).unwrap();
    assert!(err(s.execute("type.dynamicText", &json!({}))).contains("select type on a path or area type"));
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

/// 12 pt point type at (100, 100), selected, with its layout bounds.
fn point(s: &mut Session, content: &str) -> (NodeId, vectorcraft_geom::Rect) {
    let r = s.execute("text.create", &json!({"x": 100, "y": 100, "text": content, "size": 12})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let b = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id)).bounds;
    (id, b)
}

/// Type on a path's path, start bracket and end bracket.
fn on_path(t: &TextObject) -> (vectorcraft_geom::PathData, f64, Option<f64>) {
    match &t.kind {
        vectorcraft_doc::TextKind::OnPath { path, start, end } => (path.clone(), *start, *end),
        k => panic!("not type on a path: {k:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn presets_put_point_type_on_a_fitted_circle_arch_or_bow() {
    let mut s = session();
    let (id, b) = point(&mut s, "Round and round");
    let w = b.width().max(b.height());
    let before = undo_len(&s);
    let r = s.execute("type.dynamicTextPreset", &json!({"preset": "circle"})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Dynamic Text");
    let t = text(&s, id);
    assert!(t.dynamic_text);
    assert_eq!(t.path_align, vectorcraft_doc::PathAlign::Descender, "above by default");
    let (path, start, end) = on_path(&t);
    assert!(path.is_closed());
    assert_eq!((start, end), (0.75, None));
    let pb = path.bounds().unwrap();
    assert!(close(pb.width(), w) && close(pb.height(), w), "{pb:?} vs {w}");
    assert!(close(pb.center().x, b.center().x) && close(pb.center().y, b.center().y), "{pb:?} vs {b:?}");
    // The start bracket, 3/4 of the way round, is the circle's bottom.
    let bottom = path.subpaths[0].anchors[3].p;
    assert!(close(bottom.y, pb.y1) && close(bottom.x, pb.center().x), "{bottom:?}");
    // Round a circle as wide as the text, the text grows to fill it and does not overflow.
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.fit_scale > 2.0 && !lay.overflow, "{} {}", lay.fit_scale, lay.overflow);
    assert_eq!(r["fitScale"], json!(lay.fit_scale));
    assert_eq!(t.cached_bounds, Some(lay.bounds));
    // Undo gives the point type back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(text(&s, id).kind, vectorcraft_doc::TextKind::Point) && !text(&s, id).dynamic_text);

    for (preset, up) in [("arch", true), ("bow", false)] {
        s.execute("type.dynamicTextPreset", &json!({"preset": preset, "ids": [id.0]})).unwrap();
        let t = text(&s, id);
        let (path, start, end) = on_path(&t);
        assert!(!path.is_closed() && t.dynamic_text);
        assert_eq!((start, end), (0.0, None));
        let a = &path.subpaths[0].anchors;
        let (first, middle, last) = (a[0].p, a[1].p, a[a.len() - 1].p);
        // Left to right, the chord as wide as the text, the arc rising (or falling) half its radius.
        assert!(first.x < last.x && close(last.x - first.x, w) && close(first.y, last.y), "{preset}: {first:?} {last:?}");
        let pb = path.bounds().unwrap();
        assert!(close(pb.height(), w / 3f64.sqrt() / 2.0), "{preset}: {pb:?}");
        assert!(close(pb.center().y, b.center().y), "{preset}: {pb:?} vs {b:?}");
        assert_eq!(middle.y < first.y, up, "{preset}: {middle:?} {first:?}");
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        assert!(lay.fit_scale > 1.0 && !lay.overflow, "{preset}: {}", lay.fit_scale);
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn preset_position_and_flip_set_align_to_path_and_direction() {
    let mut s = session();
    let (id, _) = point(&mut s, "Over the hill");
    for (position, align) in [
        ("above", vectorcraft_doc::PathAlign::Descender),
        ("center", vectorcraft_doc::PathAlign::Center),
        ("below", vectorcraft_doc::PathAlign::Ascender),
    ] {
        s.execute("type.dynamicTextPreset", &json!({"preset": "arch", "position": position})).unwrap();
        assert_eq!(text(&s, id).path_align, align, "{position}");
    }
    // Flipped, the arch runs right to left and the text stands on its other side.
    s.execute("type.dynamicTextPreset", &json!({"preset": "arch", "flip": true})).unwrap();
    let t = text(&s, id);
    let (path, start, end) = on_path(&t);
    let a = &path.subpaths[0].anchors;
    assert!(a[0].p.x > a[a.len() - 1].p.x, "{path:?}");
    assert_eq!((start, end), (0.0, None));
    assert_eq!(t.path_align, vectorcraft_doc::PathAlign::Descender);
    assert!(!vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow);
    // A flipped circle runs the other way round from its bottom.
    s.execute("type.dynamicTextPreset", &json!({"preset": "circle", "flip": true})).unwrap();
    let (path, start, end) = on_path(&text(&s, id));
    assert_eq!((start, end), (0.25, None));
    let at = path.subpaths[0].anchors[1].p;
    assert!(close(at.y, path.bounds().unwrap().y1), "reversed, the bottom is a quarter of the way: {at:?}");
    // The effect and Spacing of Type on a Path Options stay.
    s.execute("type.pathOptions", &json!({"effect": "skew", "spacing": 5})).unwrap();
    s.execute("type.dynamicTextPreset", &json!({"preset": "bow"})).unwrap();
    let t = text(&s, id);
    assert_eq!((t.path_effect.id(), t.path_spacing), ("skew", 5.0));
}

#[test]
fn presets_fit_type_on_a_path_to_its_path_and_record_nothing_when_nothing_changes() {
    let mut s = session();
    let id = on_line(&mut s, "Along the line");
    s.execute("type.dynamicTextPreset", &json!({"preset": "bow"})).unwrap();
    let (path, ..) = on_path(&text(&s, id));
    let a = &path.subpaths[0].anchors;
    // The bow is as wide as the 400 pt line, centered where it was.
    assert!(close(a[0].p.x, 100.0) && close(a[a.len() - 1].p.x, 500.0), "{a:?}");
    assert!(close(path.bounds().unwrap().center().y, 300.0));
    // The same preset again: nothing changes and no undo step is recorded.
    let before = undo_len(&s);
    let r = s.execute("type.dynamicTextPreset", &json!({"preset": "bow"})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(undo_len(&s), before);
    // Another position is a change.
    s.execute("type.dynamicTextPreset", &json!({"preset": "bow", "position": "below"})).unwrap();
    assert_eq!(undo_len(&s), before + 1);
    // The fit follows later edits to the text.
    let f = fit(&s, id);
    s.execute("text.setText", &json!({"ids": [id.0], "text": "Along the line and on past it"})).unwrap();
    assert!(fit(&s, id) < f * 0.8, "{} vs {f}", fit(&s, id));
    // On a rotated object the arch still bulges up the page, and on a squashed one the circle is
    // round on the page.
    let (turned, _) = point(&mut s, "Turned");
    s.execute("object.rotate", &json!({"ids": [turned.0], "angle": 45})).unwrap();
    s.execute("type.dynamicTextPreset", &json!({"preset": "arch", "ids": [turned.0]})).unwrap();
    let t = text(&s, turned);
    let page = on_path(&t).0.transformed(t.xf);
    let a = &page.subpaths[0].anchors;
    assert!(close(a[0].p.y, a[2].p.y) && a[1].p.y < a[0].p.y && a[0].p.x < a[2].p.x, "{a:?}");
    // The same preset again leaves the rotated arch as it is.
    let before = undo_len(&s);
    s.execute("type.dynamicTextPreset", &json!({"preset": "arch", "ids": [turned.0]})).unwrap();
    assert_eq!(undo_len(&s), before);
    let (squashed, _) = point(&mut s, "Squashed");
    s.execute("object.scale", &json!({"ids": [squashed.0], "sx": 200, "sy": 50})).unwrap();
    s.execute("type.dynamicTextPreset", &json!({"preset": "circle", "ids": [squashed.0]})).unwrap();
    let t = text(&s, squashed);
    let pb = on_path(&t).0.transformed(t.xf).bounds().unwrap();
    assert!(close(pb.width(), pb.height()), "{pb:?}");
    // Empty 12 pt point type gets a circle four ems (48 pt) across.
    let (empty, _) = point(&mut s, "");
    s.execute("type.dynamicTextPreset", &json!({"preset": "circle"})).unwrap();
    let (path, ..) = on_path(&text(&s, empty));
    let pb = path.bounds().unwrap();
    assert!(close(pb.width(), 48.0) && close(pb.height(), 48.0), "{pb:?}");
}

#[test]
fn presets_refuse_bad_params_area_type_and_threads() {
    let mut s = session();
    let (id, _) = point(&mut s, "Point");
    let before = undo_len(&s);
    for p in [
        json!({}),
        json!({"preset": "square"}),
        json!({"preset": 1}),
        json!({"preset": "arch", "position": "top"}),
        json!({"preset": "arch", "position": true}),
        json!({"preset": "arch", "flip": "yes"}),
    ] {
        let e = err(s.execute("type.dynamicTextPreset", &p));
        let key = ["position", "flip"].into_iter().find(|k| p.get(k).is_some()).unwrap_or("preset");
        assert!(e.contains(key), "{p}: {e}");
    }
    assert_eq!(undo_len(&s), before);
    assert!(matches!(text(&s, id).kind, vectorcraft_doc::TextKind::Point));
    // Area type alone is refused; beside point type it is left as it is.
    let area =
        s.execute("text.create", &json!({"x": 10, "y": 300, "text": "Area", "area": {"width": 100, "height": 50}})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [area]})).unwrap();
    assert!(err(s.execute("type.dynamicTextPreset", &json!({"preset": "circle"}))).contains("select point type or type on a path"));
    s.execute("select.set", &json!({"ids": [area, id.0]})).unwrap();
    let r = s.execute("type.dynamicTextPreset", &json!({"preset": "circle"})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert!(matches!(text(&s, NodeId(area)).kind, vectorcraft_doc::TextKind::Area { .. }) && !text(&s, NodeId(area)).dynamic_text);
    // Type on a path in a thread is refused by id.
    let threaded = on_line(&mut s, "Threaded story");
    let frame = s.execute("shape.rectangle", &json!({"x": 100, "y": 400, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [threaded.0, frame]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    let before = undo_len(&s);
    let e = err(s.execute("type.dynamicTextPreset", &json!({"preset": "arch", "ids": [threaded.0, id.0]})));
    assert!(e.contains("in a thread") && e.contains(&threaded.0.to_string()), "{e}");
    assert_eq!(undo_len(&s), before);
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
