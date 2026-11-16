//! Show Sample in Composite Fonts: the font picked set in a line of sample text, with lines that
//! mark its ICF box, em box, baseline, cap height, highest ascent and lowest descent, highest
//! ascender and x-height, each shown or hidden by its button, at the size Zoom picks.
//!
//! The sample text, the zoom steps, the size at 100% and what each line measures are VectorCraft's
//! own: the em box and ICF box are those of the face of the part that sets the sample's first Kanji
//! (the Kanji part, or a custom set that holds that character)
//! ([`vectorcraft_text::FontFace::ideographic_centre`], [`vectorcraft_text::FontFace::icf_margins`]),
//! the cap height and x-height those of the face the Roman letters are drawn in, Max Ascent/Descent
//! the highest ascent and lowest descent among the faces the sample is drawn in (fallback faces
//! included), and Max Ascender the top of the sample's tallest glyph. Each takes its part's size,
//! baseline and vertical scale.
//!
//! Behavior reference (public documentation):
//! <https://help.adobe.com/archive/en_US/illustrator/cs4/illustrator_cs4_help.pdf#page=352>,
//! <https://web.archive.org/web/20220911143322/https://helpx.adobe.com/pdf/cs6/illustrator_reference.pdf#page=348>.

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Color32, Rect, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{CharClass, CharStyle, CompositeClass, CompositeFont, FontHeightReference, TextObject};
use vectorcraft_geom::kurbo::BezPath;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_text::FontDb;

use crate::state::Dialog;
use crate::widgets;

/// The text the sample sets: two Kanji, two kana, two punctuation marks, a symbol, Roman letters
/// with an ascender and a descender, and two numerals, then the first character of each custom set.
pub(super) const TEXT: &str = "永国あア、。※Agh12";

/// The Zoom menu's magnifications, in percent.
pub(super) const ZOOMS: [u64; 7] = [25, 50, 75, 100, 200, 400, 800];

/// The size the sample is set at with Zoom at 100%, in points.
const SIZE: f64 = 36.0;

/// How tall the sample is drawn, in points.
const HEIGHT: f32 = 150.0;

/// How far the sample text starts from the left edge, in points.
const INSET: f64 = 12.0;

/// The width of the column of line buttons and Zoom beside the sample, in points.
const BUTTONS: f32 = 190.0;

/// The narrowest and widest the sample is drawn, in points.
const WIDTH: std::ops::RangeInclusive<f32> = 240.0..=720.0;

/// The name the sample lends the font under, so a name that is empty, being typed or shared with
/// another font still draws.
const NAME: &str = "VectorCraft Composite Sample";

/// A line Show Sample can mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    IcfBox,
    EmBox,
    Baseline,
    CapHeight,
    AscentDescent,
    Ascender,
    XHeight,
}

impl Mark {
    /// The marks, in the order their buttons show.
    pub(super) const ALL: [Self; 7] =
        [Self::IcfBox, Self::EmBox, Self::Baseline, Self::CapHeight, Self::AscentDescent, Self::Ascender, Self::XHeight];

    /// The dialog field that holds whether the mark shows.
    pub(super) fn field(self) -> &'static str {
        match self {
            Self::IcfBox => "lineIcfBox",
            Self::EmBox => "lineEmBox",
            Self::Baseline => "lineBaseline",
            Self::CapHeight => "lineCapHeight",
            Self::AscentDescent => "lineAscentDescent",
            Self::Ascender => "lineAscender",
            Self::XHeight => "lineXHeight",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::IcfBox => tl!("ICF Box"),
            Self::EmBox => tl!("Em Box"),
            Self::Baseline => tl!("Baseline"),
            Self::CapHeight => tl!("Cap Height"),
            Self::AscentDescent => tl!("Max Ascent/Descent"),
            Self::Ascender => tl!("Max Ascender"),
            Self::XHeight => tl!("x-height"),
        }
    }

    /// Shown when the dialog opens: the em box, the ICF box and the baseline.
    pub(super) fn shown_at_first(self) -> bool {
        matches!(self, Self::IcfBox | Self::EmBox | Self::Baseline)
    }

    fn color(self) -> Color32 {
        match self {
            Self::IcfBox => Color32::from_rgb(0, 150, 170),
            Self::EmBox => Color32::from_rgb(220, 60, 60),
            Self::Baseline => Color32::from_rgb(40, 110, 230),
            Self::CapHeight => Color32::from_rgb(40, 160, 70),
            Self::AscentDescent => Color32::from_rgb(230, 140, 30),
            Self::Ascender => Color32::from_rgb(150, 80, 200),
            Self::XHeight => Color32::from_rgb(200, 60, 160),
        }
    }
}

