use super::build::Frame;
use super::slots::OUTLINE_REACH;
use super::{Arc, Document, Node, NodeKind, Rect};

/// `doc` with its layers that don't print hidden, when it has any.
fn as_printed(doc: &Document) -> Option<Document> {
    fn hide(nodes: &mut [Arc<Node>]) -> bool {
        let mut any = false;
        for n in nodes {
            if matches!(n.kind, NodeKind::Layer { printable: false, .. }) && n.visible {
                Arc::make_mut(n).visible = false;
                any = true;
            } else if matches!(n.kind, NodeKind::Layer { .. })
                && let Some(c) = Arc::make_mut(n).children_mut()
            {
                any |= hide(c);
            }
        }
        any
    }
    let mut d = doc.clone();
    hide(&mut d.layers).then_some(d)
}

/// A premultiplied pixel seen on white.
fn on_white(p: &[u8; 4]) -> [u32; 3] {
    let paper = 255 - u32::from(p[3]).min(255);
    [0, 1, 2].map(|i| (u32::from(p[i]) + paper).min(255))
}

/// The lightness (0 to 255, Rec. 601 weights) of a pixel seen on white.
fn lightness([r, g, b]: [u32; 3]) -> u32 {
    (r * 299 + g * 587 + b * 114) / 1000
}

/// How far (0 to 255) a pixel seen on white is from white: how much ink it has.
fn ink([r, g, b]: [u32; 3]) -> u32 {
    255 - r.min(g).min(b)
}

/// How far apart two pixels' lightness, or their ink where one is white, may be for them to look
/// alike.
const SAME_PIXEL: u32 = 48;

/// The most ink a pixel seen as white has.
const WHITE: u32 = 8;

/// Do two pixels look different: in lightness, or one inked (as a light yellow is) where the other
/// is white?
pub(in crate::import) fn differ(p: &[u8; 4], q: &[u8; 4]) -> bool {
    if p == q {
        return false;
    }
    let (p, q) = (on_white(p), on_white(q));
    let (a, b) = (ink(p), ink(q));
    lightness(p).abs_diff(lightness(q)) > SAME_PIXEL || (a.min(b) <= WHITE && a.abs_diff(b) > SAME_PIXEL)
}

/// Do two pixels look different seen on black: in lightness? An object drawn on its own and seen
/// only on white hides what it draws in white (white type over nothing looks like nothing): seen
/// on black too, that shows.
pub(in crate::import) fn differ_on_black(p: &[u8; 4], q: &[u8; 4]) -> bool {
    // Premultiplied, a pixel is what it shows on black.
    let on_black = |p: &[u8; 4]| [0, 1, 2].map(|i| u32::from(p[i]));
    p != q && lightness(on_black(p)).abs_diff(lightness(on_black(q))) > SAME_PIXEL
}

/// How far apart two pixels' red, green or blue may be for them to be the same colour (a colour
/// and its CMYK equivalent stay within it; a red and a dark red don't).
const SAME_COLOUR: u32 = 64;

/// Are two pixels different colours ([`differ`] also takes a red and a dark red of about the same
/// lightness to be alike)?
pub(in crate::import) fn differ_in_colour(p: &[u8; 4], q: &[u8; 4]) -> bool {
    if p == q {
        return false;
    }
    let (p, q) = (on_white(p), on_white(q));
    p.iter().zip(q).any(|(a, b)| a.abs_diff(b) > SAME_COLOUR)
}

/// How much two documents draw differently: the share (0 to 1) of the pixels that differ, of those
/// that differ away from type, and of those whose neighbors all differ too, away from type (where
/// one draws an object that the other doesn't, not the edges of one drawn a little differently, or
/// type laid out a little differently: a line the page sets in pieces is laid out afresh as one).
#[derive(Clone, Copy)]
pub(super) struct Difference {
    pub(super) any: f64,
    art: f64,
    pub(super) solid: f64,
    /// The share that differs in the type's areas (`any` less `art`), kept on its own when the
    /// shares of several regions are taken together ([`Self::max`]).
    typed: f64,
}

impl Difference {
    /// The share [`compare`] holds to [`MAX_DIFFERENCE`]: every pixel that differs, or with `reflow`
    /// (type laid out again from the file's text) those away from type.
    pub(super) fn weighed(&self, reflow: bool) -> f64 {
        if reflow { self.art } else { self.any }
    }

    /// The larger of each share of `self` and `other`: how two regions differ, the one that differs
    /// most for each.
    fn max(self, other: Self) -> Self {
        Self { any: self.any.max(other.any), art: self.art.max(other.art), solid: self.solid.max(other.solid), typed: self.typed.max(other.typed) }
    }

    /// Two drawings that can't be compared: every pixel differs.
    const ALL: Self = Self { any: 1.0, art: 1.0, solid: 1.0, typed: 0.0 };
}

/// The scale the comparisons draw a region at: about 400 pixels along its longer side.
pub(in crate::import) fn compare_scale(r: Rect) -> f64 {
    (400.0 / r.width().max(r.height()).max(1.0)).min(4.0)
}

/// A flattened print page made only of images. Sampling it directly at 400 pixels aliases
/// its fine edges while vector art receives coverage antialiasing. Compare both at a higher
/// resolution and apply the same reconstruction filter; keep the existing difference limits.
pub(super) fn comparison_samples(doc: &Document) -> u32 {
    let (mut images, mut vector) = (false, false);
    doc.walk(|n| match n.kind {
        NodeKind::Image(_) => images = true,
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Path { clipping: true, .. } | NodeKind::RulerGuide { .. } => {}
        _ => vector = true,
    });
    if images && !vector { 4 } else { 1 }
}

/// Render for the page check. The comparison scale bounds the long side to 400 px, so the
/// extra samples stay bounded to 1600 px. Filter premultiplied channels, including alpha.
pub(super) fn comparison_render(
    renderer: &mut vectorcraft_render::Renderer,
    doc: &Document,
    rect: Rect,
    scale: f64,
    samples: u32,
) -> vectorcraft_render::Rendered {
    let rendered = renderer.render_region(doc, rect, scale * f64::from(samples), false);
    if samples == 1 {
        return rendered;
    }
    let (width, height) = match vectorcraft_render::raster_size(rect, scale) {
        Ok(size) => size,
        // A malformed region cannot provide a comparable raster.
        Err(_) => return rendered,
    };
    let Some(src) = image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.pixels) else {
        return vectorcraft_render::Rendered { width: 0, height: 0, pixels: Vec::new() };
    };
    let pixels = image::imageops::resize(&src, width, height, image::imageops::FilterType::Triangle).into_raw();
    vectorcraft_render::Rendered { width, height, pixels }
}

/// The most pixels the pages of a frame are drawn in for a comparison, all of them together
/// ([`pixels_compared`]): the pages after those that reach it aren't compared. It holds about 67
/// letter-size pages drawn with one sample a pixel, and 4 drawn with 4 × 4 ([`comparison_samples`]).
pub(super) const MAX_PIXELS_COMPARED: u64 = 1 << 23;

