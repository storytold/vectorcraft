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

use super::color_wheel::{
    ID, Settings, Sliders, chroma_tick, display_hsb, oklch_rows, oklch_track, remembered_hsb, remembered_hsl, remembered_oklch, set, show, stored,
};
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
    app.run("window.panel", json!({"panel": ID})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, ID));
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

/// With None the ring and the square still take a screen reader's actions and the arrow keys. They
/// start from the last solid color the wheel shows, and the result sets the fill.
#[test]
fn with_a_none_fill_screen_readers_and_the_arrow_keys_set_the_fill() {
    let mut app = app(json!("#ff0000"));
    app.run("paint.setFill", json!({"none": true})).unwrap();
    let mut h = harness(app);
    let last = hsb(&h.state().session.last_solid);
    let (target_node, target_tree) = h.get_by_label("Hue ring").accesskit_node().locate();
    let data = Some(ActionData::NumericValue(200.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [200.0, last[1], last[2]], [0.5, 2e-3, 2e-3]), "{:?}", hsb(&fill(h.state())));
    // A click focuses the square; with the fill set to None again the arrows still step it.
    let sq = rect_of(&h, "Saturation and brightness");
    click(&mut h, in_square(sq, 0.5, 0.5), Modifiers::NONE);
    h.state_mut().run("paint.setFill", json!({"none": true})).unwrap();
    h.run_steps(1);
    assert_eq!(h.get_by_label("Saturation and brightness").value(), None, "no marker");
    let last = hsb(&h.state().session.last_solid);
    key(&mut h, egui::Key::ArrowLeft, Modifiers::NONE);
    key(&mut h, egui::Key::ArrowDown, Modifiers::NONE);
    let after = hsb(&fill(h.state()));
    assert!(near(after, [last[0], last[1] - 0.01, last[2] - 0.01], [0.5, 2e-3, 2e-3]), "{last:?} → {after:?}");
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

/// Equal colors on two appearance fills have separate remembered hues. A square pick on the
/// second white fill uses its own 0° hue after the first fill was changed from green to white.
#[test]
fn the_remembered_hue_stays_with_its_appearance_item() {
    let mut app = app(json!("#00ff00"));
    let id = app.session.active().unwrap().selection.objects[0];
    app.run("appearance.addFill", json!({})).unwrap(); // [Fill, Stroke, Fill]
    app.run("paint.setFill", json!({"item": 2, "color": "#ffffff"})).unwrap();
    app.run("appearance.setActiveItem", json!({"index": 0})).unwrap();
    let mut h = harness(app);
    let sq = rect_of(&h, "Saturation and brightness");
    drag(&mut h, &[in_square(sq, 0.995, 0.995), Pos2::new(sq.left() - 5.0, sq.top() - 3.0)], Modifiers::NONE);
    let item_fill = |app: &VectorcraftApp, index| {
        app.session.doc().unwrap().doc.node(id).unwrap().appearance.fill_at(Some(index)).unwrap().paint.color().unwrap()
    };
    assert_eq!(item_fill(h.state(), 0), Color::WHITE);
    assert_eq!(item_fill(h.state(), 2), Color::WHITE);
    h.state_mut().run("appearance.setActiveItem", json!({"index": 2})).unwrap();
    h.run_steps(1);
    click(&mut h, in_square(sq, 0.995, 0.995), Modifiers::NONE);
    let hue = hsb(&item_fill(h.state(), 2))[0];
    assert!(!(1.0..359.0).contains(&hue), "the second fill's own hue is 0°, not 120°: {hue}");
    assert_eq!(item_fill(h.state(), 0), Color::WHITE);
}

/// The Type tool can recolor a selected character range without changing the selected object.
/// The next range of the same color starts with its own hue.
#[test]
fn the_remembered_hue_stays_with_its_text_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 100, "text": "AB", "color": "#00ff00"})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "fill": "#ffffff"})).unwrap();
    app.select_tool("type");
    app.session.set_tool_option("textSelection", &json!({"id": id, "start": 0, "end": 1}));
    let mut h = harness(app);
    let sq = rect_of(&h, "Saturation and brightness");
    drag(&mut h, &[in_square(sq, 0.995, 0.995), Pos2::new(sq.left() - 5.0, sq.top() - 3.0)], Modifiers::NONE);
    h.state_mut().session.set_tool_option("textSelection", &json!({"id": id, "start": 1, "end": 2}));
    h.run_steps(1);
    click(&mut h, in_square(sq, 0.995, 0.995), Modifiers::NONE);
    let node = h.state().session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!("still text") };
    assert_eq!(t.runs.first().unwrap().style.fill.color(), Some(Color::WHITE));
    let hue = hsb(&t.runs.last().unwrap().style.fill.color().unwrap())[0];
    assert!(!(1.0..359.0).contains(&hue), "the second character's own hue is 0°, not 120°: {hue}");
}

