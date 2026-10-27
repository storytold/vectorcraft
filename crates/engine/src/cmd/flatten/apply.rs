use super::analysis::{Scratch, outlined_text, overlapping, reach};
use super::compose::{Builder, CLEAR, Composer, Elem, Fill, Kind, WHITE};
use super::options::FlattenOptions;
use super::*;

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
    let reaches: Vec<Option<Rect>> = plain.iter().map(reach).collect();
    let see_through: Vec<bool> = plain.iter().map(|n| n.shows_transparency() || o.draws_see_through_image(n)).collect();
    let cmyk = src.color_mode == ColorMode::Cmyk;
    let mut out = Plan { flat: vec![], kept: vec![], transparent: roots.iter().zip(&see_through).filter(|(_, t)| **t).map(|(id, _)| *id).collect() };
    // Baking above stopped early when cancelled; each group is a place to stop (a plan changes nothing).
    vectorcraft_doc::cancel::check()?;
    for g in overlapping(&reaches) {
        vectorcraft_doc::cancel::check()?;
        let ids: Vec<NodeId> = g.iter().map(|&i| roots[i]).collect();
        let group = g.iter().any(|&i| see_through[i]).then(|| {
            let plain: Vec<&Node> = g.iter().map(|&i| &plain[i]).collect();
            flatten_group(&plain, o, cmyk)
        });
        let Some((regions, area)) = group.flatten() else {
            out.kept.extend(ids);
            continue;
        };
        let raster = match area.as_ref().filter(|_| render) {
            Some(a) => {
                let art: Vec<Node> = g.iter().map(|&i| art[i].clone()).collect();
                Some(render_raster(&baked, &art, a.rect, a.ppi, a.clip.clone(), o)?)
            }
            None => None,
        };
        out.flat.push(Flat { roots: ids, regions, area, raster });
    }
    Ok(out)
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
    let regions = if whole { vec![] } else { po::regions(&b.shapes) };
    // An arrangement that failed (degenerate geometry) leaves only rasterizing.
    let all = regions.is_empty() || o.rasterize_all(regions.len());
    let mut vector: Vec<(usize, RegionFill)> = vec![];
    let mut complex: Vec<usize> = vec![];
    let mut gradients_only = !all;
    for (k, r) in regions.iter().enumerate() {
        if all || r.sources.iter().any(|&i| b.kinds[i] != Kind::Vector) {
            gradients_only &= r.sources.iter().all(|&i| b.kinds[i] == Kind::Gradient);
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

/// The flat colour of a region covered by `sources` (CMYK in a `cmyk` document); `None` where
/// nothing paints it.
fn region_fill(elems: &[Elem], sources: &[usize], o: &FlattenOptions, cmyk: bool) -> Option<RegionFill> {
    let mut c = Composer { sources, top: None, plane: 0 };
    let alone = c.eval(elems, CLEAR);
    if alone[3] < 0.5 / 255.0 {
        return None;
    }
    let start = if o.preserve_alpha { CLEAR } else { WHITE };
    let px = if o.preserve_alpha {
        alone
    } else {
        c.top = None;
        c.eval(elems, start)
    };
    let rgb = [px[0], px[1], px[2]];
    // The K plane composites the same way.
    let k = cmyk.then(|| {
        c.plane = 1;
        c.eval(elems, start)[0]
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
/// again (so blend modes see the white page), or over nothing with Preserve Alpha. The image and
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

/// `doc` with everything it draws flattened with `o`, as formats without transparency (EPS)
/// write it: every visible object of the visible layers, locked ones too (a clipping layer's
/// clipping path stays). `None` when that changes nothing.
pub(crate) fn flatten_document(doc: &Document, o: &FlattenOptions) -> Result<Option<Document>> {
    let outlining = o.text_to_outlines || o.strokes_to_outlines || !o.preserve_overprints;
    if !outlining && !doc.layers.iter().any(|l| l.shows_transparency() || o.draws_see_through_image(l)) {
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
        let Some(&top) = f.roots.last() else { continue };
        let mut children = vec![];
        // The image goes under the regions, reaching a pixel past its own: their edges then fall on
        // matching pixels instead of on what is behind the group.
        if let Some(r) = f.raster {
            children.push(Arc::new(raster_node(d, r)));
            rasterized += 1;
        }
        for (path, fill) in f.regions {
            let mut n = shape_node(d, path, None);
            n.appearance = Appearance {
                items: vec![AppearanceItem::Fill(FillLayer { overprint: fill.overprint, ..FillLayer::new(fill.paint) })],
                ..Appearance::default()
            }
            .into();
            n.opacity = fill.alpha.min(1.0);
            children.push(Arc::new(n));
            vector += 1;
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
