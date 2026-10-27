use vectorcraft_engine::Session;

use super::*;
use crate::panels::color::in_gamut;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn fill_hex(app: &VectorcraftApp) -> String {
    crate::panels::current_paints(app).0.color().unwrap().to_hex()
}

/// Runs one frame of the dialog with `events` in `ctx`, which keeps the keyboard focus from
/// frame to frame, and returns the texts drawn with their screen rects.
fn frame_in(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
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

/// Returns the label of the value field that has the keyboard, which is the letter (or `#`)
/// nearest to the field's left on its row, and the field's rect. Returns `None` when no text
/// field has the keyboard.
fn focused_field(ctx: &egui::Context, texts: &[(String, Rect)]) -> Option<(String, Rect)> {
    let id = ctx.memory(|m| m.focused()).filter(|&id| egui::TextEdit::load_state(ctx, id).is_some())?;
    let field = ctx.read_response(id)?.rect;
    let labels = ["H", "S", "B", "R", "G", "#", "L", "a", "b", "C", "M", "Y", "K"];
    let (label, _) = texts
        .iter()
        .filter(|(t, r)| labels.contains(&t.as_str()) && r.max.x <= field.min.x && field.y_range().contains(r.center().y))
        .max_by(|a, b| a.1.max.x.total_cmp(&b.1.max.x))?;
    Some((label.clone(), field))
}

#[test]
fn field_and_slider_map_back_per_channel() {
    let p = Picked::of(Color::rgb8(51, 153, 204));
    for ch in Channel::ALL {
        let (z, x, y) = field_pos(ch, &p);
        assert_eq!(field_pick(ch, z, x, y).color.to_hex(), "#3399cc", "{ch:?}");
        assert_eq!(Channel::parse(ch.id()), Some(ch));
    }
    // Hue: x is saturation, y brightness (white at the top left, black along the bottom).
    assert_eq!(field_pick(Channel::Hue, 0.0, 0.0, 0.0).color.to_hex(), "#ffffff");
    assert_eq!(field_pick(Channel::Hue, 0.0, 1.0, 0.0).color.to_hex(), "#ff0000");
    assert_eq!(field_pick(Channel::Hue, 0.5, 1.0, 1.0).color.to_hex(), "#000000");
    // Red: the slider is red, x blue and y green (green grows upwards).
    assert_eq!(field_pick(Channel::Red, 1.0, 1.0, 0.0).color.to_hex(), "#ffffff");
    assert_eq!(field_pick(Channel::Red, 0.0, 1.0, 1.0).color.to_hex(), "#0000ff");
    assert_eq!(field_pick(Channel::Green, 1.0, 0.0, 1.0).color.to_hex(), "#00ff00");
    assert_eq!(field_pick(Channel::Blue, 0.0, 1.0, 1.0).color.to_hex(), "#ff0000");
    // The hue slider shows the spectrum whatever the colour; the others vary one component.
    assert_eq!(slider_color(Channel::Hue, 1.0 / 3.0, 0.0, 1.0).to_hex(), "#00ff00");
    assert_eq!(slider_color(Channel::Saturation, 0.0, 0.0, 0.0).to_hex(), "#ffffff");
    // A grey keeps the hue it was picked with.
    let grey = field_pick(Channel::Hue, 0.5, 0.0, 0.5);
    assert_eq!(grey.hsb[0], 180.0);
    assert_eq!(field_pos(Channel::Hue, &grey).0, 0.5);
}

#[test]
fn only_web_colors_snaps() {
    let p = Picked::of(Color::rgb8(230, 120, 40));
    assert_eq!(p.snapped(false), p);
    assert_eq!(p.snapped(true).color.to_hex(), "#ff6633");
    assert!(is_web_safe(&p.snapped(true).color));
    let mut d = Dialog::new(KIND, json!({"color": "#e67828", "webOnly": true}));
    assert_eq!(picked(&d).color.to_hex(), "#ff6633");
    d.fields.insert("webOnly".into(), json!(false));
    assert_eq!(picked(&d).color.to_hex(), "#e67828");
}

#[test]
fn gamut_correction_gives_a_printable_colour() {
    let mut app = app();
    let blue = Color::rgb(0.0, 0.0, 1.0);
    assert!(blue.out_of_gamut());
    let fix = in_gamut(&mut app, &blue).expect("pure blue can't be printed");
    assert!(!fix.out_of_gamut(), "{fix:?}");
    assert!(matches!(fix, Color::Rgb { .. }));
    assert_eq!(in_gamut(&mut app, &Color::rgb8(128, 128, 128)), None);
    // The dialog and the Color panel ask through a cache.
    let ctx = egui::Context::default();
    assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &blue), Some(fix));
    assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &blue), Some(fix));
    assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &Color::rgb8(128, 128, 128)), None);
}

