//! Object › Path › Remove Anchor Points: the menu, the Control bar, the contextual task bar, the
//! Properties panel and the context menu run it on direct-selected anchors. The Control bar's and
//! the Properties panel's other anchor buttons: convert to corner or smooth, connect, cut.

use egui::{Event, PointerButton, Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::canvas::Xf;
use crate::menus::{self, Item};
use crate::{VectorcraftApp, canvas, chrome};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp) -> NodeId {
    NodeId(app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap())
}

fn anchor_count(app: &VectorcraftApp, id: NodeId) -> usize {
    app.session.active().and_then(|st| st.doc.node(id)).and_then(|n| n.path_data()).map(|p| p.anchor_count()).unwrap_or(0)
}

fn labels(items: &[Item]) -> Vec<&'static str> {
    items
        .iter()
        .flat_map(|it| match it {
            Item::Cmd(l, ..) => vec![*l],
            Item::Sub(l, ch) => std::iter::once(*l).chain(labels(ch)).collect(),
            _ => vec![],
        })
        .collect()
}

pub(crate) fn shapes_text(shapes: &[Shape]) -> Vec<(String, Rect)> {
    fn walk(s: &Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut v = vec![];
    shapes.iter().for_each(|s| walk(s, &mut v));
    v
}

pub(crate) fn control_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| chrome::control_bar(app, ui));
    out.textures_delta.clear();
    shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

pub(crate) fn click_control(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    control_frame(app, ctx, vec![Event::PointerMoved(at), press(true)]);
    control_frame(app, ctx, vec![press(false)]);
    control_frame(app, ctx, vec![]);
}

fn canvas_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

fn click_canvas(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, button: PointerButton) -> Vec<(String, Rect)> {
    let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() };
    canvas_frame(app, ctx, vec![Event::PointerMoved(at)]);
    canvas_frame(app, ctx, vec![press(true)]);
    canvas_frame(app, ctx, vec![press(false)]);
    canvas_frame(app, ctx, vec![])
}

pub(crate) fn properties_frame(app: &mut VectorcraftApp, width: f32) -> Vec<(String, Rect)> {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 900.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
        crate::panels::properties::show(app, ui);
    });
    out.textures_delta.clear();
    shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

pub(crate) fn has(texts: &[(String, Rect)], label: &str) -> bool {
    texts.iter().any(|(t, _)| t == label)
}