/// The most pages of a frame compared, the first included, however few pixels they take.
pub(super) const MAX_PAGES_COMPARED: usize = 128;

/// How many pixels the comparisons draw the page `page` of the PDF part in, at [`compare_scale`]
/// with `samples` × `samples` samples a pixel ([`comparison_samples`]): `u64::MAX` for one that
/// can't be drawn.
pub(super) fn pixels_compared(page: Rect, samples: u32) -> u64 {
    let samples = u64::from(samples) * u64::from(samples);
    vectorcraft_render::raster_size(page, compare_scale(page)).map_or(u64::MAX, |(w, h)| (u64::from(w) * u64::from(h)).saturating_mul(samples))
}

/// The longest side (pixels) of a sheet ([`Sheet`]).
const MAX_SHEET_SIDE: f64 = 8192.0;

/// How many times its pages' pixels a sheet may have, the gaps between them included.
const SHEET_SPREAD: f64 = 2.0;

/// One page of the PDF part, drawn once.
struct Page {
    drawn: vectorcraft_render::Rendered,
    /// Where the page's own type is, in its pixels.
    page_type: Vec<Rect>,
    /// Where objects read with their art style lie, in its pixels: the page has Illustrator's
    /// rendering of the style there, the layers VectorCraft's live effects, and the two aren't
    /// held to look alike.
    restyled: Vec<Rect>,
    /// The column and row of its pixels that its move ([`Self::region`]) took partly off its area,
    /// where the layers can draw a neighboring artboard's art and the page the space between pages:
    /// they count as alike.
    edges: Vec<Rect>,
    /// Where the page draws anything (its type, or its own outlines of type) or has its type, in its
    /// pixels row by row.
    page_marks: Vec<bool>,
    scale: f64,
    /// The page area's width and height ([`sized`]).
    size: (f64, f64),
    /// The area of the layers' document the page is compared with: its artboard ([`sized`]), moved
    /// by up to half a pixel to start on a pixel of its sheet, the page's area with it.
    region: Rect,
}

impl Page {
    /// The area `ra` of `page`, `drawn` at `scale` ([`compare_scale`]), to be compared with the layers
    /// on `region`, both moved by (`dx`, `dy`) pixels from where the artboard and page lie.
    fn on(page: &Document, drawn: vectorcraft_render::Rendered, ra: Rect, region: Rect, scale: f64, (dx, dy): (f64, f64)) -> Self {
        let mut page_type = vec![];
        type_areas(&page.layers, ra, scale, &mut page_type);
        let (w, h) = (drawn.width as usize, drawn.height as usize);
        let typed = in_areas(&page_type, w, h);
        // A pixel with nothing drawn on it is white.
        let page_marks: Vec<bool> =
            drawn.pixels.as_chunks::<4>().0.iter().zip(&typed).map(|(p, t)| *t || (p[3] != 0 && ink(on_white(p)) > WHITE)).collect();
        let (wf, hf) = (f64::from(drawn.width), f64::from(drawn.height));
        let mut edges = vec![];
        // Moved left (or up), the first column (or row) starts before the area; moved right (or
        // down), the last one ends past it.
        match dx {
            d if d > 1e-6 => edges.push(Rect::new(0.0, 0.0, 1.0, hf)),
            d if d < -1e-6 => edges.push(Rect::new(wf - 1.0, 0.0, wf, hf)),
            _ => {}
        }
        match dy {
            d if d > 1e-6 => edges.push(Rect::new(0.0, 0.0, wf, 1.0)),
            d if d < -1e-6 => edges.push(Rect::new(0.0, hf - 1.0, wf, hf)),
            _ => {}
        }
        Self { drawn, page_type, restyled: vec![], edges, page_marks, scale, size: (ra.width(), ra.height()), region }
    }

    /// The pixels that count as alike: the restyled looks' and the edges'.
    fn skipped(&self) -> Vec<Rect> {
        self.restyled.iter().chain(&self.edges).copied().collect()
    }

    /// Where [`PageView::difference_on_every_page`] counts `b`'s (on `rb`) and the page's pixels as
    /// type, in the page's pixels. With `reflow` (`b`'s type laid out again from the file's text),
    /// type counts as type only where both have something: `b`'s where the page has type or draws
    /// anything in at least [`MARKED`] of it, the page's where at least half of it lies where `b` has
    /// a text object, shown or not, or a placed file. Elsewhere it is art the other lacks.
    fn counted_type(&self, b: &Document, rb: Rect, reflow: bool) -> Vec<Rect> {
        let mut areas = vec![];
        type_areas(&b.layers, rb, self.scale, &mut areas);
        if reflow {
            let (w, h) = (self.drawn.width as usize, self.drawn.height as usize);
            let marked = shares(&self.page_marks, w, h, &areas);
            areas = areas.into_iter().zip(marked).filter(|(_, share)| *share >= MARKED).map(|(r, _)| r).collect();
            let mut texts = vec![];
            all_type_areas(&b.layers, rb, self.scale, &mut texts);
            let on_layers = shares(&in_areas(&texts, w, h), w, h, &self.page_type);
            areas.extend(self.page_type.iter().zip(on_layers).filter(|(_, share)| *share >= 0.5).map(|(r, _)| *r));
        } else {
            areas.extend(&self.page_type);
        }
        areas
    }
}

/// `rb`, or when its sides are within half a point of `size` (a page area's, as an artboard taken for
/// its page is), the area of that size at its top-left: the layers are drawn on it, so that both
/// drawings have the same number of pixels.
fn sized((w, h): (f64, f64), rb: Rect) -> Rect {
    if (rb.width() - w).abs() <= 0.5 && (rb.height() - h).abs() <= 0.5 { Rect::new(rb.x0, rb.y0, rb.x0 + w, rb.y0 + h) } else { rb }
}

/// An area of the layers' document that holds the areas of some of the pages drawn at one scale,
/// each starting on one of its pixels: the layers are drawn on it once for all of them, and cut
/// apart.
struct Sheet {
    rect: Rect,
    scale: f64,
    /// Its pages, as [`PageView::regions`] counts them.
    pages: Vec<usize>,
    /// How many pixels its pages take.
    pixels: f64,
}

impl Sheet {
    /// The move (less than half a pixel) that makes `region` start on one of the sheet's pixels, and
    /// the sheet's area with it: `None` when the sheet would then be too large ([`MAX_SHEET_SIDE`],
    /// [`SHEET_SPREAD`]).
    fn room(&self, region: Rect) -> Option<((f64, f64), Rect)> {
        let s = self.scale;
        let off = |v: f64, from: f64| {
            let px = (v - from) * s;
            (px - px.round()) / s
        };
        let (dx, dy) = (off(region.x0, self.rect.x0), off(region.y0, self.rect.y0));
        let moved = Rect::new(region.x0 - dx, region.y0 - dy, region.x1 - dx, region.y1 - dy);
        let all = self.rect.union(moved);
        let (w, h) = (all.width() * s, all.height() * s);
        let pixels = self.pixels + moved.width() * s * moved.height() * s;
        (w.is_finite() && h.is_finite() && w.max(h) <= MAX_SHEET_SIDE && w * h <= SHEET_SPREAD * pixels).then_some(((dx, dy), all))
    }
}

