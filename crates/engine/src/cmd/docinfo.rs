//! Document Info panel data, its categories and the text report (Document Info › Save…, File →
//! Package), and Object → Make Pixel Perfect.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{ColorMode, Document, Node, NodeId, NodeKind, RepeatKind, RepeatSpec};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_text::embed::Embedding;

use super::{CommandSpec, Result, Session, bad, bool_or, cmd, has_doc, has_selection, str_param, targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "document.info",
            "Document Info",
            ["Window", "Document Info"],
            None,
            "{selectionOnly?, category?: document|objects|graphicStyles|brushes|spotColors|patterns|gradients|symbols|fonts|fontDetails|linkedImages|embeddedImages|repeats (only that one in sections and the text), format?: \"text\" (→ {text}: the report Document Info › Save… and Package write)} → {document, objects: {paths, compoundPaths, groups, …}, pathLength: pt (the paths' total length, compound paths' included, guides left out), fonts, images, swatches, graphicStyleNames (with selectionOnly: the styles the selected objects are linked to), brushNames (the Brushes panel's brushes; with selectionOnly: the brushes the selected objects' strokes use), repeatObjects: [{id, name, type: radial|grid|mirror, instances, rows?, cols?}] (the Repeat objects, those in groups and other repeats included; with selectionOnly: those in the selection), …, sections: [{id, title, rows: [[label, value]]}] (the panel's categories; fontDetails: each font's file and whether its license lets it be embedded)}",
            has_doc,
            info
        ),
        cmd!(
            "object.makePixelPerfect",
            "Make Pixel Perfect",
            ["Object"],
            None,
            "{ids?|id?} snap each object's edges to the pixel grid (odd stroke weights on half pixels) and round stroke weights",
            has_selection,
            pixel_perfect
        ),
    ]
}

/// Document Info's categories: (id, title), in the order the panel and the report show them.
pub const CATEGORIES: [(&str, &str); 13] = [
    ("document", "Document"),
    ("objects", "Objects"),
    ("graphicStyles", "Graphic Styles"),
    ("brushes", "Brushes"),
    ("spotColors", "Spot Colors"),
    ("patterns", "Pattern Objects"),
    ("gradients", "Gradient Swatches"),
    ("symbols", "Symbols"),
    ("fonts", "Fonts"),
    ("fontDetails", "Font Details"),
    ("linkedImages", "Linked Images"),
    ("embeddedImages", "Embedded Images"),
    ("repeats", "Repeat Objects"),
];

