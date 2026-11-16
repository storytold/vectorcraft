use super::*;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

/// Run the panel for one frame with `events` and `modifiers`; returns the spectrum's rect (when
/// the options are shown) and the panel's height.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> (Option<Rect>, f32) {
    frame_with(app, ctx, events, modifiers, "color-spectrum")
}

/// [`frame`] returning the rect of the panel's widget `id` instead of the spectrum's.
fn frame_with(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> (Option<Rect>, f32) {
    let mut out = (None, 0.0);
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let input = egui::RawInput { events, ..Default::default() };
    let mut full = ctx.run_ui(input, |ui| {
        show(app, ui);
        out = (ctx.read_response(ui.scope_id().with(&id)).map(|r| r.rect), ui.min_rect().height());
    });
    full.textures_delta.clear();
    out
}

/// Click the spectrum at (x, y) (0..1 each) with `modifiers` held.
fn click_spectrum(app: &mut VectorcraftApp, ctx: &egui::Context, (x, y): (f32, f32), modifiers: egui::Modifiers) {
    let r = frame(app, ctx, vec![], modifiers).0.expect("the spectrum is drawn");
    let pos = egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(app, ctx, vec![egui::Event::PointerMoved(pos), button(true)], modifiers);
    frame(app, ctx, vec![button(false)], modifiers);
    frame(app, ctx, vec![], egui::Modifiers::NONE);
}

#[test]
fn a_recent_colour_recolours_the_selected_gradient_stop() {
    // #835: it replaced the whole gradient with the colour.
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let red = app.session.recent_colors.iter().position(|c| c.to_hex() == "#ff0000").expect("red is a recent colour");
    let ctx = egui::Context::default();
    let none = egui::Modifiers::NONE;
    let chip = frame_with(&mut app, &ctx, vec![], none, ("recent", red)).0.expect("the chip is drawn").center();
    let button = |pressed| egui::Event::PointerButton { pos: chip, button: egui::PointerButton::Primary, pressed, modifiers: none };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(chip), button(true)], none);
    frame(&mut app, &ctx, vec![button(false)], none);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    let hex: Vec<String> = g.gradient.stops.iter().map(|s| s.color.to_hex()).collect();
    assert_eq!(hex, ["#ffffff", "#ff0000"]);
}

/// In a narrow panel the Recent Colors chips wrap onto further rows, so all 30 can be clicked.
#[test]
fn all_thirty_recent_colors_wrap_into_a_narrow_panel() {
    let mut app = app();
    let hex = |i: u8| Color::rgb8(i * 8, 0, 255 - i * 8).to_hex();
    for i in 0..30u8 {
        app.run("paint.setFill", json!({"color": hex(i)})).unwrap();
    }
    assert_eq!(app.session.recent_colors.len(), 30);
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(160.0, 2000.0));
    let chips = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut rects = vec![];
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            show(app, ui);
            rects = (0..30usize).map(|i| ctx.read_response(ui.scope_id().with(("recent", i))).map(|r| r.rect)).collect();
        });
        full.textures_delta.clear();
        rects
    };
    let rects: Vec<Rect> = chips(&mut app, vec![]).into_iter().map(|r| r.expect("every chip is drawn")).collect();
    for (i, r) in rects.iter().enumerate() {
        assert!(r.right() <= screen.right(), "chip {i} fits the panel's width: {r:?}");
        for (j, o) in rects.iter().enumerate().skip(i + 1) {
            assert!(!r.intersects(*o), "chips {i} and {j} do not overlap");
        }
    }
    assert!(rects[29].top() > rects[0].top(), "the chips wrap onto another row");
    // The last chip, the oldest color, applies like the others.
    let pos = rects[29].center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    chips(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
    chips(&mut app, vec![button(false)]);
    let Paint::Solid { color, .. } = crate::panels::current_paints(&app).0 else { panic!("a solid fill") };
    assert_eq!(color.to_hex(), hex(0));
}

/// With a few recent colors the chips take one row, as tall as before.
#[test]
fn a_short_recent_colors_list_takes_one_row() {
    let mut app = app();
    for h in ["#ff0000", "#00ff00", "#0000ff"] {
        app.run("paint.setFill", json!({"color": h})).unwrap();
    }
    let ctx = egui::Context::default();
    let first = frame_with(&mut app, &ctx, vec![], egui::Modifiers::NONE, ("recent", 0usize)).0.expect("the first chip is drawn");
    let last = frame_with(&mut app, &ctx, vec![], egui::Modifiers::NONE, ("recent", 2usize)).0.expect("the third chip is drawn");
    assert_eq!(first.top(), last.top());
    assert_eq!(last.left() - first.left(), 38.0);
}

/// In a CMYK document the sliders keep the model picked in the panel for a gradient stop and a
/// freeform point, as they do for a solid fill.
#[test]
fn a_gradient_stop_and_a_freeform_point_keep_the_panels_model_in_a_cmyk_document() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let stops = json!([{"offset": 0, "color": {"c": 0, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 100, "y": 100, "k": 0}}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let orange = Color::rgb8(255, 128, 0);
    let tgt = target_with(&app, false);
    apply(&mut app, &tgt, orange, Live::Released, false);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    assert_eq!(g.gradient.stops[1].color, orange);
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    let tgt = target_with(&app, false);
    assert!(matches!(tgt, Target::Point { index: 0, .. }), "a freeform point is the target");
    apply(&mut app, &tgt, orange, Live::Released, false);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    assert_eq!(g.freeform.map(|f| f.points[0].color), Some(orange));
}

/// One headless frame of the panel menu; returns its painted labels.
fn menu_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
    out.textures_delta.clear();
    crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

/// Click the panel menu's item `label`.
fn click_menu_item(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let labels = menu_frame(app, ctx, vec![]);
    let at = labels.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label} in the menu"));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    menu_frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
    menu_frame(app, ctx, vec![button(false)]);
}

/// In a CMYK document the panel menu's Create New Swatch saves the color the panel shows in
/// that color's model: an RGB object gives an RGB swatch, and CMYK, gray and spot colors give
/// swatches of their own colors.
#[test]
fn create_new_swatch_keeps_the_shown_colors_model_in_a_cmyk_document() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 60, "y": 90, "k": 0}, "spot": true})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let newest = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch");
    let ink = app.session.doc().unwrap().doc.swatch("Ink").and_then(|w| w.paint.color()).unwrap();
    for (fill, model) in [
        (json!({"color": "#336699", "keepModel": true}), vectorcraft_color::cms::Model::Rgb),
        (json!({"color": {"c": 10, "m": 20, "y": 30, "k": 5}}), vectorcraft_color::cms::Model::Cmyk),
        (json!({"color": {"gray": 0.4}}), vectorcraft_color::cms::Model::Gray),
        (json!({"swatch": "Ink"}), ink.model()),
    ] {
        app.run("paint.setFill", fill.clone()).unwrap();
        let shown = crate::panels::current_paints(&app).0.color().expect("a solid fill");
        assert_eq!(shown.model(), model, "{fill}");
        let count = app.session.doc().unwrap().doc.swatches.len();
        click_menu_item(&mut app, &ctx, "Create New Swatch…");
        assert_eq!(app.session.doc().unwrap().doc.swatches.len(), count + 1, "{fill}: one new swatch");
        let w = newest(&app);
        assert_eq!(w.paint.color(), Some(shown), "{fill}: the swatch has the shown color, {}", w.name);
    }
}

