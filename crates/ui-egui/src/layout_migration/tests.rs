use super::*;
use serde_json::json;

/// `ui.json` as VectorCraft 0.16 (8bf090b25) saved it after: Swatches moved below Layers;
/// Color, then Stroke onto it, floated; Gradient floated with Transparency below it (a set);
/// Align floated and collapsed; Pathfinder floated with Transform below it, the set docked as
/// column 1; Libraries docked on the icon column; Symbols moved beside Swatches, then closed
/// (0.16 then showed it in the icon column again);
/// Swatches collapsed to its tab; the dock widened to 380; saved as workspace "Migration Test".
const SAVED_BY_0_16: &str = include_str!("../../tests/fixtures/ui-before-canvas-layout.json");
/// Preferences as the production app saves them, with Tools on the right, toolbars and strips.
const PRODUCTION_TOOLBARS: &str = include_str!("../../tests/fixtures/ui-production-toolbars.json");

fn read(saved: &Value) -> ReadPreferences {
    UiState::read_preferences(&serde_json::to_vec(saved).unwrap()).unwrap()
}

/// The file's `dock_tab_order`, which its workspace records convert with.
fn tab_order(saved: &Value) -> Vec<DockTab> {
    serde_json::from_value(saved["dock_tab_order"].clone()).unwrap_or_default()
}

/// The id of the workspace `name`.
fn id_of(ui: &UiState, name: &str) -> String {
    crate::workspaces::find(ui, name).expect("the workspace is kept").id.clone()
}

/// Whether the workspace `name` is kept.
fn has_workspace(ui: &UiState, name: &str) -> bool {
    crate::workspaces::names(ui).iter().any(|n| n == name)
}

#[test]
fn legacy_help_bar_visibility_follows_each_layouts_status_bar() {
    let prefs = read(&json!({
        "status_bar": false,
        "custom_workspaces": [{"name":"Hidden", "statusBar": false}, {"name":"Shown", "statusBar":true}],
        "workspace_changes": [{"name":"Shown", "statusBar":false}]
    }));
    let ui = &prefs.ui;
    assert!(!ui.help_bar);
    assert!(!ui.workspaces.records[&id_of(ui, "Hidden")].help_bar);
    assert!(ui.workspaces.records[&id_of(ui, "Shown")].help_bar);
    assert!(!ui.workspaces.edited[&id_of(ui, "Shown")].help_bar, "the changes kept for a workspace follow their own status bar");
    assert!(read(&json!({})).ui.help_bar);
    for (status, help) in [(true, false), (false, true)] {
        let prefs = read(&json!({"status_bar":status, "help_bar":help,
            "custom_workspaces":[{"name":"Explicit", "statusBar":status, "helpBar":help}]}));
        assert_eq!(prefs.ui.help_bar, help);
        assert_eq!(prefs.ui.workspaces.records[&id_of(&prefs.ui, "Explicit")].help_bar, help);
    }
}

/// The columns of the dock at `side` of a valid workspace.
fn dock(workspace: &Workspace<String>, side: DockEdge) -> &[Column<String>] {
    fn find(node: &Node<String>, side: DockEdge) -> Option<&[Column<String>]> {
        match node {
            Node::Dock { side: at, columns, .. } if *at == side => Some(columns),
            Node::Split { first, second, .. } => find(first, side).or_else(|| find(second, side)),
            _ => None,
        }
    }
    workspace.validate().unwrap();
    workspace.layout().root.as_ref().and_then(|root| find(root, side)).unwrap_or_default()
}

fn groups(column: &Column<String>) -> Vec<Vec<&str>> {
    column.groups.iter().map(|g| g.panels.iter().map(String::as_str).collect()).collect()
}

fn window_groups(window: &Floating<String>) -> Vec<Vec<&str>> {
    window.groups().iter().map(|g| g.panels.iter().map(String::as_str).collect()).collect()
}

