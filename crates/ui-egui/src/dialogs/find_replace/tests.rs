use craft::text::ui::find::FindReplaceDialog;
use craft::ui::dialog::DialogHost;
use craft::ui::dialog::StackDialog;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::VectorcraftApp;
use crate::tests::panel_harness::Panel;

/// An app on a document holding point type `texts`, bottom first, with nothing selected → the
/// app and the type objects' ids.
fn app_with(texts: &[&str]) -> (VectorcraftApp, Vec<u64>) {
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ids = texts.iter().map(|t| s.execute("text.create", &json!({"x": 50, "y": 50, "text": t})).unwrap()["id"].as_u64().unwrap()).collect();
    s.execute("select.none", &json!({})).unwrap();
    (VectorcraftApp::new(s, crate::Services::default()), ids)
}

/// One frame of the window's dialogs and of their pump, as the shell runs them.
fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| host.run(ui));
    }
    crate::dialogs::host::pump(app, ui.ctx());
}

/// Choose the menu item of `command`, as a click on it does.
fn choose_command(p: &Panel, command: &str) {
    let (id, params) = crate::shortcut_editor::menu_target(command).expect("the menu item gives params");
    crate::menus::invoke(&mut p.app(), &id, params);
}

fn choose(p: &Panel) {
    choose_command(p, super::COMMAND);
}

/// Choose Edit › Find Next, as a click on the menu item does.
fn choose_find_next(p: &Panel) {
    let e = crate::menus::menu_entries(&p.app()).into_iter().find(|e| e.label == "Find Next").expect("in the Edit menu");
    assert_eq!((e.command.as_deref(), e.enabled), (Some(super::NEXT), true));
    crate::menus::invoke(&mut p.app(), super::NEXT, json!({"find": ""}));
}

fn with_dialog<R>(p: &Panel, f: impl FnOnce(&mut FindReplaceDialog) -> R) -> R {
    p.app().dialogs().expect("a window").with(|host| f(host.get_mut::<FindReplaceDialog>().expect("the dialog is open")))
}

fn is_open(p: &Panel) -> bool {
    p.app().dialogs().expect("a window").with(|host| host.get::<FindReplaceDialog>().is_some())
}

/// Set a field of the open dialog, as the user editing it does.
fn set(p: &Panel, k: &str, v: Value) {
    with_dialog(p, |d| d.set_json_field(p.ctx(), k, v).unwrap());
}

fn field(p: &Panel, k: &str) -> Value {
    with_dialog(p, |d| d.json_fields().unwrap_or(Value::Null)[k].clone())
}

fn message(p: &Panel) -> String {
    field(p, "message").as_str().unwrap_or_default().to_string()
}

/// The place the last search reached.
fn from(p: &Panel) -> Value {
    p.app().ui.last_find.as_ref().map(|l| l["from"].clone()).unwrap_or(Value::Null)
}

/// Run the dialogs' pump once, as the end of a frame does.
fn pump(p: &Panel) {
    crate::dialogs::host::pump(&mut p.app(), p.ctx());
}

/// The match Replace would change, as (object, start, end).
fn matched(p: &Panel) -> Option<(u64, u64, u64)> {
    let query = with_dialog(p, |d| super::form(d).query);
    let app = p.app();
    let last = app.ui.last_find.as_ref()?;
    let at = super::current_match(&app, last, &query)?;
    Some((at["id"].as_u64()?, at["start"].as_u64()?, at["end"].as_u64()?))
}

fn plain(p: &Panel, id: u64) -> String {
    match &p.app().session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not type"),
    }
}

fn undo_steps(p: &Panel) -> usize {
    p.app().session.active().unwrap().history.undo.len()
}

fn selected(p: &Panel) -> Vec<u64> {
    p.app().session.active().unwrap().selection.objects.iter().map(|id| id.0).collect()
}