/// The selected art takes each colour as it is picked; OK keeps the last as one undo step and
/// Cancel puts the art back.
#[test]
fn the_selection_shows_the_colour_live() {
    let mut app = app();
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let before = undo(&app);
    app.run("ui.colorPicker", json!({})).unwrap();
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previews before the colour changes");
    for hex in ["ff0000", "00ff00"] {
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!(hex));
        frame(&mut app);
        assert_eq!(fill_hex(&app), format!("#{hex}"), "live, before OK");
    }
    super::super::confirm(&mut app).unwrap();
    assert_eq!(fill_hex(&app), "#00ff00");
    assert_eq!(undo(&app), before + 1, "one step");
    // Cancel: back to the colour it had.
    app.run("ui.colorPicker", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
    frame(&mut app);
    assert_eq!(fill_hex(&app), "#0000ff");
    super::super::cancel(&mut app);
    assert_eq!(fill_hex(&app), "#00ff00");
    assert_eq!(undo(&app), before + 1);
}

/// Opened for an artboard (Properties › Artboards › Fill), the picker starts from the artboard's
/// background color (white without one) and shows each color on its paper live, with nothing
/// selected. OK keeps the last as one undo step, the art's fill unchanged, and Cancel puts the
/// artboard back.
#[test]
fn an_artboard_shows_its_background_color_live() {
    let mut app = app();
    app.run("select.set", json!({"ids": []})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let art = app.session.doc().unwrap().doc.layers.clone();
    let before = undo(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "FFFFFF", "starts from white paper");
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previews before the color changes");
    for hex in ["ff0000", "00ff00"] {
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!(hex));
        frame(&mut app);
        assert_eq!(color(&app), Some(format!("#{hex}")), "live, before OK");
    }
    super::super::confirm(&mut app).unwrap();
    assert_eq!(color(&app).as_deref(), Some("#00ff00"));
    assert_eq!(undo(&app), before + 1, "one step");
    assert_eq!(app.session.doc().unwrap().doc.layers, art, "the art keeps its fill");
    // Cancel: back to the color it had, which the picker starts from.
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "00FF00");
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
    frame(&mut app);
    assert_eq!(color(&app).as_deref(), Some("#0000ff"));
    super::super::cancel(&mut app);
    assert_eq!(color(&app).as_deref(), Some("#00ff00"));
    assert_eq!(undo(&app), before + 1);
    assert!(app.run("ui.colorPicker", json!({"artboard": 9})).is_err());
}

/// OK on a picker opened for an artboard without a background color, with the color left at the
/// white it starts from, leaves the artboard without one and adds no undo step. A white the
/// opener passes as `color` is set.
#[test]
fn an_untouched_picker_leaves_an_artboard_without_a_color() {
    let mut app = app();
    app.run("artboard.new", json!({})).unwrap();
    let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let before = undo(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "OK closes the picker");
    assert_eq!(color(&app), None, "no color");
    assert_eq!(undo(&app), before, "no undo step");
    app.run("ui.colorPicker", json!({"artboard": 1, "color": "#ffffff"})).unwrap();
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(color(&app).as_deref(), Some("#ffffff"));
    assert_eq!(undo(&app), before + 1);
}

