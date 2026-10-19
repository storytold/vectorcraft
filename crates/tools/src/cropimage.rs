//! Crop Image (#734): a crop box over the selected image. It starts on the part of the image over
//! its artboard (the whole image when it lies inside one); its handles and its inside drag it,
//! always within the image, and the part outside it is dimmed. Enter (or the Control bar's Apply)
//! crops the image to it (`object.cropImage {rect}`), Escape (or Cancel) leaves it as it was; both
//! go back to the Selection tool. The Control bar edits the box through the `rect` option.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle};
use crate::xform::CYAN;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id; Object › Crop Image and the image's Crop Image buttons choose it.
pub const ID: &str = "cropImage";

/// The smallest the box gets, in points.
const MIN: f64 = 1.0;

/// Create the Crop Image tool by id (None = not ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == ID).then(|| Box::new(CropImageTool::default()) as Box<dyn Tool>)
}

/// The selected image (the first image of the selection) and its bounds.
pub fn selected_image(doc: &Document, sel: &Selection) -> Option<(NodeId, Rect)> {
    sel.objects.iter().find_map(|id| {
        let n = doc.node(*id)?;
        matches!(n.kind, NodeKind::Image(_)).then_some(())?;
        Some((*id, n.geometric_bounds()?))
    })
}

/// Where the box starts for an image with bounds `image`: its part over the artboard its centre is
/// on, or all of it when it lies inside that artboard or isn't on one.
pub fn initial_box(doc: &Document, image: Rect) -> Rect {
    doc.artboard_at(image.center())
        .and_then(|i| doc.artboards.get(i))
        .map(|a| a.rect.intersect(image))
        .filter(|r| r.width() >= MIN && r.height() >= MIN)
        .unwrap_or(image)
}

/// The selected image, its bounds and the box over it: `set` kept within the image, or where the
/// box starts ([`initial_box`]).
pub fn crop_box(doc: &Document, sel: &Selection, set: Option<Rect>) -> Option<(NodeId, Rect, Rect)> {
    let (id, image) = selected_image(doc, sel)?;
    Some((id, image, set.map_or_else(|| initial_box(doc, image), |r| within(r, image))))
}

/// `r` (moved whole) slid back within `image` without changing its size; `r` lies within `image`
/// to begin with, so it fits.
fn slid_within(r: Rect, image: Rect) -> Rect {
    let x0 = r.x0.min(image.x1 - r.width()).max(image.x0);
    let y0 = r.y0.min(image.y1 - r.height()).max(image.y0);
    Rect::new(x0, y0, x0 + r.width(), y0 + r.height())
}

/// `r` cut to `image`, at least [`MIN`] wide and high.
fn within(r: Rect, image: Rect) -> Rect {
    let r = r.abs().intersect(image);
    let w = r.width().max(MIN).min(image.width());
    let h = r.height().max(MIN).min(image.height());
    let x0 = r.x0.min(image.x1 - w).max(image.x0);
    let y0 = r.y0.min(image.y1 - h).max(image.y0);
    Rect::new(x0, y0, x0 + w, y0 + h)
}

#[derive(Clone, Copy)]
enum Drag {
    /// A handle, with the box as it was pressed.
    Handle(Handle, Rect),
    /// The box itself, from where it was pressed.
    Move(Point, Rect),
}

#[derive(Default)]
pub struct CropImageTool {
    /// The box, for the image it was made for (None: set from outside, for the selected image);
    /// None until it's dragged or set.
    rect: Option<(Option<NodeId>, Rect)>,
    drag: Option<Drag>,
    /// Where the pointer last was on the canvas (measurement label during a drag).
    last: Point,
}

impl CropImageTool {
    /// The selected image, its bounds and the box over it.
    fn current(&self, cx: &ToolContext) -> Option<(NodeId, Rect, Rect)> {
        let (id, _) = selected_image(cx.doc, cx.selection)?;
        let set = self.rect.filter(|(of, _)| of.is_none_or(|of| of == id)).map(|(_, r)| r);
        crop_box(cx.doc, cx.selection, set)
    }

