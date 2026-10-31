//! Eyedropper tool (I).
//!
//! Click an object: copy its attributes (what Eyedropper Options pick up and apply) to the
//! selection and the paint defaults (`appearance.copyFrom`); Alt-click: the selection's attributes
//! onto the clicked object (`reverse`); Shift+Alt-click: add the clicked object's appearance to the
//! selection's (`append`). Shift-click: sample only the colour under the cursor into the active
//! fill/stroke (`paint.sampleColor`; in a CMYK document an RGB color is stored as CMYK). On a
//! blend or an envelope it samples the art drawn there (a blend's steps, an envelope's distorted
//! contents), or what lies below where that art leaves the point empty. Clicking an image samples
//! its pixels (averaged over the options' raster sample size; a CMYK image gives its ink amounts).
//!
//! The Gradient panel's eyedropper sets the `stop` option (the tool to return to): the next click
//! samples the colour under the cursor into the selected gradient stop (`paint.sampleColor
//! {stop}`) and switches back to that tool.
//!
//! Brush Options' Key Color eyedropper sets `brushKeyColor` and `stop`: the next click samples into
//! the open dialog (`brushKeyColor` UI request) and switches back.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::hit::{Hit, HitKind, hit_test, hit_test_visible};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
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

/// The visible object under `p`, using the Eyedropper's source selection rules.
pub(crate) fn sample_source(cx: &ToolContext, p: Point) -> Option<(vectorcraft_doc::NodeId, HitKind)> {
    let opts = vectorcraft_doc::hit::HitOptions { path_only: false, type_path_only: false, ..cx.hit_options() };
    let h = hit_test_visible(cx.doc, p, opts)?;
    Some((paint_owner(cx.doc, h.leaf), h.kind))
}

/// Most blends and envelopes one sample evaluates, front to back, for art that paints the point.
const MAX_LIVE: usize = 1_000;

/// The visible color under `p`, including image pixels, intermediate gradient colors, the steps a
/// blend draws and an envelope's distorted contents.
pub(crate) fn sample_color_at(cx: &ToolContext, p: Point) -> Option<Color> {
    let opts = vectorcraft_doc::hit::HitOptions { path_only: false, type_path_only: false, ..cx.hit_options() };
    // A blend or an envelope is hit anywhere in its bounds: it is replaced by its evaluated art in
    // a copy of the document, and the point is hit again there. Where that art leaves the point
    // empty, the objects below it are hit. The art made for one sample counts against one
    // `MAX_GENERATED`.
    let mut drawn: Option<Document> = None;
    let mut generated = 0usize;
    for _ in 0..=MAX_LIVE {
        let d = drawn.as_ref().unwrap_or(cx.doc);
        let h = hit_test_visible(d, p, opts)?;
        let Some(live) = drawn_live(d, &h) else {
            let n = d.node(paint_owner(d, h.leaf))?;
            return if matches!(n.kind, NodeKind::Image(_)) { image_color_at(cx, d, n, p) } else { paint_sample(n, h.kind == HitKind::Stroke, p) };
        };
        let doc = drawn.get_or_insert_with(|| cx.doc.clone());
        let mut art = vectorcraft_effects::expand_live_deep(Some(doc), doc.node(live)?);
        generated = generated.saturating_add(art.count());
        if generated > vectorcraft_doc::live::MAX_GENERATED {
            return None;
        }
        renumber_children(doc, &mut art);
        *doc.node_mut(live)? = art;
    }
    None
}

/// The outermost blend or envelope (other than one whose contents are being edited) that `h` hit
/// or hit inside: the canvas draws its evaluated art, not the objects it holds.
fn drawn_live(doc: &Document, h: &Hit) -> Option<NodeId> {
    h.ancestry
        .iter()
        .copied()
        .find(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. } | NodeKind::Envelope { editing: false, .. })))
}

