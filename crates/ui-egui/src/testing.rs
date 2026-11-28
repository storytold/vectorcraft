//! The app's own interface install for tests: the runtime, the fonts and the root theme the app
//! gives a real window, so a test lays text out and paints in the app's look; and the frame the
//! hosts run.

use craft::ui::runtime::frame::{FrameInput, set_input, take_output};
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
    let prefs = vectorcraft_engine::Prefs::default();
    let print = crate::state::print_ppi(prefs.display_print_size, None);
    set_input(ctx, FrameInput::new(crate::app::root_env(&prefs, 1.0, print)));
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

/// One frame of the whole app on `ctx` as the hosts run it (kb.C26): the app has the window's context, as the hosts
/// hand it when they create the app, gives the input of the frame
/// (`embed::before_input`, once, as the hosts call it), the window draws with `raw`, and the app takes what the runtime
/// gathered in it, running the commands it fired, as the host's next frame begins ([`run_fired`]). The frame's texture
/// uploads are handled.
pub(crate) fn app_frame(app: &mut crate::VectorcraftApp, ctx: &egui::Context, raw: egui::RawInput) -> egui::FullOutput {
    part_frame(app, ctx, raw, |app, ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    })
}

/// [`app_frame`] with `draw` showing part of the window in place of the whole app.
pub(crate) fn part_frame(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    raw: egui::RawInput,
    draw: impl FnMut(&mut crate::VectorcraftApp, &mut egui::Ui),
) -> egui::FullOutput {
    let mut out = part_frame_uploads(app, ctx, raw, draw);
    out.textures_delta.clear();
    out
}

/// [`part_frame`] with the frame's texture uploads kept in the output, for a test that reads them. The test clears
/// `textures_delta` once it has read it.
pub(crate) fn part_frame_uploads(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    mut raw: egui::RawInput,
    mut draw: impl FnMut(&mut crate::VectorcraftApp, &mut egui::Ui),
) -> egui::FullOutput {
    app.adopt_context(ctx);
    app.raw_input(ctx, &mut raw);
    embed::before_input(ctx, |out| app.frame_input(out));
    let out = ctx.run_ui(raw, |ui| draw(app, ui));
    run_fired(app, ctx);
    out
}

/// The app takes what the runtime gathered on `ctx` since the last frame began and runs the commands it fired, as the
/// host's next `embed::before_input` does, without giving the runtime its input: the next frame gives it. A second
/// input in one frame makes every command state the next frame publishes look changed, and the frame asks for a rerun.
pub(crate) fn run_fired(app: &mut crate::VectorcraftApp, ctx: &egui::Context) {
    let _ = app.frame_input(take_output(ctx));
}

/// Held by a test that changes the custom dictionary, which the whole process shares, or that needs it to hold still:
/// every change moves the dictionary's generation, and a Check Spelling walk that sees it move checks its scope again
/// from the start.
pub(crate) fn custom_dictionary() -> std::sync::MutexGuard<'static, ()> {
    static HELD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs the test `name` (its path in the crate, such as `canvas::tests::a_test`) alone in a child test process, for a
/// test that loads or removes a color profile. The loaded profiles are the whole process's, and the canvas's frame key
/// holds their generation ([`vectorcraft_color::cms::profiles_generation`]), so a profile loaded in this process while
/// another test renders changes that test's keys. A test that assigns a profile to its front document, or changes
/// Color Settings, also runs here: the active color settings ([`vectorcraft_color::cms::active`]) are the whole
/// process's too, and another test converting a color while they are changed gets that profile's values. In the child
/// it returns `true`, and the test goes on. Here it waits for the child, fails when the child's test fails, and returns
/// `false`.
pub(crate) fn in_own_process(name: &str) -> bool {
    const CHILD: &str = "VECTORCRAFT_TEST_OWN_PROCESS";
    if std::env::var_os(CHILD).is_some_and(|v| v == name) {
        return true;
    }
    let output =
        std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", name, "--nocapture"]).env(CHILD, name).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
        "{name} failed in its own process:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    false
}

/// The default proof view, held: View › Proof Colors, Overprint Preview and Separations Preview are the whole process's
/// and the canvas draws through them, so a test that sets them, or that compares the canvas's renders, holds this. It
/// leaves the default view when it is dropped.
pub(crate) struct DefaultProofView {
    _held: std::sync::MutexGuard<'static, ()>,
}

impl DefaultProofView {
    pub(crate) fn new() -> Self {
        static HELD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        vectorcraft_render::proof::set_view(Default::default());
        DefaultProofView { _held: held }
    }
}

impl Drop for DefaultProofView {
    fn drop(&mut self) {
        vectorcraft_render::proof::set_view(Default::default());
    }
}

/// Gives `fixture` the app's input for its next frame as the hosts give it (`embed::before_input`): the commands the
/// frames since the last call fired run first.
pub(crate) fn fixture_input(app: &mut crate::VectorcraftApp, fixture: &mut craft::ui::testing::Fixture) {
    let input = app.frame_input(craft::ui::runtime::frame::take_output(fixture.ctx()));
    fixture.root(input.root).commands(input.commands);
    if let Some(host) = input.host {
        fixture.host(host);
    }
}
