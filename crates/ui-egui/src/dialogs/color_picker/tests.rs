use craft::color_ui::ui::{ScreenPickGesture, StillCapture};
use vectorcraft_engine::Session;

use super::*;

fn app() -> (VectorcraftApp, egui::Context) {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui_ctx = Some(ctx.clone());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    (app, ctx)
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let mut out = ctx.run_ui(Default::default(), |ui| super::pump(app, ui.ctx()));
    out.textures_delta.clear();
}

fn fill_hex(app: &VectorcraftApp) -> String {
    crate::panels::current_paints(app).0.color().unwrap().to_hex()
}

fn id(app: &VectorcraftApp) -> DialogId {
    app.color_dialogs.pickers.last().unwrap().id
}

/// `ui.dialog.set {field: "color"}` on the newest picker, then a pass.
fn set_hex(app: &mut VectorcraftApp, ctx: &egui::Context, hex: &str) {
    let id = id(app);
    app.dialogs().unwrap().with(|h| h.set_json_field(ctx, id, "color", json!(hex)).unwrap());
    frame(app, ctx);
}

fn ok(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let id = id(app);
    assert!(app.dialogs().unwrap().with(|h| h.confirm_by_id(ctx, id)));
    frame(app, ctx);
}

fn cancel(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let id = id(app);
    assert!(app.dialogs().unwrap().with(|h| h.cancel_by_id(ctx, id)));
    frame(app, ctx);
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn artboard_color(app: &VectorcraftApp) -> Option<Color> {
    app.session.doc().unwrap().doc.artboards[1].color
}

fn two_documents_with_artboards() -> (VectorcraftApp, egui::Context) {
    let (mut app, ctx) = app();
    app.run("artboard.new", json!({})).unwrap();
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    (app, ctx)
}

/// The fill (or stroke) colour of the active document's selection.
fn paint_hex(app: &VectorcraftApp, stroke: bool) -> Option<String> {
    let (fill, line) = crate::panels::current_paints(app);
    if stroke { line } else { fill }.color().map(|c| c.to_hex())
}

/// Two documents, each with a selected rectangle; the first is active.
fn two_documents_with_selected_art() -> (VectorcraftApp, egui::Context) {
    let (mut app, ctx) = app();
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 60})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    (app, ctx)
}

fn picker_params(stroke: bool) -> Value {
    json!({ "stroke": stroke })
}

#[test]
fn fill_and_stroke_pickers_preview_only_in_their_opening_document() {
    for stroke in [false, true] {
        let (mut app, ctx) = two_documents_with_selected_art();
        app.run("ui.colorPicker", picker_params(stroke)).unwrap();
        set_hex(&mut app, &ctx, "#ff0000");
        assert_eq!(paint_hex(&app, stroke).as_deref(), Some("#ff0000"), "the opening document previews");

        app.run("document.activate", json!({"index": 1})).unwrap();
        let other = app.session.doc().unwrap().doc.layers.clone();
        let before = undo_len(&app);
        set_hex(&mut app, &ctx, "#00ff00");
        assert_eq!(app.session.doc().unwrap().doc.layers, other, "the other document never previews the color");
        assert!(!app.session.in_interaction());
        assert_eq!(undo_len(&app), before);

        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_ne!(paint_hex(&app, stroke).as_deref(), Some("#ff0000"), "leaving the source rolled its preview back");
        let before = undo_len(&app);
        ok(&mut app, &ctx);
        assert_eq!(paint_hex(&app, stroke).as_deref(), Some("#00ff00"), "OK back in the opening document applies");
        assert_eq!(undo_len(&app), before + 1);
    }
}

#[test]
fn confirming_a_fill_or_stroke_picker_in_another_document_leaves_both_unchanged() {
    for stroke in [false, true] {
        let (mut app, ctx) = two_documents_with_selected_art();
        let source = app.session.doc().unwrap().doc.layers.clone();
        let source_undo = undo_len(&app);
        app.run("ui.colorPicker", json!({"stroke": stroke, "color": "#00ff00"})).unwrap();
        frame(&mut app, &ctx);
        app.run("document.activate", json!({"index": 1})).unwrap();
        let destination = app.session.doc().unwrap().doc.layers.clone();
        let destination_undo = undo_len(&app);
        ok(&mut app, &ctx);
        assert_eq!(app.ui.status, "The active document changed while the Color Picker was open");
        assert_eq!(app.session.doc().unwrap().doc.layers, destination);
        assert_eq!(undo_len(&app), destination_undo);

        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.layers, source);
        assert_eq!(undo_len(&app), source_undo);
    }
}

