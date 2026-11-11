use super::analysis::{Scratch, outlined_text, overlapping, reach};
use super::compose::{Builder, CLEAR, Composer, Elem, Fill, Kind, Px, WHITE};
use super::options::FlattenOptions;
use super::{
    Affine, Appearance, AppearanceItem, Arc, BlendMode, Color, ColorMode, Document, EngineError, FillLayer, FillRule, ImageBlob, ImageObject, Inks,
    MAX_PIXELS, Node, NodeId, NodeKind, Paint, PathData, Rect, Result, composite, effects, isolated_doc, outline_strokes_under, planes_cmyk, po,
    separate, shape_node, shapes, unique_key,
};

// ---------- plan ----------

/// What the flattening does, worked out before the document changes.
pub(super) struct Plan {
    pub(super) flat: Vec<Flat>,
    /// Objects without transparency (they stay, but for the outline options).
    pub(super) kept: Vec<NodeId>,
    /// The objects with transparency of their own.
    pub(super) transparent: Vec<NodeId>,
}

/// One group of overlapping objects with transparency and what replaces it.
pub(super) struct Flat {
    /// Its objects in paint order; the result takes the place of the last.
    pub(super) roots: Vec<NodeId>,
    pub(super) regions: Regions,
    /// Where its image goes, and the image (once rendered).
    pub(super) area: Option<Area>,
    raster: Option<Raster>,
    /// When the group is one block of type over several lines: its lines, to group the regions by.
    lines: Option<Lines>,
}

/// The lines of a block of type: document space → its line space, where each line spans a band
/// across the lines (its ascent above the baseline to its descent below).
struct Lines {
    to_lines: Affine,
    bands: Vec<(f64, f64)>,
}

impl Lines {
    /// The lines of `n` when it is type laid out in two lines or more (not on a path).
    fn of(n: &Node) -> Option<Lines> {
        let NodeKind::Text(t) = &n.kind else { return None };
        if t.xf.determinant().abs() < 1e-12 {
            return None;
        }
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        if layout.on_path {
            return None;
        }
        let bands: Vec<(f64, f64)> = layout.lines.iter().filter(|l| l.x1 > l.x0).map(|l| (l.baseline - l.ascent, l.baseline + l.descent)).collect();
        let to_lines = if layout.vertical { layout.line_xf.inverse() * t.xf.inverse() } else { t.xf.inverse() };
        (bands.len() > 1).then_some(Lines { to_lines, bands })
    }

    /// The line nearest to the middle of `path`.
    fn line_of(&self, path: &PathData) -> usize {
        let Some(b) = path.bounds() else { return 0 };
        let y = (self.to_lines * b.center()).y;
        let away = |&(lo, hi): &(f64, f64)| if y < lo { lo - y } else { (y - hi).max(0.0) };
        self.bands.iter().enumerate().min_by(|a, b| away(a.1).total_cmp(&away(b.1))).map_or(0, |(i, _)| i)
    }
}

/// Where a group's image goes.
pub(super) struct Area {
    rect: Rect,
    ppi: f64,
    /// The complex regions it is clipped to (Clip Complex Regions).
    clip: Option<PathData>,
    /// The raster/vector balance rasterizes the whole group (it splits into too many regions).
    pub(super) whole: bool,
}

impl Area {
    /// The area's outline.
    pub(super) fn outline(&self) -> PathData {
        self.clip.clone().unwrap_or_else(|| shapes::rectangle(self.rect))
    }
}

/// Flat-colour regions: their outlines and paints.
pub(super) type Regions = Vec<(PathData, RegionFill)>;

/// A flat-colour region's paint.
pub(super) struct RegionFill {
    paint: Paint,
    alpha: f32,
    overprint: bool,
}

/// A rendered image of complex regions, clipped to them or not.
struct Raster {
    blob: ImageBlob,
    width: u32,
    height: u32,
    xf: Affine,
    clip: Option<PathData>,
}