/// Replacing a gradient can keep the same selected stop and color while changing the paint it
/// belongs to. The new gradient starts with that stop's own hue.
#[test]
fn the_remembered_hue_stays_with_its_gradient() {
    let mut app = app(json!("#00ff00"));
    let first = json!([{"offset": 0, "color": "#00ff00"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": first}})).unwrap();
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let mut h = harness(app);
    let sq = rect_of(&h, "Saturation and brightness");
    drag(&mut h, &[in_square(sq, 0.995, 0.995), Pos2::new(sq.left() - 5.0, sq.top() - 3.0)], Modifiers::NONE);
    let second = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#ff0000"}]);
    h.state_mut().run("paint.setFill", json!({"gradient": {"stops": second}})).unwrap();
    assert_eq!(h.state().session.selected_stop(), Some(0));
    h.run_steps(1);
    click(&mut h, in_square(sq, 0.995, 0.995), Modifiers::NONE);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let hue = hsb(&g.gradient.stops[0].color)[0];
    assert!(!(1.0..359.0).contains(&hue), "the new gradient's white stop has hue 0°, not 120°: {hue}");
}

/// The remembered HSB holds only while the color displays as it did when picked: in another
/// working RGB space the wheel shows the color's own HSB there.
#[test]
fn the_remembered_hsb_follows_a_change_of_working_space() {
    let srgb = Cms::default();
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let c = Color::rgb(0.5, 0.3, 0.2);
    let picked = [25.0, 0.6, 0.5];
    let shown = srgb.display_rgb(&c);
    assert_eq!(remembered_hsb(&srgb, &c, shown, picked), picked);
    let own = display_hsb(&p3, &c);
    assert_ne!(own, picked);
    assert_eq!(remembered_hsb(&p3, &c, shown, picked), own);
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
/// with both of its components, whose actions move its saturation as → and ← do.
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
    let square = h.get_by_label("Saturation and brightness");
    let node = square.accesskit_node();
    assert_eq!((node.min_numeric_value(), node.max_numeric_value(), node.numeric_value_step()), (Some(0.0), Some(100.0), Some(1.0)));
    assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)));
    assert!(node.numeric_value().is_some_and(|v| (v - 50.0).abs() < 1e-3), "{:?}", node.numeric_value());
    let (target_node, target_tree) = node.locate();
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::Increment, data: None }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [200.0, 0.51, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
    let data = Some(ActionData::NumericValue(20.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [200.0, 0.2, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::Decrement, data: None }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [200.0, 0.19, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&fill(h.state())));
}

#[test]
fn a_huge_set_value_on_the_ring_wraps_like_any_other_hue() {
    let mut h = harness(app(json!([0.3, 0.6, 0.3])));
    let (target_node, target_tree) = h.get_by_label("Hue ring").accesskit_node().locate();
    let data = Some(ActionData::NumericValue(1e300));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    let color = fill(h.state());
    let Color::Rgb { r, g, b } = color else { panic!("expected RGB fill: {color:?}") };
    assert!([r, g, b].iter().all(|v| v.is_finite()), "{color:?}");
    assert!(hsb(&color).iter().all(|v| v.is_finite()), "{:?}", hsb(&color));
    assert!(near(hsb(&color), [0.0, 0.5, 0.6], [1e-3, 1e-3, 1e-3]), "{:?}", hsb(&color));
}

/// A step past the edge of the square leaves the fill alone: a CMYK cyan shows at 100% saturation,
/// and a step up from there neither separates it again nor adds an undo step.
#[test]
fn a_step_past_the_edge_of_the_square_changes_nothing() {
    let mut h = harness(app(json!({"c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0})));
    let before = fill(h.state());
    assert_eq!(hsb(&before)[1], 1.0, "{:?}", hsb(&before));
    let undo = undo_len(h.state());
    let (target_node, target_tree) = h.get_by_label("Saturation and brightness").accesskit_node().locate();
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::Increment, data: None }));
    h.run_steps(1);
    assert_eq!(fill(h.state()), before);
    assert_eq!(undo_len(h.state()), undo, "no undo step");
}

/// The Color Picker's color field is the same widget: a press focuses it, the arrows step the
/// components it shows, and screen readers read its name and both components and step the first.
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
    let (target_node, target_tree) = h.get_by_label("Color field").accesskit_node().locate();
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::Increment, data: None }));
    h.run_steps(1);
    let stepped = picked(&h);
    assert!(near(stepped, [after[0], after[1] + 0.01, after[2]], [1e-3, 1e-4, 1e-4]), "{after:?} → {stepped:?}");
    assert!(h.state().ui.dialog.is_some(), "the dialog stays open");
}

