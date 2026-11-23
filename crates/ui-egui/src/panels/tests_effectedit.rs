//! Headless frames of the Appearance panel's editing depth: effect names open their dialog
//! prefilled, rows drag (Alt copies), several fills/strokes select together, stroke notes, the
//! chip's Shift-click, the thumbnail drag and the Properties fx button.

use craft::ui::dialog::PromptChoice;
use serde_json::{Value, json};

use super::tests_appearance::{
    answer_prompt, app_frame, app_with_rect, cancel_dialog, click_row, confirm_dialog, dialog_json, dialog_number, dialog_open, drag_row, row_spot,
    run, set_dialog,
};
use super::*;
use crate::tests::panel_harness::Panel;

fn effects(app: &VectorcraftApp, item: Option<usize>) -> Vec<vectorcraft_doc::Effect> {
    first_selected(app).unwrap().appearance.effects_at(item).cloned().unwrap_or_default()
}

#[test]
fn an_effect_name_opens_its_dialog_prefilled_and_ok_edits_it_in_place() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "params": {"size": 9}}));
    let mut panel = Panel::new(app, appearance::show);
    panel.click_role("Roughen", craft::ui::contract::Role::LINK);
    let mut app = panel.app();
    assert!(dialog_open(&app), "the effect's dialog");
    assert_eq!(dialog_number(&mut app, "size"), 9.0);
    // The dialog previews; OK keeps the new value on the same effect as one undo step.
    app_frame(&mut app);
    set_dialog(&mut app, "size", json!(20));
    confirm_dialog(&mut app);
    let fx = effects(&app, None);
    assert_eq!((fx.len(), fx[0].params["size"].clone()), (1, json!(20)));
    assert!(!dialog_open(&app) && app.last_effect.is_none(), "an edit is not a new Last Effect");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&app, None)[0].params["size"], json!(9));
}

/// The inline editor shows Glass's loaded texture by its size, not as its stored text.
#[test]
fn the_inline_editor_shows_a_loaded_texture_by_its_size() {
    let mut app = app_with_rect();
    let mut png = Vec::new();
    {
        let __dyn: image::DynamicImage = (image::GrayImage::from_fn(5, 3, |x, _| image::Luma([(x * 60) as u8]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let file = json!({"name": "t.png", "dataBase64": vectorcraft_format::base64_encode(&png)});
    run(&mut app, "effect.apply", json!({"effect": "distort.glass", "params": {"textureImage": file}}));
    let stored = effects(&app, None)[0].params["textureImage"].as_str().unwrap().to_string();
    // PLAN: assumed `RowKey::Effect(k)` for the inline editor of object effect `k`
    app.ui.appearance_panel.open.insert(appearance::RowKey::Effect(0));
    let mut panel = Panel::new(app, appearance::show);
    panel.frame();
    assert!(panel.shows("5 × 3 px"));
    assert!(!panel.shows(&stored[..24]), "the stored text isn't shown");
}

#[test]
fn free_distorts_inline_editor_opens_its_dialog_for_the_corners() {
    let mut app = app_with_rect();
    let corners = json!([[0.2, 0.0], [1.0, 0.1], [1.0, 1.0], [0.0, 1.0]]);
    run(&mut app, "effect.apply", json!({"effect": "distort.freeDistort", "params": {"corners": corners}}));
    app.ui.appearance_panel.open.insert(appearance::RowKey::Effect(0));
    let mut panel = Panel::new(app, appearance::show);
    panel.click("Edit…");
    let mut app = panel.app();
    assert!(dialog_open(&app), "Free Distort's dialog");
    assert_eq!(dialog_json(&mut app, "corners"), Some(corners));
}

#[test]
fn choosing_an_applied_effect_asks_to_edit_it_or_add_another() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 0, "params": {"size": 9}}));
    run(&mut app, "appearance.setActiveItem", json!({"index": 0}));
    // The active fill already has it: the question comes first; OK edits the applied one.
    assert_eq!(app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap(), json!({"pending": "effectExists"}));
    answer_prompt(&mut app, PromptChoice::Proceed);
    assert_eq!(dialog_number(&mut app, "size"), 9.0);
    confirm_dialog(&mut app);
    assert_eq!(effects(&app, Some(0)).len(), 1);
    assert!(effects(&app, None).is_empty());
    // "Add New": a fresh dialog with the defaults, which adds a second one.
    app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap();
    answer_prompt(&mut app, PromptChoice::Alternate);
    assert_eq!(dialog_number(&mut app, "size"), 5.0);
    confirm_dialog(&mut app);
    assert_eq!(effects(&app, Some(0)).len(), 2);
    // Another list without it opens the dialog directly.
    assert_eq!(app.run("effect.dialog", json!({"effect": "distort.roughen", "item": null})).unwrap(), Value::Null);
    assert!(dialog_open(&app));
    cancel_dialog(&mut app);
}

