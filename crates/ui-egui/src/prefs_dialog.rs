//! Edit → Preferences (⌘K): Illustrator-style category list + fields, rendered from the engine's
//! preference table ([`PREF_SPECS`]). The dialog edits a working copy in `Dialog::fields`
//! (one field per preference key, so agents can `ui.dialog.set` any of them) and OK applies it
//! through `prefs.set`. Also: persistence of engine prefs inside `UiState` and the per-frame
//! application of the UI-side preferences (brightness, canvas colour, UI scaling, render threads).

use serde_json::{Map, Value, json};
use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::{GPU_PREFERENCES, PREF_CATEGORIES, PREF_GROUPS, PREF_SPECS, PrefKind, folder_check};

use crate::state::Dialog;
use crate::theme::{self, Accent, Brightness, Scheme, Tokens};
use crate::{VectorcraftApp, widgets};

/// UI-only fields shown in the dialog (backed by `UiState`, not engine prefs).
pub const UI_FIELDS: &[(&str, &str, &str)] = &[
    ("__smartGuides", "Smart Guides", "Smart Guides (View → Smart Guides)"),
    ("__snapToGrid", "Guides & Grid", "Snap to Grid"),
    ("__taskBar", "General", "Enable Contextual Task Bar"),
];

/// The Units ▸ General value Reset Preferences put in the dialog, while it is still the field's value.
const RESET_UNITS: &str = "__resetUnits";

/// Whether User Interface shows VectorCraft Title Bar on Every Desktop (`appTitleBar`): Linux and the
/// BSDs, where the desktop may draw the window's frame. macOS and Windows never read it.
const APP_TITLE_BAR_SHOWN: bool = cfg!(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")));

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
    // Read-only: whether VECTORCRAFT_TITLEBAR chose this window's frame (System Title Bar's dimmed
    // row and VectorCraft Title Bar on Every Desktop's note).
    fields.insert("__titlebarFromEnv".into(), json!(app.titlebar_from_env));
    fields.insert("__search".into(), json!(""));
    fields.insert("__focusSearch".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: "preferences".into(), fields });
}

/// A setting the dialog shows, by its key, category, section ("" for none) and label, in English.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Setting {
    pub key: &'static str,
    pub category: &'static str,
    pub section: &'static str,
    pub label: &'static str,
}

/// Every setting the dialog shows on this platform: the UI-only fields, then the engine's
/// preferences, each category's in the order the dialog lists them.
fn settings() -> impl Iterator<Item = Setting> {
    let ui_fields = UI_FIELDS.iter().map(|&(key, category, label)| Setting { key, category, section: "", label });
    const SYSTEM_TITLE_BAR: bool = cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos")));
    const DISPLAY_SERVER_CHOICE: bool = cfg!(target_os = "linux");
    let shown = |key: &str| match key {
        "systemTitleBar" => SYSTEM_TITLE_BAR,
        "appTitleBar" => APP_TITLE_BAR_SHOWN,
        "linuxDisplayServer" => DISPLAY_SERVER_CHOICE,
        _ => true,
    };
    let prefs =
        PREF_SPECS.iter().filter(move |s| shown(s.key)).map(|s| Setting { key: s.key, category: s.category, section: s.section, label: s.label });
    ui_fields.chain(prefs)
}

/// The settings whose label, section or category contains `query`, ignoring case, in the language
/// the dialog is drawn in or in English, grouped by category in the dialog's order. An empty
/// `query` finds none.
pub(crate) fn search(query: &str) -> Vec<Setting> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let hit = |s: &str| !s.is_empty() && (tl!(s).to_lowercase().contains(&q) || s.to_lowercase().contains(&q));
    let mut found: Vec<Setting> = settings().filter(|s| hit(s.label) || hit(s.section) || hit(s.category)).collect();
    found.sort_by_key(|s| PREF_CATEGORIES.iter().position(|c| *c == s.category).unwrap_or(usize::MAX));
    found
}

/// Goes to the setting `s` a search found: its category is shown, the list scrolls to it and its
/// row is highlighted. The search field is cleared.
fn go_to(d: &mut Dialog, s: &Setting) {
    d.fields.insert("__category".into(), json!(s.category));
    d.fields.insert("__found".into(), json!(s.key));
    d.fields.insert("__scrollTo".into(), json!(true));
    d.fields.insert("__search".into(), json!(""));
}

/// The search field's memory key ([`widgets::search_field`]).
fn search_id() -> egui::Id {
    egui::Id::unique("prefs-search")
}

