//! The Color Wheel panel's contrast lines on the box's field and the wheel's square, and the chip of
//! the closest passing color in the contrast row, drawn headless through the `Fixture`.

use egui::{Color32, Modifiers};
use serde_json::json;
use vectorcraft_color::{Color, Paint, cms, hsb_to_rgb};
use vectorcraft_engine::Session;

use super::color_wheel::{Channel, Lines, contrast_lines, field_pick};
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;

/// An app with a selected rectangle filled with #FDC96E, the contrast row shown against a pinned
/// #355F78 (4.4963:1, just under AA for normal text), and `view` with `channel`.
fn app(view: &str, channel: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#fdc96e"})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true, "contrastBackground": "#355f78", "view": view, "channel": channel})).unwrap();
    app
}

fn lines(p: &Panel) -> Lines {
    p.app().ui.color_wheel_panel.lines.clone().expect("the field's contrast lines were found").1
}

/// Whether the lines were found coarsely, during a drag that changes the field.
fn coarse(p: &Panel) -> bool {
    p.app().ui.color_wheel_panel.lines.as_ref().expect("lines were found").0.4
}

/// Whether the color picked at field point (x, y) meets AA for normal text against #355F78, as
/// `color.contrast` answers it; `hsb_at` maps the point to the HSB the field picks there.
fn passes_aa(p: &Panel, hsb_at: &dyn Fn(f32, f32) -> [f32; 3], (x, y): (f32, f32)) -> bool {
    let [r, g, b] = hsb_to_rgb(hsb_at(x, y));
    let r = p.app_mut().session.execute("color.contrast", &json!({"color": [r, g, b], "against": "#355f78"})).unwrap();
    r["normalText"]["aa"].as_bool().unwrap()
}

/// The middle point of the longest AA line, and the points a little above and below it.
fn either_side(lines: &Lines) -> [(f32, f32); 2] {
    let aa = lines.first().expect("an AA level");
    let line = aa.iter().max_by_key(|l| l.len()).expect("an AA line");
    let (x, y) = line[line.len() / 2];
    [(x, (y - 0.04).max(0.0)), (x, (y + 0.04).min(1.0))]
}

/// The display HSB of #FDC96E.
fn shown_hsb() -> [f32; 3] {
    vectorcraft_color::rgb_to_hsb(cms::active().display_rgb(&Color::from_hex("#fdc96e").unwrap()))
}

/// The clip rects of the dark strokes of the solid (AA) lines the panel draws, in a fresh frame.
fn dark_line_clips(p: &mut Panel) -> Vec<egui::Rect> {
    p.fx.frame_clipped_shapes()
        .into_iter()
        .filter(|(_, shape)| matches!(shape, egui::Shape::Path(path) if path.stroke.width == 3.0 && matches!(path.stroke.color, egui::epaint::ColorMode::Solid(c) if c == Color32::BLACK)))
        .map(|(clip, _)| clip)
        .collect()
}

#[test]
fn the_box_marks_where_its_colors_meet_aa() {
    let mut p = Panel::new(app("box", "hue"));
    let lines = lines(&p);
    assert_eq!(lines.len(), 2, "AA and AAA for normal text");
    let hsb = shown_hsb();
    let at = move |x: f32, y: f32| field_pick(Channel::Hue, x, y, hsb);
    let [above, below] = either_side(&lines);
    let (a, b) = (passes_aa(&p, &at, above), passes_aa(&p, &at, below));
    assert_ne!(a, b, "the points either side of the line fall on either side of AA");
    // Against a dark background the lighter colors, at the top, pass.
    assert!(a && !b);
    let field = p.node("Color box").rect;
    let clips = dark_line_clips(&mut p);
    assert!(!clips.is_empty() && clips.iter().all(|c| *c == field), "the lines are drawn over the field: {clips:?} {field:?}");
}