/// How many documents' drawings a [`PageView`] keeps.
const KEPT_DOCUMENTS: usize = 2;

/// The layers of one document as drawn on the pages' areas, kept between comparisons.
struct Drawn {
    /// The document's layers, held so that a document changed since never shares them. Only the
    /// layers are compared: the import changes the documents it compares in their layers, and two
    /// documents with the same layers and other symbols, patterns, swatches or color mode would
    /// share their drawings.
    layers: Vec<Arc<Node>>,
    /// Each page's drawing, once drawn.
    pages: Vec<Option<Layered>>,
}

impl Drawn {
    /// Are these the drawings of `b`, for `n` pages?
    fn of(&self, b: &Document, n: usize) -> bool {
        self.pages.len() == n && self.layers.len() == b.layers.len() && self.layers.iter().zip(&b.layers).all(|(x, y)| Arc::ptr_eq(x, y))
    }
}

/// The layers drawn on one page's area, with how they differ from the page once weighed.
struct Layered {
    pixels: vectorcraft_render::Rendered,
    /// [`compared`] as [`PageView::difference_on_every_page`] weighs it, without and with `reflow`.
    laid: Option<Difference>,
    reflowed: Option<Difference>,
    /// [`colour_share`].
    colour: Option<f64>,
}

/// The pages drawn once, to weigh the layers' art against them, and the layers' drawings of the last
/// two documents weighed. The layers are drawn with a renderer of their own, which keeps their decoded
/// images between draws (the page's images can share their keys and not their pixels, so the two
/// don't share one).
pub(super) struct PageView {
    /// The frame's first page, then its other pages ([`Frame::more`]).
    pages: Vec<Page>,
    /// The areas in the layers' document that the pages are compared on, as the frame gives them.
    boards: Vec<Rect>,
    sheets: Vec<Sheet>,
    samples: u32,
    layers: vectorcraft_render::Renderer,
    /// The drawings of the last [`KEPT_DOCUMENTS`] documents weighed, the last one weighed last (the
    /// import weighs two readings of a file back and forth).
    drawn: Vec<Drawn>,
    /// The drawings of the last document [`Self::weigh_without`] weighed, with their shares.
    without: Vec<(usize, vectorcraft_render::Rendered, f64)>,
    /// How many times the layers were drawn.
    draws: usize,
}

impl PageView {
    pub(super) fn new(frame: &Frame<'_>) -> Self {
        let samples = comparison_samples(frame.page);
        // Each page's area, the area of the layers' document it is compared with, and its scale.
        let (mut planned, mut boards, mut sheets) = (vec![], vec![], Vec::<Sheet>::new());
        for (ra, rb) in std::iter::once((frame.page_rect, frame.rect)).chain(frame.more.iter().copied()) {
            let (scale, i) = (compare_scale(ra), planned.len());
            let region = sized((ra.width(), ra.height()), rb);
            let pixels = region.width() * scale * region.height() * scale;
            // A page whose drawings are resampled ([`comparison_samples`]) is drawn on its own.
            // Pages of one size can have scales a rounding error apart: they take the sheet's.
            let room = sheets.last().filter(|s| samples == 1 && (s.scale - scale).abs() <= scale * 1e-9).and_then(|s| s.room(region));
            let (dx, dy, scale) = match (room, sheets.last_mut()) {
                (Some(((dx, dy), all)), Some(sheet)) => {
                    sheet.rect = all;
                    sheet.pixels += pixels;
                    sheet.pages.push(i);
                    (dx, dy, sheet.scale)
                }
                _ => {
                    sheets.push(Sheet { rect: region, scale, pages: vec![i], pixels });
                    (0.0, 0.0, scale)
                }
            };
            let moved = |r: Rect| Rect::new(r.x0 - dx, r.y0 - dy, r.x1 - dx, r.y1 - dy);
            planned.push((moved(ra), moved(region), scale, (dx * scale, dy * scale)));
            boards.push(rb);
        }
        // Drawn on nothing and then seen on white: blending modes have no page to blend with, as in
        // the app that wrote the file.
        let mut renderer = vectorcraft_render::Renderer::new();
        let pages = planned
            .into_iter()
            .map(|(ra, region, scale, moved)| {
                Page::on(frame.page, comparison_render(&mut renderer, frame.page, ra, scale, samples), ra, region, scale, moved)
            })
            .collect();
        Self { pages, boards, sheets, samples, layers: vectorcraft_render::Renderer::new(), drawn: vec![], without: vec![], draws: 0 }
    }

    /// The areas in the layers' document that the pages are compared on: `rb` for the first page,
    /// then each other page's artboard.
    pub(super) fn regions(&self, rb: Rect) -> Vec<Rect> {
        std::iter::once(rb).chain(self.boards.iter().skip(1).copied()).collect()
    }

    /// Leave the drawn looks in `looks` (on `region`, the first page's area in the layers' document)
    /// out of the comparisons ([`Structure::restyled`](ai::Structure)), on the other pages' artboards
    /// too.
    pub(super) fn restyled(&mut self, looks: &[Rect], region: Rect) {
        for (i, page) in self.pages.iter_mut().enumerate() {
            let (s, region) = (page.scale, if i == 0 { region } else { page.region });
            page.restyled = looks
                .iter()
                .map(|r| r.inflate(OUTLINE_REACH, OUTLINE_REACH))
                .map(|r| Rect::new((r.x0 - region.x0) * s, (r.y0 - region.y0) * s, (r.x1 - region.x0) * s, (r.y1 - region.y0) * s))
                .collect();
        }
        self.drawn.clear();
    }

    /// `b`, as printed, drawn on the areas of the pages `wanted`: on their sheet when they are more
    /// than one and take at least half its pixels, else each on its own.
    fn draw(&mut self, b: &Document, wanted: &[usize]) -> Vec<(usize, vectorcraft_render::Rendered)> {
        let mut out = vec![];
        for sheet in &self.sheets {
            let on: Vec<usize> = sheet.pages.iter().copied().filter(|i| wanted.contains(i)).collect();
            let s = sheet.scale;
            let taken: f64 = on.iter().filter_map(|i| self.pages.get(*i)).map(|p| p.region.width() * s * p.region.height() * s).sum();
            // A pixel more on the right and at the bottom, for the pages' areas rounded up.
            let whole = (on.len() > 1 && taken * 2.0 >= sheet.rect.width() * s * sheet.rect.height() * s).then(|| {
                self.draws += 1;
                let r = sheet.rect;
                comparison_render(&mut self.layers, b, Rect::new(r.x0, r.y0, r.x1 + 1.0 / s, r.y1 + 1.0 / s), s, 1)
            });
            for i in on {
                let Some(page) = self.pages.get(i) else { continue };
                let r = page.region;
                let part = whole
                    .as_ref()
                    .zip(vectorcraft_render::raster_size(r, s).ok())
                    .and_then(|(whole, (w, h))| cut(whole, (r.x0 - sheet.rect.x0) * s, (r.y0 - sheet.rect.y0) * s, w, h));
                let drawn = match part {
                    Some(part) => part,
                    None => {
                        self.draws += 1;
                        comparison_render(&mut self.layers, b, r, page.scale, self.samples)
                    }
                };
                out.push((i, drawn));
            }
        }
        out
    }

