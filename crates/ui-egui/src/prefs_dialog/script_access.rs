//! Preferences › General › Script File Access: the file access people allowed scripts with Always
//! Allow This Script in a script's file dialog (`script.grants`), one row for each script and path,
//! each with a Revoke button (`script.grants.revoke`).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use vectorcraft_plugins::permissions;

use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog field that holds the rows, `[{script, path, access, versions}]`. It is absent where
/// the session keeps no grants (no settings folder, and the web), and the section is not shown.
pub(super) const ROWS: &str = "__scriptAccess";

/// The dialog field that holds the row whose Revoke button takes the keyboard focus next.
const FOCUS: &str = "__scriptAccessFocus";

/// The dialog field that holds, in English, why the grants file cannot be read or changed
/// (`script.grants`' `error`), absent when it can.
pub(super) const ERROR: &str = "__scriptAccessError";

/// The most characters of a path a row shows.
const MAX_SHOWN_PATH: usize = 1024;

/// A row's height in the list.
const ROW_HEIGHT: f32 = 30.0;

/// The list's height before it scrolls.
const LIST_HEIGHT: f32 = 180.0;

/// The keys of a row that hold its script and path as it shows them, each with the key that holds
/// it whole.
const WHOLE: [(&str, &str); 2] = [("script", "wholeScript"), ("path", "wholePath")];

/// The kept grants (`script.grants`) as the rows of the field [`ROWS`] in the Preferences dialog's
/// `fields`. Where the session keeps no grants, the field is removed. The grants file is the app's
/// own, so it is read with no automation roots in force: each row keeps its script and path whole
/// (`wholeScript` and `wholePath`) for `script.grants.revoke`, and shows them (`script` and `path`)
/// as `script.grants` names them to the request that opened the dialog ([`as_listed`]). When the
/// grants file cannot be read or changed, its error goes in the field [`ERROR`], with the rows of the
/// grants that could still be read (none when its lock cannot be taken).
pub(super) fn load(app: &mut VectorcraftApp, fields: &mut Map<String, Value>) {
    fields.remove(ERROR);
    match vectorcraft_engine::file_access::unconfined(|| app.session.execute("script.grants", &json!({}))) {
        Ok(v) if !v["file"].is_null() => {
            if let Some(error) = v["error"].as_str() {
                fields.insert(ERROR.into(), json!(error));
            }
            let mut rows = rows(&v["grants"]);
            for row in rows.iter_mut().filter_map(Value::as_object_mut) {
                for (shown, whole) in WHOLE {
                    let Some(path) = row.get(shown).and_then(Value::as_str).map(str::to_string) else { continue };
                    row.insert(shown.into(), json!(as_listed(&path)));
                    row.insert(whole.into(), json!(path));
                }
            }
            fields.insert(ROWS.into(), Value::Array(rows));
        }
        _ => {
            fields.remove(ROWS);
        }
    }
}

/// The rows for the grants `grants` lists: one for each script and path, sorted by script and then
/// by path, with the widest access any kept version of the pair has (`folder`, then `write`, then
/// `read`) and how many versions are kept. A grant without a script or a path is left out.
pub(super) fn rows(grants: &Value) -> Vec<Value> {
    let rank = |access: &str| match access {
        "folder" => 2,
        "write" => 1,
        _ => 0,
    };
    let mut pairs: BTreeMap<(&str, &str), (&str, usize)> = BTreeMap::new();
    for g in grants.as_array().into_iter().flatten() {
        let (Some(script), Some(path)) = (g["script"].as_str(), g["path"].as_str()) else { continue };
        let access = if g["folder"] == true {
            "folder"
        } else if g["write"] == true {
            "write"
        } else {
            "read"
        };
        let pair = pairs.entry((script, path)).or_insert((access, 0));
        if rank(access) > rank(pair.0) {
            pair.0 = access;
        }
        pair.1 += 1;
    }
    pairs
        .into_iter()
        .map(|((script, path), (access, versions))| json!({"script": script, "path": path, "access": access, "versions": versions}))
        .collect()
}