#[test]
fn canceling_a_fill_or_stroke_picker_preserves_the_other_documents_interaction() {
    for stroke in [false, true] {
        let (mut app, ctx) = two_documents_with_selected_art();
        app.run("ui.colorPicker", picker_params(stroke)).unwrap();
        set_hex(&mut app, &ctx, "#ff0000");
        app.run("document.activate", json!({"index": 1})).unwrap();
        app.begin_interaction("Other edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#445566"})).unwrap();

        cancel(&mut app, &ctx);
        assert!(app.session.in_interaction(), "the current document keeps its own interaction");
        assert_eq!(paint_hex(&app, false).as_deref(), Some("#445566"));
    }
}

#[test]
fn a_picker_opened_without_a_document_leaves_a_document_opened_later_unchanged() {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui_ctx = Some(ctx.clone());
    app.run("ui.colorPicker", json!({"color": "#00ff00"})).unwrap();
    frame(&mut app, &ctx);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let art = app.session.doc().unwrap().doc.layers.clone();
    let steps = undo_len(&app);
    ok(&mut app, &ctx);
    assert_eq!(app.ui.status, "The active document changed while the Color Picker was open");
    assert_eq!(app.session.doc().unwrap().doc.layers, art);
    assert_eq!(undo_len(&app), steps);
}

