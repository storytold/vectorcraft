//! Shared widgets. Colours come from theme tokens.

use std::ops::RangeInclusive;

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::Unit;

use crate::theme::{self, Tokens};
use crate::{icons, scrub};

mod blend;
mod buttons;
mod chips;
mod color;
mod drag;
mod dropdowns;
mod labels;
mod numeric;
mod panel_chrome;
mod text;
mod toggles;

pub use blend::{
    TransparencyEdit, angle_dial, blend_dropdown, blend_param, blend_param_dropdown, blend_separator_before, opacity_blend, reference_point,
};
pub use buttons::{
    SPLIT_ARROW, flat_button, flat_button_fit, icon_button, icon_button_enabled, menu_item, menu_item_name, paint_icon_button, primary_button,
    primary_button_enabled, secondary_button, split_button, split_button_named, toggle_icon,
};
pub(crate) use buttons::{icon_button_info, primary_button_width, secondary_button_width};
#[cfg(test)]
use chips::gradient_chip_colors;
pub use chips::{ProxyClicks, chip_button, chip_button_named, fill_stroke_proxy, gradient_chip, paint_chip, proxy_chip, swatch_tile};
pub(crate) use color::pointer_phase;
pub use color::{
    Live, SliderValue, WheelResponse, channel_slider, channel_slider_named, channel_slider_valued, color_field, color_mesh, color_slider,
    color_slider_named, color_slider_stepped, color_slider_valued, harmony_wheel, hex_field, hex_field_named, region_texture,
};
pub use drag::{PanelDrag, SwatchRows, art_drop, doc_preview, drag_source};
#[cfg(test)]
use dropdowns::combo_button_id;
pub(crate) use dropdowns::{combo, combo_menu};
pub use dropdowns::{
    dropdown, dropdown_named, dropdown_names, dropdown_names_named, dropdown_with, dropdown_with_named, list_scroll_style, menu_room, menu_scroll,
    panel_scroll, popover, set_menu_floor, strip_scroll,
};
pub use labels::{dim_label, dim_name, divider, field_label, label_row, link_label, section_header, section_toggle, section_toggle_name, subheader};
pub(crate) use labels::{label_field, take_previous_label};
pub use numeric::{
    SpinPick, mixed_field, mixed_field_named, num_field, num_field_named, opt_field, opt_field_named, range_field, range_field_named,
    range_field_stepped, size_field, size_field_named, slider, spin_field, spin_field_auto, spin_field_auto_named, spin_field_named,
    spin_field_stops_named, spin_plain, spin_plain_named,
};
#[cfg(test)]
use numeric::{decimal_point, share_of, spinner_step, step_through, step_to_multiple, within_a_millionth};
pub(crate) use numeric::{round_to, set_bare_numbers_are_points, typed_unit};
pub use panel_chrome::{bottom_bar, layout_button, list_box, orientation_button, tab_bar};
pub(crate) use text::{
    arm_field_focus, dialog_focus_flag, disarm_field_focus, enter_modifiers, focus_field, own_field_shift_enter, set_dialog_open, take_dialog_focus,
};
pub use text::{
    exact_text_field, exact_text_field_named, plain_field, plain_field_named, search_field, text_field, text_field_named, text_presets,
    text_presets_named,
};
pub use toggles::{check, check_box, check_name, check_tip, check3, radio, radio_button};

/// A widget id salt that needs only [`std::hash::Hash`]. egui 0.37's [`egui::AsIdSalt`] also
/// requires `Debug` for its debug-only id-source registry; VectorCraft's public field helpers do
/// not need a `Debug` bound merely to salt a widget id, and never make an accessible name from it.
#[derive(Clone, Copy)]
struct HashOnly<T>(T);

impl<T: std::hash::Hash> std::hash::Hash for HashOnly<T> {
    /// Hash only the wrapped id salt, deliberately omitting the debug-only identity that egui 0.37's
    /// `AsIdSalt` requires.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T> std::fmt::Debug for HashOnly<T> {
    /// Format any `HashOnly` salt as the fixed text `widget id` so ids do not expose their value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("widget id")
    }
}

