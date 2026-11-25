//! The dock area as the user sees it: whole-window frames driven by pointer and key events.

use egui::{Event, PointerButton, Pos2, Rect, ViewportId, pos2, vec2};
use vectorcraft_engine::Session;

use craft::keymap::Trigger;
use craft::ui::runtime::commands;
use craft::ui::runtime::frame::{ChromeToggle, request_chrome_toggle};

use super::*;
use crate::tests_synthetic::key_frame;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 1000.0));

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>,
}

impl Harness {
    /// Create a blank document and settle the initial docking layout over three frames.
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let ctx = crate::testing::ui_context();
        ctx.enable_accesskit();
        let mut h = Self { app, ctx, time: 0.0, nodes: Vec::new() };
        h.frames(3);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let input = egui::RawInput { time: Some(self.time), events, screen_rect: Some(SCREEN), ..Default::default() };
        self.run(input);
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(vec![]);
        }
    }

    /// Draw one application frame and retain its accessibility output for assertions.
    fn run(&mut self, input: egui::RawInput) {
        let out = crate::testing::app_frame(&mut self.app, &self.ctx, input);
        self.nodes = out.platform_output.accesskit_update.map_or_else(Vec::new, |update| update.nodes);
    }

    /// The rect of the node of `role` named like `panel` (its title, ignoring case and spaces) in the last frame.
    // PLAN: assumed the dock names a tab and an icon by the panel's title, with the roles Tab and Button.
    fn named(&self, panel: &str, role: egui::accesskit::Role) -> Option<Rect> {
        // A library panel's title is its library's name.
        let title = crate::panels::library_panel::tab_label(&self.app, panel).unwrap_or_else(|| panel.to_owned());
        let wanted = title.to_lowercase().replace(' ', "");
        self.nodes
            .iter()
            .filter(|(_, node)| node.role() == role && node.label().is_some_and(|l| l.to_lowercase().replace(' ', "") == wanted))
            .find_map(|(_, node)| node.bounds())
            .map(|b| Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
    }

    /// The centre of the text field named `label` in the last frame.
    fn text_field(&self, label: &str) -> Option<Pos2> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::TextInput && n.label() == Some(label))
            .find_map(|(_, n)| n.bounds())
            .map(|b| pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
    }

    fn tab(&self, panel: &str) -> Option<Rect> {
        self.named(panel, egui::accesskit::Role::Tab)
    }

    fn icon(&self, panel: &str) -> Option<Rect> {
        self.named(panel, egui::accesskit::Role::Button)
    }

    /// The canvas rect as the last frame drew it.
    fn canvas(&self) -> Rect {
        self.app.canvas_view().expect("the canvas was drawn").rect
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

    fn f7(&mut self) {
        self.press(egui::Key::F7, egui::Modifiers::NONE);
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
        self.frames(2);
    }

    fn layout(&self) -> Layout<String> {
        layout_of(&self.app.ui).clone()
    }

    /// Whether the workspace remembers `panel` as hidden.
    fn hidden(&self, panel: &str) -> bool {
        self.app.ui.dock_workspace.hidden().iter().any(|(id, _)| id == panel)
    }

    /// Puts `layout` in place of the workspace's, keeping the closed and floated panels' records.
    fn set_layout(&mut self, layout: Layout<String>) {
        let ws = &self.app.ui.dock_workspace;
        let next = Workspace::from_parts(layout, ws.hidden().to_vec(), ws.floated().to_vec());
        Arc::make_mut(&mut self.app.ui.dock_workspace).replace(next);
    }

    /// Hides the panels the way Tab (`Panels`) or Shift+Tab (`PanelsKeepTools`) does.
    fn hide_panels(&mut self, toggle: ChromeToggle) {
        request_chrome_toggle(&self.ctx, ViewportId::ROOT, toggle);
        self.frames(2);
        assert!(!panels_shown(&self.app), "the panels are hidden");
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn main_panels(layout: &Layout<String>) -> Vec<String> {
    dock_columns(layout).first().map(|c| c.groups.iter().flat_map(|g| g.panels.clone()).collect()).unwrap_or_default()
}

/// The columns of the dock at the window's end edge, from the edge inwards.
fn end_columns(layout: &Layout<String>) -> &[Column<String>] {
    edge_columns(layout, DockEdge::End)
}

/// The columns of the end dock, to set a layout no command makes.
fn end_dock_mut(layout: &mut Layout<String>) -> Option<&mut Vec<Column<String>>> {
    let path = layout.dock_path(DockEdge::End)?;
    let mut node = layout.root.as_mut()?;
    for second_child in path {
        let Node::Split { first, second, .. } = node else { return None };
        node = if second_child { second } else { first };
    }
    match node {
        Node::Dock { columns, .. } => Some(columns),
        _ => None,
    }
}

/// The panels of the group holding `panel`, docked or floating.
fn group_of(layout: &Layout<String>, panel: &String) -> Vec<String> {
    layout.place(panel).and_then(|place| layout.group(&place)).map(|group| group.panels.clone()).unwrap_or_default()
}

/// Is `panel` in a column collapsed to icons?
fn in_iconic_column(layout: &Layout<String>, panel: &str) -> bool {
    layout.opens_in_flyout(&panel.to_owned())
}

/// Answers the topmost open dialog with OK or Cancel, then runs the frame's dialog pump. A held OK is answered
/// through the pump.
fn answer(app: &mut VectorcraftApp, ok: bool) {
    let ctx = app.ui_ctx.clone().expect("a frame ran");
    let id = app.dialogs().expect("a window").with(|host| host.open_dialogs().last().map(|d| d.id)).expect("a dialog is open");
    if ok {
        crate::tests::picks::answer_dialog(app, id, &[], true);
    } else {
        assert!(app.dialogs().expect("a window").with(|host| host.cancel_by_id(&ctx, id)), "the dialog took the answer");
        crate::dialogs::host::pump(app, &ctx);
    }
    let still = app.dialogs().expect("a window").with(|host| host.open_dialogs().iter().any(|d| d.id == id));
    assert!(!still, "the dialog took the answer");
}

fn first_column_iconic(h: &Harness) -> bool {
    let layout = h.layout();
    dock_columns(&layout).first().is_some_and(|c| c.iconic)
}

#[test]
fn the_default_layout_is_the_canvas_beside_the_main_column_and_the_icon_column() {
    let mut h = Harness::new();
    let layout = h.layout();
    assert!(layout.contains(&CANVAS.to_string()), "the canvas is in the layout");
    let columns = dock_columns(&layout);
    assert_eq!(main_panels(&layout), ["properties", "layers", "libraries"]);
    let icons = columns.get(1).expect("an icon column");
    assert!(icons.iconic && !columns[0].iconic);
    assert_eq!(icons.groups.len(), ICON_PANEL_GROUPS.len());
    let properties = h.tab("properties").expect("the main column's tabs are drawn");
    let swatches = h.icon("swatches").expect("the icon column is drawn");
    assert!(swatches.right() <= properties.left(), "the icon column is inside the main column");
    // The canvas takes the rest and gets the pointer: the Rectangle tool draws on it.
    let canvas = h.canvas();
    assert!(canvas.right() <= swatches.left() + 1.0 && canvas.width() > 800.0);
    h.app.select_tool("rectangle");
    let count = |h: &Harness| h.app.session.active().unwrap().doc.node_count();
    let before = count(&h);
    h.drag(canvas.center(), canvas.center() + vec2(60.0, 40.0));
    assert_eq!(count(&h), before + 1);
}

/// The floating window holding `panel`, by its panels.
fn window_of(layout: &Layout<String>, panel: &str) -> Vec<String> {
    floating_window(layout, panel).map(window_panels).unwrap_or_default()
}

fn column_panels(layout: &Layout<String>) -> Vec<Vec<String>> {
    end_columns(layout).iter().map(|c| c.groups.iter().flat_map(|g| g.panels.clone()).collect()).collect()
}

/// Window › Layers: the menu item's command and params.
fn window_layers() -> Value {
    json!({"panel": "layers", "toggle": true})
}

fn checked(h: &Harness, params: &Value) -> Option<bool> {
    crate::menus::checked(&h.app, "window.panel", params)
}

fn group_hidden_all(h: &Harness) -> bool {
    ["properties", "layers", "libraries"].iter().all(|panel| !h.layout().contains(&panel.to_string()))
}

type Case = (&'static str, fn(&mut Harness));

/// Each panel command (and key) gives the placement it gave before the dock was the framework's: the
/// command goes in, the placement comes out of the layout.
#[test]
fn panel_commands_give_the_placement_the_old_tests_checked() {
    let cases: [Case; 21] = [
        ("reveal brings the panel to the front of its open flyout", |h| {
            h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
            h.app.run("window.panel", json!({"panel": "properties"})).unwrap();
            h.frames(1);
            apply(&mut h.app, Some(&h.ctx), vec![Action::activate("layers".into())]).unwrap();
            h.frames(1);
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("properties"), "the flyout is still named for its opening icon");
            assert_eq!(flyout_tab(&h.layout(), "properties").as_deref(), Some("layers"), "Layers is in front");
            reveal_panel(&mut h.app, "properties").unwrap();
            h.frames(1);
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("properties"), "the flyout stays open");
            assert_eq!(flyout_tab(&h.layout(), "properties").as_deref(), Some("properties"), "Properties is in front");
            reveal_panel(&mut h.app, "properties").unwrap();
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("properties"), "revealed while in front, it stays");
        }),
        ("a character field shortcut raises the floating character panel", |h| {
            let id = h.app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX"})).unwrap()["id"].clone();
            h.app.run("select.set", json!({"ids": [id]})).unwrap();
            h.app.run("window.panel.float", json!({"panel": "character"})).unwrap();
            h.frames(1);
            assert!(showing(&h.app.ui, "character"));
            assert_eq!(h.app.dock_rt.raise, None);
            for command in ["type.highlightKerning", "type.highlightTracking", "type.highlightFontName", "type.highlightFontSize"] {
                h.app.run(command, json!({})).unwrap();
                assert_eq!(h.app.dock_rt.raise.as_deref(), Some("character"), "{command}");
                h.frames(1);
                assert!(showing(&h.app.ui, "character"), "{command} leaves the panel showing");
            }
        }),
        ("a character field shortcut ends the hide of the panels", |h| {
            let id = h.app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX"})).unwrap()["id"].clone();
            h.app.run("select.set", json!({"ids": [id]})).unwrap();
            h.app.run("window.panel.float", json!({"panel": "character"})).unwrap();
            h.frames(1);
            let rect = floating_window(&h.layout(), "character").map(|w| w.rect);
            h.hide_panels(ChromeToggle::Panels);
            h.app.run("type.highlightKerning", json!({})).unwrap();
            h.frames(2);
            assert!(panels_shown(&h.app) && visible(&h.app, "character"), "the panel shows");
            assert_eq!(floating_window(&h.layout(), "character").map(|w| w.rect), rect, "where it was");
            // In Presentation Mode the shortcut still focuses the field, and the panel shows when the mode ends.
            h.app.ui.screen_mode = 3;
            assert_eq!(h.app.run("type.highlightTracking", json!({})).unwrap(), json!({"field": "tracking"}));
            h.app.ui.screen_mode = 0;
            assert!(visible(&h.app, "character"), "the panel stays in place");
        }),
        ("the dock collapses to icons and back", |h| {
            assert!(!first_column_iconic(h));
            h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
            h.frames(1);
            assert!(first_column_iconic(h) && h.icon("properties").is_some());
            h.app.run("window.collapseDock", json!({"collapsed": false})).unwrap();
            h.frames(1);
            assert!(!first_column_iconic(h) && h.tab("properties").is_some());
        }),
        ("float onto, above and below stack with a floating group", |h| {
            h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
            h.app.run("window.panel.float", json!({"panel": "stroke", "onto": "Swatches"})).unwrap();
            assert_eq!(group_of(&h.layout(), &"stroke".into()), vec!["swatches".to_string(), "stroke".into()], "a tab of its group");
            h.app.run("window.panel.float", json!({"panel": "color", "below": "swatches"})).unwrap();
            h.app.run("window.panel.float", json!({"panel": "gradient", "above": "swatches"})).unwrap();
            let layout = h.layout();
            let window = window_of(&layout, "swatches");
            for panel in ["color", "gradient", "stroke"] {
                assert!(window.contains(&panel.to_string()), "{panel} in the window: {window:?}");
            }
            assert_eq!(group_of(&layout, &"color".into()), vec!["color".to_string()], "a group of its own below");
            assert_eq!(group_of(&layout, &"gradient".into()), vec!["gradient".to_string()]);
            h.frames(1);
            // The window floats at the theme's float size, where the strip of Swatches' group shows its front tab,
            // Stroke, and lists Swatches in its overflow.
            let (top, middle, bottom) = (h.tab("gradient").unwrap(), h.tab("stroke").unwrap(), h.tab("color").unwrap());
            assert!(top.top() < middle.top() && middle.top() < bottom.top(), "gradient above, color below");
            // `onto` names a floating group; a docked one is refused.
            assert!(h.app.run("window.panel.float", json!({"panel": "stroke", "onto": "layers"})).is_err());
        }),
        ("float collapsed minimizes its window", |h| {
            h.app.run("window.panel.float", json!({"panel": "swatches", "collapsed": true})).unwrap();
            assert!(floating_window(&h.layout(), "swatches").is_some() && group_minimized(&h.layout(), "swatches"));
            h.app.run("window.panel.float", json!({"panel": "swatches", "collapsed": false})).unwrap();
            assert!(floating_window(&h.layout(), "swatches").is_some() && !group_minimized(&h.layout(), "swatches"));
            let r = h.app.run("window.panel.collapse", json!({"panel": "Layers"})).unwrap();
            assert_eq!((r["panel"].clone(), r["collapsed"].clone()), (json!("layers"), json!(true)), "a docked group minimizes too");
        }),
        ("dock column n docks a column at that place", |h| {
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
        }),
        ("dock icon puts a panel in the icon column", |h| {
            h.app.run("window.panel.dock", json!({"panel": "layers", "icon": true})).unwrap();
            let layout = h.layout();
            assert!(in_iconic_column(&layout, "layers"));
            assert!(!main_panels(&layout).contains(&"layers".to_string()));
            assert_eq!(group_of(&layout, &"layers".into()), vec!["layers".to_string()], "a row of its own");
        }),
        ("a docked column moves to a later place and back", |h| {
            h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
            h.app.run("window.panel.dock", json!({"panel": "color", "column": 2})).unwrap();
            assert_eq!(column_panels(&h.layout())[2..], [vec!["swatches".to_string()], vec!["color".to_string()]]);
            h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 2})).unwrap();
            assert_eq!(column_panels(&h.layout())[2..], [vec!["color".to_string()], vec!["swatches".to_string()]], "swatches moved inwards");
            h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
            assert_eq!(column_panels(&h.layout())[2..], [vec!["swatches".to_string()], vec!["color".to_string()]], "and back");
            let before = h.layout();
            h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
            assert_eq!(h.layout(), before, "already there");
        }),
        ("column places and icon count from the icon column", |h| {
            // A layout converted from an old dock split side by side, with the dock collapsed: two
            // collapsed tree columns before the icon column. No command makes it, so it is set directly.
            let mut layout = h.layout();
            {
                let columns = end_dock_mut(&mut layout).expect("the layout has an end dock");
                let libraries = columns[0].groups[0].panels.pop().unwrap();
                assert_eq!(libraries, "libraries");
                columns.insert(1, Column::new(vec![Group::new(vec![libraries], 0)]).with_iconic(true));
                columns[0].iconic = true;
            }
            assert!(layout.validate().is_ok());
            h.set_layout(layout);
            h.app.run("window.panel.dock", json!({"panel": "layers", "icon": true})).unwrap();
            let columns = column_panels(&h.layout());
            assert!(columns[2].contains(&"layers".to_string()), "{columns:?}");
            h.app.run("window.panel.dock", json!({"panel": "swatches", "column": 1})).unwrap();
            let columns = column_panels(&h.layout());
            assert_eq!(columns[..2], [vec!["properties".to_string()], vec!["libraries".to_string()]]);
            assert_eq!(columns[3], ["swatches"], "next to the icon column: {columns:?}");

            // No icon column: `icon` makes one.
            let mut layout = h.layout();
            {
                let columns = end_dock_mut(&mut layout).expect("the layout has an end dock");
                columns.remove(2);
            }
            h.set_layout(layout);
            assert_eq!(icon_column(end_columns(&h.layout())), None);
            h.app.run("window.panel.dock", json!({"panel": "properties", "icon": true})).unwrap();
            let layout = h.layout();
            assert!(in_iconic_column(&layout, "properties"));
            assert!(icon_column(end_columns(&layout)).is_none_or(|k| column_panels(&layout)[k].contains(&"properties".to_string())));
        }),
        ("F7 hides and restores the expanded Layers tab", |h| {
            h.f7();
            assert!(showing(&h.app.ui, "layers"), "the first press activates the tab");
            h.f7();
            assert!(!showing(&h.app.ui, "layers"), "a second press hides the active tab");
            assert!(h.hidden("layers"));
            h.f7();
            assert!(showing(&h.app.ui, "layers"), "the next press restores it");
            assert_eq!(main_panels(&h.layout()), ["properties", "layers", "libraries"]);

            h.app.run("window.panel.collapse", json!({"panel": "layers", "collapsed": true})).unwrap();
            let group_minimized = |h: &Harness| {
                let layout = h.layout();
                dock_columns(&layout).iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(&"layers".to_string())).unwrap().minimized
            };
            assert!(group_minimized(h));
            h.f7();
            assert!(!group_minimized(h), "the first press expands a collapsed group");
            h.f7();
            assert!(!showing(&h.app.ui, "layers"), "the following press hides the expanded group");
        }),
        ("F7 activates a background tab before hiding it", |h| {
            h.app.run("window.panel.move", json!({"panel": "layers", "anchor": "properties"})).unwrap();
            h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
            let before = group_of(&h.layout(), &"layers".into());
            assert!(!showing(&h.app.ui, "layers"));
            h.f7();
            assert!(showing(&h.app.ui, "layers"));
            h.f7();
            assert!(group_hidden_all(h));
            h.f7();
            assert!(showing(&h.app.ui, "layers"));
            assert_eq!(group_of(&h.layout(), &"layers".into()), before);
        }),
        ("F7 hides and restores the whole expanded group", |h| {
            h.f7();
            h.f7();
            assert!(group_hidden_all(h));
            h.f7();
            assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
            assert!(showing(&h.app.ui, "layers"));
            h.f7();
            h.app.run("window.panel", json!({"panel": "properties", "toggle": true})).unwrap();
            assert_eq!(group_of(&h.layout(), &"properties".into()), ["properties", "layers", "libraries"]);
            assert!(showing(&h.app.ui, "properties"));
        }),
        ("a tool's panel request shows the panel and a second keeps it shown", |h| {
            // A tool's request for the Transform panel (a double-click on a shape's corner widget) shows and
            // selects it wherever it is: closed, floating, behind another tab, in an icon column or in a hidden dock.
            let ask_twice = |h: &mut Harness, case: &str| {
                for n in 1..=2 {
                    crate::canvas::apply_requests(&mut h.app, Ok(vec![vectorcraft_engine::UiRequest::Panel("transform".into())]));
                    h.frames(2);
                    assert!(visible(&h.app, "transform"), "{case}: request {n}");
                }
            };
            if h.layout().contains(&"transform".to_string()) {
                h.app.run("window.panel.close", json!({"panel": "transform"})).unwrap();
            }
            assert!(!visible(&h.app, "transform"));
            ask_twice(h, "closed");
            h.app.run("window.panel.float", json!({"panel": "transform"})).unwrap();
            h.frames(1);
            ask_twice(h, "floating");
            h.app.run("window.panel.move", json!({"panel": "transform", "anchor": "properties"})).unwrap();
            h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
            h.frames(1);
            assert!(!visible(&h.app, "transform"));
            ask_twice(h, "behind another tab");
            h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
            h.frames(1);
            assert!(in_iconic_column(&h.layout(), "transform"));
            ask_twice(h, "in an icon column");
            h.hide_panels(ChromeToggle::Panels);
            ask_twice(h, "in a hidden dock");
            assert!(panels_shown(&h.app), "the request ends the hide");
        }),
        ("F7 hides and restores a floating tab group", |h| {
            h.app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
            h.app.run("window.panel.activate", json!({"panel": "layers"})).unwrap();
            h.frames(1);
            let before = h.layout();
            let rect = floating_window(&before, "layers").unwrap().rect;
            assert_eq!(group_of(&before, &"layers".into()), ["properties", "layers", "libraries"]);
            h.f7();
            assert!(floating_window(&h.layout(), "layers").is_none());
            assert!(group_hidden_all(h));
            h.f7();
            assert_eq!(floating_window(&h.layout(), "layers").unwrap().rect, rect);
            assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
            assert!(showing(&h.app.ui, "layers"));
        }),
        ("F7 toggles a multi-tab icon flyout", |h| {
            h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
            h.frames(1);
            assert!(in_iconic_column(&h.layout(), "layers"));
            assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
            h.f7();
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
            h.f7();
            assert_eq!(h.app.ui.open_panel, None);
            assert!(in_iconic_column(&h.layout(), "layers"));
            assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers", "libraries"]);
        }),
        ("panel function keys toggle icon flyouts and floating groups", |h| {
            h.press(egui::Key::F5, egui::Modifiers::NONE);
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("brushes"));
            h.press(egui::Key::F5, egui::Modifiers::NONE);
            assert_eq!(h.app.ui.open_panel, None);
            h.press(egui::Key::F5, egui::Modifiers::NONE);
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("brushes"));

            h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
            h.frames(1);
            let rect = floating_window(&h.layout(), "layers").unwrap().rect;
            h.f7();
            assert!(floating_window(&h.layout(), "layers").is_none());
            h.f7();
            assert_eq!(floating_window(&h.layout(), "layers").unwrap().rect, rect);
            h.app.run("window.panel.dock", json!({"panel": "layers"})).unwrap();
            h.f7();
            assert!(!showing(&h.app.ui, "layers"), "the expanded dock also toggles");
        }),
        ("Window > Layers from a Tab-hidden dock reveals the named front tab", |h| {
            // Choosing it shows the dock with Layers where it was instead of closing the tab.
            let item = window_layers();
            h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
            assert_eq!(checked(h, &item), Some(true));
            let before = h.layout();
            h.hide_panels(ChromeToggle::Panels);
            assert_eq!(checked(h, &item), Some(false), "no check mark while Tab hides the panels");
            h.app.run("window.panel", item.clone()).unwrap();
            h.frames(2);
            assert!(panels_shown(&h.app) && visible(&h.app, "layers"), "Layers shows");
            assert_eq!(h.layout(), before, "in its place");
            assert_eq!(checked(h, &item), Some(true));
            h.app.run("window.panel", item.clone()).unwrap();
            assert!(!h.layout().contains(&"layers".to_string()), "a visible panel closes");
            assert!(h.layout().contains(&"properties".to_string()), "only the panel named");
            assert_eq!(checked(h, &item), Some(false));
        }),
        ("Window > Layers from a Shift+Tab-hidden floating window reveals it", |h| {
            let item = window_layers();
            h.app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
            h.frames(1);
            let rect = floating_window(&h.layout(), "layers").map(|w| w.rect);
            assert!(rect.is_some());
            for params in [item.clone(), json!({"panel": "layers"})] {
                h.hide_panels(ChromeToggle::PanelsKeepTools);
                assert_eq!(checked(h, &item), Some(false), "{params}: no check mark while Shift+Tab hides the panels");
                h.app.run("window.panel", params.clone()).unwrap();
                h.frames(2);
                assert!(panels_shown(&h.app) && visible(&h.app, "layers"), "{params}: Layers shows");
                assert_eq!(floating_window(&h.layout(), "layers").map(|w| w.rect), rect, "{params}: where it was");
            }
        }),
        ("Window > Layers with a hidden iconic or closed panel", |h| {
            let item = window_layers();
            h.app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
            h.app.run("window.panel", item.clone()).unwrap();
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"));
            assert_eq!(checked(h, &item), Some(true));
            h.hide_panels(ChromeToggle::Panels);
            assert_eq!(checked(h, &item), Some(false), "no check mark with the dock hidden");
            h.app.run("window.panel", item.clone()).unwrap();
            h.frames(2);
            assert!(panels_shown(&h.app), "the dock shows");
            assert_eq!(h.app.ui.open_panel.as_deref(), Some("layers"), "the flyout stays open");
            assert_eq!(checked(h, &item), Some(true));
            h.app.run("window.panel", item.clone()).unwrap();
            assert_eq!(h.app.ui.open_panel, None, "a visible flyout puts away");

            *h = Harness::new();
            let libraries = json!({"panel": "libraries", "toggle": true});
            h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
            h.app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
            h.hide_panels(ChromeToggle::Panels);
            assert_eq!((checked(h, &libraries), checked(h, &item)), (Some(false), Some(false)));
            h.app.run("window.panel", libraries.clone()).unwrap();
            h.frames(2);
            assert!(panels_shown(&h.app) && visible(&h.app, "libraries"), "a closed panel comes back");
            h.app.run("window.panel", item.clone()).unwrap();
            assert!(visible(&h.app, "layers") && !visible(&h.app, "properties"), "a background tab comes to the front");
            assert_eq!((checked(h, &libraries), checked(h, &item)), (Some(false), Some(true)), "the tab in front is checked");
        }),
        ("F7 from a Tab-hidden dock reveals the named group", |h| {
            h.f7();
            assert!(visible(&h.app, "layers"));
            let before = h.layout();
            h.hide_panels(ChromeToggle::Panels);
            h.f7();
            h.frames(2);
            assert!(panels_shown(&h.app) && visible(&h.app, "layers"), "F7 shows the hidden group");
            assert_eq!(h.layout(), before);
            h.f7();
            assert!(group_hidden_all(h), "the next press hides the group");
        }),
    ];
    for (name, case) in cases {
        eprintln!("case: {name}");
        case(&mut Harness::new());
    }
}