/// The fields Show Sample adds to the dialog when it opens: hidden, at 100%, with the marks of
/// [`Mark::shown_at_first`].
pub(super) fn defaults(fields: &mut serde_json::Map<String, Value>) {
    fields.insert("sample".into(), json!(false));
    fields.insert("zoom".into(), json!(100));
    for m in Mark::ALL {
        fields.insert(m.field().into(), json!(m.shown_at_first()));
    }
}

/// Why `ui.dialog.set` can't set Show Sample's `field` to the value `d` now holds: `sample` and the
/// marks take true or false, and `zoom` one of [`ZOOMS`].
pub(super) fn check_field(d: &Dialog, field: &str) -> Result<(), String> {
    let v = d.fields.get(field);
    let key = field;
    if field == "sample" || Mark::ALL.iter().any(|m| m.field() == field) {
        if !v.is_some_and(Value::is_boolean) {
            return Err(format!("`{key}` must be true or false"));
        }
    } else if field == "zoom" && !v.and_then(Value::as_u64).is_some_and(|z| ZOOMS.contains(&z)) {
        return Err(format!("`{key}` must be one of {}", ZOOMS.map(|z| z.to_string()).join(", ")));
    }
    Ok(())
}

/// The Zoom `d` holds, 100 when it holds none of [`ZOOMS`].
fn zoom(d: &Dialog) -> u64 {
    d.fields.get("zoom").and_then(Value::as_u64).filter(|z| ZOOMS.contains(z)).unwrap_or(100)
}

/// The heights the marks are drawn at, in points above the baseline (a mark's line is not drawn
/// when its height is `None`). The baseline is at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Heights {
    /// The em box's bottom and top.
    pub em_box: Option<(f64, f64)>,
    /// The ICF box's bottom and top.
    pub icf_box: Option<(f64, f64)>,
    pub cap_height: Option<f64>,
    pub x_height: Option<f64>,
    /// The lowest descent and the highest ascent.
    pub ascent_descent: Option<(f64, f64)>,
    /// The top of the tallest glyph.
    pub ascender: Option<f64>,
}

impl Heights {
    /// The heights of mark `m`.
    fn of(&self, m: Mark) -> Vec<f64> {
        let pair = |p: Option<(f64, f64)>| p.map_or_else(Vec::new, |(a, b)| vec![a, b]);
        match m {
            Mark::IcfBox => pair(self.icf_box),
            Mark::EmBox => pair(self.em_box),
            Mark::Baseline => vec![0.0],
            Mark::CapHeight => self.cap_height.into_iter().collect(),
            Mark::AscentDescent => pair(self.ascent_descent),
            Mark::Ascender => self.ascender.into_iter().collect(),
            Mark::XHeight => self.x_height.into_iter().collect(),
        }
    }
}

/// `f` as the sample sets it: named [`NAME`], without its custom sets when one of them can't be
/// used (a set whose name is empty or shared while it is typed).
fn sample_font(f: &CompositeFont) -> CompositeFont {
    let mut s = f.clone();
    s.name = NAME.to_string();
    if s.check().is_err() {
        s.custom.clear();
    }
    s
}

/// [`TEXT`], then the first character of each of `f`'s custom sets that it does not hold yet.
pub(super) fn text(f: &CompositeFont) -> String {
    let mut t = TEXT.to_string();
    for c in f.custom.iter().filter_map(|s| s.characters.chars().next()) {
        if !t.contains(c) {
            t.push(c);
        }
    }
    t
}

