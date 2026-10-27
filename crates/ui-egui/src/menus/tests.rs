use super::*;
use crate::theme;

/// One headless frame of the in-window menu bar; returns its titles (left to right) as
/// (rect, id of the title's popup).
fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(egui::Rect, egui::Id)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let mut layer = None;
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
        layer = Some(ui.layer_id());
        menu_bar(app, ui);
    });
    out.textures_delta.clear();
    let layer = layer.unwrap();
    let mut titles: Vec<(egui::Rect, egui::Id)> = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .get_layer(layer)
            .filter(|w| w.sense.senses_click() && w.rect.top() < 30.0)
            // `egui::Popup::default_response_id` of the title's response.
            .map(|w| (w.rect, w.id.with("popup")))
            .collect()
    });
    titles.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
    titles
}

fn open_titles(ctx: &egui::Context, titles: &[(egui::Rect, egui::Id)]) -> Vec<usize> {
    (0..titles.len()).filter(|&i| egui::Popup::is_id_open(ctx, titles[i].1)).collect()
}

/// File ends with Exit, running the app's own quit flow (unsaved documents are asked about).
#[test]
fn file_ends_with_exit() {
    let tree = menu_tree();
    let (_, file) = tree.iter().find(|(t, _)| *t == "File").unwrap();
    let last = file.iter().rev().find(|i| !matches!(i, Item::Sep)).unwrap();
    assert!(matches!(last, Item::Cmd("Exit", "app.quit", _)), "{last:?}");
}

/// No application menu in the window (as in Illustrator): the bar starts at File.
#[test]
fn the_bar_starts_at_file() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
        menu_bar(&mut app, ui);
    });
    out.textures_delta.clear();
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    let titles = ["File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"];
    for t in titles {
        assert!(shown.iter().any(|s| s == t), "{t} missing from {shown:?}");
    }
    assert!(!shown.iter().any(|s| s == "VectorCraft"), "{shown:?}");
}

#[test]
fn hovering_another_title_switches_the_open_menu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    bar_frame(&mut app, &ctx, vec![]);
    let titles = bar_frame(&mut app, &ctx, vec![]);
    // No application menu in the window: the bar starts at File on every platform.
    assert_eq!(titles.len(), menu_tree().len() - 1);
    let (file, edit, object) = (titles[0].0.center(), titles[1].0.center(), titles[2].0.center());
    let frames = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut t = bar_frame(app, &ctx, events);
        for _ in 0..2 {
            t = bar_frame(app, &ctx, vec![]);
        }
        t
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

    // Nothing open: hovering a title opens nothing.
    let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
    assert!(open_titles(&ctx, &t).is_empty());

    // Click File, then move onto Edit and on to Object: each opens in turn, alone.
    frames(&mut app, vec![egui::Event::PointerMoved(file)]);
    frames(&mut app, vec![button(file, true)]);
    let t = frames(&mut app, vec![button(file, false)]);
    assert_eq!(open_titles(&ctx, &t), vec![0], "a click opens File");
    let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
    assert_eq!(open_titles(&ctx, &t), vec![1], "hovering Edit opens it and closes File");
    let t = frames(&mut app, vec![egui::Event::PointerMoved(object)]);
    assert_eq!(open_titles(&ctx, &t), vec![2], "hovering Object opens it and closes Edit");

    // A pointer resting on a title doesn't switch: File opened another way (the keyboard)
    // stays open while the pointer stays still over Object.
    egui::Popup::open_id(&ctx, t[0].1);
    let t = frames(&mut app, vec![]);
    assert_eq!(open_titles(&ctx, &t), vec![0], "a still pointer leaves the open menu alone");
}

#[test]
fn a_popup_covering_a_title_does_not_switch_menus() {
    let ctx = egui::Context::default();
    let p = egui::pos2(90.0, 12.0);
    let mut reaches = (true, false);
    // The popup becomes hit-testable once egui has laid it out: draw a few frames.
    for _ in 0..3 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            let covered = ui.interact(egui::Rect::from_center_size(p, egui::vec2(80.0, 24.0)), egui::Id::unique("covered"), egui::Sense::click());
            let q = p + egui::vec2(0.0, 100.0);
            let free = ui.interact(egui::Rect::from_center_size(q, egui::vec2(80.0, 24.0)), egui::Id::unique("free"), egui::Sense::click());
            egui::Area::new(egui::Id::unique("popup")).order(egui::Order::Foreground).fixed_pos(p - egui::vec2(10.0, 10.0)).show(&ctx, |ui| {
                ui.allocate_space(egui::vec2(20.0, 20.0));
            });
            reaches = (pointer_reaches_title(&ctx, &covered, p), pointer_reaches_title(&ctx, &free, q));
        });
        out.textures_delta.clear();
    }
    assert_eq!(reaches, (false, true));
}

