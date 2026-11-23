use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::VectorcraftApp;

/// An app on a document holding point type `texts`, bottom first, with nothing selected → the
/// app and the type objects' ids.
fn app_with(texts: &[&str]) -> (VectorcraftApp, Vec<u64>) {
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ids = texts.iter().map(|t| s.execute("text.create", &json!({"x": 50, "y": 50, "text": t})).unwrap()["id"].as_u64().unwrap()).collect();
    s.execute("select.none", &json!({})).unwrap();
    (VectorcraftApp::new(s, crate::Services::default()), ids)
}

/// Choose Edit › Find and Replace…, as a click on the menu item does.
fn choose(app: &mut VectorcraftApp) {
    let (id, p) = crate::shortcut_editor::menu_target(super::COMMAND).expect("the menu item gives params");
    crate::menus::invoke(app, &id, p);
}

/// Choose Edit › Find Next, as a click on the menu item does.
fn choose_find_next(app: &mut VectorcraftApp) {
    let e = crate::menus::menu_entries(app).into_iter().find(|e| e.label == "Find Next").expect("in the Edit menu");
    assert_eq!((e.command.as_deref(), e.enabled), (Some(super::NEXT), true));
    crate::menus::invoke(app, super::NEXT, json!({"find": ""}));
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().expect("the dialog is open").fields.get(k).cloned().unwrap_or(Value::Null)
}

/// The match Replace would change, as (object, start, end).
fn matched(app: &VectorcraftApp) -> Option<(u64, u64, u64)> {
    let at = super::current_match(app, app.ui.dialog.as_ref()?)?;
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

/// One frame of the dialog on an 800 × 600 window → the text painted, with its rect.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    // A window takes a sizing frame and fades in: each frame is a quarter second on.
    let time = ctx.input(|i| i.time) + 0.25;
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
    let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut v = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
    v
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> Vec<(String, Rect)> {
    frame(app, ctx, vec![]);
    let texts = frame(app, ctx, vec![]);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at)]);
    frame(app, ctx, vec![press(true)]);
    frame(app, ctx, vec![press(false)]);
    frame(app, ctx, vec![])
}

/// Edit › Find and Replace… opens the dialog of its own, with the fields and options of the
/// public reference's "Find and replace text", instead of the generic parameter dialog.
#[test]
fn the_edit_menu_opens_the_find_and_replace_dialog() {
    let (mut app, _) = app_with(&["cat"]);
    let e = crate::menus::menu_entries(&app).into_iter().find(|e| e.label == "Find and Replace…").expect("in the Edit menu");
    assert_eq!((e.path.as_slice(), e.command.as_deref(), e.enabled), (&["Edit".to_string()][..], Some(super::COMMAND), true));
    choose(&mut app);
    let d = app.ui.dialog.as_ref().expect("Find and Replace opens");
    assert_eq!((d.kind.as_str(), d.fields["__command"].clone()), (super::KIND, json!(super::COMMAND)));
    for (key, want) in [("find", json!("")), ("replace", json!("")), ("action", json!("find"))] {
        assert_eq!(d.fields[key], want, "{key}");
    }
    for key in ["matchCase", "wholeWord", "backwards", "hiddenLayers", "lockedLayers"] {
        assert_eq!(d.fields[key], json!(false), "{key}");
    }
    let ctx = context();
    frame(&mut app, &ctx, vec![]);
    let texts: Vec<String> = frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
    for label in [
        "Find and Replace",
        "Find:",
        "Replace With:",
        "Match Case",
        "Find Whole Word",
        "Search Backwards",
        "Check Hidden Layers",
        "Check Locked Layers",
        "Find",
        "Replace",
        "Replace & Find",
        "Replace All",
        "Done",
    ] {
        assert!(texts.iter().any(|t| t == label), "no `{label}` in {texts:?}");
    }
}

