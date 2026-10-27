use super::*;

// ---------- Image Trace ----------

fn is_trace_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(TRACE_NAME)
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

/// The `view` param ([`TraceView`] id), if given.
fn view_param(cmd: &str, p: &Value) -> Result<Option<TraceView>> {
    let Some(v) = p.get("view").filter(|v| !v.is_null()) else { return Ok(None) };
    let ids = || TraceView::ALL.map(TraceView::id).join(", ");
    v.as_str().and_then(TraceView::from_id).map(Some).ok_or_else(|| bad(cmd, format!("view must be one of {}", ids())))
}

/// The parameters command `c` traces with: the preset's, with `params` over them.
fn trace_params(c: &str, p: &Value) -> Result<tr::TraceParams> {
    let name = str_param(p, "preset").unwrap_or("Default");
    let enhanced = switch_or(p, "enhanced", false, c)?;
    // (A preset without an enhanced version is traced as it is.)
    let base = enhanced.then(|| tr::enhanced_preset(name)).flatten().or_else(|| tr::preset(name));
    let base = base.ok_or_else(|| bad(c, format!("unknown preset `{name}` (see imageTrace.presets)")))?;
    let Some(over) = p.get("params").filter(|v| !v.is_null()) else { return Ok(base) };
    let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
    let (Some(o), Some(src)) = (v.as_object_mut(), over.as_object()) else { return Err(bad(c, "params must be an object")) };
    // A key that isn't one of the settings is an error, with the nearest setting.
    if let Some(key) = src.keys().find(|k| !o.contains_key(k.as_str())) {
        return Err(crate::cmd::params::unknown(c, key, &o.keys().map(String::as_str).collect::<Vec<_>>()));
    }
    for (k, val) in src {
        o.insert(k.clone(), val.clone());
    }
    let params: tr::TraceParams = serde_json::from_value(v).map_err(|e| bad(c, e.to_string()))?;
    if !params.fills && !params.strokes {
        return Err(bad(c, "Create needs fills, strokes or both"));
    }
    if !(params.stroke_width.is_finite() && params.stroke_width > 0.0) {
        return Err(bad(c, "strokeWidth must be a positive number of pixels"));
    }
    // A color group is refused unless the trace uses it: Color mode with Document Library and the
    // document's swatches.
    if params.color_group.as_deref().is_some_and(|g| !g.is_empty()) {
        if params.palette != tr::Palette::DocumentLibrary {
            return Err(bad(c, "`colorGroup` needs `palette: \"documentLibrary\"`"));
        }
        if params.library != crate::cmd::swatchlib::DOCUMENT_SWATCHES {
            return Err(bad(c, "`colorGroup` needs `library: \"document\"`"));
        }
        if params.mode != tr::Mode::Color {
            return Err(bad(c, "`colorGroup` needs `mode: \"color\"`"));
        }
    }
    Ok(params)
}

/// Document Library with `colorGroup`: the solid colors of the active document's color group
/// `name` (so named exactly, else ignoring case). Errors name command `c`.
/// ([`trace_params`] has checked that the library is the document's swatches.)
fn trace_group_colors(c: &str, s: &Session, name: &str) -> Result<Vec<Color>> {
    let groups = &s.doc()?.doc.swatch_groups;
    let lower = name.to_lowercase();
    let group = groups
        .iter()
        .find(|g| g.name == name)
        .or_else(|| groups.iter().find(|g| g.name.to_lowercase() == lower))
        .ok_or_else(|| bad(c, format!("no color group `{name}` in the document (see swatch.list)")))?;
    Ok(group.swatches.iter().filter_map(|w| w.paint.color()).collect())
}

