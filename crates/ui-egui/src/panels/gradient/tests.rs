use egui::{Event, Modifiers, PointerButton};
use vectorcraft_engine::Session;

use super::*;

#[test]
fn offsets_and_json() {
    assert_eq!(x_to_offset(50.0, 0.0, 200.0), 0.25);
    assert_eq!(x_to_offset(-5.0, 0.0, 200.0), 0.0);
    assert_eq!(x_to_offset(500.0, 0.0, 200.0), 1.0);
    let j = stops_json(&Gradient::default().stops);
    assert_eq!(j.as_array().unwrap().len(), 2);
    assert_eq!(j[1]["midpoint"], json!(0.5));
}

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

#[test]
fn dragged_stops_move_copy_or_swap() {
    let g = Gradient::default();
    let (v, i) = dragged_stops(&g.stops, 0, 0.4, false, Some(1));
    assert_eq!((v.len(), i, v[0].offset), (2, 0, 0.4));
    let (v, i) = dragged_stops(&g.stops, 0, 0.4, true, None);
    assert_eq!((v.len(), i, v[1].offset, v[1].color), (3, 1, 0.4, Color::WHITE));
    let (v, i) = dragged_stops(&g.stops, 0, 1.0, true, Some(1));
    assert_eq!((v.len(), i, v[0].color, v[1].color), (2, 0, Color::BLACK, Color::WHITE));
    // Dropped on itself: a copy.
    assert_eq!(dragged_stops(&g.stops, 0, 0.0, true, Some(0)).0.len(), 3);
}

// ---------- headless frames ----------

/// A selected rectangle filled with `fill` (`paint.setFill` params).
fn app(fill: Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("paint.setFill", fill).unwrap();
    app
}

/// A headless Gradient panel: frames share one context and a clock.
struct Panel {
    ctx: egui::Context,
    time: f64,
}

impl Panel {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0 }
    }

    /// One frame with `events` (and `modifiers` held); returns the texts drawn and where.
    fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<Event>, modifiers: Modifiers) -> Vec<(String, Pos2)> {
        fn texts(s: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        self.time += 0.1;
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            time: Some(self.time),
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(260.0, 600.0))),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    /// The rect of the widget keyed `key` (last frame). egui 0.37's `WidgetRect.parent_id` is
    /// the parent Ui's unstable `unique_id`, not the stable `scope_id` widgets are salted with;
    /// debug builds remember the `with(key)` that made each id, so match that.
    fn widget(&self, key: impl std::hash::Hash + std::fmt::Debug + Copy) -> Rect {
        let suffix = format!(".with({key:?})");
        self.ctx
            .viewport(|vp| {
                vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| format!("{:?}", w.id).ends_with(&suffix)).map(|w| w.rect)
            })
            .expect("widget drawn")
    }

    fn click(&mut self, app: &mut VectorcraftApp, at: Pos2) -> Vec<(String, Pos2)> {
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at), b(true)], Modifiers::NONE);
        self.frame(app, vec![b(false)], Modifiers::NONE);
        self.frame(app, vec![], Modifiers::NONE)
    }

    /// The `size`-square widgets right of `at` on its row (last frame), left to right.
    fn row_after(&self, at: Pos2, size: f32) -> Vec<Rect> {
        let mut v: Vec<Rect> = self.ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .map(|w| w.rect)
                .filter(|r| (r.width() - size).abs() < 0.5 && (r.height() - size).abs() < 0.5 && (r.center().y - at.y).abs() < 8.0 && r.left() > at.x)
                .collect()
        });
        v.sort_by(|a, b| a.left().total_cmp(&b.left()));
        v.dedup();
        v
    }

    /// Press at `from`, move to `to` in steps and release there, with `m` held.
    fn drag(&mut self, app: &mut VectorcraftApp, from: Pos2, to: Pos2, m: Modifiers) {
        let b = |at, pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: m };
        self.frame(app, vec![Event::PointerMoved(from), b(from, true)], m);
        for k in 1..=4 {
            self.frame(app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))], m);
        }
        self.frame(app, vec![b(to, false)], m);
        self.frame(app, vec![], Modifiers::NONE);
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

