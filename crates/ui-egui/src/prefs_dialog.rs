//! Edit → Preferences (⌘K): Illustrator-style category list + fields, rendered from the engine's
//! preference table ([`PREF_SPECS`]). The dialog edits a working copy in `Dialog::fields`
//! (one field per preference key, so agents can `ui.dialog.set` any of them) and OK applies it
//! through `prefs.set`. Also: persistence of engine prefs inside `UiState` and the per-frame
//! application of the UI-side preferences (brightness, canvas colour, UI scaling, render threads).

use serde_json::{Map, Value, json};
use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::{
    APPEARANCE_MODES, DARK_THEMES, GPU_PREFERENCES, LIGHT_THEMES, PREF_CATEGORIES, PREF_GROUPS, PREF_SPECS, PrefKind,
};

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
    let mut p = Prefs::from_saved(app.ui.engine_prefs.clone());
    if app.ui.engine_prefs.is_null() {
        // Older preference files: keep the saved brightness (as a fixed Dark or Light mode).
        let _ = p.select_brightness(app.ui.brightness.id());
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
    // 0.5.0 saved its default, `powerSaving`, which Automatic replaced (#502): any value this
    // version doesn't offer reads as the default, as the desktop app reads it at startup.
    if !GPU_PREFERENCES.iter().any(|(v, _)| *v == p.gpu_preference) {
        p.gpu_preference = Prefs::default().gpu_preference;
    }
    app.session.apply_prefs(p);
}

#[derive(Clone, Copy, PartialEq)]
struct Applied {
    brightness: Brightness,
    /// User Interface › Canvas Color, unless it matches the interface brightness.
    canvas: Option<egui::Color32>,
    threads: i32,
    tool_tips: bool,
    scrub: bool,
    bare_points: bool,
}

/// User Interface › Canvas Color: the canvas around the artboards, or `None` to match the
/// interface brightness. The greys keep the white page's edge in view (#663).
fn canvas_color(key: &str) -> Option<egui::Color32> {
    match key {
        "white" => Some(egui::Color32::WHITE),
        "lightGray" => Some(egui::Color32::from_gray(0xc8)),
        "mediumGray" => Some(egui::Color32::from_gray(0x96)),
        "darkGray" => Some(egui::Color32::from_gray(0x5a)),
        _ => None,
    }
}

/// The brightness the appearance preferences show: the light or dark theme for the mode, Sync
/// with system following `system` (Dark when the system doesn't say).
pub(crate) fn selected_brightness(p: &Prefs, system: Option<egui::Theme>) -> Brightness {
    let light = match p.appearance_mode.as_str() {
        "light" => true,
        "auto" => system == Some(egui::Theme::Light),
        _ => false,
    };
    let (id, fallback) = if light { (&p.light_theme, Brightness::Light) } else { (&p.dark_theme, Brightness::MediumDark) };
    Brightness::parse(id).filter(|b| Tokens::for_brightness(*b).dark != light).unwrap_or(fallback)
}

/// The system's light or dark appearance: the desktop app's service first (it also covers Linux
/// desktops whose windowing doesn't report it), then what egui reports.
pub(crate) fn system_theme(app: &VectorcraftApp, ctx: &egui::Context) -> Option<egui::Theme> {
    app.services.system_theme.as_ref().and_then(|read| read(ctx)).or_else(|| ctx.system_theme())
}

/// The appearance button and `window.appearanceMode` without a mode: Sync with system, then Light,
/// then Dark, then back. The light and dark theme choices stay as they are.
pub(crate) fn next_appearance_mode(mode: &str) -> &'static str {
    match mode {
        "auto" => "light",
        "light" => "dark",
        _ => "auto",
    }
}

/// Set the appearance mode (`None`: the next one); [`apply_runtime`] shows it from the next frame.
pub(crate) fn set_appearance_mode(app: &mut VectorcraftApp, mode: Option<&str>) -> Result<Value, String> {
    let mode = mode.unwrap_or_else(|| next_appearance_mode(&app.session.prefs.appearance_mode)).to_string();
    app.run("prefs.set", json!({"key": "appearanceMode", "value": mode}))?;
    Ok(json!(app.session.prefs.appearance_mode))
}

