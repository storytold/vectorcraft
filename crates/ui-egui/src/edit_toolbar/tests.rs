use egui::{Event, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use vectorcraft_engine::Session;

use super::*;

/// Tall enough for the whole Basic toolbar, the whole drawer and the whole Add Tool… menu.
const SCREEN: egui::Vec2 = vec2(1000.0, 5000.0);
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
    let out = crate::testing::app_frame(app, ctx, input);
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

/// The drawer's Show toggles are labeled: they sit in a field named "Show" whose label comes before them.
#[test]
fn the_drawer_labels_its_show_toggles() {
    let mut app = app();
    let ctx = ctx();
    settle(&mut app, &ctx, 0.0);
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    let nodes = settle(&mut app, &ctx, 1.0);
    let open = drawer(&nodes);
    let first = inside(&nodes, "Fill and Stroke Controls", open);
    let label = named(&nodes, "Show")
        .into_iter()
        .chain(named(&nodes, "Show:"))
        .find(|r| open.contains(r.center()))
        .unwrap_or_else(|| panic!("the drawer shows a Show label: {:?}", nodes.iter().filter(|(_, r)| open.contains(r.center())).collect::<Vec<_>>()));
    assert!(label.expand(0.5).contains_rect(first), "the Show field {label:?} holds the toggles {first:?}");
    assert!(label.left() < first.left(), "the Show label in {label:?} comes before the toggles {first:?}");
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

/// The Polygonal Lasso Tool is in the Lasso tool's flyout in both toolbars, after the Enclosing
/// Lasso, once in the All Tools drawer (under Select) and in Keyboard Shortcuts' list, with no
/// shortcut; its button's tip names it; Reset Toolbar keeps it there.
#[test]
fn polygonal_lasso_is_in_the_lasso_flyout() {
    let mut app = app();
    let id = vectorcraft_tools::addons::polygonal_lasso::ID;
    let enclosing = vectorcraft_tools::addons::enclosing_lasso::ID;
    for advanced in [false, true] {
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        let s = slots(&app);
        let slot = slot_of(&app, "lasso").map(|i| s[i].clone()).expect("a Lasso slot");
        let lasso = slot.iter().position(|t| *t == "lasso").expect("the Lasso");
        assert_eq!(slot.get(lasso + 1..lasso + 3), Some(&[enclosing, id][..]), "advanced: {advanced}");
        assert_eq!(s.iter().filter(|ids| ids.contains(&id)).count(), 1, "advanced: {advanced}");
    }
    assert_eq!(heading(id), "Select");
    assert_eq!(sections().iter().flat_map(|(_, tools)| tools.iter()).filter(|t| t.id == id).count(), 1, "listed once in the drawer");
    // Keyboard Shortcuts lists the window's commands once it has drawn a frame.
    let ctx = ctx();
    settle(&mut app, &ctx, 0.0);
    let rows = app.run("shortcuts.list", json!({"query": id})).unwrap();
    let rows: Vec<_> = rows.as_array().unwrap().iter().filter(|r| r["id"] == format!("tool:{id}")).collect();
    assert_eq!(rows.len(), 1, "listed once in Keyboard Shortcuts");
    assert_eq!((&rows[0]["group"], &rows[0]["shortcut"]), (&json!("Tools"), &Value::Null));
    let info = vectorcraft_tools::tool_info(id).expect("catalogued");
    assert_eq!((info.label, info.shortcut), ("Polygonal Lasso Tool", None));
    assert_eq!(crate::toolbar::tip(&ctx, info), "Polygonal Lasso Tool");
    app.run("tool.select", json!({"tool": id})).unwrap();
    assert_eq!(app.session.tool_id(), id);
    let shown = slot_of(&app, id).map(|i| crate::toolbar::shown_tool(&app, &slots(&app)[i]));
    assert_eq!(shown, Some(id), "the slot's button shows the tool once chosen");
    app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
    assert!(slot_of(&app, id).is_none(), "it can be taken out");
    app.run("toolbar.addTool", json!({"tool": id})).unwrap();
    assert!(slot_of(&app, id).is_some(), "the drawer adds it back");
    app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
    app.run("toolbar.reset", json!({})).unwrap();
    let after_reset = slot_of(&app, "lasso").map(|i| slots(&app)[i].contains(&id));
    assert_eq!(after_reset, Some(true), "Reset Toolbar puts it back");
}

/// A toolbar edited and saved before the Polygonal Lasso Tool joined the Lasso group keeps its
/// saved layout, in both toolbars: the tool stays in the drawer until Reset Toolbar or a drag adds
/// it, and a toolbar that already had it in a slot of its own keeps it there, once.
#[test]
fn a_saved_toolbar_without_polygonal_lasso_keeps_its_layout() {
    let id = vectorcraft_tools::addons::polygonal_lasso::ID;
    let enclosing = vectorcraft_tools::addons::enclosing_lasso::ID;
    let category = |slots: &[&[&str]]| ToolbarCategory {
        name: Some("Select".into()),
        slots: slots.iter().map(|s| s.iter().map(|t| t.to_string()).collect()).collect(),
    };
    for advanced in [false, true] {
        let edits = |saved: Vec<ToolbarCategory>| {
            if advanced { ToolbarEdits { basic: None, advanced: Some(saved) } } else { ToolbarEdits { basic: Some(saved), advanced: None } }
        };
        let mut app = app();
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        app.ui.toolbar_edits = edits(vec![category(&[&["lasso", enclosing]])]);
        app.ui = std::mem::take(&mut app.ui).sanitized();
        assert_eq!(slots(&app), [["lasso", enclosing]], "advanced: {advanced}");
        assert!(slot_of(&app, id).is_none(), "advanced: {advanced}");
        app.run("toolbar.reset", json!({})).unwrap();
        let slot = slot_of(&app, "lasso").map(|i| slots(&app)[i].clone());
        assert!(slot.is_some_and(|s| s.contains(&id)), "Reset Toolbar adds it; advanced: {advanced}");
        app.ui.toolbar_edits = edits(vec![category(&[&["lasso", enclosing], &[id]])]);
        app.ui = std::mem::take(&mut app.ui).sanitized();
        assert_eq!(slots(&app), [vec!["lasso", enclosing], vec![id]], "advanced: {advanced}");
    }
}

/// The Lasso group torn off the Tools panel shows the Polygonal Lasso Tool after the Enclosing
/// Lasso, in both toolbars, and shows it again after Reset Toolbar puts it back.
#[test]
fn a_torn_off_lasso_group_shows_polygonal_lasso() {
    let id = vectorcraft_tools::addons::polygonal_lasso::ID;
    let enclosing = vectorcraft_tools::addons::enclosing_lasso::ID;
    for advanced in [false, true] {
        let mut app = app();
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        app.run("window.floatTools", json!({"tool": "lasso", "floating": true})).unwrap();
        let lasso = |app: &VectorcraftApp| crate::toolbar::strips(app, None).into_iter().find(|g| g.first() == Some(&"lasso"));
        let after_lasso = |app: &VectorcraftApp| lasso(app).and_then(|g| g.get(1..3).map(|s| s.to_vec()));
        assert_eq!(after_lasso(&app), Some(vec![enclosing, id]), "advanced: {advanced}");
        app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
        assert!(lasso(&app).is_some_and(|g| !g.contains(&id)), "advanced: {advanced}");
        app.run("toolbar.reset", json!({})).unwrap();
        assert_eq!(after_lasso(&app), Some(vec![enclosing, id]), "advanced: {advanced}");
    }
}

/// The Smart Pen Tool is in the Pen tool's flyout in both toolbars, after the Pen family's own
/// tools, once in the All Tools drawer (under Draw) and in Keyboard Shortcuts' list, with no
/// shortcut; its button's tip names it; Reset Toolbar keeps it there.
#[test]
fn smart_pen_is_in_the_pen_flyout() {
    let mut app = app();
    let id = vectorcraft_tools::addons::smart_pen::ID;
    for advanced in [false, true] {
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        let s = slots(&app);
        let slot = slot_of(&app, "pen").map(|i| s[i].clone()).expect("a Pen slot");
        assert_eq!(slot.first(), Some(&"pen"), "advanced: {advanced}");
        assert_eq!(slot[slot.len().saturating_sub(2)..], ["quickPen", id], "advanced: {advanced}");
        assert_eq!(s.iter().filter(|ids| ids.contains(&id)).count(), 1, "advanced: {advanced}");
    }
    assert_eq!(heading(id), "Draw");
    assert_eq!(sections().iter().flat_map(|(_, tools)| tools.iter()).filter(|t| t.id == id).count(), 1, "listed once in the drawer");
    // Keyboard Shortcuts lists the window's commands once it has drawn a frame.
    let ctx = ctx();
    settle(&mut app, &ctx, 0.0);
    let rows = app.run("shortcuts.list", json!({"query": id})).unwrap();
    let rows: Vec<_> = rows.as_array().unwrap().iter().filter(|r| r["id"] == format!("tool:{id}")).collect();
    assert_eq!(rows.len(), 1, "listed once in Keyboard Shortcuts");
    assert_eq!((&rows[0]["group"], &rows[0]["shortcut"]), (&json!("Tools"), &Value::Null));
    let info = vectorcraft_tools::tool_info(id).expect("catalogued");
    assert_eq!((info.label, info.shortcut), ("Smart Pen Tool", None));
    assert_eq!(crate::toolbar::tip(&ctx, info), "Smart Pen Tool");
    app.run("tool.select", json!({"tool": id})).unwrap();
    assert_eq!(app.session.tool_id(), id);
    let shown = slot_of(&app, id).map(|i| crate::toolbar::shown_tool(&app, &slots(&app)[i]));
    assert_eq!(shown, Some(id), "the slot's button shows the tool once chosen");
    app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
    assert!(slot_of(&app, id).is_none(), "it can be taken out");
    app.run("toolbar.addTool", json!({"tool": id})).unwrap();
    assert!(slot_of(&app, id).is_some(), "the drawer adds it back");
    app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
    app.run("toolbar.reset", json!({})).unwrap();
    let after_reset = slot_of(&app, "pen").map(|i| slots(&app)[i].contains(&id));
    assert_eq!(after_reset, Some(true), "Reset Toolbar puts it back");
}

/// A toolbar edited and saved before the Smart Pen Tool joined the Pen group keeps its saved
/// layout, in both toolbars: the tool stays in the drawer until Reset Toolbar or a drag adds it,
/// and a toolbar that already had it in a slot of its own keeps it there, once.
#[test]
fn a_saved_toolbar_without_smart_pen_keeps_its_layout() {
    let id = vectorcraft_tools::addons::smart_pen::ID;
    let category = |slots: &[&[&str]]| ToolbarCategory {
        name: Some("Draw".into()),
        slots: slots.iter().map(|s| s.iter().map(|t| t.to_string()).collect()).collect(),
    };
    for advanced in [false, true] {
        let edits = |saved: Vec<ToolbarCategory>| {
            if advanced { ToolbarEdits { basic: None, advanced: Some(saved) } } else { ToolbarEdits { basic: Some(saved), advanced: None } }
        };
        let mut app = app();
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        app.ui.toolbar_edits = edits(vec![category(&[&["pen", "addAnchor"]])]);
        app.ui = std::mem::take(&mut app.ui).sanitized();
        assert_eq!(slots(&app), [["pen", "addAnchor"]], "advanced: {advanced}");
        assert!(slot_of(&app, id).is_none(), "advanced: {advanced}");
        app.run("toolbar.reset", json!({})).unwrap();
        let slot = slot_of(&app, "pen").map(|i| slots(&app)[i].clone());
        assert!(slot.is_some_and(|s| s.contains(&id)), "Reset Toolbar adds it; advanced: {advanced}");
        app.ui.toolbar_edits = edits(vec![category(&[&["pen", "addAnchor"], &[id]])]);
        app.ui = std::mem::take(&mut app.ui).sanitized();
        assert_eq!(slots(&app), [vec!["pen", "addAnchor"], vec![id]], "advanced: {advanced}");
    }
}

/// The Pen group torn off the Tools panel shows the Smart Pen Tool last, in both toolbars, and
/// shows it again after Reset Toolbar puts it back.
#[test]
fn a_torn_off_pen_group_shows_smart_pen() {
    let id = vectorcraft_tools::addons::smart_pen::ID;
    for advanced in [false, true] {
        let mut app = app();
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        app.run("window.floatTools", json!({"tool": "pen", "floating": true})).unwrap();
        let pen = |app: &VectorcraftApp| crate::toolbar::strips(app, None).into_iter().find(|g| g.first() == Some(&"pen"));
        assert_eq!(pen(&app).and_then(|g| g.last().copied()), Some(id), "advanced: {advanced}");
        app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
        assert!(pen(&app).is_some_and(|g| !g.contains(&id)), "advanced: {advanced}");
        app.run("toolbar.reset", json!({})).unwrap();
        assert_eq!(pen(&app).and_then(|g| g.last().copied()), Some(id), "advanced: {advanced}");
    }
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
    assert_eq!(slots(&app)[3], ["lasso", "enclosingLasso", "polygonalLasso"]);
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
    // The drawer and its title are named like the ••• buttons: the button is in neither Mine nor the drawer.
    let open = drawer(&nodes);
    let main_edit =
        named(&nodes, EDIT).into_iter().find(|r| !bar.contains(r.center()) && !open.contains(r.center())).expect("the Tools panel's ••• button");
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
        crate::menus::invoke(&mut app, "window.toolbarAdvanced", json!({ "advanced": advanced }));
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
/// Scissors tool's group, the Enclosing Lasso and Polygonal Lasso tools, in the Lasso tool's
/// group, and the Smart Pen Tool, in the Pen tool's group, aside): the drawer lists them under
/// their add-on's heading, and they can be added from there.
#[test]
fn addon_tools_are_in_the_drawer_not_the_toolbars() {
    let mut app = app();
    let contour = vectorcraft_tools::addons::contour::ID;
    let box3d = vectorcraft_tools::addons::box3d::ID;
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
            contour,
            box3d
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
    assert_eq!(listed(contour), Some(("Contour", vec![contour])));
    assert_eq!(listed(box3d), Some(("3D Box", vec![box3d])));
    for tool in ["extendPath", contour, box3d] {
        app.run("toolbar.addTool", json!({"tool": tool})).unwrap();
        assert!(slot_of(&app, tool).is_some(), "{tool}");
    }
}

/// The command buttons of the toolbar made with New Toolbar called Mine.
fn mine_commands(app: &VectorcraftApp) -> Vec<String> {
    crate::custom_toolbars::find(&app.ui, "Mine").unwrap().commands.clone()
}

/// `f` run with `app` and a `Ui` of one frame of `ctx` as the hosts run it ([`crate::testing::part_frame`]).
fn in_pass<T>(app: &mut VectorcraftApp, ctx: &egui::Context, f: impl FnOnce(&mut VectorcraftApp, &egui::Ui) -> T) -> T {
    let mut f = Some(f);
    let mut got = None;
    let _ = crate::testing::part_frame(app, ctx, RawInput::default(), |app, ui| {
        if let Some(f) = f.take() {
            got = Some(f(app, ui));
        }
    });
    got.expect("the frame ran")
}

/// `toolbar.addCommand` puts command buttons on a toolbar made with New Toolbar and moves them,
/// `toolbar.removeCommand` takes them off, and a bad call changes nothing. Manage Toolbars' copy
/// keeps them, and `toolbar.reset` takes them off with the tools.
#[test]
fn command_buttons_are_added_moved_and_removed_by_their_commands() {
    let mut app = app();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    let added = app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    assert_eq!(added, json!(["object.group"]));
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine", "at": 0})).unwrap();
    assert_eq!(mine_commands(&app), ["object.path.cleanUp", "object.group"], "moved, not added twice");
    for (p, why) in [
        (json!({"command": "no.such", "toolbar": "Mine"}), "an unknown command"),
        (json!({"command": "tool:pen", "toolbar": "Mine"}), "a tool"),
        (json!({"command": "object.group", "toolbar": "Mine", "at": 2}), "a place past the end"),
        (json!({"command": "object.group", "toolbar": "Mine", "at": -1}), "a negative place"),
        (json!({"command": "toolbar.addTool", "toolbar": "Mine"}), "a command that needs params"),
        (json!({"command": "object.group"}), "no toolbar"),
        (json!({"toolbar": "Mine"}), "no command"),
    ] {
        assert!(app.run("toolbar.addCommand", p.clone()).is_err(), "{why}: {p}");
        assert_eq!(mine_commands(&app), ["object.path.cleanUp", "object.group"], "{why} changes nothing");
    }
    assert!(app.run("toolbar.removeCommand", json!({"command": "object.group"})).is_err(), "no toolbar");
    let list = app.run("window.toolbar.list", json!({})).unwrap();
    assert_eq!(list[0]["commands"], json!(["object.path.cleanUp", "object.group"]));
    let copy = app.run("window.toolbar.duplicate", json!({"name": "Mine"})).unwrap();
    let copy = copy["toolbar"].as_str().unwrap().to_string();
    assert_eq!(crate::custom_toolbars::find(&app.ui, &copy).unwrap().commands, mine_commands(&app), "the copy has them");
    let left = app.run("toolbar.removeCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    assert_eq!(left, json!(["object.path.cleanUp"]));
    app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
    app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap();
    assert!(mine_commands(&app).is_empty(), "Reset makes the toolbar blank");
    assert_eq!(crate::custom_toolbars::find(&app.ui, &copy).unwrap().commands.len(), 2, "the copy keeps its own");
}

/// What a frame drew inside the toolbar called Mine, by access name: (name, bounds, dimmed).
fn on_mine(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> Vec<(String, Rect, bool)> {
    frame(app, ctx, time, vec![]);
    let input = RawInput { time: Some(time + 0.05), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), ..Default::default() };
    let out = crate::testing::app_frame(app, ctx, input);
    let nodes: Vec<(String, Rect, bool)> = out
        .platform_output
        .accesskit_update
        .map(|u| u.nodes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let rect = Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32));
            Some((n.label()?.to_string(), rect, n.is_disabled()))
        })
        .collect();
    let bar = nodes.iter().filter(|(n, ..)| n == "Mine").map(|(_, r, _)| *r).max_by(|a, b| a.area().total_cmp(&b.area()));
    let Some(bar) = bar else { return vec![] };
    nodes.into_iter().filter(|(n, r, _)| n != "Mine" && bar.contains(r.center())).collect()
}

/// The button inside the toolbar called Mine whose name is `name`: its bounds and whether it is dimmed.
fn mine_button(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, name: &str) -> Option<(Rect, bool)> {
    on_mine(app, ctx, time).into_iter().find(|(n, ..)| n.trim() == name).map(|(_, r, off)| (r, off))
}

/// A command button shows its command's label, without the ellipsis, and a click runs what the
/// command's menu item runs. While that item would be dimmed, the button is dimmed too and a click
/// runs nothing.
#[test]
fn a_command_button_runs_its_command_and_is_dimmed_while_it_cannot() {
    let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), Default::default());
    let ctx = ctx();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    for c in ["object.group", "object.path.cleanUp"] {
        app.run("toolbar.addCommand", json!({"command": c, "toolbar": "Mine"})).unwrap();
    }
    let undo = |app: &VectorcraftApp| app.session.active().map_or(0, |d| d.history.undo.len());
    app.run("select.set", json!({"ids": []})).unwrap();
    let (at, off) = mine_button(&mut app, &ctx, 0.0, "Group").expect("the button shows Group");
    assert!(off, "dimmed with nothing selected");
    let before = undo(&app);
    click(&mut app, &ctx, 1.0, at.center());
    assert_eq!(undo(&app), before, "a dimmed button runs nothing");
    app.run("select.all", json!({})).unwrap();
    let (at, off) = mine_button(&mut app, &ctx, 2.0, "Group").expect("the button shows Group");
    assert!(!off, "not dimmed with objects selected");
    click(&mut app, &ctx, 3.0, at.center());
    assert!(undo(&app) > before, "Group ran");
    assert!(mine_button(&mut app, &ctx, 4.0, "Clean Up").is_some(), "the button shows Clean Up");
    assert!(mine_button(&mut app, &ctx, 4.5, "Clean Up…").is_none(), "without the ellipsis");
}

