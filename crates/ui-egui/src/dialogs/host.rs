//! The app's access to its window's dialog host, and the once-per-frame pump that serves every dialog
//! module and every held dialog.

use craft_ui::dialog::{DialogId, DialogResult, HoldsOk};
use craft_ui::shell::{WindowDialogs, WindowKey};
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
    // PLAN: assumed `WindowKey::main()` names the main window's key
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
    pub cancel: Option<Box<dyn FnOnce(&mut VectorcraftApp)>>,
}

/// Runs one held dialog's pass; `Some(cancelled)` once the dialog is gone.
type Drain = Box<dyn FnMut(&mut VectorcraftApp) -> Option<bool>>;

struct Record {
    drain: Drain,
    cancel: Option<Box<dyn FnOnce(&mut VectorcraftApp)>>,
}

/// The held dialogs of the app: one record each, served by [`pump`].
#[derive(Default)]
pub(crate) struct Held(Vec<Record>);

fn drain<D: HoldsOk>(id: DialogId, mut ok: impl FnMut(&mut VectorcraftApp, &D::Answer) -> Result<(), String> + 'static) -> Drain {
    Box::new(move |app| {
        // Without a window nothing can answer: the dialog counts as cancelled so its rollback runs.
        let Some(host) = app.dialogs() else { return Some(true) };
        if let Some(answer) = host.with(|h| h.get_mut_by_id::<D>(id).and_then(D::take_ok)) {
            // The host's lock is released while `ok` runs: it may open dialogs.
            let verdict = ok(app, &answer).map_err(Into::into);
            host.with(|h| {
                if let Some(d) = h.get_mut_by_id::<D>(id) {
                    d.answer_ok(verdict);
                }
            });
        }
        host.with(|h| h.take_by_id::<D>(id)).map(|r| matches!(r, DialogResult::Cancel))
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
    let id = host.with(|h| match options.parent {
        Some(parent) => h.open_child(dialog, parent),
        None => h.open(dialog),
    });
    app.held.0.push(Record { drain: drain::<D>(id, ok), cancel: options.cancel });
    Some(id)
}

/// The dialog on top of the stack.
fn top_dialog(app: &VectorcraftApp) -> Option<DialogId> {
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
// PLAN: every module pump below is assumed `pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context)`;
// `file_asks`' crate path and `missing_fonts::open_next` are assumed too.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    sync(app);
    pumps!(app, ctx;
        crate::file_asks::pump,
        crate::io::save::pump,
        super::profile_mismatch::pump,
        super::recovery::pump,
        super::missing_links::pump,
        super::find_links::pump,
        super::missing_fonts::open_next,
        super::text_import::pump,
        super::psd_import::pump,
        super::dxf_import::pump,
        super::import_pdf::pump,
        super::place::pump,
        super::new_document::pump,
        super::document_setup::pump,
        super::package::pump,
        super::graphic_style_options::pump,
        super::layer_options::pump,
        super::layers_panel_options::pump,
        super::artboard_options::pump,
        super::rearrange_artboards::pump,
        super::rename_artboards::pump,
        super::navigator_panel_options::pump,
        super::variables::pump,
        super::effect::pump,
        super::halftone::pump,
        super::raster_effects::pump,
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
        super::tool_options::pump,
        super::dimension::pump,
        super::option_form::pump,
        super::live_paint_gaps::pump,
        super::graph_type::pump,
        super::graph_data::pump,
        super::graph_design::pump,
        super::perspective_grid::pump,
        super::perspective_presets::pump,
        super::perspective_options::pump,
        super::perspective_plane::pump,
        super::slices::pump,
        super::blend_options::pump,
        super::corners::pump,
        super::expand::pump,
        super::saved_names::pump,
        super::indexed_color::pump,
        super::tracing_presets::pump,
        super::plugin::pump,
        crate::addons::dialogs::pump,
        super::char_style_options::pump,
        super::para_style_options::pump,
        super::new_text_style::pump,
        super::warichu::pump,
        super::tate_chu_yoko::pump,
        super::hyphenation::pump,
        super::justification::pump,
        super::smart_punctuation::pump,
        super::kinsoku_sets::pump,
        super::composite_fonts::pump,
        super::bullets_numbering::pump,
        super::find_replace::pump,
        super::check_spelling::pump,
        super::custom_dictionary::pump,
        super::confirm::pump_missing_font,
        super::actions::pump,
        super::script_run::pump,
        super::color_picker::pump,
        super::proof_setup::pump,
        super::command::pump,
    );
    // A drain's `ok` may open held dialogs: those join the records that stay.
    let mut kept = Vec::new();
    for mut record in std::mem::take(&mut app.held.0) {
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
    kept.append(&mut app.held.0);
    app.held.0 = kept;
}
