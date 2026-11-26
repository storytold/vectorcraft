use egui::accesskit::{Node, Role, Toggled};
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

/// Shift+Tab hides the panels and leaves the Tools panel, and a second Shift+Tab shows them again.
#[test]
fn shift_tab_hides_the_panels_and_keeps_the_tools() {
    let mut app = app_with(300, 200);
    let ctx = reader();
    let properties = |nodes: &[Node]| nodes.iter().any(|n| n.role() == Role::Tab && n.label().is_some_and(|l| l.eq_ignore_ascii_case("properties")));
    let shift = egui::Modifiers::SHIFT;
    let key = |pressed| Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: shift };
    let nodes = settled(&mut app, &ctx, 0.0, 800.0);
    assert!(properties(&nodes), "the Properties panel shows at first");
    for (k, hidden) in [true, false].into_iter().enumerate() {
        let t = 1.0 + k as f64;
        frame(&mut app, &ctx, t, vec![Event::ModifiersChanged(shift), key(true)], 800.0);
        frame(&mut app, &ctx, t + 0.05, vec![key(false), Event::ModifiersChanged(egui::Modifiers::NONE)], 800.0);
        let nodes = settled(&mut app, &ctx, t + 0.1, 800.0);
        assert_eq!(properties(&nodes), !hidden, "Shift+Tab {}", if hidden { "hides the panels" } else { "shows them again" });
        assert_eq!(crate::panel_docking::panels_shown(&app), !hidden);
        assert!(!crate::panel_docking::tab_hidden(&app), "Shift+Tab is not Tab's hide");
        tool_rect(&nodes, "selection");
    }
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
    assert_eq!(app.view().unwrap().zoom, crate::state::zoom_baseline(app.print_ppi()));
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

/// A screen reader reads the drawing-mode buttons' state as the rail shows it, in every mode: in two columns the
/// current mode's button reads pressed and the other two not pressed; in one column the single button is named for the
/// current mode and cycles the modes, so it reads as a plain button with no pressed state.
#[test]
fn the_drawing_mode_buttons_report_the_current_mode() {
    let mut app = app_with(200, 200);
    let ctx = reader();
    // Draw Inside draws into one selected path.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let modes = [("normal", "Draw Normal (Shift+D)"), ("behind", "Draw Behind (Shift+D)"), ("inside", "Draw Inside (Shift+D)")];
    let mut t0 = 0.0;
    for double in [false, true] {
        app.run("window.toolbarColumns", json!({ "double": double })).unwrap();
        for (current, _) in modes {
            app.run("view.drawMode", json!({ "mode": current })).unwrap();
            let nodes = settled(&mut app, &ctx, t0, 900.0);
            t0 += 10.0;
            for (mode, label) in modes {
                let node = nodes.iter().find(|n| n.label() == Some(label));
                if double {
                    let node = node.unwrap_or_else(|| panic!("two columns, {current:?}: {label} is drawn"));
                    let pressed = if mode == current { Toggled::True } else { Toggled::False };
                    assert_eq!(node.toggled(), Some(pressed), "two columns, {current:?}: {label}");
                } else if mode == current {
                    let node = node.unwrap_or_else(|| panic!("one column, {current:?}: {label} is drawn"));
                    assert_eq!(node.role(), Role::Button, "one column, {current:?}: {label}");
                    assert_eq!(node.toggled(), None, "one column, {current:?}: {label} has no pressed state");
                } else {
                    assert!(node.is_none(), "one column, {current:?}: {label} is not shown");
                }
            }
        }
    }
}

/// A Learn link's tool in a collapsed group of the Tools panel opens that group's flyout with the tool in it, and
/// chooses nothing: the rail reveals the located tool.
#[test]
fn a_located_tool_in_a_collapsed_group_opens_its_flyout_on_the_tools_panel() {
    let mut app = app_with(400, 400);
    let ctx = reader();
    let menus = |nodes: &[Node]| nodes.iter().filter(|n| n.role() == Role::Menu).count();
    let row = tool_info("smooth").map(button_name).unwrap_or_default();
    let nodes = settled(&mut app, &ctx, 0.0, 900.0);
    assert_eq!(menus(&nodes), 0, "no flyout before the link");
    assert_eq!(app.run("learn.locateTool", json!({"tool": "smooth"})).unwrap()["location"], "flyout");
    let nodes = settled(&mut app, &ctx, 1.0, 900.0);
    assert!(menus(&nodes) > 0, "the located tool's group opens its flyout");
    assert!(nodes.iter().any(|n| n.label() == Some(row.as_str())), "the flyout lists {row}");
    assert_eq!(app.session.tool_id(), "selection", "the link chooses no tool");
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

/// The real Tools dock column; distinguishes its footer Search from the app bar's Search.
fn review_8758_panel(app: &VectorcraftApp, ctx: &egui::Context) -> egui::Rect {
    let report = crate::panel_docking::inspect(app, ctx);
    let columns = report["columns"].as_array().expect("dock columns");
    let r = &columns.iter().find(|column| column["panel"] == "tools").expect("the Tools column")["rect"];
    egui::Rect::from_min_size(
        egui::pos2(r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32),
        egui::vec2(r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32),
    )
}

fn review_8758_button<'a>(app: &VectorcraftApp, ctx: &egui::Context, nodes: &'a [Node], label: &str) -> &'a Node {
    let panel = review_8758_panel(app, ctx);
    nodes
        .iter()
        .find(|n| n.role() == Role::Button && n.label() == Some(label) && bounds(n).is_some_and(|r| panel.contains_rect(r)))
        .unwrap_or_else(|| panic!("Tools footer button {label:?} is drawn"))
}

