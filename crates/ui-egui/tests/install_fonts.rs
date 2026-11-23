//! The interface fonts ([`ui_fonts`]) installed on a fresh context, as an embedder that draws the panels and dialogs
//! installs them (`embed::install`): every family they draw in is bound, so an effect dialog draws. CI also runs it
//! as an embedder that brings its own document fonts builds the crate
//! (`cargo test -p vectorcraft-ui-egui --no-default-features --test install_fonts`).

use craft::ui::fonts::SEMIBOLD;
use craft::ui::shell::embed;
use egui::accesskit::Role;
use egui::{FontFamily, Pos2, RawInput, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, ui_fonts};

/// One frame of the whole app → whether its accessibility tree holds a dialog.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let raw = RawInput { focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() };
    embed::before_input(ctx, |out| app.frame_input(out));
    let mut output = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    output.textures_delta.clear();
    output.platform_output.accesskit_update.is_some_and(|update| update.nodes.iter().any(|(_, node)| node.role() == Role::Dialog))
}

/// An effect dialog, whose titles are semibold, draws on a context whose fonts only `embed::install` set up, and the
/// families the panels draw in (proportional, monospace and semibold) each have a face.
#[test]
fn an_effect_dialog_draws_with_the_fonts_embed_install_sets_up() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let ctx = egui::Context::default();
    embed::install(&ctx, craft::ui::testing::install_spec(), &ui_fonts()).unwrap();
    ctx.enable_accesskit();
    // The dialog opens in the app's window, which its first frame makes.
    frame(&mut app, &ctx);
    app.run("effect.dialog", json!({"effect": "blur.gaussian"})).unwrap();
    assert!((0..4).any(|_| frame(&mut app, &ctx)), "the effect dialog draws");
    let families = ctx.fonts(|fonts| fonts.definitions().families.clone());
    for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(SEMIBOLD.into())] {
        assert!(families.get(&family).is_some_and(|faces| !faces.is_empty()), "{family:?} has no face");
    }
}
