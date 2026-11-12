//! Stroke boundaries shared by type drawing, outline conversion and writers.

use std::mem::size_of;
use std::ops::Range;

use vectorcraft_doc::TextObject;
use vectorcraft_geom::{BezPath, FillRule, PathData};
use vectorcraft_pathops::PathOpsError;
use vectorcraft_text::TextLayout;

/// One consecutive character-stroke paint group. Its original fills precede its one stroke.
#[derive(Debug)]
pub struct Group {
    pub runs: Range<usize>,
    /// Normalized filled boundary; refusal leaves `None` to preserve the singleton's raw stroke.
    pub boundary: Option<BezPath>,
}

/// Immutable stroke geometry in text space, prepared before a renderer caches its text geometry.
#[derive(Debug, Default)]
pub struct Plan {
    pub groups: Vec<Group>,
    /// Whole-text boundary, or `None` when not requested or when normalization refused it.
    pub all: Option<BezPath>,
    pub object_requested: bool,
    // Only allocated for character strokes. usize::MAX marks a run that ends no group.
    ending: Vec<usize>,
    // BezPath exposes its elements but not their capacity. Record capacity while owning the Vec.
    path_bytes: usize,
}

impl Plan {
    pub fn group_ending_at(&self, run: usize) -> Option<&Group> {
        self.ending.get(run).and_then(|index| self.groups.get(*index))
    }

    /// Retained heap allocations, excluding this Plan's fixed header and temporary preparation.
    pub fn heap_bytes(&self) -> usize {
        self.groups
            .capacity()
            .saturating_mul(size_of::<Group>())
            .saturating_add(self.ending.capacity().saturating_mul(size_of::<usize>()))
            .saturating_add(self.path_bytes)
    }
}

/// Unite complete glyph filled regions under NonZero, retaining counters and detached contours.
/// Even one operand is normalized: raw overlapping contours can retain internal stroke borders.
/// Empty input succeeds with an empty path; a refused input returns its error without losing art.
pub fn boundary(glyphs: &[BezPath]) -> Result<BezPath, PathOpsError> {
    boundary_refs(glyphs.iter())
}

fn boundary_refs<'a>(glyphs: impl IntoIterator<Item = &'a BezPath>) -> Result<BezPath, PathOpsError> {
    let paths: Vec<PathData> = glyphs.into_iter().map(PathData::from_bezpath).collect();
    let operands: Vec<(&PathData, FillRule)> = paths.iter().map(|path| (path, FillRule::NonZero)).collect();
    vectorcraft_pathops::try_unite_all(&operands).map(|path| path.to_bezpath())
}

fn retained(path: BezPath, bytes: &mut usize) -> BezPath {
    let elements = path.into_elements();
    *bytes = bytes.saturating_add(elements.capacity().saturating_mul(size_of::<vectorcraft_geom::PathEl>()));
    BezPath::from_vec(elements)
}

fn scopes(t: &TextObject, layout: &TextLayout) -> Vec<(Range<usize>, bool)> {
    let mut blocked: Vec<bool> = t.runs.iter().map(|run| run.inline.is_some()).collect();
    for glyph in layout.glyphs.iter().chain(&layout.ruby) {
        if (glyph.image.is_some() || glyph.colour.is_some())
            && let Some(blocked) = blocked.get_mut(glyph.run)
        {
            *blocked = true;
        }
    }
    for inline in &layout.inlines {
        if let Some(blocked) = blocked.get_mut(inline.run) {
            *blocked = true;
        }
    }
    compatible_scopes(t, &blocked)
}

fn compatible_scopes(t: &TextObject, blocked: &[bool]) -> Vec<(Range<usize>, bool)> {
    let mut scopes = Vec::new();
    let mut at = 0;
    while let Some(run) = t.runs.get(at) {
        if !run.style.has_stroke() {
            at += 1;
            continue;
        }
        let start = at;
        let barrier = blocked.get(at).copied().unwrap_or(false);
        at += 1;
        if !barrier {
            let stroke = run.style.stroke_layer();
            while let Some(next) = t.runs.get(at) {
                if blocked.get(at).copied().unwrap_or(false) || !next.style.has_stroke() || next.style.stroke_layer() != stroke {
                    break;
                }
                at += 1;
            }
        }
        scopes.push((start..at, barrier));
    }
    scopes
}

fn push_group(plan: &mut Plan, runs: Range<usize>, normalized: Option<BezPath>) {
    match normalized {
        Some(path) => plan.groups.push(Group { runs, boundary: Some(retained(path, &mut plan.path_bytes)) }),
        None => {
            // A refused union must not defer early strokes past later fills.
            for run in runs {
                plan.groups.push(Group { runs: run..run + 1, boundary: None });
            }
        }
    }
}