#[test]
fn the_proxy_toggles_fill_and_stroke_and_stays_with_options_hidden() {
    let mut app = app(json!({"gradient": {}}));
    let mut p = Panel::new();
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    assert!(texts.iter().any(|(t, _)| t == "Opacity:"));
    let stroke = p.widget("stroke-proxy");
    p.click(&mut app, stroke.right_bottom() - vec2(2.0, 2.0));
    assert!(!app.session.fill_active, "the Stroke proxy came to the front");
    set_pstate(&p.ctx, HIDE_OPTIONS, true);
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    assert!(!texts.iter().any(|(t, _)| t == "Opacity:" || t == "Type:"), "{texts:?}");
    let fill = p.widget("fill-proxy");
    p.click(&mut app, fill.left_top() + vec2(2.0, 2.0));
    assert!(app.session.fill_active, "the proxy works with the options hidden");
    p.widget(("grad-stop", 1));
}

#[test]
fn the_stroke_buttons_lay_the_gradient_within_along_or_across_the_stroke() {
    let mut app = app(json!({"gradient": {}}));
    app.run("paint.setStroke", json!({"gradient": {}, "focus": false})).unwrap();
    assert!(app.session.fill_active);
    let mode = |app: &VectorcraftApp| app.session.shown_stroke().unwrap().gradient_mode;
    let mut p = Panel::new();
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    let label = texts.iter().find(|(t, _)| t == "Stroke:").expect("the Stroke buttons").1;
    // With the Fill proxy in front they are off.
    let buttons = p.row_after(label, 22.0);
    assert_eq!(buttons.len(), 3);
    p.click(&mut app, buttons[1].center());
    assert_eq!(mode(&app), StrokeGradientMode::Within);
    let stroke = p.widget("stroke-proxy");
    p.click(&mut app, stroke.right_bottom() - vec2(2.0, 2.0));
    assert!(!app.session.fill_active);
    for (i, want) in [(1, StrokeGradientMode::Along), (2, StrokeGradientMode::Across), (0, StrokeGradientMode::Within)] {
        p.click(&mut app, buttons[i].center());
        assert_eq!(mode(&app), want);
    }
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(mode(&app), StrokeGradientMode::Across, "one undo step per click");
}

#[test]
fn the_dither_box_dithers_the_gradient_fill_and_is_off_for_strokes() {
    let mut app = app(json!({"gradient": {}}));
    app.run("paint.setStroke", json!({"gradient": {}, "focus": false})).unwrap();
    let mut p = Panel::new();
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    let at = texts.iter().find(|(t, _)| t == "Dither").expect("the Dither box").1;
    p.click(&mut app, at);
    assert!(fill(&app).gradient.dither, "the fill's gradient is dithered");
    p.click(&mut app, at);
    assert!(!fill(&app).gradient.dither, "a second click clears it");
    app.run("edit.undo", json!({})).unwrap();
    assert!(fill(&app).gradient.dither, "one undo step per click");
    // Behind the Stroke proxy the box is dimmed and does nothing.
    let stroke = p.widget("stroke-proxy");
    p.click(&mut app, stroke.right_bottom() - vec2(2.0, 2.0));
    assert!(!app.session.fill_active);
    let undo = app.session.doc().unwrap().history.undo.len();
    p.click(&mut app, at);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "no edit on the stroke");
    let Paint::Gradient(st) = app.session.shown_stroke().unwrap().paint.clone() else { panic!("a gradient stroke") };
    assert!(!st.gradient.dither);
}

#[test]
fn the_method_menu_makes_the_gradient_perceptual() {
    let mut app = app(json!({"gradient": {}}));
    let mut p = Panel::new();
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    assert!(texts.iter().any(|(t, _)| t == "Method:"), "{texts:?}");
    let shown = texts.iter().find(|(t, _)| t == "Classic").expect("Classic shown").1;
    let undo = app.session.doc().unwrap().history.undo.len();
    let texts = p.click(&mut app, shown);
    let option = texts.iter().find(|(t, _)| t == "Perceptual").expect("the menu lists Perceptual").1;
    p.click(&mut app, option);
    assert_eq!(fill(&app).gradient.method, GradientMethod::Perceptual);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    assert!(texts.iter().any(|(t, _)| t == "Perceptual"), "the menu shows the method: {texts:?}");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(fill(&app).gradient.method, GradientMethod::Classic);
}