#[test]
fn names_in_menus_are_not_interface_labels() {
    for (id, label) in [
        ("file.openRecent1", "Layers.svg"),
        ("view.goto2", "Layers"),
        ("type.recentFont1", "Regular"),
        ("text.setStyle", "Black"),
        ("window.workspace", "Layers"),
        ("plugin.dialog", "Group"),
        ("window.userSwatchLibrary3", "Default"),
    ] {
        assert!(shows_a_name(id, label), "{id} {label}");
    }
    for (id, label) in [("object.group", "Group"), ("window.workspace", "Essentials"), ("effect.apply", "Drop Shadow")] {
        assert!(!shows_a_name(id, label), "{id} {label}");
    }
}

#[test]
fn last_effect_dialog_and_view_toggles() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert!(!enabled(&app, "effect.last"));
    app.last_effect = Some(("distort.roughen".into(), json!({"size": 9})));
    assert!(enabled(&app, "effect.last"));
    assert_eq!(dynamic_label(&app, "effect.last", "Last Effect…"), "Last Effect: Roughen…");
    app.run("effect.last", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("__effect"), d.f64("size", 0.0)), ("effect", "distort.roughen".to_string(), 9.0));
    for (id, on, off) in [
        ("view.textThreads", "Hide Text Threads", "Show Text Threads"),
        ("view.gradientAnnotator", "Hide Gradient Annotator", "Show Gradient Annotator"),
        ("view.livePaintGaps", "Show Live Paint Gaps", "Hide Live Paint Gaps"),
        ("type.hiddenCharacters", "Show Hidden Characters", "Hide Hidden Characters"),
        ("view.template", "Hide Template", "Show Template"),
    ] {
        assert_eq!(dynamic_label(&app, id, ""), on);
        app.run(id, json!({})).unwrap();
        assert_eq!(dynamic_label(&app, id, ""), off);
    }
}

