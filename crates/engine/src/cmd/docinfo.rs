//! Document Info panel data, its categories and the text report (Document Info › Save…, File →
//! Package), and Object → Make Pixel Perfect.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{ColorMode, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_text::embed::Embedding;

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "document.info",
            "Document Info",
            ["Window", "Document Info"],
            None,
            "{selectionOnly?, category?: document|objects|graphicStyles|spotColors|patterns|gradients|symbols|fonts|fontDetails|linkedImages|embeddedImages (only that one in sections and the text), format?: \"text\" (→ {text}: the report Document Info › Save… and Package write)} → {document, objects: {paths, compoundPaths, groups, …}, fonts, images, swatches, graphicStyleNames (with selectionOnly: the styles the selected objects are linked to), …, sections: [{id, title, rows: [[label, value]]}] (the panel's categories; fontDetails: each font's file and whether its licence lets it be embedded)}",
            has_doc,
            info
        ),
        cmd!(
            "object.makePixelPerfect",
            "Make Pixel Perfect",
            ["Object"],
            None,
            "{ids?} snap each object's edges to the pixel grid (odd stroke weights on half pixels) and round stroke weights",
            has_selection,
            pixel_perfect
        ),
    ]
}

/// Document Info's categories: (id, title), in the order the panel and the report show them.
pub const CATEGORIES: [(&str, &str); 11] = [
    ("document", "Document"),
    ("objects", "Objects"),
    ("graphicStyles", "Graphic Styles"),
    ("spotColors", "Spot Colors"),
    ("patterns", "Pattern Objects"),
    ("gradients", "Gradient Swatches"),
    ("symbols", "Symbols"),
    ("fonts", "Fonts"),
    ("fontDetails", "Font Details"),
    ("linkedImages", "Linked Images"),
    ("embeddedImages", "Embedded Images"),
];

/// The object counts of `document.info` and their labels.
const OBJECT_LABELS: [(&str, &str); 17] = [
    ("paths", "Paths"),
    ("compoundPaths", "Compound Paths"),
    ("groups", "Groups"),
    ("clipGroups", "Clipping Masks"),
    ("textObjects", "Text Objects"),
    ("images", "Images"),
    ("placedDocuments", "Placed Documents"),
    ("symbolInstances", "Symbol Instances"),
    ("gradients", "Gradient Objects"),
    ("patterns", "Pattern Objects"),
    ("meshes", "Gradient Meshes"),
    ("blends", "Blends"),
    ("envelopes", "Envelopes"),
    ("repeats", "Repeats"),
    ("opacityMasks", "Opacity Masks"),
    ("liveEffects", "Objects with Effects"),
    ("guides", "Guides"),
];

/// One category of Document Info: its id, title and `(label, value)` rows (a list item has an
/// empty value).
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub rows: Vec<(String, String)>,
}

/// What a font's `fsType` lets a copy of it do, read as exports read it ([`Embedding`]): the
/// most permissive usage bit wins, and bitmap-only fonts may not be embedded.
pub fn embedding_label(fs_type: u16) -> &'static str {
    if Embedding::from_fs_type(fs_type) == Embedding::Forbidden {
        "embedding not allowed"
    } else if fs_type & 0x0008 != 0 {
        "embedding for editing"
    } else if fs_type & 0x0004 != 0 {
        "embedding for preview and print"
    } else {
        "embedding allowed"
    }
}

/// An image object as Document Info lists it: (name, width, height, linked file).
type ImageRow = (String, u32, u32, Option<String>);