/// OK on an untouched picker opened for an artboard with an RGB background color in a CMYK
/// document keeps the RGB color and adds no undo step. A color picked in it is set as CMYK.
#[test]
fn an_untouched_picker_keeps_an_artboards_rgb_color_in_a_cmyk_document() {
    let mut app = app();
    app.run("artboard.new", json!({})).unwrap();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
    let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color;
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let start = color(&app);
    assert!(matches!(start, Some(Color::Rgb { .. })), "set as RGB");
    let before = undo(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "OK closes the picker");
    assert_eq!(color(&app), start, "the RGB color is kept");
    assert_eq!(undo(&app), before, "no undo step");
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("00ff00"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(matches!(color(&app), Some(Color::Cmyk { .. })), "a picked color is set as CMYK");
    assert_eq!(undo(&app), before + 1);
}

/// OK on an untouched picker opened for an artboard with a background color in an RGB
/// document keeps the color and adds no undo step.
#[test]
fn an_untouched_picker_adds_no_undo_step_for_an_artboards_color() {
    let mut app = app();
    app.run("artboard.new", json!({})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
    let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let before = undo(&app);
    app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "OK closes the picker");
    assert_eq!(color(&app).as_deref(), Some("#ff0000"));
    assert_eq!(undo(&app), before, "no undo step");
}

#[test]
fn open_set_hex_and_confirm_fills() {
    let mut app = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(super::super::DialogKind::of(&d.kind), Some(super::super::DialogKind::ColorPicker));
    assert_eq!(d.str("hex"), "FFFFFF", "starts from the fill");
    assert!(!d.bool("stroke"));
    frame(&mut app);
    // `ui.dialog.set {field: "hex"}` then `ui.dialog.confirm`.
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("00ff00"));
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "00FF00");
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(fill_hex(&app), "#00ff00");
    // Without a frame in between, a set hex still wins; `color` works as well.
    app.run("ui.colorPicker", json!({"stroke": true})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("#3366CC"));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(crate::panels::current_paints(&app).1.color().unwrap().to_hex(), "#3366cc");
    // The stroke proxy is in front now: the opener follows it unless told otherwise.
    app.run("ui.colorPicker", json!({})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().bool("stroke"));
    app.run("ui.colorPicker", json!({"stroke": false})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!({"c": 0, "m": 1, "y": 1, "k": 0}));
    super::super::confirm(&mut app).unwrap();
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK is kept");
    assert!(app.run("ui.colorPicker", json!({"color": "nope"})).is_err());
    // Colours take every form the paint commands take (CMYK in percent too).
    app.run("ui.colorPicker", json!({"stroke": false, "color": {"c": 0, "m": 100, "y": 100, "k": 0}})).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert_eq!(crate::panels::current_paints(&app).0.color(), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
}

/// #1692: Tab goes through one color model's fields before the next: H, S, B, then R, G, B, the
/// hex field, then L, a, b, then C, M, Y, K. Each Lab or CMYK field is level with the field on
/// its left.
#[test]
fn tab_goes_through_one_color_models_fields_before_the_next() {
    let mut app = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // The picker opens with the keyboard in H. The loop presses Tab until K has the keyboard
    // and records each stop. A stop is `None` when no text field has the keyboard, as at a
    // channel's radio button.
    let mut texts = vec![];
    for _ in 0..3 {
        texts = frame_in(&ctx, &mut app, vec![]);
    }
    let mut stops = vec![focused_field(&ctx, &texts)];
    while stops.len() < 40 && stops.last().and_then(|s| s.as_ref()).is_none_or(|(label, _)| label != "K") {
        texts = frame_in(&ctx, &mut app, vec![tab.clone()]);
        stops.push(focused_field(&ctx, &texts));
    }
    let names: Vec<Option<&str>> = stops.iter().map(|s| s.as_ref().map(|(label, _)| label.as_str())).collect();
    let fields: Vec<&str> = names.iter().flatten().copied().collect();
    assert_eq!(fields, ["H", "S", "B", "R", "G", "B", "#", "L", "a", "b", "C", "M", "Y", "K"], "{names:?}");
    assert_eq!(names[names.len() - 2..], [Some("Y"), Some("K")], "one Tab takes Y to K");
    let found: Vec<&(String, Rect)> = stops.iter().flatten().collect();
    for (left, right) in found[..7].iter().zip(&found[7..]) {
        // Two grids: their rows land on the same subpixel-rounded line, not necessarily bit-equal.
        assert!((left.1.center().y - right.1.center().y).abs() <= 0.5, "{} and {} are level", left.0, right.0);
    }
}