/// Command buttons are kept with the preferences, and a toolbar saved before toolbars held commands
/// loads with none. A command that is gone, such as an uninstalled plug-in's, stays: its button is
/// dimmed and shows its id, and Remove Command takes it off.
#[test]
fn a_missing_command_stays_dimmed_until_it_is_removed() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    crate::custom_toolbars::find_mut(&mut app.ui, "Mine").unwrap().commands.push("plugin:x".into());
    let kept = app.ui.custom_toolbars.clone();
    let prefs: crate::state::UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
    assert_eq!(prefs.sanitized().custom_toolbars, kept, "kept with the preferences, the missing command too");
    let mut old = serde_json::to_value(&kept[0]).unwrap();
    old.as_object_mut().unwrap().remove("commands");
    let old: crate::state::CustomToolbar = serde_json::from_value(old).unwrap();
    assert!(old.commands.is_empty(), "an older toolbar loads without commands");
    let (_, off) = mine_button(&mut app, &ctx, 0.0, "plugin:x").expect("the button shows the command's id");
    assert!(off, "dimmed while it is gone");
    let left = app.run("toolbar.removeCommand", json!({"command": "plugin:x", "toolbar": "Mine"})).unwrap();
    assert_eq!(left, json!(["object.group"]));
}

/// The footer looks up all its command buttons in one call: each id gets what a lookup of it alone
/// gives, in order, and none for a tool, a command that needs params or a command that is gone.
#[test]
fn command_buttons_are_looked_up_together() {
    let mut app = app();
    let ctx = ctx();
    let ids: Vec<String> =
        ["object.group", "tool:pen", "plugin:x", "toolbar.addTool", "object.path.cleanUp"].into_iter().map(str::to_string).collect();
    let together = in_pass(&mut app, &ctx, |app, ui| crate::palette::button_texts(app, ui, &ids));
    let alone: Vec<_> = ids
        .iter()
        .map(|id| crate::palette::is_command(id).then(|| in_pass(&mut app, &ctx, |app, ui| crate::palette::button_text(app, ui, id))).flatten())
        .collect();
    assert_eq!(together, alone);
    assert!(together[0].is_some() && together[4].is_some(), "Group and Clean Up are found: {together:?}");
    assert!(together[1..4].iter().all(Option::is_none), "a tool, a missing command and one that needs params: {together:?}");
    assert!(in_pass(&mut app, &ctx, |app, ui| crate::palette::button_texts(app, ui, &[])).is_empty());
}