#[test]
fn a_layout_saved_before_the_canvas_joined_keeps_its_arrangement() {
    let saved: Value = serde_json::from_str(SAVED_BY_0_16).unwrap();
    let read = read(&saved);
    assert!(read.intact && read.converted);
    let ui = read.ui;
    let workspace = &ui.dock_workspace;
    let columns = dock(workspace, DockEdge::End);
    assert_eq!(columns.len(), 3, "{columns:#?}");

    let main = &columns[0];
    assert_eq!(groups(main), [vec!["properties", "layers"], vec!["swatches"]]);
    assert_eq!(main.groups[0].active, 1, "Layers in front");
    assert!(main.groups[1].minimized, "Swatches collapsed to its tab");
    assert!(!main.groups[0].minimized && !main.iconic);
    assert_eq!(main.width, Some(380.0), "dock width");

    let icons = &columns[1];
    assert!(icons.iconic);
    assert_eq!(groups(icons)[0], ["libraries"], "dropped on the icon column, at its head");
    let iconic: Vec<&str> = groups(icons).concat();
    assert!(iconic.contains(&"colorGuide") && iconic.contains(&"brushes") && iconic.contains(&"variables"));
    assert!(groups(icons).contains(&vec!["brushes", "symbols", "patternOptions"]), "closed Symbols in its icon row");
    for elsewhere in ["swatches", "color", "stroke", "gradient", "transparency", "align", "pathfinder", "transform"] {
        assert!(!iconic.contains(&elsewhere), "{elsewhere} has a place of its own");
    }

    let docked = &columns[2];
    assert_eq!(groups(docked), [vec!["pathfinder"], vec!["transform"]], "set docked as a column");
    assert!(!docked.iconic);

    floating_windows_keep_their_places(workspace.layout());

    // Symbols is in the icon column, and the floating panels came from it: none keeps a
    // place in the dock to go back to.
    assert!(workspace.hidden().is_empty() && workspace.floated().is_empty(), "{:?} {:?}", workspace.hidden(), workspace.floated());

    // The saved workspace carries the same arrangement as the top level.
    assert!(has_workspace(&ui, "Migration Test"));
    let (record, _) = legacy_layout(&saved["custom_workspaces"][0], &tab_order(&saved));
    assert_eq!(record.layout(), workspace.layout());
    assert_eq!(record.hidden(), workspace.hidden());
}

/// The three floating windows of the migrated layout.
fn floating_windows_keep_their_places(layout: &Layout<String>) {
    assert_eq!(layout.floating.len(), 3);
    let color = &layout.floating[0];
    assert_eq!(window_groups(color), [vec!["color", "stroke"]]);
    assert_eq!(color.groups()[0].active, 1, "Stroke in front");
    assert_eq!(color.rect, Rect::from_min_size(pos2(300.0, 120.0), vec2(300.0, 400.0)));
    let set = &layout.floating[1];
    assert_eq!(window_groups(set), [vec!["gradient"], vec!["transparency"]], "floating set");
    assert_eq!((set.rect.min, set.rect.width()), (pos2(640.0, 100.0), 300.0));
    assert!(!set.column.iconic);
    let align = &layout.floating[2];
    assert_eq!(window_groups(align), [vec!["align"]]);
    assert!(align.column.iconic, "collapsed floating group");
    assert_eq!(align.rect.min, pos2(300.0, 560.0));
}

#[test]
fn a_converted_file_saved_again_has_no_legacy_fields_and_loads_the_same() {
    let saved: Value = serde_json::from_str(SAVED_BY_0_16).unwrap();
    let ui = read(&saved).ui;
    let again = serde_json::to_value(&ui).unwrap();
    let text = again.to_string();
    for key in [
        "floating_panels",
        "docking_icons",
        "docking_collapsed",
        "docking",
        "docking_hidden",
        "toolbar_pos",
        "floating_flyouts",
        "slot_tool",
        "floatingPanels",
        "dockingIcons",
        "dockingCollapsed",
        "dockingHidden",
        "toolbarPos",
        "floatingFlyouts",
        "slotTool",
    ] {
        assert!(!text.contains(&format!("\"{key}\"")), "{key} written");
    }
    let reread = read(&again);
    assert!(reread.intact && !reread.converted);
    assert_eq!(reread.ui.dock_workspace, ui.dock_workspace);
    let ids = |ui: &UiState| ui.dock_workspace.layout().floating.iter().map(|w| w.id).collect::<Vec<_>>();
    assert_eq!(ids(&reread.ui), ids(&ui), "floating windows keep their ids");
}

