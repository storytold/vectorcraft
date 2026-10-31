use egui::{Pos2, Rect, pos2};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

thread_local! {
    /// Stroke previews rendered (cache misses), and drawn, on this test thread.
    pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static DRAWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One frame of the panel → (previews rendered, previews drawn) in it.
fn counted(ctx: &egui::Context, app: &mut VectorcraftApp) -> (usize, usize) {
    BUILDS.with(|b| b.set(0));
    DRAWN.with(|d| d.set(0));
    frame(ctx, app, vec![]);
    (BUILDS.with(std::cell::Cell::get), DRAWN.with(std::cell::Cell::get))
}

use super::*;

/// One headless frame of the panel, 236 pt wide as in the dock.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
}

fn button(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// One frame of `draw` at `time` with `modifiers` held, its output kept.
fn frame_with(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    time: f64,
    draw: fn(&mut VectorcraftApp, &mut Ui),
) -> egui::FullOutput {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0)));
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let raw = egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
    out.textures_delta.clear();
    out
}

/// A click at `at` with `modifiers` held, in a frame of `draw` at `time`.
fn click(ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2, modifiers: egui::Modifiers, time: f64, draw: fn(&mut VectorcraftApp, &mut Ui)) {
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frame_with(ctx, app, vec![egui::Event::PointerMoved(at), press(true), press(false)], modifiers, time, draw);
}

/// The accessibility nodes of `out` that take a click, with their rectangles, top to bottom
/// and then left to right.
fn clickable(out: &egui::FullOutput) -> Vec<(Option<String>, Rect)> {
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let mut v: Vec<(Option<String>, Rect)> = nodes
        .iter()
        .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Click))
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((n.label().map(|l| l.trim().to_string()), Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect();
    v.sort_by(|a, b| a.1.min.y.total_cmp(&b.1.min.y).then(a.1.min.x.total_cmp(&b.1.min.x)));
    v
}

/// #852: double-clicking a brush opens its Brush Options; OK changes the brush (and strokes
/// painted with it). A single click only applies the brush.
#[test]
fn double_clicking_a_brush_opens_its_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    let clicks = |at: Pos2, n: usize| -> Vec<egui::Event> {
        std::iter::once(egui::Event::PointerMoved(at)).chain((0..n).flat_map(|_| [button(at, true), button(at, false)])).collect()
    };
    let at_time = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let raw =
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0))), time: Some(time), events, ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    at_time(&mut app, 0.0, vec![]);
    let (list, _) = brushes(&mut app);
    let first = list[0].0.clone();
    assert_eq!(list[0].1, "calligraphic");
    // The first tile, clicked once: it becomes current, no dialog.
    let tile = pos2(36.0, 16.0);
    at_time(&mut app, 1.0, clicks(tile, 1));
    assert!(app.ui.dialog.is_none());
    assert_eq!(brushes(&mut app).1.as_deref(), Some(first.as_str()));
    // Double-clicked: Calligraphic Brush Options on that brush.
    at_time(&mut app, 3.0, clicks(tile, 2));
    let d = app.ui.dialog.as_mut().expect("Brush Options opened");
    assert_eq!((d.kind.as_str(), d.str("name"), d.str("sizeMode")), (crate::dialogs::brush_options::KIND, first.clone(), "fixed".into()));
    d.fields.insert("sizeMode".into(), json!("pressure"));
    d.fields.insert("sizeVariation".into(), json!(2));
    d.fields.insert("name".into(), json!("Pressure Round"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let def = app.run("brush.get", json!({"name": "Pressure Round"})).unwrap();
    assert_eq!((def["modes"].clone(), def["variation"][2].clone()), (json!(["fixed", "fixed", "pressure"]), json!(2.0)));
    assert_eq!(brushes(&mut app).1.as_deref(), Some("Pressure Round"), "renamed, still current");
}

/// The New Brush button and the panel menu's New Brush… open New Brush, with nothing selected
/// too, and make no brush.
#[test]
fn the_new_brush_button_and_menu_item_open_new_brush() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let before = names(&mut app);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
    let new = clickable(&out).into_iter().find(|(l, _)| l.as_deref() == Some("New Brush")).expect("a New Brush button").1.center();
    click(&ctx, &mut app, new, egui::Modifiers::NONE, 1.0, show);
    let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
    assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::new_brush::KIND));
    app.ui.dialog = None;
    menu_click(&ctx, &mut app, "New Brush…", 2.0);
    assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::new_brush::KIND));
    assert_eq!(names(&mut app), before);
}