/// Add Tool…'s menu lists a toolbar's command buttons under Remove Command, where a click takes one
/// off, and its Add Command submenu searches the Command Palette's results other than tools: a click
/// on a match adds its button at the end of the toolbar.
#[test]
fn the_add_tool_menu_adds_and_removes_commands() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    let id = crate::custom_toolbars::find(&app.ui, "Mine").unwrap().id;
    // The menu's items called `name`, top first: inside the menu, the Add Tool… node larger than the + button.
    let items = |nodes: &Nodes, name: &str| -> Vec<Rect> {
        let menu = named(nodes, ADD).into_iter().max_by(|a, b| a.area().total_cmp(&b.area())).expect("the menu is open");
        named(nodes, name).into_iter().filter(|r| menu.contains(r.center())).collect()
    };
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
    assert_eq!(named(&nodes, "Remove Command:").len(), 1, "a heading over the toolbar's commands");
    // The submenu lists the matches of its search; a click on one adds it at the end.
    ctx.data_mut(|d| d.insert_temp(crate::custom_toolbars::command_query_id(id), "clean up".to_string()));
    let row = items(&nodes, "Add Command");
    assert_eq!(row.len(), 1, "the Add Command row");
    let nodes = click(&mut app, &ctx, 1.0, row[0].center());
    let name = in_pass(&mut app, &ctx, |app, ui| crate::palette::button_text(app, ui, "object.path.cleanUp")).expect("Clean Up's result").name;
    let at = named(&nodes, &name).first().copied().unwrap_or_else(|| panic!("{name} is listed"));
    click(&mut app, &ctx, 2.0, at.center());
    assert_eq!(mine_commands(&app), ["object.group", "object.path.cleanUp"], "added at the end");
    let pen = in_pass(&mut app, &ctx, |app, ui| crate::palette::command_matches(app, ui, "pen", 30));
    assert!(!pen.is_empty() && pen.iter().all(|(id, _)| !id.starts_with("tool:")), "tools are not listed among the commands: {pen:?}");
    let twice = |name: &str| {
        let parts: Vec<&str> = name.split(" › ").collect();
        (1..=parts.len() / 2).any(|k| parts[..k] == parts[k..2 * k])
    };
    assert!(pen.iter().all(|(_, name)| !twice(name)), "a label that carries its menu path is not given it twice: {pen:?}");
    let toolbar = in_pass(&mut app, &ctx, |app, ui| crate::palette::command_matches(app, ui, "toolbar add", 30));
    assert!(toolbar.iter().all(|(id, _)| id != "toolbar.addTool" && id != "toolbar.addCommand"), "commands that need params: {toolbar:?}");
    // Under Remove Command, Group is listed by its full name, and a click takes it off.
    let name = in_pass(&mut app, &ctx, |app, ui| crate::palette::button_text(app, ui, "object.group")).expect("Group's result").name;
    let nodes = open(&mut app, 10.0);
    let group = items(&nodes, &name);
    assert!(!group.is_empty(), "{name} under Remove Command");
    click(&mut app, &ctx, 11.0, group[0].center());
    assert_eq!(mine_commands(&app), ["object.path.cleanUp"]);
}

