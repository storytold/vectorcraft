use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> VectorcraftApp {
    VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default())
}

#[test]
fn panel_commands_group_split_reorder_float_and_restore_without_duplicate_ownership() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.move", json!({"panel":"properties", "anchor":"swatches", "before":"swatches"})).unwrap();
    let before_float = app.ui.docking.clone().unwrap();
    app.run("window.panel.float", json!({"panel":"properties", "x":50,"y":70,"width":330,"height":410})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().floating.iter().any(|g| g.panels == ["properties"]));
    app.run("window.panel.dock", json!({"panel":"properties"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap(), &before_float);
    app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
    assert!(!app.ui.docking.as_ref().unwrap().contains(&"properties".into()));
    app.run("window.panel.activate", json!({"panel":"properties"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap(), &before_float);
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(valid(layout));
    let unique: std::collections::HashSet<_> = layout.panels().into_iter().collect();
    assert_eq!(unique.len(), layout.panels().len());
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.docking, app.ui.docking);
    assert_eq!(restored.docking_hidden, app.ui.docking_hidden);
}

#[test]
fn invalid_panel_requests_are_atomic_and_do_not_enter_a_custom_workspace() {
    let mut app = app();
    for (id, params) in [
        ("window.panel.move", json!({"panel":"layers","anchor":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","before":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","zone":42})),
        ("window.panel.float", json!({"panel":"layers","width":-10})),
        ("window.panel.float", json!({"panel":"unknown"})),
        ("window.panel.float", json!({"panel":"layers","x":"invalid"})),
    ] {
        let old = app.ui.docking.clone();
        let hidden = app.ui.docking_hidden.clone();
        assert!(app.run(id, params).is_err(), "{id}");
        assert_eq!(app.ui.docking, old);
        assert_eq!(app.ui.docking_hidden, hidden);
    }
}

fn harness(app: VectorcraftApp, size: egui::Vec2) -> Harness<'static, VectorcraftApp> {
    let mut ready = false;
    let mut harness = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !ready {
                crate::theme::install_fonts(ui.ctx());
                egui_extras::install_image_loaders(ui.ctx());
                crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                ready = true;
                return;
            }
            crate::floating::track(app, ui.ctx());
            crate::dock::show(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
            crate::floating::show(app, ui.ctx());
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn drag(h: &mut Harness<'static, VectorcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn ordinary_panel_tab_drag_enters_shared_docking_and_can_redock_into_another_tab_group() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    drag(&mut h, source, egui::pos2(160.0, 140.0));
    let layout = h.state().ui.docking.as_ref().expect("ordinary tab drag starts shared docking");
    assert!(layout.floating.iter().any(|g| g.panels.contains(&"layers".into())), "{layout:?}");
    let source = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    let destination = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"properties".to_string()))).unwrap().rect.center();
    drag(&mut h, source, destination);
    let layout = h.state().ui.docking.as_ref().unwrap();
    assert!(!layout.floating.iter().any(|g| g.panels.contains(&"layers".into())));
    assert!(valid(layout));
}

#[test]
fn saved_workspace_restores_the_custom_tree_and_hidden_panel_placement() {
    let mut app = app();

    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":50,"y":80})).unwrap();
    app.run("window.panel.close", json!({"panel":"swatches"})).unwrap();
    let layout = app.ui.docking.clone();
    let hidden = app.ui.docking_hidden.clone();
    app.run("window.workspace.new", json!({"name":"Custom docking"})).unwrap();
    app.run("window.workspace", json!({"name":"Essentials"})).unwrap();
    assert!(app.ui.docking.is_none());
    app.run("window.workspace", json!({"name":"Custom docking"})).unwrap();
    assert_eq!(app.ui.docking, layout);
    assert_eq!(app.ui.docking_hidden, hidden);
}

