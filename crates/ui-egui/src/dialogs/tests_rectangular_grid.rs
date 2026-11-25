//! Rectangular Grid click-to-size flow, from canvas dispatch through the hosted dialog.
//! All geometry here is original, synthetic test data. The host helpers below are shared by the
//! other Tool Options dialogs' tests.

use std::sync::Arc;

use craft::ui::dialog::{HeldOk, OpenDialog, dialog_id};
use craft::ui::shell::WindowDialogs;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{PathData, Point, Rect};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::VectorcraftApp;

const KIND: &str = "rectangularGrid";

/// The window's dialog host and its context; a test app gets its window before it opens a dialog.
fn host(app: &VectorcraftApp) -> (WindowDialogs, egui::Context) {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    (super::host::window_dialogs(&ctx), ctx)
}

/// The open dialog of `kind`, as the host lists it.
fn open_dialog(app: &VectorcraftApp, kind: &str) -> Option<OpenDialog> {
    let id = dialog_id(kind);
    app.dialogs()?.with(|h| h.open_dialogs().into_iter().find(|d| d.id == id))
}

/// Whether the dialog of `kind` is open.
pub(super) fn is_open(app: &VectorcraftApp, kind: &str) -> bool {
    open_dialog(app, kind).is_some()
}

/// The value of `key` in the open dialog of `kind` (null when it has none).
pub(super) fn field_of(app: &VectorcraftApp, kind: &str, key: &str) -> Value {
    open_dialog(app, kind).and_then(|d| d.fields?.get(key).cloned()).unwrap_or_default()
}

/// Set `key` of the open dialog of `kind`, as automation does; a row refuses a value it cannot hold.
pub(super) fn set_of(app: &mut VectorcraftApp, kind: &str, key: &str, value: Value) -> Result<(), String> {
    let (host, ctx) = host(app);
    host.with(|h| h.set_json_field(&ctx, dialog_id(kind), key, value))
}

/// Press OK on the dialog of `kind` and let the app answer it; the reason when it refuses.
pub(super) fn try_confirm(app: &mut VectorcraftApp, kind: &str) -> Result<(), String> {
    let id = dialog_id(kind);
    let (host, ctx) = host(app);
    host.with(|h| h.confirm_by_id(&ctx, id));
    super::host::pump(app, &ctx);
    // The app's answer to a held OK reaches the dialog in its next pass, which closes it or shows the refusal.
    super::tests::frame(app, vec![]);
    let refused = match host.with(|h| h.held_state(id)) {
        Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
        _ => Ok(()),
    };
    super::tests::frame(app, vec![]);
    refused
}

/// Press Cancel on the dialog of `kind`.
fn cancel_of(app: &mut VectorcraftApp, kind: &str) {
    let (host, ctx) = host(app);
    host.with(|h| h.cancel_by_id(&ctx, dialog_id(kind)));
    super::host::pump(app, &ctx);
}

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.select_tool(KIND);
    // Pointer-up persists tool settings even for a click. Seed them so opening/cancelling can
    // be compared with the complete pre-click preferences, including an unchanged store.
    saved_counts(&mut app, 5, 5);
    app
}

fn saved_counts(app: &mut VectorcraftApp, rows: u32, columns: u32) {
    let values = json!({"rows": rows, "columns": columns});
    app.session.set_tool_options(Some(KIND), values.as_object().unwrap());
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, snap_to_point: false, snap_to_grid: false, snap_to_pixel: false, ..Default::default() }
}

fn pointer(app: &mut VectorcraftApp, kind: PointerKind, p: Point) {
    crate::canvas::dispatch(app, &PointerEvent::new(kind, p.x, p.y), view());
}

fn click(app: &mut VectorcraftApp, at: Point) {
    app.select_tool(KIND);
    pointer(app, PointerKind::Down, at);
    assert!(!is_open(app, KIND), "the dialog opens on release");
    pointer(app, PointerKind::Up, at);
    assert!(is_open(app, KIND));
    assert!(!app.session.tool_busy());
    assert!(!app.session.in_interaction());
}

fn field(app: &mut VectorcraftApp, key: &str) -> Value {
    field_of(app, KIND, key)
}

fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
    set_of(app, KIND, key, value).unwrap();
}

fn values(app: &mut VectorcraftApp, width: Value, height: Value, rows: Value, columns: Value) {
    for (key, value) in [("width", width), ("height", height), ("rows", rows), ("columns", columns)] {
        set(app, key, value);
    }
}

/// OK on the dialog of `kind`: the group it created, which is the selection.
fn confirm_of(app: &mut VectorcraftApp, kind: &str) -> NodeId {
    try_confirm(app, kind).unwrap();
    assert!(!is_open(app, kind));
    *app.session.doc().unwrap().selection.objects.first().expect("confirmation selects the created group")
}

fn confirm(app: &mut VectorcraftApp) -> NodeId {
    confirm_of(app, KIND)
}

fn paths(app: &VectorcraftApp, id: NodeId) -> Vec<PathData> {
    let doc = &app.session.doc().unwrap().doc;
    let group = doc.node(id).unwrap();
    assert!(matches!(group.kind, NodeKind::Group { .. }));
    group.children().unwrap().iter().map(|n| n.path_data().expect("every grid child is an editable path").clone()).collect()
}

fn assert_path(path: &PathData, expected: &[Point], closed: bool) {
    assert_eq!(path.subpaths.len(), 1);
    let subpath = &path.subpaths[0];
    assert_eq!(subpath.closed, closed);
    assert_eq!(subpath.anchors.len(), expected.len());
    for (anchor, expected) in subpath.anchors.iter().zip(expected) {
        assert!(anchor.p.distance(*expected) < 1e-7, "{:?} != {expected:?}", anchor.p);
        assert!(anchor.h_in.distance(anchor.p) < 1e-7 && anchor.h_out.distance(anchor.p) < 1e-7, "grid edges stay straight");
    }
}

/// Independent coordinate oracle: one closed outer frame at the bottom, then the interior
/// dividers the counts mean.
fn assert_grid(app: &VectorcraftApp, id: NodeId, bounds: Rect, rows: u32, columns: u32) {
    let paths = paths(app, id);
    assert_eq!(paths.len(), (rows + columns + 1) as usize);
    let mut remaining = paths.iter();
    assert_path(
        remaining.next().unwrap(),
        &[Point::new(bounds.x0, bounds.y0), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)],
        true,
    );
    for i in 1..=rows {
        let y = bounds.y0 + bounds.height() * f64::from(i) / f64::from(rows + 1);
        assert_path(remaining.next().unwrap(), &[Point::new(bounds.x0, y), Point::new(bounds.x1, y)], false);
    }
    for i in 1..=columns {
        let x = bounds.x0 + bounds.width() * f64::from(i) / f64::from(columns + 1);
        assert_path(remaining.next().unwrap(), &[Point::new(x, bounds.y0), Point::new(x, bounds.y1)], false);
    }
    assert!(remaining.next().is_none());
}

struct Snapshot {
    doc: Arc<Document>,
    selected: Vec<NodeId>,
    prefs: Value,
    options: Value,
    undo: usize,
    redo: usize,
    journal: vectorcraft_engine::Journal,
    dirty: bool,
}

impl Snapshot {
    fn take(app: &VectorcraftApp) -> Self {
        let state = app.session.doc().unwrap();
        Self {
            doc: state.doc.clone(),
            selected: state.selection.objects.clone(),
            prefs: app.session.prefs.to_json(),
            options: app.session.tool_options_of(KIND),
            undo: state.history.undo.len(),
            redo: state.history.redo.len(),
            journal: app.session.journal.clone(),
            dirty: state.is_dirty(),
        }
    }

    fn assert_unchanged(&self, app: &VectorcraftApp) {
        let state = app.session.doc().unwrap();
        assert_eq!(state.doc, self.doc, "the document, including allocated ids, stays unchanged");
        assert_eq!(state.selection.objects, self.selected);
        assert_eq!(app.session.prefs.to_json(), self.prefs);
        assert_eq!(app.session.tool_options_of(KIND), self.options);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), (self.undo, self.redo));
        assert_eq!(app.session.journal, self.journal);
        assert_eq!(state.is_dirty(), self.dirty);
        assert!(!app.session.in_interaction());
    }
}