fn paints_hex(app: &VectorcraftApp) -> (String, String) {
    let (f, s) = crate::panels::current_paints(app);
    (f.color().map(|c| c.to_hex()).unwrap_or_default(), s.color().map(|c| c.to_hex()).unwrap_or_default())
}

#[test]
fn a_drag_on_the_spectrum_holds_still_near_white() {
    let mut app = app();
    let ctx = egui::Context::default();
    let none = egui::Modifiers::NONE;
    // White needs no warning, #fefefe the web one, a pale violet both: one panel height.
    let heights: Vec<f32> = ["#ffffff", "#fefefe", "#ded3ff"]
        .iter()
        .map(|c| {
            app.run("paint.setFill", json!({"color": c})).unwrap();
            frame(&mut app, &ctx, vec![], none);
            frame(&mut app, &ctx, vec![], none).1
        })
        .collect();
    assert!(heights.windows(2).all(|w| w[0] == w[1]), "{heights:?}");
    // Drag along the spectrum's top edge (pale colours, some out of gamut), stopping at each
    // point: the colour holds while the pointer does.
    let r = frame(&mut app, &ctx, vec![], none).0.expect("the spectrum is drawn");
    let at = |x: f32| egui::pos2(r.left() + x * r.width(), r.top() + 2.0);
    let button = |pressed| egui::Event::PointerButton { pos: at(0.02), button: egui::PointerButton::Primary, pressed, modifiers: none };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at(0.02)), button(true)], none);
    for i in 1..20 {
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at(i as f32 * 0.05))], none);
        let fills: Vec<String> = (0..3)
            .map(|_| {
                frame(&mut app, &ctx, vec![], none);
                paints_hex(&app).0
            })
            .collect();
        assert!(fills.windows(2).all(|w| w[0] == w[1]), "at {}: {fills:?}", i as f32 * 0.05);
    }
    frame(&mut app, &ctx, vec![button(false)], none);
}

#[test]
fn tandem_keeps_ratios() {
    let t = tandem(Mode::Rgb, &[100.0, 50.0, 25.0], 0, 200.0);
    assert_eq!(t, vec![200.0, 100.0, 50.0]);
    // It stops where a component reaches its maximum.
    let t = tandem(Mode::Rgb, &[200.0, 100.0, 0.0], 1, 200.0);
    assert_eq!(t, vec![255.0, 127.5, 0.0]);
    let t = tandem(Mode::Cmyk, &[10.0, 40.0, 20.0, 5.0], 1, 20.0);
    assert_eq!(t, vec![5.0, 20.0, 10.0, 2.5]);
    // From 0 the others move by the same amount.
    assert_eq!(tandem(Mode::Rgb, &[0.0, 50.0, 250.0], 0, 10.0), vec![10.0, 60.0, 255.0]);
    assert!(Mode::Rgb.tandem() && Mode::Cmyk.tandem() && !Mode::Hsb.tandem() && !Mode::Grayscale.tandem());
}

#[test]
fn the_mode_follows_the_colours_model() {
    let cmyk = Color::cmyk(0.1, 0.2, 0.3, 0.4);
    let rgb = Color::rgb(0.2, 0.4, 0.6);
    assert_eq!(display_mode(Some(Mode::Rgb), Some(&cmyk)), Mode::Cmyk);
    assert_eq!(display_mode(Some(Mode::Hsb), Some(&rgb)), Mode::Hsb, "HSB is remembered for RGB colours");
    assert_eq!(display_mode(Some(Mode::Cmyk), Some(&rgb)), Mode::Rgb);
    assert_eq!(display_mode(Some(Mode::Rgb), Some(&Color::gray(0.5))), Mode::Grayscale);
    assert_eq!(display_mode(Some(Mode::Cmyk), None), Mode::Cmyk);
    assert_eq!(convert_to(Mode::Hsb, &rgb), rgb);
    assert!(matches!(convert_to(Mode::Cmyk, &rgb), Color::Cmyk { .. }));
    assert_eq!(convert_to(Mode::WebSafe, &rgb).to_hex(), "#336699");
    assert_eq!(Mode::Rgb.next(), Mode::Hsb);
    assert_eq!(Mode::WebSafe.next(), Mode::Grayscale);
    // In the panel: an RGB pick, then a CMYK object shows its 4 sliders.
    let mut app = app();
    let ctx = egui::Context::default();
    app.ui.color_mode = Some(Mode::Rgb);
    app.run("paint.setFill", json!({"color": {"c": 0.1, "m": 0.2, "y": 0.3, "k": 0.4}})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let mode = shown_mode(&app, target(&app, ui).color().as_ref());
        assert_eq!(mode.labels(), ["C", "M", "Y", "K"]);
    });
}

/// The slider labels one frame of the panel draws.
fn slider_labels(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut full = ctx.run_ui(Default::default(), |ui| show(app, ui));
    full.textures_delta.clear();
    let mut out = vec![];
    full.shapes.iter().for_each(|s| texts(&s.shape, &mut out));
    out.retain(|t| ["K", "R", "G", "B", "H", "S", "C", "M", "Y"].contains(&t.as_str()));
    out
}

