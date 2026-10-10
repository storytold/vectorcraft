//! Corners: double-clicking a Live Corners widget (or `ui.corners`) sets the corner kind and
//! radius of a path's corners: those whose widgets show (the Direct-Selected ones, else every
//! corner). OK runs `object.setLiveShape` for them, one undo step.
//!
//! Fields: `id`, `corners` (anchor indices of the path with its corners uncut: a rectangle's 0–3
//! from the top-left clockwise), `kind` ("round", "invertedRound" or "chamfer") and `radius`
//! (pt). While the corners differ in kind or radius, that field is absent and OK leaves it as it
//! is.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_engine::doc::{LiveCorners, NodeId};
use vectorcraft_geom::shapes::CornerKind;

use super::swatch_options::{grid, label};
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Corners (the corner widgets' double-click).
pub const KIND: &str = vectorcraft_tools::corners::DIALOG;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Corners").into(), body, confirm, min_width: 280.0, ..DialogSpec::FORM };

/// Open Corners for `{id?, corners?}`: the path `id` (default: the selected one) and its
/// `corners` (default: the selected corners), filled in with their kind and radius.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let id = match p.get("id").and_then(Value::as_u64) {
        Some(id) => NodeId(id),
        None => match st.selection.objects[..] {
            [id] => id,
            _ => return Err("select one path, or give its `id`".into()),
        },
    };
    let Some(live) = st.doc.node(id).and_then(LiveCorners::of) else {
        return Err("Corners edits the corners of a path".into());
    };
    let corners: BTreeSet<usize> = match p.get("corners").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).filter_map(|k| usize::try_from(k).ok()).filter(|k| live.corner(*k).is_some()).collect(),
        None => live.picked(st.selection.partial(id)),
    };
    if corners.is_empty() {
        return Err("give corners of the path: anchors between two straight sides".into());
    }
    let (radius, kind) = live.style(&corners);
    let mut fields = json!({"id": id.0, "corners": corners});
    if let Some(k) = kind {
        fields["kind"] = json!(k);
    }
    if let Some(r) = radius {
        fields["radius"] = json!(r);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let current = d.fields.get("kind").and_then(|k| serde_json::from_value::<CornerKind>(k.clone()).ok());
    grid(ui, |ui| {
        label(ui, tl!("Corner:"));
        ui.vertical(|ui| {
            for k in CornerKind::ALL {
                if widgets::radio(ui, kind_label(k), Some(k) == current, true) {
                    d.fields.insert("kind".into(), json!(k));
                }
            }
        });
        ui.end_row();
        label(ui, tl!("Radius:"));
        form::length(ui, d, "radius", unit, 120.0);
        ui.end_row();
    });
    false
}

