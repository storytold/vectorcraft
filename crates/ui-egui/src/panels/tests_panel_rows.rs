//! Headless frames of panel rows that the framework lays out: controls side by side stay inside the panel's width.

use super::tests_appearance::app_with_rect;
use super::*;
use crate::panel_docking::PANEL_MIN_WIDTH;
use crate::tests::panel_harness::{Panel, Place, ROOM, floated};

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

/// The controls named `names` (catalog strings, shown in `lang`) that reach past either side of panel `panel`,
/// floated at the narrowest panel width in the whole app window, each with its rect.
fn past_a_narrow_panel(panel: &str, lang: &str, names: &[&str]) -> Vec<String> {
    floated(app_with_rect(), panel, lang, Place::Floating(PANEL_MIN_WIDTH), names).past_named(names)
}

#[test]
fn the_profile_row_stays_on_one_line_in_a_wide_stroke_panel() {
    let names = ["Profile", "Flip Along", "Flip Across"];
    let panel = floated(app_with_rect(), "stroke", "en", Place::Floating(360.0), &names);
    // The field, its caption, the list and the two buttons share one line, so their middles are level.
    let middles = panel.middles(&names);
    assert!(middles.windows(2).all(|m| (m[0] - m[1]).abs() < 1.0), "{middles:?}");
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
    use crate::i18n::{Lang, LangExt as _, tr};

    let captions = ["Tolerance:"];
    let panel = floated(app_with_rect(), "magicWand", "fr", Place::Floating(PANEL_MIN_WIDTH), &captions);
    let prefix = format!("{} ", tr(Lang::from_code("fr").unwrap(), "Tolerance:"));
    let fields: Vec<_> = panel.nodes.iter().filter(|(name, _)| name.starts_with(&prefix)).cloned().collect();
    assert_eq!(fields.len(), 4, "{fields:?}");
    let mut past = panel.past_named(&captions);
    past.extend(panel.past(fields));
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
fn the_symbols_panel_with_no_symbols_says_how_to_make_one() {
    let panel = Panel::new(app_with_rect(), symbols::show);
    assert!(panel.shows("No symbols in this document"));
    assert!(panel.shows("Select art and click New Symbol to make one."), "the empty panel has no hint");
}

#[test]
fn the_appearance_swatches_and_labels_line_up() {
    let panel = Panel::new(app_with_rect(), appearance::show);
    let (stroke, fill) = (panel.rect("Stroke swatch"), panel.rect("Fill swatch"));
    assert!((stroke.left() - fill.left()).abs() < 1.0, "the stroke swatch at {stroke:?} and the fill swatch at {fill:?} are in different columns");
    let opacity = panel.rect("Opacity:");
    for name in ["Stroke:", "Fill:"] {
        let at = panel.rect(name);
        assert!((at.left() - opacity.left()).abs() < 1.0, "Opacity: at {opacity:?} does not start where {name} at {at:?} does");
    }
}

#[test]
fn align_to_shares_the_distribute_spacing_line_inside_the_align_panel() {
    let panel = Panel::new(app_with_rect(), align::show);
    let (field, to) = (panel.rect("Distribute Spacing"), panel.rect("Align to Artboard"));
    assert!(to.left() > field.right(), "Align To at {to:?} is not beside the spacing field {field:?}");
    assert!((field.top()..=field.bottom()).contains(&to.center().y), "Align To at {to:?} is off the spacing field's line {field:?}");
    assert!(to.right() <= ROOM.x, "Align To at {to:?} runs past the panel");
}
