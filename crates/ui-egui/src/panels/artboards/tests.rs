use super::*;
use craft::behavior::Mixed;
use craft::ui::contract::ReorderMove;
use craft::ui::dialog::StackDialog;
use serde_json::Value;
use std::sync::Arc;
use vectorcraft_engine::Session;

use crate::dialogs::artboard_options::ArtboardOptionsDialog;
use crate::dialogs::rearrange_artboards::RearrangeArtboardsDialog;
use crate::dialogs::rename_artboards;

/// The key of every dialog open on the window's host.
fn open_keys(app: &VectorcraftApp) -> Vec<String> {
    app.dialogs().map(|d| d.with(|host| host.open_dialogs().into_iter().filter_map(|o| o.key).collect())).unwrap_or_default()
}

/// Whether a dialog of type `D` is open.
fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<D>()))
}

/// The fields of the dialog on top.
fn dialog_fields(app: &VectorcraftApp) -> Value {
    app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().pop().and_then(|o| o.fields))).unwrap_or_default()
}

/// Cancel every open dialog.
fn cancel_open(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if let Some(d) = app.dialogs() {
        d.with(|host| {
            for o in host.open_dialogs() {
                host.cancel_by_id(ctx, o.id);
            }
        });
    }
}

/// Set `fields` on the dialog on top and confirm it, as its OK does, then let the app act on the answer.
fn confirm_open(app: &mut VectorcraftApp, ctx: &egui::Context, fields: &[(&str, Value)]) {
    let Some(id) = app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().pop().map(|o| o.id))) else {
        crate::dialogs::host::pump(app, ctx);
        return;
    };
    crate::tests::picks::answer_dialog(app, id, fields, true);
}

/// A plain selection of `rows` as the list reports it.
fn set_rows(rows: &[usize]) -> PageListEvent {
    PageListEvent::Selection(SelectionChange::Set(rows.to_vec()))
}

/// Whether the panel menu's command `id` is enabled.
fn enabled_state(app: &VectorcraftApp, id: &'static str) -> bool {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    states.get(&CommandRef::new(id)).is_some_and(CommandState::is_enabled)
}

/// Every event the list reports and every row of the panel menu reaches the artboard commands: the
/// artboard state, the tool, the panel's view state and the undo steps are what each one changes.
#[test]
fn list_events_and_menu_rows_reach_their_commands() {
    // Go and a plain selection make the row active and the only one selected, with any tool.
    for event in [PageListEvent::Go(1), set_rows(&[1])] {
        let mut app = menu_app(3);
        app.select_tool("selection");
        apply(&mut app, vec![event]);
        assert_eq!((selected(&app, 3), picked(&app, 3), app.session.tool_id()), (1, vec![1], "selection"));
    }

    // Several rows, or a toggle, choose the Artboard tool first; the last row of a range is active.
    let mut app = menu_app(4);
    app.select_tool("selection");
    apply(&mut app, vec![set_rows(&[0, 2])]);
    assert_eq!((app.session.tool_id(), picked(&app, 4), selected(&app, 4)), ("artboard", vec![0, 2], 2));
    apply(&mut app, vec![PageListEvent::Selection(SelectionChange::Toggle(3))]);
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![0, 2, 3], 3), "Ctrl/Cmd-click adds");
    apply(&mut app, vec![PageListEvent::Selection(SelectionChange::Toggle(0))]);
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![2, 3], 2), "and takes out");
    app.select_tool("selection");
    apply(&mut app, vec![PageListEvent::Go(1), PageListEvent::Selection(SelectionChange::Toggle(3))]);
    assert_eq!((app.session.tool_id(), picked(&app, 4)), ("artboard", vec![1, 3]), "a toggle with another tool chooses this one");

    // A name goes to every picked artboard the row is one of, numbered, in one undo step; a blank one changes nothing.
    let steps = undo_steps(&app);
    apply(&mut app, vec![PageListEvent::rename(1, "  ")]);
    assert_eq!(undo_steps(&app), steps);
    apply(&mut app, vec![PageListEvent::rename(1, "Page")]);
    assert_eq!(board_names(&app), ["Artboard 1", "Page-1", "Artboard 3", "Page-2"]);
    assert_eq!(undo_steps(&app), steps + 1);

    // A finished move reorders and makes the moved artboard active; a move to where it is changes nothing.
    let mut app = menu_app(3);
    let steps = undo_steps(&app);
    let mv = |to| ReorderMove::new(0, vec![0], to, false);
    apply(&mut app, vec![PageListEvent::Reorder(EditEvent::set(Mixed::Same(mv(0)), mv(0), Arc::from("Move"), None))]);
    assert_eq!(undo_steps(&app), steps);
    apply(&mut app, vec![PageListEvent::Reorder(EditEvent::set(Mixed::Same(mv(0)), mv(1), Arc::from("Move Down"), None))]);
    assert_eq!(
        (board_names(&app), selected(&app, 3), undo_steps(&app)),
        (vec!["Artboard 2".into(), "Artboard 1".into(), "Artboard 3".into()], 1, steps + 1)
    );

    // The lock click locks and unlocks in one undo step each; Delete leaves a locked artboard.
    let mut app = menu_app(3);
    let steps = undo_steps(&app);
    let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    apply(&mut app, vec![PageListEvent::lock(1, true)]);
    assert_eq!((locked(&app), undo_steps(&app)), (vec![false, true, false], steps + 1));
    select(&mut app, 1);
    apply(&mut app, vec![PageListEvent::Delete(vec![1])]);
    assert_eq!(board_names(&app).len(), 3, "a locked artboard stays");
    apply(&mut app, vec![PageListEvent::lock(1, false)]);
    assert_eq!((locked(&app), undo_steps(&app)), (vec![false; 3], steps + 2));
    apply(&mut app, vec![PageListEvent::Delete(vec![1])]);
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 3"]);

    // New adds one and makes it active; a dropped row duplicates its artboard; Alt asks for options first.
    let mut app = menu_app(3);
    apply(&mut app, vec![PageListEvent::new_row(None, false)]);
    assert_eq!((board_names(&app).len(), selected(&app, 4)), (4, 3));
    apply(&mut app, vec![PageListEvent::new_row(Some(0), false)]);
    assert!(board_names(&app).contains(&"Artboard 1 copy".to_string()));
    apply(&mut app, vec![PageListEvent::new_row(None, true)]);
    assert_eq!(board_names(&app).len(), 5, "no artboard is added before OK");
    assert!(is_open::<ArtboardOptionsDialog>(&app));
    cancel_open(&mut app, &crate::testing::ui_context());

    // Options, Fit and Rearrange.
    let mut app = menu_app(3);
    apply(&mut app, vec![PageListEvent::Options(2)]);
    assert_eq!((selected(&app, 3), is_open::<ArtboardOptionsDialog>(&app)), (2, true));
    cancel_open(&mut app, &crate::testing::ui_context());
    apply(&mut app, vec![PageListEvent::Fit(1)]);
    assert_eq!(app.view().unwrap().artboard, 1);
    apply(&mut app, vec![PageListEvent::Rearrange]);
    assert!(is_open::<RearrangeArtboardsDialog>(&app));
    cancel_open(&mut app, &crate::testing::ui_context());

    // The panel menu's rows: Rename asks the list to open a field; another id is not the panel's.
    let mut app = menu_app(3);
    select(&mut app, 2);
    assert!(run(&mut app, "ui.artboards.rename", &json!({})).is_some());
    assert_eq!(app.ui.artboards_ui.rename, Some(2));
    assert!(run(&mut app, "ui.artboards.nothing", &json!({})).is_none());
    assert_eq!(menu_rows(&app).len(), 12);
}

