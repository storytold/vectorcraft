//! Preferences › General › Script File Access: the file access people allowed scripts with Always
//! Allow This Script in a script's file dialog (`script.grants`), one row for each script and path,
//! each with a Revoke button (`script.grants.revoke`).

use std::collections::BTreeMap;

use craft::theme::TypeRole;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Lazy, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::layout_components::{Row as Line, Stack};
use craft::ui::runtime::focus::FocusRequest;
use serde_json::{Map, Value, json};
use vectorcraft_plugins::permissions;

use crate::VectorcraftApp;

/// The key of the section's row in General, after Show External Script Warning.
pub(super) const KEY: &str = "scriptAccess";

/// The field of `ui.inspect`'s Preferences dialog that holds the rows, `[{script, path, access, versions}]`.
/// It is absent where the session keeps no grants (no settings folder, and the web), and the section is not shown.
pub(super) const ROWS: &str = "__scriptAccess";

/// The field of `ui.inspect`'s Preferences dialog that holds, in English, why the grants file cannot be read
/// (`script.grants`' error), absent when it can.
pub(super) const ERROR: &str = "__scriptAccessError";

/// The most characters of a path a row shows.
const MAX_SHOWN_PATH: usize = 1024;

/// How tall a line of the list is taken to be before it was first shown.
const LINE_ESTIMATE: f32 = 30.0;

/// The height the list offers its lines before it scrolls.
const LIST_HEIGHT: f32 = 180.0;

/// One script and path with kept access.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Grant {
    /// The script file's path, whole, for `script.grants.revoke`.
    script: String,
    /// The allowed file or folder's path, whole, for `script.grants.revoke`.
    path: String,
    /// The script as the request that opened Preferences may see it ([`as_listed`]).
    shown_script: String,
    /// The path as the request that opened Preferences may see it ([`as_listed`]).
    shown_path: String,
    /// The widest access any kept version has: `folder`, `write` or `read`.
    access: &'static str,
    /// How many versions of the script keep access to the path.
    versions: usize,
}

/// The section's state while Preferences is open: the rows, why the grants file cannot be read, and the row whose
/// Revoke button takes the keyboard focus next.
#[derive(Debug, Default)]
pub(super) struct Access {
    rows: Vec<Grant>,
    error: Option<String>,
    focus: Option<usize>,
}

/// The kept grants (`script.grants`), or `None` where the session keeps none (no settings folder, and the web), and
/// the section is not shown. The grants file is the app's own, so it is read with no automation roots in force: each
/// row keeps its script and path whole for `script.grants.revoke`, and shows them as `script.grants` names them to
/// the request that opened Preferences ([`as_listed`]). When the grants file cannot be read, the section holds the
/// error and no rows.
pub(super) fn load(app: &mut VectorcraftApp) -> Option<Access> {
    match vectorcraft_engine::file_access::unconfined(|| app.session.execute("script.grants", &json!({}))) {
        Ok(v) if v["file"].is_null() => None,
        Ok(v) => Some(Access { rows: rows(&v["grants"]), error: None, focus: None }),
        Err(e) => Some(Access { rows: Vec::new(), error: Some(e.to_string()), focus: None }),
    }
}

