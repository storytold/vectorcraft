use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
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

/// A floating group lies on the canvas, not on the dock's card, so its tab strip and body paint
/// their own opaque background (#2421, #2646).
#[test]
fn floating_group_paints_an_opaque_background() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties","x":400,"y":200,"width":300,"height":420})).unwrap();
    let h = harness(app, vec2(1280.0, 800.0));
    // Paint order: the last big filled rectangle over a point decides what the user sees there
    // (outlines, small widgets and fully transparent fills, which paint nothing, don't count).
    fn topmost(shape: &egui::Shape, p: Pos2, found: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.rect.contains(p) && r.rect.width() >= 200.0 && r.stroke.is_empty() && r.fill.a() > 0 => *found = Some(r.fill),
            egui::Shape::Vec(v) => v.iter().for_each(|s| topmost(s, p, found)),
            _ => {}
        }
    }
    let opaque_at = |p: Pos2| {
        let mut found = None;
        h.output().shapes.iter().for_each(|s| topmost(&s.shape, p, &mut found));
        found.is_some_and(|c| c.a() == 255)
    };
    // Tab strip to the right of the tabs, and the body's empty bottom-right corner.
    assert!(opaque_at(pos2(690.0, 210.0)), "tab strip is transparent");
    assert!(opaque_at(pos2(690.0, 610.0)), "body is transparent");
}

/// Undocking a tabbed dock panel leaves an opaque backdrop even in the empty part of its body (#2646).
#[test]
fn undocked_layers_panel_body_is_opaque() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"layers","x":300,"y":160,"width":300,"height":420})).unwrap();
    let h = harness(app, vec2(1280.0, 800.0));
    fn topmost(shape: &egui::Shape, p: Pos2, found: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.rect.contains(p) && r.rect.width() >= 80.0 && r.stroke.is_empty() && r.fill.a() > 0 => *found = Some(r.fill),
            egui::Shape::Vec(v) => v.iter().for_each(|s| topmost(s, p, found)),
            _ => {}
        }
    }
    let opaque_at = |p: Pos2| {
        let mut found = None;
        h.output().shapes.iter().for_each(|s| topmost(&s.shape, p, &mut found));
        found.is_some_and(|c| c.a() == 255)
    };
    assert!(opaque_at(pos2(580.0, 520.0)), "empty layers body shows the canvas");
}

/// Library panels (#2603) float in the shared layout titled by their libraries' names, and their
/// menu's Persistent item keeps them for the next launch.
#[test]
fn library_panels_float_with_their_names_and_a_persistent_menu_item() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    for (library, x) in [("pastels", 40), ("brights", 420)] {
        let panel = format!("library:swatches:{library}");
        app.run("window.swatchLibrary", json!({"library": library})).unwrap();
        app.run("window.panel.float", json!({"panel": panel, "x": x, "y": 100})).unwrap();
    }
    let mut h = harness(app, vec2(1280.0, 900.0));
    for name in ["Pastels", "Brights"] {
        assert!(h.query_all_by_label(name).next().is_some(), "the {name} tab");
    }
    let click = |h: &mut Harness<'static, VectorcraftApp>, at: Pos2| {
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
    };
    let menu = h.query_all_by_label("Panel menu").find(|node| node.rect().center().x < 400.0).expect("the Pastels panel's menu").rect().center();
    click(&mut h, menu);
    let item = h.query_all_by_label("   Persistent").next().expect("Persistent is in the panel menu").rect().center();
    click(&mut h, item);
    let persistent: Vec<&str> = h.state().ui.library_panels.iter().filter(|p| p.persistent).map(|p| p.id.as_str()).collect();
    assert_eq!(persistent, ["pastels"]);
}

/// The dock groups the last frame registered as drop targets.
fn root_groups(h: &Harness<'static, VectorcraftApp>) -> Vec<crate::floating::RootGroup> {
    h.ctx.data(|d| d.get_temp::<Vec<crate::floating::RootGroup>>(crate::floating::root_groups_id())).unwrap_or_default()
}

/// The dock's group holding `panel`, as panel ids.
fn root_group_of(app: &VectorcraftApp, panel: &str) -> Vec<String> {
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(Layout { root: layout.root.clone(), floating: Vec::new() }.contains(&panel.to_string()), "{panel} is in the dock: {layout:?}");
    group_members(layout, panel)
}