/// The outlines of the sample of `f` set at `size` points with its baseline at y 0 (y down), and
/// the heights of its marks.
pub(super) fn measure(f: &CompositeFont, size: f64) -> (BezPath, Heights) {
    let db = FontDb::global();
    let font = sample_font(f);
    let _lent = db.lend_composite_fonts(std::slice::from_ref(&font));
    let text = text(&font);
    let style = CharStyle { font_family: NAME.to_string(), size, ..Default::default() };
    let layout = vectorcraft_text::layout(db, &TextObject::point(Point::ZERO, &text, style));
    let path = layout.to_bezpath();
    let classes = font.classes();
    // The face of character `c`'s part, the face layout draws it in (a fallback face when the part's
    // font lacks it), the size of its em and the shift of its baseline (points), and its part's
    // vertical scale, as layout sets them: Kana scaled at center keep the middle of their part's em
    // box where it is.
    let placed = |c: char| {
        let (part_face, _) = db.composite_resolve(NAME, c)?;
        let drawn = text.find(c).and_then(|at| layout.glyphs.iter().find(|g| g.byte == at)).and_then(|g| db.face_by_id(g.font_id));
        let class = classes.of(c);
        let p = font.part(class).unwrap_or(&font.parts.kanji);
        let em = size * p.size / 100.0;
        let mut shift = size * p.baseline / 100.0;
        if class == CompositeClass::Fixed(CharClass::Kana) && p.scale_at_center && p.v_scale != 100.0 {
            shift += em * part_face.ideographic_centre() * (1.0 - p.v_scale / 100.0);
        }
        let drawn = drawn.unwrap_or_else(|| part_face.clone());
        Some((part_face, drawn, em, shift, p.v_scale / 100.0))
    };
    let mut h = Heights::default();
    // The em box and ICF box are those of the part that sets the sample's first Kanji (the Kanji
    // part, or a custom set that holds that character), as layout measures that part's face for
    // Scale at Center.
    if let Some((face, _, em, shift, v)) = TEXT.chars().next().and_then(placed) {
        let tall = em * v;
        let mid = shift + face.ideographic_centre() * tall;
        let (bottom, top) = (mid - tall * 0.5, mid + tall * 0.5);
        let m = face.icf_margins();
        h.em_box = Some((bottom, top));
        h.icf_box = Some((bottom + m.bottom * tall, top - m.top * tall));
    }
    if let Some((_, face, em, shift, v)) = placed('A') {
        let height = |r| face.font_height(r, false);
        let cap = height(FontHeightReference::CapHeight).unwrap_or_else(|| face.cap_height() / face.units_per_em());
        h.cap_height = Some(shift + cap * em * v);
        let x = height(FontHeightReference::XHeight).unwrap_or_else(|| face.x_height() / face.units_per_em());
        h.x_height = Some(shift + x * em * v);
    }
    let mut seen = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let Some((_, face, em, shift, v)) = placed(c) else { continue };
        if seen.contains(&(face.id(), em.to_bits(), shift.to_bits(), v.to_bits())) {
            continue;
        }
        seen.push((face.id(), em.to_bits(), shift.to_bits(), v.to_bits()));
        let (ascent, descent) = face.vertical_metrics();
        let scale = em * v / face.units_per_em();
        let (low, high) = (shift - descent * scale, shift + ascent * scale);
        h.ascent_descent = Some(h.ascent_descent.map_or((low, high), |(l, t)| (l.min(low), t.max(high))));
    }
    // Outlines, color glyphs and bitmap glyphs alike.
    h.ascender = layout.ink_bounds().filter(|ink| ink.height() > 0.0).map(|ink| -ink.y0);
    (path, h)
}

/// The sample drawn for a key, kept until the font, zoom, size, display scale, text color or the
/// fonts installed change.
struct Drawn {
    key: u64,
    texture: Option<egui::TextureHandle>,
    heights: Heights,
}

thread_local! {
    static DRAWN: crate::graphics::TexCache<Option<Drawn>> = crate::graphics::TexCache::default();
    static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
}

/// Show Sample's area under the table: the sample of `f` with its marks, then the mark buttons and
/// Zoom beside it.
pub(super) fn show(ui: &mut Ui, d: &mut Dialog, f: &CompositeFont) {
    let zoom = zoom(d);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - BUTTONS).clamp(*WIDTH.start(), *WIDTH.end());
        let (rect, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
        paint(ui, rect, d, f, zoom);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for m in Mark::ALL {
                let on = d.bool(m.field());
                ui.horizontal(|ui| {
                    let (swatch, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(swatch, 1.0, m.color());
                    if ui.add(egui::Button::selectable(on, m.label())).clicked() {
                        d.fields.insert(m.field().into(), json!(!on));
                    }
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Zoom"));
                let names: Vec<String> = ZOOMS.iter().map(|z| format!("{z}%")).collect();
                let options: Vec<&str> = names.iter().map(String::as_str).collect();
                let current = format!("{zoom}%");
                if let Some(k) = widgets::dropdown(ui, "composite-fonts-zoom", &current, &options, 80.0)
                    && let Some(z) = ZOOMS.get(k)
                {
                    d.fields.insert("zoom".into(), json!(z));
                }
            });
        });
    });
}