/// Edit › Undo and Edit › Redo name the step in the UI language as the History panel lists it
/// (`panels::history::step_name`), including a step named after an effect whose catalog holds
/// only the effect's menu row with an ellipsis, as Japanese does for Feather.
#[test]
fn undo_and_redo_name_the_step_as_the_history_panel_does() {
    let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
    crate::i18n::set_current(ja);
    let mut app = app_with_a_rectangle();
    app.run("effect.apply", json!({"effect": "stylize.feather"})).unwrap();
    assert_eq!(app.session.active().unwrap().history.undo.last().map(|h| h.label.as_str()), Some("Feather"));
    let shown = |app: &VectorcraftApp, id: &'static str| {
        let item = c(if id == "edit.undo" { "Undo" } else { "Redo" }, id);
        match entry(app, &item) {
            Some(Entry::Item(row)) => row.label.to_string(),
            _ => panic!("{id} is not a menu item"),
        }
    };
    let step = |template: &str, name: &str| {
        let name = crate::panels::history::step_name(ja, name).unwrap_or_else(|| name.to_string());
        crate::i18n::fmt(crate::i18n::tr(ja, template), &[("name", &name)])
    };
    assert_eq!(shown(&app, "edit.undo"), step("Undo {name}", "Feather"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(shown(&app, "edit.redo"), step("Redo {name}", "Feather"));
    assert_eq!(shown(&app, "edit.undo"), step("Undo {name}", "Rectangle"), "a step with a plain row");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(shown(&app, "edit.redo"), "Redo Feather");
}

/// Type › Optical Margin Alignment runs `type.opticalMarginAlignment`, needs a selection, and
/// is checked while the selected type has it on. Its check, and Type Orientation's, read the
/// type inside a selected group too.
#[test]
fn optical_margin_alignment_is_a_checked_type_menu_item() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let ids: Vec<&str> = menu_tree()
        .into_iter()
        .filter(|(menu, _)| *menu == "Type")
        .flat_map(|(_, items)| items)
        .filter_map(|item| match item {
            Item::Cmd("Optical Margin Alignment", id, _) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["type.opticalMarginAlignment"]);
    let id = "type.opticalMarginAlignment";
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    assert!(!enabled(&app, id));
    let text = app.run("text.create", json!({"x": 10, "y": 10, "text": "“Hi”"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    assert!(enabled(&app, id));
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    // With the type in a selected group, the checks show the type the commands change.
    app.run("object.group", json!({})).unwrap();
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    app.run("type.orientation.vertical", json!({})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(true));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
}

/// Type › Optical Margin Alignment menu check uses `shared_text_flag`; mixed values stay unchecked.
#[test]
fn optical_margin_alignment_menu_unchecked_when_shared_flag_none() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let off = app.run("text.create", json!({"x": 10, "y": 40, "text": "Off"})).unwrap()["id"].clone();
    let on = app.run("text.create", json!({"x": 10, "y": 80, "text": "On"})).unwrap()["id"].clone();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": on})).unwrap();
    app.run("select.set", json!({"ids": [off, on]})).unwrap();
    let id = "type.opticalMarginAlignment";
    assert_eq!(crate::panels::character::shared_text_flag(&app, |t| t.optical_margin), None);
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run("select.set", json!({"ids": [on]})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
}

#[test]
fn panel_ids_are_case_insensitive_and_labels_work() {
    assert_eq!(normalize_panel("layers"), Some("layers"));
    assert_eq!(normalize_panel("Layers"), Some("layers"));
    assert_eq!(normalize_panel("LAYERS"), Some("layers"));
    assert_eq!(normalize_panel("Swatches"), Some("swatches"));
    assert_eq!(normalize_panel("swatches"), Some("swatches"));
    assert_eq!(normalize_panel("Color Guide"), Some("colorGuide"));
    assert_eq!(normalize_panel("colorguide"), Some("colorGuide"));
    assert_eq!(normalize_panel("  Stroke  "), Some("stroke"));
    assert_eq!(normalize_panel("nope"), None);

    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("window.panel", json!({"panel": "Layers"})).unwrap();
    assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers);
    app.run("window.panel", json!({"panel": "Swatches"})).unwrap();
    assert_eq!(app.ui.open_panel.as_deref(), Some("swatches"));
    assert!(checked(&app, "window.panel", &json!({"panel": "Swatches"})).unwrap());
    assert!(app.run("window.panel", json!({"panel": "nope"})).is_err());
    // A collapsed dock pops the panel out of its icon, by canonical id.
    app.ui.dock_collapsed = true;
    app.run("window.panel", json!({"panel": "Layers"})).unwrap();
    assert_eq!(app.ui.open_panel.as_deref(), Some("layers"));
    assert!(checked(&app, "window.panel", &json!({"panel": "Layers"})).unwrap());
    app.ui.dock_collapsed = false;
}

#[test]
fn saved_views_store_and_restore_the_view() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    {
        let v = app.view_mut().unwrap();
        v.zoom = 3.0;
        v.center = vectorcraft_geom::Point::new(120.0, 80.0);
    }
    app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
    assert!(enabled(&app, "view.goto1") && !enabled(&app, "view.goto2"));
    assert_eq!(dynamic_label(&app, "view.goto1", ""), "Detail");
    app.view_mut().unwrap().zoom = 0.5;
    app.run("view.goto1", json!({})).unwrap();
    let v = app.view().unwrap();
    assert_eq!((v.zoom, v.center.x, v.center.y), (3.0, 120.0, 80.0));
    // Unused slots stay out of the menu listing agents see.
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("view.goto2")));
}

