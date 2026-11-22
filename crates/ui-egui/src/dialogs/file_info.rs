//! File → File Info: the document title, author, description, rating, keywords (chips) and
//! copyright, with the created and modified dates.
//!
//! Fields: what `file.info` reports (and takes back) plus `__keyword`, the keyword being typed
//! (Enter or Add turns it into chips; OK keeps it too). OK runs `file.info` with them: one undo
//! step. On an error the dialog stays open.
//!
//! The ≡ menu at the top saves the fields as a metadata template (`file.info.saveTemplate`), imports
//! one (`file.info.applyTemplate`, after a choice of how to merge it into the fields), lists the
//! templates of the metadata templates folder and shows that folder.

use serde_json::{Value, json};
use vectorcraft_doc::CopyrightStatus;
use vectorcraft_doc::metadata::MAX_RATING;

use super::document_setup::{LABEL, choice};
use super::{DialogSpec, form};
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{FilePick, VectorcraftApp, icons, widgets};

pub(super) const KIND: &str = "fileInfo";
/// The keyword being typed.
const PENDING: &str = "__keyword";
/// The template waiting for a merge choice: `{path}` or `{name}`.
const IMPORT: &str = "__import";
/// The merge choice for [`IMPORT`]: `file.info.applyTemplate`'s `mode`.
const IMPORT_MODE: &str = "__importMode";
/// The templates in the metadata templates folder, listed when the menu opens: `[label, file]`
/// pairs, the label being the name, or the file name where two templates share a name.
const TEMPLATES: &str = "__templates";
/// The merge choices, as `file.info.applyTemplate` names them.
const MODES: [&str; 3] = ["replace", "keep", "append"];
/// The extension of a metadata template file.
const XMP_EXT: &str = "xmp";
/// Width of the text fields.
const FIELD: f32 = 320.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("File Info").into(),
    body,
    confirm,
    // OK and Enter wait while a template being imported waits for its merge choice.
    ok_enabled: Some(|_, d| !d.fields.get(IMPORT).is_some_and(Value::is_object)),
    min_width: 500.0,
    max_width: Some(500.0),
    ..DialogSpec::FORM
};

/// Open File Info on the active document's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("file.info", &json!({})).map_err(|e| e.to_string())?;
    fields[PENDING] = json!("");
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    // A keyword typed but not added yet is kept.
    add_keywords(&mut d);
    let r = app.run("file.info", form::params(&d));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// The keywords in `d`.
fn keywords(d: &Dialog) -> Vec<String> {
    d.fields.get("keywords").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// Move the typed keywords (comma- or semicolon-separated) into the list, each once.
fn add_keywords(d: &mut Dialog) {
    let mut list = keywords(d);
    for k in d.str(PENDING).split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        if !list.iter().any(|o| o.to_lowercase() == k.to_lowercase()) {
            list.push(k.to_string());
        }
    }
    d.fields.insert("keywords".into(), json!(list));
    d.fields.insert(PENDING.into(), json!(""));
}

/// What the templates menu asked for, done once the menu is drawn.
enum Act {
    Save,
    Import,
    Template(String),
    Folder,
}

/// A file dialog for metadata templates.
fn xmp_pick() -> FilePick {
    FilePick { filters: vec![FilePick::filter("XMP metadata templates", &[XMP_EXT])], ..Default::default() }
}

/// Whether the session has a metadata templates folder for Templates Folder to show (the web build
/// has none).
fn has_templates_folder(app: &VectorcraftApp) -> bool {
    app.session.metadata_templates_dir().is_some()
}

