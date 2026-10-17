//! Eyedropper tool (I).
//!
//! Click an object: copy its attributes (what Eyedropper Options pick up and apply) to the
//! selection and the paint defaults (`appearance.copyFrom`); Alt-click: the selection's attributes
//! onto the clicked object (`reverse`); Shift+Alt-click: add the clicked object's appearance to the
//! selection's (`append`). Shift-click: sample only the colour under the cursor into the active
//! fill/stroke (`paint.sampleColor`, keeping the colour's model). Clicking an image samples its
//! pixels (averaged over the options' raster sample size).
//!
//! The Gradient panel's eyedropper sets the `stop` option (the tool to return to): the next click
//! samples the colour under the cursor into the selected gradient stop (`paint.sampleColor
//! {stop}`) and switches back to that tool.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::hit::{HitKind, HitOptions, hit_test, hit_test_visible};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::paint_owner;
use crate::params::color_json;
use crate::{Action, Cursor, Mods, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Default)]
pub struct EyedropperTool {
    /// Sampling for the selected gradient stop: the tool to switch back to afterwards.
    stop: Option<String>,
}

/// The colour of `paint` at `p` (gradients are sampled along their vector, radial ones honouring
/// their aspect ratio).
pub fn paint_color_at(paint: &Paint, bounds: Option<vectorcraft_geom::Rect>, p: Point) -> Option<Color> {
    match paint {
        Paint::Solid { color, .. } => Some(*color),
        Paint::Gradient(g) => {
            let geom = g.geom.or_else(|| bounds.map(|b| g.resolve(b)))?;
            let t = geom.param_at(g.gradient.kind, p);
            Some(g.gradient.sample(t.clamp(0.0, 1.0) as f32).0)
        }
        _ => None,
    }
}

/// The colour of `n`'s fill (or its stroke, when the stroke was hit or there is no fill) at `p`,
/// sampled in the paint's own space (text space for type runs) and box.
fn paint_sample(n: &Node, on_stroke: bool, p: Point) -> Option<Color> {
    let fill_none = n.proxy_paint(false, None).is_none_or(|(p, ..)| p.is_none());
    let (paint, to_doc, bounds) = n.proxy_paint(on_stroke || fill_none, None)?;
    let p = if to_doc.determinant().abs() > 1e-12 { to_doc.inverse() * p } else { p };
    paint_color_at(paint, Some(bounds), p)
}

/// The colour of image `n`'s pixels at `p`, averaged over `raster_sample` (1, 3 or 5).
fn image_color_at(doc: &Document, n: &Node, p: Point, raster_sample: u32) -> Option<Color> {
    let NodeKind::Image(im) = &n.kind else { return None };
    if im.xf.determinant().abs() < 1e-12 {
        return None;
    }
    let px = im.xf.inverse() * p;
    let [r, g, b, _] = doc.images.get(&im.key)?.sample_object(px.x, px.y, (im.width, im.height), raster_sample)?;
    Some(Color::rgb8(r, g, b))
}

/// What an Eyedropper click reads at `p`: the object whose appearance it copies, and the colour a
/// Shift-click (or a click on an image, or a gradient-stop sample) would take from it.
pub struct EyedropperHit {
    pub source: NodeId,
    pub color: Option<Color>,
}

/// The hit an Eyedropper click uses. `editable` is Alt without Shift and without a gradient-stop
/// sample: only art that can be edited. Otherwise visible art is read, locked and outside
/// isolation included. Fills count, whatever Object Selection by Path Only says.
pub fn read_at(doc: &Document, p: Point, editable: bool, raster_sample: u32, opt: HitOptions) -> Option<EyedropperHit> {
    let opt = HitOptions { path_only: false, type_path_only: false, ..opt };
    let h = if editable { hit_test(doc, p, opt) } else { hit_test_visible(doc, p, opt) }?;
    let source = paint_owner(doc, h.leaf);
    let n = doc.node(source)?;
    let color =
        if matches!(n.kind, NodeKind::Image(_)) { image_color_at(doc, n, p, raster_sample) } else { paint_sample(n, h.kind == HitKind::Stroke, p) };
    Some(EyedropperHit { source, color })
}