#[test]
fn a_fill_or_stroke_picker_leaves_a_reverted_document_unchanged() {
    for stroke in [false, true] {
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("paint-picker-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("ui.colorPicker", picker_params(stroke)).unwrap();
        set_hex(&mut app, &ctx, "#abcdef");
        assert_eq!(paint_hex(&app, stroke).as_deref(), Some("#abcdef"));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let restored = app.session.doc().unwrap().doc.layers.clone();
        let steps = undo_len(&app);
        frame(&mut app, &ctx);
        ok(&mut app, &ctx);
        assert!(app.ui.status.contains("document was reverted"));
        assert_eq!(app.session.doc().unwrap().doc.layers, restored);
        assert_eq!(undo_len(&app), steps);
    }
}

/// The selected art takes each colour as it is picked; OK keeps the last as one undo step and
/// Cancel puts the art back.
#[test]
fn the_selection_shows_the_colour_live() {
    let (mut app, ctx) = app();
    let before = undo_len(&app);
    app.run("ui.colorPicker", json!({})).unwrap();
    frame(&mut app, &ctx);
    assert!(!app.session.in_interaction(), "nothing previews before the colour changes");
    for hex in ["#ff0000", "#00ff00"] {
        set_hex(&mut app, &ctx, hex);
        assert_eq!(fill_hex(&app), hex, "live, before OK");
    }
    ok(&mut app, &ctx);
    assert_eq!(fill_hex(&app), "#00ff00");
    assert_eq!(undo_len(&app), before + 1, "one step");
    app.run("ui.colorPicker", json!({})).unwrap();
    set_hex(&mut app, &ctx, "#0000ff");
    assert_eq!(fill_hex(&app), "#0000ff");
    cancel(&mut app, &ctx);
    assert_eq!(fill_hex(&app), "#00ff00");
    assert_eq!(undo_len(&app), before + 1);
}

/// Opened for an artboard, the picker shows each colour on its paper, with the art's fill unchanged.
#[test]
fn an_artboard_shows_its_background_color_live() {
    let (mut app, ctx) = app();
    app.run("select.set", json!({"ids": []})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    let art = app.session.doc().unwrap().doc.layers.clone();
    let before = undo_len(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app, &ctx);
    assert!(!app.session.in_interaction(), "nothing previews before the color changes");
    for hex in ["#ff0000", "#00ff00"] {
        set_hex(&mut app, &ctx, hex);
        assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some(hex), "live, before OK");
    }
    ok(&mut app, &ctx);
    assert_eq!(undo_len(&app), before + 1, "one step");
    assert_eq!(app.session.doc().unwrap().doc.layers, art, "the art keeps its fill");
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    set_hex(&mut app, &ctx, "#0000ff");
    cancel(&mut app, &ctx);
    assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#00ff00"));
    assert!(app.run("ui.colorPicker", json!({"artboard": 9})).is_err());
}

#[test]
fn the_artboard_picker_previews_only_in_its_opening_document() {
    let (mut app, ctx) = two_documents_with_artboards();
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    set_hex(&mut app, &ctx, "#ff0000");
    assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#ff0000"));

    app.run("document.activate", json!({"index": 1})).unwrap();
    let before = undo_len(&app);
    set_hex(&mut app, &ctx, "#00ff00");
    assert_eq!(artboard_color(&app), None, "the other document never previews the color");
    assert!(!app.session.in_interaction());
    assert_eq!(undo_len(&app), before);

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(artboard_color(&app), None, "leaving the source rolled its preview back");
    let before = undo_len(&app);
    ok(&mut app, &ctx);
    assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#00ff00"));
    assert_eq!(undo_len(&app), before + 1);
}

#[test]
fn confirming_an_artboard_picker_in_another_document_leaves_both_unchanged() {
    let (mut app, ctx) = two_documents_with_artboards();
    let source_undo = undo_len(&app);
    app.run("ui.colorPicker", json!({"artboard": 1, "color": "#00ff00"})).unwrap();
    frame(&mut app, &ctx);
    app.run("document.activate", json!({"index": 1})).unwrap();
    let destination_undo = undo_len(&app);
    ok(&mut app, &ctx);
    assert_eq!(app.ui.status, "The active document changed while the Color Picker was open");
    assert_eq!(artboard_color(&app), None);
    assert_eq!(undo_len(&app), destination_undo);

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(artboard_color(&app), None);
    assert_eq!(undo_len(&app), source_undo);
}

#[test]
fn canceling_an_artboard_picker_preserves_the_other_documents_interaction() {
    let (mut app, ctx) = two_documents_with_artboards();
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    set_hex(&mut app, &ctx, "#ff0000");
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.begin_interaction("Other edit").unwrap();
    app.session.preview("artboard.setProps", &json!({"index": 1, "color": "#445566"})).unwrap();

    cancel(&mut app, &ctx);
    assert!(app.session.in_interaction(), "the current document keeps its own interaction");
    assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#445566"));
}

#[test]
fn artboard_pickers_for_the_same_index_in_different_documents_have_separate_instances() {
    let (mut app, _) = two_documents_with_artboards();
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    let first = id(&app);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    assert_ne!(id(&app), first);
    assert_eq!(app.color_dialogs.pickers.len(), 2);
}

/// OK on an untouched picker for an artboard without a color leaves it without one and adds no
/// undo step; a white the opener passes as `color` is set.
#[test]
fn an_untouched_picker_leaves_an_artboard_without_a_color() {
    let (mut app, ctx) = app();
    app.run("artboard.new", json!({})).unwrap();
    let before = undo_len(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app, &ctx);
    ok(&mut app, &ctx);
    assert_eq!(artboard_color(&app), None, "no color");
    assert_eq!(undo_len(&app), before, "no undo step");
    app.run("ui.colorPicker", json!({"artboard": 1, "color": "#ffffff"})).unwrap();
    frame(&mut app, &ctx);
    ok(&mut app, &ctx);
    assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#ffffff"));
    assert_eq!(undo_len(&app), before + 1);
}

/// The same for an RGB color in a CMYK document: kept untouched, CMYK once a color is picked.
#[test]
fn an_untouched_picker_keeps_an_artboards_rgb_color_in_a_cmyk_document() {
    let (mut app, ctx) = app();
    app.run("artboard.new", json!({})).unwrap();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
    let start = artboard_color(&app);
    assert!(matches!(start, Some(Color::Rgb { .. })), "set as RGB");
    let before = undo_len(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app, &ctx);
    ok(&mut app, &ctx);
    assert_eq!(artboard_color(&app), start, "the RGB color is kept");
    assert_eq!(undo_len(&app), before, "no undo step");
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    set_hex(&mut app, &ctx, "#00ff00");
    ok(&mut app, &ctx);
    assert!(matches!(artboard_color(&app), Some(Color::Cmyk { .. })), "a picked color is set as CMYK");
    assert_eq!(undo_len(&app), before + 1);
}

/// The opener's params reach the dialog and OK writes the fill or the stroke.
#[test]
fn open_set_hex_and_confirm_fills() {
    let (mut app, ctx) = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    set_hex(&mut app, &ctx, "#00ff00");
    ok(&mut app, &ctx);
    assert_eq!(fill_hex(&app), "#00ff00");
    app.run("ui.colorPicker", json!({"stroke": true})).unwrap();
    set_hex(&mut app, &ctx, "#3366cc");
    ok(&mut app, &ctx);
    assert_eq!(crate::panels::current_paints(&app).1.color().unwrap().to_hex(), "#3366cc");
    assert!(app.run("ui.colorPicker", json!({"color": "nope"})).is_err());
    app.run("ui.colorPicker", json!({"stroke": false, "color": {"c": 0, "m": 100, "y": 100, "k": 0}})).unwrap();
    frame(&mut app, &ctx);
    ok(&mut app, &ctx);
    assert_eq!(crate::panels::current_paints(&app).0.color(), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
}

/// OK makes the picked fill or stroke color the Color Guide's base color; Cancel and an artboard's
/// color leave the base as it is.
#[test]
fn ok_makes_the_picked_color_the_color_guides_base() {
    let (mut app, ctx) = app();
    let base = |app: &VectorcraftApp| crate::panels::color_guide::base_color(app).to_hex();
    app.ui.color_guide_base = Some(Color::WHITE);
    app.run("ui.colorPicker", json!({})).unwrap();
    set_hex(&mut app, &ctx, "#2050a0");
    ok(&mut app, &ctx);
    assert_eq!(base(&app), "#2050a0");
    app.run("ui.colorPicker", json!({"stroke": true})).unwrap();
    set_hex(&mut app, &ctx, "#a02050");
    cancel(&mut app, &ctx);
    assert_eq!(base(&app), "#2050a0", "Cancel keeps the base");
    app.run("select.set", json!({"ids": []})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    set_hex(&mut app, &ctx, "#00ff00");
    ok(&mut app, &ctx);
    assert_eq!(base(&app), "#2050a0", "an artboard's color leaves the base");
}

/// A confirmed colour in a CMYK document is CMYK.
#[test]
fn every_mode_draws_and_cmyk_documents_get_cmyk() {
    let (mut app, ctx) = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("ui.colorPicker", json!({"color": "#ff0000"})).unwrap();
    frame(&mut app, &ctx);
    ok(&mut app, &ctx);
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })));
}

