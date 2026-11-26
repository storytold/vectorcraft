//! Corners: double-clicking a Live Corners widget (or `ui.corners`) sets the corner kind and
//! radius of a path's corners: the picked ones (the Direct-Selected ones, else every corner), or
//! the double-clicked widget's corner alone when Direct Selection did not pick it. OK runs
//! `object.setLiveShape` for them, one undo step.
//!
//! Fields: `id`, `corners` (anchor indices of the path with its corners uncut: a rectangle's 0–3
//! from the top-left clockwise), `kind` ("round", "invertedRound" or "chamfer"), `radii`
//! ("relative" or "absolute") and `radius` (pt, or percent while relative). While the corners
//! differ in kind or radius, that field is mixed and OK leaves it as it is.

use std::collections::BTreeSet;

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Value, json};
use vectorcraft_engine::doc::{LiveCorners, NodeId};
use vectorcraft_geom::shapes::CornerKind;

use super::host::open_held;
use crate::VectorcraftApp;
use crate::canvas::general_lengths;

/// The dialog kind of Corners (the corner widgets' double-click).
pub const KIND: &str = vectorcraft_tools::corners::DIALOG;

/// Open Corners for `{id?, corners?}`: the path `id` (default: the selected one) and its
/// `corners` (default: the selected corners), filled in with their kind and radius.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let id = match p.get("id") {
        Some(v) => NodeId(v.as_u64().ok_or_else(|| format!("object ids are non-negative integers, not {v}"))?),
        None => match st.selection.objects[..] {
            [id] => id,
            _ => return Err("select one path, or give its `id`".into()),
        },
    };
    let Some(live) = st.doc.node(id).and_then(LiveCorners::of) else {
        return Err("Corners edits the corners of a path".into());
    };
    let corners: BTreeSet<usize> = match p.get("corners") {
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(|k| usize::try_from(k.as_u64()?).ok()).collect())
            .ok_or_else(|| format!("`corners` must be an array of anchor indices, not {v}"))?,
        None => live.picked(st.selection.partial(id)),
    };
    // Every corner given is one of the path's: none is left out of the edit.
    if corners.is_empty() || !corners.iter().all(|k| live.corner(*k).is_some()) {
        return Err("give corners of the path: anchors where its sides meet at an angle".into());
    }
    let (radius, kind) = live.style(&corners);
    let relative = st.doc.node(id).and_then(|n| n.live_shape()).map(|l| l.radii_relative()).unwrap_or(false);
    // A relative radius is the first corner's fraction, as a percentage.
    let radius = radius.and_then(|r| if relative { corners.iter().find_map(|k| live.corner(*k).map(|_| live.radius(*k) * 100.0)) } else { Some(r) });
    let mut fields = DialogFields::new();
    fields.set("id", FieldValue::Json(json!(id.0)));
    fields.set("corners", FieldValue::Json(json!(corners)));
    fields.set("radii", FieldValue::Text(if relative { "relative" } else { "absolute" }.into()));
    match kind {
        Some(k) => fields.set("kind", FieldValue::Text(kind_id(k))),
        None => fields.set_mixed("kind"),
    }
    match radius {
        Some(r) => fields.set("radius", FieldValue::Number(r)),
        None => fields.set_mixed("radius"),
    }
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = general_lengths(app);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| dialog(fields.text("radii") == Some("relative"), lengths));
    open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// The id a corner kind has in the fields and in `object.setLiveShape`.
fn kind_id(k: CornerKind) -> String {
    json!(k).as_str().map(str::to_owned).unwrap_or_default()
}

/// The dialog over its rows; the radius row is a percentage while the radii are relative, a length otherwise.
fn dialog<'a>(relative: bool, lengths: LengthUnits) -> ParamDialog<'a> {
    let kinds = CornerKind::ALL.iter().map(|k| (kind_id(*k), kind_label(*k).into())).collect();
    let radius = if relative {
        Param::new("radius", tl!("Radius:"), ParamKind::number(Some(0.0), Some(100.0), 0.0, 1.0, 1)).unit(UnitKind::Percent)
    } else {
        Param::number("radius", tl!("Radius:"), 0.0, 1.0, 3).length(lengths).bounds(Some(0.0), None)
    };
    let radii = vec![("relative".to_owned(), tl!("Relative").into()), ("absolute".to_owned(), tl!("Absolute").into())];
    let params =
        vec![Param::new("kind", tl!("Corner:"), ParamKind::radio(kinds, None)), Param::new("radii", "", ParamKind::radio(radii, None)), radius];
    ParamDialog::new(KIND, tl!("Corners"), params)
}

/// A corner kind's name, in the UI language.
pub(crate) fn kind_label(k: CornerKind) -> &'static str {
    match k {
        CornerKind::Round => tl!("Round"),
        CornerKind::InvertedRound => tl!("Inverted Round"),
        CornerKind::Chamfer => tl!("Chamfer"),
    }
}

/// A key a row without a control keeps (`id`, `corners`), as JSON.
fn json_field(fields: &DialogFields, key: &str) -> Value {
    match fields.get(key) {
        Some(FieldValue::Json(v)) => v.clone(),
        Some(FieldValue::Grid(g)) => json!(g),
        _ => Value::Null,
    }
}