/// The rectangles `out` fills with the selected-row color.
fn lit(ctx: &egui::Context, out: &egui::FullOutput) -> Vec<Rect> {
    let t = Tokens::get(ctx);
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == t.row_selected => Some(r.rect),
            _ => None,
        })
        .collect()
}

fn brush_of(app: &VectorcraftApp, id: NodeId) -> Option<String> {
    app.session.active().unwrap().doc.node(id).unwrap().appearance.stroke().and_then(|s| s.brush.clone())
}

fn names(app: &mut VectorcraftApp) -> Vec<String> {
    brushes(app).0.into_iter().map(|b| b.0).collect()
}

/// Click the panel menu's item `label` (frames at `time`, `time + 0.5`).
fn menu_click(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, time: f64) {
    let out = frame_with(ctx, app, vec![], egui::Modifiers::NONE, time, menu);
    let at = clickable(&out).into_iter().find(|(l, _)| l.as_deref() == Some(label)).unwrap_or_else(|| panic!("no `{label}`")).1.center();
    click(ctx, app, at, egui::Modifiers::NONE, time + 0.5, menu);
}

/// The first three brushes (the Calligraphic ones): a click on the first applies it to the
/// selected rectangle, and then a Shift-click on the third selects all three without applying
/// the third.
fn select_three(ctx: &egui::Context, app: &mut VectorcraftApp, rect: NodeId) -> (Vec<String>, Vec<Rect>) {
    let out = frame_with(ctx, app, vec![], egui::Modifiers::NONE, 0.0, show);
    let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).take(3).collect();
    let (list, _) = brushes(app);
    assert!(list[..3].iter().all(|b| b.1 == "calligraphic"), "{list:?}");
    let three: Vec<String> = list[..3].iter().map(|b| b.0.clone()).collect();
    click(ctx, app, tiles[0].center(), egui::Modifiers::NONE, 1.0, show);
    assert_eq!(brush_of(app, rect).as_deref(), Some(three[0].as_str()));
    click(ctx, app, tiles[2].center(), egui::Modifiers::SHIFT, 2.0, show);
    assert_eq!(brush_of(app, rect).as_deref(), Some(three[0].as_str()), "a Shift-click only selects");
    (three, tiles)
}

/// Shift-click selects a range of brushes and Cmd/Ctrl-click adds or removes one; the panel
/// highlights them, and Duplicate Brush and Delete Brush act on all of them in one undo step.
#[test]
fn shift_and_command_clicks_select_brushes_for_delete_and_duplicate() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let rect = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (three, tiles) = select_three(&ctx, &mut app, rect);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.5, show);
    let lit_tiles = |out: &egui::FullOutput| -> Vec<usize> { (0..3).filter(|i| lit(&ctx, out).contains(&tiles[*i])).collect() };
    assert_eq!(lit_tiles(&out), [0, 1, 2]);
    // Cmd/Ctrl-click takes the second out again.
    click(&ctx, &mut app, tiles[1].center(), egui::Modifiers::COMMAND, 3.0, show);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.5, show);
    assert_eq!(lit_tiles(&out), [0, 2]);
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = (names(&mut app), undo(&app));
    menu_click(&ctx, &mut app, "Duplicate Brush", 4.0);
    let copies = [format!("{} copy", three[0]), format!("{} copy", three[2])];
    assert_eq!(names(&mut app)[before.0.len()..], copies);
    assert_eq!(undo(&app), before.1 + 1);
    // The bottom bar's last button, Delete Brush.
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 5.0, show);
    let trash = clickable(&out).last().unwrap().1.center();
    click(&ctx, &mut app, trash, egui::Modifiers::NONE, 6.0, show);
    let left = names(&mut app);
    assert!(!left.contains(&three[0]) && !left.contains(&three[2]) && left.contains(&three[1]), "{left:?}");
    assert!(copies.iter().all(|c| left.contains(c)), "{left:?}");
    assert_eq!(brush_of(&app, rect), None);
    assert_eq!(undo(&app), before.1 + 2);
}

