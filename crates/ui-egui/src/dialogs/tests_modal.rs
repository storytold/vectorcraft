//! #423: modal dialogs move by their heading (and stay on screen and where they were moved), and
//! the form dialogs' choice fields are dropdowns (Offset Path's Joins).

use egui::{Event, Pos2, Rect, Vec2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::theme;

const SCREEN: Vec2 = vec2(1280.0, 900.0);

/// A context drawing the dialog layer in a 1280 × 900 window.
fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    ctx
}

/// One frame of the dialog layer with `events`: every text drawn, with its screen rect.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<Event>) -> Vec<(String, Rect)> {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
    fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
    texts
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Drag with the mouse from `from` by `by`, in steps, then let go.
fn drag(ctx: &egui::Context, app: &mut VectorcraftApp, from: Pos2, by: Vec2) {
    frame(ctx, app, vec![Event::PointerMoved(from), button(from, true)]);
    for i in 1..=6 {
        frame(ctx, app, vec![Event::PointerMoved(from + by * (i as f32 / 6.0))]);
    }
    frame(ctx, app, vec![button(from + by, false)]);
    frame(ctx, app, vec![]);
}

fn click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: Pos2) -> Vec<(String, Rect)> {
    frame(ctx, app, vec![Event::PointerMoved(pos), button(pos, true)]);
    frame(ctx, app, vec![button(pos, false)]);
    frame(ctx, app, vec![])
}

/// Frames until a window that just opened has measured and placed itself.
fn settle(ctx: &egui::Context, app: &mut VectorcraftApp) {
    for _ in 0..6 {
        frame(ctx, app, vec![]);
    }
}

fn rect_of(ctx: &egui::Context, id: egui::Id) -> Rect {
    ctx.memory(|m| m.area_rect(id)).expect("the dialog window was shown")
}

fn app_with_selection() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    app
}

fn close_to(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < 1.5
}

/// Opens a dialog.
type Open = fn(&mut VectorcraftApp);

/// Every kind of modal window: the shared frame (Offset Path), and the ones drawing their own
/// window (New Document without a heading, Preferences, Keyboard Shortcuts).
#[test]
fn dialogs_move_by_their_heading_stay_on_screen_and_where_they_were_moved() {
    let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    let opens: [(&str, Open, egui::Id); 4] = [
        ("Offset Path", |app| crate::menus::invoke(app, "object.path.offsetPath", json!({})), egui::Id::new(("dialog", "offsetPath"))),
        ("New Document", open_new_document, egui::Id::new(("dialog", "newDocument"))),
        ("Preferences", |app| crate::prefs_dialog::open(app, None), egui::Id::new("dialog-preferences")),
        ("Keyboard Shortcuts", crate::shortcut_editor::open, egui::Id::new("dialog-shortcuts")),
    ];
    for (name, open, id) in opens {
        let (ctx, mut app) = (ctx(), app_with_selection());
        open(&mut app);
        settle(&ctx, &mut app);
        let start = rect_of(&ctx, id);
        // Dragging the heading (the band from the top edge down to it) moves the window.
        drag(&ctx, &mut app, start.left_top() + vec2(40.0, 10.0), vec2(-90.0, 50.0));
        let moved = rect_of(&ctx, id);
        assert!(close_to(moved.min - start.min, vec2(-90.0, 50.0)), "{name}: {start:?} → {moved:?}");
        // A drag that starts below the heading (in the bottom margin) doesn't.
        drag(&ctx, &mut app, moved.left_bottom() + vec2(6.0, -4.0), vec2(80.0, -80.0));
        assert_eq!(rect_of(&ctx, id), moved, "{name}: dragging the body moved it");
        // Dragged far past the window's edge, it stops at the edge.
        drag(&ctx, &mut app, moved.left_top() + vec2(40.0, 10.0), vec2(5000.0, 5000.0));
        let edge = rect_of(&ctx, id);
        assert!(screen.contains_rect(edge.shrink(0.5)), "{name}: off screen at {edge:?}");
        assert!((edge.right() - screen.right()).abs() < 1.5 && (edge.bottom() - screen.bottom()).abs() < 1.5, "{name}: {edge:?}");
        // Closed and opened again (this session): where it was left.
        app.ui.dialog = None;
        frame(&ctx, &mut app, vec![]);
        open(&mut app);
        settle(&ctx, &mut app);
        assert!(close_to(rect_of(&ctx, id).min - edge.min, Vec2::ZERO), "{name}: reopened at {:?}, left at {edge:?}", rect_of(&ctx, id));
    }
}