    /// Have the layers of `b` drawn on the pages `wanted`, keeping those already drawn when `b` is the
    /// document last drawn.
    fn layered(&mut self, b: &Document, wanted: &[usize]) {
        let n = self.pages.len();
        match self.drawn.iter().position(|d| d.of(b, n)) {
            Some(at) => {
                let kept = self.drawn.remove(at);
                self.drawn.push(kept);
            }
            None => {
                if self.drawn.len() >= KEPT_DOCUMENTS {
                    self.drawn.remove(0);
                }
                self.drawn.push(Drawn { layers: b.layers.clone(), pages: (0..n).map(|_| None).collect() });
            }
        }
        let missing: Vec<usize> =
            wanted.iter().copied().filter(|i| self.drawn.last().and_then(|d| d.pages.get(*i)).is_some_and(Option::is_none)).collect();
        if missing.is_empty() {
            return;
        }
        // The page doesn't print what isn't printed.
        let printed = as_printed(b);
        for (i, pixels) in self.draw(printed.as_ref().unwrap_or(b), &missing) {
            if let Some(slot) = self.drawn.last_mut().and_then(|d| d.pages.get_mut(i)) {
                *slot = Some(Layered { pixels, laid: None, reflowed: None, colour: None });
            }
        }
    }

    /// How `b` draws differently from each page ([`compared`]), the first on `rb` (its frame's area,
    /// or another rectangle its size), the others on their artboards.
    pub(super) fn differences(&mut self, b: &Document, rb: Rect, reflow: bool) -> Vec<Difference> {
        let all: Vec<usize> = (0..self.pages.len()).collect();
        self.layered(b, &all);
        let printed = as_printed(b);
        let printed = printed.as_ref().unwrap_or(b);
        let mut out = vec![];
        for i in all {
            let d = if i == 0 && self.boards.first() != Some(&rb) { self.elsewhere(printed, rb, reflow) } else { self.weighed(i, printed, reflow) };
            out.push(d);
        }
        out
    }

    /// The first page compared with `b` (as printed) on `rb`, drawn for it alone.
    fn elsewhere(&mut self, b: &Document, rb: Rect, reflow: bool) -> Difference {
        let Some(page) = self.pages.first() else { return Difference::ALL };
        let rb = sized(page.size, rb);
        self.draws += 1;
        let y = comparison_render(&mut self.layers, b, rb, page.scale, self.samples);
        compared(&page.drawn, &y, &page.counted_type(b, rb, reflow), &page.skipped())
    }

    /// Page `i` compared with its drawing of `b` (as printed, already drawn), weighed once.
    fn weighed(&mut self, i: usize, b: &Document, reflow: bool) -> Difference {
        let Some((page, Some(layered))) = self.pages.get(i).zip(self.drawn.last_mut().and_then(|d| d.pages.get_mut(i))) else {
            return Difference::ALL;
        };
        let memo = if reflow { &mut layered.reflowed } else { &mut layered.laid };
        *memo.get_or_insert_with(|| compared(&page.drawn, &layered.pixels, &page.counted_type(b, page.region, reflow), &page.skipped()))
    }

    /// [`Self::differences`] taken together: each share is that of the page that differs most.
    pub(super) fn difference_on_every_page(&mut self, b: &Document, rb: Rect, reflow: bool) -> Difference {
        self.differences(b, rb, reflow).into_iter().reduce(Difference::max).unwrap_or(Difference::ALL)
    }

    /// How `b` draws differently from the pages ([`Self::difference_on_every_page`]), and the boxes
    /// (in `b`'s terms) around each patch of the pixels counted in its `solid` share (they and all
    /// their neighbors differ, away from type), neighbors touching corner to corner counting as one
    /// patch. `None` when the two drawings of a page aren't the same size.
    pub(super) fn solid_regions_on_every_page(&mut self, b: &Document, reflow: bool) -> Option<(Difference, Vec<Rect>)> {
        let all: Vec<usize> = (0..self.pages.len()).collect();
        self.layered(b, &all);
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let (mut diff, mut regions) = (Difference { any: 0.0, art: 0.0, solid: 0.0, typed: 0.0 }, vec![]);
        for (page, layered) in self.pages.iter().zip(&self.drawn.last()?.pages) {
            let y = &layered.as_ref()?.pixels;
            let (d, solid) = compared_with_solid(&page.drawn, y, &page.counted_type(b, page.region, reflow), &page.skipped());
            regions.extend(patches(solid, y.width as usize, y.height as usize, page.region, page.scale, self.samples)?);
            diff = diff.max(d);
        }
        Some((diff, regions))
    }

    /// The share (0 to 1) of the pixels where `b` on page `i`'s area draws another color than the
    /// page ([`colour_share`]).
    fn colour_difference(&mut self, i: usize, b: &Document) -> f64 {
        self.layered(b, &[i]);
        let Some((page, Some(layered))) = self.pages.get(i).zip(self.drawn.last_mut().and_then(|d| d.pages.get_mut(i))) else { return 1.0 };
        *layered.colour.get_or_insert_with(|| colour_share(&page.drawn, &layered.pixels, &page.edges))
    }

    /// `doc`'s and `without`'s shares ([`colour_share`]) on each of the pages `on`, `without` being
    /// `doc` with an object taken out that lies on those pages only. `without`'s drawings are kept
    /// for [`Self::adopt`].
    pub(super) fn weigh_without(&mut self, doc: &Document, without: &Document, on: &[usize]) -> Vec<(usize, f64, f64)> {
        self.layered(doc, on);
        let printed = as_printed(without);
        let drawn = self.draw(printed.as_ref().unwrap_or(without), on);
        let mut shares = vec![];
        self.without.clear();
        for (i, pixels) in drawn {
            let before = self.colour_difference(i, doc);
            let Some(page) = self.pages.get(i) else { continue };
            let after = colour_share(&page.drawn, &pixels, &page.edges);
            shares.push((i, before, after));
            self.without.push((i, pixels, after));
        }
        shares
    }

    /// `doc` now has the object [`Self::weigh_without`] last took out taken out: its drawings are the
    /// ones made without the object on the pages it lay on, and the ones kept on the others.
    pub(super) fn adopt(&mut self, doc: &Document) {
        let without = std::mem::take(&mut self.without);
        let Some(drawn) = self.drawn.last_mut() else { return };
        drawn.layers = doc.layers.clone();
        // The type areas on a page can reach past the object's box.
        for layered in drawn.pages.iter_mut().flatten() {
            (layered.laid, layered.reflowed) = (None, None);
        }
        for (i, pixels, colour) in without {
            if let Some(slot) = drawn.pages.get_mut(i) {
                *slot = Some(Layered { pixels, laid: None, reflowed: None, colour: Some(colour) });
            }
        }
    }
}