/// #1006: Select › All on Active Artboard selects the art of the artboard made active here,
/// and an explicit artboard still wins.
#[test]
fn all_on_active_artboard_takes_the_active_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
    let mut rect = |x| app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let (a, b) = (rect(10), rect(130));
    let picked = |app: &mut VectorcraftApp, params| {
        crate::menus::invoke(app, "select.allOnArtboard", params);
        let ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|n| n.0).collect();
        json!(ids)
    };
    select(&mut app, 1);
    assert_eq!(picked(&mut app, json!({})), json!([b]));
    select(&mut app, 0);
    assert_eq!(picked(&mut app, json!({})), json!([a]));
    assert_eq!(picked(&mut app, json!({"artboard": 1})), json!([b]));
}

/// Object › Artboards › Fit to Artwork Bounds and Fit to Selected Art resize the artboard made
/// active here, as the panel menu's Fit to Artwork Bounds does, and leave artboard 1 as it was;
/// an explicit index still wins.
#[test]
fn fit_to_artwork_bounds_and_selected_art_resize_the_active_artboard() {
    for id in ["artboard.fitToArt", "artboard.fitToSelection"] {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
        // Partly on artboard 1 and partly on artboard 2.
        app.session.execute("shape.rectangle", &json!({"x": 90, "y": 10, "width": 60, "height": 30})).unwrap();
        app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
        let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
        let (first, art) = (boards(&app)[0], vectorcraft_geom::Rect::new(90.0, 10.0, 150.0, 40.0));
        select(&mut app, 1);
        crate::menus::invoke(&mut app, id, json!({}));
        assert_eq!(boards(&app), [first, art], "{id} resizes Artboard 2, the active one");
        crate::menus::invoke(&mut app, id, json!({"index": 0}));
        assert_eq!(boards(&app), [art, art], "{id} with an index resizes that artboard");
    }
}

/// With artboards 1 and 3 selected with the Artboard tool, Object › Artboards › Fit to Artwork
/// Bounds fits each of them to the art on it and leaves artboard 2 as it was.
#[test]
fn fit_to_artwork_bounds_fits_each_selected_artboard_to_its_own_art() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    for (x, h) in [(10, 30), (130, 20), (250, 40)] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": h})).unwrap();
    }
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = boards(&app);
    app.select_tool("artboard");
    select(&mut app, 0);
    app.session.set_tool_option("selected", &json!([0, 2]));
    crate::menus::invoke(&mut app, "artboard.fitToArt", json!({}));
    let rect = vectorcraft_geom::Rect::new;
    assert_eq!(boards(&app), [rect(10.0, 10.0, 30.0, 40.0), was[1], rect(250.0, 10.0, 270.0, 50.0)]);
}

/// A batch the app runs (as the Actions panel plays an action) with a Fit to Artwork Bounds step
/// fits artboards 1 and 3, selected with the Artboard tool, and leaves artboard 2, the active one,
/// as it was.
#[test]
fn a_batchs_fit_to_artwork_bounds_step_fits_each_selected_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    for (x, h) in [(10, 30), (130, 20), (250, 40)] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": h})).unwrap();
    }
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = boards(&app);
    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.set_tool_option("selected", &json!([0, 2]));
    app.run("command.batch", json!({"commands": [{"command": "artboard.fitToArt", "params": {}}]})).unwrap();
    let rect = vectorcraft_geom::Rect::new;
    assert_eq!(boards(&app), [rect(10.0, 10.0, 30.0, 40.0), was[1], rect(250.0, 10.0, 270.0, 50.0)]);
}

/// With the Artboard tool, a range of rows (Shift-click) selects those artboards and makes the last one
/// active, Ctrl/Cmd-click adds an artboard to the tool's selection or takes it out (#1139), a plain
/// click selects one alone; with another tool, a range or a toggle chooses the Artboard tool first (#5698).
#[test]
fn shift_clicking_rows_selects_several_artboards() {
    let mut app = menu_app(4);
    select(&mut app, 1);
    apply(&mut app, vec![set_rows(&[1, 2, 3])]);
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1, 2, 3], 3));
    apply(&mut app, vec![set_rows(&[1, 2])]);
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1, 2], 2), "the next range from the same row");
    apply(&mut app, vec![set_rows(&[0]), PageListEvent::Go(0)]);
    assert_eq!(picked(&app, 4), [0]);
    apply(&mut app, vec![PageListEvent::Selection(SelectionChange::Toggle(2))]);
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![0, 2], 2), "Ctrl/Cmd-click adds Artboard 3");
    apply(&mut app, vec![PageListEvent::Selection(SelectionChange::Toggle(2))]);
    assert_eq!(picked(&app, 4), [0], "and takes it out");
    apply(&mut app, vec![PageListEvent::Selection(SelectionChange::Toggle(0))]);
    assert_eq!(picked(&app, 4), [0], "the only selected artboard stays selected");
    // With another tool, a plain click makes the row's artboard active and keeps the tool.
    app.select_tool("selection");
    apply(&mut app, vec![PageListEvent::Go(2), set_rows(&[2])]);
    assert_eq!((app.session.tool_id(), selected(&app, 4)), ("selection", 2));
    apply(&mut app, vec![set_rows(&[1, 2, 3])]);
    assert_eq!((app.session.tool_id(), picked(&app, 4), selected(&app, 4)), ("artboard", vec![1, 2, 3], 3));
}

/// With artboards 1, 3 and 4 selected with the Artboard tool, the panel menu's Rename opens the name
/// field on the active artboard's row, and the name typed there goes to all three, numbered in panel
/// order, in one undo step. With one artboard selected, Rename gives it the name as typed.
#[test]
fn the_panel_menus_rename_numbers_several_selected_artboards() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800, 1200] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 2);
    app.session.set_tool_option("selected", &json!([2, 0, 3]));
    run(&mut app, "ui.artboards.rename", &json!({})).unwrap().unwrap();
    assert_eq!(app.ui.artboards_ui.rename.take(), Some(2), "the field opens on the active artboard's row");
    apply(&mut app, vec![PageListEvent::rename(2, "abc")]);
    assert_eq!(board_names(&app), ["abc-1", "Artboard 2", "abc-2", "abc-3"]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 4"], "one undo step");
    // One artboard selected: the name typed replaces its own.
    select(&mut app, 1);
    run(&mut app, "ui.artboards.rename", &json!({})).unwrap().unwrap();
    assert_eq!(app.ui.artboards_ui.rename.take(), Some(1));
    apply(&mut app, vec![PageListEvent::rename(1, "Solo")]);
    assert_eq!(board_names(&app), ["Artboard 1", "Solo", "Artboard 3", "Artboard 4"]);
}

/// Paste run on the session alone (`session.execute`, which leaves the view as it is) with the
/// Artboard tool selects the pasted artboard and leaves the active artboard where it was. The
/// panel menu's Rename then opens the name field on the selected artboard's row and renames
/// that artboard, not the active one.
#[test]
fn the_panel_menus_rename_opens_on_a_selected_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.execute("edit.copy", &json!({})).unwrap();
    app.session.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((selected(&app, 3), picked(&app, 3)), (1, vec![2]), "Artboard 2 is active, and the pasted Artboard 3 is selected");
    run(&mut app, "ui.artboards.rename", &json!({})).unwrap().unwrap();
    assert_eq!(app.ui.artboards_ui.rename, Some(2), "the field opens on Artboard 3's row");
    apply(&mut app, vec![PageListEvent::rename(2, "Pasted")]);
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 2", "Pasted"]);
}