/// The Edit menu's rows for Find and Replace, Check Spelling (in its Spelling submenu) and Edit Custom Dictionary
/// are enabled, and invoking each puts that dialog on the host.
#[test]
fn the_edit_menu_rows_open_their_dialogs() {
    type Shown = fn(&DialogHost) -> bool;
    let rows: [(&str, &[&str], &str, Shown); 3] = [
        ("Find and Replace…", &["Edit"], super::COMMAND, |h| h.get::<FindReplaceDialog>().is_some()),
        ("Check Spelling…", &["Edit", "Spelling"], crate::dialogs::check_spelling::COMMAND, |h| !h.open_dialogs().is_empty()),
        ("Edit Custom Dictionary…", &["Edit"], crate::dialogs::custom_dictionary::COMMAND, |h| {
            h.get::<craft::ui::patterns::form_dialogs::HostedListManager>().is_some()
        }),
    ];
    for (label, path, command, shown) in rows {
        let (app, _) = app_with(&["Hello zqvmisspelt world"]);
        let p = Panel::new(app, draw);
        let e = crate::menus::menu_entries(&p.app()).into_iter().find(|e| e.label == label).unwrap_or_else(|| panic!("{label} is in the Edit menu"));
        let shown_path: Vec<&str> = e.path.iter().map(String::as_str).collect();
        assert_eq!((shown_path.as_slice(), e.command.as_deref(), e.enabled), (path, Some(command), true), "{label}");
        // A click on the row: Check Spelling and Edit Custom Dictionary carry no params.
        let (id, params) = crate::menus::click_target(&e.label, command, &e.params);
        crate::menus::invoke(&mut p.app(), &id, params);
        assert!(p.app().dialogs().expect("a window").with(|host| shown(host)), "{label} opens its dialog");
    }
}

/// Find with other Find text starts again at the first match, the matches in the type object the
/// last Find selected included.
#[test]
fn find_with_other_text_starts_at_the_first_match() {
    let (app, ids) = app_with(&["dog", "cat dog"]);
    let (bottom, top) = (ids[0], ids[1]);
    let mut p = Panel::new(app, draw);
    choose(&p);
    set(&p, "find", json!("cat"));
    p.click("Find Next");
    assert_eq!((matched(&p), selected(&p)), (Some((top, 0, 3)), vec![top]));
    set(&p, "find", json!("dog"));
    p.click("Find Next");
    assert_eq!((matched(&p), selected(&p)), (Some((top, 4, 7)), vec![top]));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((bottom, 0, 3)));
}

/// The dialog stays open from match to match: Find Next goes to the next match and selects its
/// object, Replace changes that match, Replace & Find changes it and goes on, Replace All changes
/// the rest, and Close closes. Each Replace is one undo step, and so is Replace All; finding and a
/// Replace All with nothing to replace record none.
#[test]
fn the_buttons_step_through_the_matches() {
    let (app, ids) = app_with(&["cat cat", "a cat"]);
    let (bottom, top) = (ids[0], ids[1]);
    let mut p = Panel::new(app, draw);
    choose(&p);
    set(&p, "find", json!("cat"));
    set(&p, "replace", json!("dog"));
    let steps = undo_steps(&p);
    // Find Next: the first match, in the top object, shown with its place and its line of text.
    p.click("Find Next");
    assert_eq!((matched(&p), selected(&p), undo_steps(&p)), (Some((top, 2, 5)), vec![top], steps));
    let line = message(&p);
    assert!(line.starts_with("1 / 3") && line.contains("cat"), "{line}");
    // Replace: that match only, and the search stays where it was.
    p.click("Replace");
    assert_eq!((plain(&p, top), plain(&p, bottom), undo_steps(&p)), ("a dog".to_string(), "cat cat".to_string(), steps + 1));
    assert_eq!((matched(&p), from(&p)), (None, json!({"id": top, "start": 2, "end": 5})));
    // Find Next goes on from there, to the bottom object.
    p.click("Find Next");
    let count = p.app().ui.last_find.as_ref().map(|l| l["found"]["count"].clone());
    assert_eq!((matched(&p), selected(&p), count), (Some((bottom, 0, 3)), vec![bottom], Some(json!(2))));
    // Replace & Find: one undo step, and the next match is the one left.
    p.click("Replace & Find");
    assert_eq!((plain(&p, bottom), undo_steps(&p), matched(&p)), ("dog cat".to_string(), steps + 2, Some((bottom, 4, 7))));
    // Replace All: the rest as one undo step, with the count under the fields.
    p.click("Replace All");
    assert_eq!((plain(&p, bottom), undo_steps(&p), matched(&p)), ("dog dog".to_string(), steps + 3, None));
    assert_eq!(message(&p), "Matches replaced: 1");
    // Nothing left: no undo step, and the dialog says so, for Replace All and for Find Next.
    for button in ["Replace All", "Find Next"] {
        p.click(button);
        assert_eq!((undo_steps(&p), matched(&p)), (steps + 3, None), "{button}");
        assert_eq!(message(&p), "No matches found", "{button}");
    }
    assert!(is_open(&p), "the dialog stays open until Close");
    p.click("Close");
    assert!(!is_open(&p), "Close closes it");
    assert_eq!(undo_steps(&p), steps + 3);
}