/// The panel with OKLCH Sliders shown, in a flyout tall enough for the rows.
fn oklch_harness(mut app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap();
    oklch_harness_shown(app)
}

/// The panel in a flyout tall enough for the rows, with the app's settings as they are.
fn oklch_harness_shown(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 420.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            crate::shortcuts::handle(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}

/// The slider row named `name` (its number field has the same name).
fn oklch_slider<'a>(h: &'a Harness<'static, VectorcraftApp>, name: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_label(name).find(|n| n.accesskit_node().role() == egui::accesskit::Role::Slider).expect("a slider row")
}

/// The point at `t` (0..1) along the bar of a slider row.
fn on_rail(rail: Rect, t: f32) -> Pos2 {
    Pos2::new(rail.left() + 4.0 + t * (rail.width() - 8.0), rail.top() + 7.0)
}

fn fill_oklch(app: &VectorcraftApp) -> [f64; 3] {
    vectorcraft_color::oklab::to_oklch(cms::active().display_rgb(&fill(app)))
}

/// `ui.colorWheel {sliders: "oklch"}` shows L, C and H rows with the fill's OKLCH, and the panel
/// keeps the setting with the preferences.
#[test]
fn oklch_sliders_show_the_fills_lightness_chroma_and_hue() {
    let h = oklch_harness(app(json!("#ff0000")));
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::Oklch);
    let value = |name: &str| oklch_slider(&h, name).accesskit_node().numeric_value().unwrap();
    assert_eq!((value("Lightness"), value("Chroma"), value("Hue")), (63.0, 64.0, 29.0));
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    assert_eq!(saved["color_wheel"]["sliders"], json!("oklch"));
}

/// A drag on L past the chroma sRGB holds stores the largest chroma that fits at the same hue, and
/// the C row keeps the chroma asked for, so dragging L back brings it back.
#[test]
fn a_drag_on_lightness_keeps_the_chroma_asked_for() {
    let mut h = oklch_harness(app(json!("#0000ff")));
    let [l0, c0, h0] = fill_oklch(h.state());
    let rail = oklch_slider(&h, "Lightness").rect();
    drag(&mut h, &[on_rail(rail, l0 as f32), on_rail(rail, 0.9)], Modifiers::NONE);
    let [l1, c1, h1] = fill_oklch(h.state());
    assert!((l1 - 0.9).abs() < 0.01 && c1 < c0 - 0.1 && (h1 - h0).abs() < 1.0, "{:?} → {:?}", [l0, c0, h0], [l1, c1, h1]);
    let chroma = oklch_slider(&h, "Chroma").accesskit_node().numeric_value().unwrap();
    assert_eq!(chroma, (c0 / 0.4 * 100.0).round(), "the C row keeps the requested chroma");
    drag(&mut h, &[on_rail(rail, 0.9), on_rail(rail, l0 as f32)], Modifiers::NONE);
    let [l2, c2, _] = fill_oklch(h.state());
    assert!((l2 - l0).abs() < 0.01 && (c2 - c0).abs() < 0.01, "{:?} → {:?}", [l0, c0], [l2, c2]);
}

