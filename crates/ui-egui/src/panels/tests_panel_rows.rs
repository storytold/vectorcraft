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
fn align_to_shares_the_distribute_spacing_line_inside_the_align_panel() {
    let panel = Panel::new(app_with_rect(), align::show);
    let (field, to) = (panel.rect("Distribute Spacing"), panel.rect("Align to Artboard"));
    assert!(to.left() > field.right(), "Align To at {to:?} is not beside the spacing field {field:?}");
    assert!((field.top()..=field.bottom()).contains(&to.center().y), "Align To at {to:?} is off the spacing field's line {field:?}");
    assert!(to.right() <= ROOM.x, "Align To at {to:?} runs past the panel");
}

#[test]
fn the_swatches_footer_stays_inside_a_short_panel() {
    let height = 400.0;
    let mut panel = Panel::new(app_with_rect(), swatches::show);
    panel.room(egui::vec2(ROOM.x, height)).frame();
    // The swatch browser fills the room the footer leaves, so every footer action is in the panel.
    for name in ["Show Swatch Kinds", "New Swatch", "New Color Group", "Delete Swatch"] {
        let at = panel.rect(name);
        assert!(at.bottom() <= height && at.top() >= 0.0, "{name} at {at:?} is outside the panel's {height} pt");
    }
}

#[test]
fn the_arrowhead_heads_and_swap_share_one_line_in_the_stroke_panel() {
    let names = ["Start arrowhead", "End arrowhead", "Swap start and end arrowheads"];
    let panel = floated(app_with_rect(), "stroke", "en", Place::Floating(340.0), &names);
    let middles = panel.middles(&names);
    let rects: Vec<&(String, egui::Rect)> = panel.nodes.iter().filter(|(name, _)| names.contains(&name.as_str())).collect();
    assert!(middles.windows(2).all(|m| (m[0] - m[1]).abs() < 1.0), "{middles:?} {rects:?} in {:?}", panel.body);
    assert!(panel.past_named(&names).is_empty(), "past the panel: {:?}", panel.past_named(&names));
}

#[test]
fn the_arrowhead_scale_and_align_rows_show_their_labels() {
    let mut panel = Panel::new(app_with_rect(), stroke::show);
    let (shapes, _) = panel.paint_at(1.0);
    let texts: Vec<(String, egui::Rect)> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.galley.text().trim_end_matches(':').to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect();
    // Each label sits at the start of its row, before the row's controls.
    for (label, control) in [("Scale", "Scale"), ("Align", "Align")] {
        let field = panel.rect(control);
        let shown =
            texts.iter().find(|(text, at)| text == label && at.right() <= field.right() && (field.top()..=field.bottom()).contains(&at.center().y));
        assert!(shown.is_some(), "no {label:?} label on its row at {field:?}; painted: {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>());
    }
}