/// The ≡ templates menu at the top right, and the merge choice for a template it picked.
fn templates_menu(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let can_save = crate::picks::can(app, &PickRequest::Save(xmp_pick()));
    let can_open = crate::picks::can(app, &PickRequest::Open(xmp_pick()));
    let has_folder = has_templates_folder(app);
    let mut act = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            let resp = ui.interact(rect, ui.scope_id().with("file-info-templates"), egui::Sense::click());
            icons::paint(ui, "menu", rect.shrink(1.0), if resp.hovered() { t.text_strong } else { t.text_dim });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Templates")));
            let resp = resp.on_hover_text(tl!("Templates"));
            if resp.clicked() {
                // Listed once as the menu opens, not on every frame it shows.
                let listed = app.session.execute("file.info.templates", &json!({})).map(|r| r["templates"].clone()).unwrap_or_default();
                let listed: Vec<(&str, &str)> =
                    listed.as_array().into_iter().flatten().filter_map(|t| Some((t.get("name")?.as_str()?, t.get("file")?.as_str()?))).collect();
                let entries: Vec<Value> = listed
                    .iter()
                    .map(|&(name, file)| {
                        let shared = listed.iter().filter(|(n, _)| *n == name).count() > 1;
                        json!([if shared { file } else { name }, file])
                    })
                    .collect();
                d.fields.insert(TEMPLATES.into(), json!(entries));
            }
            egui::Popup::menu(&resp).show(|ui| {
                ui.set_min_width(220.0);
                if can_save && widgets::menu_item(ui, tl!("Save Metadata Template…"), true, false) {
                    act = Some(Act::Save);
                }
                if can_open && widgets::menu_item(ui, tl!("Import…"), true, false) {
                    act = Some(Act::Import);
                }
                let entries: Vec<(String, String)> = d
                    .fields
                    .get(TEMPLATES)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str()?.to_string())))
                    .collect();
                if !entries.is_empty() {
                    ui.separator();
                    widgets::menu_scroll(ui, |ui| {
                        for (label, file) in entries {
                            if widgets::menu_item_name(ui, &label, true, false) {
                                act = Some(Act::Template(file));
                            }
                        }
                    });
                }
                if has_folder {
                    ui.separator();
                    if widgets::menu_item(ui, tl!("Templates Folder"), true, false) {
                        act = Some(Act::Folder);
                    }
                }
            });
        })
    });
    let r = match act {
        None => Ok(()),
        // The file dialogs, shown off the UI thread, act on the dialog as it is then.
        Some(Act::Save) => crate::picks::in_dialog(app, d, save_template),
        Some(Act::Import) => crate::picks::in_dialog(app, d, pick_template),
        // `file.info.applyTemplate` takes a listed template's file name as its `name`.
        Some(Act::Template(file)) => {
            d.fields.insert(IMPORT.into(), json!({ "name": file }));
            Ok(())
        }
        Some(Act::Folder) => show_folder(app),
    };
    if let Err(e) = r {
        app.status(e);
    }
    import_options(app, ui, d);
}

/// Save Metadata Template…: the fields as they are in the dialog, to a file picked in a save
/// dialog that starts in the metadata templates folder. A name typed without `.xmp` takes it, so
/// the templates list shows the file; a file already there under that name is replaced only after
/// a question.
fn save_template(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let mut d2 = d.clone();
    add_keywords(&mut d2);
    let folder = app.session.execute("file.info.templates", &json!({ "create": true })).ok().and_then(|r| r["folder"].as_str().map(str::to_string));
    let name = format!("{}.{XMP_EXT}", d2.str("title").trim());
    let pick = FilePick { name, folder, ..xmp_pick() };
    // None: canceled, or the dialog is still open off the UI thread and runs this again.
    let Some(picked) = crate::picks::save(app, &pick) else { return Ok(()) };
    let path = if picked.to_ascii_lowercase().ends_with(&format!(".{XMP_EXT}")) { picked.clone() } else { format!("{picked}.{XMP_EXT}") };
    // The save dialog did not ask about the completed name: ask here, and the answer runs this again.
    if path != picked && crate::picks::ask_to_replace(app, PickRequest::Save(pick), &picked, None, &path) {
        return Ok(());
    }
    app.run("file.info.saveTemplate", json!({ "path": path, "fields": form::params(&d2) })).map(|_| ())
}