#[test]
fn legacy_float_group_aliases_use_atomic_shared_groups_and_preserve_tools() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"layers","group":true,"x":30,"y":50})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap().floating[0].panels.len(), 3);
    app.run("window.panel.float", json!({"panel":"swatches","onto":"layers"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap().floating[0].panels.len(), 4);
    let before = app.ui.docking.clone();
    assert!(app.run("window.panel.float", json!({"panel":"properties","onto":"missing","group":true})).is_err());
    assert_eq!(app.ui.docking, before);
    app.run("window.panel.float", json!({"panel":"tools","x":70,"y":90})).unwrap();
    assert_eq!(app.ui.toolbar_pos, Some([70.0, 90.0]));
    assert_eq!(app.ui.docking, before);
    app.run("window.panel.dock", json!({"panel":"layers","group":true})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().floating.is_empty());
}

/// Explicit offscreen evidence; this creates no native windows.
#[test]
fn capture_custom_panel_docking_visual_fixtures() {
    let Some(directory) = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    for theme in crate::theme::Brightness::ALL {
        for width in [800.0, 1280.0] {
            for scale in [1.0, 1.5, 2.0] {
                let mut app = app();
                app.run("file.new", json!({})).unwrap();
                app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
                app.run("window.panel.float", json!({"panel":"properties","x":30,"y":90,"width":300,"height":440})).unwrap();
                let mut ready = false;
                let mut harness = Harness::builder().with_size(vec2(width, 800.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                    move |ui, app: &mut VectorcraftApp| {
                        if !ready {
                            egui_extras::install_image_loaders(ui.ctx());
                            crate::theme::install_fonts(ui.ctx());
                            crate::theme::apply(ui.ctx(), theme);
                            ready = true;
                            return;
                        }
                        crate::dock::show(app, ui);
                        egui::CentralPanel::default().show(ui, |ui| {
                            ui.label("Custom panel workspace");
                        });
                    },
                    app,
                );
                harness.input_mut().max_texture_side = Some(8192);
                harness.run_steps(5);
                assert!(valid(harness.state().ui.docking.as_ref().unwrap()));
                harness.render().unwrap().save(directory.join(format!("vectorcraft-shared-docking-{}-{width}-{scale}x.png", theme.id()))).unwrap();
            }
        }
    }
}

#[test]
fn escape_cancels_the_first_drag_without_replacing_the_legacy_workspace() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    assert!(h.state().ui.docking.is_some());
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none());
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.docking.is_none());
}

#[test]
fn serialized_layout_actions_cover_geometry_reorder_accordion_and_atomic_rejection() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeSplit":{"path":[],"size":{"Ratio":0.35}}}})).unwrap();
    assert!(
        matches!(app.ui.docking.as_ref().unwrap().root, Some(Node::Split { size: craft_ui::layout::SplitSize::Ratio(value), .. }) if (value - 0.35).abs() < 0.001)
    );
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"MoveFloating":{"panel":"properties","rect":[44,66,360,450]}}})).unwrap();
    assert_eq!(
        app.ui.docking.as_ref().unwrap().floating.iter().find(|group| group.panels.iter().any(|p| p == "properties")).unwrap().rect,
        [44.0, 66.0, 360.0, 450.0]
    );
    app.run("window.panel.layout", json!({"action":{"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"swatches"}}}}}))
        .unwrap();
    assert_eq!(group_members_for_test(app.ui.docking.as_ref().unwrap(), "swatches"), ["properties", "swatches"]);
    let before = app.ui.docking.clone();
    let hidden = app.ui.docking_hidden.clone();
    for action in [
        json!({"MoveFloating":{"panel":"properties","rect":[0,0,-5,30]}}),
        json!({"ResizeSplit":{"path":[true,true,true],"size":{"Ratio":0.2}}}),
        json!({"Open":{"panel":"unknown","anchor":null}}),
        json!({"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"unknown"}}}}),
    ] {
        assert!(app.run("window.panel.layout", json!({"action":action})).is_err());
        assert_eq!(app.ui.docking, before);
        assert_eq!(app.ui.docking_hidden, hidden);
    }
    app.ui.docking = Some(Layout {
        root: Some(Node::Stack { entries: vec![craft_ui::docking::StackEntry { panel: "layers".into(), open: true, height: Some(100.0) }] }),
        floating: Vec::new(),
    });
    app.run("window.panel.layout", json!({"action":{"SetStackOpen":{"panel":"layers","open":false}}})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeStack":{"panel":"layers","height":150}}})).unwrap();
    let Some(Node::Stack { entries }) = &app.ui.docking.as_ref().unwrap().root else { panic!("expected stack") };
    assert!(!entries[0].open);
    assert_eq!(entries[0].height, Some(150.0));
}

fn group_members_for_test(layout: &Layout<String>, panel: &str) -> Vec<String> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Tabs { panels, .. } if panels.iter().any(|p| p == panel) => return panels.clone(),
            Node::Split { first, second, .. } => pending.extend([first.as_ref(), second.as_ref()]),
            _ => {}
        }
    }
    Vec::new()
}

#[test]
fn populated_canvas_keeps_a_real_stroke_edit_after_floating_and_redocking() {
    let mut app = app();
    app.run("file.new", json!({"width":480,"height":360})).unwrap();
    let id = app.run("shape.rectangle", json!({"x":80,"y":70,"width":250,"height":160})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids":[id]})).unwrap();
    app.run("paint.setFill", json!({"color":"#5297bb"})).unwrap();
    app.run("stroke.set", json!({"weight":8})).unwrap();
    app.run("window.panel.float", json!({"panel":"stroke","x":40,"y":140,"width":310,"height":490})).unwrap();
    let directory = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from);
    let builder = Harness::builder().with_size(vec2(1280.0, 800.0));
    let builder = if directory.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.input_mut().max_texture_side = Some(8192);
    h.run_steps(5);
    let mut pending: Vec<_> = h.output().shapes.iter().map(|shape| &shape.shape).collect();
    let mut checkbox = None;
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "Dashed Line" => {
                checkbox = Some(text.visual_bounding_rect().center());
                break;
            }
            egui::Shape::Vec(shapes) => pending.extend(shapes),
            _ => {}
        }
    }
    let at = checkbox.expect("painted Dashed Line checkbox");
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    let dash = crate::panels::current_stroke(h.state()).unwrap().dash.unwrap();
    assert_eq!(dash.pattern, [12.0, 12.0]);
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
        h.render().unwrap().save(directory.join("vectorcraft-populated-stroke-floating.png")).unwrap();
    }
    let from = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"stroke".to_string()))).unwrap().rect.center();
    let to = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"properties".to_string()))).unwrap().rect.center();
    drag(&mut h, from, to);
    assert!(h.state().ui.docking.as_ref().unwrap().floating.is_empty());
    assert_eq!(crate::panels::current_stroke(h.state()).unwrap().dash.unwrap(), dash);
    if let Some(directory) = &directory {
        h.render().unwrap().save(directory.join("vectorcraft-populated-stroke-redocked.png")).unwrap();
    }
}