/// A button for a panel shows the panel, as the panel's Command Palette result does.
#[test]
fn a_panel_button_shows_its_panel() {
    let mut app = app();
    let ctx = ctx();
    let panel = crate::state::all_panels()
        .map(|(id, _)| id)
        .find(|id| !crate::panel_docking::visible(&app, id) && crate::palette::is_command(&format!("panel:{id}")))
        .expect("a hidden panel");
    let id = format!("panel:{panel}");
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": id, "toolbar": "Mine"})).unwrap();
    let label = in_pass(&mut app, &ctx, |app, ui| crate::palette::button_text(app, ui, &id)).expect("the panel's result").label;
    let (at, _) = mine_button(&mut app, &ctx, 0.0, &label).unwrap_or_else(|| panic!("the button shows {label}"));
    click(&mut app, &ctx, 1.0, at.center());
    assert!(crate::panel_docking::visible(&app, panel), "{panel} is shown");
}

/// A button for a plug-in's command that is not installed is dimmed and shows the command's id;
/// once the plug-in is installed it shows the plug-in's name and can run, and when the plug-in is
/// removed again it is dimmed again.
#[test]
fn a_plugin_command_button_works_again_when_the_plugin_returns() {
    use vectorcraft_plugins::wat as tpl;
    let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), Default::default());
    let ctx = ctx();
    // A name no other test gives a plug-in.
    let filter = "test.UI.toolbar_command-probe";
    let id = format!("plugin:{filter}");
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    crate::custom_toolbars::find_mut(&mut app.ui, "Mine").unwrap().commands.push(id.clone());
    app.run("select.all", json!({})).unwrap();
    let gone = mine_button(&mut app, &ctx, 0.0, &id).map(|(_, off)| off);
    let manifest = format!(r#"{{"id":"{filter}","name":"Toolbar Probe","version":"1","kind":"filter","params":{{}}}}"#);
    vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
    let back = mine_button(&mut app, &ctx, 1.0, "Toolbar Probe").map(|(_, off)| off);
    let added = app.run("toolbar.addCommand", json!({"command": id, "toolbar": "Mine", "at": 0}));
    vectorcraft_plugins::registry::remove(filter);
    let gone_again = mine_button(&mut app, &ctx, 2.0, &id).map(|(_, off)| off);
    assert_eq!(gone, Some(true), "dimmed, with its id, while it is not installed");
    assert_eq!(back, Some(false), "its name, and it can run, once it is installed");
    assert!(added.is_ok(), "an installed plug-in's command can be added: {added:?}");
    assert_eq!(gone_again, Some(true), "dimmed again once it is removed");
}

