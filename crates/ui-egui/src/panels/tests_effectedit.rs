//! Headless frames of the Appearance panel's editing depth: effect names open their dialog
//! prefilled, rows drag (Alt copies), several fills/strokes select together, stroke notes, the
//! chip's Shift-click, the thumbnail drag and the Properties fx button.

use serde_json::{Value, json};

use super::tests_appearance::{app_with_rect, click, frame_events, frame_raw, run, text_rect};
use super::*;

fn pointer(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }
}

/// One frame of the Appearance panel with `events` and `modifiers` held.
fn panel_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<(String, egui::Rect)> {
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    frame_raw(ctx, app, egui::RawInput { events, ..Default::default() }, appearance::show)
}

/// Drag from `from` to `to` in the Appearance panel, `modifiers` held on release.
fn drag(ctx: &egui::Context, app: &mut VectorcraftApp, from: egui::Pos2, to: egui::Pos2, modifiers: egui::Modifiers) {
    let none = egui::Modifiers::NONE;
    panel_frame(ctx, app, vec![egui::Event::PointerMoved(from)], none);
    panel_frame(ctx, app, vec![pointer(from, true, none)], none);
    panel_frame(ctx, app, vec![egui::Event::PointerMoved(from + egui::vec2(0.0, 8.0))], none);
    panel_frame(ctx, app, vec![egui::Event::PointerMoved(to)], modifiers);
    panel_frame(ctx, app, vec![pointer(to, false, modifiers)], modifiers);
    panel_frame(ctx, app, vec![], none);
}

/// Click `pos` with `modifiers` held.
fn click_with(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2, modifiers: egui::Modifiers) {
    for e in [egui::Event::PointerMoved(pos), pointer(pos, true, modifiers), pointer(pos, false, modifiers)] {
        panel_frame(ctx, app, vec![e], modifiers);
    }
}

/// A point in the row of the label `s` (right of its chip, weight and notes).
fn row_of(texts: &[(String, egui::Rect)], s: &str) -> egui::Pos2 {
    let r = text_rect(texts, s);
    egui::pos2(r.left() + 200.0, r.center().y)
}

/// The rect of the panel-menu entry `s` (entries are drawn after a check-mark column).
fn menu_item(texts: &[(String, egui::Rect)], s: &str) -> egui::Rect {
    texts.iter().find(|(t, _)| t.trim_start() == s).unwrap_or_else(|| panic!("`{s}` not in the menu: {texts:?}")).1
}

fn effects(app: &VectorcraftApp, item: Option<usize>) -> Vec<vectorcraft_doc::Effect> {
    first_selected(app).unwrap().appearance.effects_at(item).cloned().unwrap_or_default()
}

#[test]
fn an_effect_name_opens_its_dialog_prefilled_and_ok_edits_it_in_place() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "params": {"size": 9}}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    click(&ctx, &mut app, text_rect(&texts, "Roughen").center(), appearance::show);
    let d = app.ui.dialog.clone().expect("the effect's dialog");
    assert_eq!((d.kind.as_str(), d.f64("size", 0.0), d.fields["__index"].clone()), ("effect", 9.0, json!(0)));
    // The dialog previews; OK keeps the new value on the same effect as one undo step.
    let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!(20));
    crate::dialogs::confirm(&mut app).unwrap();
    let fx = effects(&app, None);
    assert_eq!((fx.len(), fx[0].params["size"].clone()), (1, json!(20)));
    assert!(app.ui.dialog.is_none() && app.last_effect.is_none(), "an edit is not a new Last Effect");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&app, None)[0].params["size"], json!(9));
}