#[test]
fn changing_workspace_during_a_drag_cancels_the_stale_gesture() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let destination = app.ui.docking.clone();
    app.run("window.workspace.new", json!({"name":"Destination"})).unwrap();
    app.run("window.workspace", json!({"name":"Essentials"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    h.state_mut().run("window.workspace", json!({"name":"Destination"})).unwrap();
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(h.state().ui.docking, destination);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.docking, destination);
}

#[test]
fn floating_icon_groups_redock_to_the_rail_and_keep_saved_origins() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"color"})).unwrap();
    app.run("window.panel.float", json!({"panel":"swatches", "onto":"color"})).unwrap();
    assert_eq!(app.ui.docking_icons.len(), 2);
    let workspace = crate::workspaces::capture(&app.ui, "Icon origins");
    let saved = serde_json::to_vec(&workspace).unwrap();
    let restored: crate::workspaces::Workspace = serde_json::from_slice(&saved).unwrap();
    crate::workspaces::apply(&mut app.ui, &restored);
    app.run("window.panel.dock", json!({"panel":"color", "group":true})).unwrap();
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(!layout.contains(&"color".into()));
    assert!(!layout.contains(&"swatches".into()));
    assert!(app.ui.floating_panels.is_empty());
    assert_eq!(app.ui.open_panel.as_deref(), Some("color"), "the requested icon pops out after redocking");
    app.run("window.panel", json!({"panel":"color"})).unwrap();
    assert!(app.ui.open_panel.is_none(), "the same icon puts it away");
    app.run("window.panel", json!({"panel":"color"})).unwrap();
    assert_eq!(app.ui.open_panel.as_deref(), Some("color"));
}

#[test]
fn all_dock_group_origins_survive_float_and_return() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"layers", "group":true})).unwrap();
    assert_eq!(app.ui.floating_panels[0].panels.len(), 3);
    assert!(app.ui.docking_hidden.contains_key("properties"));
    assert!(app.ui.docking_hidden.contains_key("layers"));
    assert!(app.ui.docking_hidden.contains_key("libraries"));
    app.run("window.panel.dock", json!({"panel":"layers", "group":true})).unwrap();
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(layout.floating.is_empty());
    assert_eq!(layout.panels().into_iter().map(String::as_str).collect::<Vec<_>>(), ["properties", "layers", "libraries"]);
    assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers);
    assert!(valid(layout));
}

