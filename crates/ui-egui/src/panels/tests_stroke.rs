//! Headless frames of the Stroke panel's Dashed Line section.

use serde_json::json;

use crate::tests::panel_harness::Panel;
// PLAN: assumed path of object-panels-w8's `Panel` harness (op.A15).
use super::tests_appearance::{app_with_rect, run};
use super::*;

/// Click the editor's Dashed check box and return the stroke's dash afterwards.
fn toggle_dashes(panel: &mut Panel) -> Option<vectorcraft_doc::Dash> {
    panel.click("Dashed line");
    current_stroke(&panel.app()).unwrap().dash
}

#[test]
fn turning_dashes_on_fits_them_to_corners_and_turning_back_on_restores_the_last_ones() {
    let mut panel = Panel::new(app_with_rect(), stroke::show);
    let d = toggle_dashes(&mut panel).expect("dashes on");
    assert_eq!((d.pattern, d.align_corners), (vec![12.0, 12.0], true));
    // Exact 4/2 dashes turned off and on again come back as they were.
    run(&mut panel.app(), "stroke.set", json!({"dash": [4, 2], "alignDashes": false}));
    assert!(toggle_dashes(&mut panel).is_none());
    let d = toggle_dashes(&mut panel).expect("dashes on");
    assert_eq!((d.pattern, d.align_corners), (vec![4.0, 2.0], false));
}