/// A document's 25 views are all listed at the bottom of the View menu, by name, and the last
/// one brings back its zoom, centre and rotation after the view changed.
#[test]
fn every_saved_view_is_listed_at_the_bottom_of_the_view_menu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    for i in 1..=25 {
        {
            let v = app.view_mut().unwrap();
            (v.zoom, v.center, v.rotation) = (i as f64 / 4.0, vectorcraft_geom::Point::new(10.0 * i as f64, -5.0 * i as f64), i as f64);
        }
        invoke(&mut app, "view.saved.new", json!({}));
        crate::dialogs::confirm(&mut app).unwrap();
    }
    assert_eq!(app.session.active().unwrap().doc.views.len(), 25);
    let entries = menu_entries(&app);
    let at = |id: &str| entries.iter().position(|e| e.command.as_deref() == Some(id));
    let (edit, first, last) = (at("view.saved.edit").unwrap(), at("view.goto1").unwrap(), at("view.goto25").unwrap());
    assert!(edit < first && last == first + 24, "listed in order after Edit Views…");
    assert!(enabled(&app, "view.goto25"));
    assert_eq!(display_label(&app, "view.goto25", "Saved View"), "View 25");
    // The document's 26th view is refused, so there is never one the menu leaves out.
    invoke(&mut app, "view.saved.new", json!({}));
    assert!(crate::dialogs::confirm(&mut app).is_err());
    app.ui.dialog = None;
    {
        let v = app.view_mut().unwrap();
        (v.zoom, v.center, v.rotation) = (1.0, vectorcraft_geom::Point::new(0.0, 0.0), 0.0);
    }
    assert_eq!(app.run("view.goto25", json!({})).unwrap()["name"], "View 25");
    let v = app.view().unwrap();
    assert_eq!((v.zoom, v.center.x, v.center.y, v.rotation), (6.25, 250.0, -125.0, 25.0));
}

/// Edit Views… is off without a saved view and opens its list on the document's views, the
/// way Edit Selection… does.
#[test]
fn edit_views_opens_its_dialog_on_the_saved_views() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    assert!(!enabled(&app, "view.saved.edit"));
    invoke(&mut app, "view.saved.edit", json!({}));
    assert!(app.ui.dialog.is_none());
    assert!(app.ui.status.contains("No saved views"), "{}", app.ui.status);
    app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
    app.run("view.saved.new", json!({"name": "Page"})).unwrap();
    assert!(enabled(&app, "view.saved.edit"));
    invoke(&mut app, "view.saved.edit", json!({}));
    let d = app.ui.dialog.clone().expect("Edit Views dialog");
    assert_eq!((d.kind.as_str(), d.str("orig1"), d.str("name1")), ("editViews", "Page".into(), "Page".into()));
    // Deleting a view takes its slot out of the menu.
    let mut d = d;
    d.fields.insert("deleted0".into(), json!(true));
    app.ui.dialog = Some(d);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(dynamic_label(&app, "view.goto1", ""), "Page");
    assert!(shown_state(&app, "view.goto2", &Value::Null).is_none());
}

fn app_with_a_rectangle() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app
}

