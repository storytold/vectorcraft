//! Crop Image (#734): a crop box over the selected image. It starts on the part of the image over
//! its artboard (the whole image when it lies inside one); its handles and its inside drag it,
//! always within the image, and the part outside it is dimmed. Enter (or the Control bar's Apply)
//! crops the image to it (`object.cropImage {rect}`), Escape (or Cancel) leaves it as it was; both
//! go back to the Selection tool. The Control bar edits the box through the `rect` option, the
//! point its X and Y name through `reference`, whether its W and H keep their proportions through
//! `linked`, and the cropped image's resolution through `ppi`.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageObject, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle};
use crate::xform::CYAN;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id; Object › Crop Image and the image's Crop Image buttons choose it.
pub const ID: &str = "cropImage";

/// The smallest the box gets, in points.
const MIN: f64 = 1.0;

/// The resolutions the Control bar's PPI list offers; those above the image's own are disabled.
pub const PPI_CHOICES: [f64; 5] = [72.0, 96.0, 150.0, 200.0, 300.0];

/// The highest resolution a linked image can be cropped to.
pub const LINKED_MAX_PPI: f64 = 300.0;

/// The reference point the box's X and Y name by default: its center (of the nine points of
/// [`vectorcraft_geom::reference_point`], left to right and top to bottom).
pub const CENTER: usize = 4;

/// An image's own resolution in pixels per inch (along its denser side, measured on its pixel
/// grid, so turning it changes nothing), and the highest resolution it can be cropped to: its own,
/// or [`LINKED_MAX_PPI`] for a linked image.
pub fn image_ppi(im: &ImageObject) -> (f64, f64) {
    let [a, b, c, d, _, _] = im.xf.as_coeffs();
    // The document lengths of one pixel's sides.
    let (sx, sy) = (a.hypot(b), c.hypot(d));
    let own = 72.0 / sx.min(sy).max(1e-9);
    (own, if im.link.is_some() { LINKED_MAX_PPI } else { own })
}

/// Whether cropping `im` at `ppi` keeps its pixels as they are: its pixels are square (the same
/// resolution across and down) and at that resolution, within `tolerance` ppi.
pub fn keeps_resolution(im: &ImageObject, ppi: f64, tolerance: f64) -> bool {
    let [a, b, c, d, _, _] = im.xf.as_coeffs();
    let (x, y) = (72.0 / a.hypot(b).max(1e-9), 72.0 / c.hypot(d).max(1e-9));
    (x - ppi).abs() <= tolerance && (y - ppi).abs() <= tolerance
}

