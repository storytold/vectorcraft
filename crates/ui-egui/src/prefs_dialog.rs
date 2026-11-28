//! Edit → Preferences (⌘K): the framework's Preferences window over the engine's preference table
//! ([`PREF_SPECS`]). The window edits a draft ([`PrefsWindow::draft`]; one key per preference, so agents can
//! `ui.dialog.set` any of them) and OK applies the rows changed in it through `prefs.set`. Also: persistence of engine prefs inside
//! `UiState` and the per-frame application of the UI-side preferences ([`runtime`]).

// PLAN: assumed import paths of the framework items below through the `craft_ui` facade (it re-exports the dialog
// host, the standard prompt, the colour picker dialog, `Bind`, `SaveKey` and the preferences pattern).
use craft::color_ui::ui::ColorPickerDialog;
use craft::host::{PickRequest, Picked};
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogHost, DialogId, PromptChoice, StandardPrompt};
use craft::ui::patterns::preferences::{PreferencesDraft as _, PreferencesState, PreferencesWindow, SettingsDraft};
use craft::ui::runtime::memory::SaveKey;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::{PREF_CATEGORIES, PREF_GROUPS, spec};

use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::i18n::LangExt as _;

mod descriptions;
mod model;
mod runtime;
mod script_access;

pub use runtime::*;

use model::Live;

/// UI-only fields shown in the dialog (backed by `UiState`, not engine prefs).
pub const UI_FIELDS: &[(&str, &str, &str)] = &[
    ("__smartGuides", "Smart Guides", "Smart Guides (View → Smart Guides)"),
    ("__snapToGrid", "Guides & Grid", "Snap to Grid"),
    ("__taskBar", "General", "Enable Contextual Task Bar"),
];

