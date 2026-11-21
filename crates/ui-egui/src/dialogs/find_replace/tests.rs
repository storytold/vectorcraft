// PLAN: assumed paths of the framework crates and of the op.A15 `Panel` test harness.
use craft::text::ui::find::FindReplaceDialog;
use craft::ui::dialog::DialogHost;
use craft::ui::dialog::StackDialog;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::VectorcraftApp;
use crate::testing::Panel;

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
fn choose_command(app: &mut VectorcraftApp, command: &str) {
    let (id, p) = crate::shortcut_editor::menu_target(command).expect("the menu item gives params");
    crate::menus::invoke(app, &id, p);
}

fn choose(app: &mut VectorcraftApp) {
    choose_command(app, super::COMMAND);
}

/// Choose Edit › Find Next, as a click on the menu item does.
fn choose_find_next(app: &mut VectorcraftApp) {
    let e = crate::menus::menu_entries(app).into_iter().find(|e| e.label == "Find Next").expect("in the Edit menu");
    assert_eq!((e.command.as_deref(), e.enabled), (Some(super::NEXT), true));
    crate::menus::invoke(app, super::NEXT, json!({"find": ""}));
}

fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut FindReplaceDialog) -> R) -> R {
    app.dialogs().expect("a window").with(|host| f(host.get_mut::<FindReplaceDialog>().expect("the dialog is open")))
}

fn is_open(app: &VectorcraftApp) -> bool {
    app.dialogs().expect("a window").with(|host| host.get::<FindReplaceDialog>().is_some())
}

/// Set a field of the open dialog, as the user editing it does.
fn set(app: &VectorcraftApp, k: &str, v: Value) {
    let ctx = app.ui_ctx.clone().expect("a window");
    with_dialog(app, |d| d.set_json_field(&ctx, k, v).unwrap());
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    with_dialog(app, |d| d.json_fields().unwrap_or(Value::Null)[k].clone())
}

fn message(app: &VectorcraftApp) -> String {
    field(app, "message").as_str().unwrap_or_default().to_string()
}

/// The place the last search reached.
fn from(app: &VectorcraftApp) -> Value {
    app.ui.last_find.as_ref().map(|l| l["from"].clone()).unwrap_or(Value::Null)
}

/// Run the dialogs' pump once, as the end of a frame does.
fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    crate::dialogs::host::pump(app, &ctx);
}

/// The match Replace would change, as (object, start, end).
fn matched(app: &VectorcraftApp) -> Option<(u64, u64, u64)> {
    let query = with_dialog(app, |d| super::form(d).query);
    let last = app.ui.last_find.as_ref()?;
    let at = super::current_match(app, last, &query)?;
    Some((at["id"].as_u64()?, at["start"].as_u64()?, at["end"].as_u64()?))
}

fn plain(app: &VectorcraftApp, id: u64) -> String {
    match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not type"),
    }
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

fn selected(app: &VectorcraftApp) -> Vec<u64> {
    app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect()
}