/// A press on a row gives it the keyboard focus, and → raises it by one unit as one undo step.
/// Each row reports its range and the adjust actions to screen readers.
#[test]
fn the_oklch_rows_take_the_arrow_keys_and_screen_readers_actions() {
    let mut h = oklch_harness(app(json!("#808080")));
    for (name, max) in [("Lightness", 100.0), ("Chroma", 100.0), ("Hue", 360.0)] {
        let node = oklch_slider(&h, name).accesskit_node();
        assert_eq!((node.min_numeric_value(), node.max_numeric_value(), node.numeric_value_step()), (Some(0.0), Some(max), Some(1.0)), "{name}");
        assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)), "{name}");
    }
    let rail = oklch_slider(&h, "Lightness").rect();
    let l = fill_oklch(h.state())[0];
    click(&mut h, on_rail(rail, l as f32), Modifiers::NONE);
    let before = fill_oklch(h.state())[0];
    let undo = undo_len(h.state());
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    let after = fill_oklch(h.state())[0];
    assert!((after - ((before * 100.0).round() + 1.0) / 100.0).abs() < 0.003, "{before} → {after}");
    assert_eq!(undo_len(h.state()), undo + 1, "one undo step");
    let (gray, undo) = (fill(h.state()), undo_len(h.state()));
    let (target_node, target_tree) = oklch_slider(&h, "Hue").accesskit_node().locate();
    let data = Some(ActionData::NumericValue(200.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    // The frame that takes the request draws the row before it applies it.
    h.run_steps(2);
    assert_eq!(oklch_slider(&h, "Hue").accesskit_node().numeric_value(), Some(200.0), "a gray keeps the hue it was given");
    assert_eq!((fill(h.state()), undo_len(h.state())), (gray, undo), "a hue on a gray changes nothing and records no undo step");
}

/// Each track dims the colors outside sRGB, and the C track's tick sits at the largest chroma sRGB
/// holds, along the track's 0 to 0.4.
#[test]
fn the_oklch_tracks_dim_the_colors_outside_srgb_and_the_tick_marks_the_largest_chroma() {
    let (l, h) = (0.7, 150.0);
    let max = vectorcraft_color::oklab::max_chroma(l, h);
    for max_chroma in [Some(max), None] {
        assert_eq!(oklch_track([l, max * 0.5, h], max_chroma).a(), 255, "{max_chroma:?}: inside sRGB");
        assert!(oklch_track([l, 0.4, h], max_chroma).a() < 255, "{max_chroma:?}: outside sRGB is dimmed");
    }
    // Near white, an L or H track shows the color a pick there stores: the largest chroma that fits,
    // not each channel clamped on its own.
    let (stored, lowered) = vectorcraft_color::oklab::from_oklch(0.98, 0.2, h);
    let [r, g, b] = stored.map(|v| (f64::from(v).clamp(0.0, 1.0) * 255.0).round() as u8);
    assert!(lowered);
    assert_eq!(oklch_track([0.98, 0.2, h], None), egui::Color32::from_rgb(r, g, b).gamma_multiply(0.3));
    // At blue's L and H the ray leaves sRGB below the largest chroma and comes back at blue: the C
    // track dims that gap and shows the color a pick there stores.
    let [bl, _, bh] = vectorcraft_color::oklab::to_oklch([0.0, 0.0, 1.0]);
    let blue_max = vectorcraft_color::oklab::max_chroma(bl, bh);
    let (stored, lowered) = vectorcraft_color::oklab::from_oklch(bl, 0.29, bh);
    let [r, g, b] = stored.map(|v| (f64::from(v).clamp(0.0, 1.0) * 255.0).round() as u8);
    assert!(lowered && blue_max > 0.3);
    assert_eq!(oklch_track([bl, 0.29, bh], Some(blue_max)), egui::Color32::from_rgb(r, g, b).gamma_multiply(0.3));
    assert_eq!(oklch_track([bl, blue_max, bh], Some(blue_max)).a(), 255, "blue itself is inside sRGB");
    assert!((chroma_tick(max) - (max / 0.4) as f32).abs() < 1e-6 && chroma_tick(max) > 0.1 && chroma_tick(max) < 1.0);
    assert_eq!((chroma_tick(0.0), chroma_tick(0.5)), (0.0, 1.0));
}

/// After Edit › Color Settings picks a working RGB space other than sRGB, Panel menu › OKLCH Sliders
/// is gray and `ui.colorWheel {sliders: "oklch"}` answers an error; rows already shown are gray.
#[test]
fn oklch_sliders_are_gray_after_the_working_space_changes() {
    const CHILD: &str = "VECTORCRAFT_TEST_COLOR_WHEEL_P3";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panels::tests_color_wheel::oklch_sliders_are_gray_after_the_working_space_changes", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated working space test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let mut app = app(json!("#ff0000"));
    app.run("edit.colorSettings", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    assert_eq!(app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap_err(), "OKLCH needs sRGB as the working RGB space");
    assert_eq!(app.ui.color_wheel.sliders, Sliders::None);
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 200.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            super::color_wheel::menu(app, ui);
        },
        app,
    );
    h.run_steps(3);
    assert!(h.get_by_label("   OKLCH Sliders").accesskit_node().is_disabled());
    // The color settings stay with the process, so a new app works in Display P3 as well.
    let mut app = self::app(json!("#ff0000"));
    app.ui.color_wheel.sliders = Sliders::Oklch;
    let h = oklch_harness_shown(app);
    assert!(oklch_slider(&h, "Lightness").accesskit_node().is_disabled());
}