/// The search field's text edit, which takes the keyboard focus.
pub(crate) fn search_edit_id() -> egui::Id {
    search_id().with("edit")
}

/// Whether the search field had the keyboard focus when the dialog was last drawn.
fn search_focused_id() -> egui::Id {
    egui::Id::unique("prefs-search-focused")
}

/// Escape while the Preferences dialog's search field has text and the keyboard focus: the field is
/// cleared and keeps the focus, and the Escape goes no further. → Whether it did; otherwise Escape
/// does what it does in any dialog.
pub(crate) fn clear_search_on_escape(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == "preferences") else { return false };
    // egui takes the focus from the field when the frame with Escape begins: the field's focus is
    // the one recorded when the dialog was last drawn.
    if d.str("__search").is_empty() || !ctx.data(|m| m.get_temp::<bool>(search_focused_id()).unwrap_or(false)) {
        return false;
    }
    d.fields.insert("__search".into(), json!(""));
    d.fields.insert("__focusSearch".into(), json!(true));
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    true
}

/// The search field above the category list. It takes the keyboard focus when the dialog opens and
/// on Cmd/Ctrl+F. → Its text.
fn search_box(ui: &mut egui::Ui, d: &mut Dialog) -> String {
    let refocus = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
    if refocus {
        d.fields.insert("__focusSearch".into(), json!(true));
    }
    ui.data_mut(|m| m.insert_temp(search_id(), d.str("__search")));
    let query = widgets::search_field(ui, search_id(), tl!("Search Preferences"));
    if d.bool("__focusSearch") {
        // Not on the window's measuring frame, nor in the frame of the click that opened the
        // dialog: egui takes the focus from a field such a click missed.
        if ui.is_sizing_pass() || ui.input(|i| i.pointer.any_click()) {
            ui.ctx().request_repaint();
        } else {
            widgets::focus_field(ui, search_edit_id(), &query);
            d.fields.remove("__focusSearch");
        }
    }
    let focused = ui.memory(|m| m.has_focus(search_edit_id()));
    ui.data_mut(|m| m.insert_temp(search_focused_id(), focused));
    d.fields.insert("__search".into(), json!(query));
    query
}

/// The settings `query` finds, in place of the category list: choosing one goes to it.
fn search_results(ui: &mut egui::Ui, d: &mut Dialog, query: &str) {
    let t = Tokens::get(ui.ctx());
    let found = search(query);
    if found.is_empty() {
        ui.label(egui::RichText::new(tl!("No settings found")).color(t.text_dim).size(12.0));
        return;
    }
    egui::ScrollArea::vertical().id_salt("prefs-search-results").max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        for s in &found {
            let text = egui::RichText::new(tl!(s.label)).size(12.5).color(t.text);
            let place = if s.section.is_empty() { tl!(s.category).to_string() } else { format!("{} › {}", tl!(s.category), tl!(s.section)) };
            let b = egui::Button::selectable(false, text).min_size(egui::vec2(180.0, 22.0));
            if ui.add(b).on_hover_text(&place).clicked() {
                go_to(d, s);
            }
            ui.label(egui::RichText::new(place).color(t.text_dim).size(11.0));
            ui.add_space(3.0);
        }
    });
}

/// Before a setting's row: where to paint the highlight behind it, when it is the setting a search
/// went to ([`go_to`]).
fn found_mark(ui: &mut egui::Ui, d: &Dialog, key: &str) -> Option<(egui::layers::ShapeIdx, f32)> {
    (d.str("__found") == key).then(|| (ui.painter().add(egui::Shape::Noop), ui.cursor().top()))
}

/// After the row [`found_mark`] marked: highlights it, and scrolls the list to it once.
fn found_paint(ui: &mut egui::Ui, d: &mut Dialog, mark: Option<(egui::layers::ShapeIdx, f32)>) {
    let Some((idx, top)) = mark else { return };
    let bottom = ui.min_rect().bottom();
    if bottom <= top {
        return;
    }
    let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=bottom).expand2(egui::vec2(0.0, 2.0));
    let t = Tokens::get(ui.ctx());
    ui.painter().set(idx, egui::epaint::RectShape::filled(rect, 3.0, t.accent.gamma_multiply(0.22)));
    if d.bool("__scrollTo") && !ui.is_sizing_pass() {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        d.fields.remove("__scrollTo");
    }
}