/// OK: sets the corners' kind and radius; a mixed field is absent from the answer and stays as it is.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let f = &result.fields;
    let relative = f.text("radii") == Some("relative");
    let mut p = json!({"id": json_field(f, "id"), "corners": json_field(f, "corners"), "relativeRadii": relative});
    if let Some(k) = f.text("kind") {
        p["kind"] = json!(k);
    }
    if let Some(mut r) = f.number("radius") {
        if relative {
            let id = NodeId(json_field(f, "id").as_u64().unwrap_or(0));
            let st = app.session.active().ok_or("no document")?;
            if let Some(lc) = st.doc.node(id).and_then(LiveCorners::of) {
                let corners: BTreeSet<usize> = serde_json::from_value(json_field(f, "corners")).unwrap_or_default();
                if let Some(k) = corners.iter().next() {
                    let max = lc.corner(*k).map(|c| c.max_radius()).unwrap_or(0.0);
                    r = (r / 100.0).clamp(0.0, 1.0) * max;
                }
            }
        }
        p["radius"] = json!(r);
    }
    app.run("object.setLiveShape", p).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{DialogHost, dialog_id};
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{LiveShape, NodeKind};
    use vectorcraft_tools::corners::CornerWidgets;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// A selected 100 pt live square at (100, 100).
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app = crate::testing::windowed(app); // PLAN: assumed path of sh.SH4
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn live(app: &VectorcraftApp, id: u64) -> LiveShape {
        let Some(NodeKind::Path { live: Some(l), .. }) = app.session.active().unwrap().doc.node(NodeId(id)).map(|n| n.kind.clone()) else {
            panic!("not live")
        };
        *l
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

    fn with<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().unwrap().with(f)
    }

    /// The open dialog's fields; panics when Corners is closed.
    fn fields(app: &VectorcraftApp) -> DialogFields {
        with(app, |h| h.get_by_id::<HostedParamDialog>(dialog_id(KIND)).map(|d| d.fields().clone())).expect("Corners is open")
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        with(app, |h| h.get_by_id::<HostedParamDialog>(dialog_id(KIND)).is_some())
    }

    fn set(app: &VectorcraftApp, key: &str, value: Value) {
        let ctx = app.ui_ctx.clone().unwrap();
        with(app, |h| h.set_json_field(&ctx, dialog_id(KIND), key, value)).unwrap();
    }

    /// Presses OK; the app's frames then run the held answer and close the dialog on it.
    fn confirm(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        with(app, |h| h.confirm_by_id(&ctx, dialog_id(KIND)));
        for _ in 0..2 {
            crate::testing::app_frame(app, &ctx, egui::RawInput::default());
        }
    }

    /// Explicit UI-command parameters cannot silently fall back to the selection or drop corners.
    #[test]
    fn malformed_explicit_corner_targets_never_open_a_partial_edit() {
        let (mut app, id) = app();
        for params in [
            json!({"id": "invalid"}),
            json!({"id": -1}),
            json!({"id": id, "corners": [0, "bad", 2]}),
            json!({"id": id, "corners": [0, 1.5]}),
            json!({"id": id, "corners": [0, 9]}),
            json!({"id": id, "corners": "bad"}),
        ] {
            assert!(app.run("ui.corners", params.clone()).is_err(), "accepted {params}");
            assert!(!is_open(&app), "opened a partial edit for {params}");
        }
        app.run("ui.corners", json!({"id": id, "corners": [0, 2]})).unwrap();
        assert_eq!(json_field(&fields(&app), "corners"), json!([0, 2]));
    }

    #[test]
    fn double_clicking_a_direct_selected_corner_edits_it_alone() {
        let (mut app, id) = app();
        // Direct Selection picks the bottom-right anchor: its widget alone is picked.
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 2]], "mode": "set"})).unwrap();
        app.select_tool("directSelection");
        let view = app.view_info();
        let st = app.session.active().unwrap();
        let at: Vec<_> = CornerWidgets::of(&st.doc, &st.selection, view.zoom, true).unwrap().visible_on().filter(|w| w.2).map(|w| w.1).collect();
        assert_eq!(at.len(), 1);
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, at[0].x, at[0].y), view);
        assert!(is_open(&app), "Corners opened");
        let f = fields(&app);
        assert_eq!((json_field(&f, "corners"), f.text("kind"), f.number("radius")), (json!([2]), Some("round"), Some(0.0)));
        set(&app, "kind", json!("chamfer"));
        set(&app, "radius", json!(12));
        confirm(&mut app);
        assert!(!is_open(&app));
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
        let f = fields(&app);
        assert_eq!(json_field(&f, "corners"), json!([0, 1, 2, 3]));
        assert!(f.is_mixed("kind") && f.is_mixed("radius"), "mixed: {f:?}");
        let before = corners(&app, id);
        confirm(&mut app);
        assert_eq!(corners(&app, id), before);
        // A radius for all four keeps their kinds.
        app.run("ui.corners", json!({"id": id})).unwrap();
        set(&app, "radius", json!(3));
        confirm(&mut app);
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
        let f = fields(&app);
        assert_eq!((json_field(&f, "corners"), f.number("radius")), (json!((0..10).collect::<Vec<_>>()), Some(0.0)));
        set(&app, "radius", json!(5));
        confirm(&mut app);
        let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
        assert!(matches!(&n.kind, NodeKind::Path { live: Some(l), .. } if matches!(**l, LiveShape::Path { .. })));
        assert_eq!(crate::panels::corner_radius(&app, &n), Some(5.0));
    }
}