#[test]
fn panel_specific_menu_executes_after_first_float_regroup_and_reload() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    app.run("window.panel.float", json!({"panel":"swatches","x":40,"y":100})).unwrap();
    let mut h = harness(app, vec2(1280.0, 900.0));
    for stage in 0..3 {
        if stage == 1 {
            h.state_mut().run("window.panel.float", json!({"panel":"color","x":60,"y":120})).unwrap();
            h.state_mut().run("window.panel.float", json!({"panel":"swatches","onto":"color"})).unwrap();
        } else if stage == 2 {
            let saved = serde_json::to_vec(&h.state().ui).unwrap();
            h.state_mut().ui = serde_json::from_slice(&saved).unwrap();
        }
        h.run_steps(4);
        let at = h
            .query_all_by_label("Panel menu")
            .find(|node| node.rect().center().x < 800.0)
            .expect("floating panel menu has an accessible button")
            .rect()
            .center();
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
        let action = h.query_all_by_label("   New Swatch…").next().expect("app-specific menu action remains reachable").rect().center();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: action, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
        assert_eq!(h.state().ui.dialog.as_ref().map(|dialog| dialog.kind.as_str()), Some("newSwatch"), "stage {stage}: actual panel action executes");
        h.state_mut().ui.dialog = None;
    }
}

#[test]
fn iconic_origin_tracks_explicit_custom_dock_and_legacy_saved_floats() {
    let mut custom = app();
    custom.run("window.panel.move", json!({"panel":"color","anchor":"layers"})).unwrap();
    let before = custom.ui.docking.clone().unwrap();
    custom.run("window.panel.float", json!({"panel":"color"})).unwrap();
    custom.run("window.panel.dock", json!({"panel":"color"})).unwrap();
    assert_eq!(custom.ui.docking.as_ref(), Some(&before), "explicit custom tabs are the saved destination");
    assert!(custom.ui.open_panel.is_none(), "a docked custom tab is not an icon popout");

    let mut legacy = app();
    crate::floating::float(&mut legacy.ui, &["color"], "color", egui::pos2(120.0, 160.0));
    let saved = serde_json::to_vec(&legacy.ui).unwrap();
    legacy.ui = serde_json::from_slice(&saved).unwrap();
    assert!(legacy.ui.docking.is_none());
    legacy.run("window.panel.dock", json!({"panel":"color"})).unwrap();
    assert!(!legacy.ui.docking.as_ref().unwrap().contains(&"color".into()), "legacy icon floats return to rail");
    assert_eq!(legacy.ui.open_panel.as_deref(), Some("color"));
}

#[test]
fn native_sets_columns_and_collapse_preserve_shared_splits_and_saved_origins() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let original = app.ui.docking.clone().unwrap();
    app.run("window.panel.float", json!({"panel":"stroke","x":100,"y":100})).unwrap();
    app.run("window.panel.float", json!({"panel":"color","below":"stroke","collapsed":true})).unwrap();
    let root = app.ui.docking.as_ref().unwrap().root.clone();
    assert_eq!(root, original.root);
    let column = app.ui.floating_panels.first().unwrap().column;
    assert!(column.is_some());
    assert_eq!(app.ui.floating_panels.len(), 2);
    assert!(app.ui.floating_panels.iter().all(|g| g.column == column));
    app.run("window.panel.dock", json!({"panel":"stroke","group":true,"column":true})).unwrap();
    assert!(app.ui.floating_panels.iter().all(|g| g.docked == Some(1)));
    app.run("window.workspace.new", json!({"name":"Mixed shared native"})).unwrap();
    let saved = app.ui.floating_panels.clone();
    app.run("window.workspace", json!({"name":"Essentials"})).unwrap();
    app.run("window.workspace", json!({"name":"Mixed shared native"})).unwrap();
    assert_eq!(app.ui.floating_panels, saved);
    assert_eq!(app.ui.docking.as_ref().unwrap().root, root);
    app.run("window.panel.float", json!({"panel":"stroke","group":true})).unwrap();
    assert!(app.ui.floating_panels.iter().all(|g| g.docked.is_none()));
    let before = serde_json::to_value(&app.ui).unwrap();
    assert!(app.run("window.panel.float", json!({"panel":"stroke","below":"unknown"})).is_err());
    // Error reporting may change status, but ownership, metadata and root remain atomic.
    let mut after = serde_json::to_value(&app.ui).unwrap();
    let mut before = before;
    before.as_object_mut().unwrap().remove("status");
    after.as_object_mut().unwrap().remove("status");
    assert_eq!(before, after);
    app.run("window.panel.dock", json!({"panel":"stroke","group":true})).unwrap();
    app.run("window.panel.dock", json!({"panel":"color","group":true})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap().root, original.root);
    assert!(app.ui.floating_panels.is_empty());
    assert!(valid(app.ui.docking.as_ref().unwrap()));
}