/// With the Artboard tool, Paste and Paste in Place make the pasted artboard the active one
/// (the navigator's and the panel's), and Cut makes the artboard before the cut one active, the
/// one the tool then has active.
#[test]
fn paste_and_cut_with_the_artboard_tool_make_its_artboard_active() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["active"].clone());
    app.run("edit.copy", json!({})).unwrap();
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(active(&app), (3, json!(3)), "Paste: the pasted Artboard 4");
    app.run("edit.pasteInPlace", json!({})).unwrap();
    assert_eq!(active(&app), (4, json!(4)), "Paste in Place: the pasted Artboard 5");
    select(&mut app, 2);
    app.run("edit.cut", json!({})).unwrap();
    assert_eq!(active(&app), (1, json!(1)), "Cut Artboard 3: Artboard 2");
}

/// With the Artboard tool selecting several artboards, Paste in Place, in Front and in Back put a
/// copy of the copied object on each, where it was on the artboard it was copied from; with one
/// selected, on the active one.
#[test]
fn in_place_pastes_put_a_copy_on_each_artboard_the_artboard_tool_selects() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.origin()).collect();
    let r = app.run("shape.rectangle", json!({"x": boards[0].x + 10.0, "y": boards[0].y + 20.0, "width": 5, "height": 5})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [r]})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.select_tool("artboard");
    let offset = vectorcraft_geom::Vec2::new(10.0, 20.0);
    let pasted = |app: &mut VectorcraftApp, id: &str| -> Vec<vectorcraft_geom::Point> {
        let r = app.run(id, json!({})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        r["ids"].as_array().unwrap().iter().map(|i| doc.bounds_of(&[vectorcraft_doc::NodeId(i.as_u64().unwrap())], false).unwrap().origin()).collect()
    };
    for id in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        app.run("tool.setOption", json!({"key": "selected", "value": [2, 0]})).unwrap();
        assert_eq!(pasted(&mut app, id), [boards[0] + offset, boards[2] + offset], "{id}");
        app.run("tool.setOption", json!({"key": "selected", "value": [1]})).unwrap();
        assert_eq!(pasted(&mut app, id), [boards[1] + offset], "{id}: the active artboard");
    }
}

/// `tool.setOption` with the Artboard tool's `selected` or `active` makes the artboard the tool
/// then has active the active one.
#[test]
fn the_artboard_tools_options_make_its_artboard_active() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    app.run("tool.setOption", json!({"key": "selected", "value": [2, 0]})).unwrap();
    assert_eq!((app.view().unwrap().artboard, picked(&app, 3)), (2, vec![0, 2]));
    app.run("tool.setOption", json!({"tool": "artboard", "values": {"active": 1}})).unwrap();
    assert_eq!((app.view().unwrap().artboard, picked(&app, 3)), (1, vec![1]));
}

/// With the Artboard tool, a `command.batch` that stays in the document makes the artboard the
/// tool then has active the active one, as its steps do on their own: a batch that leaves the
/// tool's artboards keeps the active artboard, and Copy and Paste make the pasted one active.
#[test]
fn a_batch_in_one_document_makes_the_artboard_tools_artboard_active() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["active"].clone());
    app.run("command.batch", json!({"commands": [{"command": "object.unlockAll", "params": {}}]})).unwrap();
    assert_eq!(active(&app), (1, json!(1)));
    let steps = [json!({"command": "edit.copy", "params": {}}), json!({"command": "edit.paste", "params": {}})];
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(active(&app), (3, json!(3)), "the pasted Artboard 4");
}

/// With the Artboard tool, `document.activate` makes the active artboard of the document it
/// switches to the tool's, as choosing the document's tab does.
#[test]
fn switching_documents_with_the_artboard_tool_keeps_the_active_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 2);
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (2, json!([2])));
}

/// With the Artboard tool, a `command.batch` that switches to another document and back keeps
/// the active artboard, and the tool starts on it. When a later step of the batch pastes an
/// artboard, the pasted one becomes active, as Paste on its own makes it. A later Cut of the
/// tool's artboard 1 makes the new first artboard active, as Cut on its own makes it.
#[test]
fn a_batch_that_switches_documents_and_back_keeps_the_active_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 2);
    let step = |command: &str, params: Value| json!({"command": command, "params": params});
    let back = [step("document.activate", json!({"index": 1})), step("document.activate", json!({"index": 0}))];
    app.run("command.batch", json!({"commands": back})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (2, json!([2])));
    app.run("edit.copy", json!({})).unwrap();
    let mut steps = back.to_vec();
    steps.push(step("edit.paste", json!({})));
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (3, json!([3])), "the pasted Artboard 4");
    let mut steps = back.to_vec();
    steps.push(step("edit.cut", json!({})));
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.artboards.len(), 3, "Artboard 1 cut");
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (0, json!([0])), "the new first artboard");
}

/// The panel menu's Artboard Options… and a row's options icon open Artboard Options for the active
/// artboard and for the row's artboard, and leave the tool as it is.
#[test]
fn the_panel_menus_artboard_options_opens_the_active_artboards_options() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    select(&mut app, 1);
    let tool = app.session.tool_id().to_string();
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("tool.options", json!({"tool": "artboard"})).unwrap();
    assert!(is_open::<ArtboardOptionsDialog>(&app));
    let fields = dialog_fields(&app);
    assert_eq!((fields["index"].as_f64(), fields["x"].as_f64()), (Some(1.0), Some(400.0)));
    assert_eq!(app.session.tool_id(), tool);
    cancel_open(&mut app, &ctx);
    apply(&mut app, vec![PageListEvent::Options(0)]);
    let fields = dialog_fields(&app);
    assert_eq!((fields["index"].as_f64(), fields["width"].as_f64()), (Some(0.0), Some(200.0)));
    assert_eq!(app.session.tool_id(), tool);
    assert!(enabled_state(&app, "ui.artboards.rename"));
}

/// With two artboards selected with the Artboard tool, the panel menu's Duplicate Artboards
/// copies both, with their art, in one undo step, and the tool then selects the copies, the
/// active artboard's copy active.
#[test]
fn the_panel_menus_duplicate_artboards_copies_every_selected_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    for x in [10, 410] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.set_tool_option("selected", &json!([1, 0]));
    run(&mut app, "ui.artboards.duplicate", &json!({})).unwrap().unwrap();
    let d = &app.session.active().unwrap().doc;
    let names: Vec<&str> = d.artboards.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 1 copy", "Artboard 2 copy"]);
    assert_eq!(d.layers[0].children().unwrap().len(), 4, "the squares and their copies");
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (4, json!([4, 3])), "Artboard 2's copy is active");
    app.run("edit.undo", json!({})).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "one undo step");
}

/// The panel menu's New Artboard adds one artboard in one undo step and makes it the active one, as
/// the panel's button does, with any tool. With the Artboard tool, the new artboard is then the only
/// one selected.
#[test]
fn the_panel_menus_new_artboard_makes_the_new_artboard_active() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    run(&mut app, "ui.artboards.new", &json!({})).unwrap().unwrap();
    assert_eq!(boards(&app).2.len(), 3);
    assert_eq!(selected(&app, 3), 2, "the new artboard is active with the Selection tool");
    let undo_steps = app.session.active().unwrap().history.undo.len();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app).2.len(), 2, "one undo step removes the new artboard");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo_steps - 1);

    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.set_tool_option("selected", &json!([1, 0]));
    run(&mut app, "ui.artboards.new", &json!({})).unwrap().unwrap();
    assert_eq!(boards(&app).2.len(), 3);
    assert_eq!(app.view().unwrap().artboard, 2, "the new artboard is active with the Artboard tool");
    assert_eq!(app.session.tool_options()["active"], json!(2));
    assert_eq!(app.session.tool_options()["selected"], json!([2]), "and is the only one selected");
}