/// Offset Path's Joins is a dropdown of Miter, Round and Bevel (it was a text field), listed after
/// Offset; choosing Round runs the command with round joins.
#[test]
fn offset_path_joins_is_a_dropdown() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    frame(&ctx, &mut app, vec![]);
    let texts = frame(&ctx, &mut app, vec![]);
    let at = |texts: &[(String, Rect)], s: &str| texts.iter().find(|(t, _)| t == s).map(|(_, r)| *r);
    let (offset, joins, miter) = (at(&texts, "Offset:").unwrap(), at(&texts, "Joins:").unwrap(), at(&texts, "Miter Limit:").unwrap());
    assert!(offset.top() < joins.top() && joins.top() < miter.top(), "Offset, Joins, Miter Limit: {texts:?}");
    let current = at(&texts, "Miter").expect("Joins shows its choice");
    assert!(at(&texts, "miter").is_none(), "Joins is a text field: {texts:?}");
    // The list: Miter, Round, Bevel.
    let open = click(&ctx, &mut app, current.center());
    for choice in ["Round", "Bevel"] {
        assert!(at(&open, choice).is_some(), "{choice} isn't listed: {open:?}");
    }
    let round = at(&open, "Round").unwrap();
    click(&ctx, &mut app, round.center());
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["joins"], json!("round"));
    // OK: the same result as the command with round joins (not miter).
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let made = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        format!("{:?}", st.doc.node(st.selection.objects[0]).unwrap().kind)
    };
    let direct = |joins: &str| {
        let mut app = app_with_selection();
        app.run("select.all", json!({})).unwrap();
        app.run("object.path.offsetPath", json!({"offset": 10, "joins": joins, "miterLimit": 4})).unwrap();
        made(&app)
    };
    assert_eq!(made(&app), direct("round"));
    assert_ne!(made(&app), direct("miter"));
}

/// The other form dialogs' choices are dropdowns too: Average's and Shear's axis.
#[test]
fn form_dialog_axes_are_dropdowns() {
    for (id, shown) in [("path.average", "Both"), ("object.shear", "Horizontal"), ("object.reflect", "Vertical")] {
        let (ctx, mut app) = (ctx(), app_with_selection());
        app.run("select.all", json!({})).unwrap();
        crate::menus::invoke(&mut app, id, json!({}));
        frame(&ctx, &mut app, vec![]);
        let texts = frame(&ctx, &mut app, vec![]);
        assert!(texts.iter().any(|(t, _)| t == shown), "{id}: {texts:?}");
        assert!(!texts.iter().any(|(t, _)| *t == shown.to_lowercase()), "{id} has a text field: {texts:?}");
    }
}

/// The axis choices are translated in the "axis" context: in Japanese the plain Horizontal and
/// Vertical are the type orientations (horizontal and vertical writing).
#[test]
fn axis_choices_read_as_axes_in_every_complete_language() {
    use crate::i18n::{LANGUAGES, Lang, tr, tr_ctx};
    let ja = Lang::from_code("ja").unwrap();
    assert_eq!(tr_ctx(ja, "axis", "Horizontal"), "水平方向");
    assert_ne!(tr_ctx(ja, "axis", "Horizontal"), tr(ja, "Horizontal"));
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        for s in ["Horizontal", "Vertical", "Both"] {
            assert!(l.source.lines().any(|line| line.starts_with(&format!("axis\t{s}\t"))), "{}: no axis row for {s}", l.code);
        }
    }
}

/// Offset Path previews on the canvas; Cancel rolls the preview back.
#[test]
fn offset_path_previews() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("select.all", json!({})).unwrap();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum::<usize>();
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    let texts = frame(&ctx, &mut app, vec![]);
    let texts = if texts.is_empty() { frame(&ctx, &mut app, vec![]) } else { texts };
    let preview = texts.iter().find(|(t, _)| t == "Preview").map(|(_, r)| *r).expect("a Preview checkbox");
    click(&ctx, &mut app, preview.center());
    frame(&ctx, &mut app, vec![]);
    assert_eq!(count(&app), 2, "previewed");
    cancel(&mut app);
    assert_eq!(count(&app), 1, "rolled back");
}

/// Offset Path opens on the values it was last confirmed with (#1152).
#[test]
fn offset_path_remembers_its_last_values() {
    let mut app = app_with_selection();
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("offset".into(), json!(-4.5));
    d.fields.insert("joins".into(), json!("round"));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("offset", 0.0), d.str("joins"), d.f64("miterLimit", 0.0)), (-4.5, "round".to_string(), 4.0));
}