/// A field given a value of another type than its own is an error and the dialog does not open; a
/// null value gives no field. A field set to such a value once the dialog is open runs no action:
/// the error is the note, shown under the match found, and the text, the match and the last search
/// stay as they were.
#[test]
fn the_dialog_rejects_a_field_of_the_wrong_type() {
    let (mut app, ids) = app_with(&["cat"]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    crate::dialogs::confirm(&mut app).unwrap();
    let last = app.ui.last_find.clone();
    for (k, v, want) in [
        ("replace", json!(5), "`replace` must be a string"),
        ("replace", Value::Null, "`replace` must be a string"),
        ("matchCase", json!("yes"), "`matchCase` must be true or false"),
    ] {
        set(&mut app, k, v.clone());
        set(&mut app, "action", json!("replaceAll"));
        assert_eq!(crate::dialogs::confirm(&mut app).unwrap_err(), want, "{k}: {v}");
        assert_eq!((plain(&app, ids[0]), field(&app, "__note"), app.ui.last_find.clone()), ("cat".to_string(), json!(want), last.clone()));
        assert_eq!(matched(&app), Some((ids[0], 0, 3)), "Replace still changes the match found");
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        let texts: Vec<String> = frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == want) && texts.iter().any(|t| t == "cat"), "the note shows under the match: {texts:?}");
        set(&mut app, k, if k == "replace" { json!("dog") } else { json!(false) });
    }
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    let open = |app: &mut VectorcraftApp, params: Value| app.run("ui.paramDialog", json!({"command": super::COMMAND, "params": params}));
    for (params, want) in [
        (json!({"find": 17}), "`find` must be a string"),
        (json!({"replace": true}), "`replace` must be a string"),
        (json!({"matchCase": "yes"}), "`matchCase` must be true or false"),
        (json!({"backwards": 1}), "`backwards` must be true or false"),
    ] {
        assert_eq!(open(&mut app, params.clone()).unwrap_err(), want, "{params}");
        assert!(app.ui.dialog.is_none(), "{params}");
    }
    open(&mut app, json!({"find": null, "replace": "dog", "wholeWord": true})).unwrap();
    assert_eq!((field(&app, "find"), field(&app, "replace"), field(&app, "wholeWord")), (json!("cat"), json!("dog"), json!(true)));
}

