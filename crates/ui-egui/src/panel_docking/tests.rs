//! The dock area as the user sees it: whole-window frames driven by pointer and key events.

use craft_ui::docking::Hit;
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use vectorcraft_engine::Session;

use super::*;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 1000.0));

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    cursor: egui::CursorIcon,
    nodes: Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>,
}

impl Harness {
    /// Create a blank document and settle the initial docking layout over three frames.
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut h = Self { app, ctx: egui::Context::default(), time: 0.0, cursor: egui::CursorIcon::Default, nodes: Vec::new() };
        for _ in 0..3 {
            h.frame(vec![]);
        }
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let input = egui::RawInput { time: Some(self.time), events, screen_rect: Some(SCREEN), ..Default::default() };
        self.run(input);
    }

    /// A frame whose input first goes through [`VectorcraftApp::release_lost_buttons`], with the
    /// system reporting a mouse button down or not.
    fn frame_with_system_buttons(&mut self, events: Vec<Event>, system_held: bool) {
        self.time += 0.05;
        let mut input = egui::RawInput { time: Some(self.time), events, screen_rect: Some(SCREEN), ..Default::default() };
        self.app.release_lost_buttons(&self.ctx, &mut input, system_held);
        self.run(input);
    }

    /// Draw one application frame and retain its cursor and accessibility output for assertions.
    fn run(&mut self, input: egui::RawInput) {
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        self.cursor = out.platform_output.cursor_icon;
        self.nodes = out.platform_output.accesskit_update.map_or_else(Vec::new, |update| update.nodes);
    }

    fn hit(&self, hit: &Hit<String>) -> Option<Rect> {
        hit_rects(&self.ctx).into_iter().find(|(h, _)| h == hit).map(|(_, r)| r)
    }

    fn tab(&self, panel: &str) -> Option<Rect> {
        self.hit(&Hit::Tab(panel.into()))
    }

    fn click(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at), button(at, true)]);
        self.frame(vec![button(at, false)]);
        self.frame(vec![]);
    }

    fn press(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        self.frame(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
    }

    /// Press at `from` and move to `to` in steps, holding the button.
    fn press_and_move(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from), button(from, true)]);
        for k in 1..=6 {
            let at = from + (to - from) * (k as f32 / 6.0);
            self.frame(vec![Event::PointerMoved(at)]);
        }
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.press_and_move(from, to);
        self.frame(vec![button(to, false)]);
        self.frame(vec![]);
        self.frame(vec![]);
    }

    fn layout(&self) -> Layout<String> {
        layout_of(&self.app.ui)
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn main_panels(layout: &Layout<String>) -> Vec<String> {
    dock_columns(layout).first().map(|c| c.groups.iter().flat_map(|g| g.panels.clone()).collect()).unwrap_or_default()
}

#[test]
fn the_default_layout_is_the_canvas_beside_the_main_column_and_the_icon_column() {
    let mut h = Harness::new();
    let layout = h.layout();
    assert!(h.app.ui.docking.is_none(), "an untouched workspace has no saved layout");
    let columns = dock_columns(&layout);
    assert_eq!(main_panels(&layout), ["properties", "layers", "libraries"]);
    let icons = columns.get(1).expect("an icon column");
    assert!(icons.iconic && !columns[0].iconic);
    assert_eq!(icons.groups.len(), ICON_PANEL_GROUPS.len());
    let properties = h.tab("properties").expect("the main column's tabs are drawn");
    let swatches = h.hit(&Hit::Icon("swatches".into())).expect("the icon column is drawn");
    assert!(swatches.right() <= properties.left(), "the icon column is inside the main column");
    // The canvas takes the rest and gets the pointer: the Rectangle tool draws on it.
    let canvas = h.app.canvas_rect.expect("the canvas is drawn");
    assert!(canvas.right() <= swatches.left() + 1.0 && canvas.width() > 800.0);
    h.app.select_tool("rectangle");
    let count = |h: &Harness| h.app.session.active().unwrap().doc.node_count();
    let before = count(&h);
    h.drag(canvas.center(), canvas.center() + vec2(60.0, 40.0));
    assert_eq!(count(&h), before + 1);
}

#[test]
fn an_icon_opens_its_flyout_and_open_panel_follows() {
    let mut h = Harness::new();
    let icon = h.hit(&Hit::Icon("swatches".into())).unwrap();
    h.click(icon.center());
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("swatches"));
    assert!(h.tab("swatches").is_some(), "the flyout shows its group's tabs");
    // Window › Brushes switches the flyout on the next frame.
    h.app.run("window.panel", json!({"panel": "brushes"})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()).as_deref(), Some("brushes"));
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("brushes"));
    h.app.run("window.panel", json!({"panel": "brushes"})).unwrap();
    h.frame(vec![]);
    assert_eq!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()), None);
}

/// #5713: real Glyphs and Artboards bodies must retain a flyout resize through
/// content fitting, closing/reopening and serialization of the workspace layout.
#[test]
fn glyphs_and_artboards_flyouts_keep_user_sizes() {
    for panel in ["glyphs", "artboards"] {
        let mut h = Harness::new();
        let icon = h.hit(&Hit::Icon(panel.into())).unwrap().center();
        h.click(icon);
        // Glyphs reserves space from the available height, so its untouched auto-fit
        // converges over several frames before the first user resize.
        for _ in 0..30 {
            h.frame(vec![]);
        }
        // Area memory can include a panel body's minimum allocation beyond its clip.
        // The frame's actual edge widgets describe the visible, resizable rectangle.
        let rect = |h: &Harness| {
            let id = area_id().with("flyout");
            let left = h.ctx.read_response(id.with("resize-left")).unwrap().rect;
            let bottom = h.ctx.read_response(id.with("resize-bottom")).unwrap().rect;
            Rect::from_min_max(pos2(left.left(), left.top() - 12.0), pos2(bottom.right() + 12.0, bottom.bottom()))
        };
        let before = rect(&h);
        for (at, cursor) in [
            (before.left_center() + vec2(2.0, 0.0), egui::CursorIcon::ResizeHorizontal),
            (before.center_bottom() - vec2(0.0, 2.0), egui::CursorIcon::ResizeVertical),
        ] {
            h.frame(vec![Event::PointerMoved(at)]);
            h.frame(vec![]);
            assert_eq!(h.cursor, cursor, "{panel} at {at:?}, frame {before:?}");
        }
        let from = before.left_bottom() + vec2(2.0, -2.0);
        h.drag(from, from + vec2(-100.0, 150.0));
        for _ in 0..3 {
            h.frame(vec![]);
        }
        let grown = rect(&h);
        assert!((grown.width() - before.width() - 100.0).abs() < 1.0, "{panel}: {before:?} -> {grown:?}");
        assert!((grown.height() - before.height() - 150.0).abs() < 1.0, "{panel}: {before:?} -> {grown:?}");
        assert_eq!(grown.right(), before.right(), "the icon-column edge stays anchored");
        h.click(icon);
        assert_eq!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()), None);
        h.click(icon);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        assert_eq!(rect(&h).size(), grown.size());
        let saved = serde_json::to_string(&h.layout()).unwrap();
        let mut restored = Harness::new();
        restored.app.ui.docking = Some(serde_json::from_str(&saved).unwrap());
        for _ in 0..3 {
            restored.frame(vec![]);
        }
        restored.click(restored.hit(&Hit::Icon(panel.into())).unwrap().center());
        for _ in 0..3 {
            restored.frame(vec![]);
        }
        assert_eq!(rect(&restored).size(), grown.size(), "{panel} saved layout");
    }
}

/// An icon flyout's top edge and its free top corner (the canvas-side one) resize it upward:
/// the bottom and the icon-column edge stay, and the new top is kept when the flyout closes
/// and reopens and in a saved layout. The docking action that saves it is one VectorCraft
/// accepts (#1094).
#[test]
fn an_icon_flyout_grows_upward_from_its_top_edge_and_free_corner() {
    assert!(known(&Action::MoveFlyoutTop { panel: "glyphs".into(), offset: Some(-40.0) }));
    let mut h = Harness::new();
    let icon = h.hit(&Hit::Icon("glyphs".into())).unwrap().center();
    h.click(icon);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    let rect = |h: &Harness| {
        let id = area_id().with("flyout");
        let left = h.ctx.read_response(id.with("resize-left")).unwrap().rect;
        let bottom = h.ctx.read_response(id.with("resize-bottom")).unwrap().rect;
        let top = h.ctx.read_response(id.with("resize-top")).unwrap().rect;
        Rect::from_min_max(pos2(left.left(), top.top()), pos2(bottom.right() + 12.0, bottom.bottom()))
    };
    let before = rect(&h);
    assert!(before.top() > 200.0, "room above the Glyphs flyout: {before:?}");
    for (at, cursor) in [
        (pos2(before.center().x, before.top() + 2.0), egui::CursorIcon::ResizeVertical),
        (before.left_top() + vec2(2.0, 2.0), egui::CursorIcon::ResizeNwSe),
    ] {
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![]);
        assert_eq!(h.cursor, cursor, "at {at:?}, frame {before:?}");
    }
    let from = pos2(before.center().x, before.top() + 2.0);
    h.drag(from, from - vec2(0.0, 80.0));
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let raised = rect(&h);
    assert!((raised.top() - (before.top() - 80.0)).abs() < 1.0, "{before:?} -> {raised:?}");
    assert_eq!(raised.bottom(), before.bottom());
    assert_eq!(raised.right(), before.right(), "the icon-column edge stays anchored");
    let from = raised.left_top() + vec2(2.0, 2.0);
    h.drag(from, from + vec2(-60.0, -20.0));
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let grown = rect(&h);
    assert!((grown.width() - raised.width() - 60.0).abs() < 1.0, "{raised:?} -> {grown:?}");
    assert!((grown.top() - (raised.top() - 20.0)).abs() < 1.0, "{raised:?} -> {grown:?}");
    assert_eq!(grown.bottom(), before.bottom());
    assert_eq!(grown.right(), before.right());
    h.click(icon);
    assert_eq!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()), None);
    h.click(icon);
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert_eq!(rect(&h), grown, "reopened where it was");
    let saved = serde_json::to_string(&h.layout()).unwrap();
    assert!(saved.contains("flyout_offset"));
    let mut restored = Harness::new();
    restored.app.ui.docking = Some(serde_json::from_str(&saved).unwrap());
    for _ in 0..3 {
        restored.frame(vec![]);
    }
    restored.click(restored.hit(&Hit::Icon("glyphs".into())).unwrap().center());
    for _ in 0..3 {
        restored.frame(vec![]);
    }
    assert_eq!(rect(&restored), grown, "saved layout");
}