fn app_with_dense_path() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let anchors: Vec<_> = (0..20).map(|i| json!({"x": 30 + i * 15, "y": 100.0 + (i % 2) as f64 * 0.1})).collect();
    let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 19]]})).unwrap();
    app
}

/// Object › Path › Simplify, then the slider's More Options: the Simplify dialog.
fn open_simplify_dialog(app: &mut VectorcraftApp) {
    crate::menus::invoke(app, "object.path.simplify", json!({}));
    app.run("ui.simplifyMoreOptions", json!({})).unwrap();
}

/// The anchors of the paths in `app`'s document.
fn anchor_count(app: &VectorcraftApp) -> usize {
    let mut n = 0;
    app.session.doc().unwrap().doc.walk(|node| {
        if let vectorcraft_doc::NodeKind::Path { path, .. } = &node.kind {
            n += path.anchor_count();
        }
    });
    n
}

/// The parameters of the Simplify preview on the canvas.
fn simplify_preview(app: &VectorcraftApp) -> serde_json::Value {
    app.session.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).expect("a preview").1
}

/// The Simplify slider's strip, and the centers of its Auto-Simplify button, its rail and its More
/// Options button.
fn slider_strip(ctx: &egui::Context) -> (Rect, Pos2, Pos2, Pos2) {
    let r = rect_of(ctx, egui::Id::new(("dialog", simplify_slider::KIND)));
    let y = r.center().y;
    (r, Pos2::new(r.left() + 8.0 + 14.0, y), Pos2::new(r.left() + 8.0 + 28.0 + 6.0 + 90.0, y), Pos2::new(r.right() - 8.0 - 14.0, y))
}

/// Object › Path › Simplify opens a slider under the selection that previews the 1 pt result.
/// Dragging it to the right previews more anchor points and to the left fewer, and Enter keeps the
/// result as one undo step.
#[test]
fn simplify_opens_a_slider_whose_drag_previews_fewer_or_more_points() {
    let (ctx, mut app) = (ctx(), app_with_dense_path());
    let undo_len = app.session.doc().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(simplify_slider::KIND));
    settle(&ctx, &mut app);
    assert_eq!(simplify_preview(&app), json!({"level": 50.0}));
    assert_eq!(anchor_count(&app), 2, "the 1 pt result");
    let (_, _, rail, _) = slider_strip(&ctx);
    drag(&ctx, &mut app, rail, vec2(100.0, 0.0));
    assert_eq!(simplify_preview(&app), json!({"level": 100.0}));
    let most = anchor_count(&app);
    assert!(most > 2, "{most}");
    let (_, _, rail, _) = slider_strip(&ctx);
    drag(&ctx, &mut app, rail + vec2(90.0, 0.0), vec2(-190.0, 0.0));
    assert_eq!(simplify_preview(&app), json!({"level": 0.0}));
    assert_eq!(anchor_count(&app), 2);
    frame(
        &ctx,
        &mut app,
        vec![Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
    );
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    assert_eq!(anchor_count(&app), 2);
}

/// Auto-Simplify moves the slider to the level that keeps the path within 1/400 of its size, and a
/// click outside the strip keeps the result as one undo step.
#[test]
fn simplify_slider_auto_simplify_and_a_click_outside_keep_the_result() {
    let (ctx, mut app) = (ctx(), app_with_dense_path());
    let before = app.session.doc().unwrap().doc.clone();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    settle(&ctx, &mut app);
    let (_, auto, ..) = slider_strip(&ctx);
    click(&ctx, &mut app, auto);
    // The path is 285 pt wide: 0.7125 pt at most, level 55.
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("level", 0.0), 55.0);
    assert_eq!(app.session.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).unwrap().1, json!({"auto": true}));
    frame(&ctx, &mut app, vec![]);
    assert_eq!(anchor_count(&app), 2);
    assert!(app.session.in_interaction(), "the result waits for a click outside");
    click(&ctx, &mut app, Pos2::new(20.0, 20.0));
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    assert_ne!(app.session.doc().unwrap().doc, before);
}