/// Import…: a template picked in an open dialog waits for its merge choice.
fn pick_template(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    // None: canceled, or the dialog is still open off the UI thread and runs this again.
    let Some(path) = crate::picks::open(app, &xmp_pick()) else { return Ok(()) };
    d.fields.insert(IMPORT.into(), json!({ "path": path }));
    Ok(())
}

/// Templates Folder: the metadata templates folder (made when it is missing) in the file manager.
fn show_folder(app: &mut VectorcraftApp) -> Result<(), String> {
    let r = app.session.execute("file.info.templates", &json!({ "create": true })).map_err(|e| e.to_string())?;
    let folder = r["folder"].as_str().ok_or("no metadata templates folder here")?.to_string();
    crate::io::open_in_app(app, &folder)
}

/// The merge choice for the template [`IMPORT`] names: OK merges it into the dialog's fields
/// (nothing changes in the document until the dialog's own OK), Cancel drops it.
fn import_options(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let Some(source) = d.fields.get(IMPORT).and_then(Value::as_object).cloned() else { return };
    widgets::subheader(ui, tl!("Import Options"));
    let mode = d.fields.get(IMPORT_MODE).and_then(Value::as_str).filter(|m| MODES.contains(m)).unwrap_or("keep").to_string();
    let labels = [
        tl!("Clear existing fields and use the template's"),
        tl!("Keep existing fields, but replace those the template has"),
        tl!("Keep existing fields, but append those the template has"),
    ];
    for (value, label) in MODES.into_iter().zip(labels) {
        if widgets::radio(ui, label, mode == value, true) {
            d.fields.insert(IMPORT_MODE.into(), json!(value));
        }
    }
    let mut ok = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        ok = widgets::flat_button(ui, tl!("OK"), 64.0).clicked();
        cancel = widgets::flat_button(ui, tl!("Cancel"), 64.0).clicked();
    });
    ui.add_space(10.0);
    if ok {
        if let Err(e) = import(app, d, source, &mode) {
            app.status(e);
        }
    } else if cancel {
        d.fields.remove(IMPORT);
    }
}