#[test]
fn every_mode_draws_and_cmyk_documents_get_cmyk() {
    let mut app = app();
    for fields in
        [json!({"channel": "red"}), json!({"channel": "brightness", "webOnly": true}), json!({"swatches": true}), json!({"color": "#0000ff"})]
    {
        app.run("ui.colorPicker", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in fields.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
    }
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("ui.colorPicker", json!({"color": "#ff0000"})).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })));
}
/// Inspect actual painted controls, rather than a second implementation of the layout.
#[derive(Default)]
struct Drawn {
    texts: Vec<(String, Rect)>,
    visible_texts: Vec<String>,
    boxes: Vec<Rect>,
    fields: Vec<Rect>,
}

impl Drawn {
    /// Collects painted text, control bounds and color meshes, recording text visibility
    /// against the clip rectangle so tests can detect clipped labels.
    fn collect(&mut self, shape: &egui::Shape, clip: Rect) {
        match shape {
            egui::Shape::Text(t) => {
                self.texts.push((t.galley.text().into(), t.visual_bounding_rect()));
                if clip.expand(1.0).contains_rect(t.visual_bounding_rect()) {
                    self.visible_texts.push(t.galley.text().into());
                }
            }
            egui::Shape::Rect(r) => self.boxes.push(r.rect),
            egui::Shape::Mesh(m) if m.vertices.len() > 100 => self.fields.push(m.calc_bounds()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| self.collect(s, clip)),
            _ => {}
        }
    }

    /// Returns the bounds of exactly one painted label, failing on missing or duplicate text.
    fn text(&self, label: &str) -> Rect {
        let found: Vec<_> = self.texts.iter().filter(|(s, _)| s == label).collect();
        assert_eq!(found.len(), 1, "one {label}, including no duplicate footer: {:?}", self.texts);
        found[0].1
    }

    /// Finds the smallest painted control containing a unique label's center.
    fn control(&self, label: &str) -> Rect {
        let center = self.text(label).center();
        self.boxes.iter().filter(|r| r.contains(center)).min_by(|a, b| a.area().total_cmp(&b.area())).copied().unwrap()
    }
}

/// Runs a dialog frame with real egui input at a physical window size, accounting for UI zoom,
/// and returns the painted controls for layout and interaction assertions.
fn draw(ctx: &egui::Context, app: &mut VectorcraftApp, size: egui::Vec2, events: Vec<egui::Event>) -> Drawn {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size / ctx.zoom_factor())), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        super::super::show(app, ui.ctx());
    });
    let mut drawn = Drawn::default();
    for shape in &out.shapes {
        drawn.collect(&shape.shape, shape.clip_rect);
    }
    out.textures_delta.clear();
    drawn
}

/// The physical window size used for pointer and keyboard interaction tests.
fn screen() -> egui::Vec2 {
    vec2(1280.0, 900.0)
}

/// Lets the auto-sized dialog settle over six frames and returns the next frame's controls.
fn settled(ctx: &egui::Context, app: &mut VectorcraftApp, size: egui::Vec2) -> Drawn {
    for _ in 0..6 {
        draw(ctx, app, size, vec![]);
    }
    draw(ctx, app, size, vec![])
}

/// Creates a persistent test context with the application's fonts and default theme.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

/// Sends a primary pointer press and release in separate frames at the given position.
fn click_at(ctx: &egui::Context, app: &mut VectorcraftApp, at: egui::Pos2) {
    for pressed in [true, false] {
        draw(
            ctx,
            app,
            screen(),
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ],
        );
    }
}

/// Creates an unmodified, non-repeating key press for dialog shortcut tests.
fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