/// A legacy (native) floating group holding `panel`, as the Window menu or an icon tear leaves it
/// before the shared layout takes over, and a point on its title bar.
fn native_float(h: &mut Harness<'static, VectorcraftApp>, panel: &'static str) -> Pos2 {
    crate::floating::float(&mut h.state_mut().ui, &[panel], panel, egui::pos2(300.0, 300.0));
    h.run_steps(3);
    assert!(h.state().ui.docking.is_none(), "still the legacy dock");
    let r = h.ctx.memory(|m| m.area_rect(crate::floating::area_id(panel))).unwrap();
    egui::pos2(r.left() + 30.0, r.top() + 1.0 + crate::floating::TITLE / 2.0)
}

/// A floating group dropped on the dock's tab strip joins the tabbed group as tabs.
#[test]
fn a_floating_group_dropped_on_the_dock_tabs_joins_the_tabbed_group() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let title = native_float(&mut h, "pathfinder");
    let groups = root_groups(&h);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].panels, ["properties", "layers", "libraries"]);
    let onto = groups[0].strip.right_center() - vec2(60.0, 0.0);
    drag(&mut h, title, onto);
    assert!(h.state().ui.floating_panels.is_empty(), "{:?}", h.state().ui.floating_panels);
    assert_eq!(root_group_of(h.state(), "pathfinder"), ["properties", "layers", "libraries", "pathfinder"]);
    assert!(!h.state().ui.docking_icons.contains("pathfinder"), "its home is the tabbed group now");
    assert!(valid(h.state().ui.docking.as_ref().unwrap()));
}

/// A floating group dropped on a dock group's lower half becomes a new group below it.
#[test]
fn a_floating_group_dropped_on_the_lower_half_of_the_dock_splits_it() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let title = native_float(&mut h, "pathfinder");
    let group = root_groups(&h)[0].rect;
    drag(&mut h, title, egui::pos2(group.center().x, group.bottom() - 80.0));
    assert!(h.state().ui.floating_panels.is_empty());
    let layout = h.state().ui.docking.clone().unwrap();
    let Some(Node::Split { axis: craft_ui::layout::SplitAxis::Vertical, second, .. }) = &layout.root else {
        panic!("a top/bottom split: {layout:?}")
    };
    assert_eq!(**second, Node::Tabs { panels: vec!["pathfinder".into()], active: 0 });
    assert_eq!(root_group_of(h.state(), "layers"), ["properties", "layers", "libraries"]);
    // The two groups are drawn one above the other, and another group dropped on the upper half of
    // the lower one goes between them.
    h.run_steps(3);
    let groups = root_groups(&h);
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert!(groups[0].rect.bottom() <= groups[1].rect.top() + 1.0, "{groups:?}");
    assert_eq!(groups[1].panels, ["pathfinder"]);
    let lower = groups[1].rect;
    h.state_mut().run("window.panel.float", json!({"panel":"align","x":300,"y":300})).unwrap();
    h.run_steps(3);
    let title =
        h.ctx.memory(|m| m.area_rect(egui::Id::new("vectorcraft-panel-docking").with(("floating", &"align".to_string())))).unwrap().left_top()
            + vec2(6.0, 16.0);
    drag(&mut h, title, egui::pos2(lower.center().x, lower.top() + 50.0));
    assert!(h.state().ui.docking.as_ref().unwrap().floating.is_empty());
    h.run_steps(3);
    let panels: Vec<_> = root_groups(&h).into_iter().map(|g| g.panels).collect();
    assert_eq!(panels, [vec!["properties", "layers", "libraries"], vec!["align"], vec!["pathfinder"]]);
}

/// A floating group dropped on the icon column becomes icons there.
#[test]
fn a_floating_group_dropped_on_the_icon_column_becomes_icons() {
    let mut h = harness(app(), vec2(1280.0, 1400.0));
    h.state_mut().run("window.panel.float", json!({"panel":"layers","x":300,"y":300})).unwrap();
    h.run_steps(3);
    let title =
        h.ctx.memory(|m| m.area_rect(egui::Id::new("vectorcraft-panel-docking").with(("floating", &"layers".to_string())))).unwrap().left_top()
            + vec2(6.0, 16.0);
    let column = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::icons_rect_id())).unwrap();
    drag(&mut h, title, column.center());
    let ui = &h.state().ui;
    assert!(ui.docking_icons.contains("layers"));
    assert!(!ui.docking.as_ref().unwrap().contains(&"layers".to_string()));
    assert_eq!(ui.open_panel.as_deref(), Some("layers"), "it pops out of its new icon");
}

