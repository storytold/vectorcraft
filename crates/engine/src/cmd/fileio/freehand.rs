//! FreeHand drawings (`.fh*`, and FreeHand templates `.ft*`): pages, layers and paths from
//! `vectorcraft-freehand`, mapped to editable VectorCraft art. The file is never written back.

use std::sync::Arc;

use kurbo::{BezPath, Point, Rect, Vec2};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, Dash, FillLayer, StrokeLayer};
use vectorcraft_doc::geom::{FillRule, PathData, shapes};
use vectorcraft_doc::text::{CharStyle, TextKind, TextObject, TextRun};
use vectorcraft_doc::{Artboard, Document, LayerColor, Node, NodeKind};
use vectorcraft_freehand::{self as fh, Fill, Item, Path as FhPath, Rgba, Stroke};

use super::load::err;
use crate::Result;

pub(super) fn import(bytes: &[u8]) -> Result<(Document, Vec<String>)> {
    let drawn = fh::read(bytes, fh::Limits::default()).map_err(|e| err(e.to_string()))?;
    let mut b = Builder { doc: Document::new(612.0, 792.0), warnings: Vec::new(), layer: 0 };
    b.document(&drawn);
    b.doc.limit_depth_noting(&mut b.warnings);
    let mut warnings: Vec<String> = drawn.warnings.iter().map(|w| format!("FreeHand: {w}.")).collect();
    warnings.extend(b.warnings.iter().map(|w| format!("FreeHand: {w}.")));
    Ok((b.doc, warnings))
}

struct Builder {
    doc: Document,
    warnings: Vec<String>,
    layer: u8,
}

impl Builder {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn document(&mut self, drawn: &fh::Document) {
        let pages = if drawn.pages.is_empty() { vec![fh::Page { x: 0.0, y: 0.0, width: 612.0, height: 792.0 }] } else { drawn.pages.clone() };
        self.doc.artboards = pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.width.is_finite() && p.height.is_finite() && p.width > 0.0 && p.height > 0.0)
            .map(|(i, p)| Artboard {
                id: i as u32 + 1,
                name: format!("Page {}", i + 1),
                rect: Rect::new(p.x, p.y, p.x + p.width, p.y + p.height),
                show_center_mark: false,
                show_cross_hairs: false,
            })
            .collect();
        if self.doc.artboards.is_empty() {
            self.doc.artboards = Document::new(612.0, 792.0).artboards;
            self.warn("the file had no page with a size; a US Letter page was used");
        }
        self.doc.layers = drawn
            .layers
            .iter()
            .map(|layer| {
                let kids = layer.items.iter().filter_map(|item| self.item(item)).collect();
                self.layer_node(&layer.name, kids, layer.visible, layer.locked)
            })
            .collect();
        if self.doc.layers.is_empty() {
            let empty = self.layer_node("Layer 1", Vec::new(), true, false);
            self.doc.layers.push(empty);
        }
    }

    fn layer_node(&mut self, name: &str, children: Vec<Arc<Node>>, visible: bool, locked: bool) -> Arc<Node> {
        let id = self.doc.alloc_id();
        self.layer = self.layer.wrapping_add(1);
        let label = if name.is_empty() { format!("Layer {}", self.layer) } else { name.to_string() };
        let mut n = Node::layer(id, &label, LayerColor::Preset(self.layer % 9));
        n.visible = visible;
        n.locked = locked;
        if let Some(c) = n.children_mut() {
            *c = children;
        }
        Arc::new(n)
    }

    fn item(&mut self, item: &Item) -> Option<Arc<Node>> {
        match item {
            Item::Group(kids) => {
                let children: Vec<_> = kids.iter().filter_map(|k| self.item(k)).collect();
                Some(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children, clip: false })))
            }
            Item::ClipGroup { clip, children } => {
                let mut nodes = Vec::new();
                nodes.push(self.path_node(clip, true, false));
                nodes.extend(children.iter().filter_map(|k| self.item(k)));
                Some(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children: nodes, clip: true })))
            }
            Item::Compound(path) => Some(self.compound(path)),
            Item::Path(path) => Some(self.path_node(path, false, true)),
            Item::Text(text) => self.text(text),
        }
    }

    fn path_node(&mut self, path: &FhPath, clipping: bool, painted: bool) -> Arc<Node> {
        let rule = if path.even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
        let data = path_data(path);
        let mut n = Node::new(self.doc.alloc_id(), NodeKind::Path { path: data.clone(), rule, live: None, clipping, guide: false });
        if painted {
            n.appearance = self.appearance(path, data.bounds()).into();
        }
        Arc::new(n)
    }

    fn compound(&mut self, path: &FhPath) -> Arc<Node> {
        let rule = if path.even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
        let data = path_data(path);
        let mut children = Vec::new();
        for sub in &path.subpaths {
            let one = FhPath { subpaths: vec![sub.clone()], even_odd: path.even_odd, fill: Fill::None, stroke: None };
            let child = path_data(&one);
            children.push(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Path { path: child, rule, live: None, clipping: false, guide: false })));
        }
        let mut n = Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule });
        n.appearance = self.appearance(path, data.bounds()).into();
        Arc::new(n)
    }

    fn appearance(&mut self, path: &FhPath, bounds: Option<Rect>) -> Appearance {
        let bounds = bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
        let mut items = Vec::new();
        if let Some((paint, opacity)) = self.fill(&path.fill, bounds) {
            let mut layer = FillLayer::new(paint);
            layer.opacity = opacity;
            items.push(AppearanceItem::Fill(layer));
        }
        if let Some(stroke) = &path.stroke
            && let Some(layer) = self.stroke(stroke)
        {
            items.push(AppearanceItem::Stroke(layer));
        }
        Appearance { items, ..Appearance::default() }
    }

    fn fill(&mut self, fill: &Fill, bounds: Rect) -> Option<(Paint, f32)> {
        match fill {
            Fill::None => None,
            Fill::Solid(c) => Some((Paint::solid(color(*c)), c.a.clamp(0.0, 1.0) as f32)),
            Fill::Linear { angle, stops } => {
                let kind = GradientKind::Linear;
                let geom = GradientGeom::fit(kind, bounds, *angle);
                Some((gradient(kind, stops, geom, *angle), 1.0))
            }
            Fill::Radial { cx, cy, stops } => {
                let kind = GradientKind::Radial;
                let centre = Point::new(bounds.x0 + *cx * bounds.width(), bounds.y0 + *cy * bounds.height());
                let radius = bounds.width().max(bounds.height()) / 2.0;
                let geom = GradientGeom { start: centre, end: centre + Vec2::new(radius.max(0.0), 0.0), aspect: 1.0, focal: None };
                Some((gradient(kind, stops, geom, 0.0), 1.0))
            }
        }
    }

    fn stroke(&mut self, stroke: &Stroke) -> Option<StrokeLayer> {
        if !stroke.width.is_finite() {
            self.warn("a stroke with a non-finite weight was left off");
            return None;
        }
        let mut layer = StrokeLayer::new(Paint::solid(color(stroke.color)), stroke.width);
        layer.opacity = stroke.color.a.clamp(0.0, 1.0) as f32;
        if stroke.dash.len() >= 2 {
            layer.dash = Some(Dash { pattern: stroke.dash.clone(), offset: 0.0, align_corners: false });
        }
        Some(layer)
    }

    fn text(&mut self, text: &fh::Text) -> Option<Arc<Node>> {
        if text.runs.is_empty() {
            return None;
        }
        let first = text.runs.first()?;
        let mut obj = TextObject::point(Point::new(text.x, text.y), &first.text, style_of(first));
        obj.runs = text.runs.iter().map(|r| TextRun::new(r.text.clone(), style_of(r))).collect();
        if text.width > 0.5 && text.height > 0.5 && text.width.is_finite() && text.height.is_finite() {
            obj.xf = kurbo::Affine::translate((text.x, text.y));
            obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, text.width, text.height)) };
        }
        Some(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Text(Box::new(obj)))))
    }
}