/// The `w` × `h` pixels of `from` from pixel (`x`, `y`): `None` unless they lie in it, from a pixel.
fn cut(from: &vectorcraft_render::Rendered, x: f64, y: f64, w: u32, h: u32) -> Option<vectorcraft_render::Rendered> {
    let (rx, ry) = (x.round(), y.round());
    if !((x - rx).abs() < 0.01 && (y - ry).abs() < 0.01 && rx >= 0.0 && ry >= 0.0) {
        return None;
    }
    let (x, y, wu, hu) = (rx as usize, ry as usize, w as usize, h as usize);
    let stride = from.width as usize * 4;
    if x + wu > from.width as usize || y + hu > from.height as usize {
        return None;
    }
    let mut pixels = Vec::with_capacity(wu * hu * 4);
    for row in y..y + hu {
        pixels.extend_from_slice(from.pixels.get(row * stride + x * 4..row * stride + (x + wu) * 4)?);
    }
    Some(vectorcraft_render::Rendered { width: w, height: h, pixels })
}

/// The boxes (in the terms of the document drawn on `rb` at `scale` with `samples` × `samples`
/// samples a pixel, [`comparison_render`]) around each patch of the `w` × `h` pixels `solid` marks,
/// neighbors touching corner to corner counting as one patch. `None` when `solid` doesn't have that
/// many pixels.
fn patches(mut solid: Vec<bool>, w: usize, h: usize, rb: Rect, scale: f64, samples: u32) -> Option<Vec<Rect>> {
    if solid.len() != w * h {
        return None;
    }
    // A pixel's size in points: [`comparison_render`] draws at `scale * samples`, rounds that
    // raster to whole pixels and resizes it to `w` × `h`, so the two sizes can round differently.
    let k = f64::from(samples);
    let drawn = vectorcraft_render::raster_size(rb, scale * k).map_or((w as f64, h as f64), |(dw, dh)| (f64::from(dw), f64::from(dh)));
    let pitch = (drawn.0 / (w as f64 * k * scale), drawn.1 / (h as f64 * k * scale));
    let mut regions = vec![];
    let mut stack = vec![];
    for start in 0..solid.len() {
        if !solid.get(start).copied().unwrap_or(false) {
            continue;
        }
        // The patch's pixels are cleared as they are taken, so each is taken once.
        if let Some(s) = solid.get_mut(start) {
            *s = false;
        }
        stack.push(start);
        let (mut i0, mut j0, mut i1, mut j1) = (start % w, start / w, start % w, start / w);
        while let Some(k) = stack.pop() {
            let (i, j) = (k % w, k / w);
            (i0, j0, i1, j1) = (i0.min(i), j0.min(j), i1.max(i), j1.max(j));
            for n in j.saturating_sub(1)..=(j + 1).min(h - 1) {
                for m in i.saturating_sub(1)..=(i + 1).min(w - 1) {
                    if let Some(s) = solid.get_mut(n * w + m).filter(|s| **s) {
                        *s = false;
                        stack.push(n * w + m);
                    }
                }
            }
        }
        let at = |i: usize, j: usize| (rb.x0 + i as f64 * pitch.0, rb.y0 + j as f64 * pitch.1);
        let ((x0, y0), (x1, y1)) = (at(i0, j0), at(i1 + 1, j1 + 1));
        regions.push(Rect::new(x0, y0, x1, y1));
    }
    Some(regions)
}

/// The share (0 to 1) of the pixels where `y` draws another color than the page `x`
/// ([`differ_in_colour`]), type included and pixels in `skip` counting as alike: what taking an
/// object out of the layers changes there, when it is covered by one of about the same lightness
/// (#1102).
fn colour_share(x: &vectorcraft_render::Rendered, y: &vectorcraft_render::Rendered, skip: &[Rect]) -> f64 {
    if (y.width, y.height) != (x.width, x.height) {
        return 1.0;
    }
    let skipped = in_areas(skip, x.width as usize, x.height as usize);
    let differing = x
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(y.pixels.as_chunks::<4>().0)
        .zip(&skipped)
        .filter(|((p, q), s)| !**s && differ_in_colour(p, q))
        .count();
    differing as f64 / (f64::from(y.width) * f64::from(y.height)).max(1.0)
}

/// How `a` on `ra` and `b` on `rb` (rectangles of the same size) draw differently ([`compared`]),
/// for a region other than the page's ([`PageView`] draws that one once).
pub(super) fn difference(a: &Document, ra: Rect, b: &Document, rb: Rect) -> Difference {
    let printed = as_printed(b);
    let b = printed.as_ref().unwrap_or(b);
    let scale = compare_scale(ra);
    // A renderer each: the two documents' images can share their keys and not their pixels.
    let samples = comparison_samples(a);
    let x = comparison_render(&mut vectorcraft_render::Renderer::new(), a, ra, scale, samples);
    let y = comparison_render(&mut vectorcraft_render::Renderer::new(), b, rb, scale, samples);
    let mut areas = vec![];
    type_areas(&a.layers, ra, scale, &mut areas);
    type_areas(&b.layers, rb, scale, &mut areas);
    compared(&x, &y, &areas, &[])
}

/// How two drawings of the same size differ, each drawn on nothing and seen on white, away from
/// the type in `areas` (pixels), pixels in `skip` counting as alike. Pixels are compared by lightness, and by whether they are inked
/// at all ([`differ`]): the page of an RGB document is written in the colours' CMYK equivalents,
/// and what the comparison looks for is art that is missing or out of place.
pub(super) fn compared(x: &vectorcraft_render::Rendered, y: &vectorcraft_render::Rendered, areas: &[Rect], skip: &[Rect]) -> Difference {
    compared_with_solid(x, y, areas, skip).0
}

/// [`compared`], and which of the pixels (row by row) are counted in its `solid` share: none when
/// the drawings can't be compared.
fn compared_with_solid(x: &vectorcraft_render::Rendered, y: &vectorcraft_render::Rendered, areas: &[Rect], skip: &[Rect]) -> (Difference, Vec<bool>) {
    if x.width == 0 || x.height == 0 || x.width != y.width || x.height != y.height {
        return (Difference { any: 1.0, art: 1.0, solid: 1.0, typed: 0.0 }, vec![]);
    }
    let (w, h) = (x.width as usize, x.height as usize);
    // Pixels in `skip` count as alike.
    let skipped = in_areas(skip, w, h);
    let differs: Vec<bool> =
        x.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).zip(&skipped).map(|((p, q), s)| !*s && differ(p, q)).collect();
    if !differs.contains(&true) {
        return (Difference { any: 0.0, art: 0.0, solid: 0.0, typed: 0.0 }, vec![false; w * h]);
    }
    let typed = in_areas(areas, w, h);
    let art = differs.iter().zip(&typed).filter(|(d, t)| **d && !**t).count();
    // Which pixels differ with their left and right neighbors.
    let mut across = vec![false; w * h];
    for (k, a) in across.iter_mut().enumerate() {
        let i = k % w;
        *a = i > 0 && i + 1 < w && differs.get(k - 1..=k + 1).is_some_and(|d| d.iter().all(|d| *d));
    }
    // A pixel on the region's edge has neighbours outside it, which don't count as differing.
    let mut solid = vec![false; w * h];
    for (k, s) in solid.iter_mut().enumerate().skip(w).take(w * h.saturating_sub(2)) {
        *s = !typed.get(k).copied().unwrap_or(true) && [k - w, k, k + w].iter().all(|n| across.get(*n).copied().unwrap_or(false));
    }
    let area = (w as f64 * h as f64).max(1.0);
    let any = differs.iter().filter(|d| **d).count() as f64 / area;
    let art = art as f64 / area;
    let shares = Difference { any, art, solid: solid.iter().filter(|s| **s).count() as f64 / area, typed: any - art };
    (shares, solid)
}