/// A drag on an OKLCH row during which Edit › Color Settings picks a working RGB space other than
/// sRGB keeps the color the drag previewed as one undo step and leaves no interaction open, though
/// the gray rows report no release.
#[test]
fn a_working_space_change_during_an_oklch_drag_keeps_its_color_as_one_undo_step() {
    const CHILD: &str = "VECTORCRAFT_TEST_COLOR_WHEEL_P3_DRAG";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "panels::tests_color_wheel::a_working_space_change_during_an_oklch_drag_keeps_its_color_as_one_undo_step",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated working space test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let mut h = oklch_harness(app(json!("#ff8000")));
    let rail = oklch_slider(&h, "Lightness").rect();
    let undo = undo_len(h.state());
    press(&mut h, on_rail(rail, 0.5), true, Modifiers::NONE);
    for _ in 0..2 {
        h.event(Event::PointerMoved(on_rail(rail, 0.25)));
        h.run_steps(1);
    }
    assert!(h.state().session.in_interaction(), "the drag previews");
    let previewed = fill(h.state());
    h.state_mut().run("edit.colorSettings", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    h.run_steps(2);
    assert!(oklch_slider(&h, "Lightness").accesskit_node().is_disabled());
    press(&mut h, on_rail(rail, 0.25), false, Modifiers::NONE);
    assert!(!h.state().session.in_interaction(), "the interaction ended");
    assert_eq!((fill(h.state()), undo_len(h.state())), (previewed, undo + 1));
}

/// A CMYK fill's L, C and H are those of its display color, not of its naive separation.
#[test]
fn a_cmyk_fills_oklch_is_that_of_its_display_color() {
    let h = oklch_harness(app(json!({"c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0})));
    let shown = vectorcraft_color::oklab::to_oklch(cms::active().display_rgb(&Color::cmyk(1.0, 0.0, 0.0, 0.0)));
    let naive = vectorcraft_color::oklab::to_oklch([0.0, 1.0, 1.0]);
    let lightness = oklch_slider(&h, "Lightness").accesskit_node().numeric_value().unwrap();
    assert_eq!(lightness, (shown[0] * 100.0).round());
    assert_ne!(lightness, (naive[0] * 100.0).round());
}

/// Panel menu › OKLCH Sliders shows the rows and, checked, hides them.
#[test]
fn the_panel_menu_shows_and_hides_the_oklch_rows() {
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 200.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which hold the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            super::color_wheel::menu(app, ui);
        },
        app(json!("#ff0000")),
    );
    h.run_steps(3);
    h.get_by_label("   OKLCH Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::Oklch);
    h.get_by_label("✓ OKLCH Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::None);
}

/// In a working RGB space other than sRGB, `sliders: "oklch"` is an error and the rows are gray;
/// the error leaves the settings as they were.
#[test]
fn oklch_sliders_need_srgb_as_the_working_space() {
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let mut settings = Settings::default();
    assert_eq!(set(&mut settings, &json!({"sliders": "oklch"}), &p3).unwrap_err(), "OKLCH needs sRGB as the working RGB space");
    assert_eq!(settings.sliders, Sliders::None);
    set(&mut settings, &json!({"sliders": "oklch"}), &Cms::default()).unwrap();
    assert_eq!(settings.sliders, Sliders::Oklch);
    set(&mut settings, &json!({"sliders": "none"}), &p3).unwrap();
    assert_eq!(settings.sliders, Sliders::None, "the rows can be hidden in any working space");
    let mut h = Harness::builder().with_size(vec2(260.0, 200.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            let tgt = super::color::wheel_target(app, ui);
            let color = tgt.color();
            let picked = oklch_rows(app, ui, &tgt, color, &p3);
            assert!(picked.is_none(), "a gray row picks nothing: {picked:?}");
        },
        app(json!("#ff0000")),
    );
    h.run_steps(2);
    assert!(oklch_slider(&h, "Lightness").accesskit_node().is_disabled());
    let rail = oklch_slider(&h, "Lightness").rect();
    click(&mut h, on_rail(rail, 0.2), Modifiers::NONE);
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    set_slider_value(&mut h, "Lightness", 20.0);
}

/// The rows show what they asked for while the color displays as it did, and the color's own
/// OKLCH after the color settings change its display.
#[test]
fn the_remembered_oklch_follows_a_change_of_working_space() {
    let srgb = Cms::default();
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let c = Color::rgb(0.5, 0.3, 0.2);
    let asked = [0.5, 0.3, 40.0];
    let shown = srgb.display_rgb(&c);
    assert_eq!(remembered_oklch(&srgb, &c, shown, asked), asked);
    assert_eq!(remembered_oklch(&p3, &c, shown, asked), vectorcraft_color::oklab::to_oklch(p3.display_rgb(&c)));
}

/// The panel with HSL Sliders shown, in a flyout tall enough for the rows. `oklch_slider` and
/// `oklch_harness_shown` find and draw any slider row, the HSL rows included.
fn hsl_harness(mut app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    app.run("ui.colorWheel", json!({"sliders": "hsl"})).unwrap();
    oklch_harness_shown(app)
}

fn fill_hsl(app: &VectorcraftApp) -> [f32; 3] {
    vectorcraft_color::rgb_to_hsl(cms::active().display_rgb(&fill(app)))
}

/// Sets the slider row named `name` to `v` as a screen reader does.
fn set_slider_value(h: &mut Harness<'static, VectorcraftApp>, name: &str, v: f64) {
    let (target_node, target_tree) = oklch_slider(h, name).accesskit_node().locate();
    let data = Some(ActionData::NumericValue(v));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    // The frame that takes the request draws the row before it applies it.
    h.run_steps(2);
}

/// `ui.colorWheel {sliders: "hsl"}` shows H, S and L rows with the fill's HSL, CSS Color 4's
/// `hsl()` of its display sRGB, and the panel keeps the setting with the preferences.
#[test]
fn hsl_sliders_show_the_fills_hue_saturation_and_lightness() {
    let h = hsl_harness(app(json!("#ff8000")));
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::Hsl);
    let value = |name: &str| oklch_slider(&h, name).accesskit_node().numeric_value().unwrap();
    assert_eq!((value("HSL Hue"), value("HSL Saturation"), value("HSL Lightness")), (30.0, 100.0, 50.0));
    assert!(h.query_by_label("Chroma").is_none(), "OKLCH Sliders are hidden");
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    assert_eq!(saved["color_wheel"]["sliders"], json!("hsl"));
}

/// In Box View with the Saturation channel, screen readers tell the box strip's HSB saturation
/// from the S row's HSL saturation by their names.
#[test]
fn the_hsl_rows_names_differ_from_the_box_strips() {
    let mut app = app(json!("#996633"));
    app.run("ui.colorWheel", json!({"view": "box", "channel": "saturation"})).unwrap();
    let h = hsl_harness(app);
    let sliders = |name: &str| h.query_all_by_label(name).filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Slider).count();
    assert_eq!((sliders("Saturation"), sliders("HSL Saturation")), (1, 1));
    let strip = oklch_slider(&h, "Saturation").accesskit_node().numeric_value().unwrap();
    let row = oklch_slider(&h, "HSL Saturation").accesskit_node().numeric_value().unwrap();
    assert_ne!(strip, row, "HSB and HSL saturation differ for this color");
}

/// A drag on L changes the lightness and keeps the hue and the saturation.
#[test]
fn a_drag_on_hsl_lightness_keeps_the_hue_and_saturation() {
    let mut h = hsl_harness(app(json!("#ff8000")));
    let rail = oklch_slider(&h, "HSL Lightness").rect();
    let undo = undo_len(h.state());
    drag(&mut h, &[on_rail(rail, 0.5), on_rail(rail, 0.25)], Modifiers::NONE);
    assert!(near(fill_hsl(h.state()), [30.0, 1.0, 0.25], [1.0, 0.01, 0.01]), "{:?}", fill_hsl(h.state()));
    assert_eq!(undo_len(h.state()), undo + 1, "a drag is one undo step");
}

/// A press on S gives it the keyboard focus, and → raises it by 1 % as one undo step. Each row
/// reports its range and the adjust actions to screen readers.
#[test]
fn the_hsl_rows_take_the_arrow_keys_and_screen_readers_actions() {
    let mut h = hsl_harness(app(json!("#996633")));
    for (name, max) in [("HSL Hue", 360.0), ("HSL Saturation", 100.0), ("HSL Lightness", 100.0)] {
        let node = oklch_slider(&h, name).accesskit_node();
        assert_eq!((node.min_numeric_value(), node.max_numeric_value(), node.numeric_value_step()), (Some(0.0), Some(max), Some(1.0)), "{name}");
        assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)), "{name}");
    }
    let rail = oklch_slider(&h, "HSL Saturation").rect();
    let s = fill_hsl(h.state())[1];
    click(&mut h, on_rail(rail, s), Modifiers::NONE);
    let before = fill_hsl(h.state());
    let undo = undo_len(h.state());
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    let after = fill_hsl(h.state());
    let raised = ((before[1] * 100.0).round() + 1.0) / 100.0;
    assert!(near(after, [before[0], raised, before[2]], [1.0, 0.008, 0.003]), "{before:?} → {after:?}");
    assert_eq!(undo_len(h.state()), undo + 1, "one undo step");
}

