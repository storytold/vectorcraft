use egui::{Color32, Event, PointerButton, RawInput};
use vectorcraft_engine::Session;

use super::*;

/// Tall enough for the whole Basic toolbar and the whole drawer.
const SCREEN: egui::Vec2 = vec2(1000.0, 4000.0);
/// A toolbar tool button.
const BUTTON: egui::Vec2 = vec2(36.0, 29.0);

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

/// One headless frame of the toolbar, its keys handled first as in the app; returns the texts
/// painted: (text, a point inside it, colour).
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<(String, Pos2, Color32)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Pos2, Color32)>) {
        match s {
            egui::Shape::Text(t) => {
                let color = t.galley.job.sections.first().map_or(t.fallback_color, |s| s.format.color);
                out.push((t.galley.text().to_string(), t.pos + vec2(4.0, t.galley.size().y / 2.0), color));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let input = RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        crate::toolbar::show(app, ui);
    });
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    shown
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2) {
    frame(app, ctx, time, vec![Event::PointerMoved(at), button(at, true)]);
    frame(app, ctx, time + 0.05, vec![button(at, false)]);
    frame(app, ctx, time + 0.1, vec![]);
}

/// A press at `from` dragged to `to` in steps, one frame each, then released there.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, from: Pos2, to: Pos2) {
    frame(app, ctx, time, vec![Event::PointerMoved(from), button(from, true)]);
    for k in 1..=6 {
        frame(app, ctx, time + 0.05 * f64::from(k), vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
    }
    frame(app, ctx, time + 0.4, vec![button(to, false)]);
    frame(app, ctx, time + 0.45, vec![]);
}

fn drawer_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.memory(|m| m.area_rect(drawer_id()))
}

fn drawer_shown(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, drawer_id())))
}

