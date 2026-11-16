//! The Color Wheel panel's contrast lines on the box's field and the wheel's square, and the chip of
//! the closest passing color in the contrast row, drawn headless.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use vectorcraft_color::{Color, Paint, cms, hsb_to_rgb};
use vectorcraft_engine::Session;

use super::color_wheel::{LINES, Lines, LinesKey, box_pick, show};
use super::pstate;
use crate::VectorcraftApp;
use crate::dialogs::color_picker::Channel;

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

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            // The height the panel took, for the test that checks nothing moves.
            let h = ui.min_rect().height();
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("panel-height"), h));
        },
        app,
    );
    h.run_steps(2);
    h
}

fn lines(h: &Harness<'static, VectorcraftApp>) -> Lines {
    pstate::<Option<(LinesKey, Lines)>>(&h.ctx, LINES).expect("the field's contrast lines were found").1
}

/// Whether the color picked at field point (x, y) meets AA for normal text against #355F78, as
/// `color.contrast` answers it; `hsb_at` maps the point to the HSB the field picks there.
fn passes_aa(h: &mut Harness<'static, VectorcraftApp>, hsb_at: &dyn Fn(f32, f32) -> [f32; 3], (x, y): (f32, f32)) -> bool {
    let [r, g, b] = hsb_to_rgb(hsb_at(x, y));
    let r = h.state_mut().session.execute("color.contrast", &json!({"color": [r, g, b], "against": "#355f78"})).unwrap();
    r["normalText"]["aa"].as_bool().unwrap()
}

/// The middle point of the longest AA line, and the points a little above and below it.
fn either_side(lines: &Lines) -> [(f32, f32); 2] {
    let aa = lines.first().expect("an AA level");
    let line = aa.iter().max_by_key(|l| l.len()).expect("an AA line");
    let (x, y) = line[line.len() / 2];
    [(x, (y - 0.04).max(0.0)), (x, (y + 0.04).min(1.0))]
}

#[test]
fn the_box_marks_where_its_colors_meet_aa() {
    let mut h = harness(app("box", "hue"));
    let lines = lines(&h);
    assert_eq!(lines.len(), 2, "AA and AAA for normal text");
    let hsb = vectorcraft_color::rgb_to_hsb(cms::active().display_rgb(&Color::from_hex("#fdc96e").unwrap()));
    let z = super::color_wheel::box_pos(Channel::Hue, hsb).0;
    let at = move |x: f32, y: f32| box_pick(Channel::Hue, z, x, y, hsb);
    let [above, below] = either_side(&lines);
    let (a, b) = (passes_aa(&mut h, &at, above), passes_aa(&mut h, &at, below));
    assert_ne!(a, b, "the points either side of the line fall on either side of AA");
    // Against a dark background the lighter colors, at the top, pass.
    assert!(a && !b);
}

#[test]
fn the_wheels_square_marks_where_its_colors_meet_aa() {
    let mut h = harness(app("wheel", "hue"));
    let lines = lines(&h);
    let hsb = vectorcraft_color::rgb_to_hsb(cms::active().display_rgb(&Color::from_hex("#fdc96e").unwrap()));
    let at = move |x: f32, y: f32| [hsb[0], x, 1.0 - y];
    let [above, below] = either_side(&lines);
    assert!(passes_aa(&mut h, &at, above) && !passes_aa(&mut h, &at, below));
}

/// White with Difference over a pinned #757575: in the gray box (saturation 0) only grays within a
/// few hundredths of #757575 draw dark enough for AA, a band narrower than the samples.
#[test]
fn a_band_narrower_than_the_samples_gets_its_lines() {
    let mut app = app("box", "saturation");
    app.run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    app.run("transparency.set", json!({"blend": "difference"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#757575"})).unwrap();
    let mut h = harness(app);
    let lines = lines(&h);
    let aa = lines.first().expect("an AA level");
    assert!(aa.len() >= 2, "a line on each side of the band: {aa:?}");
    h.state_mut().run("paint.setFill", json!({"color": "#757575"})).unwrap();
    let r = h.state_mut().session.execute("color.contrast", &json!({"against": "#757575"})).unwrap();
    assert!(r["normalText"]["aa"] == true, "#757575 draws black and passes: {r}");
}

/// Multiply over a pinned #FF0000 keeps only the red of a pick, so in the blue box (red across)
/// the contrast changes across and not up: the AA line runs top to bottom.
#[test]
fn a_line_that_runs_up_the_field_is_found() {
    let mut app = app("box", "blue");
    app.run("transparency.set", json!({"blend": "multiply"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ff0000"})).unwrap();
    let h = harness(app);
    let lines = lines(&h);
    let line = lines.first().and_then(|aa| aa.iter().max_by_key(|l| l.len())).expect("an AA line");
    let (xs, ys): (Vec<f32>, Vec<f32>) = line.iter().copied().unzip();
    let spread = |v: &[f32]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max) - v.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(spread(&ys) > 0.9 && spread(&xs) < 0.05, "{line:?}");
}

#[test]
fn graphics_have_one_line() {
    let mut app = app("box", "saturation");
    app.run("ui.colorWheel", json!({"contrastFor": "graphics"})).unwrap();
    let h = harness(app);
    assert_eq!(lines(&h).len(), 1, "graphics have AA only");
}

const CHIP: &str = "The closest color that passes AA: click to apply it.";

#[test]
fn the_chip_applies_the_fix_color_answers() {
    let mut h = harness(app("wheel", "hue"));
    let want = h.state_mut().session.execute("color.contrast", &json!({"against": "#355f78"})).unwrap();
    let fix: Color = serde_json::from_value(want["fix"]["aa"].clone()).unwrap();
    h.get_by_label(CHIP).click();
    h.run_steps(2);
    match h.state().session.proxy_paints().0 {
        Paint::Solid { color, .. } => assert_eq!(color, fix),
        other => panic!("the fill is {other:?}"),
    }
    assert!(h.query_by_label(CHIP).is_none(), "the color now passes AA: no chip");
    assert!(h.query_by_label("AA Pass").is_some());
}

#[test]
fn the_chip_offers_aaa_once_aa_passes_and_nothing_once_both_pass() {
    let mut app = app("wheel", "hue");
    app.run("ui.colorWheel", json!({"contrastFor": "largeText"})).unwrap();
    let h = harness(app);
    assert!(h.query_by_label(CHIP).is_none(), "4.49:1 meets large text AA");
    assert!(h.query_by_label("The closest color that passes AAA: click to apply it.").is_some());
    let mut app = self::app("wheel", "hue");
    app.run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastFor": "largeText"})).unwrap();
    let h = harness(app);
    assert!(h.query_by_label_contains("The closest color that passes").is_none());
}

#[test]
fn the_panel_keeps_its_height_when_the_chip_goes() {
    let height = |fill: &str| {
        let mut app = app("box", "hue");
        app.run("paint.setFill", json!({"color": fill})).unwrap();
        let h = harness(app);
        h.ctx.data(|d| d.get_temp::<f32>(egui::Id::unique("panel-height"))).expect("a height")
    };
    // #FDC96E fails AA and shows the chip; #FFFFFF passes both levels and hides it.
    assert_eq!(height("#fdc96e"), height("#ffffff"));
}
