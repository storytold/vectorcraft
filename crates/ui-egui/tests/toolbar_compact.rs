//! Optional compact main Tools layout preserves the tools while using less room.
// Tests may unwrap/panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use egui::{Event, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{UiState, VectorcraftApp, i18n, theme, toolbar};

static CASE: Mutex<()> = Mutex::new(());

struct Language(i18n::Lang);
impl Language {
    fn english() -> Self {
        let previous = Self(i18n::current());
        i18n::set_current(i18n::Lang::from_code("en").unwrap());
        previous
    }
}
impl Drop for Language {
    fn drop(&mut self) {
        i18n::set_current(self.0);
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn main_toolbar_app(double: bool, compact: Option<bool>) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "profile": null})).unwrap();
    app.run("prefs.set", json!({"key": "toolGroupLabels", "value": false})).unwrap();
    let mut state = serde_json::to_value(&app.ui).unwrap();
    state["toolbar_double"] = json!(double);
    match compact {
        Some(on) => state["toolbar_compact"] = json!(on),
        None => {
            state.as_object_mut().unwrap().remove("toolbar_compact");
        }
    }
    // The old UiState reader ignores this future field; no future Rust field is referenced.
    app.ui = serde_json::from_value::<UiState>(state).unwrap();
    app
}

#[derive(Clone, Debug)]
struct ToolbarGeometry {
    tools: Vec<String>,
    cells: Vec<Rect>,
    row_pitch: Option<f32>,
    column_pitch: Option<f32>,
    panel_width: f32,
}

fn pitch(mut centers: Vec<f32>) -> Option<f32> {
    centers.sort_by(f32::total_cmp);
    centers.dedup_by(|a, b| near(*a, *b));
    centers.windows(2).map(|p| p[1] - p[0]).filter(|d| *d > 0.01).min_by(f32::total_cmp)
}

fn tool_ids(nodes: &[egui::accesskit::Node]) -> Vec<String> {
    let mut ids: Vec<String> = nodes
        .iter()
        .filter(|n| n.role() == egui::accesskit::Role::Button && n.toggled().is_some())
        .filter_map(|n| {
            let label = n.label()?;
            vectorcraft_tools::TOOL_GROUPS
                .iter()
                .flat_map(|group| group.iter())
                .find_map(|tool| (label == tool.label || label.starts_with(&format!("{} (", tool.label))).then(|| tool.id.to_string()))
        })
        .collect();
    ids.sort();
    ids
}

struct ToolbarFrames {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    height: f32,
    nodes: Vec<egui::accesskit::Node>,
}
impl ToolbarFrames {
    fn new(app: VectorcraftApp, height: f32) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, theme::Brightness::MediumDark);
        Self { app, ctx, time: 0.0, height, nodes: vec![] }
    }

    fn frame(&mut self, events: Vec<Event>) -> ToolbarGeometry {
        self.time += 0.05;
        let input = egui::RawInput {
            time: Some(self.time),
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, self.height))),
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let mut panel_width = 0.0;
        let mut output = ctx.run_ui(input, |ui| {
            let before = ui.available_rect_before_wrap();
            toolbar::show(&mut self.app, ui);
            panel_width = before.width() - ui.available_rect_before_wrap().width();
        });
        if let Some(update) = output.platform_output.accesskit_update.take() {
            self.nodes = update.nodes.into_iter().map(|(_, node)| node).collect();
        }
        output.textures_delta.clear();
        // These are actual allocated widgets, not computed layout expectations. Both sizes are
        // accepted so old source can finish every requested measurement before primary assertions.
        let mut cells: Vec<Rect> = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|w| {
                    let s = w.rect.size();
                    (near(s.x, 36.0) && near(s.y, 29.0)) || (near(s.x, 30.0) && near(s.y, 25.0))
                })
                .map(|w| w.rect)
                .collect()
        });
        cells.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
        ToolbarGeometry {
            tools: tool_ids(&self.nodes),
            row_pitch: pitch(cells.iter().map(|r| r.center().y).collect()),
            column_pitch: pitch(cells.iter().map(|r| r.center().x).collect()),
            cells,
            panel_width,
        }
    }

    fn settled(&mut self) -> ToolbarGeometry {
        self.frame(vec![]);
        self.frame(vec![]);
        self.frame(vec![])
    }
}