/// Where the type that shows in `nodes` is, in pixels of `region` drawn at `scale` (with a margin
/// for the strokes round it). The type in a live envelope isn't: the envelope draws it bent, away
/// from its text object's box, and is weighed as the art it draws, as its drawn art would be.
fn type_areas(nodes: &[Arc<Node>], region: Rect, scale: f64, out: &mut Vec<Rect>) {
    for n in nodes.iter().filter(|n| n.visible) {
        if let NodeKind::Text(_) = n.kind
            && let Some(b) = n.visual_bounds()
        {
            let px =
                |r: Rect| Rect::new((r.x0 - region.x0) * scale, (r.y0 - region.y0) * scale, (r.x1 - region.x0) * scale, (r.y1 - region.y0) * scale);
            out.push(px(b.inflate(OUTLINE_REACH, OUTLINE_REACH)));
        } else if !matches!(n.kind, NodeKind::Envelope { .. })
            && let Some(c) = n.children()
        {
            type_areas(c, region, scale, out);
        }
    }
}

/// Where every text object in `nodes` is, shown or not, as [`type_areas`] gives it, and every placed
/// file (whose preview draws the type it has).
fn all_type_areas(nodes: &[Arc<Node>], region: Rect, scale: f64, out: &mut Vec<Rect>) {
    for n in nodes {
        let placed = matches!(&n.kind, NodeKind::Image(im) if im.link.is_some());
        if (placed || matches!(n.kind, NodeKind::Text(_)))
            && let Some(b) = n.visual_bounds()
        {
            let b = b.inflate(OUTLINE_REACH, OUTLINE_REACH);
            out.push(Rect::new((b.x0 - region.x0) * scale, (b.y0 - region.y0) * scale, (b.x1 - region.x0) * scale, (b.y1 - region.y0) * scale));
        } else if let Some(c) = n.children() {
            all_type_areas(c, region, scale, out);
        }
    }
}

/// Which of `w` × `h` pixels are marked, counted so that the share of any rectangle's pixels that
/// are is read at once.
pub(super) struct Shares {
    /// How many of the marked pixels lie above and left of each pixel corner.
    sums: Vec<u32>,
    w: usize,
    h: usize,
}

impl Shares {
    /// The pixels `marks` marks (row by row).
    pub(super) fn of(marks: &[bool], w: usize, h: usize) -> Self {
        let stride = w + 1;
        let mut sums = vec![0u32; stride * (h + 1)];
        // Row by row: the marked pixels left of each corner in its row, added to the row above's.
        let mut above = vec![0u32; stride];
        for (j, out) in sums.chunks_exact_mut(stride).skip(1).enumerate() {
            let row = marks.get(j * w..).unwrap_or(&[]).iter().copied().chain(std::iter::repeat(false)).take(w);
            let mut run = 0u32;
            for ((marked, s), up) in row.zip(out.iter_mut().skip(1)).zip(above.iter().skip(1)) {
                run += u32::from(marked);
                *s = up + run;
            }
            above.copy_from_slice(out);
        }
        Self { sums, w, h }
    }

    /// The share (0 to 1) of `r`'s pixels that are marked: 0 for one with none of the pixels.
    pub(super) fn share(&self, r: &Rect) -> f64 {
        let (w, h, stride) = (self.w, self.h, self.w + 1);
        let (x0, y0, x1, y1) = pixel_box(r, w, h);
        if !r.is_finite() || x1 <= x0 || y1 <= y0 {
            return 0.0;
        }
        let at = |x: usize, y: usize| self.sums.get(y * stride + x).copied().unwrap_or(0);
        let inside = (at(x1, y1) + at(x0, y0)).saturating_sub(at(x0, y1) + at(x1, y0));
        f64::from(inside) / ((x1 - x0) as f64 * (y1 - y0) as f64)
    }
}

/// The pixels `r` touches of `w` × `h` (first and last column and row, the last ones excluded).
fn pixel_box(r: &Rect, w: usize, h: usize) -> (usize, usize, usize, usize) {
    let clamp = |v: f64, max: usize| if v.is_finite() { v.clamp(0.0, max as f64) as usize } else { 0 };
    (clamp(r.x0.floor(), w), clamp(r.y0.floor(), h), clamp(r.x1.ceil(), w), clamp(r.y1.ceil(), h))
}

/// [`Shares::share`] of each of `rects` for the `w` × `h` pixels `marks` marks: counted one by one
/// when the rectangles hold fewer pixels in all than there are, else with [`Shares`].
fn shares(marks: &[bool], w: usize, h: usize, rects: &[Rect]) -> Vec<f64> {
    let few = rects
        .iter()
        .map(|r| pixel_box(r, w, h))
        .try_fold(0usize, |n, (x0, y0, x1, y1)| n.checked_add(x1.saturating_sub(x0).checked_mul(y1.saturating_sub(y0))?));
    if !few.is_some_and(|n| n <= w * h) {
        let counted = Shares::of(marks, w, h);
        return rects.iter().map(|r| counted.share(r)).collect();
    }
    let share = |r: &Rect| {
        let (x0, y0, x1, y1) = pixel_box(r, w, h);
        if !r.is_finite() || x1 <= x0 || y1 <= y0 {
            return 0.0;
        }
        let inside: usize = (y0..y1).map(|y| marks.get(y * w + x0..y * w + x1).map_or(0, |row| row.iter().filter(|m| **m).count())).sum();
        inside as f64 / ((x1 - x0) as f64 * (y1 - y0) as f64)
    };
    rects.iter().map(share).collect()
}

