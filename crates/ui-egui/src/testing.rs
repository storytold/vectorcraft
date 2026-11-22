//! The app's own interface install for tests: the runtime, the fonts and the root theme the app
//! gives a real window, so a test lays text out and paints in the app's look.

use craft::ui::runtime::frame::{FrameInput, set_input};
use craft::ui::shell::embed;

/// Installs the runtime with a window system that has no OS services, the app's interface fonts
/// ([`crate::ui_fonts`]) and its root values from the default preferences on `ctx`, as the hosts
/// and [`crate::VectorcraftApp::frame_input`] do. A fixture's context has the runtime already and
/// gets the fonts alone.
pub(crate) fn install(ctx: &egui::Context) {
    let fonts = crate::ui_fonts();
    if embed::install(ctx, craft::ui::testing::install_spec(), &fonts).is_err() {
        fonts.install(ctx);
    }
    set_input(ctx, FrameInput::new(crate::app::root_env(&vectorcraft_engine::Prefs::default(), 1.0)));
}

/// `app` shown in a window of its own: its context is the one every frame of the test runs on, so its dialogs open on
/// the window's host, focus and drags last from frame to frame, and the access tree is on for the tests that find a
/// control by its role.
pub(crate) fn windowed(mut app: crate::VectorcraftApp) -> crate::VectorcraftApp {
    let ctx = ui_context();
    ctx.enable_accesskit();
    app.ui_ctx = Some(ctx);
    app
}

/// A context with the runtime, the app's interface fonts and its root theme installed.
pub(crate) fn ui_context() -> egui::Context {
    let ctx = egui::Context::default();
    install(&ctx);
    ctx
}