/// The labels given to the command buttons of the toolbar called Mine.
fn mine_labels(app: &VectorcraftApp) -> std::collections::BTreeMap<String, String> {
    crate::custom_toolbars::find(&app.ui, "Mine").unwrap().labels.clone()
}

/// `toolbar.renameCommand` gives a command button a label of its own, an empty label gives it back
/// its command's, and a bad call changes nothing. `toolbar.addCommand` takes the label too. A move
/// and Manage Toolbars' copy keep it, Remove and Reset drop it, and the preferences keep it while a
/// toolbar saved without labels loads unchanged.
#[test]
fn a_command_button_takes_a_label_of_its_own() {
    let mut app = app();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    let done = app.run("toolbar.renameCommand", json!({"command": "object.group", "toolbar": "Mine", "label": "  Grp "})).unwrap();
    assert_eq!(done, json!({"command": "object.group", "label": "Grp"}), "spaces at the ends are dropped");
    let list = app.run("window.toolbar.list", json!({})).unwrap();
    assert_eq!(list[0]["labels"], json!({"object.group": "Grp"}), "listed");
    let long = "x".repeat(65);
    for (p, why) in [
        (json!({"command": "object.group", "toolbar": "Mine", "label": long}), "a label past the limit"),
        (json!({"command": "object.group", "toolbar": "Mine", "label": 3}), "a label that is not text"),
        (json!({"command": "object.ungroup", "toolbar": "Mine", "label": "U"}), "a button the toolbar does not have"),
        (json!({"command": "object.group", "label": "G"}), "no toolbar"),
        (json!({"toolbar": "Mine", "label": "G"}), "no command"),
    ] {
        assert!(app.run("toolbar.renameCommand", p.clone()).is_err(), "{why}: {p}");
        assert_eq!(mine_labels(&app).get("object.group").map(String::as_str), Some("Grp"), "{why} changes nothing");
    }
    for p in [
        json!({"command": "object.path.cleanUp", "toolbar": "Mine", "label": "x".repeat(65)}),
        json!({"command": "object.path.cleanUp", "toolbar": "Mine", "label": false}),
    ] {
        assert!(app.run("toolbar.addCommand", p.clone()).is_err(), "{p}");
        assert_eq!(mine_commands(&app), ["object.group"], "a bad label adds nothing: {p}");
    }
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine", "at": 0, "label": "Tidy"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine", "at": 0})).unwrap();
    assert_eq!(mine_commands(&app), ["object.group", "object.path.cleanUp"]);
    let both =
        std::collections::BTreeMap::from([("object.group".to_string(), "Grp".to_string()), ("object.path.cleanUp".to_string(), "Tidy".to_string())]);
    assert_eq!(mine_labels(&app), both, "a move keeps the label");
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine", "label": "Clear"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine", "label": ""})).unwrap();
    assert!(!mine_labels(&app).contains_key("object.path.cleanUp"), "an empty label on addCommand gives back the command's");
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine", "label": "Tidy"})).unwrap();
    assert_eq!(mine_labels(&app), both);
    // Kept with the preferences; an older toolbar without labels loads with none.
    let kept = app.ui.custom_toolbars.clone();
    let prefs: crate::state::UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
    assert_eq!(prefs.sanitized().custom_toolbars, kept, "kept with the preferences");
    let mut old = serde_json::to_value(&kept[0]).unwrap();
    old.as_object_mut().unwrap().remove("labels");
    let old: crate::state::CustomToolbar = serde_json::from_value(old).unwrap();
    assert!(old.labels.is_empty() && old.commands == kept[0].commands, "an older toolbar loads with its buttons and no labels");
    let copy = app.run("window.toolbar.duplicate", json!({"name": "Mine"})).unwrap();
    let copy = copy["toolbar"].as_str().unwrap().to_string();
    assert_eq!(crate::custom_toolbars::find(&app.ui, &copy).unwrap().labels, both, "the copy has them");
    // An empty label gives back the command's own.
    let done = app.run("toolbar.renameCommand", json!({"command": "object.group", "toolbar": "Mine", "label": " "})).unwrap();
    assert_eq!(done, json!({"command": "object.group", "label": null}));
    assert!(!mine_labels(&app).contains_key("object.group"));
    // Removing the button drops its label, so adding it again shows the command's label.
    app.run("toolbar.removeCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine"})).unwrap();
    assert!(mine_labels(&app).is_empty(), "removed with its button");
    app.run("toolbar.renameCommand", json!({"command": "object.group", "toolbar": "Mine", "label": "G"})).unwrap();
    app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap();
    assert!(mine_labels(&app).is_empty(), "Reset drops the labels");
    assert_eq!(crate::custom_toolbars::find(&app.ui, &copy).unwrap().labels, both, "the copy keeps its own");
}