/// Document Library in Color mode: look the library's colors (or its color group's) up into
/// `params.swatches` → each one as traced (sRGB) with the color to fill with. Errors name command `c`.
fn trace_library(c: &str, s: &Session, params: &mut tr::TraceParams) -> Result<Vec<([u8; 3], Color)>> {
    if params.mode != tr::Mode::Color || params.palette != tr::Palette::DocumentLibrary {
        return Ok(vec![]);
    }
    let group = params.color_group.as_deref().filter(|g| !g.is_empty());
    let key = group.unwrap_or(params.library.as_str());
    let colors = match group {
        Some(name) => trace_group_colors(c, s, name)?,
        None => crate::cmd::swatchlib::library_colors(s, key)
            .ok_or_else(|| bad(c, format!("no swatch library `{key}` (see swatch.library.list; \"document\": the document's swatches)")))?,
    };
    if colors.is_empty() {
        return Err(bad(c, format!("`{key}` has no colours")));
    }
    let library: Vec<([u8; 3], Color)> = colors
        .into_iter()
        .map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            ([r, g, b], c)
        })
        .collect();
    params.swatches = library.iter().map(|l| l.0).collect();
    Ok(library)
}

/// The most anchor points an Image Trace may make, a guard for memory: a trace past it is refused
/// with advice instead (#525). A trace keeps about 90 bytes an anchor in the document (measured on
/// code-drawn photos of 2000 × 2000 to 6000 × 4000 pixels), so 8,000,000 anchors are about
/// 0.7 GB, with a peak of about 1 GB while tracing: a 4000 × 3000 photo at High Fidelity Photo
/// makes about 1.9 million, and even at Paths 100, Noise 1 and every tone about 7.9 million. On the
/// web, where a page has at most 4 GB of memory, the limit stays at 2,000,000.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_TRACE_ANCHORS: usize = 8_000_000;

/// The most anchor points an Image Trace may make on the web (see the native limit).
#[cfg(target_arch = "wasm32")]
pub const MAX_TRACE_ANCHORS: usize = 2_000_000;

