use super::load::{err, raster_doc, raster_image};
use super::pdfimport::LoadOptions;
use crate::Result;
use crate::cmd::parse_range;
use kurbo::{Affine, BezPath, Point, Rect};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use vectorcraft_cdr::PathCmd;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, FillLayer, StrokeLayer};
use vectorcraft_doc::geom::FillRule;
use vectorcraft_doc::text::{AreaFit, CharStyle, TextKind, TextObject};
use vectorcraft_doc::{Artboard, Document, ImageObject, LayerColor, Node, NodeId, NodeKind};
use vectorcraft_geom::PathData;
use vectorcraft_pdf::{CropTo, TextAs};

pub(super) struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
    pub preview_only: bool,
}

#[derive(Clone, Copy)]
struct AlignedText {
    baseline: f64,
    font_size: f64,
}

pub(super) fn import(name: &str, bytes: &[u8], opts: &LoadOptions) -> Result<Imported> {
    match vectorcraft_cdr::read(bytes, vectorcraft_cdr::Limits::default()) {
        Ok(d) => {
            let mut warnings = d.warnings.clone();
            let ver = d.version.as_deref().unwrap_or("CorelDRAW");
            let num_pages = d.artboards.len().max(1);
            warnings.push(format!(
                "{ver}: imported document geometry ({} × {} pt, {} pages, {} layers, {} objects).",
                d.width.round(),
                d.height.round(),
                num_pages,
                d.layers.len(),
                d.objects.len()
            ));
            let mut doc = Document::new(d.width, d.height);

            // Determine which pages to import (range filtering)
            let total_pages = d.artboards.len().max(1);
            let page_indices: Vec<usize> = if let Some(range) = &opts.pages {
                parse_range(range, total_pages).map_err(|e| err(format!("pages: {e}")))?
            } else {
                (0..total_pages).collect()
            };

            // Populate multi-page artboards if present
            if !d.artboards.is_empty() {
                doc.artboards = page_indices
                    .iter()
                    .enumerate()
                    .filter_map(|(new_idx, &orig_idx)| {
                        d.artboards.get(orig_idx).map(|ab| Artboard {
                            id: (new_idx as u32) + 1,
                            name: ab.name.clone(),
                            rect: Rect::new(ab.x, ab.y, ab.x + ab.width, ab.y + ab.height),
                            locked: false,
                            show_center_mark: false,
                            show_cross_hairs: false,
                            show_video_safe_areas: false,
                            video_ruler_pixel_aspect_ratio: 1.0,
                            color: None,
                        })
                    })
                    .collect();
            }

            // Setup layers from CorelDRAW
            let mut layer_ids = Vec::new();
            if !d.layers.is_empty() {
                doc.layers.clear();
                for (i, layer_info) in d.layers.iter().enumerate() {
                    let id = doc.alloc_id();
                    let mut node = Node::layer(id, &layer_info.name, LayerColor::Preset((i as u8) % 9));
                    node.visible = layer_info.visible;
                    node.locked = layer_info.locked;
                    doc.layers.push(Arc::new(node));
                    layer_ids.push(id);
                }
            } else if let Some(first) = doc.layers.first() {
                layer_ids.push(first.id);
            }

            let default_layer = layer_ids.first().copied();

            // Pre-decode embedded bitmaps (`LIST [bmpt]`) so multiple instances share the same image key
            let mut decoded_bitmaps: HashMap<u32, (String, u32, u32)> = HashMap::new();
            for (&bmp_id, bmp) in &d.bitmaps {
                if let Ok(r) = raster_image(&bmp.bmp_bytes) {
                    doc.images.insert(r.key.clone(), r.blob);
                    decoded_bitmaps.insert(bmp_id, (r.key, r.width, r.height));
                }
            }

            let aligned_text = compute_aligned_text(&d);

            if !d.objects.is_empty() {
                insert_cdr_objects(&mut doc, &d, &layer_ids, default_layer, &decoded_bitmaps, &aligned_text, opts.text_as);
            } else if let Some(r) = vectorcraft_cdr::preview(bytes).ok().and_then(|p| raster_image(&p.data).ok()) {
                // Fallback to preview only if no vector objects exist
                let id = doc.alloc_id();
                let sx = d.width / (r.width as f64).max(1.0);
                let sy = d.height / (r.height as f64).max(1.0);
                let xf = Affine::scale_non_uniform(sx, sy);
                let mut n = Node::new(
                    id,
                    NodeKind::Image(Box::new(ImageObject {
                        key: r.key.clone(),
                        width: r.width,
                        height: r.height,
                        xf,
                        link: None,
                        placement: Default::default(),
                        pixels: None,
                    })),
                );
                n.name = Some(format!("{name} (rendered)"));
                doc.images.insert(r.key, r.blob);
                let _ = doc.insert(default_layer, 0, n);
            }

            apply_crop_options(&mut doc, opts.crop);

            Ok(Imported { doc, warnings, preview_only: false })
        }
        Err(native) => {
            let (doc, w) = preview(name, bytes, &native)?;
            Ok(Imported { doc, warnings: vec![w], preview_only: true })
        }
    }
}