#[test]
fn a_selected_diamond_shows_and_sets_its_midpoint_in_location() {
    let stops = json!([{"offset": 0, "color": "#ffffff", "midpoint": 0.3}, {"offset": 1, "color": "#000000"}]);
    let mut app = app(json!({"gradient": {"stops": stops}}));
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let mut p = Panel::new();
    let texts = p.frame(&mut app, vec![], Modifiers::NONE);
    assert!(texts.iter().any(|(t, _)| t == "100%"), "the stop's location: {texts:?}");
    let diamond = p.widget(("grad-mid", 0));
    let texts = p.click(&mut app, diamond.center());
    assert_eq!(app.session.selected_stop(), None, "the diamond replaces the stop selection");
    assert!(texts.iter().any(|(t, _)| t == "30%"), "the midpoint: {texts:?}");
    // Typing a location moves the midpoint.
    let field = texts.iter().find(|(t, _)| t == "30%").unwrap().1;
    p.click(&mut app, field);
    let key = |k, modifiers| Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers };
    p.frame(&mut app, vec![key(egui::Key::A, Modifiers::COMMAND), Event::Text("60".into())], Modifiers::NONE);
    p.frame(&mut app, vec![key(egui::Key::Enter, Modifiers::NONE)], Modifiers::NONE);
    assert!((fill(&app).gradient.stops[0].midpoint - 0.6).abs() < 1e-6, "{:?}", fill(&app).gradient.stops);
    // Clicking a stop selects it again.
    let stop = p.widget(("grad-stop", 0));
    let texts = p.click(&mut app, stop.center());
    assert_eq!(app.session.selected_stop(), Some(0));
    assert!(texts.iter().any(|(t, _)| t == "0%"));
}

#[test]
fn the_thumbnail_menu_applies_gradient_swatches_and_saves_one() {
    let mut app = app(json!({"color": "#336699"}));
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("swatch.new", json!({"name": "Dusk", "gradient": {"stops": stops}})).unwrap();
    let mut p = Panel::new();
    p.frame(&mut app, vec![], Modifiers::NONE);
    let menu = p.widget("grad-swatches").center();
    let texts = p.click(&mut app, menu);
    let dusk = texts.iter().find(|(t, _)| t == "Dusk").unwrap_or_else(|| panic!("the menu lists the swatch: {texts:?}")).1;
    assert!(texts.iter().any(|(t, _)| t.ends_with("Save to Swatches")));
    p.click(&mut app, dusk);
    assert_eq!(hexes(&app), ["#ff0000", "#0000ff"]);
    // Save to Swatches adds the current gradient.
    let before = gradient_swatches(&app).len();
    let texts = p.click(&mut app, menu);
    let save = texts.iter().find(|(t, _)| t.ends_with("Save to Swatches")).unwrap().1;
    p.click(&mut app, save);
    assert_eq!(gradient_swatches(&app).len(), before + 1);
}

#[test]
fn alt_drag_copies_a_stop_and_alt_dropping_on_a_stop_swaps_colours() {
    let mut app = app(json!({"gradient": {}}));
    let mut p = Panel::new();
    p.frame(&mut app, vec![], Modifiers::NONE);
    let (a, b) = (p.widget(("grad-stop", 0)).center(), p.widget(("grad-stop", 1)).center());
    let mid = pos2((a.x + b.x) / 2.0, a.y);
    let offsets = |app: &VectorcraftApp| fill(app).gradient.stops.iter().map(|s| (s.offset * 10.0).round() / 10.0).collect::<Vec<_>>();
    // A plain drag moves the stop.
    p.drag(&mut app, a, mid, Modifiers::NONE);
    assert_eq!(offsets(&app), [0.5, 1.0]);
    app.run("edit.undo", json!({})).unwrap();
    // Alt leaves the original and drags a copy, selected on release; one undo step.
    p.frame(&mut app, vec![], Modifiers::NONE);
    p.drag(&mut app, a, mid, Modifiers::ALT);
    assert_eq!((offsets(&app), hexes(&app)), (vec![0.0, 0.5, 1.0], vec!["#ffffff".to_string(), "#ffffff".into(), "#000000".into()]));
    assert_eq!(app.session.selected_stop(), Some(1));
    // Alt-dropping the first stop on the last swaps their colours.
    p.drag(&mut app, a, b, Modifiers::ALT);
    assert_eq!((offsets(&app), hexes(&app)), (vec![0.0, 0.5, 1.0], vec!["#000000".to_string(), "#ffffff".into(), "#ffffff".into()]));
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(hexes(&app), ["#ffffff", "#000000"]);
}

