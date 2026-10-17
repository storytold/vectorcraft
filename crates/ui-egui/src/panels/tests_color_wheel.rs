//! The Color Wheel panel drawn headless: Window › Color Wheel, picks on the hue ring and the
//! square, Shift and Alt, gradient stops, None, the remembered hue, the color model of a pick, the
//! arrow keys and what screen readers read.

use egui::accesskit::{Action, ActionData, ActionRequest};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use vectorcraft_color::cms::{self, Cms, ColorSettings, delta_e2000};
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::Session;

use super::color_wheel::{ID, display_hsb, show, stored};
use crate::VectorcraftApp;

/// An app with a selected rectangle filled with `fill`.
fn app(fill: serde_json::Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app
}

/// The panel in a 260-point-wide flyout, with the app's shortcuts after it as in a frame of the app.
fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            crate::shortcuts::handle(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}

fn rect_of(h: &Harness<'static, VectorcraftApp>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// The point at hue `deg` on the ring drawn in `ring`, `out` points outside the middle of its band.
fn on_ring(ring: Rect, deg: f32, out: f32) -> Pos2 {
    let r = ring.width() / 2.0 - 4.0 - 7.0 + out;
    let a = deg.to_radians();
    ring.center() + vec2(a.cos(), -a.sin()) * r
}

/// The point (x across, y up; 0..1 each) of the square.
fn in_square(sq: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(sq.left() + x * sq.width(), sq.bottom() - y * sq.height())
}

fn press(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, modifiers: Modifiers) {
    press(h, pos, true, modifiers);
    press(h, pos, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// A click whose press and release come in one frame (`Harness::event` gives each event a frame of
/// its own).
fn quick_click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([button(true), button(false)]);
    h.run_steps(2);
}

/// A drag through `points`, pressed at the first and released at the last.
fn drag(h: &mut Harness<'static, VectorcraftApp>, points: &[Pos2], modifiers: Modifiers) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return };
    press(h, *first, true, modifiers);
    for p in points {
        for _ in 0..2 {
            h.event(Event::PointerMoved(*p));
            h.run_steps(1);
        }
    }
    press(h, *last, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

fn key(h: &mut Harness<'static, VectorcraftApp>, key: egui::Key, modifiers: Modifiers) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(1);
}

fn fill(app: &VectorcraftApp) -> Color {
    crate::panels::current_paints(app).0.color().expect("a solid fill")
}

fn hsb(c: &Color) -> [f32; 3] {
    display_hsb(&cms::active(), c)
}

fn near(a: [f32; 3], b: [f32; 3], tol: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol[i])
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// Window › Color Wheel opens the panel, and a click on the left of the ring turns a red fill cyan
/// at the same saturation and brightness.
#[test]
fn window_color_wheel_opens_the_panel_and_the_ring_picks_the_hue() {
    let mut app = app(json!("#ff0000"));
    assert_eq!(app.run("window.panel", json!({"panel": ID})).unwrap()["open"], ID);
    assert_eq!(app.ui.open_panel.as_deref(), Some(ID));
    assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "Color Wheel"));
    let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.path == ["Window"] && e.label == "Color Wheel");
    assert!(item.is_some(), "the Window menu lists it");
    let mut h = harness(app);
    let ring = rect_of(&h, "Hue ring");
    let undo = undo_len(h.state());
    click(&mut h, on_ring(ring, 180.0, 0.0), Modifiers::NONE);
    assert_eq!(fill(h.state()).to_hex(), "#00ffff");
    assert_eq!(undo_len(h.state()), undo + 1, "one undo step");
}

#[test]
fn the_square_picks_saturation_across_and_brightness_up() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3])));
    let sq = rect_of(&h, "Saturation and brightness");
    click(&mut h, sq.right_top() + vec2(-0.5, 0.5), Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [0.0, 1.0, 1.0], [0.5, 0.01, 0.01]), "{:?}", hsb(&fill(h.state())));
    click(&mut h, in_square(sq, 0.25, 0.5), Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [0.0, 0.25, 0.5], [0.5, 0.01, 0.01]), "{:?}", hsb(&fill(h.state())));
}