/// The panel menu's Convert to Artboards is a plain command that is off with nothing selected; with
/// rectangles selected it turns the geometric bounds of each into a new artboard and removes the
/// rectangles, in one undo step, as Object › Artboards › Convert to Artboards does.
#[test]
fn the_panel_menus_convert_to_artboards_converts_the_selected_objects() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [10, 60] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 30})).unwrap();
    }
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let first = boards(&app)[0];
    app.session.execute("select.none", &json!({})).unwrap();
    assert!(!enabled_state(&app, "artboard.convertToArtboards"), "off with nothing selected");
    app.session.execute("select.all", &json!({})).unwrap();
    assert!(enabled_state(&app, "artboard.convertToArtboards"));
    crate::menus::invoke(&mut app, "artboard.convertToArtboards", json!({}));
    let rect = vectorcraft_geom::Rect::new;
    assert_eq!(boards(&app), [first, rect(10.0, 10.0, 30.0, 40.0), rect(60.0, 10.0, 80.0, 40.0)]);
    let d = app.session.active().unwrap();
    assert_eq!((d.doc.layers[0].children().unwrap().len(), d.selection.objects.len()), (0, 0), "the rectangles are removed");
    app.run("edit.undo", json!({})).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (1, 2), "one undo step");
}

/// The artboards' rectangles, the undo steps and the artboards' names.
fn boards(app: &VectorcraftApp) -> (Vec<vectorcraft_geom::Rect>, usize, Vec<String>) {
    let st = app.session.active().unwrap();
    (st.doc.artboards.iter().map(|a| a.rect).collect(), st.history.undo.len(), st.doc.artboards.iter().map(|a| a.name.clone()).collect())
}

#[test]
fn control_bar_name_cancels_typing_and_commits_one_rename() {
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};

    fn draw(app: &mut VectorcraftApp, ui: &mut Ui) {
        assert!(control_bar(app, ui, true, false));
    }

    for commit in [Key::Enter, Key::Tab] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        select(&mut app, 1);
        let mut panel = Panel::new(app, draw);
        panel.room(egui::vec2(1000.0, 500.0));
        let original = panel.app().session.active().unwrap().doc.clone();
        let revision = panel.app().session.active().unwrap().revision;
        let undo = undo_steps(&panel.app());
        let name = panel.value_role("Name:", Role::TEXT_INPUT);
        panel.click_role("Name:", Role::TEXT_INPUT);
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("Discarded");
        panel.type_text(" draft");
        assert_eq!(panel.value_role("Name:", Role::TEXT_INPUT).as_deref(), Some("Discarded draft"));
        assert_eq!(panel.app().session.active().unwrap().doc, original, "typing only changes the field draft");
        panel.key(Key::Escape, Modifiers::NONE);
        assert_eq!(panel.value_role("Name:", Role::TEXT_INPUT), name);
        assert_eq!(panel.app().session.active().unwrap().doc, original);
        assert_eq!(panel.app().session.active().unwrap().revision, revision);
        assert_eq!(undo_steps(&panel.app()), undo);

        panel.click_role("Name:", Role::TEXT_INPUT);
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("Renamed");
        panel.type_text(" board");
        panel.key(commit, Modifiers::NONE);
        {
            let app = panel.app();
            let doc = &app.session.active().unwrap().doc;
            assert_eq!(doc.artboards[1].name, "Renamed board");
            assert_eq!(doc.artboards[0], original.artboards[0]);
            assert_eq!(doc.artboards[2], original.artboards[2]);
            assert_eq!(undo_steps(&app), undo + 1);
        }
        panel.app().run("edit.undo", json!({})).unwrap();
        panel.frame();
        assert_eq!(panel.app().session.active().unwrap().doc, original);
    }
}

/// The Control bar's size fields resize the active artboard about the reference point in one undo
/// step: about its center, and from its top-left corner once the top-left point is chosen.
#[test]
fn the_control_bar_resizes_the_active_artboard_about_the_reference_point() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 100, "units": "Points", "artboards": 2})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let (before, undo, _) = boards(&app);
    let b = before[1];
    // PLAN: assumed `transform_panel.reference_point` is the 9-point locator's index (4 the center, 0 the top-left).
    app.ui.transform_panel.reference_point = 4;
    let refi = app.ui.transform_panel.reference_point;
    crate::panels::transform::set_artboard(&mut app, 1, refi, [None, None, Some(300.0), None], false);
    let mut want = before.clone();
    want[1] = vectorcraft_geom::Rect::new(b.x0 - 50.0, b.y0, b.x1 + 50.0, b.y1);
    assert_eq!(boards(&app).0, want, "Artboard 2 widens about its center");
    assert_eq!(boards(&app).1, undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app).0, before);
    app.ui.transform_panel.reference_point = 0;
    let refi = app.ui.transform_panel.reference_point;
    crate::panels::transform::set_artboard(&mut app, 1, refi, [None, None, Some(300.0), None], false);
    want[1] = vectorcraft_geom::Rect::new(b.x0, b.y0, b.x0 + 300.0, b.y1);
    assert_eq!(boards(&app).0, want, "Artboard 2 widens from its left edge");
}

/// Choosing the Artboard tool starts it on the active artboard, the one the panel highlights:
/// Artboard Options shows that artboard, the pointer moving over another artboard leaves it
/// active, and Delete removes it. Shift+O while the tool is in use keeps the artboards it has
/// selected.
#[test]
fn choosing_the_artboard_tool_starts_it_on_the_active_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    select(&mut app, 1);
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    assert_eq!(app.session.tool_options()["active"], json!(1));
    assert_eq!(app.ui.artboards_ui.rename, None);
    app.run("tool.options", json!({"tool": "artboard"})).unwrap();
    assert_eq!(dialog_fields(&app)["index"].as_f64(), Some(1.0), "Artboard Options shows the active artboard");
    let ctx = app.ui_ctx.clone().expect("a window");
    cancel_open(&mut app, &ctx);
    let view = app.view_info();
    crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Move, 100.0, 75.0), view);
    assert_eq!(selected(&app, 3), 1, "the pointer over Artboard 1 leaves Artboard 2 active");
    let r = app.session.tool_key(vectorcraft_tools::ToolKey::Delete, Default::default(), view);
    crate::canvas::apply_requests(&mut app, r);
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 3"], "Delete removes Artboard 2");
    // Shift+O while the tool is in use keeps the artboards it has selected.
    app.session.set_tool_option("selected", &json!([0, 1]));
    let shift_o = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::Text("O".into())];
    // PLAN: assumed `tests::synthetic::key_frame(app, ctx, events)` (kb.C26, shortcuts-w10).
    crate::tests::synthetic::key_frame(&mut app, &ctx, shift_o);
    assert_eq!((app.session.tool_id(), picked(&app, 2)), ("artboard", vec![0, 1]));
}

/// A row dropped on New Artboard duplicates its artboard with its art (#446), and the copy becomes the
/// active artboard; a plain New adds an empty one.
#[test]
fn dragging_a_row_onto_new_artboard_duplicates_it() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    apply(&mut app, vec![PageListEvent::new_row(Some(0), false)]);
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (2, 2), "the artboard and its art");
    assert_eq!(app.view().unwrap().artboard, 1);
    apply(&mut app, vec![PageListEvent::new_row(None, false)]);
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "a plain New adds no art");
}