/// The flattening of `roots` with `o`; `render` renders the images (the preview only needs where
/// they go). Fails only when an image can't be encoded.
pub(super) fn plan(src: &Document, roots: &[NodeId], o: &FlattenOptions, render: bool) -> Result<Plan> {
    let tmp = isolated_doc(src, roots.iter().filter_map(|id| src.node(*id).cloned()).collect());
    // Geometry effects first: what is left on the art are raster effects.
    let baked = effects::bake_document(&tmp).unwrap_or(tmp);
    let art: Vec<Node> = baked.layers[0].children().into_iter().flatten().map(|a| (**a).clone()).collect();
    let mut sc = Scratch { brushes: vectorcraft_brush::library(&baked), doc: baked.clone() };
    let plain: Vec<Node> = art.iter().map(|n| sc.plain(n)).collect();
    // Each object as its clip groups and clipping layers show it.
    let clipped: Vec<Node> = plain.iter().zip(roots).map(|(n, id)| clipped_by_ancestors(src, *id, n)).collect();
    let reaches: Vec<Option<Rect>> = plain.iter().zip(&clipped).map(|(n, c)| reach(n).zip(reach(c)).map(|(a, b)| a.intersect(b))).collect();
    let see_through: Vec<bool> = plain.iter().map(|n| o.shows_transparency(&baked, n) || o.draws_see_through_image(n)).collect();
    let cmyk = src.color_mode == ColorMode::Cmyk;
    let mut out = Plan { flat: vec![], kept: vec![], transparent: roots.iter().zip(&see_through).filter(|(_, t)| **t).map(|(id, _)| *id).collect() };
    // Baking above stopped early when cancelled; each group is a place to stop (a plan changes nothing).
    vectorcraft_doc::cancel::check()?;
    for g in overlapping(&reaches)? {
        vectorcraft_doc::cancel::check()?;
        let ids: Vec<NodeId> = g.iter().map(|&i| roots[i]).collect();
        let group = g.iter().any(|&i| see_through[i]).then(|| {
            let plain: Vec<&Node> = g.iter().map(|&i| &clipped[i]).collect();
            flatten_group(&plain, o, cmyk)
        });
        let Some((regions, area)) = group.flatten() else {
            out.kept.extend(ids);
            continue;
        };
        let raster = match area.as_ref().filter(|_| render) {
            Some(a) => {
                // Drawn as planned: the image can land outside the clip groups and clipping layers
                // around its art.
                let art: Vec<Node> = g.iter().map(|&i| clipped_by_ancestors(src, roots[i], &art[i])).collect();
                Some(render_raster(&baked, &art, a.rect, a.ppi, a.clip.clone(), o)?)
            }
            None => None,
        };
        let lines = match g.as_slice() {
            [i] => Lines::of(&art[*i]),
            _ => None,
        };
        out.flat.push(Flat { roots: ids, regions, area, raster, lines });
    }
    Ok(out)
}

/// `plain` (object `id` of `src` as plain art) inside a clip group for each clip group and
/// clipping layer around `id`, innermost first, each clipped by that group's or layer's clipping
/// path. `plain` itself when nothing around it clips.
fn clipped_by_ancestors(src: &Document, id: NodeId, plain: &Node) -> Node {
    let mut out = plain.clone();
    if !clips_count() {
        return out;
    }
    let ancestry = src.ancestry(id).unwrap_or_default();
    for pair in ancestry.windows(2).rev() {
        let Some(above) = src.node(pair[0]).filter(|n| n.clips()) else { continue };
        let Some(clip) = above.children().and_then(|c| c.first()) else { continue };
        // The clipping path itself is not clipped art.
        if clip.id == pair[1] {
            continue;
        }
        // Its outline, unpainted: the group or layer around the object still paints the path.
        // Nothing to clip by hides the clipped art, as in the compositor.
        let Some((outline, rule)) = effects::clip_outline(clip) else {
            out.visible = false;
            return out;
        };
        let mut path = Node::path(clip.id, PathData::from_bezpath(&outline), Appearance::default());
        if let NodeKind::Path { rule: r, .. } = &mut path.kind {
            *r = rule;
        }
        let mut g = Node::group(above.id, vec![Arc::new(path), Arc::new(out)]);
        g.set_clips(true);
        out = g;
    }
    out
}

