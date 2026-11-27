use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, TextKind};
use vectorcraft_engine::Session;

use crate::addons::dialogs::tests::{cancel, confirm, ctx, dialog_id};
use crate::{VectorcraftApp, menus};

/// One item of an add-on submenu: how its dialog opens, what is set in it and what OK leaves behind.
struct Row {
    name: &'static str,
    menu: &'static str,
    label: &'static str,
    command: &'static str,
    /// Makes the document the item works on.
    setup: fn(&mut VectorcraftApp),
    /// The values the dialog opens on (a subset of its fields).
    opens: fn() -> Value,
    /// The values set before OK.
    set: fn() -> Value,
    /// What the command left behind, given the document as it was before; OK was one undo step.
    check: fn(&mut VectorcraftApp, &Document),
}

fn new_app(width: u32, height: u32) -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    app
}

fn id_of(result: Value) -> u64 {
    result["id"].as_u64().unwrap()
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64, width: f64, height: f64) -> u64 {
    id_of(app.run("shape.rectangle", json!({"x": x, "y": y, "width": width, "height": height})).unwrap())
}

fn circle(app: &mut VectorcraftApp, cx: f64, cy: f64, r: f64) -> u64 {
    id_of(app.run("shape.ellipse", json!({"x": cx - r, "y": cy - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap())
}

fn select(app: &mut VectorcraftApp, ids: &[u64]) {
    app.run("select.set", json!({ "ids": ids })).unwrap();
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn nodes(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.node_count()
}

/// The ids of the first layer's children in a document.
fn children(doc: &Document) -> Vec<u64> {
    doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect()
}

/// Chooses Object › Plug-ins › `menu` › `label`, which must be enabled.
fn open_item(app: &mut VectorcraftApp, menu: &str, label: &str) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", menu] && e.label == label).unwrap_or_else(|| panic!("{label} is listed"));
    assert!(item.enabled, "{label} enabled");
    let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &cmd, p);
}

/// The open dialog's command key and its values as automation lists them.
fn open_dialog(app: &VectorcraftApp) -> (Option<String>, Value) {
    app.dialogs()
        .expect("window host")
        .with(|host| host.open_dialogs().first().map(|d| (d.key.clone(), d.fields.clone().unwrap_or(Value::Null))))
        .expect("a dialog opens")
}

/// Sets a value of the open dialog as automation does.
fn set_field(app: &VectorcraftApp, key: &str, value: Value) -> Result<(), String> {
    let (ctx, id) = (ctx(app), dialog_id(app));
    app.dialogs().expect("window host").with(|host| host.set_json_field(&ctx, id, key, value))
}

/// Answers the open dialog with OK or Cancel, then runs the frame's pump.
fn answer(app: &mut VectorcraftApp, ok: bool) {
    let id = dialog_id(app);
    if ok {
        confirm(app, id);
    } else {
        cancel(app, id);
    }
    let ctx = ctx(app);
    crate::dialogs::host::pump(app, &ctx);
    // A held OK closes at the dialog's next pass.
    crate::tests_synthetic::key_frame(app, &ctx, vec![]);
}

fn closed(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.open_dialogs().is_empty()))
}

/// Whether every value `want` names is held by `fields` (numbers compare as numbers).
fn holds(fields: &Value, want: &Value) -> bool {
    want.as_object().is_some_and(|want| {
        want.iter().all(|(key, value)| {
            fields.get(key).is_some_and(|held| match (held.as_f64(), value.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => held == value,
            })
        })
    })
}

/// Runs Weed Box and Lines on object `id` with `sets` set in its dialog.
fn weed_with(app: &mut VectorcraftApp, id: u64, sets: &[(&str, Value)]) {
    select(app, &[id]);
    open_item(app, "Cutting Tools", "Weed Box and Lines…");
    for (key, value) in sets {
        set_field(app, key, value.clone()).unwrap_or_else(|e| panic!("{key}: {e}"));
    }
    answer(app, true);
}

/// The left edge of node `id`.
fn left(app: &VectorcraftApp, id: u64) -> f64 {
    app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap().x0
}