/// A dock tab dragged onto the icon column becomes an icon whose flyout shows the panel.
#[test]
fn a_dock_tab_dragged_onto_the_icon_column_becomes_an_icon_and_its_flyout_shows() {
    let mut h = harness(app(), vec2(1280.0, 1400.0));
    let tab = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    let column = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::icons_rect_id())).unwrap();
    drag(&mut h, tab, column.center());
    let ui = &h.state().ui;
    assert!(ui.docking_icons.contains("layers"), "{:?}", ui.docking_icons);
    assert_eq!(root_group_of(h.state(), "properties"), ["properties", "libraries"]);
    // Its icon heads the column; a click puts the popped-out panel away and another shows it again.
    h.state_mut().ui.open_panel = None;
    h.run_steps(3);
    let column = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::icons_rect_id())).unwrap();
    let icon = h.query_all_by_label("Layers").map(|node| node.rect()).find(|r| column.contains_rect(*r)).expect("a Layers icon in the column");
    h.event(Event::PointerMoved(icon.center()));
    h.event(Event::PointerButton { pos: icon.center(), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: icon.center(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.open_panel.as_deref(), Some("layers"));
    // The preferences keep it in the column.
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert!(restored.sanitized().docking_icons.contains("layers"));
    // Dragged from its icon back onto the tabbed group's tabs, it is a tab again.
    let strip = root_groups(&h)[0].strip;
    drag(&mut h, icon.center(), strip.right_center() - vec2(30.0, 0.0));
    assert_eq!(root_group_of(h.state(), "layers"), ["properties", "libraries", "layers"]);
}

/// A dock tab's context menu closes its panel, and the Window menu brings it back.
#[test]
fn a_dock_tab_context_menu_closes_its_panel_and_the_window_menu_brings_it_back() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    assert!(h.state().ui.docking.is_none());
    let tab = h.query_all_by_label("Libraries").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(tab));
    h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    h.get_by_label("Close Panel").click();
    h.run_steps(3);
    let layout = h.state().ui.docking.clone().expect("closing starts the shared layout");
    assert!(!layout.contains(&"libraries".to_string()));
    assert_eq!(root_group_of(h.state(), "layers"), ["properties", "layers"]);
    // In the shared dock the tabs' context menu closes them too.
    let tab = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    h.event(Event::PointerMoved(tab));
    h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    h.get_by_label("Close Panel").click();
    h.run_steps(3);
    assert_eq!(root_group_of(h.state(), "properties"), ["properties"]);
    h.state_mut().run("window.panel", json!({"panel":"libraries"})).unwrap();
    h.state_mut().run("window.panel", json!({"panel":"layers"})).unwrap();
    let mut panels = root_group_of(h.state(), "properties");
    panels.sort();
    assert_eq!(panels, ["layers", "libraries", "properties"]);
}

/// `window.panel.dock {icon}` and `{anchor, zone}` check their parameters and survive a workspace switch.
#[test]
fn icon_docking_commands_validate_and_round_trip_through_workspaces() {
    let mut app = app();
    for params in [json!({"panel":"layers","icon":3}), json!({"panel":"layers","icon":true,"anchor":"properties"})] {
        assert!(app.run("window.panel.dock", params).is_err());
        assert!(app.ui.docking.is_none());
    }
    assert!(app.run("window.panel.move", json!({"panel":"layers","anchor":"properties","icon":true})).is_err());
    let r = app.run("window.panel.dock", json!({"panel":"layers","icon":true})).unwrap();
    assert!(r.get("layout").is_some() && r.get("column").is_none(), "the shared layout's result: {r}");
    assert!(app.ui.docking_icons.contains("layers"));
    assert!(!app.ui.docking.as_ref().unwrap().contains(&"layers".to_string()));
    let r = app.run("window.panel.dock", json!({"panel":"pathfinder","anchor":"properties","zone":"bottom"})).unwrap();
    assert!(r.get("layout").is_some() && r.get("column").is_none(), "the shared layout's result: {r}");
    assert!(matches!(&app.ui.docking.as_ref().unwrap().root, Some(Node::Split { axis: craft_ui::layout::SplitAxis::Vertical, .. })));
    app.run("window.workspace.new", json!({"name":"Icons"})).unwrap();
    app.run("window.workspace", json!({"name":"Essentials"})).unwrap();
    assert!(!app.ui.docking_icons.contains("layers"));
    app.run("window.workspace", json!({"name":"Icons"})).unwrap();
    assert!(app.ui.docking_icons.contains("layers"));
    assert_eq!(root_group_of(&app, "pathfinder"), ["pathfinder"]);
    // From the Window menu it pops out of its icon.
    app.run("window.panel", json!({"panel":"layers"})).unwrap();
    assert_eq!(app.ui.open_panel.as_deref(), Some("layers"));
}