/// The pipette's pick reaches the dialog, previews on the art, and Cancel restores the original.
#[test]
fn sampler_above_the_field_previews_and_cancel_restores_the_original() {
    let (mut app, ctx) = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    let original = fill_hex(&app);
    let owner = super::owner(id(&app));
    assert_eq!(screen_color::hold(&mut app, &ctx, owner, ScreenPickGesture::Clicked), None);
    assert!(app.screen_pick.busy());
    let green = Srgba { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    for _ in 0..2 {
        app.screen_pick.tick_with(&ctx, screen_color::target(&ctx), &|| Box::new(StillCapture::color(green)));
    }
    frame(&mut app, &ctx);
    assert_eq!(fill_hex(&app), "#00ff00");
    cancel(&mut app, &ctx);
    assert_eq!(fill_hex(&app), original);
}

#[test]
fn the_artboard_picker_leaves_restored_artboards_unchanged() {
    for accept in [true, false] {
        let (mut app, ctx) = app();
        app.run("artboard.new", json!({})).unwrap();
        app.run("artboard.setProps", json!({"index": 1, "color": "#123456"})).unwrap();
        let dir = vectorcraft_testkit::TempDir::new("artboard-picker-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        set_hex(&mut app, &ctx, "#abcdef");
        assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#abcdef"));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let restored = artboard_color(&app);
        assert_eq!(restored.map(|c| c.to_hex()).as_deref(), Some("#123456"));
        let steps = undo_len(&app);
        frame(&mut app, &ctx);
        assert_eq!(artboard_color(&app), restored);
        assert!(!app.session.in_interaction());
        if accept {
            ok(&mut app, &ctx);
            assert!(app.ui.status.contains("document was reverted"));
        } else {
            app.begin_interaction("New drawing edit").unwrap();
            cancel(&mut app, &ctx);
            assert!(app.session.in_interaction());
            app.session.cancel_interaction().unwrap();
        }
        assert_eq!(artboard_color(&app), restored);
        assert_eq!(undo_len(&app), steps);

        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        set_hex(&mut app, &ctx, "#abcdef");
        ok(&mut app, &ctx);
        assert_eq!(artboard_color(&app).map(|c| c.to_hex()).as_deref(), Some("#abcdef"));
    }
}