impl Tool for EyedropperTool {
    fn id(&self) -> &'static str {
        "eyedropper"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        let stop = self.stop.as_ref().zip(cx.gradient_stop);
        // Reading a source is independent of its editability. Only Alt's reverse operation
        // writes to the clicked object; Shift+Alt appends its appearance to the selection.
        let editable = ev.mods.alt && !ev.mods.shift && stop.is_none();
        let Some(read) = read_at(cx.doc, ev.pos, editable, cx.raster_sample, cx.hit_options()) else { return vec![] };
        let src = read.source;
        let Some(n) = cx.doc.node(src) else { return vec![] };
        if ev.mods.alt && stop.is_none() {
            let how = if ev.mods.shift { "append" } else { "reverse" };
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0, how: true }))];
        }
        let image = matches!(n.kind, NodeKind::Image(_));
        if !(ev.mods.shift || stop.is_some() || image) {
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0 }))];
        }
        let Some(c) = read.color else { return vec![] };
        let color = color_json(&c);
        let Some((back, i)) = stop else { return vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color }))] };
        let out = vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color, "stop": i })), Action::SwitchTool(back.clone())];
        self.stop = None;
        out
    }

    fn options(&self) -> Value {
        json!({ "stop": self.stop })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "stop" {
            self.stop = value.as_str().map(str::to_string);
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Eyedropper
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn click_copies_appearance_shift_samples_color() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(a, vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0}))]);
        let shift = Mods { shift: true, ..Default::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::WHITE)}))]);
        // On the stroke edge → the stroke colour.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::BLACK)}))]);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 400.0)).is_empty());
    }

    #[test]
    fn read_at_samples_a_fill_even_when_path_only_is_set() {
        let (d, id) = doc_with_rect();
        let opt = HitOptions { path_only: true, type_path_only: true, ..Default::default() };
        let read = read_at(&d, Point::new(150.0, 150.0), false, 1, opt).unwrap();
        assert_eq!(read.source, id);
        assert_eq!(read.color, Some(Color::WHITE));
        assert!(read_at(&d, Point::new(400.0, 400.0), false, 1, opt).is_none());
    }

    #[test]
    fn the_stop_option_samples_into_the_selected_stop_then_switches_back() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        t.set_option("stop", &json!("gradient"));
        assert_eq!(t.options(), json!({"stop": "gradient"}));
        // Without a selected stop the click copies the appearance as usual.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert!(matches!(&a[0], Action::Exec(c, _) if c == "appearance.copyFrom"));
        cx.gradient_stop = Some(1);
        // Empty canvas: still waiting.
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 400.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 150.0));
        assert_eq!(
            a,
            vec![
                Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::BLACK), "stop": 1})),
                Action::SwitchTool("gradient".into())
            ]
        );
        assert_eq!(t.options(), json!({"stop": null}), "one sample only");
    }

    /// The hex of the colour a `paint.sampleColor` action samples.
    fn sampled_hex(a: &[Action]) -> String {
        let [Action::Exec(c, p)] = a else { panic!("one action: {a:?}") };
        assert_eq!(c, "paint.sampleColor");
        let v: Vec<f32> = serde_json::from_value(p["color"].clone()).unwrap();
        Color::rgb(v[0], v[1], v[2]).to_hex()
    }

    #[test]
    fn alt_sends_reverse_and_shift_alt_append() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let mut t = EyedropperTool::default();
        let at = |m: Mods| PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(m);
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(t.pointer(&cx(&d, &s, &p), &at(alt)), vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0, "reverse": true}))]);
        let both = Mods { alt: true, shift: true, ..Default::default() };
        assert_eq!(t.pointer(&cx(&d, &s, &p), &at(both)), vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0, "append": true}))]);
    }

    #[test]
    fn sampling_keeps_the_colour_model() {
        let (mut d, id) = doc_with_rect();
        let cmyk = Color::cmyk(0.1, 0.2, 0.3, 0.4);
        d.node_mut(id).unwrap().appearance.set_fill(Paint::solid(cmyk));
        d.node_mut(id).unwrap().locked = true;
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&cmyk)}))]);
    }

    #[test]
    fn clicking_an_image_samples_its_pixels() {
        use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Three pixels, red, green, blue, 10 pt each.
        let img = image::RgbaImage::from_raw(3, 1, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]].concat()).unwrap();
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        d.images.insert("px".into(), ImageBlob::png(png));
        let im = ImageObject {
            key: "px".into(),
            width: 3,
            height: 1,
            xf: vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
        };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        let click = PointerEvent::new(PointerKind::Down, 15.0, 5.0);
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&cx, &click)), "#00ff00");
        cx.raster_sample = 3;
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&cx, &click)), "#555555", "3 x 3 average");
        // A template layer and a locked image still supply pixels. Hidden layers do not.
        d.node_mut(id).unwrap().locked = true;
        if let NodeKind::Layer { template, .. } = &mut d.node_mut(l).unwrap().kind {
            *template = true;
        }
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&crate::testutil::cx(&d, &s, &p), &click)), "#00ff00");
        d.node_mut(l).unwrap().visible = false;
        assert!(EyedropperTool::default().pointer(&crate::testutil::cx(&d, &s, &p), &click).is_empty());
    }

    #[test]
    fn gradient_sampled_along_vector() {
        let g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
        let paint = Paint::Gradient(Box::new(g));
        let r = vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 10.0);
        assert_eq!(paint_color_at(&paint, Some(r), Point::new(0.0, 5.0)).unwrap().to_hex(), "#ffffff");
        assert_eq!(paint_color_at(&paint, Some(r), Point::new(100.0, 5.0)).unwrap().to_hex(), "#000000");
    }

    #[test]
    fn type_gradients_are_sampled_in_text_space() {
        use vectorcraft_color::{Gradient, GradientGeom, GradientPaint};
        use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Type turned 90°: its baseline runs down from (100, 100).
        let mut t = TextObject::point(Point::ZERO, "Gradient", CharStyle { size: 20.0, ..Default::default() });
        t.xf = vectorcraft_geom::Affine::translate((100.0, 100.0)) * vectorcraft_geom::Affine::rotate(std::f64::consts::FRAC_PI_2);
        let mut g = GradientPaint::new(Gradient::default());
        g.geom = Some(GradientGeom { start: Point::ZERO, end: Point::new(100.0, 0.0), aspect: 1.0, focal: None });
        t.runs[0].style.fill = Paint::Gradient(Box::new(g));
        d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        // 80 % along the baseline (text space (80, -5)).
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 105.0, 180.0).with_mods(shift));
        assert_eq!(sampled_hex(&a), "#333333");
    }

    #[test]
    fn radial_sample_honours_aspect() {
        let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient {
            kind: vectorcraft_color::GradientKind::Radial,
            ..Default::default()
        });
        // Radius 100 along x, 50 across: 50 pt above the centre is already the outer (black) edge.
        g.geom = Some(vectorcraft_color::GradientGeom { start: Point::ZERO, end: Point::new(100.0, 0.0), aspect: 0.5, focal: None });
        let paint = Paint::Gradient(Box::new(g));
        assert_eq!(paint_color_at(&paint, None, Point::new(0.0, -50.0)).unwrap().to_hex(), "#000000");
        assert_eq!(paint_color_at(&paint, None, Point::new(50.0, 0.0)).unwrap().to_hex(), "#808080");
    }
}