/// `column` goes with neither `anchor` nor `icon: true`, so it is never silently dropped;
/// `icon: false` is the same as no `icon`, so a dock column still takes the panel.
#[test]
fn a_dock_column_is_refused_with_an_anchor_or_icon_and_icon_false_is_no_icon() {
    let mut app = app();
    for params in [json!({"panel":"color","column":1,"anchor":"properties"}), json!({"panel":"color","column":true,"icon":true})] {
        assert!(app.run("window.panel.dock", params.clone()).is_err(), "{params}");
        assert!(app.ui.docking.is_none() && app.ui.floating_panels.is_empty(), "{params} changed nothing");
    }
    let r = app.run("window.panel.dock", json!({"panel":"color","column":true,"icon":false})).unwrap();
    assert_eq!((r["column"].clone(), r["floating"].clone()), (json!(1), json!(false)));
}

/// A group dropped on the icon column leaves what the column can't show (a library panel) in the
/// dock instead of losing it.
#[test]
fn a_library_panel_dropped_on_the_icon_column_with_its_group_stays_in_the_dock() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    let library = "library:swatches:pastels".to_string();
    app.run("window.panel.float", json!({"panel": library, "x": 300, "y": 100})).unwrap();
    app.run("window.panel.move", json!({"panel":"color", "anchor": library})).unwrap();
    assert_eq!(group_members(app.ui.docking.as_ref().unwrap(), &library).len(), 2, "one floating group");
    app.run("window.panel.dock", json!({"panel": library, "group": true, "icon": true})).unwrap();
    assert!(app.ui.docking_icons.contains("color"));
    assert!(!app.ui.docking_icons.contains(&library));
    assert!(Layout { root: app.ui.docking.as_ref().unwrap().root.clone(), floating: Vec::new() }.contains(&library), "in the dock");
}

/// A panel outside the icon groups still gets an icon once dropped on the column.
#[test]
fn a_panel_the_icon_column_has_no_place_for_still_shows_there_once_dropped_on_it() {
    let mut h = harness(app(), vec2(1280.0, 1600.0));
    assert!(!crate::state::ICON_PANEL_GROUPS.iter().any(|group| group.contains(&"charStyles")));
    h.state_mut().run("window.panel.float", json!({"panel":"charStyles","x":300,"y":300})).unwrap();
    h.run_steps(3);
    let title =
        h.ctx.memory(|m| m.area_rect(egui::Id::new("vectorcraft-panel-docking").with(("floating", &"charStyles".to_string())))).unwrap().left_top()
            + vec2(6.0, 16.0);
    let column = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::icons_rect_id())).unwrap();
    drag(&mut h, title, column.center());
    assert!(h.state().ui.docking_icons.contains("charStyles"));
    h.state_mut().ui.open_panel = None;
    h.run_steps(3);
    let column = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::icons_rect_id())).unwrap();
    assert!(h.query_all_by_label("Character Styles").any(|node| column.contains_rect(node.rect())), "its icon is in the column");
}

/// Double-clicks at `at`, then waits out the double-click time.
fn double_click(h: &mut Harness<'static, VectorcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    // Past the double- and triple-click times, so the next press is a click of its own.
    h.run_steps(45);
}

/// The Layers panel's body is drawn (its search field).
fn layers_drawn(h: &Harness<'static, VectorcraftApp>) -> bool {
    h.ctx.read_response(egui::Id::new("layers-search").with("edit")).is_some()
}