fn review_8758_query(nodes: &[Node]) -> &str {
    nodes
        .iter()
        .find(|n| n.role() == Role::TextInput && n.label() == Some("Search commands"))
        .and_then(Node::value)
        .expect("the native palette's query field")
}

fn review_8758_unchanged_content(app: &VectorcraftApp) -> Value {
    let st = app.session.active().unwrap();
    json!({"document": &*st.doc, "selection": format!("{:?}", st.selection),
        "undo": st.history.undo.len(), "redo": st.history.redo.len(), "slots": slots(app)})
}

/// Paint the native tooltip over whole app frames, keeping the same window and dock measurements.
fn review_8758_hovered_texts(app: &mut VectorcraftApp, ctx: &egui::Context, t0: f64, at: Pos2) -> Vec<String> {
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
            _ => {}
        }
    }
    let mut shown = vec![];
    for i in 0..20 {
        let events = match i {
            0 => vec![Event::PointerGone],
            1 => vec![Event::PointerMoved(at)],
            _ => vec![],
        };
        let out = crate::testing::app_frame(
            app,
            ctx,
            egui::RawInput {
                time: Some(t0 + f64::from(i) * 0.1),
                events,
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 1400.0))),
                ..Default::default()
            },
        );
        out.shapes.iter().for_each(|shape| texts(&shape.shape, &mut shown));
    }
    shown
}

fn review_8758_edit_control(app: &mut VectorcraftApp, ctx: &egui::Context, nodes: &[Node]) {
    let edit = bounds(review_8758_button(app, ctx, nodes, "Edit Toolbar")).unwrap();
    let panel = review_8758_panel(app, ctx);
    let before = slots(app);
    let nodes = click(app, ctx, 1.0, edit.center(), 1400.0);
    assert!(app.ui.tools_drawer && app.ui.drawer_toolbar.is_none(), "Edit opens the Tools customization drawer");
    let drawer = nodes
        .iter()
        .find(|n| n.role() == Role::Dialog && n.label() == Some("Edit Toolbar"))
        .and_then(bounds)
        .expect("the native customization drawer is drawn");
    // Main anchors its native drawer to the rail's canvas side; the footer button remains its focus opener.
    assert!(
        drawer.left() >= panel.right() - 1.0 && drawer.top() < panel.bottom() && drawer.bottom() > panel.top(),
        "the customization drawer remains beside its Tools rail: {drawer:?}, {panel:?}"
    );
    assert!(nodes.iter().any(|n| n.label() == Some("All Tools")), "the customization list is present");
    assert_eq!(slots(app), before, "opening customization does not change tool membership");
    assert!(before.iter().any(|(_, tools)| tools.contains(&"rectangle")));
    app.run("toolbar.removeTool", json!({"tool": "rectangle"})).unwrap();
    assert!(!slots(app).iter().any(|(_, tools)| tools.contains(&"rectangle")), "customization still edits the toolbar");
    // Use the native close control, which returns focus to this footer's Edit button.
    let close = named_rect(&nodes, "Close");
    click(app, ctx, 2.0, close.center(), 1400.0);
    assert!(!app.ui.tools_drawer);
    let key = |pressed| Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(app, ctx, 2.2, vec![key(true), key(false)], 1400.0);
    settled(app, ctx, 2.3, 1400.0);
    assert!(app.ui.tools_drawer, "focus returns to Edit: Enter reopens its customization drawer");
    app.run("window.editToolbar", json!({})).unwrap();
    settled(app, ctx, 2.6, 1400.0);
}