/// Selecting other art ends the multi-selection: selecting the first art again does not bring
/// it back, and with a path stroked with another brush selected, the panel highlights that
/// brush and Delete Brush deletes it alone.
#[test]
fn selecting_other_art_ends_the_brush_multi_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let rect = |app: &mut VectorcraftApp, x: f64| {
        NodeId(app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
    };
    let a = rect(&mut app, 10.0);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (three, tiles) = select_three(&ctx, &mut app, a);
    let lit_tiles = |out: &egui::FullOutput| -> Vec<usize> { (0..3).filter(|i| lit(&ctx, out).contains(&tiles[*i])).collect() };
    // A path without a brush, then the first one again: the current brush is highlighted.
    rect(&mut app, 60.0);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.0, show);
    assert_eq!(lit_tiles(&out), [0]);
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.5, show);
    assert_eq!(lit_tiles(&out), [0]);
    // A path stroked with the second brush.
    let c = rect(&mut app, 110.0);
    app.run("brush.apply", json!({"name": three[1]})).unwrap();
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 4.0, show);
    assert_eq!(lit_tiles(&out), [1]);
    menu_click(&ctx, &mut app, "Delete Brush", 5.0);
    let left = names(&mut app);
    assert!(left.contains(&three[0]) && !left.contains(&three[1]) && left.contains(&three[2]), "{left:?}");
    assert_eq!((brush_of(&app, a).as_deref(), brush_of(&app, c)), (Some(three[0].as_str()), None));
}

/// After other art is selected, a Shift-click selects the range from that art's brush, not from
/// the brush clicked before.
#[test]
fn a_shift_click_after_selecting_other_art_starts_from_its_brush() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
    let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).take(3).collect();
    let (list, _) = brushes(&mut app);
    click(&ctx, &mut app, tiles[0].center(), egui::Modifiers::NONE, 1.0, show);
    app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("brush.apply", json!({"name": list[2].0})).unwrap();
    click(&ctx, &mut app, tiles[1].center(), egui::Modifiers::SHIFT, 2.0, show);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.5, show);
    assert_eq!(lit(&ctx, &out).iter().filter(|r| tiles.contains(r)).collect::<Vec<_>>(), [&tiles[1], &tiles[2]]);
}

/// Cmd/Ctrl-clicks can deselect every brush: none is highlighted, and Delete Brush and
/// Duplicate Brush are off.
#[test]
fn command_clicks_deselect_every_brush() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let a = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (_, tiles) = select_three(&ctx, &mut app, a);
    for (i, tile) in tiles.iter().enumerate() {
        click(&ctx, &mut app, tile.center(), egui::Modifiers::COMMAND, 3.0 + i as f64, show);
    }
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 7.0, show);
    assert!(!lit(&ctx, &out).iter().any(|r| tiles.contains(r)));
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 8.0, menu);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    for label in ["Duplicate Brush", "Delete Brush"] {
        let item = nodes.iter().find(|(_, n)| n.label().map(str::trim) == Some(label)).unwrap_or_else(|| panic!("no `{label}`"));
        assert!(item.1.is_disabled(), "{label}");
    }
}

/// Select All Unused in the panel menu highlights every brush but the one the rectangle is
/// painted with, and Delete Brush then deletes all of them.
#[test]
fn select_all_unused_selects_the_brushes_no_stroke_paints_with() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let (list, _) = brushes(&mut app);
    let a = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
    app.run("brush.apply", json!({"name": list[1].0})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    menu_click(&ctx, &mut app, "Select All Unused", 1.0);
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.0, show);
    let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).collect();
    assert_eq!(tiles.len(), list.len());
    let on: Vec<usize> = (0..tiles.len()).filter(|i| lit(&ctx, &out).contains(&tiles[*i])).collect();
    assert_eq!(on, (0..tiles.len()).filter(|i| *i != 1).collect::<Vec<_>>());
    menu_click(&ctx, &mut app, "Delete Brush", 3.0);
    assert_eq!(names(&mut app), [list[1].0.clone()]);
    assert_eq!(brush_of(&app, a), Some(list[1].0.clone()));
}

