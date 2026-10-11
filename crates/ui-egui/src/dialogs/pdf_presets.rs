//! Edit → PDF Presets: every preset in a list (built-in ones in brackets, read only) with the
//! selected one's description and how its settings differ from the app default beside it; New…
//! (a preset starting from the selected one), Edit… (saved presets), Delete, Import… and Export…
//! (the selected preset) under the list. New… and Edit… open the preset editor (the Save PDF
//! dialog, kind `pdfPreset`), whose OK comes back here.
//!
//! Field: `selected` (a preset's name).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::pdf;
use vectorcraft_engine::cmd::pdfcmds::PRESET_FORMAT;

use super::DialogSpec;
use super::save_pdf::{open_preset, option_label, option_rows, section_of};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of PDF Presets.
pub const KIND: &str = "pdfPresets";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("PDF Presets").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 680.0,
    ..DialogSpec::FORM
};

/// Open the presets manager on `selected` (default: the app default).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "selected": selected.unwrap_or(pdf::DEFAULT_PRESET) })));
}

/// What a button asked for, done once the list and the details are drawn.
enum Action {
    Select(String),
    New,
    Edit,
    Delete,
    Import,
    Export,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let presets = app.session.pdf_presets();
    let builtins = presets.len() - app.session.prefs.pdf_presets.len();
    let i = presets.iter().position(|p| p.name.eq_ignore_ascii_case(d.str("selected").trim())).unwrap_or(0);
    let Some(preset) = presets.get(i) else { return false };
    let builtin = i < builtins;
    let mut act = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(300.0);
            widgets::dim_label(ui, tl!("Presets:"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("pdf-presets").min_scrolled_height(260.0).max_height(260.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(260.0);
                    for (k, p) in presets.iter().enumerate() {
                        let label = if k < builtins { format!("[{}]", tl!(&p.name)) } else { p.name.clone() };
                        if ui.selectable_label(k == i, label).clicked() {
                            act = Some(Action::Select(p.name.clone()));
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::flat_button(ui, tl!("New…"), 48.0).on_hover_text(tl!("A new preset starting from the selected one")).clicked() {
                    act = Some(Action::New);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Edit…"), 48.0)).inner.clicked() {
                    act = Some(Action::Edit);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Delete"), 52.0)).inner.clicked() {
                    act = Some(Action::Delete);
                }
                if widgets::flat_button(ui, tl!("Import…"), 64.0).clicked() {
                    act = Some(Action::Import);
                }
                if widgets::flat_button(ui, tl!("Export…"), 64.0).on_hover_text(tl!("Save the selected preset to a file")).clicked() {
                    act = Some(Action::Export);
                }
            });
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_width(340.0);
            ui.label(egui::RichText::new(&preset.name).color(t.text_strong).strong());
            if !preset.description.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&preset.description).color(t.text));
            }
            ui.add_space(10.0);
            details(ui, &t, preset, builtin);
        });
    });
    if let Some(a) = act {
        let r = match a {
            // Their file dialogs, shown off the UI thread, import into or export from the dialog
            // as it is then.
            Action::Import | Action::Export => {
                let (current, export) = (preset.name.clone(), matches!(a, Action::Export));
                crate::picks::in_dialog(app, d, move |app, d| run(app, d, if export { Action::Export } else { Action::Import }, &current))
            }
            a => run(app, d, a, &preset.name),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

/// What sets the preset apart: its settings that differ from the app default, and the notes on
/// what Save PDF can't honour or apply yet.
fn details(ui: &mut egui::Ui, t: &Tokens, preset: &vectorcraft_pdf::PdfPreset, builtin: bool) {
    let dim = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(tl!(text)).color(t.text_dim).size(11.5));
    widgets::dim_label(ui, &crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", pdf::DEFAULT_PRESET)]));
    let base = vectorcraft_pdf::builtin_preset(pdf::DEFAULT_PRESET).map(|p| p.settings).unwrap_or_default();
    let mut changed = pdf::changes(&preset.settings, &base);
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()));
    egui::ScrollArea::vertical().id_salt("pdf-preset-details").max_height(180.0).show(ui, |ui| {
        if changed.is_empty() {
            dim(ui, tl!("None."));
        }
        option_rows(ui, &changed, option_label);
    });
    ui.add_space(6.0);
    if let Err(e) = preset.settings.check() {
        dim(ui, &format!("⚠ {e}"));
    }
    for w in preset.settings.warnings() {
        dim(ui, &format!("⚠ {w}"));
    }
    if builtin {
        ui.add_space(6.0);
        dim(ui, tl!("Built-in presets are read-only: New… starts an editable copy."));
    }
}

/// Select the preset `name`.
fn select(d: &mut Dialog, name: &str) {
    d.fields.insert("selected".into(), json!(name));
}

fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(d, &name),
        // The editor replaces this dialog; its OK comes back here.
        Action::New => {
            open_preset(app, &json!({ "preset": current }))?;
            *d = app.ui.dialog.take().ok_or("no preset editor")?;
        }
        Action::Edit => {
            open_preset(app, &json!({ "name": current }))?;
            *d = app.ui.dialog.take().ok_or("no preset editor")?;
        }
        Action::Delete => {
            let i = app.session.prefs.pdf_presets.iter().position(|p| p.name == current);
            app.run("pdf.preset.delete", json!({ "name": current }))?;
            // The one above it takes its place (the last built-in one above the first saved one).
            let builtins = vectorcraft_pdf::builtin_presets().len();
            let next = app.session.pdf_presets().get((i.unwrap_or(0) + builtins).saturating_sub(1)).map(|p| p.name.clone());
            select(d, &next.unwrap_or_default());
        }
        Action::Import => {
            // On the web the picked file arrives later and imports through `io::open_bytes`.
            if let Some(f) = app.services.open_async.as_mut() {
                f();
                return Ok(());
            }
            let pick = crate::FilePick { filters: vec![("PDF presets", vectorcraft_engine::cmd::pdfcmds::PRESET_EXTS)], ..Default::default() };
            let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
            let r = app.run("pdf.preset.import", json!({ "path": path }))?;
            if let Some(first) = r["imported"].get(0).and_then(Value::as_str) {
                select(d, first);
            }
        }
        Action::Export => {
            crate::io::save_command_output(app, "pdf.preset.export", PRESET_FORMAT, json!({ "names": [current] }))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn kind(app: &VectorcraftApp) -> Option<&str> {
        app.ui.dialog.as_ref().map(|d| d.kind.as_str())
    }

    /// Press a manager button on the preset `current`, as the body does.
    fn press(app: &mut VectorcraftApp, act: Action, current: &str) {
        let mut d = app.ui.dialog.take().unwrap();
        run(app, &mut d, act, current).unwrap();
        app.ui.dialog = Some(d);
        frame(app);
    }

    #[test]
    fn new_edit_and_delete_go_through_the_preset_editor() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.pdfPresetsDialog", json!({"selected": "Press Quality"})).unwrap();
        frame(&mut app);
        assert_eq!(kind(&app), Some(KIND));
        // New… opens the editor on a copy of the selected preset; its OK saves it and comes back.
        press(&mut app, Action::New, "Press Quality");
        assert_eq!(kind(&app), Some("pdfPreset"));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.str("name"), d.fields["compatibility"].clone()), ("PDF Preset 1".to_string(), json!("1.6")));
        d.fields.insert("name".into(), json!("Press 1.5"));
        d.fields.insert("compatibility".into(), json!("1.5"));
        d.fields.insert("description".into(), json!("Mine"));
        for s in super::super::save_pdf::SECTIONS {
            app.ui.dialog.as_mut().unwrap().fields.insert("__section".into(), json!(s));
            frame(&mut app);
        }
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["name"], "Press 1.5");
        assert_eq!((kind(&app), app.ui.dialog.as_ref().unwrap().str("selected")), (Some(KIND), "Press 1.5".to_string()));
        let saved = &app.session.prefs.pdf_presets[0];
        assert_eq!((saved.settings.compatibility.id(), saved.description.as_str()), ("1.5", "Mine"));
        assert!(saved.settings.bleed.use_document, "the rest comes from Press Quality");
        frame(&mut app);
        // Edit… renames it; a new preset can't take a taken name.
        press(&mut app, Action::Edit, "Press 1.5");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("__editing"), "Press 1.5");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Press"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.prefs.pdf_presets[0].name, "Press");
        press(&mut app, Action::New, "Press");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("press"));
        assert!(super::super::confirm(&mut app).is_err());
        assert_eq!(kind(&app), Some("pdfPreset"), "stays open on a taken name");
        // Built-in presets can't be edited; Delete selects the preset above.
        assert!(open_preset(&mut app, &json!({"name": "Press Quality"})).is_err());
        open(&mut app, Some("Press"));
        press(&mut app, Action::Delete, "Press");
        assert!(app.session.prefs.pdf_presets.is_empty());
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("selected"), "PDF/X-4:2010");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "Close");
    }

    #[test]
    fn save_pdf_saves_its_settings_as_a_preset() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("ui.savePdfDialog", json!({"preset": "High Quality Print", "compatibility": "1.5"})).unwrap();
        frame(&mut app);
        // Save Preset… asks for a name; OK (or Enter) then saves the preset, not the PDF.
        app.ui.dialog.as_mut().unwrap().fields.insert("__savePresetAs".into(), json!("Proofs"));
        frame(&mut app);
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["name"], "Proofs");
        let d = app.ui.dialog.as_ref().expect("Save PDF stays open");
        assert_eq!((d.kind.as_str(), d.str("preset")), ("savePdf", "Proofs".to_string()));
        assert!(!d.fields.contains_key("__savePresetAs"));
        assert!(d.fields["__presets"].as_array().unwrap().contains(&json!("Proofs")));
        let saved = &app.session.prefs.pdf_presets[0].settings;
        assert_eq!(saved.compatibility.id(), "1.5");
        assert_eq!(
            saved,
            &vectorcraft_pdf::PdfSettings {
                compatibility: saved.compatibility,
                ..vectorcraft_pdf::builtin_preset("High Quality Print").unwrap().settings
            }
        );
        // A built-in name is refused and the row stays.
        app.ui.dialog.as_mut().unwrap().fields.insert("__savePresetAs".into(), json!("Press Quality"));
        assert!(super::super::confirm(&mut app).is_err());
        frame(&mut app);
        assert_eq!(app.session.prefs.pdf_presets.len(), 1);
    }

    #[test]
    fn import_and_export_go_through_files() {
        let dir = std::env::temp_dir().join(format!("vc-pdfpresets-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mine.vcpdfpresets").to_string_lossy().to_string();
        let picked = file.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": true})).unwrap();
        app.run("ui.pdfPresetsDialog", json!({"selected": "Mine"})).unwrap();
        frame(&mut app);
        press(&mut app, Action::Export, "Mine");
        // Opening the file imports it (also how the web's picked file arrives).
        let bytes = std::fs::read(&file).unwrap();
        crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
        let names: Vec<&str> = app.session.prefs.pdf_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "Mine 2"]);
        assert!(app.session.prefs.pdf_presets[1].settings.thumbnails);
        assert!(app.ui.status.contains("PDF presets"), "{}", app.ui.status);
        let _ = std::fs::remove_dir_all(dir);
    }
}
