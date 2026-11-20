//! Search belongs to the real All Tools drawer, independently of global Command Palette search.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::accesskit::{Action, Node, NodeId, Role};
use egui::{Event, Pos2, RawInput, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

struct Observed {
    nodes: Vec<(NodeId, Node)>,
    text: Vec<(String, Pos2)>,
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Observed {
    fn texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Pos2)>) {
        match shape {
            Shape::Text(text) => {
                let at = text.pos + vec2(4.0, text.galley.size().y / 2.0);
                if clip.contains(at) {
                    out.push((text.galley.text().to_string(), at));
                }
            }
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, clip, out)),
            _ => {}
        }
    }
    let mut raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1200.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut output = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    let mut text = Vec::new();
    for clipped in &output.shapes {
        texts(&clipped.shape, clipped.clip_rect, &mut text);
    }
    let nodes = output.platform_output.accesskit_update.map(|update| update.nodes).unwrap_or_default();
    output.textures_delta.clear();
    Observed { nodes, text }
}

fn inside(node: &Node, area: Rect) -> bool {
    node.bounds().is_some_and(|bounds| {
        let rect = Rect::from_min_max(egui::pos2(bounds.x0 as f32, bounds.y0 as f32), egui::pos2(bounds.x1 as f32, bounds.y1 as f32));
        area.contains_rect(rect)
    })
}

fn search_name(node: &Node, nodes: &[(NodeId, Node)]) -> bool {
    let search = |name: &str| name.to_lowercase().contains("search");
    node.label().is_some_and(search)
        || node.placeholder().is_some_and(search)
        || node.labelled_by().iter().any(|label| {
            nodes.iter().find(|(id, _)| id == label).is_some_and(|(_, label)| label.label().is_some_and(search) || label.value().is_some_and(search))
        })
}

fn toolbar_contents(app: &VectorcraftApp) -> serde_json::Value {
    json!({"advanced": app.ui.toolbar_advanced, "edits": app.ui.toolbar_edits,
        "custom": app.ui.custom_toolbars, "show": app.ui.toolbar_show})
}

#[test]
fn all_tools_drawer_exposes_a_named_editable_search_inside_its_own_bounds() {
    let _language = ui_guard();
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 320, "height": 240})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("shape.rectangle", json!({"x": 120, "y": 80, "width": 40, "height": 30})).unwrap();
    app.select_tool("selection");
    app.ui.toolbar_advanced = false;
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..8 {
        let _ = frame(&mut app, &ctx, vec![]);
    }
    let state = app.session.active().unwrap();
    let (uid, document, selection, revision) = (state.uid, state.doc.clone(), state.selection.clone(), state.revision);
    assert_eq!(selection.objects.len(), 1, "the fixture has a real selected object");
    let history = (state.history.undo.len(), state.history.redo.len());
    let paint = app.session.paint.clone();
    let toolbar = toolbar_contents(&app);
    assert!(!app.ui.palette_open, "global Command Palette search is closed");

    // The existing public command opens the same actual drawer as the toolbar's Edit Toolbar button.
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    for _ in 0..4 {
        let _ = frame(&mut app, &ctx, vec![]);
    }
    let observed = frame(&mut app, &ctx, vec![]);
    let area = ctx.memory(|memory| memory.area_rect(egui::Id::unique("tools-drawer"))).expect("the actual All Tools drawer area");
    assert!(app.ui.tools_drawer && app.ui.drawer_toolbar.is_none());
    assert!(ctx.memory(|memory| memory.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("tools-drawer")))));
    assert!(observed.text.iter().any(|(text, at)| text == "All Tools" && area.contains(*at)), "the actual title is painted inside the drawer");
    assert!(
        observed.nodes.iter().any(|(_, node)| node.role() == Role::Button && node.label() == Some("Drawer menu") && inside(node, area)),
        "the real drawer menu is present"
    );
    for name in ["Selection Tool", "Direct Selection Tool"] {
        assert!(
            observed.text.iter().any(|(text, at)| text == name && area.contains(*at)),
            "an existing tool row is really drawn in this drawer: {name}"
        );
    }
    let state = app.session.active().unwrap();
    assert_eq!(state.uid, uid);
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!(state.revision, revision);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    assert_eq!(app.session.paint, paint);
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(toolbar_contents(&app), toolbar);
    assert!(!app.ui.palette_open, "a global search field must not satisfy this drawer's request");

    let search = observed.nodes.iter().any(|(_, node)| {
        node.role() == Role::TextInput
            && !node.is_disabled()
            && !node.is_read_only()
            && node.supports_action(Action::SetValue)
            && inside(node, area)
            && search_name(node, &observed.nodes)
    });
    assert!(
        search,
        "All Tools must contain its own named editable Search input inside the actual tools-drawer bounds; global Command Palette/App bar search is excluded"
    );
}

