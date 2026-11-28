//! The Preferences window's model: the pages and rows built from the engine's preference table, and what OK, Apply
//! and Cancel do with the draft.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

// PLAN: assumed import paths through the `craft_ui` facade, as in `prefs_dialog.rs`.
use craft::host::{PickKind, PickRequest};
use craft::theme::Rgba;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::form::{SliderField, SliderFieldLayout};
use craft::ui::layout_components::Row;
use craft::ui::patterns::preferences::{GroupSpec, PageSpec, PathSetting, PrefValue, PreferencesDraft, PreferencesModel, RowKind, RowSpec};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Map, Value, json};
use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::{PREF_CATEGORIES, PREF_SPECS, PrefKind, PrefSpec, folder_check, spec};

use super::{APP_TITLE_BAR_SHOWN, PrefsWindow, UI_FIELDS, descriptions, script_access};
use crate::VectorcraftApp;
use crate::i18n::{Lang, LangExt as _};
use craft::color_ui::ui::ColorPickerDialog;
use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};

/// The pixel size every point-stored length field of this port uses.
const PIXELS_PER_INCH: f64 = 72.0;

/// The model of the open window: the app, and the window taken out of it for the pass.
pub(super) struct Live<'a> {
    pub(super) app: &'a mut VectorcraftApp,
    pub(super) w: &'a mut PrefsWindow,
}

/// `text` as a string that lives for the process, once per distinct text: rows borrow their help and search text,
/// and a row's help joins translated notes, so it has no owner to borrow from. The set is bounded by the notes and
/// descriptions in the catalogs.
fn intern(text: String) -> &'static str {
    static POOL: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut pool = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&known) = pool.get(text.as_str()) {
        return known;
    }
    let leaked: &'static str = Box::leak(text.into_boxed_str());
    pool.insert(leaked);
    leaked
}

/// The unit a preference's `unit` text counts, when the framework has a kind for it.
fn counted(unit: &str) -> Option<UnitKind> {
    match unit {
        "°" => Some(UnitKind::Degrees),
        "pt" => Some(UnitKind::Points),
        "px" => Some(UnitKind::Pixels),
        "%" => Some(UnitKind::Percent),
        "min" => Some(UnitKind::Minutes),
        _ => None,
    }
}

/// The note under a setting the desktop app reads once, when it starts.
fn applies_at_start() -> String {
    tl!("Applies the next time VectorCraft starts.").to_string()
}

/// The groups of a page, one per run of equal sections.
struct Groups<'a> {
    groups: Vec<GroupSpec<'a>>,
    last: Option<&'static str>,
}

impl<'a> Groups<'a> {
    fn add(&mut self, section: &'static str, row: RowSpec<'a>) {
        if self.last != Some(section) {
            self.groups.push(GroupSpec::new((!section.is_empty()).then(|| tl!(section)), Vec::new()));
            self.last = Some(section);
        }
        if let Some(group) = self.groups.last_mut() {
            group.rows.push(row);
        }
    }
}