/// #1675: the mode picked last is kept with the UI state, which the app saves with its
/// preferences, so after a restart the panel shows the defaults for new art in it. A selected
/// colour still shows in its own model.
#[test]
fn the_picked_mode_comes_back_after_a_restart() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(slider_labels(&mut app, &ctx), ["C", "M", "Y", "K"], "the white default follows the CMYK document");
    let _ = ctx.run_ui(Default::default(), |ui| {
        let tgt = target(&app, ui);
        set_mode(&mut app, &tgt, Mode::Cmyk);
    });
    assert_eq!(app.ui.color_mode, Some(Mode::Cmyk));
    assert_eq!(slider_labels(&mut app, &ctx), ["C", "M", "Y", "K"]);
    // Quit and start again: the saved UI state comes back, the session and the panel are new.
    let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui = saved.sanitized();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    assert_eq!(app.ui.color_mode, Some(Mode::Cmyk), "the picked mode comes back");
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "the defaults follow the CMYK document");
    let ctx = egui::Context::default();
    assert_eq!(slider_labels(&mut app, &ctx), ["C", "M", "Y", "K"]);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699", "keepModel": true})).unwrap();
    assert_eq!(slider_labels(&mut app, &ctx), ["R", "G", "B"], "an RGB object shows RGB");
    // A saved value that isn't a menu mode reads as none, and the rest of the state still loads.
    for v in [json!("lab"), json!("CMYK"), json!(3), serde_json::Value::Null] {
        let ui: crate::UiState = serde_json::from_value(json!({"color_mode": v, "place_link": false})).unwrap();
        assert_eq!((ui.color_mode, ui.place_link), (None, false), "{v}");
    }
    assert_eq!(serde_json::from_value::<crate::UiState>(json!({"color_mode": "websafe"})).unwrap().color_mode, Some(Mode::WebSafe));
}

#[test]
fn lab_colours_show_lab_sliders_with_signed_ranges() {
    let lab = Color::lab(50.0, -20.0, 30.0);
    assert_eq!(Mode::of(&lab), Mode::Lab);
    assert_eq!(display_mode(Some(Mode::Rgb), Some(&lab)), Mode::Lab);
    assert!(!Mode::ALL.contains(&Mode::Lab), "not a panel menu mode");
    assert_eq!(components(Mode::Lab, &lab), vec![50.0, -20.0, 30.0]);
    assert_eq!(from_components(Mode::Lab, &[50.0, -20.0, 30.0]), lab);
    assert_eq!(from_components(Mode::Lab, &[150.0, -300.0, 300.0]), Color::lab(100.0, -128.0, 127.0), "clamped");
    assert_eq!((Mode::Lab.unit(1, -128.0), Mode::Lab.unit(2, 127.0), Mode::Lab.unit(0, 50.0)), (0.0, 1.0, 0.5));
    assert_eq!((Mode::Lab.value_at(1, 0.0), Mode::Lab.value_at(0, 1.0)), (-128.0, 100.0));
    assert_eq!((Mode::Rgb.value_at(0, 0.5), Mode::WebSafe.value_at(0, 0.45)), (128.0, 102.0), "other modes as before");
    assert_eq!(track_color(Mode::Lab, &[50.0, -20.0, 30.0], 1, 0.0), Color::lab(50.0, -128.0, 30.0));
    assert_eq!(convert_to(Mode::Lab, &Color::rgb(1.0, 0.0, 0.0)).model(), vectorcraft_color::cms::Model::Lab);
    // In the panel: a Lab fill shows L, a, b.
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": {"l": 50, "a": -20, "b": 30}})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let mode = shown_mode(&app, target(&app, ui).color().as_ref());
        assert_eq!(mode.labels(), ["L", "a", "b"]);
    });
}

#[test]
fn pure_blue_is_out_of_gamut_in_rgb() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    let (of, _, fix) = pstate::<Option<GamutFix>>(&ctx, "color-gamut").expect("RGB mode checks the gamut");
    assert_eq!(of.to_hex(), "#0000ff");
    assert!(fix.is_some_and(|f| !f.out_of_gamut()));
    app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert_eq!(pstate::<Option<GamutFix>>(&ctx, "color-gamut").unwrap().2, None);
}

#[test]
fn alt_click_on_the_spectrum_paints_the_stroke() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#123456"})).unwrap();
    app.run("paint.setStroke", json!({"color": "#000000", "focus": false})).unwrap();
    assert!(app.session.fill_active);
    click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::ALT);
    assert_eq!(paints_hex(&app), ("#123456".into(), "#ff0000".into()), "the stroke changes");
    assert!(app.session.fill_active, "the fill stays in front");
    // A plain click paints the fill.
    click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::NONE);
    assert_eq!(paints_hex(&app).0, "#ff0000");
}

/// A color picked in the panel, for either proxy, becomes the Color Guide's base color (#3137).
#[test]
fn a_picked_color_becomes_the_color_guides_base() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    app.ui.color_guide_base = Some(Color::WHITE);
    click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::NONE);
    assert_eq!(crate::panels::color_guide::base_color(&app).to_hex(), "#ff0000");
    // Alt: the stroke takes it, and it is still the base.
    click_spectrum(&mut app, &ctx, (0.5, 0.5), egui::Modifiers::ALT);
    let stroke = paints_hex(&app).1;
    assert_ne!(stroke, "#ff0000");
    assert_eq!(crate::panels::color_guide::base_color(&app).to_hex(), stroke);
}

#[test]
fn shift_click_on_the_spectrum_cycles_modes() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    click_spectrum(&mut app, &ctx, (0.5, 0.5), egui::Modifiers::SHIFT);
    assert_eq!(app.ui.color_mode, Some(Mode::Hsb));
    assert_eq!(paints_hex(&app).0, "#336699", "HSB keeps the RGB colour");
    click_spectrum(&mut app, &ctx, (0.5, 0.5), egui::Modifiers::SHIFT);
    assert_eq!(app.ui.color_mode, Some(Mode::Cmyk));
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK converts the colour");
}

#[test]
fn cmd_click_on_the_spectrum_applies_complement_and_cmd_shift_click_invert() {
    let mut app = app();
    let ctx = egui::Context::default();
    let cmd = egui::Modifiers::COMMAND;
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("paint.setStroke", json!({"color": "#123456", "focus": false})).unwrap();
    assert!(app.session.fill_active);
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd);
    assert_eq!(paints_hex(&app), ("#996633".into(), "#123456".into()), "Complement of the fill");
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd | egui::Modifiers::SHIFT);
    assert_eq!(paints_hex(&app), ("#6699cc".into(), "#123456".into()), "Invert of the fill");
    assert_eq!(app.ui.color_mode, None, "Cmd/Ctrl+Shift-click does not cycle the modes");
    // With Alt the inactive proxy (the stroke) changes, and the fill stays in front.
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd | egui::Modifiers::ALT);
    assert_eq!(paints_hex(&app), ("#6699cc".into(), "#563412".into()), "Complement of the stroke");
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd | egui::Modifiers::SHIFT | egui::Modifiers::ALT);
    assert_eq!(paints_hex(&app), ("#6699cc".into(), "#a9cbed".into()), "Invert of the stroke");
    assert!(app.session.fill_active, "the fill stays in front");
    // With the stroke in front, Alt recolors the fill.
    app.run("paint.toggleActive", json!({})).unwrap();
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd);
    assert_eq!(paints_hex(&app), ("#6699cc".into(), "#edcba9".into()), "Complement of the stroke in front");
    click_spectrum(&mut app, &ctx, (0.0, 0.5), cmd | egui::Modifiers::ALT);
    assert_eq!(paints_hex(&app), ("#cc9966".into(), "#edcba9".into()), "Complement of the fill behind");
    assert!(!app.session.fill_active, "the stroke stays in front");
    // A plain click still picks the spectrum's color.
    click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::NONE);
    assert_eq!(paints_hex(&app).1, "#ff0000");
}