/// `field` of the Preferences dialog `d` is about to be `value` (a frame's edit or `ui.dialog.set`):
/// choosing another General unit after Reset Preferences makes OK set the document's units again.
pub(crate) fn note_set(d: &mut Dialog, field: &str, value: &Value) {
    if field == "unitsGeneral" && d.fields.get(RESET_UNITS).is_some_and(|v| v != value) {
        d.fields.remove(RESET_UNITS);
    }
}

/// OK: validate and apply every preference in the dialog (nothing changes on error).
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let values: Map<String, Value> = d.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let touch_ws = values.get("touchWorkspace").and_then(Value::as_bool);
    let document_units = d.fields.get(RESET_UNITS).is_none_or(|v| values.get("unitsGeneral") != Some(v));
    app.run("prefs.set", json!({ "values": values, "documentUnits": document_units }))?;
    if touch_ws.is_some() {
        crate::workspaces::sync_touch_workspace(app, app.touch_screen);
    }
    app.ui.view.smart_guides = d.fields.get("__smartGuides").and_then(Value::as_bool).unwrap_or(app.ui.view.smart_guides);
    app.ui.view.snap_to_grid = d.fields.get("__snapToGrid").and_then(Value::as_bool).unwrap_or(app.ui.view.snap_to_grid);
    app.ui.task_bar = d.fields.get("__taskBar").and_then(Value::as_bool).unwrap_or(app.ui.task_bar);
    app.ui.dialog = None;
    app.ui.engine_prefs = app.session.prefs.to_json();
    app.prefs_changed = true;
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
    accent: Accent,
    scheme: Scheme,
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

/// Per frame: push UI-side preferences into egui / the renderer when they change.
pub fn apply_runtime(app: &mut VectorcraftApp, ctx: &egui::Context) {
    apply(app, ctx, false);
}