impl Live<'_> {
    fn bool_of(&self, key: &str) -> bool {
        self.w.draft.get::<bool>(key).unwrap_or(false)
    }

    fn num_of(&self, key: &str) -> Option<f64> {
        self.w.draft.get::<f64>(key)
    }

    fn text_of(&self, key: &str) -> &str {
        self.w.strings.get(key).map_or("", String::as_str)
    }

    /// Is one of the Units (General, Stroke, Type, East Asian Type) Picas?
    fn picas_in_use(&self) -> bool {
        ["unitsGeneral", "unitsStroke", "unitsType", "unitsAsianType"].iter().any(|k| self.text_of(k) == vectorcraft_doc::Unit::Picas.key())
    }

    /// Whether the row `key` acts now: a setting that works only while another allows it is dimmed otherwise.
    fn enabled(&self, key: &str) -> bool {
        match key {
            // Only tells points from picas: dimmed unless a unit is Picas.
            "numbersWithoutUnitsArePoints" => self.picas_in_use(),
            // Replaces the tool tips: dimmed while Show Tool Tips is off.
            "showToolCards" => self.bool_of("showToolTips"),
            "animatedZoom" => self.bool_of("gpuPerformance"),
            // Draws the samples that Enable in-menu font previews shows.
            "fontSamplesOnWhite" => self.bool_of("fontPreview"),
            "keyHoldDelay" => self.bool_of("holdShortcuts"),
            // VECTORCRAFT_TITLEBAR chooses the frame while it is set.
            "systemTitleBar" => !self.app.titlebar_from_env,
            // System Title Bar and VECTORCRAFT_TITLEBAR take precedence over it.
            "appTitleBar" => !self.app.titlebar_from_env && !self.bool_of("systemTitleBar"),
            // The Classic colour scheme has its own accent.
            "uiAccent" => crate::theme::Scheme::parse(self.text_of("uiColorScheme")).unwrap_or_default() == crate::theme::Scheme::Modern,
            "editOriginal" => !self.bool_of("editOriginalSystemDefaults"),
            _ => true,
        }
    }

    /// Whether this platform shows the row `key`.
    fn visible(&self, key: &str) -> bool {
        match key {
            // macOS always has the system's title bar, the web has none.
            "systemTitleBar" => cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos"))),
            "appTitleBar" => APP_TITLE_BAR_SHOWN,
            "linuxDisplayServer" => cfg!(target_os = "linux"),
            "monitorProfileDisplay" | "lowLatencyDrawing" => !cfg!(target_arch = "wasm32"),
            // Only where the session keeps script grants: not without a settings folder, and not on the web.
            script_access::KEY => self.w.script_access.is_some(),
            "editOriginal" => self.app.services.check_application.is_some() && self.app.services.open_in_application.is_some(),
            _ => true,
        }
    }

    /// The notes drawn under the row `key`, in order.
    fn notes(&self, key: &str) -> Vec<String> {
        let from_env = self.app.titlebar_from_env;
        match key {
            "systemTitleBar" => {
                let mut notes = Vec::new();
                // The window's decorations are chosen once, when the app starts. On Linux and the BSDs the note is
                // under the next row, which also applies at the next start.
                if !APP_TITLE_BAR_SHOWN {
                    notes.push(applies_at_start());
                }
                // Where the desktop draws the frame, its title bar stays with this option on or off until
                // VectorCraft Title Bar on Every Desktop is turned on.
                if APP_TITLE_BAR_SHOWN && self.app.desktop_keeps_frame && !from_env && !self.bool_of("appTitleBar") {
                    notes.push(
                        tl!(
                            "This desktop draws the window's title bar with this option on or off while VectorCraft Title Bar on Every Desktop is off."
                        )
                        .to_string(),
                    );
                }
                notes
            }
            "appTitleBar" => {
                let mut notes = Vec::new();
                if self.bool_of("systemTitleBar") && !from_env {
                    notes.push(tl!("System Title Bar takes precedence over this option while it is on.").to_string());
                }
                notes.push(if from_env {
                    tl!("The VECTORCRAFT_TITLEBAR environment variable chooses the title bar while it is set.").to_string()
                } else {
                    applies_at_start()
                });
                notes
            }
            "gpuPreference" | "lowLatencyDrawing" => vec![applies_at_start()],
            "linuxDisplayServer" => vec![
                applies_at_start(),
                // Many Wayland desktops give a drawing tablet's pen to the window only through XWayland (#2028).
                tl!("If a drawing tablet's pen doesn't draw, choose an X11 (XWayland) option.").to_string(),
            ],
            "monitorProfileDisplay" => vec![self.app.display_profile.note(self.bool_of(key))],
            _ => Vec::new(),
        }
    }

    /// A row's help: its notes, then the setting's sentence, one paragraph each.
    fn help(&self, key: &str) -> Option<&'static str> {
        let mut paragraphs = self.notes(key);
        paragraphs.extend(descriptions::description(key).map(|d| tl!(d).to_string()));
        (!paragraphs.is_empty()).then(|| intern(paragraphs.join("\n")))
    }

    /// The row `key`: its label, search text, help and state.
    fn row<'s>(&'s self, key: &'s str, label: &'static str, english: &'static str, kind: RowKind<'s>) -> RowSpec<'s> {
        let row = RowSpec::new(key, label, kind).keywords(intern(format!("{english} {key}"))).visible(self.visible(key)).enabled(self.enabled(key));
        match self.help(key) {
            Some(help) => row.help(help),
            None => row,
        }
    }

    /// The row of the preference `sp`.
    fn spec_row(&self, sp: &'static PrefSpec) -> RowSpec<'_> {
        let key = sp.key;
        let label = tl!(sp.label);
        let custom = |label: &'static str| self.row(key, label, sp.label, RowKind::Custom);
        let kind = match sp.kind {
            PrefKind::Bool => RowKind::Bool(self.bool_of(key)),
            PrefKind::Num { .. } | PrefKind::Int { .. } if matches!(key, "uiScaling" | "anchorSize") => return custom(label),
            PrefKind::Num { min, max, unit } => {
                let field = RowKind::float(self.num_of(key).unwrap_or(min), min..=max, 3);
                match counted(unit) {
                    Some(kind) => field.counting(kind),
                    None if !unit.is_empty() => field.unit_text(tl!(unit)),
                    None => field,
                }
            }
            PrefKind::Length { min, max, measure } => {
                // In the window's own Units choice, so a change there shows right away.
                let shown = crate::canvas::rulers::unit_kind(vectorcraft_doc::Unit::named(self.text_of(measure.pref_key())).unwrap_or_default());
                let lengths = LengthUnits::new(UnitKind::Points, shown, PIXELS_PER_INCH);
                let lengths = crate::fields::bare_points(lengths, self.app.session.prefs.numbers_without_units_are_points);
                RowKind::length(self.num_of(key).unwrap_or(min), min..=max, lengths)
            }
            PrefKind::Int { min, max } => RowKind::int(self.w.draft.get::<i64>(key).unwrap_or(min), min..=max),
            PrefKind::Choice(opts) => {
                let value = opts.iter().find(|o| o.0 == self.text_of(key)).map_or("", |o| o.0);
                RowKind::choice(value, opts.iter().map(|o| Entry::new(o.0, tl!(o.1))).collect())
            }
            PrefKind::Color => {
                let hex = self.w.draft.get::<String>(key).unwrap_or_default();
                RowKind::Color(Rgba::from_hex(&hex).unwrap_or(Rgba::BLACK))
            }
            PrefKind::Text if key == "interfaceLanguage" => {
                // `auto` plus every registered language, by its own name.
                let options: Vec<Entry<&str>> = std::iter::once(Entry::new("auto", tl!("Auto")))
                    .chain(Lang::for_picker().into_iter().map(|l| Entry::new(l.code(), l.name())))
                    .collect();
                let value = Lang::for_picker().into_iter().map(Lang::code).find(|c| c.eq_ignore_ascii_case(self.text_of(key))).unwrap_or("auto");
                RowKind::choice(value, options)
            }
            // File Handling › Data Recovery › Folder and Plug-ins & Scratch Disks › Additional Plug-ins Folder:
            // typed, or picked where the host has a folder picker (not on the web).
            PrefKind::Text if folder_check(key).is_some() && crate::file_asks::can(self.app, PickKind::Folder) => {
                return self.row(key, "", sp.label, RowKind::Custom);
            }
            PrefKind::Text => RowKind::Text(self.text_of(key)),
        };
        self.row(key, label, sp.label, kind)
    }

    /// The page of the English category `cat`: the UI-only rows, then the preferences, in sections.
    fn page(&self, cat: &'static str) -> PageSpec<'_> {
        let mut groups = Groups { groups: Vec::new(), last: None };
        for &(key, category, label) in UI_FIELDS.iter().filter(|f| f.1 == cat) {
            let _ = category;
            groups.add("", self.row(key, tl!(label), label, RowKind::Bool(self.bool_of(key))));
        }
        for sp in PREF_SPECS.iter().filter(|s| s.category == cat) {
            groups.add(sp.section, self.spec_row(sp));
            if sp.key == "editOriginalSystemDefaults" {
                groups.add(sp.section, self.row("editOriginal", "", "Edit Original with PhotoCraft", RowKind::Custom));
            }
            if sp.key == "showExternalScriptWarning" {
                let english = "Script File Access revoke read write file folder";
                groups.add(sp.section, self.row(script_access::KEY, tl!("Script File Access"), english, RowKind::CustomRows));
            }
        }
        if cat == PREF_CATEGORIES[0] {
            let tools = self.row("prefsFile", "", "export import reset preferences", RowKind::Custom);
            groups.add("", tools);
        }
        // The id is the English category (the window's `selected`, `prefs_dialog::page`).
        let mut page = PageSpec::new(cat, tl!(cat), Vec::new());
        page.groups = groups.groups;
        page
    }

    /// Sets every key of the draft to its default, the UI-only rows ([`UI_FIELDS`]) included, as the answered
    /// Reset question does.
    fn reset_all(&mut self) {
        let keys: Vec<String> =
            Prefs::default().to_json().as_object().into_iter().flatten().map(|(k, _)| k.clone()).filter(|k| super::is_known(k)).collect();
        self.w.reset_keys(keys.iter().map(String::as_str));
        self.reset_ui_rows(|_| true);
        crate::original_editor::reset(self.app);
    }

    /// Sets the UI-only rows of the categories `reset` names to their defaults.
    fn reset_ui_rows(&mut self, reset: impl Fn(&str) -> bool) {
        for (key, value) in super::default_ui_fields(&self.app.ui) {
            if UI_FIELDS.iter().any(|&(k, c, _)| k == key && reset(c)) {
                self.w.set_row(key, &json!(value));
            }
        }
    }

    /// The answers of the dialogs the window opened: the Reset question and the colour picker.
    pub(super) fn take_answers(&mut self) {
        if let Some(id) = self.w.reset_asked
            && let Some(result) = self.w.nested.take_by_id::<StandardPrompt>(id)
        {
            self.w.reset_asked = None;
            if result.ok().is_some_and(|a| a.choice == PromptChoice::Proceed) {
                self.reset_all();
            }
        }
        while let Some(result) = self.w.nested.take::<ColorPickerDialog>() {
            if let DialogResult::Ok(done) = result {
                self.set_color(&done.key, done.picked.color);
            }
        }
    }

    /// A slider over `range` for the number row `key`, which writes `to_value` of its result.
    fn slider(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        range: std::ops::RangeInclusive<f64>,
        step: f64,
        unit: Option<UnitKind>,
        to_value: fn(f64) -> Value,
    ) -> bool {
        let label = spec(key).map_or("", |s| tl!(s.label));
        let old = self.num_of(key).unwrap_or(*range.start());
        let mut value = old;
        let mut field = SliderField::new(label, Bind::bound(&mut value), range).step(step).layout(SliderFieldLayout::Inline);
        if let Some(unit) = unit {
            field = field.unit_kind(unit);
        }
        field.show(ui);
        let changed = value != old;
        if changed {
            let _ = self.w.put(key, &to_value(value));
        }
        changed
    }
}

impl PreferencesModel for Live<'_> {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        PREF_CATEGORIES.iter().map(|&cat| self.page(cat)).collect()
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        let value = match value {
            PrefValue::Bool(b) => json!(b),
            PrefValue::Int(i) => json!(i),
            PrefValue::Float(f) => json!(f),
            PrefValue::Text(s) | PrefValue::Choice(s) => json!(s),
            // The six hex digits: preferences hold no alpha.
            PrefValue::Color(c) => json!(c.to_hex().chars().take(7).collect::<String>()),
            // A kind of value no row of this window shows.
            _ => return,
        };
        // A row the window shows is always a known key of the right kind.
        let _ = self.w.put(key, &value);
    }

    fn custom_row(&mut self, ui: &mut egui::Ui, key: &str) -> bool {
        match key {
            "uiScaling" => self.slider(ui, key, 0.75..=2.0, 0.05, Some(UnitKind::Times), |v| json!(v)),
            "anchorSize" => self.slider(ui, key, 1.0..=7.0, 1.0, None, |v| json!(v.round() as i64)),
            "editOriginal" => {
                let enabled = self.enabled(key);
                crate::original_editor::show(self.app, ui, enabled);
                false
            }
            script_access::KEY => {
                script_access::show(self.app, &mut self.w.script_access, ui);
                false
            }
            "prefsFile" => {
                let (mut export, mut import, mut reset) = (false, false, false);
                let web = crate::io::is_web(self.app);
                Row::new().wrap(true).show(ui, |ui, _| {
                    export = Button::new(tl!("Export Preferences…")).show(ui).clicked();
                    // Not on the web, whose open dialog answers with the file's contents, not a path to import.
                    if !web {
                        import = Button::new(tl!("Import Preferences…")).show(ui).clicked();
                    }
                    reset = Button::new(tl!("Reset Preferences")).show(ui).clicked();
                });
                if export {
                    super::export_preferences(self.app);
                }
                if import {
                    super::import_preferences(self.app);
                }
                if reset {
                    super::ask(self.w);
                }
                false
            }
            _ => {
                // The folder rows: typed, or picked with Browse….
                let Some(sp) = spec(key) else { return false };
                let changed = PathSetting::new(tl!(sp.label), key, &mut self.w.draft, PickRequest::folder(key)).show(ui).changed;
                self.w.sync_strings();
                changed
            }
        }
    }

    fn custom_page(&mut self, _ui: &mut egui::Ui, _page: &str) -> bool {
        false
    }

    fn reset_page(&mut self, page: &str) {
        let keys: Vec<&str> = PREF_SPECS.iter().filter(|s| s.category == page).map(|s| s.key).collect();
        self.w.reset_keys(keys);
        self.reset_ui_rows(|c| c == page);
        // The Edit Original row of its page.
        if spec("editOriginalSystemDefaults").is_some_and(|s| s.category == page) {
            crate::original_editor::reset(self.app);
        }
    }

    fn nested(&mut self, ui: &mut egui::Ui) {
        self.w.nested.run(ui);
        self.take_answers();
    }

    fn nested_open(&self) -> bool {
        !self.w.nested.is_empty()
    }
}