/// The Document Info categories of `d` (only `category` when given) from `info` (what
/// `document.info` reports), its fonts `(family, style)`, image objects and used patterns.
fn sections(d: &Document, info: &Value, fonts: &BTreeSet<(String, String)>, images: &[ImageRow], category: Option<&str>) -> Vec<Section> {
    let names = |k: &str| -> Vec<(String, String)> {
        info[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(|n| (n.to_string(), String::new())).collect()
    };
    let u = d.units;
    CATEGORIES
        .iter()
        .filter(|(id, _)| category.is_none_or(|c| c == *id))
        .map(|&(id, title)| {
            let rows = match id {
                "document" => {
                    let mut rows = vec![
                        ("Name".to_string(), info["document"]["name"].as_str().unwrap_or_default().to_string()),
                        ("Color Mode".into(), info["document"]["colorMode"].as_str().unwrap_or_default().into()),
                        ("Units".into(), u.label().into()),
                        ("Artboards".into(), d.artboards.len().to_string()),
                    ];
                    rows.extend(
                        d.artboards.iter().map(|a| (a.name.clone(), format!("{} × {}", u.format(a.rect.width()), u.format(a.rect.height())))),
                    );
                    rows.push(("Raster Effects".into(), format!("{} ppi", d.raster_effects_ppi)));
                    let counts = [
                        ("Swatches", "swatches"),
                        ("Character Styles", "characterStyles"),
                        ("Paragraph Styles", "paragraphStyles"),
                        ("Pattern Swatches", "patterns"),
                    ];
                    rows.extend(counts.map(|(label, k)| (label.to_string(), info[k].to_string())));
                    rows
                }
                "objects" => {
                    OBJECT_LABELS.iter().filter_map(|(k, label)| Some((label.to_string(), info["objects"][k].as_u64()?.to_string()))).collect()
                }
                "graphicStyles" => names("graphicStyleNames"),
                "spotColors" => names("spotColors"),
                "patterns" => names("patternNames"),
                "gradients" => d.swatches_iter().filter(|s| matches!(s.paint, Paint::Gradient(_))).map(|s| (s.name.clone(), String::new())).collect(),
                "symbols" => names("symbols"),
                "fonts" => fonts.iter().map(|(f, s)| (format!("{f} {s}"), String::new())).collect(),
                "fontDetails" => {
                    let db = vectorcraft_text::FontDb::global();
                    fonts
                        .iter()
                        .map(|(family, style)| {
                            // Found by any of its names; a style the family lacks shows in another.
                            let detail = match db.resolve(family, style) {
                                Some((f, m)) if m != vectorcraft_text::FontMatch::Missing => {
                                    let file =
                                        f.path().and_then(|p| p.file_name()).map_or_else(|| "built in".into(), |n| n.to_string_lossy().into_owned());
                                    let found = format!("{file}; {}", embedding_label(f.fs_type()));
                                    if m == vectorcraft_text::FontMatch::Style {
                                        format!("substituted: shown in {} {} ({found})", f.family, f.style)
                                    } else {
                                        found
                                    }
                                }
                                Some((f, _)) => format!("missing: shown in {} {}", f.family, f.style),
                                None => "missing".into(),
                            };
                            (format!("{family} {style}"), detail)
                        })
                        .collect()
                }
                "linkedImages" => {
                    images.iter().filter_map(|(name, w, h, link)| Some((name.clone(), format!("{w} × {h} px — {}", link.as_ref()?)))).collect()
                }
                _ => images.iter().filter(|i| i.3.is_none()).map(|(name, w, h, _)| (name.clone(), format!("{w} × {h} px"))).collect(),
            };
            Section { id, title, rows }
        })
        .collect()
}

/// `sections` as the plain-text report of document `name`: a heading, then each category's title
/// and rows.
pub fn report_text(name: &str, selection_only: bool, sections: &[Section]) -> String {
    let mut out = format!("Document Info: {name}\n");
    if selection_only {
        out.push_str("(the selection only)\n");
    }
    for sec in sections {
        out.push_str(&format!("\n{}\n", sec.title.to_uppercase()));
        if sec.rows.is_empty() {
            out.push_str("None\n");
        }
        for (label, value) in &sec.rows {
            if value.is_empty() {
                out.push_str(&format!("{label}\n"));
            } else {
                out.push_str(&format!("{label}: {value}\n"));
            }
        }
    }
    out
}

/// The text report of the active document (`document.info {format: "text"}` as a string).
pub fn report(s: &mut Session, selection_only: bool) -> Result<String> {
    let v = info(s, &json!({ "selectionOnly": selection_only, "format": "text" }))?;
    Ok(v["text"].as_str().unwrap_or_default().to_string())
}

fn paint_kind(p: &Paint) -> Option<&'static str> {
    match p {
        Paint::Gradient(_) => Some("gradients"),
        Paint::Pattern { .. } => Some("patterns"),
        _ => None,
    }
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.info";
    let category = str_param(p, "category");
    if let Some(c) = category.filter(|c| !CATEGORIES.iter().any(|(id, _)| id == c)) {
        let ids: Vec<&str> = CATEGORIES.iter().map(|(id, _)| *id).collect();
        return Err(bad(C, format!("category `{c}`: one of {}", ids.join(", "))));
    }
    let text = match str_param(p, "format") {
        None | Some("json") => false,
        Some("text") => true,
        Some(f) => return Err(bad(C, format!("format `{f}`: text or json"))),
    };
    let st = s.doc()?;
    let d = &st.doc;
    let selection_only = bool_or(p, "selectionOnly", false);
    let roots: Vec<&Node> =
        if selection_only { st.selection.objects.iter().filter_map(|id| d.node(*id)).collect() } else { d.layers.iter().map(|l| &**l).collect() };
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut fonts = BTreeSet::new();
    let mut images: BTreeMap<String, Value> = BTreeMap::new();
    let mut image_rows: Vec<ImageRow> = vec![];
    let mut patterns = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    let mut spot = BTreeSet::new();
    let mut styles: Vec<&str> = if selection_only { vec![] } else { d.graphic_styles.iter().map(|g| g.name.as_str()).collect() };
    for r in roots {
        r.walk(&mut |n| {
            let k = match &n.kind {
                NodeKind::Layer { .. } => None,
                NodeKind::Group { clip: true, .. } => Some("clipGroups"),
                NodeKind::Group { .. } => Some("groups"),
                NodeKind::Path { guide: true, .. } => Some("guides"),
                NodeKind::Path { .. } => Some("paths"),
                NodeKind::Compound { .. } => Some("compoundPaths"),
                NodeKind::Text(t) => {
                    for run in &t.runs {
                        fonts.insert((run.style.font_family.clone(), run.style.font_style.clone()));
                    }
                    Some("textObjects")
                }
                NodeKind::Image(im) => {
                    images.insert(
                        im.key.clone(),
                        json!({ "width": im.width, "height": im.height, "linked": im.link.is_some(), "link": im.link.as_ref().map(|l| &l.path) }),
                    );
                    let name = im.link.as_ref().map_or_else(|| n.display_name(), |l| l.name().to_string());
                    image_rows.push((name, im.width, im.height, im.link.as_ref().map(|l| l.path.clone())));
                    Some("images")
                }
                NodeKind::SymbolInstance { symbol, .. } => {
                    symbols.insert(symbol.clone());
                    Some("symbolInstances")
                }
                NodeKind::Blend { .. } => Some("blends"),
                NodeKind::Envelope { .. } => Some("envelopes"),
                NodeKind::Mesh(_) => Some("meshes"),
                NodeKind::Repeat(_) => Some("repeats"),
                NodeKind::PlacedDocument(_) => Some("placedDocuments"),
            };
            if let Some(k) = k {
                *counts.entry(k).or_default() += 1;
            }
            for item in &n.appearance.items {
                let paint = match item {
                    vectorcraft_doc::AppearanceItem::Fill(f) => &f.paint,
                    vectorcraft_doc::AppearanceItem::Stroke(s) => &s.paint,
                };
                if let Some(k) = paint_kind(paint) {
                    *counts.entry(k).or_default() += 1;
                }
                if let Paint::Pattern { pattern, .. } = paint {
                    patterns.insert(pattern.clone());
                }
                if let Paint::Solid { swatch: Some(name), .. } = paint
                    && d.swatch(name).is_some_and(|s| s.spot)
                {
                    spot.insert(name.clone());
                }
            }
            if n.mask.is_some() {
                *counts.entry("opacityMasks").or_default() += 1;
            }
            if !n.appearance.effects.is_empty() {
                *counts.entry("liveEffects").or_default() += 1;
            }
            if selection_only
                && let Some(g) = super::style::linked_style(d, n)
                && !styles.contains(&g.name.as_str())
            {
                styles.push(&g.name);
            }
        });
    }
    let font_names: Vec<String> = fonts.iter().map(|(f, s)| format!("{f} {s}")).collect();
    let mut out = json!({
        "document": document_summary(d),
        "objects": counts,
        "fonts": font_names,
        "images": images,
        "symbols": symbols,
        "spotColors": spot,
        "swatches": d.swatches_iter().count(),
        "graphicStyles": d.graphic_styles.len(),
        "graphicStyleNames": styles,
        "characterStyles": d.char_styles.len(),
        "paragraphStyles": d.para_styles.len(),
        "patterns": d.patterns.len(),
        "patternNames": patterns,
    });
    // The document's name: its file's, else its title.
    out["document"]["name"] = json!(st.title());
    let sections = sections(d, &out, &fonts, &image_rows, category);
    if text {
        return Ok(json!({ "text": report_text(&st.title(), selection_only, &sections) }));
    }
    out["sections"] = sections.iter().map(|s| json!({ "id": s.id, "title": s.title, "rows": s.rows })).collect();
    Ok(out)
}

