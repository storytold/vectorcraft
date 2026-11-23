use egui::accesskit::{Node, Role};
use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use craft::ui::dialog::DialogId;
use craft::ui::rails::ToolTips as _;

use super::*;

/// A fresh app with a document `width` by `height` points.
fn app_with(width: u32, height: u32) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    app
}

/// A context that makes screen-reader nodes, set up as the app runs.
fn reader() -> egui::Context {
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    ctx
}

/// One whole app frame in a window 1200 points wide and `height` tall; returns the screen-reader nodes.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>, height: f32) -> Vec<Node> {
    let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, height)));
    let mut out = crate::testing::app_frame(app, ctx, egui::RawInput { time: Some(time), events, screen_rect, ..Default::default() });
    out.platform_output.accesskit_update.take().map(|update| update.nodes.into_iter().map(|(_, node)| node).collect()).unwrap_or_default()
}

/// Three frames from `t0`, enough for the dock to measure; returns the last one's nodes.
fn settled(app: &mut VectorcraftApp, ctx: &egui::Context, t0: f64, height: f32) -> Vec<Node> {
    frame(app, ctx, t0, vec![], height);
    frame(app, ctx, t0 + 0.05, vec![], height);
    frame(app, ctx, t0 + 0.1, vec![], height)
}

/// A click at `at`, then a settled frame; returns that frame's nodes.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2, height: f32) -> Vec<Node> {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, time, vec![Event::PointerMoved(at), button(true)], height);
    frame(app, ctx, time + 0.05, vec![button(false)], height);
    frame(app, ctx, time + 0.1, vec![], height)
}

/// One frame of the canvas alone over the view's rect: a fit or zoom a command asked for waits for the canvas (a view
/// request, canvas-w10 note 3).
fn canvas_pass(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    let screen_rect = app.view().map(|v| v.rect);
    crate::testing::part_frame(app, &ctx, egui::RawInput { screen_rect, ..Default::default() }, crate::canvas::show);
}

