//! View → Proof Setup → Customize…: the device to simulate (the working CMYK space, or any CMYK or
//! RGB profile), Preserve CMYK Numbers or Preserve RGB Numbers, the rendering intent (used while
//! the numbers are not preserved), Simulate Paper Color and Simulate Black Ink (CMYK devices only).
//! Preview shows the proof on the canvas while the dialog is open; OK sets the proof setup and
//! turns Proof Colors on (`view.proofSetup`), and Cancel puts the earlier proof view back.
//!
//! Fields: `device` (`workingCmyk`, `cmyk:<profile>` or `rgb:<profile>`), `preserveNumbers`,
//! `intent`, `simulatePaper`, `simulateBlack` and `preview`.

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, ProfileKind, ProofTarget};
use vectorcraft_render::proof;

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets;

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