#[test]
fn cmd_click_on_the_spectrum_recolors_the_selected_gradient_stop() {
    let mut app = app();
    let ctx = egui::Context::default();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#336699"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::COMMAND);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    let hex: Vec<String> = g.gradient.stops.iter().map(|s| s.color.to_hex()).collect();
    assert_eq!(hex, ["#ff0000", "#996633"], "only the selected stop changes");
}

#[test]
fn cmd_click_on_the_spectrum_with_the_options_hidden() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("ui.colorPanel", json!({"options": false})).unwrap();
    // The spectrum fills the row right of the proxy.
    let mut row = Rect::NOTHING;
    let input = egui::RawInput::default();
    let mut full = ctx.run_ui(input, |ui| {
        show(&mut app, ui);
        row = ui.min_rect();
    });
    full.textures_delta.clear();
    let pos = egui::pos2(row.right() - 4.0, row.center().y);
    let cmd = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: cmd };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos), button(true)], cmd);
    frame(&mut app, &ctx, vec![button(false)], cmd);
    assert_eq!(paints_hex(&app).0, "#cc9966", "Invert of the fill");
}

#[test]
fn double_clicking_a_proxy_opens_the_color_picker_for_it() {
    let mut app = app();
    let ctx = egui::Context::default();
    // One frame of the proxy alone; returns the stroke square's rect.
    let run = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let input = egui::RawInput { time: Some(time), events, ..Default::default() };
        let mut rect = None;
        let mut out = ctx.run_ui(input, |ui| {
            crate::panels::proxy(app, ui, 40.0);
            rect = ctx.read_response(ui.scope_id().with("stroke-proxy")).map(|r| r.rect);
        });
        out.textures_delta.clear();
        rect
    };
    let pos = run(&mut app, 0.0, vec![]).expect("the stroke square is drawn").right_bottom() - vec2(3.0, 3.0);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run(&mut app, 0.1, vec![egui::Event::PointerMoved(pos), button(true)]);
    run(&mut app, 0.15, vec![button(false)]);
    assert!(!app.session.fill_active, "a click brings the stroke to the front");
    assert!(app.ui.dialog.is_none());
    run(&mut app, 0.2, vec![button(true)]);
    run(&mut app, 0.25, vec![button(false)]);
    let d = app.ui.dialog.as_ref().expect("a double-click opens the Color Picker");
    assert_eq!(d.kind, "colorPicker");
    assert!(d.bool("stroke"));
}

#[test]
fn a_global_colour_shows_tint_mode_and_the_t_slider_sets_its_tint() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let tgt = target(&app, ui);
        let t = tgt.tint().expect("Tint mode");
        assert_eq!((t.swatch.as_str(), t.spot, t.tint, t.base), ("Ink", true, 0.5, Color::cmyk(0.0, 1.0, 0.0, 0.0)));
    });
    // The chip's name and the T row are drawn; a click on the slider's right part raises the
    // tint and keeps the link.
    let texts = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(260.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    };
    let drawn = texts(&mut app, vec![]);
    let at = |label: &str| drawn.iter().find(|(t, _)| t == label).map(|(_, p)| *p);
    assert!(at("Ink").is_some(), "the swatch's name: {drawn:?}");
    let (t_label, field) = (at("T").expect("the T row"), at("50%").expect("the tint field"));
    let pos = egui::pos2(field.x - 40.0, t_label.y);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    texts(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
    texts(&mut app, vec![button(false)]);
    let Paint::Solid { swatch, tint, .. } = crate::panels::current_paints(&app).0 else { panic!() };
    assert_eq!(swatch.as_deref(), Some("Ink"));
    assert!(tint > 0.6 && tint < 1.0, "{tint}");
    // Picking a mode makes it a process colour.
    let _ = ctx.run_ui(Default::default(), |ui| {
        let tgt = target(&app, ui);
        set_mode(&mut app, &tgt, Mode::Cmyk);
    });
    assert!(matches!(crate::panels::current_paints(&app).0, Paint::Solid { swatch: None, .. }));
}

/// Screen readers get each slider's number and the text of the field beside it, with the
/// component's range: R, G and B from 0 to 255, H in degrees, S, B, the inks and T in percent.
#[test]
fn screen_readers_get_the_numbers_the_fields_show() {
    use crate::widgets::accessibility_tests::{slider_value, tree};
    let mut app = app();
    let v = |n: f64, max: f64, text: &str| (Some(n), Some(0.0), Some(max), Some(text.to_string()));
    app.run("paint.setFill", json!({"color": "#ff8000"})).unwrap();
    let rgb = tree(|ui| show(&mut app, ui));
    assert_eq!(slider_value(&rgb, "R"), v(255.0, 255.0, "255"));
    assert_eq!(slider_value(&rgb, "G"), v(128.0, 255.0, "128"));
    assert_eq!(slider_value(&rgb, "B"), v(0.0, 255.0, "0"));
    app.run("ui.colorPanel", json!({"mode": "hsb"})).unwrap();
    let hsb = tree(|ui| show(&mut app, ui));
    assert_eq!(slider_value(&hsb, "H"), v(30.0, 360.0, "30°"));
    assert_eq!(slider_value(&hsb, "S"), v(100.0, 100.0, "100%"));
    assert_eq!(slider_value(&hsb, "B"), v(100.0, 100.0, "100%"));
    app.run("paint.setFill", json!({"color": {"c": 10, "m": 20, "y": 30, "k": 40}})).unwrap();
    let cmyk = tree(|ui| show(&mut app, ui));
    assert_eq!(slider_value(&cmyk, "C"), v(10.0, 100.0, "10%"));
    assert_eq!(slider_value(&cmyk, "K"), v(40.0, 100.0, "40%"));
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 35})).unwrap();
    let tint = tree(|ui| show(&mut app, ui));
    assert_eq!(slider_value(&tint, "T"), v(35.0, 100.0, "35%"));
}

#[test]
fn hide_options_leaves_the_proxy_and_spectrum() {
    let mut app = app();
    let ctx = egui::Context::default();
    let (spectrum, full) = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert!(spectrum.is_some());
    assert_eq!(app.run("ui.colorPanel", json!({"options": false})).unwrap()["options"], false);
    let (_, hidden) = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert!(hidden <= 44.0 && full > 150.0, "{hidden} / {full}");
    app.run("ui.colorPanel", json!({"options": true})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], egui::Modifiers::NONE).1, full);
}

