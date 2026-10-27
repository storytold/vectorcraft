use super::build::Frame;
use super::slots::OUTLINE_REACH;
use super::*;

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
pub(super) fn differ(p: &[u8; 4], q: &[u8; 4]) -> bool {
    let (p, q) = (on_white(p), on_white(q));
    let (a, b) = (ink(p), ink(q));
    lightness(p).abs_diff(lightness(q)) > SAME_PIXEL || (a.min(b) <= WHITE && a.abs_diff(b) > SAME_PIXEL)
}

/// How far apart two pixels' red, green or blue may be for them to be the same colour (a colour
/// and its CMYK equivalent stay within it; a red and a dark red don't).
const SAME_COLOUR: u32 = 64;

/// Are two pixels different colours ([`differ`] also takes a red and a dark red of about the same
/// lightness to be alike)?
pub(super) fn differ_in_colour(p: &[u8; 4], q: &[u8; 4]) -> bool {
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
}

/// The scale the comparisons draw a region at: about 400 pixels along its longer side.
fn compare_scale(r: Rect) -> f64 {
    (400.0 / r.width().max(r.height()).max(1.0)).min(4.0)
}

/// A flattened print page made only of images. Sampling it directly at 400 pixels aliases
/// its fine edges while vector art receives coverage antialiasing. Compare both at a higher
/// resolution and apply the same reconstruction filter; keep the existing difference limits.
pub(super) fn comparison_samples(doc: &Document) -> u32 {
    let (mut images, mut vector) = (false, false);
    doc.walk(|n| match n.kind {
        NodeKind::Image(_) => images = true,
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Path { clipping: true, .. } => {}
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

/// The page drawn once, to weigh the layers' art against it: [`Self::difference`] draws the layers
/// with a renderer of their own, which keeps their decoded images between draws (the page's images
/// can share their keys and not their pixels, so the two don't share one).
pub(super) struct PageView {
    page: vectorcraft_render::Rendered,
    /// Where the page's own type is, in its pixels.
    page_type: Vec<Rect>,
    /// Where objects read with their art style lie, in its pixels: the page has Illustrator's
    /// rendering of the style there, the layers VectorCraft's live effects, and the two aren't
    /// held to look alike.
    restyled: Vec<Rect>,
    layers: vectorcraft_render::Renderer,
    scale: f64,
    samples: u32,
}

impl PageView {
    pub(super) fn new(frame: &Frame<'_>) -> Self {
        let ra = frame.page_rect;
        let scale = compare_scale(ra);
        // Drawn on nothing and then seen on white: blending modes have no page to blend with, as in
        // the app that wrote the file.
        let samples = comparison_samples(frame.page);
        let page = comparison_render(&mut vectorcraft_render::Renderer::new(), frame.page, ra, scale, samples);
        let mut page_type = vec![];
        type_areas(&frame.page.layers, ra, scale, &mut page_type);
        Self { page, page_type, restyled: vec![], layers: vectorcraft_render::Renderer::new(), scale, samples }
    }

    /// Leave the drawn looks in `looks` (on `region`, the page's area in the layers' document) out
    /// of the comparisons ([`Structure::restyled`](ai::Structure)).
    pub(super) fn restyled(&mut self, looks: &[Rect], region: Rect) {
        let s = self.scale;
        self.restyled = looks
            .iter()
            .map(|r| r.inflate(OUTLINE_REACH, OUTLINE_REACH))
            .map(|r| Rect::new((r.x0 - region.x0) * s, (r.y0 - region.y0) * s, (r.x1 - region.x0) * s, (r.y1 - region.y0) * s))
            .collect();
    }

    /// How `b` on `rb` (a rectangle the page's size) draws differently from the page ([`compared`]).
    pub(super) fn difference(&mut self, b: &Document, rb: Rect) -> Difference {
        // The page doesn't print what isn't printed.
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let y = comparison_render(&mut self.layers, b, rb, self.scale, self.samples);
        let mut areas = self.page_type.clone();
        type_areas(&b.layers, rb, self.scale, &mut areas);
        compared(&self.page, &y, &areas, &self.restyled)
    }

    /// The share (0 to 1) of the pixels where `b` on `rb` draws another colour than the page
    /// ([`differ_in_colour`]), type included: what taking an object out of the layers changes
    /// there, when it is covered by one of about the same lightness (#1102).
    pub(super) fn colour_difference(&mut self, b: &Document, rb: Rect) -> f64 {
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let y = comparison_render(&mut self.layers, b, rb, self.scale, self.samples);
        if (y.width, y.height) != (self.page.width, self.page.height) {
            return 1.0;
        }
        let differing = self.page.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).filter(|(p, q)| differ_in_colour(p, q)).count();
        differing as f64 / (f64::from(y.width) * f64::from(y.height)).max(1.0)
    }
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
    if x.width == 0 || x.height == 0 || x.width != y.width || x.height != y.height {
        return Difference { any: 1.0, art: 1.0, solid: 1.0 };
    }
    let (w, h) = (x.width as usize, x.height as usize);
    // Pixels in `skip` count as alike.
    let skipped = in_areas(skip, w, h);
    let differs: Vec<bool> =
        x.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).zip(&skipped).map(|((p, q), s)| !*s && differ(p, q)).collect();
    let at = |i: usize, j: usize| j.checked_mul(w).and_then(|row| row.checked_add(i)).and_then(|k| differs.get(k)).copied().unwrap_or(false);
    let typed = in_areas(areas, w, h);
    let art = differs.iter().zip(&typed).filter(|(d, t)| **d && !**t).count();
    // A pixel on the region's edge has neighbours outside it, which don't count as differing.
    let solid = (1..h.saturating_sub(1))
        .flat_map(|j| (1..w.saturating_sub(1)).map(move |i| (i, j)))
        .filter(|&(i, j)| !typed.get(j * w + i).copied().unwrap_or(true) && (j - 1..=j + 1).all(|n| (i - 1..=i + 1).all(|m| at(m, n))))
        .count();
    let area = (w as f64 * h as f64).max(1.0);
    Difference { any: differs.iter().filter(|d| **d).count() as f64 / area, art: art as f64 / area, solid: solid as f64 / area }
}

/// Where the type that shows in `nodes` is, in pixels of `region` drawn at `scale` (with a margin
/// for the strokes round it).
fn type_areas(nodes: &[Arc<Node>], region: Rect, scale: f64, out: &mut Vec<Rect>) {
    for n in nodes.iter().filter(|n| n.visible) {
        if let NodeKind::Text(_) = n.kind
            && let Some(b) = n.visual_bounds()
        {
            let px =
                |r: Rect| Rect::new((r.x0 - region.x0) * scale, (r.y0 - region.y0) * scale, (r.x1 - region.x0) * scale, (r.y1 - region.y0) * scale);
            out.push(px(b.inflate(OUTLINE_REACH, OUTLINE_REACH)));
        } else if let Some(c) = n.children() {
            type_areas(c, region, scale, out);
        }
    }
}

/// Which of `w` × `h` pixels (row by row) lie in one of `areas`.
fn in_areas(areas: &[Rect], w: usize, h: usize) -> Vec<bool> {
    // Each area adds one inside it: corners marked, then summed along the rows and down the columns.
    let stride = w + 1;
    let mut count = vec![0i64; stride * (h + 1)];
    let clamp = |v: f64, max: usize| if v.is_finite() { v.clamp(0.0, max as f64) as usize } else { 0 };
    for r in areas {
        let (x0, y0, x1, y1) = (clamp(r.x0.floor(), w), clamp(r.y0.floor(), h), clamp(r.x1.ceil(), w), clamp(r.y1.ceil(), h));
        for (x, y, d) in [(x0, y0, 1), (x1, y0, -1), (x0, y1, -1), (x1, y1, 1)] {
            if let Some(c) = count.get_mut(y * stride + x) {
                *c += d;
            }
        }
    }
    for row in count.chunks_mut(stride) {
        let mut run = 0;
        for c in row {
            run += *c;
            *c = run;
        }
    }
    let mut column = vec![0i64; stride];
    for row in count.chunks_mut(stride) {
        for (c, sum) in row.iter_mut().zip(column.iter_mut()) {
            *sum += *c;
            *c = *sum;
        }
    }
    (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| count.get(j * stride + i).is_some_and(|c| *c > 0)).collect()
}

/// More of the page than this differing between the layers' art and the page's own, and the
/// layers aren't used.
pub(super) const MAX_DIFFERENCE: f64 = 0.05;

/// More than this and the import says so.
const NOTABLE_DIFFERENCE: f64 = 0.002;

/// More of the page than this where one draws an object that the other doesn't (see
/// [`Difference`]), and the layers aren't used: about a centimetre square on a letter page.
pub(super) const MAX_SOLID_DIFFERENCE: f64 = 0.001;

/// How far (points) the art the layers print may reach past a page that holds it all.
const PAST_THE_PAGE: f64 = 1.0;

/// Do the layers' art and the page's own look alike (where the page holds all the art that prints,
/// also round it)? An error says they don't; `Ok` has the note to give when they differ a little:
/// with the share of the pixels that differ away from type when that is notable, otherwise with the
/// share of those that differ in the type's areas.
pub(super) fn compare(frame: &Frame<'_>, view: &mut PageView, layered: &Document) -> Result<Option<String>, String> {
    let mut diff = view.difference(layered, frame.rect);
    if frame.whole
        && let Some(art) = vectorcraft_render::encode::art_bounds(as_printed(layered).as_ref().unwrap_or(layered))
            .filter(|a| !frame.rect.inflate(PAST_THE_PAGE, PAST_THE_PAGE).contains_rect(*a))
    {
        // The page and the art the layers print beyond it, each in its own document's place.
        let all = frame.rect.union(art);
        let (dx, dy) = (frame.page_rect.x0 - frame.rect.x0, frame.page_rect.y0 - frame.rect.y0);
        let round = difference(frame.page, Rect::new(all.x0 + dx, all.y0 + dy, all.x1 + dx, all.y1 + dy), layered, all);
        diff = Difference { any: diff.any.max(round.any), art: diff.art.max(round.art), solid: diff.solid.max(round.solid) };
    }
    if diff.any > MAX_DIFFERENCE || diff.solid > MAX_SOLID_DIFFERENCE {
        return Err(format!("the art on them differs from the page's in {:.1}% of it", diff.any * 100.0));
    }
    if diff.any <= NOTABLE_DIFFERENCE {
        return Ok(None);
    }
    Ok(Some(if diff.art > NOTABLE_DIFFERENCE {
        format!(
            "the art on the layers differs from the file's own page in about {:.1}% of it (brush strokes, live effects and the like that the layers' data doesn't describe)",
            diff.art * 100.0
        )
    } else {
        format!(
            "the type on the layers differs from the file's own page in about {:.1}% of it (type laid out again from the file's text, or drawn in another font)",
            (diff.any - diff.art) * 100.0
        )
    }))
}