/// Where a screen-reader node is drawn.
fn bounds(node: &Node) -> Option<egui::Rect> {
    let b = node.bounds()?;
    Some(egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

/// Where the node named `label` is drawn.
fn named_rect(nodes: &[Node], label: &str) -> egui::Rect {
    nodes.iter().find(|n| n.label() == Some(label)).and_then(bounds).unwrap_or_else(|| panic!("{label} is drawn"))
}

/// The name of `tool`'s button: the rail names a tool by its label (its shortcut is in its tip).
fn button_name(tool: &ToolInfo) -> String {
    tl!(tool.label).to_string()
}

/// Where the button of `tool` is drawn, found by its name among the rail's tool buttons.
fn tool_rect(nodes: &[Node], tool: &str) -> egui::Rect {
    let name = tool_info(tool).map(button_name).unwrap_or_default();
    nodes.iter().find(|n| n.toggled().is_some() && n.label() == Some(name.as_str())).and_then(bounds).unwrap_or_else(|| panic!("{name} is drawn"))
}

/// The rects of the Tools rail's tool buttons, top to bottom then left to right.
fn cells(app: &VectorcraftApp, nodes: &[Node]) -> Vec<egui::Rect> {
    let names: Vec<String> = slots(app).iter().flat_map(|(_, tools)| tools.iter()).filter_map(|id| tool_info(id).map(button_name)).collect();
    let mut found: Vec<egui::Rect> = nodes
        .iter()
        .filter(|n| n.role() == Role::Button && n.toggled().is_some() && n.label().is_some_and(|l| names.iter().any(|name| name == l)))
        .filter_map(bounds)
        .collect();
    found.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    found
}

use crate::testing::windowed;

/// The open tool-options dialog: its id and the fields automation reads.
fn tool_dialog(app: &VectorcraftApp) -> Option<(DialogId, serde_json::Value)> {
    app.dialogs()?.with(|host| host.open_dialogs().pop()).map(|d| (d.id, d.fields.unwrap_or_default()))
}

/// The field `key` of the open tool-options dialog.
fn field(app: &VectorcraftApp, key: &str) -> Option<serde_json::Value> {
    tool_dialog(app)?.1.get(key).cloned()
}

/// Types `value` into the field `key` of the open tool-options dialog; a value the dialog refuses fails the test.
fn set_field(app: &VectorcraftApp, key: &str, value: serde_json::Value) {
    let (id, _) = tool_dialog(app).expect("the dialog is open");
    let ctx = app.ui_ctx.clone().expect("a window");
    app.dialogs().expect("a window").with(|host| host.set_json_field(&ctx, id, key, value)).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// Answers the open tool-options dialog with OK or Cancel; the frame's pump takes the answer. A held OK files the
/// answer for the pump and the dialog closes once the pump accepted it.
fn answer(app: &mut VectorcraftApp, ok: bool) {
    let (id, _) = tool_dialog(app).expect("the dialog is open");
    let ctx = app.ui_ctx.clone().expect("a window");
    if ok {
        crate::tests::picks::answer_dialog(app, id, &[], true);
    } else {
        app.dialogs().expect("a window").with(|host| host.cancel_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
    }
    assert!(tool_dialog(app).is_none_or(|(open, _)| open != id), "the dialog took the answer");
}

/// A double-click on a tool's button opens what `open_options` opens: the artboard's, a graph's and the tools' dialogs
/// (each with what it needs selected), the view fits for Hand and Zoom, and a tool without options refuses.
#[test]
fn tools_open_their_options() {
    let mut app = windowed(app_with(300, 200));
    let kind = |app: &mut VectorcraftApp, tool: &str| open_options(app, tool).unwrap()["dialog"].clone();
    // Without the Artboard tool, the window's artboard; with it, its active artboard.
    assert_eq!(kind(&mut app, "artboard"), "artboardOptions");
    app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    app.run("tool.setOption", json!({"key": "active", "value": 1})).unwrap();
    assert_eq!(kind(&mut app, "artboard"), "artboardOptions");
    // Graph Type needs a selected graph.
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(open_options(&mut app, "pieGraph").is_err());
    app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
    assert_eq!(kind(&mut app, "pieGraph"), "graphType");
    // Nothing selected: the transform and selection tools refuse, as their menu items are disabled.
    app.run("select.set", json!({"ids": []})).unwrap();
    for tool in ["rotate", "scale", "reflect", "shear", "selection", "directSelection", "groupSelection"] {
        assert_eq!(open_options(&mut app, tool), Err("nothing selected".into()), "{tool}");
    }
    assert!(open_options(&mut app, "lasso").is_err(), "a tool without options");
    // Hand fits the active artboard in the window, Zoom goes to 100%. The Artboard tool made the second active above:
    // the first is the active one again.
    crate::panels::artboards::select(&mut app, 0);
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(660.0, 460.0));
    app.run("view.setZoom", json!({"zoom": 333, "center": [10, 10]})).unwrap();
    open_options(&mut app, "hand").unwrap();
    canvas_pass(&mut app);
    let v = *app.view().unwrap();
    assert_eq!((v.center.x, v.center.y, v.zoom), (150.0, 100.0, 2.0));
    open_options(&mut app, "zoom").unwrap();
    canvas_pass(&mut app);
    assert_eq!(app.view().unwrap().zoom, 1.0);
    // With an object selected, the transform tools open their dialogs and the selection tools open Move.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    for (tool, dialog) in [
        ("rotate", "rotate"),
        ("scale", "scale"),
        ("reflect", "reflect"),
        ("shear", "shear"),
        ("selection", "move"),
        ("directSelection", "move"),
        ("groupSelection", "move"),
    ] {
        assert_eq!(kind(&mut app, tool), dialog, "{tool}");
    }
    // Line, arc and the grids open their tool options; the eyedropper its options.
    for tool in ["lineSegment", "arc", "rectangularGrid", "polarGrid", "eyedropper"] {
        open_options(&mut app, tool).unwrap_or_else(|e| panic!("{tool}: {e}"));
    }
}

/// Type on a Path opens its options for the selected type on a path only: nothing selected and a path without type refuse.
#[test]
fn type_on_a_path_tools_open_options_for_the_selected_type_on_a_path() {
    for tool in ["typeOnPath", "verticalTypeOnPath"] {
        let mut app = app_with(600, 400);
        app.select_tool(tool);
        assert_eq!(open_options(&mut app, tool), Err("nothing selected".into()), "{tool}");
        let path = app.run("path.create", json!({"d": "M100 200 L500 200"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [path]})).unwrap();
        let e = open_options(&mut app, tool).unwrap_err();
        assert!(e.contains("`type.pathOptions`: no text objects selected"), "{tool}: {e}");
        let r = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [300, 210]})).unwrap();
        app.run("select.set", json!({"ids": [r["id"]]})).unwrap();
        open_options(&mut app, tool).unwrap_or_else(|e| panic!("{tool}: {e}"));
    }
}

