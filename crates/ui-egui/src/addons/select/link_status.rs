//! Select by Link Status in the interface (#5526): Select Linked, Embedded, Missing and Modified
//! (in Object › Plug-ins › Selection Tools and the Links panel's menu; Shift adds to the
//! selection), the Select by Link Status dialog, and the status bar's line saying what was
//! selected and what was left out. Everything goes through `addon.select.byLinkStatus`; the dialog
//! lists what `links.list` reports.

use craft::behavior::{ColumnKey, ColumnWidth, EditEvent, keyed};
use craft::i18n::Folder;
use craft::i18n::matcher::Matcher;
use craft::keymap::Mods;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, ToggleColumn, ToggleEdit,
};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id, serde_fields, set_serde_field};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{ListRow, ToggleCell, ToggleState};
use craft::ui::menu::{Alternate, PanelRow};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{finished, label};
use crate::VectorcraftApp;
use crate::menus::cp;

/// Select by Link Status, and its dialog's kind.
pub const COMMAND: &str = "addon.select.byLinkStatus";
pub const KIND: &str = "selectByLinkStatus";
/// The statuses, as the command names them, with their menu items' and the dialog's labels.
pub const STATUSES: [(&str, &str, &str); 4] = [
    ("linked", "Select Linked", "Linked"),
    ("embedded", "Select Embedded", "Embedded"),
    ("missing", "Select Missing", "Missing"),
    ("modified", "Select Modified", "Modified"),
];
/// The dialog's Show choices: (value, label).
const SHOW: [(&str, &str); 5] = [("all", "All"), ("linked", "Linked"), ("embedded", "Embedded"), ("missing", "Missing"), ("modified", "Modified")];
/// The rows the image table shows before it scrolls.
const LIST_ROWS: u16 = 9;
/// How often the statuses are looked at again while the document stays as it is (s): files
/// change on disk without the document knowing.
const RECHECK: f64 = 2.0;
/// The least time between two looks while the document keeps changing (s).
const SETTLE: f64 = 0.5;
/// The table's columns.
const CHECK: ColumnKey = ColumnKey::new("check");
const NAME: ColumnKey = ColumnKey::new("name");
const STATUS: ColumnKey = ColumnKey::new("status");
const FORMAT: ColumnKey = ColumnKey::new("format");

/// The menu items: one per status, then the dialog.
pub fn menu() -> Vec<Node> {
    let mut v: Vec<Node> = STATUSES.iter().map(|&(status, label, _)| cp(label, COMMAND, json!({ "status": status }))).collect();
    v.push(cp("Select by Link Status…", COMMAND, Value::Null));
    v
}

/// The Links panel's four Select rows: each runs its status, and with Shift adds to the selection
/// ([`with_shift`]).
pub(crate) fn rows() -> Vec<PanelRow> {
    STATUSES
        .iter()
        .map(|&(status, ..)| {
            let params = json!({ "status": status });
            let shifted = CommandRef::with_params(COMMAND, with_shift(params.clone()));
            PanelRow::Alternated(CommandRef::with_params(COMMAND, params), Alternate::new(Mods::SHIFT, shifted))
        })
        .collect()
}

/// Dims the Select row of each status no image or placed document has ([`enabled`]), in the states of
/// the commands the Links panel's menu shows.
pub(crate) fn states(app: &VectorcraftApp, states: &mut CommandStates) {
    for &(status, ..) in &STATUSES {
        let params = json!({ "status": status });
        states.set(CommandRef::with_params(COMMAND, params.clone()), CommandState::new().enabled(enabled(app, &params)));
    }
}

/// Which statuses ([`STATUSES`] order) the active document's images and placed documents have,
/// for the document and revision it was read for and when.
#[derive(Clone, Debug, Default)]
pub struct Present {
    key: Option<(u64, u64)>,
    at: f64,
    pub has: [bool; 4],
}

/// The statuses the rows of the `links.list` result `list` that can be selected hold, in
/// [`STATUSES`] order.
pub(crate) fn statuses_in(list: &Value) -> [bool; 4] {
    let rows = canvas_rows(list);
    STATUSES.map(|(status, ..)| rows.iter().any(|r| command_status(r) == status))
}