/// Pre-align artistic text objects on the same line to share a consistent baseline and font size.
fn compute_aligned_text(d: &vectorcraft_cdr::Document) -> HashMap<usize, AlignedText> {
    let mut aligned_text: HashMap<usize, AlignedText> = HashMap::new();
    let mut page_text_indices: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, obj) in d.objects.iter().enumerate() {
        if obj.text.is_some() && !obj.is_area_text {
            page_text_indices.entry(obj.page_index).or_default().push(idx);
        }
    }

    for (_page, mut indices) in page_text_indices {
        // Sort by vertical center of bounding box
        indices.sort_by(|&a, &b| {
            let ya = d.objects[a].y + d.objects[a].height * 0.5;
            let yb = d.objects[b].y + d.objects[b].height * 0.5;
            ya.total_cmp(&yb)
        });

        // Cluster into lines: objects on the same line have vertical center distance <= 5.0 pt
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for idx in indices {
            let obj = &d.objects[idx];
            let yc = obj.y + obj.height * 0.5;
            if let Some(last_cluster) = clusters.last_mut() {
                let cluster_yc = d.objects[last_cluster[0]].y + d.objects[last_cluster[0]].height * 0.5;
                if (yc - cluster_yc).abs() <= 5.0 {
                    last_cluster.push(idx);
                    continue;
                }
            }
            clusters.push(vec![idx]);
        }

        for cluster in clusters {
            let mut baselines: Vec<f64> = Vec::new();
            let mut font_sizes: Vec<f64> = Vec::new();

            for &idx in &cluster {
                let obj = &d.objects[idx];
                let text = obj.text.as_deref().unwrap_or("");
                let is_dot = text == "." || text == "…";
                let has_caps = text.chars().any(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                let has_descender = text.chars().any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | 'ą' | 'ę' | ',' | ';'));

                if (has_caps || is_dot) && !has_descender {
                    baselines.push(obj.y + obj.height);
                }
                if !is_dot && obj.font_size >= 4.0 {
                    font_sizes.push(obj.font_size);
                }
            }

            font_sizes.sort_by(|a, b| a.total_cmp(b));
            let line_font_size = if !font_sizes.is_empty() { font_sizes[font_sizes.len() / 2] } else { d.objects[cluster[0]].font_size };

            baselines.sort_by(|a, b| a.total_cmp(b));
            let line_baseline = if !baselines.is_empty() {
                baselines[baselines.len() / 2]
            } else {
                let obj0 = &d.objects[cluster[0]];
                obj0.y + line_font_size
            };

            for &idx in &cluster {
                aligned_text.insert(idx, AlignedText { baseline: line_baseline, font_size: line_font_size });
            }
        }
    }
    aligned_text
}

/// Insert native vector, text, and bitmap objects into the document.
fn insert_cdr_objects(
    doc: &mut Document,
    d: &vectorcraft_cdr::Document,
    layer_ids: &[NodeId],
    default_layer: Option<NodeId>,
    decoded_bitmaps: &HashMap<u32, (String, u32, u32)>,
    aligned_text: &HashMap<usize, AlignedText>,
    text_as: TextAs,
) {
    for (obj_idx, obj) in d.objects.iter().enumerate() {
        let target_layer = layer_ids.get(obj.layer_index).copied().or(default_layer);
        let id = doc.alloc_id();
        if let Some(bmp_id) = obj.bitmap_id {
            if let Some((key, px_w, px_h)) = decoded_bitmaps.get(&bmp_id) {
                let sx = obj.width.max(1.0) / (*px_w as f64).max(1.0);
                let sy = obj.height.max(1.0) / (*px_h as f64).max(1.0);
                let xf = Affine::translate((obj.x, obj.y)) * Affine::scale_non_uniform(sx, sy);
                let mut n = Node::new(
                    id,
                    NodeKind::Image(Box::new(ImageObject {
                        key: key.clone(),
                        width: *px_w,
                        height: *px_h,
                        xf,
                        link: None,
                        placement: Default::default(),
                        pixels: None,
                    })),
                );
                n.name = Some(format!("Bitmap {bmp_id}"));
                let _ = doc.insert(target_layer, usize::MAX, n);
            }
        } else if let Some(text) = &obj.text {
            insert_text_object(doc, target_layer, obj, text, aligned_text.get(&obj_idx).copied(), text_as);
        } else {
            insert_path_object(doc, target_layer, id, obj);
        }
    }
}