#[test]
fn art_dropped_on_the_panel_becomes_an_art_brush_and_a_brush_drags_out() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, vec![]);
    let (before, _) = brushes(&mut app);
    // Art dragged off the canvas and released over the list.
    let over = pos2(100.0, 60.0);
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(id)]));
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over), button(over, false)]);
    // New Brush opens on Art for the dropped art, and OK twice makes the brush.
    let d = app.ui.dialog.as_ref().expect("New Brush opened");
    assert_eq!((d.kind.as_str(), d.str("type"), d.fields.get("__ids")), (crate::dialogs::new_brush::KIND, "art".into(), Some(&json!([id]))));
    assert_eq!(brushes(&mut app).0, before);
    crate::dialogs::confirm(&mut app).unwrap();
    crate::dialogs::confirm(&mut app).unwrap();
    let (after, _) = brushes(&mut app);
    assert_eq!(after.len(), before.len() + 1);
    let new: Vec<_> = after.iter().filter(|b| !before.contains(b)).collect();
    assert!(matches!(new.as_slice(), [(_, ty)] if ty == "art"), "{new:?}");
    // Dragging the first tile out of the panel carries that brush.
    frame(&ctx, &mut app, vec![]);
    let tile = pos2(36.0, 16.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
    let drag = egui::DragAndDrop::payload::<PanelDrag>(&ctx);
    assert!(matches!(drag.as_deref(), Some(PanelDrag::Brush { name, def }) if *name == after[0].0 && def["name"] == json!(name)), "{drag:?}");
}

/// A brush dragged onto the left half of another brush of its type goes before it, with a bar
/// marking the slot while it is held there, as one undo step; dropped on a brush of another
/// type, it stays where it is and the status bar says why.
#[test]
fn a_brush_dragged_onto_another_of_its_type_moves_before_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
    let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).collect();
    let (list, _) = brushes(&mut app);
    let order: Vec<String> = shown(&list, &[]).into_iter().map(str::to_string).collect();
    assert_eq!(order[..4], ["3 pt. Round", "6 pt. Flat", "10 pt. Oval", "Dots"]);
    let accent = Tokens::get(&ctx).accent;
    let bars = |out: &egui::FullOutput| -> Vec<f32> {
        let lines = out.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color == accent => Some(points[0].x),
            _ => None,
        });
        lines.collect()
    };
    // Press on a tile, drag it to `to`, hold it there for a frame (its output returned) and release.
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2, time: f64| -> egui::FullOutput {
        frame_with(&ctx, app, vec![egui::Event::PointerMoved(from), button(from, true)], egui::Modifiers::NONE, time, show);
        frame_with(&ctx, app, vec![egui::Event::PointerMoved(from + vec2(20.0, 0.0))], egui::Modifiers::NONE, time + 0.1, show);
        frame_with(&ctx, app, vec![egui::Event::PointerMoved(to)], egui::Modifiers::NONE, time + 0.2, show);
        let held = frame_with(&ctx, app, vec![], egui::Modifiers::NONE, time + 0.3, show);
        frame_with(&ctx, app, vec![button(to, false)], egui::Modifiers::NONE, time + 0.4, show);
        held
    };
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let steps = undo(&app);
    let calligraphic =
        |app: &mut VectorcraftApp| -> Vec<String> { brushes(app).0.into_iter().filter(|b| b.1 == "calligraphic").map(|b| b.0).collect() };
    // "10 pt. Oval" onto the left half of "3 pt. Round".
    let held = drag(&mut app, tiles[2].center(), tiles[0].center() - vec2(16.0, 0.0), 1.0);
    assert_eq!(bars(&held), [tiles[0].left() - 1.5]);
    assert_eq!(calligraphic(&mut app), ["10 pt. Oval", "3 pt. Round", "6 pt. Flat"]);
    assert_eq!(undo(&app), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(calligraphic(&mut app), ["3 pt. Round", "6 pt. Flat", "10 pt. Oval"]);
    // "3 pt. Round" onto "Dots", a scatter brush: no bar, and nothing moves.
    let held = drag(&mut app, tiles[0].center(), tiles[3].center(), 3.0);
    assert!(bars(&held).is_empty(), "{:?}", bars(&held));
    assert_eq!(brushes(&mut app).0, list);
    assert_eq!(undo(&app), steps);
    assert!(app.ui.status.contains("brushes move only within their type"), "{}", app.ui.status);
}

