//! JavaScript Files (the SVG Interactivity panel's Link JavaScript Files…): the JavaScript files
//! the active document links, in a list. Add links the path or URL in File or URL
//! (`svgInteractivity.addFile`; Choose… puts a picked file's `file:///` URL there), and Remove
//! unlinks the file selected in the list (`svgInteractivity.removeFile`). SVG export writes a
//! `<script xlink:href>` element for each linked file. Add and Remove act at once. The dialog has
//! only Close.
//!
//! Fields: `file` (the path or URL to link), `selected` (a linked file).
//!
//! The dialog lives on the window's dialog host. Its body has no app, so it records the one
//! action a click asked for and [`pump`] runs it, and pushes the document's files in each frame.
//!
//! In a frame automation drives with read roots in force, the list names a file outside them by
//! its file name alone, as `svgInteractivity.info` does; `selected` keeps the file as linked.

use craft::behavior::{Selection, keyed};
use craft::host::PickRequest;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::PathRow;
use craft::ui::primitives::Revision;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::links::shown_script;

use crate::VectorcraftApp;
use crate::panels::svg_interactivity::caption;

/// The key the dialog is known by: its id on the dialog stack, and the name `ui.javascriptFilesDialog`
/// documents.
const KEY: &str = "javascriptFiles";

/// Names the file selected in an open JavaScript Files dialog, in `ui.inspect`'s list of open `dialogs`, as
/// `svgInteractivity.info` names the active document's linked files: outside the read roots, by its file name alone.
pub(crate) fn show_selected(app: &VectorcraftApp, dialogs: &mut Value) {
    let (Some(st), Some(open)) = (app.session.active(), dialogs.as_array_mut()) else { return };
    let id = dialog_id(KEY).value().to_string();
    for dialog in open.iter_mut().filter(|d| d["id"] == id.as_str()) {
        if let Some(Value::String(selected)) = dialog.pointer_mut("/fields/selected") {
            *selected = shown_script(st, selected).into_owned();
        }
    }
}

/// What the body asks [`pump`] to do with the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Add,
    Remove,
}

/// The values the body edits.
#[derive(Debug, Default)]
struct Form {
    /// The path or URL to link.
    file: String,
    /// The linked file selected in the list, as linked or as the list shows it.
    selected: String,
    pending: Option<Pending>,
}

impl Form {
    /// The form once `action` ran: an added file is the selected one, a removed one leaves none.
    fn finished(&mut self, action: Pending, file: String) {
        match action {
            Pending::Add => {
                self.selected = file;
                self.file.clear();
            }
            Pending::Remove => self.selected.clear(),
        }
    }
}

/// The JavaScript Files dialog on the window's dialog host; at most one is open.
#[derive(Debug)]
pub(crate) struct JavaScriptFiles {
    open: bool,
    /// The active document's linked files ([`listed`]), pushed by [`pump`].
    files: Vec<(String, String)>,
    form: Form,
}

impl JavaScriptFiles {
    fn new(files: Vec<(String, String)>) -> Self {
        JavaScriptFiles { open: true, files, form: Form::default() }
    }

    /// `selected` as the list shows it.
    // PLAN: `json_fields` has no document, so `shown_script` cannot run when automation reads the dialog; it
    // reports the form `pump` pushed in the last frame, which is the read-root form only when that frame ran under
    // the roots. Needs the rule by document path from the engine (links.rs `shown_script`), or hub-w2's inspect to
    // pump under the roots.
    fn shown_selected(&self) -> &str {
        let selected = self.form.selected.as_str();
        self.files.iter().find(|(linked, _)| linked == selected).map_or(selected, |(_, shown)| shown.as_str())
    }
}