/// The box's reference point sits on the click: the center for 4, so a wider box grows both ways.
#[test]
fn rectangular_grid_reference_locator_reanchors_to_the_click() {
    for (width, height, expected) in [(100, 100, Rect::new(50.0, 50.0, 150.0, 150.0)), (200, 100, Rect::new(0.0, 50.0, 200.0, 150.0))] {
        let mut app = app();
        click(&mut app, Point::new(100.0, 100.0));
        assert_eq!(field(&mut app, "reference"), json!(0));
        set(&mut app, "reference", json!(4));
        values(&mut app, json!(width), json!(height), json!(5), json!(5));
        let id = confirm(&mut app);
        assert_grid(&app, id, expected, 5, 5);
    }
}

#[test]
fn rectangular_grid_canvas_click_opens_the_dialog_with_saved_counts_and_point_defaults() {
    let mut app = app();
    saved_counts(&mut app, 7, 3);
    app.run("document.setUnits", json!({"units": "mm"})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(137.25, 86.5));
    before.assert_unchanged(&app);
    assert_eq!((field(&mut app, "x"), field(&mut app, "y")), (json!(137.25), json!(86.5)));
    assert_eq!((field(&mut app, "width"), field(&mut app, "height")), (json!(100.0), json!(100.0)));
    assert_eq!((field(&mut app, "rows"), field(&mut app, "columns")), (json!(7), json!(3)));
    let id = confirm(&mut app);
    assert_grid(&app, id, Rect::new(137.25, 86.5, 237.25, 186.5), 7, 3);
}

#[test]
fn rectangular_grid_creates_exact_dividers_as_one_undo_redo_step() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 12, "y": 18, "width": 9, "height": 11})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(125.0, 75.0));
    values(&mut app, json!(120), json!(60), json!(2), json!(3));
    let id = confirm(&mut app);
    assert_grid(&app, id, Rect::new(125.0, 75.0, 245.0, 135.0), 2, 3);
    assert_eq!(app.session.doc().unwrap().selection.objects, vec![id]);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo + 1);
    assert_eq!(app.session.journal.last().unwrap().0, "shape.rectangularGrid");
    assert_eq!(app.session.journal.len(), before.journal.len() + 1);
    let created = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before.doc);
    assert_eq!(app.session.doc().unwrap().selection.objects, before.selected);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo);
    let saved = json!({"rows": 2, "columns": 3, "rowSkew": 0.0, "columnSkew": 0.0, "frame": true, "fill": false, "width": 120.0, "height": 60.0, "reference": 0});
    assert_eq!(app.session.tool_options_of(KIND), saved, "undo does not undo the saved tool settings");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, created);
    assert_grid(&app, id, Rect::new(125.0, 75.0, 245.0, 135.0), 2, 3);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo + 1);
}

#[test]
fn rectangular_grid_cancel_discards_edits_and_reopens_from_saved_values() {
    let mut app = app();
    saved_counts(&mut app, 2, 8);
    // A pending redo must survive opening, editing and cancelling the size dialog.
    app.run("shape.rectangle", json!({"x": 5, "y": 6, "width": 7, "height": 8})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(111.0, 147.0));
    values(&mut app, json!(18), json!(30), json!(9), json!(1));
    cancel_of(&mut app, KIND);
    assert!(!is_open(&app, KIND));
    before.assert_unchanged(&app);
    click(&mut app, Point::new(209.0, 173.0));
    assert_eq!((field(&mut app, "x"), field(&mut app, "y")), (json!(209.0), json!(173.0)));
    assert_eq!((field(&mut app, "width"), field(&mut app, "height")), (json!(100.0), json!(100.0)));
    assert_eq!((field(&mut app, "rows"), field(&mut app, "columns")), (json!(2), json!(8)));
    cancel_of(&mut app, KIND);
    before.assert_unchanged(&app);
}