/// The row menu's Rename runs `artboard.rename`, which opens the Rename dialog and renames nothing
/// until OK: on a row of the selection, the dialog takes every selected artboard and OK numbers the name
/// in one undo step; on a row that isn't selected, that row's artboard first becomes the only selected
/// one and the dialog takes it alone, and Cancel keeps its name.
#[test]
fn the_row_menus_rename_opens_the_rename_dialog_for_the_selected_artboards() {
    let mut app = menu_app(4);
    let ctx = app.ui_ctx.clone().expect("a window");
    select(&mut app, 2);
    app.session.set_tool_option("selected", &json!([2, 1]));
    let steps = undo_steps(&app);
    crate::menus::invoke(&mut app, "artboard.rename", json!({}));
    assert!(open_keys(&app).iter().any(|k| k == rename_artboards::CMD), "the Rename dialog");
    let fields = dialog_fields(&app);
    // The artboards are held by the dialog's OK, not a field (document-panels-w11): OK below shows which it took.
    assert_eq!(fields["name"].as_str(), Some(""), "the dialog takes both selected artboards");
    assert_eq!(picked(&app, 4), [1, 2], "the selection stays");
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 4"], "nothing is renamed before OK");
    assert_eq!(undo_steps(&app), steps);
    confirm_open(&mut app, &ctx, &[("name", json!("Page"))]);
    assert_eq!(board_names(&app), ["Artboard 1", "Page-1", "Page-2", "Artboard 4"]);
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");

    // The list selects an unpicked row on the secondary press, before the menu's Rename runs.
    apply(&mut app, vec![set_rows(&[3])]);
    crate::menus::invoke(&mut app, "artboard.rename", json!({}));
    let fields = dialog_fields(&app);
    assert_eq!(fields["name"].as_str(), Some("Artboard 4"), "the dialog takes the row's artboard alone");
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![3], 3), "the row's artboard is the only one selected");
    cancel_open(&mut app, &ctx);
    assert!(open_keys(&app).is_empty());
    assert_eq!(board_names(&app), ["Artboard 1", "Page-1", "Page-2", "Artboard 4"]);
    assert_eq!(undo_steps(&app), steps + 1, "Cancel records no undo step");
}

/// Without the Artboard tool the row menu's Rename takes the right-clicked row's artboard, which
/// becomes the active one; a locked artboard can be renamed too.
#[test]
fn the_row_menu_renames_the_right_clicked_artboard_with_any_tool() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 100, "height": 50, "units": "Points", "artboards": 3})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 2, "locked": true})).unwrap();
    select(&mut app, 0);
    let ctx = app.ui_ctx.clone().expect("a window");
    apply(&mut app, vec![set_rows(&[1])]);
    assert_eq!((app.session.tool_id(), selected(&app, 3)), (app.session.tool_id(), 1), "the right-clicked artboard is active, the tool unchanged");
    apply(&mut app, vec![set_rows(&[2])]);
    crate::menus::invoke(&mut app, "artboard.rename", json!({}));
    let fields = dialog_fields(&app);
    assert_eq!(fields["name"].as_str(), Some("Artboard 3"));
    confirm_open(&mut app, &ctx, &[("name", json!("Back"))]);
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 2", "Back"], "the locked artboard is renamed");
}

/// The lock click locks and unlocks an artboard, each one undo step, and Delete Artboard leaves a
/// locked artboard.
#[test]
fn the_lock_column_locks_and_unlocks_an_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 0, "width": 200, "height": 150})).unwrap();
    select(&mut app, 1);
    let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    apply(&mut app, vec![PageListEvent::lock(1, true)]);
    assert_eq!(locked(&app), [false, true]);
    assert!(deletable(&app, 2).is_empty(), "Delete Artboard leaves it");
    delete_selected(&mut app);
    assert_eq!(app.session.active().unwrap().doc.artboards.len(), 2);
    apply(&mut app, vec![PageListEvent::lock(1, false)]);
    assert_eq!(locked(&app), [false, false]);
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(locked(&app), [false, true], "one undo step");
}

/// A new size for the selected artboards leaves a locked one among them as it is. The
/// artboard the field belongs to stays listed when it is locked, so `artboard.setProps`
/// refuses the change. A name typed for them goes to the locked one too.
#[test]
fn a_new_size_for_the_selected_artboards_leaves_a_locked_one() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &json!([0, 1, 2]));
    app.session.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert_eq!(resized_with(&app, 0), [0, 2]);
    assert_eq!(resized_with(&app, 1), [0, 1, 2], "the field's own artboard stays listed");
    crate::panels::transform::set_artboard(&mut app, 0, 0, [None, None, Some(300.0), None], false);
    let widths = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect::<Vec<_>>();
    assert_eq!(widths, [300.0, 200.0, 300.0]);
    let all = renamed_with(&app, 0);
    assert_eq!(all, [0, 1, 2]);
    rename(&mut app, &all, "abc").unwrap();
    let names = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    assert_eq!(names, ["abc-1", "abc-2", "abc-3"]);
}

/// New with Alt opens Artboard Options for the new artboard and adds nothing yet: the fields are those
/// a plain New's artboard gets. Cancel adds nothing; OK adds the artboard with the fields in one undo
/// step and makes it the active one.
#[test]
fn alt_clicking_new_artboard_opens_artboard_options_for_the_new_artboard() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    select(&mut app, 0);
    let ctx = app.ui_ctx.clone().expect("a window");
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.len();
    // Alt: the dialog for Artboard 3, 20 pt right of Artboard 2 and as large; no artboard yet.
    apply(&mut app, vec![PageListEvent::new_row(None, true)]);
    assert!(is_open::<ArtboardOptionsDialog>(&app));
    let fields = dialog_fields(&app);
    assert_eq!(fields["name"].as_str(), Some("Artboard 3"));
    assert_eq!(["x", "y", "width", "height"].map(|k| fields[k].as_f64()), [Some(520.0), Some(300.0), Some(100.0), Some(50.0)]);
    assert_eq!(count(&app), 2, "no artboard is added before OK");
    // A plain New adds the artboard the dialog showed, with no dialog.
    cancel_open(&mut app, &ctx);
    apply(&mut app, vec![PageListEvent::new_row(None, false)]);
    let ab = app.session.active().unwrap().doc.artboards[2].clone();
    assert_eq!((ab.name.as_str(), ab.rect), ("Artboard 3", vectorcraft_geom::Rect::new(520.0, 300.0, 620.0, 350.0)));
    assert!(open_keys(&app).is_empty());
    app.run("edit.undo", json!({})).unwrap();
    select(&mut app, 0);
    // Cancel adds nothing.
    apply(&mut app, vec![PageListEvent::new_row(None, true)]);
    cancel_open(&mut app, &ctx);
    assert_eq!((count(&app), app.view().unwrap().artboard), (2, 0));
    // OK adds it with the fields, in one undo step, and makes it the active artboard.
    apply(&mut app, vec![PageListEvent::new_row(None, true)]);
    let undo = app.session.doc().unwrap().history.undo.len();
    confirm_open(&mut app, &ctx, &[("name", json!("Cover")), ("width", json!(300))]);
    let ab = app.session.active().unwrap().doc.artboards[2].clone();
    assert_eq!((ab.name.as_str(), ab.rect), ("Cover", vectorcraft_geom::Rect::new(520.0, 300.0, 820.0, 350.0)));
    assert_eq!((app.view().unwrap().artboard, app.session.doc().unwrap().history.undo.len()), (2, undo + 1));
    assert!(open_keys(&app).is_empty());
}

