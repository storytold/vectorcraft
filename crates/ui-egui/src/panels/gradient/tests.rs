use std::cell::{RefCell, RefMut};
use std::rc::Rc;

use craft::ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, Pos2, pos2, vec2};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::*;

#[test]
fn dropped_colours_recolour_or_add_an_opaque_stop() {
    let mut g = Gradient::default();
    g.stops[1].opacity = 0.5;
    let red = Color::rgb(1.0, 0.0, 0.0);
    let (v, i) = drop_color(&g, Some(1), 0.3, red, None);
    assert_eq!((v.len(), i, v[1].color, v[1].opacity), (2, 1, red, 1.0));
    let (v, i) = drop_color(&g, None, 0.3, red, None);
    assert_eq!((v.len(), i, v[1].color, v[1].offset, v[1].opacity), (3, 1, red, 0.3, 1.0));
}

/// In a CMYK document an RGB swatch's color applied to the selected stop or freeform point
/// keeps its model, as the swatch applied to a fill does.
#[test]
fn a_swatch_color_on_a_stop_or_a_point_keeps_its_model_in_a_cmyk_document() {
    let stops = json!([{"offset": 0, "color": {"c": 0, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 0, "y": 0, "k": 1}}]);
    let mut app = app(json!({"gradient": {"stops": stops}}));
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let orange = Color::rgb8(255, 128, 0);
    app.run("swatch.new", json!({"name": "Screen", "color": "#ff8000", "keepModel": true})).unwrap();
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    assert!(recolor_selected(&mut app, orange, &json!({"swatch": "Screen"})));
    let Paint::Gradient(g) = active_paint(&app) else { panic!("a gradient fill") };
    assert_eq!(g.gradient.stops[1].color, orange);
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    app.run("paint.freeform.selectPoint", json!({"index": 1})).unwrap();
    assert!(recolor_selected(&mut app, orange, &json!({"swatch": "Screen"})));
    let Paint::Gradient(g) = active_paint(&app) else { panic!("a gradient fill") };
    assert_eq!(g.freeform.map(|f| f.points[1].color), Some(orange));
}

/// In a CMYK document Add to Swatches and Save to Swatches keep the gradient's RGB stops RGB.
#[test]
fn a_gradient_saved_as_a_swatch_keeps_its_stops_models_in_a_cmyk_document() {
    let mut app = app(json!({"color": "#000000"}));
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ff8000"}, {"offset": 1, "color": {"c": 0, "m": 0, "y": 0, "k": 1}}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}, "keepModel": true})).unwrap();
    let g = current(&app).expect("a gradient fill");
    let colors = |g: &GradientPaint| g.gradient.stops.iter().map(|s| s.color).collect::<Vec<_>>();
    assert_eq!(colors(&g), [Color::rgb8(255, 128, 0), Color::cmyk(0.0, 0.0, 0.0, 1.0)]);
    save_to_swatches(&mut app, &g);
    let Some(Paint::Gradient(saved)) = app.session.doc().unwrap().doc.swatches.last().map(|w| w.paint.clone()) else { panic!("a gradient swatch") };
    assert_eq!(colors(&saved), colors(&g));
}

// ---------- framework fixture ----------

/// A selected rectangle filled with `fill` (`paint.setFill` params).
fn app(fill: Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("paint.setFill", fill).unwrap();
    app
}

/// The Gradient panel in a framework fixture over one app: the trials drive it through its
/// controls' accessible names and read the app. Each `app()` borrow ends before the next drive.
struct Rig {
    app: Rc<RefCell<VectorcraftApp>>,
    fixture: Fixture,
}

impl Rig {
    /// The panel alone.
    fn new(app: VectorcraftApp) -> Self {
        Self::with(app, show)
    }

    /// `draw` (the panel, or the panel with others) over `app`.
    fn with(app: VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = Rc::clone(&app);
        let mut fixture = Fixture::new(move |ui| draw(&mut shown.borrow_mut(), ui));
        fixture.room(vec2(320.0, 1200.0)).settle();
        Rig { app, fixture }
    }

    fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// The control named `label`.
    fn target(&self, label: &str) -> Target {
        self.fixture.by_label(label).unwrap_or_else(|| panic!("no `{label}` in the panel"))
    }

    /// Where the control named `label` is.
    fn at(&self, label: &str) -> Pos2 {
        self.target(label).center()
    }

