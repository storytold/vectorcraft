//! Optional compact main Tools layout preserves the tools while using less room.
// Tests may unwrap/panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use craft::ui::shell::embed;
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::i18n::LangExt;
use vectorcraft_ui_egui::{UiState, VectorcraftApp, i18n, workspaces};

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

fn same(a: egui::Vec2, b: egui::Vec2) -> bool {
    near(a.x, b.x) && near(a.y, b.y)
}

fn main_toolbar_app(double: bool, compact: Option<bool>) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "profile": null})).unwrap();
    app.run("prefs.set", json!({"key": "toolGroupLabels", "value": false})).unwrap();
    app.run("window.toolbarColumns", json!({ "double": double })).unwrap();
    if let Some(on) = compact {
        app.run("window.toolbarCompact", json!({ "on": on })).unwrap();
    }
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

fn access_rect(node: &egui::accesskit::Node) -> Option<Rect> {
    let r = node.bounds()?;
    Some(Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)))
}

/// The tool buttons the screen-reader nodes hold, each with its tool's id.
fn tool_cells(nodes: &[egui::accesskit::Node]) -> Vec<(String, Rect)> {
    nodes
        .iter()
        .filter(|n| n.role() == egui::accesskit::Role::Button && n.toggled().is_some())
        .filter_map(|n| {
            let label = n.label()?;
            let id = vectorcraft_tools::TOOL_GROUPS
                .iter()
                .flat_map(|group| group.iter())
                .find_map(|tool| (label == tool.label || label.starts_with(&format!("{} (", tool.label))).then(|| tool.id.to_string()))?;
            Some((id, access_rect(n)?))
        })
        .collect()
}

/// A tool button's size in `geometry`.
fn cell_size(geometry: &ToolbarGeometry) -> egui::Vec2 {
    geometry.cells[0].size()
}

/// `compact` draws smaller buttons than `regular` and keeps its tools.
fn assert_smaller(compact: &ToolbarGeometry, regular: &ToolbarGeometry) {
    let (c, r) = (cell_size(compact), cell_size(regular));
    assert!(c.x < r.x && c.y < r.y, "compact {c:?} against comfortable {r:?}");
    assert_eq!(compact.tools, regular.tools);
}

/// `a` and `b` draw the same button sizes in the same number of columns.
fn assert_same(a: &ToolbarGeometry, b: &ToolbarGeometry) {
    assert!(same(cell_size(a), cell_size(b)), "{:?} against {:?}", cell_size(a), cell_size(b));
    assert_eq!(a.column_pitch.is_some(), b.column_pitch.is_some());
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
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        ctx.enable_accesskit();
        Self { app, ctx, time: 0.0, height, nodes: vec![] }
    }

    /// One whole app frame.
    fn frame(&mut self) -> ToolbarGeometry {
        self.time += 0.05;
        let input = egui::RawInput {
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, self.height))),
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let app = &mut self.app;
        embed::before_input(&ctx, |out| app.frame_input(out));
        let mut output = ctx.run_ui(input, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        if let Some(update) = output.platform_output.accesskit_update.take() {
            self.nodes = update.nodes.into_iter().map(|(_, node)| node).collect();
        }
        output.textures_delta.clear();
        let found = tool_cells(&self.nodes);
        let mut tools: Vec<String> = found.iter().map(|(id, _)| id.clone()).collect();
        tools.sort();
        let mut cells: Vec<Rect> = found.into_iter().map(|(_, r)| r).collect();
        cells.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
        // PLAN: assumed the Tools rail's screen-reader name is "Tools"
        let panel_width = self.nodes.iter().find(|n| n.label() == Some("Tools")).and_then(access_rect).map_or(0.0, |r| r.width());
        ToolbarGeometry {
            tools,
            row_pitch: pitch(cells.iter().map(|r| r.center().y).collect()),
            column_pitch: pitch(cells.iter().map(|r| r.center().x).collect()),
            cells,
            panel_width,
        }
    }

    fn settled(&mut self) -> ToolbarGeometry {
        self.frame();
        self.frame();
        let geometry = self.frame();
        assert!(geometry.cells.len() > 10 && !geometry.tools.is_empty(), "actual Tools widgets were measured: {geometry:?}");
        geometry
    }

    fn tool_rect(&self, tool: &str) -> Rect {
        tool_cells(&self.nodes).into_iter().find(|(id, _)| id == tool).map(|(_, r)| r).unwrap_or_else(|| panic!("tool {tool} is drawn"))
    }
}