/// The rows of the `links.list` result `list` of objects in the layer tree, which can be
/// selected (a row with an `owner` is in a symbol, graph design, pattern or opacity mask).
fn canvas_rows(list: &Value) -> Vec<Value> {
    list["links"].as_array().into_iter().flatten().filter(|r| r["owner"].is_null()).cloned().collect()
}

/// The status of a `links.list` row as the command names it (`ok` is `linked`).
fn command_status(row: &Value) -> &str {
    match row["status"].as_str().unwrap_or_default() {
        "ok" => "linked",
        s => s,
    }
}

/// Keep [`Present`] up to date: at once when another document comes to the front, at most every
/// [`SETTLE`] seconds while its revision changes (a drag changes it every frame, and each look
/// asks the file system about every linked file), and every [`RECHECK`] seconds while it has
/// linked files.
pub fn refresh(app: &mut VectorcraftApp, now: f64) {
    let key = app.session.active().map(|st| (st.uid, st.revision));
    let p = &app.ui.link_statuses;
    let linked = p.has.iter().zip(STATUSES).any(|(on, (status, ..))| *on && status != "embedded");
    let other_document = p.key.map(|k| k.0) != key.map(|k| k.0);
    let due = if p.key == key { linked && now - p.at >= RECHECK } else { other_document || now - p.at >= SETTLE };
    if !due {
        return;
    }
    let has = if key.is_some() { app.session.execute("links.list", &json!({})).map(|l| statuses_in(&l)).unwrap_or_default() } else { [false; 4] };
    app.ui.link_statuses = Present { key, at: now, has };
}

/// Is menu item `status` (a [`STATUSES`] value) on: does an image or placed document have it?
pub(crate) fn enabled(app: &VectorcraftApp, p: &Value) -> bool {
    let Some(status) = p.get("status").and_then(Value::as_str) else { return true };
    STATUSES.iter().position(|(s, ..)| *s == status).and_then(|i| app.ui.link_statuses.has.get(i).copied()).unwrap_or(false)
}

/// A menu item's params with Shift held: add to the selection.
pub(crate) fn with_shift(mut p: Value) -> Value {
    if let Some(o) = p.as_object_mut()
        && o.contains_key("status")
    {
        o.insert("add".into(), json!(true));
    }
    p
}

/// Run the command; with `zoom` and something selected, Zoom to Selection; the status bar says
/// what was selected and what was left out. Without params (the menu's "Select by Link Status…")
/// it opens the dialog instead.
pub(crate) fn run(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if p.as_object().is_none_or(serde_json::Map::is_empty) {
        return open(app);
    }
    let r = app.session.execute(COMMAND, p).map_err(|e| e.to_string())?;
    let count = r["count"].as_u64().unwrap_or(0);
    if r["zoom"] == true && count > 0 {
        crate::addons::arrange::zoom(app)?;
    }
    let statuses: Vec<&str> = match &p["status"] {
        Value::Array(v) => v.iter().filter_map(Value::as_str).collect(),
        v => v.as_str().into_iter().collect(),
    };
    let reasons: Vec<&str> = r["skipped"].as_array().map_or(vec![], |v| v.iter().filter_map(|k| k["reason"].as_str()).collect());
    app.status(message(&statuses, count, &reasons));
    Ok(r)
}

/// The status bar's line: `count` objects of `statuses` selected, and the `reasons` the others
/// were left out for (e.g. "Selected 9 missing images; 3 more are hidden or locked").
pub(crate) fn message(statuses: &[&str], count: u64, reasons: &[&str]) -> String {
    let n = count;
    let selected = match statuses {
        ["linked"] => crate::i18n::msg_n(n, "Selected {n} linked image", "Selected {n} linked images", &[]),
        ["embedded"] => crate::i18n::msg_n(n, "Selected {n} embedded image", "Selected {n} embedded images", &[]),
        ["missing"] => crate::i18n::msg_n(n, "Selected {n} missing image", "Selected {n} missing images", &[]),
        ["modified"] => crate::i18n::msg_n(n, "Selected {n} modified image", "Selected {n} modified images", &[]),
        _ => crate::i18n::msg_n(n, "Selected {n} image", "Selected {n} images", &[]),
    };
    let Some(first) = reasons.first() else { return selected };
    // Hidden and locked go together; a template layer, or the isolated group, alone.
    /// The group reason `r` is told in: hidden and locked are one.
    fn kind(r: &str) -> &str {
        match r {
            "hidden" | "locked" => "locked",
            r => r,
        }
    }
    let (one, other) = match kind(first) {
        k if reasons.iter().any(|r| kind(r) != k) => ("{selected}; {n} more can't be selected now", "{selected}; {n} more can't be selected now"),
        "template" => ("{selected}; {n} more is on a template layer", "{selected}; {n} more are on template layers"),
        "isolation" => ("{selected}; {n} more is outside the isolated group", "{selected}; {n} more are outside the isolated group"),
        _ => ("{selected}; {n} more is hidden or locked", "{selected}; {n} more are hidden or locked"),
    };
    crate::i18n::msg_n(reasons.len() as u64, one, other, &[("selected", &selected)])
}