use egui::{Color32, Modifiers, PointerButton};

static UI_LANGUAGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct LanguageGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: vectorcraft_ui_egui::i18n::Lang,
}

impl Drop for LanguageGuard {
    fn drop(&mut self) {
        vectorcraft_ui_egui::i18n::set_current(self.previous);
    }
}

fn ui_guard() -> LanguageGuard {
    let lock = UI_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = vectorcraft_ui_egui::i18n::current();
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    LanguageGuard { _lock: lock, previous }
}

struct Details {
    nodes: Vec<(NodeId, Node)>,
    text: Vec<(String, Pos2, Color32)>,
    rectangles: Vec<(Rect, f32)>,
}

/// The same real App frame as the before case, retaining painted colors/rectangles for its
/// existing dimmed-row and Learn-mark contracts, and allowing a normal narrow window.
fn draw(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2, events: Vec<Event>) -> Details {
    fn collect(shape: &Shape, clip: Rect, out: &mut Details) {
        match shape {
            Shape::Text(text) => {
                let at = text.pos + vec2(4.0, text.galley.size().y / 2.0);
                if clip.contains(at) {
                    let color = text.galley.job.sections.first().map_or(text.fallback_color, |section| section.format.color);
                    out.text.push((text.galley.text().to_string(), at, color));
                }
            }
            Shape::Rect(rectangle) if clip.intersects(rectangle.rect) => out.rectangles.push((rectangle.rect, rectangle.stroke.width)),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, out)),
            _ => {}
        }
    }
    let mut raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut output = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    let mut details = Details { nodes: Vec::new(), text: Vec::new(), rectangles: Vec::new() };
    output.shapes.iter().for_each(|clipped| collect(&clipped.shape, clipped.clip_rect, &mut details));
    details.nodes = output.platform_output.accesskit_update.map(|update| update.nodes).unwrap_or_default();
    output.textures_delta.clear();
    details
}

fn settle(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2) -> Details {
    for _ in 0..3 {
        let _ = draw(app, ctx, screen, vec![]);
    }
    draw(app, ctx, screen, vec![])
}

fn area(ctx: &egui::Context) -> Rect {
    ctx.memory(|memory| memory.area_rect(egui::Id::unique("tools-drawer"))).expect("real drawer bounds")
}

fn center(node: &Node) -> Pos2 {
    let b = node.bounds().unwrap();
    egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32)
}

fn named_control(details: &Details, bounds: Rect, name: &str) -> Pos2 {
    center(
        &details
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name) && inside(node, bounds))
            .unwrap_or_else(|| panic!("actual control {name}"))
            .1,
    )
}

fn search_node(details: &Details, bounds: Rect) -> &Node {
    let name = vectorcraft_ui_egui::i18n::t("Search");
    &details
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::TextInput && inside(node, bounds) && (node.label() == Some(name) || node.placeholder() == Some(name)))
        .expect("the actual drawer search field")
        .1
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2, at: Pos2) {
    let _ = draw(app, ctx, screen, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at)]);
    for pressed in [true, false] {
        let _ = draw(app, ctx, screen, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    let _ = settle(app, ctx, screen);
}

fn key(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2, key: egui::Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        let _ = draw(
            app,
            ctx,
            screen,
            vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }],
        );
    }
}

fn query(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2, text: &str) -> Details {
    let shown = settle(app, ctx, screen);
    click(app, ctx, screen, center(search_node(&shown, area(ctx))));
    key(app, ctx, screen, egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE });
    // Filtering is live: no Enter, tool command, direct query-state injection or dialog callback.
    let _ = draw(app, ctx, screen, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text(text.into())]);
    settle(app, ctx, screen)
}

fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, screen: egui::Vec2, from: Pos2, to: Pos2) {
    let _ = draw(
        app,
        ctx,
        screen,
        vec![
            Event::ModifiersChanged(Modifiers::NONE),
            Event::PointerMoved(from),
            Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        ],
    );
    for n in 1..=6 {
        let _ = draw(app, ctx, screen, vec![Event::PointerMoved(from + (to - from) * (n as f32 / 6.0))]);
    }
    let _ =
        draw(app, ctx, screen, vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    let _ = settle(app, ctx, screen);
}

fn row(details: &Details, bounds: Rect, name: &str) -> (Pos2, Color32) {
    details
        .text
        .iter()
        .find(|(text, at, _)| text == name && bounds.contains(*at))
        .map(|(_, at, color)| (*at, *color))
        .unwrap_or_else(|| panic!("actual visible tool row {name}"))
}

fn visible_tools(details: &Details, bounds: Rect) -> Vec<String> {
    let labels: Vec<_> = vectorcraft_tools::catalog::all_tools().map(|tool| vectorcraft_ui_egui::i18n::t(tool.label)).collect();
    details.text.iter().filter(|(text, at, _)| bounds.contains(*at) && labels.contains(&text.as_str())).map(|(text, _, _)| text.clone()).collect()
}

fn inert(app: &VectorcraftApp) -> (serde_json::Value, vectorcraft_tools::PaintDefaults) {
    let state = app.session.active().unwrap();
    (
        json!({"uid": state.uid, "doc": state.doc, "selection": state.selection, "revision": state.revision,
        "undo": state.history.undo.len(), "redo": state.history.redo.len()}),
        app.session.paint.clone(),
    )
}

fn fixture(screen: egui::Vec2) -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 320, "height": 240})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("shape.rectangle", json!({"x": 120, "y": 80, "width": 40, "height": 30})).unwrap();
    app.select_tool("selection");
    app.session.prefs.interface_language = "en".into();
    app.ui.toolbar_advanced = false;
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let _ = settle(&mut app, &ctx, screen);
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    let _ = settle(&mut app, &ctx, screen);
    (app, ctx)
}

#[test]
fn live_filter_clear_no_matches_and_selection_keep_the_existing_drawer_contracts() {
    let _language = ui_guard();
    let screen = vec2(1600.0, 1200.0);
    let (mut app, ctx) = fixture(screen);
    let initial = settle(&mut app, &ctx, screen);
    let original_rows = visible_tools(&initial, area(&ctx));
    let original = inert(&app);
    let toolbar = toolbar_contents(&app);
    let shown = query(&mut app, &ctx, screen, "pEn");
    let _ = row(&shown, area(&ctx), "Pen Tool");
    let _ = row(&shown, area(&ctx), "Pencil Tool");
    assert!(!shown.text.iter().any(|(text, at, _)| area(&ctx).contains(*at) && (text == "Selection Tool" || text == "Select:")));
    assert_eq!(inert(&app), original);
    assert_eq!(app.session.tool_id(), "selection", "typed tool letters belong to Search");
    assert_eq!(toolbar_contents(&app), toolbar);
    let shown = query(&mut app, &ctx, screen, "qzx-no-tool");
    assert!(visible_tools(&shown, area(&ctx)).is_empty());
    assert!(shown.text.iter().any(|(text, at, _)| text == "No matches." && area(&ctx).contains(*at)));
    assert!(shown.text.iter().any(|(text, at, _)| text == "Show:" && area(&ctx).contains(*at)));
    let show = app.ui.toolbar_show.coloring;
    click(&mut app, &ctx, screen, named_control(&shown, area(&ctx), "Coloring Controls"));
    assert_eq!(app.ui.toolbar_show.coloring, !show);
    let shown = settle(&mut app, &ctx, screen);
    click(&mut app, &ctx, screen, named_control(&shown, area(&ctx), "Coloring Controls"));
    let shown = settle(&mut app, &ctx, screen);
    click(&mut app, &ctx, screen, named_control(&shown, area(&ctx), "Clear"));
    let shown = settle(&mut app, &ctx, screen);
    assert_eq!(visible_tools(&shown, area(&ctx)), original_rows, "empty query restores today's complete visible order");
    assert_eq!(inert(&app), original);
    assert_eq!(toolbar_contents(&app), toolbar);
    let shown = query(&mut app, &ctx, screen, "pen");
    click(&mut app, &ctx, screen, named_control(&shown, area(&ctx), "Drawer menu"));
    let menu = settle(&mut app, &ctx, screen);
    for name in ["Basic", "Advanced", "Reset"] {
        assert!(menu.text.iter().any(|(text, _, _)| text.trim().trim_start_matches('✓').trim() == name), "actual menu item {name}");
    }
    key(&mut app, &ctx, screen, egui::Key::Escape, Modifiers::NONE);
    assert!(!app.ui.tools_drawer, "Escape preserves the existing drawer-first close contract with its menu open");
    assert_eq!(inert(&app), original);
    assert_eq!(toolbar_contents(&app), toolbar);
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    let shown = settle(&mut app, &ctx, screen);
    assert_eq!(search_node(&shown, area(&ctx)).value(), Some(""));
    assert_eq!(visible_tools(&shown, area(&ctx)), original_rows);
    let _ = query(&mut app, &ctx, screen, "pen");
    key(&mut app, &ctx, screen, egui::Key::Escape, Modifiers::NONE);
    assert!(!app.ui.tools_drawer, "Escape also closes the drawer while Search owns text input");
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    let shown = settle(&mut app, &ctx, screen);
    assert_eq!(search_node(&shown, area(&ctx)).value(), Some(""));
    assert_eq!(visible_tools(&shown, area(&ctx)), original_rows);
    assert!(serde_json::to_value(&app.ui).unwrap().get("tools_drawer_query").is_none(), "query is not a persisted preference");
    let shown = query(&mut app, &ctx, screen, "rectangle");
    let at = row(&shown, area(&ctx), "Rectangle Tool").0;
    click(&mut app, &ctx, screen, at);
    assert_eq!(app.session.tool_id(), "rectangle", "an installed/dimmed matching tool stays clickable");
    assert!(!app.ui.tools_drawer);
    assert_eq!(inert(&app), original);
    assert_eq!(toolbar_contents(&app), toolbar);
    app.select_tool("selection");
    app.run("window.editToolbar", json!({"open": true})).unwrap();
    let _ = settle(&mut app, &ctx, screen);
    let outside = app.canvas_rect.unwrap().right_bottom() - vec2(40.0, 40.0);
    assert!(!area(&ctx).contains(outside));
    click(&mut app, &ctx, screen, outside);
    assert!(!app.ui.tools_drawer, "an ordinary outside click still closes the drawer");
}