/// Replace and Replace & Find wait for a match found with the Find text and the options as they
/// are: before the first Find, and after the Find text or an option changes, they change nothing.
/// With no Find text, nothing is found or replaced.
#[test]
fn replace_waits_for_a_match_of_the_text_as_it_is() {
    let (app, ids) = app_with(&["cat cart"]);
    let mut p = Panel::new(app, draw);
    choose(&p);
    let steps = undo_steps(&p);
    set(&p, "replace", json!("x"));
    for button in ["Find Next", "Replace", "Replace & Find", "Replace All"] {
        p.click(button);
        assert_eq!((plain(&p, ids[0]), undo_steps(&p), matched(&p)), ("cat cart".to_string(), steps, None), "{button} with no Find text");
    }
    set(&p, "find", json!("cat"));
    for button in ["Replace", "Replace & Find"] {
        p.click(button);
        assert_eq!((plain(&p, ids[0]), undo_steps(&p), matched(&p)), ("cat cart".to_string(), steps, None), "{button} before Find");
    }
    p.click("Find Next");
    assert_eq!(matched(&p), Some((ids[0], 0, 3)));
    assert!(p.enabled("Replace"));
    // The Find text changes: the match found is no longer the one to replace.
    set(&p, "find", json!("ca"));
    pump(&p);
    assert_eq!(matched(&p), None);
    // The buttons' state is what a frame drew.
    p.frame();
    assert!(!p.enabled("Replace"));
    for button in ["Replace", "Replace & Find"] {
        p.click(button);
        assert_eq!((plain(&p, ids[0]), undo_steps(&p)), ("cat cart".to_string(), steps), "{button} after the Find text changed");
    }
    // An option changes: the same.
    set(&p, "find", json!("cat"));
    assert_eq!(matched(&p), Some((ids[0], 0, 3)));
    set(&p, "matchCase", json!(true));
    assert_eq!(matched(&p), None);
    // Search Backwards is a direction, not what a match is.
    set(&p, "matchCase", json!(false));
    set(&p, "backwards", json!(true));
    assert_eq!(matched(&p), Some((ids[0], 0, 3)));
    pump(&p);
    p.click("Replace");
    assert_eq!((plain(&p, ids[0]), undo_steps(&p)), ("x cart".to_string(), steps + 1));
}

/// Edit › Find Next goes on from the dialog's last search while the dialog is closed, with its
/// options, and says when nothing matches. With no search yet, it opens the dialog, and the
/// dialog opens again on the last search.
#[test]
fn find_next_goes_on_from_the_last_search() {
    let (app, ids) = app_with(&["Cat", "cat", "cat"]);
    let mut p = Panel::new(app, draw);
    choose_find_next(&p);
    assert!(is_open(&p), "no search yet: the dialog opens");
    set(&p, "find", json!("cat"));
    set(&p, "replace", json!("dog"));
    set(&p, "matchCase", json!(true));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((ids[2], 0, 3)));
    p.click("Close");
    let steps = undo_steps(&p);
    // On from the match the dialog found; Match Case still leaves out "Cat".
    for want in [ids[1], ids[2], ids[1]] {
        choose_find_next(&p);
        assert_eq!((selected(&p), is_open(&p), undo_steps(&p)), (vec![want], false, steps));
    }
    // The dialog opens on the last search and the place it reached.
    choose(&p);
    assert_eq!((field(&p, "find"), field(&p, "replace"), field(&p, "matchCase")), (json!("cat"), json!("dog"), json!(true)));
    assert_eq!(from(&p), json!({"id": ids[1], "start": 0, "end": 3}));
    set(&p, "find", json!("zebra"));
    p.click("Find Next");
    assert_eq!(message(&p), "No matches found");
    p.click("Close");
    p.app().ui.status.clear();
    choose_find_next(&p);
    let status = p.app().ui.status.clone();
    assert_eq!((status.as_str(), selected(&p)), ("No matches found", vec![ids[1]]));
}