/// Whether User Interface shows VectorCraft Title Bar on Every Desktop (`appTitleBar`): Linux and the
/// BSDs, where the desktop may draw the window's frame. macOS and Windows never read it.
const APP_TITLE_BAR_SHOWN: bool = cfg!(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")));

/// The open Preferences window: the framework's page state, the draft being edited and the dialogs it opened.
pub struct PrefsWindow {
    state: PreferencesState,
    draft: SettingsDraft,
    /// The draft as it was when the window opened or last applied: Apply is enabled while the draft differs.
    opened: SettingsDraft,
    /// Each row's value as the window last read it from the preferences, the document or the UI state. OK and Apply
    /// write only the rows whose draft value differs from it ([`Self::edited`]), so a setting changed outside the
    /// window while it is open keeps its new value.
    source: SettingsDraft,
    /// The Units ▸ General value Reset Preferences put in the draft, while it is still the row's value: OK then
    /// leaves the open document's units alone.
    reset_units: Option<String>,
    /// The rows set in the window (by the user, `ui.dialog.set` or a reset) since it last read them. OK writes them
    /// even when they hold the value they were read with: a reset of a row a command changed while the window is
    /// open, or of Units ▸ General, which shows the document's units, is still written.
    touched: std::collections::BTreeSet<String>,
    /// The colour picker and the Reset question, drawn in the window's own viewport.
    nested: DialogHost,
    reset_asked: Option<DialogId>,
    /// The draft's text and choice values, which the model's rows borrow (the draft hands out owned values only).
    strings: std::collections::BTreeMap<String, String>,
    /// Structured graphics choices and their borrowed native-menu ids, from the app's snapshot.
    graphics: model::GraphicsChoices,
    /// General › Script File Access: the file access kept with Always, read when the window opens; `None` where the
    /// session keeps none (no settings folder, and the web).
    script_access: Option<script_access::Access>,
}

impl PrefsWindow {
    /// Writes `value` at `key`. `Err` for a key that is no preference or a value of another kind than the
    /// preference holds. Choosing another General unit after Reset Preferences makes OK set the document's units
    /// again.
    fn put(&mut self, key: &str, value: &Value) -> Result<(), String> {
        if !is_known(key) {
            return Err(format!("unknown preference `{key}`"));
        }
        if let Some(old) = self.draft.get::<Value>(key)
            && std::mem::discriminant(&old) != std::mem::discriminant(value)
        {
            return Err(format!("`{key}` takes a value of another kind than {value}"));
        }
        if key == "unitsGeneral" && self.reset_units.as_ref().is_some_and(|u| value.as_str() != Some(u)) {
            self.reset_units = None;
        }
        self.set_row(key, value);
        self.sync_strings();
        Ok(())
    }

    /// Whether the row `key` was changed in the window since it was last read: it was set there
    /// ([`Self::touched`]), or its draft value differs from [`Self::source`]. Numbers compare by value, so `4` and
    /// `4.0` are the same.
    fn edited(&self, key: &str) -> bool {
        if self.touched.contains(key) {
            return true;
        }
        let (now, read) = (self.draft.get::<Value>(key), self.source.get::<Value>(key));
        match (now.as_ref().and_then(Value::as_f64), read.as_ref().and_then(Value::as_f64)) {
            (Some(a), Some(b)) => a != b,
            _ => now != read,
        }
    }

    /// Writes `value` at `key` in the draft and in [`Self::source`]: the row shows a value read from outside the
    /// window, and OK does not write it back.
    fn read(&mut self, key: &str, value: &Value) {
        self.draft.set(key, value);
        self.source.set(key, value);
        self.touched.remove(key);
    }

    fn explicitly_edited(&self, key: &str) -> bool {
        if self.touched.contains(key) {
            return true;
        }
        let (now, opened) = (self.draft.get::<Value>(key), self.opened.get::<Value>(key));
        match (now.as_ref().and_then(Value::as_f64), opened.as_ref().and_then(Value::as_f64)) {
            (Some(a), Some(b)) => a != b,
            _ => now != opened,
        }
    }

    fn read_external(&mut self, key: &str, value: &Value) {
        if self.explicitly_edited(key) {
            self.source.set(key, value);
        } else {
            self.read(key, value);
            self.opened.set(key, value);
        }
    }

    /// Sets the row `key` in the window, as the user or a reset does.
    fn set_row(&mut self, key: &str, value: &Value) {
        self.draft.set(key, value);
        self.touched.insert(key.to_string());
    }

    /// Reads the draft's text and choice values again, after any write to the draft.
    fn sync_strings(&mut self) {
        use vectorcraft_engine::cmd::prefscmds::{PREF_SPECS, PrefKind};
        self.strings = PREF_SPECS
            .iter()
            .filter(|s| matches!(s.kind, PrefKind::Text | PrefKind::Choice(_)))
            .filter_map(|s| Some((s.key.to_string(), self.draft.get::<String>(s.key)?)))
            .collect();
        self.graphics.sync(&self.draft.get::<craft_gpu_device::Preference>("gpuPreference").unwrap_or_default());
    }

    /// Writes every pref of `prefs` the draft already holds, as after an import; Units ▸ General then shows the
    /// preference, and OK leaves the open document's units alone unless the row is changed.
    fn refresh(&mut self, prefs: &Value) {
        for (k, v) in prefs.as_object().into_iter().flatten() {
            if self.draft.get::<Value>(k).is_some() {
                self.read(k, v);
                self.opened.set(k, v);
            }
        }
        self.reset_units = self.draft.get::<String>("unitsGeneral");
        self.sync_strings();
    }

    /// Sets every key of `keys` to its default; Units ▸ General among them is then the reset value.
    fn reset_keys<'k>(&mut self, keys: impl IntoIterator<Item = &'k str>) {
        let defaults = Prefs::default().to_json();
        for key in keys {
            if let Some(v) = defaults.get(key) {
                self.set_row(key, v);
            }
        }
        if self.draft.get::<String>("unitsGeneral").is_some() {
            self.reset_units = self.draft.get::<String>("unitsGeneral");
        }
        self.sync_strings();
    }
}

/// Whether `key` is a row of the window: a UI-only field, a preference or a preference group.
fn is_known(key: &str) -> bool {
    UI_FIELDS.iter().any(|f| f.0 == key) || spec(key).is_some() || PREF_GROUPS.contains(&key)
}