#[test]
fn localized_visible_name_filters_in_a_bounded_narrow_drawer() {
    let _language = ui_guard();
    let screen = vec2(800.0, 700.0);
    let (mut app, ctx) = fixture(screen);
    let lang = vectorcraft_ui_egui::i18n::Lang::from_code("de").unwrap();
    app.session.prefs.interface_language = "de".into();
    let _ = settle(&mut app, &ctx, screen);
    let label = vectorcraft_ui_egui::i18n::tr(lang, "Rectangle Tool").to_string();
    assert_ne!(label, "Rectangle Tool", "this control really uses a translated visible name");
    let before = inert(&app);
    let toolbar = toolbar_contents(&app);
    let shown = query(&mut app, &ctx, screen, &label.to_uppercase());
    let matches = visible_tools(&shown, area(&ctx));
    assert!(matches.contains(&label));
    assert!(
        matches.iter().all(|name| name.to_lowercase().contains(&label.to_lowercase())),
        "literal matching may also retain a longer visible name containing the query"
    );
    assert!(Rect::from_min_size(Pos2::ZERO, screen).expand(1.0).contains_rect(area(&ctx)));
    assert!(area(&ctx).width() <= 320.0, "the search does not widen the bounded drawer");
    let input = search_node(&shown, area(&ctx));
    assert!(inside(input, area(&ctx)));
    assert_eq!(inert(&app), before);
    assert_eq!(toolbar_contents(&app), toolbar);
    if let Some(dir) = std::env::var_os("VECTORCRAFT_TOOLS_SEARCH_EVIDENCE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bounds = area(&ctx);
        let rows: Vec<_> = shown
            .text
            .iter()
            .filter(|(_, at, _)| bounds.contains(*at))
            .map(|(text, at, color)| json!({"text": text, "at": [at.x,at.y], "color": color.to_array()}))
            .collect();
        std::fs::write(
            dir.join("localized-drawer.json"),
            serde_json::to_vec_pretty(&json!({"language": "de", "screen": [screen.x,screen.y],
            "drawer": [bounds.min.x,bounds.min.y,bounds.max.x,bounds.max.y], "search_bounds": format!("{:?}", input.bounds()), "painted": rows}))
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn filtered_basic_and_advanced_rows_keep_drag_ids_and_installed_drag_restriction() {
    let _language = ui_guard();
    let screen = vec2(1600.0, 1200.0);
    for advanced in [false, true] {
        let (mut app, ctx) = fixture(screen);
        if advanced {
            app.run("window.toolbarAdvanced", json!({})).unwrap();
        }
        app.run("toolbar.removeTool", json!({"tool": "artboard"})).unwrap();
        app.run("window.editToolbar", json!({"open": true})).unwrap();
        let original = inert(&app);
        let contents = toolbar_contents(&app);
        let shown = query(&mut app, &ctx, screen, "artboard");
        assert_eq!(toolbar_contents(&app), contents);
        let (from, normal) = row(&shown, area(&ctx), "Artboard Tool");
        let target = shown
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == Role::Button && node.label().is_some_and(|label| label.starts_with("Selection Tool")) && !inside(node, area(&ctx))
            })
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let to = egui::pos2(((target.x0 + target.x1) * 0.5) as f32, target.y1 as f32 - 2.0);
        drag(&mut app, &ctx, screen, from, to);
        let edits = if advanced { &app.ui.toolbar_edits.advanced } else { &app.ui.toolbar_edits.basic };
        let count = edits.as_ref().unwrap().iter().flat_map(|category| &category.slots).flatten().filter(|id| id.as_str() == "artboard").count();
        assert_eq!(count, 1, "the original matching ToolInfo id reaches the correct toolbar");
        assert_eq!(inert(&app), original);
        assert_eq!(app.session.tool_id(), "selection", "dragging is not row activation");
        let shown = settle(&mut app, &ctx, screen);
        let (from, dimmed) = row(&shown, area(&ctx), "Artboard Tool");
        assert_ne!(normal, dimmed, "installed tools keep the existing dimmed appearance");
        let added = toolbar_contents(&app);
        drag(&mut app, &ctx, screen, from, to);
        assert_eq!(toolbar_contents(&app), added, "a dimmed installed row cannot be dragged in again");
        assert_eq!(inert(&app), original);
    }
}

#[test]
fn filtered_custom_toolbar_drop_uses_its_target_and_remains_clickable_when_installed() {
    let _language = ui_guard();
    let screen = vec2(1600.0, 1200.0);
    let (mut app, ctx) = fixture(screen);
    app.run("window.toolbar.new", json!({"name": "Search fixture"})).unwrap();
    app.run("window.editToolbar", json!({"open": true, "toolbar": "Search fixture"})).unwrap();
    let _ = settle(&mut app, &ctx, screen);
    let main = app.ui.toolbar_edits.clone();
    let original = inert(&app);
    let shown = query(&mut app, &ctx, screen, "ellipse");
    let (from, normal) = row(&shown, area(&ctx), "Ellipse Tool");
    let bar_id = egui::Id::unique(("custom-toolbar", "search fixture"));
    let bar = ctx.memory(|memory| memory.area_rect(bar_id)).expect("the actual named custom toolbar");
    drag(&mut app, &ctx, screen, from, bar.center());
    let slots = &app.ui.custom_toolbars.iter().find(|bar| bar.name == "Search fixture").unwrap().slots;
    assert_eq!(slots, &[vec![String::from("ellipse")]]);
    assert_eq!(app.ui.toolbar_edits, main, "the Tools panel's toolbar was not the drop target");
    assert_eq!(inert(&app), original);
    let shown = settle(&mut app, &ctx, screen);
    let (at, dimmed) = row(&shown, area(&ctx), "Ellipse Tool");
    assert_ne!(normal, dimmed, "only the toolbar the drawer edits decides installed dimming");
    assert!(!shown.text.iter().any(|(text, at, _)| text == "Show:" && area(&ctx).contains(*at)));
    click(&mut app, &ctx, screen, at);
    assert_eq!(app.session.tool_id(), "ellipse");
    assert!(!app.ui.tools_drawer);
    assert_eq!(inert(&app), original);
    assert_eq!(app.ui.toolbar_edits, main);
}

#[test]
fn learn_reveal_clears_a_stale_filter_and_marks_the_real_target_row() {
    let _language = ui_guard();
    let screen = vec2(1600.0, 1200.0);
    let (mut app, ctx) = fixture(screen);
    app.run("toolbar.removeTool", json!({"tool": "artboard"})).unwrap();
    let before = inert(&app);
    let contents = toolbar_contents(&app);
    let shown = query(&mut app, &ctx, screen, "pen");
    assert!(!visible_tools(&shown, area(&ctx)).iter().any(|name| name == "Artboard Tool"));
    let result = app.run("learn.locateTool", json!({"tool": "artboard"})).unwrap();
    assert_eq!(result["location"], "allTools");
    for _ in 0..12 {
        let _ = draw(&mut app, &ctx, screen, vec![]);
    }
    let shown = settle(&mut app, &ctx, screen);
    assert_eq!(search_node(&shown, area(&ctx)).value(), Some(""));
    let (at, _) = row(&shown, area(&ctx), "Artboard Tool");
    assert_eq!(app.ui.learn_locate.as_ref().unwrap().tool, "artboard");
    assert!(
        shown.rectangles.iter().any(|(rect, width)| *width == 2.0
            && rect.height() >= 20.0
            && rect.height() <= 28.0
            && rect.width() > 100.0
            && rect.contains(at)),
        "the actual Learn outline marks the revealed row"
    );
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(inert(&app), before);
    assert_eq!(toolbar_contents(&app), contents);
}

/// The positive lowercase query must reach a real painted row before the uppercase
/// assertion can diagnose matching. All input goes through the existing App fixture.
fn assert_localized_case_query(locale: &str, english: &str, lowercase: &str, uppercase: &str) {
    let screen = vec2(1000.0, 800.0);
    let (mut app, ctx) = fixture(screen);
    let lang = vectorcraft_ui_egui::i18n::Lang::from_code(locale).unwrap();
    app.session.prefs.interface_language = locale.into();
    let _ = settle(&mut app, &ctx, screen);
    assert_eq!(vectorcraft_ui_egui::i18n::current().code(), locale);
    let label = vectorcraft_ui_egui::i18n::tr(lang, english).to_string();
    assert_ne!(label, english, "this fixture uses a real translated catalog row");
    let before = inert(&app);
    let toolbar = toolbar_contents(&app);
    let shown = query(&mut app, &ctx, screen, lowercase);
    assert!(visible_tools(&shown, area(&ctx)).contains(&label), "{locale}: lowercase positive control {label}");
    assert_eq!(app.ui.tools_drawer_query, lowercase, "real lowercase text input reached Search");
    assert_eq!(inert(&app), before);
    assert_eq!(toolbar_contents(&app), toolbar);
    assert_eq!(app.session.tool_id(), "selection");
    let shown = query(&mut app, &ctx, screen, uppercase);
    assert_eq!(app.ui.tools_drawer_query, uppercase, "real uppercase text input reached Search");
    assert_eq!(inert(&app), before);
    assert_eq!(toolbar_contents(&app), toolbar);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(!app.ui.palette_open, "this is the drawer's own Search");
    assert!(inside(search_node(&shown, area(&ctx)), area(&ctx)));
    assert!(Rect::from_min_size(Pos2::ZERO, screen).expand(1.0).contains_rect(area(&ctx)));
    let visible = visible_tools(&shown, area(&ctx));
    assert!(visible.contains(&label), "{locale}: uppercase {uppercase:?} must retain {label:?}; actual rows: {visible:?}");
}

#[test]
fn localized_tool_search_turkish_case_and_canonical_text() {
    let _language = ui_guard();
    // ARACI is broad: Selection is the first matching row, while Width may be offscreen.
    assert_localized_case_query("tr", "Selection Tool", "aracı", "ARACI");
    assert_localized_case_query("tr", "Width Tool", "genişlik aracı", "GENİŞLİK ARACI");
    assert_localized_case_query("tr", "Width Tool", "genişlik aracı", "GENI\u{307}ŞLI\u{307}K ARACI");
}

#[test]
fn localized_tool_search_greek_final_and_medial_sigma() {
    let _language = ui_guard();
    assert_localized_case_query("el", "Add Anchor Point Tool", "προσ", "ΠΡΟΣ");
}

#[test]
fn localized_tool_search_full_fold_keeps_georgian_and_german_expansion() {
    let _language = ui_guard();
    // Georgian uppercase is an existing-behavior control before the ß/ss falsifier.
    let lang = vectorcraft_ui_egui::i18n::Lang::from_code("ka").unwrap();
    let label = vectorcraft_ui_egui::i18n::tr(lang, "Width Tool").to_string();
    assert_localized_case_query("ka", "Width Tool", &label, &label.to_uppercase());
    assert_localized_case_query("de", "Dimension Tool", "bemaßungswerkzeug", "BEMASSUNGSWERKZEUG");
}
