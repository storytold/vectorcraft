use egui::{Event, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use vectorcraft_engine::Session;

use super::*;

/// Tall enough for the whole Basic toolbar and the whole drawer.
const SCREEN: egui::Vec2 = vec2(1000.0, 4000.0);
/// The rail's Edit Toolbar button, by its access name.
const EDIT: &str = "Edit Toolbar"; // PLAN: assumed `RailWords::edit_toolbar` in English
/// The + button of a toolbar made with New Toolbar.
const ADD: &str = "Add Tool…";

/// What a frame drew, by access name: (name, bounds).
type Nodes = Vec<(String, Rect)>;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

fn slots(app: &VectorcraftApp) -> Vec<Vec<&'static str>> {
    crate::toolbar::slots(app).into_iter().map(|(_, s)| s).collect()
}

/// The slot holding `tool`.
fn slot_of(app: &VectorcraftApp, tool: &str) -> Option<usize> {
    slots(app).iter().position(|s| s.contains(&tool))
}

fn ctx() -> egui::Context {
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    ctx
}

/// One whole app frame at `time`; returns the named nodes it drew.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Nodes {
    let input = RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), ..Default::default() };
    let out = crate::testing::app_frame(app, &ctx, input);
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    nodes
        .into_iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((n.label()?.to_string(), Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect()
}

/// Two frames, so a drawer or menu opened by the first has its size and is drawn.
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> Nodes {
    frame(app, ctx, time, vec![]);
    frame(app, ctx, time + 0.05, vec![])
}

/// The bounds of the nodes called `name`, top first.
fn named(nodes: &Nodes, name: &str) -> Vec<Rect> {
    let mut at: Vec<Rect> = nodes.iter().filter(|(n, _)| n.trim() == name).map(|(_, r)| *r).collect();
    at.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    at
}

/// The first node called `name`.
fn one(nodes: &Nodes, name: &str) -> Rect {
    named(nodes, name).first().copied().unwrap_or_else(|| panic!("no `{name}` drawn"))
}

/// The first node called `name` that lies inside `within`.
fn inside(nodes: &Nodes, name: &str, within: Rect) -> Rect {
    named(nodes, name).into_iter().find(|r| within.contains(r.center())).unwrap_or_else(|| panic!("no `{name}` inside {within:?}"))
}

/// The open drawer: the Edit Toolbar node that holds its cells, larger than the ••• buttons of that name.
fn drawer(nodes: &Nodes) -> Rect {
    named(nodes, EDIT).into_iter().max_by(|a, b| a.area().total_cmp(&b.area())).expect("the drawer is open")
}

/// The toolbar button of `tool`: the rail names a tool by its label (its shortcut is in its tip).
fn button_name(tool: &str) -> String {
    tl!(tool_info(tool).unwrap().label).to_string()
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2) -> Nodes {
    frame(app, ctx, time, vec![Event::PointerMoved(at), button(at, true)]);
    frame(app, ctx, time + 0.05, vec![button(at, false)]);
    settle(app, ctx, time + 0.1)
}

/// A press at `from` dragged to `to` in steps, one frame each, then released there.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, from: Pos2, to: Pos2) -> Nodes {
    frame(app, ctx, time, vec![Event::PointerMoved(from), button(from, true)]);
    for k in 1..=6 {
        frame(app, ctx, time + 0.05 * f64::from(k), vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
    }
    frame(app, ctx, time + 0.4, vec![button(to, false)]);
    settle(app, ctx, time + 0.45)
}

/// The tools of the toolbar made with New Toolbar called `name`.
fn held(app: &VectorcraftApp, name: &str) -> Vec<Vec<String>> {
    crate::custom_toolbars::find(&app.ui, name).unwrap().slots.clone()
}

/// The drawer frames each flyout group whole, under the heading most of its tools have: every
/// tool once, the Pencil's group (led by the Shaper) under Draw.
#[test]
fn the_drawer_frames_every_flyout_group_once() {
    let listed: Vec<&str> = groups().iter().flat_map(|(_, gs)| gs.iter().flat_map(|g| g.iter().map(|t| t.id))).collect();
    let all: Vec<&str> = vectorcraft_tools::catalog::all_tools().map(|t| t.id).collect();
    assert_eq!(listed.len(), all.len());
    assert!(all.iter().all(|id| listed.contains(id)));
    let under = |id: &str| groups().iter().find(|(_, gs)| gs.iter().any(|g| g.iter().any(|t| t.id == id))).map(|(c, _)| *c);
    assert_eq!(under("pencil"), Some("Draw"));
    assert_eq!(under("shaper"), Some("Draw"), "framed with the Pencil");
    assert_eq!(under("rectangle"), Some("Shapes"));
    assert_eq!(under("objectsOnPath"), Some("Modify"), "framed with the Blend tool");
    assert_eq!(heading("objectsOnPath"), "Modify", "though not in the Basic toolbar");
}

/// The Show section's controls (#1736): each shows (true), hides (false) or toggles, saved with
/// the preferences; an unknown control is an error naming them.
#[test]
fn toolbar_show_turns_the_controls_under_the_tools_on_and_off() {
    let mut app = app();
    let all_on = json!({"fillStroke": true, "coloring": true, "drawMode": true, "screenMode": true});
    assert_eq!(app.run("toolbar.show", json!({})).unwrap(), all_on);
    assert_eq!(app.run("toolbar.show", json!({"control": "coloring"})).unwrap()["coloring"], json!(false));
    assert_eq!(app.run("toolbar.show", json!({"control": "screenMode", "on": false})).unwrap()["screenMode"], json!(false));
    assert!(!app.ui.toolbar_show.coloring && !app.ui.toolbar_show.screen_mode && app.ui.toolbar_show.fill_stroke);
    let err = app.run("toolbar.show", json!({"control": "swatches"})).unwrap_err();
    assert!(err.contains("swatches") && err.contains("fillStroke"), "{err}");
    // Saved with the preferences; older preferences show them all.
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(serde_json::from_value::<crate::state::UiState>(saved).unwrap().toolbar_show, app.ui.toolbar_show);
    let mut old = serde_json::to_value(&app.ui).unwrap();
    old.as_object_mut().unwrap().remove("toolbar_show");
    assert_eq!(serde_json::from_value::<crate::state::UiState>(old).unwrap().toolbar_show, crate::state::ToolbarShow::default());
}

/// Every tool is listed once, under a heading; the Basic toolbar's groups come first, in its order.
#[test]
fn every_tool_is_listed_once_under_a_heading() {
    let s = sections();
    let listed: Vec<&str> = s.iter().flat_map(|(_, tools)| tools.iter().map(|t| t.id)).collect();
    let all: Vec<&str> = vectorcraft_tools::catalog::all_tools().map(|t| t.id).collect();
    assert_eq!(listed.len(), all.len());
    assert!(all.iter().all(|id| listed.contains(id)));
    let heads: Vec<&str> = s.iter().map(|(c, _)| *c).collect();
    assert_eq!(&heads[..3], ["Select", "Shapes", "Draw"]);
    assert!(heads.contains(&"Symbols") && heads.contains(&"Graph"), "{heads:?}");
    assert!(!heads.contains(&"More"), "every tool has a named heading: {heads:?}");
}

/// A toolbar edited and saved before the Enclosing Lasso Tool joined the Lasso group keeps its
/// saved layout: the tool stays in the drawer until Reset Toolbar or a drag adds it.
#[test]
fn a_saved_toolbar_without_enclosing_lasso_keeps_its_layout() {
    let mut app = app();
    let id = vectorcraft_tools::addons::enclosing_lasso::ID;
    let saved = ToolbarCategory { name: Some("Select".into()), slots: vec![["lasso", "magicWand"].iter().map(|t| t.to_string()).collect()] };
    app.ui.toolbar_edits = ToolbarEdits { basic: Some(vec![saved]), advanced: None };
    app.ui = std::mem::take(&mut app.ui).sanitized();
    assert_eq!(slots(&app), [["lasso", "magicWand"]]);
    assert!(slot_of(&app, id).is_none());
    app.run("toolbar.reset", json!({})).unwrap();
    let slot = slot_of(&app, "lasso").map(|i| slots(&app)[i].clone());
    assert!(slot.is_some_and(|s| s.contains(&id)), "Reset Toolbar adds it");
}

#[test]
fn the_commands_add_move_remove_and_reset_tools() {
    let mut app = app();
    let shipped = slots(&app);
    // At the end; the toolbar's slots come back.
    let r = app.run("toolbar.addTool", json!({"tool": "artboard"})).unwrap();
    assert_eq!(r, json!(slots(&app)));
    assert_eq!(slots(&app).last().unwrap(), &["artboard"]);
    // Before the first slot, moved out of its group.
    app.run("toolbar.addTool", json!({"tool": "magicWand", "slot": 0})).unwrap();
    assert_eq!(&slots(&app)[..3], [&["magicWand"][..], &["selection"], &["directSelection", "groupSelection"]]);
    assert_eq!(slots(&app)[3], ["lasso", "enclosingLasso"]);
    // After a slot, and into a group (last), moved out of the Rectangle group.
    app.run("toolbar.addTool", json!({"tool": "slice", "slot": 1, "place": "after"})).unwrap();
    assert_eq!(slots(&app)[2], ["slice"]);
    let ellipse = slot_of(&app, "ellipse").unwrap();
    app.run("toolbar.addTool", json!({"tool": "star", "slot": ellipse, "place": "group"})).unwrap();
    assert_eq!(slots(&app)[ellipse], ["ellipse", "circle", "curvatureCircle", "star"]);
    assert!(!slots(&app)[slot_of(&app, "rectangle").unwrap()].contains(&"star"));
    // Onto its own group: nothing changes.
    let before = slots(&app);
    app.run("toolbar.addTool", json!({"tool": "ellipse", "slot": ellipse, "place": "group"})).unwrap();
    assert_eq!(slots(&app), before);
    // The category label still names the first slot of its category.
    assert_eq!(crate::toolbar::slots(&app)[0].0, Some("Select"));
    // Out: one tool, or its whole group; an emptied slot goes.
    app.run("toolbar.removeTool", json!({"tool": "ellipse"})).unwrap();
    assert_eq!(slots(&app)[ellipse], ["circle", "curvatureCircle", "star"]);
    app.run("toolbar.removeTool", json!({"tool": "addAnchor", "group": true})).unwrap();
    assert!(slot_of(&app, "pen").is_none() && slot_of(&app, "addAnchor").is_none());
    // Bad params change nothing.
    let before = slots(&app);
    for p in [
        json!({}),
        json!({"tool": "nope"}),
        json!({"tool": "pen", "slot": 999}),
        json!({"tool": "pen", "slot": -1}),
        json!({"tool": "pen", "place": "inside"}),
    ] {
        assert!(app.run("toolbar.addTool", p.clone()).is_err(), "{p}");
    }
    assert_eq!(app.run("toolbar.removeTool", json!({"tool": "pen"})).unwrap(), json!(before), "not in the toolbar: stays out");
    assert!(app.run("toolbar.addTool", json!({"tool": "pen", "slot": "first"})).unwrap_err().contains("between 0 and"));
    assert!(app.run("toolbar.removeTool", json!({"tool": "star", "group": "yes"})).is_err());
    assert_eq!(slots(&app), before);
    // The Advanced toolbar keeps its own edits.
    app.run("window.toolbarAdvanced", json!({})).unwrap();
    assert_eq!(slots(&app).len(), TOOL_GROUPS.len());
    app.run("toolbar.removeTool", json!({"tool": "selection"})).unwrap();
    assert!(app.ui.toolbar_edits.advanced.is_some() && slot_of(&app, "selection").is_none());
    app.run("window.toolbarAdvanced", json!({})).unwrap();
    assert_eq!(slots(&app)[0], ["magicWand"]);
    // Reset puts the current one back as it ships.
    assert_eq!(app.run("toolbar.reset", json!({})).unwrap(), json!(shipped));
    assert!(app.ui.toolbar_edits.basic.is_none() && app.ui.toolbar_edits.advanced.is_some());
    // An edit that gives back the shipped toolbar forgets the edits.
    app.run("toolbar.addTool", json!({"tool": "artboard"})).unwrap();
    app.run("toolbar.removeTool", json!({"tool": "artboard"})).unwrap();
    assert!(app.ui.toolbar_edits.basic.is_none());
    // Everything out, then a tool back: it starts its own category.
    for tool in shipped.iter().flatten() {
        app.run("toolbar.removeTool", json!({"tool": tool})).unwrap();
    }
    assert!(slots(&app).is_empty());
    app.run("toolbar.addTool", json!({"tool": "pen"})).unwrap();
    assert_eq!(crate::toolbar::slots(&app), [(Some("Draw"), vec!["pen"])]);
}

#[test]
fn the_edited_toolbar_is_saved_and_loads_sanitized() {
    let mut app = app();
    app.run("toolbar.addTool", json!({"tool": "artboard", "slot": 0})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.toolbar_edits, app.ui.toolbar_edits);
    // Preferences saved before Edit Toolbar load the shipped toolbars.
    let mut old = saved;
    old.as_object_mut().unwrap().remove("toolbar_edits");
    assert_eq!(serde_json::from_value::<crate::state::UiState>(old).unwrap().toolbar_edits, ToolbarEdits::default());
    // A hand-edited file: unknown tools and names go, each tool once, emptied slots go.
    let cat = |name: Option<&str>, slots: &[&[&str]]| ToolbarCategory {
        name: name.map(str::to_string),
        slots: slots.iter().map(|s| s.iter().map(|id| id.to_string()).collect()).collect(),
    };
    let ui = crate::state::UiState {
        toolbar_edits: ToolbarEdits {
            basic: Some(vec![cat(Some("Nope"), &[&["pen", "nope"], &["pen"], &["nope"]]), cat(Some("Draw"), &[&[]])]),
            advanced: None,
        },
        tools_drawer: true,
        ..Default::default()
    }
    .sanitized();
    assert_eq!(ui.toolbar_edits.basic, Some(vec![cat(None, &[&["pen"]])]));
    assert!(!ui.tools_drawer, "the drawer starts closed");
}

/// A toolbar made with New Toolbar floats blank; its ••• button opens the drawer beside it, and
/// tools dragged from the drawer fill it, leaving the Tools panel's toolbar as it was. A drop on the
/// blank toolbar adds a slot; on the lower third of a tool, a slot of its own after it; on the middle
/// of a tool, into its group.
#[test]
fn a_new_toolbar_fills_from_the_drawer_beside_it() {
    let mut app = app();
    let ctx = ctx();
    let shipped = slots(&app);
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    let nodes = settle(&mut app, &ctx, 0.0);
    let edit = inside(&nodes, EDIT, one(&nodes, "Mine"));
    click(&mut app, &ctx, 1.0, edit.center());
    assert!(app.ui.tools_drawer && crate::custom_toolbars::drawer_target(&app.ui) == Some("Mine"));
    assert_eq!(crate::menus::checked(&app, "window.editToolbar", &json!({"toolbar": "Mine"})), Some(true));
    assert_eq!(crate::menus::checked(&app, "window.editToolbar", &json!({})), Some(false));
    // Dragged onto the blank toolbar, a tool is added to it.
    let nodes = settle(&mut app, &ctx, 2.0);
    let bar = one(&nodes, "Mine");
    drag(&mut app, &ctx, 3.0, inside(&nodes, "Ellipse Tool", drawer(&nodes)).center(), bar.center());
    assert_eq!(held(&app, "Mine"), [["ellipse"]]);
    assert_eq!(slots(&app), shipped, "the Tools panel's toolbar is untouched");
    // Onto the lower third of its tool: a slot of its own after it.
    let nodes = settle(&mut app, &ctx, 4.0);
    let tool = inside(&nodes, &button_name("ellipse"), one(&nodes, "Mine"));
    drag(&mut app, &ctx, 5.0, inside(&nodes, "Artboard Tool", drawer(&nodes)).center(), pos2(tool.center().x, tool.top() + 0.85 * tool.height()));
    assert_eq!(held(&app, "Mine"), [["ellipse"], ["artboard"]]);
    // Onto the middle of the first tool: into its group.
    let nodes = settle(&mut app, &ctx, 6.0);
    let tool = inside(&nodes, &button_name("ellipse"), one(&nodes, "Mine"));
    drag(&mut app, &ctx, 7.0, inside(&nodes, "Slice Tool", drawer(&nodes)).center(), tool.center());
    assert_eq!(held(&app, "Mine"), [vec!["ellipse", "slice"], vec!["artboard"]]);
    // A click on a tool picks it; the drawer stays open.
    let nodes = settle(&mut app, &ctx, 8.0);
    let artboard = inside(&nodes, &button_name("artboard"), one(&nodes, "Mine"));
    let nodes = click(&mut app, &ctx, 9.0, artboard.center());
    assert_eq!(app.session.tool_id(), "artboard");
    assert!(app.ui.tools_drawer);
    // Dragged from the toolbar into the drawer, a tool leaves it.
    let artboard = inside(&nodes, &button_name("artboard"), one(&nodes, "Mine"));
    drag(&mut app, &ctx, 10.0, artboard.center(), inside(&nodes, "Ellipse Tool", drawer(&nodes)).center());
    assert_eq!(held(&app, "Mine"), [["ellipse", "slice"]]);
    assert_eq!(slots(&app), shipped, "the Tools panel's toolbar is untouched");
    // The Tools panel's ••• button takes the drawer back beside its toolbar.
    let nodes = settle(&mut app, &ctx, 11.0);
    let bar = one(&nodes, "Mine");
    let main_edit = named(&nodes, EDIT).into_iter().find(|r| !bar.contains(r.center())).expect("the Tools panel's ••• button");
    click(&mut app, &ctx, 12.0, main_edit.center());
    assert!(app.ui.tools_drawer && crate::custom_toolbars::drawer_target(&app.ui).is_none());
}

/// The + menu of a toolbar made with New Toolbar: a tool not held is enabled and picking it adds it at
/// the end of this toolbar alone, without the drawer; a held tool is disabled under its heading and
/// listed under Remove Tool, where picking it takes it out again.
#[test]
fn the_add_tool_menu_fills_a_new_toolbar_without_a_drag() {
    let mut app = app();
    let ctx = ctx();
    let shipped = slots(&app);
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    // The menu's items called `name`, top first: inside the menu, the Add Tool… node larger than the + button.
    let items = |nodes: &Nodes, name: &str| -> Vec<Rect> {
        let menu = named(nodes, ADD).into_iter().max_by(|a, b| a.area().total_cmp(&b.area())).expect("the menu is open");
        named(nodes, name).into_iter().filter(|r| menu.contains(r.center())).collect()
    };
    // The menu at `time`, opened with a click on the + button unless it is open.
    let open = |app: &mut VectorcraftApp, time: f64| -> Nodes {
        let nodes = settle(app, &ctx, time);
        if !named(&nodes, "Select:").is_empty() {
            return nodes;
        }
        let plus = inside(&nodes, ADD, one(&nodes, "Mine"));
        click(app, &ctx, time + 0.2, plus.center());
        settle(app, &ctx, time + 0.5)
    };
    let nodes = open(&mut app, 0.0);
    assert!(named(&nodes, "Remove Tool:").is_empty(), "nothing to take out of a blank toolbar");
    let lasso = items(&nodes, "Lasso Tool");
    assert_eq!(lasso.len(), 1, "listed once, under Select");
    click(&mut app, &ctx, 1.0, lasso[0].center());
    assert_eq!(held(&app, "Mine"), [["lasso"]]);
    assert_eq!(slots(&app), shipped, "the Tools panel's toolbar is untouched");
    assert!(!app.ui.tools_drawer, "no drawer opened");
    // A second tool goes after the first.
    let nodes = open(&mut app, 10.0);
    click(&mut app, &ctx, 11.0, items(&nodes, "Magic Wand Tool")[0].center());
    assert_eq!(held(&app, "Mine"), [["lasso"], ["magicWand"]]);
    // A held tool is listed under Remove Tool, then dimmed under its heading, where a click adds nothing.
    let nodes = open(&mut app, 20.0);
    assert_eq!(named(&nodes, "Remove Tool:").len(), 1);
    let lasso = items(&nodes, "Lasso Tool");
    assert_eq!(lasso.len(), 2, "under Remove Tool, then under Select");
    click(&mut app, &ctx, 21.0, lasso[1].center());
    assert_eq!(held(&app, "Mine"), [["lasso"], ["magicWand"]]);
    // Under Remove Tool, a click takes it out.
    let nodes = open(&mut app, 30.0);
    click(&mut app, &ctx, 31.0, items(&nodes, "Lasso Tool")[0].center());
    assert_eq!(held(&app, "Mine"), [["magicWand"]]);
    assert_eq!(slots(&app), shipped);
}

/// A click on another toolbar's ••• button takes the drawer there, whichever was made first.
#[test]
fn the_drawer_moves_between_new_toolbars() {
    let mut app = app();
    let ctx = ctx();
    for name in ["A", "B"] {
        app.run("window.toolbar.new", json!({"name": name})).unwrap();
    }
    // B away from A, which the cascade puts it over.
    let b = crate::custom_toolbars::panel_id(crate::custom_toolbars::find(&app.ui, "B").unwrap().id);
    crate::panel_docking::place_floating(&mut app, &b, pos2(600.0, 900.0), 0.0).unwrap();
    for (k, name) in ["B", "A", "B"].into_iter().enumerate() {
        let nodes = settle(&mut app, &ctx, 1.0 + 2.0 * k as f64);
        let edit = inside(&nodes, EDIT, one(&nodes, name));
        click(&mut app, &ctx, 2.0 + 2.0 * k as f64, edit.center());
        assert_eq!(crate::custom_toolbars::drawer_target(&app.ui), Some(name), "after the click on {name}'s ••• button");
    }
}

/// Basic or Advanced in the drawer's menu, picked while the drawer edits a toolbar made with New
/// Toolbar, switches the Tools panel and takes the drawer to it, so Reset and drops edit it.
#[test]
fn basic_or_advanced_takes_the_drawer_to_the_tools_panel() {
    let mut app = app();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
    // With the Tools panel hidden too: it shows.
    app.run("window.toolbar", json!({})).unwrap();
    for advanced in [false, true] {
        app.run("window.editToolbar", json!({"toolbar": "Mine", "open": true})).unwrap();
        pick_layout(&mut app, advanced).unwrap();
        assert!(app.ui.tools_drawer && crate::custom_toolbars::drawer_target(&app.ui).is_none(), "{advanced}");
        assert_eq!(app.ui.toolbar_advanced, advanced);
        assert!(crate::toolbar::shown(&app.ui), "the Tools panel shows beside the drawer");
    }
    assert_eq!(crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots, [["pen"]], "Mine as it was");
}

/// A toolbar edited and saved before the Trim at Crossings Tool joined the Scissors group keeps
/// its saved layout: the tool stays in the drawer until Reset Toolbar or a drag adds it.
#[test]
fn a_saved_toolbar_without_trim_at_crossings_keeps_its_layout() {
    let mut app = app();
    let trim = vectorcraft_tools::addons::trim::ID;
    let saved = ToolbarCategory { name: Some("Draw".into()), slots: vec![["eraser", "scissors", "knife"].iter().map(|id| id.to_string()).collect()] };
    app.ui.toolbar_edits = ToolbarEdits { basic: Some(vec![saved]), advanced: None };
    app.ui = std::mem::take(&mut app.ui).sanitized();
    assert_eq!(slots(&app), [["eraser", "scissors", "knife"]]);
    assert!(slot_of(&app, trim).is_none());
    app.run("toolbar.reset", json!({})).unwrap();
    let slot = slot_of(&app, "scissors").map(|i| slots(&app)[i].clone());
    assert!(slot.is_some_and(|s| s.contains(&trim)), "Reset Toolbar adds it");
}

/// The add-ons' tools are in neither toolbar as they ship (the Trim at Crossings Tool, in the
/// Scissors tool's group, and the Enclosing Lasso Tool, in the Lasso tool's group, aside): the
/// drawer lists them under their add-on's heading, and they can be added from there.
#[test]
fn addon_tools_are_in_the_drawer_not_the_toolbars() {
    let mut app = app();
    let contour = vectorcraft_tools::addons::contour::ID;
    let box3d = vectorcraft_tools::addons::box3d::ID;
    let polygonal = vectorcraft_tools::addons::polygonal_lasso::ID;
    let addon: Vec<&str> = vectorcraft_tools::addons::GROUPS.iter().flat_map(|g| g.iter().map(|t| t.id)).collect();
    assert_eq!(
        addon,
        [
            "anchorEraserBrush",
            "extendPath",
            "measureInspect",
            "parametricShape",
            "cornerTool",
            "slideAnchor",
            "sharedEdge",
            "tangentCircle",
            "smartPen",
            contour,
            box3d,
            polygonal
        ]
    );
    for advanced in [false, true] {
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        assert!(addon.iter().all(|id| slot_of(&app, id).is_none()), "advanced: {advanced}");
    }
    let listed = |id: &str| {
        sections().iter().find(|(_, tools)| tools.iter().any(|t| t.id == id)).map(|(h, tools)| (*h, tools.iter().map(|t| t.id).collect::<Vec<_>>()))
    };
    assert_eq!(listed("extendPath"), Some(("Path Precision Tools", addon[..8].to_vec())));
    assert_eq!(listed("smartPen"), Some(("Smart Pen", vec!["smartPen"])));
    assert_eq!(listed(contour), Some(("Contour", vec![contour])));
    assert_eq!(listed(box3d), Some(("3D Box", vec![box3d])));
    assert_eq!(listed(polygonal), Some(("Polygonal Lasso", vec![polygonal])));
    for tool in ["extendPath", "smartPen", contour, box3d, polygonal] {
        app.run("toolbar.addTool", json!({"tool": tool})).unwrap();
        assert!(slot_of(&app, tool).is_some(), "{tool}");
    }
}