#[test]
fn window_panels_peek_rejects_what_it_cannot_show_and_changes_nothing() {
    let mut h = Harness::new();
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap_err(), "the panels are not hidden");
    h.hide_panels(ChromeToggle::Panels);
    for side in [json!("up"), json!(1), json!(true), json!("Right")] {
        assert_eq!(h.app.run("window.panels.peek", json!({"side": side})).unwrap_err(), "`side` must be left, right or null", "{side}");
    }
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "left"})).unwrap_err(), "no panels are docked on the left side");
    assert_eq!(h.app.run("window.panels.peek", json!({})).unwrap(), Value::Null, "nothing to hide");
    h.frames(2);
    assert!(h.tab("properties").is_none() && !panels_shown(&h.app), "the refusals changed nothing");
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap(), json!("right"));
    assert_eq!(h.app.run("window.panels.peek", json!({"side": null})).unwrap(), Value::Null);
    h.app.ui.screen_mode = 3;
    assert_eq!(h.app.run("window.panels.peek", json!({"side": "right"})).unwrap_err(), "panels can't be shown in Presentation Mode");
    h.app.ui.screen_mode = 0;
}

#[test]
fn edge_panels_are_the_docked_panels_on_each_side_of_the_canvas() {
    let layout = default_workspace(&[]).layout().clone();
    let right = peek::edge_panels(&layout, peek::Edge::Right);
    assert!(right.contains(&"properties".to_string()) && right.contains(&"swatches".to_string()));
    assert!(!right.contains(&CANVAS.to_string()));
    assert!(peek::edge_panels(&layout, peek::Edge::Left).is_empty());
    let mirrored = Layout::new(
        Some(Node::split(Axis::X, SplitSize::FixedFirst(310.0), Node::tabs(vec!["layers".to_string()], 0), Node::tabs(vec![CANVAS.to_string()], 0))),
        Vec::new(),
    );
    assert_eq!(peek::edge_panels(&mirrored, peek::Edge::Left), ["layers"]);
    assert!(peek::edge_panels(&mirrored, peek::Edge::Right).is_empty());
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
fn ui_docking_reports_the_layout_and_the_dock_state() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.frame(vec![]);
    let tab = h.tab("layers").unwrap();
    h.click(tab.center());
    // A press on a tab leaves the keys where they were (the dock's TabStrip::keeps_focus); a click in the panel's
    // search field gives the panel the keys.
    h.click(h.text_field("Search layers").expect("the Layers panel's search field"));
    let report = inspect(&h.app, &h.ctx);
    assert_eq!(report["layout"], serde_json::to_value(h.layout()).unwrap());
    assert_eq!(report["focused"], json!("layers"));
    for key in ["flyout", "hidden", "columns", "windows", "peek", "panelsShown"] {
        assert!(report.get(key).is_some(), "{key} in the report");
    }
    assert!(report.get("hitRects").is_none() && report.get("customized").is_none(), "the dock-only hit list is gone");
    assert!(report["columns"].as_array().is_some_and(|columns| !columns.is_empty()), "the columns follow the layout");
    assert_eq!(report["panelsShown"], json!(panels_shown(&h.app)));
}