/// A press between the square and the ring's inner edge picks the closest point of the square.
#[test]
fn a_press_inside_the_ring_beside_the_square_picks_its_closest_point() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3])));
    let sq = rect_of(&h, "Saturation and brightness");
    // 1.4 points beyond the corner along the diagonal: the band's inner edge is 2 points out.
    click(&mut h, sq.right_top() + vec2(1.0, -1.0), Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [0.0, 1.0, 1.0], [0.5, 1e-4, 1e-4]), "{:?}", hsb(&fill(h.state())));
}

/// The part a press starts on keeps the drag: from the ring into the square only the hue changes.
#[test]
fn a_drag_from_the_ring_into_the_square_changes_only_the_hue() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3])));
    let (ring, sq) = (rect_of(&h, "Hue ring"), rect_of(&h, "Saturation and brightness"));
    let before = hsb(&fill(h.state()));
    drag(&mut h, &[on_ring(ring, 90.0, 0.0), sq.center() + vec2(-10.0, 0.0)], Modifiers::NONE);
    let after = hsb(&fill(h.state()));
    assert!(near(after, [180.0, before[1], before[2]], [0.5, 1e-3, 1e-3]), "{before:?} → {after:?}");
}

/// A press up to 4 points outside the band still picks a hue; one in the rect's corner does nothing.
#[test]
fn a_press_just_outside_the_ring_picks_a_hue_and_the_corners_do_nothing() {
    let mut h = harness(app(json!("#ff0000")));
    let ring = rect_of(&h, "Hue ring");
    click(&mut h, on_ring(ring, 180.0, 7.0 + 3.0), Modifiers::NONE);
    assert_eq!(fill(h.state()).to_hex(), "#00ffff");
    click(&mut h, ring.left_top() + vec2(2.0, 2.0), Modifiers::NONE);
    assert_eq!(fill(h.state()).to_hex(), "#00ffff");
}

/// Each click takes the part of the ring under it, also when its press and release come in one
/// frame, and a click on the band gives the ring the keyboard focus.
#[test]
fn a_click_in_one_frame_takes_the_part_under_it() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3])));
    let (ring, sq) = (rect_of(&h, "Hue ring"), rect_of(&h, "Saturation and brightness"));
    click(&mut h, ring.left_top() + vec2(2.0, 2.0), Modifiers::NONE);
    quick_click(&mut h, on_ring(ring, 120.0, 0.0));
    assert!(near(hsb(&fill(h.state())), [120.0, 0.5, 0.6], [0.5, 1e-3, 1e-3]), "after a corner: {:?}", hsb(&fill(h.state())));
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [121.0, 0.5, 0.6], [0.5, 1e-3, 1e-3]), "focused: {:?}", hsb(&fill(h.state())));
    quick_click(&mut h, sq.right_top() + vec2(1.0, -1.0));
    assert!(near(hsb(&fill(h.state())), [121.0, 1.0, 1.0], [0.5, 1e-3, 1e-3]), "after the band: {:?}", hsb(&fill(h.state())));
}