/// `parent.with(salt)` for a salt that needs only `Hash`.
fn id_with(parent: egui::Id, salt: impl std::hash::Hash) -> egui::Id {
    parent.with(HashOnly(salt))
}

#[cfg(test)]
pub(crate) mod accessibility_tests {
    use egui::accesskit::{Node, Role, Toggled};

    use super::*;

    /// The screen-reader tree of the second of two frames of `add` in a 1280 × 900 window, in
    /// English.
    pub(crate) fn tree(mut add: impl FnMut(&mut Ui)) -> Vec<Node> {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0));
        let mut nodes = vec![];
        for time in [0.0, 0.1] {
            let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, &mut add);
            out.textures_delta.clear();
            nodes = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
        }
        nodes
    }

    /// The node of `tree` that screen readers name `label`.
    pub(crate) fn named<'a>(tree: &'a [Node], label: &str) -> &'a Node {
        tree.iter().find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("nothing is named {label:?}"))
    }

    /// The role of the node named `label`, and whether it is pressed, checked or chosen.
    pub(crate) fn role_and_state(tree: &[Node], label: &str) -> (Role, Option<Toggled>) {
        let n = named(tree, label);
        (n.role(), n.toggled())
    }

    #[test]
    fn buttons_are_named_by_their_text_or_tip() {
        let tree = tree(|ui| {
            icon_button(ui, "x", "Remove", false, 24.0);
            flat_button(ui, "Make", 64.0);
            split_button_named(ui, "Image Trace", "Tracing Presets", 104.0);
            primary_button(ui, "OK");
            primary_button_enabled(ui, "Create", false);
            secondary_button(ui, "Cancel");
            link_label(ui, "Stroke:");
            icon_button_enabled(ui, "trash-2", "Delete Brush", false, false, 24.0);
            fill_stroke_proxy(ui, &Paint::None, &Paint::None, (false, false), true, 36.0);
            chip_button_named(ui, 22.0, true, "Graphic Style", |_, _| {});
            spin_field(ui, "weight", Some(1.0), Unit::Points, 80.0, 1.0, 0.0, &[1.0, 2.0]);
            opacity_blend(ui, "opacity", Some(1.0), Some(BlendMode::Normal), true);
        });
        for label in [
            "Remove",
            "Make",
            "Image Trace",
            "Tracing Presets",
            "OK",
            "Create",
            "Cancel",
            "Stroke:",
            "Delete Brush",
            "Swap Fill and Stroke (Shift+X)",
            "Graphic Style",
            "Step up or down",
            "Presets",
            "Opacity",
        ] {
            assert_eq!(role_and_state(&tree, label), (Role::Button, None), "{label}");
        }
        assert_eq!(role_and_state(&tree, "Default Fill and Stroke (D)"), (Role::Button, None));
        assert_eq!(role_and_state(&tree, "Fill (X), double-click for the Color Picker"), (Role::Button, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Stroke (X), double-click for the Color Picker"), (Role::Button, Some(Toggled::False)));
        assert!(named(&tree, "Create").is_disabled(), "a primary button that is not enabled");
        assert!(named(&tree, "Delete Brush").is_disabled(), "an icon button that is not enabled");
        assert!(!named(&tree, "OK").is_disabled());
    }

    #[test]
    fn toggles_say_whether_they_are_on() {
        let tree = tree(|ui| {
            icon_button(ui, "link", "Constrain Proportions", true, 24.0);
            toggle_icon(ui, "eye", true, 18.0, "Show all plates");
            toggle_icon(ui, "eye", false, 18.0, "Show/hide plate (Alt-click: only this plate)");
            orientation_button(ui, false, true, "Portrait");
            orientation_button(ui, true, false, "Landscape");
            layout_button(ui, &[(0, 0), (1, 0)], true, "Grid by Row");
            tab_bar(ui, &["Recent", "Saved"], 1);
        });
        let on = |label| assert_eq!(role_and_state(&tree, label), (Role::Button, Some(Toggled::True)), "{label}");
        let off = |label| assert_eq!(role_and_state(&tree, label), (Role::Button, Some(Toggled::False)), "{label}");
        on("Constrain Proportions");
        on("Show all plates");
        off("Show/hide plate (Alt-click: only this plate)");
        on("Portrait");
        off("Landscape");
        on("Grid by Row");
        off("Recent");
        on("Saved");
    }

    #[test]
    fn checkboxes_and_radio_buttons_are_named_and_checked() {
        let tree = tree(|ui| {
            check(ui, "Preview", true, true);
            check(ui, "Snap to Pixel", false, false);
            check3(ui, "Overprint Fill", None, true);
            check_name(ui, "Artboard 1", true);
            check_box(ui, "Cyan", false, true);
            radio(ui, "Relative", true, true);
            radio(ui, "Absolute", false, true);
            radio_button(ui, "per slice", true, true);
            reference_point(ui, 4);
        });
        assert_eq!(role_and_state(&tree, "Preview"), (Role::CheckBox, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Snap to Pixel"), (Role::CheckBox, Some(Toggled::False)));
        assert!(named(&tree, "Snap to Pixel").is_disabled());
        assert_eq!(role_and_state(&tree, "Overprint Fill"), (Role::CheckBox, Some(Toggled::Mixed)));
        assert_eq!(role_and_state(&tree, "Artboard 1"), (Role::CheckBox, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Relative"), (Role::RadioButton, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Absolute"), (Role::RadioButton, Some(Toggled::False)));
        assert_eq!(role_and_state(&tree, "Cyan"), (Role::CheckBox, Some(Toggled::False)));
        assert_eq!(role_and_state(&tree, "per slice"), (Role::RadioButton, Some(Toggled::True)));
        for (i, label) in ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"].into_iter().enumerate()
        {
            let chosen = if i == 4 { Toggled::True } else { Toggled::False };
            assert_eq!(role_and_state(&tree, label), (Role::RadioButton, Some(chosen)), "{label}");
        }
    }

    #[test]
    fn sliders_dials_and_combo_boxes_are_named() {
        let red = Color::from_hsb(0.0, 1.0, 1.0);
        let tree = tree(|ui| {
            color_slider_named(ui, "fidelity", "Fidelity:", 0.25, 120.0, &|_| Color32::GRAY);
            angle_dial(ui, "angle", 45.0, 28.0);
            channel_slider_named(ui, "channel", "H", 0.5, vec2(22.0, 80.0), &|_| Color32::GRAY);
            harmony_wheel(ui, "wheel", 120.0, &[red], Some(0));
            combo_menu(ui, "font", "Regular", 120.0, |_| None::<usize>);
        });
        let slider = |label| {
            let n = named(&tree, label);
            (n.role(), n.numeric_value())
        };
        assert_eq!(slider("Fidelity:"), (Role::Slider, None));
        assert_eq!(slider("Angle"), (Role::Slider, Some(45.0)));
        assert_eq!(slider("H"), (Role::Slider, None));
        assert_eq!(slider("Color wheel"), (Role::Slider, None));
        assert!(tree.iter().any(|n| n.role() == Role::ComboBox && n.value() == Some("Regular")), "the searchable combo box shows its text");
    }

    /// The number, range and text of the slider of `tree` that screen readers name `label`.
    pub(crate) fn slider_value(tree: &[Node], label: &str) -> (Option<f64>, Option<f64>, Option<f64>, Option<String>) {
        let n = tree.iter().find(|n| n.role() == Role::Slider && n.label() == Some(label)).unwrap_or_else(|| panic!("no slider is named {label:?}"));
        (n.numeric_value(), n.min_numeric_value(), n.max_numeric_value(), n.value().map(str::to_string))
    }

    /// The `_valued` sliders give screen readers the number and text of their field, with the rail's
    /// range extended to a number past it; the harmony wheel's Brightness gives its percent.
    #[test]
    fn sliders_report_the_value_their_field_shows() {
        let dark_red = Color::from_hsb(0.0, 1.0, 0.5);
        let gray = |_: f32| Color32::GRAY;
        let tree = tree(|ui| {
            color_slider_valued(ui, "opacity", "Opacity:", Some(SliderValue::field(49.6, 0.0..=100.0, "%", 0)), 0.5, 120.0, &gray);
            color_slider_valued(ui, "scale", "Horizontal:", Some(SliderValue::field(312.5, 0.0..=200.0, "%", 2)), 1.0, 120.0, &gray);
            color_slider_valued(ui, "move", "Vertical:", Some(SliderValue::text(-1.5, -14.0..=14.0, "-1.5 in".into())), 0.4, 120.0, &gray);
            color_slider_valued(ui, "count", "Ridges per segment:", Some(SliderValue::number(4.0, 0.0..=100.0)), 0.04, 120.0, &gray);
            color_slider_valued(ui, "axis", "Weight", None, 0.5, 120.0, &gray);
            channel_slider_valued(ui, "channel", "H", Some(SliderValue::field(210.0, 0.0..=360.0, "°", 0)), 210.0 / 360.0, vec2(22.0, 80.0), &gray);
            harmony_wheel(ui, "wheel", 120.0, &[dark_red], Some(0));
        });
        let s = |label| slider_value(&tree, label);
        assert_eq!(s("Opacity:"), (Some(50.0), Some(0.0), Some(100.0), Some("50%".into())));
        assert_eq!(s("Horizontal:"), (Some(312.5), Some(0.0), Some(312.5), Some("312.5%".into())));
        assert_eq!(s("Vertical:"), (Some(-1.5), Some(-14.0), Some(14.0), Some("-1.5 in".into())));
        assert_eq!(s("Ridges per segment:"), (Some(4.0), Some(0.0), Some(100.0), None));
        assert_eq!(s("Weight"), (None, None, None, None), "a mixed value");
        assert_eq!(s("H"), (Some(210.0), Some(0.0), Some(360.0), Some("210°".into())));
        assert_eq!(s("Brightness"), (Some(50.0), Some(0.0), Some(100.0), Some("50%".into())));
    }
}

#[cfg(test)]
mod spinner_modifier_tests {
    use super::*;

    fn pt(v: f64) -> String {
        Unit::Points.format(v)
    }

    #[test]
    fn shift_steps_stroke_weight_by_ten_points() {
        assert_eq!(spinner_step(14.0, 1.0, 0.0, true, egui::Modifiers::SHIFT, &[], &pt), 24.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, false, egui::Modifiers::SHIFT, &[], &pt), 4.0);
    }

    #[test]
    fn unmodified_and_command_steps_keep_existing_behavior() {
        assert_eq!(spinner_step(14.0, 1.0, 0.0, true, egui::Modifiers::NONE, &[], &pt), 15.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, false, egui::Modifiers::COMMAND, &[], &pt), 13.9);
        assert_eq!(spinner_step(0.0, 1.0, 0.0, false, egui::Modifiers::SHIFT, &[], &pt), 0.0);
    }

    #[test]
    fn plain_and_command_steps_go_to_the_next_multiple_as_arrow_keys_do() {
        // As ↑/↓ in the field (#2881): a plain step goes to the next whole number, Ctrl/Cmd to the
        // next tenth, rounded to the places fields keep; Shift adds or takes ten.
        assert_eq!(spinner_step(1.37, 1.0, 0.0, true, egui::Modifiers::NONE, &[], &pt), 2.0);
        assert_eq!(spinner_step(1.37, 1.0, 0.0, false, egui::Modifiers::NONE, &[], &pt), 1.0);
        assert_eq!(spinner_step(1.37, 1.0, 0.0, true, egui::Modifiers::COMMAND, &[], &pt), 1.4);
        assert_eq!(spinner_step(1.37, 1.0, 0.0, false, egui::Modifiers::COMMAND, &[], &pt), 1.3);
        assert_eq!(spinner_step(1.37, 1.0, 0.0, true, egui::Modifiers::SHIFT, &[], &pt), 11.37);
        assert_eq!(spinner_step(0.37, 1.0, 0.0, false, egui::Modifiers::NONE, &[], &pt), 0.0);
    }

    /// A plain click on the spinner stops at each stop; Shift and Ctrl/Cmd steps do not.
    #[test]
    fn a_plain_spinner_step_stops_at_the_stops() {
        let stops = [0.25, 0.5, 0.75, 1.0, 2.0, 10.0, 20.0];
        let plain = egui::Modifiers::NONE;
        assert_eq!(spinner_step(0.52, 1.0, 0.0, false, plain, &stops, &pt), 0.5);
        assert_eq!(spinner_step(0.52, 1.0, 0.0, true, plain, &stops, &pt), 0.75);
        assert_eq!(spinner_step(0.1, 1.0, 0.0, false, plain, &stops, &pt), 0.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, true, plain, &stops, &pt), 15.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, false, egui::Modifiers::SHIFT, &stops, &pt), 4.0);
        assert_eq!(spinner_step(0.52, 1.0, 0.0, true, egui::Modifiers::COMMAND, &stops, &pt), 0.6);
    }

    /// A stop the spinner's text shows as the value counts as reached: 2.004 pt and the 2 pt stop
    /// both show as `0.167 p`, so a click down goes on to 1 pt.
    #[test]
    fn a_spinner_stop_shown_as_the_value_is_passed() {
        let pc = |v: f64| Unit::Picas.format(v);
        assert_eq!(spinner_step(2.004, 1.0, 0.0, false, egui::Modifiers::NONE, &[1.0, 2.0, 3.0], &pc), 1.0);
        assert_eq!(spinner_step(2.004, 1.0, 0.0, true, egui::Modifiers::NONE, &[1.0, 2.0, 3.0], &pc), 3.0);
    }
}