pub(crate) fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}`"))
}

#[test]
fn the_path_menu_and_bars_run_it_for_direct_selected_anchors() {
    const LABEL: &str = "Remove Anchor Points";
    let mut app = app();
    let id = rect(&mut app);
    assert_eq!(anchor_count(&app, id), 4);
    let entries: Vec<_> = menus::menu_entries(&app).into_iter().filter(|e| e.label == LABEL).collect();
    assert_eq!(entries.len(), 1, "one Remove Anchor Points item");
    assert_eq!(entries[0].command.as_deref(), Some("path.removeAnchors"));
    assert_eq!(entries[0].path, ["Object", "Path"]);
    assert!(!entries[0].enabled, "an object selection is not enough");
    assert!(!labels(&menus::context_items(&app)).contains(&LABEL));
    assert!(crate::palette::items().iter().any(|(_, cmd, _)| cmd == "path.removeAnchors"));

    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    assert!(menus::enabled(&app, "path.removeAnchors"));
    assert!(labels(&menus::context_items(&app)).contains(&LABEL));

    // The Control bar's icon follows its "Anchors:" label.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click_button(&mut app, &ctx, "Anchors:", 0);
    assert_eq!(anchor_count(&app, id), 3, "the Control bar button removes the anchor");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.last().unwrap().label, LABEL);
    let sp = &st.doc.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(sp.closed, "the path stays closed");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(anchor_count(&app, id), 4);

    app.run("select.anchors", json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    // 230 pt is the dock's minimum width. Each row of anchor buttons has to fit in it.
    let props = properties_frame(&mut app, 230.0);
    for label in ["Convert:", "Anchors:"] {
        let row = props.iter().find(|(t, _)| t == label).map(|(_, r)| *r).expect("properties");
        assert!(row.min.x >= -0.5 && button(&props, &ctx, label, 2).x + 12.0 <= 230.5, "properties clips the {label} row: {row:?}");
    }

    // The task bar's area settles on the second frame.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    canvas_frame(&mut app, &ctx, vec![]);
    let texts = canvas_frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, LABEL), "task bar: {texts:?}");

    // Right-click keeps the anchor selection (the object is already selected) and runs the item.
    let p = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(100.0, 90.0));
    let texts = click_canvas(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(has(&texts, LABEL), "context menu: {texts:?}");
    let texts = click_canvas(&mut app, &ctx, at(&texts, LABEL), PointerButton::Primary);
    assert_eq!(anchor_count(&app, id), 3);
    assert!(!has(&texts, "Cut Path at Selected Anchor Points"), "the menu closes after the command");
    // The next anchor along is selected (#2241), so the task bar offers the command again.
    assert_eq!(texts.iter().filter(|(t, _)| t == LABEL).count(), 1, "task bar: {texts:?}");
}

#[test]
fn the_control_bar_and_properties_hide_it_without_anchors() {
    let mut app = app();
    rect(&mut app);
    let props = crate::tests_labels::painted_text(&mut app, crate::panels::properties::show);
    assert!(!props.contains("Anchors:"), "{props}");
}

/// The centre of the `i`-th 24 pt icon button after `label`.
fn button(texts: &[(String, Rect)], ctx: &egui::Context, label: &str, i: usize) -> Pos2 {
    let r = texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("no `{label}`"));
    let gap = ctx.global_style().spacing.item_spacing.x;
    Pos2::new(r.max.x + gap + 12.0 + i as f32 * (24.0 + gap), r.center().y)
}

/// Click the Control bar's `i`-th icon button after `label`.
fn click_button(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, i: usize) {
    let at = button(&control_frame(app, ctx, vec![]), ctx, label, i);
    click_control(app, ctx, at);
}

#[test]
fn the_control_bar_converts_connects_and_cuts_selected_anchors() {
    let mut app = app();
    let id = rect(&mut app);
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let anchor = |app: &VectorcraftApp, id: NodeId, ai: usize| {
        app.session.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[ai]
    };
    let last_undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().unwrap().label.clone();
    // Convert: smooth, then corner again.
    click_button(&mut app, &ctx, "Convert:", 1);
    assert!(anchor(&app, id, 1).has_in() && anchor(&app, id, 1).has_out(), "smooth");
    assert_eq!(last_undo(&app), "Convert Anchor Points");
    click_button(&mut app, &ctx, "Convert:", 0);
    assert!(!anchor(&app, id, 1).has_in() && !anchor(&app, id, 1).has_out(), "corner");
    // Cut: the rectangle opens at the anchor, one of its two ends selected.
    click_button(&mut app, &ctx, "Anchors:", 2);
    assert_eq!(last_undo(&app), "Cut Path");
    let st = app.session.active().unwrap();
    let sp = &st.doc.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(!sp.closed && sp.anchors.len() == 5, "{sp:?}");
    assert_eq!(st.selection.anchors[&id].iter().collect::<Vec<_>>(), [&(0, 0)]);
    // Connect: closes it again.
    click_button(&mut app, &ctx, "Anchors:", 1);
    assert_eq!(last_undo(&app), "Join");
    assert!(app.session.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].closed);
    assert_eq!(anchor_count(&app, id), 4, "the coincident ends merge");
}

/// #504: with a tool that edits anchors (Direct Selection, the Pen…), a path selected as a whole
/// shows all its anchors selected, and the Control bar and the Properties panel offer to convert
/// them; removing and cutting wait for anchors direct-selected. The Selection tool shows neither.
#[test]
fn a_whole_path_converts_with_the_tools_that_edit_anchors() {
    let mut app = app();
    let id = rect(&mut app);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.select_tool("selection");
    assert!(!has(&control_frame(&mut app, &ctx, vec![]), "Convert:"));
    for tool in ["directSelection", "pen"] {
        app.select_tool(tool);
        let texts = control_frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Convert:") && !has(&texts, "Anchors:"), "{tool}: {texts:?}");
        let props = crate::tests_labels::painted_text(&mut app, crate::panels::properties::show);
        assert!(props.contains("Convert:") && !props.contains("Anchors:"), "{tool}: {props}");
    }
    click_button(&mut app, &ctx, "Convert:", 1);
    let sp = app.session.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
    assert!(sp.anchors.iter().all(|a| a.has_in() && a.has_out()), "every corner made smooth: {sp:?}");
}

/// Use the normal application frame: canvas pointer picking and the keyboard shortcut dispatcher.
fn anchor_delete_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0));
    let time = ctx.input(|i| i.time) + 0.1;
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

#[test]
fn open_anchor_delete_and_backspace_cut_the_clicked_anchor_then_clear() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let id = NodeId(
            app.run(
                "path.create",
                json!({"anchors": [
                    {"x": 50, "y": 100}, {"x": 100, "y": 80}, {"x": 150, "y": 100},
                    {"x": 200, "y": 120}, {"x": 250, "y": 100}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        app.select_tool("directSelection");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for _ in 0..3 {
            anchor_delete_frame(&mut app, &ctx, vec![]);
        }
        let at = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(150.0, 100.0));
        let click = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        anchor_delete_frame(&mut app, &ctx, vec![Event::PointerMoved(at), click(true)]);
        anchor_delete_frame(&mut app, &ctx, vec![click(false)]);
        assert_eq!(app.session.tool_id(), "directSelection");
        let st = app.session.doc().unwrap();
        assert_eq!(st.selection.anchors[&id], [(0, 2)].into(), "the real canvas click selected the interior anchor");
        assert!(st.selection.segments.is_empty());
        let before = st.doc.clone();
        let selection = st.selection.clone();
        let undo = st.history.undo.len();
        let press = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        anchor_delete_frame(&mut app, &ctx, vec![press(true)]);
        anchor_delete_frame(&mut app, &ctx, vec![press(false)]);
        let st = app.session.doc().unwrap();
        let path = st.doc.node(id).unwrap().path_data().unwrap();
        let points: Vec<Vec<_>> = path.subpaths.iter().map(|sp| sp.anchors.iter().map(|a| a.p).collect()).collect();
        assert_eq!(
            points,
            vec![vec![Point::new(50.0, 100.0), Point::new(100.0, 80.0)], vec![Point::new(200.0, 120.0), Point::new(250.0, 100.0)]],
            "{key:?} must not join the gap"
        );
        assert!(path.subpaths.iter().all(|sp| !sp.closed));
        assert_eq!(st.selection.objects, vec![id]);
        assert!(st.selection.anchors.is_empty() && st.selection.segments.is_empty());
        assert_eq!(st.history.undo.len(), undo + 1);
        let after = st.doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
        anchor_delete_frame(&mut app, &ctx, vec![press(true)]);
        anchor_delete_frame(&mut app, &ctx, vec![press(false)]);
        assert!(app.session.doc().unwrap().doc.node(id).is_none(), "the next {key:?} clears the whole remainder");
    }
}