/// `path` as a row shows it: control and bidirectional formatting characters written as escapes,
/// and at most [`MAX_SHOWN_PATH`] characters.
pub(super) fn shown(path: &str) -> String {
    permissions::display_text(path, MAX_SHOWN_PATH)
}

/// `path` as a row's tooltip and its Revoke button's screen-reader description give it: control and
/// bidirectional formatting characters written as escapes, and whole, so two paths that a row shows
/// alike stay apart.
fn shown_whole(path: &str) -> String {
    permissions::display_text(path, usize::MAX)
}

/// A kept script or path as `script.grants` names it to the request being served: whole where that
/// request may read it, else its file name alone (empty for a path that ends in no name).
pub(crate) fn as_listed(path: &str) -> &str {
    if vectorcraft_engine::file_access::check_read(path).is_ok() {
        return path;
    }
    let name = path.rsplit(vectorcraft_doc::links::path_separators(path)).next().unwrap_or_default();
    if matches!(name, "." | "..") { "" } else { name }
}

/// The rows of `ROWS` in a copy of the Preferences dialog's `fields` (`ui.inspect` and
/// `ui.dialog.set`'s reply) with each script and path as [`as_listed`] gives it to the request being
/// served, and without the whole ones.
pub(crate) fn list_as_listed(fields: &mut Map<String, Value>) {
    for row in fields.get_mut(ROWS).and_then(Value::as_array_mut).into_iter().flatten().filter_map(Value::as_object_mut) {
        for (shown, whole) in WHOLE {
            if let Some(path) = row.remove(whole).as_ref().and_then(Value::as_str) {
                row.insert(shown.into(), json!(as_listed(path)));
            }
        }
    }
}

/// The label of a row's access.
fn access_label(access: &str) -> &'static str {
    match access {
        "folder" => tl!("Read and write folder"),
        "write" => tl!("Read and write file"),
        _ => tl!("Read file"),
    }
}

/// A line of the list: the script that the rows below it belong to, or a row, by its index.
enum Line {
    Script(usize),
    Row(usize),
}