/// The place a search reached goes with the Find text and the switches it was reached with: a
/// search for other text starts again, in the dialog and when `ui.paramDialog` gives other text,
/// and the same search goes on from that place.
#[test]
fn a_changed_search_starts_again() {
    let (app, ids) = app_with(&["cat dog cat", "dog"]);
    let (bottom, top) = (ids[0], ids[1]);
    let mut p = Panel::new(app, draw);
    choose(&p);
    set(&p, "find", json!("cat"));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((bottom, 0, 3)));
    set(&p, "find", json!("dog"));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((top, 0, 3)), "not the dog after the cat found");
    set(&p, "find", json!("cat"));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((bottom, 0, 3)));
    p.click("Close");
    crate::menus::invoke(&mut p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": {"find": "dog"}}));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((top, 0, 3)));
    p.click("Close");
    choose(&p);
    assert_eq!(field(&p, "find"), json!("dog"));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((bottom, 4, 7)), "the same search goes on");
}

/// The place a search reached belongs to its document: in another document, Edit › Find Next and
/// the dialog start at that document's first match, even where an object there has the same id.
/// The params of a `ui.paramDialog` call take the place of the last search's, a switch that is off
/// included, while the menu item's own params leave the last search as it was.
#[test]
fn a_search_place_stays_in_its_document_and_given_params_win() {
    let (app, ids) = app_with(&["cat cat"]);
    let mut p = Panel::new(app, draw);
    choose(&p);
    set(&p, "find", json!("cat"));
    set(&p, "matchCase", json!(true));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((ids[0], 0, 3)));
    p.click("Close");
    // A second document, whose type object has the id of the first one's.
    p.app().run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let other = p.app().run("text.create", json!({"x": 50, "y": 50, "text": "cat cat"})).unwrap()["id"].as_u64().unwrap();
    p.app().run("select.none", json!({})).unwrap();
    assert_eq!(other, ids[0]);
    choose_find_next(&p);
    assert_eq!((selected(&p), from(&p)), (vec![other], json!({"id": other, "start": 0, "end": 3})));
    // The dialog opens on the last search; Find Next goes on in this document.
    choose(&p);
    assert_eq!(field(&p, "matchCase"), json!(true));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((other, 4, 7)));
    // Back in the first document, the dialog's place is not used: Replace changes nothing, and
    // Find Next starts at the first match.
    assert!(p.app().session.set_active(0));
    assert_eq!(matched(&p), None);
    set(&p, "replace", json!("dog"));
    p.click("Replace");
    assert_eq!((plain(&p, ids[0]), matched(&p)), ("cat cat".to_string(), None));
    p.click("Find Next");
    assert_eq!(matched(&p), Some((ids[0], 0, 3)));
    p.click("Close");
    // Given params win over the last search; the menu item's own do not.
    crate::menus::invoke(&mut p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": {"matchCase": false, "replace": "dog"}}));
    assert_eq!((field(&p, "find"), field(&p, "matchCase"), field(&p, "replace")), (json!("cat"), json!(false), json!("dog")));
    p.click("Close");
    let empty = json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false});
    crate::menus::invoke(&mut p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": empty}));
    assert_eq!((field(&p, "find"), field(&p, "matchCase")), (json!(""), json!(false)));
    p.click("Close");
    choose(&p);
    assert_eq!((field(&p, "find"), field(&p, "matchCase")), (json!("cat"), json!(true)));
}
