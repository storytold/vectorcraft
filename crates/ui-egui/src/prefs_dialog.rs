//! Edit → Preferences (⌘K): Illustrator-style category list + fields, rendered from the engine's
//! preference table ([`PREF_SPECS`]). The dialog edits a working copy in `Dialog::fields`
//! (one field per preference key, so agents can `ui.dialog.set` any of them) and OK applies it
//! through `prefs.set`. Also: persistence of engine prefs inside `UiState` and the per-frame
//! application of the UI-side preferences (brightness, canvas colour, UI scaling, render threads).

use serde_json::{Map, Value, json};
use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::{PREF_CATEGORIES, PREF_GROUPS, PREF_SPECS, PrefKind};

use crate::state::Dialog;
use crate::theme::{self, Brightness, Tokens};
use crate::{VectorcraftApp, widgets};

/// UI-only fields shown in the dialog (backed by `UiState`, not engine prefs).
pub const UI_FIELDS: &[(&str, &str, &str)] = &[
    ("__smartGuides", "Smart Guides", "Smart Guides (View → Smart Guides)"),
    ("__snapToGrid", "Guides & Grid", "Snap to Grid"),
    ("__taskBar", "General", "Enable Contextual Task Bar"),
];

pub fn open(app: &mut VectorcraftApp, category: Option<&str>) {
    // One field per preference key or group (the flattener presets have commands of their own).
    let mut fields: Map<String, Value> = app.session.prefs.to_json().as_object().cloned().unwrap_or_default();
    fields.retain(|k, _| vectorcraft_engine::cmd::prefscmds::spec(k).is_some() || PREF_GROUPS.contains(&k.as_str()));
    // Units ▸ General shows (and OK sets) the open document's units.
    if let Some(st) = app.session.active() {
        fields.insert("unitsGeneral".into(), json!(st.doc.units.key()));
    }
    let cat = category.and_then(|c| PREF_CATEGORIES.iter().find(|x| x.eq_ignore_ascii_case(c))).copied().unwrap_or(PREF_CATEGORIES[0]);
    fields.insert("__category".into(), json!(cat));
    fields.insert("__smartGuides".into(), json!(app.ui.view.smart_guides));
    fields.insert("__snapToGrid".into(), json!(app.ui.view.snap_to_grid));
    fields.insert("__taskBar".into(), json!(app.ui.task_bar));
    app.ui.dialog = Some(Dialog { kind: "preferences".into(), fields });
}

/// OK: validate and apply every preference in the dialog (nothing changes on error).
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let values: Map<String, Value> = d.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    app.run("prefs.set", json!({ "values": values }))?;
    app.ui.view.smart_guides = d.fields.get("__smartGuides").and_then(Value::as_bool).unwrap_or(app.ui.view.smart_guides);
    app.ui.view.snap_to_grid = d.fields.get("__snapToGrid").and_then(Value::as_bool).unwrap_or(app.ui.view.snap_to_grid);
    app.ui.task_bar = d.fields.get("__taskBar").and_then(Value::as_bool).unwrap_or(app.ui.task_bar);
    app.ui.dialog = None;
    app.ui.engine_prefs = app.session.prefs.to_json();
    Ok(Value::Null)
}

/// Copy the engine prefs into `UiState` so the host persists them with the UI state.
pub fn snapshot(app: &mut VectorcraftApp) {
    app.ui.engine_prefs = app.session.prefs.to_json();
}

/// Restore engine prefs from `UiState` after the host loaded saved UI state.
pub fn restore(app: &mut VectorcraftApp) {
    let mut p: Prefs = serde_json::from_value(app.ui.engine_prefs.clone()).unwrap_or_default();
    if app.ui.engine_prefs.is_null() {
        // Older preference files: keep the saved brightness.
        p.ui_brightness = app.ui.brightness.id().into();
    }
    // Older preference files kept the interface language with the UI state; English was the
    // default there, so only another language is carried over (onto an unset preference).
    if let Some(code) = app.ui.legacy_language.take()
        && p.interface_language == "auto"
        && code != "en"
        && crate::i18n::Lang::from_code(&code).is_some()
    {
        p.interface_language = code;
    }
    app.session.apply_prefs(p);
}

