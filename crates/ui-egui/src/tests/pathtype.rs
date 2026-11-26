//! Type → Type on a Path → Type on a Path Options… (#429): the dialog shows the selected type's
//! options (Effect, Flip, Align to Path, Spacing) and OK leaves its brackets where they are; its
//! Preview shows them on the canvas while it is open (#5360).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextKind};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus};

/// The start and end brackets of type on a path `id`.
fn brackets(app: &VectorcraftApp, id: NodeId) -> (f64, Option<f64>) {
    match app.session.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => match t.kind {
            TextKind::OnPath { start, end, .. } => (start, end),
            ref k => panic!("not type on a path: {k:?}"),
        },
        k => panic!("not text: {k:?}"),
    }
}

/// One whole frame of the app, then the dialog pump: the dialog host runs inside the app's pass, and a held OK or a
/// Cancel is answered by the pump.
pub(super) fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    crate::testing::app_frame(app, ctx, egui::RawInput::default());
    dialogs::host::pump(app, ctx);
}

/// The top open dialog's id and its script fields.
pub(super) fn top(app: &VectorcraftApp) -> (egui::Id, Value) {
    // PLAN: assumed `DialogId` is `egui::Id` (ui-panels dialog/stack.rs:6).
    let open = app.dialogs().expect("a window").with(|h| h.open_dialogs()).pop().expect("a dialog is open");
    (open.id, open.fields.expect("the dialog lists its values"))
}

/// Sets the top dialog's script field `key`, as the user editing that control would.
pub(super) fn set_field(app: &VectorcraftApp, ctx: &egui::Context, key: &str, value: Value) {
    let (id, _) = top(app);
    app.dialogs().expect("a window").with(|h| h.set_json_field(ctx, id, key, value)).unwrap();
}

/// OK on the top dialog, whose held OK the pump answers, then the frame after it.
pub(super) fn confirm(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (id, _) = top(app);
    super::picks::answer_dialog(app, id, &[], true);
    frame(app, ctx);
}

/// Cancel on the top dialog, then the frame that answers it.
fn cancel(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (id, _) = top(app);
    assert!(app.dialogs().expect("a window").with(|h| h.cancel_by_id(ctx, id)));
    frame(app, ctx);
}

/// Whether any dialog is still open.
fn any_open(app: &VectorcraftApp) -> bool {
    !app.dialogs().expect("a window").with(|h| h.open_dialogs()).is_empty()
}

/// The app with its window shown once (the dialog host lives in it), and the context it runs in.
pub(super) fn shown(app: &mut VectorcraftApp) -> egui::Context {
    let ctx = crate::testing::ui_context();
    frame(app, &ctx);
    ctx
}

/// Open Type on a Path Options… from its menu item.
fn open_options(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let entry = menus::menu_entries(app).into_iter().find(|e| e.label == "Type on a Path Options…").expect("menu item");
    assert_eq!(entry.path, ["Type", "Type on a Path"]);
    assert!(entry.enabled);
    let (cmd, params) = menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    menus::invoke(app, &cmd, params);
    frame(app, ctx);
}

#[test]
fn type_on_a_path_options_keep_the_type_where_it_is() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let path = app.run("path.create", json!({"d": "M100 200 L500 200"})).unwrap()["id"].as_u64().unwrap();
    let r = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [300, 210]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let (start, _) = brackets(&app, id);
    assert!((start - 0.5).abs() < 1e-3, "{start}");
    let ctx = shown(&mut app);

    // OK as it opened: nothing moves.
    open_options(&mut app, &ctx);
    confirm(&mut app, &ctx);
    assert!(!any_open(&app));
    assert_eq!(brackets(&app, id), (start, None));

    // Flip and Align to Path: the type turns to the other side of the same stretch of its path.
    open_options(&mut app, &ctx);
    set_field(&app, &ctx, "flip", Value::Bool(true));
    set_field(&app, &ctx, "alignToPath", json!("center"));
    confirm(&mut app, &ctx);
    let (s, e) = brackets(&app, id);
    assert!(s.abs() < 1e-9 && e.is_some_and(|e| (e - (1.0 - start)).abs() < 1e-9), "{s} {e:?}");
    // The dialog shows the new alignment, and Flip is unchecked again (it flips once more).
    open_options(&mut app, &ctx);
    let (_, fields) = top(&app);
    assert_eq!((fields.get("alignToPath"), fields.get("flip")), (Some(&json!("center")), Some(&json!(false))));
}