#[test]
fn compact_tools_reduce_both_axes_and_reserved_panel_width() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut observations = vec![];
    for double in [false, true] {
        let regular = ToolbarFrames::new(main_toolbar_app(double, Some(false)), 1600.0).settled();
        let compact = ToolbarFrames::new(main_toolbar_app(double, Some(true)), 1600.0).settled();
        observations.push((double, regular, compact));
    }
    // Both one/two-column layouts have actually drawn before a missing feature can fail.
    println!("actual regular/compact main Tools geometry: {observations:#?}");
    for (double, regular, compact) in observations {
        assert!(regular.cells.len() > 10 && !regular.tools.is_empty(), "actual Tools widgets and IDs were measured: {regular:?}");
        assert_eq!(compact.tools, regular.tools, "the same usual tools remain, double={double}");
        assert_eq!(compact.cells.len(), regular.cells.len(), "tool-button count, double={double}");
        assert!(compact.row_pitch.unwrap() < regular.row_pitch.unwrap(), "compact must reduce vertical pitch, double={double}: {compact:?}");
        if double {
            assert!(compact.column_pitch.unwrap() < regular.column_pitch.unwrap(), "compact must reduce the inter-column pitch: {compact:?}");
        } else {
            assert!(compact.column_pitch.is_none());
        }
        assert!(compact.panel_width < regular.panel_width, "compact must reserve less real panel width, double={double}: {compact:?}");
        assert!(compact.cells.iter().all(|r| near(r.width(), 30.0) && near(r.height(), 25.0)));
    }
}

#[test]
fn compact_tools_default_and_explicit_false_keep_the_comfortable_layout() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    for double in [false, true] {
        let missing = ToolbarFrames::new(main_toolbar_app(double, None), 1600.0).settled();
        let off = ToolbarFrames::new(main_toolbar_app(double, Some(false)), 1600.0).settled();
        assert!(missing.cells.len() > 10 && !missing.tools.is_empty());
        assert_eq!(missing.tools, off.tools);
        assert_eq!(missing.cells, off.cells, "omitted and false keep exactly the same button geometry");
        assert!(missing.cells.iter().all(|r| near(r.width(), 36.0) && near(r.height(), 29.0)));
        assert!(near(missing.row_pitch.unwrap(), 29.0), "{missing:?}");
        assert_eq!(missing.column_pitch.is_some(), double);
        if double {
            assert!(near(missing.column_pitch.unwrap(), 36.0), "{missing:?}");
        }
        assert!(near(missing.panel_width, if double { 86.0 } else { 54.0 }), "the current panel plus its 6pt outer frame: {missing:?}");
        assert!(near(missing.panel_width, off.panel_width));
    }
}

// Additional public controls. The root-formatted two-case baseline above remains an exact prefix.

fn compact_value(app: &VectorcraftApp) -> bool {
    serde_json::to_value(&app.ui).unwrap()["toolbar_compact"].as_bool().unwrap_or(false)
}

fn access_rect(node: &egui::accesskit::Node) -> Option<Rect> {
    let r = node.bounds()?;
    Some(Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)))
}

impl ToolbarFrames {
    fn named_rect(&self, label: &str) -> Rect {
        self.nodes.iter().find(|n| n.label() == Some(label)).and_then(access_rect).unwrap_or_else(|| panic!("{label} is drawn"))
    }

    fn tool_rect(&self, tool: &str) -> Rect {
        let name = vectorcraft_tools::tool_info(tool).unwrap().label;
        self.nodes
            .iter()
            .find(|n| {
                n.role() == egui::accesskit::Role::Button
                    && n.toggled().is_some()
                    && n.label().is_some_and(|l| l == name || l.starts_with(&format!("{name} (")))
            })
            .and_then(access_rect)
            .unwrap_or_else(|| panic!("tool {tool} is drawn"))
    }