/// Opaque pixels of brush definition `def`'s 80 × 40 pt preview with the resources of `doc`,
/// and those of them that are Pinstripes blue.
fn pixels(doc: &Document, def: &Value) -> (usize, usize) {
    let preview = preview_doc(doc, def, 80.0, 40.0).expect("a preview");
    let img = vectorcraft_render::Renderer::new().render_region(&preview, vectorcraft_geom::Rect::new(0.0, 0.0, 80.0, 40.0), 2.0, false);
    let painted: Vec<&[u8; 4]> = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 200).collect();
    let blue = painted.iter().filter(|p| p[2] > 170 && p[0] < 90 && (100..160).contains(&p[1])).count();
    (painted.len(), blue)
}

/// A library brush's preview shows the pattern its art is filled with: it renders with the
/// library's resources, which a bare document doesn't have.
#[test]
fn library_brush_previews_show_the_patterns_their_art_uses() {
    let s = Session::new();
    let (_, lib) = brushlib::library(&s, "art-ribbons").unwrap();
    let def = serde_json::to_value(lib.brush("Striped Ribbon").unwrap()).unwrap();
    let (painted, blue) = pixels(&lib.doc, &def);
    assert!(painted > 0 && blue * 5 >= painted, "Pinstripes paints it: {painted} painted, {blue} blue");
    let (painted, blue) = pixels(&Document::new(10.0, 10.0), &def);
    assert_eq!(blue, 0, "without the pattern: {painted} painted");
    // A square tile still shows a stroke.
    let square = preview_doc(&lib.doc, &def, 40.0, 40.0).unwrap();
    let img = vectorcraft_render::Renderer::new().render_region(&square, vectorcraft_geom::Rect::new(0.0, 0.0, 40.0, 40.0), 2.0, false);
    assert!(img.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 200));
}

#[test]
fn a_library_brush_dropped_on_the_panel_is_added() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, vec![]);
    let at = pos2(120.0, 40.0);
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::LibraryBrush { library: "pattern-borders".into(), name: "Beads".into() });
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, &mut app, vec![button(at, false)]);
    assert!(vectorcraft_brush::find(&app.session.active().unwrap().doc, "Beads").is_some());
}

/// A preview is cached by its brush and the patterns and symbols its art shows: adding other
/// brushes (and the patterns and symbols they bring) renders only the new one, every preview
/// is drawn in the first frame, and changing a pattern a brush shows renders that brush again.
/// Symbol thumbnails are keyed the same way.
#[test]
fn previews_render_again_only_when_what_they_show_changes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let count = |app: &mut VectorcraftApp| brushes(app).0.len();
    let n = count(&mut app);
    assert_eq!(counted(&ctx, &mut app), (n, n), "the first frame renders and draws every preview");
    assert_eq!(counted(&ctx, &mut app), (0, n), "then they are cached");
    // Striped Ribbon brings its pattern, Sparkles its symbol: only they render.
    app.run("brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Striped Ribbon"})).unwrap();
    assert_eq!(counted(&ctx, &mut app), (1, n + 1));
    let symbol_key = |app: &VectorcraftApp, name: &str| super::super::symbols::document_symbols(app.session.active().unwrap()).key(name).unwrap_or(0);
    let sparkle_before = symbol_key(&app, "Sparkle");
    app.run("brush.addFromLibrary", json!({"library": "scatter-shapes", "name": "Sparkles"})).unwrap();
    assert_eq!(counted(&ctx, &mut app), (1, n + 2));
    // A symbol added beside Sparkle leaves its thumbnail key alone.
    let sparkle = symbol_key(&app, "Sparkle");
    assert_ne!(sparkle, sparkle_before, "0 before Sparkle came in");
    app.run("symbol.addFromLibrary", json!({"library": "basic-shapes", "name": "Star"})).unwrap();
    assert_eq!(symbol_key(&app, "Sparkle"), sparkle);
    assert_eq!(counted(&ctx, &mut app), (0, n + 2));
    // A new Pinstripes tile: Striped Ribbon renders again, the others don't.
    app.session
        .edit("Pattern", |d, _| {
            let p = d.patterns.iter_mut().find(|p| p.name == "Pinstripes").unwrap();
            let dot =
                Node::path(NodeId(0), vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 3.0, 3.0)), Appearance::default_art());
            p.art = vec![std::sync::Arc::new(dot)];
            Ok(())
        })
        .unwrap();
    assert_eq!(counted(&ctx, &mut app), (1, n + 2));
}