impl StackDialog for JavaScriptFiles {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let Self { open, files, form } = self;
        let shown = Dialog::new(tl!("JavaScript Files"))
            .buttons(vec![FooterButton::new(tl!("Close"), DialogRole::Cancel)])
            .opened(Bind::bound(open))
            .show(ui, |ui, _| body(ui, files, form));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "file": self.form.file, "selected": self.shown_selected() }))
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let target = match field {
            "file" => &mut self.form.file,
            "selected" => &mut self.form.selected,
            _ => return Err(format!("JavaScript Files has no field `{field}`")),
        };
        *target = value.as_str().ok_or_else(|| format!("`{field}` takes a string"))?.to_string();
        Ok(())
    }
}

/// Open JavaScript Files for the active document; an open one stays as it is.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    let files = listed(app);
    let dialogs = app.dialogs().ok_or("the dialog host isn't available")?;
    // An open dialog keeps its input: the refused one is dropped.
    let _ = dialogs.with(|host| host.open_unless_open(JavaScriptFiles::new(files)));
    Ok(Value::Null)
}

/// Pushes the document's linked files into the open dialog and runs the Add or Remove its body asked
/// for. A dialog that closed leaves only an empty answer, which is dropped.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let open = dialogs.with(|host| {
        let _ = host.take::<JavaScriptFiles>();
        host.is_open::<JavaScriptFiles>()
    });
    if !open {
        return;
    }
    let files = listed(app);
    let asked = dialogs.with(|host| {
        let dialog = host.get_mut::<JavaScriptFiles>()?;
        dialog.files = files;
        let form = &mut dialog.form;
        form.pending.take().map(|action| (action, form.file.trim().to_string(), form.selected.clone()))
    });
    let Some((action, file, selected)) = asked else { return };
    let result = match action {
        Pending::Add => app.run("svgInteractivity.addFile", json!({ "file": file })),
        Pending::Remove => app.run("svgInteractivity.removeFile", json!({ "file": selected })),
    };
    match result {
        Ok(_) => dialogs.with(|host| {
            if let Some(dialog) = host.get_mut::<JavaScriptFiles>() {
                dialog.form.finished(action, file);
            }
        }),
        Err(e) => app.status(e),
    }
}

/// The active document's linked files, each as it is linked and as the list shows it
/// ([`shown_script`]).
fn listed(app: &VectorcraftApp) -> Vec<(String, String)> {
    app.session.active().map_or_else(Vec::new, |st| st.doc.svg_scripts.iter().map(|f| (f.clone(), shown_script(st, f).into_owned())).collect())
}

/// The row of `files` ([`listed`]) that `selected` names: the one row linked or shown as it. A
/// click selects a file as it is linked; a request may also give it as the list shows it. A name
/// that fits several rows selects none, as `svgInteractivity.removeFile` refuses it.
fn selected_row(files: &[(String, String)], selected: &str) -> Option<usize> {
    let mut rows = files.iter().enumerate().filter(|(_, (linked, shown))| linked == selected || shown == selected);
    match (rows.next(), rows.next()) {
        (Some((i, _)), None) => Some(i),
        _ => None,
    }
}

/// The rows the linked-files list shows, empty or not, before it scrolls.
const LIST_ROWS: u16 = 6;