/// A corner kind's name, in the UI language.
pub(crate) fn kind_label(k: CornerKind) -> &'static str {
    match k {
        CornerKind::Round => tl!("Round"),
        CornerKind::InvertedRound => tl!("Inverted Round"),
        CornerKind::Chamfer => tl!("Chamfer"),
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({"id": d.fields.get("id"), "corners": d.fields.get("corners")});
    if let Some(k) = d.fields.get("kind") {
        p["kind"] = k.clone();
    }
    if d.fields.contains_key("radius") {
        p["radius"] = json!(d.f64("radius", 0.0));
    }
    run_and_close(app, "object.setLiveShape", p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{LiveShape, NodeKind};
    use vectorcraft_tools::corners::CornerWidgets;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// A selected 100 pt live square at (100, 100).
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn live(app: &VectorcraftApp, id: u64) -> LiveShape {
        let Some(NodeKind::Path { live: Some(l), .. }) = app.session.active().unwrap().doc.node(NodeId(id)).map(|n| n.kind.clone()) else {
            panic!("not live")
        };
        l
    }

    fn corners(app: &VectorcraftApp, id: u64) -> ([f64; 4], [CornerKind; 4]) {
        let LiveShape::Rectangle { radii, kinds, .. } = live(app, id) else { panic!("not a rectangle") };
        (radii, kinds)
    }

    /// The radius the Properties panel's Corner Radius field shows.
    fn panel_radius(app: &VectorcraftApp, id: u64) -> Option<f64> {
        let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
        crate::panels::corner_radius(app, &n)
    }

    fn shown(app: &mut VectorcraftApp) -> Vec<String> {
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
    fn double_clicking_a_direct_selected_corner_edits_it_alone() {
        let (mut app, id) = app();
        // Direct Selection picks the bottom-right anchor: only its widget shows.
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 2]], "mode": "set"})).unwrap();
        app.select_tool("directSelection");
        let view = app.view_info();
        let st = app.session.active().unwrap();
        let at: Vec<_> = CornerWidgets::of(&st.doc, &st.selection, view.zoom, true).unwrap().visible().collect();
        assert_eq!(at.len(), 1);
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, at[0].x, at[0].y), view);
        let d = app.ui.dialog.clone().expect("Corners opened");
        assert_eq!(
            (d.kind.as_str(), d.fields.get("corners"), d.fields.get("kind"), d.f64("radius", -1.0)),
            (KIND, Some(&json!([2])), Some(&json!("round")), 0.0)
        );
        let texts = shown(&mut app);
        for s in ["Corners", "Corner:", "Round", "Inverted Round", "Chamfer", "Radius:"] {
            assert!(texts.iter().any(|t| t == s), "{s} in {texts:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("kind".into(), json!("chamfer"));
        d.fields.insert("radius".into(), json!(12));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(corners(&app, id), ([0.0, 0.0, 12.0, 0.0], [CornerKind::Round, CornerKind::Round, CornerKind::Chamfer, CornerKind::Round]));
        // The panels' radius field shows the selected corner's radius, blank for the whole
        // shape (its corners differ).
        assert_eq!(panel_radius(&app, id), Some(12.0));
        app.run("select.set", json!({"ids": [id]})).unwrap();
        assert_eq!(panel_radius(&app, id), None);
    }

    #[test]
    fn ui_corners_opens_on_the_selection_and_leaves_mixed_values_alone() {
        let (mut app, id) = app();
        app.run("object.setLiveShape", json!({"corners": [0], "radius": 5, "kind": "invertedRound"})).unwrap();
        app.run("ui.corners", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(d.fields.get("corners"), Some(&json!([0, 1, 2, 3])));
        assert!(d.fields.get("kind").is_none() && d.fields.get("radius").is_none(), "mixed: {d:?}");
        let before = corners(&app, id);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(corners(&app, id), before);
        // A radius for all four keeps their kinds.
        app.run("ui.corners", json!({"id": id})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), json!("3 pt"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(corners(&app, id), ([3.0; 4], before.1));
        // Only paths, and only real corners.
        assert!(app.run("ui.corners", json!({"id": 9999})).is_err());
        assert!(app.run("ui.corners", json!({"corners": [7]})).is_err());
        let e = app.run("shape.ellipse", json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap()["id"].as_u64().unwrap();
        assert!(app.run("ui.corners", json!({"id": e})).is_err());
    }

    /// #511: a star's corners open in Corners too; OK rounds them.
    #[test]
    fn corners_edits_a_star() {
        let (mut app, _) = app();
        let id = app.run("shape.star", json!({"cx": 200, "cy": 200, "radius1": 60, "radius2": 30})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.corners", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.fields.get("corners"), d.f64("radius", -1.0)), (Some(&json!((0..10).collect::<Vec<_>>())), 0.0));
        d.fields.insert("radius".into(), json!(5));
        super::super::confirm(&mut app).unwrap();
        let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
        assert!(matches!(&n.kind, NodeKind::Path { live: Some(LiveShape::Path { .. }), .. }));
        assert_eq!(crate::panels::corner_radius(&app, &n), Some(5.0));
    }
}
