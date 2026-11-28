//! Object → Vector Halftone: the dot shape, the screen frequency and angle, mono (in one colour)
//! or CMYK screens, Invert, Clip to Art and Keep Original, previewed live on the canvas; OK keeps
//! the preview as one undo step (`object.vectorHalftone`).
//!
//! Fields: `shape`, `frequency`, `angle`, `mode`, `color`, `invert`, `clip`, `keepOriginal` (the
//! command's parameters). The live effect's dialog shows the same rows but Keep Original
//! ([`screen_params`]).

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::value_drag::SliderScale;
use serde_json::{Value, json};

use super::form::{fields_of, json_of};
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;

/// The command Vector Halftone runs, and the key of its dialog.
pub(crate) const CMD: &str = "object.vectorHalftone";

/// The open Object › Vector Halftone dialog and its live preview.
pub(crate) struct Open {
    /// The dialog's id on the host.
    pub id: DialogId,
    /// The drawing whose art the dialog previews and changes: its uid and revert count.
    document: (u64, u64),
    /// The temporary dots the dialog shows while it previews.
    pub preview: LivePreview,
}

/// The dialog's fields as it opens: the command's defaults.
pub fn fields() -> Value {
    json!({"shape": "circle", "frequency": 20, "angle": 45, "mode": "mono", "color": "#000000", "invert": false, "clip": true, "keepOriginal": false})
}

/// A choice's options from `(id, label)` pairs, the labels translated.
pub(super) fn options<I: AsRef<str>>(list: &[(I, &str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| (id.as_ref().to_string(), Label::from(tl!(label).to_owned()))).collect()
}

/// The rows Object › Vector Halftone and the live effect (Effect › Color Effects › Vector
/// Halftone…, `live`) share: the dot shape, the screen frequency and angle, mono or CMYK screens,
/// the dot colour (or, with the effect's `fromArt`, the art's), Invert and Clip to Art. Object ›
/// Vector Halftone adds Keep Original. The Color row depends on two values, so the rows are built
/// from `fields`.
pub(crate) fn screen_params(live: bool, fields: &DialogFields) -> Vec<Param> {
    let shapes = [("circle", "Circle"), ("ellipse", "Ellipse"), ("square", "Square"), ("diamond", "Diamond"), ("line", "Line")];
    let mut rows = vec![
        Param::new("shape", tl!("Shape"), ParamKind::choice(options(&shapes), Some("circle".into()))),
        Param::new("frequency", tl!("Frequency"), ParamKind::range(1.0, 150.0, 20.0, SliderScale::Linear)).unit(UnitKind::LinesPerInch),
        Param::new("angle", tl!("Angle"), ParamKind::range(-90.0, 90.0, 45.0, SliderScale::Linear)).unit(UnitKind::Degrees),
        Param::new("mode", tl!("Screens"), ParamKind::choice(options(&[("mono", "Mono"), ("cmyk", "CMYK")]), Some("mono".into()))),
    ];
    if live {
        rows.push(Param::new("fromArt", tl!("Dot Color from Art"), ParamKind::bool(false)).visible_when("mode", "mono"));
    }
    let from_art = live && fields.bool("fromArt").unwrap_or(false);
    if fields.text("mode") != Some("cmyk") && !from_art {
        rows.push(Param::new("color", tl!("Color"), ParamKind::color(Some([0.0; 3]))));
    }
    rows.push(Param::new("invert", tl!("Invert"), ParamKind::bool(false)));
    rows.push(Param::new("clip", tl!("Clip to Art"), ParamKind::bool(true)));
    if !live {
        rows.push(Param::new("keepOriginal", tl!("Keep Original"), ParamKind::bool(false)));
    }
    rows
}

/// Opens Object › Vector Halftone, previewing from its first frame.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let document = app.session.active().map(|doc| (doc.uid, doc.reverts)).ok_or("no document")?;
    let defaults = fields();
    let rows = screen_params(false, &DialogFields::new());
    let initial = fields_of(&rows, defaults.as_object().unwrap_or(&Default::default()));
    let dialog = HostedParamDialog::command(&ctx, CMD, initial, |fields| {
        let rows = screen_params(false, fields);
        ParamDialog::command(CMD, tl!("Vector Halftone"), rows).previews(true).values(Bind::bound(fields))
    });
    let ok = |app: &mut VectorcraftApp, result: &ParamResult| {
        let open = app.effect_dialogs.halftone.as_ref().ok_or("the dialog is closed")?;
        if !open.in_source(app) {
            let why = if open.reverted(app) {
                tl!("The document was reverted. Reopen this dialog to continue.")
            } else {
                tl!("Return to the original drawing to apply Vector Halftone")
            };
            return Err(why.into());
        }
        let params = Value::Object(json_of(&screen_params(false, &result.fields), &result.fields));
        let mut open = app.effect_dialogs.halftone.take().ok_or("the dialog is closed")?;
        let done = std::mem::take(&mut open.preview).commit(app, CMD, params);
        if done.is_err() {
            app.effect_dialogs.halftone = Some(open);
        }
        done.map(|_| ())
    };
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(|app| {
        if let Some(open) = app.effect_dialogs.halftone.take()
            && open.in_source(app)
            && owns_preview(app)
        {
            open.preview.cancel(app);
        }
    });
    let id = open_held_with(app, dialog, ok, HeldOptions { cancel: Some(cancel), ..Default::default() }).ok_or("no window")?;
    app.effect_dialogs.halftone = Some(Open { id, document, preview: LivePreview::default() });
    Ok(Value::Null)
}