#[test]
fn rectangular_grid_perspective_click_projects_each_anchor_and_is_one_undo_step() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
    app.run("perspective.grid.set", json!({"snap": false})).unwrap();
    app.run("perspective.plane.set", json!({"plane": "right"})).unwrap();
    let grid = PerspectiveGrid::effective(&app.session.doc().unwrap().doc);
    let at = grid.to_page(Plane::Right, Point::new(65.0, 85.0)).unwrap();
    let map = grid.size_homography(Plane::Right, at, false).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, at);
    values(&mut app, json!(96), json!(54), json!(1), json!(2));
    let id = confirm(&mut app);
    let expected = [
        vec![at, Point::new(at.x + 96.0, at.y), Point::new(at.x + 96.0, at.y + 54.0), Point::new(at.x, at.y + 54.0)],
        vec![Point::new(at.x, at.y + 27.0), Point::new(at.x + 96.0, at.y + 27.0)],
        vec![Point::new(at.x + 32.0, at.y), Point::new(at.x + 32.0, at.y + 54.0)],
        vec![Point::new(at.x + 64.0, at.y), Point::new(at.x + 64.0, at.y + 54.0)],
    ];
    let actual = paths(&app, id);
    assert_eq!(actual.len(), expected.len());
    for (index, (path, points)) in actual.iter().zip(expected).enumerate() {
        let projected: Vec<Point> = points.into_iter().map(|p| map.apply(p).unwrap()).collect();
        assert_path(path, &projected, index == 0);
    }
    let state = app.session.doc().unwrap();
    assert_eq!(PerspectiveGrid::effective(&state.doc).attachment_of(&state.doc, id), Some((Plane::Right, 0.0)));
    assert_eq!(state.history.undo.len(), before.undo + 1);
    let (command, params) = app.session.journal.last().unwrap();
    assert_eq!(command, "perspective.draw");
    assert_eq!(params["command"], json!("shape.rectangularGrid"));
    assert_eq!(app.session.journal.len(), before.journal.len() + 1);
    let created = state.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before.doc);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, created);
}

/// A refused creation saves no options and keeps the dialog open with the reason.
#[test]
fn rectangular_grid_failed_creation_does_not_save_counts() {
    let mut app = app();
    saved_counts(&mut app, 2, 3);
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"ids": [layer.0], "locked": true})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(30.0, 40.0));
    values(&mut app, json!(120), json!(60), json!(7), json!(8));
    assert!(try_confirm(&mut app, KIND).is_err());
    assert!(is_open(&app, KIND), "a refusal keeps the dialog open");
    assert_eq!(app.session.tool_options_of(KIND), before.options);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo);
}

/// Tool Options with a width the tool refuses is an error that keeps the dialog open and sets
/// nothing.
#[test]
fn rectangular_grid_tool_options_with_a_refused_width_stay_open() {
    let mut app = app();
    let before = app.session.tool_options_of(KIND);
    super::shapes::open_options(&mut app, KIND).unwrap();
    set(&mut app, "width", json!(0));
    let e = try_confirm(&mut app, KIND).unwrap_err();
    assert!(e.contains("`width`"), "{e}");
    assert!(is_open(&app, KIND));
    assert_eq!(app.session.tool_options_of(KIND), before);
}