#[test]
fn a_dropped_swatch_adds_or_recolours_a_stop() {
    let mut app = app(json!({"gradient": {}}));
    let mut p = Panel::new();
    p.frame(&mut app, vec![], Modifiers::NONE);
    let (a, b) = (p.widget(("grad-stop", 0)).center(), p.widget(("grad-stop", 1)).center());
    let drop = |p: &mut Panel, app: &mut VectorcraftApp, at: Pos2| {
        egui::DragAndDrop::set_payload(&p.ctx, PanelDrag::paint(Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
        p.frame(app, vec![Event::PointerMoved(at)], Modifiers::NONE);
        let up = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        p.frame(app, vec![up], Modifiers::NONE);
    };
    drop(&mut p, &mut app, pos2((a.x + b.x) / 2.0, a.y - 20.0));
    assert_eq!(hexes(&app), ["#ffffff", "#ff0000", "#000000"]);
    p.frame(&mut app, vec![], Modifiers::NONE);
    let last = p.widget(("grad-stop", 2)).center();
    drop(&mut p, &mut app, last);
    assert_eq!(hexes(&app), ["#ffffff", "#ff0000", "#ff0000"]);
    // A spot swatch tile dropped on a stop links it: editing the swatch recolours the stop.
    app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
    let ink = app.session.doc().unwrap().doc.swatch("Ink").unwrap().paint.clone();
    egui::DragAndDrop::set_payload(&p.ctx, PanelDrag::Paint { paint: ink, params: json!({"swatch": "Ink"}), rows: None, library: None });
    p.frame(&mut app, vec![Event::PointerMoved(a)], Modifiers::NONE);
    p.frame(
        &mut app,
        vec![Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
        Modifiers::NONE,
    );
    let st = fill(&app).gradient.stops[0].clone();
    assert_eq!((st.swatch.as_deref(), st.tint, st.color.to_hex()), (Some("Ink"), 1.0, "#cc0066".to_string()));
    app.run("swatch.edit", json!({"name": "Ink", "color": "#0066cc"})).unwrap();
    assert_eq!(hexes(&app)[0], "#0066cc");
}

/// In a CMYK document a color dropped on the ramp of an RGB gradient keeps its model, as the
/// gradient's other stops do.
#[test]
fn a_color_dropped_on_the_ramp_keeps_its_model_in_a_cmyk_document() {
    let mut app = app(json!({"gradient": {}}));
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let rgb = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": rgb}, "keepModel": true})).unwrap();
    let mut p = Panel::new();
    p.frame(&mut app, vec![], Modifiers::NONE);
    let (a, b) = (p.widget(("grad-stop", 0)).center(), p.widget(("grad-stop", 1)).center());
    let at = pos2((a.x + b.x) / 2.0, a.y - 20.0);
    let red = Color::rgb(1.0, 0.0, 0.0);
    egui::DragAndDrop::set_payload(&p.ctx, PanelDrag::paint(Paint::solid(red)));
    p.frame(&mut app, vec![Event::PointerMoved(at)], Modifiers::NONE);
    let up = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
    p.frame(&mut app, vec![up], Modifiers::NONE);
    let stops: Vec<Color> = fill(&app).gradient.stops.iter().map(|s| s.color).collect();
    assert_eq!(stops, [Color::WHITE, red, Color::BLACK]);
}

#[test]
fn double_clicking_a_stop_opens_the_shared_popover_beside_it() {
    let mut app = app(json!({"gradient": {}}));
    let mut p = Panel::new();
    p.frame(&mut app, vec![], Modifiers::NONE);
    let stop = p.widget(("grad-stop", 1));
    let b = |pressed| Event::PointerButton { pos: stop.center(), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    p.frame(&mut app, vec![Event::PointerMoved(stop.center()), b(true), b(false), b(true), b(false)], Modifiers::NONE);
    let d = app.ui.dialog.clone().expect("the popover opened");
    assert_eq!((d.kind.as_str(), &d.fields["index"], &d.fields["screen"]), ("gradientStop", &json!(1), &json!([stop.left(), stop.bottom() + 4.0])));
    assert_eq!(app.session.selected_stop(), Some(1));
}

#[test]
fn the_stop_eyedropper_arms_the_eyedropper_for_the_stop() {
    let mut app = app(json!({"gradient": {}}));
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let texts = Panel::new().frame(&mut app, vec![], Modifiers::NONE);
    assert!(texts.iter().any(|(t, _)| t == "#FFFFFF"), "the stop's colour row: {texts:?}");
    stop_eyedropper(&mut app);
    assert_eq!((app.session.tool_id(), app.session.tool_options()), ("eyedropper", json!({"stop": "gradient", "brushKeyColor": false})));
}

/// The gradient panel names its Angle and Aspect Ratio inputs.
#[test]
fn angle_and_aspect_fields_are_named() {
    use egui_kittest::kittest::Queryable;
    let app = app(json!({"gradient": {}}));
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(280.0, 640.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui);
        },
        app,
    );
    h.run_steps(3);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Angle");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Aspect Ratio");
}