/// Opens the window on `category` (matched ignoring case; General when it names none). OK writes only the rows
/// changed in the window: a setting changed some other way while it is open (a command, a panel menu, a script)
/// keeps its new value.
pub fn open(app: &mut VectorcraftApp, category: Option<&str>) {
    crate::original_editor::open(app);
    let mut draft = SettingsDraft::default();
    // One key per preference or group (the flattener presets have commands of their own).
    let mut fields: Map<String, Value> = app.session.prefs.to_json().as_object().cloned().unwrap_or_default();
    fields.retain(|k, _| is_known(k));
    // A restored settings file can hold a Hold Shortcut Delay outside its range: the window shows,
    // and OK sets, the delay in use.
    let saved_delay = fields.get("keyHoldDelay").cloned();
    if saved_delay.is_some() {
        fields.insert("keyHoldDelay".into(), json!(app.session.prefs.key_hold_delay()));
    }
    // Units ▸ General shows (and OK sets) the open document's units.
    if let Some(st) = app.session.active() {
        fields.insert("unitsGeneral".into(), json!(st.doc.units.key()));
    }
    for (k, v) in &fields {
        draft.set(k, v);
    }
    draft.set("__smartGuides", &app.ui.view.smart_guides);
    draft.set("__snapToGrid", &app.ui.view.snap_to_grid);
    draft.set("__taskBar", &app.ui.task_bar);
    let selected = category.and_then(|c| PREF_CATEGORIES.iter().find(|x| x.eq_ignore_ascii_case(c))).copied().unwrap_or(PREF_CATEGORIES[0]);
    // The saved delay, so OK writes the delay in use in its place.
    let mut source = draft.clone();
    if let Some(saved) = &saved_delay {
        source.set("keyHoldDelay", saved);
    }
    let script_access = script_access::load(app);
    let mut w = PrefsWindow {
        state: PreferencesState::default(),
        opened: draft.clone(),
        source,
        draft,
        reset_units: None,
        touched: Default::default(),
        nested: DialogHost::new(),
        reset_asked: None,
        strings: Default::default(),
        graphics: model::GraphicsChoices::new(&app.graphics_adapters),
        script_access,
    };
    w.state.selected = selected.to_string();
    w.sync_strings();
    app.prefs = Some(Box::new(w));
}

/// One pass of the window, while it is open. The window leaves `app` for the pass so its model can use the app.
pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let Some(mut w) = app.prefs.take() else { return };
    // Rows drawn by `PathSetting` edit the draft directly.
    w.sync_strings();
    let mut state = std::mem::take(&mut w.state);
    let out = {
        let mut live = Live { app: &mut *app, w: &mut w };
        PreferencesWindow::new(Bind::bound(&mut state)).remember(SaveKey::new("vectorcraft.preferences")).show(ui, &mut live)
    };
    w.state = state;
    if let Some(request) = out.pick_color {
        w.nested.open(ColorPickerDialog::from_request(request).swatch_view(crate::dialogs::color_picker::swatch_view(app)));
    }
    if out.closed.is_none() {
        app.prefs = Some(w);
    }
}

/// OK: validate and apply every preference in the window, then close it. On error nothing changes and the window
/// stays open.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(mut w) = app.prefs.take() else { return Err("no dialog open".into()) };
    match (Live { app: &mut *app, w: &mut w }).apply() {
        Ok(()) => Ok(Value::Null),
        Err(e) => {
            app.prefs = Some(w);
            Err(e)
        }
    }
}

/// Cancel: drops the draft and closes the window.
pub(crate) fn cancel(app: &mut VectorcraftApp) {
    if let Some(mut w) = app.prefs.take() {
        (Live { app: &mut *app, w: &mut w }).cancel();
    }
}

/// Whether the window is open.
pub(crate) fn is_open(app: &VectorcraftApp) -> bool {
    app.prefs.is_some()
}

/// The English name of the category the window shows.
#[cfg(test)]
pub(crate) fn page(app: &VectorcraftApp) -> Option<&str> {
    app.prefs.as_ref().map(|w| w.state.selected.as_str())
}

/// The language the draft chooses, so a change shows before OK; `None` while the window is closed.
pub(crate) fn draft_language(app: &VectorcraftApp) -> Option<crate::i18n::Lang> {
    let code = app.prefs.as_ref()?.draft.get::<String>("interfaceLanguage")?;
    Some(crate::i18n::Lang::from_pref(&code))
}

/// The draft as JSON, plus `__category`, `__search` and General › Script File Access's rows and error
/// (`__scriptAccess`, `__scriptAccessError`, each script and path named as `script.grants` names it to the request
/// being served); `None` while the window is closed.
pub(crate) fn fields(app: &VectorcraftApp) -> Option<Map<String, Value>> {
    let w = app.prefs.as_ref()?;
    let mut fields = w.draft.to::<Map<String, Value>>().unwrap_or_default();
    fields.insert("__category".into(), json!(w.state.selected));
    fields.insert("__search".into(), json!(w.state.search));
    script_access::inspect(w.script_access.as_ref(), &mut fields);
    Some(fields)
}