/// Clicks at `at`.
fn click(h: &mut Harness<'static, VectorcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// The middle of `panel`'s tab in the shared dock.
/// Two stacked groups both collapsed are both as short as their tabs: the lower one doesn't take
/// the rest of the dock as a blank pane.
#[test]
fn two_stacked_groups_both_collapsed_are_both_as_short_as_their_tabs() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.collapse", json!({"panel":"swatches", "collapsed": true})).unwrap();
    app.run("window.panel.collapse", json!({"panel":"layers", "collapsed": true})).unwrap();
    let h = harness(app, vec2(1280.0, 800.0));
    let groups = root_groups(&h);
    assert_eq!(groups.len(), 2, "{groups:?}");
    for group in &groups {
        assert!(group.rect.height() < 40.0, "{groups:?}");
    }
}

fn tab_center(h: &Harness<'static, VectorcraftApp>, panel: &str) -> Pos2 {
    h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &panel.to_string()))).unwrap().rect.center()
}

/// A double-click on a tab of the shared dock collapses its group to its tabs; another tab expands it.
#[test]
fn a_double_click_on_a_dock_tab_collapses_its_group_to_its_tabs_and_another_expands_it() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    let saved = app.ui.docking.clone();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let groups = root_groups(&h);
    assert_eq!(groups.len(), 2, "{groups:?}");
    let (top, lower) = (groups[0].rect, groups[1].rect);
    assert!(top.height() > 100.0, "{top:?}");

    // The upper group shrinks to its tabs and the lower one takes the room; the saved layout keeps
    // its sizes.
    let at = tab_center(&h, "swatches");
    double_click(&mut h, at);
    assert!(h.state().ui.docking_collapsed.contains("swatches"));
    let groups = root_groups(&h);
    assert!(groups[0].rect.height() < 40.0, "{groups:?}");
    assert!(groups[1].rect.top() < lower.top() - 50.0, "{groups:?}");
    assert_eq!(h.state().ui.docking, saved);
    // Again, and it is as tall as before.
    let at = tab_center(&h, "swatches");
    double_click(&mut h, at);
    assert!(h.state().ui.docking_collapsed.is_empty());
    let groups = root_groups(&h);
    assert!((groups[0].rect.height() - top.height()).abs() < 2.0, "{groups:?} {top:?}");

    // The lower group, tabbed: its body goes, and a click on another of its tabs brings it back
    // showing that panel.
    let at = tab_center(&h, "layers");
    double_click(&mut h, at);
    assert!(h.state().ui.docking_collapsed.contains("layers"));
    assert!(!layers_drawn(&h), "no Layers body");
    let at = tab_center(&h, "properties");
    click(&mut h, at);
    assert!(h.state().ui.docking_collapsed.is_empty());
    assert_eq!(root_group_of(h.state(), "properties"), ["properties", "layers", "libraries"]);
    assert!(valid(h.state().ui.docking.as_ref().unwrap()));
}

/// A double-click on a tab of a group beside another does nothing, and leaves no error in the
/// status bar: that group doesn't collapse (`a_group_beside_another_does_not_collapse`).
#[test]
fn a_double_click_on_a_tab_of_a_group_beside_another_does_nothing() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"left"})).unwrap();
    let saved = app.ui.docking.clone();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let status = h.state().ui.status.clone();
    let shown: Vec<&str> = root_groups(&h).iter().map(|g| g.shown).collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    for panel in shown {
        let at = tab_center(&h, panel);
        double_click(&mut h, at);
        assert_eq!(h.state().ui.status, status, "no error: {panel}");
        assert!(h.state().ui.docking_collapsed.is_empty(), "{panel}");
        assert_eq!(h.state().ui.docking, saved, "{panel}");
    }
}

/// A double-click on a tab of the legacy dock collapses the tabbed group to its tabs.
#[test]
fn a_double_click_on_a_legacy_dock_tab_collapses_the_tabbed_group() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let tab = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    click(&mut h, tab);
    h.run_steps(45);
    assert!(layers_drawn(&h), "Layers shows");
    double_click(&mut h, tab);
    assert!(h.state().ui.docking.is_none(), "still the legacy dock");
    assert!(h.state().ui.docking_collapsed.contains("layers"));
    assert!(!layers_drawn(&h), "no Layers body");
    double_click(&mut h, tab);
    assert!(h.state().ui.docking_collapsed.is_empty());
    assert!(layers_drawn(&h), "Layers shows again");
}