#[derive(Clone, Copy, PartialEq)]
struct Applied {
    brightness: Brightness,
    white_canvas: bool,
    threads: i32,
}

/// Per frame: push UI-side preferences into egui / the renderer when they change.
pub fn apply_runtime(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let p = &app.session.prefs;
    if let Some(b) = Brightness::parse(&p.ui_brightness)
        && b != app.ui.brightness
    {
        app.ui.brightness = b;
    }
    let want = Applied { brightness: app.ui.brightness, white_canvas: p.canvas_color == "white", threads: p.render_threads };
    let id = egui::Id::new("dc-applied-prefs");
    let prev: Option<Applied> = ctx.data(|d| d.get_temp::<Option<Applied>>(id)).flatten();
    if prev != Some(want) {
        theme::apply(ctx, want.brightness);
        if want.white_canvas {
            let mut t = Tokens::get(ctx);
            t.pasteboard = egui::Color32::WHITE;
            ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
        }
        if prev.is_some_and(|pr| pr.threads != want.threads) {
            vectorcraft_render::set_default_threads(u16::try_from(want.threads).ok());
            app.canvas.renderer.threads = vectorcraft_render::default_threads();
            app.canvas.worker = None;
            app.canvas.worker_started = false;
        }
        app.canvas.key = None;
        ctx.data_mut(|d| d.insert_temp(id, Some(want)));
    }
    let z = app.session.prefs.ui_scaling.clamp(0.75, 2.0) as f32;
    if (ctx.zoom_factor() - z).abs() > 1e-3 {
        ctx.set_zoom_factor(z);
    }
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut cancel, mut reset) = (false, false, false);
    egui::Area::new(egui::Id::new("modal-dim")).order(egui::Order::Middle).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
        ui.allocate_rect(ctx.content_rect(), egui::Sense::click());
    });
    let cat = d.str("__category");
    let cat = PREF_CATEGORIES.iter().find(|c| **c == cat).copied().unwrap_or(PREF_CATEGORIES[0]);
    egui::Window::new(tl!("Preferences"))
        .id(egui::Id::new("dialog-preferences"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -20.0))
        .constrain(true)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(20)))
        .show(ctx, |ui| {
            ui.set_width(760.0);
            ui.label(egui::RichText::new(tl!("Preferences")).font(theme::semibold(16.0)).color(t.text));
            ui.add_space(12.0);
            ui.horizontal_top(|ui| {
                // Category list.
                egui::Frame::NONE.fill(t.panel_darker).corner_radius(egui::CornerRadius::same(4)).inner_margin(egui::Margin::same(6)).show(
                    ui,
                    |ui| {
                        ui.set_width(196.0);
                        ui.set_min_height(430.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for c in PREF_CATEGORIES {
                                let sel = *c == cat;
                                let text = egui::RichText::new(tl!(*c)).size(12.5).color(if sel { t.text_strong } else { t.text });
                                let b = egui::Button::selectable(sel, text).frame_when_inactive(false).min_size(egui::vec2(184.0, 24.0));
                                if ui.add(b).clicked() {
                                    d.fields.insert("__category".into(), json!(c));
                                }
                            }
                        });
                    },
                );
                ui.add_space(14.0);
                // Fields.
                ui.vertical(|ui| {
                    ui.set_width(530.0);
                    ui.label(egui::RichText::new(tl!(cat)).font(theme::semibold(14.0)).color(t.text_strong));
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt(("prefs", cat)).max_height(400.0).auto_shrink([false, false]).show(ui, |ui| {
                        category_fields(ui, &mut d, cat);
                    });
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if widgets::secondary_button(ui, tl!("Reset Preferences"))
                    .on_hover_text(tl!("Restore every preference to its default (applied on OK)"))
                    .clicked()
                {
                    reset = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, tl!("OK")).clicked() {
                        ok = true;
                    }
                    ui.add_space(8.0);
                    if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                        cancel = true;
                    }
                    ui.add_space(16.0);
                    let i = PREF_CATEGORIES.iter().position(|c| *c == cat).unwrap_or(0);
                    if ui.add_enabled(i + 1 < PREF_CATEGORIES.len(), egui::Button::new(tl!("Next"))).clicked() {
                        d.fields.insert("__category".into(), json!(PREF_CATEGORIES[i + 1]));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new(tl!("Previous"))).clicked() {
                        d.fields.insert("__category".into(), json!(PREF_CATEGORIES[i - 1]));
                    }
                });
            });
        });
    if reset {
        for (k, v) in Prefs::default().to_json().as_object().cloned().unwrap_or_default() {
            d.fields.insert(k, v);
        }
    }
    app.ui.dialog = Some(d);
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

