//! Type › Smart Punctuation…: which keyboard characters to replace with their typographic forms,
//! where (the selected type or the whole document) and whether to report how many of each were
//! replaced. OK runs `type.smartPunctuation` as one undo step, and the dialog opens on the fields
//! confirmed last.
//!
//! Fields: the switches `fiLigatures`, `flLigatures`, `quotes`, `spaces`, `dashes`, `ellipsis` and
//! `fractions`, `scope` (`selection` or `document`) and `report`.

use serde_json::{Value, json};

use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Smart Punctuation.
pub const KIND: &str = "smartPunctuation";

const CMD: &str = "type.smartPunctuation";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Smart Punctuation").into(), body, confirm, min_width: 300.0, ..DialogSpec::FORM };

/// The replacement options in the dialog's order: (field, the key of their count in the
/// command's reply, label).
fn options() -> [(&'static str, &'static str, &'static str); 7] {
    [
        ("fiLigatures", "fiLigatures", tl!("ff, fi, ffi Ligatures")),
        ("flLigatures", "flLigatures", tl!("ff, fl, ffl Ligatures")),
        ("quotes", "quotes", tl!("Smart Quotes")),
        ("spaces", "spaces", tl!("Smart Spaces")),
        ("dashes", "dashes", tl!("En, Em Dashes")),
        ("ellipsis", "ellipses", tl!("Ellipses")),
        ("fractions", "fractions", tl!("Expert Fractions")),
    ]
}

/// The fields the menu item opens with: those confirmed last, else quotes, dashes and ellipses on
/// (the command's defaults). With nothing selected, the whole document.
pub fn fields(app: &VectorcraftApp) -> Value {
    let mut f = match &app.ui.smart_punctuation {
        Value::Object(o) => Value::Object(o.clone()),
        _ => json!({
            "fiLigatures": false,
            "flLigatures": false,
            "quotes": true,
            "spaces": false,
            "dashes": true,
            "ellipsis": true,
            "fractions": false,
            "scope": "selection",
            "report": false,
        }),
    };
    if !has_selection(app) {
        f["scope"] = json!("document");
    }
    f
}

fn has_selection(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|d| !d.selection.is_empty())
}

/// The command's parameters: the switches and the scope.
fn params(d: &Dialog) -> Value {
    let mut p = serde_json::Map::new();
    for (key, ..) in options() {
        p.insert(key.into(), json!(d.bool(key)));
    }
    p.insert("scope".into(), json!(if d.str("scope") == "document" { "document" } else { "selection" }));
    Value::Object(p)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    for (key, _, label) in options() {
        form::check(ui, d, key, label);
    }
    ui.add_space(8.0);
    widgets::section_header(ui, tl!("Replace In"));
    let selected = has_selection(app);
    let scope = d.str("scope");
    for (value, label, enabled) in [("selection", tl!("Selected Text Only"), selected), ("document", tl!("Entire Document"), true)] {
        let on = if selected { scope == value } else { value == "document" };
        if widgets::radio(ui, label, on, enabled) {
            d.fields.insert("scope".into(), json!(value));
        }
    }
    ui.add_space(8.0);
    form::check(ui, d, "report", tl!("Report Results"));
    false
}

/// Runs the command, keeps the fields for the next time, and with Report Results on lists how many
/// of each chosen replacement it made.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let r = run_and_close(app, CMD, params(d))?;
    app.ui.smart_punctuation = Value::Object(d.fields.clone());
    if d.bool("report") {
        let lines: Vec<String> = options()
            .into_iter()
            .filter(|(key, ..)| d.bool(key))
            .map(|(_, count, label)| format!("{label}: {}", r["replaced"][count].as_u64().unwrap_or(0)))
            .collect();
        super::confirm::tell(app, tl!("Smart Punctuation"), &lines.join("\n"));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app_with_text(text: &str) -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": text, "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn plain(app: &VectorcraftApp, id: u64) -> String {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => String::new(),
        }
    }

    /// The menu item opens the dialog; OK replaces in one undo step, reports each chosen
    /// replacement's count, and the next opening starts from the fields confirmed.
    #[test]
    fn menu_item_opens_the_dialog_and_ok_reports_the_counts() {
        let (mut app, id) = app_with_text("\"Hi\".  1/2 -- off");
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, CMD, json!({}));
        let d = app.ui.dialog.as_mut().expect("Smart Punctuation opens");
        assert_eq!((d.kind.as_str(), d.str("scope")), (KIND, "selection".to_string()));
        for key in ["spaces", "fractions", "fiLigatures", "report"] {
            d.fields.insert(key.into(), json!(true));
        }
        d.fields.insert("ellipsis".into(), json!(false));
        let undo = app.session.doc().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert_eq!(plain(&app, id), "“Hi”. ½ – o\u{FB00}");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        let told = app.ui.dialog.as_ref().expect("Report Results shows the counts");
        assert_eq!(told.kind, crate::dialogs::confirm::MESSAGE);
        assert_eq!(told.str("detail"), "ff, fi, ffi Ligatures: 1\nSmart Quotes: 2\nSmart Spaces: 1\nEn, Em Dashes: 1\nExpert Fractions: 1");
        app.ui.dialog = None;
        crate::menus::invoke(&mut app, CMD, json!({}));
        let again = app.ui.dialog.as_ref().unwrap();
        assert!(again.bool("fractions") && again.bool("report") && !again.bool("ellipsis"));
    }

    /// With nothing selected the dialog replaces in the entire document, and without Report
    /// Results it shows no message. An unchanged document keeps its undo history.
    #[test]
    fn with_nothing_selected_the_whole_document_is_replaced() {
        let (mut app, id) = app_with_text("a--b");
        app.run("select.none", json!({})).unwrap();
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("scope"), "document");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(plain(&app, id), "a–b");
        assert!(app.ui.dialog.is_none());
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::menus::invoke(&mut app, CMD, json!({}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    }
}