/// A click on the Blend tool's button while the Blend tool holds a first pick finishes the
/// blend: the tool stays chosen, and the next click on the canvas picks a new first object
/// instead of blending with the first.
#[test]
fn clicking_the_active_blend_tool_button_finishes_the_blend() {
    let mut app = app_with(600, 300);
    app.run("window.toolbarAdvanced", json!({})).unwrap();
    for x in [0, 200] {
        app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap();
    }
    app.select_tool("blend");
    let ctx = reader();
    let at = tool_rect(&settled(&mut app, &ctx, 0.0, 2000.0), "blend").center();
    let down = |x, y| vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Down, x, y);
    let view = app.view_info();
    app.session.pointer(&down(20.0, 20.0), view).unwrap();
    assert!(app.session.tool_busy(), "the first rectangle is picked");
    let layers = app.session.active().unwrap().doc.layers.clone();
    click(&mut app, &ctx, 1.0, at, 2000.0);
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "the click on the button drops the pick");
    app.session.pointer(&down(220.0, 20.0), view).unwrap();
    assert!(app.session.tool_busy(), "the second rectangle is a new first pick");
    assert_eq!(app.session.active().unwrap().doc.layers, layers, "no blend was made");
}

#[test]
fn polygon_and_star_canvas_counts_persist_without_new_toolbar_dialogs() {
    for (tool, key, count) in [("polygon", "sides", 3), ("star", "points", 7)] {
        let mut app = windowed(app_with(300, 300));
        app.select_tool(tool);
        assert!(open_options(&mut app, tool).is_err(), "upstream toolbar opening behavior stays unchanged");
        assert!(tool_dialog(&app).is_none());
        let objects =
            |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum::<usize>();
        crate::dialogs::open_tool_dialog(&mut app, tool, json!({"x":120,"y":130}));
        assert!(field(&app, if tool == "polygon" { "radius" } else { "radius1" }).is_some());
        set_field(&app, key, json!(count));
        answer(&mut app, true);
        assert_eq!(objects(&app), 1, "deliberate exact-size placement");
        assert_eq!(app.session.tool_options()[key], count);
        for round in 0..3 {
            if round == 2 {
                app.select_tool("ellipse");
                app.select_tool(tool);
            }
            let view = app.view_info();
            for (kind, x, y) in [
                (vectorcraft_tools::PointerKind::Down, 50.0, 50.0),
                (vectorcraft_tools::PointerKind::Drag, 90.0, 90.0),
                (vectorcraft_tools::PointerKind::Up, 90.0, 90.0),
            ] {
                app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, x, y), view).unwrap();
            }
            assert_eq!(objects(&app), round + 2);
            let st = app.session.active().unwrap();
            let node = st.doc.node(*st.selection.objects.first().unwrap()).unwrap();
            assert_eq!(node.path_data().unwrap().anchor_count(), if tool == "polygon" { 3 } else { 14 });
        }
        crate::dialogs::open_tool_dialog(&mut app, tool, json!({"x":140,"y":150}));
        assert_eq!(field(&app, key), Some(json!(count)), "reopening uses the chosen count");
        set_field(&app, key, json!(9));
        answer(&mut app, false);
        assert_eq!(objects(&app), 4);
        assert_eq!(app.session.tool_options()[key], count, "Cancel keeps the count");
    }
}

