//! View → Proof Setup → Customize…: the device to simulate (the working CMYK space, or any CMYK or
//! RGB profile), Preserve CMYK Numbers or Preserve RGB Numbers, the rendering intent (used while
//! the numbers are not preserved), Simulate Paper Color and Simulate Black Ink (CMYK devices only).
//! Preview shows the proof on the canvas while the dialog is open; OK sets the proof setup and
//! turns Proof Colors on (`view.proofSetup`), and Cancel puts the earlier proof view back. Save…
//! writes the dialog's settings to a proof setup file (`view.proofSetup.save`) and Load… reads one
//! into the dialog (`view.proofSetup.read`). Both are desktop only. Their pickers start in the
//! Proof Setups folder next to the preferences, whose setups View → Proof Setup lists below
//! Customize… ([`saved_items`]): Save… makes the folder when it may be written, and Load…'s picker
//! starts in its own default folder until the folder exists.
//!
//! Fields: `device` (`workingCmyk`, `cmyk:<profile>` or `rgb:<profile>`), `preserveNumbers`,
//! `intent`, `simulatePaper`, `simulateBlack` and `preview`.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, ProfileKind, ProofTarget};
use vectorcraft_engine::cmd::colormgmt::proof_file::{self, EXT};
use vectorcraft_render::proof;

use super::{DialogSpec, form};
use crate::menus::Item;
use crate::state::Dialog;
use crate::widgets;
use crate::{FilePick, VectorcraftApp};

/// The dialog kind of Proof Setup › Customize.
pub const KIND: &str = "proofSetup";

const CMD: &str = "view.proofSetup";

/// The proof view as the dialog found it, which Cancel puts back after a preview.
const BEFORE: &str = "__before";
/// The settings the canvas shows while Preview is on, as last sent.
const PREVIEWED: &str = "__previewed";

/// The device a proof target that is not a device (an RGB display or a color blindness type)
/// starts the dialog on.
const WORKING: &str = "workingCmyk";

const LABEL_W: f32 = 140.0;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Proof Setup").into(), body, confirm, min_width: 420.0, ..DialogSpec::FORM };

/// Open the dialog on the current proof setup. While it is open, it stays as it is, so Cancel
/// still puts back the view it found first.
pub fn open(app: &mut VectorcraftApp) {
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND) {
        return;
    }
    // A preview whose dialog another one replaced is undone before this dialog takes the view.
    restore_replaced(app);
    let v = proof::view();
    let s = &v.setup;
    let device = match &s.target {
        ProofTarget::WorkingCmyk | ProofTarget::Cmyk(_) | ProofTarget::Rgb(_) => s.target.id(),
        _ => WORKING.to_string(),
    };
    let before = snapshot(&v);
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({
            "device": device,
            "preserveNumbers": s.preserve_numbers,
            "intent": s.intent.id(),
            "simulatePaper": s.simulate_paper,
            "simulateBlack": s.simulate_black,
            "preview": true,
            BEFORE: before,
        }),
    ));
}

/// The parts of the proof view `v` that Cancel puts back, as the dialog keeps them in `BEFORE`.
fn snapshot(v: &proof::ProofView) -> Value {
    let s = &v.setup;
    json!({
        "target": s.target.id(),
        "intent": s.intent.id(),
        "simulatePaper": s.simulate_paper,
        "preserveNumbers": s.preserve_numbers,
        "simulateBlack": s.simulate_black,
        "proof": v.proof_colors,
    })
}

/// The devices to simulate, as (`device` value, name shown): the working CMYK space, then every
/// CMYK profile and every RGB profile.
fn devices() -> Vec<(String, String)> {
    let profiles = cms::profiles();
    let of = |kind: ProfileKind, prefix: &str| {
        profiles.iter().filter(move |p| p.kind == kind).map(move |p| (format!("{prefix}{}", p.name), p.name.clone())).collect::<Vec<_>>()
    };
    std::iter::once((WORKING.to_string(), tl!("Working CMYK").to_string()))
        .chain(of(ProfileKind::Cmyk, "cmyk:"))
        .chain(of(ProfileKind::Rgb, "rgb:"))
        .collect()
}

/// Whether `device` is an RGB profile.
fn rgb_device(device: &str) -> bool {
    device.starts_with("rgb:")
}