/// Merge the template `source` names into the dialog's fields as `mode` says.
fn import(app: &mut VectorcraftApp, d: &mut Dialog, source: serde_json::Map<String, Value>, mode: &str) -> Result<(), String> {
    let mut p = Value::Object(source);
    p["mode"] = json!(mode);
    p["apply"] = json!(false);
    let mut d2 = d.clone();
    add_keywords(&mut d2);
    p["base"] = form::params(&d2);
    let merged = app.session.execute("file.info.applyTemplate", &p).map_err(|e| e.to_string())?;
    for (k, v) in merged.as_object().into_iter().flatten() {
        d.fields.insert(k.clone(), v.clone());
    }
    d.fields.insert(PENDING.into(), json!(""));
    d.fields.remove(IMPORT);
    Ok(())
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    templates_menu(app, ui, d);
    for (key, label) in [("title", tl!("Document Title:")), ("author", tl!("Author:")), ("authorTitle", tl!("Author Title:"))] {
        widgets::label_row(ui, label, LABEL, |ui| {
            form::text(ui, d, key, FIELD);
        });
        ui.add_space(4.0);
    }
    top_row(ui, tl!("Description:"), |ui| {
        form::text_area(ui, d, "description", FIELD, 3);
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Rating:"), LABEL, |ui| rating(ui, d));
    ui.add_space(4.0);
    top_row(ui, tl!("Keywords:"), |ui| {
        ui.vertical(|ui| {
            ui.set_max_width(FIELD + 12.0);
            chips(ui, d);
            ui.horizontal(|ui| {
                let r = form::text_edit(ui, d, PENDING, FIELD - 70.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    // Enter adds the keyword (it doesn't press OK) and keeps typing in the field.
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    r.request_focus();
                }
                let typed = !d.str(PENDING).trim().is_empty();
                if (widgets::flat_button(ui, tl!("Add"), 56.0).clicked() || enter) && typed {
                    add_keywords(d);
                }
            });
            form::caption(ui, tl!("Separate several keywords with commas."));
        });
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Copyright"));
    let statuses: Vec<(&str, &str)> = CopyrightStatus::ALL.iter().map(|c| (c.id(), c.label())).collect();
    choice(ui, d, "copyrightStatus", tl!("Copyright Status:"), &statuses);
    ui.add_space(4.0);
    top_row(ui, tl!("Copyright Notice:"), |ui| {
        form::text_area(ui, d, "copyrightNotice", FIELD, 2);
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Copyright Info URL:"), LABEL, |ui| {
        form::text(ui, d, "copyrightUrl", FIELD);
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Dates"));
    for (key, label) in [("created", tl!("Created:")), ("modified", tl!("Modified:"))] {
        // ISO 8601 UTC from the engine, shown as "2026-10-05 14:03:00 UTC".
        let shown = d
            .fields
            .get(key)
            .and_then(Value::as_str)
            .map_or_else(|| "—".to_string(), |s| format!("{} UTC", s.replace('T', " ").trim_end_matches('Z')));
        widgets::label_row(ui, label, LABEL, |ui| {
            ui.label(egui::RichText::new(shown).color(t.text_dim));
        });
    }
    false
}

/// A form row whose label sits at the top of a taller field.
fn top_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(LABEL, 24.0), egui::Sense::hover());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(12.5), t.text);
        add(ui);
    });
}

/// Five stars: a click sets the rating, a click on the current one clears it.
fn rating(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let current = d.f64("rating", 0.0) as u8;
    ui.spacing_mut().item_spacing.x = 2.0;
    for i in 1..=MAX_RATING {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
        let on = i <= current;
        let tint = if on {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.text_dim
        };
        icons::paint(ui, "star", rect.shrink(2.0), tint);
        let resp = resp.on_hover_text(crate::i18n::tn(u64::from(i), "{n} star", "{n} stars"));
        if resp.clicked() {
            d.fields.insert("rating".into(), json!(if i == current { 0 } else { i }));
        }
    }
}

/// The keywords as chips; a chip's × removes it.
fn chips(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let list = keywords(d);
    if list.is_empty() {
        return;
    }
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (i, k) in list.iter().enumerate() {
            let galley = ui.painter().layout_no_wrap(k.clone(), egui::FontId::proportional(12.0), t.text_strong);
            let size = egui::vec2(galley.size().x + 30.0, 22.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(rect, egui::CornerRadius::same(11), t.hover);
            ui.painter().rect_stroke(rect, egui::CornerRadius::same(11), egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
            ui.painter().galley(egui::pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, t.text_strong);
            let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), egui::vec2(14.0, 14.0));
            let resp = ui
                .interact(x, ui.scope_id().with(("keyword-remove", i)), egui::Sense::click())
                .on_hover_text(crate::i18n::fmt(tl!("Remove “{keyword}”"), &[("keyword", k.as_str())]));
            icons::paint(ui, "x", x.shrink(2.0), if resp.hovered() { t.text_strong } else { t.text_dim });
            if resp.clicked() {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        let mut list = list;
        list.remove(i);
        d.fields.insert("keywords".into(), json!(list));
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn file_info_draws_and_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        // The menu item and its shortcut open the dialog; agents use ui.fileInfoDialog.
        crate::menus::invoke(&mut app, "file.info", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(d.fields["created"].is_string(), "a new document has its creation date");
        for (k, v) in [("author", json!("Ada")), ("keywords", json!(["poster"])), ("rating", json!(4)), ("copyrightStatus", json!("copyrighted"))] {
            d.fields.insert(k.into(), v);
        }
        d.fields.insert(super::PENDING.into(), json!("print, Poster; blue"));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        let m = &st.doc.metadata;
        assert_eq!((m.author.as_str(), m.rating, m.copyright_status), ("Ada", 4, vectorcraft_doc::CopyrightStatus::Copyrighted));
        assert_eq!(m.keywords, ["poster", "print", "blue"], "typed keywords join the chips, each once");
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("rating".into(), json!(9));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }

    #[test]
    fn an_imported_template_fills_the_fields_and_ok_is_one_undo_step() {
        let dir = vectorcraft_testkit::TempDir::new("ui-file-info-template");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.attach_metadata_templates_dir(dir.join("Metadata Templates"));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.info.saveTemplate", json!({"name": "Studio", "fields": {"author": "Grace", "keywords": ["print"], "rating": 3}})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("author".into(), json!("Ada"));
        d.fields.insert(super::PENDING.into(), json!("fair"));
        // A listed template waits for its merge choice; the choice draws.
        d.fields.insert(super::IMPORT.into(), json!({"name": "Studio"}));
        d.fields.insert(super::IMPORT_MODE.into(), json!("append"));
        frame(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        super::import(&mut app, &mut d, json!({"name": "Studio"}).as_object().unwrap().clone(), "append").unwrap();
        assert!(!d.fields.contains_key(super::IMPORT));
        // Appended: the typed author stays, the typed keyword is kept and the template's added.
        assert_eq!(
            (d.fields["author"].clone(), d.fields["keywords"].clone(), d.fields["rating"].clone()),
            (json!("Ada"), json!(["fair", "print"]), json!(3))
        );
        assert_eq!(app.session.active().unwrap().history.undo.len(), 0, "nothing changes before OK");
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.metadata.author.as_str(), st.doc.metadata.rating), ("Ada", 3));
        assert_eq!(st.history.undo.len(), 1);
    }

    #[test]
    fn templates_folder_is_offered_only_where_there_is_one() {
        let dir = vectorcraft_testkit::TempDir::new("ui-file-info-folder");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(!super::has_templates_folder(&app), "a session without a folder, as on the web, offers no Templates Folder");
        app.session.attach_metadata_templates_dir(dir.join("Metadata Templates"));
        assert!(super::has_templates_folder(&app));
    }

    #[test]
    fn ok_waits_for_the_merge_choice_of_a_template_being_imported() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let ok = |app: &VectorcraftApp, d: &crate::state::Dialog| super::SPEC.ok_enabled.is_none_or(|f| f(app, d));
        assert!(ok(&app, &d));
        d.fields.insert(super::IMPORT.into(), json!({"name": "Studio"}));
        assert!(!ok(&app, &d), "OK and Enter wait while the merge choice is shown");
        d.fields.remove(super::IMPORT);
        assert!(ok(&app, &d));
    }

    #[test]
    fn a_template_saved_without_an_extension_gets_xmp_and_is_listed() {
        let dir = vectorcraft_testkit::TempDir::new("ui-file-info-save-ext");
        let folder = dir.join("Metadata Templates");
        let typed = folder.join("Studio").to_string_lossy().into_owned();
        let services = crate::Services {
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            exists: Some(std::sync::Arc::new(|p: &str| std::path::Path::new(p).exists())),
            pick_save: Some(Box::new(move |_: &crate::FilePick| Some(typed.clone()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.session.attach_metadata_templates_dir(folder.clone());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("author".into(), json!("Ada"));
        crate::picks::in_dialog(&mut app, &mut d, super::save_template).unwrap();
        assert!(folder.join("Studio.xmp").is_file() && !folder.join("Studio").exists(), "the typed name takes .xmp");
        let listed = app.session.execute("file.info.templates", &json!({})).unwrap();
        assert!(listed.to_string().contains("Studio"), "{listed}");
        // A file already there under the completed name is not replaced without asking.
        std::fs::write(folder.join("Studio.xmp"), b"kept").unwrap();
        crate::picks::in_dialog(&mut app, &mut d, super::save_template).unwrap();
        assert_eq!(std::fs::read(folder.join("Studio.xmp")).unwrap(), b"kept");
        assert_eq!(d.kind, crate::dialogs::confirm::REPLACE, "the dialog asks before replacing it");
    }
}