fn insert_text_object(
    doc: &mut Document,
    target_layer: Option<NodeId>,
    obj: &vectorcraft_cdr::CdrObject,
    text: &str,
    aligned: Option<AlignedText>,
    text_as: TextAs,
) {
    let id = doc.alloc_id();
    let fill_color = obj.fill.map(|c| Color::Rgb { r: c.r, g: c.g, b: c.b }).unwrap_or(Color::Rgb { r: 0.0, g: 0.0, b: 0.0 });
    let (font_size, baseline) =
        if let Some(aligned) = aligned { (aligned.font_size, aligned.baseline) } else { (obj.font_size, obj.y + obj.font_size) };
    let style = CharStyle { size: font_size, fill: Paint::solid(fill_color), ..Default::default() };
    let to = if obj.is_area_text {
        let mut area_to = TextObject::point(Point::new(obj.x, obj.y), text, style);
        area_to.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, obj.width.max(10.0), obj.height.max(10.0))) };
        area_to.area.fit = AreaFit::ShrinkText { min_percent: 30.0 };
        area_to
    } else {
        TextObject::point(Point::new(obj.x, baseline), text, style)
    };

    if text_as == TextAs::Outlines {
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &to);
        let mut children = Vec::new();
        for g in &lay.glyphs {
            let path = PathData::from_bezpath(&g.outline).transformed(to.xf);
            if path.is_empty() {
                continue;
            }
            let st = to.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| to.first_style());
            let gid = doc.alloc_id();
            let mut gnode = Node::new(gid, NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false });
            gnode.appearance = st.appearance().into();
            children.push(Arc::new(gnode));
        }
        let mut n = Node::new(id, NodeKind::Group { children, clip: false });
        n.name = Some("<Text Outlines>".to_string());
        let _ = doc.insert(target_layer, usize::MAX, n);
    } else {
        let mut n = Node::new(id, NodeKind::Text(Box::new(to)));
        n.name = Some(text.lines().next().unwrap_or(text).to_string());
        let _ = doc.insert(target_layer, usize::MAX, n);
    }
}

fn insert_path_object(doc: &mut Document, target_layer: Option<NodeId>, id: NodeId, obj: &vectorcraft_cdr::CdrObject) {
    let pd = if !obj.path.is_empty() {
        let mut bp = BezPath::new();
        for &cmd in &obj.path {
            match cmd {
                PathCmd::MoveTo(x, y) => bp.move_to(Point::new(x, y)),
                PathCmd::LineTo(x, y) => bp.line_to(Point::new(x, y)),
                PathCmd::CubicTo(x1, y1, x2, y2, x3, y3) => {
                    bp.curve_to(Point::new(x1, y1), Point::new(x2, y2), Point::new(x3, y3));
                }
                PathCmd::ClosePath => bp.close_path(),
            }
        }
        PathData::from_bezpath(&bp)
    } else {
        let rect = Rect::new(obj.x, obj.y, obj.x + obj.width, obj.y + obj.height);
        vectorcraft_geom::shapes::rectangle(rect)
    };

    if pd.is_empty() {
        return;
    }

    let mut app = Appearance::default();
    if let Some(f) = obj.fill {
        let color = Color::Rgb { r: f.r, g: f.g, b: f.b };
        app.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(color))));
    }
    if let Some(s) = obj.stroke {
        let color = Color::Rgb { r: s.color.r, g: s.color.g, b: s.color.b };
        app.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(color), s.width)));
    }

    let mut n = Node::new(id, NodeKind::Path { path: pd, rule: FillRule::EvenOdd, live: None, clipping: false, guide: false });
    n.appearance = app.into();
    let _ = doc.insert(target_layer, usize::MAX, n);
}

/// Apply CropTo box option if requested.
fn apply_crop_options(doc: &mut Document, crop: CropTo) {
    match crop {
        CropTo::Bounding | CropTo::Art => {
            for ab in &mut doc.artboards {
                let mut bbox: Option<Rect> = None;
                for layer in &doc.layers {
                    if let Some(children) = layer.children() {
                        for child in children {
                            if let Some(bounds) = child.geometric_bounds()
                                && (ab.rect.contains(bounds.origin()) || ab.rect.intersect(bounds).area() > 0.0)
                            {
                                bbox = Some(bbox.map_or(bounds, |b| b.union(bounds)));
                            }
                        }
                    }
                }
                if let Some(b) = bbox {
                    ab.rect = b;
                }
            }
        }
        CropTo::Trim => {}
        CropTo::Bleed => {
            for ab in &mut doc.artboards {
                ab.rect = ab.rect.inset(-9.0);
            }
        }
        CropTo::Crop | CropTo::Media => {}
    }
}

/// The embedded preview, for files whose native document can't be read.
fn preview(name: &str, bytes: &[u8], native: &vectorcraft_cdr::Error) -> Result<(Document, String)> {
    let p = vectorcraft_cdr::preview(bytes).map_err(|e| err(format!("{native}; its embedded preview can't be read either ({e})")))?;

    let mut reader = image::ImageReader::new(Cursor::new(&p.data));
    reader = reader.with_guessed_format().map_err(err)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    let _pixels = reader.decode().map_err(err)?;
    let d = raster_doc(name, &p.data)?;
    Ok((d, format!("CorelDRAW: {native}; opened its embedded preview ({}x{} pixels).", p.width, p.height)))
}
