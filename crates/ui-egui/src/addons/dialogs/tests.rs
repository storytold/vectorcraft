use craft::ui::dialog::{DialogId, HeldOk};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;

use crate::tests_synthetic::key_frame;
use crate::{VectorcraftApp, menus};

/// Clicks the Object › Plug-ins › `group` item `label`.
pub(crate) fn open_item(app: &mut VectorcraftApp, group: &str, label: &str) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", group] && e.label == label).expect(label);
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

/// The window context of `app`, which its dialogs live on.
pub(crate) fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the test window's context")
}

/// The id of the one dialog open.
pub(crate) fn dialog_id(app: &VectorcraftApp) -> DialogId {
    app.dialogs().unwrap().with(|h| h.open_dialogs().first().map(|d| d.id)).unwrap()
}

/// Reads the values of the hosted dialog `id`.
pub(crate) fn read<R>(app: &VectorcraftApp, id: DialogId, f: impl FnOnce(&DialogFields) -> R) -> R {
    app.dialogs().unwrap().with(|h| f(h.get_by_id::<HostedParamDialog>(id).unwrap().fields()))
}

/// Sets a value of dialog `id` as automation does.
pub(crate) fn set(app: &VectorcraftApp, id: DialogId, key: &str, value: Value) {
    let ctx = ctx(app);
    app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, key, value)).unwrap();
}

/// Presses OK of dialog `id`; whether it was enabled. A held OK reports `false` and files its answer for the app.
pub(crate) fn confirm(app: &VectorcraftApp, id: DialogId) -> bool {
    let ctx = ctx(app);
    app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id) || h.held_state(id) == Some(HeldOk::Waiting))
}

/// Presses Cancel of dialog `id`.
pub(crate) fn cancel(app: &VectorcraftApp, id: DialogId) {
    let ctx = ctx(app);
    app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id));
}

/// One frame, then the pump that answers what the dialogs filed.
pub(crate) fn frame(app: &mut VectorcraftApp) {
    let ctx = ctx(app);
    key_frame(app, &ctx, vec![]);
    crate::dialogs::host::pump(app, &ctx);
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn count(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers[0].children().map_or(0, Vec::len)
}

fn x0(app: &VectorcraftApp, i: usize) -> f64 {
    app.session.doc().unwrap().doc.layers[0].children().unwrap()[i].geometric_bounds().unwrap().x0
}

fn new_app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app
}

/// Two rectangles without a stroke, both selected.
fn two_rects() -> VectorcraftApp {
    let mut app = new_app();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 40, "height": 40})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app
}