/// The live shape for a shape the tracer found (in the image's pixels), placed in the document by `xf`.
fn traced_shape(shape: tr::TracedShape, xf: vectorcraft_geom::Affine) -> LiveShape {
    use vectorcraft_geom::Affine;
    let about = |c: Point, deg: f64| Affine::translate(c.to_vec2()) * Affine::rotate(deg.to_radians());
    let mut live = match shape {
        tr::TracedShape::Rectangle { center, width, height, radius, rotation } => LiveShape::Rectangle {
            w: width,
            h: height,
            radii: [radius; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: about(center, rotation) * Affine::translate((-width / 2.0, -height / 2.0)),
        },
        tr::TracedShape::Ellipse { center, rx, ry, rotation } => {
            LiveShape::Ellipse { w: 2.0 * rx, h: 2.0 * ry, pie: (0.0, 360.0), xf: about(center, rotation) * Affine::translate((-rx, -ry)) }
        }
        tr::TracedShape::Polygon { center, radius, sides, rotation } => {
            LiveShape::Polygon { radius, sides, xf: about(center, rotation), radii: vec![], kinds: vec![], radii_relative: false }
        }
    };
    // (Whether the move changed the shape's own numbers does not matter: its path is made from them next.)
    let _ = live.transform(xf);
    live
}

/// The gradient fill for a linear gradient the tracer found (in the image's pixels), placed in the
/// document by `xf`.
fn traced_gradient(g: &tr::TracedGradient, xf: vectorcraft_geom::Affine) -> Paint {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = g.stops.iter().map(|&(at, c)| GradientStop::new(at as f32, Color::rgb8(c[0], c[1], c[2]))).collect();
    let mut paint = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    paint.geom = Some(GradientGeom { start: xf * g.start, end: xf * g.end, aspect: 1.0, focal: None });
    Paint::Gradient(Box::new(paint))
}

/// Auto Grouping: `nodes` (one per traced path, bottom-most first) with each path that has others
/// inside it (`parents`, as the tracer gives them) in a group with them, in the order they had:
/// abutting paths painted in another order would show a seam where they meet. Without parents,
/// the nodes as they are.
fn grouped(d: &mut Document, nodes: Vec<Node>, parents: &[Option<usize>]) -> Vec<Arc<Node>> {
    let n = nodes.len();
    let mut inside: Vec<Vec<usize>> = vec![vec![]; n];
    let mut roots = vec![];
    for i in 0..n {
        match parents.get(i).copied().flatten().and_then(|p| inside.get_mut(p).filter(|_| p != i)) {
            Some(of) => of.push(i),
            None => roots.push(i),
        }
    }
    // Outermost first, so that backwards each path comes after those inside it.
    let mut order = roots.clone();
    let mut at = 0;
    while let Some(&i) = order.get(at) {
        order.extend(inside.get(i).into_iter().flatten());
        at += 1;
    }
    let mut nodes: Vec<Option<Node>> = nodes.into_iter().map(Some).collect();
    let mut built: Vec<Option<Arc<Node>>> = vec![None; n];
    for &i in order.iter().rev() {
        let Some(node) = nodes.get_mut(i).and_then(Option::take) else { continue };
        let mut within: Vec<(usize, Arc<Node>)> = inside.get(i).into_iter().flatten().filter_map(|&c| Some((c, built.get_mut(c)?.take()?))).collect();
        let made = if within.is_empty() {
            node
        } else {
            within.push((i, Arc::new(node)));
            within.sort_by_key(|m| m.0);
            Node::group(d.alloc_id(), within.into_iter().map(|m| m.1).collect())
        };
        if let Some(slot) = built.get_mut(i) {
            *slot = Some(Arc::new(made));
        }
    }
    // (Paths that are inside each other in a ring are reached from no root: the tracer makes none,
    // and they are kept all the same.)
    let ring = nodes.into_iter().flatten().map(Arc::new).collect::<Vec<_>>();
    roots.iter().filter_map(|&r| built.get_mut(r)?.take()).chain(ring).collect()
}

pub(super) fn trace_make(s: &mut Session, p: &Value, expand: bool) -> Result<Value> {
    let c = if expand { "imageTrace.makeAndExpand" } else { "imageTrace.make" };
    let mut params = trace_params(c, p)?;
    let view = view_param(c, p)?;
    let params_json = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    // The preset's name if the parameters are exactly that preset's or its enhanced version's,
    // else "Custom".
    let named = str_param(p, "preset").unwrap_or("Default");
    let is = |b: Option<tr::TraceParams>| b.and_then(|b| serde_json::to_value(&b).ok()).as_ref() == Some(&params_json);
    let preset_label = if is(tr::preset(named)) || is(tr::enhanced_preset(named)) { named.to_string() } else { "Custom".to_string() };
    let roots = root_ids(s, p)?;
    // The image (inside a trace group, or selected directly).
    let (target, img) = {
        let d = &s.doc()?.doc;
        roots
            .iter()
            .find_map(|r| {
                let n = d.node(*r)?;
                match &n.kind {
                    NodeKind::Image(i) => Some((*r, i.clone())),
                    _ if is_trace_group(n) => match &n.children()?.first()?.kind {
                        NodeKind::Image(i) => Some((*r, i.clone())),
                        _ => None,
                    },
                    _ => None,
                }
            })
            .ok_or_else(|| bad(c, "select an image (or an Image Trace group)"))?
    };
    let blob = s.doc()?.doc.images.get(&img.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", img.key)))?;
    // An image over the tracer's size is traced scaled down (the paths are scaled back below).
    let raster = tr::Raster::decode_to_fit(&blob.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let library = trace_library(c, s, &mut params)?;
    let res = tr::trace_within(&raster, &params, MAX_TRACE_ANCHORS).map_err(|e| EngineError::Other(e.to_string()))?;
    let warnings = trace_warnings((img.width, img.height), (raster.width, raster.height), res.worked_at);
    // Pixel space of the decoded raster → the image object's pixel space → document.
    let sx = img.width.max(1) as f64 / raster.width.max(1) as f64;
    let sy = img.height.max(1) as f64 / raster.height.max(1) as f64;
    let xf = img.xf * vectorcraft_geom::Affine::scale_non_uniform(sx, sy);
    // Document units per raster pixel (stroke widths).
    let unit = xf.determinant().abs().sqrt();
    let (npaths, anchors, colors) = (res.paths.len(), res.anchor_count(), res.palette.len());
    let label = if expand { "Image Trace (Make and Expand)" } else { "Image Trace" };
    let id = s.edit(label, |d, sel| {
        let src = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        let mut image = if is_trace_group(&src) {
            src.children()
                .and_then(|c| c.first())
                .map(|c| (**c).clone())
                .ok_or_else(|| EngineError::Other("Image Trace: the traced image is missing".into()))?
        } else {
            src.clone()
        };
        image.visible = true;
        let mut children: Vec<Arc<Node>> = vec![];
        if !expand {
            let mut hidden = image.clone();
            hidden.visible = false;
            children.push(Arc::new(hidden));
        }
        let mut traced = Vec::with_capacity(res.paths.len());
        for tp in res.paths {
            let path = tp.path.transformed(xf);
            let mut n = shape_node(d, path, None);
            // Shapes: a path the tracer found to be a shape is that live shape.
            if let (Some(shape), NodeKind::Path { path, live, .. }) = (tp.shape, &mut n.kind) {
                let shape = traced_shape(shape, xf);
                *path = shape.to_path();
                *live = Some(Box::new(shape));
            }
            // A library colour fills with the swatch's own colour (its model kept).
            let color = library.iter().find(|(rgb, _)| *rgb == tp.color).map_or_else(|| Color::rgb8(tp.color[0], tp.color[1], tp.color[2]), |l| l.1);
            // Gradients: a path the tracer found to be a linear gradient is filled with it.
            let paint = tp.gradient.as_ref().map_or_else(|| Paint::solid(color), |g| traced_gradient(g, xf));
            n.appearance = match tp.stroke {
                // Traced lines: round corners, and ends round or square as the tracer found them.
                Some(width) => Appearance {
                    items: vec![AppearanceItem::Stroke(StrokeLayer {
                        cap: if tp.square_caps { LineCap::Square } else { LineCap::Round },
                        join: LineJoin::Round,
                        ..StrokeLayer::new(paint, width * unit)
                    })],
                    ..Default::default()
                }
                .into(),
                None => fill_only(paint).into(),
            };
            traced.push(n);
        }
        children.extend(grouped(d, traced, &res.parents));
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = if expand { None } else { Some(TRACE_NAME.into()) };
        if !expand {
            // Tracing again keeps the object's view unless another is asked for.
            let view = view.unwrap_or_else(|| src.trace_view());
            g.extras_mut().trace = Some(Box::new(json!({ "preset": preset_label, "params": params_json, "view": view.id() })));
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "paths": npaths, "anchors": anchors, "colors": colors, "warnings": warnings }))
}

/// Images larger than this on a side are traced, but more slowly: the trace says so.
pub const LARGE_TRACE_SIDE: u32 = 1500;

/// What a trace of an image of `size` pixels, traced at `traced` pixels, tells the user: that
/// Flat Logo averaged it down to `worked` pixels first, or else that a large image takes longer
/// (the public documentation's warning for images over 1500 px) and that one over the tracer's
/// [`tr::MAX_PIXELS`] was traced scaled down.
pub(crate) fn trace_warnings(size: (u32, u32), traced: (u32, u32), worked: Option<(u32, u32)>) -> Vec<String> {
    let mut out = vec![];
    if worked.is_none() && size.0.max(size.1) > LARGE_TRACE_SIDE {
        out.push(format!(
            "The image is {} × {} pixels, more than {LARGE_TRACE_SIDE} on a side: it is traced, but more slowly. A lower-fidelity preset or a smaller copy of the image traces faster.",
            size.0, size.1
        ));
    }
    if let Some(worked) = worked {
        out.push(format!("Flat Logo traced the image at {} × {} pixels, averaged down from {} × {}.", worked.0, worked.1, size.0, size.1));
    } else if u64::from(traced.0) * u64::from(traced.1) < u64::from(size.0) * u64::from(size.1) {
        out.push(format!(
            "The image was traced at {} × {} pixels, scaled down from {} × {} ({} megapixels at most).",
            traced.0,
            traced.1,
            size.0,
            size.1,
            tr::MAX_PIXELS >> 20
        ));
    }
    out
}

fn trace_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let v: Vec<NodeId> = root_ids(s, p)?.into_iter().filter(|i| d.node(*i).is_some_and(is_trace_group)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select an Image Trace object".into()));
    }
    Ok(v)
}

pub(super) fn trace_set_view(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.setView";
    let view = view_param(C, p)?.ok_or_else(|| bad(C, "view is required"))?;
    let targets = trace_targets(s, p)?;
    let d = &s.doc()?.doc;
    if targets.iter().all(|g| d.node(*g).is_some_and(|n| n.trace_view() == view)) {
        return Ok(json!({ "ids": ids_json(&targets), "view": view.id() }));
    }
    s.edit("Image Trace View", |d, _| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            if let Some(t) = n.extras_if_any().and_then(|e| e.trace.as_deref_mut()).and_then(Value::as_object_mut) {
                t.insert("view".into(), json!(view.id()));
            } else {
                n.extras_mut().trace = Some(Box::new(json!({ "preset": "Custom", "view": view.id() })));
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets), "view": view.id() }))
}