/// A gray has no hue: a hue given to it stays on the H row while the color is unchanged, and a
/// saturation picked next uses that hue.
#[test]
fn a_gray_keeps_the_hsl_hue_it_was_given() {
    let mut h = hsl_harness(app(json!("#808080")));
    let (gray, undo) = (fill(h.state()), undo_len(h.state()));
    set_slider_value(&mut h, "HSL Hue", 200.0);
    assert_eq!(oklch_slider(&h, "HSL Hue").accesskit_node().numeric_value(), Some(200.0), "a gray keeps the hue it was given");
    assert_eq!((fill(h.state()), undo_len(h.state())), (gray, undo), "a hue on a gray changes nothing and records no undo step");
    set_slider_value(&mut h, "HSL Saturation", 60.0);
    let got = fill_hsl(h.state());
    assert!(near(got, [200.0, 0.6, 0.5], [1.0, 0.01, 0.01]), "{got:?}");
}

/// In a working RGB space wider than sRGB, an HSL pick goes from display sRGB into that space
/// through the color settings: the fill displays the HSL asked for, and its stored RGB numbers are
/// those of the working space, not the sRGB numbers of that HSL.
#[test]
fn an_hsl_pick_in_display_p3_goes_into_the_working_space() {
    const CHILD: &str = "VECTORCRAFT_TEST_COLOR_WHEEL_HSL_P3";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panels::tests_color_wheel::an_hsl_pick_in_display_p3_goes_into_the_working_space", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated working space test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let mut app = app(json!("#996633"));
    app.run("edit.colorSettings", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    let mut h = hsl_harness(app);
    let [hue, sat, _] = fill_hsl(h.state());
    let undo = undo_len(h.state());
    set_slider_value(&mut h, "HSL Lightness", 60.0);
    let asked = [hue, sat, 0.6];
    assert!(near(fill_hsl(h.state()), asked, [0.5, 0.005, 0.005]), "{asked:?} → {:?}", fill_hsl(h.state()));
    assert_eq!(undo_len(h.state()), undo + 1, "one undo step");
    let Color::Rgb { r, g, b } = fill(h.state()) else { panic!("an RGB fill stays RGB") };
    let srgb = vectorcraft_color::hsl_to_rgb(asked);
    assert!((0..3).any(|i| ([r, g, b][i] - srgb[i]).abs() > 0.01), "{:?} is stored as the sRGB numbers {srgb:?}", [r, g, b]);
}

/// Panel menu › HSL Sliders shows the H, S and L rows in place of OKLCH Sliders and, checked,
/// hides them; it is offered in any working RGB space.
#[test]
fn the_panel_menu_shows_and_hides_the_hsl_rows() {
    let mut fonts = false;
    let mut app = app(json!("#ff0000"));
    app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(260.0, 200.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which hold the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            super::color_wheel::menu(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("   HSL Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::Hsl);
    assert!(h.query_by_label("✓ OKLCH Sliders").is_none(), "the two slider sets show one at a time");
    h.get_by_label("✓ HSL Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.sliders, Sliders::None);
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let mut settings = Settings::default();
    set(&mut settings, &json!({"sliders": "hsl"}), &p3).unwrap();
    assert_eq!(settings.sliders, Sliders::Hsl, "HSL needs no particular working space");
}

/// The rows show what they asked for while the color displays as it did, and the color's own HSL
/// after the color settings change its display.
#[test]
fn the_remembered_hsl_follows_a_change_of_working_space() {
    let srgb = Cms::default();
    let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..ColorSettings::default() }).unwrap();
    let c = Color::rgb(0.5, 0.3, 0.2);
    let asked = [20.0, 0.43, 0.35];
    let shown = srgb.display_rgb(&c);
    assert_eq!(remembered_hsl(&srgb, &c, shown, asked), asked);
    assert_eq!(remembered_hsl(&p3, &c, shown, asked), vectorcraft_color::rgb_to_hsl(p3.display_rgb(&c)));
}

/// Saved settings are read key by key: a value this version does not know (one a newer version
/// wrote) gives the default, and the other preferences still load.
#[test]
fn saved_color_wheel_settings_are_read_leniently() {
    let mut saved = serde_json::to_value(crate::UiState::default()).unwrap();
    saved["color_rgb_percent"] = json!(true);
    for (value, sliders) in [
        (json!({"sliders": "oklch"}), Sliders::Oklch),
        (json!({"sliders": "hsl"}), Sliders::Hsl),
        (json!({"sliders": "hwb", "view": "box"}), Sliders::None),
        (json!(5), Sliders::None),
    ] {
        saved["color_wheel"] = value.clone();
        let ui: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!((ui.color_wheel.sliders, ui.color_rgb_percent), (sliders, true), "{value}");
    }
}

/// `ui.colorWheel {sliders}` sets the rows and keeps the view and the channel, and an unknown
/// `sliders` value is an error that changes nothing.
#[test]
fn ui_color_wheel_sets_and_reads_the_sliders() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("ui.colorWheel", json!({"view": "box", "channel": "red"})).unwrap();
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap()["sliders"], json!("none"));
    assert_eq!(app.run("ui.colorWheel", json!({"sliders": "hwb"})).unwrap_err(), "`sliders` must be none, oklch or hsl");
    let got = app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap();
    let want = json!({"view": "box", "channel": "red", "gamut": false, "harmony": false, "harmonyRule": "complementary", "vision": false, "sliders": "oklch", "documentColors": false});
    assert_eq!(got, want);
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "sliders": "hwb"})).unwrap_err();
    assert_eq!(e, "`sliders` must be none, oklch or hsl");
    assert_eq!(app.ui.color_wheel.view, crate::panels::color_wheel::View::Box, "an error changes nothing");
}