#[test]
fn a_file_from_before_the_shared_dock_converts_and_remembered_changes_too() {
    let saved = json!({
        "dock_tab": "Libraries", "dock_collapsed": true, "dock_width": 400.0, "workspace": "Essentials",
        "floating_panels": [{"panels": ["layers"], "active": 0, "pos": [10.0, 20.0]}],
        "docking_icons": ["properties"],
        "workspace_changes": [{
            "name": "Essentials", "dockTab": "Layers",
            "floatingPanels": [{"panels": ["swatches", "brushes"], "active": 1, "pos": [50.0, 60.0], "width": 260.0, "collapsed": true}],
        }],
    });
    let ui = read(&saved).ui;
    let workspace = &ui.dock_workspace;
    let columns = dock(workspace, DockEdge::End);
    assert_eq!(groups(&columns[0]), [vec!["libraries"]]);
    assert!(columns[0].iconic, "collapsed dock");
    assert_eq!(columns[0].width, Some(400.0));
    assert_eq!(groups(&columns[1])[0], ["properties"]);
    assert_eq!(window_groups(&workspace.layout().floating[0]), [vec!["layers"]]);
    assert_eq!(workspace.layout().floating[0].rect, Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 400.0)));

    let (changed, _) = legacy_layout(&saved["workspace_changes"][0], &tab_order(&saved));
    let columns = dock(&changed, DockEdge::End);
    assert_eq!(groups(&columns[0]), [vec!["properties", "layers", "libraries"]]);
    assert_eq!(columns[0].groups[0].active, 1);
    assert!(!columns[0].iconic);
    assert!(groups(&columns[1]).contains(&vec!["symbols", "patternOptions"]));
    let window = &changed.layout().floating[0];
    assert_eq!(window_groups(window), [vec!["swatches", "brushes"]]);
    assert_eq!(window.groups()[0].active, 1);
    assert!(window.column.iconic);
    assert_eq!(window.rect, Rect::from_min_size(pos2(50.0, 60.0), vec2(260.0, 400.0)));

    // A floating set keeps the heights of its groups.
    let ui = read(&json!({
        "floating_panels": [
            {"panels": ["stroke"], "active": 0, "pos": [0.0, 0.0], "column": 1},
            {"panels": ["gradient"], "active": 0, "pos": [0.0, 0.0], "column": 1},
        ],
        "docking": {"root": {"Tabs": {"panels": ["properties"], "active": 0}}, "floating": [
            {"panels": ["stroke"], "active": 0, "rect": [0.0, 0.0, 300.0, 150.0]},
            {"panels": ["gradient"], "active": 0, "rect": [0.0, 0.0, 300.0, 450.0]},
        ]},
    }))
    .ui;
    let set = &ui.dock_workspace.layout().floating[0];
    assert_eq!(set.groups().iter().map(|g| g.height).collect::<Vec<_>>(), [Some(150.0), Some(450.0)]);
    assert_eq!(set.rect.height(), 600.0);

    // Flags alone describe a layout too: the user's collapsed, widened column survives.
    let read = read(&json!({"dock_tab": "Layers", "dock_collapsed": true, "dock_width": 400.0}));
    assert!(read.converted);
    let columns = dock(&read.ui.dock_workspace, DockEdge::End);
    assert!(columns[0].iconic);
    assert_eq!(columns[0].width, Some(400.0));
    assert_eq!(columns[0].groups[0].active_panel().map(String::as_str), Some("layers"));
}

