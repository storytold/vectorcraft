//! Width Point Edit: double-clicking a stroke with the Width tool (or `ui.widthPointEdit`)
//! creates or edits a point and its side widths. Total Width changes both sides in proportion,
//! the link keeps them equal,
//! and Adjust Adjoining Width Points changes the nearest points either side in proportion too.
//! Shift-double-click on a width point opens the dialog with that option enabled.
//! OK runs `stroke.widthPoint.set`; Delete removes an existing point (`stroke.widthPoint.remove`)
//! or discards a prospective point without changing the document.
//!
//! Fields: `id`, `index` (an existing width point, otherwise null), `t` (its place), `side1`,
//! `side2` (the left and right widths, points), `linked`, `adjustAdjoining`; Delete sets `discard: true` and confirms.

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_engine::doc::NodeId;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Width Point Edit.
pub const KIND: &str = "widthPoint";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Width Point Edit").into(), body, confirm, discard: Some("Delete"), min_width: 300.0, ..DialogSpec::FORM };

/// Width point `index` of `id`: its place and side widths (points).
fn point(app: &VectorcraftApp, id: u64, index: usize) -> Option<(f64, f64, f64)> {
    let st = app.session.active()?.doc.node(NodeId(id))?.appearance.stroke()?;
    let half = st.width / 2.0;
    st.profile.as_ref()?.points.get(index).map(|&(t, l, r)| (t, l * half, r * half))
}