/// A hue given to a Gray color does not change how it displays, so its paint stays a Gray color
/// and no undo step is recorded; the hue row keeps the hue.
#[test]
fn a_hue_on_a_gray_color_leaves_its_paint_gray() {
    for (rows, hue) in [("hsl", "HSL Hue"), ("oklch", "Hue")] {
        let mut app = app(json!({"gray": 0.5}));
        app.run("ui.colorWheel", json!({"sliders": rows})).unwrap();
        let mut h = oklch_harness_shown(app);
        let (gray, undo) = (fill(h.state()), undo_len(h.state()));
        assert!(matches!(gray, Color::Gray { .. }), "{gray:?}");
        set_slider_value(&mut h, hue, 200.0);
        assert_eq!(oklch_slider(&h, hue).accesskit_node().numeric_value(), Some(200.0), "{rows}");
        assert_eq!((fill(h.state()), undo_len(h.state())), (gray, undo), "{rows}: the Gray fill stays as it was");
    }
}

/// For a fill outside sRGB, the HSL and OKLCH rows show the components of its display color
/// clipped to sRGB, the color the panel shows, not of the unclipped one.
#[test]
fn the_rows_show_a_color_outside_srgb_clipped_to_srgb() {
    let rgb = json!([1.6, 0.2, -0.4]);
    let raw = cms::active().display_rgb(&fill(&app(rgb.clone())));
    let clipped = raw.map(|v| v.clamp(0.0, 1.0));
    assert_ne!(raw, clipped, "the fill lies outside sRGB");
    let want_hsl = (f64::from(vectorcraft_color::rgb_to_hsl(clipped)[2]) * 100.0).round();
    assert_ne!(want_hsl, (f64::from(vectorcraft_color::rgb_to_hsl(raw)[2]) * 100.0).round(), "the unclipped lightness differs");
    let h = hsl_harness(app(rgb.clone()));
    assert_eq!(oklch_slider(&h, "HSL Lightness").accesskit_node().numeric_value(), Some(want_hsl));
    let want_l = (vectorcraft_color::oklab::to_oklch(clipped)[0] * 100.0).round();
    assert_ne!(want_l, (vectorcraft_color::oklab::to_oklch(raw)[0] * 100.0).round(), "the unclipped OKLCH lightness differs");
    let h = oklch_harness(app(rgb));
    assert_eq!(oklch_slider(&h, "Lightness").accesskit_node().numeric_value(), Some(want_l));
}

