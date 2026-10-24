//! Linked dimensions: generated groups share the existing text/path render and export pipeline.
use super::*;
use serde_json::{Value, json};
use std::sync::Arc;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::dimension::{ArrowStyle, Dimension, DimensionMode, DimensionOptions, DimensionSource, LineType, TextAlignment, TextPosition};
use vectorcraft_doc::{Appearance, Arrowhead, CharStyle, ColorMode, Dash, Document, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Vec2};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "dimension.create",
            "Create Dimension",
            [],
            None,
            "{mode?: linear|angular|radius|diameter, a?: [x,y], b?: [x,y], vertex?: [x,y], source?: {kind: segment, edge: {node,subpath,segment}}|{kind: angle, first: edge, second: edge}|{kind: circle, node}|{kind: arc, edge}|{kind: points, a?, b?, vertex?: {node,subpath,anchor}}, offset?: pt, options?: {units: in|mm|pt|pc|px|cm|m|ft|yd, precision: 0-6, scale: drawing:real, hideUnits, lineWeight, lineType: solid|dashed|dotted, lineColor: [r,g,b], arrowStyle: filled|open|tick|none, arrowScale: %, extensionWeight, extensionType, extensionColor, extensionOffset, extensionLength, hideExtensions, fontFamily, fontStyle, fontSize, textColor, textPosition: away|between, textAlignment: left|center|right, textOffset}} → {id,label}; linked measurements follow source edits",
            has_doc,
            create
        ),
        cmd!(
            "dimension.update",
            "Update Dimensions",
            [],
            None,
            "{ids?: [dimension ids] (default selection), options?: dimension options, offset?: pt} restyle or move existing dimensions → {ids}",
            has_doc,
            update
        ),
        cmd!(
            "dimension.applyToAll",
            "Apply to All",
            [],
            None,
            "{id?: dimension id (default the first selected dimension)} gives every dimension in the document that dimension's style (its scale too), as one undo step → {ids}",
            has_doc,
            apply_to_all
        ),
    ]
}

fn error(s: impl ToString) -> EngineError {
    EngineError::Other(s.to_string())
}

fn options(s: &Session, p: &Value) -> Result<DimensionOptions> {
    let default = s.tool_options_of("dimension");
    let value = p.get("options").cloned().unwrap_or_else(|| default.get("style").cloned().unwrap_or(json!({})));
    DimensionOptions::from_params(value).map_err(error)
}

fn point(p: &Value, key: &str) -> Result<Point> {
    let a = p.get(key).and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| error(format!("{key} must be [x,y]")))?;
    let x = a.first().and_then(Value::as_f64).ok_or_else(|| error("Invalid point"))?;
    let y = a.get(1).and_then(Value::as_f64).ok_or_else(|| error("Invalid point"))?;
    Ok(Point::new(x, y))
}

fn distance(p: &Value, fallback: f64) -> Result<f64> {
    match p.get("offset") {
        Some(v) => v.as_f64().filter(|n| n.is_finite() && n.abs() <= 1e6).ok_or_else(|| error("Invalid dimension offset")),
        None => Ok(fallback),
    }
}

fn recipe(s: &Session, p: &Value) -> Result<Dimension> {
    let mode: DimensionMode =
        p.get("mode").map(|v| serde_json::from_value(v.clone()).map_err(|_| error("Unknown dimension mode"))).transpose()?.unwrap_or_default();
    let source: Option<DimensionSource> = p.get("source").map(|v| serde_json::from_value(v.clone()).map_err(error)).transpose()?;
    if let Some(source) = &source {
        let doc = &s.doc()?.doc;
        for id in source.nodes() {
            if doc.ancestry(id).is_some_and(|path| path.iter().any(|id| doc.node(*id).is_some_and(|n| n.extras().dimension.is_some()))) {
                return Err(error("Choose original artwork, not a dimension's generated paths"));
            }
        }
        let compatible = matches!(
            (mode, source),
            (DimensionMode::Linear, DimensionSource::Segment { .. })
                | (DimensionMode::Angular, DimensionSource::Angle { .. })
                | (DimensionMode::Radius | DimensionMode::Diameter, DimensionSource::Circle { .. } | DimensionSource::Arc { .. })
                | (DimensionMode::Linear | DimensionMode::Angular, DimensionSource::Points { .. })
        );
        if !compatible {
            return Err(error("Dimension mode does not match its source"));
        }
    }
    let a = if source.is_none() || p.get("a").is_some() { point(p, "a")? } else { Point::ZERO };
    let b = if source.is_none() || p.get("b").is_some() { point(p, "b")? } else { Point::new(1.0, 0.0) };
    let vertex = if mode == DimensionMode::Angular && (source.is_none() || matches!(source, Some(DimensionSource::Points { .. }))) {
        point(p, "vertex")?
    } else {
        Point::ZERO
    };
    let mut d = Dimension { mode, a, b, vertex, offset: distance(p, 30.0)?, length: a.distance(b), options: options(s, p)?, source, moved: None };
    d.resolve(&s.doc()?.doc).map_err(error)?;
    Ok(d)
}

