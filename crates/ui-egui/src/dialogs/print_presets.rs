//! Edit → Print Presets: every preset in a list ([Default], protected, then the saved ones) with
//! how the selected one differs from [Default] beside it; New… (a preset starting from the
//! selected one), Edit… (saved presets), Delete, Import… and Export… (the selected preset) under
//! the list. New… and Edit… open the preset editor (the Print dialog, kind `printPreset`), whose OK
//! comes back here.
//!
//! Field: `selected` (a preset's name).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::printpresets::{DEFAULT_PRESET, PRESET_EXTS, PRESET_FORMAT, changes};

use super::DialogSpec;
use super::print::{open_preset, option_label, section_of};
use super::save_pdf::option_rows;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Print Presets.
pub const KIND: &str = "printPresets";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Print Presets").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 620.0,
    ..DialogSpec::FORM
};

/// Open the presets manager on `selected` (default: [Default]).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "selected": selected.unwrap_or(DEFAULT_PRESET) })));
}

/// What a button asked for, done once the list and the details are drawn.
pub(super) enum Action {
    Select(String),
    New,
    Edit,
    Delete,
    Import,
    Export,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let saved = &app.session.prefs.print_presets;
    let selected = d.str("selected");
    // 0 is [Default]; saved presets follow.
    let i = saved.iter().position(|p| p.name.eq_ignore_ascii_case(selected.trim())).map_or(0, |i| i + 1);
    let name = i.checked_sub(1).and_then(|k| saved.get(k)).map_or(DEFAULT_PRESET, |p| p.name.as_str()).to_string();
    let builtin = i == 0;
    let mut act = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(280.0);
            widgets::dim_label(ui, tl!("Presets:"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("print-presets").min_scrolled_height(240.0).max_height(240.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(240.0);
                    for (k, n) in std::iter::once(DEFAULT_PRESET).chain(saved.iter().map(|p| p.name.as_str())).enumerate() {
                        if ui.selectable_label(k == i, if k == 0 { tl!(n) } else { n }).clicked() {
                            act = Some(Action::Select(n.to_string()));
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
                if widgets::flat_button(ui, tl!("Import…"), 60.0).clicked() {
                    act = Some(Action::Import);
                }
                if widgets::flat_button(ui, tl!("Export…"), 60.0).on_hover_text(tl!("Save the selected preset to a file")).clicked() {
                    act = Some(Action::Export);
                }
            });
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_width(300.0);
            ui.label(egui::RichText::new(&name).color(t.text_strong).strong());
            ui.add_space(8.0);
            widgets::dim_label(ui, &crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", DEFAULT_PRESET)]));
            let settings = i.checked_sub(1).and_then(|k| saved.get(k)).map(|p| &p.settings);
            let mut changed = settings.map(|s| changes(s, &Default::default())).unwrap_or_default();
            changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()).0);
            egui::ScrollArea::vertical().id_salt("print-preset-details").max_height(200.0).show(ui, |ui| {
                if changed.is_empty() {
                    ui.label(egui::RichText::new(tl!("None.")).color(t.text_dim).size(11.5));
                }
                option_rows(ui, &changed, option_label);
            });
            if builtin {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("[Default] is protected: New… starts an editable copy.")).color(t.text_dim).size(11.5));
            }
        });
    });
    if let Some(a) = act {
        let r = match a {
            // Their file dialogs, shown off the UI thread, import into or export from the dialog
            // as it is then.
            Action::Import | Action::Export => {
                let (current, export) = (name.clone(), matches!(a, Action::Export));
                crate::picks::in_dialog(app, d, move |app, d| run(app, d, if export { Action::Export } else { Action::Import }, &current))
            }
            a => run(app, d, a, &name),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

fn select(d: &mut Dialog, name: &str) {
    d.fields.insert("selected".into(), json!(name));
}

pub(super) fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action, current: &str) -> Result<(), String> {
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
            let i = app.session.prefs.print_presets.iter().position(|p| p.name == current);
            app.run("print.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place ([Default] above the first saved one).
            let next = i.and_then(|i| i.checked_sub(1)).and_then(|i| app.session.prefs.print_presets.get(i)).map(|p| p.name.clone());
            select(d, next.as_deref().unwrap_or(DEFAULT_PRESET));
        }
        Action::Import => {
            // On the web the picked file arrives later and imports through `io::open_bytes`.
            if let Some(f) = app.services.open_async.as_mut() {
                f();
                return Ok(());
            }
            let pick = crate::FilePick { filters: vec![("Print presets", PRESET_EXTS)], ..Default::default() };
            let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
            let r = app.run("print.presets.import", json!({ "path": path }))?;
            if let Some(first) = r["imported"].get(0).and_then(Value::as_str) {
                select(d, first);
            }
        }
        Action::Export => {
            crate::io::save_command_output(app, "print.presets.export", PRESET_FORMAT, json!({ "names": [current] }))?;
        }
    }
    Ok(())
}