    /// Crop to the box and go back to the Selection tool (only back when it covers the image).
    fn apply(&mut self, cx: &ToolContext) -> Vec<Action> {
        let current = self.current(cx);
        self.reset();
        let mut out = vec![];
        if let Some((_, image, r)) = current
            && !same(r, image)
        {
            out.push(Action::Exec("object.cropImage".into(), json!({ "rect": [r.x0, r.y0, r.width(), r.height()] })));
        }
        out.push(Action::SwitchTool("selection".into()));
        out
    }

    fn reset(&mut self) {
        self.rect = None;
        self.drag = None;
        self.last = Point::ZERO;
    }
}

/// Whether two rectangles are the same to a thousandth of a point.
fn same(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-3)
}

/// The box `r` with the sides handle `h` holds moved to `p`.
fn resized(r: Rect, h: Handle, p: Point) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
    match h {
        Handle::TopLeft | Handle::Left | Handle::BottomLeft => x0 = p.x.min(x1 - MIN),
        Handle::TopRight | Handle::Right | Handle::BottomRight => x1 = p.x.max(x0 + MIN),
        Handle::Top | Handle::Bottom => {}
    }
    match h {
        Handle::TopLeft | Handle::Top | Handle::TopRight => y0 = p.y.min(y1 - MIN),
        Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => y1 = p.y.max(y0 + MIN),
        Handle::Left | Handle::Right => {}
    }
    Rect::new(x0, y0, x1, y1)
}