/// Draw the sample of `f` at `zoom` percent into `rect`, its em box centered in it, with the marks
/// `d` shows.
fn paint(ui: &Ui, rect: Rect, d: &Dialog, f: &CompositeFont, zoom: u64) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let ppp = ui.ctx().pixels_per_point();
    let ink = visuals.text_color();
    let mut h = DefaultHasher::new();
    serde_json::to_string(f).unwrap_or_default().hash(&mut h);
    (zoom, ppp.to_bits(), rect.width().to_bits(), rect.height().to_bits(), ink.to_array(), FontDb::global().generation()).hash(&mut h);
    let key = h.finish();
    let size = SIZE * zoom as f64 / 100.0;
    let (texture, heights) = DRAWN.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(drawn) = cache.as_ref().filter(|c| c.key == key) {
            return (drawn.texture.clone(), drawn.heights);
        }
        let (_, heights) = measure(f, size);
        let texture = render(ui.ctx(), f, size, baseline(&heights, size, rect), rect, ppp, ink);
        *cache = Some(Drawn { key, texture: texture.clone(), heights });
        (texture, heights)
    });
    if let Some(t) = &texture {
        painter.image(t.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    let base = rect.top() + baseline(&heights, size, rect) as f32;
    for m in Mark::ALL.into_iter().filter(|m| d.bool(m.field())) {
        for y in heights.of(m) {
            painter.hline(rect.x_range(), base - y as f32, Stroke::new(1.0, m.color()));
        }
    }
    painter.rect_stroke(rect, 2.0, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
}

/// Where the baseline goes, in points down from the top of `rect`: the em box's middle (else
/// 0.38 em above the baseline) at the middle of `rect`.
fn baseline(h: &Heights, size: f64, rect: Rect) -> f64 {
    let mid = h.em_box.map_or(size * 0.38, |(bottom, top)| (bottom + top) * 0.5);
    f64::from(rect.height()) * 0.5 + mid
}

/// The sample of `f` drawn into a texture the size of `rect` in pixels ([`draw`]). `None` when
/// `rect` is empty or larger than the largest texture.
fn render(ctx: &egui::Context, f: &CompositeFont, size: f64, baseline: f64, rect: Rect, ppp: f32, ink: Color32) -> Option<egui::TextureHandle> {
    let (w, h) = ((rect.width() * ppp).round(), (rect.height() * ppp).round());
    let max = ctx.input(|i| i.max_texture_side) as f32;
    if w < 1.0 || h < 1.0 || w > max || h > max || w > f32::from(u16::MAX) || h > f32::from(u16::MAX) {
        return None;
    }
    let image = draw(f, size, baseline, (w as u16, h as u16), ppp, ink);
    Some(ctx.load_texture("composite-font-sample", crate::canvas::color_image_owned(image), egui::TextureOptions::LINEAR))
}

/// The sample of `f` set at `size` points in `ink`, its baseline `baseline` points down from the
/// top, drawn by the renderer as the canvas draws type (color and bitmap glyphs in their own
/// colors) into `px` pixels at `ppp` pixels per point.
pub(super) fn draw(f: &CompositeFont, size: f64, baseline: f64, px: (u16, u16), ppp: f32, ink: Color32) -> vectorcraft_render::Rendered {
    let (w, h) = px;
    let mut doc = vectorcraft_doc::Document::new(f64::from(w) / f64::from(ppp), f64::from(h) / f64::from(ppp));
    let font = sample_font(f);
    let sample = text(&font);
    *doc.composite_fonts = vec![font];
    let [r, g, b, _] = ink.to_normalized_gamma_f32();
    let fill = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(r, g, b));
    let style = CharStyle { font_family: NAME.to_string(), size, fill, ..Default::default() };
    let t = TextObject::point(Point::new(INSET, baseline), &sample, style);
    let node = std::sync::Arc::new(vectorcraft_doc::Node::new(doc.alloc_id(), vectorcraft_doc::NodeKind::Text(Box::new(t))));
    let view = Affine::scale(f64::from(ppp));
    let empty = || vectorcraft_render::Rendered { width: u32::from(w), height: u32::from(h), pixels: vec![0; usize::from(w) * usize::from(h) * 4] };
    RENDERER.with(|r| r.borrow_mut().render_node(&doc, &node, w, h, view)).unwrap_or_else(empty)
}