/// The section, after General › Show External Script Warning, when the dialog `d` has rows: each
/// script, its paths with their access and a Revoke button, or a line saying there are none. When
/// the grants file cannot be read or changed, its error shows first, and the line saying there are
/// none does not.
pub(super) fn show(ui: &mut egui::Ui, app: &mut VectorcraftApp, d: &mut Dialog) {
    let Some(rows) = d.fields.get(ROWS).and_then(Value::as_array).cloned() else { return };
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Script File Access")).font(theme::semibold(12.5)).color(t.text_strong));
    ui.add_space(2.0);
    let error = d.fields.get(ERROR).and_then(Value::as_str);
    if let Some(error) = error {
        ui.label(egui::RichText::new(crate::i18n::msg(error)).color(t.error).size(11.5));
    }
    if rows.is_empty() {
        if error.is_none() {
            ui.label(egui::RichText::new(tl!("No script has kept file access.")).color(t.text_dim).size(11.5));
        }
        return;
    }
    let mut lines = Vec::with_capacity(rows.len() * 2);
    for (i, row) in rows.iter().enumerate() {
        if i == 0 || rows[i - 1]["wholeScript"] != row["wholeScript"] {
            lines.push(Line::Script(i));
        }
        lines.push(Line::Row(i));
    }
    let focus = d.fields.get(FOCUS).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let mut revoke_at = None;
    // The widths of the access column and of the Revoke button, so the path takes the rest.
    let text_width = |text: &str, font: egui::FontId| ui.painter().layout_no_wrap(text.to_string(), font, t.text).size().x;
    let access_width =
        ["folder", "write", "read"].map(|a| text_width(access_label(a), egui::FontId::proportional(11.5))).into_iter().fold(0.0, f32::max);
    let revoke_width = (text_width(tl!("Revoke"), theme::semibold(12.5)) + 32.0).max(72.0);
    // The list is as tall as its lines, up to LIST_HEIGHT, also at the bottom of the scrolled field
    // list, where a scroll area would otherwise take only the height left in view.
    let spacing = ui.spacing().item_spacing;
    let height = ((ROW_HEIGHT + spacing.y) * lines.len() as f32 - spacing.y).min(LIST_HEIGHT);
    let scroll =
        egui::ScrollArea::vertical().id_salt("prefs-script-access").max_height(LIST_HEIGHT).min_scrolled_height(height).auto_shrink([false, true]);
    scroll.show_rows(ui, ROW_HEIGHT, lines.len(), |ui, range| {
        for line in &lines[range] {
            let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
            match *line {
                Line::Script(i) => {
                    let script = rows[i]["script"].as_str().unwrap_or_default();
                    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let text = egui::RichText::new(shown(script)).color(t.text_strong).size(12.0);
                        ui.add(egui::Label::new(text).truncate()).on_hover_text(shown_whole(script));
                    });
                }
                Line::Row(i) => {
                    let row = &rows[i];
                    let (script, path) = (row["script"].as_str().unwrap_or_default(), row["path"].as_str().unwrap_or_default());
                    let (script, path, path_whole) = (shown_whole(script), shown(path), shown_whole(path));
                    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        // The path, then the access and the Revoke button at the right.
                        let gaps = 12.0 + 2.0 * ui.spacing().item_spacing.x;
                        let path_width = (ui.available_width() - access_width - revoke_width - gaps).max(40.0);
                        let cell = |width: f32| egui::vec2(width, ROW_HEIGHT);
                        ui.allocate_ui_with_layout(cell(path_width), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_width(path_width);
                            ui.add(egui::Label::new(egui::RichText::new(&path).color(t.text)).truncate()).on_hover_text(&path_whole);
                        });
                        ui.add_space(12.0);
                        ui.allocate_ui_with_layout(cell(access_width), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_width(access_width);
                            let access = egui::RichText::new(access_label(row["access"].as_str().unwrap_or_default())).color(t.text_dim).size(11.5);
                            ui.add(egui::Label::new(access).wrap_mode(egui::TextWrapMode::Extend));
                        });
                        let button = widgets::secondary_button(ui, tl!("Revoke"));
                        // A screen reader names the script and the path along with the button.
                        ui.ctx().accesskit_node_builder(button.id, |node| node.set_description(format!("{script}: {path_whole}")));
                        if focus == Some(i) {
                            button.request_focus();
                        }
                        if button.on_hover_text(&path_whole).clicked() {
                            revoke_at = Some(i);
                        }
                    });
                }
            }
        }
    });
    if focus.is_some() {
        d.fields.remove(FOCUS);
    }
    let note = tl!("Revoke removes the access kept for every version of the script. A folder's access covers everything in it.");
    ui.label(egui::RichText::new(note).color(t.text_dim).size(11.0));
    if let Some(i) = revoke_at {
        revoke(app, d, i);
    }
}

/// Revoke on the row `i` of the Preferences dialog `d`: every kept grant of its script and path,
/// whole, is removed (`script.grants.revoke`), the rows are read again and the row now at `i`, or the last
/// row, takes the keyboard focus. The status bar says what happened; on an error the rows stay as
/// they were.
pub(super) fn revoke(app: &mut VectorcraftApp, d: &mut Dialog, i: usize) {
    let Some(row) = d.fields.get(ROWS).and_then(|rows| rows.get(i)).cloned() else { return };
    match app.run("script.grants.revoke", json!({"script": row["wholeScript"], "path": row["wholePath"]})) {
        Ok(v) => {
            load(app, &mut d.fields);
            let left = d.fields.get(ROWS).and_then(Value::as_array).map_or(0, Vec::len);
            if left > 0 {
                d.fields.insert(FOCUS.into(), json!(i.min(left - 1)));
            }
            if v["removed"].as_u64().unwrap_or(0) > 0 {
                app.status("Script file access revoked");
            } else {
                app.status("The script file access was already revoked");
            }
        }
        Err(e) => app.status(e),
    }
}
