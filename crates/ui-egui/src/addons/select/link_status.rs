//! Select by Link Status in the interface (#5526): Select Linked, Embedded, Missing and Modified
//! (in Object › Plug-ins › Selection Tools and the Links panel's menu; Shift adds to the
//! selection), the Select by Link Status dialog, and the status bar's line saying what was
//! selected and what was left out. Everything goes through `addon.select.byLinkStatus`; the dialog
//! lists what `links.list` reports.

use egui::{Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::menus::Item;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

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
/// How often the statuses are looked at again while the document stays as it is (s): files
/// change on disk without the document knowing.
const RECHECK: f64 = 2.0;
/// The least time between two looks while the document keeps changing (s).
const SETTLE: f64 = 0.5;
const ROW_H: f32 = 22.0;
const LIST_H: f32 = 240.0;
const STATUS_W: f32 = 80.0;
const FORMAT_W: f32 = 70.0;

/// The menu items: one per status, then the dialog.
pub fn menu() -> Vec<Item> {
    let mut v: Vec<Item> = STATUSES.iter().map(|&(status, label, _)| Item::Cmd(label, COMMAND, json!({ "status": status }))).collect();
    v.push(Item::Cmd("Select by Link Status…", COMMAND, Value::Null));
    v
}

/// Which statuses ([`STATUSES`] order) the active document's images and placed documents have,
/// for the document and revision it was read for and when.
#[derive(Clone, Debug, Default)]
pub struct Present {
    key: Option<(u64, u64)>,
    at: f64,
    pub has: [bool; 4],
}

/// The statuses the `links.list` result `list` holds, in [`STATUSES`] order.
pub(crate) fn statuses_in(list: &Value) -> [bool; 4] {
    let rows = list["links"].as_array().map_or(&[][..], Vec::as_slice);
    STATUSES.map(|(status, ..)| rows.iter().any(|r| command_status(r) == status))
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
        if app.session.active().is_none() {
            return Err("no document".into());
        }
        let fields = fields(app);
        app.ui.dialog = Some(Dialog::new(KIND, fields));
        return Ok(json!({ "dialog": KIND }));
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

/// The dialog's fields to start with: every image and placed document (`__rows`, as `links.list`
/// lists them, read once), those selected chosen, All shown, Zoom to Selection on.
pub(crate) fn fields(app: &mut VectorcraftApp) -> Value {
    let list = app.session.execute("links.list", &json!({})).unwrap_or_default();
    let rows: Vec<Value> = list["links"].as_array().cloned().unwrap_or_default();
    let selected: Vec<u64> = app.session.active().map(|st| st.selection.objects.iter().map(|id| id.0).collect()).unwrap_or_default();
    let chosen: Vec<u64> = rows.iter().filter_map(|r| r["id"].as_u64()).filter(|id| selected.contains(id)).collect();
    json!({ "__rows": rows, "ids": chosen, "show": "all", "zoom": true })
}

pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Select by Link Status").to_string(),
    body,
    confirm,
    ok: Some("Select"),
    min_width: 420.0,
    max_width: Some(480.0),
    ok_enabled: Some(|_, d| !chosen(d).is_empty()),
    ..DialogSpec::FORM
};

/// The ids chosen in dialog `d`.
pub(crate) fn chosen(d: &Dialog) -> Vec<u64> {
    d.fields.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect()
}

/// The rows of `rows` the name search `query` (ignoring case) and the Show choice `show` let through.
pub(crate) fn shown<'a>(rows: &'a [Value], query: &str, show: &str) -> Vec<&'a Value> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .filter(|r| show == "all" || command_status(r) == show)
        .filter(|r| q.is_empty() || r["name"].as_str().unwrap_or_default().to_lowercase().contains(&q))
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

/// Tick or untick row `id`.
pub(crate) fn toggle(chosen: &mut Vec<u64>, id: u64) {
    match chosen.iter().position(|c| *c == id) {
        Some(i) => {
            chosen.remove(i);
        }
        None => chosen.push(id),
    }
}

/// A status's label in the list.
fn status_label(status: &str) -> &'static str {
    STATUSES.iter().find(|(s, ..)| *s == status).map_or("", |&(.., label)| tl!(label))
}

/// The dialog: the search, the Show filter, a checkbox row per image shown (name, status, format),
/// All / None with the count chosen, and Zoom to Selection. Returns false: nothing to preview.
fn body(_app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let rows: Vec<Value> = d.fields.get("__rows").and_then(Value::as_array).cloned().unwrap_or_default();
    let query = widgets::search_field(ui, ui.scope_id().with("select-by-link-status-search"), tl!("Search images"));
    d.fields.insert("__search".into(), json!(query));
    ui.add_space(4.0);
    form::choice(ui, d, "show", tl!("Show:"), (60.0, 140.0), &SHOW);
    let show = d.str("show");
    let visible = shown(&rows, &query, &show);
    let mut picked = chosen(d);
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(LIST_H).auto_shrink([false, true]).show(ui, |ui| {
        if visible.is_empty() {
            ui.label(egui::RichText::new(tl!("No images match.")).color(t.text_dim));
        }
        for r in &visible {
            let Some(id) = r["id"].as_u64() else { continue };
            let name = r["name"].as_str().unwrap_or_default();
            let status = status_label(command_status(r));
            let format = r["format"].as_str().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.set_min_height(ROW_H);
                let on = picked.contains(&id);
                let name_w = (ui.available_width() - STATUS_W - FORMAT_W - 40.0).max(60.0);
                let check = ui.scope(|ui| {
                    ui.set_width(name_w + 24.0);
                    widgets::check_name(ui, name, on)
                });
                if check.inner {
                    toggle(&mut picked, id);
                }
                ui.add_sized(vec2(STATUS_W, ROW_H), egui::Label::new(egui::RichText::new(status).color(t.text_dim)).truncate());
                ui.add_sized(vec2(FORMAT_W, ROW_H), egui::Label::new(egui::RichText::new(format).color(t.text_dim)).truncate());
            });
        }
    });
    ui.add_space(4.0);
    let ids: Vec<u64> = visible.iter().filter_map(|r| r["id"].as_u64()).collect();
    ui.horizontal(|ui| {
        if widgets::flat_button_fit(ui, "All", 50.0).clicked() {
            choose_shown(&mut picked, &ids, true);
        }
        if widgets::flat_button_fit(ui, "None", 50.0).clicked() {
            choose_shown(&mut picked, &ids, false);
        }
        let count = crate::i18n::fmt(tl!("Chosen: {n} of {total}"), &[("n", &picked.len().to_string()), ("total", &rows.len().to_string())]);
        ui.label(egui::RichText::new(count).color(t.text_dim));
    });
    d.fields.insert("ids".into(), json!(picked));
    ui.add_space(6.0);
    form::check(ui, d, "zoom", tl!("Zoom to Selection"));
    false
}

/// The command's params for dialog `d`: the rows chosen, whatever their status.
pub(crate) fn params(d: &Dialog) -> Value {
    json!({ "status": STATUSES.map(|(s, ..)| s), "ids": chosen(d), "zoom": d.bool("zoom") })
}

/// Select: select the rows chosen (and zoom to them) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    app.run(COMMAND, params(d))
}

#[cfg(test)]
mod tests;
