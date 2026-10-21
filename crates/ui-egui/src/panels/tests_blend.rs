//! The Blend panel drawn headless: Window › Blend opens it, and its controls do nothing until the
//! selection allows them. Make needs two or more objects, Spacing and Orientation need a blend or
//! two or more objects, and the other buttons need a blend. Make blends the selection, Spacing and
//! Orientation edit the selected blend through `object.blend.options`, and each other button runs
//! its Object › Blend command.

use egui::{Event, Pos2, Rect, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing};
use vectorcraft_doc::{BlendSpec, NodeId, NodeKind};
use vectorcraft_engine::Session;

use super::blend::{ID, show};
use crate::{VectorcraftApp, theme};

/// Frames of the panel alone in a 300 × 600 window, with the accessibility tree on.
struct Frames {
    ctx: egui::Context,
    time: f64,
}

impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        Self { ctx, time: 0.0 }
    }

    fn run(&mut self, app: &mut VectorcraftApp, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 600.0));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    }

    /// The center of the text `label` the panel draws.
    fn text(&mut self, app: &mut VectorcraftApp, label: &str) -> Pos2 {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let mut texts = vec![];
        fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
        texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no {label:?} in {texts:?}")).1.center()
    }

    /// The center of the button that screen readers name `label`.
    fn button(&mut self, app: &mut VectorcraftApp, label: &str) -> Pos2 {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let node = nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button);
        let b = node.unwrap_or_else(|| panic!("no button named {label:?}")).1.bounds().expect("bounds");
        pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
    }

    /// Move to `at` and click there. egui finds what the pointer is over from the frame before, so
    /// the move gets a frame of its own, drawn with the selection as it is now.
    fn click(&mut self, app: &mut VectorcraftApp, at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.run(app, vec![Event::PointerMoved(at)]);
        self.run(app, vec![button(true)]);
        self.run(app, vec![button(false)]);
        self.run(app, vec![]);
    }

    /// Click the text `label` the panel draws.
    fn click_text(&mut self, app: &mut VectorcraftApp, label: &str) {
        let at = self.text(app, label);
        self.click(app, at);
    }
}

/// A document with two 20 pt squares 200 pt apart, both selected.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app
}

/// [`app`] with the squares blended at Smooth Color; the blend is selected.
fn blended() -> (VectorcraftApp, NodeId) {
    let mut app = app();
    let id = app.run("object.blend.make", json!({"smooth": true})).unwrap()["id"].as_u64().unwrap();
    (app, NodeId(id))
}

fn spec(app: &VectorcraftApp, id: NodeId) -> BlendSpec {
    match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Blend { spec, .. } => (**spec).clone(),
        k => panic!("not a blend: {k:?}"),
    }
}

fn undo_depth(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The top-level objects of the first layer.
fn top(app: &VectorcraftApp) -> Vec<(NodeId, bool)> {
    let doc = &app.session.active().unwrap().doc;
    doc.layers[0].children().unwrap().iter().map(|n| (n.id, matches!(n.kind, NodeKind::Blend { .. }))).collect()
}

#[test]
fn window_blend_opens_the_panel() {
    let mut app = app();
    let entries = crate::menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Window"] && e.label == "Blend").expect("Window › Blend");
    assert_eq!(item.command.as_deref(), Some("window.panel"));
    assert_eq!(item.params, json!({"panel": ID, "toggle": true}));
    crate::menus::invoke(&mut app, "window.panel", item.params.clone());
    assert_eq!(app.ui.open_panel.as_deref(), Some(ID));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::show_icon_panel(app, ui, ID));
    for label in ["Make", "Spacing:", "Orientation:", "Direction:", "Reverse Front to Back", "Reverse Spine", "Replace Spine", "Expand", "Release"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    assert!(!text.contains("on the roadmap"), "{text}");
}

#[test]
fn the_controls_are_dimmed_until_a_blend_or_two_objects_are_selected() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let depth = undo_depth(&app);
    let mut f = Frames::new();
    let page = f.button(&mut app, "Align to Page");
    let path = f.button(&mut app, "Align to Path");
    // Nothing selected: no button acts and Orientation does not change the options new blends
    // start with.
    for label in ["Make", "Reverse Front to Back", "Reverse Spine", "Replace Spine", "Expand", "Release"] {
        f.click_text(&mut app, label);
    }
    f.click(&mut app, path);
    assert_eq!(app.session.execute("object.blend.info", &json!({})).unwrap()["orientation"], "page");
    assert_eq!((top(&app).len(), undo_depth(&app)), (2, depth), "nothing ran");
    assert_eq!(app.ui.status, "", "no command ran to fail");
    // One object: Make stays dimmed.
    let first = top(&app)[0].0;
    app.run("select.set", json!({"ids": [first.0]})).unwrap();
    f.click_text(&mut app, "Make");
    assert_eq!((top(&app).len(), undo_depth(&app), app.ui.status.as_str()), (2, depth, ""), "Make did not run on one object");
    // Two objects: Orientation sets the options Make then uses.
    app.run("select.all", json!({})).unwrap();
    f.click(&mut app, path);
    assert_eq!(app.session.execute("object.blend.info", &json!({})).unwrap()["orientation"], "path");
    f.click(&mut app, page);
    assert_eq!(app.session.execute("object.blend.info", &json!({})).unwrap()["orientation"], "page");
}

