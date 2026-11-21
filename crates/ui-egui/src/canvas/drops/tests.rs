//! Drops on the canvas: the effect of each payload, the cross-document symbol, the mesh hit rules and the carry out
//! of the canvas.

use craft::behavior::DropOperation;
use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use super::{CarriedArt, Dropped, Payload, apply_drop};
use crate::panel_drag::PanelDrag;
use crate::panels::symbols::SymbolKey;
use crate::{VectorcraftApp, canvas};

/// One frame of the canvas inside a central panel. → nothing; the app's view is set by it.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    app.sync_views();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().show(ui, |ui| canvas::show(app, ui));
    });
    out.textures_delta.clear();
}

/// A new 400 × 300 document with the canvas shown once, so the view exists.
fn app_with_canvas() -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn dropped(app: &VectorcraftApp, payload: Payload, at: Point, op: DropOperation) -> Dropped {
    Dropped { payload, at, zoom: app.canvas_view().unwrap().zoom, op }
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Each payload's effect, on art and off it: a style applies to the object hit (Alt adds on top), a paint fills it
/// and leaves the selection, an appearance copies unless dropped on its source, and off art nothing changes.
#[test]
fn a_drop_runs_the_effect_of_its_payload() {
    use DropOperation::{Copy, Move};
    let (mut app, _ctx) = app_with_canvas();
    let a = NodeId(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    let b = NodeId(app.run("shape.rectangle", json!({"x": 200, "y": 20, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let on_b = Point::new(250.0, 70.0);
    let off = Point::new(350.0, 250.0);
    let red = Paint::solid(Color::from_hex("#ed1c24").unwrap());
    let swatch = PanelDrag::Paint { paint: red.clone(), params: json!({"swatch": "Red"}), rows: None, library: None };
    let n = undo_len(&app);
    apply_drop(&mut app, &dropped(&app, Payload::Panel(swatch.clone()), off, Move)).unwrap();
    assert_eq!(undo_len(&app), n, "off art nothing happens");
    apply_drop(&mut app, &dropped(&app, Payload::Panel(swatch), on_b, Move)).unwrap();
    assert_eq!(app.session.active().unwrap().doc.node(b).unwrap().appearance.fill_paint(), red);
    assert_eq!(app.session.active().unwrap().selection.objects, [a], "the selection stays");
    let name = app.session.active().unwrap().doc.graphic_styles[2].name.clone();
    apply_drop(&mut app, &dropped(&app, Payload::Style(name.clone()), on_b, Copy)).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.node(b).unwrap().extras().graphic_style, st.doc.graphic_style(&name).map(|g| g.id));
    let n = undo_len(&app);
    apply_drop(&mut app, &dropped(&app, Payload::Panel(PanelDrag::Appearance(b)), Point::new(250.0, 70.0), Move)).unwrap();
    assert_eq!(undo_len(&app), n, "dropped on its own source: nothing changes");
}

/// A symbol of another document is added here and placed in one undo step, the source is unchanged, and a symbol of
/// a closed document places nothing.
#[test]
fn a_symbol_from_another_document_is_added_and_placed_in_one_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Leaf"})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    let source = app.session.active().unwrap().uid;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let key = |doc| Payload::Symbol(SymbolKey { doc, name: "Leaf".into() });
    let at = Point::new(250.0, 180.0);
    let n = undo_len(&app);
    apply_drop(&mut app, &dropped(&app, key(source), at, DropOperation::Move)).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), n + 1);
    let placed = st.selection.objects[0];
    assert!(matches!(&st.doc.node(placed).unwrap().kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Leaf"));
    let n = undo_len(&app);
    let _ = apply_drop(&mut app, &dropped(&app, key(u64::MAX), at, DropOperation::Move));
    assert_eq!(undo_len(&app), n, "a closed source places nothing");
}

/// A colour dropped on a selected mesh recolors the point under the pointer, else the patch's four points; the same
/// colour again records no step.
#[test]
fn a_color_dropped_on_a_selected_mesh_recolors_the_point_or_patch_under_it() {
    use vectorcraft_doc::GradientMesh;
    let (mut app, _ctx) = app_with_canvas();
    let id = NodeId(app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    app.run("object.mesh.create", json!({"ids": [id.0], "rows": 2, "columns": 2})).unwrap();
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let mesh = |app: &VectorcraftApp| -> Option<GradientMesh> {
        match &app.session.active().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Mesh(m) => Some(m.clone()),
            _ => None,
        }
    };
    let m = mesh(&app).unwrap();
    let before: Vec<_> = m.points.iter().map(|q| q.color).collect();
    let drop = |app: &mut VectorcraftApp, c: Color, at: Point| {
        let d = dropped(app, Payload::Panel(PanelDrag::paint(Paint::solid(c))), at, DropOperation::Move);
        apply_drop(app, &d).unwrap();
        mesh(app).unwrap().points.iter().map(|q| q.color).collect::<Vec<_>>()
    };
    let red = Color::rgb(1.0, 0.0, 0.0);
    let after = drop(&mut app, red, m.points[4].p);
    assert_eq!((0..9).filter(|i| after[*i] != before[*i]).count(), 1, "only the point under the pointer");
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let patch = Point::new((m.points[0].p.x + m.points[4].p.x) / 2.0, (m.points[0].p.y + m.points[4].p.y) / 2.0);
    let after = drop(&mut app, blue, patch);
    assert_eq!([0, 1, 3, 4].map(|i| after[i]), [blue; 4]);
    let n = undo_len(&app);
    drop(&mut app, blue, patch);
    assert_eq!(undo_len(&app), n, "the same color records no step");
}

/// Carried artwork dropped in another document is copied there and the source stays; a source edited since the carry
/// began is not carried (the drop's target refuses it, so `apply_drop` is never reached: its revision no longer matches).
#[test]
fn carried_art_dropped_in_another_document_is_copied_and_the_source_stays() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Source"})).unwrap();
    let id = NodeId(app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    let st = app.session.active().unwrap();
    let art = CarriedArt { document: st.uid, ids: vec![id], grab: Point::new(100.0, 100.0), revision: st.revision };
    let source = st.doc.clone();
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Target"})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let n = undo_len(&app);
    apply_drop(&mut app, &dropped(&app, Payload::Art(art), Point::new(170.0, 120.0), DropOperation::Move)).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), n + 1);
    let placed = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert!((placed.center().x - 170.0).abs() < 0.01 && (placed.center().y - 120.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[0].doc, source);
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A Selection, Direct Selection or Group Selection move that leaves the canvas hands the art to the drag session
/// with the ids the move started with, and the move waits where it was last.
#[test]
fn art_moved_off_the_canvas_is_carried_by_every_selection_tool() {
    for tool in ["selection", "directSelection", "groupSelection"] {
        let (mut app, ctx) = app_with_canvas();
        let id = NodeId(app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
        app.run("select.none", json!({})).unwrap();
        app.select_tool(tool);
        frame(&mut app, &ctx, vec![]);
        let view = app.canvas_view().unwrap();
        let start = view.to_screen(Point::new(100.0, 100.0));
        frame(&mut app, &ctx, vec![Event::PointerMoved(start), button(start, true)]);
        frame(&mut app, &ctx, vec![Event::PointerMoved(start + egui::vec2(40.0, 0.0))]);
        assert!(app.session.active().unwrap().interaction.is_some(), "{tool}: the move is live");
        // The tab strip is off the canvas.
        frame(&mut app, &ctx, vec![Event::PointerMoved(Pos2::new(500.0, 2.0))]);
        frame(&mut app, &ctx, vec![]);
        let carried = craft::ui::runtime::drag::active::<PanelDrag>(&ctx); // PLAN: assumed `active::<P>(&Context) -> Option<Arc<P>>`
        assert!(matches!(carried.as_deref(), Some(PanelDrag::Art { ids, .. }) if ids == &[id]), "{tool}");
    }
}

/// A move of anchors stays a reshape: no art is handed to the panels.
#[test]
fn an_anchor_dragged_off_the_canvas_is_not_carried() {
    let (mut app, ctx) = app_with_canvas();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0]]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let corner = app.canvas_view().unwrap().to_screen(Point::new(50.0, 50.0));
    frame(&mut app, &ctx, vec![Event::PointerMoved(corner), button(corner, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(corner + egui::vec2(20.0, 0.0))]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(Pos2::new(500.0, 2.0))]);
    frame(&mut app, &ctx, vec![]);
    assert!(craft::ui::runtime::drag::active::<PanelDrag>(&ctx).is_none());
}