fn keyed() -> usize {
    counters::KEYED.with(|k| k.replace(0))
}

/// The Brushes panel serialises and hashes the brushes of a document version once: frames
/// after the first, of it or of it again after another document was active, do neither; an
/// edit, an undo and a redo each work them out once again and render no preview again. Brush
/// Options works its preview's key out again only when its definition or the document changes.
#[test]
fn frames_after_the_first_serialise_and_hash_no_brush() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let n = vectorcraft_brush::library(&app.session.active().unwrap().doc).len();
    keyed();
    assert_eq!(counted(&ctx, &mut app), (n, n));
    assert_eq!(keyed(), n, "the first frame works each brush out");
    for _ in 0..3 {
        assert_eq!(counted(&ctx, &mut app), (0, n));
    }
    assert_eq!(keyed(), 0, "later frames serialise and hash none");
    // Another document, then this one again.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    frame(&ctx, &mut app, vec![]);
    keyed();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(counted(&ctx, &mut app), (0, n));
    assert_eq!(keyed(), 0, "the document's brushes are still known");
    // An edit to something else: the brushes are worked out again, no preview renders.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(counted(&ctx, &mut app), (0, n));
    assert_eq!(keyed(), n);
    for cmd in ["edit.undo", "edit.redo"] {
        app.run(cmd, json!({})).unwrap();
        assert_eq!(counted(&ctx, &mut app), (0, n), "{cmd}");
        frame(&ctx, &mut app, vec![]);
        assert!(keyed() <= n, "{cmd}: once at most");
    }
    // Brush Options' preview of a definition.
    let (_, lib) = brushlib::library(&app.session, "art-ribbons").unwrap();
    let mut def = serde_json::to_value(lib.brush("Striped Ribbon").unwrap()).unwrap();
    let options = |app: &VectorcraftApp, def: &Value| {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 100.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| definition_chip(ui, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 40.0)), app.session.active(), def));
        out.textures_delta.clear();
    };
    options(&app, &def);
    assert_eq!(keyed(), 1);
    for _ in 0..3 {
        options(&app, &def);
    }
    assert_eq!(keyed(), 0, "the same definition in the same document");
    def["width"] = json!(150.0);
    options(&app, &def);
    options(&app, &def);
    assert_eq!(keyed(), 1, "a field changed");
    app.run("shape.rectangle", json!({"x": 40, "y": 10, "width": 20, "height": 20})).unwrap();
    options(&app, &def);
    options(&app, &def);
    assert_eq!(keyed(), 1, "the document changed");
}

/// One headless frame of the Symbols panel → thumbnails rendered in it.
fn symbols_rendered(ctx: &egui::Context, app: &mut VectorcraftApp) -> usize {
    use super::super::symbols::{self, counters::RENDERED};
    RENDERED.with(|c| c.set(0));
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| symbols::show(app, ui));
    out.textures_delta.clear();
    RENDERED.with(std::cell::Cell::get)
}