/// `ui.newArtboard` opens Artboard Options and fails without a document. A name left empty gives the new
/// artboard its default name, and with the Artboard tool in use the tool moves to the new artboard.
#[test]
fn new_artboard_options_with_an_empty_name_add_the_default_name() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    let ctx = app.ui_ctx.clone().expect("a window");
    assert!(app.run("ui.newArtboard", json!({})).is_err(), "no document");
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.select_tool("artboard");
    app.run("ui.newArtboard", json!({})).unwrap();
    assert!(is_open::<ArtboardOptionsDialog>(&app));
    confirm_open(&mut app, &ctx, &[("name", json!(" "))]);
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 2"]);
    assert_eq!((selected(&app, 2), app.session.tool_options()["active"].clone()), (1, json!(1)));
}

/// The panel menu's Delete Empty Artboards is disabled while every artboard has art, and
/// enabled otherwise; running it deletes the empty ones in one undo step and makes the artboard
/// left active.
#[test]
fn delete_empty_artboards_deletes_the_empty_ones_and_selects_one_left() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    let rect =
        |app: &mut VectorcraftApp, x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let ids: Vec<Value> = [10.0, 450.0, 850.0].into_iter().map(|x| rect(&mut app, x)).collect();
    assert!(!enabled_state(&app, "ui.artboards.deleteEmpty"), "every artboard has art");
    app.run("select.set", json!({"ids": [ids[0], ids[2]]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    select(&mut app, 2);
    assert!(enabled_state(&app, "ui.artboards.deleteEmpty"), "two artboards are empty");
    let steps = undo_steps(&app);
    run(&mut app, "ui.artboards.deleteEmpty", &json!({})).unwrap().unwrap();
    assert_eq!(board_names(&app), ["Artboard 2"]);
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    assert_eq!(selected(&app, 1), 0, "the artboard left is active");
}

/// With the Artboard tool, the canvas context menu locks and unlocks artboards: Lock Selected
/// locks the selected one (the others stay free), Unlock All frees every artboard.
#[test]
fn the_artboard_tools_context_menu_locks_and_unlocks_artboards() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    let run = |app: &mut VectorcraftApp, menu: &str, label: &str| {
        let items = crate::menus::context_items(app);
        let Some(sub) = items.iter().find(|i| label_of(i) == menu && !children_of(i).is_empty()) else { panic!("no {menu}") };
        let Some((id, p)) = children_of(sub).iter().find(|i| label_of(i) == label).and_then(command_of) else { panic!("no {label}") };
        crate::menus::invoke(app, id, p.clone());
    };
    // Without the Artboard tool the menu has no such submenu.
    assert!(!menu_labels(&crate::menus::context_items(&app)).contains(&"Lock Artboards"));
    app.select_tool("artboard");
    select(&mut app, 1);
    run(&mut app, "Lock Artboards", "Lock Selected");
    assert_eq!(locked(&app), [false, true, false]);
    run(&mut app, "Lock Artboards", "Lock All Except Selected");
    assert_eq!(locked(&app), [true, true, true], "the others lock, the locked one stays");
    run(&mut app, "Unlock Artboards", "Unlock Selected");
    assert_eq!(locked(&app), [true, false, true]);
    run(&mut app, "Unlock Artboards", "Unlock All");
    assert_eq!(locked(&app), [false, false, false]);
}

/// A document of `boards` artboards of 100 × 50 pt with the Artboard tool chosen.
fn menu_app(boards: usize) -> VectorcraftApp {
    menu_app_with(boards, Default::default())
}

/// [`menu_app`] over `services`.
fn menu_app_with(boards: usize, services: crate::Services) -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.session.execute("file.new", &json!({"width": 100, "height": 50, "units": "Points", "artboards": boards})).unwrap();
    app.select_tool("artboard");
    app
}

/// A menu node's label: an item's or a submenu's, empty for the other rows.
fn label_of(node: &crate::menus::Node) -> &str {
    match node {
        crate::menus::Node::Item(item) => &item.label,
        crate::menus::Node::Submenu { label, .. } => label,
        _ => "",
    }
}

/// A menu node's command id and params, when it runs one.
fn command_of(node: &crate::menus::Node) -> Option<(&str, &Value)> {
    match node {
        crate::menus::Node::Item(item) => Some((&item.command.id, &item.command.params)),
        _ => None,
    }
}

/// A menu node's submenu rows.
fn children_of(node: &crate::menus::Node) -> &[crate::menus::Node] {
    match node {
        crate::menus::Node::Submenu { children, .. } => children,
        _ => &[],
    }
}

/// The labels of `items`, submenus by their own label, separators left out.
fn menu_labels(items: &[crate::menus::Node]) -> Vec<&str> {
    items.iter().map(label_of).filter(|l| !l.is_empty()).collect()
}

/// The entry of the Artboard tool's canvas menu named `label`, chosen as a click on it chooses it.
fn choose(app: &mut VectorcraftApp, label: &str) {
    let items = crate::menus::context_items(app);
    let Some((l, (id, p))) = items.iter().filter(|i| label_of(i) == label).find_map(|i| command_of(i).map(|c| (label_of(i), c))) else {
        panic!("no {label}")
    };
    let (id, p) = crate::menus::click_target(l, id, p);
    crate::menus::invoke(app, &id, p);
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

fn board_names(app: &VectorcraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect()
}

/// The Artboard tool's canvas menu lists the public page's entries in the page's order, with
/// nothing for objects or the view; another tool's menu lists none of them.
#[test]
fn the_artboard_tools_menu_lists_the_pages_entries_in_order() {
    let mut app = menu_app(3);
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(
        menu_labels(&crate::menus::context_items(&app)),
        [
            "Cut",
            "Copy",
            "Paste",
            "Undo",
            "Redo",
            "New Artboard",
            "Duplicate Artboards",
            "Rename",
            "Delete Artboards",
            "Delete Empty Artboards",
            "Select All Artboards",
            "Lock Artboards",
            "Rearrange",
            "Fit to Artwork Bounds",
            "Switch Orientation",
            "Artboard Options",
            "Export",
        ]
    );
    app.select_tool("selection");
    let items = crate::menus::context_items(&app);
    let other = menu_labels(&items);
    for label in
        ["New Artboard", "Duplicate Artboards", "Delete Artboards", "Select All Artboards", "Lock Artboards", "Rearrange", "Switch Orientation"]
    {
        assert!(!other.contains(&label), "{label} in {other:?}");
    }
}

/// An entry whose command can't run is left out: Cut with every artboard selected, or a locked one;
/// Delete Artboards in a document of one; Select All Artboards with all selected; Fit to Artwork
/// Bounds without art; Switch Orientation on a square artboard. Rearrange lays out the one selected
/// artboard.
#[test]
fn the_artboard_tools_menu_leaves_out_what_cannot_run() {
    let shown = |app: &VectorcraftApp, label: &str| menu_labels(&crate::menus::context_items(app)).contains(&label);
    let mut app = menu_app(1);
    for label in ["Cut", "Delete Artboards", "Delete Empty Artboards", "Select All Artboards", "Fit to Artwork Bounds"] {
        assert!(!shown(&app, label), "{label} with one empty artboard");
    }
    assert!(shown(&app, "Copy") && shown(&app, "Switch Orientation") && shown(&app, "Rearrange"));
    app.session.execute("artboard.setProps", &json!({"index": 0, "width": 50})).unwrap();
    assert!(!shown(&app, "Switch Orientation"), "a square artboard");

    let mut app = menu_app(2);
    app.session.set_tool_option("selected", &json!([0, 1]));
    assert!(!shown(&app, "Cut"), "every artboard selected");
    assert!(!shown(&app, "Select All Artboards"));
    app.session.set_tool_option("selected", &json!([1]));
    app.session.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    for label in ["Cut", "Delete Artboards", "Switch Orientation"] {
        assert!(!shown(&app, label), "{label} on a locked artboard");
    }
    assert!(shown(&app, "Rename"), "a locked artboard can be renamed");
}

/// With no selection of its own the tool has its active artboard selected, and the menu acts on it.
#[test]
fn the_artboard_tools_menu_acts_on_the_active_artboard_without_a_selection() {
    let mut app = menu_app(3);
    select(&mut app, 1);
    choose(&mut app, "Switch Orientation");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.artboards[1].rect.width(), doc.artboards[1].rect.height()), (50.0, 100.0));
    assert_eq!(doc.artboards[0].rect.width(), 100.0);
}

