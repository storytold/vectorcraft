//! The app's access to its window's dialog host, and the once-per-frame pump that serves every dialog
//! module and every held dialog.

use craft::ui::dialog::{DialogId, DialogResult, HoldsOk};
use craft::ui::runtime::env::WindowKey;
use craft::ui::shell::WindowDialogs;
use serde_json::Value;

use crate::VectorcraftApp;

/// The most dialogs waiting for an answer at once.
pub(crate) const MAX_WAITING: usize = 16;

/// What the app told the user for a dialog while it waited, as (dialog key, message) pairs, at most
/// [`MAX_WAITING`] of them.
#[derive(Default)]
pub(crate) struct Told(pub Vec<(String, String)>);

/// The main window's dialog host.
pub(crate) fn window_dialogs(ctx: &egui::Context) -> WindowDialogs {
    WindowDialogs::of(ctx, &WindowKey::main())
}

impl VectorcraftApp {
    /// The dialog host of the window, `None` before the first frame gave the app its context.
    pub(crate) fn dialogs(&self) -> Option<WindowDialogs> {
        self.ui_ctx.as_ref().map(window_dialogs)
    }
}

/// What a held dialog's open adds to [`open_held_with`].
#[derive(Default)]
pub(crate) struct HeldOptions {
    /// The dialog this one opens over; it closes with its parent.
    pub parent: Option<DialogId>,
    /// Runs when the dialog is cancelled (a live preview's rollback).
    pub cancel: Option<Cancel>,
}

/// Rolls back what a held dialog did, when it is cancelled.
pub(crate) type Cancel = Box<dyn FnOnce(&mut VectorcraftApp)>;

/// Runs one held dialog's pass; `Some(cancelled)` once the dialog is gone.
type Drain = Box<dyn FnMut(&mut VectorcraftApp) -> Option<bool>>;

struct Record {
    id: DialogId,
    drain: Drain,
    cancel: Option<Cancel>,
}

/// The held dialogs of the app: one record each, served by [`pump`].
#[derive(Default)]
pub(crate) struct Held(Vec<Record>);

impl Held {
    fn reopened(&self, id: DialogId) -> bool {
        self.0.iter().any(|record| record.id == id)
    }
}

fn drain<D: HoldsOk>(id: DialogId, serial: u64, mut ok: impl FnMut(&mut VectorcraftApp, &D::Answer) -> Result<(), String> + 'static) -> Drain {
    Box::new(move |app| {
        // Without a window nothing can answer: the dialog counts as cancelled so its rollback runs.
        let Some(host) = app.dialogs() else { return Some(true) };
        match host.with(|h| h.open_serial(id)) {
            None => return host.with(|h| h.take_by_id::<D>(id)).map(|r| matches!(r, DialogResult::Cancel)),
            Some(open) if open != serial => return Some(false),
            Some(_) => {}
        }
        if let Some(answer) = host.with(|h| h.get_mut_by_id::<D>(id).and_then(D::take_ok)) {
            // The host's lock is released while `ok` runs: it may open dialogs.
            let verdict = ok(app, &answer).map_err(Into::into);
            // A callback can reopen the same key. Its old verdict cannot answer that opening.
            if app.held.reopened(id) {
                return Some(false);
            }
            host.with(|h| {
                if h.open_serial(id) == Some(serial)
                    && let Some(d) = h.get_mut_by_id::<D>(id)
                {
                    d.answer_ok(verdict);
                }
            });
        }
        // A replace-by-open can leave the retired opening's Cancel under the same dialog ID.
        // A live opening has no final result for this record yet.
        None
    })
}

/// Opens `dialog` holding its OK: OK runs `ok`, and an `Err` keeps the dialog open with the reason under its
/// rows. `None` without a window.
pub(crate) fn open_held<D: HoldsOk>(
    app: &mut VectorcraftApp,
    dialog: D,
    ok: impl FnMut(&mut VectorcraftApp, &D::Answer) -> Result<(), String> + 'static,
) -> Option<DialogId> {
    open_held_with(app, dialog, ok, HeldOptions::default())
}

/// [`open_held`] with a parent and a cancel hook.
pub(crate) fn open_held_with<D: HoldsOk>(
    app: &mut VectorcraftApp,
    dialog: D,
    ok: impl FnMut(&mut VectorcraftApp, &D::Answer) -> Result<(), String> + 'static,
    options: HeldOptions,
) -> Option<DialogId> {
    let host = app.dialogs()?;
    let dialog = dialog.held_ok();
    let (id, serial) = host.with(|h| {
        let id = match options.parent {
            Some(parent) => h.open_child(dialog, parent),
            None => h.open(dialog),
        };
        h.open_serial(id).map(|serial| (id, serial))
    })?;
    // A host key can be reopened before the previous result has been pumped.
    app.held.0.retain(|record| record.id != id);
    app.held.0.push(Record { id, drain: drain::<D>(id, serial, ok), cancel: options.cancel });
    Some(id)
}