/// Gives everything in `n` new ids from `doc` (evaluated art repeats its objects' ids).
fn renumber_children(doc: &mut Document, n: &mut Node) {
    for c in n.children_mut().into_iter().flatten() {
        let c = std::sync::Arc::make_mut(c);
        c.id = doc.alloc_id();
        renumber_children(doc, c);
    }
}

/// Whether the source has fill or stroke paint for an attribute sample.
pub(crate) fn has_sampleable_paint(cx: &ToolContext, id: vectorcraft_doc::NodeId) -> bool {
    cx.doc.node(id).is_some_and(|n| [false, true].into_iter().any(|stroke| n.proxy_paint(stroke, None).is_some_and(|(paint, ..)| !paint.is_none())))
}

/// The colour of `n`'s fill (or its stroke, when the stroke was hit or there is no fill) at `p`,
/// sampled in the paint's own space (text space for type runs) and box.
fn paint_sample(n: &Node, on_stroke: bool, p: Point) -> Option<Color> {
    let fill_none = n.proxy_paint(false, None).is_none_or(|(p, ..)| p.is_none());
    let (paint, to_doc, bounds) = n.proxy_paint(on_stroke || fill_none, None)?;
    let p = if to_doc.determinant().abs() > 1e-12 { to_doc.inverse() * p } else { p };
    paint_color_at(paint, Some(bounds), p)
}