/// Checks accessible action names and each channel slider's displayed value, range and units.
#[test]
fn compact_picker_preserves_screen_reader_controls_and_channel_values() {
    use egui::accesskit::Role;

    for (channel, value, maximum, text) in [
        (Channel::Hue, 210.0, 360.0, "210°"),
        (Channel::Saturation, 67.0, 100.0, "67%"),
        (Channel::Brightness, 60.0, 100.0, "60%"),
        (Channel::Red, 51.0, 255.0, "51"),
        (Channel::Green, 102.0, 255.0, "102"),
        (Channel::Blue, 153.0, 255.0, "153"),
    ] {
        let mut app = app();
        app.services.screen_color = Some(Box::new(|| Ok(std::sync::mpsc::channel().1)));
        open(&mut app, &json!({"color": "#336699"})).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        d.fields.insert("channel".into(), json!(channel.id()));
        let tree = widgets::accessibility_tests::tree(|ui| {
            body(&mut app, ui, &mut d);
        });
        let slider = tree.iter().find(|n| n.role() == Role::Slider && n.label() == Some(channel.label())).unwrap();
        assert_eq!(slider.numeric_value(), Some(value));
        assert_eq!(slider.min_numeric_value(), Some(0.0));
        assert_eq!(slider.max_numeric_value(), Some(maximum));
        assert_eq!(slider.value(), Some(text));
        for label in ["OK", "Cancel", "Color Swatches", "Sample Color from Screen"] {
            assert!(tree.iter().any(|n| n.role() == Role::Button && n.label() == Some(label)), "missing button: {label}");
        }
    }
}

/// Checks aligned actions and input rows at two UI scales, and stable dimensions when
/// switching between color models and swatches.
#[test]
fn inline_actions_are_aligned_and_modes_keep_the_window_size() {
    // Language selection is process-global; German is checked in the native app instead
    // of racing the other parallel UI tests by changing the locale here.
    for zoom in [1.0, 1.5] {
        let (ctx, mut app) = (context(), app());
        ctx.set_zoom_factor(zoom);
        app.run("ui.colorPicker", json!({})).unwrap();
        let drawn = settled(&ctx, &mut app, screen());
        let ok = drawn.control(tl!("OK"));
        let cancel = drawn.control(tl!("Cancel"));
        let toggle = drawn.control(tl!("Color Swatches"));
        assert!(ok.bottom() < cancel.top() && cancel.bottom() < toggle.top());
        for r in [cancel, toggle] {
            assert!((r.left() - ok.left()).abs() < 1.0 && (r.width() - ok.width()).abs() < 1.0);
        }
        let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
        assert!((field.width() - field.height()).abs() < 1.0);
        assert!((field.top() - ok.top()).abs() < 1.0);
        let k = drawn.text("K");
        let last_input = drawn
            .boxes
            .iter()
            .find(|r| (r.width() - VALUE_W).abs() < 0.5 && r.left() > k.right() && r.contains(egui::pos2(r.center().x, k.center().y)))
            .unwrap();
        assert!((last_input.bottom() - field.bottom()).abs() < 2.0, "{last_input:?} vs {field:?}");
        // Fonts and translated actions may leave one normal gutter, but never an unused column.
        assert!((last_input.right() - ok.right()).abs() < COLUMN_GAP, "no unused column beside the inputs: {last_input:?} vs {ok:?}");
        let id = egui::Id::unique(("dialog", KIND));
        let before = ctx.memory(|m| m.area_rect(id)).unwrap();
        assert!(drawn.boxes.iter().all(|r| r.left() >= before.left() - 1.0 && r.right() <= before.right() + 1.0), "controls fit horizontally");
        let title = drawn.text(tl!("Color Picker"));
        assert!((title.center().x - before.center().x).abs() < 1.0, "centred heading");
        assert!(title.bottom() < field.top(), "a separate title band above the content");
        assert!(drawn.visible_texts.iter().any(|s| s == tl!("Only Web Colors")), "the checkbox label is fully visible");
        assert!(drawn.text("L").top() - toggle.bottom() >= PREVIEW_GAP, "space below the actions");
        click_at(&ctx, &mut app, toggle.center());
        assert!(app.ui.dialog.as_ref().unwrap().bool("swatches"));
        let drawn = settled(&ctx, &mut app, screen());
        assert!((drawn.control(tl!("Color Models")).width() - ok.width()).abs() < 1.0);
        let after = ctx.memory(|m| m.area_rect(id)).unwrap();
        assert!((before.size() - after.size()).length() < 1.0, "{before:?} vs {after:?}");
        assert!(drawn.boxes.iter().all(|r| r.left() >= after.left() - 1.0 && r.right() <= after.right() + 1.0), "swatch controls fit horizontally");
        click_at(&ctx, &mut app, drawn.text(tl!("Color Models")).center());
        assert!(!app.ui.dialog.as_ref().unwrap().bool("swatches"));
    }
}