fn insert(doc: &mut Document, parent: Option<NodeId>, dim: Dimension) -> Result<NodeId> {
    let id = doc.alloc_id();
    let mut group = Node::new(id, NodeKind::Group { children: vec![], clip: false });
    group.name = Some(format!("Dimension · {}", dim.label().map_err(error)?));
    group.kind = NodeKind::Group { children: artwork(doc, &dim, &[])?, clip: false };
    doc.dimensions = true;
    group.extras_mut().dimension = Some(Box::new(dim));
    doc.insert(parent, usize::MAX, group)?;
    Ok(id)
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let dim = recipe(s, p)?;
    let label = dim.label().map_err(error)?;
    let parent = s.doc()?.insertion_parent();
    s.note_journal("options", serde_json::to_value(&dim.options).map_err(error)?);
    let id = s.edit("Create Dimension", |doc, sel| {
        let id = insert(doc, parent, dim)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({"id":id.0,"label":label}))
}

fn dimension_of(doc: &Document, id: NodeId) -> Result<Dimension> {
    doc.node(id).and_then(|n| n.extras().dimension.as_deref()).cloned().ok_or_else(|| error("Select only dimensions"))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<NodeId> = match p.get("ids") {
        Some(v) => serde_json::from_value(v.clone()).map_err(error)?,
        None => s.doc()?.selection.objects.clone(),
    };
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(error("Select dimensions to update"));
    }
    let o = if p.get("options").is_some() { Some(options(s, p)?) } else { None };
    s.edit("Update Dimensions", |doc, _| {
        for id in &ids {
            if !doc.is_editable(*id) {
                return Err(error("Dimension is locked or hidden"));
            }
            let mut dim = dimension_of(doc, *id)?;
            if let Some(o) = &o {
                dim.options = o.clone();
            }
            dim.offset = distance(p, dim.offset)?;
            dim.resolve(doc).map_err(error)?;
            replace(doc, *id, dim)?;
        }
        Ok(())
    })?;
    Ok(json!({"ids":ids}))
}

fn apply_to_all(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let from = match p.get("id") {
        Some(v) => NodeId(v.as_u64().ok_or_else(|| error("id must be a dimension id"))?),
        None => *st
            .selection
            .objects
            .iter()
            .find(|id| st.doc.node(**id).is_some_and(|n| n.extras().dimension.is_some()))
            .ok_or_else(|| error("Select a dimension first"))?,
    };
    let o = dimension_of(&st.doc, from)?.options;
    let mut ids = vec![];
    let editable = st.doc.editable();
    st.doc.walk(|n| {
        if n.extras().dimension.is_some() && editable(n.id) {
            ids.push(n.id);
        }
    });
    drop(editable);
    s.edit("Apply to All", |doc, _| {
        for id in &ids {
            let mut dim = dimension_of(doc, *id)?;
            dim.options = o.clone();
            dim.resolve(doc).map_err(error)?;
            replace(doc, *id, dim)?;
        }
        Ok(())
    })?;
    Ok(json!({"ids":ids}))
}

fn replace(doc: &mut Document, id: NodeId, dim: Dimension) -> Result<()> {
    let old: Vec<NodeId> = doc.node(id).and_then(Node::children).map(|ch| ch.iter().map(|n| n.id).collect()).unwrap_or_default();
    let children = artwork(doc, &dim, &old)?;
    if let Some(n) = doc.node_mut(id) {
        n.kind = NodeKind::Group { children, clip: false };
        n.name = Some(format!("Dimension · {}", dim.label().map_err(error)?));
        n.extras_mut().dimension = Some(Box::new(dim));
    }
    Ok(())
}

