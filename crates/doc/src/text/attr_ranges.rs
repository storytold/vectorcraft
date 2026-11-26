//! The ranges the character and paragraph style commands keep each numeric attribute to, checked
//! in one place: the style commands check the attributes a call changes, and type a plug-in
//! returns has every attribute checked.

use std::fmt;
use std::ops::RangeInclusive;

use super::{CharStyle, JustificationOptions, ParaStyle, TabStop};

/// Font size ([`CharStyle::size`]), in points.
pub const FONT_SIZE: RangeInclusive<f64> = 0.1..=1296.0;
/// Leading ([`CharStyle::leading`]) other than Auto, in points.
pub const LEADING: RangeInclusive<f64> = 0.1..=5000.0;
/// Horizontal and vertical scale ([`CharStyle::h_scale`], [`CharStyle::v_scale`]), in percent.
pub const CHAR_SCALE: RangeInclusive<f64> = 1.0..=10000.0;
/// Tracking, kerning and the space before a line's first character ([`CharStyle::tracking`],
/// [`CharStyle::kerning`], [`CharStyle::kerning_before`]), in 1/1000 em.
pub const CHAR_SPACING: RangeInclusive<f64> = -1000.0..=10000.0;
/// The largest baseline shift and Tate-chu-yoko Up/Down and Left/Right either way, in points.
pub const MAX_BASELINE_SHIFT: f64 = 1296.0;
/// Character rotation ([`CharStyle::rotation`]), in degrees.
pub const CHAR_ROTATION: RangeInclusive<f64> = -360.0..=360.0;
/// The miter limit of a character stroke ([`CharStyle::stroke_miter_limit`]).
pub const CHAR_MITER_LIMIT: RangeInclusive<f64> = 1.0..=500.0;
/// The largest paragraph indent, space before or after and Hyphenation Zone either way, in points.
pub const MAX_PARA_DISTANCE: f64 = 1296.0;

/// An attribute outside the range the style commands keep it to.
#[derive(Clone, Debug, PartialEq)]
pub struct AttrRangeError {
    /// The attribute's JSON name, a nested field after a dot (`hyphenation.zone`).
    pub key: String,
    /// What is wrong, naming the attribute.
    pub message: String,
}

impl fmt::Display for AttrRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AttrRangeError {}

fn out(key: &str, message: String) -> AttrRangeError {
    AttrRangeError { key: key.to_string(), message }
}

/// `value` within `lo..=hi`, or the error naming `key` (a value that is not a number is outside).
fn within(key: &str, value: f64, lo: f64, hi: f64) -> Result<(), AttrRangeError> {
    if (lo..=hi).contains(&value) { Ok(()) } else { Err(out(key, format!("`{key}` must be from {lo} to {hi}"))) }
}

impl CharStyle {
    /// Checks each numeric attribute `check` names, by its JSON key, against the range the
    /// character style commands keep it to: size, leading, scales, tracking, kerning, baseline
    /// shift, rotation, the character stroke's weight, miter limit and dashes, Small Caps, Insert
    /// Aki, Tsume, diacritic positions, the Tate-chu-yoko offsets, the Warichu settings and the
    /// variable font axes. The first attribute out of its range is the error.
    pub fn check_ranges(&self, check: impl Fn(&str) -> bool) -> Result<(), AttrRangeError> {
        for (key, value, range) in [("size", self.size, FONT_SIZE), ("h_scale", self.h_scale, CHAR_SCALE), ("v_scale", self.v_scale, CHAR_SCALE)] {
            if check(key) {
                within(key, value, *range.start(), *range.end())?;
            }
        }
        if check("leading") && self.leading.is_some_and(|value| !LEADING.contains(&value)) {
            let (lo, hi) = (LEADING.start(), LEADING.end());
            return Err(out("leading", format!("`leading` must be from {lo} to {hi}, or null for Auto")));
        }
        let shift = MAX_BASELINE_SHIFT;
        for (key, value) in [("baseline_shift", self.baseline_shift), ("tcyUpDown", self.tcy_up_down), ("tcyLeftRight", self.tcy_left_right)] {
            if check(key) {
                within(key, value, -shift, shift)?;
            }
        }
        let spacing = [("tracking", Some(self.tracking)), ("kerning", self.kerning), ("kerningBefore", Some(self.kerning_before))];
        for (key, value) in spacing {
            if check(key)
                && let Some(value) = value
            {
                within(key, value, *CHAR_SPACING.start(), *CHAR_SPACING.end())?;
            }
        }
        if check("rotation") {
            within("rotation", self.rotation, *CHAR_ROTATION.start(), *CHAR_ROTATION.end())?;
        }
        for (key, value) in [("akiLeft", self.aki_left), ("akiRight", self.aki_right)] {
            if check(key) {
                within(key, value, 0.0, super::MAX_AKI)?;
            }
        }
        if check("tsume") {
            within("tsume", self.tsume, 0.0, super::MAX_TSUME)?;
        }
        let hi = super::MAX_DIACRITIC_POSITION;
        for (key, value) in [("diacriticHorizontal", self.diacritic_horizontal), ("diacriticVertical", self.diacritic_vertical)] {
            if check(key) {
                within(key, value, -hi, hi)?;
            }
        }
        let lines = super::WARICHU_LINES;
        if check("warichuLines") && !lines.contains(&self.warichu_lines) {
            return Err(out("warichuLines", format!("`warichuLines` must be a whole number from {} to {}", lines.start(), lines.end())));
        }
        let scale = super::WARICHU_SCALE;
        for (key, value, lo, hi) in
            [("warichuGap", self.warichu_gap, 0.0, super::MAX_WARICHU_GAP), ("warichuScale", self.warichu_scale, *scale.start(), *scale.end())]
        {
            if check(key) {
                within(key, value, lo, hi)?;
            }
        }
        if check("stroke_width") {
            within("stroke_width", self.stroke_width, 0.0, crate::MAX_COORD)?;
        }
        if check("stroke_miter_limit") {
            within("stroke_miter_limit", self.stroke_miter_limit, *CHAR_MITER_LIMIT.start(), *CHAR_MITER_LIMIT.end())?;
        }
        if check("stroke_dash")
            && let Some(dash) = &self.stroke_dash
        {
            for value in &dash.pattern {
                within("stroke_dash.pattern", *value, 0.0, crate::MAX_COORD)?;
            }
            within("stroke_dash.offset", dash.offset, -crate::MAX_COORD, crate::MAX_COORD)?;
        }
        if check("small_caps")
            && let Some(value) = self.small_caps
        {
            within("small_caps", value, 1.0, crate::setup::MAX_SMALL_CAPS)?;
        }
        if check("fontAxes")
            && let Some((tag, _)) = self.font_axes.iter().find(|(_, v)| !v.is_finite())
        {
            return Err(out("fontAxes", format!("`fontAxes`: the setting of axis `{tag}` must be a number")));
        }
        Ok(())
    }
}

