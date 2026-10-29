//! Eyedropper tool (I).
//!
//! Click an object: copy its attributes (what Eyedropper Options pick up and apply) to the
//! selection and the paint defaults (`appearance.copyFrom`); Alt-click: the selection's attributes
//! onto the clicked object (`reverse`); Shift+Alt-click: add the clicked object's appearance to the
//! selection's (`append`). Shift-click: sample only the colour under the cursor into the active
//! fill/stroke (`paint.sampleColor`; in a CMYK document an RGB color is stored as CMYK). Clicking
//! an image samples its pixels (averaged over the options' raster sample size; a CMYK image gives
//! its ink amounts).
//!
//! The Gradient panel's eyedropper sets the `stop` option (the tool to return to): the next click
//! samples the colour under the cursor into the selected gradient stop (`paint.sampleColor
//! {stop}`) and switches back to that tool.
//!
//! Brush Options' Key Color eyedropper sets `brushKeyColor` and `stop`: the next click samples into
//! the open dialog (`brushKeyColor` UI request) and switches back.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::hit::{HitKind, hit_test, hit_test_visible};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;

use super::paint_owner;
use crate::params::color_json;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Default)]
pub struct EyedropperTool {
    /// Sampling for the selected gradient stop: the tool to switch back to afterwards.
    stop: Option<String>,
    /// The Type tool's highlighted range while sampling, including its selection overlay.
    text_selection: crate::text::TypeTool,
    /// Brush Options › Key Color (Hue Shift): sample into the dialog, then [`Self::stop`].
    brush_key_color: bool,
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

/// The color of image `n`'s pixels at `p`, averaged over the raster sample size: a CMYK image's
/// ink amounts as they are, other images' sRGB pixels in the working RGB space (the color that
/// shows as that pixel).
fn image_color_at(cx: &ToolContext, n: &Node, p: Point) -> Option<Color> {
    let NodeKind::Image(im) = &n.kind else { return None };
    if im.xf.determinant().abs() < 1e-12 {
        return None;
    }
    let px = im.xf.inverse() * p;
    let blob = cx.doc.images.get(&im.key)?;
    let object = (im.width, im.height);
    if let Some(inks) = blob.sample_cmyk_object(px.x, px.y, object, cx.raster_sample) {
        let [c, m, y, k] = inks.map(|v| f32::from(v) / 255.0);
        return Some(Color::cmyk(c, m, y, k));
    }
    let [r, g, b, _] = blob.sample_object(px.x, px.y, object, cx.raster_sample)?;
    let [r, g, b] = vectorcraft_color::cms::active().srgb_to_rgb([r, g, b].map(|v| f32::from(v) / 255.0));
    Some(Color::rgb(r, g, b))
}

impl Tool for EyedropperTool {
    fn id(&self) -> &'static str {
        "eyedropper"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        // A click inside a fill or among characters samples it too, whatever Object and Type
        // Object Selection by Path Only say.
        let opts = vectorcraft_doc::hit::HitOptions { path_only: false, type_path_only: false, ..cx.hit_options() };
        if self.brush_key_color {
            let h = hit_test_visible(cx.doc, ev.pos, opts);
            let Some(h) = h else { return vec![] };
            let src = paint_owner(cx.doc, h.leaf);
            let Some(n) = cx.doc.node(src) else { return vec![] };
            let image = matches!(n.kind, NodeKind::Image(_));
            let sampled = if image { image_color_at(cx, n, ev.pos) } else { paint_sample(n, h.kind == HitKind::Stroke, ev.pos) };
            let Some(c) = sampled else { return vec![] };
            let back = self.stop.clone().unwrap_or_else(|| "selection".to_string());
            self.brush_key_color = false;
            self.stop = None;
            return vec![Action::Dialog("brushKeyColor".into(), json!({ "color": color_json(&c) })), Action::SwitchTool(back)];
        }
        let stop = self.stop.as_ref().zip(cx.gradient_stop);
        // Reading a source is independent of its editability. Only Alt's reverse operation
        // writes to the clicked object; Shift+Alt appends its appearance to the selection.
        let h = if ev.mods.alt && !ev.mods.shift && stop.is_none() { hit_test(cx.doc, ev.pos, opts) } else { hit_test_visible(cx.doc, ev.pos, opts) };
        let Some(h) = h else { return vec![] };
        let src = paint_owner(cx.doc, h.leaf);
        let Some(n) = cx.doc.node(src) else { return vec![] };
        if ev.mods.alt && stop.is_none() {
            let how = if ev.mods.shift { "append" } else { "reverse" };
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0, how: true }))];
        }
        let image = matches!(n.kind, NodeKind::Image(_));
        if !(ev.mods.shift || stop.is_some() || image) {
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0 }))];
        }
        let sampled = if image { image_color_at(cx, n, ev.pos) } else { paint_sample(n, h.kind == HitKind::Stroke, ev.pos) };
        let Some(c) = sampled else { return vec![] };
        let color = color_json(&c);
        let Some((back, i)) = stop else { return vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color }))] };
        let out = vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color, "stop": i })), Action::SwitchTool(back.clone())];
        self.stop = None;
        out
    }

    /// Expose a transient text range alongside gradient-stop and brush-key sampling state.
    fn options(&self) -> Value {
        let selected = self.text_selection.options();
        let mut options = if selected.get("editing").is_some_and(|v| !v.is_null()) { selected } else { json!({}) };
        if let Some(o) = options.as_object_mut() {
            o.insert("stop".into(), json!(self.stop));
            o.insert("brushKeyColor".into(), json!(self.brush_key_color));
        }
        options
    }

    /// Accept a stop target or the selection transferred from the Type tool.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "stop" => self.stop = value.as_str().map(str::to_string),
            "brushKeyColor" => self.brush_key_color = value.as_bool().unwrap_or(false),
            "textSelection" => self.text_selection.set_option(key, value),
            _ => {}
        }
    }

    /// Drop the range when its text is removed or no longer selected.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.text_selection.after_command(cx)
    }

    /// Keep the sampled character range highlighted without activating text input.
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if self.text_selection.options().get("editing").is_some_and(|v| !v.is_null()) { self.text_selection.overlays(cx) } else { vec![] }
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

    /// Upstream brush-key sampling and highlighted-text transfer keep independent state.
    #[test]
    fn text_range_and_brush_sampling_options_coexist() {
        let mut tool = EyedropperTool::default();
        tool.set_option("textSelection", &json!({"editing": 1, "anchor": 0, "caret": 4}));
        tool.set_option("stop", &json!("selection"));
        tool.set_option("brushKeyColor", &json!(true));
        let options = tool.options();
        assert_eq!((options["editing"].as_u64(), options["start"].as_u64(), options["end"].as_u64()), (Some(1), Some(0), Some(4)));
        assert_eq!(options["stop"], "selection");
        assert_eq!(options["brushKeyColor"], true);
        tool.set_option("brushKeyColor", &json!(false));
        assert_eq!(tool.options()["editing"], 1);
        assert_eq!(tool.options()["brushKeyColor"], false);
    }

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

    /// On a clip group's mask edge, where the art it clips lies over it, the eyedropper copies the
    /// art's appearance, not the unpainted clipping path's (the selection tools take the path there).
    #[test]
    fn on_a_masks_edge_the_eyedropper_copies_the_art() {
        let (d, _, _, art) = doc_with_masked_square();
        let (s, p) = (Selection::default(), paint());
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 498.5, 450.0));
        assert_eq!(a, vec![Action::Exec("appearance.copyFrom".into(), json!({"source": art.0}))]);
    }

    /// The tools that act on what is drawn hit with options that don't take a clipping path on
    /// its outline; only the selection tools' do.
    #[test]
    fn only_the_selection_options_take_clipping_paths_on_their_outline() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        assert!(!cx.hit_options().clip_outlines);
        assert!(cx.selection_hit_options().clip_outlines);
    }

    #[test]
    fn the_stop_option_samples_into_the_selected_stop_then_switches_back() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        t.set_option("stop", &json!("gradient"));
        assert_eq!(t.options(), json!({"stop": "gradient", "brushKeyColor": false}));
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
        assert_eq!(t.options(), json!({"stop": null, "brushKeyColor": false}), "one sample only");
    }

    #[test]
    fn brush_key_color_samples_into_the_dialog_then_switches_back() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        t.set_option("brushKeyColor", &json!(true));
        t.set_option("stop", &json!("selection"));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(
            a,
            vec![Action::Dialog("brushKeyColor".into(), json!({ "color": color_json(&Color::WHITE) })), Action::SwitchTool("selection".into())]
        );
        assert_eq!(t.options(), json!({ "stop": null, "brushKeyColor": false }));
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
            pixels: None,
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

    /// #2276: a CMYK image gives its ink amounts as they are, not the RGB its decoder shows.
    #[test]
    fn clicking_a_cmyk_image_samples_its_inks() {
        use vectorcraft_doc::cmyk::Inks;
        use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Two pixels, 10 pt each: C10 M20 Y30 K40 (in bytes), then K100.
        let inks = Inks::new(2, 1, vec![26, 51, 77, 102, 0, 0, 0, 255]).unwrap();
        d.images.insert("ink".into(), ImageBlob::cmyk_tiff(&inks).unwrap());
        let im = ImageObject {
            key: "ink".into(),
            width: 2,
            height: 1,
            xf: vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        let ink = |v: [u8; 4]| {
            let [c, m, y, k] = v.map(|b| f32::from(b) / 255.0);
            json!({ "color": color_json(&Color::cmyk(c, m, y, k)) })
        };
        let click = PointerEvent::new(PointerKind::Down, 5.0, 5.0);
        assert_eq!(EyedropperTool::default().pointer(&cx, &click), vec![Action::Exec("paint.sampleColor".into(), ink([26, 51, 77, 102]))]);
        cx.raster_sample = 3;
        assert_eq!(
            EyedropperTool::default().pointer(&cx, &click),
            vec![Action::Exec("paint.sampleColor".into(), ink([13, 26, 39, 179]))],
            "3 x 3 average of both pixels"
        );
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