#[cfg(test)]
mod step_through_tests {
    use super::*;

    /// Each step goes to the nearer of the next whole step and the next stop in its direction.
    #[test]
    fn a_step_goes_to_the_nearer_of_the_next_multiple_and_the_next_stop() {
        let stops = [0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 10.0, 20.0, 100.0];
        for (v, n, want) in [
            (0.52, -1.0, 0.5),
            (0.51, -1.0, 0.5),
            (0.4, -1.0, 0.25),
            (0.52, 1.0, 0.75),
            (0.1, 1.0, 0.25),
            (0.1, -1.0, 0.0),
            (0.25, -1.0, 0.0),
            (0.75, -1.0, 0.5),
            (1.0, -1.0, 0.75),
            (1.0, 1.0, 2.0),
            (1.5, -1.0, 1.0),
            (14.0, 1.0, 15.0),
            (14.0, -1.0, 13.0),
            (14.5, 1.0, 15.0),
            (0.4, -2.0, 0.0),
            (0.4, 3.0, 1.0),
            (100.0, 1.0, 101.0),
            (0.52, 0.0, 0.52),
        ] {
            let got = step_through(v, 1.0, n, &stops, within_a_millionth(1.0));
            assert!((got - want).abs() < 1e-9, "{v} × {n}: {got}");
        }
    }