/// Runs or ends the open dialog's preview for this frame.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.effect_dialogs.halftone.take() else { return };
    if !open.in_source(app) {
        open.preview = LivePreview::default();
        app.effect_dialogs.halftone = Some(open);
        return;
    }
    let shown = app.dialogs().and_then(|host| {
        host.with(|h| {
            h.get_by_id::<HostedParamDialog>(open.id).map(|d| {
                let params = json_of(&screen_params(false, d.fields()), d.fields());
                (d.is_previewing(), Value::Object(params))
            })
        })
    });
    if let Some((on, params)) = shown {
        // `run` reports a failed preview on the status line itself.
        let _ = open.preview.run(app, on, "Vector Halftone", CMD, &params);
    }
    app.effect_dialogs.halftone = Some(open);
}

/// Whether the open Object → Vector Halftone dialog is showing its temporary dot group.
pub(crate) fn previewing(app: &VectorcraftApp) -> bool {
    let Some(open) = app.effect_dialogs.halftone.as_ref() else { return false };
    open.in_source(app)
        && owns_preview(app)
        && app.dialogs().is_some_and(|host| host.with(|h| h.get_by_id::<HostedParamDialog>(open.id).is_some_and(HostedParamDialog::is_previewing)))
}

impl Open {
    fn in_source(&self, app: &VectorcraftApp) -> bool {
        app.session.active().is_some_and(|doc| (doc.uid, doc.reverts) == self.document)
    }

    /// Whether the active drawing is the opening one, reverted since the dialog opened.
    fn reverted(&self, app: &VectorcraftApp) -> bool {
        let (uid, reverts) = self.document;
        app.session.active().is_some_and(|doc| doc.uid == uid && doc.reverts != reverts)
    }
}