/// Hide Options is kept with the UI state, which the app saves with its preferences. A saved
/// value that isn't true or false reads as Show Options, and the rest of the state still loads.
#[test]
fn hide_options_comes_back_after_a_restart() {
    let mut app = app();
    app.run("ui.colorPanel", json!({"options": false})).unwrap();
    let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(saved.color_hide_options);
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_hide_options"], true);
    for v in [json!(7), json!("hide"), json!({}), Value::Null] {
        let ui: crate::UiState = serde_json::from_value(json!({"color_hide_options": v, "color_mode": "future", "place_link": false})).unwrap();
        assert_eq!((ui.color_hide_options, ui.color_mode, ui.place_link), (false, None, false), "{v}");
    }
}

/// The panel menu's Show Recent Colors hides the Recent Colors row and shows it again. Applied
/// colors still join the list while the row is hidden, Hide Options hides the row either way, and
/// the Swatches panel keeps its own row.
#[test]
fn show_recent_colors_hides_and_shows_the_row() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let none = egui::Modifiers::NONE;
    // The first recent chip's rect and the panel's height. The second frame's answer counts, since
    // egui also answers for a widget the frame before drew.
    let chip = |app: &mut VectorcraftApp| {
        frame_with(app, &ctx, vec![], none, ("recent", 0usize));
        frame_with(app, &ctx, vec![], none, ("recent", 0usize))
    };
    let (shown, full) = chip(&mut app);
    assert!(shown.is_some(), "the row shows by default");
    // The item is checked while the row shows.
    click_menu_item(&mut app, &ctx, "✓ Show Recent Colors");
    assert!(!app.ui.color_show_recent_colors);
    let (hidden, short) = chip(&mut app);
    assert!(hidden.is_none() && short < full, "the row is hidden: {short} / {full}");
    // A color applied while the row is hidden still joins the list.
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    assert_eq!(app.session.recent_colors.first().map(|c| c.to_hex()).as_deref(), Some("#00ff00"));
    // The Swatches panel keeps its row (drawn in a context of its own, whose chip ids the Color
    // panel's would share).
    let mut swatch_chip = None;
    let screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(280.0, 700.0)));
    let swatches = egui::Context::default();
    let mut out = swatches.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| {
        crate::panels::swatches::show(&mut app, ui);
        swatch_chip = swatches.read_response(ui.scope_id().with(("recent", 0usize))).map(|r| r.rect);
    });
    out.textures_delta.clear();
    assert!(swatch_chip.is_some(), "the Swatches panel still draws its recent colors");
    // Show Options leaves the row hidden, and Hide Options hides it with the row checked too.
    app.run("ui.colorPanel", json!({"options": false})).unwrap();
    app.run("ui.colorPanel", json!({"options": true})).unwrap();
    assert!(chip(&mut app).0.is_none(), "Show Options keeps Show Recent Colors unchecked");
    click_menu_item(&mut app, &ctx, "Show Recent Colors");
    assert!(chip(&mut app).0.is_some(), "checking the item shows the row again");
    assert_eq!(app.run("ui.colorPanel", json!({"options": false})).unwrap()["recentColors"], true);
    assert!(chip(&mut app).0.is_none(), "Hide Options hides the row");
    assert!(app.ui.color_show_recent_colors, "Hide Options leaves Show Recent Colors checked");
    let r = app.run("ui.colorPanel", json!({"options": true, "recentColors": false})).unwrap();
    assert_eq!((&r["options"], &r["recentColors"]), (&json!(true), &json!(false)));
    assert!(chip(&mut app).0.is_none());
    app.run("ui.colorPanel", json!({"recentColors": true})).unwrap();
    assert_eq!(chip(&mut app).1, full, "the panel is as tall as before");
}

/// Show Recent Colors is kept with the UI state, which the app saves with its preferences. A saved
/// state without it, or with a value that isn't true or false, reads as checked, and the rest of the
/// state still loads. A value that isn't true or false in the command answers an error and changes
/// nothing.
#[test]
fn show_recent_colors_comes_back_after_a_restart() {
    let mut app = app();
    app.run("ui.colorPanel", json!({"recentColors": false})).unwrap();
    let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(!saved.color_show_recent_colors);
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_show_recent_colors"], false);
    let old: crate::UiState = serde_json::from_value(json!({"place_link": false})).unwrap();
    assert!(old.color_show_recent_colors, "a state saved before the setting existed shows the row");
    for v in [json!(7), json!("hide"), json!({}), Value::Null] {
        let ui: crate::UiState = serde_json::from_value(json!({"color_show_recent_colors": v, "place_link": false})).unwrap();
        assert_eq!((ui.color_show_recent_colors, ui.place_link), (true, false), "{v}");
    }
    for v in [json!("no"), json!(0), json!([])] {
        let e = app.run("ui.colorPanel", json!({"mode": "cmyk", "options": false, "recentColors": v})).unwrap_err();
        assert_eq!(e, "`recentColors` must be true or false", "{v}");
    }
    assert_eq!((app.ui.color_mode, app.ui.color_hide_options, app.ui.color_show_recent_colors), (None, false, false));
}

/// The fill's R, G and B from 0 to 1.
fn fill_rgb(app: &VectorcraftApp) -> [f32; 3] {
    working_rgb(&crate::panels::current_paints(app).0.color().expect("a solid fill"))
}

/// One frame of the panel at `time` with `events`; returns the rect of the cell right of the field
/// of slider row `i` (None: the row has no cell).
fn units_cell(app: &mut VectorcraftApp, ctx: &egui::Context, i: usize, time: f64, events: Vec<egui::Event>) -> Option<Rect> {
    let input = egui::RawInput { time: Some(time), events, ..Default::default() };
    let mut rect = None;
    let mut out = ctx.run_ui(input, |ui| {
        show(app, ui);
        rect = ctx.read_response(ui.scope_id().with("color-units").with(i)).map(|r| r.rect);
    });
    out.textures_delta.clear();
    rect
}

/// The rect of the slider that screen readers name `label`, in a frame of `ctx` (`width`: the
/// screen's width; None: the screen of the frame before).
fn slider_rect(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, width: Option<f32>) -> Rect {
    ctx.enable_accesskit();
    let screen_rect = width.map(|w| Rect::from_min_size(egui::Pos2::ZERO, vec2(w, 800.0)));
    let mut nodes = vec![];
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    }
    let slider = nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::Slider && n.label() == Some(label));
    let b = slider.and_then(|(_, n)| n.bounds()).unwrap_or_else(|| panic!("no slider is named {label:?}"));
    Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))
}