/// The effect, alignment and spacing of type on a path `id`.
fn options(app: &VectorcraftApp, id: NodeId) -> (&'static str, &'static str, f64) {
    match app.session.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => (t.path_effect.id(), t.path_align.id(), t.path_spacing),
        k => panic!("not text: {k:?}"),
    }
}

/// #5360: Type on a Path Options has a Preview checkbox, off when it opens. With it on, the type
/// on the canvas follows Effect, Flip, Align to Path and Spacing while the dialog is open (a
/// checked Flip turns it over once, however many frames show it); Cancel puts it back without a
/// step of history; OK keeps what was previewed as one step, and none when nothing changed.
#[test]
fn type_on_a_path_options_preview_on_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let path = app.run("path.create", json!({"d": "M100 200 L500 200"})).unwrap()["id"].as_u64().unwrap();
    let r = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [300, 210]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let original = (brackets(&app, id), options(&app, id));
    let state = |app: &VectorcraftApp| (brackets(app, id), options(app, id));
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let before = steps(&app);
    let ctx = shown(&mut app);

    open_options(&mut app, &ctx);
    assert_eq!(top(&app).1.get("preview"), Some(&json!(false)), "Preview starts off");

    // Off: the fields change nothing until OK.
    set_field(&app, &ctx, "effect", json!("skew"));
    frame(&mut app, &ctx);
    assert_eq!(state(&app), original, "no preview without the checkbox");
    // On: the type follows the fields.
    set_field(&app, &ctx, "preview", json!(true));
    frame(&mut app, &ctx);
    assert_eq!(options(&app, id).0, "skew");
    set_field(&app, &ctx, "alignToPath", json!("center"));
    set_field(&app, &ctx, "spacing", json!(6.0));
    frame(&mut app, &ctx);
    assert_eq!(options(&app, id), ("skew", "center", 6.0));
    // Flip turns the type over once, frame after frame, and back when unchecked.
    set_field(&app, &ctx, "flip", json!(true));
    frame(&mut app, &ctx);
    let flipped = brackets(&app, id);
    assert_ne!(flipped, original.0, "the preview flips the type");
    frame(&mut app, &ctx);
    assert_eq!(brackets(&app, id), flipped, "and keeps it flipped once");
    set_field(&app, &ctx, "flip", json!(false));
    frame(&mut app, &ctx);
    assert_eq!(brackets(&app, id), original.0, "unchecked, it turns back");
    // Cancel puts the original back and leaves no history.
    cancel(&mut app, &ctx);
    assert_eq!(state(&app), original, "Cancel restores the type");
    assert_eq!(steps(&app), before, "and adds no undo step");
    assert!(!app.session.in_interaction());

    // OK with Preview on and nothing changed: no undo step.
    open_options(&mut app, &ctx);
    set_field(&app, &ctx, "preview", json!(true));
    frame(&mut app, &ctx);
    confirm(&mut app, &ctx);
    assert!(!any_open(&app));
    assert_eq!((state(&app), steps(&app)), (original, before));

    // OK keeps the previewed values as one undo step.
    open_options(&mut app, &ctx);
    set_field(&app, &ctx, "effect", json!("stairStep"));
    set_field(&app, &ctx, "flip", json!(true));
    set_field(&app, &ctx, "preview", json!(true));
    frame(&mut app, &ctx);
    confirm(&mut app, &ctx);
    assert!(!any_open(&app));
    assert_eq!((options(&app, id).0, brackets(&app, id)), ("stairStep", flipped));
    assert_eq!(steps(&app), before + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(state(&app), original, "one Undo takes the whole change back");
}