/// Footer controls stay at the resized panel's bottom, including empty or short lists.
/// The same shared footer is used by the other icon panels and library windows.
#[test]
fn expanded_panel_footers_stay_at_the_bottom_with_short_and_long_lists() {
    for (panel, label, artboards) in [
        ("artboards", "New Artboard", 1),
        ("artboards", "New Artboard", 30),
        ("brushes", "New Brush", 1),
        ("swatches", "New Swatch", 1),
        ("symbols", "New Symbol", 1),
        ("graphicStyles", "New Graphic Style (Alt-click to name it)", 1),
        ("appearance", "Add New Stroke", 1),
        ("charStyles", "Create New Character Style", 1),
        ("paraStyles", "Create New Paragraph Style", 1),
        ("colorGuide", "Save color group to Swatch panel", 1),
        ("links", "Show Link Info", 1),
        ("actions", "Create New Action", 1),
        ("variables", "Capture Data Set", 1),
        ("assetExport", "Export for Screens…", 1),
        ("cssProperties", "Generate CSS: the whole document's CSS while nothing is selected", 1),
        ("svgInteractivity", "Link JavaScript Files…", 1),
        ("colorThemes", "Add to Swatches", 1),
        ("glyphs", "Zoom In", 1),
    ] {
        let mut h = Harness::new();
        h.ctx.enable_accesskit();
        for _ in 1..artboards {
            h.app.run("artboard.new", json!({})).unwrap();
        }
        h.app.run("window.panel.dock", json!({"panel":panel,"icon":true})).unwrap();
        let mut layout = h.layout();
        layout.apply(Action::ResizeGroup { panel: panel.into(), height: Some(540.0) }).unwrap();
        layout.apply(Action::ResizeFlyoutWidth { panel: panel.into(), width: Some(420.0) }).unwrap();
        h.app.ui.docking = Some(layout);
        h.app.ui.open_panel = Some(panel.into());
        for _ in 0..5 {
            h.frame(vec![]);
        }
        let bottom = h.ctx.read_response(area_id().with("flyout").with("resize-bottom")).unwrap().rect.bottom();
        let control = h
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .and_then(|(_, node)| node.bounds())
            .unwrap_or_else(|| panic!("missing {label} in {panel}"));
        assert!((control.y1 as f32 - (bottom - 10.0)).abs() < 2.0, "{panel}/{artboards}: footer {control:?}, bottom {bottom}");
        if panel == "artboards" && artboards == 1 {
            let at = pos2(((control.x0 + control.x1) * 0.5) as f32, ((control.y0 + control.y1) * 0.5) as f32);
            h.click(at);
            assert_eq!(h.app.session.active().unwrap().doc.artboards.len(), 2, "the pinned New button works");
        }
    }
}

#[test]
fn a_tab_dragged_out_floats_and_comes_back_where_it_was_after_its_window_closes() {
    let mut h = Harness::new();
    let tab = h.tab("layers").unwrap();
    let canvas = h.app.canvas_rect.unwrap();
    h.drag(tab.center(), canvas.center());
    let layout = h.layout();
    let window = floating_window(&layout, "layers").expect("Layers floats");
    assert_eq!(window_panels(window), ["layers"]);
    let rect = window.rect;
    assert_eq!(main_panels(&layout), ["properties", "libraries"]);
    let close = h.hit(&Hit::Close("layers".into())).expect("the floating window has a close button");
    h.click(close.center());
    assert!(!h.layout().contains(&"layers".to_string()), "× hides its panels");
    assert!(h.app.ui.docking_hidden.contains_key("layers"));
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    let layout = h.layout();
    let window = floating_window(&layout, "layers").expect("Window › Layers brings it back where it was");
    assert_eq!(window.rect, rect);
    assert!(h.tab("layers").is_some());
}

#[test]
fn thumbnail_work_is_needed_only_while_layers_or_links_can_be_seen() {
    let mut h = Harness::new();
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    assert!(thumbnail_panel_showing(&h.app.ui, &h.ctx));
    h.app.ui.dock = false;
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx), "Tab hides every panel");
    h.app.ui.dock = true;
    h.app.ui.screen_mode = 3;
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx), "presentation mode hides panels");
    h.app.ui.screen_mode = 0;
    h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    assert!(thumbnail_panel_showing(&h.app.ui, &h.ctx), "a floating Layers panel still needs thumbnails");
    h.app.run("window.panel", json!({"panel": "layers", "toggle": true})).unwrap();
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx));
    h.app.run("window.panel", json!({"panel": "links"})).unwrap();
    h.frame(vec![]);
    assert!(thumbnail_panel_showing(&h.app.ui, &h.ctx), "the Links flyout also uses row thumbnails");
}

#[test]
fn a_collapsed_dock_uses_the_flyouts_active_tab_for_thumbnail_work() {
    let mut h = Harness::new();
    h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    // A customized layout keeps the group's active tab independent of the icon that opened it.
    h.app.ui.docking = Some(layout_of(&h.app.ui));
    h.app.run("window.panel", json!({"panel": "properties"})).unwrap();
    h.frame(vec![]);
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx), "Properties does not use row thumbnails");
    // A tab press emits Activate without revealing a different flyout anchor.
    gesture(&mut h.app, Action::Activate { panel: "layers".into() }).unwrap();
    h.frame(vec![]);
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("properties"), "the flyout is still named for its opening icon");
    assert!(thumbnail_panel_showing(&h.app.ui, &h.ctx), "the active Layers tab keeps thumbnail jobs");
    gesture(&mut h.app, Action::Activate { panel: "properties".into() }).unwrap();
    h.frame(vec![]);
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx), "returning to Properties drops hidden Layers jobs");
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    assert!(thumbnail_panel_showing(&h.app.ui, &h.ctx));
    gesture(&mut h.app, Action::Activate { panel: "properties".into() }).unwrap();
    h.frame(vec![]);
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
    assert!(!thumbnail_panel_showing(&h.app.ui, &h.ctx), "a Properties tab under the Layers icon does not need thumbnails");
}

/// A Character field shortcut with the Character panel in front of its floating window brings
/// that window forward and leaves the panel showing.
#[test]
fn a_character_field_shortcut_brings_the_floating_character_panel_forward() {
    let mut h = Harness::new();
    let id = h.app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX"})).unwrap()["id"].clone();
    h.app.run("select.set", json!({"ids": [id]})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "character"})).unwrap();
    h.frame(vec![]);
    assert!(showing(&h.app.ui, "character"));
    assert_eq!(h.app.dock_rt.raise, None);
    for command in ["type.highlightKerning", "type.highlightTracking", "type.highlightFontName", "type.highlightFontSize"] {
        h.app.run(command, json!({})).unwrap();
        assert_eq!(h.app.dock_rt.raise.as_deref(), Some("character"), "{command}");
        h.frame(vec![]);
        assert!(showing(&h.app.ui, "character"), "{command} leaves the panel showing");
    }
}

/// With the panels hidden by Tab, a Character field shortcut shows the dock with the floating
/// Character panel, so the field it focuses can be seen.
#[test]
fn a_character_field_shortcut_shows_the_panels_tab_hid() {
    let mut h = Harness::new();
    let id = h.app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX"})).unwrap()["id"].clone();
    h.app.run("select.set", json!({"ids": [id]})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "character"})).unwrap();
    h.frame(vec![]);
    let rect = floating_window(&h.layout(), "character").map(|w| w.rect);
    h.app.run("window.dock", json!({})).unwrap();
    assert!(!visible(&h.app.ui, "character"));
    h.app.run("type.highlightKerning", json!({})).unwrap();
    assert!(h.app.ui.dock && visible(&h.app.ui, "character"), "the panel shows");
    assert_eq!(floating_window(&h.layout(), "character").map(|w| w.rect), rect, "where it was");
    // In Presentation Mode the shortcut still focuses the field, and the panel shows when the mode ends.
    h.app.ui.screen_mode = 3;
    assert_eq!(h.app.run("type.highlightTracking", json!({})).unwrap(), json!({"field": "tracking"}));
    h.app.ui.screen_mode = 0;
    assert!(visible(&h.app.ui, "character"), "the panel stays in place");
}

#[test]
fn the_dock_collapses_to_icons_and_back() {
    let mut h = Harness::new();
    let collapse = h.hit(&Hit::Collapse("properties".into())).expect("the main column's double arrow");
    h.click(collapse.center());
    assert!(dock_collapsed(&h.app.ui) && h.app.ui.dock_collapsed);
    assert!(h.app.ui.docking.is_none(), "collapsing keeps the workspace's default layout");
    assert!(h.tab("properties").is_none() && h.hit(&Hit::Icon("properties".into())).is_some());
    assert!(h.app.run("window.collapseDock", json!({"collapsed": false})).is_ok());
    h.frame(vec![]);
    assert!(!dock_collapsed(&h.app.ui));
    assert!(h.tab("properties").is_some());
}

#[test]
fn tab_hides_the_panels_and_the_canvas_keeps_drawing() {
    let mut h = Harness::new();
    let narrow = h.app.canvas_rect.unwrap();
    h.app.run("window.dock", json!({})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.tab("properties").is_none() && h.hit(&Hit::Icon("swatches".into())).is_none());
    let wide = h.app.canvas_rect.expect("the canvas still draws");
    assert!(wide.width() > narrow.width(), "the canvas takes the panels' room");
    h.app.run("window.dock", json!({})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.tab("properties").is_some());
}

/// Hide the panels with Tab and settle; returns the canvas rect with every panel hidden.
fn hide_with_tab(h: &mut Harness) -> Rect {
    h.app.run("window.dock", json!({})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.tab("properties").is_none());
    h.app.canvas_rect.expect("the canvas draws")
}

/// Frames with the pointer still, for `secs` of the harness clock.
fn wait(h: &mut Harness, secs: f64) {
    let until = h.time + secs;
    while h.time < until {
        h.frame(vec![]);
    }
}

#[test]
fn resting_on_the_right_edge_shows_the_hidden_panels_over_the_canvas_until_the_pointer_leaves() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    let edge = pos2(SCREEN.right() - 2.0, 500.0);
    h.frame(vec![Event::PointerMoved(edge)]);
    wait(&mut h, peek::DELAY / 2.0);
    assert!(h.tab("properties").is_none(), "the panels wait for the pointer to rest");
    wait(&mut h, peek::DELAY);
    let tab = h.tab("properties").expect("resting on the edge shows the right-hand panels");
    assert!(h.hit(&Hit::Icon("swatches".into())).is_some(), "the icon column shows with them");
    assert!(!h.app.ui.dock, "the panels stay hidden for good");
    assert_eq!(h.app.canvas_rect, Some(wide), "the canvas keeps its size, so the art stays where it is");
    assert_eq!(inspect(&h.app, &h.ctx)["peek"], json!("right"));
    h.frame(vec![Event::PointerMoved(tab.center())]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_some(), "they stay while the pointer is over them");
    h.frame(vec![Event::PointerMoved(wide.center())]);
    wait(&mut h, peek::GRACE / 2.0);
    assert!(h.tab("properties").is_some(), "a short trip away keeps them");
    wait(&mut h, peek::GRACE);
    assert!(h.tab("properties").is_none(), "away from them, they hide again");
    assert_eq!(inspect(&h.app, &h.ctx)["peek"], Value::Null);
}

#[test]
fn a_press_on_the_canvas_while_the_panels_show_from_the_edge_does_not_reach_the_art_under_them() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    h.app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    h.frame(vec![]);
    fn selected(app: &VectorcraftApp) -> bool {
        !app.session.active().unwrap().selection.is_empty()
    }
    assert!(selected(&h.app));
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap(), json!("right"));
    h.frame(vec![]);
    h.frame(vec![]);
    let tab = h.tab("properties").expect("the right-hand panels show");
    assert!(tab.left() < wide.right(), "they cover the canvas's right edge");
    assert_eq!(h.app.canvas_rect, Some(wide));
    // A click in the panels, where the canvas lies under them, leaves the selection.
    h.click(pos2(tab.center().x, wide.bottom() - 40.0));
    assert!(selected(&h.app), "the canvas under the panels takes no click");
    // A click on the uncovered canvas away from the art deselects it, as it always does.
    h.click(pos2(wide.left() + 40.0, wide.bottom() - 40.0));
    assert!(!selected(&h.app));
}

#[test]
fn the_canvas_keeps_the_hidden_panels_size_when_the_window_resizes_or_the_panels_show_at_once() {
    let mut h = Harness::new();
    // Shown before any frame has drawn the panels hidden.
    h.app.run("window.dock", json!({})).unwrap();
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap(), json!("right"));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.tab("properties").is_some());
    let wide = h.app.canvas_rect.unwrap();
    h.app.run("window.panels.peek", json!({"side": null})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.app.canvas_rect, Some(wide), "the canvas had the size it has with every panel hidden");
    // A narrower window while they show.
    h.app.run("window.panels.peek", json!({"side": "right"})).unwrap();
    let narrow = Rect::from_min_max(SCREEN.min, pos2(SCREEN.right() - 200.0, SCREEN.bottom()));
    for _ in 0..3 {
        h.time += 0.05;
        h.run(egui::RawInput { time: Some(h.time), screen_rect: Some(narrow), ..Default::default() });
    }
    assert!(h.tab("properties").is_some());
    let canvas = h.app.canvas_rect.unwrap();
    assert!((canvas.width() - (wide.width() - 200.0)).abs() < 1.0, "{canvas:?} follows the window, under the panels");
}

