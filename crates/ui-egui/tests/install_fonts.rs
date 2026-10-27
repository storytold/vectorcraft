//! `theme::install_fonts` on a context the app never ran on, as an embedder that draws the panels
//! or dialogs itself makes it: every family they draw in is bound, so an effect dialog draws. With
//! the `interface-fonts` feature the interface's own faces head them; without it
//! (`cargo test -p vectorcraft-ui-egui --no-default-features --test install_fonts`) they are
//! egui's.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::{FontFamily, FontId};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, dialogs, theme};

/// The faces `install_fonts` adds with the `interface-fonts` feature.
const INTERFACE_FACES: [&str; 5] = ["Inter", "Inter-SemiBold", "SourceSans3", "SourceSans3-Semibold", "JetBrainsMono"];

/// A context with the interface's fonts installed and nothing else done to it.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    ctx
}

/// One headless frame running `draw`.
fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
    let mut out = ctx.run_ui(egui::RawInput::default(), draw);
    out.textures_delta.clear();
}

/// Text in each family the panels draw in lays out, and the faces at the head of those families are
/// the interface's with `interface-fonts` and egui's without it.
#[test]
fn every_family_the_panels_draw_in_is_bound() {
    let ctx = context();
    frame(&ctx, |ui| {
        for font in [theme::semibold(12.0), theme::mono(12.0), FontId::new(12.0, FontFamily::Name(theme::FONT_UI.into()))] {
            ui.label(egui::RichText::new("Gaussian Blur").font(font));
        }
    });
    let definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    for name in [theme::FONT_UI, theme::FONT_UI_SEMIBOLD, theme::FONT_MONO] {
        let family = &definitions.families[&FontFamily::Name(name.into())];
        assert!(!family.is_empty(), "{name} has no font");
    }
    let installed: Vec<&str> = INTERFACE_FACES.into_iter().filter(|face| definitions.font_data.contains_key(*face)).collect();
    let family = |family: FontFamily| definitions.families[&family].clone();
    let named = |name: &str| family(FontFamily::Name(name.into()));
    if cfg!(feature = "interface-fonts") {
        assert_eq!(installed, INTERFACE_FACES);
        assert_eq!(named(theme::FONT_UI)[0], "Inter");
        assert_eq!(named(theme::FONT_UI_SEMIBOLD)[0], "Inter-SemiBold");
        assert_eq!(named(theme::FONT_MONO)[0], "JetBrainsMono");
        assert_eq!(family(FontFamily::Proportional)[0], "Inter");
    } else {
        assert!(installed.is_empty(), "{installed:?}");
        let egui = egui::FontDefinitions::default();
        let proportional = &egui.families[&FontFamily::Proportional];
        // egui 0.37 only bundles its faces with its `default_fonts` feature; this crate asks for
        // it, since a dependency's dev-features do not reach us and an empty family lays out tofu.
        assert!(!proportional.is_empty(), "egui's default fonts are not compiled in");
        assert!(named(theme::FONT_UI).starts_with(proportional));
        assert!(named(theme::FONT_UI_SEMIBOLD).starts_with(proportional));
        assert!(named(theme::FONT_MONO).starts_with(&egui.families[&FontFamily::Monospace]));
        assert!(family(FontFamily::Proportional).starts_with(proportional));
    }
}

/// An effect dialog, whose titles are semibold, draws without panicking on a context only
/// `install_fonts` prepared.
#[test]
fn an_effect_dialog_draws_on_a_context_the_app_never_ran_on() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "blur.gaussian"})).unwrap();
    assert!(app.ui.dialog.is_some());
    let ctx = context();
    frame(&ctx, |ui| dialogs::show(&mut app, ui.ctx()));
}
