//! Accumulate disjoint opaque fill regions before compositing their shared coverage (#983).
//! Source-over of per-path coverage leaves backdrop at a perfectly shared edge. Keeping the
//! visible regions disjoint lets their analytic coverage add, preserving thin art and images.

use std::sync::Arc;

use vectorcraft_color::{BlendMode, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, FillLayer, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Shape};
use vectorcraft_pathops::{BoolOp, DEFAULT_PRECISION, try_boolean};
use vello_cpu::{RenderContext, peniko};

use crate::{AntiAlias, Frame, Renderer, fill_rule, fx, group::Composite, paint};

// Booleans can grow quadratically. This is an optional export optimization: unsupported or
// over-budget runs retain ordinary rendering, never drop art or force its alpha to opaque.
const MAX_NODES: usize = 64;
const MAX_SEGMENTS: usize = 1024;

struct Fill<'a> {
    path: &'a PathData,
    rule: FillRule,
    paint: &'a Paint,
}

fn compatible(n: &Node) -> Option<Fill<'_>> {
    if !n.visible || n.opacity != 1.0 || n.blend != BlendMode::Normal || n.isolate || n.mask.is_some() || fx::has_fx(n) {
        return None;
    }
    let NodeKind::Path { path, rule, guide: false, .. } = &n.kind else { return None };
    let mut items = n.appearance.items.iter().filter(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    });
    let Some(AppearanceItem::Fill(FillLayer { paint, opacity: 1.0, blend: BlendMode::Normal, overprint: false, .. })) = items.next() else {
        return None;
    };
    if items.next().is_some() {
        return None;
    }
    let opaque = match paint {
        Paint::Solid { .. } => true,
        Paint::Gradient(g) => {
            !g.gradient.stops.is_empty() && g.gradient.kind != GradientKind::Freeform && g.gradient.stops.iter().all(|s| s.opacity == 1.0)
        }
        _ => false,
    };
    opaque.then_some(Fill { path, rule: *rule, paint })
}

fn segments(p: &PathData) -> usize {
    p.subpaths.iter().fold(0usize, |n, s| n.saturating_add(s.anchors.len()))
}

/// Preserve painter order geometrically: each fill loses only the area of later opaque fills.
/// Identical solid paints need only their union, painted once with the first equivalent paint.
/// Finish every fallible operation before touching the context so fallback is all-or-nothing.
fn visible_regions(fills: &[Fill<'_>], px: f64) -> Result<Vec<PathData>, String> {
    if fills.len() > MAX_NODES || fills.iter().fold(0usize, |n, f| n.saturating_add(segments(f.path))) > MAX_SEGMENTS {
        return Err("opaque fill run exceeds the geometry budget".into());
    }
    let mut extent: f64 = 0.0;
    for fill in fills {
        for a in fill.path.subpaths.iter().flat_map(|s| &s.anchors) {
            for p in [a.p, a.h_in, a.h_out] {
                if !p.x.is_finite() || !p.y.is_finite() {
                    return Err("non-finite opaque fill geometry".into());
                }
                extent = extent.max(p.x.abs()).max(p.y.abs());
            }
        }
    }
    // Match pathops' sweep tolerance (boolean::eps_for). At extreme scale/origin it can
    // collapse visible hairlines even when the refit precision is small: keep analytic art.
    let tolerance = (extent * f64::EPSILON * 64.0).max(1e-6);
    if !px.is_finite() || tolerance > px * 0.001 {
        return Err("opaque fill sweep tolerance exceeds the pixel budget".into());
    }
    let precision = DEFAULT_PRECISION.min(px * 0.001);
    let uniform = fills.first().is_some_and(|first| matches!(first.paint, Paint::Solid { .. }) && fills.iter().all(|f| f.paint == first.paint));
    let mut covered = PathData::default();
    let mut regions = Vec::with_capacity(fills.len());
    let mut total = 0usize;
    for fill in fills.iter().rev() {
        if !uniform {
            let region = try_boolean(fill.path, fill.rule, &covered, FillRule::NonZero, BoolOp::Difference, precision).map_err(|e| e.to_string())?;
            total = total.saturating_add(segments(&region));
            regions.push(region);
        }
        covered = try_boolean(fill.path, fill.rule, &covered, FillRule::NonZero, BoolOp::Union, precision).map_err(|e| e.to_string())?;
        if total > MAX_SEGMENTS || segments(&covered) > MAX_SEGMENTS {
            return Err("opaque fill regions exceed the geometry budget".into());
        }
    }
    if uniform {
        // One fill avoids per-region coverage rounding changing a uniform ink colour by 1 LSB.
        regions.push(covered);
    } else {
        regions.reverse();
    }
    Ok(regions)
}

impl Renderer {
    pub(super) fn draw_seamless_fills(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>]) -> bool {
        if !f.opts.precise || f.opts.anti_alias != AntiAlias::Art || f.opts.outline || self.shape_of != 0 {
            return false;
        }
        let mut rest = children;
        while let Some((first, after)) = rest.split_first() {
            if compatible(first).is_none() || f.opts.hidden.contains(&first.id) {
                self.draw_arc(ctx, f, first);
                rest = after;
                continue;
            }
            let n = rest.iter().take_while(|n| compatible(n).is_some() && !f.opts.hidden.contains(&n.id)).count();
            let (run, after) = rest.split_at(n);
            let fills: Vec<_> = run.iter().take(MAX_NODES + 1).filter_map(|n| compatible(n)).collect();
            let regions = (run.len() > 1 && run.len() <= MAX_NODES).then(|| visible_regions(&fills, f.px));
            if let Some(Ok(regions)) = regions {
                self.group(ctx, f, Composite::default(), &mut |_, c, fr| {
                    c.set_transform(fr.view);
                    c.set_fill_rule(fill_rule(FillRule::NonZero));
                    for (fill, region) in fills.iter().zip(&regions) {
                        if paint::set_paint(c, fill.paint, fill.path.to_bezpath().bounding_box(), fr) {
                            // Plus must apply to the already-covered region, as a layer. Per-path
                            // Plus clamps the unmasked source before coverage and still leaves seams.
                            c.push_blend_layer(peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Plus));
                            c.fill_path(&region.to_bezpath());
                            c.pop_layer();
                        }
                    }
                    c.reset_paint_transform();
                });
                self.stats.drawn += run.len();
            } else {
                // An optimization failure must keep the original art and rendering semantics.
                for n in run {
                    self.draw_arc(ctx, f, n);
                }
            }
            rest = after;
        }
        true
    }
}