    fn click_at(&mut self, at: Pos2, button: egui::PointerButton) {
        let event = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE };
        self.frame(vec![Event::ModifiersChanged(egui::Modifiers::NONE), Event::PointerMoved(at), event(true)]);
        self.frame(vec![event(false)]);
        self.frame(vec![]);
    }
}

#[test]
fn compact_tools_toggle_roundtrip_and_columns_keep_the_selected_tool() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut app = main_toolbar_app(false, None);
    app.select_tool("pen");
    assert_eq!(app.run("window.toolbarCompact", json!({})).unwrap(), true);
    assert!(compact_value(&app));
    assert_eq!(app.run("window.toolbarCompact", json!({"on": true})).unwrap(), true);
    app.run("window.toolbarColumns", json!({"double": true})).unwrap();
    assert!(compact_value(&app) && app.ui.toolbar_double);
    assert_eq!(app.session.tool_id(), "pen");
    app.ui = serde_json::from_value::<UiState>(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(compact_value(&app) && app.ui.toolbar_double, "saved preferences keep both independent choices");
    let mut frames = ToolbarFrames::new(app, 1600.0);
    let compact = frames.settled();
    assert!(compact.cells.iter().all(|r| near(r.width(), 30.0) && near(r.height(), 25.0)));
    assert_eq!(frames.app.session.tool_id(), "pen");
    assert_eq!(frames.app.run("window.toolbarCompact", json!({})).unwrap(), false);
    assert!(!compact_value(&frames.app));
    assert_eq!(frames.app.run("window.toolbarCompact", json!({"on": false})).unwrap(), false);
    let regular = frames.settled();
    assert!(regular.cells.iter().all(|r| near(r.width(), 36.0) && near(r.height(), 29.0)));
    assert_eq!(compact.tools, regular.tools);
    assert!(frames.app.ui.toolbar_double);
    assert_eq!(frames.app.session.tool_id(), "pen");
}

#[test]
fn compact_tools_actual_buttons_flyout_and_short_window_footer_work() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut frames = ToolbarFrames::new(main_toolbar_app(true, Some(true)), 1000.0);
    frames.settled();
    let pen = frames.tool_rect("pen").center();
    frames.click_at(pen, egui::PointerButton::Primary);
    assert_eq!(frames.app.session.tool_id(), "pen", "an actual compact tool hit target selects the tool");
    let rectangle = frames.tool_rect("rectangle").center();
    frames.click_at(rectangle, egui::PointerButton::Secondary);
    assert!(frames.app.ui.flyout.is_some());
    let flyout = frames.ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).unwrap();
    assert!(flyout.width() > 30.0 && flyout.height() > 25.0, "the actual grouped-tool flyout is drawn");

    // A separate normal short window avoids retaining the flyout over the footer interaction.
    let mut short = ToolbarFrames::new(main_toolbar_app(true, Some(true)), 420.0);
    let top = short.settled();
    let before = short.named_rect("Edit Toolbar");
    assert!(before.bottom() > short.height, "this ordinary window actually needs scrolling: {before:?}");
    short.frame(vec![
        Event::PointerMoved(top.cells[0].center()),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -2000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    for _ in 0..24 {
        short.frame(vec![]);
    }
    let edit = short.named_rect("Edit Toolbar");
    assert!(edit.top() >= 0.0 && edit.bottom() <= short.height, "the actual footer scrolls into view: {edit:?}");
    let header = short.named_rect("Toggle single/double column");
    for label in ["Draw Normal (Shift+D)", "Draw Behind (Shift+D)", "Draw Inside (Shift+D)"] {
        let r = short.named_rect(label);
        assert!(
            r.left() >= header.left() - 0.01 && r.right() <= header.right() + 0.01,
            "footer fits within compact panel: {label} {r:?} in {header:?}"
        );
    }
    let behind = short.named_rect("Draw Behind (Shift+D)").center();
    short.click_at(behind, egui::PointerButton::Primary);
    assert_eq!(short.app.session.draw_mode, vectorcraft_engine::DrawMode::Behind);
    let edit = short.named_rect("Edit Toolbar").center();
    short.click_at(edit, egui::PointerButton::Primary);
    assert!(short.app.ui.tools_drawer, "the real footer button opens Edit Toolbar");
}

#[test]
fn compact_tools_advanced_and_edited_layouts_keep_their_tool_sets() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    for advanced in [true, false] {
        let mut observations = vec![];
        for compact in [false, true] {
            let mut app = main_toolbar_app(true, Some(compact));
            if advanced {
                app.run("window.toolbarAdvanced", json!({})).unwrap();
            } else {
                app.run("toolbar.removeTool", json!({"tool": "selection"})).unwrap();
            }
            observations.push(ToolbarFrames::new(app, 1600.0).settled());
        }
        let regular = &observations[0];
        let compact = &observations[1];
        assert_eq!(regular.tools, compact.tools, "same Advanced/edited tool IDs, advanced={advanced}");
        assert_eq!(regular.cells.len(), compact.cells.len());
        assert!(compact.row_pitch.unwrap() < regular.row_pitch.unwrap());
        assert!(compact.column_pitch.unwrap() < regular.column_pitch.unwrap());
        assert!(compact.panel_width < regular.panel_width);
        if !advanced {
            assert!(!compact.tools.iter().any(|id| id == "selection"), "the user's main-toolbar edit remains");
        }
    }
}