/// Edit › Undo and Edit › Redo name a step named after a style as the History panel lists it:
/// the template's translation with the style's name in it.
#[test]
fn undo_and_redo_name_a_style_step_around_the_style_name() {
    let mut app = app_with_a_rectangle();
    let text = app.run("text.create", json!({"x": 0, "y": 40, "text": "Hello"})).unwrap()["id"].clone();
    app.run("charStyle.new", json!({"name": "Brush"})).unwrap();
    app.run("charStyle.apply", json!({"name": "Brush", "id": text})).unwrap();
    let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
    crate::i18n::set_current(ja);
    let step = |template: &str| {
        let applied = crate::i18n::fmt(crate::i18n::tr(ja, "Apply {name}"), &[("name", "Brush")]);
        crate::i18n::fmt(crate::i18n::tr(ja, template), &[("name", &applied)])
    };
    assert_eq!(display_label(&app, "edit.undo", "Undo"), step("Undo {name}"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(display_label(&app, "edit.redo", "Redo"), step("Redo {name}"));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(display_label(&app, "edit.redo", "Redo"), "Redo Apply Brush");
}

#[test]
fn save_selection_asks_for_a_name() {
    let mut app = app_with_a_rectangle();
    invoke(&mut app, "select.save", json!({}));
    let d = app.ui.dialog.clone().expect("Save Selection dialog");
    assert_eq!(
        (d.kind.as_str(), d.str("__command"), d.str("__label"), d.str("name")),
        ("command", "select.save".into(), "Save Selection".into(), "Selection 1".into())
    );
    // OK saves under the name typed.
    let mut d = d;
    d.fields.insert("name".into(), json!("Logo"));
    app.ui.dialog = Some(d);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.run("select.savedList", json!({})).unwrap(), json!(["Logo"]));
    // The next one starts from the first free name.
    invoke(&mut app, "select.save", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")), Some("Selection 1".into()));
}

#[test]
fn saved_selections_are_listed_at_the_bottom_of_the_select_menu() {
    let mut app = app_with_a_rectangle();
    app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    // Nothing saved: no slot is listed or enabled.
    assert!(!enabled(&app, "select.recall1"));
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall1")));
    app.run("select.save", json!({"name": "Ellipse"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("select.save", json!({"name": "Everything"})).unwrap();
    assert!(enabled(&app, "select.recall1") && enabled(&app, "select.recall2") && !enabled(&app, "select.recall3"));
    assert_eq!(dynamic_label(&app, "select.recall2", "Saved Selection"), "Everything");
    // A name is shown as it is, not translated.
    assert_eq!(display_label(&app, "select.recall2", "Saved Selection"), "Everything");
    assert!(menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall2")));
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall3")));
    // Choosing one selects its objects again.
    app.run("select.none", json!({})).unwrap();
    let r = app.run("select.recall2", json!({})).unwrap();
    assert!(r["count"].as_u64().unwrap() >= 1);
    assert_eq!(app.session.active().unwrap().selection.len(), 2);
    // A deleted selection's slot leaves the menus.
    app.run("select.editSaved", json!({"name": "Ellipse", "delete": true})).unwrap();
    assert!(!enabled(&app, "select.recall2"));
    assert!(shown_state(&app, "select.recall2", &Value::Null).is_none());
    assert!(shown_state(&app, "select.recall1", &Value::Null).is_some());
}

#[test]
fn update_selection_follows_save_selection_and_needs_a_recall() {
    let mut app = app_with_a_rectangle();
    let entries = menu_entries(&app);
    let at = |id: &str| entries.iter().position(|e| e.command.as_deref() == Some(id));
    assert_eq!(at("select.update").map(|i| i - 1), at("select.save"));
    app.run("select.save", json!({"name": "Box"})).unwrap();
    assert!(!enabled(&app, "select.update"));
    app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("select.recall1", json!({})).unwrap();
    assert!(enabled(&app, "select.update"));
    app.run("select.all", json!({})).unwrap();
    invoke(&mut app, "select.update", json!({}));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(app.run("select.recall1", json!({})).unwrap()["count"], 2);
}

#[test]
fn edit_selection_opens_its_dialog_on_the_saved_selections() {
    let mut app = app_with_a_rectangle();
    // Nothing saved yet: the item is off, and invoking it (palette, shortcut) only says so.
    assert!(!enabled(&app, "select.editSaved"));
    invoke(&mut app, "select.editSaved", json!({}));
    assert!(app.ui.dialog.is_none());
    assert!(app.ui.status.contains("No saved selections"), "{}", app.ui.status);
    app.run("select.save", json!({"name": "Logo"})).unwrap();
    assert!(enabled(&app, "select.editSaved"));
    invoke(&mut app, "select.editSaved", json!({}));
    let d = app.ui.dialog.clone().expect("Edit Selection dialog");
    assert_eq!((d.kind.as_str(), d.str("orig0"), d.str("name0")), (crate::dialogs::edit_selection::KIND, "Logo".into(), "Logo".into()));
}

#[test]
fn recent_fonts_and_corner_widget_toggle() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let fams = vectorcraft_text::FontDb::global().families();
    let (a, b) = (fams[0].clone(), fams[fams.len() - 1].clone());
    app.run("text.create", json!({"x": 10, "y": 10, "text": "Hi"})).unwrap();
    app.run("text.setStyle", json!({"font": a})).unwrap();
    app.run("text.setStyle", json!({"font": b})).unwrap();
    assert_eq!(app.ui.recent_fonts, vec![b.clone(), a.clone()]);
    assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), a);
    assert!(enabled(&app, "type.recentFont2") && !enabled(&app, "type.recentFont3"));
    app.run("type.recentFont2", json!({})).unwrap();
    assert_eq!(app.ui.recent_fonts[0], a);
    assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Hide Corner Widget");
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Show Corner Widget");
}

/// View › Precise Cursors has no menu item or default key, but the command (assignable in Keyboard Shortcuts)
/// writes General › Use Precise Cursors, the value the dialog and the saved preferences read.
#[test]
fn precise_cursors_command_toggles_the_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(!app.session.prefs.use_precise_cursors);
    assert_eq!(app.run("view.preciseCursors", json!({})).unwrap(), json!(true));
    assert!(app.session.prefs.use_precise_cursors);
    assert_eq!(app.ui.engine_prefs["usePreciseCursors"], json!(true), "saved with the UI state");
    assert!(UI_COMMANDS.iter().any(|c| c.0 == "view.preciseCursors" && c.2.is_empty()), "assignable, no default key");
    assert_eq!(app.run("view.preciseCursors", json!({})).unwrap(), json!(false));
    assert!(!app.session.prefs.use_precise_cursors);
}

/// The command is listed in Edit › Keyboard Shortcuts without a key and takes one the user types; with the
/// key it is the command that runs.
#[test]
fn precise_cursors_command_is_listed_and_assignable_in_the_shortcut_editor() {
    use crate::shortcut_editor as ed;
    let e = ed::entry("view.preciseCursors").expect("listed in the editor");
    assert_eq!((e.label.as_str(), e.group.as_str(), e.is_tool), ("Precise Cursors", "View", false));
    assert_eq!(ed::default_of("view.preciseCursors"), None, "no default key");
    let mut overrides = craft_keymap::Overrides::new();
    assert_eq!(ed::effective_in(&overrides, "view.preciseCursors"), None);
    assert_eq!(ed::assign(&mut overrides, "view.preciseCursors", Some("Cmd+Alt+Shift+U"), false), Ok(vec![]));
    let chord = ed::effective_in(&overrides, "view.preciseCursors").expect("assigned");
    assert_eq!(ed::normalize(&chord), ed::normalize("Cmd+Alt+Shift+U"));
    // A key another command holds is refused, so the new command can't take it over silently.
    assert!(ed::assign(&mut overrides, "view.preciseCursors", Some("Cmd+Alt+Shift+P"), false).is_err());
    assert_eq!(ed::assign(&mut overrides, "view.preciseCursors", None, false), Ok(vec![]));
    assert_eq!(ed::effective_in(&overrides, "view.preciseCursors"), None, "cleared again");
}

/// The command is a preference, not an edit: no undo step, no unsaved changes, with a document open.
#[test]
fn precise_cursors_command_is_not_an_edit() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let steps = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
    };
    let before = steps(&app);
    assert!(!before.2);
    app.run("view.preciseCursors", json!({})).unwrap();
    assert!(app.session.prefs.use_precise_cursors);
    assert_eq!(steps(&app), before);
}