/// Each entry runs its command on the selected artboards, in one undo step, and the view follows.
#[test]
fn the_artboard_tools_menu_entries_run_on_the_selected_artboards() {
    let mut app = menu_app(4);
    app.session.set_tool_option("selected", &json!([2, 1]));
    app.view_mut().unwrap().artboard = 2;

    let steps = undo_steps(&app);
    choose(&mut app, "Switch Orientation");
    let sizes = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect::<Vec<_>>();
    assert_eq!(sizes(&app), [100.0, 50.0, 50.0, 100.0]);
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");

    choose(&mut app, "Duplicate Artboards");
    assert_eq!(board_names(&app).len(), 6);
    assert_eq!(undo_steps(&app), steps + 2, "one undo step");
    assert_eq!(app.session.tool_options()["selected"], json!([5, 4]), "the copies are selected, artboard 3's copy active");
    assert_eq!(app.view().unwrap().artboard, 5);

    choose(&mut app, "Delete Artboards");
    assert_eq!(board_names(&app).len(), 4);
    assert_eq!(undo_steps(&app), steps + 3, "one undo step");
    assert_eq!(selected(&app, 4), 3, "the artboard before the first one deleted is active");
    assert_eq!(app.session.tool_options()["selected"], json!([3]), "and the only one selected");

    choose(&mut app, "New Artboard");
    assert_eq!(board_names(&app).len(), 5);
    assert_eq!(selected(&app, 5), 4, "the new artboard is active");
    assert_eq!(app.session.tool_options()["selected"], json!([4]), "and the only one selected");

    choose(&mut app, "Select All Artboards");
    assert_eq!(app.session.tool_options()["selected"], json!([4, 0, 1, 2, 3]));
    assert_eq!(undo_steps(&app), steps + 4, "selecting records no undo step");
    assert_eq!(selected(&app, 5), 4, "the active artboard stays active");
}

/// Cut and Copy from the menu take the selected artboards, in one undo step for the cut.
#[test]
fn the_artboard_tools_menu_cuts_and_copies_the_selected_artboards() {
    let mut app = menu_app(4);
    app.session.set_tool_option("selected", &json!([1, 2]));
    app.view_mut().unwrap().artboard = 1;
    choose(&mut app, "Copy");
    assert_eq!(board_names(&app).len(), 4);
    assert_eq!(app.session.clipboard.artboards.len(), 2);
    let steps = undo_steps(&app);
    choose(&mut app, "Cut");
    assert_eq!(board_names(&app), ["Artboard 1", "Artboard 4"]);
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
}

/// The menu's Cut and Copy record which artboards they took, so a replay in a new session, where
/// the Artboard tool isn't chosen, acts on the same artboards.
#[test]
fn the_artboard_tools_menu_replays_its_cut_and_copy_in_a_new_session() {
    let mut app = menu_app(4);
    app.session.set_tool_option("selected", &json!([1, 2]));
    app.view_mut().unwrap().artboard = 1;
    choose(&mut app, "Copy");
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!((cmd.as_str(), &p["selectedArtboards"]), ("artboard.copy", &json!([1, 2])));
    choose(&mut app, "Cut");
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!((cmd.as_str(), &p["selectedArtboards"]), ("artboard.cut", &json!([1, 2])));
    let mut replay = Session::new();
    for (id, p) in &app.session.journal {
        replay.execute(id, p).unwrap();
    }
    let names: Vec<String> = replay.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 4"]);
    assert_eq!(replay.clipboard.artboards.len(), 2);
}

/// The menu's Cut and Copy publish the copied art to the system clipboard, as Edit › Cut and Copy do.
#[test]
fn the_artboard_tools_menu_publishes_its_cut_and_copy() {
    let mut app = menu_app_with(2, crate::tests::sysclip::fake_clipboard_services());
    let on = app.session.active().unwrap().doc.artboards[1].rect;
    app.session.execute("shape.rectangle", &json!({"x": on.x0 + 10.0, "y": on.y0 + 10.0, "width": 20, "height": 20})).unwrap();
    app.session.set_tool_option("selected", &json!([1]));
    app.view_mut().unwrap().artboard = 1;
    for label in ["Copy", "Cut"] {
        assert_eq!(app.clipboard.write_text("").unwrap(), craft::platform::clipboard::Written::Written, "{label}: the clipboard is cleared");
        choose(&mut app, label);
        assert!(app.written_clipboard_text().is_some_and(|svg| svg.contains("<svg")), "{label}");
    }
}

/// Fit to Artwork Bounds fits the selected artboards only, in one undo step, and Undo from the menu
/// takes the step back.
#[test]
fn the_artboard_tools_menu_fits_the_selected_artboards_and_undoes() {
    let mut app = menu_app(2);
    let rects = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let before = rects(&app);
    let on = |r: vectorcraft_geom::Rect| json!({"x": r.x0 + 10.0, "y": r.y0 + 10.0, "width": 20, "height": 20});
    app.session.execute("shape.rectangle", &on(before[0])).unwrap();
    app.session.execute("shape.rectangle", &on(before[1])).unwrap();
    app.session.set_tool_option("selected", &json!([1]));
    app.view_mut().unwrap().artboard = 1;
    let steps = undo_steps(&app);
    choose(&mut app, "Fit to Artwork Bounds");
    let fitted = rects(&app);
    assert_eq!(fitted[0], before[0], "the artboard that isn't selected stays");
    assert_eq!((fitted[1].width(), fitted[1].height()), (21.0, 21.0), "the rectangle with its 1 pt stroke");
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    choose(&mut app, "Undo");
    assert_eq!(rects(&app), before);
}

/// Delete Empty Artboards leaves a remaining artboard active, and the dialog entries open their
/// dialogs: Rename, Rearrange, Artboard Options and Export.
#[test]
fn the_artboard_tools_menu_deletes_empty_artboards_and_opens_dialogs() {
    let mut app = menu_app(3);
    let ctx = app.ui_ctx.clone().expect("a window");
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    // PLAN: assumed `crate::dialogs::ExportForScreensDialog` (ex.A8).
    type IsOpen = fn(&VectorcraftApp) -> bool;
    let opened: [(&str, IsOpen); 4] = [
        ("Rename", |app| open_keys(app).iter().any(|k| k == rename_artboards::CMD)),
        ("Rearrange", is_open::<RearrangeArtboardsDialog>),
        ("Artboard Options", is_open::<ArtboardOptionsDialog>),
        ("Export", is_open::<crate::dialogs::ExportForScreensDialog>),
    ];
    for (label, is_shown) in opened {
        choose(&mut app, label);
        assert!(is_shown(&app), "{label}");
        cancel_open(&mut app, &ctx);
    }
    select(&mut app, 2);
    choose(&mut app, "Delete Empty Artboards");
    assert_eq!(board_names(&app), ["Artboard 1"]);
    assert_eq!(selected(&app, 1), 0);
}