/// The regions and the image area replacing one group of overlapping objects (`plain`: the art as
/// composited; `cmyk`: in a CMYK document); `None` when it paints nothing.
fn flatten_group(plain: &[&Node], o: &FlattenOptions, cmyk: bool) -> Option<(Regions, Option<Area>)> {
    let mut b = Builder { cmyk, ..Default::default() };
    let elems: Vec<Elem> = plain.iter().filter_map(|n| b.elem(n)).collect();
    if b.shapes.is_empty() {
        return None;
    }
    // An unclipped full raster needs no regions.
    let whole = o.balance <= 0.0 && !o.clip_complex_regions;
    // Nor does one that splits into more regions than the balance allows: the build stops at the
    // first region past the limit.
    let (regions, over) = match o.region_limit().filter(|_| !every_region()) {
        _ if whole => (vec![], false),
        Some(limit) => po::regions_within(&b.shapes, limit).map_or((vec![], true), |r| (r, false)),
        None => (po::regions(&b.shapes), false),
    };
    // An arrangement that failed (degenerate geometry) leaves only rasterizing.
    let all = over || regions.is_empty() || o.rasterize_all(regions.len());
    let mut vector: Vec<(usize, RegionFill)> = vec![];
    let mut complex: Vec<usize> = vec![];
    let mut gradients_only = !all;
    for (k, r) in regions.iter().enumerate() {
        // Art outside the clipping path of its clip group draws nothing there.
        let shown = |i: &&usize| b.kinds[**i] == Kind::Vector || !(clips_count() && b.hidden(**i, &r.sources));
        if all || r.sources.iter().filter(shown).any(|&i| b.kinds[i] != Kind::Vector) {
            gradients_only &= r.sources.iter().filter(shown).all(|&i| b.kinds[i] == Kind::Gradient);
            complex.push(k);
        } else if let Some(fill) = region_fill(&elems, &r.sources, o, cmyk) {
            vector.push((k, fill));
        }
    }
    let bounds = |ks: &[usize]| ks.iter().filter_map(|&k| regions[k].path.bounds()).reduce(|a, b| a.union(b));
    if !o.clip_complex_regions && !all {
        // A rectangle of raster takes in every region it overlaps.
        while let Some(r) = bounds(&complex) {
            let before = complex.len();
            let rect = shapes::rectangle(r);
            vector.retain(|(k, _)| {
                let path = &regions[*k].path;
                let hit = path.bounds().is_some_and(|b| b.intersect(r).area() > 0.0)
                    && po::area(&po::boolean(path, FillRule::NonZero, &rect, FillRule::NonZero, po::BoolOp::Intersect), FillRule::NonZero) > 1e-6;
                if hit {
                    complex.push(*k);
                }
                !hit
            });
            if complex.len() == before {
                break;
            }
            gradients_only = false;
        }
    }
    let raster_rect = if all { plain.iter().filter_map(|n| reach(n)).reduce(|a, b| a.union(b)) } else { bounds(&complex) };
    let area = raster_rect.map(|rect| {
        let clip = o.clip_complex_regions.then(|| PathData::new(complex.iter().flat_map(|&k| regions[k].path.subpaths.iter().cloned()).collect()));
        let ppi = if gradients_only { o.gradient_ppi } else { o.line_art_ppi };
        Area { rect, ppi, clip: clip.filter(|c| !c.is_empty()), whole: all }
    });
    let vector = vector.into_iter().map(|(k, f)| (regions[k].path.clone(), f)).collect();
    Some((vector, area))
}