fn review_8758_layout(double: bool) {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = app_with(300, 300);
    app.run("window.toolbarColumns", json!({"double": double})).unwrap();
    let ctx = reader();
    let nodes = settled(&mut app, &ctx, 0.0, 1400.0);
    review_8758_edit_control(&mut app, &ctx, &nodes);
    let nodes = settled(&mut app, &ctx, 3.0, 1400.0);
    let search = bounds(review_8758_button(&app, &ctx, &nodes, "Search commands")).unwrap();
    let edit = bounds(review_8758_button(&app, &ctx, &nodes, "Edit Toolbar")).unwrap();
    let panel = review_8758_panel(&app, &ctx);
    assert!(panel.contains_rect(search) && panel.contains_rect(edit), "both controls fit the actual Tools panel");
    assert!(search.bottom() <= 1400.0 && edit.bottom() <= 1400.0, "both controls fit the window");
    if double {
        assert!(search.right() <= edit.left() && (search.center().y - edit.center().y).abs() < 0.01, "Search left of Edit on one row");
    } else {
        assert!(
            search.bottom() <= edit.top() && (search.center().x - edit.center().x).abs() < 0.01,
            "one column keeps both controls in a fitted stack"
        );
    }
    assert!(review_8758_hovered_texts(&mut app, &ctx, 4.0, search.center()).iter().any(|t| t == "Search commands, tools, panels and effects"));
    let before = review_8758_unchanged_content(&app);
    let tool = app.session.tool_id().to_string();
    click(&mut app, &ctx, 8.0, search.center(), 1400.0);
    let nodes = frame(&mut app, &ctx, 9.0, vec![Event::Text("old query".into())], 1400.0);
    assert_eq!(review_8758_query(&nodes), "old query", "the opened native search has keyboard focus");
    app.run("help.commandPalette", json!({})).unwrap();
    settled(&mut app, &ctx, 10.0, 1400.0);
    let nodes = click(&mut app, &ctx, 11.0, search.center(), 1400.0);
    assert!(app.ui.palette_open, "the actual footer button opens the existing command palette");
    assert!(review_8758_query(&nodes).is_empty(), "the native palette opens with a fresh query");
    let nodes = frame(&mut app, &ctx, 12.0, vec![Event::Text("focused".into())], 1400.0);
    assert_eq!(review_8758_query(&nodes), "focused", "reopening gives the search field keyboard focus");
    assert!(!app.ui.tools_drawer, "Search is distinct from Edit Toolbar");
    assert_eq!(review_8758_unchanged_content(&app), before, "Search changes no document, history, selection or toolbar membership");
    assert_eq!(app.session.tool_id(), tool);
}

#[test]
fn review_8758_toolbar_search_fits_beside_edit_and_opens_the_existing_palette() {
    for double in [true, false] {
        review_8758_layout(double);
    }
}

#[test]
fn review_8758_toolbar_search_accepts_a_tool_query_and_is_disabled_over_a_dialog() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = app_with(300, 300);
    let ctx = reader();
    let nodes = settled(&mut app, &ctx, 0.0, 1400.0);
    let search = bounds(review_8758_button(&app, &ctx, &nodes, "Search commands")).unwrap();
    let before = review_8758_unchanged_content(&app);
    click(&mut app, &ctx, 0.2, search.center(), 1400.0);
    assert!(app.ui.palette_open);
    let nodes = frame(&mut app, &ctx, 0.4, vec![Event::Text("Scissors".into())], 1400.0);
    assert_eq!(review_8758_query(&nodes), "Scissors", "the actual focused query field receives typing");
    let nodes = settled(&mut app, &ctx, 0.5, 1400.0);
    let tool = nodes
        .iter()
        .find(|n| n.role() == Role::ListBoxOption && n.label().is_some_and(|label| label.starts_with("Scissors")))
        .expect("the native palette lists the matching Scissors tool");
    assert!(!tool.is_disabled());
    click(&mut app, &ctx, 0.7, bounds(tool).unwrap().center(), 1400.0);
    assert_eq!(app.session.tool_id(), "scissors", "activating the real result chooses the tool");
    assert!(!app.ui.palette_open, "a pick closes the palette");
    assert_eq!(app.ui.recent_commands.first().map(|command| command.key_text().into_owned()), Some("tool:scissors".to_string()));
    assert_eq!(review_8758_unchanged_content(&app), before, "choosing a tool changes no artwork/history or toolbar membership");

    // Current main owns query state inside CommandPalette and resets it on opening. A blocked footer press must not
    // open that component at all, invoke a command, steal focus or change the existing real dialog's fields.
    open_options(&mut app, "lineSegment").unwrap();
    let nodes = settled(&mut app, &ctx, 1.0, 1400.0);
    let dialog = tool_dialog(&app).expect("the real Tool Options dialog");
    let focus = craft::ui::runtime::focus::focused_rec(&ctx);
    let runs = app.run_count;
    let recent = app.ui.recent_commands.clone();
    let search = review_8758_button(&app, &ctx, &nodes, "Search commands");
    assert!(search.is_disabled(), "screen readers find Search disabled while a modal window is open");
    click(&mut app, &ctx, 1.2, bounds(search).unwrap().center(), 1400.0);
    assert!(!app.ui.palette_open, "the disabled footer cannot open Search over a dialog");
    assert_eq!(app.run_count, runs, "the disabled footer does not invoke the palette command");
    assert_eq!(craft::ui::runtime::focus::focused_rec(&ctx), focus, "the existing modal keeps focus");
    assert_eq!(app.ui.recent_commands, recent);
    assert_eq!(tool_dialog(&app), Some(dialog));
    assert_eq!(review_8758_unchanged_content(&app), before);
}