#[test]
fn the_wheels_square_marks_where_its_colors_meet_aa() {
    let mut p = Panel::new(app("wheel", "hue"));
    let lines = lines(&p);
    let hsb = shown_hsb();
    let at = move |x: f32, y: f32| [hsb[0], x, 1.0 - y];
    let [above, below] = either_side(&lines);
    assert!(passes_aa(&p, &at, above) && !passes_aa(&p, &at, below));
    let square = p.node("Saturation and brightness").rect;
    let clips = dark_line_clips(&mut p);
    assert!(!clips.is_empty() && clips.iter().all(|c| *c == square), "the lines are drawn over the square: {clips:?} {square:?}");
}

/// White with Difference over a pinned #757575: in the gray box (saturation 0) only grays within a
/// few hundredths of #757575 draw dark enough for AA, a band narrower than the samples.
#[test]
fn a_band_narrower_than_the_samples_gets_its_lines() {
    let mut app = app("box", "saturation");
    app.run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    app.run("transparency.set", json!({"blend": "difference"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#757575"})).unwrap();
    let p = Panel::new(app);
    let lines = lines(&p);
    let aa = lines.first().expect("an AA level");
    assert!(aa.len() >= 2, "a line on each side of the band: {aa:?}");
    p.app_mut().run("paint.setFill", json!({"color": "#757575"})).unwrap();
    let r = p.app_mut().session.execute("color.contrast", &json!({"against": "#757575"})).unwrap();
    assert!(r["normalText"]["aa"] == true, "#757575 draws black and passes: {r}");
}

/// Multiply over a pinned #FF0000 keeps only the red of a pick, so in the blue box (red across)
/// the contrast changes across and not up: the AA line runs top to bottom.
#[test]
fn a_line_that_runs_up_the_field_is_found() {
    let mut app = app("box", "blue");
    app.run("transparency.set", json!({"blend": "multiply"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ff0000"})).unwrap();
    let p = Panel::new(app);
    let lines = lines(&p);
    let line = lines.first().and_then(|aa| aa.iter().max_by_key(|l| l.len())).expect("an AA line");
    let (xs, ys): (Vec<f32>, Vec<f32>) = line.iter().copied().unzip();
    let spread = |v: &[f32]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max) - v.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(spread(&ys) > 0.9 && spread(&xs) < 0.05, "{line:?}");
}

/// A passing band around x = 0.515625 that is narrower than the samples near the top and wider
/// below: every point of its center passes, so no line crosses it, and its two edges run on either
/// side of it from the top to the bottom.
#[test]
fn a_band_that_widens_past_the_samples_is_not_cut_across() {
    let center = 0.515625;
    let ratio = |x: f32, y: f32| 4.5 + 0.009 + 0.013 * f64::from(y) - f64::from((x - center).abs());
    let lines = contrast_lines(&ratio, &[4.5], 32, true);
    let aa = lines.first().expect("one level");
    for line in aa {
        for pair in line.windows(2) {
            let [p, q] = pair else { continue };
            assert!((p.0 - center) * (q.0 - center) > 0.0, "{p:?} to {q:?} crosses the passing center");
        }
    }
    let side = |left: bool| aa.iter().flatten().filter(|p| (p.0 < center) == left).map(|p| p.1).collect::<Vec<_>>();
    for points in [side(true), side(false)] {
        let low = points.iter().copied().fold(f32::INFINITY, f32::min);
        let high = points.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(low < 0.05 && high > 0.95, "an edge runs from the top to the bottom: {low} to {high}");
    }
}

/// With a fill of None the row shows a dash, and the field still marks where a pick there passes.
#[test]
fn a_fill_of_none_keeps_its_lines() {
    let mut app = app("box", "hue");
    app.run("paint.setFill", json!({"none": true})).unwrap();
    let p = Panel::new(app);
    assert!(p.fx.by_label("—").is_some(), "no color to measure");
    let lines = lines(&p);
    assert!(lines.first().is_some_and(|aa| !aa.is_empty()), "an AA line: {lines:?}");
}

/// While Alt is held a pick goes to the other proxy, so the field's lines, which measure the active
/// one, are not drawn; they come back once Alt is let go.
#[test]
fn the_lines_are_hidden_while_alt_is_held() {
    let mut p = Panel::new(app("box", "hue"));
    p.app_mut().ui.color_wheel_panel.lines = None;
    p.fx.hold(Modifiers::ALT).idle(2);
    assert!(p.app().ui.color_wheel_panel.lines.is_none(), "no lines with Alt held");
    assert!(dark_line_clips(&mut p).is_empty(), "none drawn with Alt held");
    p.fx.hold(Modifiers::NONE).idle(2);
    assert!(p.app().ui.color_wheel_panel.lines.is_some(), "the lines are back");
}

#[test]
fn graphics_have_one_line() {
    let mut app = app("box", "saturation");
    app.run("ui.colorWheel", json!({"contrastFor": "graphics"})).unwrap();
    let p = Panel::new(app);
    assert_eq!(lines(&p).len(), 1, "graphics have AA only");
}

const CHIP: &str = "The closest color that passes AA: click to apply it.";

/// The fix `color.contrast` answers for AA against the pinned #355F78.
fn aa_fix(p: &Panel) -> Color {
    let want = p.app_mut().session.execute("color.contrast", &json!({"against": "#355f78"})).unwrap();
    serde_json::from_value(want["fix"]["aa"].clone()).unwrap()
}

fn fill(p: &Panel) -> Paint {
    p.app().session.proxy_paints().0
}

#[test]
fn the_chip_applies_the_fix_color_answers() {
    let mut p = Panel::new(app("wheel", "hue"));
    let fix = aa_fix(&p);
    p.point(CHIP);
    match fill(&p) {
        Paint::Solid { color, .. } => assert_eq!(color, fix),
        other => panic!("the fill is {other:?}"),
    }
    assert!(p.fx.by_label(CHIP).is_none(), "the color now passes AA: no chip");
    assert!(p.fx.by_label("AA Pass").is_some());
}

/// A screen reader's click on the chip applies the color as a pointer click does.
#[test]
fn a_screen_readers_click_applies_the_fix() {
    let mut p = Panel::new(app("box", "hue"));
    let fix = aa_fix(&p);
    p.click(CHIP);
    match fill(&p) {
        Paint::Solid { color, .. } => assert_eq!(color, fix),
        other => panic!("the fill is {other:?}"),
    }
}

#[test]
fn the_chip_offers_aaa_once_aa_passes_and_nothing_once_both_pass() {
    let mut app = app("wheel", "hue");
    app.run("ui.colorWheel", json!({"contrastFor": "largeText"})).unwrap();
    let p = Panel::new(app);
    assert!(p.fx.by_label(CHIP).is_none(), "4.49:1 meets large text AA");
    assert!(p.fx.by_label("The closest color that passes AAA: click to apply it.").is_some());
    let mut app = self::app("wheel", "hue");
    app.run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastFor": "largeText"})).unwrap();
    let p = Panel::new(app);
    for level in ["AA", "AAA"] {
        assert!(p.fx.by_label(&format!("The closest color that passes {level}: click to apply it.")).is_none());
    }
}

#[test]
fn the_panel_keeps_its_height_when_the_chip_goes() {
    let height = |fill: &str| {
        let mut app = app("box", "hue");
        app.run("paint.setFill", json!({"color": fill})).unwrap();
        Panel::new(app).fx.shown().height()
    };
    // #FDC96E fails AA and shows the chip; #FFFFFF passes both levels and hides it.
    assert_eq!(height("#fdc96e"), height("#ffffff"));
}

/// A drag that changes the field on every frame (here the box's hue) finds the lines coarsely, and
/// in full again once it ends; a drag that leaves the field as it is keeps the full lines.
#[test]
fn a_drag_that_changes_the_field_finds_its_lines_coarsely_until_it_ends() {
    let mut p = Panel::new(app("box", "hue"));
    assert!(!coarse(&p));
    p.app_mut().session.begin_interaction("test").unwrap();
    p.fx.idle(2);
    assert!(!coarse(&p), "the field is unchanged, so the full lines are kept");
    p.app_mut().run("paint.setFill", json!({"color": "#6efdc9"})).unwrap();
    assert!(p.app().session.in_interaction(), "the new color is part of the drag");
    p.fx.idle(2);
    assert!(coarse(&p), "another hue during the drag");
    p.app_mut().session.commit_interaction().unwrap();
    p.fx.idle(2);
    assert!(!coarse(&p), "found in full once the drag ends");
}