/// The paint chips, drawing modes and the rest of the footer lie inside the rail at one and at two columns (one column
/// shows the current drawing mode alone), and in two columns the three drawing modes sit side by side, each setting
/// its mode (#1129).
#[test]
fn the_paint_and_drawing_mode_buttons_fit_the_toolbar_width() {
    let mut app = app_with(200, 200);
    let ctx = reader();
    let footer =
        ["Color (, or Shift+,)", "Gradient (. or Shift+.)", "None (/)", "Draw Normal (Shift+D)", "Draw Behind (Shift+D)", "Draw Inside (Shift+D)"];
    let mut nodes = vec![];
    for (k, double) in [false, true].into_iter().enumerate() {
        app.run("window.toolbarColumns", json!({ "double": double })).unwrap();
        nodes = settled(&mut app, &ctx, k as f64 * 10.0, 900.0);
        // The rail itself, not its dock grip, which is named for the pane too.
        let rail = nodes.iter().find(|n| n.role() == Role::Toolbar && n.label() == Some("Tools")).and_then(bounds).expect("the rail is drawn");
        for label in &footer[..if double { 6 } else { 4 }] {
            let r = named_rect(&nodes, label);
            assert!(rail.expand(0.5).contains_rect(r), "double {double}: {label} {r:?} in {rail:?}");
        }
    }
    let modes = ["Draw Normal (Shift+D)", "Draw Behind (Shift+D)", "Draw Inside (Shift+D)"].map(|label| named_rect(&nodes, label));
    assert!(modes.iter().all(|r| (r.top() - modes[0].top()).abs() < 0.5), "side by side: {modes:?}");
    click(&mut app, &ctx, 20.0, modes[1].center(), 900.0);
    assert_eq!(app.session.draw_mode, vectorcraft_engine::DrawMode::Behind);
}

/// How many columns of tool buttons `rects` make.
fn columns(rects: &[egui::Rect]) -> usize {
    let mut lefts: Vec<i32> = rects.iter().map(|r| r.left().round() as i32).collect();
    lefts.sort_unstable();
    lefts.dedup();
    lefts.len()
}

/// Rows of tool buttons by their tops: how many buttons each row holds, top to bottom.
fn row_sizes(rects: &[egui::Rect]) -> Vec<usize> {
    let mut rows: Vec<(i32, usize)> = vec![];
    for r in rects {
        let top = r.top().round() as i32;
        match rows.last_mut() {
            Some((t, n)) if *t == top => *n += 1,
            _ => rows.push((top, 1)),
        }
    }
    rows.into_iter().map(|(_, n)| n).collect()
}

/// `window.toolbarColumns` toggles or sets the columns, refuses a non-boolean, the menu agrees, and the Tools rail lays
/// its buttons out in the columns chosen, Advanced too.
#[test]
fn the_columns_command_toggles_or_sets_the_toolbar_columns() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = reader();
    // 700 pt: too short for the single column, which scrolls instead of turning into two.
    let laid_out = |app: &mut VectorcraftApp, t0: f64| {
        let nodes = settled(app, &ctx, t0, 700.0);
        cells(app, &nodes)
    };
    assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(true));
    let two = laid_out(&mut app, 0.0);
    assert_eq!(columns(&two), 2);
    let rows = row_sizes(&two);
    assert_eq!(rows.iter().sum::<usize>(), two.len());
    assert!(rows[..rows.len() - 1].iter().all(|n| *n == 2), "every row but the last is full: {rows:?}");
    assert_eq!(app.run("window.toolbarColumns", json!({"double": true})).unwrap(), json!(true));
    assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(false));
    assert_eq!(crate::menus::checked(&app, "window.toolbarColumns", &json!({})), Some(false));
    assert_eq!(columns(&laid_out(&mut app, 1.0)), 1);
    assert!(app.run("window.toolbarColumns", json!({"double": "yes"})).is_err());
    // Advanced (every tool group) is taller still and keeps the column the user chose too.
    app.run("window.toolbarAdvanced", json!({})).unwrap();
    assert_eq!(columns(&laid_out(&mut app, 2.0)), 1);
}