/// A 4 × 4 PNG of colour `rgb`.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Brush previews and symbol thumbnails render again when a swatch, an image, or a pattern's
/// tile type, overlap or spacing their art uses changes (only those that use it), and not when
/// one they don't use does.
#[test]
fn previews_follow_the_swatches_images_and_pattern_options_their_art_uses() {
    use vectorcraft_doc::pattern::{Overlap, TileType};
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind, Symbol};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    // Striped Ribbon brings Pinstripes.
    app.run("brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Striped Ribbon"})).unwrap();
    let (_, lib) = brushlib::library(&app.session, "art-ribbons").unwrap();
    let wave = lib.brush("Wave").unwrap().clone();
    let striped = lib.brush("Striped Ribbon").unwrap().art()[0].clone();
    let ink = Paint::Solid { color: Color::BLACK, swatch: Some("Ink".into()), tint: 1.0 };
    let inked = Node::path(
        NodeId(0),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 10.0)),
        Appearance::basic(ink, Paint::None, 0.0),
    );
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 4,
        xf: vectorcraft_geom::Affine::scale(5.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let pictured = Node::new(NodeId(0), NodeKind::Image(Box::new(im)));
    // Filled with the gradient swatch Fade.
    let fade = |c: Color| {
        use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
        GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, c)]))
    };
    let mut faded_paint = fade(Color::BLACK);
    faded_paint.swatch = Some("Fade".into());
    let faded = Node::path(
        NodeId(0),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 10.0)),
        Appearance::basic(Paint::Gradient(Box::new(faded_paint)), Paint::None, 0.0),
    );
    app.session
        .edit("Resources", |d, _| {
            for name in ["Ink", "Other"] {
                d.swatches.push(vectorcraft_color::Swatch { name: name.into(), paint: Paint::solid(Color::BLACK), global: true, spot: false });
            }
            let paint = Paint::Gradient(Box::new(fade(Color::BLACK)));
            d.swatches.push(vectorcraft_color::Swatch { name: "Fade".into(), paint, global: false, spot: false });
            d.images.insert("pic".into(), ImageBlob::png(png([255, 0, 0])));
            let mut brushes = vectorcraft_brush::library(d);
            for (name, art) in [("Inked", &inked), ("Pictured", &pictured), ("Faded", &faded)] {
                let mut b = wave.clone();
                b.name = name.into();
                if let vectorcraft_brush::BrushKind::Art(a) = &mut b.kind {
                    a.art = art.clone();
                }
                brushes.push(b);
            }
            vectorcraft_brush::store(d, &brushes);
            for (name, art) in [("Ink Symbol", &inked), ("Picture Symbol", &pictured), ("Stripes Symbol", &striped), ("Fade Symbol", &faded)] {
                d.symbols.push(Symbol { name: name.into(), art: Arc::new(art.clone()) });
            }
            Ok(())
        })
        .unwrap();
    let ctx = egui::Context::default();
    let n = brushes(&mut app).0.len();
    assert_eq!((counted(&ctx, &mut app).0, symbols_rendered(&ctx, &mut app)), (n, 4));
    assert_eq!((counted(&ctx, &mut app).0, symbols_rendered(&ctx, &mut app)), (0, 0));
    let change = |app: &mut VectorcraftApp, what: &str, f: &dyn Fn(&mut Document)| {
        app.session
            .edit("Change", |d, _| {
                f(d);
                Ok(())
            })
            .unwrap();
        (counted(&ctx, app).0, symbols_rendered(&ctx, app), what.to_string())
    };
    let swatch =
        |name: &'static str| move |d: &mut Document| d.swatches.iter_mut().find(|w| w.name == name).unwrap().paint = Paint::solid(Color::WHITE);
    fn pinstripes(d: &mut Document) -> &mut vectorcraft_doc::pattern::PatternDef {
        d.patterns.iter_mut().find(|p| p.name == "Pinstripes").unwrap()
    }
    type Case<'a> = (&'a str, (usize, usize), &'a dyn Fn(&mut Document));
    let cases: [Case; 8] = [
        ("Ink", (1, 1), &swatch("Ink")),
        ("Other", (0, 0), &swatch("Other")),
        ("Fade", (1, 1), &|d| d.swatches.iter_mut().find(|w| w.name == "Fade").unwrap().paint = Paint::Gradient(Box::new(fade(Color::WHITE)))),
        ("pic", (1, 1), &|d| {
            d.images.insert("pic".into(), ImageBlob::png(png([0, 0, 255])));
        }),
        ("another image", (0, 0), &|d| {
            d.images.insert("pic2".into(), ImageBlob::png(png([0, 255, 0])));
        }),
        ("tile type", (1, 1), &|d| pinstripes(d).tile_type = TileType::BrickByRow { offset: 0.5 }),
        ("overlap", (1, 1), &|d| pinstripes(d).overlap = Overlap { right_in_front: true, bottom_in_front: true }),
        ("spacing", (1, 1), &|d| pinstripes(d).h_spacing = 4.0),
    ];
    for (what, (brushes, symbols), f) in cases {
        assert_eq!(change(&mut app, what, f), (brushes, symbols, what.to_string()));
    }
}