    /// Where the Swatches panel's tile of swatch `name` is: a tile is named by its swatch and the colour it shows
    /// ("Drop Red, #ff0000").
    fn swatch(&self, name: &str) -> Pos2 {
        let tree = self.fixture.tree();
        let prefix = format!("{name}, ");
        let full = tree.keys().into_iter().filter_map(|k| tree.name_of(k)).find(|n| n.starts_with(&prefix)).map(str::to_owned);
        self.at(&full.unwrap_or_else(|| panic!("no swatch tile `{name}`")))
    }

    /// The number the field named `label` shows.
    fn number(&self, label: &str) -> Option<f64> {
        self.fixture.tree().numeric_of(self.target(label).node).and_then(|n| n.value)
    }

    /// Whether the control named `label` is disabled.
    fn disabled(&self, label: &str) -> bool {
        self.fixture.tree().is_disabled_key(self.target(label).node)
    }

    /// Click the control named `label`.
    fn click(&mut self, label: &str) {
        let target = self.target(label);
        self.fixture.click(&target).settle();
    }

    /// Cmd/Ctrl-click the control named `label`.
    fn cmd_click(&mut self, label: &str) {
        let target = self.target(label);
        self.fixture.hold(Modifiers::COMMAND).click(&target);
        self.fixture.hold(Modifiers::NONE).settle();
    }

    /// Type `text` over the text of the field named `label` and commit it with Enter.
    fn type_into(&mut self, label: &str, text: &str) {
        let field = self.target(label);
        self.fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    }

    /// Drag from `from` and release at `to`.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.fixture.drag_at(from, to - from).settle();
    }
}

fn fill(app: &VectorcraftApp) -> GradientPaint {
    match crate::panels::current_paints(app).0 {
        Paint::Gradient(g) => *g,
        p => panic!("expected a gradient fill, got {p:?}"),
    }
}

fn hexes(app: &VectorcraftApp) -> Vec<String> {
    fill(app).gradient.stops.iter().map(|s| s.color.to_hex()).collect()
}

/// A diamond clicked on the slider takes the selection from the stop, shows its midpoint in
/// Location, and a location typed there moves the midpoint; a stop clicked takes it back.
#[test]
fn a_selected_diamond_shows_and_sets_its_midpoint_in_location() {
    let stops = json!([{"offset": 0, "color": "#ffffff", "midpoint": 0.3}, {"offset": 1, "color": "#000000"}]);
    let mut app = app(json!({"gradient": {"stops": stops}}));
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let mut rig = Rig::new(app);
    assert_eq!(rig.number("Location"), Some(100.0), "the stop's location");
    rig.click("Midpoint 1");
    assert_eq!(rig.app().session.selected_stop(), None, "the diamond replaces the stop selection");
    assert_eq!(rig.app().ui.gradient_panel.mid_selected.map(|(i, _)| i), Some(0));
    assert_eq!(rig.number("Location"), Some(30.0), "the midpoint");
    rig.type_into("Location", "60");
    let stops = fill(&rig.app()).gradient.stops;
    assert!((stops[0].midpoint - 0.6).abs() < 1e-6, "{stops:?}");
    rig.click("Color stop 1 of 2");
    assert_eq!(rig.app().session.selected_stop(), Some(0));
    assert_eq!(rig.number("Location"), Some(0.0), "the stop's location again");
}

/// A diamond picked on the selected object's gradient is not selected on another gradient the
/// panel shows with that object still selected: the Live Paint Bucket's, or the stroke's after the
/// proxy toggles. Going back to the object's fill selects it again.
#[test]
fn a_picked_diamond_stays_with_the_gradient_it_was_picked_on() {
    let stops = json!([{"offset": 0, "color": "#ffffff", "midpoint": 0.3}, {"offset": 1, "color": "#000000"}]);
    let mut app = app(json!({"gradient": {"stops": stops}}));
    app.run("paint.setStroke", json!({"gradient": {"kind": "radial"}})).unwrap();
    app.session.fill_active = true;
    let mut rig = Rig::new(app);
    rig.click("Midpoint 1");
    let mid = |rig: &Rig| selected_mid(&rig.app(), 2);
    assert_eq!(mid(&rig), Some(0), "the diamond is picked");
    {
        let mut app = rig.app();
        app.select_tool("livePaintBucket");
        app.run("paint.setFill", json!({"gradient": {"kind": "radial"}})).unwrap();
    }
    rig.fixture.settle();
    assert_eq!(mid(&rig), None, "the bucket's gradient");
    rig.app().select_tool("selection");
    rig.fixture.settle();
    assert_eq!(mid(&rig), Some(0), "the object's fill again");
    rig.app().session.fill_active = false;
    rig.fixture.settle();
    assert_eq!(mid(&rig), None, "the object's stroke");
}

