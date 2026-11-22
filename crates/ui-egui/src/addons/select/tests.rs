use craft::i18n::Folder;
use craft::ui::dialog::{DialogHost, DialogId, dialog_id};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::{ArtboardsDialog, COMMAND, KIND, choose_shown, merge_chosen, shown};
use crate::{VectorcraftApp, menus};

/// An app with the window the dialogs open in.
pub(super) fn new_app() -> VectorcraftApp {
    crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
}

/// The window context of `app`.
pub(super) fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the test context")
}

/// Runs `f` on the host the dialogs live on.
pub(super) fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("a window").with(f)
}

/// Sets `field` of the open dialog `id` as its control would.
pub(super) fn set(app: &VectorcraftApp, id: DialogId, field: &str, value: Value) -> Result<(), String> {
    let ctx = ctx(app);
    host(app, |h| h.set_json_field(&ctx, id, field, value))
}

/// OK on the open dialog `id`, then the frame's pump that runs its command. `false` when OK is refused.
pub(super) fn confirm(app: &mut VectorcraftApp, id: DialogId) -> bool {
    let ctx = ctx(app);
    let accepted = host(app, |h| h.confirm_by_id(&ctx, id));
    crate::dialogs::host::pump(app, &ctx);
    accepted
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A 100 × 100 document with a Banner artboard at x 150, an object in each artboard, and nothing selected.
fn banner() -> (VectorcraftApp, u64) {
    let mut app = new_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 150, "y": 0, "width": 200, "height": 50, "name": "Banner"})).unwrap();
    let inside = app.run("shape.rectangle", json!({"x": 160, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, inside)
}

#[test]
fn the_search_filters_artboards_by_name_ignoring_case_and_accents() {
    let folder = Folder::new("en");
    let n = names(&["Cover", "Page 2", "Back cover", "Inside", "Café"]);
    assert_eq!(shown(&n, "", &folder), vec![0, 1, 2, 3, 4]);
    assert_eq!(shown(&n, "  COVER ", &folder), vec![0, 2]);
    assert_eq!(shown(&n, "zzz", &folder), Vec::<usize>::new());
    assert_eq!(shown(&n, "cafe", &folder), vec![4], "an accent folds");
}

#[test]
fn select_all_and_none_act_on_the_rows_shown() {
    // Board 4 is chosen but hidden by the search: neither button touches it.
    let mut c = vec![1, 4];
    choose_shown(&mut c, &[0, 1, 2], true);
    assert_eq!(c, vec![0, 1, 2, 4]);
    choose_shown(&mut c, &[0, 1, 2], false);
    assert_eq!(c, vec![4]);
}

#[test]
fn select_by_artboards_opens_on_the_active_artboard_and_selects_on_ok() {
    let (mut app, inside) = banner();
    menus::invoke(&mut app, COMMAND, Value::Null);
    let id = dialog_id(KIND);
    assert_eq!(host(&app, |h| h.get::<ArtboardsDialog>().map(|d| d.fields.artboards.clone())), Some(vec![0]), "the active artboard is chosen");
    // With nothing chosen OK is refused, and the dialog stays.
    set(&app, id, "artboards", json!([])).unwrap();
    assert!(!confirm(&mut app, id), "OK is off with nothing chosen");
    assert!(host(&app, |h| h.is_open::<ArtboardsDialog>()));
    set(&app, id, "artboards", json!([1])).unwrap();
    set(&app, id, "relation", json!("inside")).unwrap();
    assert!(confirm(&mut app, id));
    assert!(host(&app, |h| !h.is_open::<ArtboardsDialog>()));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(inside)]);
}

#[test]
fn a_search_keeps_the_choice_it_hides_and_a_double_click_fits_its_artboard() {
    // Boards 0 and 1 chosen, the search shows 0 and 2: picking row 2 leaves hidden board 1 chosen and drops shown board 0.
    assert_eq!(merge_chosen(&[0, 1], &[0, 2], &[2]), vec![1, 2]);
    // Nothing picked among the rows shown: the hidden choice stays, so a search never loses it.
    assert_eq!(merge_chosen(&[1], &[0, 2], &[]), vec![1]);

    let mut app = new_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    for x in [150, 300] {
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    app.run("view.goToArtboard", json!({"index": 0})).unwrap();
    menus::invoke(&mut app, COMMAND, Value::Null);
    let id = dialog_id(KIND);
    set(&app, id, "artboards", json!([0, 1])).unwrap();
    // The activation the list reports for a double click on row 2, as the body queues it.
    host(&app, |h| h.get_mut::<ArtboardsDialog>().map(|d| d.go_to = Some(2))).unwrap();
    let window = ctx(&app);
    crate::dialogs::host::pump(&mut app, &window);
    assert_eq!(app.view().unwrap().artboard, 2, "the artboard is active");
    assert!(host(&app, |h| h.is_open::<ArtboardsDialog>()), "the dialog stays open");
    assert_eq!(
        host(&app, |h| h.get::<ArtboardsDialog>().map(|d| (d.fields.artboards.clone(), d.go_to))),
        Some((vec![0, 1], None)),
        "the choice is kept, the request done"
    );
}
