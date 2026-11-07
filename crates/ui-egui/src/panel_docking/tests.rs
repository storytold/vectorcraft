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
    for command in ["type.highlightKerning", "type.highlightTracking", "type.highlightFontName"] {
        h.app.run(command, json!({})).unwrap();
        assert_eq!(h.app.dock_rt.raise.as_deref(), Some("character"), "{command}");
        h.frame(vec![]);
        assert!(showing(&h.app.ui, "character"), "{command} leaves the panel showing");
    }
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
    let zone = h.ctx.data(|d| d.get_temp::<Rect>(crate::tools_float::tools_zone_id())).expect("the Tools panel's docking strip");
    let title = zone.min + vec2(20.0, 12.0);
    h.drag(title, pos2(600.0, 400.0));
    let [x, y] = h.app.ui.toolbar_pos.expect("the Tools panel floats");
    assert!((300.0..700.0).contains(&x) && (250.0..450.0).contains(&y), "{x} {y}");
    let title = pos2(x + 20.0, y + 7.0);
    h.drag(title, zone.center());
    assert_eq!(h.app.ui.toolbar_pos, None, "dropped on the left edge, it docks");
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