#[test]
fn window_menu_panels_without_a_place_open_in_their_icon_group_or_float() {
    let mut h = Harness::new();
    // A saved layout that lost Swatches without remembering where it was.
    let mut layout = h.layout();
    layout.apply(Action::close("swatches".into())).unwrap();
    h.set_layout(layout);
    Arc::make_mut(&mut h.app.ui.dock_workspace).forget_hidden(|_| true);
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

/// The tab in front of the docked group that holds `panel`.
fn front_of_group(layout: &Layout<String>, panel: &str) -> Option<String> {
    dock_columns(layout).into_iter().flat_map(|c| &c.groups).find(|g| g.panels.iter().any(|p| p == panel)).and_then(|g| g.active_panel()).cloned()
}

#[test]
fn a_panel_that_opens_floating_leaves_the_docked_tabs_as_they_were() {
    let mut h = Harness::new();
    assert_eq!(front_of_group(&h.layout(), "properties").as_deref(), Some("properties"));
    for panel in ["openType", "docInfo", "attributes", "imageTrace"] {
        h.app.run("window.panel", json!({"panel": panel})).unwrap();
        h.frame(vec![]);
        let layout = h.layout();
        assert!(floating_window(&layout, panel).is_some(), "{panel} floats");
        assert_eq!(main_panels(&layout), ["properties", "layers", "libraries"], "after {panel}");
        assert_eq!(front_of_group(&layout, "properties").as_deref(), Some("properties"), "Properties stays in front after {panel}");
    }
    // A panel placed in a window of its own at a point leaves them as well.
    place_floating(&mut h.app, "magicWand", pos2(200.0, 200.0), 0.0).unwrap();
    let layout = h.layout();
    assert!(floating_window(&layout, "magicWand").is_some());
    assert_eq!(front_of_group(&layout, "properties").as_deref(), Some("properties"), "Properties stays in front after a placed panel");
}

#[test]
fn f7_restores_only_tabs_hidden_with_the_group() {
    let mut h = Harness::new();
    h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
    assert!(h.hidden("libraries"));
    h.f7();
    h.f7();
    assert!(["properties", "layers"].iter().all(|panel| h.hidden(panel)));
    h.f7();
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers"]);
    assert!(!h.layout().contains(&"libraries".to_string()));
    assert!(showing(&h.app.ui, "layers"));
}

#[test]
fn independently_reopened_tab_is_not_restored_by_the_group_key() {
    let mut h = Harness::new();
    h.f7();
    assert!(showing(&h.app.ui, "layers"));
    h.f7();
    assert!(["properties", "layers", "libraries"].iter().all(|panel| h.hidden(panel)));
    assert!(group_hidden_all(&h));
    h.app.run("window.panel.activate", json!({"panel": "libraries"})).unwrap();
    assert!(h.layout().contains(&"libraries".to_string()));
    assert!(!h.hidden("libraries"));
    assert!(["properties", "layers"].iter().all(|panel| h.hidden(panel)));
    h.app.run("window.panel.close", json!({"panel": "libraries"})).unwrap();
    assert!(h.hidden("libraries"));
    assert!(!h.layout().contains(&"libraries".to_string()));
    h.f7();
    assert_eq!(group_of(&h.layout(), &"layers".into()), ["properties", "layers"]);
    assert!(!h.layout().contains(&"libraries".to_string()));
    assert!(showing(&h.app.ui, "layers"));
}

#[test]
fn showing_a_hidden_floating_group_returns_its_window_panels() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "swatches"})).unwrap();
    h.app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
    h.app.run("window.panel.activate", json!({"panel": "layers"})).unwrap();
    h.frame(vec![]);
    h.f7();
    assert!(floating_window(&h.layout(), "layers").is_none());
    let reply = h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    assert_eq!(reply, json!({"floating": ["properties", "layers", "libraries"]}), "only the restored window's panels");
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
    let state = |h: &Harness| (h.layout(), panels_shown(&h.app), h.app.ui.open_panel.clone(), h.app.ui.dock_workspace.hidden().len());
    let before = state(&h);
    for params in [item.clone(), json!({"panel": "layers"}), json!({"panel": "swatches"}), json!({"panel": "brushes"})] {
        let error = h.app.run("window.panel", params.clone()).unwrap_err();
        assert_eq!(error, "panels can't be shown in Presentation Mode", "{params}");
        assert_eq!(state(&h), before, "{params} changed nothing");
    }
    h.f7();
    assert_eq!(h.app.ui.screen_mode, 3);
    assert_eq!(h.layout(), before.0, "F7 is ignored");
    for mode in [1, 2] {
        h.app.run("view.screenMode", json!({"mode": mode})).unwrap();
        assert_eq!(checked(&h, &item), Some(true), "mode {mode}");
        h.app.run("window.panel", item.clone()).unwrap();
        assert!(!visible(&h.app, "layers"), "mode {mode}: the checked item hides the panel");
        h.app.run("window.panel", item.clone()).unwrap();
        assert!(visible(&h.app, "layers"), "mode {mode}: and shows it again");
    }
}

