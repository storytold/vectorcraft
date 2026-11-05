//! Type → Type on a Path → Type on a Path Options… (#429): the dialog shows the selected type's
//! options (Effect, Flip, Align to Path, Spacing) and OK leaves its brackets where they are; its
//! Preview shows them on the canvas while it is open (#5360). The Selection tool draws no brackets
//! on a huge selection.

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

/// Open Type on a Path Options… from its menu item.
fn open_options(app: &mut VectorcraftApp) {
    let entry = menus::menu_entries(app).into_iter().find(|e| e.label == "Type on a Path Options…").expect("menu item");
    assert_eq!(entry.path, ["Type", "Type on a Path"]);
    assert!(entry.enabled);
    let (cmd, params) = menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    menus::invoke(app, &cmd, params);
}

/// Two headless canvas frames → the vertices of the second one's tessellated mesh, which the GPU
/// takes in one buffer.
fn canvas_vertices(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    ctx.run_ui(raw(), |ui| crate::canvas::show(app, ui)).textures_delta.clear();
    let mut out = ctx.run_ui(raw(), |ui| crate::canvas::show(app, ui));
    out.textures_delta.clear();
    let mesh = |p: &egui::ClippedPrimitive| if let egui::epaint::Primitive::Mesh(m) = &p.primitive { m.vertices.len() } else { 0 };
    ctx.tessellate(out.shapes, out.pixels_per_point).iter().map(mesh).sum()
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

    open_options(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), ("type.pathOptions".to_string(), "Type on a Path Options".to_string()));
    for (k, v) in [("effect", json!("rainbow")), ("flip", json!(false)), ("alignToPath", json!("baseline")), ("spacing", json!(0.0))] {
        assert_eq!(d.fields.get(k), Some(&v), "{k}");
    }
    assert!(!d.fields.contains_key("start") && !d.fields.contains_key("end"), "the brackets aren't the dialog's");
    // It draws headlessly: the choices by name.
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| dialogs::show(app, ui.ctx()));
    for label in ["Type on a Path Options", "Effect:", "Rainbow", "Flip:", "Align to Path:", "Baseline", "Spacing:"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    // OK as it opened: nothing moves.
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(brackets(&app, id), (start, None));

    // Flip and Align to Path: the type turns to the other side of the same stretch of its path.
    open_options(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("flip".into(), Value::Bool(true));
    d.fields.insert("alignToPath".into(), json!("center"));
    dialogs::confirm(&mut app).unwrap();
    let (s, e) = brackets(&app, id);
    assert!(s.abs() < 1e-9 && e.is_some_and(|e| (e - (1.0 - start)).abs() < 1e-9), "{s} {e:?}");
    // The dialog shows the new alignment, and Flip is unchecked again (it flips once more).
    open_options(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields.get("alignToPath"), d.fields.get("flip")), (Some(&json!("center")), Some(&json!(false))));
}

/// The effect, alignment and spacing of type on a path `id`.
fn options(app: &VectorcraftApp, id: NodeId) -> (&'static str, &'static str, f64) {
    match app.session.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => (t.path_effect.id(), t.path_align.id(), t.path_spacing),
        k => panic!("not text: {k:?}"),
    }
}

/// One headless frame of the open dialog.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
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

    open_options(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(dialogs::accepts_field(d, "preview"));
    assert!(!d.bool("preview"), "Preview starts off");
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| dialogs::show(app, ui.ctx()));
    assert!(text.contains("Preview"), "{text}");

    // Off: the fields change nothing until OK.
    app.ui.dialog.as_mut().unwrap().fields.insert("effect".into(), json!("skew"));
    frame(&mut app);
    assert_eq!(state(&app), original, "no preview without the checkbox");
    // On: the type follows the fields.
    dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    frame(&mut app);
    assert_eq!(options(&app, id).0, "skew");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("alignToPath".into(), json!("center"));
    d.fields.insert("spacing".into(), json!(6.0));
    frame(&mut app);
    assert_eq!(options(&app, id), ("skew", "center", 6.0));
    // Flip turns the type over once, frame after frame, and back when unchecked.
    app.ui.dialog.as_mut().unwrap().fields.insert("flip".into(), json!(true));
    frame(&mut app);
    let flipped = brackets(&app, id);
    assert_ne!(flipped, original.0, "the preview flips the type");
    frame(&mut app);
    assert_eq!(brackets(&app, id), flipped, "and keeps it flipped once");
    app.ui.dialog.as_mut().unwrap().fields.insert("flip".into(), json!(false));
    frame(&mut app);
    assert_eq!(brackets(&app, id), original.0, "unchecked, it turns back");
    // Cancel puts the original back and leaves no history.
    dialogs::cancel(&mut app);
    assert_eq!(state(&app), original, "Cancel restores the type");
    assert_eq!(steps(&app), before, "and adds no undo step");
    assert!(!app.session.in_interaction());

    // OK with Preview on and nothing changed: no undo step.
    open_options(&mut app);
    dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!((state(&app), steps(&app)), (original, before));

    // OK keeps the previewed values as one undo step.
    open_options(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("effect".into(), json!("stairStep"));
    d.fields.insert("flip".into(), json!(true));
    dialogs::set_field(d, "preview".into(), json!(true));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!((options(&app, id).0, brackets(&app, id)), ("stairStep", flipped));
    assert_eq!(steps(&app), before + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(state(&app), original, "one Undo takes the whole change back");
}

/// The Selection tool's brackets on one selected type on a path add their lines to the frame's
/// mesh. On 32,768 (an opened PDF can hold as many curved text lines) they add none.
#[test]
fn a_huge_selection_of_type_on_a_path_draws_no_brackets() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let path = app.run("path.create", json!({"d": "M20 100 L380 100"})).unwrap()["id"].clone();
    app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along"})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.bounding_box = false;
    let ctx = egui::Context::default();
    // The vertices that the Selection tool adds over the Rectangle tool, which draws nothing of its
    // own, with everything selected.
    let brackets = |app: &mut VectorcraftApp| {
        app.run("select.all", json!({})).unwrap();
        app.select_tool("rectangle");
        let rectangle = canvas_vertices(app, &ctx);
        app.select_tool("selection");
        canvas_vertices(app, &ctx).saturating_sub(rectangle)
    };
    let one = brackets(&mut app);
    assert!(one > 0, "one shows its brackets");
    for _ in 0..15 {
        app.run("select.all", json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    let many = brackets(&mut app);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 32_768);
    assert_eq!(many, 0, "32,768 show no brackets, not {many} vertices");
}
