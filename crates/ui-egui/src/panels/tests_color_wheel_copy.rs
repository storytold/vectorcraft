//! The Color Wheel panel's Copy Color As: the text each item puts on the clipboard, the opacity it
//! carries, the status bar message, and that `color.parse` reads each text back.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use vectorcraft_color::cms;
use vectorcraft_engine::Session;

use super::color_wheel::{copy_color_as, copy_menu, menu};
use crate::VectorcraftApp;

/// A harness that draws `draw` with the app's fonts installed.
fn harness(app: VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Harness<'static, VectorcraftApp> {
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            draw(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// An app with a selected rectangle filled with #336699 at `opacity` %.
fn app(opacity: f64) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699", "opacity": opacity})).unwrap();
    app
}

/// The texts Copy Color As › `notation` puts on the clipboard in one frame.
fn copy(app: &mut VectorcraftApp, notation: &str) -> Vec<String> {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| copy_color_as(app, ui, notation));
    out.textures_delta.clear();
    out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect()
}

fn undo_count(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// Copy Color As › CSS rgb() puts CSS Color 4's space-separated form on the clipboard, with the
/// opacity after a slash, and says what it copied; the document does not change.
#[test]
fn css_rgb_copies_the_color_and_the_opacity() {
    let mut app = app(50.0);
    let before = undo_count(&app);
    assert_eq!(copy(&mut app, "rgb"), ["rgb(51 102 153 / 50%)"]);
    assert_eq!(app.ui.status, "Copied the color as rgb(51 102 153 / 50%)");
    assert_eq!(undo_count(&app), before, "copying records no undo step");
}

/// Hex and CMYK Values never carry the opacity; Hex with Alpha always does, as `ff` at 100 %; CSS
/// adds it only under 100 %.
#[test]
fn each_item_writes_its_notation() {
    let mut half = app(50.0);
    assert_eq!(copy(&mut half, "hex"), ["#336699"]);
    assert_eq!(copy(&mut half, "hexAlpha"), ["#33669980"]);
    assert_eq!(copy(&mut half, "hsl"), ["hsl(210 50% 40% / 50%)"]);
    let oklch = copy(&mut half, "oklch");
    assert!(oklch[0].starts_with("oklch(") && oklch[0].ends_with(" / 50%)"), "{oklch:?}");
    let mut full = app(100.0);
    assert_eq!(copy(&mut full, "hexAlpha"), ["#336699ff"]);
    assert_eq!(copy(&mut full, "rgb"), ["rgb(51 102 153)"]);
    let cmyk = copy(&mut full, "cmyk");
    assert_eq!(copy(&mut half, "cmyk"), cmyk, "CMYK Values leaves the opacity out");
    // The working CMYK numbers through the color settings, as color.convert gives them.
    let r = full.run("color.convert", json!({"color": "#336699", "to": "cmyk"})).unwrap();
    let pct: Vec<f64> = r["values"].as_array().unwrap().iter().map(|v| (v.as_f64().unwrap() * 100.0).round() + 0.0).collect();
    assert_eq!(cmyk, [format!("C={} M={} Y={} K={}", pct[0], pct[1], pct[2], pct[3])]);
}

/// A CMYK fill copies its own CMYK values.
#[test]
fn a_cmyk_fill_copies_its_own_values() {
    let mut app = app(100.0);
    app.run("paint.setFill", json!({"color": {"c": 10, "m": 20, "y": 30, "k": 0}})).unwrap();
    assert_eq!(copy(&mut app, "cmyk"), ["C=10 M=20 Y=30 K=0"]);
}

/// `color.parse` reads every copied text but CMYK Values back to the color the hex field shows,
/// and the alpha to the fill's opacity.
#[test]
fn color_parse_reads_each_copied_text_back() {
    let mut app = app(25.0);
    for notation in ["hex", "hexAlpha", "rgb", "hsl", "oklch"] {
        let text = copy(&mut app, notation).pop().unwrap();
        let r = app.run("color.parse", json!({"text": text})).unwrap();
        assert_eq!(r["hex"], "#336699", "{notation}: {text}");
        let opacity = r["opacity"].as_f64();
        match notation {
            "hex" => assert_eq!(opacity, None, "{text}"),
            // 64 of 255.
            "hexAlpha" => assert_eq!(opacity, Some(25.1), "{text}"),
            _ => assert_eq!(opacity, Some(25.0), "{text}"),
        }
    }
}

/// With nothing selected the panel shows the color for new art, which has no opacity of its own:
/// the copied text carries none.
#[test]
fn with_nothing_selected_the_color_for_new_art_is_copied_without_an_opacity() {
    let mut app = app(50.0);
    app.run("select.none", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    assert_eq!(copy(&mut app, "hexAlpha"), ["#336699ff"]);
    assert_eq!(copy(&mut app, "rgb"), ["rgb(51 102 153)"]);
}

/// Without a solid color (a fill of None) nothing is copied.
#[test]
fn nothing_is_copied_without_a_solid_color() {
    let mut app = app(100.0);
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.ui.status.clear();
    assert!(copy(&mut app, "hex").is_empty());
    assert_eq!(app.ui.status, "");
}

/// Panel menu › Copy Color As is gray without a solid color and available with one.
#[test]
fn copy_color_as_is_gray_without_a_solid_color() {
    let h = harness(app(100.0), menu);
    assert!(!h.get_by_label("   Copy Color As").accesskit_node().is_disabled());
    let mut none = app(100.0);
    none.run("paint.setFill", json!({"none": true})).unwrap();
    let h = harness(none, menu);
    assert!(h.get_by_label("   Copy Color As").accesskit_node().is_disabled());
}

/// While the working RGB space is not sRGB, Copy Color As › CSS oklch() is gray, the other items are
/// not, and copying as oklch() puts the error in the status bar and nothing on the clipboard.
#[test]
fn css_oklch_is_gray_while_the_working_space_is_not_srgb() {
    const CHILD: &str = "VECTORCRAFT_TEST_COLOR_WHEEL_COPY_P3";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panels::tests_color_wheel_copy::css_oklch_is_gray_while_the_working_space_is_not_srgb", "--nocapture"])
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
    let h = harness(app(100.0), copy_menu);
    assert!(!h.get_by_label("   CSS oklch()").accesskit_node().is_disabled(), "sRGB");
    let mut app = app(100.0);
    app.run("edit.colorSettings", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    assert!(copy(&mut app, "oklch").is_empty());
    assert!(app.ui.status.ends_with("OKLCH needs sRGB as the working RGB space"), "{}", app.ui.status);
    let h = harness(app, copy_menu);
    assert!(h.get_by_label("   CSS oklch()").accesskit_node().is_disabled());
    for label in ["   Hex", "   Hex with Alpha", "   CSS rgb()", "   CSS hsl()", "   CMYK Values"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
}
