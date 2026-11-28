//! Image Trace objects' View: how a live trace draws on screen.
//!
//! An Image Trace object is a group whose `trace` holds `{preset, params, view}` (see
//! `imageTrace.make`). The view is a viewing aid only: exports, printing and Expand always use the
//! tracing result, whatever it is set to.

use crate::node::Node;

/// Image Trace › View.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceView {
    /// The traced shapes as they are filled.
    #[default]
    Result,
    /// The traced shapes, with their outlines over them.
    ResultWithOutlines,
    /// Only the traced shapes' outlines.
    Outlines,
    /// The source image, with the traced shapes' outlines over it.
    OutlinesWithSource,
    /// Only the source image.
    Source,
    /// The source image on the part of the image left of a divider across it, and the traced
    /// shapes everywhere else ([`TraceView::split_of`]).
    Split,
}

/// Where the Split view's divider is when the `trace` settings name no valid position: across the
/// middle of the source image.
pub const DEFAULT_TRACE_SPLIT: f64 = 0.5;

impl TraceView {
    /// Panel order.
    pub const ALL: [Self; 6] = [Self::Result, Self::ResultWithOutlines, Self::Outlines, Self::OutlinesWithSource, Self::Source, Self::Split];

    /// The id used in command params and in the saved `trace` object.
    pub fn id(self) -> &'static str {
        match self {
            Self::Result => "tracingResult",
            Self::ResultWithOutlines => "tracingResultWithOutlines",
            Self::Outlines => "outlines",
            Self::OutlinesWithSource => "outlinesWithSourceImage",
            Self::Source => "sourceImage",
            Self::Split => "split",
        }
    }

    /// The view an id names (any case).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id().eq_ignore_ascii_case(id))
    }

    /// The view an Image Trace object's `trace` settings ask for (the tracing result when they
    /// name none, or an unknown one).
    pub fn of(trace: &serde_json::Value) -> Self {
        trace.get("view").and_then(serde_json::Value::as_str).and_then(Self::from_id).unwrap_or_default()
    }

    /// The Split view's divider in `trace` settings: the fraction of the source image's width, from
    /// its left edge in its own pixels, that shows the source image ([`DEFAULT_TRACE_SPLIT`] when
    /// `split` is missing, not a number or outside 0 to 1).
    pub fn split_of(trace: &serde_json::Value) -> f64 {
        trace.get("split").and_then(serde_json::Value::as_f64).filter(|f| (0.0..=1.0).contains(f)).unwrap_or(DEFAULT_TRACE_SPLIT)
    }

    /// Whether the source image shows (in the Split view, on one side of the divider).
    pub fn shows_source(self) -> bool {
        matches!(self, Self::OutlinesWithSource | Self::Source | Self::Split)
    }

    /// Whether the traced shapes show filled (in the Split view, on one side of the divider).
    pub fn shows_result(self) -> bool {
        matches!(self, Self::Result | Self::ResultWithOutlines | Self::Split)
    }

    /// Whether the traced shapes' outlines show.
    pub fn shows_outlines(self) -> bool {
        matches!(self, Self::ResultWithOutlines | Self::Outlines | Self::OutlinesWithSource)
    }
}

impl Node {
    /// The View of an Image Trace object (the tracing result for anything else, or an unknown id).
    pub fn trace_view(&self) -> TraceView {
        self.extras().trace.as_deref().map(TraceView::of).unwrap_or_default()
    }