/// The inline editor shows Glass's loaded texture by its size, not as its stored text.
#[test]
fn the_inline_editor_shows_a_loaded_texture_by_its_size() {
    let mut app = app_with_rect();
    let mut png = Vec::new();
    image::GrayImage::from_fn(5, 3, |x, _| image::Luma([(x * 60) as u8]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let file = json!({"name": "t.png", "dataBase64": vectorcraft_format::base64_encode(&png)});
    run(&mut app, "effect.apply", json!({"effect": "distort.glass", "params": {"textureImage": file}}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    let stored = effects(&app, None)[0].params["textureImage"].as_str().unwrap().to_string();
    assert!(texts.iter().any(|(t, _)| t == "5 × 3 px"), "{texts:?}");
    assert!(!texts.iter().any(|(t, _)| t.contains(&stored[..24])), "the stored text isn't shown");
}

#[test]
fn free_distorts_inline_editor_opens_its_dialog_for_the_corners() {
    let mut app = app_with_rect();
    let corners = json!([[0.2, 0.0], [1.0, 0.1], [1.0, 1.0], [0.0, 1.0]]);
    run(&mut app, "effect.apply", json!({"effect": "distort.freeDistort", "params": {"corners": corners}}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    click(&ctx, &mut app, text_rect(&texts, "Edit…").center(), appearance::show);
    let d = app.ui.dialog.clone().expect("Free Distort's dialog");
    assert_eq!((d.kind.as_str(), d.str("__effect"), d.fields["__index"].clone()), ("effect", "distort.freeDistort".into(), json!(0)));
    assert_eq!(d.fields["corners"], corners);
}

#[test]
fn choosing_an_applied_effect_asks_to_edit_it_or_add_another() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 0, "params": {"size": 9}}));
    run(&mut app, "appearance.setActiveItem", json!({"index": 0}));
    // The active fill already has it: the question comes first; OK edits the applied one.
    assert_eq!(app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap(), json!({"pending": "effectExists"}));
    crate::dialogs::confirm(&mut app).unwrap();
    let d = app.ui.dialog.clone().unwrap();
    assert_eq!((d.kind.as_str(), d.f64("size", 0.0), d.fields["__index"].clone(), d.fields["__item"].clone()), ("effect", 9.0, json!(0), json!(0)));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(effects(&app, Some(0)).len(), 1);
    // "Add New": a fresh dialog with the defaults, which adds a second one.
    app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    let d = app.ui.dialog.clone().unwrap();
    assert!(d.fields.get("__index").is_none() && d.f64("size", 0.0) == 5.0, "{:?}", d.fields);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(effects(&app, Some(0)).len(), 2);
    // Another list without it opens the dialog directly.
    assert_eq!(app.run("effect.dialog", json!({"effect": "distort.roughen", "item": null})).unwrap(), Value::Null);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "effect");
}

#[test]
fn effect_rows_drag_to_reorder_and_into_an_item() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen"}));
    let ctx = egui::Context::default();
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    // Twist below Roughen.
    let below = text_rect(&texts, "Roughen").center() + egui::vec2(60.0, 9.0);
    drag(&ctx, &mut app, row_of(&texts, "Twist"), below, egui::Modifiers::NONE);
    let ids = |fx: Vec<vectorcraft_doc::Effect>| fx.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids(effects(&app, None)), ["distort.roughen", "distort.twist"]);
    // Alt-dragging Twist onto the stroke row copies it into the stroke's effects.
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    drag(&ctx, &mut app, row_of(&texts, "Twist"), row_of(&texts, "Stroke:"), egui::Modifiers::ALT);
    assert_eq!(ids(effects(&app, None)), ["distort.roughen", "distort.twist"]);
    assert_eq!(ids(effects(&app, Some(1))), ["distort.twist"]);
    // Dropped effects become the selected row (their item active).
    assert_eq!(app.session.appearance_item(), Some(1));
    app.run("edit.undo", json!({})).unwrap();
    assert!(effects(&app, Some(1)).is_empty());
}