/// Loading drops the spaces at a label's ends, and drops a label whose button the toolbar does not
/// have, one past the limit and an empty one.
#[test]
fn labels_load_sanitized() {
    let mut app = app();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.ungroup", "toolbar": "Mine"})).unwrap();
    let bar = crate::custom_toolbars::find_mut(&mut app.ui, "Mine").unwrap();
    bar.labels.insert("object.group".into(), " Grp ".into());
    bar.labels.insert("object.ungroup".into(), "x".repeat(65));
    bar.labels.insert("object.path.cleanUp".into(), "Tidy".into());
    bar.commands.push("edit.selectAll".into());
    bar.labels.insert("edit.selectAll".into(), String::new());
    let loaded = app.ui.clone().sanitized();
    let bar = loaded.custom_toolbars.iter().find(|b| b.name == "Mine").unwrap();
    assert_eq!(bar.labels, std::collections::BTreeMap::from([("object.group".to_string(), "Grp".to_string())]));
}

/// A button with a label of its own shows that label, and a screen reader says the label and then
/// the command's name.
/// Control characters and characters that change the text's direction show as escapes.
#[test]
fn a_renamed_button_shows_its_label_and_keeps_the_command_name() {
    let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), Default::default());
    let ctx = ctx();
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine", "label": "Grp"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.ungroup", "toolbar": "Mine", "label": "a\u{202E}b"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.path.cleanUp", "toolbar": "Mine"})).unwrap();
    let text = |app: &mut VectorcraftApp, id: &str| in_pass(app, &ctx, |app, ui| crate::palette::button_text(app, ui, id)).expect("the result");
    let texts = |app: &mut VectorcraftApp| in_pass(app, &ctx, |app, ui| crate::custom_toolbars::button_texts(app, ui, "Mine"));
    let group = text(&mut app, "object.group");
    let shown = texts(&mut app);
    let (label, said) = &shown[0];
    assert_eq!(label, "Grp");
    assert!(said.starts_with("Grp") && said.contains(&group.name), "the screen reader says the label, then the name: {said}");
    assert!(!shown[1].0.contains('\u{202E}') && shown[1].0.contains("202E"), "escaped: {:?}", shown[1].0);
    assert_eq!(shown[2].0, text(&mut app, "object.path.cleanUp").label, "no label: the command's");
    // Drawn on the toolbar, and named so in the screen-reader tree.
    assert!(mine_button(&mut app, &ctx, 0.0, said).is_some(), "the tree names it {said}");
    assert!(mine_button(&mut app, &ctx, 0.2, &group.label).is_none(), "and not by the command's label");
    // A label that is the command's own label is said once; escapes do not cut a label short.
    app.run("toolbar.renameCommand", json!({"command": "object.group", "toolbar": "Mine", "label": group.label.clone()})).unwrap();
    let escaped = format!("{}{}", "\u{202E}".repeat(20), "y".repeat(44));
    app.run("toolbar.renameCommand", json!({"command": "object.ungroup", "toolbar": "Mine", "label": escaped})).unwrap();
    let texts = texts(&mut app);
    assert_eq!(texts[0].1, group.label, "said once");
    assert!(texts[1].0.ends_with(&"y".repeat(44)) && !texts[1].0.contains('…'), "not cut: {:?}", texts[1].0);
}