/// Which of `w` × `h` pixels (row by row) lie in one of `areas`.
fn in_areas(areas: &[Rect], w: usize, h: usize) -> Vec<bool> {
    if areas.is_empty() {
        return vec![false; w * h];
    }
    let pixels: Vec<(usize, usize, usize, usize)> = areas.iter().map(|r| pixel_box(r, w, h)).collect();
    // Areas that cover fewer pixels in all than there are are marked one by one.
    let covered =
        pixels.iter().try_fold(0usize, |n, &(x0, y0, x1, y1)| if x1 < x0 || y1 < y0 { None } else { n.checked_add((x1 - x0).checked_mul(y1 - y0)?) });
    if covered.is_some_and(|n| n <= w * h) {
        let mut inside = vec![false; w * h];
        for (x0, y0, x1, y1) in pixels {
            for y in y0..y1 {
                if let Some(row) = inside.get_mut(y * w + x0..y * w + x1) {
                    row.fill(true);
                }
            }
        }
        return inside;
    }
    // Each area adds one inside it: corners marked, then summed along the rows and down the columns.
    let stride = w + 1;
    let mut count = vec![0i64; stride * (h + 1)];
    for (x0, y0, x1, y1) in pixels {
        for (x, y, d) in [(x0, y0, 1), (x1, y0, -1), (x0, y1, -1), (x1, y1, 1)] {
            if let Some(c) = count.get_mut(y * stride + x) {
                *c += d;
            }
        }
    }
    let mut inside = vec![false; w * h];
    if w == 0 {
        return inside;
    }
    let mut column = vec![0i64; stride];
    for (row, out) in count.chunks_exact(stride).zip(inside.chunks_exact_mut(w)) {
        let mut run = 0;
        for ((c, sum), o) in row.iter().zip(column.iter_mut()).zip(out) {
            run += *c;
            *sum += run;
            *o = *sum > 0;
        }
    }
    inside
}

/// The least share of a text object's area (with the margin round it) where the page draws anything
/// or has type, for its type laid out again to count as type.
const MARKED: f64 = 0.05;

/// More of the page than this differing between the layers' art and the page's own, and the
/// layers aren't used.
pub(in crate::import) const MAX_DIFFERENCE: f64 = 0.05;

/// More than this and the import says so.
const NOTABLE_DIFFERENCE: f64 = 0.002;

/// More of the page than this where one draws an object that the other doesn't (see
/// [`Difference`]), and the layers aren't used: about a centimetre square on a letter page.
pub(in crate::import) const MAX_SOLID_DIFFERENCE: f64 = 0.001;

/// How far (points) the art the layers print may reach past a page that holds it all.
const PAST_THE_PAGE: f64 = 1.0;