#[test]
fn other_function_keys_and_keyboard_guards_keep_their_routes() {
    let mut h = Harness::new();
    h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
    let platform = crate::keymap::platform();
    let keymap = commands::keymap(&h.ctx).expect("a frame ran");
    for (chord, command) in [("F8", "ui.symbolOptions"), ("F12", "file.revert")] {
        let want = Trigger::parse(chord).expect("a trigger").effective(platform);
        assert!(keymap.bindings(&CommandRef::new(command)).iter().any(|b| b.trigger.effective(platform) == want), "{chord} runs {command}");
    }
    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    h.press(egui::Key::F8, egui::Modifiers::NONE);
    assert!(h.app.any_dialog_open(), "F8 opens the Symbol Options dialog");
    answer(&mut h.app, true);
    assert_eq!(h.app.session.doc().unwrap().doc.symbols.len(), 1);
    crate::menus::invoke(&mut h.app, "view.rotate", json!({}));
    assert!(h.app.any_dialog_open());
    h.f7();
    assert!(showing(&h.app.ui, "layers"), "F7 does nothing while a dialog is open");
    answer(&mut h.app, false);

    // The Layers panel's search field, clicked once the dialog has gone: it has the keys until a click on the canvas
    // takes them.
    h.frames(2);
    let field = h.text_field("Search layers").expect("the Layers panel's search field");
    h.click(field);
    h.f7();
    assert!(showing(&h.app.ui, "layers"), "F7 does nothing while a text field has focus");
    let canvas = h.canvas();
    h.click(canvas.right_bottom() - vec2(20.0, 20.0));
    h.f7();
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
    let ctx = crate::testing::ui_context();
    let key = Event::Key { key: egui::Key::F12, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // The first frame builds the keymap the press goes through.
    key_frame(&mut saved, &ctx, vec![]);
    key_frame(&mut saved, &ctx, vec![key]);
    assert!(saved.any_dialog_open(), "F12 asks before reverting");
    answer(&mut saved, true);
    assert_eq!(saved.session.doc().unwrap().doc.node_count(), 2, "F12 reloads the saved document");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Each panel with display sizes steps through them and the smallest size goes back to the largest.
#[test]
fn cycle_options_steps_through_the_sizes_and_returns_to_the_largest() {
    let mut h = Harness::new();
    for panel in ["align", "attributes", "character", "color", "gradient", "info", "paragraph", "stroke", "transform"] {
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": panel})).unwrap();
        assert_eq!(r, json!({"panel": panel, "size": 1, "sizes": 2}), "{panel}");
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": panel})).unwrap();
        assert_eq!(r["size"], 0, "{panel}");
    }
    // The Flattener Preview's options start hidden: its first step shows them.
    assert!(!h.app.ui.flattener_preview.show_options);
    let r = h.app.run("window.panel.cycleOptions", json!({"panel": "Flattener Preview"})).unwrap();
    assert_eq!(r, json!({"panel": crate::panels::flattener_preview::ID, "size": 0, "sizes": 2}));
    assert!(h.app.ui.flattener_preview.show_options);
    h.app.run("window.panel.cycleOptions", json!({"panel": crate::panels::flattener_preview::ID})).unwrap();
    assert!(!h.app.ui.flattener_preview.show_options);
    // Transparency: thumbnails and options, thumbnails alone, neither, and back.
    for size in [1, 2, 0] {
        let r = h.app.run("window.panel.cycleOptions", json!({"panel": "transparency"})).unwrap();
        assert_eq!((r["size"].clone(), r["sizes"].clone()), (json!(size), json!(3)));
    }
    // Thumbnails hidden with the options shown matches no size, so the next step is the largest.
    crate::panels::set_part_hidden(&mut h.app, "tr-hide-thumbs", true);
    assert_eq!(h.app.run("window.panel.cycleOptions", json!({"panel": "transparency"})).unwrap()["size"], 0);
    assert!(!crate::panels::part_hidden(&h.app, "tr-hide-thumbs"), "the largest size shows the thumbnails");
}

/// A panel the Window menu floats opens tall enough for its content: every control of Asset Export,
/// and every Image Trace control down to the top of its Advanced section, lies between the body's
/// top and bottom, none scrolled or clipped away.
#[test]
fn window_menu_floats_open_tall_enough_for_their_content() {
    for (panel, title, until) in [("assetExport", "Asset Export", None), ("imageTrace", "Image Trace", Some("Advanced"))] {
        let mut h = Harness::new();
        h.app.run("window.panel", json!({"panel": panel})).unwrap();
        assert!(floating_window(&h.layout(), panel).is_some(), "{panel} floats");
        h.frames(3);
        let nodes: HashMap<_, _> = h.nodes.iter().map(|(id, node)| (*id, node)).collect();
        let rect =
            |node: &egui::accesskit::Node| node.bounds().map(|b| Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)));
        let (body_id, body) = h
            .nodes
            .iter()
            .find(|(_, node)| node.role() == egui::accesskit::Role::TabPanel && node.label() == Some(title))
            .and_then(|(id, node)| Some((*id, rect(node)?)))
            .unwrap_or_else(|| panic!("{panel}'s body is drawn"));
        // Every node drawn under the panel's body; its controls are the ones without children.
        let mut drawn = Vec::new();
        let mut stack = vec![body_id];
        while let Some(id) = stack.pop() {
            let Some(node) = nodes.get(&id) else { continue };
            stack.extend(node.children().iter().copied());
            if let Some(r) = rect(node).filter(|r| r.is_positive()) {
                drawn.push((node, r));
            }
        }
        let limit = until.map_or(f32::INFINITY, |section| {
            let found = drawn.iter().find(|(node, _)| node.role() == egui::accesskit::Role::Group && node.label() == Some(section));
            found.unwrap_or_else(|| panic!("{panel} draws its {section} section")).1.top()
        });
        let outside: Vec<_> = drawn
            .iter()
            .filter(|(node, r)| node.children().is_empty() && r.top() <= limit && (r.top() < body.top() - 0.5 || r.bottom() > body.bottom() + 0.5))
            .map(|(node, r)| (node.role(), node.label().map(str::to_owned), *r))
            .collect();
        assert!(outside.is_empty(), "{panel}: controls outside its body {body:?}: {outside:?}");
    }
}