/// A mirrored conic's Aspect Ratio field shows the extent's size, and a typed value keeps the
/// sweep mirrored (the field's value times the mirror's sign).
#[test]
fn the_aspect_field_keeps_a_mirrored_conic_mirrored() {
    let mut rig = Rig::new(app(json!({"gradient": {"kind": "conic", "aspect": -50}})));
    assert_eq!(rig.number("Aspect Ratio"), Some(50.0), "the size shows");
    rig.type_into("Aspect Ratio", "80");
    let geom = fill(&rig.app()).geom.unwrap();
    assert!((geom.aspect + 0.8).abs() < 1e-9 && geom.is_mirrored(), "{geom:?}");
}

/// With nothing selected, and while the Live Paint Bucket is active, the Aspect Ratio field is
/// disabled: the default gradient fits each object and has no aspect. A centred gradient on a
/// selected object takes one.
#[test]
fn the_aspect_field_is_dimmed_for_the_default_gradient() {
    for bucket in [false, true] {
        let mut app = app(json!({"gradient": {"kind": "radial"}}));
        if bucket {
            app.select_tool("livePaintBucket");
        } else {
            app.run("select.set", json!({"ids": []})).unwrap();
        }
        let rig = Rig::new(app);
        assert!(rig.disabled("Aspect Ratio"), "bucket {bucket}: the default gradient has no aspect");
    }
    let rig = Rig::new(app(json!({"gradient": {"kind": "radial"}})));
    assert!(!rig.disabled("Aspect Ratio"), "a radial gradient on the selected rectangle");
}

/// Cmd/Ctrl-click on the Gradient Fill box gives an unpainted stroke (no paint, no weight) the
/// White, Black gradient with the 1 pt weight, so the gradient shows.
#[test]
fn cmd_click_on_the_gradient_fill_box_gives_an_unpainted_stroke_its_weight() {
    let mut app = app(json!({"color": "#336699"}));
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    let stroke = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        n.appearance.stroke().map(|s| (s.paint.clone(), s.width)).unwrap()
    };
    assert_eq!(stroke(&app).1, 0.0);
    app.session.fill_active = false;
    let mut rig = Rig::new(app);
    rig.cmd_click("Gradient Fill");
    let (paint, width) = stroke(&rig.app());
    let Paint::Gradient(g) = paint else { panic!("expected a gradient stroke, got {paint:?}") };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(width, 1.0, "the stroke gets the 1 pt default");
}

/// With characters selected with the Type tool, Cmd/Ctrl-click on the Gradient Fill box gives
/// only those characters the White, Black gradient, as one undo step.
#[test]
fn cmd_click_on_the_gradient_fill_box_resets_only_the_selected_characters() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Sample Text", "size": 24})).unwrap()["id"].clone();
    let id = NodeId(id.as_u64().unwrap());
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 6}));
    let runs = |app: &VectorcraftApp| match app.session.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(vectorcraft_doc::NodeKind::Text(t)) => t.runs.iter().map(|r| (r.text.clone(), r.style.fill.clone())).collect::<Vec<_>>(),
        k => panic!("expected type, got {k:?}"),
    };
    let before = runs(&app)[0].1.clone();
    let mut rig = Rig::new(app);
    rig.cmd_click("Gradient Fill");
    let after = runs(&rig.app());
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].0, "Sample");
    let Paint::Gradient(g) = &after[0].1 else { panic!("expected a gradient fill, got {:?}", after[0].1) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(g.gradient.stops.iter().map(|s| s.color.to_hex()).collect::<Vec<_>>(), ["#ffffff", "#000000"]);
    assert_eq!(after[1], (" Text".to_string(), before.clone()), "the rest keeps its fill");
    rig.app().run("edit.undo", json!({})).unwrap();
    assert!(runs(&rig.app()).iter().all(|(_, f)| *f == before), "one undo step");
}