fn document_summary(d: &Document) -> Value {
    let u = d.units.points();
    json!({
        "title": d.title,
        "colorMode": match d.color_mode { ColorMode::Rgb => "RGB", ColorMode::Cmyk => "CMYK" },
        "units": d.units.label(),
        "artboards": d.artboards.iter().map(|a| json!({ "name": a.name, "width": a.rect.width() / u, "height": a.rect.height() / u })).collect::<Vec<_>>(),
        "rasterEffectsPpi": d.raster_effects_ppi,
    })
}

/// The affine snapping `b` to whole pixels (`half`: edges on half pixels, for odd stroke weights).
fn snap_xf(b: Rect, half: bool) -> Affine {
    let off = if half { 0.5 } else { 0.0 };
    let snap = |v: f64| (v - off).round() + off;
    let (x0, y0) = (snap(b.x0), snap(b.y0));
    // Keep at least one pixel of size; widths snap to whole pixels.
    let (w, h) = (b.width().round().max(if b.width() > 0.0 { 1.0 } else { 0.0 }), b.height().round().max(if b.height() > 0.0 { 1.0 } else { 0.0 }));
    let sx = if b.width() > 1e-9 { w / b.width() } else { 1.0 };
    let sy = if b.height() > 1e-9 { h / b.height() } else { 1.0 };
    Affine::translate((x0, y0)) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-b.x0, -b.y0))
}