    /// A value a float error away from a stop moves on to the next one.
    #[test]
    fn a_value_next_to_a_stop_moves_past_it() {
        assert_eq!(step_through(0.5 + 1e-9, 1.0, -1.0, &[0.25, 0.5], within_a_millionth(1.0)), 0.25);
        assert_eq!(step_through(0.5 - 1e-9, 1.0, 1.0, &[0.5, 0.75], within_a_millionth(1.0)), 0.75);
    }

    /// A stop shown with the same text as the value moves on to the next one: 2 pt shows as
    /// `0.167 p`. A stop shown differently is still reached: `0.016 in` goes down to `0.0156 in`.
    #[test]
    fn a_stop_shown_as_the_value_moves_past_it() {
        let shown = |unit: Unit| move |a: f64, b: f64| unit.number(unit.to_pt(a)) == unit.number(unit.to_pt(b));
        let picas = [1.0 / 12.0, 2.0 / 12.0, 3.0 / 12.0];
        assert_eq!(step_through(0.167, 1.0, -1.0, &picas, shown(Unit::Picas)), 1.0 / 12.0);
        assert_eq!(step_through(0.083, 1.0, 1.0, &picas, shown(Unit::Picas)), 2.0 / 12.0);
        assert_eq!(step_through(0.016, 1.0, -1.0, &[0.0078, 0.0156], shown(Unit::Inches)), 0.0156);
        // The next whole step shown as the value is passed too: 2.0004 pt shows as `2 pt`.
        assert_eq!(step_through(2.0004, 1.0, -1.0, &[1.0, 2.0], shown(Unit::Points)), 1.0);
        assert_eq!(step_through(2.0004, 1.0, -1.0, &[], shown(Unit::Points)), 1.0);
    }
}