/// More Options opens the Simplify dialog at the slider's tolerance. With "Open this dialog with
/// these settings next time" ticked, OK makes the menu item open the dialog on its settings; with
/// it unticked, the slider again.
#[test]
fn simplify_slider_more_options_and_the_dialog_that_opens_directly() {
    let (ctx, mut app) = (ctx(), app_with_dense_path());
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    app.ui.dialog.as_mut().unwrap().fields.insert("level".into(), json!(60));
    settle(&ctx, &mut app);
    assert_eq!(simplify_preview(&app), json!({"level": 60.0}));
    let (_, _, _, more) = slider_strip(&ctx);
    click(&ctx, &mut app, more);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.f64("tolerance", 0.0), d.bool("retain")), ("simplify", 0.5, false));
    settle(&ctx, &mut app);
    assert_eq!(simplify_preview(&app)["tolerance"], json!(0.5));
    let texts = frame(&ctx, &mut app, vec![]);
    let retain = texts.iter().find(|(t, _)| t == "Open this dialog with these settings next time").map(|(_, r)| *r).expect("the checkbox");
    click(&ctx, &mut app, retain.center());
    app.ui.dialog.as_mut().unwrap().fields.insert("straightLines".into(), json!(true));
    confirm(&mut app).unwrap();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    let d = app.ui.dialog.clone().unwrap();
    assert_eq!((d.kind.as_str(), d.f64("tolerance", 0.0), d.bool("straightLines"), d.bool("retain")), ("simplify", 0.5, true, true));
    app.ui.dialog.as_mut().unwrap().fields.insert("retain".into(), json!(false));
    confirm(&mut app).unwrap();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(simplify_slider::KIND));
    assert!(app.run("ui.simplifyAuto", json!({})).is_ok());
    cancel(&mut app);
    assert!(app.run("ui.simplifyMoreOptions", json!({})).is_err(), "the slider is not open");
}

#[test]
fn simplify_dialog_preview_counts_and_cancel_restore_the_original() {
    let (ctx, mut app) = (ctx(), app_with_dense_path());
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    open_simplify_dialog(&mut app);
    settle(&ctx, &mut app);
    let texts = frame(&ctx, &mut app, vec![]);
    let preview = texts.iter().find(|(t, _)| t == "Preview").map(|(_, r)| *r).expect("a Preview checkbox");
    assert!(texts.iter().any(|(t, _)| t == "Anchor Points: 20 → 2"), "the reduction is visible: {texts:?}");
    assert!(app.session.in_interaction());
    assert!(app.session.doc().unwrap().selection.anchors.is_empty());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len);
    // The dialog focuses its first field. Type as a user does, then Tab commits it
    // without confirming the dialog; replacing only its bound value leaves old focused text.
    frame(&ctx, &mut app, vec![Event::Text("0.01 pt".into())]);
    frame(&ctx, &mut app, vec![Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("tolerance", 0.0), 0.01);
    let fine = app.session.doc().unwrap().doc.clone();
    click(&ctx, &mut app, preview.center());
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert!(!app.session.in_interaction());
    let texts = frame(&ctx, &mut app, vec![]);
    assert!(!texts.iter().any(|(t, _)| t.starts_with("Anchor Points:")), "no stale counts while Preview is off");
    let preview = texts.iter().find(|(t, _)| t == "Preview").unwrap().1;
    click(&ctx, &mut app, preview.center());
    assert_eq!(app.session.doc().unwrap().doc, fine);
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len);
    app.run("object.path.simplify", json!({"tolerance": 0.01})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, fine, "changing tolerance refits the original, not the previous preview");
}

#[test]
fn simplify_dialog_ok_keeps_exactly_one_undo_with_or_without_preview() {
    for on in [true, false] {
        let (ctx, mut app) = (ctx(), app_with_dense_path());
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let undo_len = app.session.doc().unwrap().history.undo.len();
        open_simplify_dialog(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(on));
        settle(&ctx, &mut app);
        if on {
            app.ui.dialog.as_mut().unwrap().fields.insert("tolerance".into(), json!("0.01 pt"));
            frame(&ctx, &mut app, vec![]);
            app.ui.dialog.as_mut().unwrap().fields.insert("tolerance".into(), json!("1 pt"));
            frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len);
        confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        assert!(app.session.doc().unwrap().selection.anchors.is_empty());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
        let after = app.session.doc().unwrap().doc.clone();
        assert_ne!(after, before);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
        assert!(app.session.doc().unwrap().selection.anchors.is_empty());
    }
}

/// Whether every segment of every path in `app`'s document is straight (false with no segments).
fn only_straight_segments(app: &VectorcraftApp) -> bool {
    let mut lines = vec![];
    app.session.doc().unwrap().doc.walk(|n| {
        if let vectorcraft_doc::NodeKind::Path { path, .. } = &n.kind {
            for sp in &path.subpaths {
                lines.extend((0..sp.segment_count()).map(|i| sp.segment_is_line(i)));
            }
        }
    });
    !lines.is_empty() && lines.iter().all(|l| *l)
}