/// Double-click the cell right of the field of slider row `i`, at `time` and a moment later.
fn double_click_units(app: &mut VectorcraftApp, ctx: &egui::Context, i: usize, time: f64) {
    let run = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| units_cell(app, ctx, i, time, events);
    let pos = run(app, time, vec![]).expect("the cell is drawn").center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run(app, time + 0.05, vec![egui::Event::PointerMoved(pos), button(true)]);
    run(app, time + 0.1, vec![button(false)]);
    run(app, time + 0.15, vec![button(true)]);
    run(app, time + 0.2, vec![button(false)]);
    run(app, time + 0.25, vec![]);
}

/// In RGB mode a double-click right of an RGB field switches R, G and B between 0 to 255 and
/// percent, as `ui.colorPanel {rgbPercent}` does. The color does not change; a typed percent and a
/// slider click convert back to 0 to 255. Web Safe RGB and the hex field stay in 0 to 255. Only
/// RGB rows have the cell, and Tab does not stop on it.
#[test]
fn rgb_percent_switches_the_rgb_fields_between_percent_and_0_to_255() {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    use crate::widgets::accessibility_tests::{slider_value, tab_stops, tree};
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    let texts = |app: &mut VectorcraftApp| frame_events(&ctx, app, vec![], show).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let has = |texts: &[String], all: &[&str]| all.iter().all(|s| texts.iter().any(|t| t == s));
    assert!(has(&texts(&mut app), &["51", "102", "153"]));
    assert_eq!(app.run("ui.colorPanel", json!({"rgbPercent": true})).unwrap()["rgbPercent"], true);
    let shown = texts(&mut app);
    assert!(has(&shown, &["20%", "40%", "60%", "336699"]), "{shown:?}");
    assert_eq!(paints_hex(&app).0, "#336699", "the color does not change");
    let v = |n: f64, text: &str| (Some(n), Some(0.0), Some(100.0), Some(text.to_string()));
    let sliders = tree(|ui| show(&mut app, ui));
    assert_eq!(slider_value(&sliders, "G"), v(40.0, "40%"));
    // A typed percent sets the component: 50 % is 127.5 of 255.
    let field = text_rect(&frame_events(&ctx, &mut app, vec![], show), "20%").center();
    click(&ctx, &mut app, field, show);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame_events(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("50".into())], show);
    frame_events(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], show);
    let [r, g, b] = fill_rgb(&app);
    assert!((r - 0.5).abs() < 1e-6 && (g - 0.4).abs() < 1e-6 && (b - 0.6).abs() < 1e-6, "{r} {g} {b}");
    assert!(has(&texts(&mut app), &["50%", "40%", "60%"]));
    // A click a third of the way along the B slider's bar picks 33 %, a whole percent.
    let rgb_slider = slider_rect(&mut app, &ctx, "B", None);
    let (bar_left, bar_w) = (rgb_slider.left() + 4.0, rgb_slider.width() - 8.0);
    click(&ctx, &mut app, egui::pos2(bar_left + bar_w / 3.0, rgb_slider.center().y), show);
    let p = fill_rgb(&app)[2] * 100.0;
    assert!((p - 33.0).abs() < 1e-3, "{p}");
    // The cells take no keyboard focus: Tab goes from field to slider without stopping on them.
    let cells: Vec<_> = (0..3).filter_map(|i| units_cell(&mut app, &ctx, i, 5.0, vec![]).map(|_| i)).collect();
    assert_eq!(cells, [0, 1, 2], "each RGB row has a cell");
    let mut units = egui::Id::NULL;
    let stops = tab_stops(&ctx, 24, |events| {
        frame_events(&ctx, &mut app, events, |app, ui| {
            show(app, ui);
            units = ui.scope_id().with("color-units");
        });
    });
    let cell_ids: Vec<egui::Id> = (0..3).map(|i| units.with(i)).collect();
    assert!(stops.len() >= 6 && stops.iter().all(|s| !cell_ids.contains(s)), "{stops:?}");
    app.run("paint.setFill", json!({"color": "#803366"})).unwrap();
    assert!((fill_rgb(&app)[0] - 128.0 / 255.0).abs() < 1e-6);
    // A double-click right of the G field switches back to 0 to 255, and another to percent.
    double_click_units(&mut app, &ctx, 1, 10.0);
    assert!(!app.ui.color_rgb_percent);
    assert!(has(&texts(&mut app), &["128", "51", "102"]));
    assert_eq!(paints_hex(&app).0, "#803366", "the color does not change");
    double_click_units(&mut app, &ctx, 1, 20.0);
    assert!(app.ui.color_rgb_percent);
    assert!(has(&texts(&mut app), &["50%", "20%", "40%"]));
    assert_eq!(app.run("ui.colorPanel", json!({})).unwrap()["rgbPercent"], true);
    // RGB sliders are narrower by the cell and its spacing, in a narrow panel too.
    let cell_room = 10.0 + ctx.global_style().spacing.item_spacing.x;
    let rgb_w = slider_rect(&mut app, &ctx, "B", Some(400.0)).width();
    let narrow_rgb = slider_rect(&mut app, &ctx, "B", Some(150.0)).width();
    // Web Safe RGB shows 0 to 255. It has no cells, and its sliders keep the room the RGB cells
    // take.
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("ui.colorPanel", json!({"mode": "websafe"})).unwrap();
    assert!(has(&texts(&mut app), &["51", "102", "153"]));
    assert_eq!(units_cell(&mut app, &ctx, 1, 30.0, vec![]), None);
    let web_w = slider_rect(&mut app, &ctx, "B", Some(400.0)).width();
    assert!((web_w - rgb_w - cell_room).abs() < 0.5, "{web_w} {rgb_w}");
    let narrow_web = slider_rect(&mut app, &ctx, "B", Some(150.0)).width();
    assert!((narrow_web - narrow_rgb - cell_room).abs() < 0.5, "{narrow_web} {narrow_rgb}");
    // HSB shows its own units, with no cells and sliders as wide as Web Safe RGB's.
    app.run("ui.colorPanel", json!({"mode": "hsb"})).unwrap();
    assert!(has(&texts(&mut app), &["210°", "67%", "60%"]));
    assert_eq!(units_cell(&mut app, &ctx, 1, 31.0, vec![]), None);
    assert!((slider_rect(&mut app, &ctx, "B", Some(400.0)).width() - web_w).abs() < 0.5);
    assert!(app.ui.color_rgb_percent);
}