/// The object counts of `document.info` and their labels.
const OBJECT_LABELS: [(&str, &str); 18] = [
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
    ("intertwines", "Intertwine Groups"),
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

/// A Repeat object as Document Info lists it: its `repeatObjects` entry, and its row, which gives
/// its name and then its kind with the number of instances it draws (for a grid, its rows and
/// columns, followed by the number it draws when that is fewer).
fn repeat_entry(d: &Document, n: &Node, spec: &RepeatSpec) -> (Value, (String, String)) {
    let name = n.display_name();
    let count = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let instances = spec.instances(Some(d));
    let mut entry = json!({ "id": n.id.0, "name": name, "instances": instances });
    let kind = &spec.kind;
    let detail = match kind {
        RepeatKind::Radial { .. } => {
            entry["type"] = json!("radial");
            count(instances, "instance", "instances")
        }
        RepeatKind::Mirror { .. } => {
            entry["type"] = json!("mirror");
            count(instances, "instance", "instances")
        }
        RepeatKind::Grid { rows, cols, .. } => {
            // `repeat_transforms` clamps the rows and columns the same way.
            let (rows, cols) = ((*rows).clamp(1, 500), (*cols).clamp(1, 500));
            entry["type"] = json!("grid");
            entry["rows"] = json!(rows);
            entry["cols"] = json!(cols);
            let grid = format!("{} × {}", count(rows as usize, "row", "rows"), count(cols as usize, "column", "columns"));
            if instances < (rows * cols) as usize { format!("{grid}, {}", count(instances, "instance", "instances")) } else { grid }
        }
    };
    (entry, (name, format!("{}, {detail}", kind.label())))
}

/// The Document Info categories of `d` (only `category` when given) from `info` (what
/// `document.info` reports), its fonts, its image objects and the rows of its Repeat objects.
fn sections(
    d: &Document,
    info: &Value,
    fonts: &BTreeSet<super::fonts::UsedFont>,
    images: &[ImageRow],
    repeats: &[(String, String)],
    category: Option<&str>,
) -> Vec<Section> {
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
                    let mut rows: Vec<(String, String)> =
                        OBJECT_LABELS.iter().filter_map(|(k, label)| Some((label.to_string(), info["objects"][k].as_u64()?.to_string()))).collect();
                    // The paths' total length, in the document's units, right after their count.
                    if let Some(at) = rows.iter().position(|(label, _)| label == "Paths") {
                        rows.insert(at + 1, ("Path Length".into(), u.format(info["pathLength"].as_f64().unwrap_or(0.0))));
                    }
                    rows
                }
                "graphicStyles" => names("graphicStyleNames"),
                "brushes" => names("brushNames"),
                "spotColors" => names("spotColors"),
                "patterns" => names("patternNames"),
                "gradients" => d.swatches_iter().filter(|s| matches!(s.paint, Paint::Gradient(_))).map(|s| (s.name.clone(), String::new())).collect(),
                "symbols" => names("symbols"),
                "fonts" => fonts.iter().map(|f| (super::fonts::font_label(f), String::new())).collect(),
                "fontDetails" => {
                    let db = vectorcraft_text::FontDb::global();
                    fonts
                        .iter()
                        .map(|used| {
                            let (family, style, version) = used;
                            // Found by any of its names (in the version the type names); a style the
                            // family lacks shows in another.
                            let resolved =
                                db.resolve(family, style).map(|(f, m)| (db.face_version(family, style, version.as_deref()).unwrap_or(f), m));
                            let detail = match resolved {
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
                            (super::fonts::font_label(used), detail)
                        })
                        .collect()
                }
                "linkedImages" => {
                    images.iter().filter_map(|(name, w, h, link)| Some((name.clone(), format!("{w} × {h} px — {}", link.as_ref()?)))).collect()
                }
                "repeats" => repeats.to_vec(),
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
    // The paths' total length, as Document Info › Objects shows it (the length of a cut line, a
    // strip of LEDs along the letters, #1958).
    let mut path_length = 0.0;
    let mut fonts = BTreeSet::new();
    let mut images: BTreeMap<String, Value> = BTreeMap::new();
    let mut image_rows: Vec<ImageRow> = vec![];
    let mut patterns = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    let mut spot = BTreeSet::new();
    let mut styles: Vec<&str> = if selection_only { vec![] } else { d.graphic_styles.iter().map(|g| g.name.as_str()).collect() };
    // `brushes` holds the Brushes panel's brushes, or with selectionOnly it holds the brushes that the selected objects' strokes use.
    let mut brushes: Vec<String> = if selection_only { vec![] } else { vectorcraft_brush::library(d).into_iter().map(|b| b.name).collect() };
    // `repeats` and `repeat_rows` hold the `repeatObjects` entries and the rows of the Repeat objects that the walk visits, in walk order.
    let mut repeats: Vec<Value> = vec![];
    let mut repeat_rows: Vec<(String, String)> = vec![];
    for r in roots {
        r.walk(&mut |n| {
            let k = match &n.kind {
                NodeKind::Layer { .. } => None,
                NodeKind::Group { clip: true, .. } => Some("clipGroups"),
                NodeKind::Group { .. } => Some("groups"),
                NodeKind::Path { guide: true, .. } => Some("guides"),
                NodeKind::Path { path, .. } => {
                    let l = path.length();
                    if l.is_finite() {
                        path_length += l;
                    }
                    Some("paths")
                }
                NodeKind::Compound { .. } => Some("compoundPaths"),
                NodeKind::Text(t) => {
                    for run in &t.runs {
                        fonts.insert((run.style.font_family.clone(), run.style.font_style.clone(), run.style.font_version.clone()));
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
                NodeKind::Intertwine { .. } => Some("intertwines"),
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
            if n.extras().mask.is_some() {
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
            if selection_only {
                for item in &n.appearance.items {
                    if let vectorcraft_doc::AppearanceItem::Stroke(st) = item
                        && let Some(b) = &st.brush
                        && !brushes.contains(b)
                    {
                        brushes.push(b.clone());
                    }
                }
            }
            if let NodeKind::Repeat(spec) = &n.kind {
                let (entry, row) = repeat_entry(d, n, spec);
                repeats.push(entry);
                repeat_rows.push(row);
            }
        });
    }
    let font_names: Vec<String> = fonts.iter().map(super::fonts::font_label).collect();
    let mut out = json!({
        "document": document_summary(d),
        "objects": counts,
        "pathLength": path_length,
        "fonts": font_names,
        "images": images,
        "symbols": symbols,
        "spotColors": spot,
        "swatches": d.swatches_iter().count(),
        "graphicStyles": d.graphic_styles.len(),
        "graphicStyleNames": styles,
        "brushNames": brushes,
        "repeatObjects": repeats,
        "characterStyles": d.char_styles.len(),
        "paragraphStyles": d.para_styles.len(),
        "patterns": d.patterns.len(),
        "patternNames": patterns,
    });
    // The document's name: its file's, else its title.
    out["document"]["name"] = json!(st.title());
    let sections = sections(d, &out, &fonts, &image_rows, &repeat_rows, category);
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
    let ids: Vec<NodeId> = if p.get("ids").is_some() || p.get("id").is_some() {
        super::edit::outermost(&s.doc()?.doc, targets(s, p)?)
    } else {
        super::edit::selected_roots(s)?
    };
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

    /// Document Info › Objects gives the paths' total length in the document's units (#1958): an
    /// open line, a closed rectangle's perimeter and a compound path's pieces count, a guide doesn't.
    #[test]
    fn info_gives_the_paths_total_length() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400, "units": "Millimeters"})).unwrap();
        let line = s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 72, "y2": 0})).unwrap()["id"].clone();
        let rect = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 10, "height": 20})).unwrap()["id"].clone();
        let length = |s: &mut Session, p: Value| s.execute("document.info", &p).unwrap()["pathLength"].as_f64().unwrap();
        assert!((length(&mut s, json!({})) - 132.0).abs() < 1e-6);
        s.execute("select.set", &json!({"ids": [line]})).unwrap();
        assert!((length(&mut s, json!({"selectionOnly": true})) - 72.0).abs() < 1e-6);
        let rows = s.execute("document.info", &json!({"selectionOnly": true, "category": "objects"})).unwrap()["sections"][0]["rows"].clone();
        assert_eq!(rows, json!([["Paths", "1"], ["Path Length", "25.4 mm"]]));
        // A compound path's pieces are paths: their lengths count.
        let other = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 5, "height": 5})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [rect, other]})).unwrap();
        s.execute("object.compoundPath.make", &json!({})).unwrap();
        assert!((length(&mut s, json!({"selectionOnly": true})) - 80.0).abs() < 1e-6);
        // Nothing with no paths, and no length row.
        s.execute("select.none", &json!({})).unwrap();
        assert_eq!(length(&mut s, json!({"selectionOnly": true})), 0.0);
    }

    /// Document Info › Brushes lists the Brushes panel's brushes, and with Selection Only it lists
    /// the brushes the selected objects' strokes use, each once.
    #[test]
    fn info_lists_the_brushes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let panel: Vec<Value> =
            s.execute("brush.list", &json!({})).unwrap()["brushes"].as_array().unwrap().iter().map(|b| b["name"].clone()).collect();
        let i = s.execute("document.info", &json!({"category": "brushes"})).unwrap();
        assert_eq!(i["brushNames"], json!(panel));
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("brushes"), Some("Brushes")));
        let rows: Vec<Value> = panel.iter().map(|n| json!([n, ""])).collect();
        assert_eq!(i["sections"][0]["rows"], json!(rows));
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let c = s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("brush.apply", &json!({"ids": [a], "name": "Charcoal"})).unwrap();
        s.execute("brush.apply", &json!({"ids": [b], "name": "Arrow"})).unwrap();
        s.execute("brush.apply", &json!({"ids": [c], "name": "Charcoal"})).unwrap();
        let names =
            |s: &mut Session| s.execute("document.info", &json!({"selectionOnly": true, "category": "brushes"})).unwrap()["brushNames"].clone();
        s.execute("select.set", &json!({"ids": [a, b, c]})).unwrap();
        assert_eq!(names(&mut s), json!(["Charcoal", "Arrow"]));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        assert_eq!(names(&mut s), json!(["Arrow"]));
        // A stroke without a brush lists none.
        let plain = s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [plain]})).unwrap();
        assert_eq!(names(&mut s), json!([]));
        let text = s.execute("document.info", &json!({"category": "brushes", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.contains(&format!("\nBRUSHES\n{}\n", panel[0].as_str().unwrap())), "{text}");
        assert!(text.contains("\nCharcoal\n") && text.contains("\nArrow\n") && !text.contains("OBJECTS"), "{text}");
    }

    /// Document Info › Repeat Objects lists each Repeat object by name with its kind and the number
    /// of instances it makes (for a grid, its rows and columns, followed by the number it draws when
    /// that is fewer), one in a group included. With Selection Only it lists the Repeat objects in
    /// the selection.
    #[test]
    fn info_lists_the_repeat_objects() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let square =
            |s: &mut Session, x: f64| s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let (a, b, c, d) = (square(&mut s, 0.0), square(&mut s, 100.0), square(&mut s, 200.0), square(&mut s, 300.0));
        let radial = s.execute("object.repeat.radial", &json!({"ids": [a], "instances": 6})).unwrap()["id"].clone();
        let grid = s.execute("object.repeat.grid", &json!({"ids": [b], "rows": 2, "cols": 1})).unwrap()["id"].clone();
        s.execute("layer.setProps", &json!({"id": grid, "name": "Tiles"})).unwrap();
        let mirror = s.execute("object.repeat.mirror", &json!({"ids": [c]})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [mirror, d]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
        let i = s.execute("document.info", &json!({"category": "repeats"})).unwrap();
        assert_eq!(
            i["repeatObjects"],
            json!([
                {"id": radial, "name": "<Radial Repeat>", "type": "radial", "instances": 6},
                {"id": grid, "name": "Tiles", "type": "grid", "rows": 2, "cols": 1, "instances": 2},
                {"id": mirror, "name": "<Mirror Repeat>", "type": "mirror", "instances": 2},
            ])
        );
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("repeats"), Some("Repeat Objects")));
        assert_eq!(
            i["sections"][0]["rows"],
            json!([
                ["<Radial Repeat>", "Radial Repeat, 6 instances"],
                ["Tiles", "Grid Repeat, 2 rows × 1 column"],
                ["<Mirror Repeat>", "Mirror Repeat, 2 instances"],
            ])
        );
        let names = |s: &mut Session| -> Vec<Value> {
            let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "repeats"})).unwrap();
            i["repeatObjects"].as_array().unwrap().iter().map(|r| r["name"].clone()).collect()
        };
        s.execute("select.set", &json!({"ids": [grid, group]})).unwrap();
        assert_eq!(names(&mut s), ["Tiles", "<Mirror Repeat>"]);
        s.execute("select.set", &json!({"ids": [radial]})).unwrap();
        assert_eq!(names(&mut s), ["<Radial Repeat>"]);
        s.execute("select.set", &json!({"ids": [d]})).unwrap();
        assert!(names(&mut s).is_empty());
        let text = s.execute("document.info", &json!({"category": "repeats", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nREPEAT OBJECTS\n<Radial Repeat>: Radial Repeat, 6 instances\nTiles: Grid Repeat, 2 rows × 1 column\n"), "{text}");
        assert!(!text.contains("\nOBJECTS\n"), "{text}");
        // A radial of 1,000 copies of a 500 × 500 grid draws one copy within the instance budget,
        // and Document Info reports 1 instance.
        let e = square(&mut s, 350.0);
        let big = s.execute("object.repeat.grid", &json!({"ids": [e], "rows": 500, "cols": 500})).unwrap()["id"].clone();
        let outer = s.execute("object.repeat.radial", &json!({"ids": [big], "instances": 1000})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [outer]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "repeats"})).unwrap();
        assert_eq!((i["repeatObjects"][0]["id"].clone(), i["repeatObjects"][0]["instances"].clone()), (outer, json!(1)));
        assert_eq!((i["repeatObjects"][1]["id"].clone(), i["repeatObjects"][1]["instances"].clone()), (big, json!(250_000)));
        assert_eq!(i["sections"][0]["rows"][0], json!(["<Radial Repeat>", "Radial Repeat, 1 instance"]));
        // A 500 × 500 grid of a mirror draws 125,000 copies within the budget, and its row gives
        // that number after its rows and columns.
        let f = square(&mut s, 380.0);
        let pair = s.execute("object.repeat.mirror", &json!({"ids": [f]})).unwrap()["id"].clone();
        let tiles = s.execute("object.repeat.grid", &json!({"ids": [pair], "rows": 500, "cols": 500})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [tiles]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "repeats"})).unwrap();
        assert_eq!((i["repeatObjects"][0]["id"].clone(), i["repeatObjects"][0]["instances"].clone()), (tiles, json!(125_000)));
        assert_eq!(i["sections"][0]["rows"][0], json!(["<Grid Repeat>", "Grid Repeat, 500 rows × 500 columns, 125000 instances"]));
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