fn category_fields(ui: &mut egui::Ui, d: &mut Dialog, cat: &str) {
    let t = Tokens::get(ui.ctx());
    let mut section = "";
    let mut first = true;
    let specs: Vec<_> = PREF_SPECS.iter().filter(|s| s.category == cat).collect();
    for (key, c, label) in UI_FIELDS {
        if *c == cat && cat != "Smart Guides" {
            bool_row(ui, d, key, label);
        }
    }
    if cat == "Smart Guides" {
        bool_row(ui, d, "__smartGuides", UI_FIELDS[0].2);
        ui.add_space(4.0);
    }
    for sp in specs {
        if sp.section != section {
            section = sp.section;
            if !section.is_empty() {
                if !first {
                    ui.add_space(8.0);
                }
                ui.label(egui::RichText::new(tl!(section)).font(theme::semibold(12.5)).color(t.text_strong));
                ui.add_space(2.0);
            }
        }
        first = false;
        let v = d.fields.get(sp.key).cloned().unwrap_or(Value::Null);
        match sp.kind {
            PrefKind::Bool => bool_row(ui, d, sp.key, sp.label),
            PrefKind::Num { min, max, unit } => {
                labeled(ui, sp.label, |ui| {
                    let mut x = v.as_f64().unwrap_or(min);
                    let r = if sp.key == "uiScaling" {
                        ui.add(egui::Slider::new(&mut x, min..=max).step_by(0.05).text(tl!("Smaller ↔ Larger")))
                    } else {
                        let speed = if max - min > 100.0 { 0.5 } else { 0.05 };
                        ui.add(egui::DragValue::new(&mut x).range(min..=max).speed(speed).max_decimals(3).suffix(format!(" {unit}")))
                    };
                    if r.changed() {
                        d.fields.insert(sp.key.into(), json!(x));
                    }
                });
            }
            PrefKind::Length { min, max, measure } => {
                // In the dialog's own Units choice, so a change there shows right away.
                let unit = vectorcraft_doc::Unit::named(&d.str(measure.pref_key())).unwrap_or_default();
                let x = d.f64(sp.key, min);
                labeled(ui, sp.label, |ui| {
                    if let Some(x) = widgets::num_field(ui, sp.key, Some(x), unit, 110.0) {
                        d.fields.insert(sp.key.into(), json!(x.clamp(min, max)));
                    }
                });
            }
            PrefKind::Int { min, max } => {
                labeled(ui, sp.label, |ui| {
                    let mut x = v.as_i64().unwrap_or(min);
                    let r = if sp.key == "anchorSize" {
                        ui.add(egui::Slider::new(&mut x, min..=max).show_value(false).text(tl!("Size")))
                    } else {
                        ui.add(egui::DragValue::new(&mut x).range(min..=max).speed(0.2))
                    };
                    if r.changed() {
                        d.fields.insert(sp.key.into(), json!(x));
                    }
                });
            }
            PrefKind::Choice(opts) => {
                labeled(ui, sp.label, |ui| {
                    let cur = v.as_str().unwrap_or("");
                    let cur_label = opts.iter().find(|o| o.0 == cur).map(|o| o.1).unwrap_or(cur);
                    let labels: Vec<&str> = opts.iter().map(|o| o.1).collect();
                    if let Some(i) = widgets::dropdown(ui, sp.key, cur_label, &labels, 260.0) {
                        d.fields.insert(sp.key.into(), json!(opts[i].0));
                    }
                });
            }
            PrefKind::Color => {
                labeled(ui, sp.label, |ui| {
                    let hex = v.as_str().unwrap_or("#000000");
                    let c = vectorcraft_color::Color::from_hex(hex).map(|c| c.to_rgba8(1.0)).unwrap_or([0, 0, 0, 255]);
                    let mut rgb = [c[0], c[1], c[2]];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        d.fields.insert(sp.key.into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
                    }
                    ui.label(egui::RichText::new(hex).color(t.text_dim).size(11.0));
                });
            }
            PrefKind::Text if sp.key == "interfaceLanguage" => {
                // `auto` plus every registered language, by its own name.
                labeled(ui, sp.label, |ui| {
                    let cur = v.as_str().unwrap_or("auto");
                    let codes: Vec<&str> = std::iter::once("auto").chain(crate::i18n::Lang::all().map(|l| l.code())).collect();
                    let labels: Vec<&str> = std::iter::once(tl!("Auto")).chain(crate::i18n::Lang::all().map(|l| l.name())).collect();
                    let cur_label = codes.iter().position(|c| c.eq_ignore_ascii_case(cur)).and_then(|i| labels.get(i).copied()).unwrap_or(cur);
                    if let Some(i) = widgets::dropdown(ui, sp.key, cur_label, &labels, 260.0)
                        && let Some(code) = codes.get(i)
                    {
                        d.fields.insert(sp.key.into(), json!(code));
                    }
                });
            }
            PrefKind::Text => {
                labeled(ui, sp.label, |ui| {
                    let mut s = v.as_str().unwrap_or("").to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(260.0)).changed() {
                        d.fields.insert(sp.key.into(), json!(s));
                    }
                });
            }
        }
    }
}