/// The Edit menu's rows for Find and Replace, Check Spelling and Edit Custom Dictionary are enabled,
/// and invoking each puts that dialog on the host.
#[test]
fn the_edit_menu_rows_open_their_dialogs() {
    type Shown = fn(&DialogHost) -> bool;
    let rows: [(&str, &str, Shown); 3] = [
        ("Find and Replace…", super::COMMAND, |h| h.get::<FindReplaceDialog>().is_some()),
        ("Check Spelling…", crate::dialogs::check_spelling::COMMAND, |h| !h.open_dialogs().is_empty()),
        (
            "Edit Custom Dictionary…",
            crate::dialogs::custom_dictionary::COMMAND,
            |h| h.get::<craft::ui::patterns::form_dialogs::HostedListManager>().is_some(),
        ),
    ];
    for (label, command, shown) in rows {
        let (app, _) = app_with(&["Hello zqvmisspelt world"]);
        let mut p = Panel::new(app, draw);
        let e = crate::menus::menu_entries(p.app()).into_iter().find(|e| e.label == label).unwrap_or_else(|| panic!("{label} is in the Edit menu"));
        assert_eq!((e.path.as_slice(), e.command.as_deref(), e.enabled), (&["Edit".to_string()][..], Some(command), true), "{label}");
        choose_command(p.app(), command);
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
    choose(p.app());
    set(p.app(), "find", json!("cat"));
    p.click("Find Next");
    assert_eq!((matched(p.app()), selected(p.app())), (Some((top, 0, 3)), vec![top]));
    set(p.app(), "find", json!("dog"));
    p.click("Find Next");
    assert_eq!((matched(p.app()), selected(p.app())), (Some((top, 4, 7)), vec![top]));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((bottom, 0, 3)));
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
    choose(p.app());
    set(p.app(), "find", json!("cat"));
    set(p.app(), "replace", json!("dog"));
    let steps = undo_steps(p.app());
    // Find Next: the first match, in the top object, shown with its place and its line of text.
    p.click("Find Next");
    assert_eq!((matched(p.app()), selected(p.app()), undo_steps(p.app())), (Some((top, 2, 5)), vec![top], steps));
    let line = message(p.app());
    assert!(line.starts_with("1 / 3") && line.contains("cat"), "{line}");
    // Replace: that match only, and the search stays where it was.
    p.click("Replace");
    assert_eq!((plain(p.app(), top), plain(p.app(), bottom), undo_steps(p.app())), ("a dog".to_string(), "cat cat".to_string(), steps + 1));
    assert_eq!((matched(p.app()), from(p.app())), (None, json!({"id": top, "start": 2, "end": 5})));
    // Find Next goes on from there, to the bottom object.
    p.click("Find Next");
    let count = p.app().ui.last_find.as_ref().map(|l| l["found"]["count"].clone());
    assert_eq!((matched(p.app()), selected(p.app()), count), (Some((bottom, 0, 3)), vec![bottom], Some(json!(2))));
    // Replace & Find: one undo step, and the next match is the one left.
    p.click("Replace & Find");
    assert_eq!((plain(p.app(), bottom), undo_steps(p.app()), matched(p.app())), ("dog cat".to_string(), steps + 2, Some((bottom, 4, 7))));
    // Replace All: the rest as one undo step, with the count under the fields.
    p.click("Replace All");
    assert_eq!((plain(p.app(), bottom), undo_steps(p.app()), matched(p.app())), ("dog dog".to_string(), steps + 3, None));
    assert_eq!(message(p.app()), "Matches replaced: 1");
    // Nothing left: no undo step, and the dialog says so, for Replace All and for Find Next.
    for button in ["Replace All", "Find Next"] {
        p.click(button);
        assert_eq!((undo_steps(p.app()), matched(p.app())), (steps + 3, None), "{button}");
        assert_eq!(message(p.app()), "No matches found", "{button}");
    }
    assert!(is_open(p.app()), "the dialog stays open until Close");
    p.click("Close");
    assert!(!is_open(p.app()), "Close closes it");
    assert_eq!(undo_steps(p.app()), steps + 3);
}

/// Replace and Replace & Find wait for a match found with the Find text and the options as they
/// are: before the first Find, and after the Find text or an option changes, they change nothing.
/// With no Find text, nothing is found or replaced.
#[test]
fn replace_waits_for_a_match_of_the_text_as_it_is() {
    let (app, ids) = app_with(&["cat cart"]);
    let mut p = Panel::new(app, draw);
    choose(p.app());
    let steps = undo_steps(p.app());
    set(p.app(), "replace", json!("x"));
    for button in ["Find Next", "Replace", "Replace & Find", "Replace All"] {
        p.click(button);
        assert_eq!((plain(p.app(), ids[0]), undo_steps(p.app()), matched(p.app())), ("cat cart".to_string(), steps, None), "{button} with no Find text");
    }
    set(p.app(), "find", json!("cat"));
    for button in ["Replace", "Replace & Find"] {
        p.click(button);
        assert_eq!((plain(p.app(), ids[0]), undo_steps(p.app()), matched(p.app())), ("cat cart".to_string(), steps, None), "{button} before Find");
    }
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((ids[0], 0, 3)));
    assert!(p.enabled("Replace"));
    // The Find text changes: the match found is no longer the one to replace.
    set(p.app(), "find", json!("ca"));
    pump(p.app());
    assert_eq!(matched(p.app()), None);
    assert!(!p.enabled("Replace"));
    for button in ["Replace", "Replace & Find"] {
        p.click(button);
        assert_eq!((plain(p.app(), ids[0]), undo_steps(p.app())), ("cat cart".to_string(), steps), "{button} after the Find text changed");
    }
    // An option changes: the same.
    set(p.app(), "find", json!("cat"));
    assert_eq!(matched(p.app()), Some((ids[0], 0, 3)));
    set(p.app(), "matchCase", json!(true));
    assert_eq!(matched(p.app()), None);
    // Search Backwards is a direction, not what a match is.
    set(p.app(), "matchCase", json!(false));
    set(p.app(), "backwards", json!(true));
    assert_eq!(matched(p.app()), Some((ids[0], 0, 3)));
    pump(p.app());
    p.click("Replace");
    assert_eq!((plain(p.app(), ids[0]), undo_steps(p.app())), ("x cart".to_string(), steps + 1));
}

