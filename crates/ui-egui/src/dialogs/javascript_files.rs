//! JavaScript Files (the SVG Interactivity panel's Link JavaScript Files…): the JavaScript files
//! the active document links, in a list. Add links the path or URL in File or URL
//! (`svgInteractivity.addFile`; Choose… puts a picked file's `file:///` URL there where the host
//! has an open dialog), and Remove unlinks the file selected in the list
//! (`svgInteractivity.removeFile`). SVG export writes a `<script xlink:href>` element for each
//! linked file. Add and Remove act at once. The dialog has only Close.
//!
//! Fields: `file` (the path or URL to link), `selected` (a linked file).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp, widgets};

/// The dialog kind of JavaScript Files.
pub const KIND: &str = "javascriptFiles";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("JavaScript Files").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 420.0,
    ..DialogSpec::FORM
};

/// Open JavaScript Files for the active document.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "file": "", "selected": "" })));
    Ok(Value::Null)
}

/// The extension of JavaScript files.
const JS_EXTS: &[&str] = &["js"];

/// The open dialog for a JavaScript file.
fn pick() -> FilePick {
    FilePick { filters: vec![FilePick::filter("JavaScript", JS_EXTS)], ..Default::default() }
}

/// Choose…: the file picked in the open dialog into File or URL, as a `file:///` URL
/// ([`crate::io::file_url`]).
fn choose(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    if let Some(path) = crate::picks::open(app, &pick()) {
        d.fields.insert("file".into(), json!(crate::io::file_url(&path)));
    }
    Ok(())
}

/// What a button asked for, done once the dialog is drawn.
enum Action {
    Select(String),
    Choose,
    Add,
    Remove,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let files: Vec<String> = app.session.active().map(|st| st.doc.svg_scripts.clone()).unwrap_or_default();
    let selected = d.str("selected");
    let mut act = None;
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("javascript-files").min_scrolled_height(140.0).max_height(140.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(140.0);
            // Paths and URLs are data: shown as they are.
            for f in &files {
                if ui.selectable_label(*f == selected, f.as_str()).clicked() {
                    act = Some(Action::Select(f.clone()));
                }
            }
        });
    });
    ui.add_space(8.0);
    widgets::label_row(ui, tl!("File or URL:"), 84.0, |ui| {
        if let Some(v) = widgets::exact_text_field(ui, "javascript-file", &d.str("file"), 240.0) {
            d.fields.insert("file".into(), json!(v));
        }
        if crate::picks::can(app, &PickRequest::Open(pick())) && widgets::flat_button(ui, tl!("Choose…"), 64.0).clicked() {
            act = Some(Action::Choose);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.add_enabled_ui(!d.str("file").trim().is_empty(), |ui| widgets::flat_button(ui, tl!("Add"), 64.0)).inner.clicked() {
            act = Some(Action::Add);
        }
        if ui.add_enabled_ui(files.contains(&selected), |ui| widgets::flat_button(ui, tl!("Remove"), 64.0)).inner.clicked() {
            act = Some(Action::Remove);
        }
    });
    let r = match act {
        None => Ok(()),
        Some(Action::Select(f)) => {
            d.fields.insert("selected".into(), json!(f));
            Ok(())
        }
        // The open dialog, shown off the UI thread, fills the dialog as it is then.
        Some(Action::Choose) => crate::picks::in_dialog(app, d, choose),
        Some(Action::Add) => {
            let file = d.str("file").trim().to_string();
            app.run("svgInteractivity.addFile", json!({ "file": file })).map(|_| {
                d.fields.insert("selected".into(), json!(file));
                d.fields.insert("file".into(), json!(""));
            })
        }
        Some(Action::Remove) => app.run("svgInteractivity.removeFile", json!({ "file": selected })).map(|_| {
            d.fields.insert("selected".into(), json!(""));
        }),
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// One headless frame of the dialog's body: the texts drawn.
    fn frame(app: &mut VectorcraftApp, d: &mut Dialog) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            body(app, ui, d);
        });
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    #[test]
    fn opens_on_the_documents_linked_files() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(open(&mut app).is_err(), "no document");
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("svgInteractivity.addFile", &json!({"file": "scripts/menu.js"})).unwrap();
        crate::menus::invoke(&mut app, "ui.javascriptFilesDialog", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let mut d = app.ui.dialog.clone().unwrap();
        // (Only paths are checked: tests running alongside change the language labels are drawn in.)
        let texts = frame(&mut app, &mut d);
        assert!(texts.iter().any(|t| t == "scripts/menu.js"), "{texts:?}");
        d.fields.insert("file".into(), json!("https://example.com/b.js"));
        let texts = frame(&mut app, &mut d);
        assert!(texts.iter().any(|t| t == "https://example.com/b.js"), "the field shows `file`: {texts:?}");
    }

    /// Choose… puts the picked file in File or URL as a `file:///` URL, with no `\` and with
    /// spaces percent-encoded.
    #[test]
    fn choose_puts_the_picked_file_in_as_a_file_url() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let picked = std::env::temp_dir().join("my menu.js").to_string_lossy().into_owned();
        app.services.pick_open = Some(Box::new(move |_: &FilePick| Some(picked.clone())));
        let mut d = Dialog::new(KIND, json!({ "file": "", "selected": "" }));
        choose(&mut app, &mut d).unwrap();
        let file = d.str("file");
        assert!(file.starts_with("file:///") && file.ends_with("/my%20menu.js") && !file.contains('\\'), "{file}");
    }
}