#[test]
fn shift_drag_on_the_ring_steps_the_hue_by_45_degrees() {
    let mut h = harness(app(json!("#ff0000")));
    let ring = rect_of(&h, "Hue ring");
    drag(&mut h, &[on_ring(ring, 10.0, 0.0), on_ring(ring, 60.0, 0.0), on_ring(ring, 100.0, 0.0)], Modifiers::SHIFT);
    assert!(near(hsb(&fill(h.state())), [90.0, 1.0, 1.0], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
}

#[test]
fn alt_click_paints_the_stroke_and_the_fill_stays_in_front() {
    let mut app = app(json!("#ff0000"));
    app.run("paint.setStroke", json!({"color": "#808080", "focus": false})).unwrap();
    assert!(app.session.fill_active);
    let mut h = harness(app);
    let ring = rect_of(&h, "Hue ring");
    click(&mut h, on_ring(ring, 120.0, 0.0), Modifiers::ALT);
    let (f, s) = crate::panels::current_paints(h.state());
    assert_eq!((f.color().unwrap().to_hex(), s.color().unwrap().to_hex()), ("#ff0000".into(), "#00ff00".into()));
    assert!(h.state().session.fill_active, "the fill stays in front");
}

#[test]
fn a_pick_recolors_the_selected_gradient_stop() {
    let mut app = app(json!("#ff0000"));
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let mut h = harness(app);
    let ring = rect_of(&h, "Hue ring");
    click(&mut h, on_ring(ring, 120.0, 0.0), Modifiers::NONE);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let hex: Vec<String> = g.gradient.stops.iter().map(|s| s.color.to_hex()).collect();
    assert_eq!(hex, ["#ff0000", "#00ff00"]);
}

/// With None there is no color to edit: no markers, and a click sets the fill.
#[test]
fn with_a_none_fill_the_wheel_has_no_marker_and_a_click_sets_the_fill() {
    let mut app = app(json!("#ff0000"));
    app.run("paint.setFill", json!({"none": true})).unwrap();
    let mut h = harness(app);
    assert_eq!(h.get_by_label("Saturation and brightness").value(), None, "no marker");
    let sq = rect_of(&h, "Saturation and brightness");
    click(&mut h, in_square(sq, 0.5, 0.5), Modifiers::NONE);
    let c = fill(h.state());
    assert!(near(hsb(&c), [hsb(&h.state().session.last_solid)[0], 0.5, 0.5], [0.5, 0.01, 0.01]), "{:?}", hsb(&c));
}

/// The hue stays while the square passes through a gray: a drag to the left edge (white, whose own
/// hue is 0°) and back ends at the color a click at its end gives, for an RGB and for a CMYK fill
/// (whose stored color differs from the picked RGB), and an RGB green is green again.
#[test]
fn a_gray_keeps_the_hue_it_was_dragged_from() {
    for fill_json in [json!("#00ff00"), json!({"c": 0.8, "m": 0.1, "y": 0.9, "k": 0.0})] {
        let mut h = harness(app(fill_json.clone()));
        let before = hsb(&fill(h.state()));
        let sq = rect_of(&h, "Saturation and brightness");
        // Inside the square's edges; the drag past its left edge picks saturation 0.
        let at = |x: f32| in_square(sq, x.clamp(0.005, 0.995), before[2].clamp(0.005, 0.995));
        let gray = Pos2::new(sq.left() - 5.0, at(0.0).y);
        drag(&mut h, &[at(before[1]), gray, at(before[1])], Modifiers::NONE);
        let dragged = fill(h.state());
        let mut direct = harness(app(fill_json.clone()));
        click(&mut direct, at(before[1]), Modifiers::NONE);
        assert_eq!(dragged.to_hex(), fill(direct.state()).to_hex(), "{fill_json}: {before:?} → {:?}", hsb(&dragged));
        assert!((hsb(&dragged)[0] - before[0]).abs() < 5.0, "{fill_json}: {before:?} → {:?}", hsb(&dragged));
    }
    let mut h = harness(app(json!("#00ff00")));
    let sq = rect_of(&h, "Saturation and brightness");
    drag(&mut h, &[in_square(sq, 0.995, 0.995), Pos2::new(sq.left() - 5.0, sq.top() + 1.0), in_square(sq, 0.995, 0.995)], Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [120.0, 1.0, 1.0], [0.5, 0.01, 0.01]), "{:?}", hsb(&fill(h.state())));
}

/// The remembered hue belongs to the object it was picked on: another object of the same color
/// shows its own hue.
#[test]
fn the_remembered_hue_stays_with_its_object() {
    let mut app = app(json!("#00ff00"));
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"color": "#ffffff", "ids": [b]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let mut h = harness(app);
    let sq = rect_of(&h, "Saturation and brightness");
    // Green to white (saturation 0 at the top edge): the wheel keeps hue 120°.
    drag(&mut h, &[in_square(sq, 0.995, 0.995), Pos2::new(sq.left() - 5.0, sq.top() - 3.0)], Modifiers::NONE);
    assert_eq!(fill(h.state()), Color::WHITE);
    h.state_mut().run("select.set", json!({"ids": [b]})).unwrap();
    h.run_steps(1);
    click(&mut h, in_square(sq, 0.995, 0.995), Modifiers::NONE);
    let hue = hsb(&fill(h.state()))[0];
    assert!(!(1.0..359.0).contains(&hue), "the white object's own hue is 0°, not 120°: {hue}");
}

/// A CMYK fill stays CMYK and keeps its look (ΔE 2000 under 2) when its marker is clicked; the
/// working profile separates the picked color again, so its inks can move.
#[test]
fn a_cmyk_fill_stays_cmyk_and_keeps_its_look() {
    let [c, m, y, k] = Color::rgb(0.2, 0.5, 0.7).to_cmyk_managed(cms::active().settings().intent);
    for inks in [[c, m, y, k], [0.2, 0.2, 0.2, 0.0]] {
        let mut h = harness(app(json!({"c": inks[0], "m": inks[1], "y": inks[2], "k": inks[3]})));
        let before = fill(h.state());
        let [_, s, b] = hsb(&before);
        let sq = rect_of(&h, "Saturation and brightness");
        click(&mut h, in_square(sq, s, b), Modifiers::NONE);
        let after = fill(h.state());
        assert!(matches!(after, Color::Cmyk { .. }), "{after:?}");
        let cms = cms::active();
        assert!(delta_e2000(cms.lab(&before), cms.lab(&after)) < cms::GAMUT_THRESHOLD, "{before:?} → {after:?}");
    }
}

/// A Gray fill becomes a process color in the document's color mode.
#[test]
fn a_gray_fill_becomes_rgb_or_cmyk_with_the_document() {
    for (mode, rgb) in [("rgb", true), ("cmyk", false)] {
        let mut app = app(json!("#ff0000"));
        app.run("file.documentColorMode", json!({"mode": mode})).unwrap();
        app.run("paint.setFill", json!({"color": {"gray": 0.5}})).unwrap();
        let mut h = harness(app);
        let ring = rect_of(&h, "Hue ring");
        click(&mut h, on_ring(ring, 0.0, 0.0), Modifiers::NONE);
        let c = fill(h.state());
        assert_eq!(matches!(c, Color::Rgb { .. }), rgb, "{mode}: {c:?}");
        assert_eq!(matches!(c, Color::Cmyk { .. }), !rgb, "{mode}: {c:?}");
    }
}

/// In a working RGB space wider than sRGB a pick goes into it through the color settings, so a hue
/// change keeps the displayed saturation and brightness.
#[test]
fn a_pick_in_display_p3_keeps_the_components_it_does_not_change() {
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let was = Color::rgb(0.5, 0.3, 0.2);
    let [h, s, b] = display_hsb(&p3, &was);
    let new = stored(&p3, [h + 40.0, s, b], Some(&was), false);
    let [h2, s2, b2] = display_hsb(&p3, &new);
    assert!(matches!(new, Color::Rgb { .. }));
    assert!((h2 - (h + 40.0)).abs() < 1.0 && (s2 - s).abs() < 0.01 && (b2 - b).abs() < 0.01, "{:?} → {:?}", [h, s, b], [h2, s2, b2]);
}

/// A press gives the ring or the square the keyboard focus. The arrows then step the color, each
/// press one undo step, the app's tool keys wait, and after Escape they work again.
#[test]
fn the_arrow_keys_step_the_focused_ring_or_square() {
    let mut h = harness(app(json!("#ff0000")));
    let ring = rect_of(&h, "Hue ring");
    click(&mut h, on_ring(ring, 0.0, 0.0), Modifiers::NONE);
    let undo = undo_len(h.state());
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [1.0, 1.0, 1.0], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
    key(&mut h, egui::Key::ArrowUp, Modifiers::SHIFT);
    assert!(near(hsb(&fill(h.state())), [11.0, 1.0, 1.0], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
    assert_eq!(undo_len(h.state()), undo + 2, "a step each");
    key(&mut h, egui::Key::ArrowLeft, Modifiers::SHIFT);
    key(&mut h, egui::Key::ArrowDown, Modifiers::NONE);
    key(&mut h, egui::Key::ArrowDown, Modifiers::NONE);
    assert!(near(hsb(&fill(h.state())), [359.0, 1.0, 1.0], [1e-3, 1e-3, 1e-3]), "wraps: {:?}", hsb(&fill(h.state())));
    let sq = rect_of(&h, "Saturation and brightness");
    click(&mut h, in_square(sq, 0.5, 0.5), Modifiers::NONE);
    let before = hsb(&fill(h.state()));
    key(&mut h, egui::Key::ArrowUp, Modifiers::NONE);
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    let after = hsb(&fill(h.state()));
    assert!(near(after, [before[0], before[1] + 0.01, before[2] + 0.01], [1e-3, 1e-4, 1e-4]), "{before:?} → {after:?}");
    // The tool keys wait while a control has the focus.
    let tool = h.state().session.tool_id().to_string();
    let p = |h: &mut Harness<'static, VectorcraftApp>| {
        h.event(Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.event(Event::Text("p".into()));
        h.run_steps(1);
    };
    p(&mut h);
    assert_eq!(h.state().session.tool_id(), tool);
    key(&mut h, egui::Key::Escape, Modifiers::NONE);
    p(&mut h);
    assert_eq!(h.state().session.tool_id(), "pen");
}

/// Screen readers find the ring as a slider from 0 to 360 that their actions move, and the square
/// with both of its components.
#[test]
fn screen_readers_read_and_move_the_ring_and_the_square() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3])));
    let ring = h.get_by_label("Hue ring");
    let node = ring.accesskit_node();
    assert_eq!((node.min_numeric_value(), node.max_numeric_value(), node.numeric_value_step()), (Some(0.0), Some(360.0), Some(1.0)));
    assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)));
    assert_eq!(node.numeric_value(), Some(0.0));
    let (target_node, target_tree) = node.locate();
    assert_eq!(h.get_by_label("Saturation and brightness").value().as_deref(), Some("Saturation 50%, Brightness 60%"));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::Increment, data: None }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [1.0, 0.5, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
    let data = Some(ActionData::NumericValue(200.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [200.0, 0.5, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
}

/// The Color Picker's color field is the same widget: a press focuses it, the arrows step the
/// components it shows, and screen readers read its name and both components.
#[test]
fn the_color_pickers_field_takes_the_arrow_keys() {
    let mut app = app(json!([0.6, 0.3, 0.3]));
    app.run("ui.colorPicker", json!({})).unwrap();
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(3);
    let field = rect_of(&h, "Color field");
    assert_eq!(h.get_by_label("Color field").value().as_deref(), Some("Saturation 50%, Brightness 60%"));
    click(&mut h, field.center(), Modifiers::NONE);
    let picked = |h: &Harness<'static, VectorcraftApp>| -> [f32; 3] {
        serde_json::from_value(h.state().ui.dialog.as_ref().unwrap().fields["__hsb"].clone()).unwrap()
    };
    let before = picked(&h);
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    key(&mut h, egui::Key::ArrowDown, Modifiers::SHIFT);
    let after = picked(&h);
    assert!(near(after, [before[0], before[1] + 0.01, before[2] - 0.1], [1e-3, 1e-4, 1e-4]), "{before:?} → {after:?}");
    assert!(h.state().ui.dialog.is_some(), "the dialog stays open");
}