fn body(ui: &mut Ui, files: &[(String, String)], form: &mut Form) {
    let source = keyed(files, |(linked, _): &(String, String)| linked.clone());
    let selected = selected_row(files, &form.selected).map(|i| &files[i].0);
    let mut selection = selected.map(|linked| Selection::single(linked.clone())).unwrap_or_default();
    List::new(tl!("Linked files"))
        .select(SelectMode::Single)
        .rows_stamp(Revision::of(&selected).get())
        .min_rows(LIST_ROWS)
        .max_rows(LIST_ROWS)
        .selection(Bind::bound(&mut selection))
        .sizing(Sizing::FillWidth)
        .empty(tl!("No JavaScript files linked"))
        .show_items(ui, &source, &mut |key, _| {
            // Paths and URLs are data: shown as they are.
            ListRow::new(files.iter().find(|(linked, _)| linked == key).map_or_else(String::new, |(_, shown)| shown.clone()))
        });
    if selection.items.first() != selected {
        form.selected = selection.items.into_iter().next().unwrap_or_default();
    }
    Field::new(caption("File or URL:")).show(ui, |ui, _| {
        let request = PickRequest::open("javascript").filter("JavaScript", &["js"]);
        let out = PathRow::new(caption("File or URL:"), Bind::bound(&mut form.file)).button(tl!("Choose…")).pick(request).show(ui);
        // The answer is a path; the dialog links a `file:///` URL ([`crate::io::file_url`]).
        if let Some(path) = out.picked {
            form.file = crate::io::file_url(&path);
        }
    });
    let chosen = selected_row(files, &form.selected);
    Row::new().show(ui, |ui, _| {
        if clicked(&Button::new(tl!("Add")).enabled(!form.file.trim().is_empty()).show(ui)) {
            form.pending = Some(Pending::Add);
        }
        if clicked(&Button::new(tl!("Remove")).enabled(chosen.is_some()).show(ui)) {
            form.pending = Some(Pending::Remove);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::{AppInfo, Host, Picked, ScriptedFiles};
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use vectorcraft_engine::Session;

    use super::*;
    use crate::tests::panel_harness::Panel;

    /// The window's dialog host and the pump, as a frame of the app runs them.
    fn draw(app: &mut VectorcraftApp, ui: &mut Ui) {
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.run(ui));
        }
        pump(app, ui.ctx());
    }

    fn set_field(panel: &Panel, field: &str, value: &str) {
        let dialogs = panel.app().dialogs().unwrap();
        dialogs.with(|host| host.set_json_field(panel.ctx(), dialog_id(KEY), field, json!(value))).unwrap();
    }

    fn json_selected(panel: &Panel) -> Value {
        let dialogs = panel.app().dialogs().unwrap();
        dialogs.with(|host| host.get::<JavaScriptFiles>().and_then(StackDialog::json_fields)).unwrap()["selected"].clone()
    }

    #[test]
    fn opens_on_the_documents_linked_files() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(open(&mut app).is_err(), "no document");
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("svgInteractivity.addFile", &json!({"file": "scripts/menu.js"})).unwrap();
        let mut panel = Panel::new(app, draw);
        panel.app().run("ui.javascriptFilesDialog", json!({})).unwrap();
        panel.frame();
        panel.frame();
        assert!(panel.shows("scripts/menu.js"), "the list names the linked file");
        set_field(&panel, "file", "https://example.com/b.js");
        panel.frame();
        assert_eq!(panel.value(caption("File or URL:")).as_deref(), Some("https://example.com/b.js"), "the field shows `file`");
        // Add acts at once and selects what it linked.
        panel.click(tl!("Add"));
        panel.frame();
        let linked = panel.app().session.active().map(|st| st.doc.svg_scripts.clone()).unwrap_or_default();
        assert!(linked.iter().any(|f| f == "https://example.com/b.js"), "{linked:?}");
        assert_eq!(json_selected(&panel), json!("https://example.com/b.js"));
    }

    #[test]
    fn arrow_navigation_changes_which_javascript_file_remove_unlinks() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        for file in ["scripts/first.js", "scripts/second.js"] {
            app.run("svgInteractivity.addFile", json!({"file": file})).unwrap();
        }
        let mut panel = Panel::new(app, draw);
        panel.app().run("ui.javascriptFilesDialog", json!({})).unwrap();
        panel.frame();
        panel.frame();

        panel.click("scripts/first.js");
        assert_eq!(json_selected(&panel), "scripts/first.js");
        panel.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(json_selected(&panel), "scripts/second.js");
        // A selection changed outside the list gives the next arrow its new starting row.
        set_field(&panel, "selected", "scripts/first.js");
        panel.frame();
        panel.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(json_selected(&panel), "scripts/second.js");
        panel.click(tl!("Remove"));
        assert_eq!(panel.app().session.active().unwrap().doc.svg_scripts, ["scripts/first.js"]);
    }

    /// In a frame drawn with read roots in force, the list names a linked file outside them by its
    /// file name alone, and that name unlinks it; the person sees the whole path.
    #[test]
    fn automation_javascript_files_dialog_hides_outside_root_paths() {
        let base = std::env::temp_dir().join(format!("vc-js-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("in")).unwrap();
        let outside = base.join("outside").join("menu.js").to_string_lossy().into_owned();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("svgInteractivity.addFile", &json!({ "file": outside })).unwrap();
        let mut panel = Panel::new(app, draw);
        panel.app().run("ui.javascriptFilesDialog", json!({})).unwrap();
        set_field(&panel, "selected", &outside);
        panel.frame();
        panel.frame();
        assert!(panel.shows(&outside), "the person's list");
        assert_eq!(json_selected(&panel), json!(outside), "the person's");
        let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
        // The pump pushes the list after the shell pass (wg.A18): the second frame under the roots draws what the first
        // one pushed.
        vectorcraft_engine::file_access::confine(roots.as_ref(), || {
            panel.frame();
            panel.frame();
        });
        assert!(panel.shows("menu.js") && !panel.shows(&outside), "the list names the file alone");
        assert_eq!(json_selected(&panel), json!("menu.js"), "`selected` as the request may name it");
        // A row keeps the file as linked, so a click selects it by its whole path, which stays
        // selected in the person's list; a name a request gives selects it too.
        let app = panel.app();
        let rows = vectorcraft_engine::file_access::confine(roots.as_ref(), || listed(&app));
        assert_eq!(rows, [(outside.clone(), "menu.js".to_string())]);
        assert_eq!((selected_row(&rows, &outside), selected_row(&rows, "menu.js")), (Some(0), Some(0)));
        // A name two rows show selects neither.
        let twins = [rows[0].clone(), ("/elsewhere/menu.js".to_string(), "menu.js".to_string())];
        assert_eq!((selected_row(&twins, "menu.js"), selected_row(&twins, "/elsewhere/menu.js")), (None, Some(1)));
        assert_eq!(listed(&app), [(outside.clone(), outside.clone())], "the person's");
        drop(app);
        // Remove sends the selected name.
        let removed =
            vectorcraft_engine::file_access::confine(roots.as_ref(), || panel.app().run("svgInteractivity.removeFile", json!({"file": "menu.js"})));
        assert!(removed.is_ok(), "{removed:?}");
        assert!(panel.app().session.active().is_some_and(|st| st.doc.svg_scripts.is_empty()));
        let _ = std::fs::remove_dir_all(base);
    }

    /// Choose… puts the picked file in File or URL as a `file:///` URL, with no `\` and with
    /// spaces percent-encoded.
    #[test]
    fn choose_puts_the_picked_file_in_as_a_file_url() {
        let files = Arc::new(ScriptedFiles::held());
        let form = Rc::new(RefCell::new(Form::default()));
        let shown = Rc::clone(&form);
        let mut fixture = Fixture::new(move |ui| body(ui, &[], &mut shown.borrow_mut()));
        fixture.host(Arc::new(Host::builder(AppInfo { id: "test", name: "Test", version: "1.0" }).files(files.clone()).build()));
        fixture.settle();
        let choose = fixture.by_label(tl!("Choose…")).expect("the Choose button");
        fixture.click(&choose);
        fixture.settle();
        let picked = std::env::temp_dir().join("my menu.js");
        assert!(files.answer(Picked::Paths(vec![picked])).is_some(), "Choose… filed a pick");
        fixture.idle(3);
        let file = form.borrow().file.clone();
        assert!(file.starts_with("file:///") && file.ends_with("/my%20menu.js") && !file.contains('\\'), "{file}");
    }
}