/// Every add-on dialog of the cutting, path precision, type and colour submenus opens on its item's values and
/// runs its command on OK as one undo step.
#[test]
fn each_addon_dialog_opens_on_its_items_values_and_runs_its_command() {
    let rows = [
        Row {
            name: "stencil bridges",
            menu: "Cutting Tools",
            label: "Stencil Bridges…",
            command: super::BRIDGES,
            setup: |app| {
                let outer = rect(app, 100.0, 100.0, 100.0, 100.0);
                let inner = rect(app, 120.0, 120.0, 60.0, 60.0);
                select(app, &[outer, inner]);
                let o = app.run("object.pathfinder.minusFront", json!({})).unwrap()["ids"][0].clone();
                app.run("select.set", json!({"ids": [o]})).unwrap();
                // Each way the list offers is one the command takes; no bridge fits that wide, so nothing changes.
                let n = steps(app);
                for (_, way) in super::BRIDGE_DIRECTIONS {
                    assert!(app.session.execute(super::BRIDGES, &json!({"direction": way, "width": 1000})).is_ok(), "{way}");
                }
                assert_eq!(steps(app), n, "nothing changed");
            },
            opens: || json!({"width": 2, "count": 1, "direction": "vertical", "keepOriginal": true}),
            set: || json!({}),
            check: |app, _| {
                let layers = &app.session.doc().unwrap().doc.layers;
                assert!(layers.iter().any(|l| l.name.as_deref() == Some("Stencil Original") && !l.visible), "the original is on a hidden layer");
                // Run again on the result, which has no islands left: a message, and nothing changes.
                let n = steps(app);
                open_item(app, "Cutting Tools", "Stencil Bridges…");
                answer(app, true);
                assert_eq!(steps(app), n);
                assert!(app.ui.status.contains("no islands"), "{}", app.ui.status);
            },
        },
        Row {
            name: "weed box and lines",
            menu: "Cutting Tools",
            label: "Weed Box and Lines…",
            command: super::WEED,
            setup: |app| {
                let id = rect(app, 100.0, 100.0, 50.0, 40.0);
                select(app, &[id]);
            },
            opens: || json!({"direction": "both", "box": true}),
            set: || json!({}),
            check: |app, start| {
                assert_eq!(nodes(app), start.node_count() + 6, "a group, the box and four lines");
                let id = children(start)[0];
                // A Spacing above 0 is sent instead of the Count: the 68 pt wide box in strips at most 20 pt wide
                // takes three vertical lines, each cut in two by the rectangle.
                app.run("edit.undo", json!({})).unwrap();
                weed_with(app, id, &[("direction", json!("vertical")), ("spacing", json!(20))]);
                assert_eq!(nodes(app), start.node_count() + 8, "a group, the box and six pieces of line");
                // A Spacing below 0 is refused, not swapped for the Count: the dialog stays open.
                app.run("edit.undo", json!({})).unwrap();
                let n = steps(app);
                weed_with(app, id, &[("spacing", json!(-5))]);
                assert_eq!((steps(app), closed(app)), (n, false), "refused");
                answer(app, false);
                // A Spacing of 0 leaves the Count in use: a Count of 0 draws the box alone. The row holds a number,
                // so it has no empty value to try.
                weed_with(app, id, &[("spacing", json!(0)), ("count", json!(0))]);
                assert_eq!(nodes(app), start.node_count() + 2, "a group and the box");
                app.run("edit.undo", json!({})).unwrap();
            },
        },
        Row {
            name: "banner hems and grommets",
            menu: "Cutting Tools",
            label: "Banner Hems and Grommets…",
            command: super::BANNER,
            setup: |app| {
                *app = new_app(600, 400);
                app.run("select.none", json!({})).unwrap();
            },
            opens: || json!({"grommets": true, "top": true}),
            set: || json!({"spacing": 100}),
            check: |app, start| assert!(nodes(app) > start.node_count(), "the document gains hems and grommets"),
        },
        Row {
            name: "nest shapes",
            menu: "Cutting Tools",
            label: "Nest Shapes…",
            command: super::NEST,
            setup: |app| {
                let a = rect(app, 700.0, 100.0, 50.0, 40.0);
                let b = rect(app, 800.0, 100.0, 60.0, 30.0);
                select(app, &[a, b]);
            },
            opens: || json!({"roll": false, "rotate": false, "usePreviewBounds": true}),
            set: || json!({"margin": 0}),
            check: |app, start| {
                let ids = children(start);
                let (a, b) = (left(app, ids[0]), left(app, ids[1]));
                assert!(a < 100.0, "the first object moved onto the artboard: {a}");
                assert!(b > a, "the second object lies right of the first: {a}, {b}");
            },
        },
        Row {
            name: "common tangents",
            menu: "Path Precision Tools",
            label: "Common Tangents…",
            command: crate::addons::pathtools::COMMON_TANGENTS,
            setup: |app| {
                let (a, b) = (circle(app, 60.0, 150.0, 20.0), circle(app, 220.0, 150.0, 40.0));
                select(app, &[a, b]);
            },
            opens: || json!({"tangents": "all", "centerline": false}),
            set: || json!({}),
            check: |app, start| {
                assert_eq!(nodes(app), start.node_count() + 4, "four tangent lines");
                // Each kind the list offers is one the command takes.
                for (_, kind) in crate::addons::pathtools::TANGENT_KINDS {
                    let params = json!({"tangents": kind});
                    assert!(vectorcraft_engine::check_params(crate::addons::pathtools::COMMON_TANGENTS, &params).is_ok(), "{kind}");
                }
                // A circle inside another: nothing changes, and a message says why.
                let ids = children(start);
                let inner = circle(app, 220.0, 150.0, 10.0);
                select(app, &[ids[1], inner]);
                let (n, count) = (steps(app), nodes(app));
                open_item(app, "Path Precision Tools", "Common Tangents…");
                answer(app, true);
                assert_eq!((steps(app), nodes(app)), (n, count));
                assert!(app.ui.status.contains("inside the other"), "{}", app.ui.status);
            },
        },
        Row {
            name: "add bounding rectangle",
            menu: "Path Precision Tools",
            label: "Add Bounding Rectangle…",
            command: crate::addons::pathtools::BOUNDING_RECTANGLE,
            setup: |app| {
                let a = rect(app, 10.0, 20.0, 30.0, 40.0);
                let b = rect(app, 100.0, 20.0, 20.0, 40.0);
                select(app, &[a, b]);
            },
            opens: || json!({"offset": 0, "each": false}),
            set: || json!({}),
            check: |app, start| {
                // One shape around the whole selection, from (10, 20) to (120, 60).
                let st = app.session.doc().unwrap();
                let r = st.doc.bounds_of(&st.selection.objects, false).unwrap();
                let near = [r.x0, r.y0, r.x1, r.y1].iter().zip([10.0, 20.0, 120.0, 60.0]).all(|(v, w)| (v - w).abs() < 1e-6);
                assert!(near, "{r:?}");
                // An Offset of 5 pt and One per Object: a larger shape around each object, one undo step.
                let ids = children(start);
                select(app, &ids);
                let n = steps(app);
                open_item(app, "Path Precision Tools", "Add Bounding Rectangle…");
                set_field(app, "offset", json!(5)).unwrap();
                set_field(app, "each", json!(true)).unwrap();
                answer(app, true);
                assert_eq!(steps(app), n + 1);
                let st = app.session.doc().unwrap();
                assert_eq!(st.selection.objects.len(), 2, "a shape around each object");
                let made: Vec<_> = st.selection.objects.iter().map(|id| st.doc.bounds_of(&[*id], false).unwrap()).collect();
                for id in ids {
                    let o = st.doc.bounds_of(&[NodeId(id)], false).unwrap();
                    assert!(made.iter().any(|m| m.x0 < o.x0 && m.y0 < o.y0 && m.x1 > o.x1 && m.y1 > o.y1), "a larger shape around {id}");
                }
            },
        },
        Row {
            name: "release type",
            menu: "Type Tools",
            label: "Release Type on a Path…",
            command: crate::addons::typetools::RELEASE,
            setup: |app| {
                *app = new_app(600, 400);
                let path = id_of(app.run("path.create", json!({"d": "M100 200 C200 100 400 100 500 200"})).unwrap());
                let text = id_of(app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Release me"})).unwrap());
                select(app, &[text]);
            },
            opens: || json!({"mode": "both", "keepOriginal": false}),
            set: || json!({}),
            check: |app, start| {
                let on_path = |doc: &Document| {
                    doc.layers[0]
                        .children()
                        .unwrap()
                        .iter()
                        .filter(|n| matches!(&n.kind, NodeKind::Text(t) if matches!(t.kind, TextKind::OnPath { .. })))
                        .count()
                };
                assert_eq!(on_path(start), 1, "the fixture has type on a path");
                assert_eq!(on_path(&app.session.doc().unwrap().doc), 0, "the type is released");
            },
        },
        Row {
            name: "print ink fixes",
            menu: "Color Effects",
            label: "Print Ink Fixes…",
            command: "addon.color.printFixes",
            setup: |app| {
                *app = new_app(600, 400);
                let id = rect(app, 10.0, 10.0, 50.0, 50.0);
                select(app, &[id]);
                app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
                app.run("paint.setFill", json!({"color": {"c": 100, "m": 100, "y": 100, "k": 100}})).unwrap();
            },
            opens: || json!({}),
            set: || json!({}),
            check: |app, _| {
                let st = app.session.active().unwrap();
                let fill = st.doc.node(st.selection.objects[0]).unwrap().appearance.fill_paint().color().unwrap();
                assert_eq!(fill, vectorcraft_color::Color::cmyk(0.75, 0.75, 0.75, 0.75), "scaled to 300 %");
            },
        },
    ];
    for row in rows {
        let mut app = new_app(600, 400);
        (row.setup)(&mut app);
        let (start, n) = (app.session.doc().unwrap().doc.clone(), steps(&app));
        open_item(&mut app, row.menu, row.label);
        let (key, fields) = open_dialog(&app);
        assert_eq!(key.as_deref(), Some(row.command), "{}", row.name);
        assert!(holds(&fields, &(row.opens)()), "{}: opens on {fields}", row.name);
        if let Some(values) = (row.set)().as_object() {
            for (key, value) in values {
                set_field(&app, key, value.clone()).unwrap_or_else(|e| panic!("{}: {key}: {e}", row.name));
            }
        }
        answer(&mut app, true);
        assert!(closed(&app), "{}: OK closes the dialog", row.name);
        assert_eq!(steps(&app), n + 1, "{}: one undo step", row.name);
        (row.check)(&mut app, &start);
    }
}