/// Sets one field as the user editing it would (`ui.dialog.set`). `Err` while the window is closed, for an unknown
/// key or a value of the wrong kind.
pub(crate) fn set_field(app: &mut VectorcraftApp, key: &str, value: Value) -> Result<(), String> {
    let w = app.prefs.as_mut().ok_or("no dialog open")?;
    match key {
        "__category" => {
            let name = value.as_str().ok_or("`__category` takes text")?;
            let category = PREF_CATEGORIES.iter().find(|c| c.eq_ignore_ascii_case(name)).ok_or_else(|| format!("no category `{name}`"))?;
            w.state.selected = category.to_string();
            Ok(())
        }
        "__search" => {
            w.state.search = value.as_str().ok_or("`__search` takes text")?.to_string();
            Ok(())
        }
        _ => w.put(key, &value),
    }
}

/// Asks whether to reset the preferences, as the Reset Preferences button does. OK puts every default in the
/// draft; nothing is applied until OK in Preferences.
#[cfg(test)]
pub(crate) fn ask_reset(app: &mut VectorcraftApp) {
    if let Some(w) = app.prefs.as_mut() {
        ask(w);
    }
}

/// The Reset question over `w`, once.
fn ask(w: &mut PrefsWindow) {
    if w.reset_asked.is_some_and(|id| w.nested.stack().contains(id)) {
        return;
    }
    let question = StandardPrompt::custom(tl!("Reset all preferences to their defaults?"))
        .paragraph(tl!("The defaults are applied when you click OK in Preferences. Cancel there keeps your current preferences."))
        .button(PromptChoice::Proceed, tl!("OK"))
        .button(PromptChoice::Cancel, tl!("Cancel"));
    w.reset_asked = Some(w.nested.open(question));
}

/// Answers the Reset question as the button `choice` would; false when the question is not asked.
pub(crate) fn answer_reset(app: &mut VectorcraftApp, choice: PromptChoice) -> bool {
    let Some(ctx) = app.ui_ctx.clone() else { return false };
    let Some(mut w) = app.prefs.take() else { return false };
    let asked = w.reset_asked;
    if let Some(id) = asked {
        if choice == PromptChoice::Cancel {
            w.nested.cancel_by_id(&ctx, id);
        } else {
            if let Some(prompt) = w.nested.get_mut::<StandardPrompt>() {
                prompt.set_choice(choice);
            }
            w.nested.confirm_by_id(&ctx, id);
        }
    }
    (Live { app: &mut *app, w: &mut w }).take_answers();
    app.prefs = Some(w);
    asked.is_some()
}

/// The file name Export Preferences… suggests.
const PREFS_FILE_NAME: &str = "VectorCraft Preferences.json";

/// Export Preferences…: the applied preferences (`prefs.export`) go to a file picked in a save
/// dialog (the web downloads it). Changes in the window not yet applied with OK are not in it.
fn export_preferences(app: &mut VectorcraftApp) {
    // PLAN: assumed fl.A21 `save_command_output_named(app, id, name, params, then)` returning `Result<Value, String>`.
    let r = crate::io::save_command_output_named(app, "prefs.export", PREFS_FILE_NAME, json!({}), |_, _| Ok(Value::Null));
    if let Err(e) = r
        && e != "cancelled"
    {
        app.status(e);
    }
}

/// Import Preferences…: the preferences in a file picked in an open dialog are applied (`prefs.import`), and the
/// window shows them ([`imported`]). On an error nothing changes and the status bar says why.
fn import_preferences(app: &mut VectorcraftApp) {
    let request = PickRequest::open("preferences").filter("Preferences", &["json"]);
    file_asks::ask(app, request, Owner::Free, |app, picked| {
        let params = match picked {
            Picked::Paths(paths) => {
                let Some(path) = paths.first() else { return Ok(Value::Null) };
                json!({ "path": path.to_string_lossy() })
            }
            Picked::Contents(files) => {
                let Some((_, bytes)) = files.into_iter().next() else { return Ok(Value::Null) };
                // Match prefs.import's file cap before allocating its JSON text parameter.
                if bytes.len() > 1 << 20 {
                    return Err("The preferences file is larger than 1 MB".into());
                }
                let data = String::from_utf8(bytes).map_err(|_| "The preferences file is not valid UTF-8")?;
                json!({ "data": data })
            }
            _ => return Ok(Value::Null),
        };
        match app.run("prefs.import", params) {
            Ok(_) => app.status("Preferences imported."),
            Err(e) => app.status(e),
        }
        Ok(Value::Null)
    });
}