#[test]
fn compact_tools_do_not_resize_named_toolbars_or_torn_off_strips() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut app = main_toolbar_app(false, Some(false));
    app.run("window.toolbar.new", json!({"name": "Named"})).unwrap();
    for tool in ["pen", "rectangle", "selection"] {
        app.run("toolbar.addTool", json!({"tool": tool, "toolbar": "Named"})).unwrap();
    }
    app.run("window.floatTools", json!({"tool": "rectangle", "floating": true})).unwrap();
    let mut frames = ToolbarFrames::new(app, 1000.0);
    frames.settled();
    // Existing observable egui Area IDs, matching the production areas; no private helper import.
    let named_id = egui::Id::unique(("custom-toolbar", "named".to_string()));
    let strip_id = egui::Id::unique(("tool-floating", "rectangle"));
    let before = frames.ctx.memory(|m| (m.area_rect(named_id).unwrap().size(), m.area_rect(strip_id).unwrap().size()));
    frames.app.run("window.toolbarCompact", json!({"on": true})).unwrap();
    frames.settled();
    let after = frames.ctx.memory(|m| (m.area_rect(named_id).unwrap().size(), m.area_rect(strip_id).unwrap().size()));
    assert_eq!(before, after, "the main Tools choice must not shrink the shared strip pitch or named-toolbar buttons");
}

#[test]
fn compact_tools_workspace_roundtrip_restores_density_and_legacy_defaults() {
    use vectorcraft_ui_egui::workspaces;

    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut app = main_toolbar_app(true, Some(true));
    app.select_tool("pen");
    let workspace = workspaces::capture(&app.ui, "Compact drawing");
    let saved = serde_json::to_value(workspace).unwrap();
    assert_eq!(saved["toolbarCompact"], true, "Workspace uses its existing camelCase wire convention");
    let restored: workspaces::Workspace = serde_json::from_value(saved.clone()).unwrap();
    app.run("window.toolbarCompact", json!({"on": false})).unwrap();
    app.run("window.toolbarColumns", json!({"double": false})).unwrap();
    workspaces::apply(&mut app.ui, &restored);
    assert!(compact_value(&app) && app.ui.toolbar_double, "applying the saved workspace restores both layout choices");
    assert_eq!(app.session.tool_id(), "pen", "workspace layout does not select another tool");
    let mut legacy = saved;
    legacy.as_object_mut().unwrap().remove("toolbarCompact");
    let legacy: workspaces::Workspace = serde_json::from_value(legacy).unwrap();
    workspaces::apply(&mut app.ui, &legacy);
    assert!(!compact_value(&app), "a workspace saved before this option restores the Comfortable default");
    assert!(app.ui.toolbar_double, "the older saved column choice is retained");
    assert_eq!(app.session.tool_id(), "pen");
}