#[test]
fn unreadable_docking_data_is_dropped_and_reported() {
    let saved: Value = serde_json::from_str(SAVED_BY_0_16).unwrap();
    for (key, value) in [
        ("docking", json!({"root": {"FutureNode": {}}, "floating": []})),
        ("docking_hidden", json!(["malformed"])),
        ("floating_panels", json!([{"panels": "color"}])),
        ("docking_icons", json!(7)),
    ] {
        let mut broken = saved.clone();
        broken[key] = value.clone();
        broken["custom_workspaces"][0]["docking"] = json!(false);
        let read = read(&broken);
        assert!(!read.intact, "{key}");
        assert!(has_workspace(&read.ui, "Migration Test"), "the saved workspace is kept");
        assert!(read.ui.dock_workspace.validate().is_ok(), "{key}: what is left still converts");
    }
    assert!(UiState::read_preferences(b"[1, 2]").is_none());
    // `toolbar` (Window › Tools) is the layout's now, so another of the old fields stands in.
    assert!(UiState::read_preferences(br#"{"toolbar_compact": "invalid"}"#).is_none(), "other fields stay strict");
}

/// A hand-edited file: unknown and repeated panels, sizes out of range, more groups than any
/// layout holds.
#[test]
fn untrusted_legacy_fields_convert_without_failing() {
    let mut floating: Vec<Value> =
        (0..2000).map(|k| json!({"panels": ["nosuch", "color"], "active": k, "pos": [1e39, -1e39], "column": k % 3})).collect();
    floating.insert(0, json!({"panels": ["stroke"], "active": 99, "pos": [0.0, 0.0], "width": 1e30, "docked": 4000000000u32}));
    let ui = read(&json!({
        "dock_width": -5.0,
        "floating_panels": floating,
        "docking_icons": ["nosuch", "glyphs", "tabs"],
        "docking_collapsed": ["properties", "nosuch"],
        "docking": {"root": {"Tabs": {"panels": ["properties", "properties", "nosuch"], "active": 9}}, "floating": [
            {"panels": ["brushes"], "active": 0, "rect": [0.0, 0.0, 0.0, 0.0]},
            {"panels": ["info"], "active": 0, "rect": [1e39, 1e39, 1e39, 1e39]},
            {"panels": ["navigator"], "active": 0, "rect": [-5.0, 2e6, -1.0, 1e7]},
        ]},
    }))
    .ui;
    let workspace = &ui.dock_workspace;
    let columns = dock(workspace, DockEdge::End);
    assert_eq!(groups(&columns[0]), [vec!["properties"]]);
    assert!(columns[0].groups[0].minimized);
    assert!(columns[0].width.is_some_and(f32::is_finite));
    assert!(groups(&columns[1]).last().is_some_and(|g| g == &["tabs"]), "a panel outside the rows dropped on the icon column");
    let last = columns.last().unwrap();
    assert_eq!(groups(last), [vec!["stroke"]]);
    assert!(last.width.is_some_and(|w| w <= COLUMN_MAX));
    assert_eq!(workspace.layout().floating.len(), 4, "every old window kept");
    assert!(workspace.layout().floating.iter().all(|w| w.rect.is_finite() && w.rect.width() > 0.0 && w.rect.height() > 0.0));
}

#[test]
fn a_production_preferences_file_loads_into_the_same_arrangement() {
    let saved: Value = serde_json::from_str(PRODUCTION_TOOLBARS).unwrap();
    let read = read(&saved);
    assert!(read.converted);
    let ui = read.ui;
    let workspace = &ui.dock_workspace;

    let ids: Vec<u32> = ui.custom_toolbars.iter().map(|t| t.id).collect();
    assert_eq!(ids, [1, 2], "ids in list order, none repeated");

    let end = dock(workspace, DockEdge::End);
    let tools = end.last().unwrap();
    assert_eq!(groups(tools), [vec!["tools"]], "Tools stands on the right");
    assert_eq!(tools.steps, Some(2), "the double toolbar");

    let start = dock(workspace, DockEdge::Start);
    assert_eq!(start.len(), 1);
    assert_eq!(groups(&start[0]), [vec!["toolbar:2"]], "the docked custom toolbar");
    assert_eq!(start[0].steps, Some(2));

    let floating = &workspace.layout().floating;
    let toolbar = floating.iter().find(|w| window_groups(w) == [vec!["toolbar:1"]]).expect("the floating toolbar");
    assert_eq!(toolbar.rect.min, pos2(500.0, 200.0));
    assert_eq!(floating.iter().filter(|w| w.panels().any(|p| p.starts_with("strip:"))).count(), 1, "one torn-off strip");
    assert!(floating.iter().any(|w| window_groups(w) == [vec!["layers"]]), "the floating panel");

    assert!(slot_of("nosuchtool").is_none());
    let slot = slot_of("pen").unwrap();
    assert_eq!(ui.tools_last_used.get(slot).map(String::as_str), Some("addAnchor"));
    assert!(ui.tools_last_used.get(slot_of("selection").unwrap()).is_none());
}

#[test]
fn keep_tools_beside_the_canvas_is_saved_and_off_in_state_saved_without_it() {
    let toolbar = json!({"name": "T", "slots": [["pen"]], "pos": [0.0, 0.0], "dock": 0});
    let tools_at = |workspace: &Workspace<String>| {
        let start = dock(workspace, DockEdge::Start);
        assert_eq!(start.len(), 2, "{start:#?}");
        start.iter().position(|c| groups(c) == [vec!["tools"]]).expect("Tools in the start dock")
    };
    let beside = read(&json!({"tools_beside_canvas": true, "custom_toolbars": [toolbar]})).ui;
    assert_eq!(tools_at(&beside.dock_workspace), 1, "the inner place, next to the canvas");
    let edge = read(&json!({"custom_toolbars": [toolbar]})).ui;
    assert_eq!(tools_at(&edge.dock_workspace), 0, "state saved without the flag keeps Tools at the window's edge");

    let record = json!({"name": "Mine", "toolsBesideCanvas": true, "toolbars": [toolbar]});
    assert_eq!(tools_at(&legacy_layout(&record, &[]).0), 1, "a workspace record keeps the setting");
    let record = json!({"name": "Mine", "toolbars": [toolbar]});
    assert_eq!(tools_at(&legacy_layout(&record, &[]).0), 0);
}