#[test]
fn native_set_close_pointer_restores_shared_split_ownership() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let root = app.ui.docking.as_ref().unwrap().root.clone();
    app.run("window.panel.float", json!({"panel":"stroke","x":70,"y":90})).unwrap();
    app.run("window.panel.float", json!({"panel":"color","below":"stroke"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 900.0));
    if let Some(directory) = std::env::var_os("CRAFT_UI_DOCKING_SNAPSHOT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        h.render().unwrap().save(directory.join("vectorcraft-native-set-shared-split.png")).unwrap();
    }
    let area = h.ctx.memory(|m| m.area_rect(crate::floating::area_id("stroke"))).unwrap();
    let at = egui::pos2(area.right() - 10.0, area.top() + 1.0 + crate::floating::TITLE / 2.0);
    for pressed in [true, false] {
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    assert!(h.state().ui.floating_panels.is_empty());
    assert_eq!(h.state().ui.docking.as_ref().unwrap().root, root);
    assert!(valid(h.state().ui.docking.as_ref().unwrap()));
}

#[test]
fn a_single_native_tab_redock_retains_the_remaining_float_and_return_origin() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let root = app.ui.docking.as_ref().unwrap().root.clone();
    app.run("window.panel.float", json!({"panel":"swatches","x":80,"y":90})).unwrap();
    app.run("window.panel.float", json!({"panel":"color","onto":"swatches","collapsed":true})).unwrap();
    assert!(
        matches!(app.ui.docking.as_ref().unwrap().root.as_ref(), Some(Node::Tabs { panels, active }) if panels.get(*active).is_some_and(|panel| panel == "properties")),
        "floating Color intentionally activates the first root fallback, Properties"
    );
    let mut expected = Layout { root, floating: vec![] };
    expected.apply(Action::Activate { panel: "properties".to_string() }).unwrap();
    app.run("window.panel.dock", json!({"panel":"swatches"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap().root, expected.root);
    assert_eq!(app.ui.floating_panels.len(), 1);
    assert_eq!(app.ui.floating_panels[0].panels, ["color"]);
    assert!(valid(app.ui.docking.as_ref().unwrap()));
}

#[test]
fn explicit_shared_activation_expands_a_native_collapsed_owner() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"layers","collapsed":true})).unwrap();
    assert!(app.ui.floating_panels[0].collapsed);
    app.run("window.panel.layout", json!({"action":Action::Activate { panel:"layers".to_string() }})).unwrap();
    assert!(!app.ui.floating_panels[0].collapsed);
    assert_eq!(app.ui.floating_panels[0].panels, ["layers"]);
    assert!(valid(app.ui.docking.as_ref().unwrap()));
}

#[test]
fn escape_restores_native_set_position_and_metadata_after_several_frames() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"stroke","x":80,"y":100})).unwrap();
    app.run("window.panel.float", json!({"panel":"color","below":"stroke"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 900.0));
    let original = h.state().ui.floating_panels.clone();
    let layout = h.state().ui.docking.clone();
    let area = h.ctx.memory(|m| m.area_rect(crate::floating::area_id("stroke"))).unwrap();
    let from = area.left_top() + vec2(30.0, 1.0 + crate::floating::TITLE / 2.0);
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from + vec2(step as f32 * 12.0, step as f32 * 9.0)));
        h.run_steps(1);
    }
    assert_ne!(h.state().ui.floating_panels, original, "actual title gesture moved the set");
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.floating_panels, original);
    assert_eq!(h.state().ui.docking, layout);
}