/// The Tools group of `first`'s strip panel, with the rect of its floating window.
fn strip_rect(app: &VectorcraftApp, first: &str) -> Option<egui::Rect> {
    crate::panel_docking::layout_of(&app.ui).location(&strip_id(&Owner::Tools, first)).ok()?.floating
}

#[test]
fn window_float_tools_floats_and_puts_back_a_group() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let check = |app: &VectorcraftApp| crate::menus::checked(app, "window.floatTools", &json!({"tool": "star"}));
    assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(true));
    // The Basic toolbar's Shapes group, named by its first tool.
    let rect = strip_rect(&app, "rectangle").expect("the Shapes strip floats");
    assert!(is_floating(&app, None, "rectangle") && check(&app) == Some(true));
    // Already floating: stays where it is.
    assert_eq!(app.run("window.floatTools", json!({"tool": "rectangle", "floating": true})).unwrap(), json!(true));
    assert_eq!(strip_rect(&app, "rectangle"), Some(rect));
    // Omitted toggles it back.
    assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(false));
    assert!(!is_floating(&app, None, "rectangle") && check(&app) == Some(false));
    assert_eq!(app.run("window.floatTools", json!({"tool": "pen", "floating": false})).unwrap(), json!(false));
    // A tool alone in its slot, an unknown tool, a missing tool, a bad flag: errors that change nothing.
    assert!(app.run("window.floatTools", json!({"tool": "polygon"})).is_err());
    assert!(app.run("window.floatTools", json!({"tool": "nope"})).is_err());
    assert!(app.run("window.floatTools", json!({})).is_err());
    assert!(app.run("window.floatTools", json!({"tool": "pen", "floating": "yes"})).is_err());
    assert!(strip_rect(&app, "rectangle").is_none() && !is_floating(&app, None, "pen"));
}

/// A floated group is a panel of the workspace's layout: a saved workspace brings it back, the built-in ones have none.
#[test]
fn workspaces_keep_floating_flyouts_and_reset_puts_them_back() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("window.floatTools", json!({"tool": "pen"})).unwrap();
    app.run("window.workspace.new", json!({"name": "Strips"})).unwrap();
    app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
    assert!(!is_floating(&app, None, "pen"), "the built-in workspaces float none");
    app.run("window.workspace", json!({"name": "Strips"})).unwrap();
    assert!(is_floating(&app, None, "pen"));
}

/// General › Show Tool Cards: a tool's tool tip becomes a card with what it does and one line per action of its hint, the
/// modifier keys named for the platform; a tool has a card only with cards and tool tips on and a description.
#[test]
fn a_tool_card_shows_what_the_tool_does_and_what_its_keys_do() {
    let lines = card::lines("selection", false);
    let plain = |line: &Vec<(String, bool)>| line.iter().map(|(s, _)| s.as_str()).collect::<String>();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(plain(&lines[0]), "Click the object to select");
    let shift = crate::keymap::chord_text("Shift+Click");
    assert_eq!(lines[1], vec![(shift.clone(), true), (" to select multiple objects".to_string(), false)]);
    assert!(plain(&lines[2]).ends_with(" the object to duplicate"), "{lines:?}");
    // A segment that runs on past a key keeps its pieces on one line.
    let warp = card::lines("warp", false);
    assert_eq!(warp.len(), 2, "{warp:?}");
    assert!(plain(&warp[1]).ends_with(" keeps its proportions)"), "{warp:?}");
    // The Zoom tool's card follows the animated zoom's hint.
    assert_ne!(card::lines("zoom", false), card::lines("zoom", true));

    let mut app = app_with(300, 300);
    assert!(!card::Cards::new(&app).has(&"selection".to_string()), "off by default");
    app.run("prefs.set", json!({"key": "showToolCards", "value": true})).unwrap();
    assert!(card::Cards::new(&app).has(&"selection".to_string()));
    assert_eq!(vectorcraft_tools::catalog::tool_description("lasso"), None);
    assert!(!card::Cards::new(&app).has(&"lasso".to_string()), "no description: the tool tip");
    app.run("prefs.set", json!({"key": "showToolTips", "value": false})).unwrap();
    assert!(!card::Cards::new(&app).has(&"selection".to_string()), "tool tips off: no card");
}