fn style_of(run: &fh::TextRun) -> CharStyle {
    let mut style = CharStyle::default();
    if !run.font.is_empty() {
        style.font_family = run.font.clone();
    }
    if run.size.is_finite() && run.size > 0.0 {
        style.size = run.size;
    }
    style.fill = Paint::solid(color(run.color));
    style
}

fn color(c: Rgba) -> Color {
    let f = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 };
    Color::Rgb { r: f(c.r), g: f(c.g), b: f(c.b) }
}

fn gradient(kind: GradientKind, stops: &[fh::Stop], geom: GradientGeom, angle: f64) -> Paint {
    let stops = stops
        .iter()
        .filter(|s| s.offset.is_finite())
        .map(|s| {
            let mut stop = GradientStop::new(s.offset.clamp(0.0, 1.0) as f32, color(s.color));
            stop.opacity = s.color.a.clamp(0.0, 1.0) as f32;
            stop
        })
        .collect();
    let mut paint = GradientPaint::new(Gradient { kind, stops });
    paint.angle = angle;
    paint.geom = Some(geom);
    Paint::Gradient(Box::new(paint))
}

fn path_data(path: &FhPath) -> PathData {
    let mut bp = BezPath::new();
    for sub in &path.subpaths {
        if !sub.start[0].is_finite() || !sub.start[1].is_finite() {
            continue;
        }
        bp.move_to(Point::new(sub.start[0], sub.start[1]));
        for seg in &sub.cubics {
            let [c1, c2, end] = *seg;
            if ![c1[0], c1[1], c2[0], c2[1], end[0], end[1]].iter().all(|v| v.is_finite()) {
                continue;
            }
            let (c1, c2, end) = (Point::new(c1[0], c1[1]), Point::new(c2[0], c2[1]), Point::new(end[0], end[1]));
            if c1 == c2 && c2 == end {
                bp.line_to(end);
            } else {
                bp.curve_to(c1, c2, end);
            }
        }
        if sub.closed {
            bp.close_path();
        }
    }
    PathData::from_bezpath(&bp)
}