/// The rows for the grants `grants` lists: one for each script and path, sorted by script and then by path, with the
/// widest access any kept version of the pair has (`folder`, then `write`, then `read`) and how many versions are
/// kept. A grant without a script or a path, or with an empty one, which `script.grants.revoke` does not take, is
/// left out.
fn rows(grants: &Value) -> Vec<Grant> {
    let rank = |access: &str| match access {
        "folder" => 2,
        "write" => 1,
        _ => 0,
    };
    let mut pairs: BTreeMap<(&str, &str), (&'static str, usize)> = BTreeMap::new();
    for g in grants.as_array().into_iter().flatten() {
        let (Some(script), Some(path)) = (g["script"].as_str(), g["path"].as_str()) else { continue };
        if script.is_empty() || path.is_empty() {
            continue;
        }
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
        .map(|((script, path), (access, versions))| Grant {
            script: script.to_string(),
            path: path.to_string(),
            shown_script: as_listed(script).to_string(),
            shown_path: as_listed(path).to_string(),
            access,
            versions,
        })
        .collect()
}

/// `path` as a row shows it: control and bidirectional formatting characters written as escapes, and at most
/// [`MAX_SHOWN_PATH`] characters.
pub(super) fn shown(path: &str) -> String {
    permissions::display_text(path, MAX_SHOWN_PATH)
}

/// `path` as a row's tooltip and its Revoke button's screen-reader description give it: control and bidirectional
/// formatting characters written as escapes, and whole, so two paths that a row shows alike stay apart.
fn shown_whole(path: &str) -> String {
    permissions::display_text(path, usize::MAX)
}

/// A kept script or path as `script.grants` names it to the request being served: whole where that request may read
/// it, else its file name alone (empty for a path that ends in no name).
fn as_listed(path: &str) -> &str {
    if vectorcraft_engine::file_access::check_read(path).is_ok() {
        return path;
    }
    let name = path.rsplit(vectorcraft_doc::links::path_separators(path)).next().unwrap_or_default();
    if matches!(name, "." | "..") { "" } else { name }
}

/// Puts the section in `fields`, the Preferences dialog `ui.inspect` shows: the rows in [`ROWS`], each script and
/// path as [`as_listed`] gives it to the request being served, and the error in [`ERROR`].
pub(super) fn inspect(access: Option<&Access>, fields: &mut Map<String, Value>) {
    let Some(access) = access else { return };
    let rows = access
        .rows
        .iter()
        .map(|g| json!({"script": as_listed(&g.script), "path": as_listed(&g.path), "access": g.access, "versions": g.versions}))
        .collect();
    fields.insert(ROWS.into(), Value::Array(rows));
    if let Some(error) = &access.error {
        fields.insert(ERROR.into(), json!(error));
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
enum Entry {
    Script(usize),
    Grant(usize),
}

/// The section: its heading, each script, its paths with their access and a Revoke button, or a line saying there
/// are none. When the grants file cannot be read, its error shows in place of that line. Nothing shows without
/// `access`.
pub(super) fn show(app: &mut VectorcraftApp, access: &mut Option<Access>, ui: &mut egui::Ui) {
    let Some(state) = access.as_mut() else { return };
    Text::new(tl!("Script File Access")).role(TypeRole::BodyStrong).show(ui);
    if let Some(error) = &state.error {
        Text::new(crate::i18n::msg(error).to_string()).tone(Tone::Bad).show(ui);
    }
    if state.rows.is_empty() {
        if state.error.is_none() {
            Text::new(tl!("No script has kept file access.")).shade(Shade::Muted).show(ui);
        }
        return;
    }
    let rows = &state.rows;
    let mut lines = Vec::with_capacity(rows.len() * 2);
    for (i, row) in rows.iter().enumerate() {
        if i == 0 || rows[i - 1].script != row.script {
            lines.push(Entry::Script(i));
        }
        lines.push(Entry::Grant(i));
    }
    let focus = state.focus.take();
    // The body returns the row whose Revoke was clicked; the revoke runs after the list is shown.
    let list = Stack::new().id_salt("prefs-script-access").label(tl!("Script File Access")).fill_width();
    let shown_list = list.scroll(Scroll::vertical()).room_height(LIST_HEIGHT).show(ui, |ui, _| {
        let mut clicked = None;
        // Each line runs only while it is in view, so a long list costs no more per frame than a short one.
        for line in &lines {
            match *line {
                Entry::Script(i) => {
                    let script = &rows[i].shown_script;
                    Lazy::new(("prefs-script", i), LINE_ESTIMATE).show(ui, |ui| {
                        Text::new(shown(script)).role(TypeRole::BodyStrong).elide(Elide::Path).tooltip(shown_whole(script)).show(ui);
                    });
                }
                Entry::Grant(i) => {
                    let row = &rows[i];
                    let shown_line = Lazy::new(("prefs-script-row", i), LINE_ESTIMATE).reveal(focus == Some(i)).show(ui, |ui| {
                        let (script, path) = (shown_whole(&row.shown_script), shown_whole(&row.shown_path));
                        let line = Line::new().id_salt(("prefs-script-row", i)).show(ui, |ui, _| {
                            Text::new(shown(&row.shown_path)).elide(Elide::Path).sizing(Sizing::FillWidth).tooltip(path.clone()).show(ui);
                            Text::new(access_label(row.access)).shade(Shade::Muted).show(ui);
                            // A screen reader names the script and the path along with the button.
                            let mut button = Button::new(tl!("Revoke")).id_salt(("prefs-script-revoke", i)).tooltip(path.clone());
                            button = button.description(format!("{script}: {path}"));
                            if focus == Some(i) {
                                button = button.request_focus(FocusRequest::Take);
                            }
                            button.show(ui).clicked()
                        });
                        line.inner
                    });
                    if shown_line.flatten() == Some(true) {
                        clicked = Some(i);
                    }
                }
            }
        }
        clicked
    });
    let revoke_at = shown_list.inner.flatten();
    Text::new(tl!("Revoke removes the access kept for every version of the script. A folder's access covers everything in it."))
        .shade(Shade::Muted)
        .show(ui);
    if let Some(i) = revoke_at {
        revoke(app, access, i);
    }
}

/// Revoke on the row `i`: every kept grant of its script and path, whole, is removed (`script.grants.revoke`), the
/// rows are read again and the row now at `i`, or the last row, takes the keyboard focus. The status bar says what
/// happened; on an error the rows stay as they were.
pub(super) fn revoke(app: &mut VectorcraftApp, access: &mut Option<Access>, i: usize) {
    let Some(row) = access.as_ref().and_then(|a| a.rows.get(i)).cloned() else { return };
    match app.run("script.grants.revoke", json!({"script": row.script, "path": row.path})) {
        Ok(v) => {
            *access = load(app);
            if let Some(state) = access.as_mut()
                && !state.rows.is_empty()
            {
                state.focus = Some(i.min(state.rows.len() - 1));
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

/// The rows the section shows, as (script, path, access, versions), with the script and path as they show.
#[cfg(test)]
pub(super) fn shown_rows(access: Option<&Access>) -> Vec<(String, String, String, usize)> {
    let rows = access.map_or(&[][..], |a| a.rows.as_slice());
    rows.iter().map(|g| (g.shown_script.clone(), g.shown_path.clone(), g.access.to_string(), g.versions)).collect()
}

/// Why the grants file cannot be read, as the section holds it.
#[cfg(test)]
pub(super) fn error(access: Option<&Access>) -> Option<&str> {
    access?.error.as_deref()
}