/// The dialog on top of the stack.
pub(crate) fn top_dialog(app: &VectorcraftApp) -> Option<DialogId> {
    // `open_dialogs` lists in opening order, the top one last.
    app.dialogs()?.with(|h| h.open_dialogs().last().map(|d| d.id))
}

/// Whether the top dialog has a Reset to run.
pub(crate) fn can_reset_top(app: &VectorcraftApp) -> bool {
    let (Some(host), Some(top)) = (app.dialogs(), top_dialog(app)) else { return false };
    host.with(|h| h.can_reset_by_id(top))
}

/// `ui.dialogReset`: resets the top dialog.
pub(crate) fn reset_top(app: &mut VectorcraftApp) -> Result<Value, String> {
    let (Some(host), Some(ctx), Some(top)) = (app.dialogs(), app.ui_ctx.clone(), top_dialog(app)) else {
        return Err("no dialog open".into());
    };
    if host.with(|h| h.reset_by_id(&ctx, top)) { Ok(Value::Null) } else { Err("the dialog has no Reset".into()) }
}

/// Pushes the app's facts into the dialogs that convert against them (Create Object Mosaic against the selected
/// image, Width Point Edit against the selected stroke).
pub(crate) fn sync(app: &mut VectorcraftApp) {
    super::object_mosaic::sync(app);
    super::width_point::sync(app);
}

macro_rules! pumps {
    ($app:ident, $ctx:ident; $($path:path),+ $(,)?) => { $( $path($app, $ctx); )+ };
}

/// Runs once per frame after the shell pass: every dialog module's pump, then the held dialogs.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    sync(app);
    pumps!(app, ctx;
        crate::file_asks::pump,
        crate::io::save::pump,
        crate::unsaved::pump,
        super::safe_mode::pump,
        crate::updates_ui::pump,
        super::profile_mismatch::pump,
        crate::recovery::pump,
        super::missing_links::pump,
        super::find_links::pump,
    );
    super::missing_fonts::open_next(app);
    pumps!(app, ctx;
        super::dxf_import::pump,
        super::import_pdf::pump,
        crate::place::pump,
        super::new_document::pump,
        super::document_setup::pump,
        super::file_info::pump,
        super::package::pump,
        super::layer_options::pump,
        super::artboard_options::pump,
        super::rearrange_artboards::pump,
        super::navigator_panel_options::pump,
        super::effect::pump,
        super::halftone::pump,
        super::envelope::pump,
        super::effect_gallery::pump,
        super::three_d::pump,
        super::svg_filter::pump,
        super::export_host::pump,
        super::export_for_screens::pump,
        super::save_for_web::pump,
        super::save_pdf::pump,
        super::flatten::pump,
        super::preset_manager::pump,
        super::print::pump,
        super::javascript_files::pump,
        super::script_check::pump,
        super::object_dialogs::pump,
        super::transform::pump,
        super::transform_each::pump,
        super::shapes::pump,
        super::path_ops::pump,
        super::dimension::pump,
        super::option_form::pump,
        super::live_paint_gaps::pump,
        super::graph_type::pump,
        super::graph_data::pump,
        super::graph_design::pump,
        super::perspective_grid::pump,
        super::perspective_presets::pump,
        super::slices::pump,
        super::blend_options::pump,
        super::saved_names::pump,
        super::object_mosaic::pump,
        super::width_point::pump,
        super::indexed_color::pump,
        super::tracing_presets::pump,
        super::plugin::pump,
        crate::addons::dialogs::pump,
        super::char_style_options::pump,
        super::para_style_options::pump,
        super::new_text_style::pump,
        super::justification::pump,
        super::kinsoku_sets::pump,
        super::composite_fonts::pump,
        super::bullets_numbering::pump,
        super::find_replace::pump,
        super::check_spelling::pump,
        super::custom_dictionary::pump,
        super::confirm::pump,
        super::confirm::pump_missing_font,
        super::actions::pump,
        super::script_run::pump,
        super::color_picker::pump,
        super::proof_setup::pump,
        super::recolor::pump,
        super::saturate::pump,
        super::color_balance::pump,
        super::flare_options::pump,
        super::gradient_stop::pump,
        super::swatch_options::pump,
        super::new_swatch::pump,
        super::brush_options::pump,
        super::brush_delete::pump,
        super::brush_replace_art::pump,
        super::symbol_delete::pump,
        crate::find_font::pump,
        crate::workspaces::pump,
        crate::custom_toolbars::pump,
        super::command::pump,
    );
    // A drain's `ok` may open held dialogs: those join the records that stay.
    let mut kept = Vec::new();
    for mut record in std::mem::take(&mut app.held.0) {
        if app.held.reopened(record.id) {
            continue;
        }
        match (record.drain)(app) {
            None => kept.push(record),
            Some(true) => {
                if let Some(cancel) = record.cancel.take() {
                    cancel(app);
                }
            }
            Some(false) => {}
        }
    }
    kept.retain(|record| !app.held.reopened(record.id));
    kept.append(&mut app.held.0);
    app.held.0 = kept;
}

