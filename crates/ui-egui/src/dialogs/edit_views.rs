//! View → Edit Views…: the document's saved views in a list, with the chosen one's name to edit
//! and Delete. Nothing changes until OK, which applies every rename and delete as one undo step
//! (`view.saved.edit` `edits`); Cancel leaves them as they were.
//!
//! Fields: `count` and, for each saved view `i`, `orig<i>` (its name when the dialog opened),
//! `name<i>` (as edited) and `deleted<i>`; `selected` (the row picked).

use serde_json::{Value, json};
use vectorcraft_engine::doc::SavedView;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Edit Views.
pub const KIND: &str = "editViews";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Edit Views").into(), body, confirm, min_width: 420.0, ..DialogSpec::FORM };

/// Open Edit Views on the active document's saved views.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let names: Vec<String> = app.session.active().map(|d| d.doc.views.iter().map(|v| v.name.clone()).collect()).unwrap_or_default();
    if names.is_empty() {
        return Err("No saved views (View → New View…)".into());
    }
    let mut fields = json!({ "count": names.len(), "selected": 0 });
    for (i, n) in names.iter().enumerate() {
        fields[format!("orig{i}")] = json!(n);
        fields[format!("name{i}")] = json!(n);
        fields[format!("deleted{i}")] = json!(false);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn count(d: &Dialog) -> usize {
    d.fields.get("count").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0).min(SavedView::MAX)
}

/// The rows still in the list (not deleted).
fn rows(d: &Dialog) -> Vec<usize> {
    (0..count(d)).filter(|i| !d.bool(&format!("deleted{i}"))).collect()
}

fn selected(d: &Dialog, rows: &[usize]) -> Option<usize> {
    let want = d.fields.get("selected").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    want.filter(|w| rows.contains(w)).or_else(|| rows.first().copied())
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let rows = rows(d);
    let current = selected(d, &rows);
    let mut pick = None;
    let mut delete = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(200.0);
            widgets::dim_label(ui, tl!("Saved Views"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("saved-views").min_scrolled_height(180.0).max_height(180.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(180.0);
                    for &i in &rows {
                        if ui.selectable_label(Some(i) == current, d.str(&format!("name{i}"))).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(190.0);
            if let Some(i) = current {
                widgets::dim_label(ui, tl!("Name:"));
                form::text(ui, d, &format!("name{i}"), 190.0);
                ui.add_space(10.0);
                delete = widgets::flat_button(ui, tl!("Delete"), 70.0).clicked();
            }
        });
    });
    if let Some(i) = pick {
        d.fields.insert("selected".into(), json!(i));
    }
    if let (true, Some(i)) = (delete, current) {
        d.fields.insert(format!("deleted{i}"), json!(true));
        // The row below takes its place (the one above, for the last).
        let next = rows.iter().copied().find(|r| *r > i).or_else(|| rows.iter().copied().rfind(|r| *r < i));
        d.fields.insert("selected".into(), json!(next.unwrap_or(0)));
    }
    false
}

/// The edits OK applies: each deleted view, and each one whose name was changed.
fn edits(d: &Dialog) -> Vec<Value> {
    (0..count(d))
        .filter_map(|i| {
            let (orig, name) = (d.str(&format!("orig{i}")), d.str(&format!("name{i}")));
            if d.bool(&format!("deleted{i}")) {
                Some(json!({ "name": orig, "delete": true }))
            } else if !name.trim().is_empty() && name.trim() != orig {
                Some(json!({ "name": orig, "newName": name.trim() }))
            } else {
                None
            }
        })
        .collect()
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let edits = edits(d);
    // A name two views would share keeps the dialog open, to be changed.
    let r = if edits.is_empty() { Ok(Value::Null) } else { app.run("view.saved.edit", json!({ "edits": edits })) };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::VectorcraftApp;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        for n in ["Overview", "Logo", "Text"] {
            app.run("view.saved.new", json!({ "name": n })).unwrap();
        }
        app
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: serde_json::Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.views.iter().map(|v| v.name.clone()).collect()
    }

    #[test]
    fn nothing_changes_until_ok_and_all_edits_are_one_undo_step() {
        let mut app = app();
        super::open(&mut app).unwrap();
        // Rename the first and delete the last.
        set(&mut app, "name0", json!(" Whole page "));
        set(&mut app, "deleted2", json!(true));
        assert_eq!(names(&app), ["Overview", "Logo", "Text"]);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(names(&app), ["Whole page", "Logo"]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(names(&app), ["Overview", "Logo", "Text"]);
    }

    #[test]
    fn cancel_leaves_them_and_a_shared_name_keeps_the_dialog_open() {
        let mut app = app();
        super::open(&mut app).unwrap();
        set(&mut app, "name0", json!("Gone"));
        super::super::cancel(&mut app);
        assert_eq!(names(&app), ["Overview", "Logo", "Text"]);
        super::open(&mut app).unwrap();
        set(&mut app, "name0", json!("Text"));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        assert_eq!(names(&app), ["Overview", "Logo", "Text"]);
        // Deleting the other one frees the name.
        set(&mut app, "deleted2", json!(true));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(names(&app), ["Text", "Logo"]);
    }

    #[test]
    fn it_needs_a_saved_view() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        assert!(super::open(&mut app).is_err());
        assert!(app.ui.dialog.is_none());
    }
}