/// Clicks the sampler above the field, receives a color and checks Cancel restores the paint.
#[test]
fn sampler_above_the_field_previews_and_cancel_restores_the_original() {
    let (ctx, mut app) = (context(), app());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut receiver = Some(rx);
    app.services.screen_color = Some(Box::new(move || receiver.take().ok_or("sampler already started".into())));
    app.run("ui.colorPicker", json!({})).unwrap();
    let original = fill_hex(&app);
    let drawn = settled(&ctx, &mut app, screen());
    let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
    let hover = egui::pos2(field.right() - 12.0, field.top() - 20.0);
    draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(hover)]);
    let drawn = settled(&ctx, &mut app, screen());
    let sampler = drawn.boxes.iter().find(|r| r.size() == vec2(24.0, 24.0) && r.bottom() < field.top()).unwrap();
    assert!(sampler.top() > drawn.text(tl!("Color Picker")).bottom() && sampler.right() <= field.right());
    click_at(&ctx, &mut app, sampler.center());
    assert!(app.screen_sampler.is_some(), "the button above the field starts the host sampler");
    tx.send(Ok(Some([0.0, 1.0, 0.0]))).unwrap();
    crate::screen_color::poll(&mut app, &ctx);
    let drawn = settled(&ctx, &mut app, screen());
    assert_eq!(fill_hex(&app), "#00ff00");
    click_at(&ctx, &mut app, drawn.text(tl!("Cancel")).center());
    assert!(app.ui.dialog.is_none());
    assert_eq!(fill_hex(&app), original);
}

/// Checks both correction warnings fit below the preview without widening the dialog,
/// and that clicking the web warning still corrects the color.
#[test]
fn warnings_fit_below_the_preview_without_widening_the_dialog() {
    let (ctx, mut app) = (context(), app());
    app.run("ui.colorPicker", json!({"color":"#ffffff"})).unwrap();
    settled(&ctx, &mut app, screen());
    let id = egui::Id::unique(("dialog", KIND));
    let before = ctx.memory(|m| m.area_rect(id)).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000FE"));
    let drawn = settled(&ctx, &mut app, screen());
    let ok = drawn.control(tl!("OK"));
    let toggle = drawn.control(tl!("Color Swatches"));
    let h = drawn.text("H");
    let mut warnings: Vec<_> =
        drawn.boxes.iter().filter(|r| (r.width() - 16.0).abs() < 0.5 && (r.height() - 16.0).abs() < 0.5 && r.bottom() < h.top()).copied().collect();
    warnings.dedup(); // The tile's fill and border share one rectangle.
    assert_eq!(warnings.len(), 2, "gamut and web correction chips are both present");
    for r in &warnings {
        assert!(r.right() < ok.left() && r.bottom() <= toggle.bottom());
        assert!(r.top() > ok.top() + CHIP_H, "warnings below the preview");
    }
    let after = ctx.memory(|m| m.area_rect(id)).unwrap();
    assert!((before.size() - after.size()).length() < 1.0, "{before:?} vs {after:?}");
    click_at(&ctx, &mut app, warnings[1].center());
    assert!(is_web_safe(&picked(app.ui.dialog.as_ref().unwrap()).color), "web correction remains clickable");
}

/// Drags the custom title band and checks the dialog moves by the pointer's displacement.
#[test]
fn colour_picker_moves_by_its_title_band() {
    let (ctx, mut app) = (context(), app());
    app.run("ui.colorPicker", json!({})).unwrap();
    let drawn = settled(&ctx, &mut app, screen());
    let title = drawn.text(tl!("Color Picker")).center();
    let id = egui::Id::unique(("dialog", KIND));
    let before = ctx.memory(|m| m.area_rect(id)).unwrap();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(title), pointer(title, true)]);
    for i in 1..=4 {
        draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(title + vec2(6.0, 4.0) * i as f32)]);
    }
    draw(&ctx, &mut app, screen(), vec![pointer(title + vec2(24.0, 16.0), false)]);
    settled(&ctx, &mut app, screen());
    let after = ctx.memory(|m| m.area_rect(id)).unwrap();
    assert!((after.center() - before.center() - vec2(24.0, 16.0)).length() < 1.5);
    assert!(app.ui.dialog.is_some());
}