// PLAN: assumed color-w10's Swatches panel: each swatch tile is named by its swatch's name and drags a `PanelDrag::Paint` with `params {swatch}` (wg.S7).
/// The Swatches panel over the Gradient panel: a swatch tile dragged onto the slider is a real drop.
fn with_swatches(app: &mut VectorcraftApp, ui: &mut Ui) {
    Stack::new().show(ui, |ui, _| {
        super::super::swatches::show(app, ui);
        show(app, ui);
    });
}

/// A swatch tile dropped on the slider between two stops adds a stop of its colour; dropped on a
/// stop it recolours it. A spot swatch dropped on a stop links it: editing the swatch recolours
/// the stop.
#[test]
fn a_dropped_swatch_adds_or_recolours_a_stop() {
    let mut app = app(json!({"gradient": {}}));
    app.run("swatch.new", json!({"name": "Drop Red", "color": "#ff0000"})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
    let mut rig = Rig::with(app, with_swatches);
    let (a, b) = (rig.at("Color stop 1 of 2"), rig.at("Color stop 2 of 2"));
    rig.drag(rig.swatch("Drop Red"), pos2((a.x + b.x) / 2.0, a.y));
    assert_eq!(hexes(&rig.app()), ["#ffffff", "#ff0000", "#000000"]);
    rig.drag(rig.swatch("Drop Red"), rig.at("Color stop 3 of 3"));
    assert_eq!(hexes(&rig.app()), ["#ffffff", "#ff0000", "#ff0000"]);
    rig.drag(rig.swatch("Ink"), rig.at("Color stop 1 of 3"));
    let st = fill(&rig.app()).gradient.stops[0].clone();
    assert_eq!((st.swatch.as_deref(), st.tint, st.color.to_hex()), (Some("Ink"), 1.0, "#cc0066".to_string()));
    rig.app().run("swatch.edit", json!({"name": "Ink", "color": "#0066cc"})).unwrap();
    assert_eq!(hexes(&rig.app())[0], "#0066cc");
}

/// In a CMYK document a colour dropped on the slider of an RGB gradient keeps its model, as the
/// gradient's other stops do.
#[test]
fn a_color_dropped_on_the_ramp_keeps_its_model_in_a_cmyk_document() {
    let mut app = app(json!({"gradient": {}}));
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let rgb = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": rgb}, "keepModel": true})).unwrap();
    app.run("swatch.new", json!({"name": "Drop Red", "color": "#ff0000", "keepModel": true})).unwrap();
    let mut rig = Rig::with(app, with_swatches);
    let (a, b) = (rig.at("Color stop 1 of 2"), rig.at("Color stop 2 of 2"));
    rig.drag(rig.swatch("Drop Red"), pos2((a.x + b.x) / 2.0, a.y));
    let stops: Vec<Color> = fill(&rig.app()).gradient.stops.iter().map(|s| s.color).collect();
    assert_eq!(stops, [Color::WHITE, Color::rgb(1.0, 0.0, 0.0), Color::BLACK]);
}

#[test]
fn the_stop_eyedropper_arms_the_eyedropper_for_the_stop() {
    let mut app = app(json!({"gradient": {}}));
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let rig = Rig::new(app);
    assert!(rig.fixture.by_label("#FFFFFF").is_some(), "the stop's colour row");
    stop_eyedropper(&mut rig.app());
    {
        let app = rig.app();
        assert_eq!((app.session.tool_id(), app.session.tool_options()), ("eyedropper", json!({"stop": "gradient", "brushKeyColor": false})));
    }
    // A second click on the eyedropper while the pick is armed still hands back to the Gradient tool.
    stop_eyedropper(&mut rig.app());
    assert_eq!(rig.app().session.tool_options()["stop"], "gradient");
}

/// #8024: Escape ends the stop eyedropper's pick and returns to the Gradient tool. The stop keeps
/// its color and the selection stays.
#[test]
fn escape_ends_the_stop_eyedropper_and_returns_to_the_gradient_tool() {
    let mut app = app(json!({"gradient": {}}));
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let (paint, selected) = (current(&app), app.session.active().unwrap().selection.objects.clone());
    stop_eyedropper(&mut app);
    let ctx = crate::testing::ui_context();
    let escape = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![escape]);
    assert_eq!(app.session.tool_id(), "gradient");
    assert_eq!(current(&app), paint, "the stop keeps its color");
    assert_eq!(app.session.active().unwrap().selection.objects, selected, "the selection stays");
}