/// Per frame: push UI-side preferences into egui / the renderer when they change.
pub fn apply_runtime(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // Appearance Mode: reading the system's appearance is a load (no polling); the desktop app's
    // service asks for a frame when it changes, and egui does on its own theme events.
    let system = system_theme(app, ctx);
    ctx.data_mut(|m| m.insert_temp(egui::Id::new(SYSTEM_THEME_ID), system));
    let b = selected_brightness(&app.session.prefs, system);
    if b != app.ui.brightness {
        app.ui.brightness = b;
    }
    let p = &app.session.prefs;
    let want = Applied {
        brightness: app.ui.brightness,
        canvas: canvas_color(&p.canvas_color),
        threads: p.render_threads,
        tool_tips: p.show_tool_tips,
        scrub: p.scrub_numeric_fields,
        bare_points: p.numbers_without_units_are_points,
    };
    let id = egui::Id::new("dc-applied-prefs");
    let prev: Option<Applied> = ctx.data(|d| d.get_temp::<Option<Applied>>(id)).flatten();
    if prev != Some(want) {
        theme::apply(ctx, want.brightness);
        // General › Show Tool Tips: off, no button or field shows its tool tip (they never come
        // due), whichever widget asks for one.
        let delay = if want.tool_tips { egui::style::Interaction::default().tooltip_delay } else { f32::INFINITY };
        ctx.global_style_mut(|s| s.interaction.tooltip_delay = delay);
        crate::scrub::set_enabled(ctx, want.scrub);
        crate::widgets::set_bare_numbers_are_points(ctx, want.bare_points);
        if let Some(c) = want.canvas {
            let mut t = Tokens::get(ctx);
            t.pasteboard = c;
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
    let cat = d.str("__category");
    let cat = PREF_CATEGORIES.iter().find(|c| **c == cat).copied().unwrap_or(PREF_CATEGORIES[0]);
    crate::dialogs::modal::show(ctx, tl!("Preferences"), egui::Id::new("dialog-preferences"), -20.0, 20, |ui| {
        ui.set_width(760.0);
        crate::dialogs::modal::heading(ui, tl!("Preferences"));
        ui.add_space(12.0);
        ui.horizontal_top(|ui| {
            // Category list.
            egui::Frame::NONE.fill(t.panel_darker).corner_radius(egui::CornerRadius::same(4)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.set_width(196.0);
                ui.set_min_height(430.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for c in PREF_CATEGORIES {
                        let sel = *c == cat;
                        if category_button(ui, tl!(*c), sel, &t).clicked() {
                            d.fields.insert("__category".into(), json!(c));
                        }
                    }
                });
            });
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

/// Selected categories keep the app accent even when egui switches its light/dark style.
fn category_button(ui: &mut egui::Ui, label: &str, selected: bool, t: &Tokens) -> egui::Response {
    let text = egui::RichText::new(label).size(12.5).color(if selected { egui::Color32::WHITE } else { t.text });
    let button = egui::Button::selectable(selected, text).min_size(egui::vec2(184.0, 24.0));
    ui.add(if selected { button.fill(t.accent_strong).stroke(egui::Stroke::NONE) } else { button })
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
    if cat == "User Interface" {
        appearance_rows(ui, d);
    }
    for sp in specs {
        // Shown by `appearance_rows`; the legacy single brightness isn't shown.
        if APPEARANCE_KEYS.contains(&sp.key) {
            continue;
        }
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
            // Units › Numbers Without Units Are Points only tells points from picas: dimmed
            // unless a unit is Picas.
            PrefKind::Bool if sp.key == "numbersWithoutUnitsArePoints" => {
                ui.add_enabled_ui(picas_in_use(d), |ui| bool_row(ui, d, sp.key, sp.label));
            }
            // Performance › Animated Zoom needs GPU Performance: dimmed while it is off.
            PrefKind::Bool if sp.key == "animatedZoom" => {
                ui.add_enabled_ui(d.bool("gpuPerformance"), |ui| bool_row(ui, d, sp.key, sp.label));
            }
            PrefKind::Bool => bool_row(ui, d, sp.key, sp.label),
            PrefKind::Num { min, max, unit } => {
                labeled(ui, sp.label, |ui| {
                    let mut x = v.as_f64().unwrap_or(min);
                    let new = if sp.key == "uiScaling" {
                        ui.add(egui::Slider::new(&mut x, min..=max).step_by(0.05).text(tl!("Smaller ↔ Larger"))).changed().then_some(x)
                    } else {
                        widgets::range_field(ui, sp.key, x, min..=max, &format!(" {unit}"), 3, 110.0)
                    };
                    if let Some(x) = new {
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
                    let new = if sp.key == "anchorSize" {
                        ui.add(egui::Slider::new(&mut x, min..=max).show_value(false).text(tl!("Size"))).changed().then_some(x)
                    } else {
                        widgets::range_field(ui, sp.key, x as f64, min as f64..=max as f64, "", 0, 110.0).map(|x| x as i64)
                    };
                    if let Some(x) = new {
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
                if sp.key == "gpuPreference" {
                    // The window's graphics device is chosen once, when the app starts.
                    ui.label(egui::RichText::new(tl!("Applies the next time VectorCraft starts.")).color(t.text_dim).size(11.0));
                }
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

/// User Interface preferences the appearance block shows instead of the generic rows.
const APPEARANCE_KEYS: &[&str] = &["uiBrightness", "appearanceMode", "darkTheme", "lightTheme"];

/// User Interface › Appearance: the mode, then the light and dark themes side by side, each with a
/// preview of the editor in it. The card the mode shows now is outlined.
fn appearance_rows(ui: &mut egui::Ui, d: &mut Dialog) {
    let mode = d.str("appearanceMode");
    labeled(ui, "Appearance Mode", |ui| {
        let cur = APPEARANCE_MODES.iter().find(|(v, _)| *v == mode).map_or(tl!("Dark"), |(_, label)| tl!(*label));
        let labels: Vec<&str> = APPEARANCE_MODES.iter().map(|(_, label)| tl!(*label)).collect();
        if let Some(i) = widgets::dropdown(ui, "appearanceMode", cur, &labels, 260.0)
            && let Some((v, _)) = APPEARANCE_MODES.get(i)
        {
            d.fields.insert("appearanceMode".into(), json!(v));
        }
    });
    ui.add_space(6.0);
    let system = ui.ctx().data(|m| m.get_temp::<Option<egui::Theme>>(egui::Id::new(SYSTEM_THEME_ID))).flatten().or_else(|| ui.ctx().system_theme());
    let mode = d.str("appearanceMode");
    let light_active = mode == "light" || (mode == "auto" && system == Some(egui::Theme::Light));
    let gap = 10.0;
    let width = ((ui.available_width() - gap) / 2.0).floor().max(180.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        theme_card(ui, d, "lightTheme", "Light Theme", LIGHT_THEMES, light_active, width);
        theme_card(ui, d, "darkTheme", "Dark Theme", DARK_THEMES, !light_active, width);
    });
    ui.add_space(10.0);
}

/// The id under which [`apply_runtime`] leaves the system appearance for the dialog.
const SYSTEM_THEME_ID: &str = "vc-system-theme";

fn theme_card(ui: &mut egui::Ui, d: &mut Dialog, key: &str, title: &str, options: &[(&str, &str)], active: bool, width: f32) {
    let t = Tokens::get(ui.ctx());
    let Some((first, _)) = options.first() else { return };
    let cur = d.str(key);
    let shown = options.iter().map(|(v, _)| *v).find(|v| *v == cur).unwrap_or(first);
    egui::Frame::NONE
        .fill(t.panel_darker)
        .stroke(egui::Stroke::new(if active { 1.5 } else { 1.0 }, if active { t.accent } else { t.divider }))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                let inner = width - 16.0;
                ui.set_width(inner);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!(title)).font(theme::semibold(12.5)).color(t.text_strong));
                    if active {
                        ui.label(egui::RichText::new(tl!("Active")).size(11.0).color(t.accent));
                    }
                });
                ui.add_space(2.0);
                theme_preview(ui, Brightness::parse(shown).unwrap_or_default(), inner);
                ui.add_space(4.0);
                for (v, label) in options {
                    if ui.radio(*v == shown, tl!(*label)).clicked() {
                        d.fields.insert(key.into(), json!(v));
                    }
                }
            });
        });
}

/// A small VectorCraft window in brightness `b`'s colours: the app bar with its search box, the
/// document tab, the two-column Tools panel, an artboard with selected art on the canvas, the
/// panel icon column, the Properties | Layers | Libraries dock and the status bar.
fn theme_preview(ui: &mut egui::Ui, b: Brightness, width: f32) {
    use egui::{Rect, pos2, vec2};
    let p = Tokens::for_brightness(b);
    let height = (width * 0.56).round().clamp(96.0, 150.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let bar = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), vec2(w, h));
    painter.rect_filled(rect, 3.0, p.panel);
    let (l, top, r, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    // App bar: the brand mark, menus, the search box and the workspace switcher.
    let app_bar = bar(l, top, width, 12.0);
    painter.rect_filled(app_bar, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, p.app_bar);
    painter.rect_filled(bar(l + 4.0, top + 3.0, 6.0, 6.0), 1.5, p.accent);
    for (i, w) in [9.0, 8.0, 11.0, 8.0, 10.0].into_iter().enumerate() {
        painter.rect_filled(bar(l + 14.0 + i as f32 * 13.0, top + 5.0, w, 2.0), 1.0, p.text_dim);
    }
    painter.rect_filled(bar(r - width * 0.44, top + 3.0, width * 0.22, 6.0), 3.0, p.input);
    painter.rect_filled(bar(r - width * 0.19, top + 3.5, width * 0.15, 5.0), 1.0, p.panel);
    // Status bar.
    let status = bar(l, bottom - 7.0, width, 7.0);
    painter.rect_filled(status, egui::CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, p.panel_darker);
    painter.rect_filled(bar(l + 4.0, bottom - 4.5, 14.0, 2.0), 1.0, p.text_dim);
    // Tools: two columns of tools under their group labels, the Selection tool active.
    let tools = Rect::from_min_max(pos2(l, app_bar.bottom()), pos2(l + 20.0, status.top()));
    painter.rect_filled(tools, 0.0, p.panel);
    let mut y = tools.top() + 4.0;
    for group in 0..3 {
        painter.rect_filled(bar(l + 6.0, y, 8.0, 1.5), 0.5, p.text_disabled);
        y += 4.0;
        for row in 0..2 {
            if y + 5.0 > tools.bottom() - 2.0 {
                break;
            }
            for col in 0..2 {
                let cell = bar(l + 3.0 + col as f32 * 8.0, y, 6.0, 5.0);
                if group == 0 && row == 0 && col == 0 {
                    painter.rect_filled(cell.expand(1.0), 1.0, p.tool_active);
                }
                painter.rect_filled(cell.shrink(1.0), 0.5, p.icon);
            }
            y += 8.0;
        }
        y += 2.0;
    }
    // The dock: Properties | Layers | Libraries, then labelled fields; the icon column before it.
    let dock_w = (width * 0.3).round();
    let dock = Rect::from_min_max(pos2(r - dock_w, app_bar.bottom()), pos2(r, status.top()));
    let icons = Rect::from_min_max(pos2(dock.left() - 11.0, dock.top()), pos2(dock.left(), dock.bottom()));
    painter.rect_filled(icons, 0.0, p.panel_darker);
    for i in 0..6 {
        let cy = icons.top() + 6.0 + i as f32 * 9.0;
        if cy + 4.0 < icons.bottom() {
            painter.rect_filled(bar(icons.left() + 3.0, cy, 5.0, 5.0), 1.0, p.icon);
        }
    }
    painter.rect_filled(dock, 0.0, p.panel);
    let tabs = bar(dock.left(), dock.top(), dock_w, 9.0);
    painter.rect_filled(tabs, 0.0, p.tab_strip);
    painter.rect_filled(bar(dock.left(), dock.top(), dock_w * 0.38, 9.0), 0.0, p.panel);
    painter.rect_filled(bar(dock.left() + 3.0, dock.top() + 3.5, dock_w * 0.28, 2.0), 1.0, p.text_strong);
    painter.rect_filled(bar(dock.left() + dock_w * 0.45, dock.top() + 3.5, dock_w * 0.2, 2.0), 1.0, p.text_dim);
    painter.rect_filled(bar(dock.left() + dock_w * 0.72, dock.top() + 3.5, dock_w * 0.22, 2.0), 1.0, p.text_dim);
    let mut y = tabs.bottom() + 5.0;
    while y + 6.0 < dock.bottom() - 2.0 {
        painter.rect_filled(bar(dock.left() + 3.0, y + 2.0, dock_w * 0.22, 2.0), 1.0, p.text_dim);
        let field = bar(dock.left() + dock_w * 0.32, y, dock_w * 0.6, 6.0);
        painter.rect_filled(field, 1.0, p.input);
        painter.rect_stroke(field, 1.0, egui::Stroke::new(0.5, p.input_border), egui::StrokeKind::Inside);
        y += 10.0;
    }
    // The canvas: the document tab, then an artboard with a selected shape on the pasteboard.
    let canvas = Rect::from_min_max(pos2(tools.right(), app_bar.bottom()), pos2(icons.left(), status.top()));
    painter.rect_filled(canvas, 0.0, p.pasteboard);
    let tab_strip = bar(canvas.left(), canvas.top(), canvas.width(), 7.0);
    painter.rect_filled(tab_strip, 0.0, p.tab_strip);
    painter.rect_filled(bar(canvas.left(), canvas.top(), canvas.width().min(36.0), 7.0), 0.0, p.panel);
    painter.rect_filled(bar(canvas.left() + 4.0, canvas.top() + 2.5, canvas.width().min(36.0) - 10.0, 2.0), 1.0, p.text);
    for x in [canvas.left(), icons.left(), dock.left()] {
        painter.vline(x, app_bar.bottom()..=status.top(), egui::Stroke::new(1.0, p.border));
    }
    let area = Rect::from_min_max(pos2(canvas.left(), tab_strip.bottom()), canvas.max);
    let h = (area.height() - 10.0).max(10.0);
    let board = Rect::from_center_size(area.center(), vec2((h * 0.78).min(area.width() - 10.0), h));
    painter.rect_filled(board.translate(vec2(1.0, 1.5)), 0.0, egui::Color32::from_black_alpha(60));
    painter.rect_filled(board, 0.0, egui::Color32::WHITE);
    let sun = board.center() + vec2(0.0, board.height() * 0.08);
    let rad = board.width() * 0.24;
    painter.circle_filled(sun, rad, p.accent);
    painter.rect_filled(Rect::from_min_max(pos2(board.left(), sun.y + rad * 0.45), board.max), 0.0, p.accent_strong.gamma_multiply(0.55));
    let sel = Rect::from_center_size(sun, vec2(rad * 2.0, rad * 2.0)).expand(1.5);
    painter.rect_stroke(sel, 0.0, egui::Stroke::new(1.0, p.selection), egui::StrokeKind::Middle);
    for c in [sel.left_top(), sel.right_top(), sel.left_bottom(), sel.right_bottom()] {
        painter.rect_filled(Rect::from_center_size(c, vec2(3.0, 3.0)), 0.0, egui::Color32::WHITE);
        painter.rect_stroke(Rect::from_center_size(c, vec2(3.0, 3.0)), 0.0, egui::Stroke::new(0.8, p.selection), egui::StrokeKind::Middle);
    }
}

/// Is one of the dialog's Units (General, Stroke, Type, East Asian Type) Picas?
fn picas_in_use(d: &Dialog) -> bool {
    ["unitsGeneral", "unitsStroke", "unitsType", "unitsAsianType"].iter().any(|k| d.str(k) == vectorcraft_doc::Unit::Picas.key())
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

    /// User Interface › Canvas Color (#663): White and the three greys paint the canvas around the
    /// artboards; Match User Interface Brightness keeps the theme's.
    #[test]
    fn canvas_color_choices_paint_the_pasteboard() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        let ctx = egui::Context::default();
        apply_runtime(&mut app, &ctx);
        let themed = Tokens::get(&ctx).pasteboard;
        for (key, want) in [
            ("white", egui::Color32::WHITE),
            ("lightGray", egui::Color32::from_gray(0xc8)),
            ("mediumGray", egui::Color32::from_gray(0x96)),
            ("darkGray", egui::Color32::from_gray(0x5a)),
            ("matchUi", themed),
        ] {
            app.run("prefs.set", serde_json::json!({"key": "canvasColor", "value": key})).unwrap();
            apply_runtime(&mut app, &ctx);
            assert_eq!(Tokens::get(&ctx).pasteboard, want, "{key}");
        }
        assert!(app.run("prefs.set", serde_json::json!({"key": "canvasColor", "value": "pink"})).is_err());
    }

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

    /// General › Show Tool Tips: off, a button held under the pointer shows no tool tip; back on,
    /// it does again (#394).
    #[test]
    fn show_tool_tips_off_hides_tool_tips() {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut a = app();
        let ctx = egui::Context::default();
        let mut time = 0.0;
        // Come from elsewhere and hover the button for two seconds: did its tool tip show?
        let mut hover = |a: &mut VectorcraftApp| {
            let mut shown = vec![];
            for i in 0..20 {
                time += 0.1;
                let events = match i {
                    0 => vec![egui::Event::PointerGone],
                    1 => vec![egui::Event::PointerMoved(egui::pos2(20.0, 12.0))],
                    _ => vec![],
                };
                let raw = egui::RawInput {
                    events,
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(raw, |ui| {
                    apply_runtime(a, ui.ctx());
                    ui.button("Button").on_hover_text("The tool tip");
                });
                out.textures_delta.clear();
                out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
            }
            shown.iter().any(|t| t == "The tool tip")
        };
        assert!(hover(&mut a), "on by default");
        a.run("prefs.set", json!({"key": "showToolTips", "value": false})).unwrap();
        assert!(!hover(&mut a), "off: no tool tip");
        a.run("prefs.set", json!({"key": "showToolTips", "value": true})).unwrap();
        assert!(hover(&mut a), "on again");
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

    /// Performance › Graphics Processor (#306, #502): shown with its restart note, applied by OK
    /// and saved with the UI state under the key the desktop app reads before the window opens.
    #[test]
    fn graphics_processor_preference_shows_and_persists() {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut a = app();
        open(&mut a, Some("Performance"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // The window sizes itself on the first frame and draws on the second.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let mut shown = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        assert!(shown.iter().any(|t| t.starts_with("Graphics Processor")), "{shown:?}");
        assert!(shown.iter().any(|t| t == "Automatic"), "{shown:?}");
        assert!(shown.iter().any(|t| t == "Applies the next time VectorCraft starts."), "{shown:?}");
        a.ui.dialog.as_mut().unwrap().fields.insert("gpuPreference".into(), json!("highPerformance"));
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.gpu_preference, "highPerformance");
        let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
        assert_eq!(saved["engine_prefs"]["gpuPreference"], json!("highPerformance"));
    }

    /// Units › Numbers Without Units Are Points (#394): the preference reaches the fields when it
    /// changes, and its checkbox is dimmed unless a unit is Picas.
    #[test]
    fn numbers_without_units_are_points_reaches_the_fields_and_dims_without_picas() {
        let mut a = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |a: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                apply_runtime(a, ui.ctx());
                show(a, ui.ctx());
            });
            out.textures_delta.clear();
        };
        let read = |ctx: &egui::Context| widgets::typed_unit(ctx, vectorcraft_doc::Unit::Picas);
        frame(&mut a);
        assert_eq!(read(&ctx), vectorcraft_doc::Unit::Points, "on by default");
        a.run("prefs.set", json!({"key": "numbersWithoutUnitsArePoints", "value": false})).unwrap();
        frame(&mut a);
        assert_eq!(read(&ctx), vectorcraft_doc::Unit::Picas, "off");
        // The checkbox: dimmed with every unit in points, enabled once a unit is Picas.
        open(&mut a, Some("Units"));
        let d = a.ui.dialog.as_mut().unwrap();
        assert!(!picas_in_use(d), "dimmed in points");
        d.fields.insert("unitsStroke".into(), json!("picas"));
        assert!(picas_in_use(d), "enabled with a unit in picas");
    }

    /// 0.5.0 saved `powerSaving` for everyone (its default): it reads as Automatic, while the
    /// choices this version offers are kept (#502).
    #[test]
    fn a_graphics_processor_this_version_does_not_offer_reads_as_automatic() {
        for (saved, read) in [("powerSaving", "automatic"), ("turbo", "automatic"), ("lowPower", "lowPower"), ("highPerformance", "highPerformance")]
        {
            let mut a = app();
            a.ui.engine_prefs = json!({"gpuPreference": saved});
            restore(&mut a);
            assert_eq!(a.session.prefs.gpu_preference, read, "{saved}");
        }
    }

    fn prefs_with(mode: &str, dark: &str, light: &str) -> Prefs {
        Prefs { appearance_mode: mode.into(), dark_theme: dark.into(), light_theme: light.into(), ..Prefs::default() }
    }

    /// Appearance Mode: Dark and Light show their saved theme whatever the system says; Sync with
    /// system follows it, and shows Dark when it says nothing.
    #[test]
    fn appearance_resolves_both_saved_themes_and_follows_system() {
        use egui::Theme::{Dark, Light};
        let p = prefs_with("auto", "mediumDark", "light");
        assert_eq!(selected_brightness(&p, Some(Dark)), Brightness::MediumDark);
        assert_eq!(selected_brightness(&p, Some(Light)), Brightness::Light);
        assert_eq!(selected_brightness(&p, None), Brightness::MediumDark, "no system answer: Dark");
        let p = prefs_with("auto", "dark", "mediumLight");
        assert_eq!(selected_brightness(&p, Some(Dark)), Brightness::Dark);
        assert_eq!(selected_brightness(&p, Some(Light)), Brightness::MediumLight);
        assert_eq!(selected_brightness(&prefs_with("dark", "dark", "mediumLight"), Some(Light)), Brightness::Dark);
        assert_eq!(selected_brightness(&prefs_with("light", "dark", "mediumLight"), Some(Dark)), Brightness::MediumLight);
        // A theme from the wrong family (hand-edited file) shows the family's default.
        assert_eq!(selected_brightness(&prefs_with("light", "dark", "dark"), None), Brightness::Light);
        // New users keep Medium Dark.
        assert_eq!(selected_brightness(&Prefs::default(), Some(Light)), Brightness::MediumDark);
    }

    /// Sync with system reads the desktop app's service (Linux portal) where egui reports nothing,
    /// and a change shows on the next frame.
    #[test]
    fn auto_follows_the_system_theme_service() {
        let system = std::sync::Arc::new(std::sync::Mutex::new(Some(egui::Theme::Light)));
        let read = system.clone();
        let mut a = app();
        a.services.system_theme = Some(Box::new(move |_| *read.lock().unwrap()));
        let ctx = egui::Context::default();
        a.run("prefs.set", json!({"key": "appearanceMode", "value": "auto"})).unwrap();
        apply_runtime(&mut a, &ctx);
        assert_eq!(a.ui.brightness, Brightness::Light);
        assert!(!Tokens::get(&ctx).dark);
        assert_eq!(ctx.theme(), egui::Theme::Light, "egui's own style follows");
        *system.lock().unwrap() = Some(egui::Theme::Dark);
        apply_runtime(&mut a, &ctx);
        assert_eq!(a.ui.brightness, Brightness::MediumDark);
        assert!(Tokens::get(&ctx).dark);
        *system.lock().unwrap() = None;
        apply_runtime(&mut a, &ctx);
        assert_eq!(a.ui.brightness, Brightness::MediumDark, "no answer: Dark");
    }

    /// The app bar's appearance button (`window.appearanceMode` without a mode) goes Sync with
    /// system, Light, Dark and round again, keeping both theme choices.
    #[test]
    fn appearance_button_cycles_modes_without_losing_theme_choices() {
        let mut a = app();
        a.services.system_theme = Some(Box::new(|_| Some(egui::Theme::Dark)));
        let ctx = egui::Context::default();
        a.run("prefs.set", json!({"values": {"darkTheme": "dark", "lightTheme": "mediumLight"}})).unwrap();
        for (mode, shown) in [("auto", Brightness::Dark), ("light", Brightness::MediumLight), ("dark", Brightness::Dark), ("auto", Brightness::Dark)]
        {
            assert_eq!(a.run("window.appearanceMode", json!({})).unwrap(), json!(mode));
            apply_runtime(&mut a, &ctx);
            assert_eq!(a.session.prefs.appearance_mode, mode);
            assert_eq!(a.ui.brightness, shown, "{mode}");
            assert_eq!((a.session.prefs.dark_theme.as_str(), a.session.prefs.light_theme.as_str()), ("dark", "mediumLight"));
        }
        assert_eq!(a.run("window.appearanceMode", json!({"mode": "light"})).unwrap(), json!("light"));
        assert!(a.run("window.appearanceMode", json!({"mode": "sepia"})).is_err());
        assert_eq!(a.session.prefs.appearance_mode, "light");
        assert!(a.run("window.appearanceMode", json!({"mode": 42})).is_err());
        assert!(a.run("window.appearanceMode", json!([])).is_err());
        assert_eq!(a.session.prefs.appearance_mode, "light");
    }

    /// UI Brightness (menu, `ui.set {brightness}`) shows that brightness and fixes the mode to its
    /// family, keeping the other family's theme.
    #[test]
    fn ui_brightness_menu_fixes_the_mode_to_its_family() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.run("window.appearanceMode", json!({"mode": "auto"})).unwrap();
        a.run("window.brightness", json!({"brightness": "mediumLight"})).unwrap();
        apply_runtime(&mut a, &ctx);
        assert_eq!(a.ui.brightness, Brightness::MediumLight);
        assert_eq!((a.session.prefs.appearance_mode.as_str(), a.session.prefs.light_theme.as_str()), ("light", "mediumLight"));
        a.run("window.brightness", json!({"brightness": "dark"})).unwrap();
        apply_runtime(&mut a, &ctx);
        assert_eq!(a.ui.brightness, Brightness::Dark);
        let p = &a.session.prefs;
        assert_eq!((p.appearance_mode.as_str(), p.dark_theme.as_str(), p.light_theme.as_str()), ("dark", "dark", "mediumLight"));
    }

    /// Preferences saved before appearance modes (only `uiBrightness`) load as that brightness in
    /// a fixed mode, not Sync with system.
    #[test]
    fn saved_single_brightness_loads_as_a_fixed_mode() {
        let mut a = app();
        a.ui.engine_prefs = json!({"uiBrightness": "light", "gridlineEvery": 50});
        restore(&mut a);
        apply_runtime(&mut a, &egui::Context::default());
        assert_eq!((a.session.prefs.appearance_mode.as_str(), a.session.prefs.light_theme.as_str()), ("light", "light"));
        assert_eq!(a.ui.brightness, Brightness::Light);
        assert_eq!(a.session.prefs.gridline_every, 50.0);
    }

    /// User Interface shows the mode above the light and dark theme cards (the one showing marked
    /// Active), and OK saves what they pick.
    #[test]
    fn appearance_block_shows_the_theme_cards_and_ok_saves_them() {
        fn texts(s: &egui::Shape, out: &mut Vec<String>, positions: &mut Vec<(String, egui::Pos2)>) {
            match s {
                egui::Shape::Text(t) => {
                    out.push(t.galley.text().to_string());
                    positions.push((t.galley.text().to_string(), t.pos));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out, positions)),
                _ => {}
            }
        }
        let mut a = app();
        open(&mut a, Some("User Interface"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut shown = vec![];
        let mut positions = vec![];
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
            out.textures_delta.clear();
            shown.clear();
            positions.clear();
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown, &mut positions));
        }
        for want in ["Appearance Mode:", "Light Theme", "Dark Theme", "Active", "Medium Dark", "Medium Light"] {
            assert!(shown.iter().any(|t| t == want), "{want}: {shown:?}");
        }
        assert!(!shown.iter().any(|t| t.starts_with("Brightness")), "the legacy row is hidden: {shown:?}");
        let at = |label| positions.iter().find(|(text, _)| text == label).map(|(_, pos)| *pos).unwrap();
        assert!(at("Light Theme").x < at("Dark Theme").x, "cards sit side by side");
        assert!(at("Medium Dark").y > at("Dark Theme").y + 50.0, "options follow the preview vertically");
        assert!(at("Medium Light").y > at("Light Theme").y + 50.0, "options follow the preview vertically");
        let d = a.ui.dialog.as_mut().unwrap();
        d.fields.insert("appearanceMode".into(), json!("auto"));
        d.fields.insert("lightTheme".into(), json!("mediumLight"));
        confirm(&mut a).unwrap();
        assert_eq!((a.session.prefs.appearance_mode.as_str(), a.session.prefs.light_theme.as_str()), ("auto", "mediumLight"));
        assert_eq!(a.ui.engine_prefs["appearanceMode"], json!("auto"));
    }

    /// The selected category has a painted accent row even when the pointer is idle.
    #[test]
    fn selected_preferences_category_paints_accent() {
        fn has_accent(shape: &egui::Shape, accent: egui::Color32) -> bool {
            match shape {
                egui::Shape::Rect(rect) => rect.fill == accent,
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_accent(shape, accent)),
                _ => false,
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Brightness::MediumDark);
        let tokens = Tokens::for_brightness(Brightness::MediumDark);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            category_button(ui, "User Interface", true, &tokens);
        });
        output.textures_delta.clear();
        assert!(output.shapes.iter().any(|clipped| has_accent(&clipped.shape, tokens.accent_strong)));
    }

    #[test]
    fn every_category_renders_fields() {
        for c in PREF_CATEGORIES {
            assert!(PREF_SPECS.iter().any(|s| s.category == *c));
        }
    }

    /// Preferences › Type › Show Font Names in English (#394): OK on the dialog stores the flag the
    /// font menus read (Character panel, Type → Font).
    #[test]
    fn show_font_names_in_english_dialog_ok_stores_the_flag() {
        let mut a = app();
        assert!(a.session.prefs.font_names_in_english);
        open(&mut a, Some("Type"));
        assert_eq!(a.ui.dialog.as_ref().unwrap().str("__category"), "Type");
        a.ui.dialog.as_mut().unwrap().fields.insert("fontNamesInEnglish".into(), json!(false));
        confirm(&mut a).unwrap();
        assert!(a.ui.dialog.is_none());
        assert!(!a.session.prefs.font_names_in_english);
        assert!(!crate::font_menu::MenuLook::of(&a).english_names);
    }

    /// Preferences › Hyphenation › Exceptions (#394): OK on the dialog pushes the list into the
    /// hyphenator the same way `prefs.set` does (no need to drive the live Preferences window).
    #[test]
    fn hyphenation_exceptions_dialog_ok_reaches_the_hyphenator() {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                vectorcraft_text::set_hyphenation_exceptions("");
            }
        }
        let _clear = Clear;
        let mut a = app();
        open(&mut a, Some("Hyphenation"));
        let d = a.ui.dialog.as_mut().unwrap();
        assert_eq!(d.str("__category"), "Hyphenation");
        d.fields.insert("hyphenationExceptions".into(), json!("typography, hap-pen"));
        confirm(&mut a).unwrap();
        assert!(a.ui.dialog.is_none());
        assert_eq!(a.session.prefs.hyphenation_exceptions, "typography, hap-pen");
        assert_eq!(vectorcraft_text::hyphenation_exceptions(), "typography, hap-pen");
        assert!(vectorcraft_text::hyphen::hyphen_points("typography").is_empty());
        assert_eq!(vectorcraft_text::hyphen::hyphen_points("happen"), vec![3]);
    }
}
