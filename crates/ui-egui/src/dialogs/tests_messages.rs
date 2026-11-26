//! Messages and questions on the dialog host: the queue of messages and the order a confirmed
//! command runs in.

use serde_json::json;

use super::confirm::{self, KIND, MESSAGE};
use crate::VectorcraftApp;

/// An app with a document and a window to host dialogs in.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let ctx = app.ui_ctx.get_or_insert_with(egui::Context::default).clone();
    crate::testing::install(&ctx);
    app
}

/// One frame of the host, then the pump.
fn pass(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let Some(dialogs) = app.dialogs() else { return };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs.with(|host| host.run(ui)));
    out.textures_delta.clear();
    super::host::pump(app, &ctx);
}

/// The keys of the open dialogs.
fn keys(app: &VectorcraftApp) -> Vec<Option<String>> {
    app.dialogs().map(|d| d.with(|host| host.open_dialogs().into_iter().map(|o| o.key).collect())).unwrap_or_default()
}

/// Closes the top dialog: OK when `ok`, else Cancel; then a frame.
fn close_top(app: &mut VectorcraftApp, ok: bool) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            if let Some(id) = host.open_dialogs().last().map(|o| o.id) {
                if ok {
                    host.confirm_by_id(&ctx, id);
                } else {
                    host.cancel_by_id(&ctx, id);
                }
            }
        });
    }
    pass(app);
}

#[test]
fn a_message_told_while_a_dialog_is_open_shows_after_it_once_per_text() {
    let mut app = app();
    confirm::ask(&mut app, "Question?", "", "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}));
    pass(&mut app);
    confirm::tell(&mut app, "First", "");
    confirm::tell(&mut app, "First", "");
    confirm::tell(&mut app, "Second", "");
    pass(&mut app);
    assert_eq!(keys(&app), [Some(KIND.to_string())], "the messages wait behind the question");
    close_top(&mut app, false);
    assert_eq!(keys(&app), [Some(MESSAGE.to_string())], "the first message shows");
    close_top(&mut app, false);
    assert_eq!(keys(&app), [Some(MESSAGE.to_string())], "then the distinct one");
    close_top(&mut app, false);
    assert!(keys(&app).is_empty(), "the duplicate never shows");
}

#[test]
fn a_question_closes_before_its_command_runs_and_cancel_runs_nothing() {
    let mut app = app();
    let define = json!({"command": "variable.define"});
    confirm::ask(&mut app, "Define a variable?", "", "ui.menuDialog", define.clone());
    pass(&mut app);
    assert_eq!(keys(&app), [Some(KIND.to_string())]);
    close_top(&mut app, true);
    let open = keys(&app);
    assert_eq!(open.len(), 1, "only the dialog the command opened");
    assert_ne!(open[0].as_deref(), Some(KIND));
    close_top(&mut app, false);
    confirm::ask(&mut app, "Define a variable?", "", "ui.menuDialog", define);
    pass(&mut app);
    close_top(&mut app, false);
    assert!(keys(&app).is_empty(), "Cancel runs nothing");
}