#[test]
fn panels_held_by_a_press_hide_after_its_release_away_from_them() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    h.frame(vec![Event::PointerMoved(pos2(SCREEN.right() - 2.0, 500.0))]);
    wait(&mut h, peek::DELAY * 2.0);
    let tab = h.tab("properties").expect("the panels show");
    let body = pos2(tab.center().x, wide.bottom() - 40.0);
    h.frame(vec![Event::PointerMoved(body), button(body, true)]);
    h.frame(vec![Event::PointerMoved(wide.center())]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_some(), "a held button keeps them");
    h.frame(vec![button(wide.center(), false)]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_none(), "released away from them, they hide");
}

#[test]
fn a_drag_to_the_edge_or_a_pointer_on_the_left_edge_shows_nothing() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    // The default layout docks no panel on the left.
    h.frame(vec![Event::PointerMoved(pos2(SCREEN.left() + 2.0, 500.0))]);
    wait(&mut h, peek::DELAY * 2.0);
    assert!(h.tab("properties").is_none());
    // A drag that reaches the right edge holds a button down.
    h.frame(vec![Event::PointerMoved(wide.center()), button(wide.center(), true)]);
    h.frame(vec![Event::PointerMoved(pos2(SCREEN.right() - 2.0, 500.0))]);
    wait(&mut h, peek::DELAY * 2.0);
    assert!(h.tab("properties").is_none(), "a drag never shows them");
    h.frame(vec![button(pos2(SCREEN.right() - 2.0, 500.0), false)]);
}

#[test]
fn tab_while_the_panels_show_from_the_edge_shows_them_for_good() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    h.frame(vec![Event::PointerMoved(pos2(SCREEN.right() - 2.0, 500.0))]);
    wait(&mut h, peek::DELAY * 2.0);
    assert!(h.tab("properties").is_some());
    h.app.run("window.dock", json!({})).unwrap();
    h.frame(vec![Event::PointerMoved(wide.center())]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.app.ui.dock && h.tab("properties").is_some());
    assert_eq!(inspect(&h.app, &h.ctx)["peek"], Value::Null);
}

#[test]
fn shift_tab_hidden_panels_show_from_the_edge_too() {
    let mut h = Harness::new();
    h.app.run("window.panels", json!({})).unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.tab("properties").is_none());
    h.frame(vec![Event::PointerMoved(pos2(SCREEN.right() - 2.0, 500.0))]);
    wait(&mut h, peek::DELAY * 2.0);
    assert!(h.tab("properties").is_some());
}

#[test]
fn window_panels_peek_rejects_what_it_cannot_show_and_changes_nothing() {
    let mut h = Harness::new();
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap_err(), "the panels are not hidden");
    hide_with_tab(&mut h);
    for side in [json!("up"), json!(1), json!(true), json!("Right")] {
        assert_eq!(h.app.run("window.panels.peek", json!({"side": side})).unwrap_err(), "`side` must be left, right or null", "{side}");
    }
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "left"})).unwrap_err(), "no panels are docked on the left side");
    assert_eq!(h.app.run("window.panels.peek", json!({})).unwrap(), Value::Null, "nothing to hide");
    h.frame(vec![]);
    assert!(h.tab("properties").is_none() && !h.app.ui.dock);
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap(), json!("right"));
    h.frame(vec![]);
    assert!(h.tab("properties").is_some(), "the panels stay while the pointer has not moved");
    assert_eq!(h.app.run("window.panels.peek", json!({"side": null})).unwrap(), Value::Null);
    h.frame(vec![]);
    assert!(h.tab("properties").is_none());
    h.app.ui.screen_mode = 3;
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap_err(), "panels can't be shown in Presentation Mode");
    h.app.ui.screen_mode = 0;
}

#[test]
fn panels_shown_by_window_panels_peek_stay_until_the_pointer_moves_or_leaves_the_window() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap(), json!("right"));
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_some(), "with no pointer movement, the panels stay");
    assert_eq!(inspect(&h.app, &h.ctx)["peek"], json!("right"));
    h.frame(vec![Event::PointerMoved(wide.center())]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_none(), "the pointer moved away from them, so they hide");
    // The pointer leaving the window counts as moving away.
    h.app.run("window.panels.peek", json!({"side": "right"})).unwrap();
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_some());
    h.frame(vec![Event::PointerGone]);
    wait(&mut h, peek::GRACE * 2.0);
    assert!(h.tab("properties").is_none(), "the pointer left the window, so they hide");
    assert_eq!(inspect(&h.app, &h.ctx)["peek"], Value::Null);
}

#[test]
fn panels_held_by_a_flyout_hide_after_it_closes_with_the_pointer_still_away_from_them() {
    let mut h = Harness::new();
    let wide = hide_with_tab(&mut h);
    // The pointer rests on the canvas, then outside the window, while a flyout holds the panels.
    for away in [vec![Event::PointerMoved(wide.center())], vec![Event::PointerGone]] {
        h.app.run("window.panels.peek", json!({"side": "right"})).unwrap();
        h.app.ui.open_panel = Some("swatches".into());
        h.frame(away);
        wait(&mut h, peek::GRACE * 2.0);
        assert!(h.tab("properties").is_some(), "an open flyout keeps them");
        h.app.ui.open_panel = None;
        wait(&mut h, peek::GRACE * 2.0);
        assert!(h.tab("properties").is_none(), "the flyout closed with the pointer away from them, so they hide");
        assert_eq!(inspect(&h.app, &h.ctx)["peek"], Value::Null);
    }
}

#[test]
fn edge_panels_are_the_docked_panels_on_each_side_of_the_canvas() {
    let layout = default_from(DockTab::ALL[0], &[], false, None, None);
    let right = peek::edge_panels(&layout, peek::Edge::Right);
    assert!(right.contains(&"properties".to_string()) && right.contains(&"swatches".to_string()));
    assert!(!right.contains(&CANVAS.to_string()));
    assert!(peek::edge_panels(&layout, peek::Edge::Left).is_empty());
    let mirrored = Layout {
        root: Some(Node::Split {
            axis: craft_ui::layout::SplitAxis::Horizontal,
            size: craft_ui::layout::SplitSize::FixedFirst(310.0),
            first: Box::new(Node::Tabs { panels: vec!["layers".to_string()], active: 0 }),
            second: Box::new(Node::Tabs { panels: vec![CANVAS.to_string()], active: 0 }),
        }),
        floating: Vec::new(),
    };
    assert_eq!(peek::edge_panels(&mirrored, peek::Edge::Left), ["layers"]);
    assert!(peek::edge_panels(&mirrored, peek::Edge::Right).is_empty());
}

#[test]
fn switching_workspace_during_a_tab_drag_keeps_the_new_workspace_layout() {
    let mut h = Harness::new();
    let tab = h.tab("layers").unwrap();
    let canvas = h.app.canvas_rect.unwrap();
    h.press_and_move(tab.center(), canvas.center());
    h.app.run("window.workspace", json!({"name": "Layout"})).unwrap();
    h.frame(vec![Event::PointerMoved(canvas.center())]);
    h.frame(vec![button(canvas.center(), false)]);
    h.frame(vec![]);
    assert!(h.app.ui.docking.is_none(), "the drag ended with the switch");
    assert!(h.layout().floating.is_empty());
    assert_eq!(main_panels(&h.layout()), ["properties", "layers", "libraries"]);
}

#[test]
fn the_tools_panel_floats_by_its_title_bar_and_docks_on_the_left_edge() {
    let mut h = Harness::new();
    let zone = docked_tools(&h);
    let title = zone.min + vec2(20.0, 12.0);
    h.drag(title, pos2(600.0, 400.0));
    let [x, y] = h.app.ui.toolbar_pos.expect("the Tools panel floats");
    assert!((300.0..700.0).contains(&x) && (250.0..450.0).contains(&y), "{x} {y}");
    let title = pos2(x + 20.0, y + 7.0);
    h.drag(title, zone.center());
    assert_eq!(h.app.ui.toolbar_pos, None, "dropped on the left edge, it docks");
}

/// The Tools panel also docks on the right, left of the panels: dropped on the window's right edge
/// it stands between the canvas and the panels, its flyouts open toward the canvas, the side is
/// kept with the preferences, and it drags back to the left edge.
#[test]
fn the_tools_panel_docks_on_the_right_left_of_the_panels() {
    let mut h = Harness::new();
    let zone = docked_tools(&h);
    h.drag(zone.min + vec2(20.0, 12.0), pos2(600.0, 400.0));
    let [x, y] = h.app.ui.toolbar_pos.expect("the Tools panel floats");
    h.drag(pos2(x + 20.0, y + 7.0), pos2(SCREEN.right() - 10.0, 500.0));
    assert_eq!((h.app.ui.toolbar_pos, h.app.ui.toolbar_right), (None, true), "dropped on the right edge, it docks on the right");
    settle(&mut h);
    let bar = docked_tools(&h);
    let canvas = h.app.canvas_rect.expect("the canvas");
    let layers = h.tab("layers").expect("the Layers panel, docked right");
    assert!(canvas.right() <= bar.left() + 1.0 && bar.right() <= layers.left() + 1.0, "canvas {canvas:?}, Tools {bar:?}, Layers {layers:?}");
    assert!(SCREEN.right() - bar.right() > 100.0, "the panels keep the window's right edge: {bar:?}");
    // Its flyouts and drawer open toward the canvas, not over the panels.
    let room = crate::toolbar::room(&h.app, &h.ctx);
    assert!(room.right() <= bar.right() + 0.5, "{room:?}");
    let left = crate::toolbar::beside(bar, 160.0, 8.0, room);
    assert!(left + 160.0 <= bar.left() - 7.0, "left of the toolbar: {left}");
    let prefs: crate::state::UiState = serde_json::from_slice(&serde_json::to_vec(&h.app.ui).unwrap()).unwrap();
    assert!(prefs.toolbar_right, "kept with the preferences");
    // Dragged back to the window's left edge, it docks there again.
    h.drag(bar.min + vec2(20.0, 12.0), pos2(600.0, 400.0));
    let [x, y] = h.app.ui.toolbar_pos.expect("floats again");
    h.drag(pos2(x + 20.0, y + 7.0), pos2(12.0, 500.0));
    assert_eq!((h.app.ui.toolbar_pos, h.app.ui.toolbar_right), (None, false));
    settle(&mut h);
    assert!(docked_tools(&h).left() < 10.0, "at the window's left edge again");
    // Agents: window.panel.dock takes the side (left/right, or Start/End as panels do).
    let r = h.app.run("window.panel.dock", json!({"panel": "tools", "side": "End"})).unwrap();
    assert_eq!((r["side"].as_str(), h.app.ui.toolbar_right), (Some("right"), true));
    h.app.run("window.panel.float", json!({"panel": "tools"})).unwrap();
    assert_eq!(h.app.run("window.panel.dock", json!({"panel": "tools"})).unwrap()["side"], "right", "back on the side it docked at last");
    assert!(h.app.run("window.panel.dock", json!({"panel": "tools", "side": "top"})).is_err());
    // A side that isn't text is an error too, not the last side: it leaves Tools where it is.
    h.app.run("window.panel.float", json!({"panel": "tools"})).unwrap();
    for side in [json!(5), json!(true), json!(["left"])] {
        assert!(h.app.run("window.panel.dock", json!({"panel": "tools", "side": side})).is_err(), "{side}");
        assert!(h.app.ui.toolbar_pos.is_some(), "still floating after side {side}");
    }
    assert_eq!(h.app.run("window.panel.dock", json!({"panel": "tools", "side": null})).unwrap()["side"], "right", "null: the last side");
}