#[cfg(test)]
thread_local! {
    /// Groups flattened on this thread build all of their regions ([`with_every_region`]).
    static EVERY_REGION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether groups build all of their regions, past the balance's limit too.
#[cfg(test)]
fn every_region() -> bool {
    EVERY_REGION.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn every_region() -> bool {
    false
}

/// `f` with every group it flattens building all of its regions, past the balance's limit too.
#[cfg(test)]
pub(crate) fn with_every_region<T>(f: impl FnOnce() -> T) -> T {
    EVERY_REGION.with(|e| e.set(true));
    let out = f();
    EVERY_REGION.with(|e| e.set(false));
    out
}

#[cfg(test)]
thread_local! {
    /// Groups flattened on this thread rasterize art where its clip group or clipping layer hides
    /// it too ([`with_clips_ignored`]).
    static CLIPS_IGNORED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a region outside the clipping path of a clip group or clipping layer leaves out the
/// art it hides there.
#[cfg(test)]
fn clips_count() -> bool {
    !CLIPS_IGNORED.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn clips_count() -> bool {
    true
}

/// `f` with every group it flattens rasterizing art where its clip group or clipping layer hides
/// it too.
#[cfg(test)]
pub(crate) fn with_clips_ignored<T>(f: impl FnOnce() -> T) -> T {
    CLIPS_IGNORED.with(|e| e.set(true));
    let out = f();
    CLIPS_IGNORED.with(|e| e.set(false));
    out
}

/// The flat colour of a region covered by `sources` (CMYK in a `cmyk` document); `None` where
/// nothing paints it.
fn region_fill(elems: &[Elem], sources: &[usize], o: &FlattenOptions, cmyk: bool) -> Option<RegionFill> {
    let mut c = Composer { sources, top: None, plane: 0 };
    let alone = c.eval(elems, CLEAR);
    if alone[3] < 0.5 / 255.0 {
        return None;
    }
    // The white goes under the finished art, as under the renderer's: blend modes read only the art.
    let on_page = |p: Px| if o.preserve_alpha { p } else { composite(BlendMode::Normal, WHITE, p) };
    let px = on_page(alone);
    let rgb = [px[0], px[1], px[2]];
    // The K plane composites the same way.
    let k = cmyk.then(|| {
        c.plane = 1;
        on_page(c.eval(elems, CLEAR))[0]
    });
    // A paint showing as it is keeps its colour (spot, process, swatch) and overprint.
    let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
    let same = |f: &Fill| px[3] >= 1.0 - 1e-4 && near(&f.planes[0], &rgb) && k.is_none_or(|k| near(&f.planes[1][..1], &[k]));
    let color = match k {
        Some(k) => {
            let [c, m, y, k] = planes_cmyk(rgb, k);
            Color::cmyk(c, m, y, k)
        }
        None => Color::rgb(rgb[0], rgb[1], rgb[2]),
    };
    let (paint, overprint) = match c.top {
        Some(f) if o.preserve_overprints && same(f) => (f.paint.clone(), f.overprint),
        _ => (Paint::solid(color), false),
    };
    Some(RegionFill { paint, alpha: px[3], overprint })
}

/// `art` rendered over `rect` at `ppi`: over white with the white the art doesn't cover taken out
/// again, or over nothing with Preserve Alpha. The image and
/// its `clip` reach a pixel further. An opaque image in a CMYK document holds the art's ink
/// amounts (a CMYK TIFF), so its CMYK colours keep their numbers (#2160); other images are PNGs.
/// Fails when the image can't be encoded.
fn render_raster(doc: &Document, art: &[Node], rect: Rect, ppi: f64, clip: Option<PathData>, o: &FlattenOptions) -> Result<Raster> {
    let mut scale = ppi / 72.0;
    scale = scale.min((MAX_PIXELS / (rect.width() * rect.height()).max(1.0)).sqrt());
    let px = 1.0 / scale;
    let rect = rect.inflate(px, px);
    let clip = clip.map(|c| Some(po::offset_path(&c, px, po::Join::Miter, 4.0)).filter(|g| !g.is_empty()).unwrap_or(c));
    let region = Rect::new(
        rect.x0,
        rect.y0,
        rect.x0 + (rect.width() * scale).ceil().max(1.0) / scale,
        rect.y0 + (rect.height() * scale).ceil().max(1.0) / scale,
    );
    let xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
    let tmp = isolated_doc(doc, art.to_vec());
    let mut r = vectorcraft_render::Renderer::new();
    let no_alpha = o.no_transparency.is_some() && clip.is_some() && !o.preserve_alpha;
    if no_alpha && doc.color_mode == ColorMode::Cmyk {
        // Drawn as inks over the paper: as screen colours, the file would separate them again,
        // and a CMYK colour comes back with other inks (K moves into C, M and Y).
        let opts = vectorcraft_render::RenderOptions { background: Some([255; 4]), skip_templates: true, ..Default::default() };
        let inks = r.render_region_inks(&tmp, region, scale, &opts);
        vectorcraft_doc::cancel::check()?;
        let (width, height) = vectorcraft_render::region_pixels(region, scale);
        // Past the size of a CMYK image: drawn in screen colours, below.
        if let Some(blob) = Inks::new(width, height, inks).and_then(|i| ImageBlob::cmyk_tiff(&i)) {
            return Ok(Raster { blob, width, height, xf, clip });
        }
    }
    let mut img = r.render_region(&tmp, region, scale, !o.preserve_alpha);
    // The drawing stops early when cancelled: this image is not whole.
    vectorcraft_doc::cancel::check()?;
    if no_alpha {
        // Without transparency the clip alone shapes the image: it is opaque, over white (its
        // edge pixels too).
        for p in img.pixels.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
    } else if !o.preserve_alpha || !o.anti_alias {
        // The art's coverage: drawn opaque, so only its own edges (and soft effects) are partial.
        let cover: Vec<Node> = art
            .iter()
            .map(|n| {
                let mut c = n.clone();
                opaque(&mut c);
                c
            })
            .collect();
        let cover = r.render_region(&isolated_doc(doc, cover), region, scale, false);
        for (px, c) in img.pixels.as_chunks_mut::<4>().0.iter_mut().zip(cover.pixels.as_chunks::<4>().0) {
            let k = c[3];
            let k2 = if o.anti_alias || k == 0 || k == 255 {
                k
            } else if k >= 128 {
                255
            } else {
                0
            };
            if o.preserve_alpha {
                let f = k2 as f32 / k.max(1) as f32;
                for v in px.iter_mut() {
                    *v = (*v as f32 * f).round().min(255.0) as u8;
                }
            } else {
                let white = 255 - k2;
                for v in &mut px[..3] {
                    *v = v.saturating_sub(white).min(k2);
                }
                px[3] = k2;
            }
        }
    }
    let png = img.to_png().map_err(|e| EngineError::Other(format!("Flatten Transparency: {e}")))?;
    Ok(Raster { blob: ImageBlob::new("image/png", png), width: img.width, height: img.height, xf, clip })
}

/// `n` at full opacity with Normal blending throughout (objects, fills and strokes).
pub(super) fn opaque(n: &mut Node) {
    n.opacity = 1.0;
    n.blend = BlendMode::Normal;
    for i in &mut n.appearance.items {
        match i {
            AppearanceItem::Fill(f) => (f.opacity, f.blend) = (1.0, BlendMode::Normal),
            AppearanceItem::Stroke(s) => (s.opacity, s.blend) = (1.0, BlendMode::Normal),
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            opaque(Arc::make_mut(c));
        }
    }
}

/// Whether `doc` has pattern fills whose image tiles need transparency flattening with `o`
/// ([`expand_transparent_tile_images`]).
pub(crate) fn has_transparent_tile_images(doc: &Document, o: &FlattenOptions) -> bool {
    let keys = o.no_transparency.clone().unwrap_or_default();
    let colorize = |d: &Document, n: &Node| o.counts_colorize(d, n);
    !doc.patterns.is_empty() && crate::cmd::expand::has_transparent_image_patterns(doc, &keys, &colorize)
}

/// Expand pattern fills whose image tiles need transparency flattening.
pub(crate) fn expand_transparent_tile_images(doc: &Document, o: &FlattenOptions) -> Result<Option<Document>> {
    if !has_transparent_tile_images(doc, o) {
        return Ok(None);
    }
    let keys = o.no_transparency.clone().unwrap_or_default();
    let colorize = |d: &Document, n: &Node| o.counts_colorize(d, n);
    let mut d = doc.clone();
    let layers: Vec<NodeId> =
        d.layers.iter().filter(|l| l.visible && !matches!(l.kind, NodeKind::Layer { template: true, .. })).map(|l| l.id).collect();
    let mut changed = 0;
    for id in layers {
        changed += crate::cmd::expand::expand_transparent_image_patterns(&mut d, id, &keys, &colorize)?;
    }
    Ok((changed > 0).then_some(d))
}

/// `doc` with everything it draws flattened with `o`, as formats without transparency (EPS)
/// write it: every visible object of the visible layers, locked ones too (a clipping layer's
/// clipping path stays). Pattern tile images with opacity, blending or a colorize paint that
/// `o` counts ([`FlattenOptions::counts_colorize`]) are expanded; see-through tile pixels are
/// expanded when `o.no_transparency` counts them. `None` when nothing changes.
pub(crate) fn flatten_document(doc: &Document, o: &FlattenOptions) -> Result<Option<Document>> {
    let expanded = expand_transparent_tile_images(doc, o)?;
    Ok(flatten_expanded(expanded.as_ref().unwrap_or(doc), o)?.or(expanded))
}

/// [`flatten_document`] for `doc` whose pattern tile images were expanded already
/// ([`expand_transparent_tile_images`]).
pub(crate) fn flatten_expanded(doc: &Document, o: &FlattenOptions) -> Result<Option<Document>> {
    let outlining = o.text_to_outlines || o.strokes_to_outlines || !o.preserve_overprints;
    if !outlining && !doc.layers.iter().any(|l| o.shows_transparency(doc, l) || o.draws_see_through_image(l)) {
        return Ok(None);
    }
    let mut roots = vec![];
    for l in doc.layers.iter().filter(|l| l.visible && !matches!(l.kind, NodeKind::Layer { template: true, .. })) {
        let children = l.children().map_or(&[][..], Vec::as_slice);
        roots.extend(children.iter().skip(usize::from(l.clips())).filter(|c| c.visible).map(|c| c.id));
    }
    let plan = plan(doc, &roots, o, true)?;
    if plan.flat.is_empty() && !plan.kept.iter().any(|id| doc.node(*id).is_some_and(|n| kept_work(n, o))) {
        return Ok(None);
    }
    let mut d = doc.clone();
    apply(&mut d, &mut vectorcraft_doc::Selection::default(), plan, o)?;
    Ok(Some(d))
}

// ---------- apply ----------

pub(super) fn apply(d: &mut Document, sel: &mut vectorcraft_doc::Selection, plan: Plan, o: &FlattenOptions) -> Result<(Vec<NodeId>, usize, usize)> {
    let (mut vector, mut rasterized) = (0, 0);
    let (mut ids, mut flat) = (vec![], vec![]);
    // The groups replace their objects in one pass at the end. When an object is listed twice or
    // lies inside another listed one, each group replaces its objects as it is made instead.
    let one_pass = separate(d, &plan.flat.iter().flat_map(|f| f.roots.iter().copied()).collect::<Vec<_>>());
    for f in plan.flat {
        // A group always has objects.
        let Some(top) = place_of(d, &f.roots) else { continue };
        let mut children = vec![];
        // The image goes under the regions, reaching a pixel past its own: their edges then fall on
        // matching pixels instead of on what is behind the group.
        if let Some(r) = f.raster {
            children.push(Arc::new(raster_node(d, r)));
            rasterized += 1;
        }
        // Type over several lines gives a group per line.
        let mut by_line: Vec<Vec<Arc<Node>>> = f.lines.as_ref().map_or(vec![], |l| vec![vec![]; l.bands.len()]);
        for (path, fill) in f.regions {
            let line = f.lines.as_ref().map(|l| l.line_of(&path));
            let mut n = shape_node(d, path, None);
            n.appearance = Appearance {
                items: vec![AppearanceItem::Fill(FillLayer { overprint: fill.overprint, ..FillLayer::new(fill.paint) })],
                ..Appearance::default()
            }
            .into();
            n.opacity = fill.alpha.min(1.0);
            match line.and_then(|i| by_line.get_mut(i)) {
                Some(l) => l.push(Arc::new(n)),
                None => children.push(Arc::new(n)),
            }
            vector += 1;
        }
        by_line.retain(|l| !l.is_empty());
        if by_line.len() > 1 {
            for l in by_line {
                children.push(Arc::new(Node::group(d.alloc_id(), l)));
            }
        } else {
            children.extend(by_line.into_iter().flatten());
        }
        let g = Node::group(d.alloc_id(), children);
        ids.push(g.id);
        if one_pass {
            // The group replaces the topmost object, and the other objects are removed.
            flat.extend(f.roots.iter().filter(|r| **r != top).map(|r| (*r, vec![])));
            flat.push((top, vec![g]));
        } else {
            let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
            d.insert(par, idx + 1, g)?;
            for r in &f.roots {
                d.remove(*r)?;
            }
        }
    }
    if !flat.is_empty() {
        d.insert_beside(flat, true)?;
    }
    let brushes = if o.strokes_to_outlines { vectorcraft_brush::library(d) } else { vec![] };
    for mut id in plan.kept {
        if o.text_to_outlines {
            id = outline_texts_under(d, id)?;
        }
        if o.strokes_to_outlines {
            id = outline_strokes_under(d, &brushes, id)?.0;
        }
        if !o.preserve_overprints
            && let Some(n) = d.node_mut(id)
        {
            drop_overprints(n);
        }
        ids.push(id);
    }
    ids.sort_by_key(|id| d.index_path(*id));
    sel.set(ids.iter().copied());
    Ok((ids, vector, rasterized))
}

/// The object of `roots` (in paint order) whose place their flattened group takes: the topmost one
/// that no clip group or clipping layer clips unless it clips every object of `roots`, else the
/// topmost one.
fn place_of(d: &Document, roots: &[NodeId]) -> Option<NodeId> {
    let top = roots.last().copied();
    if !clips_count() || roots.len() < 2 {
        return top;
    }
    let clips = |id: NodeId| -> Vec<NodeId> {
        let ancestry = d.ancestry(id).unwrap_or_default();
        ancestry.iter().rev().skip(1).filter(|a| d.node(**a).is_some_and(Node::clips)).copied().collect()
    };
    let all: Vec<Vec<NodeId>> = roots.iter().map(|r| clips(*r)).collect();
    let shared = |mine: &Vec<NodeId>| mine.iter().all(|c| all.iter().all(|theirs| theirs.contains(c)));
    roots.iter().zip(&all).rev().find(|(_, mine)| shared(mine)).map(|(r, _)| *r).or(top)
}

/// An image of `r` (in a clip group when it is clipped).
fn raster_node(d: &mut Document, r: Raster) -> Node {
    let key = unique_key(d, "flattened");
    d.images.insert(key.clone(), r.blob);
    let image = Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key,
            width: r.width,
            height: r.height,
            xf: r.xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let Some(path) = r.clip else { return image };
    crate::cmd::rasterfx::clip_group(d, path, FillRule::NonZero, image)
}

/// Replace the type under `root` by its outlines: → the id now standing for `root`.
fn outline_texts_under(d: &mut Document, root: NodeId) -> Result<NodeId> {
    let mut texts = vec![];
    if let Some(n) = d.node(root) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Text(_)) {
                texts.push(c.clone());
            }
        });
    }
    let mut out = root;
    for t in texts {
        let Some(o) = outlined_text(&t) else { continue };
        let o = d.reid(&o);
        let nid = o.id;
        *d.node_mut(t.id).ok_or(EngineError::NoNode(t.id))? = o;
        if t.id == root {
            out = nid;
        }
    }
    Ok(out)
}

/// Whether the outline and overprint options change `n`, an object without transparency.
pub(super) fn kept_work(n: &Node, o: &FlattenOptions) -> bool {
    let mut any = false;
    n.walk(&mut |c| {
        any |= (o.text_to_outlines && matches!(c.kind, NodeKind::Text(_)))
            || (o.strokes_to_outlines && stroked(c))
            || (!o.preserve_overprints && overprints(c));
    });
    any
}

/// Whether `n` is a path with a visible stroke (outlining turns it into a fill).
pub(super) fn stroked(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })
        && n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0))
}

fn overprints(n: &Node) -> bool {
    n.appearance.items.iter().any(AppearanceItem::overprint)
        || matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| r.style.overprint_fill || r.style.overprint_stroke))
}

fn drop_overprints(n: &mut Node) {
    if overprints(n) {
        for i in &mut n.appearance.items {
            *i.overprint_mut() = false;
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                (r.style.overprint_fill, r.style.overprint_stroke) = (false, false);
            }
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            drop_overprints(Arc::make_mut(c));
        }
    }
}