/// Selection & Anchor Display › Zoom to Selection (#394): Zoom In and Zoom Out bring the
/// selection to the middle; off, they keep the view's centre.
#[test]
fn zoom_to_selection_centres_the_selection() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 20})).unwrap();
    let center = |app: &VectorcraftApp| app.view().unwrap().center;
    let start = center(&app);
    app.run("prefs.set", json!({"key": "zoomToSelection", "value": false})).unwrap();
    app.run("view.zoomIn", json!({})).unwrap();
    assert_eq!(center(&app), start, "off: the view's centre stays");
    app.run("prefs.set", json!({"key": "zoomToSelection", "value": true})).unwrap();
    app.run("view.zoomOut", json!({})).unwrap();
    assert_eq!(center(&app), vectorcraft_geom::Point::new(30.0, 30.0), "on: the selection's centre");
}

/// Preferences › Type › Number of Recent Fonts (#394): Type › Recent Fonts lists that many.
#[test]
fn number_of_recent_fonts_sets_how_many_are_listed() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.ui.recent_fonts = (1..=15).map(|i| format!("Font {i}")).collect();
    let listed = |app: &VectorcraftApp| RECENT_FONT_IDS.iter().filter(|id| enabled(app, id)).count();
    assert_eq!(listed(&app), 10, "10 by default");
    for n in [15, 2] {
        app.run("prefs.set", json!({"key": "recentFontsCount", "value": n})).unwrap();
        assert_eq!(listed(&app), n);
    }
    assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), "Font 2");
    assert!(app.run("type.recentFont3", json!({})).is_err(), "past the count");
}