/// Illustrator also collapses the group from the blank part of the strip, see
/// <https://helpx.adobe.com/illustrator/using/customizing-workspace.html>. The legacy dock's strip right of its tabs collapses the tabbed group, and floats nothing.
#[test]
fn a_double_click_on_the_legacy_dock_strip_right_of_the_tabs_collapses_the_tabbed_group() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let libraries = h.query_all_by_label("Libraries").find(|node| node.rect().height() <= 45.0).unwrap().rect();
    let dock = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::floating::dock_rect_id())).unwrap();
    let blank = pos2(libraries.right() + 12.0, libraries.center().y);
    assert!(blank.x < dock.right() - 30.0, "left of the panel menu: {libraries:?} {dock:?}");
    let tab = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    click(&mut h, tab);
    h.run_steps(45);
    assert!(layers_drawn(&h), "Layers shows");
    double_click(&mut h, blank);
    assert!(h.state().ui.docking.is_none() && h.state().ui.floating_panels.is_empty(), "nothing floats");
    assert!(h.state().ui.docking_collapsed.contains("layers"));
    assert!(!layers_drawn(&h), "no Layers body");
    double_click(&mut h, blank);
    assert!(h.state().ui.docking_collapsed.is_empty());
    assert!(layers_drawn(&h), "Layers shows again");
}

/// The shared dock's strip right of a group's tabs, and the grip beside its panel menu, collapse
/// the group to its tabs and expand it, without floating it.
#[test]
fn a_double_click_on_the_shared_dock_strip_right_of_the_tabs_collapses_its_group() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    let saved = app.ui.docking.clone();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let strip = root_groups(&h)[0].strip;
    let tab = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"swatches".to_string()))).unwrap().rect;
    // Between the tab and the group's header (80 wide), and on the header's grip.
    let gap = pos2(tab.right() + 10.0, strip.center().y);
    assert!(gap.x < strip.right() - 80.0, "{tab:?} {strip:?}");
    let grip = pos2(strip.right() - 50.0, strip.center().y);
    for at in [gap, grip] {
        double_click(&mut h, at);
        assert!(h.state().ui.docking_collapsed.contains("swatches"), "{at:?}");
        assert!(root_groups(&h)[0].rect.height() < 40.0, "{at:?}");
        double_click(&mut h, at);
        assert!(h.state().ui.docking_collapsed.is_empty(), "{at:?}");
        assert_eq!(h.state().ui.docking, saved, "nothing floats: {at:?}");
    }
}

/// `window.panel.collapse` checks its parameters, and a collapsed group survives a reload.
#[test]
fn the_collapse_command_validates_and_survives_a_reload() {
    let mut app = app();
    assert_eq!(app.run("window.panel.collapse", json!({"panel":"layers"})).unwrap(), json!({"panel":"layers", "collapsed":true}));
    assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers, "it shows the panel");
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap().sanitized();
    assert!(restored.docking_collapsed.contains("layers"));
    assert_eq!(app.run("window.panel.collapse", json!({"panel":"layers", "collapsed":false})).unwrap()["collapsed"], false);
    for params in [json!({"panel":"nope"}), json!({"panel":"layers", "collapsed":"yes"}), json!({"panel":"swatches"})] {
        assert!(app.run("window.panel.collapse", params.clone()).is_err(), "{params}");
    }
    app.run("window.panel.float", json!({"panel":"layers", "x":300, "y":300})).unwrap();
    assert!(app.run("window.panel.collapse", json!({"panel":"layers"})).is_err(), "floating groups collapse with float");
    assert!(app.ui.docking_collapsed.is_empty());
}