/// Edit › Find Next goes on from the dialog's last search while the dialog is closed, with its
/// options, and says when nothing matches. With no search yet, it opens the dialog, and the
/// dialog opens again on the last search.
#[test]
fn find_next_goes_on_from_the_last_search() {
    let (app, ids) = app_with(&["Cat", "cat", "cat"]);
    let mut p = Panel::new(app, draw);
    choose_find_next(p.app());
    assert!(is_open(p.app()), "no search yet: the dialog opens");
    set(p.app(), "find", json!("cat"));
    set(p.app(), "replace", json!("dog"));
    set(p.app(), "matchCase", json!(true));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((ids[2], 0, 3)));
    p.click("Close");
    let steps = undo_steps(p.app());
    // On from the match the dialog found; Match Case still leaves out "Cat".
    for want in [ids[1], ids[2], ids[1]] {
        choose_find_next(p.app());
        assert_eq!((selected(p.app()), is_open(p.app()), undo_steps(p.app())), (vec![want], false, steps));
    }
    // The dialog opens on the last search and the place it reached.
    choose(p.app());
    assert_eq!((field(p.app(), "find"), field(p.app(), "replace"), field(p.app(), "matchCase")), (json!("cat"), json!("dog"), json!(true)));
    assert_eq!(from(p.app()), json!({"id": ids[1], "start": 0, "end": 3}));
    set(p.app(), "find", json!("zebra"));
    p.click("Find Next");
    assert_eq!(message(p.app()), "No matches found");
    p.click("Close");
    p.app().ui.status.clear();
    choose_find_next(p.app());
    assert_eq!((p.app().ui.status.as_str(), selected(p.app())), ("No matches found", vec![ids[1]]));
}

/// The place a search reached goes with the Find text and the switches it was reached with: a
/// search for other text starts again, in the dialog and when `ui.paramDialog` gives other text,
/// and the same search goes on from that place.
#[test]
fn a_changed_search_starts_again() {
    let (app, ids) = app_with(&["cat dog cat", "dog"]);
    let (bottom, top) = (ids[0], ids[1]);
    let mut p = Panel::new(app, draw);
    choose(p.app());
    set(p.app(), "find", json!("cat"));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((bottom, 0, 3)));
    set(p.app(), "find", json!("dog"));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((top, 0, 3)), "not the dog after the cat found");
    set(p.app(), "find", json!("cat"));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((bottom, 0, 3)));
    p.click("Close");
    crate::menus::invoke(p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": {"find": "dog"}}));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((top, 0, 3)));
    p.click("Close");
    choose(p.app());
    assert_eq!(field(p.app(), "find"), json!("dog"));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((bottom, 4, 7)), "the same search goes on");
}

/// The place a search reached belongs to its document: in another document, Edit › Find Next and
/// the dialog start at that document's first match, even where an object there has the same id.
/// The params of a `ui.paramDialog` call take the place of the last search's, a switch that is off
/// included, while the menu item's own params leave the last search as it was.
#[test]
fn a_search_place_stays_in_its_document_and_given_params_win() {
    let (app, ids) = app_with(&["cat cat"]);
    let mut p = Panel::new(app, draw);
    choose(p.app());
    set(p.app(), "find", json!("cat"));
    set(p.app(), "matchCase", json!(true));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((ids[0], 0, 3)));
    p.click("Close");
    // A second document, whose type object has the id of the first one's.
    p.app().run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let other = p.app().run("text.create", json!({"x": 50, "y": 50, "text": "cat cat"})).unwrap()["id"].as_u64().unwrap();
    p.app().run("select.none", json!({})).unwrap();
    assert_eq!(other, ids[0]);
    choose_find_next(p.app());
    assert_eq!((selected(p.app()), from(p.app())), (vec![other], json!({"id": other, "start": 0, "end": 3})));
    // The dialog opens on the last search; Find Next goes on in this document.
    choose(p.app());
    assert_eq!(field(p.app(), "matchCase"), json!(true));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((other, 4, 7)));
    // Back in the first document, the dialog's place is not used: Replace changes nothing, and
    // Find Next starts at the first match.
    assert!(p.app().session.set_active(0));
    assert_eq!(matched(p.app()), None);
    set(p.app(), "replace", json!("dog"));
    p.click("Replace");
    assert_eq!((plain(p.app(), ids[0]), matched(p.app())), ("cat cat".to_string(), None));
    p.click("Find Next");
    assert_eq!(matched(p.app()), Some((ids[0], 0, 3)));
    p.click("Close");
    // Given params win over the last search; the menu item's own do not.
    crate::menus::invoke(p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": {"matchCase": false, "replace": "dog"}}));
    assert_eq!((field(p.app(), "find"), field(p.app(), "matchCase"), field(p.app(), "replace")), (json!("cat"), json!(false), json!("dog")));
    p.click("Close");
    let empty = json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false});
    crate::menus::invoke(p.app(), "ui.paramDialog", json!({"command": super::COMMAND, "params": empty}));
    assert_eq!((field(p.app(), "find"), field(p.app(), "matchCase")), (json!(""), json!(false)));
    p.click("Close");
    choose(p.app());
    assert_eq!((field(p.app(), "find"), field(p.app(), "matchCase")), (json!("cat"), json!(true)));
}