fn two_circles() -> VectorcraftApp {
    let mut app = new_app();
    let ids: Vec<u64> = [100.0, 160.0]
        .iter()
        .map(|x| app.run("shape.ellipse", json!({"x": x - 20.0, "y": 80, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
        .collect();
    app.run("select.set", json!({ "ids": ids })).unwrap();
    app
}

fn a_path() -> VectorcraftApp {
    let mut app = new_app();
    let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}]);
    let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app
}

fn a_text() -> VectorcraftApp {
    let mut app = new_app();
    let id = app.run("text.create", json!({"x": 40, "y": 100, "text": "abc", "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app
}

fn a_swatch() -> VectorcraftApp {
    let mut app = new_app();
    app.run("swatch.new", json!({"name": "Ink", "color": "#123456"})).unwrap();
    app
}

/// One dialog of the family: where its menu item is, the document it opens on, the values set before OK, and what
/// OK must have done.
struct Row {
    group: &'static str,
    label: &'static str,
    setup: fn() -> VectorcraftApp,
    /// Values that turn OK off, set first.
    refused: fn() -> Vec<(&'static str, Value)>,
    values: fn() -> Vec<(&'static str, Value)>,
    check: fn(&VectorcraftApp),
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            group: "Arrange Tools",
            label: "Arrange in Grid…",
            setup: two_rects,
            refused: Vec::new,
            values: || vec![("layout", json!("row"))],
            check: |app| assert_eq!((x0(app, 0), x0(app, 1)), (0.0, 30.0)),
        },
        Row {
            group: "Arrange Tools",
            label: "Randomize Opacity…",
            setup: two_rects,
            refused: || vec![("min", json!(80)), ("max", json!(20))],
            values: || vec![("min", json!(40)), ("max", json!(60))],
            check: |app| {
                for i in 0..2 {
                    let opacity = app.session.doc().unwrap().doc.layers[0].children().unwrap()[i].opacity;
                    assert!((0.4..=0.6).contains(&opacity), "{opacity}");
                }
            },
        },
        Row {
            group: "Arrange Tools",
            label: "Sort Objects…",
            setup: two_rects,
            refused: Vec::new,
            // Large to small along x: the larger b takes the left edge 0, a the left edge 100.
            values: || vec![("apply", json!("x")), ("order", json!("descending"))],
            check: |app| assert_eq!((x0(app, 1), x0(app, 0)), (0.0, 100.0)),
        },
        Row {
            group: "Path Precision Tools",
            label: "Smooth Bridge…",
            setup: two_circles,
            refused: Vec::new,
            values: || vec![("keepOriginals", json!(true))],
            check: |app| assert_eq!(count(app), 3, "the circles and the outline above them"),
        },
        Row {
            group: "Path Precision Tools",
            label: "Divide into Dashes…",
            setup: a_path,
            refused: Vec::new,
            values: || vec![("gap", json!(10))],
            // 10 + 10 + … over 100: five dashes, the gaps left empty.
            check: |app| assert_eq!(count(app), 5),
        },
        Row {
            group: "Type Tools",
            label: "Text Tweaker…",
            setup: a_text,
            refused: Vec::new,
            values: Vec::new,
            check: |app| {
                let sizes: Vec<f64> = match &app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].kind {
                    NodeKind::Text(t) => t.runs.iter().flat_map(|r| r.text.chars().map(|_| r.style.size)).collect(),
                    _ => vec![],
                };
                assert_eq!(sizes, [12.0, 24.0, 36.0]);
            },
        },
        Row {
            group: "Color Effects",
            label: "Create Swatch Legend…",
            setup: a_swatch,
            refused: Vec::new,
            values: || vec![("hex", json!(true)), ("cmyk", json!(true)), ("columns", json!(2))],
            check: |app| {
                let doc = &app.session.doc().unwrap().doc;
                let group = doc.layers[0].children().unwrap().last().unwrap();
                let label = group.children().unwrap().iter().find_map(|c| match &c.kind {
                    NodeKind::Text(t) if t.plain_text().starts_with("Ink") => Some(t.plain_text()),
                    _ => None,
                });
                let label = label.expect("Ink's label");
                assert!(label.contains("#123456") && label.contains("C="), "{label}");
            },
        },
    ]
}

#[test]
fn add_on_dialogs_preview_cancel_and_run_their_command() {
    // Preview and Cancel, on the first dialog.
    let mut app = two_rects();
    open_item(&mut app, "Arrange Tools", "Arrange in Grid…");
    let id = dialog_id(&app);
    set(&app, id, "layout", json!("row"));
    set(&app, id, "preview", json!(true));
    frame(&mut app);
    assert_eq!(x0(&app, 1), 30.0, "previewed");
    cancel(&app, id);
    frame(&mut app);
    assert_eq!(x0(&app, 1), 100.0, "rolled back");
    assert!(app.dialogs().unwrap().with(|h| h.open_dialogs().is_empty()));

    for row in rows() {
        let mut app = (row.setup)();
        open_item(&mut app, row.group, row.label);
        let id = dialog_id(&app);
        let undo = undo_len(&app);
        let refused = (row.refused)();
        if !refused.is_empty() {
            for (key, value) in refused {
                set(&app, id, key, value);
            }
            assert!(!confirm(&app, id), "{}: OK is off", row.label);
        }
        for (key, value) in (row.values)() {
            set(&app, id, key, value);
        }
        assert!(confirm(&app, id), "{}: OK is on", row.label);
        frame(&mut app);
        assert_eq!(undo_len(&app), undo + 1, "{}: one undo step", row.label);
        assert!(app.dialogs().unwrap().with(|h| h.open_dialogs().is_empty()), "{}: closed", row.label);
        (row.check)(&app);
    }
}

#[test]
fn opening_a_second_add_on_dialog_closes_the_first_and_rolls_back_its_preview() {
    let mut app = two_rects();
    open_item(&mut app, "Arrange Tools", "Arrange in Grid…");
    let id = dialog_id(&app);
    set(&app, id, "layout", json!("row"));
    set(&app, id, "preview", json!(true));
    frame(&mut app);
    assert_eq!(x0(&app, 1), 30.0, "previewed");
    open_item(&mut app, "Arrange Tools", "Randomize Opacity…");
    frame(&mut app);
    assert_eq!(app.dialogs().unwrap().with(|h| h.open_dialogs().len()), 1);
    assert_eq!(x0(&app, 1), 100.0, "the first preview rolled back");
}