#[cfg(test)]
mod opening_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::dialog::HeldOk;
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamDialog};
    use vectorcraft_engine::Session;

    use super::*;

    const KEY: &str = "testHeldOpening";

    fn app() -> (VectorcraftApp, egui::Context) {
        let app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let ctx = app.ui_ctx.clone().unwrap();
        (app, ctx)
    }

    fn dialog(ctx: &egui::Context) -> HostedParamDialog {
        HostedParamDialog::new(ctx, KEY, DialogFields::new(), |_| ParamDialog::new(KEY, "Test dialog", vec![]))
    }

    #[test]
    fn held_ok_uses_the_replacement_openings_callback() {
        let (mut app, ctx) = app();
        let host = app.dialogs().unwrap();
        let retired = Rc::new(RefCell::new(Vec::new()));
        let before = retired.clone();
        let first = open_held(&mut app, dialog(&ctx), move |_, _| {
            before.borrow_mut().push("old");
            Ok(())
        })
        .unwrap();
        assert!(host.with(|h| h.cancel_by_id(&ctx, first)));

        let answers = retired.clone();
        let fresh = open_held(&mut app, dialog(&ctx), move |_, _| {
            answers.borrow_mut().push("new");
            Ok(())
        })
        .unwrap();
        assert_eq!(first, fresh);
        assert!(!host.with(|h| h.confirm_by_id(&ctx, fresh)));
        assert_eq!(host.with(|h| h.held_state(fresh)), Some(HeldOk::Waiting));
        pump(&mut app, &ctx);
        assert_eq!(*retired.borrow(), ["new"]);
        assert_eq!(host.with(|h| h.held_state(fresh)), Some(HeldOk::Accepted));
    }

    #[test]
    fn pumping_a_replacement_before_ok_keeps_its_callback() {
        let (mut app, ctx) = app();
        let host = app.dialogs().unwrap();
        open_held(&mut app, dialog(&ctx), |_, _| panic!("the retired opening cannot answer")).unwrap();

        let accepted = Rc::new(RefCell::new(Vec::new()));
        let seen = accepted.clone();
        let fresh = open_held(&mut app, dialog(&ctx), move |_, _| {
            seen.borrow_mut().push("new");
            Ok(())
        })
        .unwrap();

        pump(&mut app, &ctx);
        assert!(!host.with(|h| h.confirm_by_id(&ctx, fresh)));
        pump(&mut app, &ctx);
        assert_eq!(*accepted.borrow(), ["new"]);
        assert_eq!(host.with(|h| h.held_state(fresh)), Some(HeldOk::Accepted));
    }

    #[test]
    fn cancelling_a_replacement_runs_only_its_cancel_callback() {
        let (mut app, ctx) = app();
        let host = app.dialogs().unwrap();
        let cancelled = Rc::new(RefCell::new(Vec::new()));
        for flag in ["old", "new"] {
            let log = cancelled.clone();
            open_held_with(
                &mut app,
                dialog(&ctx),
                |_, _| Ok(()),
                HeldOptions { parent: None, cancel: Some(Box::new(move |_| log.borrow_mut().push(flag))) },
            )
            .unwrap();
        }
        assert!(host.with(|h| h.cancel_by_id(&ctx, craft::ui::dialog::dialog_id(KEY))));
        pump(&mut app, &ctx);
        assert_eq!(*cancelled.borrow(), ["new"]);
    }

    #[test]
    fn an_old_verdict_does_not_answer_a_replacement_opened_by_its_callback() {
        let (mut app, ctx) = app();
        let host = app.dialogs().unwrap();
        let replacement_ctx = ctx.clone();
        let id = open_held(&mut app, dialog(&ctx), move |app, _| {
            let id = open_held(app, dialog(&replacement_ctx), |_, _| Ok(())).unwrap();
            let host = app.dialogs().unwrap();
            assert!(!host.with(|h| h.confirm_by_id(&replacement_ctx, id)));
            assert_eq!(host.with(|h| h.held_state(id)), Some(HeldOk::Waiting));
            Err("old verdict".into())
        })
        .unwrap();
        assert!(!host.with(|h| h.confirm_by_id(&ctx, id)));
        pump(&mut app, &ctx);
        assert_eq!(host.with(|h| h.held_state(id)), Some(HeldOk::Waiting));
        pump(&mut app, &ctx);
        assert_eq!(host.with(|h| h.held_state(id)), Some(HeldOk::Accepted));
    }
}