/// While a mixed selection disables the RGB rows, a double-click right of an RGB field does not
/// switch the units, as the fields beside it take no input.
#[test]
fn rgb_percent_does_not_switch_while_the_rows_are_disabled() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("ui.colorPanel", json!({"mode": "rgb"})).unwrap();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setFill", json!({"color": "#00ff00", "ids": [b]})).unwrap();
    units_cell(&mut app, &ctx, 1, 1.0, vec![]);
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false));
    double_click_units(&mut app, &ctx, 1, 2.0);
    assert!(!app.ui.color_rgb_percent);
    // With one color the rows take input, and the same double-click switches the units.
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    double_click_units(&mut app, &ctx, 1, 3.0);
    assert!(app.ui.color_rgb_percent);
}

/// RGB in percent is kept with the UI state, which the app saves with its preferences. A saved
/// state without it, or with a value that isn't true or false, reads as 0 to 255, and the rest of
/// the state still loads. A value that isn't true or false in the command answers an error and
/// changes nothing.
#[test]
fn rgb_percent_comes_back_after_a_restart() {
    let mut app = app();
    app.run("ui.colorPanel", json!({"rgbPercent": true})).unwrap();
    let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(saved.color_rgb_percent);
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_rgb_percent"], true);
    let old: crate::UiState = serde_json::from_value(json!({"place_link": false})).unwrap();
    assert!(!old.color_rgb_percent, "a state saved before the setting existed shows 0 to 255");
    for v in [json!(7), json!("percent"), json!({}), Value::Null] {
        let ui: crate::UiState = serde_json::from_value(json!({"color_rgb_percent": v, "place_link": false})).unwrap();
        assert_eq!((ui.color_rgb_percent, ui.place_link), (false, false), "{v}");
    }
    for v in [json!("yes"), json!(1), json!([])] {
        let e = app.run("ui.colorPanel", json!({"mode": "cmyk", "rgbPercent": v})).unwrap_err();
        assert_eq!(e, "`rgbPercent` must be true or false", "{v}");
    }
    assert_eq!((app.ui.color_mode, app.ui.color_rgb_percent), (None, true));
    assert_eq!(app.run("ui.colorPanel", json!({"rgbPercent": false})).unwrap()["rgbPercent"], false);
}

/// Toggle RGB Percent switches the units from the keyboard: the command palette lists it, Keyboard
/// Shortcuts takes a key for it, and each run switches R, G and B between percent and 0 to 255 as
/// the double-click does, without changing the color.
#[test]
fn toggle_rgb_percent_switches_the_units_from_the_keyboard() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("ui.colorPanel", json!({"mode": "rgb"})).unwrap();
    let listed = crate::palette::items().into_iter().any(|(label, id, _)| id == "ui.colorRgbPercent" && label == "Toggle RGB Percent");
    assert!(listed, "the command palette lists it");
    let e = crate::shortcut_editor::entry("ui.colorRgbPercent").expect("listed in Keyboard Shortcuts");
    assert_eq!(e.label, "Toggle RGB Percent");
    assert_eq!(crate::shortcut_editor::default_of("ui.colorRgbPercent"), None, "no default key");
    assert_eq!(app.run("ui.colorRgbPercent", json!({})).unwrap(), json!(true));
    assert!(app.ui.color_rgb_percent);
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert_eq!(app.run("ui.colorPanel", json!({})).unwrap()["rgbPercent"], true);
    for p in [json!({"rgbPercent": true}), json!({"on": false})] {
        let e = app.run("ui.colorRgbPercent", p.clone()).unwrap_err();
        assert!(e.ends_with("; the command takes no parameters"), "{p}: {e}");
    }
    for p in [json!(false), json!([]), json!("on")] {
        assert_eq!(app.run("ui.colorRgbPercent", p.clone()).unwrap_err(), format!("the params must be an object, not {p}"));
    }
    assert!(app.ui.color_rgb_percent, "a parameter changes nothing");
    assert_eq!(app.run("ui.colorRgbPercent", json!({})).unwrap(), json!(false));
    assert!(!app.ui.color_rgb_percent);
    assert_eq!(paints_hex(&app).0, "#336699", "the color does not change");
}

/// `ui.colorPanel {mode}` picks a mode as the panel menu does: HSB shows an RGB fill as H, S and
/// B, and CMYK converts it as one undo step.
#[test]
fn ui_color_panel_picks_a_mode_as_the_menu_does() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    let r = app.run("ui.colorPanel", json!({"mode": "hsb"})).unwrap();
    assert_eq!(r, json!({"mode": "hsb", "options": true, "recentColors": true, "rgbPercent": false, "shown": "hsb"}));
    assert_eq!(slider_labels(&mut app, &ctx), ["H", "S", "B"]);
    assert_eq!(crate::control::inspect(&app, &ctx)["ui"]["color_mode"], "hsb");
    assert_eq!(paints_hex(&app).0, "#336699", "HSB keeps the RGB color");
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk"})).unwrap()["shown"], "cmyk");
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK converts the color");
    app.run("edit.undo", json!({})).unwrap();
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Rgb { .. })), "undo restores it");
    assert_eq!(paints_hex(&app).0, "#336699");
    // `{}` reads the settings: CMYK stays picked, and the RGB fill shows in its own model.
    assert_eq!(
        app.run("ui.colorPanel", json!({})).unwrap(),
        json!({"mode": "cmyk", "options": true, "recentColors": true, "rgbPercent": false, "shown": "rgb"})
    );
    // A tint of a global swatch shows Tint mode.
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
    assert_eq!(app.run("ui.colorPanel", json!({})).unwrap()["shown"], "tint");
}

/// A mode the panel menu doesn't list, Lab included, and an options value that isn't true or
/// false answer an error, and the command then changes nothing.
#[test]
fn ui_color_panel_answers_an_error_for_a_mode_off_the_menu() {
    let mut app = app();
    for m in [json!("lab"), json!("oklch"), json!("hsl"), json!("future"), json!("RGB"), json!(3)] {
        let e = app.run("ui.colorPanel", json!({"mode": m})).unwrap_err();
        assert_eq!(e, "`mode` must be grayscale, rgb, hsb, cmyk or websafe", "{m}");
    }
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk", "options": "no"})).unwrap_err(), "`options` must be true or false");
    assert_eq!((app.ui.color_mode, app.ui.color_hide_options), (None, false));
}

/// `ui.colorPanel` answers every junk parameter, with and without a document, without a panic.
#[test]
fn ui_color_panel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorPanel").expect("registered").3;
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
        let _ = app.run("ui.colorPanel", p);
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk"})).unwrap()["mode"], "cmyk");
}

#[test]
fn a_mixed_selection_has_no_colour_to_edit() {
    let mut app = app();
    let ctx = egui::Context::default();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setFill", json!({"color": "#00ff00", "ids": [b]})).unwrap();
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false));
    let _ = ctx.run_ui(Default::default(), |ui| assert_eq!(target(&app, ui).color(), None));
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false), "a new revision recomputes it");
}