/// What the dialog's controls edit; its serde names are the field names automation sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Fields {
    /// The ids of the rows chosen.
    pub(crate) ids: Vec<u64>,
    /// The Show choice: `all` or a status.
    pub(crate) show: String,
    /// The name search.
    pub(crate) search: String,
    /// Zoom to Selection.
    pub(crate) zoom: bool,
}

/// Select by Link Status: every image and placed document of the layer tree with its status.
pub(crate) struct LinkStatusDialog {
    pub(crate) fields: Fields,
    /// The rows as `links.list` lists them, read once when the dialog opened.
    pub(crate) rows: Vec<Value>,
}

/// Opens the dialog on every image and placed document in the layer tree, those selected chosen, All
/// shown, Zoom to Selection on.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let list = app.session.execute("links.list", &json!({})).unwrap_or_default();
    let rows = canvas_rows(&list);
    let selected: Vec<u64> = app.session.active().map(|st| st.selection.objects.iter().map(|id| id.0).collect()).unwrap_or_default();
    let ids = rows.iter().filter_map(|r| r["id"].as_u64()).filter(|id| selected.contains(id)).collect();
    let fields = Fields { ids, show: "all".into(), search: String::new(), zoom: true };
    app.dialogs().ok_or("no window")?.with(|host| host.open(LinkStatusDialog { fields, rows }));
    Ok(json!({ "dialog": KIND }))
}

/// The rows of `rows` the name search `query` and the Show choice `show` let through, the search
/// folded for the language of `folder`.
pub(crate) fn shown<'a>(rows: &'a [Value], query: &str, show: &str, folder: &Folder) -> Vec<&'a Value> {
    let matcher = Matcher::new(folder, query);
    rows.iter()
        .filter(|r| show == "all" || command_status(r) == show)
        .filter(|r| matcher.score(r["name"].as_str().unwrap_or_default()).is_some())
        .collect()
}

/// All (`on`) or None: choose every row `shown`, or none of them; rows the search or Show hides
/// keep their state.
pub(crate) fn choose_shown(chosen: &mut Vec<u64>, shown: &[u64], on: bool) {
    if on {
        for id in shown {
            if !chosen.contains(id) {
                chosen.push(*id);
            }
        }
    } else {
        chosen.retain(|id| !shown.contains(id));
    }
}

/// A status's label in the list.
fn status_label(status: &str) -> &'static str {
    STATUSES.iter().find(|(s, ..)| *s == status).map_or("", |&(.., label)| tl!(label))
}

/// One row of the dialog's table.
struct Line {
    id: u64,
    name: String,
    status: &'static str,
    format: String,
}

/// The tick that flips a row in a toggle edit of the table's check column: `Update`, `Set` and
/// `Commit` all carry the cell's new state, so a sweep and a press read alike.
fn ticked(edit: &EditEvent<ToggleEdit<u64>>) -> Option<&ToggleEdit<u64>> {
    match edit {
        EditEvent::Update { value, .. } | EditEvent::Set { after: value, .. } | EditEvent::Commit { after: value, .. } if value.column == CHECK => {
            Some(value)
        }
        _ => None,
    }
}