/// What OK sends to `view.proofSetup`.
fn params(d: &Dialog) -> Value {
    json!({
        "target": d.str("device"),
        "preserveNumbers": d.bool("preserveNumbers"),
        "intent": d.str("intent"),
        "simulatePaper": d.bool("simulatePaper"),
        "simulateBlack": d.bool("simulateBlack"),
        "proof": true,
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let devices = devices();
    let device = d.str("device");
    // A profile that is no longer loaded shows its name as it is.
    let shown =
        devices.iter().find(|(id, _)| *id == device).map_or_else(|| device.split_once(':').map_or(device.as_str(), |(_, n)| n), |(_, n)| n.as_str());
    let names: Vec<&str> = devices.iter().map(|(_, n)| n.as_str()).collect();
    widgets::label_row(ui, tl!("Device to Simulate:"), LABEL_W, |ui| {
        if let Some((id, _)) = widgets::dropdown_names(ui, "proof-setup-device", shown, &names, 260.0).and_then(|i| devices.get(i)) {
            d.fields.insert("device".into(), json!(id));
        }
    });
    ui.add_space(4.0);
    let rgb = rgb_device(&d.str("device"));
    let preserve = if rgb { tl!("Preserve RGB Numbers") } else { tl!("Preserve CMYK Numbers") };
    form::check(ui, d, "preserveNumbers", preserve);
    let intents: Vec<(&str, &str)> = Intent::ALL.iter().map(|i| (i.id(), i.label())).collect();
    ui.add_enabled_ui(!d.bool("preserveNumbers"), |ui| form::choice(ui, d, "intent", tl!("Rendering intent:"), (LABEL_W, 200.0), &intents));
    ui.add_space(4.0);
    // Paper and black ink are a printing press's: an RGB device has neither.
    let paper = d.bool("simulatePaper");
    if widgets::check(ui, tl!("Simulate Paper Color"), paper && !rgb, !rgb) {
        d.fields.insert("simulatePaper".into(), json!(!paper));
    }
    // Simulate Paper Color shows the press black too, so the box shows checked while it is on.
    let black = d.bool("simulateBlack");
    if widgets::check(ui, tl!("Simulate Black Ink"), (black || paper) && !rgb, !rgb && !paper) {
        d.fields.insert("simulateBlack".into(), json!(!black));
    }
    // The web has no folder to keep setups in.
    if !cfg!(target_arch = "wasm32") {
        ui.add_space(6.0);
        let (mut save_clicked, mut load_clicked) = (false, false);
        ui.horizontal(|ui| {
            save_clicked = widgets::flat_button(ui, tl!("Save…"), 64.0).clicked();
            load_clicked = widgets::flat_button(ui, tl!("Load…"), 64.0).clicked();
        });
        // Their file dialogs, shown off the UI thread, save from or load into the dialog as it is
        // then.
        let r = if save_clicked {
            crate::picks::in_app_dialog(app, d, save)
        } else if load_clicked {
            crate::picks::in_app_dialog(app, d, load)
        } else {
            Ok(())
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    ui.add_space(8.0);
    form::preview_check(ui, d);
    preview(app, d);
    false
}

/// While Preview is on, the canvas shows the dialog's settings, sent again whenever they change;
/// turned off, the earlier proof view comes back.
fn preview(app: &mut VectorcraftApp, d: &mut Dialog) {
    // Back from under another dialog that undid its preview, the dialog starts from the view the
    // canvas shows now and previews again.
    if d.fields.contains_key(PREVIEWED) && app.ui.proof_preview.is_none() {
        d.fields.remove(PREVIEWED);
    }
    if d.bool("preview") {
        let p = params(d);
        if d.fields.get(PREVIEWED) != Some(&p) {
            // A preview that starts while the canvas shows no preview of this dialog (none yet, or a
            // proof setup a command set since) takes the view the canvas shows now as the one to
            // put back.
            let v = proof::view();
            let start = match &app.ui.proof_preview {
                Some((_, previewed)) if dialog_part(&v) == dialog_part(previewed) => None,
                _ => Some(snapshot(&v)),
            };
            match app.run(CMD, p.clone()) {
                Ok(_) => {
                    if let Some(before) = start {
                        d.fields.insert(BEFORE.into(), before);
                    }
                    d.fields.insert(PREVIEWED.into(), p);
                    app.ui.proof_preview = d.fields.get(BEFORE).map(|b| (b.clone(), proof::view()));
                }
                Err(e) => app.ui.status = e,
            }
        }
    } else if d.fields.remove(PREVIEWED).is_some() {
        restore(app);
    }
}

/// Put back the proof view the dialog found before its preview. A proof setup set since the
/// preview (by `view.proofSetup`, for example) stays.
fn restore(app: &mut VectorcraftApp) {
    if let Some((before, previewed)) = app.ui.proof_preview.take()
        && dialog_part(&proof::view()) == dialog_part(&previewed)
    {
        set_view(app, &before);
    }
}

/// When another dialog has taken the place of a previewing Proof Setup dialog, put back the proof
/// view that dialog found.
pub(super) fn restore_replaced(app: &mut VectorcraftApp) {
    if !app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND) {
        restore(app);
    }
}

/// The parts of a proof view the dialog sets: the device, the intent, the three options and Proof
/// Colors.
fn dialog_part(v: &proof::ProofView) -> (&ProofTarget, Intent, bool, bool, bool, bool) {
    let s = &v.setup;
    (&s.target, s.intent, s.simulate_paper, s.preserve_numbers, s.simulate_black, v.proof_colors)
}

/// Set the proof view `before` holds and redraw the documents. The view is set directly, not
/// through `view.proofSetup`, which refuses a device profile removed since it was set.
fn set_view(app: &mut VectorcraftApp, before: &Value) {
    let text = |k: &str| before.get(k).and_then(Value::as_str).unwrap_or_default();
    let on = |k: &str| before.get(k).and_then(Value::as_bool).unwrap_or_default();
    let (Some(target), Some(intent)) = (ProofTarget::parse(text("target")), Intent::parse(text("intent"))) else { return };
    let mut v = proof::view();
    let was = v.clone();
    v.setup.target = target;
    v.setup.intent = intent;
    v.setup.simulate_paper = on("simulatePaper");
    v.setup.preserve_numbers = on("preserveNumbers");
    v.setup.simulate_black = on("simulateBlack");
    v.proof_colors = on("proof");
    if v == was {
        return;
    }
    proof::set_view(v);
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    for uid in uids {
        if let Some(d) = app.session.document_mut(uid) {
            d.revision += 1;
        }
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run(CMD, params(d))?;
    app.ui.proof_preview = None;
    app.ui.dialog = None;
    Ok(r)
}

/// Cancel: after a preview, the earlier proof view comes back.
pub(super) fn cancel(app: &mut VectorcraftApp, d: &Dialog) {
    if d.fields.contains_key(PREVIEWED) {
        restore(app);
    }
}

/// A file dialog for proof setup files, starting in the Proof Setups folder.
fn pick(app: &VectorcraftApp, name: String) -> FilePick {
    let folder = app.session.proof_setup_dir().map(|f| f.to_string_lossy().into_owned());
    FilePick { name, folder, filters: vec![FilePick::filter("Proof setups", &[EXT])] }
}

/// Save…: the dialog's settings go to the picked file, and the menu lists the saved setups again.
fn save(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    // The picker opens in the folder, made for it when it may be written. When the picker is
    // canceled, the folder it made is removed again while it is still empty: here for a picker
    // shown in line, and by [`poll`] once a picker shown off the UI thread has closed.
    let made = app
        .session
        .proof_setup_dir()
        .filter(|dir| !dir.is_dir() && vectorcraft_engine::file_access::check_write(&dir.to_string_lossy()).is_ok())
        .map(std::path::Path::to_path_buf)
        .filter(|dir| std::fs::create_dir_all(dir).is_ok());
    let pick = pick(app, format!("{}.{EXT}", tl!("Proof Setup")));
    let Some((picked, file_type)) = crate::picks::save_with_type(app, &pick) else {
        if let Some(dir) = made {
            if app.picks.is_waiting() {
                saved().made = Some(dir);
            } else {
                let _ = std::fs::remove_dir(dir);
            }
        }
        return Ok(());
    };
    let path = if std::path::Path::new(&picked).extension().is_none() { format!("{picked}.{EXT}") } else { picked.clone() };
    // The picker asked only about the name as typed: a file at the name with the extension added
    // is replaced after a question, whose OK runs Save… again.
    if path != picked && crate::picks::ask_to_replace(app, crate::picks::PickRequest::Save(pick.clone()), &picked, file_type.as_deref(), &path) {
        return Ok(());
    }
    let mut p = params(d);
    if let Some(o) = p.as_object_mut() {
        o.remove("proof");
        o.insert("path".into(), json!(path));
    }
    app.run("view.proofSetup.save", p)?;
    // The read asks for a frame when it ends, so an idle window lists the setup.
    refresh_saved(app, app.ui_ctx.as_ref());
    app.status(format!("Saved {path}"));
    Ok(())
}

/// Load…: the picked file's settings fill the dialog (Preview then shows them).
fn load(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let mut pick = pick(app, String::new());
    // Before a setup is saved there is no folder, and the picker starts in its own default folder.
    if pick.folder.as_deref().is_some_and(|f| !std::path::Path::new(f).is_dir()) {
        pick.folder = None;
    }
    // A canceled picker leaves the dialog as it is.
    let Some(path) = crate::picks::open(app, &pick) else { return Ok(()) };
    let r = app.run("view.proofSetup.read", json!({ "path": path }))?;
    let fields = [
        ("device", "target"),
        ("intent", "intent"),
        ("preserveNumbers", "preserveNumbers"),
        ("simulatePaper", "simulatePaper"),
        ("simulateBlack", "simulateBlack"),
    ];
    for (field, key) in fields {
        d.fields.insert(field.into(), r[key].clone());
    }
    Ok(())
}

// ---------- the saved setups in View › Proof Setup ----------

/// The command a saved setup's menu item runs.
pub const LOAD: &str = "view.proofSetup.load";

#[derive(Default)]
struct Saved {
    /// (name, path) of each setup, as `view.proofSetup.list` last gave them.
    setups: Vec<(&'static str, String)>,
    /// How many listings differed from the one before (the macOS menu bar follows it).
    revision: u64,
    /// A first listing was read.
    started: bool,
    /// The window was the active one at the last [`poll`].
    focused: bool,
    /// The session's count of saved setups at the last [`poll`] (`Session::proof_setup_saves`).
    saves: u64,
    /// The Proof Setups folder that Save… made for a picker shown off the UI thread, removed by
    /// [`poll`] once the picker has closed if it is still empty.
    made: Option<std::path::PathBuf>,
    /// The listing being read on another thread, as (name, path) of each setup.
    reading: Option<Receiver<Vec<(String, String)>>>,
    /// Another read was asked for while one was under way; [`poll`] starts it when that one ends.
    pending: bool,
}

/// The listing the menus show. One per process in the app; one per thread in unit tests, which run
/// in parallel threads, each with its own folder.
fn saved() -> MutexGuard<'static, Saved> {
    #[cfg(not(test))]
    let saved: &'static Mutex<Saved> = {
        static S: std::sync::OnceLock<Mutex<Saved>> = std::sync::OnceLock::new();
        S.get_or_init(Default::default)
    };
    #[cfg(test)]
    let saved: &'static Mutex<Saved> = {
        thread_local!(static S: &'static Mutex<Saved> = Box::leak(Box::default()));
        S.with(|s| *s)
    };
    saved.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read the Proof Setups folder's listing again on another thread (`view.proofSetup.list`'s
/// listing: file names only, at most 100), unless the session has no folder. While a read is under
/// way, another read starts when it ends ([`poll`]). `ctx` is asked to draw when the read ends.
pub fn refresh_saved(app: &VectorcraftApp, ctx: Option<&egui::Context>) {
    let Some(dir) = app.session.proof_setup_dir().map(std::path::Path::to_path_buf) else { return };
    let mut saved = saved();
    if saved.reading.is_some() {
        saved.pending = true;
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.cloned();
    let spawned = std::thread::Builder::new().name("vectorcraft-proof-setups".into()).spawn(move || {
        // The app's own folder, read for the person at the keyboard.
        let listed = vectorcraft_engine::file_access::unconfined(|| proof_file::listed(&dir));
        let setups = listed
            .ok()
            .flatten()
            .and_then(|r| r["setups"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["path"].as_str()?.to_string())))
            .collect();
        if tx.send(setups).is_ok()
            && let Some(ctx) = ctx
        {
            ctx.request_repaint();
        }
    });
    if spawned.is_ok() {
        saved.reading = Some(rx);
    }
}

/// Take in the read under way: when it has ended, or with `wait` once it ends. A name past the
/// menu's name limit is left out.
fn take(wait: bool) {
    let mut saved = saved();
    let Some(reading) = &saved.reading else { return };
    let listed = if wait { reading.recv().map_err(|_| TryRecvError::Disconnected) } else { reading.try_recv() };
    match listed {
        Ok(listed) => {
            let setups: Vec<(&'static str, String)> =
                listed.into_iter().filter_map(|(name, path)| Some((crate::scripts_menu::intern(&name)?, path))).collect();
            if saved.setups != setups {
                saved.setups = setups;
                saved.revision += 1;
            }
            saved.reading = None;
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => saved.reading = None,
    }
}

/// Wait for the read under way and take it in.
#[cfg(test)]
pub(crate) fn wait() {
    take(true);
}

/// Once per iteration: list the folder on the first one, after `view.proofSetup.save` wrote a
/// setup (from the dialog, a control client or the command palette), and when the window becomes
/// the active one, so a setup copied into the folder is listed without a restart; take in a read
/// that ended, and then start a read asked for while it was under way. Once Save…'s picker shown
/// off the UI thread has closed, the folder it made is removed if it is still empty.
pub fn poll(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let focused = ctx.input(|i| i.focused);
    let saves = app.session.proof_setup_saves();
    let start = {
        let mut saved = saved();
        if !app.picks.is_waiting()
            && let Some(dir) = saved.made.take()
        {
            // A folder that a setup was saved in is not empty, and stays.
            let _ = std::fs::remove_dir(dir);
        }
        let start = !saved.started || (focused && !saved.focused) || saves != saved.saves;
        (saved.started, saved.focused, saved.saves) = (true, focused, saves);
        start
    };
    if start {
        refresh_saved(app, Some(ctx));
    }
    take(false);
    let again = {
        let mut saved = saved();
        let again = saved.pending && saved.reading.is_none();
        if again {
            saved.pending = false;
        }
        again
    };
    if again {
        refresh_saved(app, Some(ctx));
    }
}

/// Changes whenever the menus' listing changes.
pub fn saved_revision() -> u64 {
    saved().revision
}

/// View › Proof Setup's saved setups, each by its file's name, loading it with Proof Colors on.
pub fn saved_items() -> Vec<Item> {
    saved().setups.iter().map(|&(name, ref path)| Item::Cmd(name, LOAD, json!({ "path": path }))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// Proof views are process-wide: these tests take turns, and each leaves the default view.
    static VIEW: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct DefaultView<'a>(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>);

    impl DefaultView<'_> {
        fn new() -> Self {
            let g = VIEW.lock().unwrap_or_else(|e| e.into_inner());
            proof::set_view(Default::default());
            DefaultView(g)
        }
    }

    impl Drop for DefaultView<'_> {
        fn drop(&mut self) {
            proof::set_view(Default::default());
        }
    }

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn app() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        (app, ctx)
    }

    /// View › Proof Setup › Customize… opens the dialog on the current proof setup; OK sets the
    /// device, Preserve Numbers, the intent and Simulate Black Ink, and turns Proof Colors on.
    #[test]
    fn customize_sets_the_proof_setup_and_turns_proof_colors_on() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.path == ["View", "Proof Setup"] && e.label == "Customize…").unwrap();
        app.run(item.command.as_deref().unwrap(), item.params.clone()).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, KIND);
        assert_eq!((d.str("device"), d.str("intent"), d.bool("preserveNumbers")), (WORKING.to_string(), "relative".to_string(), false));
        frame(&mut app, &ctx);
        let device = format!("cmyk:{}", cms::DEVICE_CMYK);
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in [("device", json!(device)), ("intent", json!("perceptual")), ("simulateBlack", json!(true)), ("preview", json!(false))] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let v = proof::view();
        assert!(v.proof_colors);
        assert_eq!(v.setup.target, ProofTarget::Cmyk(cms::DEVICE_CMYK.into()));
        assert_eq!(v.setup.intent, Intent::Perceptual);
        assert!(v.setup.simulate_black && !v.setup.preserve_numbers && !v.setup.simulate_paper);
        // Opened again, it shows those settings, and the menu checks Customize….
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("device"), d.str("intent"), d.bool("simulateBlack")), (device, "perceptual".to_string(), true));
        assert_eq!(crate::menus::checked(&app, "ui.proofSetupDialog", &json!({})), Some(true));
        super::super::cancel(&mut app);
    }

    /// Preview shows the dialog's settings on the canvas at once, follows their changes, and
    /// Cancel (or turning Preview off) puts the earlier proof view back.
    #[test]
    fn preview_shows_the_settings_and_cancel_puts_the_view_back() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        app.run("view.proofSetup", json!({"target": "protanopia"})).unwrap();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("device"), WORKING, "a color blindness proof starts on the working CMYK");
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("device".into(), json!(rgb));
        d.fields.insert("preserveNumbers".into(), json!(true));
        frame(&mut app, &ctx);
        let v = proof::view();
        assert!(v.proof_colors, "the preview turns Proof Colors on");
        assert_eq!(v.setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        assert!(v.setup.preserve_numbers);
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
        frame(&mut app, &ctx);
        assert_eq!(proof::view(), before, "Preview off puts the view back");
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(proof::view(), before, "Cancel puts the view back");
    }

    /// Cancel and Preview off put back a proof view whose device profile was removed after it was
    /// set, and redraw the documents.
    #[test]
    fn cancel_puts_back_a_view_whose_profile_was_removed() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let name = "Proof Setup Removed RGB";
        cms::register_icc(&cms::icc_bytes(cms::DISPLAY_P3).unwrap(), Some(name.into())).unwrap();
        app.run("view.proofSetup", json!({"target": format!("rgb:{name}"), "proof": true})).unwrap();
        app.run("color.removeProfile", json!({"name": name})).unwrap();
        let before = proof::view();
        assert_eq!(before.setup.target, ProofTarget::Rgb(name.into()), "removing the profile leaves it the device");
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(WORKING));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::WorkingCmyk, "the preview shows the working CMYK");
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
        frame(&mut app, &ctx);
        assert_eq!(proof::view(), before, "Preview off puts the view back");
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        frame(&mut app, &ctx);
        let revision = app.session.doc().unwrap().revision;
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(proof::view(), before, "Cancel puts the view back");
        assert!(app.session.doc().unwrap().revision > revision, "the document redraws");
    }

    /// Run again while the dialog is open, Proof Setup keeps the dialog as it is, so Cancel still
    /// puts back the view it found the first time.
    #[test]
    fn running_proof_setup_again_keeps_the_open_dialog() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("device"), rgb, "the open dialog stays as it is");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), before, "Cancel puts back the view the first run found");
    }

    /// When another dialog (Saturate) takes the place of a previewing Proof Setup dialog, the proof
    /// view the dialog found comes back, and a later Proof Setup starts from that view.
    #[test]
    fn another_dialog_in_its_place_puts_the_view_back() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows");
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app.run("ui.saturateDialog", json!({})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, super::super::saturate::KIND);
        assert_eq!(proof::view(), before, "the preview is undone");
        super::super::cancel(&mut app);
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("device"), WORKING, "the next Proof Setup starts from the view it found");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), before);
    }

    /// Opened again before a frame has drawn the dialog that replaced a previewing Proof Setup
    /// dialog, Proof Setup starts from the view the first dialog found, and Cancel keeps it.
    #[test]
    fn reopened_before_a_frame_starts_from_the_view_it_found() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows");
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app.run("ui.saturateDialog", json!({})).unwrap();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(proof::view(), before, "the preview is undone");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("device"), WORKING);
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), before, "Cancel keeps the view the first dialog found");
    }

    /// A proof view set after another dialog replaced a previewing Proof Setup dialog, before a
    /// frame, stays: the preview is not put back over it.
    #[test]
    fn a_view_set_after_the_replacement_stays() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app.run("ui.saturateDialog", json!({})).unwrap();
        app.run("view.proofSetup", json!({"target": "protanopia", "proof": true})).unwrap();
        let set = proof::view();
        frame(&mut app, &ctx);
        assert_eq!(proof::view(), set, "the view set after the replacement stays");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), set);
    }

    /// Overprint Preview turned on during the preview stays on when another dialog replaces the
    /// dialog, and the proof setup the dialog found comes back.
    #[test]
    fn overprint_set_during_the_preview_stays_after_the_replacement() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        app.run("view.overprintPreview", json!({"on": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app.run("ui.saturateDialog", json!({})).unwrap();
        frame(&mut app, &ctx);
        let v = proof::view();
        assert!(v.overprint, "Overprint Preview stays on");
        assert_eq!((v.setup, v.proof_colors), (before.setup, before.proof_colors), "the proof setup comes back");
        super::super::cancel(&mut app);
    }

    /// Back from under a dialog opened over it (here Saturate, as a file-replacement question is),
    /// which undid the preview, the dialog previews its settings again, and Cancel puts back the
    /// view it found.
    #[test]
    fn back_from_under_another_dialog_it_previews_again() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        let under = app.ui.dialog.clone().unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app.run("ui.saturateDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert(super::super::PARENT.into(), serde_json::to_value(under).unwrap());
        frame(&mut app, &ctx);
        assert_eq!(proof::view(), before, "the preview is undone while the other dialog shows");
        super::super::cancel(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, KIND, "Proof Setup comes back");
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows again");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), before, "Cancel puts back the view it found");
    }

    /// A proof setup set by a command while the dialog previews stays when Preview is turned off
    /// and when the dialog is canceled.
    #[test]
    fn a_view_set_while_the_dialog_previews_stays() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        app.run("view.proofSetup", json!({"target": "protanopia", "proof": true})).unwrap();
        let set = proof::view();
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
        frame(&mut app, &ctx);
        assert_eq!(proof::view(), set, "Preview off keeps the view the command set");
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows again");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), set, "Cancel puts back the view the command set");
    }

    /// After a command sets a proof setup while the dialog previews, a changed setting previews
    /// again, and Cancel puts back the proof setup the command set.
    #[test]
    fn a_changed_setting_after_a_command_keeps_the_commands_view_for_cancel() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        app.ui.dialog.as_mut().unwrap().fields.insert("device".into(), json!(rgb));
        frame(&mut app, &ctx);
        app.run("view.proofSetup", json!({"target": "protanopia", "proof": true})).unwrap();
        let set = proof::view();
        app.ui.dialog.as_mut().unwrap().fields.insert("intent".into(), json!("perceptual"));
        frame(&mut app, &ctx);
        assert_eq!(proof::view().setup.intent, Intent::Perceptual, "the changed setting previews");
        super::super::cancel(&mut app);
        assert_eq!(proof::view(), set, "Cancel puts back the view the command set");
    }

    /// A device the dialog can't use (a profile that is not loaded) is refused, and the dialog
    /// stays open with the proof view as it was.
    #[test]
    fn ok_with_an_unknown_device_keeps_the_dialog_open() {
        let _v = DefaultView::new();
        let (mut app, _) = app();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("device".into(), json!("cmyk:Nope"));
        d.fields.insert("preview".into(), json!(false));
        assert!(super::super::confirm(&mut app).unwrap_err().contains("unknown CMYK profile"));
        assert!(app.ui.dialog.is_some());
        assert_eq!(proof::view(), Default::default());
    }

    /// Save… writes the dialog's settings to the picked file in the Proof Setups folder, which View ›
    /// Proof Setup then lists; Load… puts a file's settings back in the dialog, and the menu item
    /// applies the setup with Proof Colors on.
    #[test]
    fn save_and_load_keep_a_custom_proof_setup() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog");
        let folder = dir.join("Proof Setups");
        app.session.attach_proof_setups(folder.clone());
        refresh_saved(&app, None);
        wait();
        assert!(saved_items().is_empty());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let rgb = format!("rgb:{}", cms::DISPLAY_P3);
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in [("device", json!(rgb)), ("intent", json!("saturation")), ("preserveNumbers", json!(true)), ("preview", json!(false))] {
            d.fields.insert(k.into(), v);
        }
        let picked = std::rc::Rc::new(std::cell::RefCell::new(None::<FilePick>));
        let seen = picked.clone();
        let path = folder.join("Screen").to_string_lossy().to_string();
        let answer = path.clone();
        app.services.pick_save = Some(Box::new(move |p: &FilePick| {
            *seen.borrow_mut() = Some(p.clone());
            Some(answer.clone())
        }));
        let mut d = app.ui.dialog.take().unwrap();
        save(&mut app, &mut d).unwrap();
        let pick = picked.borrow().clone().unwrap();
        assert_eq!(pick.folder.as_deref(), Some(folder.to_string_lossy().as_ref()), "the picker starts in the Proof Setups folder");
        let file = format!("{path}.{EXT}");
        assert!(std::path::Path::new(&file).is_file(), "the extension is added");
        assert_eq!(proof::view(), Default::default(), "saving leaves the proof view alone");
        wait();
        let items = crate::menus::menu_entries(&app).into_iter().filter(|e| e.path == ["View", "Proof Setup"] && e.command.as_deref() == Some(LOAD));
        let items: Vec<_> = items.map(|e| (e.label.to_string(), e.params.clone())).collect();
        assert_eq!(items, [("Screen".to_string(), json!({ "path": file }))]);
        // A control client that may not read the folder is not shown the saved setup.
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let write_only = vectorcraft_engine::file_access::AutomationRoots::new(&[], &[&dir.join("out")]).unwrap();
        let listed = vectorcraft_engine::file_access::confine(write_only.as_ref(), || crate::menus::menu_entries(&app));
        assert!(!listed.iter().any(|e| e.command.as_deref() == Some(LOAD)), "a setup outside the read roots is left out");
        // A setup the command saves, outside the dialog, is listed at the next iteration.
        poll(&mut app, &ctx);
        wait();
        app.run("view.proofSetup.save", json!({ "name": "Direct" })).unwrap();
        poll(&mut app, &ctx);
        wait();
        assert_eq!(saved_items().len(), 2, "the menu lists the new setup");
        std::fs::remove_file(folder.join(format!("Direct.{EXT}"))).unwrap();
        refresh_saved(&app, None);
        wait();
        // Load… puts the file's settings back in the dialog.
        for (k, v) in [("device", json!(WORKING)), ("intent", json!("perceptual")), ("preserveNumbers", json!(false))] {
            d.fields.insert(k.into(), v);
        }
        let answer = file.clone();
        app.services.pick_open = Some(Box::new(move |_: &FilePick| Some(answer.clone())));
        load(&mut app, &mut d).unwrap();
        assert_eq!((d.str("device"), d.str("intent"), d.bool("preserveNumbers")), (rgb.clone(), "saturation".to_string(), true));
        assert_eq!(proof::view(), Default::default(), "loading into the dialog leaves the proof view alone");
        app.ui.dialog = Some(d);
        frame(&mut app, &ctx);
        super::super::cancel(&mut app);
        // The menu item applies the saved setup and turns Proof Colors on.
        app.run(LOAD, json!({ "path": file })).unwrap();
        let v = proof::view();
        assert!(v.proof_colors && v.setup.preserve_numbers);
        assert_eq!((v.setup.target, v.setup.intent), (ProofTarget::Rgb(cms::DISPLAY_P3.into()), Intent::Saturation));
    }

    /// Save… whose picker is canceled saves nothing and leaves no Proof Setups folder behind.
    #[test]
    fn a_canceled_save_leaves_no_folder() {
        let _v = DefaultView::new();
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-cancel");
        let folder = dir.join("Proof Setups");
        app.session.attach_proof_setups(folder.clone());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        let opened = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = opened.clone();
        let f = folder.clone();
        app.services.pick_save = Some(Box::new(move |_: &FilePick| {
            seen.set(f.is_dir());
            None
        }));
        save(&mut app, &mut d).unwrap();
        assert!(opened.get(), "the picker opens in the folder");
        assert!(!folder.exists(), "the canceled save removes the folder it made");
        // A picker shown off the UI thread is still open when Save… returns: its folder stays.
        let (shown, _) = crate::tests::picks::off_the_ui_thread(&mut app.services);
        app.ui.dialog = Some(d.clone());
        crate::picks::in_app_dialog(&mut app, &mut d, save).unwrap();
        assert_eq!(shown.requests().len(), 1, "the picker is shown off the UI thread");
        poll(&mut app, &ctx);
        assert!(folder.is_dir(), "the open picker's folder stays");
        // Canceled, the picker leaves no folder once the next iteration runs.
        shown.answer(&[]);
        crate::picks::poll(&mut app, &ctx);
        poll(&mut app, &ctx);
        assert!(!folder.exists(), "the canceled picker's folder is removed");
        // A picker that answers a file there saves it, and the folder stays.
        crate::picks::in_app_dialog(&mut app, &mut d, save).unwrap();
        let file = folder.join(format!("Kept.{EXT}"));
        shown.answer(&[&file.to_string_lossy()]);
        crate::picks::poll(&mut app, &ctx);
        poll(&mut app, &ctx);
        assert!(file.is_file(), "the picked file is saved");
    }

    /// The listing Save… starts asks for a frame when it ends, so an idle window takes it in.
    #[test]
    fn the_listing_after_a_save_asks_for_a_frame() {
        let _v = DefaultView::new();
        let (mut app, _) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-frame");
        let folder = dir.join("Proof Setups");
        app.session.attach_proof_setups(folder.clone());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        d.fields.insert("preview".into(), json!(false));
        let path = folder.join("Screen").to_string_lossy().to_string();
        app.services.pick_save = Some(Box::new(move |_: &FilePick| Some(path.clone())));
        let ctx = egui::Context::default();
        let (tx, frames) = std::sync::mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        app.ui_ctx = Some(ctx);
        save(&mut app, &mut d).unwrap();
        // With the app's copy of the context dropped, only the listing's thread can still ask for
        // a frame; the channel closes without one when that thread ends without asking.
        app.ui_ctx = None;
        assert_eq!(frames.recv(), Ok(()), "the finished listing asks for a frame");
        wait();
        assert_eq!(saved_items().len(), 1);
    }

    /// A saved setup whose name ends in an ellipsis still loads when its menu item is chosen.
    #[test]
    fn a_saved_setup_named_with_an_ellipsis_loads() {
        let p = json!({ "path": "Press….vcproof" });
        assert_eq!(crate::menus::click_target("Press…", LOAD, &p), (LOAD.to_string(), p.clone()));
    }

    /// A saved setup keeps its file's name in every UI language, also a name the catalogs translate.
    #[test]
    fn a_saved_setup_keeps_its_name_in_other_languages() {
        let (app, _) = app();
        crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
        let shown = crate::menus::display_label(&app, LOAD, "Save…");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(shown, "Save…");
    }

    /// Load…'s picker starts in the Proof Setups folder once it exists, and in its own default
    /// folder before; Load… does not make the folder.
    #[test]
    fn load_starts_in_the_folder_once_it_exists() {
        let _v = DefaultView::new();
        let (mut app, _) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-load-folder");
        let folder = dir.join("Proof Setups");
        app.session.attach_proof_setups(folder.clone());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        let picked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = picked.clone();
        app.services.pick_open = Some(Box::new(move |p: &FilePick| {
            seen.borrow_mut().push(p.folder.clone());
            None
        }));
        load(&mut app, &mut d).unwrap();
        assert!(!folder.exists(), "Load… does not make the folder");
        std::fs::create_dir_all(&folder).unwrap();
        load(&mut app, &mut d).unwrap();
        assert_eq!(*picked.borrow(), [None, Some(folder.to_string_lossy().into_owned())]);
    }

    /// Load… of a file whose device is not loaded leaves the dialog's settings as they were.
    #[test]
    fn load_of_an_unusable_file_keeps_the_dialog() {
        let _v = DefaultView::new();
        let (mut app, _) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-bad");
        let file = dir.join("gone.vcproof").to_string_lossy().to_string();
        let text = r#"{"format": "vectorcraft-proof-setup", "version": 1, "target": "cmyk:Not Loaded", "intent": "relative"}"#;
        std::fs::write(&file, text).unwrap();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        let before = d.fields.clone();
        app.services.pick_open = Some(Box::new(move |_: &FilePick| Some(file.clone())));
        let e = load(&mut app, &mut d).unwrap_err();
        assert!(e.contains("`Not Loaded`, which is not loaded"), "{e}");
        assert_eq!(d.fields, before);
    }

    /// The device list holds the working CMYK space and every CMYK and RGB profile, each a target
    /// `view.proofSetup` accepts.
    #[test]
    fn devices_list_every_cmyk_and_rgb_profile() {
        let list = devices();
        assert_eq!(list[0].0, WORKING);
        for p in cms::profiles().iter().filter(|p| p.kind != ProfileKind::Gray) {
            assert!(list.iter().any(|(_, n)| *n == p.name), "{} is listed", p.name);
        }
        for (id, _) in &list {
            let t = ProofTarget::parse(id).unwrap();
            assert_eq!(&t.id(), id);
            assert!(rgb_device(id) == matches!(t, ProofTarget::Rgb(_)));
        }
    }
}