#[test]
fn make_blends_the_selected_objects_in_one_undo_step() {
    let mut app = app();
    let depth = undo_depth(&app);
    let mut f = Frames::new();
    f.click_text(&mut app, "Make");
    let objects = top(&app);
    assert_eq!(objects.len(), 1, "{objects:?}");
    assert!(objects[0].1, "a blend");
    assert_eq!(undo_depth(&app), depth + 1);
}

#[test]
fn spacing_and_orientation_edit_the_selected_blend() {
    let (mut app, id) = blended();
    let mut f = Frames::new();
    // The dropdown shows the blend's Smooth Color; Specified Steps gives it the steps shown.
    f.click_text(&mut app, "Smooth Color");
    let depth = undo_depth(&app);
    f.click_text(&mut app, "Specified Steps");
    assert_eq!(spec(&app, id).spacing, BlendSpacing::Steps(5));
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    f.click_text(&mut app, "Specified Steps");
    f.click_text(&mut app, "Specified Distance");
    assert_eq!(spec(&app, id).spacing, BlendSpacing::Distance(10.0));
    let path = f.button(&mut app, "Align to Path");
    f.click(&mut app, path);
    assert_eq!(spec(&app, id).orientation, BlendOrientation::AlignToPath);
    assert_eq!(spec(&app, id).spacing, BlendSpacing::Distance(10.0), "Orientation keeps the spacing");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(spec(&app, id).orientation, BlendOrientation::AlignToPage);
}

#[test]
fn each_button_runs_its_blend_command() {
    // Reverse Front to Back reverses the keys' stacking order.
    let (mut app, id) = blended();
    let keys = |app: &VectorcraftApp| -> Vec<NodeId> {
        app.session.active().unwrap().doc.node(id).unwrap().children().unwrap().iter().map(|k| k.id).collect()
    };
    let before = keys(&app);
    let mut f = Frames::new();
    f.click_text(&mut app, "Reverse Front to Back");
    assert_eq!(keys(&app), before.iter().rev().copied().collect::<Vec<_>>());
    // Reverse Spine swaps the keys' places along the spine.
    let x = |app: &VectorcraftApp, k: NodeId| app.session.active().unwrap().doc.node(k).unwrap().geometric_bounds().unwrap().x0;
    let (a, b) = (keys(&app)[0], keys(&app)[1]);
    let (xa, xb) = (x(&app, a), x(&app, b));
    f.click_text(&mut app, "Reverse Spine");
    assert_eq!((x(&app, a), x(&app, b)), (xb, xa));
    // Replace Spine puts the blend on the selected path, which it takes in.
    let line = app.run("shape.line", json!({"x1": 0, "y1": 100, "x2": 200, "y2": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id.0, line]})).unwrap();
    f.click_text(&mut app, "Replace Spine");
    assert!(spec(&app, id).spine.is_some(), "the line is the spine");
    assert!(app.session.active().unwrap().doc.node(NodeId(line)).is_none(), "the line was taken in");
    // Expand turns the blend into a group of its keys and steps.
    f.click_text(&mut app, "Expand");
    assert!(matches!(app.session.active().unwrap().doc.node(id).unwrap().kind, NodeKind::Group { .. }));
    // Release gives back the keys of another blend.
    let (mut app, _) = blended();
    f.click_text(&mut app, "Release");
    let objects = top(&app);
    assert!(objects.len() >= 2 && objects.iter().all(|(_, blend)| !blend), "the keys are back: {objects:?}");
}

#[test]
fn panel_easing_type_ramp_and_shift_edit_the_blend_and_undo() {
    // Type and Ramp need a spine: Align to Path gives the blend one.
    let mut app = app();
    let id = NodeId(app.run("object.blend.make", json!({"smooth": true, "orientation": "path"})).unwrap()["id"].as_u64().unwrap());
    let mut f = Frames::new();
    for label in ["Type:", "Ease In", "Ramp:", "Shift:"] {
        f.text(&mut app, label);
    }
    f.click_text(&mut app, "Ease In");
    f.click_text(&mut app, "Ease Out In");
    assert_eq!(spec(&app, id).ease.kind, vectorcraft_doc::BlendEaseType::OutIn);
    // Click each real slider: its command updates the engine, and Undo restores the old value.
    for (index, key) in [(0, "ramp"), (1, "colorShift")] {
        let out = f.run(&mut app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
        let mut sliders: Vec<_> = nodes.iter().filter(|(_, n)| n.role() == egui::accesskit::Role::Slider).collect();
        // The tree is not in layout order: Ramp sits above Shift.
        sliders.sort_by(|a, b| a.1.bounds().unwrap().y0.total_cmp(&b.1.bounds().unwrap().y0));
        let slider = sliders[index];
        let b = slider.1.bounds().unwrap();
        let at = pos2((b.x0 + 0.3 * (b.x1 - b.x0)) as f32, ((b.y0 + b.y1) * 0.5) as f32);
        let before = app.session.execute("object.blend.info", &json!({})).unwrap()[key].clone();
        f.click(&mut app, at);
        let after = app.session.execute("object.blend.info", &json!({})).unwrap()[key].clone();
        assert_ne!(after, before, "{key}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.execute("object.blend.info", &json!({})).unwrap()[key], before);
    }
    app.run("ui.blendOptions", json!({})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(!text.contains("Ease In:") && !text.contains("Ramp:") && !text.contains("Color Ease"), "{text}");
}