/// Simplify's dialog sends Corner Point Angle Threshold and Convert to Straight Lines with the
/// tolerance, and ticking Convert to Straight Lines previews straight segments only.
#[test]
fn simplify_dialog_sends_corner_angle_and_straight_lines() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("shape.ellipse", json!({"x": 100, "y": 200, "width": 160, "height": 90})).unwrap();
    open_simplify_dialog(&mut app);
    settle(&ctx, &mut app);
    let texts = frame(&ctx, &mut app, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Corner Point Angle Threshold:"), "{texts:?}");
    let previewed = |app: &VectorcraftApp| app.session.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).expect("a preview").1;
    assert_eq!((&previewed(&app)["cornerAngle"], &previewed(&app)["straightLines"]), (&json!(30.0), &json!(false)));
    assert!(!only_straight_segments(&app), "the fit keeps the ellipse's curves");
    let straight = texts.iter().find(|(t, _)| t == "Convert to Straight Lines").map(|(_, r)| *r).expect("a Convert to Straight Lines checkbox");
    click(&ctx, &mut app, straight.center());
    assert_eq!(previewed(&app)["straightLines"], json!(true));
    assert!(only_straight_segments(&app), "Convert to Straight Lines previews straight segments only");
    // A higher threshold keeps more corners: the command's turn angle goes down as it goes up.
    app.ui.dialog.as_mut().unwrap().fields.insert("angleThreshold".into(), json!(170));
    frame(&ctx, &mut app, vec![]);
    assert_eq!(previewed(&app)["cornerAngle"], json!(10.0));
    confirm(&mut app).unwrap();
    assert!(only_straight_segments(&app), "OK keeps the straight segments");
}

/// Corner Point Angle Threshold: an octagon's 135° corners stay corners at the 150° the dialog opens
/// with, and a threshold below 135° smooths them into curves.
#[test]
fn simplify_dialog_corner_point_angle_threshold_keeps_or_smooths_corners() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("edit.clear", json!({})).unwrap();
    app.run("shape.polygon", json!({"cx": 200, "cy": 200, "radius": 100, "sides": 8})).unwrap();
    open_simplify_dialog(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("tolerance".into(), json!("10 pt"));
    settle(&ctx, &mut app);
    assert!(only_straight_segments(&app), "the corners stay and the sides stay straight");
    app.ui.dialog.as_mut().unwrap().fields.insert("angleThreshold".into(), json!(120));
    frame(&ctx, &mut app, vec![]);
    assert!(!only_straight_segments(&app), "the corners are smoothed into curves");
}

/// Show Original Path: while Simplify previews with it on, the canvas outlines the paths as they
/// were before the preview.
#[test]
fn simplify_dialog_show_original_path_outlines_the_paths_before_the_preview() {
    let (ctx, mut app) = (ctx(), app_with_dense_path());
    let st = app.session.doc().unwrap();
    let (id, original) = match st.doc.node(st.selection.objects[0]).map(|n| (n.id, &n.kind)) {
        Some((id, vectorcraft_doc::NodeKind::Path { path, .. })) => (id, path.clone()),
        _ => panic!("a path is selected"),
    };
    open_simplify_dialog(&mut app);
    settle(&ctx, &mut app);
    let texts = frame(&ctx, &mut app, vec![]);
    assert!(super::path_ops::original_paths(&app).is_empty(), "Show Original Path is off as the dialog opens");
    let show = texts.iter().find(|(t, _)| t == "Show Original Path").map(|(_, r)| *r).expect("a Show Original Path checkbox");
    click(&ctx, &mut app, show.center());
    assert_eq!(super::path_ops::original_paths(&app), vec![(id, &original)]);
    let simplified = app.session.doc().unwrap().doc.node(id).map(|n| n.kind.clone());
    assert!(!matches!(simplified, Some(vectorcraft_doc::NodeKind::Path { path, .. }) if path == original), "the preview changed the path");
    let preview = frame(&ctx, &mut app, vec![]).into_iter().find(|(t, _)| t == "Preview").map(|(_, r)| r).expect("a Preview checkbox");
    click(&ctx, &mut app, preview.center());
    assert!(super::path_ops::original_paths(&app).is_empty(), "nothing to compare with while Preview is off");
    cancel(&mut app);
    assert!(super::path_ops::original_paths(&app).is_empty());
}

#[test]
fn printer_mark_labels_are_distinct_from_language_names() {
    let cs = crate::i18n::Lang::from_code("cs").unwrap();
    assert_eq!(save_pdf::choice_label(cs, "marks.kind", "Japanese"), "Japonské");
    assert_eq!(save_pdf::choice_label(cs, "language", "Japanese"), "Japonština");
    assert_eq!(save_pdf::choice_label(crate::i18n::Lang::EN, "marks.kind", "Japanese"), "Japanese");
}