impl PreferencesDraft for Live<'_> {
    /// A row set in the window counts even when it shows the value it opened with: a reset of a row a command changed
    /// while the window is open is still applied.
    fn differs(&self) -> bool {
        self.w.draft != self.w.opened || !self.w.touched.is_empty()
    }

    fn apply(&mut self) -> Result<(), String> {
        // A `prefs.reset` run since the last frame shows in the draft before OK reads it.
        super::apply_prefs_resets_to(self.app, Some(&mut *self.w));
        // Only the rows changed in the window: a setting changed outside it while it is open keeps its new value.
        let current = self.app.session.prefs.to_json();
        let edited: Map<String, Value> = self
            .w
            .draft
            .to::<Map<String, Value>>()
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| {
                self.w.edited(k)
                    && (k != "keyHoldDelay"
                        || self.w.explicitly_edited(k)
                        || current.get(k).and_then(Value::as_f64) == self.w.source.get::<Value>(k).as_ref().and_then(Value::as_f64))
            })
            .collect();
        let values: Map<String, Value> = edited.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
        let touch_ws = values.get("touchWorkspace").and_then(Value::as_bool);
        let document_units = self.w.reset_units.as_deref().is_none_or(|u| values.get("unitsGeneral").and_then(Value::as_str) != Some(u));
        // A refused folder, typed or picked, comes back here as the engine's message.
        if !values.is_empty() {
            self.app.run("prefs.set", json!({ "values": values, "documentUnits": document_units }))?;
        }
        let row = |key: &str| edited.get(key).and_then(Value::as_bool);
        // When OK enters the Touch workspace, the task bar is set before the switch, so it goes to the workspace being
        // left; otherwise after it, so a return from Touch gives it to the workspace returned to.
        let before_switch = touch_ws.is_some() && crate::workspaces::enters_touch(self.app, self.app.touch_screen);
        if before_switch && let Some(on) = row("__taskBar") {
            self.app.ui.task_bar = on;
        }
        if touch_ws.is_some() {
            crate::workspaces::sync_touch_workspace(self.app, self.app.touch_screen);
        }
        if let Some(on) = row("__smartGuides") {
            self.app.ui.view.smart_guides = on;
        }
        if let Some(on) = row("__snapToGrid") {
            self.app.ui.view.snap_to_grid = on;
        }
        if !before_switch && let Some(on) = row("__taskBar") {
            self.app.ui.task_bar = on;
        }
        let editor = crate::original_editor::commit(self.app);
        // The host saves the settings only when one changed.
        if !edited.is_empty() || editor {
            super::snapshot(self.app);
            self.app.prefs_changed = true;
        }
        // The row shows the task bar in use: after OK enters Touch, that is Touch's, not the value the workspace left
        // took, so a later Apply does not give the workspace left's value to Touch.
        let current = self.app.session.prefs.to_json();
        for (key, value) in current.as_object().into_iter().flatten() {
            if self.w.draft.get::<Value>(key).is_some() {
                self.w.draft.set(key, value);
            }
        }
        if let Some(state) = self.app.session.active() {
            self.w.draft.set("unitsGeneral", &state.doc.units.key());
        }
        self.w.draft.set("__smartGuides", &self.app.ui.view.smart_guides);
        self.w.draft.set("__snapToGrid", &self.app.ui.view.snap_to_grid);
        self.w.draft.set("__taskBar", &self.app.ui.task_bar);
        self.w.opened = self.w.draft.clone();
        self.w.source = self.w.draft.clone();
        self.w.touched.clear();
        self.w.reset_units = None;
        self.w.sync_strings();
        Ok(())
    }

    fn cancel(&mut self) {
        crate::original_editor::cancel(self.app);
    }
}