/// The docked Tools panel as last drawn.
fn docked_tools(h: &Harness) -> Rect {
    h.ctx.data(|d| d.get_temp::<Rect>(crate::toolbar::docked_rect_id())).expect("the docked Tools panel")
}

fn settle(h: &mut Harness) {
    for _ in 0..3 {
        h.frame(vec![]);
    }
}

fn keep_tools_beside_canvas(h: &mut Harness, on: bool) {
    assert_eq!(h.app.run("window.toolsBesideCanvas", json!({"on": on})).unwrap(), json!(on));
    settle(h);
}

/// Where docked Tools sits with a left-side dock: `Some(true)` between the dock and the canvas,
/// `Some(false)` at the window's edge outside the dock.
fn tools_place(h: &Harness) -> Option<bool> {
    let tools = docked_tools(h);
    let layers = h.tab("layers")?;
    let canvas = h.app.canvas_rect?;
    if layers.right() <= tools.left() && tools.right() <= canvas.left() && canvas.left() - tools.right() < 40.0 {
        Some(true)
    } else if tools.left() < 10.0 && tools.right() <= layers.left() {
        Some(false)
    } else {
        None
    }
}

/// Off by default: docked Tools stays at the window's edge, outside a left-side dock, and the
/// setting moves it between the dock and the canvas and back.
#[test]
fn docked_tools_keep_the_window_edge_unless_kept_beside_the_canvas() {
    let mut h = Harness::new();
    assert!(!h.app.ui.tools_beside_canvas);
    assert_eq!(crate::menus::checked(&h.app, "window.toolsBesideCanvas", &json!({})), Some(false));
    h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
    settle(&mut h);
    assert_eq!(tools_place(&h), Some(false), "Tools, then the left dock, then the canvas");
    assert_eq!(crate::toolbar::docked_width(&h.app.ui), 0.0, "the canvas reserves no room for Tools");
    keep_tools_beside_canvas(&mut h, true);
    assert_eq!(crate::menus::checked(&h.app, "window.toolsBesideCanvas", &json!({})), Some(true));
    assert_eq!(tools_place(&h), Some(true), "the left dock, then Tools, then the canvas");
    keep_tools_beside_canvas(&mut h, false);
    assert_eq!(tools_place(&h), Some(false), "back at the window's edge");
}

#[test]
fn keep_tools_beside_the_canvas_toggles_and_rejects_a_value_that_is_not_true_or_false() {
    let mut h = Harness::new();
    assert_eq!(h.app.run("window.toolsBesideCanvas", json!({})).unwrap(), json!(true));
    assert_eq!(h.app.run("window.toolsBesideCanvas", json!({})).unwrap(), json!(false));
    assert!(h.app.run("window.toolsBesideCanvas", json!({"on": "yes"})).is_err());
    assert!(!h.app.ui.tools_beside_canvas, "a bad value changes nothing");
}

/// Whichever way the setting is, the canvas's left edge, right of a left-side dock, docks a
/// floating Tools panel beside the canvas and turns the setting on, and the window's left edge
/// docks it outside the dock and turns the setting off; the menu item's check mark follows.
#[test]
fn tools_dropped_on_a_left_edge_dock_there_and_the_setting_follows() {
    for beside in [false, true] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
        for at_window_edge in [false, true, false] {
            h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
            settle(&mut h);
            let canvas = h.app.canvas_rect.unwrap();
            let canvas_edge = h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id()));
            let canvas_edge = canvas_edge.expect("the canvas's left edge is a target whichever way the setting is");
            assert!(canvas_edge.left() > 60.0, "right of the left-side dock: {canvas_edge:?}");
            let target = if at_window_edge { pos2(12.0, canvas.center().y) } else { canvas_edge.center() };
            h.drag(pos2(620.0, 307.0), target);
            let case = format!("set before: {beside}, at the window edge: {at_window_edge}");
            assert_eq!(h.app.ui.toolbar_pos, None, "{case}");
            assert_eq!(h.app.ui.tools_beside_canvas, !at_window_edge, "{case}");
            assert_eq!(crate::menus::checked(&h.app, "window.toolsBesideCanvas", &json!({})), Some(!at_window_edge), "{case}");
            settle(&mut h);
            assert_eq!(tools_place(&h), Some(!at_window_edge), "{case}");
        }
    }
}

/// With no panels docked on the left, the canvas's left edge and the window's left edge overlap,
/// and a drop there docks Tools where the setting puts it, leaving the setting as it is.
#[test]
fn tools_dropped_where_the_left_edges_overlap_keep_the_setting() {
    for beside in [false, true] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
        settle(&mut h);
        let canvas_edge = h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id())).expect("the canvas-edge target");
        assert!(canvas_edge.left() < 30.0, "no panels on the left: {canvas_edge:?}");
        h.drag(pos2(620.0, 307.0), pos2(canvas_edge.left() + 4.0, canvas_edge.center().y));
        assert_eq!(h.app.ui.toolbar_pos, None, "set before: {beside}");
        assert_eq!(h.app.ui.tools_beside_canvas, beside, "the setting is unchanged");
        settle(&mut h);
        let tools = docked_tools(&h);
        assert!(tools.left() < 10.0, "docked at the left edge, set before: {beside}: {tools:?}");
        assert!(tools.right() <= h.app.canvas_rect.unwrap().left() + 1.0, "left of the canvas: {tools:?}");
    }
}

/// A toolbar made with New Toolbar docked on the left, and no panels there: Tools docks left of the
/// toolbar's lane either way, so a drop on the canvas's edge, right of the lane, keeps the setting.
#[test]
fn tools_dropped_beside_a_left_toolbar_lane_keep_the_setting() {
    for beside in [false, true] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        h.app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        h.app.run("window.toolbar.dock", json!({"name": "Mine", "side": "left"})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
        settle(&mut h);
        let canvas_edge = h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id())).expect("the canvas-edge target");
        assert!(canvas_edge.left() > 30.0, "right of the toolbar lane: {canvas_edge:?}");
        let at = pos2(canvas_edge.right() - 4.0, canvas_edge.center().y);
        let lit = crate::tools_float::drop_highlight(&h.ctx, at, beside).expect("a strip lights up");
        assert!(lit.left() < 10.0, "the window's edge, where Tools docks, lights up: {lit:?}");
        h.drag(pos2(620.0, 307.0), at);
        assert_eq!(h.app.ui.toolbar_pos, None, "set before: {beside}");
        assert_eq!(h.app.ui.tools_beside_canvas, beside, "the setting is unchanged");
        settle(&mut h);
        assert!(docked_tools(&h).left() < 10.0, "left of the lane, set before: {beside}");
    }
}

/// Dragged out of the panel docked at the window's edge and released in the next frame, right of
/// where the panel was, Tools stays floating and the setting stays off.
#[test]
fn tools_released_as_they_float_from_the_window_edge_keep_floating() {
    let mut h = Harness::new();
    let docked = docked_tools(&h);
    let title = docked.min + vec2(20.0, 12.0);
    h.frame(vec![Event::PointerMoved(title), button(title, true)]);
    h.frame(vec![Event::PointerMoved(title + vec2(0.0, 10.0))]);
    let out = pos2(docked.right().max(crate::toolbar::WIDTH) + 8.0, title.y + 10.0);
    h.frame(vec![Event::PointerMoved(out)]);
    assert!(h.app.ui.toolbar_pos.is_some(), "dragged out of the docked panel, Tools floats");
    h.frame(vec![button(out, false)]);
    settle(&mut h);
    assert!(h.app.ui.toolbar_pos.is_some(), "the strip laid out right of the docked panel docks nothing");
    assert!(!h.app.ui.tools_beside_canvas);
}

/// A drop on the right edge leaves the setting as it is, and so does a move of Tools floating over
/// the canvas's left edge that ends within a few points of where it started.
#[test]
fn tools_docked_on_the_right_or_clicked_leave_the_setting() {
    for beside in [false, true] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
        settle(&mut h);
        h.drag(pos2(620.0, 307.0), pos2(SCREEN.right() - 10.0, 500.0));
        assert_eq!((h.app.ui.toolbar_pos, h.app.ui.toolbar_right), (None, true), "set before: {beside}");
        assert_eq!(h.app.ui.tools_beside_canvas, beside, "docked on the right, the setting is unchanged");
        let canvas_edge = h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id())).expect("the canvas-edge target");
        h.app.run("window.panel.float", json!({"panel": "tools", "x": canvas_edge.left(), "y": 300})).unwrap();
        settle(&mut h);
        let [x, y] = h.app.ui.toolbar_pos.unwrap();
        let title = pos2(x + 12.0, y + 7.0);
        assert!(canvas_edge.contains(title), "the title bar is over the canvas's edge: {title:?} {canvas_edge:?}");
        h.press_and_move(title, title + vec2(30.0, 0.0));
        let moved = h.app.ui.toolbar_pos.map(|[px, _]| px - x);
        assert!(moved.is_some_and(|dx| dx > 10.0), "the move started, set before: {beside}: {moved:?}");
        let back = title + vec2(2.0, 0.0);
        h.frame(vec![Event::PointerMoved(back)]);
        h.frame(vec![button(back, false)]);
        settle(&mut h);
        assert!(h.app.ui.toolbar_pos.is_some(), "released where it started, Tools docks nowhere, set before: {beside}");
        assert_eq!(h.app.ui.tools_beside_canvas, beside, "the setting is unchanged");
    }
}

/// A left-side dock collapsed to icons is narrower than the window's strip: only the column the
/// two strips share keeps the setting, and each strip's own part docks at its place.
#[test]
fn tools_dropped_beside_a_collapsed_left_dock_follow_the_strip_dropped_on() {
    let mut h = Harness::new();
    h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
    h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    let float = |h: &mut Harness| {
        h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
        settle(h);
        h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id())).expect("the canvas-edge target")
    };
    let zone = float(&mut h);
    let window_strip = crate::toolbar::WIDTH;
    assert!(zone.left() > 10.0 && zone.left() < window_strip - 4.0, "an icon column narrower than the window's strip: {zone:?}");
    let y = zone.center().y;
    // Beside the canvas: right of the icon column; at the window's edge: left of it.
    let placed_beside = |h: &mut Harness, icons: Rect| {
        settle(h);
        let tools = docked_tools(h);
        if tools.left() >= icons.left() - 1.0 {
            Some(true)
        } else if tools.left() < 10.0 {
            Some(false)
        } else {
            None
        }
    };
    // Right of the window's strip: beside the canvas.
    h.drag(pos2(620.0, 307.0), pos2(window_strip + 4.0, y));
    assert_eq!((h.app.ui.toolbar_pos, h.app.ui.tools_beside_canvas), (None, true));
    assert_eq!(placed_beside(&mut h, zone), Some(true));
    // In the shared column: where the setting puts it, the setting unchanged.
    let zone = float(&mut h);
    h.drag(pos2(620.0, 307.0), pos2(zone.left() + 2.0, y));
    assert_eq!((h.app.ui.toolbar_pos, h.app.ui.tools_beside_canvas), (None, true));
    assert_eq!(placed_beside(&mut h, zone), Some(true));
    // Left of the icon column: at the window's edge.
    let zone = float(&mut h);
    h.drag(pos2(620.0, 307.0), pos2(zone.left() / 2.0, y));
    assert_eq!((h.app.ui.toolbar_pos, h.app.ui.tools_beside_canvas), (None, false));
    assert_eq!(placed_beside(&mut h, zone), Some(false));
}