/// An unknown panel, a panel without display sizes or a missing `panel` is an error, and the
/// panels keep their sizes.
#[test]
fn cycle_options_rejects_panels_without_sizes() {
    let mut h = Harness::new();
    let before = h.layout();
    for (params, error) in [
        (json!({"panel": "nope"}), "unknown panel `nope`"),
        (json!({}), "unknown panel ``"),
        (json!({"panel": 3}), "unknown panel ``"),
        (json!({"panel": "layers"}), "`layers` has no display sizes to cycle"),
        (json!({"panel": "swatches"}), "`swatches` has no display sizes to cycle"),
    ] {
        assert_eq!(h.app.run("window.panel.cycleOptions", params.clone()).unwrap_err(), error, "{params}");
    }
    assert_eq!(h.layout(), before, "a refusal changes nothing");
}

/// The width of the body of the panel titled `title` as the last frame drew it.
fn body_width(h: &Harness, title: &str) -> f32 {
    h.nodes
        .iter()
        .find(|(_, node)| node.role() == egui::accesskit::Role::TabPanel && node.label() == Some(title))
        .and_then(|(_, node)| node.bounds())
        .map(|b| (b.x1 - b.x0) as f32)
        .unwrap_or_else(|| panic!("{title}'s body is drawn"))
}