/// A point inside the drawer's row of the tool named `name` (as the drawer shows it), and the
/// colour of its name.
fn cell_at(shown: &[(String, Pos2, Color32)], drawer: Rect, name: &str) -> (Pos2, Color32) {
    shown
        .iter()
        .find(|(s, p, _)| s == name && drawer.contains(*p))
        .map(|(_, p, c)| (*p, *c))
        .unwrap_or_else(|| panic!("no {name} in the drawer {drawer:?}"))
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
    assert_eq!(slots(&app)[3], ["lasso"]);
    // After a slot, and into a group (last), moved out of the Rectangle group.
    app.run("toolbar.addTool", json!({"tool": "slice", "slot": 1, "place": "after"})).unwrap();
    assert_eq!(slots(&app)[2], ["slice"]);
    let ellipse = slot_of(&app, "ellipse").unwrap();
    app.run("toolbar.addTool", json!({"tool": "star", "slot": ellipse, "place": "group"})).unwrap();
    assert_eq!(slots(&app)[ellipse], ["ellipse", "star"]);
    assert!(!slots(&app)[slot_of(&app, "rectangle").unwrap()].contains(&"star"));
    // Onto its own group: nothing changes.
    let before = slots(&app);
    app.run("toolbar.addTool", json!({"tool": "ellipse", "slot": ellipse, "place": "group"})).unwrap();
    assert_eq!(slots(&app), before);
    // The category label still names the first slot of its category.
    assert_eq!(crate::toolbar::slots(&app)[0].0, Some("Select"));
    // Out: one tool, or its whole group; an emptied slot goes.
    app.run("toolbar.removeTool", json!({"tool": "ellipse"})).unwrap();
    assert_eq!(slots(&app)[ellipse], ["star"]);
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

/// #1018: the ••• button opens the drawer beside the toolbar, by the button, not a centred
/// dialog; Escape, a click outside and the button close it, a click in the toolbar doesn't.
#[test]
fn edit_toolbar_opens_a_drawer_beside_the_toolbar() {
    let mut app = app();
    let ctx = ctx();
    frame(&mut app, &ctx, 0.0, vec![]);
    let edit = crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).last().copied().unwrap();
    click(&mut app, &ctx, 1.0, edit.center());
    frame(&mut app, &ctx, 1.2, vec![]);
    assert!(app.ui.tools_drawer && app.ui.dialog.is_none());
    assert_eq!(crate::menus::checked(&app, "window.editToolbar", &json!({})), Some(true));
    let drawer = drawer_rect(&ctx).expect("the drawer is shown");
    let bar = ctx.data(|d| d.get_temp::<Rect>(toolbar_id())).unwrap();
    assert!((drawer.left() - bar.right() - GAP).abs() < 1.0, "beside the toolbar: {drawer:?} {bar:?}");
    assert!((drawer.top() - edit.top()).abs() < 1.0 || drawer.bottom() >= SCREEN.y - GAP - 1.0, "by the button: {drawer:?} {edit:?}");
    assert!(drawer.top() >= bar.top() - 0.5, "never above the toolbar (the title bar's there): {drawer:?} {bar:?}");
    assert!(drawer.y_range().contains(edit.center().y), "by the button: {drawer:?} {edit:?}");
    assert!(drawer.width() < WIDTH + 24.0, "one row of tools wide: {drawer:?}");
    // A click on a toolbar tool picks it and leaves the drawer open.
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    let ellipse = tools[slot_of(&app, "ellipse").unwrap()].center();
    click(&mut app, &ctx, 2.0, ellipse);
    assert!(app.ui.tools_drawer && app.session.tool_id() == "ellipse");
    // Escape closes it.
    let esc = |pressed| Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(&mut app, &ctx, 3.0, vec![esc(true), esc(false)]);
    assert!(!app.ui.tools_drawer);
    frame(&mut app, &ctx, 3.1, vec![]);
    assert!(!drawer_shown(&ctx));
    // A click outside closes it, and so does the ••• button.
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    frame(&mut app, &ctx, 4.0, vec![]);
    click(&mut app, &ctx, 4.5, pos2(SCREEN.x - 20.0, 300.0));
    assert!(!app.ui.tools_drawer);
    click(&mut app, &ctx, 5.0, edit.center());
    assert!(app.ui.tools_drawer);
    click(&mut app, &ctx, 6.0, edit.center());
    assert!(!app.ui.tools_drawer);
    // Picking a tool in the drawer selects it and closes the drawer.
    app.run("window.editToolbar", json!({})).unwrap();
    frame(&mut app, &ctx, 7.0, vec![]);
    let shown = frame(&mut app, &ctx, 7.1, vec![]);
    let (at, _) = cell_at(&shown, drawer_rect(&ctx).unwrap(), "Artboard Tool");
    click(&mut app, &ctx, 8.0, at);
    assert!(!app.ui.tools_drawer && app.session.tool_id() == "artboard");
}

/// The drawer dims the tools the toolbar holds.
#[test]
fn tools_in_the_toolbar_are_dimmed_in_the_drawer() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    let shown = frame(&mut app, &ctx, 0.1, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    let dim = Tokens::get(&ctx).text_disabled;
    assert_eq!(cell_at(&shown, drawer, "Ellipse Tool").1, dim);
    assert_ne!(cell_at(&shown, drawer, "Artboard Tool").1, dim);
    app.run("toolbar.addTool", json!({"tool": "artboard"})).unwrap();
    app.run("toolbar.removeTool", json!({"tool": "ellipse"})).unwrap();
    let shown = frame(&mut app, &ctx, 0.2, vec![]);
    assert_ne!(cell_at(&shown, drawer, "Ellipse Tool").1, dim);
    assert_eq!(cell_at(&shown, drawer, "Artboard Tool").1, dim);
}