/// Turning the setting on or off while Tools floats leaves it floating.
#[test]
fn toggling_while_tools_floats_keeps_it_floating() {
    let mut h = Harness::new();
    h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
    keep_tools_beside_canvas(&mut h, true);
    h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
    settle(&mut h);
    let [x, y] = h.app.ui.toolbar_pos.unwrap();
    keep_tools_beside_canvas(&mut h, false);
    assert_eq!(h.app.ui.toolbar_pos, Some([x, y]), "turning it off leaves Tools floating where it was");
    keep_tools_beside_canvas(&mut h, true);
    assert_eq!(h.app.ui.toolbar_pos, Some([x, y]), "turning it on leaves Tools floating where it was");
}

/// The setting is kept with a workspace; state and workspaces saved without it keep Tools at the
/// window's edge.
#[test]
fn keep_tools_beside_the_canvas_is_saved_and_off_in_state_saved_without_it() {
    let mut saved = serde_json::to_value(crate::state::UiState { tools_beside_canvas: true, ..Default::default() }).unwrap();
    assert_eq!(saved["tools_beside_canvas"], json!(true));
    let reloaded: crate::state::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert!(reloaded.tools_beside_canvas);
    saved.as_object_mut().unwrap().remove("tools_beside_canvas");
    let old: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert!(!old.tools_beside_canvas && old.toolbar_pos.is_none(), "older state docks Tools at the window's edge");

    let mut h = Harness::new();
    h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
    keep_tools_beside_canvas(&mut h, true);
    let mut workspace = serde_json::to_value(crate::workspaces::capture(&h.app.ui, "Mine")).unwrap();
    assert_eq!(workspace["toolsBesideCanvas"], json!(true));
    workspace.as_object_mut().unwrap().remove("toolsBesideCanvas");
    let old: crate::workspaces::Workspace = serde_json::from_value(workspace).unwrap();
    assert!(!old.tools_beside_canvas, "a workspace saved without it docks Tools at the window's edge");

    h.app.run("window.workspace.new", json!({"name": "Mine"})).unwrap();
    h.app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
    settle(&mut h);
    assert!(!h.app.ui.tools_beside_canvas, "built-in workspaces keep Tools at the window's edge");
    h.app.run("window.workspace", json!({"name": "Mine"})).unwrap();
    settle(&mut h);
    assert!(h.app.ui.tools_beside_canvas);
    assert_eq!(tools_place(&h), Some(true), "the saved workspace brings Tools back beside the canvas");
}

#[test]
fn tools_follow_left_panels_when_resized_hidden_and_restored() {
    for (beside, double) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        h.app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
        gesture(&mut h.app, Action::ResizeColumn { panel: "layers".into(), width: 430.0 }).unwrap();
        h.app.ui.toolbar_double = double;
        let width = if double { 80.0 } else { 48.0 };
        for hidden in [false, true, false] {
            if hidden == h.app.ui.dock {
                h.app.run("window.panels", json!({})).unwrap();
            }
            settle(&mut h);
            let tools = docked_tools(&h);
            assert!((tools.width() - (width + 6.0)).abs() < 1.0, "{} columns: {tools:?}", if double { 2 } else { 1 });
            if hidden {
                assert!(h.tab("layers").is_none());
                assert!(tools.left() < 10.0);
            } else {
                assert_eq!(tools_place(&h), Some(beside), "beside: {beside}, double: {double}");
                assert!(h.tab("layers").unwrap().width() > 0.0);
            }
            let canvas = h.app.canvas_rect.unwrap();
            assert!(canvas.left() - tools.left() >= width, "Tools has its own room: {tools:?}, {canvas:?}");
            assert!(canvas.width() > 200.0);
        }
    }
}

/// A layout with panels above or below the canvas: the full-height strip along the window's left
/// edge docks Tools in both placements; off, Tools runs the window's height beside those panels,
/// and on, it stays in the canvas's row.
#[test]
fn tools_redock_at_window_edge_above_and_below_canvas() {
    for (beside, above) in [(false, true), (false, false), (true, true), (true, false)] {
        let mut h = Harness::new();
        keep_tools_beside_canvas(&mut h, beside);
        let tabs = |panel: &str| Box::new(Node::Tabs { panels: vec![panel.to_string()], active: 0 });
        h.app.ui.docking = Some(Layout {
            root: Some(Node::Split {
                axis: craft_ui::layout::SplitAxis::Vertical,
                size: craft_ui::layout::SplitSize::FixedFirst(180.0),
                first: tabs("layers"),
                second: Box::new(Node::Split {
                    axis: craft_ui::layout::SplitAxis::Vertical,
                    size: craft_ui::layout::SplitSize::FixedSecond(180.0),
                    first: tabs(CANVAS),
                    second: tabs("properties"),
                }),
            }),
            floating: vec![],
        });
        assert!(valid(&h.layout()));
        h.app.run("window.panel.float", json!({"panel": "tools", "x": 600, "y": 300})).unwrap();
        for _ in 0..3 {
            h.frame(vec![]);
        }
        let panel = h.tab(if above { "layers" } else { "properties" }).unwrap();
        let canvas = h.app.canvas_rect.unwrap();
        assert!(panel.center().y < canvas.top() || panel.center().y > canvas.bottom());
        h.drag(pos2(620.0, 307.0), pos2(12.0, panel.center().y));
        assert_eq!(h.app.ui.toolbar_pos, None, "window-edge drop, beside: {beside}, above the canvas: {above}");
        assert_eq!(h.app.ui.tools_beside_canvas, beside, "no panels on the left: the setting is unchanged");
        settle(&mut h);
        let tools = docked_tools(&h);
        let panel = h.tab(if above { "layers" } else { "properties" }).unwrap();
        assert!(tools.left() < 10.0 && tools.right() <= panel.left() + 60.0, "{tools:?}");
        match (beside, above) {
            (false, true) => assert!(tools.top() <= panel.top(), "Tools runs past the panel above: {tools:?}, {panel:?}"),
            (false, false) => assert!(tools.bottom() >= panel.bottom(), "Tools runs past the panel below: {tools:?}, {panel:?}"),
            (true, true) => assert!(tools.top() >= panel.bottom(), "Tools stays below the panel above: {tools:?}, {panel:?}"),
            (true, false) => assert!(tools.bottom() <= panel.top(), "Tools stays above the panel below: {tools:?}, {panel:?}"),
        }
    }
}

#[test]
#[ignore = "manual visual check; set LEFT_TOOLS_SCREENSHOTS to an existing harness directory"]
fn capture_tools_beside_left_panels() {
    let dir = std::path::PathBuf::from(std::env::var_os("LEFT_TOOLS_SCREENSHOTS").unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.toolsBesideCanvas", json!({"on": true})).unwrap();
    app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(1200.0, 800.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, app: &mut VectorcraftApp| app.ui(ui), app);
    h.run_steps(6);
    h.render().unwrap().save(dir.join("tools-inside-left-dock.png")).unwrap();
    h.state_mut().ui.toolbar_double = true;
    h.run_steps(3);
    h.render().unwrap().save(dir.join("tools-inside-left-dock-double.png")).unwrap();
}

#[test]
fn a_library_panel_opens_renames_and_closes() {
    let mut h = Harness::new();
    h.app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    let pastels = "library:swatches:pastels".to_string();
    h.frame(vec![]);
    assert!(floating_window(&h.layout(), &pastels).is_some(), "a library panel floats");
    assert!(h.tab(&pastels).is_some());
    h.app.run("ui.libraryPanel", json!({"panel": pastels, "library": "web-safe-216"})).unwrap();
    let web = "library:swatches:web-safe-216".to_string();
    h.frame(vec![]);
    assert!(h.layout().contains(&web) && !h.layout().contains(&pastels));
    assert!(h.tab(&web).is_some());
    h.app.run("window.panel.close", json!({"panel": web})).unwrap();
    h.frame(vec![]);
    assert!(!h.layout().contains(&web));
}

/// The floating window holding `panel`, by its panels.
fn window_of(layout: &Layout<String>, panel: &str) -> Vec<String> {
    floating_window(layout, panel).map(window_panels).unwrap_or_default()
}

#[test]
fn float_onto_above_and_below_stack_with_a_floating_group() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "stroke", "onto": "Swatches"})).unwrap();
    let layout = h.layout();
    assert_eq!(group_of(&layout, &"stroke".into()), vec!["swatches".to_string(), "stroke".into()], "a tab of its group");

    h.app.run("window.panel.float", json!({"panel": "color", "below": "swatches"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "gradient", "above": "swatches"})).unwrap();
    let layout = h.layout();
    let window = window_of(&layout, "swatches");
    for panel in ["color", "gradient", "stroke"] {
        assert!(window.contains(&panel.to_string()), "{panel} in the window: {window:?}");
    }
    assert_eq!(group_of(&layout, &"color".into()), vec!["color".to_string()], "a group of its own below");
    assert_eq!(group_of(&layout, &"gradient".into()), vec!["gradient".to_string()]);
    h.frame(vec![]);
    let (top, middle, bottom) = (h.tab("gradient").unwrap(), h.tab("swatches").unwrap(), h.tab("color").unwrap());
    assert!(top.top() < middle.top() && middle.top() < bottom.top(), "gradient above, color below");

    // `onto` names a floating group; a docked one is refused.
    assert!(h.app.run("window.panel.float", json!({"panel": "stroke", "onto": "layers"})).is_err());
}

#[test]
fn float_collapsed_minimizes_its_window() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches", "collapsed": true})).unwrap();
    assert!(floating_window(&h.layout(), "swatches").unwrap().minimized);
    h.app.run("window.panel.float", json!({"panel": "swatches", "collapsed": false})).unwrap();
    assert!(!floating_window(&h.layout(), "swatches").unwrap().minimized);
    let r = h.app.run("window.panel.collapse", json!({"panel": "Layers"})).unwrap();
    assert_eq!((r["panel"].clone(), r["collapsed"].clone()), (json!("layers"), json!(true)), "a docked group minimizes too");
}

#[test]
fn dock_column_n_docks_a_column_at_that_place() {
    let mut h = Harness::new();
    let before = end_columns(&h.layout()).len();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "color", "below": "swatches"})).unwrap();
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
    let layout = h.layout();
    let columns = end_columns(&layout);
    assert_eq!(columns.len(), before + 1);
    let panels: Vec<&String> = columns[2].groups.iter().flat_map(|g| &g.panels).collect();
    assert_eq!(panels, ["swatches", "color"], "the whole floating window, next to the icon column");
    assert!(layout.floating.is_empty());

    // `true` docks at the innermost place; without `group`, the panel leaves its group.
    h.app.run("window.panel.dock", json!({"panel": "layers", "column": true})).unwrap();
    let layout = h.layout();
    let columns = end_columns(&layout);
    assert_eq!(columns.len(), before + 2);
    assert_eq!(columns.last().unwrap().groups[0].panels, vec!["layers".to_string()]);
    assert_eq!(main_panels(&layout), ["properties", "libraries"]);
}

#[test]
fn dock_icon_puts_a_panel_in_the_icon_column() {
    let mut h = Harness::new();
    h.app.run("window.panel.dock", json!({"panel": "layers", "icon": true})).unwrap();
    let layout = h.layout();
    assert!(in_iconic_column(&layout, "layers"));
    assert!(!main_panels(&layout).contains(&"layers".to_string()));
    assert_eq!(group_of(&layout, &"layers".into()), vec!["layers".to_string()], "a row of its own");
}

fn column_panels(layout: &Layout<String>) -> Vec<Vec<String>> {
    end_columns(layout).iter().map(|c| c.groups.iter().flat_map(|g| g.panels.clone()).collect()).collect()
}