/// Rename Button… in a command button's context menu opens the Rename Button dialog.
#[test]
fn rename_button_in_the_context_menu_opens_its_dialog() {
    let mut app = crate::testing::windowed(app());
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    app.run("toolbar.addCommand", json!({"command": "object.group", "toolbar": "Mine"})).unwrap();
    let group = in_pass(&mut app, &ctx, |app, ui| crate::palette::button_text(app, ui, "object.group")).expect("Group's result").label;
    let (at, _) = mine_button(&mut app, &ctx, 0.0, &group).expect("the button");
    let at = at.center();
    let secondary = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), secondary(true)]);
    frame(&mut app, &ctx, 1.05, vec![secondary(false)]);
    let shown = settle(&mut app, &ctx, 1.1);
    let item = named(&shown, "Rename Button…");
    assert_eq!(item.len(), 1, "Rename Button… in the context menu");
    click(&mut app, &ctx, 2.0, item[0].center());
    assert_eq!(crate::tests::picks::open_dialogs(&app).len(), 1, "the dialog opens");
}

/// The Rename Button dialog shows the button's label (empty: the command's own), and OK gives the
/// button the label typed. A label past the limit is refused and the dialog stays open, and Cancel
/// changes nothing. An OK after switching workspaces is refused, so it cannot rename a button of the
/// same name on a toolbar of the same name in the workspace switched to.
#[test]
fn the_rename_button_dialog_renames_the_button() {
    use crate::tests::panel_harness::Panel;
    use crate::tests::picks::open_dialogs;
    use egui::{Key, Modifiers};

    fn type_label(panel: &mut Panel, label: &str) {
        panel.click("Label");
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text(label);
        panel.key(Key::Tab, Modifiers::NONE);
    }
    let rename = json!({"command": "object.group", "toolbar": "Mine"});

    let mut panel = Panel::new(crate::testing::windowed(app()), crate::tests::picks::dialogs);
    panel.room(egui::vec2(800.0, 600.0));
    panel.app().run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    panel.app().run("toolbar.addCommand", rename.clone()).unwrap();
    assert!(panel.app().run("toolbar.renameCommand", json!({"command": "object.ungroup", "toolbar": "Mine"})).is_err(), "no such button");
    assert!(open_dialogs(&panel.app()).is_empty(), "and no dialog");
    let opened = panel.app().run("toolbar.renameCommand", rename.clone()).unwrap();
    assert_eq!(opened, Value::Null);
    panel.frame();
    assert_eq!(panel.value("Label").as_deref(), Some(""), "empty while the button shows its command's label");
    type_label(&mut panel, "Grp");
    panel.click("OK");
    panel.frame();
    assert!(open_dialogs(&panel.app()).is_empty(), "OK closes it");
    assert_eq!(mine_labels(&panel.app()).get("object.group").map(String::as_str), Some("Grp"));

    panel.app().run("toolbar.renameCommand", rename.clone()).unwrap();
    panel.frame();
    assert_eq!(panel.value("Label").as_deref(), Some("Grp"), "opened again, it shows the label");
    type_label(&mut panel, &"x".repeat(65));
    panel.click("OK");
    panel.frame();
    assert_eq!(open_dialogs(&panel.app()).len(), 1, "a label past the limit keeps the dialog open");
    assert_eq!(mine_labels(&panel.app()).get("object.group").map(String::as_str), Some("Grp"), "unchanged");
    type_label(&mut panel, "Other");
    panel.click("Cancel");
    panel.frame();
    assert!(open_dialogs(&panel.app()).is_empty());
    assert_eq!(mine_labels(&panel.app()).get("object.group").map(String::as_str), Some("Grp"), "Cancel changes nothing");

    // Drawing starts with a copy of Mine, so both workspaces have Mine with a Group button.
    panel.app().run("toolbar.renameCommand", json!({"command": "object.group", "toolbar": "Mine", "label": ""})).unwrap();
    panel.app().run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
    panel.app().run("toolbar.renameCommand", rename.clone()).unwrap();
    panel.frame();
    panel.app().run("window.workspace", json!({"name": "Essentials"})).unwrap();
    type_label(&mut panel, "Grp");
    panel.click("OK");
    panel.frame();
    assert_eq!(open_dialogs(&panel.app()).len(), 1, "an OK after switching workspaces is refused");
    assert!(mine_labels(&panel.app()).is_empty(), "Essentials' Mine keeps its command's label");
    panel.click("Cancel");
    panel.frame();
    panel.app().run("window.workspace", json!({"name": "Drawing"})).unwrap();
    assert!(mine_labels(&panel.app()).is_empty(), "and so does Drawing's");
}