/// [`apply_runtime`]; with `force`, the preferences are pushed into `ctx` even when nothing changed
/// since the last push.
fn apply(app: &mut VectorcraftApp, ctx: &egui::Context, force: bool) {
    let p = &app.session.prefs;
    if let Some(b) = Brightness::parse(&p.ui_brightness)
        && b != app.ui.brightness
    {
        app.ui.brightness = b;
    }
    let want = Applied {
        brightness: app.ui.brightness,
        accent: Accent::parse(&p.ui_accent).unwrap_or_default(),
        scheme: Scheme::parse(&p.ui_color_scheme).unwrap_or_default(),
        canvas: canvas_color(&p.canvas_color),
        threads: p.render_threads,
        tool_tips: p.show_tool_tips,
        scrub: p.scrub_numeric_fields,
        bare_points: p.numbers_without_units_are_points,
    };
    let id = applied_id();
    let prev: Option<Applied> = ctx.data(|d| d.get_temp::<Option<Applied>>(id)).flatten();
    if force || prev != Some(want) {
        theme::apply_with(ctx, want.brightness, want.accent, want.scheme);
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
    let touch = ctx.input(|i| i.has_touch_screen());
    if touch != app.touch_screen {
        app.touch_screen = touch;
        crate::workspaces::sync_touch_workspace(app, touch);
    }
}

/// Where [`apply_runtime`] keeps the preferences it last pushed into `ctx`.
fn applied_id() -> egui::Id {
    egui::Id::unique("dc-applied-prefs")
}

/// [`apply_runtime`] even when nothing changed, so the theme (brightness, accent, color scheme and
/// canvas color) and the other UI-side preferences are pushed into `ctx` again. The record of what
/// was last applied stays, so a Render Threads change since then still restarts the renderer.
pub(crate) fn reapply(app: &mut VectorcraftApp, ctx: &egui::Context) {
    apply(app, ctx, true);
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut cancel, mut reset) = (false, false, false);
    let cat = d.str("__category");
    let cat = PREF_CATEGORIES.iter().find(|c| **c == cat).copied().unwrap_or(PREF_CATEGORIES[0]);
    crate::dialogs::modal::show(ctx, tl!("Preferences"), egui::Id::unique("dialog-preferences"), -20.0, 20, |ui| {
        ui.set_width(760.0);
        crate::dialogs::modal::heading(ui, tl!("Preferences"));
        ui.add_space(12.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(208.0);
                // Search, above the category list; while it has text, the settings it finds show
                // in place of the list.
                let query = search_box(ui, &mut d);
                ui.add_space(6.0);
                // Category list.
                let list = egui::Frame::NONE.fill(t.panel_darker).corner_radius(egui::CornerRadius::same(4)).inner_margin(egui::Margin::same(6));
                list.show(ui, |ui| {
                    ui.set_width(196.0);
                    ui.set_min_height(396.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if !query.trim().is_empty() {
                            search_results(ui, &mut d, &query);
                            return;
                        }
                        for c in PREF_CATEGORIES {
                            let sel = *c == cat;
                            let text = egui::RichText::new(tl!(*c)).size(12.5).color(if sel { t.text_strong } else { t.text });
                            let b = egui::Button::selectable(sel, text).min_size(egui::vec2(184.0, 24.0));
                            if ui.add(b).clicked() {
                                d.fields.insert("__category".into(), json!(c));
                            }
                        }
                    });
                });
            });
            ui.add_space(14.0);
            // Fields.
            ui.vertical(|ui| {
                ui.set_width(530.0);
                ui.label(egui::RichText::new(tl!(cat)).font(theme::semibold(14.0)).color(t.text_strong));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt(("prefs", cat)).max_height(400.0).auto_shrink([false, false]).show(ui, |ui| {
                    category_fields(ui, app, &mut d, cat);
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
        // OK resets the Units ▸ General preference and leaves the open document's units alone.
        d.fields.insert(RESET_UNITS.into(), d.fields.get("unitsGeneral").cloned().unwrap_or(Value::Null));
    }
    if let Some(v) = d.fields.get("unitsGeneral").cloned() {
        note_set(&mut d, "unitsGeneral", &v);
    }
    // The setting a search went to stays highlighted until another category is shown.
    let found = d.str("__found");
    if !found.is_empty() && settings().find(|s| s.key == found).is_none_or(|s| s.category != d.str("__category")) {
        d.fields.remove("__found");
        d.fields.remove("__scrollTo");
    }
    app.ui.dialog = Some(d);
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

fn category_fields(ui: &mut egui::Ui, app: &mut VectorcraftApp, d: &mut Dialog, cat: &str) {
    let t = Tokens::get(ui.ctx());
    let mut section = "";
    let mut first = true;
    let specs: Vec<_> = PREF_SPECS.iter().filter(|s| s.category == cat).collect();
    for (key, c, label) in UI_FIELDS {
        if *c == cat && cat != "Smart Guides" {
            let mark = found_mark(ui, d, key);
            bool_row(ui, d, key, label);
            found_paint(ui, d, mark);
        }
    }
    if cat == "Smart Guides" {
        let mark = found_mark(ui, d, "__smartGuides");
        bool_row(ui, d, "__smartGuides", UI_FIELDS[0].2);
        found_paint(ui, d, mark);
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
        let mark = found_mark(ui, d, sp.key);
        match sp.kind {
            // General › Scroll Sideways with Cmd/Ctrl + Wheel acts only while Zoom with Mouse Wheel
            // is off: dimmed while it is on.
            PrefKind::Bool if sp.key == "ctrlWheelScrollsSideways" => {
                ui.add_enabled_ui(!d.bool("zoomWithMouseWheel"), |ui| bool_row(ui, d, sp.key, sp.label));
            }
            // Units › Numbers Without Units Are Points only tells points from picas: dimmed
            // unless a unit is Picas.
            PrefKind::Bool if sp.key == "numbersWithoutUnitsArePoints" => {
                ui.add_enabled_ui(picas_in_use(d), |ui| bool_row(ui, d, sp.key, sp.label));
            }
            // Performance › Animated Zoom needs GPU Performance: dimmed while it is off.
            PrefKind::Bool if sp.key == "animatedZoom" => {
                ui.add_enabled_ui(d.bool("gpuPerformance"), |ui| bool_row(ui, d, sp.key, sp.label));
            }
            // Windows and Linux only: macOS always has the system's title bar, the web has none.
            PrefKind::Bool if sp.key == "systemTitleBar" => {
                if cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos"))) {
                    // VECTORCRAFT_TITLEBAR chooses the frame while it is set: the checkbox is dimmed.
                    ui.add_enabled_ui(!d.bool("__titlebarFromEnv"), |ui| bool_row(ui, d, sp.key, sp.label));
                    // The window's decorations are chosen once, when the app starts. On Linux and the
                    // BSDs the note goes under the next row, which also applies at the next start.
                    if !APP_TITLE_BAR_SHOWN {
                        ui.label(egui::RichText::new(tl!("Applies the next time VectorCraft starts.")).color(t.text_dim).size(11.0));
                    }
                }
            }
            // Linux and the BSDs only, where the desktop may draw the frame: dimmed while System
            // Title Bar is on or VECTORCRAFT_TITLEBAR is set to `app`, `system` or `desktop`, which
            // take precedence over it. A note
            // names each one that is in effect.
            PrefKind::Bool if sp.key == "appTitleBar" => {
                if APP_TITLE_BAR_SHOWN {
                    let from_env = d.bool("__titlebarFromEnv");
                    let system = d.bool("systemTitleBar");
                    ui.add_enabled_ui(!from_env && !system, |ui| bool_row(ui, d, sp.key, sp.label));
                    if system && !from_env {
                        let note = tl!("System Title Bar takes precedence over this option while it is on.");
                        ui.label(egui::RichText::new(note).color(t.text_dim).size(11.0));
                    }
                    let note = if from_env {
                        tl!("The VECTORCRAFT_TITLEBAR environment variable chooses the title bar while it is set.")
                    } else {
                        tl!("Applies the next time VectorCraft starts.")
                    };
                    ui.label(egui::RichText::new(note).color(t.text_dim).size(11.0));
                }
            }
            PrefKind::Bool => bool_row(ui, d, sp.key, sp.label),
            PrefKind::Num { min, max, unit } => {
                labeled(ui, sp.label, |ui, label_id| {
                    let mut x = v.as_f64().unwrap_or(min);
                    let new = if sp.key == "uiScaling" {
                        // Without internal text, both slider controls inherit the row's name across focus changes.
                        let response = widgets::slider(ui, egui::Slider::new(&mut x, min..=max).step_by(0.05)).labelled_by(label_id);
                        ui.add(egui::Label::new(tl!("Smaller ↔ Larger")).wrap_mode(egui::TextWrapMode::Extend));
                        response.changed().then_some(x)
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
                labeled(ui, sp.label, |ui, _| {
                    if let Some(x) = widgets::num_field(ui, sp.key, Some(x), unit, 110.0) {
                        d.fields.insert(sp.key.into(), json!(x.clamp(min, max)));
                    }
                });
            }
            PrefKind::Int { min, max } => {
                labeled(ui, sp.label, |ui, label_id| {
                    let mut x = v.as_i64().unwrap_or(min);
                    let new = if sp.key == "anchorSize" {
                        let response = widgets::slider(ui, egui::Slider::new(&mut x, min..=max).show_value(false)).labelled_by(label_id);
                        ui.add(egui::Label::new(tl!("Size")).wrap_mode(egui::TextWrapMode::Extend));
                        response.changed().then_some(x)
                    } else {
                        widgets::range_field(ui, sp.key, x as f64, min as f64..=max as f64, "", 0, 110.0).map(|x| x as i64)
                    };
                    if let Some(x) = new {
                        d.fields.insert(sp.key.into(), json!(x));
                    }
                });
            }
            // Linux only: the other platforms have a single display server.
            PrefKind::Choice(_) if sp.key == "linuxDisplayServer" && !cfg!(target_os = "linux") => {}
            // The Classic colour scheme has its own accent: dimmed while it is chosen.
            PrefKind::Choice(opts) if sp.key == "uiAccent" => {
                let modern = Scheme::parse(&d.str("uiColorScheme")).unwrap_or_default() == Scheme::Modern;
                ui.add_enabled_ui(modern, |ui| choice_row(ui, d, sp.key, sp.label, opts, &v));
            }
            PrefKind::Choice(opts) => {
                choice_row(ui, d, sp.key, sp.label, opts, &v);
                if sp.key == "gpuPreference" || sp.key == "linuxDisplayServer" {
                    // The window's graphics device and display server are chosen once, when the app starts.
                    ui.label(egui::RichText::new(tl!("Applies the next time VectorCraft starts.")).color(t.text_dim).size(11.0));
                }
                if sp.key == "linuxDisplayServer" {
                    // Many Wayland desktops give a drawing tablet's pen to the window only through XWayland (#2028).
                    ui.label(
                        egui::RichText::new(tl!("If a drawing tablet's pen doesn't draw, choose an X11 (XWayland) option."))
                            .color(t.text_dim)
                            .size(11.0),
                    );
                }
            }
            PrefKind::Color => {
                labeled(ui, sp.label, |ui, label_id| {
                    let hex = v.as_str().unwrap_or("#000000");
                    let c = vectorcraft_color::Color::from_hex(hex).map(|c| c.to_rgba8(1.0)).unwrap_or([0, 0, 0, 255]);
                    let mut rgb = [c[0], c[1], c[2]];
                    if ui.color_edit_button_srgb(&mut rgb).labelled_by(label_id).changed() {
                        d.fields.insert(sp.key.into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
                    }
                    ui.label(egui::RichText::new(hex).color(t.text_dim).size(11.0));
                });
            }
            PrefKind::Text if sp.key == "interfaceLanguage" => {
                // `auto` plus every registered language, by its own name.
                labeled(ui, sp.label, |ui, _| {
                    let cur = v.as_str().unwrap_or("auto");
                    let codes: Vec<&str> = std::iter::once("auto").chain(crate::i18n::Lang::for_picker().into_iter().map(|l| l.code())).collect();
                    let labels: Vec<&str> =
                        std::iter::once(tl!("Auto")).chain(crate::i18n::Lang::for_picker().into_iter().map(|l| l.name())).collect();
                    let cur_label = codes.iter().position(|c| c.eq_ignore_ascii_case(cur)).and_then(|i| labels.get(i).copied()).unwrap_or(cur);
                    if let Some(i) = widgets::dropdown(ui, sp.key, cur_label, &labels, 260.0)
                        && let Some(code) = codes.get(i)
                    {
                        d.fields.insert(sp.key.into(), json!(code));
                    }
                });
            }
            // File Handling › Data Recovery › Folder and Plug-ins & Scratch Disks › Additional
            // Plug-ins Folder: typed, or picked with Choose… where the host has a folder picker
            // (not on the web).
            PrefKind::Text if folder_check(sp.key).is_some() => {
                labeled(ui, sp.label, |ui, _| {
                    let mut s = v.as_str().unwrap_or("").to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).align(egui::Align2::LEFT_CENTER).desired_width(200.0)).changed() {
                        d.fields.insert(sp.key.into(), json!(s));
                    }
                    if app.services.pick_folder.is_some() && ui.button(tl!("Choose…")).clicked() {
                        choose_folder(app, d, sp.key);
                    }
                });
            }
            PrefKind::Text => {
                labeled(ui, sp.label, |ui, _| {
                    let mut s = v.as_str().unwrap_or("").to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).align(egui::Align2::LEFT_CENTER).desired_width(260.0)).changed() {
                        d.fields.insert(sp.key.into(), json!(s));
                    }
                });
            }
        }
        found_paint(ui, d, mark);
    }
}

/// Choose… beside the folder preference `key`: the folder picked becomes the field's value when it
/// passes the preference's [`folder_check`]. Otherwise the status bar says why and the field keeps
/// its value, as it does when the picker is canceled. The answer fills the field even when the
/// active document closed while the picker was open.
fn choose_folder(app: &mut VectorcraftApp, d: &mut Dialog, key: &'static str) {
    let Some(check) = folder_check(key) else { return };
    let r = crate::picks::in_app_dialog(app, d, move |app, d| {
        let Some(path) = crate::picks::folder(app) else { return Ok(()) };
        check(&path)?;
        d.fields.insert(key.into(), json!(path));
        Ok(())
    });
    if let Err(e) = r {
        app.status(e);
    }
}

/// Is one of the dialog's Units (General, Stroke, Type, East Asian Type) Picas?
fn picas_in_use(d: &Dialog) -> bool {
    ["unitsGeneral", "unitsStroke", "unitsType", "unitsAsianType"].iter().any(|k| d.str(k) == vectorcraft_doc::Unit::Picas.key())
}

fn choice_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, opts: &[(&str, &str)], v: &Value) {
    labeled(ui, label, |ui, _| {
        let cur = v.as_str().unwrap_or("");
        let cur_label = opts.iter().find(|o| o.0 == cur).map(|o| o.1).unwrap_or(cur);
        let labels: Vec<&str> = opts.iter().map(|o| o.1).collect();
        if let Some(i) = widgets::dropdown(ui, key, cur_label, &labels, 260.0)
            && let Some(o) = opts.get(i)
        {
            d.fields.insert(key.into(), json!(o.0));
        }
    });
}

fn bool_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    // The app's outlined checkbox (it translates `label`): egui's own has no border at rest.
    let on = d.bool(key);
    if widgets::check(ui, label, on, ui.is_enabled()) {
        d.fields.insert(key.into(), json!(!on));
    }
}

fn labeled(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui, egui::Id)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let label = ui
            .allocate_ui_with_layout(egui::vec2(210.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(210.0);
                ui.add(egui::Label::new(egui::RichText::new(format!("{}{}", tl!(label), tl!(":"))).color(t.text)).truncate())
            })
            .inner;
        ui.label_inputs_by(label.id, |ui| add(ui, label.id));
    });
}

#[cfg(test)]
mod tests;