impl Tool for CropImageTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let Some((id, image, r)) = self.current(cx) else { return vec![] };
        match ev.kind {
            PointerKind::Down => {
                self.last = ev.pos;
                self.drag = match hit_handle(r, ev.pos, cx.tol(5.0)) {
                    Some(h) => Some(Drag::Handle(h, r)),
                    None => r.contains(ev.pos).then_some(Drag::Move(ev.pos, r)),
                };
            }
            PointerKind::Drag => {
                self.last = ev.pos;
                let next = match self.drag {
                    Some(Drag::Handle(h, from)) => within(resized(from, h, ev.pos), image),
                    Some(Drag::Move(start, from)) => {
                        let d: Vec2 = ev.pos - start;
                        slid_within(from + d, image)
                    }
                    None => return vec![],
                };
                self.rect = Some((Some(id), next));
            }
            PointerKind::Up => self.drag = None,
            PointerKind::DoubleClick => return self.apply(cx),
            PointerKind::Move => self.last = ev.pos,
        }
        vec![]
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Enter => self.apply(cx),
            ToolKey::Escape => {
                self.reset();
                vec![Action::SwitchTool("selection".into())]
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Enter | ToolKey::Escape) && self.current(cx).is_some()
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((_, image, r)) = self.current(cx) else { return vec![] };
        let shade = [0, 0, 0, 110];
        let quad = |x0: f64, y0: f64, x1: f64, y1: f64| [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];
        // The image outside the box, dimmed: above, below, then left and right of it.
        let mut out: Vec<Overlay> = [
            quad(image.x0, image.y0, image.x1, r.y0),
            quad(image.x0, r.y1, image.x1, image.y1),
            quad(image.x0, r.y0, r.x0, r.y1),
            quad(r.x1, r.y0, image.x1, r.y1),
        ]
        .into_iter()
        .filter(|q| (q[2].x - q[0].x) > 1e-9 && (q[2].y - q[0].y) > 1e-9)
        .map(|quad| Overlay::Highlight { quad, color: shade })
        .collect();
        let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        out.extend(corners.iter().zip(corners.iter().cycle().skip(1)).map(|(a, b)| Overlay::Line { a: *a, b: *b, color: CYAN, dashed: false }));
        out.extend(Handle::ALL.iter().map(|h| Overlay::Handle { p: h.pos(r), color: CYAN }));
        if cx.measurement_labels {
            let text = cx.size_label(r.width(), r.height());
            if self.drag.is_some() {
                out.push(Overlay::Measure { p: self.last, text });
            } else {
                out.push(Overlay::Label { p: Point::new(r.x0, r.y1), text, color: CYAN });
            }
        }
        out
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        let Some((_, _, r)) = self.current(cx) else { return Cursor::Arrow };
        match hit_handle(r, p, cx.tol(5.0)) {
            Some(Handle::Top | Handle::Bottom) => Cursor::ResizeV,
            Some(Handle::Left | Handle::Right) => Cursor::ResizeH,
            Some(Handle::TopLeft | Handle::BottomRight) => Cursor::ResizeNwSe,
            Some(Handle::TopRight | Handle::BottomLeft) => Cursor::ResizeNeSw,
            None if r.contains(p) => Cursor::Move,
            None => Cursor::Arrow,
        }
        .turned(cx.view_angle())
    }

    /// `rect`: the box as `[x, y, width, height]` once it's been dragged or set (until then it's
    /// [`initial_box`]).
    fn options(&self) -> Value {
        match self.rect {
            Some((_, r)) => json!({ "rect": [r.x0, r.y0, r.width(), r.height()] }),
            None => json!({}),
        }
    }

    /// `rect` `[x, y, width, height]` sets the box (kept within the selected image when used);
    /// anything else puts it back where it starts.
    fn set_option(&mut self, key: &str, value: &Value) {
        if key != "rect" {
            return;
        }
        // Invalid components must discard the whole box, not silently shift the
        // coordinates by dropping a non-numeric entry.
        let numbers = value
            .as_array()
            .filter(|a| a.len() == 4)
            .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<_>>>())
            .filter(|a| a.iter().all(|n| n.is_finite()));
        self.rect = match numbers.as_deref() {
            Some([x, y, w, h]) => Some((None, Rect::new(*x, *y, x + w, y + h))),
            _ => None,
        };
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.reset();
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{ImageObject, Node};
    use vectorcraft_geom::Affine;

    use super::*;
    use crate::testutil::{cx, paint, turned_view};

    /// A 500 × 500 document (one artboard) with a 100 × 50 px image drawn at twice that size from
    /// `at`, selected.
    fn doc_with_image(at: (f64, f64)) -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let im = ImageObject {
            key: "k".into(),
            width: 100,
            height: 50,
            xf: Affine::translate(at) * Affine::scale(2.0),
            link: None,
            placement: Default::default(),
        };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, s)
    }

    fn drag(t: &mut CropImageTool, cx: &ToolContext, from: (f64, f64), to: (f64, f64)) {
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            assert!(t.pointer(cx, &PointerEvent::new(kind, x, y)).is_empty());
        }
    }

    fn crop_rect(acts: &[Action]) -> Option<Vec<f64>> {
        acts.iter().find_map(|a| match a {
            Action::Exec(c, v) if c == "object.cropImage" => Some(v["rect"].as_array()?.iter().filter_map(Value::as_f64).collect()),
            _ => None,
        })
    }

    #[test]
    fn the_box_is_dragged_by_its_handles_and_inside_within_the_image() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CropImageTool::default();
        // An image inside its artboard: the box starts on all of it, and Enter has nothing to cut.
        assert_eq!(t.cursor(&cx, Point::new(50.0, 60.0), Mods::default()), Cursor::ResizeNwSe);
        assert_eq!(t.cursor(&cx, Point::new(150.0, 110.0), Mods::default()), Cursor::Move);
        assert!(t.claims_key(&cx, ToolKey::Enter));
        // The top-left handle in, then the box dragged left past the image's edge: it stops there.
        drag(&mut t, &cx, (50.0, 60.0), (80.0, 70.0));
        assert_eq!(t.options()["rect"], json!([80.0, 70.0, 170.0, 90.0]));
        drag(&mut t, &cx, (150.0, 110.0), (50.0, 110.0));
        assert_eq!(t.options()["rect"], json!([50.0, 70.0, 170.0, 90.0]));
        // A handle dragged past the opposite side leaves the box at least a point wide.
        drag(&mut t, &cx, (220.0, 115.0), (0.0, 115.0));
        assert_eq!(t.options()["rect"], json!([50.0, 70.0, 1.0, 90.0]));
        drag(&mut t, &cx, (51.0, 115.0), (180.0, 115.0));
        // The outside of the box is dimmed, the box drawn with its handles.
        let ov = t.overlays(&cx);
        assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Highlight { .. })).count(), 2, "above and right of the box");
        assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Handle { .. })).count(), 8);
        // Enter crops to it and goes back to the Selection tool.
        let acts = t.key(&cx, ToolKey::Enter, Mods::default());
        assert_eq!(crop_rect(&acts), Some(vec![50.0, 70.0, 130.0, 90.0]));
        assert_eq!(acts.last(), Some(&Action::SwitchTool("selection".into())));
        assert!(t.options().get("rect").is_none(), "the next crop starts afresh");
        assert!(crop_rect(&t.key(&cx, ToolKey::Enter, Mods::default())).is_none(), "the whole image: nothing to cut");
    }

    /// In a view turned 90° the handles' cursors turn with the view.
    #[test]
    fn handle_cursors_turn_with_the_view() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let mut c = cx(&d, &s, &p);
        c.screen = turned_view(90.0);
        let t = CropImageTool::default();
        assert_eq!(t.cursor(&c, Point::new(50.0, 60.0), Mods::default()), Cursor::ResizeNeSw, "top-left");
        assert_eq!(t.cursor(&c, Point::new(150.0, 60.0), Mods::default()), Cursor::ResizeH, "top-middle");
    }

    #[test]
    fn the_box_starts_on_the_part_over_the_artboard_and_escape_cancels() {
        // 200 × 100 from (-50, 100): its left 50 pt hang off the artboard.
        let (d, s) = doc_with_image((-50.0, 100.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CropImageTool::default();
        drag(&mut t, &cx, (100.0, 150.0), (110.0, 150.0));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
        assert_eq!(crop_rect(&t.key(&cx, ToolKey::Enter, Mods::default())), Some(vec![0.0, 100.0, 150.0, 100.0]));
        // The Control bar sets the box; it stays within the image.
        t.set_option("rect", &json!([100, 120, 500, 20]));
        assert_eq!(crop_rect(&t.key(&cx, ToolKey::Enter, Mods::default())), Some(vec![100.0, 120.0, 50.0, 20.0]));
        // Nothing selected: no box, and the keys go elsewhere.
        let none = Selection::default();
        let cx = crate::testutil::cx(&d, &none, &p);
        assert!(t.overlays(&cx).is_empty() && !t.claims_key(&cx, ToolKey::Enter));
    }

    #[test]
    fn crop_box_option_requires_exactly_four_finite_numeric_values() {
        let mut tool = CropImageTool::default();
        for invalid in [json!([10, "ignored", 20, 30, 40]), json!([10, 20, 30, null, 40]), json!([10, 20, 30, 40, 50]), json!("not a rectangle")] {
            tool.set_option("rect", &invalid);
            assert!(tool.options().get("rect").is_none(), "invalid box should reset crop: {invalid}");
        }
        tool.set_option("rect", &json!([10, 20, 30, 40]));
        assert_eq!(tool.options()["rect"], json!([10.0, 20.0, 30.0, 40.0]));
    }

    #[test]
    fn the_tool_is_made_by_its_id() {
        assert_eq!(crate::create(ID).id(), ID);
    }

    /// Smart Guides › Measurement Labels: the crop size readout follows the preference and the
    /// pointer while the box is dragged.
    #[test]
    fn measurement_labels_gate_the_crop_size_readout() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let mut t = CropImageTool::default();
        let count = |on: bool| {
            let mut tool = CropImageTool::default();
            let c = ToolContext { measurement_labels: on, ..cx(&d, &s, &p) };
            tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 50.0, 60.0));
            tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 80.0, 70.0));
            tool.overlays(&c).iter().filter(|o| matches!(o, Overlay::Measure { .. } | Overlay::Label { .. })).count()
        };
        assert_eq!(count(true), 1);
        assert_eq!(count(false), 0);
        let c = cx(&d, &s, &p);
        drag(&mut t, &c, (50.0, 60.0), (80.0, 70.0));
        assert!(
            t.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == &c.size_label(170.0, 90.0))),
            "idle: label at the box"
        );
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 110.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
        assert!(
            t.overlays(&c)
                .iter()
                .any(|o| matches!(o, Overlay::Measure { p, text, .. } if *p == Point::new(120.0, 100.0) && text == &c.size_label(170.0, 90.0))),
            "drag: measure at the pointer"
        );
    }
}