#[cfg(test)]
mod step_to_multiple_tests {
    use super::*;

    /// #2881: the first step goes to the next multiple of its size in its direction, and each further
    /// step adds or takes one size.
    #[test]
    fn a_step_goes_to_the_next_multiple_of_its_size() {
        for (v, size, n, want) in [
            (5.1, 1.0, 1.0, 6.0),
            (5.1, 1.0, -1.0, 5.0),
            (5.0, 1.0, 1.0, 6.0),
            (5.0, 1.0, -1.0, 4.0),
            (-2.5, 1.0, 1.0, -2.0),
            (-2.5, 1.0, -1.0, -3.0),
            (24.8, 1.0, 2.0, 26.0),
            (24.8, 1.0, -2.0, 23.0),
            (5.13, 0.1, 1.0, 5.2),
            (5.13, 0.1, -1.0, 5.1),
            (5.1, 1.0, 0.0, 5.1),
        ] {
            assert!((step_to_multiple(v, size, n) - want).abs() < 1e-9, "{v} by {size} × {n}: {}", step_to_multiple(v, size, n));
        }
    }

    /// A value a float error away from a multiple moves a whole step from it: 0.7 is 6.999… tenths.
    #[test]
    fn a_value_next_to_a_multiple_moves_a_whole_step() {
        assert!((step_to_multiple(0.7, 0.1, 1.0) - 0.8).abs() < 1e-9);
        assert!((step_to_multiple(0.3, 0.1, -1.0) - 0.2).abs() < 1e-9);
        assert!((step_to_multiple(5.000_000_1, 1.0, 1.0) - 6.0).abs() < 1e-9);
        assert!((step_to_multiple(4.999_999_9, 1.0, -1.0) - 4.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod decimal_point_tests {
    use super::*;

    /// A decimal comma reads as a point, in a length, with arithmetic and in a percentage; text
    /// with a point keeps its commas and is refused as before, and so does a thousands comma
    /// (#3837).
    #[test]
    fn a_decimal_comma_reads_as_a_point() {
        let mm = |s: &str| Unit::Millimeters.parse(&decimal_point(s)).map(|pt| Unit::Millimeters.from_pt(pt));
        for (text, want) in [("0,1", 0.1), ("0,1 mm", 0.1), ("0,1mm", 0.1), ("-0,5", -0.5), ("1,5+0,25", 1.75), ("0.1", 0.1)] {
            let got = mm(text).unwrap_or(f64::NAN);
            assert!((got - want).abs() < 1e-9, "{text}: {got}");
        }
        assert_eq!(mm("1,000.5"), None, "a point and a comma");
        assert_eq!(mm("1,2,3"), None, "two commas");
        for thousand in ["1,000", "12,500 mm", "-1,000", "999,999mm"] {
            assert_eq!(mm(thousand), None, "{thousand} reads as thousands: refused, not a thousand times too small");
        }
        for (text, want) in [("0,125", 0.125), ("1,25", 1.25), ("1,0000", 1.0), ("1,500+2", 3.5)] {
            let got = mm(text).unwrap_or(f64::NAN);
            assert!((got - want).abs() < 1e-9, "{text} is a decimal: {got}");
        }
        assert_eq!(share_of(&decimal_point("12,5%"), 200.0), Some(25.0));
    }
}

#[cfg(test)]
mod menu_room_tests {
    use super::*;

    #[test]
    fn menu_stops_above_the_bottom_bars() {
        // Window 1000 pt tall, bars start at 948: a menu opening at y = 100 gets 836 pt, not 888.
        assert_eq!(menu_room(948.0, 100.0), 836.0);
    }

    #[test]
    fn menu_keeps_a_minimum_height() {
        assert_eq!(menu_room(300.0, 290.0), 120.0);
    }
}

#[cfg(test)]
mod gradient_chip_tests {
    use super::*;
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint};

    fn paint(kind: GradientKind, angle: f64) -> GradientPaint {
        let mut gp = GradientPaint::new(Gradient { kind, ..Gradient::default() });
        gp.angle = angle;
        gp
    }

    #[test]
    fn linear_chip_follows_the_angle() {
        // 5 x 5 grid points on a 20 x 20 chip; White (start) to Black (end).
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i];
        let side = gradient_chip_colors(&paint(GradientKind::Linear, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&side, 0, 2), Color32::WHITE);
        assert_eq!(at(&side, 4, 2), Color32::BLACK);
        assert_eq!(at(&side, 2, 0), at(&side, 2, 4));
        // 90 degrees runs bottom to top: constant along rows, white at the bottom.
        let up = gradient_chip_colors(&paint(GradientKind::Linear, 90.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&up, 0, 4), Color32::WHITE);
        assert_eq!(at(&up, 0, 0), Color32::BLACK);
        assert_eq!(at(&up, 0, 2), at(&up, 4, 2));
    }

    #[test]
    fn radial_chip_runs_from_the_centre_out() {
        let colors = gradient_chip_colors(&paint(GradientKind::Radial, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(colors[2 * 5 + 2], Color32::WHITE);
        assert_eq!(colors[0], Color32::BLACK);
    }
}

#[cfg(test)]
mod combo_keyboard_tests {
    use super::*;

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, picked: &mut Option<usize>) {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if let Some(i) = dropdown(ui, "combo-test", "One", &["One", "Two", "Three"], 200.0) {
                *picked = Some(i);
            }
        });
        // No renderer takes the font atlas updates in a test; dropping them unapplied panics.
        out.textures_delta.clear();
    }

    /// An option picked with the keyboard closes the list (it used to stay open and keep the keys).
    #[test]
    fn option_picked_with_space_closes_the_list() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut picked = None;
        frame(&ctx, vec![], &mut picked);
        frame(&ctx, vec![], &mut picked);
        // Tab to the combo, open it with Space.
        for k in [egui::Key::Tab, egui::Key::Space] {
            frame(&ctx, vec![key(k, true)], &mut picked);
            frame(&ctx, vec![key(k, false)], &mut picked);
        }
        assert!(egui::Popup::is_any_open(&ctx), "Space opens the list");
        // Tab into the list, then Space picks the option under the focus.
        for _ in 0..8 {
            frame(&ctx, vec![key(egui::Key::Tab, true)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Tab, false)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Space, true)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Space, false)], &mut picked);
            if picked.is_some() {
                break;
            }
        }
        assert!(picked.is_some(), "an option was picked");
        frame(&ctx, vec![], &mut picked);
        assert!(!egui::Popup::is_any_open(&ctx), "the list closes after a keyboard pick");
    }
}

#[cfg(test)]
mod button_tests {
    use super::*;

    /// [`flat_button_fit`] preserves minimum width for short labels and expands for long ones.
    #[test]
    fn flat_button_fit_expands_for_long_text() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let raw = egui::RawInput::default();
        let mut out = ctx.run_ui(raw, |ui| {
            let resp_short = flat_button_fit(ui, "OK", 100.0);
            assert_eq!(resp_short.rect.width(), 100.0);

            let resp_long = flat_button_fit(ui, "Çalışma Yüzeylerini Düzenle", 100.0);
            assert!(resp_long.rect.width() > 160.0);
        });
        out.textures_delta.clear();
    }
}

/// How the shared widgets look once laid out and painted: shapes of whole frames, measured.
#[cfg(test)]
pub(crate) mod look_tests;
