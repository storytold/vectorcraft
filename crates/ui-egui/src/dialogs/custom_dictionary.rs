//! Edit › Edit Custom Dictionary…: the custom dictionary's words in the framework's list manager, with
//! an Entry field and Add, Delete and Done, as the public
//! [Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)
//! page describes it: enter a word and Add it, select a word and Delete it, or rename a word in
//! place. Nothing changes until Done, which deletes and adds the words that differ in one
//! `text.spelling.editDictionary`, all or none; Cancel leaves the dictionary as it was.
//!
//! The edited list lives in the window's temporary memory under [`state_id`] while the dialog is
//! open; [`pump`] applies the dialog's choices to it and shows it back.

use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::patterns::form_dialogs::{ActionEnabled, HostedListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::spelling::custom_word;

use crate::VectorcraftApp;

/// The command its menu item runs: with no params, it opens this dialog.
pub const COMMAND: &str = "text.spelling.dictionary";

/// The command Done runs: every delete and add, saved once.
const EDIT: &str = "text.spelling.editDictionary";

/// The kind of every entry: all words rename in place.
const WORD: u32 = 0;

/// The action ids of Done and Add.
const DONE: &str = "done";
const ADD: &str = "add";

/// The id `ListManagerAction::delete` gives its action.
const DELETE: &str = "delete";

/// The dialog's id on the host.
fn id() -> DialogId {
    dialog_id(COMMAND)
}

/// Where the edited list is kept while the dialog is open.
fn state_id() -> egui::Id {
    egui::Id::unique("custom-dictionary-words")
}

/// The custom dictionary's words.
fn dictionary(app: &mut VectorcraftApp) -> Result<Vec<String>, String> {
    let words = app.run(COMMAND, json!({})).map_err(|e| e.to_string())?;
    Ok(words.get("words").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default())
}

fn entries(words: &[String]) -> Vec<ListManagerEntry> {
    words.iter().map(|w| ListManagerEntry::new(w, w).kind(WORD)).collect()
}

/// Open Edit Custom Dictionary on the custom dictionary's words.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut words = dictionary(app)?;
    words.sort();
    words.dedup();
    let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) else { return Err("no window".into()) };
    let standard = app.host.words().set::<ListManagerWords>();
    let actions = vec![
        ListManagerAction::new(DONE, tl!("Done"), ActionEnabled::Always).keeps_open(),
        ListManagerAction::new(ADD, tl!("Add"), ActionEnabled::OnNewName).keeps_open(),
        ListManagerAction::delete(&standard),
    ];
    let dialog = HostedListManager::new(COMMAND, tl!("Edit Custom Dictionary"), tl!("Entry:"), actions).entries(entries(&words)).renames(WORD);
    dialogs.with(|host| host.open(dialog));
    ctx.data_mut(|data| data.insert_temp(state_id(), words));
    Ok(Value::Null)
}

/// A word the dictionary does not take: the list stays as it is.
fn refuse(app: &mut VectorcraftApp, name: &str) {
    app.status(format!("“{name}” is not a word the custom dictionary takes"));
}

/// Add, Delete or rename changes the dialog's list, kept sorted and without repeats.
fn apply(app: &mut VectorcraftApp, ctx: &egui::Context, choice: &ListManagerChoice) {
    if choice.action == DONE {
        return save(app, ctx);
    }
    let Some(mut words) = ctx.data(|data| data.get_temp::<Vec<String>>(state_id())) else { return };
    match (choice.action.as_str(), choice.entry.as_deref()) {
        (ADD, _) => match custom_word(&choice.name) {
            Some(word) => words.push(word),
            None => return refuse(app, &choice.name),
        },
        (DELETE, Some(entry)) => words.retain(|w| w != entry),
        (_, Some(entry)) if choice.is_rename() => match custom_word(&choice.name) {
            Some(word) => {
                words.retain(|w| w != entry);
                words.push(word);
            }
            None => return refuse(app, &choice.name),
        },
        _ => return,
    }
    words.sort();
    words.dedup();
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            if let Some(list) = host.get_dyn_mut(id()).and_then(|d| d.downcast_mut::<HostedListManager>()) {
                list.set_entries(entries(&words));
            }
        });
    }
    ctx.data_mut(|data| data.insert_temp(state_id(), words));
}

/// Done: deletes the words no longer in the list and adds the new ones in one
/// `text.spelling.editDictionary`, saved once. When it can't be saved, none of it is kept: the
/// dialog stays open with the error, its list as edited, and Cancel leaves the dictionary as it was.
fn save(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(words) = ctx.data(|data| data.get_temp::<Vec<String>>(state_id())) else { return };
    let orig = match dictionary(app) {
        Ok(orig) => orig,
        Err(e) => return app.status(e),
    };
    let remove: Vec<&String> = orig.iter().filter(|w| !words.contains(w)).collect();
    let add: Vec<&String> = words.iter().filter(|w| !orig.contains(w)).collect();
    match app.run(EDIT, json!({ "remove": remove, "add": add })) {
        Ok(_) => {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.cancel_by_id(ctx, id()));
            }
        }
        Err(e) => app.status(e.to_string()),
    }
}