/// Prepare only requested strokes. Equal complete character StrokeLayers share a boundary even
/// across different fills. Inline, bitmap and color-glyph runs prevent grouping across their run;
/// their ordinary outline primitives are still normalized within that singleton.
/// Normalization failure restores singleton raw-run strokes; whole-text failure keeps `all` None
/// with object_requested true, so callers can retain their raw whole-text stroke.
pub fn prepare(t: &TextObject, layout: &TextLayout, object_stroke: bool) -> Plan {
    let character_stroke = t.runs.iter().any(|run| run.style.has_stroke());
    if !character_stroke && !object_stroke {
        return Plan::default();
    }
    let candidates = if character_stroke { scopes(t, layout) } else { Vec::new() };
    let mut run_group = if character_stroke { vec![usize::MAX; t.runs.len()] } else { Vec::new() };
    for (index, (runs, _)) in candidates.iter().enumerate() {
        for member in run_group.get_mut(runs.clone()).into_iter().flatten() {
            *member = index;
        }
    }
    let mut inputs: Vec<Vec<&BezPath>> = candidates.iter().map(|_| Vec::new()).collect();
    let mut all = Vec::new();
    let bars = if t.runs.iter().any(|run| run.style.underline || run.style.strikethrough) {
        vectorcraft_text::decorations(layout, vectorcraft_text::FontDb::global(), t)
    } else {
        Vec::new()
    };
    let glyphs = layout.glyphs.iter().chain(&layout.ruby).map(|glyph| (glyph.run, &glyph.outline));
    for (run, path) in glyphs.chain(bars.iter().map(|(run, bar)| (*run, bar))) {
        if path.elements().is_empty() {
            continue;
        }
        if object_stroke {
            all.push(path);
        }
        if let Some(&group) = run_group.get(run)
            && let Some(inputs) = inputs.get_mut(group)
        {
            inputs.push(path);
        }
    }
    let mut plan = Plan { object_requested: object_stroke, ..Plan::default() };
    for ((runs, _), glyphs) in candidates.into_iter().zip(inputs) {
        let normalized = boundary_refs(glyphs).ok();
        push_group(&mut plan, runs, normalized);
    }
    if character_stroke {
        plan.ending = vec![usize::MAX; t.runs.len()];
        for (index, group) in plan.groups.iter().enumerate() {
            if let Some(end) = group.runs.end.checked_sub(1).and_then(|end| plan.ending.get_mut(end)) {
                *end = index;
            }
        }
    }
    if object_stroke {
        plan.all = boundary_refs(all).ok().map(|path| retained(path, &mut plan.path_bytes));
    }
    plan
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::StrokeLayer;
    use vectorcraft_geom::Point;
    use vectorcraft_geom::hit::fill_contains;

    use super::*;

    // Original contour fixtures, not font outlines or a substitute for Arabic shaping coverage.
    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64, reverse: bool) -> BezPath {
        let points = if reverse { [(x0, y0), (x0, y1), (x1, y1), (x1, y0)] } else { [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] };
        let mut path = BezPath::new();
        path.move_to(points[0]);
        for point in &points[1..] {
            path.line_to(*point);
        }
        path.close_path();
        path
    }

    fn covers(path: &BezPath, x: f64, y: f64) -> bool {
        fill_contains(path, FillRule::NonZero, Point::new(x, y))
    }

    fn stroked(path: &BezPath) -> BezPath {
        let stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
        crate::stroke::outline_region(&PathData::from_bezpath(path), FillRule::NonZero, &stroke).to_bezpath()
    }

    #[test]
    fn joined_glyph_boundary_removes_internal_stroke_but_keeps_counter_and_dot() {
        let mut first = rectangle(10.0, 10.0, 50.0, 50.0, false);
        first.extend(rectangle(20.0, 20.0, 30.0, 40.0, true).iter());
        first.extend(rectangle(12.0, 60.0, 20.0, 68.0, false).iter());
        // Opposite outer winding is legal for another complete glyph. Its filled overlap must
        // be united, while the counter in the first glyph must remain a hole.
        let second = rectangle(40.0, 10.0, 80.0, 50.0, true);
        let mut raw = first.clone();
        raw.extend(second.iter());
        let raw_stroke = stroked(&raw);
        assert!(covers(&raw_stroke, 40.0, 30.0), "raw glyph contours reproduce the internal border");
        assert!(covers(&raw_stroke, 50.0, 30.0));

        let united = boundary(&[first, second]).unwrap();
        let border = stroked(&united);
        assert!(covers(&united, 45.0, 30.0), "opposite glyph windings do not cancel their overlap");
        for x in [40.0, 50.0] {
            assert!(!covers(&border, x, 30.0), "no stroke across the joined interior at x={x}");
        }
        assert!(covers(&border, 10.0, 30.0), "the exterior edge still has a stroke");
        assert!(!covers(&united, 25.0, 30.0), "the counter stays unfilled");
        assert!(covers(&border, 20.0, 30.0), "the counter edge is stroked");
        assert!(!covers(&border, 25.0, 30.0), "the middle of the counter stays empty");
        assert!(covers(&united, 16.0, 64.0), "the detached dot stays filled");
        assert!(covers(&border, 12.0, 64.0), "the detached dot keeps its border");
        assert!(!covers(&border, 16.0, 64.0), "the dot's interior is not turned into border paint");
        assert!(!covers(&border, 5.0, 30.0), "no paint outside the stroke's reach");
    }

    #[test]
    fn one_glyph_operand_is_normalized_before_its_boundary_is_stroked() {
        let mut glyph = rectangle(10.0, 10.0, 50.0, 50.0, false);
        glyph.extend(rectangle(40.0, 10.0, 80.0, 50.0, false).iter());
        assert!(covers(&stroked(&glyph), 40.0, 30.0), "the raw one-operand path has an internal border");

        let united = boundary(&[glyph]).unwrap();
        let border = stroked(&united);
        assert!(covers(&united, 45.0, 30.0));
        assert!(!covers(&border, 40.0, 30.0));
        assert!(!covers(&border, 50.0, 30.0));
        assert!(covers(&border, 10.0, 30.0));
    }

    #[test]
    fn empty_and_separated_glyphs_keep_their_expected_stroke_regions() {
        assert!(boundary(&[]).unwrap().elements().is_empty());
        assert!(boundary(&[BezPath::new()]).unwrap().elements().is_empty());
        let united = boundary(&[rectangle(10.0, 10.0, 30.0, 50.0, false), rectangle(50.0, 10.0, 70.0, 50.0, true)]).unwrap();
        let border = stroked(&united);
        assert!(covers(&united, 20.0, 30.0) && covers(&united, 60.0, 30.0));
        assert!(!covers(&united, 40.0, 30.0), "separated glyphs do not acquire a bridge");
        assert!(covers(&border, 30.0, 30.0) && covers(&border, 50.0, 30.0), "facing edges remain legitimate borders");
        assert!(!covers(&border, 40.0, 30.0));
        assert!(!covers(&border, 20.0, 30.0) && !covers(&border, 60.0, 30.0));
    }

    #[test]
    fn refused_glyph_returns_the_path_operation_error_instead_of_partial_art() {
        // Reuse the pathops regression's bounded refusal: four finite points near the float
        // limit are rejected before the sweep, without constructing a complex input.
        let huge = rectangle(0.0, 0.0, 1e308, 1e308, false);
        let valid = rectangle(10.0, 10.0, 30.0, 50.0, false);
        assert_eq!(boundary(&[huge, valid]), Err(PathOpsError::Degenerate));
    }

    fn glyph(outline: BezPath, run: usize) -> vectorcraft_text::PositionedGlyph {
        vectorcraft_text::PositionedGlyph {
            outline,
            run,
            byte: run,
            origin: Point::ZERO,
            advance: 20.0,
            len: 1,
            angle: 0.0,
            line: 0,
            font_id: 0,
            gid: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            rtl: false,
            scale: 1.0,
            image: None,
            colour: None,
            bend: None,
            unturned: vectorcraft_geom::Affine::IDENTITY,
            pivot: Point::ZERO,
            carets: Vec::new(),
            width: 20.0,
            width_start: 0.0,
        }
    }

    fn stroked_style() -> vectorcraft_doc::CharStyle {
        vectorcraft_doc::CharStyle { fill: Paint::solid(Color::BLACK), stroke: Paint::solid(Color::BLACK), stroke_width: 4.0, ..Default::default() }
    }

    fn text(styles: &[vectorcraft_doc::CharStyle]) -> TextObject {
        let mut t = TextObject::point(Point::ZERO, "", Default::default());
        t.runs = styles.iter().map(|style| vectorcraft_doc::TextRun::new("x", style.clone())).collect();
        t
    }

    #[test]
    fn equal_strokes_across_different_fills_share_a_boundary_but_changed_strokes_do_not() {
        let style = stroked_style();
        let different_fill = vectorcraft_doc::CharStyle { fill: Paint::solid(Color::WHITE), ..style.clone() };
        let different_stroke = vectorcraft_doc::CharStyle { stroke_width: 8.0, ..style.clone() };
        let t = text(&[style, different_fill, different_stroke]);
        let layout = TextLayout {
            glyphs: vec![
                glyph(rectangle(10.0, 10.0, 50.0, 50.0, false), 0),
                glyph(rectangle(40.0, 10.0, 80.0, 50.0, false), 1),
                glyph(rectangle(70.0, 10.0, 110.0, 50.0, false), 2),
            ],
            ..Default::default()
        };
        let plan = prepare(&t, &layout, false);
        assert!(plan.group_ending_at(0).is_none(), "the first stroke waits for its compatible group to end");
        let joined = plan.group_ending_at(1).unwrap();
        assert_eq!(joined.runs, 0..2);
        let border = stroked(joined.boundary.as_ref().unwrap());
        assert!(!covers(&border, 40.0, 30.0) && !covers(&border, 50.0, 30.0), "mixed fills share no internal border");
        let changed = plan.group_ending_at(2).unwrap();
        assert_eq!(changed.runs, 2..3);
        assert!(covers(&stroked(changed.boundary.as_ref().unwrap()), 70.0, 30.0), "a changed stroke keeps its own border");
        assert!(!plan.object_requested && plan.all.is_none());
        assert!(plan.heap_bytes() > 0);
    }

    #[test]
    fn inline_and_color_runs_separate_but_normalize_their_outline_subsets() {
        for color in [false, true] {
            let style = stroked_style();
            let mut t = text(&[style.clone(), style.clone(), style]);
            let mut layout = TextLayout {
                glyphs: vec![
                    glyph(rectangle(10.0, 10.0, 30.0, 50.0, false), 0),
                    glyph(rectangle(100.0, 10.0, 140.0, 50.0, false), 1),
                    glyph(rectangle(130.0, 10.0, 170.0, 50.0, false), 1),
                    glyph(BezPath::new(), 1),
                    glyph(rectangle(200.0, 10.0, 220.0, 50.0, false), 2),
                ],
                ..Default::default()
            };
            if color {
                layout.glyphs[3].colour = Some(std::sync::Arc::new(vectorcraft_doc::Node::group(vectorcraft_doc::NodeId(1), Vec::new())));
            } else {
                t.runs[1].inline =
                    Some(vectorcraft_doc::InlineArt { symbol: "original test symbol".into(), scale: 1.0, baseline_shift: 0.0, bounds: None });
            }
            let plan = prepare(&t, &layout, false);
            assert_eq!(plan.groups.len(), 3, "color={color}");
            for run in 0..3 {
                assert_eq!(plan.group_ending_at(run).unwrap().runs, run..run + 1, "color={color}");
            }
            // A run holding color art can also hold joined ordinary glyph outlines: only
            // grouping across its paint order is barred, not normalizing that outlined subset.
            let border = stroked(plan.group_ending_at(1).unwrap().boundary.as_ref().unwrap());
            assert!(!covers(&border, 130.0, 30.0) && !covers(&border, 140.0, 30.0), "color={color}: no internal border");
            assert!(covers(&border, 100.0, 30.0), "color={color}: the exterior border remains");

            layout.glyphs.retain(|glyph| glyph.run != 1 || glyph.colour.is_some());
            let empty = prepare(&t, &layout, false);
            assert!(
                empty.group_ending_at(1).unwrap().boundary.as_ref().is_some_and(|path| path.elements().is_empty()),
                "an empty outlined subset succeeds rather than falling back to raw art"
            );
        }
    }

    #[test]
    fn absent_demand_keeps_no_plan_and_refused_groups_restore_each_raw_stroke() {
        let unpainted = text(&[Default::default()]);
        let huge = rectangle(0.0, 0.0, 1e308, 1e308, false);
        let layout = TextLayout { glyphs: vec![glyph(huge.clone(), 0)], ..Default::default() };
        let idle = prepare(&unpainted, &layout, false);
        assert!(idle.groups.is_empty() && idle.all.is_none() && !idle.object_requested);
        assert_eq!(idle.heap_bytes(), 0, "fill-only type retains no stroke allocations");
        let empty = prepare(&unpainted, &TextLayout::default(), true);
        assert!(empty.object_requested && empty.all.as_ref().is_some_and(|path| path.elements().is_empty()));

        let style = stroked_style();
        let t = text(&[style.clone(), style]);
        let layout = TextLayout { glyphs: vec![glyph(rectangle(10.0, 10.0, 30.0, 50.0, false), 0), glyph(huge, 1)], ..Default::default() };
        let refused = prepare(&t, &layout, true);
        assert!(refused.object_requested && refused.all.is_none(), "the object caller must fall back to its raw path");
        assert_eq!(refused.groups.len(), 2);
        for run in 0..2 {
            let group = refused.group_ending_at(run).unwrap();
            assert_eq!(group.runs, run..run + 1, "refusal restores original fill/stroke order");
            assert!(group.boundary.is_none(), "refusal must not silently omit a glyph");
        }
    }
}