/// Checks OK and Enter commit typed values as one undo step, while Cancel and Escape
/// restore the original paint after a live preview.
#[test]
fn inline_ok_commits_typed_input_and_cancel_restores_the_preview() {
    for enter in [false, true] {
        let (ctx, mut app) = (context(), app());
        app.run("ui.colorPicker", json!({"color": "#ff0000"})).unwrap();
        let drawn = settled(&ctx, &mut app, screen());
        let undo = app.session.doc().unwrap().history.undo.len();
        // The first numeric field has selected text, just as in the other form dialogs.
        draw(&ctx, &mut app, screen(), vec![egui::Event::Text("120".into())]);
        if enter {
            draw(&ctx, &mut app, screen(), vec![key(egui::Key::Enter)]);
        } else {
            click_at(&ctx, &mut app, drawn.text(tl!("OK")).center());
        }
        assert!(app.ui.dialog.is_none());
        assert_eq!(fill_hex(&app), "#00ff00");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        app.run("ui.colorPicker", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
        let drawn = settled(&ctx, &mut app, screen());
        assert_eq!(fill_hex(&app), "#0000ff");
        if enter {
            draw(&ctx, &mut app, screen(), vec![key(egui::Key::Escape)]);
        } else {
            click_at(&ctx, &mut app, drawn.text(tl!("Cancel")).center());
        }
        assert!(app.ui.dialog.is_none());
        assert_eq!(fill_hex(&app), "#00ff00");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }
}

/// Checks shrinking and stacking keep the dialog inside small windows, vertical scrolling
/// reaches the actions and last input, and Enter can still confirm.
#[test]
fn compact_picker_shrinks_then_scrolls_without_enlarging_the_window() {
    for size in [vec2(740.0, 600.0), vec2(540.0, 420.0), vec2(400.0, 280.0)] {
        let (ctx, mut app) = (context(), app());
        app.run("ui.colorPicker", json!({"color": "#0000ff"})).unwrap();
        let drawn = settled(&ctx, &mut app, size);
        let outer = ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", KIND)))).unwrap();
        assert!(outer.width() <= size.x && outer.height() <= size.y, "{outer:?} in {size:?}");
        let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
        assert!((MIN_FIELD..=FIELD).contains(&field.width()));
        assert!((field.width() - field.height()).abs() < 1.0);
        assert!(drawn.boxes.iter().all(|r| r.left() >= outer.left() - 1.0 && r.right() <= outer.right() + 1.0), "controls fit horizontally");
        if size.x < 500.0 {
            assert!(drawn.control(tl!("OK")).top() > field.bottom(), "narrow windows stack the controls below the field");
            let mut actions_visible = false;
            let mut last_input_visible = false;
            for _ in 0..30 {
                draw(
                    &ctx,
                    &mut app,
                    size,
                    vec![
                        egui::Event::PointerMoved(outer.min + vec2(30.0, 80.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0.0, -20.0),
                            phase: egui::TouchPhase::Move,
                            modifiers: Default::default(),
                            source: egui::MouseWheelSource::Unknown,
                        },
                    ],
                );
                let scrolled = settled(&ctx, &mut app, size);
                actions_visible |= scrolled.visible_texts.iter().any(|s| s == tl!("OK"));
                last_input_visible |= scrolled.visible_texts.iter().any(|s| s == "K");
            }
            assert!(actions_visible && last_input_visible, "vertical scrolling reveals actions and the last input");
        }
        // Keyboard confirmation remains available even when the actions require scrolling.
        draw(&ctx, &mut app, size, vec![key(egui::Key::Enter)]);
        assert!(app.ui.dialog.is_none());
    }
}