    /// The Split view's divider of an Image Trace object ([`TraceView::split_of`];
    /// [`DEFAULT_TRACE_SPLIT`] for anything else).
    pub fn trace_split(&self) -> f64 {
        self.extras().trace.as_deref().map_or(DEFAULT_TRACE_SPLIT, TraceView::split_of)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_views_show_the_result() {
        for v in TraceView::ALL {
            assert_eq!(TraceView::from_id(v.id()), Some(v));
            assert_eq!(TraceView::from_id(&v.id().to_ascii_uppercase()), Some(v));
            assert!(v.shows_result() || v.shows_outlines() || v.shows_source(), "{v:?} shows something");
        }
        let mut g = Node::group(crate::NodeId(1), vec![]);
        assert_eq!(g.trace_view(), TraceView::Result);
        g.extras_mut().trace = Some(Box::new(serde_json::json!({"view": "outlines"})));
        assert_eq!(g.trace_view(), TraceView::Outlines);
        g.extras_mut().trace = Some(Box::new(serde_json::json!({"view": "sepia"})));
        assert_eq!(g.trace_view(), TraceView::Result);
    }

    #[test]
    fn the_split_divider_reads_a_fraction_and_falls_back_to_the_middle() {
        let mut g = Node::group(crate::NodeId(1), vec![]);
        assert_eq!(g.trace_split(), DEFAULT_TRACE_SPLIT);
        assert_eq!(TraceView::from_id("Split"), Some(TraceView::Split));
        assert!(TraceView::Split.shows_source() && TraceView::Split.shows_result() && !TraceView::Split.shows_outlines());
        for (split, want) in [
            (serde_json::json!(0.25), 0.25),
            (serde_json::json!(0), 0.0),
            (serde_json::json!(1), 1.0),
            (serde_json::json!(-0.1), DEFAULT_TRACE_SPLIT),
            (serde_json::json!(1.5), DEFAULT_TRACE_SPLIT),
            (serde_json::json!("0.3"), DEFAULT_TRACE_SPLIT),
            (serde_json::Value::Null, DEFAULT_TRACE_SPLIT),
        ] {
            g.extras_mut().trace = Some(Box::new(serde_json::json!({"view": "split", "split": split})));
            assert_eq!(g.trace_split(), want, "{split}");
        }
    }
    #[test]
    fn live_trace_result_bounds_exclude_the_retained_bitmap_at_any_angle() {
        use crate::{Appearance, Document, ImageObject, NodeId, NodeKind};
        use std::sync::Arc;
        use vectorcraft_geom::{Affine, PathData, Point, Rect, SubPath};
        let mut source = Node::new(
            NodeId(2),
            NodeKind::Image(Box::new(ImageObject {
                key: "synthetic".into(),
                width: 200,
                height: 200,
                xf: Affine::IDENTITY,
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        );
        source.visible = false;
        let art = Node::path(
            NodeId(3),
            PathData::single(SubPath::polyline(
                &[Point::new(20.0, 30.0), Point::new(80.0, 30.0), Point::new(80.0, 90.0), Point::new(20.0, 90.0)],
                true,
            )),
            Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0),
        );
        let mut group = Node::group(NodeId(1), vec![Arc::new(source), Arc::new(art.clone())]);
        group.extras_mut().trace = Some(Box::new(serde_json::json!({})));
        let doc = Document::new(300.0, 300.0);
        for view in TraceView::ALL {
            group.extras_mut().trace = Some(Box::new(serde_json::json!({"view":view.id()})));
            let want = if view.shows_source() { Rect::new(0.0, 0.0, 200.0, 200.0) } else { Rect::new(20.0, 30.0, 80.0, 90.0) };
            assert_eq!(group.geometric_bounds(), Some(want));
            assert_eq!(group.visual_bounds(), Some(want));
            if !view.shows_source() {
                for angle in [0.0_f64, 30.0, 90.0] {
                    let xf = Affine::rotate(angle.to_radians());
                    for visual in [false, true] {
                        assert_eq!(group.bounds_in(&doc.symbol_boxes(), xf, visual), art.bounds_in(&doc.symbol_boxes(), xf, visual));
                    }
                }
            }
        }
        // The Split view measures the whole image wherever the divider is, so the selection box
        // stays put while the divider moves.
        for split in [0.0, 0.01, 1.0] {
            let want = Rect::new(0.0, 0.0, 200.0, 200.0);
            group.extras_mut().trace = Some(Box::new(serde_json::json!({"view": "split", "split": split})));
            assert_eq!((group.geometric_bounds(), group.visual_bounds()), (Some(want), Some(want)), "split {split}");
        }
        group.extras_mut().trace = None;
        assert_eq!(
            group.geometric_bounds(),
            Some(Rect::new(0.0, 0.0, 200.0, 200.0)),
            "ordinary hidden group members retain their existing bounds policy"
        );
    }
}