/// Both grids' dialogs send their skews, their switch (`frame` / `compound`) and the fill; OK draws
/// with them and stores them.
#[test]
fn a_grid_dialog_sends_its_skews_switch_and_fill() {
    let at = Point::new(10.0, 20.0);
    for kind in [KIND, "polarGrid"] {
        let mut app = app();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let (options, initial, edits) = if kind == KIND {
            (
                json!({"rowSkew": 100, "frame": false}),
                [("rowSkew", json!(100.0)), ("columnSkew", json!(0.0)), ("frame", json!(false)), ("fill", json!(false))],
                [
                    ("width", json!(100)),
                    ("height", json!(100)),
                    ("rows", json!(3)),
                    ("columns", json!(1)),
                    ("columnSkew", json!(-250)),
                    ("fill", json!(true)),
                ],
            )
        } else {
            (
                json!({"concentric": 3, "radial": 4, "radialSkew": 500}),
                [("concentricSkew", json!(0.0)), ("radialSkew", json!(500.0)), ("compound", json!(false)), ("fill", json!(false))],
                [
                    ("concentricSkew", json!(100)),
                    ("compound", json!(true)),
                    ("fill", json!(true)),
                    ("width", json!(100)),
                    ("height", json!(100)),
                    ("reference", json!(0)),
                ],
            )
        };
        app.run("tool.setOption", json!({"tool": kind, "values": options})).unwrap();
        if kind == KIND {
            click(&mut app, at);
        } else {
            super::shapes::open(&mut app, kind, Some(super::shapes::Click { at, alt: false }));
            assert!(is_open(&app, kind));
        }
        for (key, value) in initial {
            assert_eq!(field_of(&app, kind, key), value, "{kind} {key}");
        }
        for (key, value) in edits {
            set_of(&mut app, kind, key, value).unwrap();
        }
        let id = confirm_of(&mut app, kind);
        let doc = &app.session.doc().unwrap().doc;
        let children = doc.node(id).unwrap().children().unwrap();
        assert!(children.iter().all(|n| n.appearance.fill().is_some_and(|f| f.paint != vectorcraft_color::Paint::None)), "{kind}");
        if kind == KIND {
            let paths = paths(&app, id);
            // No frame: four sides, then the dividers, crowded toward the top (100 %: 1/16, 1/4 and 9/16 down).
            assert_eq!(paths.len(), 8);
            assert!(paths.iter().all(|p| !p.is_closed()));
            for (path, y) in paths[4..7].iter().zip([6.25, 25.0, 56.25]) {
                assert_path(path, &[Point::new(10.0, 20.0 + y), Point::new(110.0, 20.0 + y)], false);
            }
            let x = paths[7].subpaths[0].anchors[0].p.x;
            assert!(x > 10.0 && x < 30.0, "toward the left: {x}");
            let saved = json!({"rows": 3, "columns": 1, "rowSkew": 100.0, "columnSkew": -250.0, "frame": false, "fill": true, "width": 100.0, "height": 100.0, "reference": 0});
            assert_eq!(app.session.tool_options_of(KIND), saved);
            // A drag draws with the same options.
            pointer(&mut app, PointerKind::Down, Point::new(300.0, 300.0));
            pointer(&mut app, PointerKind::Drag, Point::new(400.0, 400.0));
            pointer(&mut app, PointerKind::Up, Point::new(400.0, 400.0));
            let dragged = app.session.doc().unwrap().selection.objects[0];
            assert_ne!(dragged, id);
            let doc = &app.session.doc().unwrap().doc;
            let offset = |id: NodeId| {
                doc.node(id).unwrap().children().unwrap().iter().map(|n| n.path_data().unwrap().subpaths[0].anchors[0].p).collect::<Vec<_>>()
            };
            for (a, b) in offset(id).iter().zip(offset(dragged)) {
                assert!((b.x - a.x - 290.0).abs() < 1e-9 && (b.y - a.y - 280.0).abs() < 1e-9, "{a:?} {b:?}");
            }
            assert!(
                doc.node(dragged)
                    .unwrap()
                    .children()
                    .unwrap()
                    .iter()
                    .all(|n| n.appearance.fill().is_some_and(|f| f.paint != vectorcraft_color::Paint::None))
            );
        } else {
            // Two rings under four radial dividers.
            assert_eq!(
                children.iter().map(|n| matches!(n.kind, NodeKind::Compound { .. })).collect::<Vec<_>>(),
                [true, true, false, false, false, false]
            );
            let widths: Vec<f64> = children[0].children().unwrap().iter().map(|n| n.geometric_bounds().unwrap().width()).collect();
            assert!((widths[0] - 100.0).abs() < 1e-6 && (widths[1] - 93.75).abs() < 1e-6, "outward: {widths:?}");
            let tips: Vec<f64> = children[3..].iter().map(|n| n.path_data().unwrap().subpaths[0].anchors[1].p.x).collect();
            assert!(tips.iter().all(|x| *x > 60.0), "right of the top divider: {tips:?}");
            let saved = json!({
                "concentric": 3, "radial": 4, "concentricSkew": 100.0, "radialSkew": 500.0, "compound": true, "fill": true,
                "width": 100.0, "height": 100.0, "reference": 0,
            });
            assert_eq!(app.session.tool_options_of(kind), saved);
        }
    }
}