/// The panel menu's Rearrange… lays out the artboards the Artboard tool has selected, once each and
/// in document order; with another tool nothing is selected by it, so Rearrange… is off and opens
/// nothing, while the other entries still act on the active artboard. The list's Rearrange button
/// stays Rearrange All Artboards.
#[test]
fn the_panels_rearrange_lays_out_only_what_the_artboard_tool_selected() {
    let mut app = menu_app(3);
    let ctx = app.ui_ctx.clone().expect("a window");
    app.session.set_tool_option("selected", &json!([2, 0, 2]));
    assert_eq!(tool_selection(&app, 3), [0, 2]);
    assert!(enabled_state(&app, "ui.artboards.rearrange"));
    run(&mut app, "ui.artboards.rearrange", &Value::Null).unwrap().unwrap();
    assert!(is_open::<RearrangeArtboardsDialog>(&app));
    assert_eq!(dialog_fields(&app)["indices"], json!([0, 2]));
    cancel_open(&mut app, &ctx);
    apply(&mut app, vec![PageListEvent::Rearrange]);
    assert!(is_open::<RearrangeArtboardsDialog>(&app));
    assert!(dialog_fields(&app).get("indices").is_none(), "the list's button lays out every artboard");
    cancel_open(&mut app, &ctx);
    app.select_tool("selection");
    select(&mut app, 1);
    assert_eq!((tool_selection(&app, 3), picked(&app, 3)), (vec![], vec![1]), "another tool");
    assert!(!enabled_state(&app, "ui.artboards.rearrange"));
    run(&mut app, "ui.artboards.rearrange", &Value::Null).unwrap().unwrap();
    assert!(!is_open::<RearrangeArtboardsDialog>(&app));
}

/// A lock entry lists only the artboards it changes and is left out when it has none, so it never
/// records an undo step that changes nothing.
#[test]
fn the_artboard_tools_lock_entries_have_something_to_change() {
    let mut app = menu_app(2);
    app.session.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    let items = crate::menus::context_items(&app);
    let subs = |menu: &str| match items.iter().find(|i| label_of(i) == menu && !children_of(i).is_empty()) {
        Some(sub) => menu_labels(children_of(sub)),
        None => vec![],
    };
    assert_eq!(subs("Lock Artboards"), ["Lock All Except Selected", "Lock All"], "the selected artboard is locked already");
    assert_eq!(subs("Unlock Artboards"), ["Unlock Selected", "Unlock All"]);
}

/// A right-click on an artboard that isn't selected selects it alone; on a selected one the selection
/// stays; off every artboard it stays too.
#[test]
fn a_right_click_with_the_artboard_tool_selects_the_artboard_under_the_pointer() {
    use vectorcraft_geom::Point;
    let mut app = menu_app(3);
    let doc = &app.session.active().unwrap().doc;
    let (a, c) = (doc.artboards[0].rect.center(), doc.artboards[2].rect.center());
    app.session.set_tool_option("selected", &json!([0, 1]));
    right_click(&mut app, a);
    assert_eq!(app.session.tool_options()["selected"], json!([0, 1]), "a selected artboard keeps the selection");
    right_click(&mut app, Point::new(-500.0, -500.0));
    assert_eq!(app.session.tool_options()["selected"], json!([0, 1]), "off every artboard");
    right_click(&mut app, c);
    assert_eq!(app.session.tool_options()["selected"], json!([2]));
    assert_eq!(app.view().unwrap().artboard, 2);
}

/// Paste from the Artboard tool's menu centers the copied artboards on the right-click's point. With
/// no point it puts them right of the last artboard.
#[test]
fn the_artboard_tools_paste_goes_to_the_right_click() {
    use vectorcraft_geom::Point;
    let mut app = menu_app(2);
    app.run("edit.copy", json!({})).unwrap();
    right_click(&mut app, Point::new(-300.0, 700.0));
    choose(&mut app, "Paste");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.artboards.len(), 3);
    let at = doc.artboards[2].rect.center();
    assert_eq!((at.x, at.y), (-300.0, 700.0));
    app.ui.context_artboards = None;
    choose(&mut app, "Paste");
    let doc = &app.session.active().unwrap().doc;
    let right = doc.artboards.iter().take(3).map(|a| a.rect.x1).fold(f64::MIN, f64::max);
    assert_eq!(doc.artboards[3].rect.x0, right + 20.0);
}

/// While the menu is open it reads what the right-click measured and does not walk the art again:
/// what it measured stands until the document or the selection changes.
#[test]
fn the_open_menu_reads_what_the_right_click_measured() {
    use vectorcraft_geom::Point;
    let mut app = menu_app(2);
    right_click(&mut app, Point::new(10.0, 10.0));
    let facts = app.ui.context_artboards.as_mut().unwrap();
    assert!(facts.any_empty && !facts.any_art);
    // Facts that differ from the document show which one the menu reads.
    facts.any_empty = false;
    facts.any_art = true;
    let items = crate::menus::context_items(&app);
    let labels = menu_labels(&items);
    assert!(!labels.contains(&"Delete Empty Artboards") && labels.contains(&"Fit to Artwork Bounds"), "{labels:?}");
    // An edit makes them out of date, and the menu measures again.
    app.session.execute("artboard.setProps", &json!({"index": 1, "name": "Other"})).unwrap();
    let items = crate::menus::context_items(&app);
    let labels = menu_labels(&items);
    assert!(labels.contains(&"Delete Empty Artboards") && !labels.contains(&"Fit to Artwork Bounds"), "{labels:?}");
}

/// Whether Fit to Artwork Bounds has art to fit is kept for a version of the document, not its
/// revision: a document replaced with no revision moved (composite fonts synced as its tab comes
/// to the front) is measured again.
#[test]
fn fit_to_art_is_measured_for_each_version_of_the_document() {
    let mut app = menu_app(2);
    assert!(!super::fit_has_art(&app, &[0, 1]));
    // Art on the first artboard, the revision left as it was.
    let st = app.session.active_mut().unwrap();
    let mut doc = (*st.doc).clone();
    let l = doc.layers[0].id;
    let id = doc.alloc_id();
    let art = vectorcraft_doc::Node::path(
        id,
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(10.0, 10.0, 20.0, 20.0)),
        vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0),
    );
    doc.insert(Some(l), 0, art).unwrap();
    st.doc = std::sync::Arc::new(doc);
    assert!(super::fit_has_art(&app, &[0, 1]), "measured again");
    assert!(!super::fit_has_art(&app, &[1]), "measured for the artboards asked about");
}

/// `artboard.selectAll` chooses the Artboard tool first when another tool is in use, and fails
/// without a document.
#[test]
fn select_all_artboards_chooses_the_tool_and_needs_a_document() {
    let mut app = menu_app(2);
    app.select_tool("selection");
    assert_eq!(app.run("artboard.selectAll", json!({})).unwrap()["selected"], json!([0, 1]));
    assert_eq!(app.session.tool_id(), "artboard");
    let mut empty = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    assert_eq!(empty.run("artboard.selectAll", json!({})).unwrap_err(), "no document");
}