#[test]
fn recent_files_track_opens_and_saves() {
    let dir = std::env::temp_dir().join(format!("dc-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = VectorcraftApp::new(
        vectorcraft_engine::Session::new(),
        crate::Services {
            read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let (a, b) = (dir.join("a.vectorcraft"), dir.join("b.vectorcraft"));
    for p in [&a, &b, &a] {
        app.run("file.saveAs", json!({"path": p.to_string_lossy()})).unwrap();
    }
    assert_eq!(app.ui.recent_files, [a.to_string_lossy(), b.to_string_lossy()]);
    assert_eq!(dynamic_label(&app, "file.openRecent2", ""), "b.vectorcraft");
    assert!(enabled(&app, "file.openRecent2") && !enabled(&app, "file.openRecent3"));
    app.run("file.openRecent2", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.ui.recent_files[0], b.to_string_lossy(), "reopening moves it to the top");
    app.run("file.clearRecent", json!({})).unwrap();
    assert!(app.ui.recent_files.is_empty() && app.run("file.openRecent1", json!({})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_bound_menu_command_exists() {
    fn walk(items: &[Item], bad: &mut Vec<String>) {
        for it in items {
            match it {
                Item::Cmd(_, id, _)
                    if vectorcraft_engine::find_command(id).is_none()
                        && !UI_COMMANDS.iter().any(|c| c.0 == *id)
                        && !id.starts_with("object.path.") =>
                {
                    bad.push(id.to_string());
                }
                Item::Sub(_, ch) => walk(ch, bad),
                _ => {}
            }
        }
    }
    let mut bad = vec![];
    for (_, items) in menu_tree() {
        walk(&items, &mut bad);
    }
    assert!(bad.is_empty(), "menu items bound to unknown commands: {bad:?}");
}

#[test]
fn shortcut_pretty() {
    if cfg!(target_os = "macos") {
        assert_eq!(pretty_shortcut("Cmd+Shift+]"), "⇧⌘]");
        assert_eq!(pretty_shortcut("Cmd+Alt+2"), "⌥⌘2");
    }
    assert_eq!(pretty_shortcut(""), "");
}

/// #543: Object › Image Trace › Make and Make and Expand trace with the Default preset at once,
/// with no form asking for a preset's name.
#[test]
fn image_trace_make_traces_with_the_default_preset() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    for cmd in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let e = menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some(cmd)).unwrap();
        assert_eq!(e.path, ["Object", "Image Trace"]);
        assert_eq!(click_target(&e.label, cmd, &e.params), (cmd.to_string(), json!({"preset": "Default"})), "{}", e.label);
    }
}

#[test]
fn expand_appearance_is_an_object_menu_item_only() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let paths: Vec<Vec<String>> =
        menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some("effect.expandAppearance")).map(|e| e.path).collect();
    assert_eq!(paths, [vec!["Object".to_string()]]);
}

#[test]
fn menus_are_as_wide_as_their_items() {
    // Items size the popup to the widest label and shortcut instead of the widest a menu may be.
    let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let tree = menu_tree();
    let (_, file) = tree.iter().find(|(t, _)| *t == "File").unwrap();
    let id = egui::Id::unique("test-menu");
    for _ in 0..3 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::Area::new(id).show(ui.ctx(), |ui| {
                egui::containers::menu::menu_style(ui.style_mut());
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, file, &mut None));
            });
        });
        out.textures_delta.clear();
    }
    let w = ctx.memory(|m| m.area_rect(id)).unwrap().width();
    assert!((230.0..340.0).contains(&w), "File menu is {w} pt wide");
}

#[test]
fn long_menus_scroll_inside_a_short_window() {
    // The Window menu is taller than a 600 pt window: it stops above the bottom edge and
    // scrolls instead of running off screen.
    let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let tree = menu_tree();
    let (_, window) = tree.iter().find(|(t, _)| *t == "Window").unwrap();
    let id = egui::Id::unique("test-long-menu");
    let rect = |height: f32| {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, height));
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::Area::new(id).fixed_pos(egui::pos2(300.0, 30.0)).show(ui.ctx(), |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, window, &mut None));
                });
            });
            out.textures_delta.clear();
        }
        ctx.memory(|m| m.area_rect(id)).unwrap()
    };
    let short = rect(600.0);
    assert!(short.bottom() <= 600.0, "Window menu ends at {} in a 600 pt window", short.bottom());
    // With room to spare it shows whole, taller than the short window allowed.
    let tall = rect(3000.0);
    assert!(tall.height() > short.height() + 100.0, "{} vs {}", tall.height(), short.height());
    assert!(tall.bottom() < 3000.0);
}

/// #3724: Window › New Window is greyed out until VectorCraft can open a second window on a
/// document, instead of being enabled and answering with an internal milestone note.
#[test]
fn new_window_is_disabled_until_it_opens_a_window() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert!(!enabled(&app, "window.newWindow"), "New Window is enabled but opens no window");
    let e = app.run("window.newWindow", json!({})).unwrap_err();
    assert!(!e.contains("M11.5"), "a user-facing message, not a milestone: {e}");
}
