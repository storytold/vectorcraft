//! Live Paint groups' Gap Options (Object › Live Paint › Gap Options): whether paint stops at small
//! gaps between the group's paths, how wide a gap it stops at, and the color gaps preview in.
//! A group without options has Gap Detection off.

use serde::{Deserialize, Serialize};

use crate::node::{LAYER_COLORS, Node};

/// Paint Stops At: the widest gap Gap Detection closes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GapSize {
    /// Small Gaps: [`GapSize::SMALL`] points.
    #[default]
    Small,
    /// Medium Gaps: [`GapSize::MEDIUM`] points.
    Medium,
    /// Large Gaps: [`GapSize::LARGE`] points.
    Large,
    /// Custom: this many points, over 0 up to [`GapSize::MAX`].
    Custom(f64),
}

impl GapSize {
    pub const SMALL: f64 = 3.0;
    pub const MEDIUM: f64 = 6.0;
    pub const LARGE: f64 = 12.0;
    /// The widest custom gap.
    pub const MAX: f64 = 72.0;

    /// The preset sizes with their ids, smallest first.
    pub const PRESETS: [(Self, &'static str); 3] = [(Self::Small, "small"), (Self::Medium, "medium"), (Self::Large, "large")];

    /// The widest gap it closes, in points.
    pub fn points(self) -> f64 {
        match self {
            Self::Small => Self::SMALL,
            Self::Medium => Self::MEDIUM,
            Self::Large => Self::LARGE,
            Self::Custom(pt) => pt,
        }
    }

    /// The id of a preset size (`small`, `medium` or `large`); None for a custom size.
    pub fn id(self) -> Option<&'static str> {
        Self::PRESETS.iter().find(|(s, _)| *s == self).map(|(_, id)| *id)
    }

    /// A preset size by its id (any case), or a custom size from a number of points over 0 up to
    /// [`GapSize::MAX`].
    pub fn parse(v: &serde_json::Value) -> Option<Self> {
        match v {
            serde_json::Value::String(id) => Self::PRESETS.iter().find(|(_, i)| i.eq_ignore_ascii_case(id.trim())).map(|(s, _)| *s),
            v => v.as_f64().filter(|pt| *pt > 0.0 && *pt <= Self::MAX).map(Self::Custom),
        }
    }
}

/// Gap Preview Color for a group that has none set: the layer color Red.
pub const DEFAULT_GAP_COLOR: [u8; 3] = LAYER_COLORS[1].1;

/// A Live Paint group's Gap Options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GapOptions {
    /// Gap Detection: paint stops at gaps up to `size` wide.
    pub detect: bool,
    /// Paint Stops At.
    pub size: GapSize,
    /// Gap Preview Color: the color gaps show in.
    pub color: [u8; 3],
}

impl Default for GapOptions {
    fn default() -> Self {
        Self { detect: false, size: GapSize::Small, color: DEFAULT_GAP_COLOR }
    }
}

impl GapOptions {
    /// The widest gap paint stops at, in points; None with Gap Detection off.
    pub fn max_gap(&self) -> Option<f64> {
        self.detect.then(|| self.size.points())
    }
}

impl Node {
    /// The Gap Options of a Live Paint group (the defaults when it has none set).
    pub fn gap_options(&self) -> GapOptions {
        self.extras().live_paint_gaps.as_deref().copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sizes_parse_from_ids_and_points() {
        for (size, id) in GapSize::PRESETS {
            assert_eq!(GapSize::parse(&json!(id)), Some(size));
            assert_eq!(GapSize::parse(&json!(id.to_ascii_uppercase())), Some(size));
            assert_eq!(size.id(), Some(id));
        }
        assert_eq!(GapSize::parse(&json!("Medium")).map(GapSize::points), Some(6.0));
        assert_eq!(GapSize::parse(&json!("large")).map(GapSize::points), Some(12.0));
        assert_eq!(GapSize::parse(&json!(4.5)), Some(GapSize::Custom(4.5)));
        assert_eq!(GapSize::Custom(4.5).points(), 4.5);
        assert_eq!(GapSize::Custom(4.5).id(), None);
        for bad in [json!(0), json!(-1), json!(72.5), json!("tiny"), json!(null), json!(true)] {
            assert_eq!(GapSize::parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn options_round_trip_and_default_to_no_detection() {
        let mut g = Node::group(crate::NodeId(1), vec![]);
        assert_eq!(g.gap_options(), GapOptions::default());
        assert_eq!(g.gap_options().max_gap(), None);
        let set = GapOptions { detect: true, size: GapSize::Custom(5.0), color: [1, 2, 3] };
        g.extras_mut().live_paint_gaps = Some(Box::new(set));
        assert_eq!(g.gap_options().max_gap(), Some(5.0));
        let text = serde_json::to_string(&g).unwrap();
        let back: Node = serde_json::from_str(&text).unwrap();
        assert_eq!(back.gap_options(), set);
        let bare: GapOptions = serde_json::from_value(json!({"detect": true})).unwrap();
        assert_eq!(bare, GapOptions { detect: true, ..GapOptions::default() });
    }
}