#[test]
fn alt_dragging_an_item_duplicates_it_and_several_items_delete_together() {
    let mut app = app_with_rect(); // [Fill, Stroke]
    let ctx = egui::Context::default();
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    drag(&ctx, &mut app, row_of(&texts, "Fill:"), row_of(&texts, "Stroke:"), egui::Modifiers::ALT);
    let kinds = |app: &VectorcraftApp| first_selected(app).unwrap().appearance.items.iter().map(|i| i.kind_name()).collect::<Vec<_>>();
    assert_eq!(kinds(&app), ["fill", "stroke", "fill"]);
    // Click the stroke, Cmd-click the top fill: both go with Remove Item.
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    click_with(&ctx, &mut app, row_of(&texts, "Stroke:"), egui::Modifiers::NONE);
    let top_fill = texts.iter().filter(|(t, _)| t == "Fill:").map(|(_, r)| *r).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    click_with(&ctx, &mut app, egui::pos2(top_fill.left() + 200.0, top_fill.center().y), egui::Modifiers::COMMAND);
    let menu = frame_events(&ctx, &mut app, vec![], appearance::menu);
    click(&ctx, &mut app, menu_item(&menu, "Remove Item").center(), appearance::menu);
    assert_eq!(kinds(&app), ["fill"]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(kinds(&app).len(), 3, "one undo step");
}

#[test]
fn stroke_rows_note_dashes_brushes_and_show_all_hidden_attributes() {
    let mut app = app_with_rect();
    let ctx = egui::Context::default();
    assert!(!frame_events(&ctx, &mut app, vec![], appearance::show).iter().any(|(t, _)| t == "Dashed"));
    run(&mut app, "stroke.set", json!({"dash": [4, 2], "profile": "lens"}));
    let brush = run(&mut app, "brush.list", json!({}))["brushes"][0]["name"].as_str().unwrap().to_string();
    run(&mut app, "brush.apply", json!({ "name": brush }));
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    assert!(texts.iter().any(|(t, _)| *t == format!("{brush}, Dashed")), "{texts:?}");
    // Show All Hidden Attributes is enabled once something is hidden, and shows it.
    let enabled = |app: &mut VectorcraftApp| {
        let menu = frame_events(&ctx, app, vec![], appearance::menu);
        let r = menu_item(&menu, "Show All Hidden Attributes");
        click(&ctx, app, r.center(), appearance::menu);
    };
    run(&mut app, "appearance.setItem", json!({"index": 0, "visible": false}));
    enabled(&mut app);
    assert!(first_selected(&app).unwrap().appearance.items[0].visible());
}

#[test]
fn shift_clicking_a_chip_opens_the_mixer_on_that_row() {
    let mut app = app_with_rect();
    let ctx = egui::Context::default();
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    // The chip sits 52 pt right of the label's start (label at EYE_W + 24, chip at EYE_W + 76).
    let label = text_rect(&texts, "Fill:");
    click_with(&ctx, &mut app, egui::pos2(label.left() + 52.0 + 9.0, label.center().y), egui::Modifiers::SHIFT);
    assert_eq!(app.ui.open_panel.as_deref(), Some("color"));
    assert_eq!(app.session.appearance_item(), Some(0));
}

#[test]
fn the_thumbnail_drags_the_appearance_and_the_properties_fx_button_opens_the_effects() {
    let mut app = app_with_rect();
    let ctx = egui::Context::default();
    let texts = panel_frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    // The thumbnail is left of the object row's label.
    let label = text_rect(&texts, "Rectangle");
    let thumb = egui::pos2(label.left() - 28.0, label.center().y);
    let none = egui::Modifiers::NONE;
    for e in [egui::Event::PointerMoved(thumb), pointer(thumb, true, none), egui::Event::PointerMoved(thumb + egui::vec2(30.0, 30.0))] {
        panel_frame(&ctx, &mut app, vec![e], none);
    }
    let id = first_selected(&app).unwrap().id;
    assert_eq!(egui::DragAndDrop::payload::<crate::widgets::PanelDrag>(&ctx).as_deref(), Some(&crate::widgets::PanelDrag::Appearance(id)));
    // Properties: the fx button lists the effect groups.
    let props = egui::Context::default();
    let texts = frame_raw(&props, &mut app, Default::default(), properties::show);
    click(&props, &mut app, text_rect(&texts, "fx").center(), properties::show);
    let texts = frame_raw(&props, &mut app, Default::default(), properties::show);
    assert!(texts.iter().any(|(t, _)| t == "Distort & Transform"), "{texts:?}");
}