/// After `prefs.import`: the preferences are saved with the UI state, Enable Touch Workspace takes
/// effect when the import changed it from `touch_workspace`, and an open Preferences window shows
/// the imported values.
pub(crate) fn imported(app: &mut VectorcraftApp, touch_workspace: bool) {
    snapshot(app);
    app.prefs_changed = true;
    if app.session.prefs.touch_workspace != touch_workspace {
        crate::workspaces::sync_touch_workspace(app, app.touch_screen);
    }
    let prefs = app.session.prefs.to_json();
    if let Some(w) = app.prefs.as_mut() {
        w.refresh(&prefs);
    }
}

/// The default of each UI-only row ([`UI_FIELDS`]), by key. Enable Contextual Task Bar takes the value the current
/// workspace was saved with, so a reset leaves the workspace as it was saved.
fn default_ui_fields(ui: &crate::UiState) -> [(&'static str, bool); 3] {
    let first = crate::UiState::default();
    let task_bar = crate::workspaces::saved_task_bar(ui);
    [("__smartGuides", first.view.smart_guides), ("__snapToGrid", first.view.snap_to_grid), ("__taskBar", task_bar)]
}

/// After `prefs.reset`, alone or as a batch step, of every category or of one: the UI-only rows ([`UI_FIELDS`]) and
/// the Edit Original app of the categories it reset take their defaults too, and an open Preferences window shows
/// the reset values. Returns whether any reset was pending.
pub(crate) fn apply_prefs_resets(app: &mut VectorcraftApp) -> bool {
    let mut w = app.prefs.take();
    let any = apply_prefs_resets_to(app, w.as_deref_mut());
    if app.prefs.is_none() {
        app.prefs = w;
    }
    any
}

/// [`apply_prefs_resets`] with the open window `w`, which is out of `app` while it draws or applies.
fn apply_prefs_resets_to(app: &mut VectorcraftApp, mut w: Option<&mut PrefsWindow>) -> bool {
    let resets = app.session.take_prefs_resets();
    if resets.is_empty() {
        return false;
    }
    let in_reset = |category: &str| resets.iter().any(|r| r.is_none_or(|c| c == category));
    for (key, value) in default_ui_fields(&app.ui) {
        if !UI_FIELDS.iter().any(|&(k, c, _)| k == key && in_reset(c)) {
            continue;
        }
        match key {
            "__smartGuides" => app.ui.view.smart_guides = value,
            "__snapToGrid" => app.ui.view.snap_to_grid = value,
            _ => app.ui.task_bar = value,
        }
        // The window shows the reset row, so OK keeps it.
        if let Some(w) = w.as_deref_mut() {
            w.read_external(key, &json!(value));
        }
    }
    // And the reset preferences, the groups with a reset of every category; Units ▸ General then leaves the open
    // document's units alone on OK.
    if let Some(w) = w {
        let every = resets.contains(&None);
        let prefs = app.session.prefs.to_json();
        for (k, v) in prefs.as_object().into_iter().flatten() {
            let reset = spec(k).map_or(every && PREF_GROUPS.contains(&k.as_str()), |s| in_reset(s.category));
            if reset && w.draft.get::<Value>(k).is_some() {
                let edited = w.explicitly_edited(k);
                w.read_external(k, v);
                if k == "unitsGeneral" && !edited {
                    w.reset_units = v.as_str().map(str::to_string);
                }
            }
        }
        w.sync_strings();
    }
    // The Edit Original row shows the app's Edit Original choice.
    if spec("editOriginalSystemDefaults").is_some_and(|s| in_reset(s.category)) {
        let before = app.ui.original_editor.clone();
        crate::original_editor::reset_saved(app);
        app.prefs_changed |= before != app.ui.original_editor;
    }
    true
}

#[cfg(test)]
mod tests;
