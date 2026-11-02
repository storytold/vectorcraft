use super::{Affine, AppearanceItem, Arc, BezPath, Node, NodeKind, StrokeLayer, brush_fx, effects};

/// Most pixels from the origin that a shape handed to the rasterizer may reach, or a stroke be
/// wide. A document's art stays within 4 million points, 2.6 billion pixels at the closest zoom.
/// Geometry far beyond that (from a malformed file) makes vello_cpu panic on a worker thread or ask
/// for more memory than there is, either of which ends the app: it is left undrawn.
const MAX_DEVICE: f64 = 1e10;

/// Can shape `bp` of `n` be drawn through `view`: its control points (finite) within
/// [`MAX_DEVICE`] pixels, and its visible strokes no wider?
pub(crate) fn drawable(bp: &BezPath, view: Affine, n: &Node) -> bool {
    let b = view.transform_rect_bbox(bp.control_box());
    let scale = view.determinant().abs().sqrt();
    let widest = n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(st) = i { Some(st.width) } else { None }).fold(0.0, f64::max);
    [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_DEVICE) && (widest * scale).is_finite() && widest * scale <= MAX_DEVICE
}

/// Exactly one visible painted item, a fill with its own Normal blend and full opacity (one
/// paint operation, so an object blend mode can be applied per draw).
pub(crate) fn single_plain_fill(n: &Node) -> bool {
    let mut painted = n.appearance.items.iter().filter(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    });
    matches!((painted.next(), painted.next()), (Some(AppearanceItem::Fill(f)), None) if f.blend == vectorcraft_color::BlendMode::Normal && f.opacity >= 1.0 && !matches!(f.paint, vectorcraft_color::Paint::Pattern { .. }))
}

/// A knockout element whose opaque paint covers exactly its knockout shape. The caller limits
/// source-over substitution to binary coverage, retaining analytical AA's intermediate rounding.
pub(crate) fn opaque_knockout_fill(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })
        && n.opacity >= 1.0
        && n.blend == vectorcraft_color::BlendMode::Normal
        && !n.isolate
        && n.extras().mask.is_none()
        && !crate::fx::has_fx(n)
        && !crate::fx::has_object_fx(n)
        && single_plain_fill(n)
        && n.appearance
            .items
            .iter()
            .any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && matches!(f.paint, vectorcraft_color::Paint::Solid { .. })))
}

/// Number of visible painted fill/stroke items (opacity folding is exact only for one). A dashed
/// stroke is one paint: its dashes are outlined into one path and filled once, so overlapping
/// dashes don't darken where they overlap. A dashed brush stroke draws its pieces one by one, so
/// it takes the layered path, with or without other paints.
pub(crate) fn painted_items(n: &Node) -> usize {
    n.appearance
        .items
        .iter()
        .filter(|i| match i {
            AppearanceItem::Fill(f) => f.visible && !f.paint.is_none() && matches!(f.paint, vectorcraft_color::Paint::Solid { .. }),
            AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && matches!(s.paint, vectorcraft_color::Paint::Solid { .. }),
        })
        .count()
        .max(if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !matches!(f.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None)) || matches!(i, AppearanceItem::Stroke(s) if s.visible && (!matches!(s.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None) || (s.dash.is_some() && s.brush.is_some() && !s.paint.is_none())))) { 2 } else { 0 })
}

/// Whether `st` is one of `n`'s own strokes (the strokes of the brush art drawn for `n` are not).
pub(crate) fn owns_stroke(n: &Node, st: &StrokeLayer) -> bool {
    n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st)))
}

/// Symbol art `art` stained by instance `inst` ([`instance_art`]) and placed by `xf`, with
/// strokes scaled by `xf` when `scale_strokes`.
pub(crate) fn place(art: &Node, xf: Affine, scale_strokes: bool, inst: Option<&Node>) -> Arc<Node> {
    let mut placed = match inst {
        Some(i) => brush_fx::instance_art(art, i),
        None => art.clone(),
    };
    placed.transform(xf, scale_strokes);
    Arc::new(placed)
}

/// Does `n` hold, in it or in the opacity masks of the objects in it, a symbol instance, type
/// with an inline graphic, an image, a placed document, or an object or a fill or stroke with a
/// visible raster effect?
pub(crate) fn depends_on_document(n: &Node) -> bool {
    let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && effects::is_raster(&e.id));
    let mut depends = false;
    n.walk(&mut |c| {
        depends |= match &c.kind {
            NodeKind::SymbolInstance { .. } | NodeKind::Image(_) | NodeKind::PlacedDocument(_) => true,
            NodeKind::Text(t) => t.runs.iter().any(|r| r.inline.is_some()),
            _ => false,
        } || raster(&c.appearance.effects)
            || c.appearance.items.iter().any(|i| raster(i.effects()))
            || c.extras().mask.as_ref().is_some_and(|m| depends_on_document(&m.art));
    });
    depends
}

/// Does `n` hold type, in it or in the opacity masks of the objects in it?
pub(crate) fn holds_type(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= matches!(c.kind, NodeKind::Text(_)) || c.extras().mask.as_ref().is_some_and(|m| holds_type(&m.art)));
    holds
}

/// Is `n` a symbol instance, or does it hold one? Its cull bounds measure the instance by its
/// symbol's art.
pub(crate) fn holds_instance(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
    holds
}

/// Does `n` hold an object with a visible plug-in effect ([`effects::has_plugin_effect`]), in it or
/// in the opacity masks of the objects in it?
pub(crate) fn holds_plugin_effect(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= effects::has_plugin_effect(c) || c.extras().mask.as_ref().is_some_and(|m| holds_plugin_effect(&m.art)));
    holds
}

/// A symbol instance's stain ([`vectorcraft_brush::stain`]) as bits for a cache key: its color
/// model, the color's components and the amount (all 0 without a stain).
pub(crate) fn stain_bits(stain: Option<(vectorcraft_color::Color, f32)>) -> [u32; 6] {
    use vectorcraft_color::Color;
    let Some((color, amount)) = stain else { return [0; 6] };
    let (model, [p, q, r, s]) = match color {
        Color::Rgb { r, g, b } => (1, [r, g, b, 0.0]),
        Color::Cmyk { c, m, y, k } => (2, [c, m, y, k]),
        Color::Gray { k } => (3, [k, 0.0, 0.0, 0.0]),
        Color::Lab { l, a, b } => (4, [l, a, b, 0.0]),
    };
    [model, p.to_bits(), q.to_bits(), r.to_bits(), s.to_bits(), amount.to_bits()]
}