/// Open Width Point Edit for `index`, or a prospective point at fraction `t` without an index
/// (`{id, index?, t?, adjustAdjoining?}`). Opening only samples the stroke: confirmation creates
/// the point. Adjoining adjustment defaults to false.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).ok_or("give the path `id`")?;
    let doc = &app.session.active().ok_or("no document")?.doc;
    let id = vectorcraft_tools::distort::stroke_owner(doc, NodeId(id)).0;
    let index = match p.get("index").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("no such width point")?),
        None => None,
    };
    let (t, l, r) = if let Some(index) = index {
        point(app, id, index).ok_or("no such width point")?
    } else {
        let t = p.get("t").and_then(Value::as_f64).filter(|t| t.is_finite() && (0.0..=1.0).contains(t)).ok_or("give a fraction `t` from 0 to 1")?;
        let st = doc
            .node(NodeId(id))
            .and_then(|n| n.appearance.stroke())
            .filter(|s| s.width.is_finite() && s.width > 0.0)
            .ok_or("object has no weighted stroke")?;
        let (l, r) = st.profile.as_ref().map_or((1.0, 1.0), |profile| profile.at(t));
        let (l, r) = (l * st.width / 2.0, r * st.width / 2.0);
        if !l.is_finite() || !r.is_finite() || l < 0.0 || r < 0.0 {
            return Err("invalid stroke widths".into());
        }
        (t, l, r)
    };
    let adjust = p.get("adjustAdjoining").and_then(Value::as_bool).unwrap_or(false);
    let fields = json!({"id": id, "index": index, "t": t, "side1": l, "side2": r, "linked": (l - r).abs() < 1e-9, "adjustAdjoining": adjust});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.stroke_unit();
    let (s1, s2, linked) = (d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked"));
    let mut sides = None;
    grid(ui, |ui| {
        label(ui, tl!("Side 1:"));
        ui.horizontal(|ui| {
            if let Some(v) = widgets::num_field(ui, "wp-side1", Some(s1), unit, 90.0) {
                sides = Some((v, if linked { v } else { s2 }));
            }
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Keep both sides the same width"), linked, 22.0).clicked() {
                d.fields.insert("linked".into(), json!(!linked));
                if !linked {
                    sides = Some((s1, s1));
                }
            }
        });
        ui.end_row();
        label(ui, tl!("Side 2:"));
        if let Some(v) = widgets::num_field(ui, "wp-side2", Some(s2), unit, 90.0) {
            sides = Some((if linked { v } else { s1 }, v));
        }
        ui.end_row();
        label(ui, tl!("Total Width:"));
        if let Some(v) = widgets::num_field(ui, "wp-total", Some(s1 + s2), unit, 90.0) {
            // Both sides in proportion (equal halves from nothing).
            let k = if s1 + s2 > 1e-9 { v / (s1 + s2) } else { 0.0 };
            sides = Some(if k > 0.0 { (s1 * k, s2 * k) } else { (v / 2.0, v / 2.0) });
        }
        ui.end_row();
    });
    if let Some((a, b)) = sides {
        d.fields.insert("side1".into(), json!(a.max(0.0)));
        d.fields.insert("side2".into(), json!(b.max(0.0)));
    }
    ui.add_space(4.0);
    ui.allocate_ui(vec2(ui.available_width(), 22.0), |ui| {
        if widgets::check(ui, tl!("Adjust Adjoining Width Points"), d.bool("adjustAdjoining"), true) {
            d.fields.insert("adjustAdjoining".into(), json!(!d.bool("adjustAdjoining")));
        }
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, index) = (d.fields.get("id").cloned().unwrap_or_default(), d.fields.get("index").cloned().unwrap_or_default());
    if d.bool("discard") {
        if index.is_null() {
            // Deleting a prospective point discards the dialog; no document point exists yet.
            app.ui.dialog = None;
            return Ok(Value::Null);
        }
        return run_and_close(app, "stroke.widthPoint.remove", json!({"id": id, "index": index}));
    }
    let (left, right) = (d.f64("side1", 0.0), d.f64("side2", 0.0));
    let p = json!({"id": id, "index": index, "t": d.f64("t", 0.0), "left": left, "right": right, "adjustAdjoining": d.bool("adjustAdjoining")});
    run_and_close(app, "stroke.widthPoint.set", p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// A 10 pt line from (100, 150) to (300, 150) with a 4 + 4 pt width point in the middle.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("shape.line", json!({"x1": 100, "y1": 150, "x2": 300, "y2": 150})).unwrap()["id"].as_u64().unwrap();
        app.run("stroke.set", json!({"ids": [id], "weight": 10})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 4, "right": 4})).unwrap();
        (app, id)
    }

    fn points(app: &VectorcraftApp, id: u64) -> Vec<(f64, f64, f64)> {
        let st = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke().unwrap().clone();
        let round = |v: f64| (v * 1e9).round() / 1e9;
        st.profile.map(|p| p.points.into_iter().map(|(t, l, r)| (round(t), round(l), round(r))).collect()).unwrap_or_default()
    }

    fn frame(app: &mut VectorcraftApp) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // A new window lays itself out in its first frame and paints in the next.
        ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx())).textures_delta.clear();
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    #[test]
    fn double_click_opens_it_and_ok_sets_the_widths() {
        for shift in [false, true] {
            let (mut app, id) = app();
            let before = app.session.doc().unwrap().doc.clone();
            let undo = app.session.doc().unwrap().history.undo.len();
            let view = app.view_info();
            app.select_tool("width");
            let mut ev = PointerEvent::new(PointerKind::DoubleClick, 200.0, 150.0);
            ev.mods.shift = shift;
            crate::canvas::dispatch(&mut app, &ev, view);
            let d = app.ui.dialog.clone().expect("Width Point Edit opened");
            assert_eq!((d.kind.as_str(), d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (KIND, 4.0, 4.0, true));
            assert_eq!(d.bool("adjustAdjoining"), shift);
            let shown = frame(&mut app);
            for s in ["Width Point Edit", "Side 1:", "Side 2:", "Total Width:", "Adjust Adjoining Width Points", "Delete"] {
                assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
            }
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("side1".into(), json!(6));
            d.fields.insert("side2".into(), json!(2));
            super::super::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
            let ends = if shift { (1.5, 0.5) } else { (1.0, 1.0) };
            assert_eq!(points(&app, id), vec![(0.0, ends.0, ends.1), (0.5, 1.2, 0.4), (1.0, ends.0, ends.1)]);
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
            if shift {
                let edited = app.session.doc().unwrap().doc.clone();
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(app.session.doc().unwrap().doc, before);
                app.run("edit.redo", json!({})).unwrap();
                assert_eq!(app.session.doc().unwrap().doc, edited);
                crate::canvas::dispatch(&mut app, &ev, view);
                app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(20));
                super::super::cancel(&mut app);
                assert!(app.ui.dialog.is_none());
                let st = app.session.doc().unwrap();
                assert_eq!(st.doc, edited);
                assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
            }
        }
    }

    #[test]
    fn delete_removes_the_point() {
        let (mut app, id) = app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(points(&app, id).len(), 2);
        assert!(app.run("ui.widthPointEdit", json!({"id": id, "index": 9})).is_err());
    }

    #[test]
    fn it_does_not_stretch_to_the_screen() {
        let height = |screen_h: f32| {
            let (mut app, id) = app();
            app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, screen_h));
            for _ in 0..4 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                ctx.run_ui(input, |ui| super::super::show(&mut app, ui.ctx())).textures_delta.clear();
            }
            ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", KIND)))).unwrap().height()
        };
        let (short, tall) = (height(1000.0), height(2000.0));
        assert!((short - tall).abs() < 1.0 && short < 400.0, "{short} vs {tall}");
    }

    fn creation_app() -> (VectorcraftApp, u64) {
        let (mut app, id) = app();
        app.run("stroke.set", json!({"ids": [id], "profile": "uniform"})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 40})).unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("width");
        (app, id)
    }

    fn creation_history(app: &VectorcraftApp) -> (usize, usize, bool) {
        let st = app.session.doc().unwrap();
        (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
    }

    /// The actual tool-event sequence from two stationary canvas clicks. No profile or dialog
    /// is installed by the fixture after either click; DoubleClick must open the pending point.
    fn creation_double_click(app: &mut VectorcraftApp, x: f64, y: f64) {
        let before = app.session.doc().unwrap().doc.clone();
        let history = creation_history(app);
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, y), view);
            assert!(app.ui.dialog.is_none());
            assert_eq!(app.session.doc().unwrap().doc, before, "click alone must not add a width point");
            assert_eq!(creation_history(app), history);
        }
        assert!(!app.session.in_interaction());
        crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::DoubleClick, x, y), view);
        assert!(app.ui.dialog.is_some(), "double-clicking a stroked segment must open Width Point Edit");
        assert_eq!(app.session.doc().unwrap().doc, before, "opening creates no point yet");
        assert_eq!(creation_history(app), history);
        assert!(!app.session.in_interaction());
    }

    #[test]
    fn new_width_point_double_click_creates_with_history() {
        let (mut app, id) = creation_app();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let history = creation_history(&app);
        creation_double_click(&mut app, 200.0, 150.0);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, KIND);
        assert_eq!(d.fields["id"], json!(id));
        assert!(d.fields.get("index").is_none_or(Value::is_null));
        assert!((d.f64("t", -1.0) - 0.5).abs() < 1e-6);
        assert_eq!((d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (5.0, 5.0, true));
        assert!(!d.bool("adjustAdjoining"));
        let shown = frame(&mut app);
        for label in ["Width Point Edit", "Side 1:", "Side 2:", "Total Width:", "Cancel"] {
            assert!(shown.iter().any(|text| text == label), "{label} in {shown:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("side1".into(), json!(6));
        d.fields.insert("side2".into(), json!(2));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 1.2, 0.4), (1.0, 1.0, 1.0)]);
        let after = app.session.doc().unwrap().doc.clone();
        assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind, "path geometry remains unchanged");
        before.walk(|node| {
            if node.id != NodeId(id) && matches!(node.kind, vectorcraft_engine::doc::NodeKind::Path { .. }) {
                assert_eq!(after.node(node.id), Some(node), "other artwork remains unchanged");
            }
        });
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
        assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Width Point");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0, 1, false));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    }

    #[test]
    fn new_width_point_cancel_and_discard_keep_redo() {
        for discard in [false, true] {
            let (mut app, _) = creation_app();
            app.run("shape.rectangle", json!({"x": 50, "y": 20, "width": 20, "height": 20})).unwrap();
            let redo = app.session.doc().unwrap().doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            app.session.doc_mut().unwrap().mark_saved();
            let before = app.session.doc().unwrap().doc.clone();
            let selection = app.session.doc().unwrap().selection.clone();
            let history = creation_history(&app);
            assert_eq!(history.1, 1);
            creation_double_click(&mut app, 200.0, 150.0);
            app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(20));
            if discard {
                app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
                super::super::confirm(&mut app).unwrap();
            } else {
                super::super::cancel(&mut app);
            }
            assert!(app.ui.dialog.is_none());
            assert_eq!(app.session.doc().unwrap().doc, before);
            assert_eq!(app.session.doc().unwrap().selection, selection);
            assert_eq!(creation_history(&app), history);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, redo);
        }
    }

    #[test]
    fn new_width_point_profile_and_compound_target() {
        for compound in [false, true] {
            let (mut app, line) = creation_app();
            let (id, member) = if compound {
                let second = app.run("shape.line", json!({"x1": 320, "y1": 50, "x2": 320, "y2": 250})).unwrap()["id"].as_u64().unwrap();
                app.run("select.set", json!({"ids": [line, second]})).unwrap();
                (app.run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap(), second)
            } else {
                (line, line)
            };
            app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 0.5], [1, 2, 1.5]]})).unwrap();
            app.select_tool("width");
            app.session.doc_mut().unwrap().mark_saved();
            let before = app.session.doc().unwrap().doc.clone();
            let selection = app.session.doc().unwrap().selection.clone();
            let history = creation_history(&app);
            // Make Compound Path can reverse the member. Choose the quarter point from its
            // stored direction; on a straight segment this is exactly one quarter of its length.
            let path = before.node(NodeId(member)).unwrap().path_data().unwrap();
            assert_eq!(path.subpaths.len(), 1);
            let subpath = &path.subpaths[0];
            assert_eq!(subpath.anchors.len(), 2);
            assert!(subpath.segment_is_line(0));
            let quarter = subpath.anchors[0].p.lerp(subpath.anchors[1].p, 0.25);
            creation_double_click(&mut app, quarter.x, quarter.y);
            let d = app.ui.dialog.as_mut().unwrap();
            assert_eq!(d.fields["id"], json!(id), "the compound owns its members' stroke");
            assert!(d.fields.get("index").is_none_or(Value::is_null));
            let actual_t = d.f64("t", -1.0);
            assert!((actual_t - 0.25).abs() < 1e-6, "compound={compound} pointer={quarter:?} actual_t={actual_t} want=0.25");
            // The two-point profile is linear: at a quarter its sides are 1.25 and 0.75 of
            // 1..2 and 0.5..1.5, on a 10pt stroke (5pt half).
            assert!((d.f64("side1", 0.0) - 6.25).abs() < 1e-6);
            assert!((d.f64("side2", 0.0) - 3.75).abs() < 1e-6);
            assert!(!d.bool("linked"));
            d.fields.insert("side1".into(), json!(8));
            d.fields.insert("side2".into(), json!(3));
            super::super::confirm(&mut app).unwrap();
            assert_eq!(points(&app, id), vec![(0.0, 1.0, 0.5), (0.25, 1.6, 0.6), (1.0, 2.0, 1.5)]);
            let after = app.session.doc().unwrap().doc.clone();
            assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind, "member paths retain their geometry");
            assert_eq!(app.session.doc().unwrap().selection, selection);
            assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, before);
            assert_eq!(creation_history(&app), (history.0, 1, false));
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, after);
        }
    }

    #[test]
    fn new_width_point_command_opens_pending_point() {
        let (mut app, id) = creation_app();
        let before = app.session.doc().unwrap().doc.clone();
        let history = creation_history(&app);
        app.run("ui.widthPointEdit", json!({"id": id, "t": 0.25})).expect("the UI command accepts a prospective point");
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.fields.get("index").is_none_or(Value::is_null));
        assert_eq!((d.f64("t", -1.0), d.f64("side1", 0.0), d.f64("side2", 0.0)), (0.25, 5.0, 5.0));
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }

    #[test]
    fn new_width_point_invalid_command_keeps_document() {
        let (mut app, id) = creation_app();
        let before = app.session.doc().unwrap().doc.clone();
        let history = creation_history(&app);
        for p in [
            json!({"id": id, "t": -0.01}),
            json!({"id": id, "t": 1.01}),
            json!({"id": id, "t": "invalid"}),
            json!({"id": id, "index": "invalid", "t": 0.5}),
            json!({"id": id, "index": 999, "t": 0.5}),
        ] {
            assert!(app.run("ui.widthPointEdit", p).is_err());
            assert!(app.ui.dialog.is_none());
            assert_eq!(app.session.doc().unwrap().doc, before);
            assert_eq!(creation_history(&app), history);
        }
    }

    fn creation_dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        fn collect(shape: &egui::Shape, clip: egui::Rect, texts: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => {
                    texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, texts)),
                _ => {}
            }
        }
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        output.textures_delta.clear();
        let mut texts = vec![];
        output.shapes.iter().for_each(|shape| collect(&shape.shape, shape.clip_rect, &mut texts));
        texts
    }

    fn creation_dialog_click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
        for _ in 0..4 {
            creation_dialog_frame(app, ctx, vec![]);
        }
        let texts = creation_dialog_frame(app, ctx, vec![]);
        let pos = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing visible control {label}: {texts:?}")).1.center();
        for pressed in [true, false] {
            creation_dialog_frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                ],
            );
        }
        creation_dialog_frame(app, ctx, vec![]);
    }

    fn creation_adjoining_checkbox(adjust: bool) {
        let (mut app, id) = creation_app();
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [1, 1, 1]]})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let history = creation_history(&app);
        creation_double_click(&mut app, 200.0, 150.0);
        assert!(app.ui.dialog.as_ref().unwrap().fields["index"].is_null());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        for _ in 0..5 {
            creation_dialog_frame(&mut app, &ctx, vec![]);
        }
        // The dialog naturally focuses and selects Side 1. Its linked partner changes
        // when the checkbox click commits the typed value; no fields are seeded.
        assert!(ctx.memory(|m| m.focused()).is_some());
        creation_dialog_frame(&mut app, &ctx, vec![egui::Event::Text("10 pt".into())]);
        creation_dialog_click(&mut app, &ctx, "Adjust Adjoining Width Points");
        assert!(app.ui.dialog.as_ref().unwrap().bool("adjustAdjoining"), "real checkbox click enables adjustment");
        if !adjust {
            creation_dialog_click(&mut app, &ctx, "Adjust Adjoining Width Points");
        }
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.bool("adjustAdjoining"), adjust);
        assert_eq!((d.f64("side1", 0.0), d.f64("side2", 0.0)), (10.0, 10.0));
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
        creation_dialog_click(&mut app, &ctx, "OK");
        assert!(app.ui.dialog.is_none(), "real OK click confirms the pending point");
        let end = if adjust { 2.0 } else { 1.0 };
        assert_eq!(points(&app, id), vec![(0.0, end, end), (0.5, 2.0, 2.0), (1.0, end, end)], "checkbox choice controls adjoining widths");
        let after = app.session.doc().unwrap().doc.clone();
        assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind);
        before.walk(|node| {
            if node.id != NodeId(id) && matches!(node.kind, vectorcraft_engine::doc::NodeKind::Path { .. }) {
                assert_eq!(after.node(node.id), Some(node), "other artwork remains unchanged");
            }
        });
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
        assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Width Point");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0, 1, false));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    }

    #[test]
    fn new_width_point_checkbox_adjusts_adjoining_in_one_step() {
        creation_adjoining_checkbox(true);
    }

    #[test]
    fn new_width_point_checkbox_off_preserves_adjoining() {
        creation_adjoining_checkbox(false);
    }
}