/// "row reorder": a row dropped on a lower row moves below it; Alt at the release copies it into the
/// stroke's effects, and the copy becomes the selected row.
#[test]
fn effect_rows_drag_to_reorder_and_into_an_item() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen"}));
    let mut panel = Panel::new(app, appearance::show);
    let ids = |fx: Vec<vectorcraft_doc::Effect>| fx.into_iter().map(|e| e.id).collect::<Vec<_>>();
    // Twist below Roughen: dropped on the lower half of its row.
    let (twist, below) = (row_spot(&mut panel, "Twist"), row_spot(&mut panel, "Roughen") + egui::vec2(0.0, 6.0));
    panel.drag_between(twist, below, egui::Modifiers::NONE);
    assert_eq!(ids(effects(&panel.app(), None)), ["distort.roughen", "distort.twist"]);
    drag_row(&mut panel, "Twist", "Stroke:", egui::Modifiers::ALT);
    assert_eq!(ids(effects(&panel.app(), None)), ["distort.roughen", "distort.twist"]);
    assert_eq!(ids(effects(&panel.app(), Some(1))), ["distort.twist"]);
    // Dropped effects become the selected row (their item active).
    assert_eq!(panel.app().session.appearance_item(), Some(1));
    run(&mut panel.app(), "edit.undo", json!({}));
    assert!(effects(&panel.app(), Some(1)).is_empty());
}

/// "row reorder" and "Delete keys": Alt at the release duplicates an item; several highlighted items
/// are removed together in one undo step.
#[test]
fn alt_dragging_an_item_duplicates_it_and_several_items_delete_together() {
    let mut panel = Panel::new(app_with_rect(), appearance::show); // [Fill, Stroke]
    let kinds = |panel: &Panel| first_selected(&panel.app()).unwrap().appearance.items.iter().map(|i| i.kind_name()).collect::<Vec<_>>();
    drag_row(&mut panel, "Fill:", "Stroke:", egui::Modifiers::ALT);
    assert_eq!(kinds(&panel), ["fill", "stroke", "fill"]);
    // Click the stroke, Cmd-click a fill (the harness takes the topmost): both go.
    click_row(&mut panel, "Stroke:", egui::Modifiers::NONE);
    click_row(&mut panel, "Fill:", egui::Modifiers::COMMAND);
    panel.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(kinds(&panel), ["fill"]);
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_eq!(kinds(&panel).len(), 3, "one undo step");
}

#[test]
fn stroke_rows_note_dashes_brushes_and_show_all_hidden_attributes() {
    let mut panel = Panel::new(app_with_rect(), appearance::show);
    panel.frame();
    assert!(!panel.shows("Dashed"));
    run(&mut panel.app(), "stroke.set", json!({"dash": [4, 2], "profile": "lens"}));
    let brush = run(&mut panel.app(), "brush.list", json!({}))["brushes"][0]["name"].as_str().unwrap().to_string();
    run(&mut panel.app(), "brush.apply", json!({ "name": brush }));
    panel.frame();
    assert!(panel.shows(&format!("{brush}, Dashed")));
    // Show All Hidden Attributes works once something is hidden, and shows it.
    run(&mut panel.app(), "appearance.setItem", json!({"index": 0, "visible": false}));
    run(&mut panel.app(), "appearance.showAllHidden", json!({}));
    assert!(first_selected(&panel.app()).unwrap().appearance.items[0].visible());
}

/// "row link to popup": Shift on a fill's swatch chip opens the Color panel on that row.
#[test]
fn shift_clicking_a_chip_opens_the_mixer_on_that_row() {
    let mut panel = Panel::new(app_with_rect(), appearance::show);
    // PLAN: assumed accessible name of the swatch chip
    panel.click_with("Fill swatch", egui::Modifiers::SHIFT);
    // The Color panel opens as the row's popup.
    assert_eq!(panel.app().ui.transient_panel.as_deref(), Some("color"));
    assert_eq!(panel.app().session.appearance_item(), Some(0));
}

#[test]
fn the_thumbnail_drags_the_appearance_and_the_properties_fx_button_opens_the_effects() {
    // PLAN: the thumbnail's `PanelDrag::Appearance` payload needs a driver that moves the pointer while a press is held (op.A15)
    let mut panel = Panel::new(app_with_rect(), properties::show);
    // Properties: the fx button lists the effect groups.
    panel.click("fx");
    assert!(shows_after_frame(&mut panel, "Distort & Transform"));
}

/// Runs a frame of `panel`, then whether it shows a control named `name`.
fn shows_after_frame(panel: &mut Panel, name: &str) -> bool {
    panel.frame();
    panel.shows(name)
}