/// With gradient mesh points picked, the panel edits and shows their color (a solid fill
/// recolors just them), not the mesh object's fill, so the sliders keep the value they set,
/// through a drag too; "?" when the picked points differ, which picking alone (no new revision)
/// updates. The Control bar's opacity is the picked point's.
#[test]
fn picked_mesh_points_are_the_color_the_panel_shows_and_edits() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    let ctx = egui::Context::default();
    let id = app.session.active().unwrap().selection.objects[0];
    app.run("paint.setFill", json!({"color": "#ff8500"})).unwrap();
    app.run("object.mesh.create", json!({"rows": 2, "cols": 2})).unwrap();
    app.run("object.mesh.setPointColor", json!({"id": id.0, "index": 0, "color": "#0000ff", "opacity": 0.5})).unwrap();
    let v = ViewInfo::default();
    app.session.select_tool("mesh", v).unwrap();
    let click = |app: &mut VectorcraftApp, x: f64, y: f64, shift: bool| {
        for k in [PointerKind::Down, PointerKind::Up] {
            let mut ev = PointerEvent::new(k, x, y);
            ev.mods.shift = shift;
            app.session.pointer(&ev, v).unwrap();
        }
    };
    let picked = |app: &VectorcraftApp| app.session.fill_mesh_points().into_iter().flat_map(|(_, v)| v).collect::<Vec<_>>();
    // The top-left corner point (index 0).
    click(&mut app, 10.0, 10.0, false);
    assert_eq!(picked(&app), vec![0], "the click picks the top-left point");
    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false));
    let tgt = target_with(&app, false);
    assert_eq!(tgt.color().map(|c| c.to_hex()), Some("#0000ff".into()), "the picked point's color, not the object's fill");
    assert_eq!(crate::panels::current_transparency(&app).map(|t| t.0), Some(0.5), "the picked point's opacity");
    // A slider drag: each step shows the color it set, and the release keeps it.
    for c in [Color::rgb8(250, 0, 120), Color::rgb8(240, 10, 140)] {
        let tgt = target_with(&app, false);
        apply(&mut app, &tgt, c, Live::Dragging, false);
        assert_eq!(target_with(&app, false).color(), Some(c), "the drag shows the color it set");
    }
    let orange = Color::rgb8(255, 128, 0);
    let tgt = target_with(&app, false);
    apply(&mut app, &tgt, orange, Live::Released, false);
    assert_eq!(target_with(&app, false).color(), Some(orange), "the panel keeps the color it set");
    let NodeKind::Mesh(m) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("still a gradient mesh") };
    assert_eq!(m.points[0].color, orange);
    let middle = m.idx(0, 1);
    assert_ne!(m.points[middle].color, orange, "only the picked point");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false));
    // Shift-click the top middle point (still the object's color): the two differ.
    click(&mut app, 35.0, 10.0, true);
    assert_eq!(picked(&app), vec![0, middle], "the Shift-click adds the top middle point");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false), "picking recomputes it");
}

#[test]
fn rgb_roundtrip() {
    let c = Color::rgb8(230, 120, 40);
    let v = components(Mode::Rgb, &c);
    assert_eq!(v.iter().map(|x| x.round() as i32).collect::<Vec<_>>(), vec![230, 120, 40]);
    assert_eq!(from_components(Mode::Rgb, &v).to_hex(), c.to_hex());
}

#[test]
fn hsb_roundtrip_and_ranges() {
    let c = Color::rgb8(230, 120, 40);
    let v = components(Mode::Hsb, &c);
    assert!(v[0] > 0.0 && v[0] < 360.0 && v[1] <= 100.0 && v[2] <= 100.0);
    assert_eq!(from_components(Mode::Hsb, &v).to_hex(), c.to_hex());
    assert_eq!(Mode::Hsb.max(0), 360.0);
    assert_eq!(Mode::Hsb.suffix(0), "°");
}

#[test]
fn cmyk_and_gray_keep_model() {
    let c = from_components(Mode::Cmyk, &[10.0, 20.0, 30.0, 40.0]);
    assert!(matches!(c, Color::Cmyk { .. }));
    let v = components(Mode::Cmyk, &c);
    assert!((v[3] - 40.0).abs() < 1e-3);
    let g = from_components(Mode::Grayscale, &[25.0]);
    assert_eq!(g, Color::gray(0.25));
    assert!((components(Mode::Grayscale, &g)[0] - 25.0).abs() < 1e-4);
    // Grayscale view of an RGB colour uses luminance.
    assert!((components(Mode::Grayscale, &Color::WHITE)[0]).abs() < 1e-4);
}

#[test]
fn web_safe_snaps() {
    let c = Color::rgb8(230, 120, 40);
    let w = web_safe(&c);
    assert_eq!(w.to_hex(), "#ff6633");
    assert!(is_web_safe(&w));
    assert!(!is_web_safe(&c));
    assert_eq!(from_components(Mode::WebSafe, &[230.0, 120.0, 40.0]).to_hex(), "#ff6633");
}

#[test]
fn slider_tracks_vary_one_component() {
    let comps = [230.0, 120.0, 40.0];
    let a = track_color(Mode::Rgb, &comps, 0, 0.0).to_rgba8(1.0);
    let b = track_color(Mode::Rgb, &comps, 0, 1.0).to_rgba8(1.0);
    assert_eq!((a[0], a[1], a[2]), (0, 120, 40));
    assert_eq!((b[0], b[1], b[2]), (255, 120, 40));
    let h = track_color(Mode::Hsb, &[0.0, 100.0, 100.0], 0, 1.0 / 3.0).to_hex();
    assert_eq!(h, "#00ff00");
}

#[test]
fn spectrum_and_hex() {
    assert_eq!(spectrum_at(Mode::Rgb, 0.0, 0.0).to_hex(), "#ffffff");
    assert_eq!(spectrum_at(Mode::Rgb, 0.0, 0.5).to_hex(), "#ff0000");
    assert_eq!(spectrum_at(Mode::Rgb, 0.3, 1.0).to_hex(), "#000000");
    assert_eq!(spectrum_at(Mode::Grayscale, 1.0, 0.3), Color::gray(1.0));
    assert_eq!(parse_hex("E67828").unwrap().to_hex(), "#e67828");
    assert_eq!(parse_hex("#fff").unwrap().to_hex(), "#ffffff");
    assert!(parse_hex("zz").is_none());
    assert!(parse_hex("12345").is_none());
    assert!(parse_hex("##e67828").is_none());
    assert!(parse_hex("###fff").is_none());
}
