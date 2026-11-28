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

use craft::theme::Srgba;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::pickers::Select;
use craft::ui::primitives::Revision;
use egui::{Color32, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::{CharClass, CharStyle, CompositeClass, CompositeFont, FontHeightReference, TextObject};
use vectorcraft_geom::kurbo::BezPath;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_text::FontDb;

/// The text the sample sets: two Kanji, two kana, two punctuation marks, a symbol, Roman letters
/// with an ascender and a descender, and two numerals, then the first character of each custom set.
pub(super) const TEXT: &str = "永国あア、。※Agh12";

/// The Zoom menu's magnifications, in percent.
pub(super) const ZOOMS: [u64; 7] = [25, 50, 75, 100, 200, 400, 800];

/// The size the sample is set at with Zoom at 100%, in points.
const SIZE: f64 = 36.0;

/// How tall the sample is rendered, in points. `Picture` fits the image into its token-sized rect at this aspect.
const HEIGHT: f32 = 150.0;

/// How far the sample text starts from the left edge, in points.
const INSET: f64 = 12.0;

/// How wide the sample is rendered, in points.
const WIDTH: f32 = 720.0;

/// The key the sample's picture is stored under; its revision says when the image changed.
const PICTURE_KEY: u64 = 0;

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

    /// The sample field that holds whether the mark shows.
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
pub(super) fn defaults(fields: &mut Map<String, Value>) {
    fields.insert("sample".into(), json!(false));
    fields.insert("zoom".into(), json!(100));
    for m in Mark::ALL {
        fields.insert(m.field().into(), json!(m.shown_at_first()));
    }
}

/// Whether `field` is one of Show Sample's.
pub(super) fn is_field(field: &str) -> bool {
    field == "sample" || field == "zoom" || Mark::ALL.iter().any(|m| m.field() == field)
}

/// Why Show Sample's `field` can't take `value`: `sample` and the marks take true or false, and
/// `zoom` one of [`ZOOMS`].
pub(super) fn check_field(field: &str, value: &Value) -> Result<(), String> {
    if field == "zoom" {
        if !value.as_u64().is_some_and(|z| ZOOMS.contains(&z)) {
            return Err(format!("`{field}` must be one of {}", ZOOMS.map(|z| z.to_string()).join(", ")));
        }
    } else if !value.is_boolean() {
        return Err(format!("`{field}` must be true or false"));
    }
    Ok(())
}

/// The Zoom `fields` hold, 100 when it holds none of [`ZOOMS`].
fn zoom(fields: &Map<String, Value>) -> u64 {
    fields.get("zoom").and_then(Value::as_u64).filter(|z| ZOOMS.contains(z)).unwrap_or(100)
}

/// Whether Show Sample is on.
pub(super) fn shown(fields: &Map<String, Value>) -> bool {
    on(fields, "sample")
}

/// Whether the sample field `field` is on.
fn on(fields: &Map<String, Value>, field: &str) -> bool {
    fields.get(field).and_then(Value::as_bool).unwrap_or(false)
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

thread_local! {
    static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
}

/// Show Sample's area under the table: the sample of `f` with the marks `fields` show, then a swatch
/// and a toggle for each mark and Zoom beside it. Zoom writes live proposals into `fields`, restores
/// its opening percentage on cancellation and preserves a later automation edit.
pub(super) fn show(ui: &mut Ui, fields: &mut Map<String, Value>, f: &CompositeFont) {
    let zoom_now = zoom(fields);
    let marks: Vec<Mark> = Mark::ALL.into_iter().filter(|m| on(fields, m.field())).collect();
    let ppp = ui.ctx().pixels_per_point();
    let ink = ui.visuals().text_color();
    // The image changes with everything it is drawn from; the picture keeps its texture while the revision holds.
    let revision = Revision::of(&(
        serde_json::to_string(f).unwrap_or_default(),
        zoom_now,
        ppp.to_bits(),
        ink.to_array(),
        FontDb::global().generation(),
        marks.iter().map(|m| m.field()).collect::<Vec<_>>(),
    ));
    let render = || image(f, zoom_now, &marks, ppp, ink);
    let entries: Vec<Entry<u64>> = ZOOMS.iter().map(|z| Entry::new(*z, format!("{z}%"))).collect();
    Row::new().show(ui, |ui, _| {
        Picture::new(tl!("Sample"), PICTURE_KEY, &render)
            .revision(revision)
            .aspect(ThumbAspect::new(WIDTH as u16, HEIGHT as u16))
            .extent(PictureSize::Sample(ThumbSize::Large))
            .show(ui);
        Stack::new().show(ui, |ui, _| {
            for m in Mark::ALL {
                Row::new().id_salt(m.field()).show(ui, |ui, _| {
                    Swatch::new(m.label(), Srgba::from(m.color())).show(ui);
                    let toggled = ToggleButton::new(m.label(), Bind::given(on(fields, m.field()))).show(ui);
                    if let Some(now) = toggled.value().copied() {
                        fields.insert(m.field().into(), json!(now));
                    }
                });
            }
            Row::new().id_salt("zoom").show(ui, |ui, _| {
                Text::new(tl!("Zoom")).show(ui);
                let mut chosen_zoom = zoom_now;
                Select::new(tl!("Zoom"), Bind::bound(&mut chosen_zoom), &entries).show(ui);
                if chosen_zoom != zoom_now {
                    fields.insert("zoom".into(), json!(chosen_zoom));
                }
            });
        });
    });
}

/// Where the baseline goes, in points down from the top of an image `height` points high: the em
/// box's middle (else 0.38 em above the baseline) at the middle of the image.
fn baseline(h: &Heights, size: f64, height: f32) -> f64 {
    let mid = h.em_box.map_or(size * 0.38, |(bottom, top)| (bottom + top) * 0.5);
    f64::from(height) * 0.5 + mid
}

/// The sample of `f` at `zoom` percent, [`WIDTH`] by [`HEIGHT`] points at `ppp` pixels per point, its
/// em box centered, with the lines of `marks` drawn into the pixels at the heights [`measure`] gives.
fn image(f: &CompositeFont, zoom: u64, marks: &[Mark], ppp: f32, ink: Color32) -> egui::ColorImage {
    let size = SIZE * zoom as f64 / 100.0;
    let (_, heights) = measure(f, size);
    let base = baseline(&heights, size, HEIGHT);
    let pixels = |points: f32| (points * ppp).round().clamp(1.0, f32::from(u16::MAX)) as u16;
    let mut rendered = draw(f, size, base, (pixels(WIDTH), pixels(HEIGHT)), ppp, ink);
    for m in marks {
        for y in heights.of(*m) {
            mark_line(&mut rendered, ((base - y) as f32 * ppp).round(), ppp.ceil().max(1.0) as usize, m.color());
        }
    }
    crate::canvas::color_image_owned(rendered)
}

/// Draws a horizontal line `thickness` pixels thick from row `y` across `image`; a row outside it draws nothing.
fn mark_line(image: &mut vectorcraft_render::Rendered, y: f32, thickness: usize, color: Color32) {
    if !y.is_finite() || y < 0.0 {
        return;
    }
    let (width, height) = (image.width as usize, image.height as usize);
    let first = y as usize;
    for row in first..first.saturating_add(thickness).min(height) {
        let start = row.saturating_mul(width).saturating_mul(4);
        if let Some(line) = image.pixels.get_mut(start..start.saturating_add(width.saturating_mul(4))) {
            for pixel in line.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&color.to_array());
            }
        }
    }
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

#[cfg(test)]
mod zoom_tests {
    use craft::ui::contract::Role;
    use craft::ui::dialog::dialog_id;
    use egui::{Key, Modifiers, vec2};
    use vectorcraft_engine::Session;

    use super::super::{KIND, open, with_dialog};
    use super::*;
    use crate::VectorcraftApp;
    use crate::tests::panel_harness::Panel;

    fn opened(width: f32) -> Panel {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let mut panel = Panel::new(app, crate::tests::picks::dialogs);
        panel.room(vec2(width, 1000.0));
        open(&mut panel.app()).unwrap();
        with_dialog(&panel.app(), |dialog| {
            dialog.fonts = vec![CompositeFont::new("Sample draft", "Source Sans 3", "Regular")];
            dialog.select(0);
        })
        .unwrap();
        panel.frame();
        panel.click("Show Sample");
        panel
    }

    fn fields(panel: &Panel) -> Map<String, Value> {
        with_dialog(&panel.app(), |dialog| dialog.sample.clone()).expect("the Composite Fonts dialog remains open")
    }

    #[test]
    fn composite_sample_zoom_escape_restores_the_opening_value() {
        for width in [1024.0, 1280.0] {
            let mut panel = opened(width);
            let before = fields(&panel);
            let (document, undo, revision) = {
                let app = panel.app();
                let st = app.session.active().unwrap();
                (st.doc.clone(), st.history.undo.len(), st.revision)
            };
            assert_eq!(panel.value_role("Zoom", Role::COMBO_BOX).as_deref(), Some("100%"));
            panel.click_role("Zoom", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            assert_ne!(fields(&panel)["zoom"], before["zoom"], "the open popup previews its proposed magnification");
            panel.key(Key::Escape, Modifiers::NONE);
            assert_eq!(fields(&panel), before, "Escape restores the complete sample draft while keeping the parent dialog open");
            assert_eq!(panel.value_role("Zoom", Role::COMBO_BOX).as_deref(), Some("100%"));
            let app = panel.app();
            let st = app.session.active().unwrap();
            assert_eq!(st.doc, document);
            assert_eq!(st.history.undo.len(), undo);
            assert_eq!(st.revision, revision);
        }
    }

    #[test]
    fn composite_sample_zoom_accepts_pointer_and_keyboard_choices() {
        let mut panel = opened(1280.0);
        panel.click_role("Zoom", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.key(Key::Enter, Modifiers::NONE);
        assert_eq!(fields(&panel)["zoom"], json!(200));
        panel.click_role("Zoom", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.key(Key::Escape, Modifiers::NONE);
        assert_eq!(fields(&panel)["zoom"], json!(200), "a later cancellation restores the last accepted choice");
        panel.click_role("Zoom", Role::COMBO_BOX);
        panel.click("400%");
        assert_eq!(fields(&panel)["zoom"], json!(400));
        assert_eq!(panel.value_role("Zoom", Role::COMBO_BOX).as_deref(), Some("400%"));
    }

    #[test]
    fn composite_sample_zoom_preserves_an_automation_edit_during_a_popup() {
        let mut panel = opened(1280.0);
        panel.click_role("Zoom", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(fields(&panel)["zoom"], json!(200));
        {
            let app = panel.app();
            let ctx = app.ui_ctx.clone().unwrap();
            app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, dialog_id(KIND), "zoom", json!(400))).unwrap();
        }
        panel.key(Key::Escape, Modifiers::NONE);
        assert_eq!(fields(&panel)["zoom"], json!(400), "canceling an older popup must not undo the later automation edit");
        assert_eq!(panel.value_role("Zoom", Role::COMBO_BOX).as_deref(), Some("400%"));
    }
}