/// Compact makes the Tools buttons smaller than Comfortable and keeps the selected tool and the columns; both choices
/// survive the saved preferences.
#[test]
fn compact_tools_toggle_roundtrip_and_columns_keep_the_selected_tool() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let regular = ToolbarFrames::new(main_toolbar_app(true, Some(false)), 1600.0).settled();
    let mut app = main_toolbar_app(false, None);
    app.select_tool("pen");
    assert_eq!(app.run("window.toolbarCompact", json!({})).unwrap(), true);
    assert_eq!(app.run("window.toolbarCompact", json!({"on": true})).unwrap(), true);
    app.run("window.toolbarColumns", json!({"double": true})).unwrap();
    assert_eq!(app.session.tool_id(), "pen");
    app.ui = serde_json::from_value::<UiState>(serde_json::to_value(&app.ui).unwrap()).unwrap();
    let mut frames = ToolbarFrames::new(app, 1600.0);
    let compact = frames.settled();
    assert!(compact.column_pitch.is_some(), "saved preferences keep the two columns");
    assert_smaller(&compact, &regular);
    assert_eq!(frames.app.session.tool_id(), "pen");
    assert_eq!(frames.app.run("window.toolbarCompact", json!({})).unwrap(), false);
    assert_eq!(frames.app.run("window.toolbarCompact", json!({"on": false})).unwrap(), false);
    let comfortable = frames.settled();
    assert_same(&comfortable, &regular);
    assert_eq!(compact.tools, comfortable.tools);
    assert_eq!(frames.app.session.tool_id(), "pen");
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

/// The density covers the Tools rail alone: a toolbar made with New Toolbar and a torn-off strip keep their buttons.
#[test]
fn compact_tools_do_not_resize_named_toolbars_or_torn_off_strips() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let mut app = main_toolbar_app(false, Some(false));
    app.run("window.toolbar.new", json!({"name": "Named"})).unwrap();
    // Tools only these two hold: the Tools rail keeps `artboard` out, and `addAnchor` is in the Pen group the strip floats.
    app.run("toolbar.addTool", json!({"tool": "artboard", "toolbar": "Named"})).unwrap();
    app.run("window.floatTools", json!({"tool": "pen", "floating": true})).unwrap();
    let mut frames = ToolbarFrames::new(app, 1000.0);
    frames.settled();
    let sizes = |frames: &ToolbarFrames| ["artboard", "addAnchor", "selection"].map(|tool| frames.tool_rect(tool).size());
    let before = sizes(&frames);
    frames.app.run("window.toolbarCompact", json!({"on": true})).unwrap();
    frames.settled();
    let after = sizes(&frames);
    assert!(same(before[0], after[0]), "the named toolbar's button: {:?} then {:?}", before[0], after[0]);
    assert!(same(before[1], after[1]), "the strip's button: {:?} then {:?}", before[1], after[1]);
    assert!(after[2].x < before[2].x && after[2].y < before[2].y, "the Tools rail's button shrinks: {:?} then {:?}", before[2], after[2]);
}

/// Removes every member named `key`, wherever it sits in `value`.
fn strip_key(value: &mut Value, key: &str) {
    match value {
        Value::Object(map) => {
            map.remove(key);
            map.values_mut().for_each(|v| strip_key(v, key));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| strip_key(v, key)),
        _ => {}
    }
}

/// A saved workspace carries the density and the columns; going back to it as saved restores both without choosing another
/// tool, and a workspace saved before the option restores the Comfortable default and keeps the columns. (Switching to the
/// workspace already current would keep its changes, as production's switch did.)
#[test]
fn compact_tools_workspace_roundtrip_restores_density_and_legacy_defaults() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let _language = Language::english();
    let regular = ToolbarFrames::new(main_toolbar_app(true, Some(false)), 1600.0).settled();
    let mut app = main_toolbar_app(true, Some(true));
    app.select_tool("pen");
    let saved = serde_json::to_value(workspaces::record(&app.ui)).unwrap();
    assert_eq!(saved["toolbarCompact"], true, "the record uses the workspace's camelCase wire convention");
    workspaces::save_as(&mut app.ui, "Compact drawing").unwrap();
    let mut legacy_state = serde_json::to_value(&app.ui).unwrap();

    app.run("window.toolbarCompact", json!({"on": false})).unwrap();
    app.run("window.toolbarColumns", json!({"double": false})).unwrap();
    workspaces::reset(&mut app);
    let mut frames = ToolbarFrames::new(app, 1600.0);
    let restored = frames.settled();
    assert_smaller(&restored, &regular);
    assert!(restored.column_pitch.is_some(), "the saved workspace restores both layout choices");
    assert_eq!(frames.app.session.tool_id(), "pen", "workspace layout does not select another tool");

    // The same workspace as an older file wrote it, without the density key.
    strip_key(&mut legacy_state, "toolbarCompact");
    let mut app = frames.app;
    app.ui = serde_json::from_value::<UiState>(legacy_state).unwrap();
    app.run("window.toolbarCompact", json!({"on": true})).unwrap();
    workspaces::reset(&mut app);
    let mut frames = ToolbarFrames::new(app, 1600.0);
    let legacy = frames.settled();
    assert_same(&legacy, &regular);
    assert!(legacy.column_pitch.is_some(), "the older saved column choice is retained");
    assert_eq!(frames.app.session.tool_id(), "pen");
}