/// #1018: tools drag from the drawer onto the toolbar (between two tools, or onto one to join
/// its group), from the toolbar into the drawer, and within the toolbar, only while the drawer
/// is open.
#[test]
fn tools_drag_between_the_drawer_and_the_toolbar() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    let shown = frame(&mut app, &ctx, 0.1, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    // Onto the divider under the Selection tool: its own slot, second.
    let (artboard, _) = cell_at(&shown, drawer, "Artboard Tool");
    drag(&mut app, &ctx, 1.0, artboard, tools[0].center_bottom() - vec2(0.0, 3.0));
    assert_eq!(&slots(&app)[..2], [&["selection"][..], &["artboard"]]);
    assert!(app.ui.tools_drawer, "the drawer stays open");
    // Onto the middle of the Ellipse tool: into its group.
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    let ellipse = slot_of(&app, "ellipse").unwrap();
    // The drawer moves with the ••• button the new slot pushed down: read it again.
    let shown = frame(&mut app, &ctx, 1.9, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    let (slice, _) = cell_at(&shown, drawer, "Slice Tool");
    drag(&mut app, &ctx, 2.0, slice, tools[ellipse].center());
    assert_eq!(slots(&app)[ellipse], ["ellipse", "slice"]);
    // A dimmed tool (in the toolbar already) doesn't drag.
    let before = slots(&app);
    let shown = frame(&mut app, &ctx, 2.9, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    let (rectangle, _) = cell_at(&shown, drawer, "Rectangle Tool");
    drag(&mut app, &ctx, 3.0, rectangle, tools[0].center_bottom() - vec2(0.0, 3.0));
    assert_eq!(slots(&app), before);
    // From the toolbar into the drawer: out of the toolbar.
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    drag(&mut app, &ctx, 4.0, tools[1].center(), drawer.center());
    assert!(slot_of(&app, "artboard").is_none(), "{:?}", slots(&app));
    // Within the toolbar: Selection onto the divider over the Lasso.
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    drag(&mut app, &ctx, 5.0, tools[0].center(), tools[2].center_top() + vec2(0.0, 3.0));
    assert_eq!(&slots(&app)[..3], [&["directSelection", "groupSelection"][..], &["selection"], &["lasso", "magicWand"]]);
    // The drawer closed, the toolbar is locked: the same drag moves nothing.
    app.run("window.editToolbar", json!({"open": false})).unwrap();
    frame(&mut app, &ctx, 6.0, vec![]);
    let before = slots(&app);
    let tools = crate::toolbar::tests::widget_rects(&ctx, BUTTON);
    drag(&mut app, &ctx, 7.0, tools[0].center(), tools[2].center_bottom() - vec2(0.0, 3.0));
    assert_eq!(slots(&app), before);
}

/// A toolbar made with New Toolbar floats blank; its ••• button opens the drawer beside it, and
/// tools dragged from the drawer fill it, leaving the Tools panel's toolbar as it was.
#[test]
fn a_new_toolbar_fills_from_the_drawer_beside_it() {
    let mut app = app();
    let ctx = ctx();
    let shipped = slots(&app);
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let area = || ctx.memory(|m| m.area_rect(crate::custom_toolbars::area_id("Mine"))).expect("the toolbar is shown");
    let bar = area();
    let edit = crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().find(|r| bar.contains(r.center())).expect("its ••• button");
    // Open beside the Tools panel's toolbar, the drawer moves to this one on its ••• click.
    app.run("window.editToolbar", json!({})).unwrap();
    frame(&mut app, &ctx, 0.5, vec![]);
    click(&mut app, &ctx, 1.0, edit.center());
    assert!(app.ui.tools_drawer && app.ui.drawer_toolbar.as_deref() == Some("Mine"));
    assert_eq!(crate::menus::checked(&app, "window.editToolbar", &json!({"toolbar": "Mine"})), Some(true));
    assert_eq!(crate::menus::checked(&app, "window.editToolbar", &json!({})), Some(false));
    let shown = frame(&mut app, &ctx, 1.2, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    assert!((drawer.left() - bar.right() - GAP).abs() < 2.0, "beside the new toolbar: {drawer:?} {bar:?}");
    // The Tools panel's tools aren't dimmed: they aren't in this toolbar. The Show section, for
    // the controls under the Tools panel's tools, is left out.
    assert!(!shown.iter().any(|(s, p, _)| s == "Show:" && drawer.contains(*p)));
    let dim = Tokens::get(&ctx).text_disabled;
    assert_ne!(cell_at(&shown, drawer, "Ellipse Tool").1, dim);
    // Dragged onto the blank toolbar, a tool is added to it.
    let (ellipse, _) = cell_at(&shown, drawer, "Ellipse Tool");
    drag(&mut app, &ctx, 2.0, ellipse, bar.center());
    assert_eq!(crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots, [["ellipse"]]);
    assert_eq!(slots(&app), shipped, "the Tools panel's toolbar is untouched");
    let shown = frame(&mut app, &ctx, 2.9, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    assert_eq!(cell_at(&shown, drawer, "Ellipse Tool").1, dim, "now in the toolbar the drawer edits");
    // A second one, below the first.
    let tools: Vec<Rect> = crate::toolbar::tests::widget_rects(&ctx, BUTTON).into_iter().filter(|r| area().contains(r.center())).collect();
    let (artboard, _) = cell_at(&shown, drawer, "Artboard Tool");
    drag(&mut app, &ctx, 3.0, artboard, tools[0].center_bottom() - vec2(0.0, 3.0));
    assert_eq!(crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots, [["ellipse"], ["artboard"]]);
    // A click on its tool picks it; the drawer stays open.
    let tools: Vec<Rect> = crate::toolbar::tests::widget_rects(&ctx, BUTTON).into_iter().filter(|r| area().contains(r.center())).collect();
    click(&mut app, &ctx, 4.0, tools[1].center());
    assert_eq!(app.session.tool_id(), "artboard");
    assert!(app.ui.tools_drawer);
    // Dragged from it into the drawer, a tool leaves it.
    drag(&mut app, &ctx, 5.0, tools[0].center(), drawer.center());
    assert_eq!(crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots, [["artboard"]]);
    // The Tools panel's ••• button takes the drawer back beside its toolbar.
    let main_edit = crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().rfind(|r| r.right() < 60.0).unwrap();
    click(&mut app, &ctx, 5.5, main_edit.center());
    assert!(app.ui.tools_drawer && app.ui.drawer_toolbar.is_none());
    // The drawer closed, a drag on the grip bar moves the toolbar.
    app.run("window.editToolbar", json!({"open": false})).unwrap();
    frame(&mut app, &ctx, 6.0, vec![]);
    let before = area();
    drag(&mut app, &ctx, 7.0, before.center_top() + vec2(0.0, 5.0), before.center_top() + vec2(120.0, 205.0));
    frame(&mut app, &ctx, 7.5, vec![]);
    let after = area();
    assert!((after.min - before.min - vec2(120.0, 200.0)).length() < 2.0, "{before:?} {after:?}");
    // Hidden, it isn't drawn.
    app.run("window.toolbar.custom", json!({"name": "Mine", "show": false})).unwrap();
    frame(&mut app, &ctx, 8.0, vec![]);
    frame(&mut app, &ctx, 8.1, vec![]);
    assert!(!ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Middle, crate::custom_toolbars::area_id("Mine")))));
}

/// New Toolbar… in the drawer's menu opens the New Toolbar dialog, which asks for a name.
#[test]
fn new_toolbar_in_the_drawer_menu_asks_for_a_name() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let drawer = drawer_rect(&ctx).expect("the drawer is open");
    let menu = crate::toolbar::tests::widget_rects(&ctx, vec2(18.0, 18.0))
        .into_iter()
        .find(|r| drawer.contains(r.center()))
        .expect("the drawer menu button");
    click(&mut app, &ctx, 1.0, menu.center());
    // The menu is sized in its first frame and drawn from the second.
    frame(&mut app, &ctx, 1.2, vec![]);
    let shown = frame(&mut app, &ctx, 1.3, vec![]);
    // A menu item's label is indented where a checked item has its ✓.
    let item = shown.iter().find(|(s, ..)| s.trim() == "New Toolbar…").map(|(_, p, _)| *p).expect("New Toolbar… in the drawer menu");
    assert!(app.ui.dialog.is_none());
    click(&mut app, &ctx, 2.0, item);
    let d = app.ui.dialog.clone().expect("the dialog asks for a name");
    assert_eq!((d.kind.as_str(), d.str("name").as_str()), (crate::custom_toolbars::KIND, "Toolbar 1"));
}