/// Called inside every edit, so dimensions and their undo history stay in step with the art they
/// measure. Returns at once in documents without dimensions.
pub(crate) fn refresh(before: &Document, doc: &mut Document) -> Result<()> {
    if !doc.dimensions {
        return Ok(());
    }
    let mut dimensions = Vec::new();
    doc.walk(|n| {
        if let Some(d) = &n.extras().dimension {
            dimensions.push((n.id, (**d).clone()));
        }
    });
    doc.dimensions = !dimensions.is_empty();
    for (id, old) in dimensions {
        let mut next = old.clone();
        let moved = next.moved.take();
        if next.source.as_ref().is_some_and(|s| s.topology_changed(before, doc)) || next.resolve(doc).is_err() {
            // A deleted/degenerate source has no valid measurement. Remove its annotation in
            // this same undo step instead of keeping a silently wrong number.
            doc.remove(id)?;
            continue;
        }
        // Moved on its own (the art it measures stayed put): it keeps its new place.
        if let Some(v) = moved
            && (next.a, next.b, next.vertex) == (old.a, old.b, old.vertex)
        {
            next.place(next.handle() + v);
            if next.resolve(doc).is_err() {
                next = old.clone();
                next.moved = None;
            }
        }
        // A new document colour mode (Document Color Mode) redraws every dimension in it.
        if next != old
            || before.color_mode != doc.color_mode
            || before.node(id).and_then(|n| n.extras().dimension.as_deref()).is_some_and(|d| d != &next)
        {
            replace(doc, id, next)?;
        }
    }
    Ok(())
}

/// Redraw the dimensions of an opened document whose art isn't in its colour mode: files saved
/// before dimensions followed the colour mode have RGB dimensions in CMYK documents. A dimension
/// that can't be redrawn keeps the art it has.
pub(crate) fn conform(doc: &mut Document) {
    if !doc.dimensions {
        return;
    }
    let mode = doc.color_mode;
    let off_mode = |c: &Color| match mode {
        ColorMode::Rgb => matches!(c, Color::Cmyk { .. }),
        ColorMode::Cmyk => matches!(c, Color::Rgb { .. }),
    };
    let mut stale = Vec::new();
    doc.walk(|n| {
        if let Some(d) = &n.extras().dimension {
            let mut off = false;
            n.walk(&mut |c| {
                off |= [c.appearance.fill_paint(), c.appearance.stroke_paint()].iter().filter_map(|p| p.color()).any(|c| off_mode(&c));
                // The label's characters carry their own fill.
                if let NodeKind::Text(t) = &c.kind {
                    off |= t.runs.iter().filter_map(|r| r.style.fill.color()).any(|c| off_mode(&c));
                }
            });
            if off {
                stale.push((n.id, (**d).clone()));
            }
        }
    });
    for (id, d) in stale {
        // An error leaves this dimension as it was; the others are still redrawn.
        let _ = replace(doc, id, d);
    }
}

/// Option colour `c` as a paint in the document's colour model: in a CMYK document black is K
/// only (a rich four-ink black would print a black dimension in every plate) and other colours
/// convert through the active colour settings.
fn paint(c: [u8; 3], mode: ColorMode) -> Paint {
    let rgb = Color::rgb8(c[0], c[1], c[2]);
    Paint::solid(match mode {
        ColorMode::Rgb => rgb,
        ColorMode::Cmyk if c == [0, 0, 0] => Color::cmyk(0.0, 0.0, 0.0, 1.0),
        ColorMode::Cmyk => {
            let cms = vectorcraft_color::cms::active();
            cms.convert(&rgb, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent)
        }
    })
}
fn line(a: Point, b: Point) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

fn appearance(o: &DimensionOptions, mode: ColorMode, ext: bool, arrows: bool) -> Appearance {
    let (color, width, kind) =
        if ext { (o.extension_color, o.extension_weight, o.extension_type) } else { (o.line_color, o.line_weight, o.line_type) };
    let mut a = Appearance::basic(Paint::None, paint(color, mode), width);
    if let Some(st) = a.stroke_mut() {
        st.dash = match kind {
            LineType::Dashed => Some(Dash { pattern: vec![width * 5.0, width * 3.0], ..Default::default() }),
            LineType::Dotted => {
                st.cap = vectorcraft_doc::LineCap::Round;
                Some(Dash { pattern: vec![0.0, width * 3.0], ..Default::default() })
            }
            LineType::Solid => None,
        };
        if arrows {
            let head = match o.arrow_style {
                ArrowStyle::Filled => Some(Arrowhead::Triangle),
                ArrowStyle::Open => Some(Arrowhead::TriangleOpen),
                ArrowStyle::Tick => Some(Arrowhead::Slash),
                ArrowStyle::None => None,
            };
            st.start_arrow = head;
            st.end_arrow = head;
            st.arrow_scale = (o.arrow_scale, o.arrow_scale);
            st.arrow_align = vectorcraft_doc::appearance::ArrowAlign::Tip;
        }
    }
    a
}