/// A floating panel's side dragged in past `PANEL_MIN_WIDTH` stops at the minimum.
#[test]
fn a_floating_panel_resized_past_the_panel_minimum_stops_at_it() {
    let mut h = Harness::new();
    h.app.run("window.panel.float", json!({"panel": "stroke", "x": 300, "y": 200, "width": 400, "height": 300})).unwrap();
    h.frames(3);
    let before = body_width(&h, "Stroke");
    assert!((before - 400.0).abs() < 1.0, "a float wider than the minimum keeps its width: {before}");
    let rect = floating_window(&h.layout(), "stroke").expect("Stroke floats").rect;
    // The window's right side is its resize grip; the drag goes 300 pt in, well past the minimum.
    let grip = pos2(rect.right() - 1.0, rect.center().y);
    h.drag(grip, grip - vec2(300.0, 0.0));
    let after = body_width(&h, "Stroke");
    assert!(after < before - 1.0, "the drag narrowed the window: {before} to {after}");
    assert!(after >= PANEL_MIN_WIDTH - 0.5, "dragged in past the minimum: {after}");
}

/// A docked column saved narrower than `PANEL_MIN_WIDTH` is drawn at the minimum.
#[test]
fn a_docked_column_is_never_narrower_than_the_panel_minimum() {
    let mut h = Harness::new();
    let mut layout = h.layout();
    let columns = end_dock_mut(&mut layout).expect("the default layout has an end dock");
    for column in columns.iter_mut().filter(|column| !column.iconic) {
        column.width = Some(100.0);
    }
    h.set_layout(layout);
    h.frames(3);
    assert!(body_width(&h, "Properties") >= PANEL_MIN_WIDTH - 0.5, "saved 100 pt wide: {}", body_width(&h, "Properties"));
}