/// The color of image `n`'s pixels (in `doc`) at `p`, averaged over the raster sample size: a CMYK
/// image's ink amounts as they are, other images' sRGB pixels in the working RGB space (the color
/// that shows as that pixel).
fn image_color_at(cx: &ToolContext, doc: &Document, n: &Node, p: Point) -> Option<Color> {
    let NodeKind::Image(im) = &n.kind else { return None };
    if im.xf.determinant().abs() < 1e-12 {
        return None;
    }
    let px = im.xf.inverse() * p;
    let blob = doc.images.get(&im.key)?;
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
            let Some(c) = sample_color_at(cx, ev.pos) else { return vec![] };
            let back = self.stop.clone().unwrap_or_else(|| "selection".to_string());
            self.brush_key_color = false;
            self.stop = None;
            return vec![Action::Dialog("brushKeyColor".into(), json!({ "color": color_json(&c) })), Action::SwitchTool(back)];
        }
        let stop = self.stop.as_ref().zip(cx.gradient_stop);
        // Reading a source is independent of its editability. Only Alt's reverse operation
        // writes to the clicked object; Shift+Alt appends its appearance to the selection.
        let source = if ev.mods.alt && !ev.mods.shift && stop.is_none() {
            hit_test(cx.doc, ev.pos, opts).map(|h| (paint_owner(cx.doc, h.leaf), h.kind))
        } else {
            sample_source(cx, ev.pos)
        };
        let Some((src, _)) = source else { return vec![] };
        let Some(n) = cx.doc.node(src) else { return vec![] };
        if ev.mods.alt && stop.is_none() {
            let how = if ev.mods.shift { "append" } else { "reverse" };
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0, how: true }))];
        }
        let image = matches!(n.kind, NodeKind::Image(_));
        if !(ev.mods.shift || stop.is_some() || image) {
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0 }))];
        }
        let Some(c) = sample_color_at(cx, ev.pos) else { return vec![] };
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

    /// An unstroked rectangle of `d` filled with `color`.
    fn solid_rect(d: &mut vectorcraft_doc::Document, r: vectorcraft_geom::Rect, color: Color) -> Node {
        let a = vectorcraft_doc::Appearance::basic(Paint::solid(color), Paint::None, 0.0);
        Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), a)
    }

    /// [`doc_with_rect`] plus a green rectangle (290, 290)–(430, 350) and, over it, `live` made by
    /// `make` from the document.
    fn doc_with_live_over_green(make: impl FnOnce(&mut vectorcraft_doc::Document) -> Node) -> vectorcraft_doc::Document {
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let green = solid_rect(&mut d, Rect::new(290.0, 290.0, 430.0, 350.0), Color::rgb(0.0, 1.0, 0.0));
        d.insert(Some(l), usize::MAX, green).unwrap();
        let live = make(&mut d);
        d.insert(Some(l), usize::MAX, live).unwrap();
        d
    }

    /// Shift-click on a blend samples the step drawn under the cursor, and a gap between its steps
    /// samples what lies below.
    #[test]
    fn shift_click_samples_the_blend_step_under_the_cursor() {
        use vectorcraft_doc::live::{BlendSpacing, BlendSpec};
        use vectorcraft_geom::Rect;
        // Red (300, 300)–(320, 320) and blue (400, 300)–(420, 320) with 1 step, half red and half
        // blue, at x 350 to 370.
        let d = doc_with_live_over_green(|d| {
            let red = solid_rect(d, Rect::new(300.0, 300.0, 320.0, 320.0), Color::rgb(1.0, 0.0, 0.0));
            let blue = solid_rect(d, Rect::new(400.0, 300.0, 420.0, 320.0), Color::rgb(0.0, 0.0, 1.0));
            let spec = BlendSpec { spacing: BlendSpacing::Steps(1), ..Default::default() };
            Node::new(d.alloc_id(), NodeKind::Blend { children: vec![std::sync::Arc::new(red), std::sync::Arc::new(blue)], spec: Box::new(spec) })
        });
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |x: f64| EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, x, 310.0).with_mods(shift));
        let mid = Color::rgb(0.5, 0.0, 0.5).to_hex();
        assert_eq!(sampled_hex(&at(360.0)), mid);
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(360.0, 310.0)).map(|c| c.to_hex()), Some(mid));
        assert_eq!(sampled_hex(&at(310.0)), "#ff0000");
        // Between the red key and the step (x 320 to 350), and below the step.
        assert_eq!(sampled_hex(&at(335.0)), "#00ff00");
        let below = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 360.0, 325.0).with_mods(shift));
        assert_eq!(sampled_hex(&below), "#00ff00");
    }

    /// Align to Path turns a blend's keys and steps to its spine, past the keys' bounds: a click
    /// there hits the blend and samples what it draws.
    #[test]
    fn shift_click_samples_blend_art_turned_past_its_bounds() {
        use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let green = solid_rect(&mut d, Rect::new(20.0, 20.0, 280.0, 280.0), Color::rgb(0.0, 1.0, 0.0));
        d.insert(Some(l), usize::MAX, green).unwrap();
        // 100 × 10 keys centred on (100, 100) and (200, 200): Align to Path turns them 45°, and the
        // red one reaches (70, 66), above the keys' bounds (y 95 to 205).
        let red = solid_rect(&mut d, Rect::new(50.0, 95.0, 150.0, 105.0), Color::rgb(1.0, 0.0, 0.0));
        let blue = solid_rect(&mut d, Rect::new(150.0, 195.0, 250.0, 205.0), Color::rgb(0.0, 0.0, 1.0));
        let spec = BlendSpec { spacing: BlendSpacing::Steps(1), orientation: BlendOrientation::AlignToPath, ..Default::default() };
        let id = d.alloc_id();
        let blend = Node::new(id, NodeKind::Blend { children: vec![std::sync::Arc::new(red), std::sync::Arc::new(blue)], spec: Box::new(spec) });
        d.insert(Some(l), usize::MAX, blend).unwrap();
        let (s, p) = (Selection::default(), paint());
        let at = Point::new(70.0, 66.0);
        assert!(d.node(id).unwrap().geometric_bounds().is_some_and(|b| !b.contains(at)));
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(id));
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), at).map(|c| c.to_hex()), Some("#ff0000".into()));
        // Where the turned key leaves the point empty, the click passes the blend by.
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(60.0, 90.0)).map(|c| c.to_hex()), Some("#00ff00".into()));
        // A hidden red key does not hit there.
        if let Some(NodeKind::Blend { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            std::sync::Arc::make_mut(&mut children[0]).visible = false;
        }
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(d.layers[0].children().unwrap()[1].id));
        // A red key clipped to its right half, centred on (125, 100): the line to the blue key's
        // centre turns it about 53°, and its hidden left half reaches (89, 52).
        let mut clip = solid_rect(&mut d, Rect::new(100.0, 95.0, 150.0, 105.0), Color::BLACK);
        clip.appearance = Default::default();
        if let NodeKind::Path { clipping, .. } = &mut clip.kind {
            *clipping = true;
        }
        let red = solid_rect(&mut d, Rect::new(50.0, 95.0, 150.0, 105.0), Color::rgb(1.0, 0.0, 0.0));
        let key = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), std::sync::Arc::new(red)], clip: true });
        if let Some(NodeKind::Blend { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            children[0] = std::sync::Arc::new(key);
        }
        let hidden = Point::new(89.0, 52.0);
        assert!(d.node(id).unwrap().geometric_bounds().is_some_and(|b| !b.contains(hidden)));
        assert_eq!(hit_test(&d, hidden, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(d.layers[0].children().unwrap()[1].id));
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), hidden).map(|c| c.to_hex()), Some("#00ff00".into()));
    }

    /// The stroke of an envelope's distorted contents paints past the envelope's bounds: a click
    /// there hits the envelope and samples the stroke.
    #[test]
    fn shift_click_samples_an_envelopes_stroke_past_its_bounds() {
        use vectorcraft_geom::{Affine, Rect};
        let mut id = None;
        let d = doc_with_live_over_green(|d| {
            let a = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 10.0);
            let red = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 420.0, 320.0)), a);
            let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true };
            let content = vec![std::sync::Arc::new(red)];
            let n = Node::new(
                d.alloc_id(),
                NodeKind::Envelope {
                    content,
                    kind: Box::new(kind),
                    fidelity: 50.0,
                    editing: false,
                    options: Default::default(),
                    frame: Box::new(Affine::IDENTITY),
                },
            );
            id = Some(n.id);
            n
        });
        let (s, p) = (Selection::default(), paint());
        // The middle of the band's top edge stays at y 300, and its 10 pt stroke reaches y 295.
        let at = Point::new(360.0, 296.0);
        assert!(d.node(id.unwrap()).unwrap().geometric_bounds().is_some_and(|b| !b.inflate(3.0, 3.0).contains(at)));
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), id);
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), at).map(|c| c.to_hex()), Some("#0000ff".into()));
    }

    /// Shift-click on an envelope samples its distorted contents where they are drawn, and a point
    /// in its bounds that they leave empty samples what lies below.
    #[test]
    fn shift_click_samples_the_envelopes_distorted_contents() {
        use vectorcraft_geom::{Affine, Rect};
        let d = doc_with_live_over_green(|d| {
            let red = solid_rect(d, Rect::new(300.0, 300.0, 420.0, 320.0), Color::rgb(1.0, 0.0, 0.0));
            let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true };
            let content = vec![std::sync::Arc::new(red)];
            let options = Default::default();
            Node::new(
                d.alloc_id(),
                NodeKind::Envelope { content, kind: Box::new(kind), fidelity: 50.0, editing: false, options, frame: Box::new(Affine::IDENTITY) },
            )
        });
        let (s, p) = (Selection::default(), paint());
        let sample = |x: f64, y: f64| crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(x, y)).map(|c| c.to_hex());
        let art = vectorcraft_effects::expand_live_deep(Some(&d), d.layers[0].children().unwrap().last().unwrap());
        let b = art.geometric_bounds().unwrap();
        // The arc bends the ends of the band down, below the 20 pt it was, and keeps its middle.
        assert!(b.y1 > 330.0, "{b:?}");
        assert_eq!(sample(360.0, 310.0), Some("#ff0000".into()));
        assert_eq!(sample(305.0, 332.0), Some("#ff0000".into()));
        assert_eq!(sample(360.0, 332.0), Some("#00ff00".into()));
        assert_eq!(sample(b.x0 + 0.5, b.y0 + 1.0), Some("#00ff00".into()));
    }
}