pub(super) fn trace_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    let ids = s.edit("Release Image Trace", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &targets);
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut img = (**n.children().and_then(|c| c.first()).ok_or(EngineError::NoNode(*g))?).clone();
            img.visible = true;
            out.push(img.id);
            released.put(d, *g, vec![img])?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn trace_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    s.edit("Expand Image Trace", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.extras_mut().trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| !matches!(c.kind, NodeKind::Image(_)));
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

#[cfg(test)]
mod trace_limit_tests {
    use super::{MAX_TRACE_ANCHORS, trace_warnings};

    #[test]
    fn trace_warnings_name_a_large_or_scaled_down_image() {
        assert!(trace_warnings((1500, 1500), (1500, 1500), None).is_empty());
        assert_eq!(trace_warnings((1501, 10), (1501, 10), None).len(), 1);
        let w = trace_warnings((16000, 12000), (9459, 7094), None);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("16000 × 12000") && w[1].contains("9459 × 7094") && w[1].contains("64 megapixels"), "{w:?}");
        // Flat Logo's own working size is named instead of the megapixel limit.
        let w = trace_warnings((1025, 1025), (1025, 1025), Some((513, 513)));
        assert_eq!(w, ["Flat Logo traced the image at 513 × 513 pixels, averaged down from 1025 × 1025."]);
        // Flat Logo traces a large image at its reduced size, so it gives no slow-trace warning.
        let w = trace_warnings((1600, 1600), (1600, 1600), Some((800, 800)));
        assert_eq!(w, ["Flat Logo traced the image at 800 × 800 pixels, averaged down from 1600 × 1600."]);
        let w = trace_warnings((16000, 12000), (9459, 7094), Some((946, 710)));
        assert_eq!(w, ["Flat Logo traced the image at 946 × 710 pixels, averaged down from 16000 × 12000."]);
    }

    // A 4000 × 3000 photo at High Fidelity Photo makes about 1.9 million anchors, which must fit;
    // the limit still guards memory (about 90 bytes an anchor). Checked when the tests build.
    const _: () = assert!(MAX_TRACE_ANCHORS >= 2_000_000 && MAX_TRACE_ANCHORS <= 16_000_000);
}