impl ParaStyle {
    /// Checks each paragraph attribute `check` names, by its JSON key and for a nested option its
    /// field, against the range the paragraph style commands keep it to: the indents and the space
    /// before and after, each Hyphenation option, each Justification option in `justification`
    /// (the options as given, since reading a paragraph style brings them into range) with
    /// Minimum, Desired and Maximum in order, and the number of tab stops with the position and
    /// leader of each stop that is not in `kept_tabs`. The first attribute out of its range is the
    /// error.
    pub fn check_ranges(
        &self,
        justification: Option<&JustificationOptions>,
        kept_tabs: &[TabStop],
        check: impl Fn(&str, Option<&str>) -> bool,
    ) -> Result<(), AttrRangeError> {
        for (key, value) in [
            ("left_indent", self.left_indent),
            ("right_indent", self.right_indent),
            ("first_line_indent", self.first_line_indent),
            ("space_before", self.space_before),
            ("space_after", self.space_after),
        ] {
            if check(key, None) {
                within(key, value, -MAX_PARA_DISTANCE, MAX_PARA_DISTANCE)?;
            }
        }
        let h = &self.hyphenation;
        for (field, value, lo, hi) in [
            ("words_longer_than", f64::from(h.words_longer_than), 1.0, 25.0),
            ("after_first", f64::from(h.after_first), 1.0, 15.0),
            ("before_last", f64::from(h.before_last), 1.0, 15.0),
            ("hyphen_limit", f64::from(h.hyphen_limit), 0.0, 25.0),
            ("zone", h.zone, 0.0, MAX_PARA_DISTANCE),
        ] {
            if check("hyphenation", Some(field)) {
                within(&format!("hyphenation.{field}"), value, lo, hi)?;
            }
        }
        if let Some(j) = justification {
            for (field, values, (lo, hi)) in [
                ("word_spacing", j.word_spacing, JustificationOptions::WORD_SPACING),
                ("letter_spacing", j.letter_spacing, JustificationOptions::LETTER_SPACING),
                ("glyph_scaling", j.glyph_scaling, JustificationOptions::GLYPH_SCALING),
            ] {
                if !check("justification", Some(field)) {
                    continue;
                }
                let name = format!("justification.{field}");
                for value in values {
                    within(&name, value, lo, hi)?;
                }
                if !(values[0] <= values[1] && values[1] <= values[2]) {
                    return Err(out(&name, format!("`{name}`: Minimum must not exceed Desired, nor Desired Maximum")));
                }
            }
            if check("justification", Some("auto_leading")) {
                let (lo, hi) = JustificationOptions::AUTO_LEADING;
                within("justification.auto_leading", j.auto_leading, lo, hi)?;
            }
        }
        if check("tabs", None) {
            let max = TabStop::MAX_STOPS;
            if self.tabs.len() > max.max(kept_tabs.len()) {
                return Err(out("tabs", format!("at most {max} tab stops")));
            }
            for stop in self.tabs.iter().filter(|stop| !kept_tabs.contains(stop)) {
                within("tabs.position", stop.position, 0.0, TabStop::MAX_POSITION)?;
                let (label, max) = ("`tabs.leader`", TabStop::MAX_LEADER);
                if stop.leader.chars().count() > max {
                    return Err(out("tabs.leader", format!("{label} must be at most {max} characters")));
                }
            }
        }
        Ok(())
    }
}
