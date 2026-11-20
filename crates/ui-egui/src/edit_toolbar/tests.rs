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

/// One headless frame of the toolbar, its keys handled first as in the app; returns what it painted.
fn run_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<egui::Shape> {
    run_frame_in(app, ctx, SCREEN, time, events)
}

/// [`run_frame`] in a window of `size`.
fn run_frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, size: egui::Vec2, time: f64, events: Vec<Event>) -> Vec<egui::Shape> {
    let input = RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        crate::toolbar::show(app, ui);
    });
    out.textures_delta.clear();
    out.shapes.into_iter().map(|c| c.shape).collect()
}

/// The texts `shape` paints, added to `out`: (text, a point inside it, colour).
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

/// One headless frame of the toolbar, its keys handled first as in the app; returns the texts
/// painted: (text, a point inside it, colour).
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<(String, Pos2, Color32)> {
    let mut shown = vec![];
    run_frame(app, ctx, time, events).iter().for_each(|s| texts(s, &mut shown));
    shown
}

/// The fills painted over `at`, in the order they are drawn.
fn fills_at(shapes: &[egui::Shape], at: Pos2) -> Vec<Color32> {
    fn fills(s: &egui::Shape, at: Pos2, out: &mut Vec<Color32>) {
        match s {
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(at) => out.push(r.fill),
            egui::Shape::Vec(v) => v.iter().for_each(|s| fills(s, at, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shapes.iter().for_each(|s| fills(s, at, &mut out));
    out
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

/// A context in `scheme` at brightness `b`, as Preferences sets them.
fn ctx_in(scheme: crate::theme::Scheme, b: crate::theme::Brightness) -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply_with(&ctx, b, crate::theme::Accent::default(), scheme);
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

/// The WCAG contrast ratio of `c` drawn on `bg`, between 1 and 21.
fn contrast(c: Color32, bg: Color32) -> f32 {
    fn channel(v: u8) -> f32 {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }
    fn lum(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (a, b) = (lum(c), lum(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
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

/// The Enclosing Lasso Tool is in the Lasso tool's flyout in both toolbars, right after the Lasso,
/// once in the All Tools drawer (under Select) and in Keyboard Shortcuts' list, with no shortcut;
/// Reset Toolbar keeps it there.
#[test]
fn enclosing_lasso_is_in_the_lasso_flyout() {
    let mut app = app();
    let id = vectorcraft_tools::addons::enclosing_lasso::ID;
    for advanced in [false, true] {
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        let s = slots(&app);
        let slot = slot_of(&app, "lasso").map(|i| s[i].clone()).expect("a Lasso slot");
        let lasso = slot.iter().position(|t| *t == "lasso").expect("the Lasso");
        assert_eq!(slot.get(lasso + 1), Some(&id), "advanced: {advanced}");
        assert_eq!(s.iter().filter(|ids| ids.contains(&id)).count(), 1, "advanced: {advanced}");
    }
    assert_eq!(heading(id), "Select");
    assert_eq!(sections().iter().flat_map(|(_, tools)| tools.iter()).filter(|t| t.id == id).count(), 1, "listed once in the drawer");
    assert_eq!(vectorcraft_tools::catalog::all_tools().filter(|t| t.id == id).count(), 1, "listed once for Keyboard Shortcuts");
    assert_eq!(vectorcraft_tools::tool_info(id).map(|t| (t.label, t.shortcut)), Some(("Enclosing Lasso Tool", None)));
    app.run("tool.select", json!({"tool": id})).unwrap();
    assert_eq!(app.session.tool_id(), id);
    let shown = slot_of(&app, id).map(|i| crate::toolbar::shown_tool(&app, &slots(&app)[i]));
    assert_eq!(shown, Some(id), "the slot's button shows the tool once chosen");
    app.run("toolbar.removeTool", json!({"tool": id})).unwrap();
    assert!(slot_of(&app, id).is_none(), "it can be taken out");
    app.run("toolbar.reset", json!({})).unwrap();
    let after_reset = slot_of(&app, "lasso").map(|i| slots(&app)[i].contains(&id));
    assert_eq!(after_reset, Some(true), "Reset Toolbar puts it back");
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

/// The drawer dims the tools the toolbar holds, in the dim text colour, which stays readable
/// against the panel; the disabled colour, at the drawer's 12.5 px, does not (#8294). Checked in
/// every colour scheme and brightness: the dim text reaches 4.5:1 wherever the palette's own dim
/// text does — everywhere but Classic at its middle-dark brightness, whose dim text is about 3.6:1
/// against its panel — and the drawer never dims it further.
#[test]
fn tools_in_the_toolbar_are_dimmed_in_the_drawer() {
    for scheme in crate::theme::Scheme::ALL {
        for b in crate::theme::Brightness::ALL {
            let mut app = app();
            let ctx = ctx_in(scheme, b);
            let what = format!("{scheme:?} {b:?}");
            app.run("window.editToolbar", json!({"open": true})).unwrap();
            frame(&mut app, &ctx, 0.0, vec![]);
            let shown = frame(&mut app, &ctx, 0.1, vec![]);
            let drawer = drawer_rect(&ctx).unwrap();
            let t = Tokens::get(&ctx);
            let dim = t.text_dim;
            let (_, color) = cell_at(&shown, drawer, "Ellipse Tool");
            assert_eq!(color, dim, "{what}");
            let readable = contrast(dim, t.panel).min(4.5);
            assert!(
                contrast(color, t.panel) >= readable,
                "{what}: a dimmed tool's name must stay readable: {color:?} on {:?} is {:.2}:1",
                t.panel,
                contrast(color, t.panel)
            );
            assert_ne!(cell_at(&shown, drawer, "Artboard Tool").1, dim, "{what}");
            assert_ne!(cell_at(&shown, drawer, "Artboard Tool").1, t.text_disabled, "{what}: the disabled colour is unreadable at the drawer's size");
        }
    }
    // A tool added to the toolbar is dimmed, one removed is not.
    let mut app = app();
    let ctx = ctx();
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let drawer = drawer_rect(&ctx).unwrap();
    let dim = Tokens::get(&ctx).text_dim;
    app.run("toolbar.addTool", json!({"tool": "artboard"})).unwrap();
    app.run("toolbar.removeTool", json!({"tool": "ellipse"})).unwrap();
    let shown = frame(&mut app, &ctx, 0.2, vec![]);
    assert_ne!(cell_at(&shown, drawer, "Ellipse Tool").1, dim);
    assert_eq!(cell_at(&shown, drawer, "Artboard Tool").1, dim);
}

/// A hovered dimmed row keeps its text readable: its hover is a recess in the panel's darker fill,
/// not the lighter hover fill, which would leave the dim text at 4.3:1 in the default theme
/// (#8294). Every colour scheme and brightness keeps 4.5:1 over that fill.
#[test]
fn a_hovered_dimmed_row_stays_readable() {
    for scheme in crate::theme::Scheme::ALL {
        for b in crate::theme::Brightness::ALL {
            let mut app = app();
            let ctx = ctx_in(scheme, b);
            let what = format!("{scheme:?} {b:?}");
            app.run("window.editToolbar", json!({"open": true})).unwrap();
            frame(&mut app, &ctx, 0.0, vec![]);
            let shown = frame(&mut app, &ctx, 0.1, vec![]);
            let drawer = drawer_rect(&ctx).unwrap();
            let t = Tokens::get(&ctx);
            let (at, _) = cell_at(&shown, drawer, "Ellipse Tool");
            let shapes = run_frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(at)]);
            let fills = fills_at(&shapes, at);
            assert!(fills.contains(&t.panel_darker), "{what}: a dimmed row's hover is a recess in the panel's darker fill: {fills:?}");
            assert!(!fills.contains(&t.hover), "{what}: not the lighter hover fill, which the dim text can't hold 4.5:1 over");
            let mut shown = vec![];
            shapes.iter().for_each(|s| texts(s, &mut shown));
            let (_, color) = cell_at(&shown, drawer, "Ellipse Tool");
            assert_eq!(color, t.text_dim, "{what}");
            assert!(
                contrast(color, t.panel_darker) >= 4.5,
                "{what}: a hovered dimmed tool's name must stay readable: {color:?} on {:?} is {:.2}:1",
                t.panel_darker,
                contrast(color, t.panel_darker)
            );
        }
    }
}

/// The drawer's headings end in the UI language's colon: in French a no-break space and a colon
/// (#7991), in English a plain colon.
#[test]
fn drawer_headings_use_the_language_colon() {
    let mut app = app();
    let ctx = ctx();
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    for (code, colon) in [("fr", "\u{a0}:"), ("en", ":")] {
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        crate::i18n::set_current(lang);
        frame(&mut app, &ctx, 0.0, vec![]);
        let shown = frame(&mut app, &ctx, 0.1, vec![]);
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let drawer = drawer_rect(&ctx).unwrap();
        let want = format!("{}{colon}", crate::i18n::tr(lang, "Select"));
        assert!(shown.iter().any(|(s, p, _)| s.trim() == want && drawer.contains(*p)), "{code}: {want:?}");
    }
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
    assert_eq!(slots(&app)[ellipse], ["ellipse", "circle", "curvatureCircle", "slice"]);
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
    assert_eq!(&slots(&app)[..3], [&["directSelection", "groupSelection"][..], &["selection"], &["lasso", "enclosingLasso", "magicWand"]]);
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
    let edit = crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().rfind(|r| bar.contains(r.center())).expect("its ••• button");
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
    let dim = Tokens::get(&ctx).text_dim;
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
    let grip = ctx.viewport(|vp| vp.prev_pass.widgets.get(crate::custom_toolbars::area_id("Mine").with("grip")).map(|w| w.sense));
    assert!(grip.is_some_and(|s| s.senses_drag() && !s.is_focusable()), "only the pointer drags the grip, so Tab does not stop on it: {grip:?}");
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

/// A toolbar made with New Toolbar has an Add Tool… button (+, above the ••• button) that the
/// keyboard reaches and opens with Enter. Its menu lists the tools under the drawer's headings: a
/// tool picked there with a click or with Enter is
/// added at the end of this toolbar alone, without the drawer, and is dimmed in the menu from then
/// on; picked under Remove Tool, it is taken out again.
#[test]
fn the_add_tool_menu_fills_a_new_toolbar_without_a_drag() {
    let mut app = app();
    let ctx = ctx();
    let shipped = slots(&app);
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let plus = || {
        let bar = ctx.memory(|m| m.area_rect(crate::custom_toolbars::area_id("Mine"))).expect("the toolbar is shown");
        let buttons: Vec<Rect> =
            crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().filter(|r| bar.contains(r.center())).collect();
        assert_eq!(buttons.len(), 2, "the + and ••• buttons: {buttons:?}");
        buttons[0]
    };
    let button = plus();
    let sense = ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect == button).map(|w| w.sense));
    assert!(sense.is_some_and(|s| s.is_focusable()), "Tab stops on the + button: {sense:?}");
    // The menu at `time`, opened with a click on the + button unless it is open.
    let open = |app: &mut VectorcraftApp, time: f64| {
        let shown = frame(app, &ctx, time, vec![]);
        if shown.iter().any(|(s, ..)| s == "Select:") {
            return shown;
        }
        click(app, &ctx, time + 0.2, plus().center());
        // The menu is sized in its first frame and drawn from the second.
        frame(app, &ctx, time + 0.5, vec![]);
        frame(app, &ctx, time + 0.6, vec![])
    };
    // The items named `name` that the menu shows (it scrolls), top first.
    let items = |shown: &[(String, Pos2, Color32)], name: &str| {
        let menu = ctx.memory(|m| m.area_rect(crate::custom_toolbars::add_menu_id("Mine"))).expect("the menu is open");
        let mut at: Vec<(Pos2, Color32)> = shown.iter().filter(|(s, p, _)| s.trim() == name && menu.contains(*p)).map(|(_, p, c)| (*p, *c)).collect();
        at.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        at
    };
    let mine = |app: &VectorcraftApp| crate::custom_toolbars::find(&app.ui, "Mine").unwrap().slots.clone();
    // The keyboard: Enter on the focused + button opens the menu, and Enter on a tool adds it.
    let focusable_at = |p: Pos2| {
        let id = ctx.viewport(|vp| {
            vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.contains(p) && w.sense.is_focusable()).last().map(|w| w.id)
        });
        id.unwrap_or_else(|| panic!("nothing focusable at {p:?}"))
    };
    let enter = |app: &mut VectorcraftApp, time: f64, at: Pos2| {
        let id = focusable_at(at);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(app, &ctx, time, vec![]);
        for (k, pressed) in [true, false].into_iter().enumerate() {
            let key = Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
            frame(app, &ctx, time + 0.1 * (k + 1) as f64, vec![key]);
        }
        frame(app, &ctx, time + 0.5, vec![]);
        frame(app, &ctx, time + 0.6, vec![])
    };
    let shown = enter(&mut app, 9.0, plus().center());
    assert!(items(&shown, "Remove Tool:").is_empty(), "nothing to take out of a blank toolbar");
    let lasso = items(&shown, "Lasso Tool");
    assert_eq!(lasso.len(), 1, "listed once, under Select");
    enter(&mut app, 10.0, lasso[0].0);
    assert_eq!(mine(&app), [["lasso"]]);
    assert_eq!(slots(&app), shipped, "the Tools panel's toolbar is untouched");
    assert!(!app.ui.tools_drawer, "no drawer opened");
    let list = app.run("window.toolbar.list", json!({})).unwrap();
    assert_eq!(
        list,
        json!([{"name": "Mine", "shown": true, "double": false, "slots": [["lasso"]], "floating": true, "side": "left", "order": null}])
    );
    // A second tool goes after the first.
    let shown = open(&mut app, 19.0);
    click(&mut app, &ctx, 20.0, items(&shown, "Magic Wand Tool")[0].0);
    assert_eq!(mine(&app), [["lasso"], ["magicWand"]]);
    // A tool in the toolbar is listed under Remove Tool, and dimmed under its heading, where a click
    // adds nothing.
    let shown = open(&mut app, 29.0);
    assert_eq!(items(&shown, "Remove Tool:").len(), 1);
    let lasso = items(&shown, "Lasso Tool");
    assert_eq!(lasso.len(), 2, "under Remove Tool, then under Select");
    let enabled = |at: Pos2| {
        let layer = egui::LayerId::new(egui::Order::Foreground, crate::custom_toolbars::add_menu_id("Mine"));
        ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .filter(|(l, _)| **l == layer)
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.contains(at))
                .last()
                .map(|w| w.enabled)
        })
    };
    assert_eq!([enabled(lasso[0].0), enabled(lasso[1].0)], [Some(true), Some(false)], "dimmed under Select");
    click(&mut app, &ctx, 30.0, lasso[1].0);
    assert_eq!(mine(&app), [["lasso"], ["magicWand"]]);
    // Under Remove Tool, a click takes it out.
    let shown = open(&mut app, 39.0);
    click(&mut app, &ctx, 40.0, items(&shown, "Lasso Tool")[0].0);
    assert_eq!(mine(&app), [["magicWand"]]);
    assert_eq!(slots(&app), shipped);
    // In French the menu's headings end in a no-break space and a colon (#7991).
    open(&mut app, 49.0);
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    frame(&mut app, &ctx, 49.7, vec![]);
    let french = frame(&mut app, &ctx, 49.8, vec![]);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    for heading in ["Remove Tool", "Select"] {
        let want = format!("{}\u{a0}:", crate::i18n::tr(fr, heading));
        assert_eq!(items(&french, &want).len(), 1, "{want:?}");
    }
}

/// A toolbar named Mine, made with New Toolbar, that holds 21 tools, drawn at (80, 40) in a window of
/// `size`; returns its + button.
fn long_toolbar(app: &mut VectorcraftApp, ctx: &egui::Context, size: egui::Vec2) -> Rect {
    app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    let tools = ["lasso", "magicWand", "ellipse", "polygon", "star", "pencil", "smooth", "eraser", "scissors", "knife", "paintbrush"];
    let tools = tools.iter().chain(&["blobBrush", "pen", "curvature", "width", "warp", "rotate", "scale", "shear", "zoom", "hand"]);
    let bar = crate::custom_toolbars::find_mut(&mut app.ui, "Mine").unwrap();
    bar.slots = tools.map(|t| vec![t.to_string()]).collect();
    bar.pos = [80.0, 40.0];
    run_frame_in(app, ctx, size, 0.0, vec![]);
    run_frame_in(app, ctx, size, 0.1, vec![]);
    let area = ctx.memory(|m| m.area_rect(crate::custom_toolbars::area_id("Mine"))).expect("the toolbar is shown");
    let buttons: Vec<Rect> = crate::toolbar::tests::widget_rects(ctx, vec2(26.0, 26.0)).into_iter().filter(|r| area.contains(r.center())).collect();
    assert_eq!(buttons.len(), 2, "the + and ••• buttons: {buttons:?}");
    buttons[0]
}

/// Opens the Add Tool… menu from `plus` in a window of `size`, then moves the pointer down its rows
/// for 16 frames; returns where the menu opened and, for each of those frames, whether it was open
/// and where it was.
fn point_down_the_add_menu(app: &mut VectorcraftApp, ctx: &egui::Context, size: egui::Vec2, plus: Rect) -> (Rect, Vec<(bool, Option<Rect>)>) {
    run_frame_in(app, ctx, size, 1.0, vec![Event::PointerMoved(plus.center()), button(plus.center(), true)]);
    run_frame_in(app, ctx, size, 1.05, vec![button(plus.center(), false)]);
    run_frame_in(app, ctx, size, 1.5, vec![]);
    run_frame_in(app, ctx, size, 1.6, vec![]);
    let id = crate::custom_toolbars::add_menu_id("Mine");
    let menu = || ctx.memory(|m| m.area_rect(id));
    let opened = menu().expect("the menu is open");
    let mut seen = vec![];
    for k in 0..16u8 {
        let at = pos2(opened.center().x, opened.top() + 12.0 + f32::from(k) * 20.0);
        run_frame_in(app, ctx, size, 2.0 + 0.05 * f64::from(k), vec![Event::PointerMoved(at)]);
        seen.push((egui::Popup::is_id_open(ctx, id), menu()));
    }
    (opened, seen)
}

/// On a toolbar made with New Toolbar that holds many tools, the + button sits low in the window,
/// with less room under it than the Add Tool… menu needs. The menu then opens above the button and
/// stays where it opened, as tall as it opened, while the pointer moves down its rows.
#[test]
fn the_add_tool_menu_stays_put_under_a_long_toolbar() {
    let mut app = app();
    let ctx = ctx();
    let size = vec2(1000.0, 800.0);
    let plus = long_toolbar(&mut app, &ctx, size);
    assert!(plus.bottom() > size.y * 0.6, "the + button is low in the window: {plus:?}");
    let (opened, seen) = point_down_the_add_menu(&mut app, &ctx, size, plus);
    assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(opened), "the menu at {opened:?} is inside the window");
    assert!(opened.bottom() <= plus.top(), "the menu at {opened:?} opens above the + button at {plus:?}");
    assert!(seen.iter().all(|s| *s == (true, Some(opened))), "opened at {opened:?}, then {seen:?}");
}

/// With the + button of the same toolbar in the middle of a taller window, the Add Tool… menu opens
/// below the button and stays there, as tall as it opened, while the pointer moves down its rows.
#[test]
fn the_add_tool_menu_stays_put_below_a_button_mid_window() {
    let mut app = app();
    let ctx = ctx();
    let size = vec2(1000.0, 1600.0);
    let plus = long_toolbar(&mut app, &ctx, size);
    assert!((size.y * 0.3..size.y * 0.7).contains(&plus.center().y), "the + button is in the middle of the window: {plus:?}");
    let (opened, seen) = point_down_the_add_menu(&mut app, &ctx, size, plus);
    assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(opened), "the menu at {opened:?} is inside the window");
    assert!(opened.top() >= plus.bottom(), "the menu at {opened:?} opens below the + button at {plus:?}");
    assert!(seen.iter().all(|s| *s == (true, Some(opened))), "opened at {opened:?}, then {seen:?}");
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
        crate::toolbar::tests::widget_rects(&ctx, vec2(26.0, 26.0)).into_iter().rfind(|r| bar.contains(r.center())).unwrap()
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

/// The Trim at Crossings Tool is in the Scissors tool's flyout in both toolbars, after the Knife,
/// once in the All Tools drawer and in Keyboard Shortcuts' list, with no shortcut; Reset Toolbar
/// keeps it there.
#[test]
fn trim_at_crossings_is_in_the_scissors_flyout() {
    let mut app = app();
    let trim = vectorcraft_tools::addons::trim::ID;
    for advanced in [false, true] {
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        let s = slots(&app);
        let slot = slot_of(&app, "scissors").map(|i| s[i].clone()).expect("a Scissors slot");
        let knife = slot.iter().position(|id| *id == "knife").expect("the Knife beside the Scissors");
        assert_eq!(slot.get(knife + 1), Some(&trim), "advanced: {advanced}");
        assert_eq!(s.iter().filter(|ids| ids.contains(&trim)).count(), 1, "advanced: {advanced}");
    }
    assert_eq!(sections().iter().flat_map(|(_, tools)| tools.iter()).filter(|t| t.id == trim).count(), 1, "listed once in the drawer");
    assert_eq!(vectorcraft_tools::catalog::all_tools().filter(|t| t.id == trim).count(), 1, "listed once for Keyboard Shortcuts");
    assert_eq!(vectorcraft_tools::tool_info(trim).map(|t| (t.label, t.shortcut)), Some(("Trim at Crossings Tool", None)));
    app.run("tool.select", json!({"tool": trim})).unwrap();
    assert_eq!(app.session.tool_id(), trim);
    let shown = slot_of(&app, trim).map(|i| crate::toolbar::shown_tool(&app, &slots(&app)[i]));
    assert_eq!(shown, Some(trim), "the slot's button shows the tool once chosen");
    app.run("toolbar.removeTool", json!({"tool": trim})).unwrap();
    assert!(slot_of(&app, trim).is_none(), "it can be taken out");
    app.run("toolbar.reset", json!({})).unwrap();
    let after_reset = slot_of(&app, "scissors").map(|i| slots(&app)[i].contains(&trim));
    assert_eq!(after_reset, Some(true), "Reset Toolbar puts it back");
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