/// A panel moved out of a collapsed group doesn't take the collapse along.
#[test]
fn a_collapsed_groups_panel_moved_elsewhere_takes_no_collapse_along() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.collapse", json!({"panel":"layers"})).unwrap();
    // Layers, shown by the collapsed group, goes to the Swatches group and is shown there.
    app.run("window.panel.move", json!({"panel":"layers", "anchor":"swatches"})).unwrap();
    app.run("window.panel.activate", json!({"panel":"layers"})).unwrap();
    crate::panel_docking::prune_collapsed(&mut app.ui);
    assert!(app.ui.docking_collapsed.is_empty(), "neither group is collapsed");
    // The same through a serialized action (a drag).
    app.run("window.panel.collapse", json!({"panel":"layers"})).unwrap();
    app.run("window.panel.layout", json!({"action": {"Move": {"panel":"layers", "anchor":"properties", "placement": {"Tab": {"before": null}}}}}))
        .unwrap();
    assert!(app.ui.docking_collapsed.is_empty());
}

/// `window.panel.collapse` refuses a panel that is in an accordion, not a tab group.
#[test]
fn the_collapse_command_rejects_an_accordion_panel() {
    let mut app = app();
    app.ui.docking = Some(Layout {
        root: Some(Node::Stack {
            entries: vec![
                craft_ui::docking::StackEntry { panel: "layers".into(), open: true, height: None },
                craft_ui::docking::StackEntry { panel: "swatches".into(), open: true, height: None },
            ],
        }),
        floating: Vec::new(),
    });
    assert!(valid(app.ui.docking.as_ref().unwrap()));
    assert!(app.run("window.panel.collapse", json!({"panel":"layers"})).is_err());
    assert!(app.ui.docking_collapsed.is_empty());
}

/// Escape on a collapsed group's drag puts it back collapsed.
#[test]
fn escape_on_a_collapsed_groups_drag_leaves_it_collapsed() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.collapse", json!({"panel":"layers"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    // An icon panel's flyout is open too.
    h.state_mut().ui.open_panel = Some("color".into());
    h.run_steps(2);
    let groups = root_groups(&h);
    let lower = groups.iter().find(|g| g.panels.contains(&"layers")).unwrap().strip;
    // The grip right of the lower group's tabs, dragged out of the dock, then Escape.
    let from = lower.right_center() - vec2(50.0, 0.0);
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=6 {
        h.event(Event::PointerMoved(from - vec2(80.0 * step as f32, 0.0)));
        h.run_steps(1);
    }
    assert!(h.state().ui.docking.as_ref().unwrap().floating.iter().any(|g| g.panels.contains(&"layers".into())), "the group floats mid-drag");
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().ui.docking.as_ref().unwrap().floating.is_empty(), "back in the dock");
    assert!(h.state().ui.docking_collapsed.contains("layers"), "and still collapsed");
    assert_eq!(h.state().ui.open_panel.as_deref(), Some("color"), "the flyout is back");
}

/// A group beside another (a left/right split) is refused: it can't be shorter than its neighbour.
#[test]
fn a_group_beside_another_does_not_collapse() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"left"})).unwrap();
    let before = app.ui.docking.clone();
    assert!(app.run("window.panel.collapse", json!({"panel":"swatches"})).is_err());
    assert!(app.run("window.panel.collapse", json!({"panel":"properties"})).is_err());
    assert_eq!(app.ui.docking, before, "refused without a change");
    assert!(app.ui.docking_collapsed.is_empty());
    // One left collapsed by an earlier layout (a hand-edited file) is expanded.
    app.ui.docking_collapsed.insert("swatches".into());
    crate::panel_docking::prune_collapsed(&mut app.ui);
    assert!(app.ui.docking_collapsed.is_empty());
}

/// A double-click on the strip of a group beside another does nothing, and leaves no error in the
/// status bar.
#[test]
fn a_double_click_on_the_strip_of_a_group_beside_another_does_nothing() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"left"})).unwrap();
    let saved = app.ui.docking.clone();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let status = h.state().ui.status.clone();
    let strip = root_groups(&h).into_iter().find(|g| g.shown == "swatches").unwrap().strip;
    let tab = h.ctx.read_response(egui::Id::new("vectorcraft-panel-docking").with(("tab", &"swatches".to_string()))).unwrap().rect;
    let gap = pos2(tab.right() + 4.0, strip.center().y);
    assert!(gap.x < strip.right() - 24.0, "{tab:?} {strip:?}");
    double_click(&mut h, gap);
    assert_eq!(h.state().ui.status, status, "no error");
    assert!(h.state().ui.docking_collapsed.is_empty());
    assert_eq!(h.state().ui.docking, saved);
}