/// Runs the choice the dialog was answered with since the last frame; every action keeps it
/// open, so its close is a Cancel, which drops the edited list.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    // `app.run` must not run inside the host's lock, so the choice is taken first.
    if let Some(choice) = dialogs.with(|host| host.get_mut_by_id::<HostedListManager>(id()).and_then(HostedListManager::take_choice)) {
        apply(app, ctx, &choice);
    }
    if dialogs.with(|host| host.take_by_id::<HostedListManager>(id())).is_some() {
        ctx.data_mut(|data| data.remove::<Vec<String>>(state_id()));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::VectorcraftApp;
    use crate::tests::panel_harness::Panel;
    use craft::ui::patterns::form_dialogs::HostedListManager;

    // The custom dictionary is the whole process's: each test here uses words no other test does.
    const A: &str = "zqvdicta";
    const B: &str = "zqvdictb";
    const C: &str = "zqvdictc";

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default())
    }

    fn has(app: &mut VectorcraftApp, w: &str) -> bool {
        app.run("text.spelling.dictionary", json!({})).unwrap()["words"].as_array().unwrap().contains(&json!(w))
    }

    fn forget(app: &mut VectorcraftApp, words: &[&str]) {
        for w in words {
            let _ = app.run("text.spelling.removeWord", json!({ "word": w }));
        }
    }

    /// One frame of the window's dialogs and of their pump, as the shell runs them.
    fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.run(ui));
        }
        crate::dialogs::host::pump(app, ui.ctx());
    }

    /// Types `text` in the Entry field without Enter, which runs the first action, Done.
    fn enter(p: &mut Panel, text: &str) {
        p.click("Entry:");
        p.key(egui::Key::A, egui::Modifiers::COMMAND);
        p.type_text(text);
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().expect("a window").with(|host| host.get_by_id::<HostedListManager>(super::id()).is_some())
    }

    /// The dialog's list, of `mine` (other tests may have their words in the dictionary meanwhile).
    fn words(app: &VectorcraftApp, mine: &[&str]) -> Vec<String> {
        let ctx = app.ui_ctx.clone().expect("a window");
        let all = ctx.data(|data| data.get_temp::<Vec<String>>(super::state_id())).unwrap_or_default();
        all.into_iter().filter(|w| mine.contains(&w.as_str())).collect()
    }

    /// Add and Delete change only the dialog's list; Cancel leaves the dictionary as it was; Done
    /// applies the list once.
    #[test]
    fn add_and_delete_apply_on_done() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        forget(&mut app, &[A, B, C]);
        app.run("text.spelling.addWord", json!({ "word": A })).unwrap();
        let mut p = Panel::new(app, draw);
        super::open(&mut p.app()).unwrap();
        // Add: the Entry field's word (kept in lower case) joins the list.
        enter(&mut p, " ZqvDictB ");
        p.click("Add");
        assert_eq!(words(&p.app(), &[A, B, C]), [A, B]);
        assert!(!has(&mut p.app(), B), "not until Done");
        // Delete the word picked.
        p.click(A);
        p.click("Delete");
        assert_eq!(words(&p.app(), &[A, B, C]), [B]);
        // Cancel: the dictionary is as it was.
        p.click("Cancel");
        assert!(!is_open(&p.app()));
        {
            let mut app = p.app();
            assert!(has(&mut app, A) && !has(&mut app, B));
        }
        // Again, then Done.
        super::open(&mut p.app()).unwrap();
        p.click(A);
        p.click("Delete");
        enter(&mut p, B);
        p.click("Add");
        enter(&mut p, C);
        p.click("Add");
        p.click("Done");
        assert!(!is_open(&p.app()), "Done closes it");
        let mut app = p.app();
        assert!(!has(&mut app, A) && has(&mut app, B) && has(&mut app, C));
        forget(&mut app, &[A, B, C]);
    }

    /// Done that can't be saved keeps none of the edits: the dialog stays open and Cancel leaves the
    /// dictionary as it was.
    #[test]
    fn a_done_that_cant_be_saved_changes_nothing() {
        let _dictionary = crate::testing::custom_dictionary();
        const P: &str = "zqvdictp";
        const Q: &str = "zqvdictq";
        let mut app = app();
        forget(&mut app, &[P, Q]);
        app.run("text.spelling.addWord", json!({ "word": P })).unwrap();
        let folder = vectorcraft_testkit::temp_dir("custom-dictionary-done").join("Custom Dictionary.txt");
        std::fs::create_dir_all(&folder).unwrap();
        let mut failing = self::app();
        failing.session.attach_custom_dictionary(folder);
        let mut p = Panel::new(failing, draw);
        super::open(&mut p.app()).unwrap();
        p.click(P);
        p.click("Delete");
        enter(&mut p, Q);
        p.click("Add");
        assert_eq!(words(&p.app(), &[P, Q]), [Q]);
        p.click("Done");
        assert!(is_open(&p.app()), "the dialog stays open with the error");
        assert!(has(&mut app, P) && !has(&mut app, Q), "neither the delete nor the add is kept");
        p.click("Cancel");
        assert!(has(&mut app, P) && !has(&mut app, Q));
        forget(&mut app, &[P, Q]);
    }
}