#[test]
fn a_docked_column_moves_to_a_later_place_and_back() {
    let mut h = Harness::new();
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
    h.app.run("window.panel.dock", json!({"panel": "color", "column": 2})).unwrap();
    let columns = column_panels(&h.layout());
    assert_eq!(columns[2..], [vec!["swatches".to_string()], vec!["color".to_string()]]);
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 2})).unwrap();
    let columns = column_panels(&h.layout());
    assert_eq!(columns[2..], [vec!["color".to_string()], vec!["swatches".to_string()]], "swatches moved inwards");
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
    let columns = column_panels(&h.layout());
    assert_eq!(columns[2..], [vec!["swatches".to_string()], vec!["color".to_string()]], "and back");
    let before = h.layout();
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
    assert_eq!(h.layout(), before, "already there");
}

/// A layout converted from an old dock split side by side, with the dock collapsed: two
/// collapsed tree columns before the icon column.
#[test]
fn column_places_and_icon_count_from_the_icon_column() {
    let mut h = Harness::new();
    let mut layout = h.layout();
    if let Some(Node::Split { second, .. }) = layout.root.as_mut()
        && let Node::Dock { columns, .. } = second.as_mut()
    {
        let libraries = columns[0].groups[0].panels.pop().unwrap();
        assert_eq!(libraries, "libraries");
        columns.insert(1, Column { groups: vec![Group::new(vec![libraries], 0)], width: 200.0, iconic: true, iconic_width: Some(ICON_WIDTH) });
        columns[0].iconic = true;
    }
    let width: f32 = end_columns(&layout).iter().map(|c| c.iconic_width.unwrap_or(ICON_WIDTH)).sum();
    if let Some(Node::Split { size, .. }) = layout.root.as_mut() {
        *size = craft_ui::layout::SplitSize::FixedSecond(width);
    }
    assert!(valid(&layout));
    h.app.ui.docking = Some(layout);
    h.app.run("window.panel.dock", json!({"panel": "layers", "icon": true})).unwrap();
    let layout = h.layout();
    let columns = column_panels(&layout);
    assert!(columns[2].contains(&"layers".to_string()), "{columns:?}");
    h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
    let columns = column_panels(&h.layout());
    assert_eq!(columns[..2], [vec!["properties".to_string()], vec!["libraries".to_string()]]);
    assert_eq!(columns[3], ["swatches"], "next to the icon column: {columns:?}");

    // No icon column: `icon` makes one.
    let mut layout = h.layout();
    if let Some(Node::Split { second, .. }) = layout.root.as_mut()
        && let Node::Dock { columns, .. } = second.as_mut()
    {
        columns.remove(2);
    }
    h.app.ui.docking = Some(layout);
    assert_eq!(icon_column(&end_columns(&h.layout())), None);
    h.app.run("window.panel.dock", json!({"panel": "properties", "icon": true})).unwrap();
    let layout = h.layout();
    assert!(in_iconic_column(&layout, "properties"));
    assert!(icon_column(&end_columns(&layout)).is_none_or(|k| column_panels(&layout)[k].contains(&"properties".to_string())));
}

#[test]
fn invalid_docking_parameters_change_nothing() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    let before = h.layout();
    for (id, params) in [
        ("window.panel.float", json!({"panel": "layers", "onto": "nope"})),
        ("window.panel.float", json!({"panel": "layers", "onto": "swatches", "below": "swatches"})),
        ("window.panel.float", json!({"panel": "layers", "onto": "swatches", "x": 10, "y": 10})),
        ("window.panel.float", json!({"panel": "swatches", "onto": "swatches"})),
        ("window.panel.float", json!({"panel": "layers", "icon": true})),
        ("window.panel.float", json!({"panel": "layers", "collapsed": "yes"})),
        ("window.panel.dock", json!({"panel": "layers", "icon": true, "anchor": "swatches"})),
        ("window.panel.dock", json!({"panel": "layers", "column": 0})),
        ("window.panel.dock", json!({"panel": "layers", "column": "left"})),
        ("window.panel.dock", json!({"panel": "layers", "icon": "yes"})),
    ] {
        assert!(h.app.run(id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(h.layout(), before, "{id} {params} changed the layout");
    }
}

#[test]
fn ui_docking_reports_the_layout_and_hit_rects() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.frame(vec![]);
    let tab = h.tab("layers").unwrap();
    h.click(tab.center());
    let report = inspect(&h.app, &h.ctx);
    assert_eq!(report["layout"], serde_json::to_value(h.layout()).unwrap());
    assert_eq!(report["focused"], json!("layers"));
    let hits = report["hitRects"].as_array().unwrap();
    for hit in [json!({"Tab": "layers"}), json!({"Title": "swatches"})] {
        let found = hits.iter().find(|h| h["hit"] == hit).unwrap_or_else(|| panic!("{hit} in {hits:?}"));
        assert_eq!(found["rect"].as_array().unwrap().len(), 4);
    }
}

/// Press on the main column's divider and drag it 60 pt toward the canvas, holding the button.
fn press_on_the_main_column_divider(h: &mut Harness) -> f32 {
    let edge =
        hit_rects(&h.ctx).into_iter().filter(|(hit, _)| matches!(hit, Hit::Divider(_))).map(|(_, r)| r.center().x).fold(f32::NEG_INFINITY, f32::max);
    h.press_and_move(pos2(edge, 600.0), pos2(edge - 60.0, 600.0));
    dock_columns(&h.layout())[0].width
}

#[test]
fn a_column_resize_whose_release_was_lost_ends_when_the_system_reports_no_button_down() {
    let mut h = Harness::new();
    let width = press_on_the_main_column_divider(&mut h);
    for x in [900.0, 1000.0, 850.0, 1100.0] {
        h.frame_with_system_buttons(vec![Event::PointerMoved(pos2(x, 600.0))], false);
        assert_eq!(dock_columns(&h.layout())[0].width, width, "the column followed the pointer to {x}");
    }
    assert!(!h.ctx.input(|i| i.pointer.any_down()));
}

#[test]
fn a_column_resize_continues_while_the_system_reports_the_button_down() {
    let mut h = Harness::new();
    let width = press_on_the_main_column_divider(&mut h);
    h.frame_with_system_buttons(vec![Event::PointerMoved(pos2(900.0, 600.0))], true);
    assert_ne!(dock_columns(&h.layout())[0].width, width);
    assert!(h.ctx.input(|i| i.pointer.primary_down()));
}

#[test]
fn a_lost_release_after_the_pointer_moved_away_is_not_a_click() {
    let mut h = Harness::new();
    h.frame(vec![Event::PointerMoved(pos2(800.0, 600.0)), button(pos2(800.0, 600.0), true)]);
    h.frame_with_system_buttons(vec![Event::PointerMoved(pos2(1000.0, 600.0))], false);
    assert!(!h.ctx.input(|i| i.pointer.any_down()));
    assert!(!h.ctx.input(|i| i.pointer.any_click()), "the release at the press position made a click");
}

#[test]
fn control_channel_drags_keep_their_button_between_steps() {
    let mut h = Harness::new();
    h.frame(vec![Event::PointerMoved(pos2(800.0, 600.0)), button(pos2(800.0, 600.0), true)]);
    h.app.synthetic.push(button(pos2(820.0, 600.0), false));
    let mut input = egui::RawInput { events: vec![Event::PointerMoved(pos2(810.0, 600.0))], ..Default::default() };
    h.app.release_lost_buttons(&h.ctx, &mut input, false);
    assert_eq!(input.events, [Event::PointerMoved(pos2(810.0, 600.0))]);
}

#[test]
fn window_menu_panels_without_a_place_open_in_their_icon_group_or_float() {
    let mut h = Harness::new();
    // A saved layout that lost Swatches without remembering where it was.
    let mut layout = h.layout();
    layout.apply(Action::Close { panel: "swatches".into() }).unwrap();
    h.app.ui.docking = Some(layout);
    h.app.ui.docking_hidden.clear();
    h.app.run("window.panel", json!({"panel": "Swatches"})).unwrap();
    let layout = h.layout();
    assert!(in_iconic_column(&layout, "swatches"), "back in the icon column");
    let group = dock_columns(&layout).iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(&"swatches".to_string())).unwrap().panels.clone();
    assert!(group.contains(&"brushes".to_string()), "with its icon group: {group:?}");
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("swatches"), "its flyout opens");
    // A panel of no icon group floats.
    h.app.run("window.panel", json!({"panel": "openType"})).unwrap();
    assert!(floating_window(&h.layout(), "openType").is_some());
    h.frame(vec![]);
    assert!(h.tab("openType").is_some());
}

#[test]
fn f7_hides_and_restores_the_expanded_layers_tab() {
    let mut h = Harness::new();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"), "the first press activates the tab");
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(!showing(&h.app.ui, "layers"), "a second press hides the active tab");
    assert!(h.app.ui.docking_hidden.contains_key("layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"), "the next press restores it");
    assert_eq!(main_panels(&h.layout()), ["properties", "layers", "libraries"]);

    h.app.run("window.panel.collapse", json!({"panel": "layers", "collapsed": true})).unwrap();
    let layout = h.layout();
    let group = dock_columns(&layout).iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(&"layers".to_string())).unwrap();
    assert!(group.minimized);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    let layout = h.layout();
    let group = dock_columns(&layout).iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(&"layers".to_string())).unwrap();
    assert!(!group.minimized, "the first press expands a collapsed group");
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(!showing(&h.app.ui, "layers"), "the following press hides the expanded group");
}