/// Do the layers' art and the page's own look alike (where the page holds all the art that prints,
/// also round it), on every page the frame has ([`Frame::more`])? An error says they don't; `Ok` has
/// the notes to give when they differ a little: with the share of the pixels that differ away from
/// type when that is notable, otherwise with the share of those that differ in the type's areas, on
/// the page that differs most. With `type_reflow` (all the type laid out again from the file's text,
/// which may show in another font), type that differs doesn't make them differ: only art away from
/// type does, and the type's share is a note of its own.
pub(super) fn compare(frame: &Frame<'_>, view: &mut PageView, layered: &Document, type_reflow: bool) -> Result<Vec<String>, String> {
    let mut diff = view.difference_on_every_page(layered, frame.rect, type_reflow);
    if frame.whole
        && let Some(art) = vectorcraft_render::encode::art_bounds(as_printed(layered).as_ref().unwrap_or(layered))
            .filter(|a| !frame.rect.inflate(PAST_THE_PAGE, PAST_THE_PAGE).contains_rect(*a))
    {
        // The page and the art the layers print beyond it, each in its own document's place.
        let all = frame.rect.union(art);
        let (dx, dy) = (frame.page_rect.x0 - frame.rect.x0, frame.page_rect.y0 - frame.rect.y0);
        diff = diff.max(difference(frame.page, Rect::new(all.x0 + dx, all.y0 + dy, all.x1 + dx, all.y1 + dy), layered, all));
    }
    if diff.weighed(type_reflow) > MAX_DIFFERENCE || diff.solid > MAX_SOLID_DIFFERENCE {
        return Err(format!("the art on them differs from the page's in {:.1}% of it", diff.weighed(type_reflow) * 100.0));
    }
    if diff.any <= NOTABLE_DIFFERENCE {
        return Ok(vec![]);
    }
    let art = format!(
        "the art on the layers differs from the file's own page in about {:.1}% of it (brush strokes, live effects and the like that the layers' data doesn't describe)",
        diff.art * 100.0
    );
    let typed = format!(
        "the type on the layers differs from the file's own page in about {:.1}% of it (type laid out again from the file's text, or drawn in another font)",
        diff.typed * 100.0
    );
    let art_notable = diff.art > NOTABLE_DIFFERENCE;
    Ok(match (art_notable, type_reflow && diff.typed > NOTABLE_DIFFERENCE) {
        (true, true) => vec![art, typed],
        (true, false) => vec![art],
        (false, _) => vec![typed],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One page that differs only in its type and another that differs only in its art keep both
    /// shares when they are taken together.
    #[test]
    fn the_type_share_of_one_page_stays_beside_the_art_share_of_another() {
        let typed = Difference { any: 0.03, art: 0.0, solid: 0.0, typed: 0.03 };
        let art = Difference { any: 0.04, art: 0.04, solid: 0.0, typed: 0.0 };
        let both = typed.max(art);
        assert_eq!((both.any, both.art, both.typed), (0.04, 0.04, 0.03));
    }

    /// Shares and areas counted pixel by pixel match those read from sums.
    #[test]
    fn pixels_counted_one_by_one_match_the_sums() {
        let (w, h) = (37, 23);
        let marks: Vec<bool> = (0..w * h).map(|k| (k * 7 + k / 5) % 3 == 0).collect();
        let rects =
            [Rect::new(-2.0, 3.5, 10.2, 9.0), Rect::new(5.0, 5.0, 12.0, 12.0), Rect::new(3.0, 3.0, 3.0, 8.0), Rect::new(10.5, 2.2, 20.7, 22.9)];
        // 66 + 49 + 0 + 231 pixels, fewer than the 851 there are: counted one by one.
        let held: usize = rects.iter().map(|r| pixel_box(r, w, h)).map(|(x0, y0, x1, y1)| x1.saturating_sub(x0) * y1.saturating_sub(y0)).sum();
        assert_eq!(held, 346);
        let summed = Shares::of(&marks, w, h);
        let one_by_one = shares(&marks, w, h, &rects);
        assert_eq!(one_by_one, rects.iter().map(|r| summed.share(r)).collect::<Vec<_>>());
        // So many rectangles that they hold more pixels than there are: read from sums.
        let many: Vec<Rect> = rects.iter().cycle().take(40).copied().collect();
        assert_eq!(shares(&marks, w, h, &many)[..4], one_by_one[..]);
        assert_eq!(in_areas(&many, w, h), in_areas(&rects, w, h));
        assert!(in_areas(&rects, w, h).contains(&true));
    }

    /// A document with a 40-point gray square in each of `n` 100-point areas side by side, `gap`
    /// apart (the one in area `nudged` 20 points to the right), the areas, and the squares' ids.
    fn squares(n: usize, gap: f64, nudged: Option<usize>) -> (Document, Vec<Rect>, Vec<vectorcraft_doc::NodeId>) {
        let mut doc = Document::new(100.0, 100.0);
        let layer = doc.layers[0].id;
        let (mut areas, mut ids) = (vec![], vec![]);
        for i in 0..n {
            let x = i as f64 * (100.0 + gap);
            areas.push(Rect::new(x, 0.0, x + 100.0, 100.0));
            let x = if nudged == Some(i) { x + 20.0 } else { x };
            ids.push(square(&mut doc, layer, Rect::new(x + 30.0, 30.0, x + 70.0, 70.0)));
        }
        (doc, areas, ids)
    }

    /// A gray square on `rect` added to `layer` of `doc`: its id.
    fn square(doc: &mut Document, layer: vectorcraft_doc::NodeId, rect: Rect) -> vectorcraft_doc::NodeId {
        filled(doc, layer, rect, 0.5)
    }

    /// A rectangle on `rect` filled with `k` black added to `layer` of `doc`: its id.
    fn filled(doc: &mut Document, layer: vectorcraft_doc::NodeId, rect: Rect, k: f32) -> vectorcraft_doc::NodeId {
        let paint = vectorcraft_color::Paint::solid(vectorcraft_color::Color::cmyk(0.0, 0.0, 0.0, k));
        let appearance = vectorcraft_doc::Appearance {
            items: vec![vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(paint))],
            ..Default::default()
        };
        let node = Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(rect), appearance);
        let id = node.id;
        doc.insert(Some(layer), 0, node).unwrap();
        id
    }

    /// A frame of `page`'s `areas`, each its own artboard.
    fn frame_of<'a>(page: &'a Document, areas: &[Rect]) -> Frame<'a> {
        let first = areas[0];
        Frame { page, page_rect: first, rect: first, whole: false, pages: vec![], more: areas[1..].iter().map(|r| (*r, *r)).collect() }
    }

    /// The layers are drawn once for all the pages of a sheet, a fraction of a pixel apart as they
    /// are, and the drawings are kept until the document changes.
    #[test]
    fn the_layers_are_drawn_once_for_the_pages_and_kept() {
        let (doc, areas, _) = squares(4, 20.3, None);
        let frame = frame_of(&doc, &areas);
        let mut view = PageView::new(&frame);
        assert_eq!(view.sheets.len(), 1);
        let each = view.differences(&doc, areas[0], false);
        assert_eq!(each.len(), 4);
        assert!(each.iter().all(|d| d.any == 0.0), "{:?}", each.iter().map(|d| d.any).collect::<Vec<_>>());
        assert_eq!(view.draws, 1, "one drawing for the four pages");
        assert!(compare(&frame, &mut view, &doc, false).is_ok());
        assert_eq!(view.difference_on_every_page(&doc, areas[0], true).any, 0.0);
        assert_eq!(view.draws, 1, "the same document isn't drawn again");
        let (nudged, _, _) = squares(4, 20.3, Some(3));
        let each = view.differences(&nudged, areas[0], false);
        assert_eq!(view.draws, 2);
        assert!(each[3].solid > MAX_SOLID_DIFFERENCE && each[..3].iter().all(|d| d.any == 0.0));
    }

    /// Taking an object out of the layers has only the pages it lies on drawn again; the other
    /// pages' drawings are kept for the document without it.
    #[test]
    fn a_removal_draws_again_only_the_pages_it_lies_on() {
        let (page, areas, _) = squares(4, 20.3, None);
        let (mut layers, _, _) = squares(4, 20.3, None);
        let layer = layers.layers[0].id;
        let extra = square(&mut layers, layer, Rect::new(2.0 * 120.3 + 5.0, 5.0, 2.0 * 120.3 + 25.0, 25.0));
        let frame = frame_of(&page, &areas);
        let mut view = PageView::new(&frame);
        let before = view.differences(&layers, areas[0], false);
        assert!(before[2].any > 0.0);
        let mut without = layers.clone();
        without.node_mut(extra).unwrap().visible = false;
        let shares = view.weigh_without(&layers, &without, &[2]);
        assert_eq!(view.draws, 2, "only the third page is drawn without it");
        assert!(shares.len() == 1 && shares[0].2 < shares[0].1, "{shares:?}");
        view.adopt(&without);
        let after = view.differences(&without, areas[0], false);
        assert_eq!(view.draws, 2, "the other pages' drawings are kept");
        let again = PageView::new(&frame).differences(&without, areas[0], false);
        for (a, b) in after.iter().zip(&again) {
            assert!((a.any - b.any).abs() < 1e-3 && (a.solid - b.solid).abs() < 1e-3);
        }
        assert_eq!(after[2].any, 0.0);
    }

    /// Pages far apart are drawn on sheets of their own, and a long row of them on sheets no longer
    /// than the limit.
    #[test]
    fn sheets_stay_within_their_limits() {
        let (doc, areas, _) = squares(3, 5000.0, None);
        assert_eq!(PageView::new(&frame_of(&doc, &areas)).sheets.len(), 3);
        let (doc, areas, _) = squares(40, 0.0, None);
        let frame = frame_of(&doc, &areas);
        let mut view = PageView::new(&frame);
        assert!(view.sheets.len() > 1);
        assert!(view.sheets.iter().all(|s| s.rect.width() * s.scale <= MAX_SHEET_SIDE));
        assert!(view.differences(&doc, areas[0], false).iter().all(|d| d.any == 0.0));
        assert_eq!(view.draws, view.sheets.len());
        let before = view.draws;
        assert_eq!(view.draw(&doc, &[0, 1]).len(), 2);
        assert_eq!(view.draws, before + 2, "two pages of a long sheet are drawn on their own");
    }

    /// A page moved by a fraction of a pixel onto its sheet doesn't count the column its move took
    /// off its artboard, where the layers have art past the artboard before it and the page has none.
    #[test]
    fn the_column_a_page_moved_onto_doesnt_count() {
        // The second area starts 400.4 pixels from the first at 4 pixels a point: it moves 0.4 pixels
        // to the left, and its first column starts at 100 points.
        let areas = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.1, 0.0, 200.1, 100.0)];
        let mut page = Document::new(100.0, 100.0);
        let layer = page.layers[0].id;
        filled(&mut page, layer, areas[0], 1.0);
        let mut layers = page.clone();
        let layer = layers.layers[0].id;
        // Black reaching past the first artboard, into the space between the two.
        filled(&mut layers, layer, Rect::new(99.0, 0.0, 100.1, 100.0), 1.0);
        let frame = frame_of(&page, &areas);
        let mut view = PageView::new(&frame);
        assert_eq!(view.sheets.len(), 1);
        let each = view.differences(&layers, areas[0], false);
        assert!(each.iter().all(|d| d.any == 0.0), "{:?}", each.iter().map(|d| d.any).collect::<Vec<_>>());
        assert_eq!(compare(&frame, &mut view, &layers, false), Ok(vec![]));
        assert_eq!(view.colour_difference(1, &layers), 0.0);
    }

    /// The drawings of two documents weighed in turn are both kept.
    #[test]
    fn the_drawings_of_two_documents_are_kept() {
        let (doc, areas, _) = squares(4, 20.3, None);
        let (other, _, _) = squares(4, 20.3, Some(1));
        let frame = frame_of(&doc, &areas);
        let mut view = PageView::new(&frame);
        for _ in 0..3 {
            view.differences(&doc, areas[0], false);
            view.differences(&other, areas[0], false);
        }
        assert_eq!(view.draws, 2, "each document is drawn once");
    }
}