/// A click on another toolbar's ••• button takes the drawer there, whichever was made first.
#[test]
fn the_drawer_moves_between_new_toolbars() {
    let mut app = app();
    let ctx = ctx();
    for name in ["A", "B"] {
        app.run("window.toolbar.new", json!({"name": name})).unwrap();
    }
    app.ui.custom_toolbars[1].pos = [600.0, 900.0];
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let edit_of = |name: &str| {
        let bar = ctx.memory(|m| m.area_rect(crate::custom_toolbars::area_id(name))).unwrap();
        crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().find(|r| bar.contains(r.center())).unwrap()
    };
    let (a, b) = (edit_of("A"), edit_of("B"));
    for (k, (button, name)) in [(b, "B"), (a, "A"), (b, "B")].into_iter().enumerate() {
        click(&mut app, &ctx, 1.0 + k as f64, button.center());
        frame(&mut app, &ctx, 1.5 + k as f64, vec![]);
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
        assert!(app.ui.toolbar, "the Tools panel shows beside the drawer");
    }
    assert_eq!(crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots, [["pen"]], "Mine as it was");
}

#[test]
fn drop_targets_follow_the_buttons_edges() {
    let r = |y: f32| Rect::from_min_size(pos2(6.0, y), BUTTON);
    let one = [(0, r(0.0)), (1, r(30.0))];
    assert_eq!(drop_target(&one, pos2(20.0, 2.0), 1), (Some(0), Place::Before));
    assert_eq!(drop_target(&one, pos2(20.0, 14.0), 1), (Some(0), Place::Group));
    assert_eq!(drop_target(&one, pos2(20.0, 27.0), 1), (Some(0), Place::After));
    // Past the last button, or beside them: next to the nearest one.
    assert_eq!(drop_target(&one, pos2(20.0, 200.0), 1), (Some(1), Place::After));
    assert_eq!(drop_target(&one, pos2(44.5, 40.0), 1), (Some(1), Place::After));
    // Two columns: the left and right edges.
    let two = [(0, Rect::from_min_size(pos2(2.0, 0.0), BUTTON)), (1, Rect::from_min_size(pos2(38.0, 0.0), BUTTON))];
    assert_eq!(drop_target(&two, pos2(40.0, 10.0), 2), (Some(1), Place::Before));
    assert_eq!(drop_target(&two, pos2(36.0, 10.0), 2), (Some(0), Place::After));
    assert_eq!(drop_target(&[], pos2(0.0, 0.0), 1), (None, Place::Before));
}

/// The add-ons' tools are in neither toolbar as they ship: the drawer lists them under their
/// add-on's heading, and they can be added from there.
#[test]
fn addon_tools_are_in_the_drawer_not_the_toolbars() {
    let mut app = app();
    let addon: Vec<&str> = vectorcraft_tools::addons::GROUPS.iter().flat_map(|g| g.iter().map(|t| t.id)).collect();
    assert_eq!(
        addon,
        ["anchorEraserBrush", "extendPath", "measureInspect", "parametricShape", "cornerTool", "slideAnchor", "sharedEdge", "smartPen"]
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
    assert_eq!(listed("extendPath"), Some(("Path Precision Tools", addon[..7].to_vec())));
    assert_eq!(listed("smartPen"), Some(("Smart Pen", vec!["smartPen"])));
    for tool in ["extendPath", "smartPen"] {
        app.run("toolbar.addTool", json!({"tool": tool})).unwrap();
        assert!(slot_of(&app, tool).is_some(), "{tool}");
    }
}