#[test]
fn f7_activates_a_background_tab_before_hiding_it() {
    let mut h = Harness::new();
    h.app.run("window.panel.move", json!({"panel": "layers", "anchor": "properties"})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
    let before = group_of(&h.layout(), &"layers".into());
    assert!(!showing(&h.app.ui, "layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string())));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"));
    assert_eq!(group_of(&h.layout(), &"layers".into()), before);
}

#[test]
fn f7_hides_and_restores_the_whole_expanded_group() {
    let mut h = Harness::new();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string())));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
    assert!(showing(&h.app.ui, "layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    h.app.run("window.panel", json!({"panel": "properties", "toggle": true})).unwrap();
    assert_eq!(group_of(&h.layout(), &"properties".into()), ["properties", "layers", "libraries"]);
    assert!(showing(&h.app.ui, "properties"));
}

/// A tool's request for the Transform panel (a double-click on a shape's corner widget) shows and
/// selects it wherever it is: closed, floating, behind another tab, in an icon column or in a dock
/// hidden by Tab. Asking again keeps it on show.
#[test]
fn a_tools_panel_request_shows_the_panel_and_a_second_keeps_it_shown() {
    let mut h = Harness::new();
    let ask_twice = |h: &mut Harness, case: &str| {
        for n in 1..=2 {
            crate::canvas::apply_requests(&mut h.app, Ok(vec![vectorcraft_engine::UiRequest::Panel("transform".into())]));
            h.frame(vec![]);
            assert!(visible(&h.app.ui, "transform"), "{case}: request {n}");
        }
    };
    if h.layout().contains(&"transform".to_string()) {
        h.app.run("window.panel.close", json!({"panel": "transform"})).unwrap();
    }
    assert!(!visible(&h.app.ui, "transform"));
    ask_twice(&mut h, "closed");
    h.app.run("window.panel.float", json!({"panel": "transform"})).unwrap();
    h.frame(vec![]);
    ask_twice(&mut h, "floating");
    h.app.run("window.panel.move", json!({"panel": "transform", "anchor": "properties"})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
    h.frame(vec![]);
    assert!(!visible(&h.app.ui, "transform"));
    ask_twice(&mut h, "behind another tab");
    h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    h.frame(vec![]);
    assert!(in_iconic_column(&h.layout(), "transform"));
    ask_twice(&mut h, "in an icon column");
    h.app.ui.dock = false;
    assert!(!visible(&h.app.ui, "transform"));
    ask_twice(&mut h, "in a hidden dock");
}

#[test]
fn f7_restores_only_tabs_hidden_with_the_group() {
    let mut h = Harness::new();
    h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
    assert!(h.app.ui.docking_hidden.contains_key("libraries"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(["properties", "layers"].iter().all(|panel| h.app.ui.docking_hidden.contains_key(*panel)));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers"]);
    assert!(!h.layout().contains(&"libraries".to_string()));
    assert!(showing(&h.app.ui, "layers"));
}

#[test]
fn independently_reopened_tab_is_not_restored_by_the_group_key() {
    let mut h = Harness::new();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(["properties", "layers", "libraries"].iter().all(|panel| h.app.ui.docking_hidden.contains_key(*panel)));
    assert!(["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string())));
    h.app.run("window.panel.activate", json!({"panel": "libraries"})).unwrap();
    assert!(h.layout().contains(&"libraries".to_string()));
    assert!(!h.app.ui.docking_hidden.contains_key("libraries"));
    assert!(["properties", "layers"].iter().all(|panel| h.app.ui.docking_hidden.contains_key(*panel)));
    h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
    assert!(h.app.ui.docking_hidden.contains_key("libraries"));
    assert!(!h.layout().contains(&"libraries".to_string()));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers"]);
    assert!(!h.layout().contains(&"libraries".to_string()));
    assert!(showing(&h.app.ui, "layers"));
}

#[test]
fn f7_hides_and_restores_a_floating_tab_group() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    let before = h.layout();
    let window = floating_window(&before, "layers").unwrap();
    let rect = window.rect;
    assert_eq!(group_of(&before, &"layers".into()), ["properties", "layers", "libraries"]);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(floating_window(&h.layout(), "layers").is_none());
    assert!(["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string())));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(floating_window(&h.layout(), "layers").unwrap().rect, rect);
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
    assert!(showing(&h.app.ui, "layers"));
}

#[test]
fn showing_a_hidden_floating_group_returns_its_window_panels() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(floating_window(&h.layout(), "layers").is_none());
    let reply = h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    assert_eq!(reply, json!({"floating": ["properties", "layers", "libraries"]}), "only the restored window's panels");
}

#[test]
fn f7_toggles_a_multi_tab_icon_flyout() {
    let mut h = Harness::new();
    h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    h.frame(vec![]);
    assert!(in_iconic_column(&h.layout(), "layers"));
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.open_panel, None);
    assert!(in_iconic_column(&h.layout(), "layers"));
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
}

#[test]
fn panel_function_keys_toggle_icon_flyouts_and_floating_groups() {
    let mut h = Harness::new();
    h.press(egui::Key::F5, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("brushes"));
    h.press(egui::Key::F5, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.open_panel, None);
    h.press(egui::Key::F5, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("brushes"));

    h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    let rect = floating_window(&h.layout(), "layers").unwrap().rect;
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(floating_window(&h.layout(), "layers").is_none());
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(floating_window(&h.layout(), "layers").unwrap().rect, rect);
    h.app.run("window.panel.dock", json!({"panel": "layers"})).unwrap();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(!showing(&h.app.ui, "layers"), "the expanded dock also toggles");
}

/// Window › Layers: the menu item's command and params.
fn window_layers() -> Value {
    json!({"panel": "layers", "toggle": true})
}

fn checked(h: &Harness, params: &Value) -> Option<bool> {
    crate::menus::checked(&h.app, "window.panel", params)
}

/// With the panels hidden by Tab, Window › Layers has no check mark, and choosing it shows the dock
/// with Layers where it was instead of closing the tab; the toolbar and the Control bar stay hidden
/// until the next Tab shows them.
#[test]
fn window_panel_from_tab_hidden_dock_reveals_named_front_tab() {
    let mut h = Harness::new();
    let item = window_layers();
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    assert_eq!(checked(&h, &item), Some(true));
    let before = h.layout();
    h.app.ui.toolbar = true;
    h.app.ui.control_bar = true;
    h.app.run("window.dock", json!({})).unwrap();
    assert!(!h.app.ui.dock && !h.app.ui.toolbar && !h.app.ui.control_bar);
    assert_eq!(checked(&h, &item), Some(false), "no check mark while Tab hides the panels");
    h.app.run("window.panel", item.clone()).unwrap();
    assert!(h.app.ui.dock && visible(&h.app.ui, "layers"), "Layers shows");
    assert_eq!(h.layout(), before, "in its place");
    assert!(!h.app.ui.toolbar && !h.app.ui.control_bar, "the toolbar and the Control bar stay hidden");
    assert_eq!(checked(&h, &item), Some(true));
    h.app.run("window.dock", json!({})).unwrap();
    assert!(h.app.ui.dock && h.app.ui.toolbar && h.app.ui.control_bar, "the next Tab shows the bars the first Tab hid");
    h.app.run("window.panel", item.clone()).unwrap();
    assert!(!h.layout().contains(&"layers".to_string()), "a visible panel closes");
    assert!(h.layout().contains(&"properties".to_string()), "only the panel named");
    assert_eq!(checked(&h, &item), Some(false));
}

/// With the panels hidden by Shift+Tab, Window › Layers and its plain command show a floating
/// Layers panel again where it was instead of closing it.
#[test]
fn window_panel_from_shift_tab_hidden_floating_window_reveals_it() {
    let mut h = Harness::new();
    let item = window_layers();
    h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    let rect = floating_window(&h.layout(), "layers").map(|w| w.rect);
    assert!(rect.is_some());
    h.app.ui.toolbar = true;
    h.app.ui.control_bar = true;
    for params in [item.clone(), json!({"panel": "layers"})] {
        h.app.run("window.panels", json!({})).unwrap();
        assert!(!h.app.ui.dock && h.app.ui.toolbar && h.app.ui.control_bar);
        assert_eq!(checked(&h, &item), Some(false), "{params}: no check mark while Shift+Tab hides the panels");
        h.app.run("window.panel", params.clone()).unwrap();
        assert!(h.app.ui.dock && visible(&h.app.ui, "layers"), "{params}: Layers shows");
        assert_eq!(floating_window(&h.layout(), "layers").map(|w| w.rect), rect, "{params}: where it was");
    }
}

/// While the dock is hidden, Window › Layers opens a flyout that was open in the icon column
/// instead of putting it away; a closed panel comes back and a background tab comes to the front.
/// With the dock showing, choosing the checked item again hides only that panel.
#[test]
fn window_panel_hidden_iconic_and_closed_cases() {
    let mut h = Harness::new();
    let item = window_layers();
    h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    h.app.run("window.panel", item.clone()).unwrap();
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
    assert_eq!(checked(&h, &item), Some(true));
    h.app.ui.dock = false;
    assert_eq!(checked(&h, &item), Some(false), "no check mark with the dock hidden");
    h.app.run("window.panel", item.clone()).unwrap();
    assert!(h.app.ui.dock, "the dock shows");
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"), "the flyout stays open");
    assert_eq!(checked(&h, &item), Some(true));
    h.app.run("window.panel", item.clone()).unwrap();
    assert_eq!(h.app.ui.open_panel, None, "a visible flyout puts away");

    let mut h = Harness::new();
    let libraries = json!({"panel": "libraries", "toggle": true});
    h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
    h.app.run("window.dock", json!({})).unwrap();
    assert_eq!((checked(&h, &libraries), checked(&h, &item)), (Some(false), Some(false)));
    h.app.run("window.panel", libraries.clone()).unwrap();
    assert!(h.app.ui.dock && visible(&h.app.ui, "libraries"), "a closed panel comes back");
    h.app.run("window.panel", item.clone()).unwrap();
    assert!(visible(&h.app.ui, "layers") && !visible(&h.app.ui, "properties"), "a background tab comes to the front");
    assert_eq!((checked(&h, &libraries), checked(&h, &item)), (Some(false), Some(true)), "the tab in front is checked");
}

/// F7 with the panels hidden by Tab shows the dock with the Layers group instead of hiding the
/// group; the next F7 hides it.
#[test]
fn function_key_from_tab_hidden_dock_reveals_named_group() {
    let mut h = Harness::new();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(visible(&h.app.ui, "layers"));
    let before = h.layout();
    h.app.run("window.dock", json!({})).unwrap();
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(h.app.ui.dock && visible(&h.app.ui, "layers"), "F7 shows the hidden group");
    assert_eq!(h.layout(), before);
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string())), "the next press hides the group");
}

/// Presentation Mode hides every panel: no panel is checked, `window.panel` fails and changes
/// nothing, and F7 is ignored. The other full screen modes show the panels as Normal does.
#[test]
fn presentation_panel_command_reports_unavailable_without_mutating_layout() {
    let mut h = Harness::new();
    let item = window_layers();
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    h.app.run("window.panel.close", json!({"panel": "swatches"})).unwrap();
    h.app.run("view.presentation", json!({})).unwrap();
    assert_eq!(h.app.ui.screen_mode, 3);
    assert_eq!(checked(&h, &item), Some(false), "no check mark in Presentation Mode");
    let before = (h.layout(), h.app.ui.dock, h.app.ui.open_panel.clone(), h.app.ui.docking_hidden.len());
    for params in [item.clone(), json!({"panel": "layers"}), json!({"panel": "swatches"}), json!({"panel": "brushes"})] {
        let error = h.app.run("window.panel", params.clone()).unwrap_err();
        assert_eq!(error, "panels can't be shown in Presentation Mode", "{params}");
        assert_eq!((h.layout(), h.app.ui.dock, h.app.ui.open_panel.clone(), h.app.ui.docking_hidden.len()), before, "{params} changed nothing");
    }
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.screen_mode, 3);
    assert_eq!(h.layout(), before.0, "F7 is ignored");
    for mode in [1, 2] {
        h.app.run("view.screenMode", json!({"mode": mode})).unwrap();
        assert_eq!(checked(&h, &item), Some(true), "mode {mode}");
        h.app.run("window.panel", item.clone()).unwrap();
        assert!(!visible(&h.app.ui, "layers"), "mode {mode}: the checked item hides the panel");
        h.app.run("window.panel", item.clone()).unwrap();
        assert!(visible(&h.app.ui, "layers"), "mode {mode}: and shows it again");
    }
}

#[test]
fn other_function_keys_and_keyboard_guards_keep_their_routes() {
    let mut h = Harness::new();
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    let keymap = crate::shortcut_editor::current();
    for (chord, command) in [("F8", "ui.symbolOptions"), ("F12", "file.revert")] {
        let bound = crate::keymap::vc_chord(chord).and_then(|c| crate::keymap::lookup(&keymap.keymap, &c)).unwrap();
        assert_eq!(bound.id, command);
    }
    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    h.press(egui::Key::F8, egui::Modifiers::NONE);
    assert_eq!(h.app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("symbolOptions"));
    crate::dialogs::confirm(&mut h.app).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc.symbols.len(), 1);
    crate::menus::invoke(&mut h.app, "view.rotate", json!({}));
    assert!(h.app.ui.dialog.is_some());
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"), "F7 does nothing while a dialog is open");
    h.app.ui.dialog = None;

    let field = egui::Id::unique("panel-shortcut-text-focus");
    h.ctx.memory_mut(|m| m.request_focus(field));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(showing(&h.app.ui, "layers"), "F7 does nothing while a text field has focus");
    h.ctx.memory_mut(|m| m.surrender_focus(field));
    h.press(egui::Key::F7, egui::Modifiers::NONE);
    assert!(!showing(&h.app.ui, "layers"), "F7 toggles when the field releases focus");

    let dir = vectorcraft_testkit::temp_dir("panel-shortcut-revert");
    let path = dir.join(format!("panel-shortcut-revert-{}.vectorcraft", std::process::id()));
    let services = crate::Services {
        read: Some(Box::new(|p| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p, data| std::fs::write(p, data).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut saved = VectorcraftApp::new(Session::new(), services);
    saved.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    saved.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    saved.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
    saved.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap();
    assert_eq!(saved.session.doc().unwrap().doc.node_count(), 3);
    let ctx = egui::Context::default();
    let key = Event::Key { key: egui::Key::F12, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| crate::shortcuts::handle(&mut saved, ui.ctx()))
        .textures_delta
        .clear();
    assert_eq!(saved.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::confirm::KIND));
    crate::dialogs::confirm(&mut saved).unwrap();
    assert_eq!(saved.session.doc().unwrap().doc.node_count(), 2, "F12 reloads the saved document");
    std::fs::remove_dir_all(dir).unwrap();
}

fn collapse_to_icons_button(h: &Harness, panel: &str) -> Option<Rect> {
    let panel = panel.to_string();
    h.ctx.read_response(area_id().with(("header", &panel)).with(("collapse-to-icons", &panel))).map(|r| r.rect)
}

#[test]
fn floating_collapse_arrow_moves_the_whole_group_into_the_icon_rail() {
    let mut h = Harness::new();
    assert!(collapse_to_icons_button(&h, "properties").is_none(), "docked headers have no extra button");
    h.app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "layers"})).unwrap();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let before = h.layout();
    let members = group_of(&before, &"layers".into());
    let columns = end_columns(&before).len();
    let arrow = collapse_to_icons_button(&h, "layers").expect("floating header offers collapse to icons");
    assert!(arrow.width() <= 24.0 && arrow.height() <= 24.0, "a compact header control: {arrow:?}");
    h.click(arrow.center());
    let layout = h.layout();
    assert!(floating_window(&layout, "layers").is_none(), "the old floating window is removed");
    assert_eq!(group_of(&layout, &"layers".into()), members, "all tabs move together");
    assert!(members.iter().all(|p| in_iconic_column(&layout, p)));
    assert_eq!(end_columns(&layout).len(), columns, "reuse the existing icon rail");
    assert!(h.app.ui.open_panel.is_none(), "the group stays collapsed until its icon is clicked");
    let icon = h.hit(&Hit::Icon("layers".into())).expect("Layers now has a rail icon");
    h.click(icon.center());
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
    assert!(h.tab("layers").is_some(), "the collapsed group can reopen as a flyout");
    assert!(collapse_to_icons_button(&h, "layers").is_some(), "the reopened flyout also offers collapse to icons");
}