/// Checks the values `tool.setOption` gives the tool before any is set: `ppi` a number from 1 to
/// 2400, and no more than `max` (the selected image's limit, from [`image_ppi`]), or null;
/// `reference` a whole number from 0 to 8; `linked` true or false.
pub fn check_options(values: &serde_json::Map<String, Value>, max: Option<f64>) -> Result<(), String> {
    for (key, v) in values {
        let error = match key.as_str() {
            "ppi" if v.is_null() => None,
            "ppi" => match v.as_f64().filter(|n| (1.0..=2400.0).contains(n)) {
                None => Some(format!("`ppi` must be a number from 1 to 2400 or null, not {v}")),
                Some(n) => {
                    max.filter(|m| n > m + 1e-6).map(|m| format!("`ppi` can't be more than {} for this image", (m * 100.0 + 1e-6).floor() / 100.0))
                }
            },
            "reference" => (!v.as_u64().is_some_and(|n| n <= 8)).then(|| format!("`reference` must be a whole number from 0 to 8, not {v}")),
            "linked" => (!v.is_boolean()).then(|| format!("`linked` must be true or false, not {v}")),
            _ => None,
        };
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok(())
}

/// The box `r` over an image with bounds `image`, with one Control bar field set: `x` or `y` moves
/// the box so its `reference` point is there; `width` or `height` resizes it about that point, the
/// other side following in proportion when `linked`. Sizes below [`MIN`] are raised to it, a
/// linked size with both sides together so it keeps its proportions. A
/// linked size is held to the image with both sides scaled together, and a box that fits is slid
/// back within the image, so its proportions survive being kept within it. `None` for another key
/// or a value that isn't finite.
pub fn edited_box(r: Rect, image: Rect, reference: usize, linked: bool, key: &str, v: f64) -> Option<Rect> {
    if !v.is_finite() {
        return None;
    }
    let p = vectorcraft_geom::reference_point(r, reference);
    let (mut w, mut h) = (r.width(), r.height());
    let fit = |b: Rect| if b.width() <= image.width() + 1e-9 && b.height() <= image.height() + 1e-9 { slid_within(b, image) } else { b };
    match key {
        "x" => return Some(fit(r + Vec2::new(v - p.x, 0.0))),
        "y" => return Some(fit(r + Vec2::new(0.0, v - p.y))),
        // Linked, both sides take one scale, raised so neither side falls below MIN.
        "width" | "height" if linked && w > 0.0 && h > 0.0 => {
            let k = (v / if key == "width" { w } else { h }).max(MIN / w).max(MIN / h);
            (w, h) = (w * k, h * k);
        }
        "width" => w = v.max(MIN),
        "height" => h = v.max(MIN),
        _ => return None,
    }
    if linked {
        let k = (image.width() / w).min(image.height() / h).min(1.0);
        if k.is_finite() && k > 0.0 {
            (w, h) = (w * k, h * k);
        }
    }
    // The reference point keeps its place: it sits at the same fraction of the new box.
    let (fx, fy) = ((reference % 3) as f64 / 2.0, (reference / 3).min(2) as f64 / 2.0);
    let (x0, y0) = (p.x - w * fx, p.y - h * fy);
    Some(fit(Rect::new(x0, y0, x0 + w, y0 + h)))
}

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

pub struct CropImageTool {
    /// The box, for the image it was made for (None: set from outside, for the selected image);
    /// None until it's dragged or set.
    rect: Option<(Option<NodeId>, Rect)>,
    drag: Option<Drag>,
    /// Where the pointer last was on the canvas (measurement label during a drag).
    last: Point,
    /// The cropped image's resolution in pixels per inch (None: the image's own).
    ppi: Option<f64>,
    /// The point of the box the Control bar's X and Y name (0..=8, [`CENTER`] by default).
    reference: usize,
    /// Whether the Control bar's W and H keep the box's proportions.
    linked: bool,
}

impl Default for CropImageTool {
    fn default() -> Self {
        Self { rect: None, drag: None, last: Point::ZERO, ppi: None, reference: CENTER, linked: false }
    }
}

impl CropImageTool {
    /// The selected image, its bounds and the box over it.
    fn current(&self, cx: &ToolContext) -> Option<(NodeId, Rect, Rect)> {
        let (id, _) = selected_image(cx.doc, cx.selection)?;
        let set = self.rect.filter(|(of, _)| of.is_none_or(|of| of == id)).map(|(_, r)| r);
        crop_box(cx.doc, cx.selection, set)
    }

    /// Crop to the box at the chosen resolution and go back to the Selection tool (only back when
    /// the box covers the image and the resolution is the image's own). The box and the resolution
    /// are forgotten when the tool is left, so a crop that fails keeps them for another try.
    fn apply(&mut self, cx: &ToolContext) -> Vec<Action> {
        let current = self.current(cx);
        let ppi = self.ppi;
        self.drag = None;
        let mut out = vec![];
        if let Some((id, image, r)) = current {
            let im = match cx.doc.node(id).map(|n| &n.kind) {
                Some(NodeKind::Image(im)) => Some(&**im),
                _ => None,
            };
            let max = im.map_or(f64::INFINITY, |im| image_ppi(im).1);
            // A resolution set before this image was selected is held to this image's limit.
            let resample = ppi.map(|v| v.min(max)).filter(|v| im.is_none_or(|im| !keeps_resolution(im, *v, 1e-3)));
            // A turned or skewed image is drawn to be cropped: the resolution goes with it even when
            // it is the image's own, which the PPI field shows until another is chosen (when it is
            // one `object.cropImage` takes). A resolution chosen for it crops even over the whole box.
            let drawn = im.filter(|im| {
                let [_, b, c, _, _, _] = im.xf.as_coeffs();
                b.abs() > 1e-9 || c.abs() > 1e-9
            });
            let own = |im: &ImageObject| Some(image_ppi(im).0).filter(|v| (1.0..=2400.0).contains(v) && *v <= max + 1e-6);
            let send = resample.or_else(|| drawn.and_then(|im| ppi.map(|v| v.min(max)).or_else(|| own(im))));
            if !same(r, image) || resample.is_some() || (drawn.is_some() && ppi.is_some()) {
                let mut p = json!({ "rect": [r.x0, r.y0, r.width(), r.height()] });
                if let Some(v) = send {
                    p["ppi"] = json!(v);
                }
                out.push(Action::Exec("object.cropImage".into(), p));
            }
        }
        out.push(Action::SwitchTool("selection".into()));
        out
    }

    /// Forget the box and the resolution; the reference point and the link stay as they were set.
    fn reset(&mut self) {
        self.rect = None;
        self.drag = None;
        self.last = Point::ZERO;
        self.ppi = None;
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
            PointerKind::TripleClick => {}
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

    /// A dropped press puts the box back as it was at the press; earlier changes to it stay.
    fn drop_press(&mut self, cx: &ToolContext) -> Vec<Action> {
        if let Some(Drag::Handle(_, from) | Drag::Move(_, from)) = self.drag.take()
            && let Some((id, _, _)) = self.current(cx)
        {
            self.rect = Some((Some(id), from));
        }
        vec![]
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
    /// [`initial_box`]); `ppi` once it's been set (until then the image's own); `reference` (0..=8)
    /// and `linked`.
    fn options(&self) -> Value {
        let mut o = json!({ "reference": self.reference, "linked": self.linked });
        if let Some((_, r)) = self.rect {
            o["rect"] = json!([r.x0, r.y0, r.width(), r.height()]);
        }
        if let Some(v) = self.ppi {
            o["ppi"] = json!(v);
        }
        o
    }

    /// `rect` `[x, y, width, height]` sets the box (kept within the selected image when used);
    /// anything else puts it back where it starts. `ppi` a positive number sets the cropped
    /// image's resolution (null: the image's own). `reference` a whole number 0..=8 sets the point X and Y name, and `linked`
    /// true or false the link of W and H. `tool.setOption` refuses other values of those three
    /// ([`check_options`]); one that reaches the tool anyway puts that setting back to its default.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "ppi" => {
                self.ppi = value.as_f64().filter(|v| v.is_finite() && *v > 0.0);
                return;
            }
            "reference" => {
                self.reference = value.as_u64().filter(|v| *v <= 8).map_or(CENTER, |v| v as usize);
                return;
            }
            "linked" => {
                self.linked = value.as_bool().unwrap_or(false);
                return;
            }
            "rect" => {}
            _ => return,
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
            pixels: None,
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

    /// Enter, then the switch to the Selection tool that a crop that ran is followed by.
    fn enter(t: &mut CropImageTool, cx: &ToolContext) -> Vec<Action> {
        let acts = t.key(cx, ToolKey::Enter, Mods::default());
        if acts.contains(&Action::SwitchTool("selection".into())) {
            assert!(t.deactivate(cx).is_empty());
        }
        acts
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
        let acts = enter(&mut t, &cx);
        assert_eq!(crop_rect(&acts), Some(vec![50.0, 70.0, 130.0, 90.0]));
        assert_eq!(acts.last(), Some(&Action::SwitchTool("selection".into())));
        assert!(t.options().get("rect").is_none(), "the next crop starts afresh");
        assert!(crop_rect(&enter(&mut t, &cx)).is_none(), "the whole image: nothing to cut");
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
        assert_eq!(crop_rect(&enter(&mut t, &cx)), Some(vec![0.0, 100.0, 150.0, 100.0]));
        // The Control bar sets the box; it stays within the image.
        t.set_option("rect", &json!([100, 120, 500, 20]));
        assert_eq!(crop_rect(&enter(&mut t, &cx)), Some(vec![100.0, 120.0, 50.0, 20.0]));
        // Nothing selected: no box, and the keys go elsewhere.
        let none = Selection::default();
        let cx = crate::testutil::cx(&d, &none, &p);
        assert!(t.overlays(&cx).is_empty() && !t.claims_key(&cx, ToolKey::Enter));
    }

    #[test]
    fn a_dropped_press_puts_the_box_back_as_it_was_at_the_press() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CropImageTool::default();
        drag(&mut t, &cx, (50.0, 60.0), (80.0, 70.0));
        assert_eq!(t.options()["rect"], json!([80.0, 70.0, 170.0, 90.0]));
        // A handle pressed and dragged, then the press dropped: the box is the adjusted one.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 250.0, 160.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 130.0));
        assert!(t.drop_press(&cx).is_empty());
        assert!(!t.busy());
        assert_eq!(t.options()["rect"], json!([80.0, 70.0, 170.0, 90.0]));
        // So is a moved box.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 110.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
        assert!(t.drop_press(&cx).is_empty());
        assert_eq!(t.options()["rect"], json!([80.0, 70.0, 170.0, 90.0]));
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

    /// The Control bar's fields: X and Y place the reference point, W and H resize about it, and
    /// the link keeps the proportions.
    #[test]
    fn fields_move_and_resize_the_box_about_its_reference_point() {
        let r = Rect::new(10.0, 20.0, 110.0, 70.0);
        let big = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        // The center (the default): today's behavior.
        assert_eq!(edited_box(r, big, CENTER, false, "x", 100.0), Some(Rect::new(50.0, 20.0, 150.0, 70.0)));
        assert_eq!(edited_box(r, big, CENTER, false, "width", 50.0), Some(Rect::new(35.0, 20.0, 85.0, 70.0)));
        // The top-left corner: X and Y name it, and a new size keeps it where it is.
        assert_eq!(edited_box(r, big, 0, false, "x", 0.0), Some(Rect::new(0.0, 20.0, 100.0, 70.0)));
        assert_eq!(edited_box(r, big, 0, false, "y", 5.0), Some(Rect::new(10.0, 5.0, 110.0, 55.0)));
        assert_eq!(edited_box(r, big, 0, false, "height", 10.0), Some(Rect::new(10.0, 20.0, 110.0, 30.0)));
        // The bottom-right corner keeps its place too.
        assert_eq!(edited_box(r, big, 8, false, "width", 40.0), Some(Rect::new(70.0, 20.0, 110.0, 70.0)));
        // Linked: the other side follows in proportion (100 × 50 → 40 × 20, then 60 × 30).
        assert_eq!(edited_box(r, big, 0, true, "width", 40.0), Some(Rect::new(10.0, 20.0, 50.0, 40.0)));
        assert_eq!(edited_box(r, big, 8, true, "height", 30.0), Some(Rect::new(50.0, 40.0, 110.0, 70.0)));
        // A size below a point is raised to one; other keys and values that aren't finite change nothing.
        assert_eq!(edited_box(r, big, 0, false, "width", -5.0), Some(Rect::new(10.0, 20.0, 11.0, 70.0)));
        assert_eq!(edited_box(r, big, 0, false, "angle", 5.0), None);
        assert_eq!(edited_box(r, big, 0, false, "x", f64::NAN), None);
        // Within a 120 × 80 image: a linked size is held to the image with both sides scaled
        // (200 × 100 → 120 × 60), and the box slides back within it, keeping its 2:1 proportions.
        let image = Rect::new(0.0, 0.0, 120.0, 80.0);
        assert_eq!(edited_box(r, image, 0, true, "width", 200.0), Some(Rect::new(0.0, 20.0, 120.0, 80.0)));
        assert_eq!(edited_box(r, image, 0, true, "height", 70.0), Some(Rect::new(0.0, 20.0, 120.0, 80.0)));
        // A move past the edge slides the box back without changing its size.
        assert_eq!(edited_box(r, image, 0, false, "x", 50.0), Some(Rect::new(20.0, 20.0, 120.0, 70.0)));
        // Unlinked, a side larger than the image is left for the box's own limit to cut.
        assert_eq!(edited_box(r, image, 0, false, "width", 200.0), Some(Rect::new(10.0, 20.0, 210.0, 70.0)));
    }

    #[test]
    fn reference_link_and_resolution_are_tool_options() {
        let mut t = CropImageTool::default();
        assert_eq!(t.options(), json!({ "reference": 4, "linked": false }));
        t.set_option("reference", &json!(0));
        t.set_option("linked", &json!(true));
        t.set_option("ppi", &json!(150));
        assert_eq!(t.options(), json!({ "reference": 0, "linked": true, "ppi": 150.0 }));
        // Values that don't read put each back to its default.
        for (k, v) in [("reference", json!(9)), ("linked", json!("yes")), ("ppi", json!(0)), ("ppi", json!(-72)), ("ppi", json!("high"))] {
            t.set_option(k, &v);
        }
        assert_eq!(t.options(), json!({ "reference": 4, "linked": false }));
    }

    #[test]
    fn option_values_are_checked_before_they_are_set() {
        let values = |v: Value| v.as_object().cloned().unwrap();
        assert_eq!(check_options(&values(json!({ "ppi": 150.5, "reference": 8, "linked": true, "rect": [0, 0, 1, 1] })), None), Ok(()));
        assert_eq!(check_options(&values(json!({ "ppi": null })), Some(36.0)), Ok(()));
        assert_eq!(check_options(&values(json!({ "ppi": 36 })), Some(36.0)), Ok(()));
        for (v, want) in [
            (json!({ "ppi": 0.5 }), "`ppi` must be a number from 1 to 2400 or null, not 0.5"),
            (json!({ "ppi": 2401 }), "`ppi` must be a number from 1 to 2400 or null, not 2401"),
            (json!({ "ppi": "high" }), "`ppi` must be a number from 1 to 2400 or null, not \"high\""),
            (json!({ "ppi": 72 }), "`ppi` can't be more than 36 for this image"),
            (json!({ "reference": 9 }), "`reference` must be a whole number from 0 to 8, not 9"),
            (json!({ "reference": 1.5 }), "`reference` must be a whole number from 0 to 8, not 1.5"),
            (json!({ "linked": "yes" }), "`linked` must be true or false, not \"yes\""),
        ] {
            assert_eq!(check_options(&values(v.clone()), Some(36.0)), Err(want.to_string()), "{v}");
        }
    }

    /// An image's own resolution, and the most it can be cropped to.
    #[test]
    fn image_resolution_and_its_limit() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let (id, _) = selected_image(&d, &s).unwrap();
        let Some(NodeKind::Image(im)) = d.node(id).map(|n| n.kind.clone()) else { panic!("an image") };
        // 100 px over 200 pt: half a pixel per point, 36 ppi.
        assert_eq!(image_ppi(&im), (36.0, 36.0));
        // Turned, it is the same image: 36 ppi, not the density across its bounding box.
        let turned = ImageObject { xf: Affine::rotate(0.5) * im.xf, ..(*im).clone() };
        assert!((image_ppi(&turned).0 - 36.0).abs() < 1e-9);
        // Stretched to 3 pt a pixel across, its denser side counts.
        let stretched = ImageObject { xf: Affine::scale_non_uniform(3.0, 2.0), ..(*im).clone() };
        assert_eq!(image_ppi(&stretched).0, 36.0);
        // Its own resolution keeps its pixels; pixels that are not square are resampled at any resolution.
        assert!(keeps_resolution(&im, 36.0, 1e-6) && !keeps_resolution(&im, 18.0, 1e-6));
        assert!(!keeps_resolution(&stretched, 36.0, 1e-6) && !keeps_resolution(&stretched, 24.0, 1e-6));
        let linked = ImageObject { link: Some("photo.png".into()), ..*im };
        assert_eq!(image_ppi(&linked), (36.0, LINKED_MAX_PPI));
    }

    /// Apply sends the chosen resolution with the box, even for a box over the whole image; the
    /// image's own resolution sends nothing new, and one above the image's limit is held to it.
    #[test]
    fn apply_sends_the_chosen_resolution() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let exec = |acts: &[Action]| {
            acts.iter().find_map(|a| match a {
                Action::Exec(c, v) if c == "object.cropImage" => Some(v.clone()),
                _ => None,
            })
        };
        let mut t = CropImageTool::default();
        t.set_option("ppi", &json!(20));
        assert_eq!(exec(&enter(&mut t, &cx)), Some(json!({ "rect": [50.0, 60.0, 200.0, 100.0], "ppi": 20.0 })));
        assert!(t.options().get("ppi").is_none(), "the next crop starts at the image's own");
        t.set_option("ppi", &json!(36));
        assert_eq!(exec(&enter(&mut t, &cx)), None, "the image's own over the whole image: nothing to do");
        t.set_option("ppi", &json!(36));
        t.set_option("rect", &json!([60, 70, 100, 50]));
        assert_eq!(exec(&enter(&mut t, &cx)), Some(json!({ "rect": [60.0, 70.0, 100.0, 50.0] })));
        // A resolution set before this image was selected is held to its limit: 300 → its own 36.
        t.set_option("ppi", &json!(300));
        t.set_option("rect", &json!([60, 70, 100, 50]));
        assert_eq!(exec(&enter(&mut t, &cx)), Some(json!({ "rect": [60.0, 70.0, 100.0, 50.0] })));
        t.set_option("ppi", &json!(300));
        assert_eq!(exec(&enter(&mut t, &cx)), None, "held to its own over the whole image: nothing to do");
    }

    /// A turned image is drawn to be cropped, so Apply sends its own resolution too, the one the
    /// PPI field shows, or the one chosen.
    #[test]
    fn apply_sends_a_turned_image_s_own_resolution() {
        // Turned about the page's corner, it stays within the artboard.
        let (mut d, s) = doc_with_image((150.0, 60.0));
        let Some(id) = s.objects.first().copied() else { panic!("an image") };
        if let Some(NodeKind::Image(im)) = d.node_mut(id).map(|n| &mut n.kind) {
            im.xf = Affine::rotate(0.5) * im.xf;
        }
        let p = paint();
        let cx = cx(&d, &s, &p);
        let ppi = |acts: &[Action]| {
            acts.iter().find_map(|a| match a {
                Action::Exec(c, v) if c == "object.cropImage" => v.get("ppi").and_then(Value::as_f64),
                _ => None,
            })
        };
        let mut t = CropImageTool::default();
        t.set_option("rect", &json!([150, 150, 20, 20]));
        assert!(ppi(&enter(&mut t, &cx)).is_some_and(|v| (v - 36.0).abs() < 1e-9));
        t.set_option("rect", &json!([150, 150, 20, 20]));
        t.set_option("ppi", &json!(36));
        assert!(ppi(&enter(&mut t, &cx)).is_some_and(|v| (v - 36.0).abs() < 1e-9));
        // Over the whole box, Apply crops at a resolution chosen, and leaves it alone without one.
        t.set_option("ppi", &json!(36));
        assert!(ppi(&enter(&mut t, &cx)).is_some_and(|v| (v - 36.0).abs() < 1e-9));
        assert!(!enter(&mut t, &cx).iter().any(|a| matches!(a, Action::Exec(..))));
        // A linked image's own resolution above what it can be cropped to is not sent in its place.
        let Some(NodeKind::Image(im)) = d.node(id).map(|n| n.kind.clone()) else { panic!("an image") };
        let mut linked = d.clone();
        if let Some(NodeKind::Image(l)) = linked.node_mut(id).map(|n| &mut n.kind) {
            l.link = Some("photo.png".into());
            l.xf = im.xf * Affine::scale(0.1);
        }
        // 0.2 pt a pixel: 360 ppi, above the 300 a linked image is cropped to.
        let cx = crate::testutil::cx(&linked, &s, &p);
        t.set_option("rect", &json!([105, 128, 4, 4]));
        assert_eq!(ppi(&enter(&mut t, &cx)), None);
    }

    /// A crop that fails leaves the tool chosen (the switch to the Selection tool is not made), so
    /// the box and the resolution stay for another try; leaving the tool forgets them.
    #[test]
    fn the_box_and_resolution_stay_until_the_tool_is_left() {
        let (d, s) = doc_with_image((50.0, 60.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CropImageTool::default();
        t.set_option("ppi", &json!(20));
        t.set_option("rect", &json!([60, 70, 100, 50]));
        let acts = t.key(&cx, ToolKey::Enter, Mods::default());
        assert_eq!(crop_rect(&acts), Some(vec![60.0, 70.0, 100.0, 50.0]));
        assert_eq!(t.options()["rect"], json!([60.0, 70.0, 100.0, 50.0]));
        assert_eq!(t.options()["ppi"], json!(20.0));
        assert!(t.deactivate(&cx).is_empty());
        assert!(t.options().get("rect").is_none() && t.options().get("ppi").is_none());
    }

    /// A linked size raised to the smallest box keeps the box's proportions: both sides grow
    /// together until the shorter one is a point.
    #[test]
    fn a_linked_size_at_the_smallest_keeps_its_proportions() {
        let big = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let wide = Rect::new(10.0, 20.0, 110.0, 70.0);
        assert_eq!(edited_box(wide, big, 0, true, "width", 1.0), Some(Rect::new(10.0, 20.0, 12.0, 21.0)));
        assert_eq!(edited_box(wide, big, 0, true, "height", 0.25), Some(Rect::new(10.0, 20.0, 12.0, 21.0)));
        let tall = Rect::new(10.0, 20.0, 60.0, 120.0);
        assert_eq!(edited_box(tall, big, 0, true, "height", 1.0), Some(Rect::new(10.0, 20.0, 11.0, 22.0)));
        assert_eq!(edited_box(tall, big, 0, true, "width", -3.0), Some(Rect::new(10.0, 20.0, 11.0, 22.0)));
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
