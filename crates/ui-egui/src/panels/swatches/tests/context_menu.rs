//! The row menu's items on swatch rows: which are enabled, and what the chosen one runs.

use super::*;

// PLAN: assumed ap.AP5 `row_items(&VectorcraftApp, &SwatchKey) -> Vec<(String, bool)>` (command id, enabled)
// and `PresetEvent::Menu { key, item }` carrying that id.
const ITEMS: [&str; 3] = ["ui.swatchOptions", "swatch.duplicate", "swatch.delete"];

fn enabled(app: &VectorcraftApp, name: &str) -> Vec<(String, bool)> {
    row_items(app, &key_of(app, name)).into_iter().filter(|(id, _)| ITEMS.contains(&id.as_str())).collect()
}

fn menu(app: &mut VectorcraftApp, name: &str, item: &str) {
    let key = key_of(app, name);
    feed(app, PresetEvent::Menu { key, item: item.to_string() });
}

#[test]
fn item_commands_run_on_the_clicked_swatch_and_paint_nothing() {
    let key = crate::dialogs::confirm::KIND;
    let mut app = app();
    select_rows(&mut app, &["Black"]);
    let (before, paint, undo) = (doc(&app), app.session.paint.fill.clone(), undo_steps(&app));
    assert!(enabled(&app, "Red").iter().all(|(_, on)| *on), "every item is enabled on a user swatch");
    menu(&mut app, "Red", "ui.swatchOptions");
    assert_eq!(dialog_field(&mut app, KIND, "__swatch"), json!("Red"));
    assert_eq!((doc(&app), undo_steps(&app)), (before.clone(), undo), "opening the dialog changes nothing");
    cancel(&mut app, KIND);
    menu(&mut app, "Red", "swatch.duplicate");
    assert_eq!(doc(&app).swatch("Red copy").map(|w| w.paint.clone()), doc(&app).swatch("Red").map(|w| w.paint.clone()));
    assert!(doc(&app).swatch("Black copy").is_none());
    assert_eq!(undo_steps(&app), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&app), before);
    // Delete asks first; Cancel keeps the swatch, OK deletes only the clicked one.
    menu(&mut app, "Red", "swatch.delete");
    assert_eq!(dialog_field(&mut app, key, "__command"), json!("swatch.delete"));
    assert_eq!(dialog_field(&mut app, key, "__params"), json!({"names": ["Red"]}));
    assert_eq!((doc(&app), undo_steps(&app)), (before.clone(), undo));
    cancel(&mut app, key);
    assert_eq!(doc(&app), before);
    menu(&mut app, "Red", "swatch.delete");
    confirm(&mut app, key);
    assert!(doc(&app).swatch("Red").is_none() && doc(&app).swatch("Black").is_some());
    assert_eq!(undo_steps(&app), undo + 1);
    assert_eq!(app.session.paint.fill, paint, "a menu item does not apply the swatch's paint");
}

#[test]
fn reserved_swatches_have_disabled_item_actions() {
    let app = app();
    for name in ["[None]", REGISTRATION] {
        let items = enabled(&app, name);
        assert_eq!(items.len(), ITEMS.len(), "{name}");
        assert!(items.iter().all(|(_, on)| !*on), "{name}: {items:?}");
    }
}

/// The items are rebuilt each pass, so a swatch deleted while its menu is open disables them.
#[test]
fn a_swatch_deleted_while_its_menu_is_open_disables_actions() {
    let mut app = app();
    let key = key_of(&app, "Red");
    assert!(row_items(&app, &key).iter().filter(|(id, _)| ITEMS.contains(&id.as_str())).all(|(_, on)| *on));
    app.run("swatch.delete", json!({"name": "Red"})).unwrap();
    let items = enabled(&app, "Red");
    assert_eq!(items.len(), ITEMS.len());
    assert!(items.iter().all(|(_, on)| !*on), "{items:?}");
}