#[test]
fn floating_collapse_arrow_leaves_other_floating_groups_alone() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "color", "below": "layers"})).unwrap();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let arrow = collapse_to_icons_button(&h, "layers").expect("floating group has its own arrow");
    h.click(arrow.center());
    let layout = h.layout();
    assert!(in_iconic_column(&layout, "layers"));
    assert!(floating_window(&layout, "color").is_some(), "another group in the same window stays floating");
    assert!(!in_iconic_column(&layout, "color"));
    assert_eq!(main_panels(&layout), ["properties", "libraries"]);
}

#[test]
fn floating_collapse_arrow_creates_a_rail_when_none_exists_and_keeps_the_menu() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    let mut layout = h.layout();
    if let Some(Node::Split { second, .. }) = layout.root.as_mut()
        && let Node::Dock { columns, .. } = second.as_mut()
    {
        columns.retain(|column| !column.iconic);
    }
    assert!(icon_column(&end_columns(&layout)).is_none());
    h.app.ui.docking = Some(layout);
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let panel = "layers".to_string();
    let menu =
        h.ctx.read_response(area_id().with(("header", &panel)).with(("panel-menu", &panel))).expect("the existing panel menu remains available").rect;
    let arrow = collapse_to_icons_button(&h, "layers").expect("floating group offers collapse");
    assert!(!menu.intersects(arrow), "menu and arrow have separate hit targets");
    h.click(menu.center());
    assert!(egui::Popup::is_any_open(&h.ctx), "the existing menu still opens");
    h.press(egui::Key::Escape, egui::Modifiers::NONE);
    h.click(arrow.center());
    let layout = h.layout();
    assert!(in_iconic_column(&layout, "layers"));
    assert!(floating_window(&layout, "layers").is_none());
    assert!(h.app.ui.open_panel.is_none(), "collapse does not immediately open a flyout");
    assert_eq!(main_panels(&layout), ["properties", "libraries"]);
}

#[test]
fn rail_flyout_collapse_arrow_folds_the_panel_back_without_moving_it() {
    let mut h = Harness::new();
    let icon = h.hit(&Hit::Icon("swatches".into())).unwrap();
    h.click(icon.center());
    let before = h.layout();
    let arrow = collapse_to_icons_button(&h, "swatches").expect("opened rail panels offer collapse to icons");
    h.click(arrow.center());
    assert!(h.app.ui.open_panel.is_none());
    assert!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()).is_none());
    assert_eq!(h.layout(), before, "folding a flyout leaves the dock layout intact");
    let icon = h.hit(&Hit::Icon("swatches".into())).expect("the rail icon stays available");
    h.click(icon.center());
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("swatches"));
    assert!(h.tab("swatches").is_some());
}

#[test]
fn rail_flyout_collapse_arrow_closes_the_opening_icon_after_switching_tabs() {
    let mut h = Harness::new();
    h.app.ui.docking = Some(h.layout());
    let icon = h.hit(&Hit::Icon("swatches".into())).unwrap();
    h.click(icon.center());
    let brushes = h.tab("brushes").unwrap();
    h.click(brushes.center());
    assert_eq!(h.app.ui.open_panel.as_deref(), Some("swatches"));
    let before = h.layout();
    let arrow = collapse_to_icons_button(&h, "brushes").expect("the active tab has a collapse arrow");
    h.click(arrow.center());
    assert!(h.app.ui.open_panel.is_none());
    assert!(craft_ui::docking::flyout::<String>(&h.ctx, area_id()).is_none());
    assert_eq!(h.layout(), before, "all tabs and their active state stay in the rail");
}

/// The texts `panel`'s menu draws, in the harness's context.
fn panel_menu_texts(h: &mut Harness, panel: &str) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().trim().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let app = &mut h.app;
    let mut out = h.ctx.run_ui(egui::RawInput { screen_rect: Some(SCREEN), ..Default::default() }, |ui| {
        crate::panels::panel_menu_items(app, ui, panel);
    });
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn cycle_button(h: &Harness, panel: &str) -> Option<Rect> {
    let panel = panel.to_string();
    h.ctx.read_response(area_id().with(("header", &panel)).with(("cycle-display-sizes", &panel))).map(|r| r.rect)
}

/// Each panel with display sizes steps between the states its panel menu's Show/Hide Options
/// item sets, and the smallest size goes back to the largest.
#[test]
fn cycle_options_steps_through_the_states_the_panel_menu_sets() {
    let mut h = Harness::new();
    for panel in ["align", "attributes", "character", "color", "gradient", "info", "paragraph", "stroke", "transform"] {
        assert!(panel_menu_texts(&mut h, panel).iter().any(|t| t == "Hide Options"), "{panel} starts with its options");
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": panel})).unwrap();
        assert_eq!(r, json!({"panel": panel, "size": 1, "sizes": 2}), "{panel}");
        assert!(panel_menu_texts(&mut h, panel).iter().any(|t| t == "Show Options"), "{panel} hides its options");
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": panel})).unwrap();
        assert_eq!(r["size"], 0, "{panel}");
        assert!(panel_menu_texts(&mut h, panel).iter().any(|t| t == "Hide Options"), "{panel} shows them again");
    }
    // The Flattener Preview's options start hidden: its first step shows them, as Show Options does.
    assert!(!h.app.ui.flattener_preview.show_options);
    let r = h.app.run("window.panel.cycleOptions", json!({"panel": "Flattener Preview"})).unwrap();
    assert_eq!(r, json!({"panel": crate::panels::flattener_preview::ID, "size": 0, "sizes": 2}));
    assert!(h.app.ui.flattener_preview.show_options);
    h.app.run("window.panel.cycleOptions", json!({"panel": crate::panels::flattener_preview::ID})).unwrap();
    assert!(!h.app.ui.flattener_preview.show_options);
    // Transparency: thumbnails and options, thumbnails alone, neither, and back.
    let shown = |h: &mut Harness| {
        let t = panel_menu_texts(h, "transparency");
        (t.iter().any(|t| t == "Hide Thumbnails"), t.iter().any(|t| t == "Hide Options"))
    };
    assert_eq!(shown(&mut h), (true, true));
    for (size, want) in [(1, (true, false)), (2, (false, false)), (0, (true, true))] {
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": "transparency"})).unwrap();
        assert_eq!((r["size"].clone(), r["sizes"].clone()), (json!(size), json!(3)));
        assert_eq!(shown(&mut h), want, "size {size}");
    }
    // Options hidden with the thumbnails shown matches no size, so the next step is the largest.
    crate::panels::set_pstate(&h.ctx, "tr-hide-thumbs", true);
    assert_eq!(h.app.run("window.panel.cycleOptions", json!({"panel": "transparency"})).unwrap()["size"], 0);
    assert_eq!(shown(&mut h), (true, true));
}

/// An unknown panel, a panel without display sizes or a missing `panel` is an error, and the
/// panels keep their sizes.
#[test]
fn cycle_options_rejects_panels_without_sizes() {
    let mut h = Harness::new();
    for (params, error) in [
        (json!({"panel": "nope"}), "unknown panel `nope`"),
        (json!({}), "unknown panel ``"),
        (json!({"panel": 3}), "unknown panel ``"),
        (json!({"panel": "layers"}), "`layers` has no display sizes to cycle"),
        (json!({"panel": "swatches"}), "`swatches` has no display sizes to cycle"),
    ] {
        assert_eq!(h.app.run("window.panel.cycleOptions", params.clone()).unwrap_err(), error, "{params}");
    }
    assert!(panel_menu_texts(&mut h, "stroke").iter().any(|t| t == "Hide Options"));
    assert!(!h.app.ui.color_hide_options);
}

/// The double triangle shows on the tab strip of a panel with display sizes, docked or floating,
/// and a click on it shows the panel's next size; other panels have none.
#[test]
fn the_double_triangle_on_the_tab_strip_cycles_the_display_sizes() {
    let mut h = Harness::new();
    h.ctx.enable_accesskit();
    h.frame(vec![]);
    assert!(cycle_button(&h, "properties").is_none() && cycle_button(&h, "layers").is_none(), "the main column's panels have no sizes");
    h.app.run("window.panel.float", json!({"panel": "stroke"})).unwrap();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let control = cycle_button(&h, "stroke").expect("a floating Stroke panel has the double triangle");
    assert!(h.nodes.iter().any(|(_, node)| node.label() == Some("Cycle Display Sizes")), "screen readers name it");
    let menu = h.ctx.read_response(area_id().with(("header", &"stroke".to_string())).with(("panel-menu", &"stroke".to_string()))).unwrap().rect;
    assert!(!menu.intersects(control), "the double triangle and the menu have separate hit targets");
    h.click(control.center());
    assert!(crate::panels::pstate::<bool>(&h.ctx, "stroke-hide-options"), "a click hides the options");
    assert!(floating_window(&h.layout(), "stroke").is_some(), "the panel stays where it is");
    h.click(control.center());
    assert!(!crate::panels::pstate::<bool>(&h.ctx, "stroke-hide-options"), "a second click shows them again");
    // A docked panel's tab strip has it too, and a floating panel without sizes does not.
    h.app.run("window.panel.dock", json!({"panel": "color", "anchor": "layers", "zone": "below"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let control = cycle_button(&h, "color").expect("a docked Color panel has the double triangle");
    h.click(control.center());
    assert!(h.app.ui.color_hide_options);
    assert!(cycle_button(&h, "swatches").is_none());
}