/// The hue given to a gray is dropped once the panel shows another color, so the same gray applied
/// again later shows its own hue, 0°.
#[test]
fn a_remembered_hue_is_dropped_after_the_paint_changes() {
    for (rows, hue) in [("hsl", "HSL Hue"), ("oklch", "Hue")] {
        let mut app = app(json!("#808080"));
        app.run("ui.colorWheel", json!({"sliders": rows})).unwrap();
        let mut h = oklch_harness_shown(app);
        set_slider_value(&mut h, hue, 240.0);
        assert_eq!(oklch_slider(&h, hue).accesskit_node().numeric_value(), Some(240.0), "{rows}");
        h.state_mut().run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        h.run_steps(2);
        h.state_mut().run("paint.setFill", json!({"color": "#808080"})).unwrap();
        h.run_steps(2);
        assert_eq!(oklch_slider(&h, hue).accesskit_node().numeric_value(), Some(0.0), "{rows}: the gray's own hue");
    }
}

/// Hiding the rows, or showing the other rows, during a drag on a row keeps the color the drag
/// previewed as one undo step and leaves no interaction open.
#[test]
fn hiding_the_rows_during_a_drag_keeps_its_color_as_one_undo_step() {
    for rows in ["none", "oklch"] {
        let mut h = hsl_harness(app(json!("#ff8000")));
        let rail = oklch_slider(&h, "HSL Lightness").rect();
        let undo = undo_len(h.state());
        press(&mut h, on_rail(rail, 0.5), true, Modifiers::NONE);
        for _ in 0..2 {
            h.event(Event::PointerMoved(on_rail(rail, 0.25)));
            h.run_steps(1);
        }
        assert!(h.state().session.in_interaction(), "{rows}: the drag previews");
        let previewed = fill(h.state());
        h.state_mut().run("ui.colorWheel", json!({"sliders": rows})).unwrap();
        h.run_steps(2);
        press(&mut h, on_rail(rail, 0.25), false, Modifiers::NONE);
        assert!(!h.state().session.in_interaction(), "{rows}: the interaction ended");
        assert_eq!((fill(h.state()), undo_len(h.state())), (previewed, undo + 1), "{rows}");
    }
}