/// An `action` that is not a string runs no action, Find included: the error is the note, and the
/// match found, the selection and the last search stay as they were.
#[test]
fn an_action_of_the_wrong_type_runs_no_action() {
    let (mut app, _) = app_with(&["cat", "cat"]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    crate::dialogs::confirm(&mut app).unwrap();
    let (at, sel, last) = (matched(&app), selected(&app), app.ui.last_find.clone());
    assert!(at.is_some());
    for v in [json!(5), Value::Null, json!(["find"])] {
        set(&mut app, "action", v.clone());
        assert_eq!(crate::dialogs::confirm(&mut app).unwrap_err(), "`action` must be a string", "{v}");
        assert_eq!((matched(&app), selected(&app), app.ui.last_find.clone()), (at, sel.clone(), last.clone()), "{v}");
        assert_eq!(field(&app, "__note"), json!("`action` must be a string"), "{v}");
    }
    set(&mut app, "action", json!("find"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_ne!(matched(&app), at, "a string action still runs");
}

/// Find with other Find text starts again at the first match, the matches in the type object the
/// last Find selected included.
#[test]
fn find_with_other_text_starts_at_the_first_match() {
    let (mut app, ids) = app_with(&["dog", "cat dog"]);
    let (bottom, top) = (ids[0], ids[1]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((matched(&app), selected(&app)), (Some((top, 0, 3)), vec![top]));
    set(&mut app, "find", json!("dog"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((matched(&app), selected(&app)), (Some((top, 4, 7)), vec![top]));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((bottom, 0, 3)));
}

/// The dialog stays open from match to match: Find goes to the next match and selects its
/// object, Replace changes that match, Replace & Find changes it and goes on, Replace All changes
/// the rest, and Done closes. Each Replace is one undo step, and so is Replace All; finding and a
/// Replace All with nothing to replace record none.
#[test]
fn the_buttons_step_through_the_matches() {
    let (mut app, ids) = app_with(&["cat cat", "a cat"]);
    let (bottom, top) = (ids[0], ids[1]);
    choose(&mut app);
    let ctx = context();
    set(&mut app, "find", json!("cat"));
    set(&mut app, "replace", json!("dog"));
    let steps = undo_steps(&app);
    // Find: the first match, in the top object, shown with its place and its line of text.
    let texts = click(&mut app, &ctx, "Find");
    assert_eq!((matched(&app), selected(&app), undo_steps(&app)), (Some((top, 2, 5)), vec![top], steps));
    assert!(texts.iter().any(|(t, _)| t.starts_with("1 / 3")) && texts.iter().any(|(t, _)| t == "cat"), "{texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "Find Next"), "Find reads Find Next once a match is found: {texts:?}");
    // Replace: that match only, and the search stays where it was.
    click(&mut app, &ctx, "Replace");
    assert_eq!((plain(&app, top), plain(&app, bottom), undo_steps(&app)), ("a dog".to_string(), "cat cat".to_string(), steps + 1));
    assert_eq!((matched(&app), field(&app, "__at")), (None, json!({"id": top, "start": 2, "end": 5})));
    // Find goes on from there, to the bottom object.
    click(&mut app, &ctx, "Find");
    assert_eq!((matched(&app), selected(&app), field(&app, "__found")["count"].clone()), (Some((bottom, 0, 3)), vec![bottom], json!(2)));
    // Replace & Find: one undo step, and the next match is the one left.
    click(&mut app, &ctx, "Replace & Find");
    assert_eq!((plain(&app, bottom), undo_steps(&app), matched(&app)), ("dog cat".to_string(), steps + 2, Some((bottom, 4, 7))));
    // Replace All: the rest as one undo step, with the count under the fields.
    let texts = click(&mut app, &ctx, "Replace All");
    assert_eq!((plain(&app, bottom), undo_steps(&app), matched(&app)), ("dog dog".to_string(), steps + 3, None));
    assert!(texts.iter().any(|(t, _)| t == "Matches replaced: 1"), "{texts:?}");
    // Nothing left: no undo step, and the dialog says so, for Replace All and for Find.
    for button in ["Replace All", "Find"] {
        let texts = click(&mut app, &ctx, button);
        assert_eq!((undo_steps(&app), matched(&app)), (steps + 3, None), "{button}");
        assert!(texts.iter().any(|(t, _)| t == "No matches found"), "{button}: {texts:?}");
    }
    assert!(app.ui.dialog.is_some(), "the dialog stays open until Done");
    click(&mut app, &ctx, "Done");
    assert!(app.ui.dialog.is_none(), "Done closes it");
    assert_eq!(undo_steps(&app), steps + 3);
}

/// Replace and Replace & Find wait for a match found with the Find text and the options as they
/// are: before the first Find, and after the Find text changes, a click on them changes nothing.
/// With no Find text, no button but Done acts. Enter runs Find.
#[test]
fn replace_waits_for_a_match_of_the_text_as_it_is() {
    let (mut app, ids) = app_with(&["cat cart"]);
    choose(&mut app);
    let ctx = context();
    let steps = undo_steps(&app);
    set(&mut app, "replace", json!("x"));
    for button in ["Find", "Replace", "Replace & Find", "Replace All"] {
        click(&mut app, &ctx, button);
        assert_eq!((plain(&app, ids[0]), undo_steps(&app), matched(&app)), ("cat cart".to_string(), steps, None), "{button} with no Find text");
    }
    set(&mut app, "find", json!("cat"));
    for button in ["Replace", "Replace & Find"] {
        click(&mut app, &ctx, button);
        assert_eq!((plain(&app, ids[0]), undo_steps(&app), matched(&app)), ("cat cart".to_string(), steps, None), "{button} before Find");
    }
    // Enter finds.
    frame(&mut app, &ctx, vec![]);
    let enter = |pressed| Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![enter(true)]);
    frame(&mut app, &ctx, vec![enter(false)]);
    assert_eq!(matched(&app), Some((ids[0], 0, 3)));
    // The Find text changes: the match found is no longer the one to replace.
    set(&mut app, "find", json!("ca"));
    assert_eq!(matched(&app), None);
    for button in ["Replace", "Replace & Find"] {
        click(&mut app, &ctx, button);
        assert_eq!((plain(&app, ids[0]), undo_steps(&app)), ("cat cart".to_string(), steps), "{button} after the Find text changed");
    }
    // An option changes: the same.
    set(&mut app, "find", json!("cat"));
    assert_eq!(matched(&app), Some((ids[0], 0, 3)));
    set(&mut app, "matchCase", json!(true));
    assert_eq!(matched(&app), None);
    // Search Backwards is a direction, not what a match is.
    set(&mut app, "matchCase", json!(false));
    set(&mut app, "backwards", json!(true));
    assert_eq!(matched(&app), Some((ids[0], 0, 3)));
    click(&mut app, &ctx, "Replace");
    assert_eq!((plain(&app, ids[0]), undo_steps(&app)), ("x cart".to_string(), steps + 1));
}

/// `ui.dialog.confirm` runs the dialog's `action`, keeps it open and puts `action` back to find;
/// done closes it. An action it does not know is refused, and so is an empty Find text.
#[test]
fn confirm_runs_the_action() {
    let (mut app, ids) = app_with(&["cat cat cat"]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    set(&mut app, "replace", json!("dog"));
    let steps = undo_steps(&app);
    let r = crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((r["start"].clone(), r["index"].clone(), r["count"].clone(), matched(&app)), (json!(0), json!(1), json!(3), Some((ids[0], 0, 3))));
    set(&mut app, "action", json!("replaceFind"));
    let r = crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((plain(&app, ids[0]), r["start"].clone(), field(&app, "action")), ("dog cat cat".to_string(), json!(4), json!("find")));
    set(&mut app, "action", json!("replace"));
    let r = crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((plain(&app, ids[0]), r["replaced"].clone(), matched(&app)), ("dog dog cat".to_string(), json!(true), None));
    // Replace with no match found: it finds one.
    set(&mut app, "action", json!("replace"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((plain(&app, ids[0]), matched(&app), undo_steps(&app)), ("dog dog cat".to_string(), Some((ids[0], 8, 11)), steps + 2));
    set(&mut app, "action", json!("replaceAll"));
    assert_eq!(crate::dialogs::confirm(&mut app).unwrap(), json!({"count": 1, "replaced": 1}));
    assert_eq!((plain(&app, ids[0]), undo_steps(&app)), ("dog dog dog".to_string(), steps + 3));
    // Matches that already read as the Replace With text are found and not counted as replaced.
    set(&mut app, "find", json!("DOG"));
    set(&mut app, "action", json!("replaceAll"));
    assert_eq!(crate::dialogs::confirm(&mut app).unwrap(), json!({"count": 3, "replaced": 0}));
    assert_eq!((undo_steps(&app), field(&app, "__note")), (steps + 3, json!("Matches replaced: 0")));
    set(&mut app, "find", json!("dog"));
    for (action, find) in [("erase", "dog"), ("find", ""), ("replaceAll", "")] {
        set(&mut app, "action", json!(action));
        set(&mut app, "find", json!(find));
        assert!(crate::dialogs::confirm(&mut app).is_err(), "{action} {find:?}");
        assert_eq!((plain(&app, ids[0]), undo_steps(&app), field(&app, "action")), ("dog dog dog".to_string(), steps + 3, json!("find")));
        assert!(field(&app, "__note").is_string(), "{action} {find:?}: the dialog shows why");
    }
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
}

/// Edit › Find Next goes on from the dialog's last search while the dialog is closed, with its
/// options, and says when nothing matches. With no search yet, it opens the dialog, and the
/// dialog opens again on the last search.
#[test]
fn find_next_goes_on_from_the_last_search() {
    let (mut app, ids) = app_with(&["Cat", "cat", "cat"]);
    choose_find_next(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND), "no search yet: the dialog opens");
    set(&mut app, "find", json!("cat"));
    set(&mut app, "replace", json!("dog"));
    set(&mut app, "matchCase", json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((ids[2], 0, 3)));
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    let steps = undo_steps(&app);
    // On from the match the dialog found; Match Case still leaves out "Cat".
    for want in [ids[1], ids[2], ids[1]] {
        choose_find_next(&mut app);
        assert_eq!((selected(&app), app.ui.dialog.is_none(), undo_steps(&app)), (vec![want], true, steps));
    }
    // The dialog opens on the last search and the place it reached.
    choose(&mut app);
    assert_eq!((field(&app, "find"), field(&app, "replace"), field(&app, "matchCase")), (json!("cat"), json!("dog"), json!(true)));
    assert_eq!(field(&app, "__at"), json!({"id": ids[1], "start": 0, "end": 3}));
    set(&mut app, "find", json!("zebra"));
    assert!(crate::dialogs::confirm(&mut app).unwrap()["id"].is_null());
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    app.ui.status.clear();
    choose_find_next(&mut app);
    assert_eq!((app.ui.status.as_str(), selected(&app)), ("No matches found", vec![ids[1]]));
}

/// The place a search reached goes with the Find text and the switches it was reached with: a
/// search for other text starts again, in the dialog and when `ui.paramDialog` gives other text,
/// and the same search goes on from that place.
#[test]
fn a_changed_search_starts_again() {
    let (mut app, ids) = app_with(&["cat dog cat", "dog"]);
    let (bottom, top) = (ids[0], ids[1]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((bottom, 0, 3)));
    set(&mut app, "find", json!("dog"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((top, 0, 3)), "not the dog after the cat found");
    set(&mut app, "find", json!("cat"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((bottom, 0, 3)));
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    crate::menus::invoke(&mut app, "ui.paramDialog", json!({"command": super::COMMAND, "params": {"find": "dog"}}));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((top, 0, 3)));
    app.ui.dialog = None;
    choose(&mut app);
    assert_eq!(field(&app, "find"), json!("dog"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((bottom, 4, 7)), "the same search goes on");
}

/// The place a search reached belongs to its document: in another document, Edit › Find Next and
/// the dialog start at that document's first match, even where an object there has the same id.
/// The params of a `ui.paramDialog` call take the place of the last search's, a switch that is off
/// included, while the menu item's own params leave the last search as it was.
#[test]
fn a_search_place_stays_in_its_document_and_given_params_win() {
    let (mut app, ids) = app_with(&["cat cat"]);
    choose(&mut app);
    set(&mut app, "find", json!("cat"));
    set(&mut app, "matchCase", json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((ids[0], 0, 3)));
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    // A second document, whose type object has the id of the first one's.
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let other = app.run("text.create", json!({"x": 50, "y": 50, "text": "cat cat"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.none", json!({})).unwrap();
    assert_eq!(other, ids[0]);
    choose_find_next(&mut app);
    assert_eq!(
        (selected(&app), app.ui.last_find.as_ref().map(|l| l["from"].clone())),
        (vec![other], Some(json!({"id": other, "start": 0, "end": 3})))
    );
    // The dialog opens on the last search; Find goes on in this document.
    choose(&mut app);
    assert_eq!(field(&app, "matchCase"), json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(matched(&app), Some((other, 4, 7)));
    // Back in the first document, the dialog's place is not used: Replace finds first and changes
    // nothing.
    assert!(app.session.set_active(0));
    assert_eq!(matched(&app), None);
    set(&mut app, "replace", json!("dog"));
    set(&mut app, "action", json!("replace"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((plain(&app, ids[0]), matched(&app)), ("cat cat".to_string(), Some((ids[0], 0, 3))));
    set(&mut app, "action", json!("done"));
    crate::dialogs::confirm(&mut app).unwrap();
    // Given params win over the last search; the menu item's own do not.
    crate::menus::invoke(&mut app, "ui.paramDialog", json!({"command": super::COMMAND, "params": {"matchCase": false, "replace": "dog"}}));
    assert_eq!((field(&app, "find"), field(&app, "matchCase"), field(&app, "replace")), (json!("cat"), json!(false), json!("dog")));
    app.ui.dialog = None;
    let empty = json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false});
    crate::menus::invoke(&mut app, "ui.paramDialog", json!({"command": super::COMMAND, "params": empty}));
    assert_eq!((field(&app, "find"), field(&app, "matchCase")), (json!(""), json!(false)));
    app.ui.dialog = None;
    choose(&mut app);
    assert_eq!((field(&app, "find"), field(&app, "matchCase")), (json!("cat"), json!(true)));
}