#[test]
fn first_legacy_icon_tear_can_join_a_native_set() {
    let mut app = app();
    crate::floating::float(&mut app.ui, &["stroke"], "stroke", egui::pos2(100.0, 100.0));
    crate::floating::float(&mut app.ui, &["color"], "color", egui::pos2(100.0, 100.0));
    crate::floating::attach(&mut app.ui, &[1], "stroke", true);
    assert!(app.ui.docking.is_none());
    let mut h = harness(app, vec2(1280.0, 900.0));
    let source = h.query_all_by_label("Swatches").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    let color = h.ctx.data(|data| data.get_temp::<egui::Rect>(crate::floating::group_rect_id("color"))).unwrap();
    drag(&mut h, source, egui::pos2(color.center().x, color.bottom() + 2.0));
    assert!(h.state().ui.docking.is_some(), "successful first tear retains shared ownership");
    let groups = &h.state().ui.floating_panels;
    let color = groups.iter().find(|g| g.panels.iter().any(|p| p == "color")).unwrap();
    let swatches = groups.iter().find(|g| g.panels.iter().any(|p| p == "swatches")).unwrap();
    assert!(color.column.is_some());
    assert_eq!(swatches.column, color.column);
    assert!(valid(h.state().ui.docking.as_ref().unwrap()));
}

#[test]
fn native_set_drop_targets_respect_the_visible_layer_above_an_ordinary_float() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":100,"y":100})).unwrap();
    app.run("window.panel.float", json!({"panel":"stroke","x":100,"y":100})).unwrap();
    app.run("window.panel.float", json!({"panel":"color","below":"stroke"})).unwrap();
    let h = harness(app, vec2(1280.0, 900.0));
    let rect = h.ctx.data(|data| data.get_temp::<egui::Rect>(crate::floating::group_rect_id("stroke"))).unwrap();
    let at = rect.center();
    assert_eq!(h.ctx.layer_id_at(at).unwrap().id, crate::floating::area_id("stroke"), "native set is visibly above the ordinary shared float");
    let target = crate::floating::drop_target(h.state(), &h.ctx, crate::floating::Moving::Panel("layers"), at).unwrap().0;
    assert!(
        matches!(target, crate::floating::Drop::Above("stroke") | crate::floating::Drop::Below("stroke")),
        "visible native group owns the drop: {target:?}"
    );
}

/// Window › Properties hides the panel while it shows, and the next click shows it again where it
/// was (#2361): in the dock as the app starts, floating, and once the layout has been changed.
/// The panel's other ways in (a tool's double-click, Show Properties Panel) only show it.
#[test]
fn the_window_menu_item_hides_a_shown_panel_and_shows_it_again() {
    let item = crate::menus::menu_entries(&app())
        .into_iter()
        .find(|e| e.path == ["Window"] && e.command.as_deref() == Some("window.panel") && e.params["panel"] == "properties")
        .expect("Window › Properties");
    let params = item.params;
    let shown = |app: &VectorcraftApp| crate::menus::checked(app, "window.panel", &params) == Some(true);
    let in_layout = |app: &VectorcraftApp| app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&"properties".into()));

    let mut app = app();
    assert!(app.ui.docking.is_none() && shown(&app), "the default dock shows Properties");
    let start = legacy(&app);
    app.run("window.panel", params.clone()).unwrap();
    assert!(!shown(&app) && !in_layout(&app), "the first click hides it");
    app.run("window.panel", params.clone()).unwrap();
    assert!(shown(&app));
    assert_eq!(app.ui.docking.as_ref(), Some(&start), "the second shows it where it was");

    app.run("window.panel.move", json!({"panel":"properties", "anchor":"layers", "zone":"top"})).unwrap();
    let moved = app.ui.docking.clone();
    app.run("window.panel", params.clone()).unwrap();
    assert!(!shown(&app) && !in_layout(&app));
    app.run("window.panel", params.clone()).unwrap();
    assert!(shown(&app) && app.ui.docking == moved);

    app.run("window.panel.float", json!({"panel":"properties", "x":50, "y":70, "width":330, "height":410})).unwrap();
    let floating = app.ui.docking.clone();
    app.run("window.panel", params.clone()).unwrap();
    assert!(!shown(&app) && !in_layout(&app), "a floating Properties hides too");
    app.run("window.panel", params.clone()).unwrap();
    assert!(shown(&app) && app.ui.docking == floating);

    app.run("window.panel", json!({"panel":"properties"})).unwrap();
    assert!(shown(&app), "without the menu's toggle the command only shows the panel");
}
