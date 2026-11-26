//! Headless frames of panel rows that the framework lays out: controls side by side stay inside the panel's width.

use serde_json::json;

use super::tests_appearance::app_with_rect;
use super::*;
use crate::i18n::Lang;
use crate::panel_docking::PANEL_MIN_WIDTH;
use crate::tests::panel_harness::{Panel, ROOM};

#[test]
fn the_arrowhead_row_stays_inside_the_stroke_panel() {
    let panel = Panel::new(app_with_rect(), stroke::show);
    // The Arrowheads field spans the panel's width.
    let row = panel.rect("Arrowheads");
    for name in ["Start arrowhead", "End arrowhead", "Swap start and end arrowheads"] {
        assert!(row.contains_rect(panel.rect(name)), "{name} at {:?} is outside the row {row:?}", panel.rect(name));
    }
}

#[test]
fn the_color_themes_hex_row_stays_inside_a_narrow_panel() {
    let mut panel = Panel::new(app_with_rect(), color_themes::show);
    panel.room(egui::vec2(PANEL_MIN_WIDTH, ROOM.y)).frame();
    // The tab strip spans the panel's width.
    let width = panel.rect("Color Themes");
    for name in ["Color", "Set the current fill or stroke to the selected color", "Set the selected color to the current color"] {
        let at = panel.rect(name);
        assert!(at.left() >= width.left() && at.right() <= width.right(), "{name} at {at:?} is outside the panel {width:?}");
    }
}

/// Panel `panel` floated `width` wide in the whole app window, drawn in `lang`: the rect of its body, and the rects of
/// the controls named each of `names` (catalog strings, shown in `lang`).
fn floated(panel: &str, lang: &str, width: f32, names: &[&str]) -> (egui::Rect, Vec<(String, Vec<egui::Rect>)>) {
    use egui::accesskit::Role;
    const AT: f32 = 520.0;
    let mut app = app_with_rect();
    app.session.prefs.interface_language = lang.into();
    app.run("window.panel", json!({ "panel": panel })).unwrap();
    app.run("window.panel.float", json!({ "panel": panel, "x": AT, "y": 120, "width": width, "height": 700 })).unwrap();
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let mut nodes = Vec::new();
    for at in 1..=8 {
        let input = egui::RawInput { time: Some(f64::from(at) * 0.1), screen_rect: Some(screen), ..Default::default() };
        let out = crate::testing::app_frame(&mut app, &ctx, input);
        nodes = out.platform_output.accesskit_update.map_or_else(Vec::new, |update| update.nodes);
    }
    // The frames set the language from the preference; the names are looked up in it before English is put back.
    let shown: Vec<&str> = names.iter().map(|name| crate::i18n::t(name)).collect();
    crate::i18n::set_current(Lang::EN);
    let rect = |node: &egui::accesskit::Node| {
        node.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
    };
    // The floating panel's body: the tab panel at the left edge the float asked for.
    let body = nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::TabPanel)
        .find_map(|(_, n)| rect(n).filter(|r| (r.left() - AT).abs() < 1.0))
        .unwrap_or_else(|| panic!("{panel} does not float at x {AT}"));
    assert!(body.width() <= width + 1.0, "{panel}'s body {body:?} is wider than the {width} the float asked for");
    let controls = shown
        .into_iter()
        .map(|name| {
            let found: Vec<egui::Rect> = nodes.iter().filter(|(_, n)| n.label().is_some_and(|l| l == name)).filter_map(|(_, n)| rect(n)).collect();
            assert!(!found.is_empty(), "{panel} shows no {name:?}");
            (name.to_owned(), found)
        })
        .collect();
    (body, controls)
}

/// The controls named `names` (catalog strings, shown in `lang`) that reach past either side of panel `panel`,
/// floated at the narrowest panel width in the whole app window, each with its rect.
fn past_a_narrow_panel(panel: &str, lang: &str, names: &[&str]) -> Vec<String> {
    let (body, controls) = floated(panel, lang, PANEL_MIN_WIDTH, names);
    let outside = |r: &egui::Rect| r.left() < body.left() - 0.5 || r.right() > body.right() + 0.5;
    controls.iter().flat_map(|(name, found)| found.iter().filter(|r| outside(r)).map(move |r| format!("{name} at {r:?}"))).collect()
}

#[test]
fn the_profile_row_stays_on_one_line_in_a_wide_stroke_panel() {
    let (_, controls) = floated("stroke", "en", 360.0, &["Profile", "Flip Along", "Flip Across"]);
    // The field, its caption, the list and the two buttons share one line, so their middles are level.
    let middles: Vec<f32> = controls.iter().flat_map(|(_, found)| found.iter().map(|r| r.center().y)).collect();
    assert!(middles.windows(2).all(|m| (m[0] - m[1]).abs() < 1.0), "{controls:?}");
}

#[test]
fn the_profile_row_wraps_in_a_narrow_stroke_panel() {
    let past = past_a_narrow_panel("stroke", "en", &["Profile", "Flip Along", "Flip Across"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_attributes_rows_wrap_in_a_narrow_panel() {
    let past = past_a_narrow_panel("attributes", "en", &["Fill Rule", "Image Map", "Browser", "URL"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_variables_footer_wraps_in_a_narrow_panel() {
    let past = past_a_narrow_panel("variables", "en", &["Delete Variable", "Lock Variables"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_align_row_wraps_in_a_narrow_properties_panel() {
    let past = past_a_narrow_panel("properties", "en", &["Vertical Align Bottom", "More Align options"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_document_info_rows_wrap_in_a_narrow_panel_in_german() {
    let past = past_a_narrow_panel("docInfo", "de", &["Flattener Preset"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_magic_wand_rows_wrap_in_a_narrow_panel_in_french() {
    let past = past_a_narrow_panel("magicWand", "fr", &["Tolerance:"]);
    assert!(past.is_empty(), "past the panel: {past:?}");
}

#[test]
fn the_color_wheel_box_and_its_strip_stay_inside_the_panel() {
    let mut app = app_with_rect();
    app.ui.color_wheel.view = color_wheel::View::Box;
    let panel = Panel::new(app, color_wheel::show);
    let (field, strip) = (panel.rect("Color box"), panel.rect("Hue"));
    assert!(strip.height() <= field.height(), "the strip {strip:?} is taller than the box {field:?}");
    assert!(field.right() <= ROOM.x && strip.right() <= ROOM.x, "the box {field:?} or its strip {strip:?} runs past the panel");
}

#[test]
fn align_to_shares_the_distribute_spacing_line_inside_the_align_panel() {
    let panel = Panel::new(app_with_rect(), align::show);
    let (field, to) = (panel.rect("Distribute Spacing"), panel.rect("Align to Artboard"));
    assert!(to.left() > field.right(), "Align To at {to:?} is not beside the spacing field {field:?}");
    assert!((field.top()..=field.bottom()).contains(&to.center().y), "Align To at {to:?} is off the spacing field's line {field:?}");
    assert!(to.right() <= ROOM.x, "Align To at {to:?} runs past the panel");
}