fn pixel_perfect(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<NodeId> = if p.get("ids").is_some() { targets(s, p)? } else { super::edit::selected_roots(s)? };
    let n = ids.len();
    s.edit("Make Pixel Perfect", |d, _| {
        for id in &ids {
            let Some(node) = d.node_mut(*id) else { continue };
            // Whole-pixel stroke weights; odd ones centre on half pixels.
            let mut odd = false;
            for item in &mut node.appearance.items {
                if let vectorcraft_doc::AppearanceItem::Stroke(st) = item
                    && st.width > 0.0
                {
                    st.width = st.width.round().max(1.0);
                    odd |= st.width as i64 % 2 == 1 && st.align == vectorcraft_doc::StrokeAlign::Center;
                }
            }
            if let Some(b) = node.geometric_bounds() {
                node.transform(snap_xf(b, odd), false);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_counts_objects_fonts_and_selection_only() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute(
            "paint.setFill",
            &json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]}}),
        )
        .unwrap();
        s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        assert_eq!(
            (i["objects"]["paths"].as_u64(), i["objects"]["textObjects"].as_u64(), i["objects"]["gradients"].as_u64()),
            (Some(1), Some(1), Some(1))
        );
        assert_eq!(i["fonts"][0], "Source Sans 3 Regular");
        assert_eq!(i["document"]["artboards"][0]["width"], 200.0);
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
        assert!(i["objects"]["textObjects"].is_null() && i["objects"]["paths"] == 1);
    }

    #[test]
    fn embedding_permissions_read_from_fs_type() {
        assert_eq!(embedding_label(0), "embedding allowed");
        assert_eq!(embedding_label(0x0002), "embedding not allowed");
        assert_eq!(embedding_label(0x0004 | 0x0100), "embedding for preview and print");
        assert_eq!(embedding_label(0x0008), "embedding for editing");
        // As exports read it: bitmap embedding only, the restricted bit with reserved bit 0, the
        // most permissive of several usage bits.
        assert_eq!(embedding_label(0x0200), "embedding not allowed");
        assert_eq!(embedding_label(0x0003), "embedding not allowed");
        assert_eq!(embedding_label(0x0006), "embedding for preview and print");
        assert_eq!(embedding_label(0x000c), "embedding for editing");
    }

    #[test]
    fn pixel_perfect_snaps_edges_and_strokes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
        s.execute("stroke.set", &json!({"weight": 0.8})).unwrap();
        s.execute("object.makePixelPerfect", &json!({})).unwrap();
        let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
        let b = n.geometric_bounds().unwrap();
        // 1 pt (odd) stroke → edges on half pixels, whole-pixel size.
        assert_eq!((b.x0, b.y0, b.width(), b.height()), (10.5, 20.5, 30.0, 10.0));
        assert_eq!(n.appearance.stroke_width(), 1.0);
        s.execute("stroke.set", &json!({"weight": 2})).unwrap();
        s.execute("object.makePixelPerfect", &json!({})).unwrap();
        let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert_eq!((b.x0.fract(), b.y0.fract()), (0.0, 0.0));
    }
}