fn owns_preview(app: &VectorcraftApp) -> bool {
    app.session.active().and_then(|st| st.interaction.as_ref()?.preview.as_ref()).is_some_and(|(cmd, _)| cmd == CMD)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::tests::panel_harness::Panel;

    const REVERTED: &str = "The document was reverted. Reopen this dialog to continue.";

    /// A panel over a saved drawing of one black ellipse, with Vector Halftone open on it. The
    /// directory holds the saved file.
    fn open_on_a_saved_drawing(tag: &str) -> (Panel, vectorcraft_testkit::TempDir) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 100})).unwrap();
        app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
        let dir = vectorcraft_testkit::TempDir::new(tag);
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        let mut panel = Panel::new(app, crate::tests::picks::dialogs);
        open_dialog(&mut panel);
        (panel, dir)
    }

    fn open_dialog(panel: &mut Panel) {
        crate::menus::invoke(&mut panel.app(), CMD, json!({}));
        panel.frame();
        panel.frame();
        assert!(panel.app().effect_dialogs.halftone.is_some(), "Vector Halftone is open");
    }

    fn id(panel: &Panel) -> DialogId {
        panel.app().effect_dialogs.halftone.as_ref().map(|o| o.id).expect("Vector Halftone is open")
    }

    /// Sets a field as a control client would, then runs two frames.
    fn set(panel: &mut Panel, field: &str, value: Value) {
        let (id, ctx) = (id(panel), panel.ctx().clone());
        let host = panel.app().dialogs().unwrap();
        host.with(|h| h.set_json_field(&ctx, id, field, value)).unwrap();
        panel.frame();
        panel.frame();
    }

    /// Reverts the drawing to its saved copy and selects the restored ellipse again; returns the
    /// restored document and its undo depth.
    fn revert_and_reselect(panel: &mut Panel) -> (std::sync::Arc<vectorcraft_doc::Document>, usize) {
        let mut app = panel.app();
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        (st.doc.clone(), st.history.undo.len())
    }

    fn doc(panel: &Panel) -> std::sync::Arc<vectorcraft_doc::Document> {
        panel.app().session.doc().unwrap().doc.clone()
    }

    fn undo_steps(panel: &Panel) -> usize {
        panel.app().session.doc().unwrap().history.undo.len()
    }

    /// Asserts that the held OK was refused for the revert and the dialog is still open.
    fn assert_refused_for_the_revert(panel: &Panel) {
        let id = id(panel);
        let held = panel.app().dialogs().unwrap().with(|h| h.held_state(id));
        assert!(
            matches!(&held, Some(craft::ui::dialog::HeldOk::Refused(reason)) if reason.as_str() == REVERTED),
            "OK is refused for the revert, the held state is {held:?}"
        );
        assert!(panel.app().any_dialog_open(), "the dialog stays open with the refusal");
    }

    /// After File > Revert the draft does not preview on the restored drawing, OK is refused and
    /// changes nothing, and Cancel changes nothing; reopened, the dialog applies again.
    #[test]
    fn vector_halftone_refuses_its_draft_after_file_revert() {
        let (mut panel, _dir) = open_on_a_saved_drawing("halftone-revert");
        set(&mut panel, "frequency", json!(25));
        assert!(previewing(&panel.app()), "the draft previews before the revert");

        let (restored, undo) = revert_and_reselect(&mut panel);
        panel.frame();
        panel.frame();
        assert_eq!(doc(&panel), restored, "Preview must leave the restored drawing unchanged");
        set(&mut panel, "frequency", json!(30));
        assert_eq!(doc(&panel), restored, "Preview must leave the restored drawing unchanged after an edit");
        assert!(!panel.app().session.in_interaction(), "no preview runs on the restored drawing");

        panel.click("OK");
        panel.frame();
        assert_eq!(doc(&panel), restored, "OK must leave the restored drawing unchanged");
        assert_eq!(undo_steps(&panel), undo, "OK must record no undo step");
        assert!(panel.app().effect_dialogs.halftone.is_some(), "the dialog stays open");
        assert_refused_for_the_revert(&panel);

        panel.click("Cancel");
        panel.frame();
        assert!(panel.app().effect_dialogs.halftone.is_none(), "Cancel closes the dialog");
        assert_eq!(doc(&panel), restored, "Cancel must leave the restored drawing unchanged");

        open_dialog(&mut panel);
        assert!(previewing(&panel.app()), "the reopened dialog previews on the restored drawing");
        panel.click("OK");
        panel.frame();
        assert!(panel.app().effect_dialogs.halftone.is_none(), "the reopened dialog applies");
        assert_ne!(doc(&panel), restored, "the reopened dialog changes the restored drawing");
        assert_eq!(undo_steps(&panel), undo + 1, "the reopened dialog's OK is one undo step");
    }

    /// With Preview off, OK after File > Revert is refused and leaves the restored drawing as it is.
    #[test]
    fn vector_halftone_without_preview_refuses_ok_after_file_revert() {
        let (mut panel, _dir) = open_on_a_saved_drawing("halftone-revert-no-preview");
        set(&mut panel, "preview", json!(false));
        set(&mut panel, "frequency", json!(25));
        assert!(!panel.app().session.in_interaction(), "nothing previews with Preview off");
        // With Preview off the drawing has no unsaved change to revert yet.
        panel.app().run("object.move", json!({"dx": 5, "dy": 0})).unwrap();

        let (restored, undo) = revert_and_reselect(&mut panel);
        panel.frame();
        panel.click("OK");
        panel.frame();
        assert_eq!(doc(&panel), restored, "OK must leave the restored drawing unchanged");
        assert_eq!(undo_steps(&panel), undo, "OK must record no undo step");
        assert_refused_for_the_revert(&panel);
    }
}