fn bool_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let mut b = d.bool(key);
    if ui.checkbox(&mut b, tl!(label)).changed() {
        d.fields.insert(key.into(), json!(b));
    }
}

fn labeled(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(210.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(210.0);
            ui.add(egui::Label::new(egui::RichText::new(format!("{}{}", tl!(label), tl!(":"))).color(t.text)).truncate());
        });
        add(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Services::default())
    }

    #[test]
    fn dialog_ok_applies_prefs_and_ui_fields() {
        let mut a = app();
        open(&mut a, Some("general"));
        let d = a.ui.dialog.as_mut().unwrap();
        assert_eq!(d.str("__category"), "General");
        d.fields.insert("keyboardIncrement".into(), json!(4));
        d.fields.insert("uiBrightness".into(), json!("light"));
        d.fields.insert("__smartGuides".into(), json!(false));
        confirm(&mut a).unwrap();
        assert!(a.ui.dialog.is_none());
        assert_eq!(a.session.prefs.keyboard_increment, 4.0);
        assert_eq!(a.session.prefs.ui_brightness, "light");
        assert!(!a.ui.view.smart_guides);
        assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(4.0));
    }

    #[test]
    fn dialog_ok_with_invalid_value_keeps_dialog_and_prefs() {
        let mut a = app();
        open(&mut a, None);
        a.ui.dialog.as_mut().unwrap().fields.insert("anchorSize".into(), json!(42));
        assert!(confirm(&mut a).is_err());
        assert!(a.ui.dialog.is_some());
        assert_eq!(a.session.prefs, Prefs::default());
    }

    #[test]
    fn prefs_persist_through_ui_state() {
        let mut a = app();
        a.run("prefs.set", json!({"key": "gridlineEvery", "value": 50})).unwrap();
        snapshot(&mut a);
        let saved = serde_json::to_vec(&a.ui).unwrap();
        let mut b = app();
        b.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
        restore(&mut b);
        assert_eq!(b.session.prefs.gridline_every, 50.0);
    }

    #[test]
    fn old_pref_files_keep_their_brightness() {
        let mut a = app();
        a.ui.brightness = Brightness::Light;
        a.ui.engine_prefs = Value::Null;
        restore(&mut a);
        assert_eq!(a.session.prefs.ui_brightness, "light");
    }

    #[test]
    fn every_category_renders_fields() {
        for c in PREF_CATEGORIES {
            assert!(PREF_SPECS.iter().any(|s| s.category == *c));
        }
    }
}