/// How far along the line (0 start, 0.5 middle, 1 end) the label sits, kept clear of the arrows.
fn along(o: &DimensionOptions) -> f64 {
    match o.text_alignment {
        TextAlignment::Left => 0.2,
        TextAlignment::Center => 0.5,
        TextAlignment::Right => 0.8,
    }
}

fn artwork(doc: &mut Document, d: &Dimension, old: &[NodeId]) -> Result<Vec<Arc<Node>>> {
    d.validate().map_err(error)?;
    let o = &d.options;
    let mode = doc.color_mode;
    let label = d.label().map_err(error)?;
    let mut paths: Vec<(BezPath, Appearance)> = Vec::new();
    let mut angle = 0.0;
    let text_at;
    // Away from the object: past the dimension line; between: on the object's side of it.
    let away = o.text_position == TextPosition::Away;
    if d.mode == DimensionMode::Angular {
        let start = (d.a - d.vertex).atan2();
        let sweep = d.angle();
        let r = d.offset.abs().max(12.0);
        let on = |t: f64| d.vertex + Vec2::from_angle(start + sweep * t) * r;
        let mut arc = BezPath::new();
        arc.move_to(on(0.0));
        for i in 1..=64 {
            arc.line_to(on(i as f64 / 64.0));
        }
        paths.push((arc, appearance(o, mode, false, true)));
        if !o.hide_extensions {
            for p in [d.a, d.b] {
                let dir = (p - d.vertex) / (p - d.vertex).hypot();
                paths.push((line(d.vertex + dir * o.extension_offset, d.vertex + dir * (r + o.extension_length)), appearance(o, mode, true, false)));
            }
        }
        let dir = Vec2::from_angle(start + sweep * along(o));
        let gap = o.text_offset + o.font_size * 0.5;
        text_at = d.vertex + dir * if away { r + gap } else { (r - gap).max(o.font_size * 0.5) };
    } else {
        let delta = d.b - d.a;
        let u = delta / delta.hypot();
        let n = Vec2::new(-u.y, u.x);
        let (a, b) = match d.mode {
            DimensionMode::Radius => (d.a, d.b),
            DimensionMode::Diameter => (d.a - delta, d.b),
            _ => (d.a + n * d.offset, d.b + n * d.offset),
        };
        // The side of the line facing away from the object.
        let outward = match d.mode {
            DimensionMode::Linear => {
                if d.offset < 0.0 {
                    -n
                } else {
                    n
                }
            }
            _ => n,
        };
        if d.mode == DimensionMode::Linear && !o.hide_extensions {
            for (p, q) in [(d.a, a), (d.b, b)] {
                paths.push((line(p + outward * o.extension_offset, q + outward * o.extension_length), appearance(o, mode, true, false)));
            }
        }
        angle = u.atan2();
        if !(-std::f64::consts::FRAC_PI_2..=std::f64::consts::FRAC_PI_2).contains(&angle) {
            angle += std::f64::consts::PI;
        }
        let shift = o.text_offset + o.font_size * 0.5;
        text_at = a.lerp(b, along(o)) + outward * if away { shift } else { -shift };
        paths.push((line(a, b), appearance(o, mode, false, true)));
    }
    let style = CharStyle {
        font_family: o.font_family.clone(),
        font_style: o.font_style.clone(),
        size: o.font_size,
        fill: paint(o.text_color, mode),
        ..Default::default()
    };
    let mut text = TextObject::point(Point::ZERO, &label, style);
    text.para.justify = vectorcraft_doc::Justify::Center;
    text.system_layout = doc.setup.system_layout;
    super::typecmd::refresh_bounds(&mut text);
    // Centre the label's box on its anchor point, along the line.
    let bounds = text.cached_bounds.unwrap_or_else(|| text.estimate_bounds());
    let middle = Affine::translate(-bounds.center().to_vec2());
    text.xf = Affine::translate(text_at.to_vec2()) * Affine::rotate(angle) * middle;
    let mut children = Vec::new();
    for (path, a) in paths {
        let id = old.get(children.len()).copied().unwrap_or_else(|| doc.alloc_id());
        children.push(Arc::new(Node::path(id, PathData::from_bezpath(&path), a)));
    }
    let id = old.get(children.len()).copied().unwrap_or_else(|| doc.alloc_id());
    let mut node = Node::new(id, NodeKind::Text(Box::new(text)));
    node.appearance = Appearance::basic(paint(o.text_color, mode), Paint::None, 0.0).into();
    children.push(Arc::new(node));
    Ok(children)
}