/// The dialog's rows: the search, the Show filter, a table of the images shown (a check, name,
/// status, format), All / None with the count chosen, and Zoom to Selection.
fn body(ui: &mut Ui, fields: &mut Fields, rows: &[Value]) {
    TextField::search(label(tl!("Search images")), Bind::bound(&mut fields.search)).show(ui);
    let shows: Vec<Entry<String>> = SHOW.iter().map(|&(value, text)| Entry::new(value.to_owned(), label(tl!(text)))).collect();
    Field::new(label(tl!("Show:"))).show(ui, |ui, _| {
        Select::new(label(tl!("Show:")), Bind::bound(&mut fields.show), &shows).show(ui);
    });
    let visible = shown(rows, &fields.search, &fields.show, &Env::of(ui).folder());
    let lines: Vec<Line> = visible
        .iter()
        .filter_map(|r| {
            Some(Line {
                id: r["id"].as_u64()?,
                name: r["name"].as_str().unwrap_or_default().to_owned(),
                status: status_label(command_status(r)),
                format: r["format"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect();
    let ids: Vec<u64> = lines.iter().map(|line| line.id).collect();
    let on = fields.ids.clone();
    let columns = vec![
        ColumnSpec::toggle(CHECK, Label::Empty, ToggleColumn::sweepable()),
        ColumnSpec::new(NAME, label(tl!("Name")), ColumnWidth::fill(1.0)),
        ColumnSpec::new(STATUS, label(tl!("Status")), ColumnWidth::Content),
        ColumnSpec::new(FORMAT, label(tl!("Format")), ColumnWidth::Content),
    ];
    let source = keyed(&ids, |id| *id).with_toggled(|id, column| *column == CHECK && on.contains(id));
    let out = Table::<u64>::new(label(tl!("Select by Link Status")), columns)
        .select(SelectMode::None)
        .empty(label(tl!("No images match.")))
        .max_rows(LIST_ROWS)
        .show_items(ui, &source, &mut |id, item| match lines.iter().find(|line| line.id == *id) {
            Some(line) => ListRow::new(line.name.clone())
                .cells(item.cells())
                .toggle(ToggleCell::check(CHECK, ToggleState::of(on.contains(id)), "{row}"))
                .cell(STATUS, line.status)
                .cell(FORMAT, line.format.clone()),
            None => ListRow::new(""),
        });
    for event in out.events {
        if let CollectionEvent::Toggle(edit) = event
            && let Some(tick) = ticked(&edit)
        {
            choose_shown(&mut fields.ids, &[tick.key], tick.on);
        }
    }
    Row::new().show(ui, |ui, _| {
        if Button::new(label(tl!("All"))).show(ui).clicked() {
            choose_shown(&mut fields.ids, &ids, true);
        }
        if Button::new(label(tl!("None"))).show(ui).clicked() {
            choose_shown(&mut fields.ids, &ids, false);
        }
        let count = crate::i18n::fmt(tl!("Chosen: {n} of {total}"), &[("n", &fields.ids.len().to_string()), ("total", &rows.len().to_string())]);
        Text::new(Label::from(count.to_string())).shade(Shade::Muted).show(ui);
    });
    Checkbox::new(Bind::bound(&mut fields.zoom)).label(label(tl!("Zoom to Selection"))).show(ui);
}

impl StackDialog for LinkStatusDialog {
    type Result = Fields;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Fields>> {
        let LinkStatusDialog { fields, rows } = self;
        let can = !fields.ids.is_empty();
        let shown = FormDialog::new(label(tl!("Select by Link Status")), Bind::bound(fields)).ok_label(label(tl!("Select"))).ok_enabled(can).show(
            ui,
            |ui, _, fields| {
                body(ui, fields, rows);
                !fields.ids.is_empty()
            },
        );
        finished(shown)
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Fields> {
        (!self.fields.ids.is_empty()).then(|| self.fields.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        serde_fields(&self.fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        set_serde_field(&mut self.fields, field, value)
    }
}

/// The command's params for the dialog's `fields`: the rows chosen, whatever their status.
pub(crate) fn params(fields: &Fields) -> Value {
    json!({ "status": STATUSES.map(|(s, ..)| s), "ids": fields.ids, "zoom": fields.zoom })
}

/// Each frame: OK selects the rows chosen (and zooms to them), after the dialog closed.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(DialogResult::Ok(fields)) = dialogs.with(|host| host.take_by_id::<LinkStatusDialog>(dialog_id(KIND)))
        && let Err(e) = run(app, &params(&fields))
    {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
