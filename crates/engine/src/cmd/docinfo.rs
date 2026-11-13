//! Document Info panel data, its categories and the text report (Document Info › Save…, File →
//! Package), and Object → Make Pixel Perfect.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
            "{selectionOnly?, category?: document|objects|graphicStyles|brushes|spotColors|patterns|gradients|symbols|fonts|fontDetails|linkedImages|embeddedImages|repeats (only that one in sections and the text), format?: \"text\" (→ {text}: the report Document Info › Save… and Package write)} → {document, objects: {paths, compoundPaths, groups, …}, pathLength: pt (the paths' total length, compound paths' included, guides left out), fonts (the type in opacity masks and in the art of symbols and patterns included; with selectionOnly: the selected type's), images, swatches, graphicStyleNames (with selectionOnly: the styles the selected objects are linked to), brushNames (the brushes the art's drawn strokes use, each once; with selectionOnly: those the selected objects' strokes use), brushes: [{name, type: calligraphic|scatter|art|pattern|bristle|null}] (those brushes with their types; null: not in the Brushes panel), panelBrushNames (the Brushes panel's brushes), repeatObjects: [{id, name, type: radial|grid|mirror, instances, rows?, cols?}] (the Repeat objects, those in groups and other repeats included; with selectionOnly: those in the selection), spotObjects: [{id, name, spotColors}] (the objects filled or stroked with a spot color swatch, as a solid color or in the stops of a gradient with a stop above opacity 0, at a tint above 0, and grayscale images colorized with one, each once, by the rules patternObjects follows, with the spot colors they use; Registration and process colors are not spot colors; objects.spots counts them), spotColorNames and spotColors (the spot colors those objects use), patternObjects: [{id, name, patterns}] (the objects filled or stroked with a pattern, each once, those in groups and type whose drawn characters use one included, hidden fills and strokes, fills and strokes at an opacity of 0, strokes with no width, guides, compound path members' own fills and strokes, images' fills and strokes without a shape and characters that draw nothing left out, with the patterns they use, which objects.patterns counts; with selectionOnly: those in the selection), patternNames (the patterns those objects use), gradientObjects: [{id, name, gradients}] (the objects filled or stroked with a gradient, by the same rules, with the gradients they use: a linked gradient swatch's name, else Linear gradient, Radial gradient or Freeform gradient; objects.gradients counts them), gradientNames (the gradients those objects use), …, sections: [{id, title, rows: [[label, value]]}] (the panel's categories; fontDetails: each font's file and whether its license lets it be embedded)}. An image linked to a file outside the automation read roots has link null, and its Linked Images row names the file alone",
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
    ("spotColors", "Spot Color Objects"),
    ("patterns", "Pattern Objects"),
    ("gradients", "Gradient Objects"),
    ("symbols", "Symbols"),
    ("fonts", "Fonts"),
    ("fontDetails", "Font Details"),
    ("linkedImages", "Linked Images"),
    ("embeddedImages", "Embedded Images"),
    ("repeats", "Repeat Objects"),
];

/// The object counts of `document.info` and their labels.
pub const OBJECT_LABELS: [(&str, &str); 19] = [
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
    ("spots", "Spot Color Objects"),
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
/// `document.info` reports), its fonts, its image objects, the rows of the brushes its art uses, the
/// rows of its Repeat objects and the rows of its objects filled or stroked with a spot color, with a
/// pattern and with a gradient.
#[allow(clippy::too_many_arguments)]
fn sections(
    d: &Document,
    info: &Value,
    fonts: &BTreeSet<super::fonts::UsedFont>,
    images: &[ImageRow],
    brushes: &[(String, String)],
    repeats: &[(String, String)],
    spot_objects: &[(String, String)],
    pattern_objects: &[(String, String)],
    gradient_objects: &[(String, String)],
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
                "brushes" => brushes.to_vec(),
                "spotColors" => spot_objects.to_vec(),
                "patterns" => pattern_objects.to_vec(),
                "gradients" => gradient_objects.to_vec(),
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

/// The pattern paint `p` uses: the pattern's name.
fn pattern_label(p: &Paint) -> Option<String> {
    match p {
        Paint::Pattern { pattern, .. } => Some(pattern.clone()),
        _ => None,
    }
}

/// The gradient paint `p` uses: the name of the gradient swatch it is linked to, else its kind
/// ("Linear gradient", "Radial gradient" or "Freeform gradient").
fn gradient_label(p: &Paint) -> Option<String> {
    match p {
        Paint::Gradient(g) => Some(g.swatch.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{} gradient", g.gradient.kind.label()))),
        _ => None,
    }
}

/// The names of `d`'s spot color swatches: the swatches whose prints go to a plate of their own
/// ([`vectorcraft_doc::inks::inks`]). A name held by several swatches is the first one's, as
/// [`Document::swatch`] finds it, and the Registration swatch prints on every plate, so it is not
/// one.
fn spot_swatches(d: &Document) -> HashSet<&str> {
    let mut first: BTreeMap<&str, bool> = BTreeMap::new();
    for s in d.swatches_iter() {
        first.entry(s.name.as_str()).or_insert(s.spot && s.paint.color().is_some());
    }
    first.into_iter().filter(|&(name, spot)| spot && name != vectorcraft_color::swatch::REGISTRATION).map(|(name, _)| name).collect()
}

/// The spot colors paint `p` prints (`spots`: [`spot_swatches`]): the spot swatch a solid color is
/// linked to, or those a gradient's stops are linked to, each once, in the order of the stops. A
/// link at a tint of 0 prints no ink, a gradient whose stops are all at an opacity of 0 draws
/// nothing, and a color that only looks like a spot color is not linked to one.
fn spot_labels(spots: &HashSet<&str>, p: &Paint) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut add = |swatch: &Option<String>, tint: f32| {
        if let Some(name) = swatch
            && tint > 0.0
            && spots.contains(name.as_str())
            && !out.contains(name)
        {
            out.push(name.clone());
        }
    };
    match p {
        Paint::Solid { swatch, tint, .. } => add(swatch, *tint),
        Paint::Gradient(g) if g.gradient.stops.iter().any(|s| s.opacity > 0.0) => g.gradient.stops.iter().for_each(|s| add(&s.swatch, s.tint)),
        _ => {}
    }
    out
}

/// The type object `n` holds and the runs its layout draws, when it uses a spot color, a pattern or
/// a gradient (`spots`: [`spot_swatches`]): the runs with a glyph outline (list markers and ruby
/// included) or an underline or strikethrough bar. Spaces and line breaks draw no outline. A type
/// object's characters carry their own fill and stroke, and the type object's own fills and strokes
/// paint on its characters, so only those runs paint. The type is laid out with its inline
/// graphics' bounds resolved from `d`'s symbols, as the renderer lays it out.
fn painted_runs<'a>(d: &Document, spots: &HashSet<&str>, n: &'a Node) -> Option<(&'a vectorcraft_doc::TextObject, BTreeSet<usize>)> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let is_listed = |p: &Paint| matches!(p, Paint::Pattern { .. } | Paint::Gradient(_)) || !spot_labels(spots, p).is_empty();
    let own = n.appearance.items.iter().any(|i| match i {
        vectorcraft_doc::AppearanceItem::Fill(f) => is_listed(&f.paint),
        vectorcraft_doc::AppearanceItem::Stroke(s) => is_listed(&s.paint),
    });
    if !own && !t.runs.iter().any(|r| is_listed(&r.style.fill) || is_listed(&r.style.stroke)) {
        return None;
    }
    Some((t, drawn_runs(d, t)))
}

/// The runs of type `t` that its layout draws: those with a glyph outline (list markers and ruby
/// included) or an underline or strikethrough bar, laid out as the renderer lays it out.
fn drawn_runs(d: &Document, t: &vectorcraft_doc::TextObject) -> BTreeSet<usize> {
    let db = vectorcraft_text::FontDb::global();
    let resolved = d.inline_resolved(t);
    let lay = vectorcraft_text::layout(db, &resolved);
    let mut painted: BTreeSet<usize> = lay.glyphs.iter().chain(&lay.ruby).filter(|g| !g.outline.elements().is_empty()).map(|g| g.run).collect();
    painted.extend(vectorcraft_text::decorations(&lay, db, &resolved).into_iter().map(|(run, _)| run));
    painted
}

/// The names a paint is listed by in a Document Info category: its spot colors, its pattern or its
/// gradient.
type PaintNames<'a> = dyn Fn(&Paint) -> Vec<String> + 'a;

/// The spot colors, patterns or gradients (`label`: [`spot_labels`], [`pattern_label`] or
/// [`gradient_label`], each paint's names) that `n`'s own fills and strokes paint, each once. A
/// hidden fill or stroke, a fill or stroke at an opacity of 0 and a stroke with no width paint
/// nothing, and neither does any fill or
/// stroke of a guide (drawn in the guide color), of a compound path's member (`members`: the
/// compound path's own fills and strokes paint it) or of type whose layout draws nothing
/// (`painted_runs`). An image's fills and strokes paint only where an effect gives them a shape.
fn own_paints(
    d: &Document,
    n: &Node,
    members: &HashSet<*const Node>,
    painted_runs: Option<&(&vectorcraft_doc::TextObject, BTreeSet<usize>)>,
    label: &PaintNames<'_>,
) -> Vec<String> {
    if matches!(n.kind, NodeKind::Path { guide: true, .. })
        || members.contains(&std::ptr::from_ref(n))
        || painted_runs.is_some_and(|(_, r)| r.is_empty())
        || n.appearance.items.iter().all(|i| label(i.paint()).is_empty())
    {
        return vec![];
    }
    // The names an image's shaped fills and strokes paint (their effects can transform the paint,
    // so they are matched by name).
    let shaped: Option<Vec<String>> = matches!(n.kind, NodeKind::Image(_))
        .then(|| vectorcraft_render::effects::image_paint(Some(d), n).into_iter().flat_map(|(_, _, item)| label(item.paint())).collect());
    let mut out: Vec<String> = vec![];
    for item in &n.appearance.items {
        if !item.visible() || item.opacity() <= 0.0 || matches!(item, vectorcraft_doc::AppearanceItem::Stroke(s) if s.width <= 0.0) {
            continue;
        }
        for name in label(item.paint()) {
            if shaped.as_ref().is_none_or(|s| s.contains(&name)) && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// [`pattern_label`] or [`gradient_label`] as the names [`own_paints`] takes.
fn one_label(label: fn(&Paint) -> Option<String>) -> impl Fn(&Paint) -> Vec<String> {
    move |p| label(p).into_iter().collect()
}

/// The brushes `n`'s own strokes draw with, in the order of its appearance. A hidden stroke, a
/// stroke at an opacity of 0, a stroke with no paint and a stroke with no width draw nothing, and neither does any stroke of a
/// guide (drawn in the guide color) or of a compound path's member (`members`: the compound path's
/// own strokes draw it). An image's strokes draw only where an effect gives them a shape, and only
/// paths, compound paths, type and images draw strokes of their own.
fn applied_brushes(d: &Document, n: &Node, members: &HashSet<*const Node>) -> Vec<String> {
    if matches!(n.kind, NodeKind::Path { guide: true, .. }) || members.contains(&std::ptr::from_ref(n)) {
        return vec![];
    }
    let brush = |item: &vectorcraft_doc::AppearanceItem| match item {
        vectorcraft_doc::AppearanceItem::Stroke(st) if item.visible() && st.opacity > 0.0 && !st.paint.is_none() && st.width > 0.0 => {
            st.brush.clone().filter(|b| !b.is_empty())
        }
        _ => None,
    };
    if matches!(n.kind, NodeKind::Image(_)) {
        return vectorcraft_render::effects::image_paint(Some(d), n).iter().filter_map(|(_, _, item)| brush(item)).collect();
    }
    let used: Vec<String> = n.appearance.items.iter().filter_map(brush).collect();
    if used.is_empty() {
        return used;
    }
    // A path or compound path with no segment gives its strokes nothing to follow, and neither does
    // type whose layout draws nothing (spaces and line breaks without an underline or strikethrough).
    let segments = |pd: &vectorcraft_geom::PathData| pd.subpaths.iter().any(|sp| sp.segment_count() > 0);
    let follows = match &n.kind {
        NodeKind::Path { .. } => n.path_data().is_some_and(segments),
        NodeKind::Compound { children, .. } => children.iter().filter_map(|c| c.path_data()).any(segments),
        NodeKind::Text(t) => !drawn_runs(d, t).is_empty(),
        // A brush on any other kind of object (a layer, a group, a ruler guide) is not listed.
        _ => false,
    };
    if follows { used } else { vec![] }
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
    // Whether the request being served may read each linked file (a relative path read from the
    // document's folder), checked once per file.
    let mut readable: HashMap<String, bool> = HashMap::new();
    let link_doc = super::links::LinkBase::of(st).doc;
    let mut may_read = |stored: &str| {
        *readable.entry(stored.to_string()).or_insert_with(|| {
            let resolved = super::fileio::document_path(stored, link_doc).ok();
            crate::file_access::check_read(resolved.as_deref().unwrap_or(stored)).is_ok()
        })
    };
    let mut patterns = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    // `spot_objects`, `spot_rows` and `spot` hold the `spotObjects` entries, the rows of the objects filled or stroked with a spot
    // color, in walk order, and the spot colors they use. `spot_swatches` names the document's spot colors once, for every paint.
    let spot_swatches = spot_swatches(d);
    let mut spot_objects: Vec<Value> = vec![];
    let mut spot_rows: Vec<(String, String)> = vec![];
    let mut spot = BTreeSet::new();
    let (spot_label, pattern, gradient) = (|p: &Paint| spot_labels(&spot_swatches, p), one_label(pattern_label), one_label(gradient_label));
    let mut styles: Vec<&str> = if selection_only { vec![] } else { d.graphic_styles.iter().map(|g| g.name.as_str()).collect() };
    // `library` holds the Brushes panel's brushes, and `brushes` the brushes the visited objects' strokes draw with, each once,
    // in walk order.
    let library = vectorcraft_brush::library(d);
    let mut brushes: Vec<String> = vec![];
    // `repeats` and `repeat_rows` hold the `repeatObjects` entries and the rows of the Repeat objects that the walk visits, in walk order.
    let mut repeats: Vec<Value> = vec![];
    let mut repeat_rows: Vec<(String, String)> = vec![];
    // `pattern_objects` and `pattern_rows` hold the `patternObjects` entries and the rows of the objects filled or stroked with a pattern, in walk order.
    let mut pattern_objects: Vec<Value> = vec![];
    let mut pattern_rows: Vec<(String, String)> = vec![];
    // `gradient_objects`, `gradient_rows` and `gradients` hold the `gradientObjects` entries, the rows of the objects filled or stroked
    // with a gradient, in walk order, and the gradients they use.
    let mut gradient_objects: Vec<Value> = vec![];
    let mut gradient_rows: Vec<(String, String)> = vec![];
    let mut gradients = BTreeSet::new();
    // The objects the walk has visited: a selection can hold a group and objects inside it, and each object counts once.
    // The set holds each object's address, so two objects that share an id (as a damaged file can hold) count as two.
    let mut seen: HashSet<*const Node> = HashSet::new();
    // The members of the document's compound paths: the compound path's own fills and strokes paint them.
    let mut members: HashSet<*const Node> = HashSet::new();
    d.walk(|n| {
        if let NodeKind::Compound { children, .. } = &n.kind {
            members.extend(children.iter().map(|c| std::ptr::from_ref(&**c)));
        }
    });
    for r in roots {
        r.walk(&mut |n| {
            if !seen.insert(std::ptr::from_ref(n)) {
                return;
            }
            let k = match &n.kind {
                // The Guides row counts paths made into guides; ruler guides are not objects.
                NodeKind::Layer { .. } | NodeKind::RulerGuide { .. } => None,
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
                    // A linked file outside a confined request's read roots: `link` is null, and the row
                    // names the file alone.
                    let link = im.link.as_ref().map(|l| (l, may_read(&l.path)));
                    images.insert(
                        im.key.clone(),
                        json!({ "width": im.width, "height": im.height, "linked": im.link.is_some(), "link": link.and_then(|(l, ok)| ok.then_some(&l.path)) }),
                    );
                    let name = im.link.as_ref().map_or_else(|| n.display_name(), |l| l.name().to_string());
                    image_rows.push((name, im.width, im.height, link.map(|(l, ok)| if ok { l.path.clone() } else { l.name().to_string() })));
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
            // The spot colors, the patterns and the gradients this object's fills and strokes paint, each once.
            let painted_runs = painted_runs(d, &spot_swatches, n);
            let mut used_spots = own_paints(d, n, &members, painted_runs.as_ref(), &spot_label);
            let mut used_patterns = own_paints(d, n, &members, painted_runs.as_ref(), &pattern);
            let mut used_gradients = own_paints(d, n, &members, painted_runs.as_ref(), &gradient);
            // The characters' own paints: an inline graphic draws no outline and does not take its
            // run's paint (a list marker set in its run does), and a stroke with no width paints
            // nothing.
            if let Some((t, painted)) = &painted_runs {
                for run in painted.iter().filter_map(|i| t.runs.get(*i)) {
                    let stroke = (run.style.stroke_width > 0.0).then_some(&run.style.stroke);
                    for paint in std::iter::once(&run.style.fill).chain(stroke) {
                        let labels: [(&mut Vec<String>, &PaintNames<'_>); 3] =
                            [(&mut used_spots, &spot_label), (&mut used_patterns, &pattern), (&mut used_gradients, &gradient)];
                        for (used, label) in labels {
                            for name in label(paint) {
                                if !used.contains(&name) {
                                    used.push(name);
                                }
                            }
                        }
                    }
                }
            }
            // A grayscale or 1-bit image colorized with a spot color prints it, while its pixels take
            // the color, as the renderer draws it.
            if let NodeKind::Image(im) = &n.kind
                && let Some(colorize) = n.extras().colorize.as_deref()
                && d.image_blob(im).is_some_and(|b| b.takes_colorize())
            {
                for name in spot_label(colorize) {
                    if !used_spots.contains(&name) {
                        used_spots.insert(0, name);
                    }
                }
            }
            // Objects counts the objects that Spot Color Objects, Pattern Objects and Gradient Objects list.
            if !used_spots.is_empty() {
                let name = n.display_name();
                spot.extend(used_spots.iter().cloned());
                spot_rows.push((name.clone(), used_spots.join(", ")));
                spot_objects.push(json!({ "id": n.id.0, "name": name, "spotColors": used_spots }));
                *counts.entry("spots").or_default() += 1;
            }
            if !used_patterns.is_empty() {
                let name = n.display_name();
                patterns.extend(used_patterns.iter().cloned());
                pattern_rows.push((name.clone(), used_patterns.join(", ")));
                pattern_objects.push(json!({ "id": n.id.0, "name": name, "patterns": used_patterns }));
                *counts.entry("patterns").or_default() += 1;
            }
            if !used_gradients.is_empty() {
                let name = n.display_name();
                gradients.extend(used_gradients.iter().cloned());
                gradient_rows.push((name.clone(), used_gradients.join(", ")));
                gradient_objects.push(json!({ "id": n.id.0, "name": name, "gradients": used_gradients }));
                *counts.entry("gradients").or_default() += 1;
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
            for b in applied_brushes(d, n, &members) {
                if !brushes.contains(&b) {
                    brushes.push(b);
                }
            }
            if let NodeKind::Repeat(spec) = &n.kind {
                let (entry, row) = repeat_entry(d, n, spec);
                repeats.push(entry);
                repeat_rows.push(row);
            }
        });
    }
    // The document's fonts include those of the type in opacity masks and in the art of its
    // symbols and patterns, as Find Font lists them.
    if !selection_only {
        fonts.extend(super::fonts::art_texts_of(d).into_iter().flat_map(|(_, t, _)| t.runs.iter().map(|r| super::fonts::font_of(&r.style))));
    }
    let font_names: Vec<String> = fonts.iter().map(super::fonts::font_label).collect();
    // Each brush the art uses with its type, found by name in the Brushes panel as the renderer finds it (the first of two
    // brushes with one name); a brush the panel does not hold draws as a plain stroke and has no type. A row names the type
    // as the New Brush dialog does (`Art Brush`), and the panel translates these values when it shows them.
    let kinds: HashMap<&str, &vectorcraft_brush::BrushKind> = library.iter().map(|b| (b.name.as_str(), &b.kind)).collect();
    let brush_entries: Vec<Value> =
        brushes.iter().map(|name| json!({ "name": name, "type": kinds.get(name.as_str()).map(|k| k.type_id()) })).collect();
    let brush_rows: Vec<(String, String)> = brushes
        .iter()
        .map(|name| {
            (name.clone(), kinds.get(name.as_str()).map_or_else(|| "missing from the Brushes panel".to_string(), |k| format!("{} Brush", k.label())))
        })
        .collect();
    let mut out = json!({
        "document": document_summary(d),
        "objects": counts,
        "pathLength": path_length,
        "fonts": font_names,
        "images": images,
        "symbols": symbols,
        "spotColors": spot,
        "spotColorNames": spot,
        "spotObjects": spot_objects,
        "swatches": d.swatches_iter().count(),
        "graphicStyles": d.graphic_styles.len(),
        "graphicStyleNames": styles,
        "brushNames": brushes,
        "brushes": brush_entries,
        "panelBrushNames": library.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        "repeatObjects": repeats,
        "characterStyles": d.char_styles.len(),
        "paragraphStyles": d.para_styles.len(),
        "patterns": d.patterns.len(),
        "patternNames": patterns,
        "patternObjects": pattern_objects,
        "gradientNames": gradients,
        "gradientObjects": gradient_objects,
    });
    // The document's name: its file's, else its title.
    out["document"]["name"] = json!(st.title());
    let sections = sections(d, &out, &fonts, &image_rows, &brush_rows, &repeat_rows, &spot_rows, &pattern_rows, &gradient_rows, category);
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
        let t = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
        let i = s.execute("document.info", &json!({})).unwrap();
        // The type's characters take the current gradient fill, so it is a gradient object too.
        assert_eq!(
            (i["objects"]["paths"].as_u64(), i["objects"]["textObjects"].as_u64(), i["objects"]["gradients"].as_u64()),
            (Some(1), Some(1), Some(2))
        );
        assert_eq!((i["gradientObjects"][0]["id"].clone(), i["gradientObjects"][1]["id"].clone()), (r.clone(), t));
        assert_eq!(i["fonts"][0], "Source Sans 3 Regular");
        assert_eq!(i["document"]["artboards"][0]["width"], 200.0);
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
        assert!(i["objects"]["textObjects"].is_null() && i["objects"]["paths"] == 1);
    }

    /// Document Info's fonts include the type in an opacity mask, in a symbol's art and in a
    /// pattern swatch that nothing paints with; with `selectionOnly`, only the selected type's.
    #[test]
    fn info_fonts_include_masks_symbols_and_patterns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
        let text = |s: &mut Session, font: &str| id(s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": font})).unwrap());
        let rect = |s: &mut Session| id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
        let (masked, in_symbol) = (rect(&mut s), rect(&mut s));
        for (owner, font) in [(masked, "Mask Family"), (in_symbol, "Symbol Mask Family")] {
            let t = text(&mut s, font);
            s.edit("Mask", |d, _| {
                let art = d.remove(t)?;
                d.node_mut(owner).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new((*art).clone(), true)));
                Ok(())
            })
            .unwrap();
        }
        s.execute("symbol.new", &json!({"name": "Mark", "ids": [in_symbol.0]})).unwrap();
        let t = text(&mut s, "Pattern Family");
        s.execute("object.pattern.make", &json!({"ids": [t.0], "name": "Unused", "edit": false})).unwrap();
        s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        let plain = text(&mut s, "Source Sans 3");
        let fonts = |s: &mut Session, p: Value| s.execute("document.info", &p).unwrap()["fonts"].clone();
        assert_eq!(
            fonts(&mut s, json!({})),
            json!(["Mask Family Regular", "Pattern Family Regular", "Source Sans 3 Regular", "Symbol Mask Family Regular"])
        );
        s.execute("select.set", &json!({"ids": [plain.0]})).unwrap();
        assert_eq!(fonts(&mut s, json!({"selectionOnly": true})), json!(["Source Sans 3 Regular"]));
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

    /// Document Info › Brushes lists the brushes the art's strokes use, each once with its type, and
    /// leaves out the Brushes panel's brushes no art uses (`panelBrushNames` lists those). With
    /// Selection Only it lists the brushes the selected objects' strokes use.
    #[test]
    fn info_lists_the_brushes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let panel: Vec<Value> =
            s.execute("brush.list", &json!({})).unwrap()["brushes"].as_array().unwrap().iter().map(|b| b["name"].clone()).collect();
        let brushes = |s: &mut Session, p: Value| s.execute("document.info", &p).unwrap();
        // No art uses a brush: the category is empty.
        let i = brushes(&mut s, json!({"category": "brushes"}));
        assert_eq!((i["brushNames"].clone(), i["brushes"].clone(), i["panelBrushNames"].clone()), (json!([]), json!([]), json!(panel)));
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("brushes"), Some("Brushes")));
        assert_eq!(i["sections"][0]["rows"], json!([]));
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let c = s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("brush.apply", &json!({"ids": [a], "name": "Charcoal"})).unwrap();
        s.execute("brush.apply", &json!({"ids": [b], "name": "Dots"})).unwrap();
        s.execute("brush.apply", &json!({"ids": [c], "name": "Charcoal"})).unwrap();
        // The whole document: the two brushes the art uses, each once, with their types; "3 pt. Round" (a Calligraphic
        // brush in the panel) is left out.
        let i = brushes(&mut s, json!({"category": "brushes"}));
        assert_eq!(i["brushNames"], json!(["Charcoal", "Dots"]));
        assert_eq!(i["brushes"], json!([{"name": "Charcoal", "type": "art"}, {"name": "Dots", "type": "scatter"}]));
        assert_eq!(i["sections"][0]["rows"], json!([["Charcoal", "Art Brush"], ["Dots", "Scatter Brush"]]));
        assert!(panel.contains(&json!("3 pt. Round")) && i["panelBrushNames"] == json!(panel));
        let text = brushes(&mut s, json!({"category": "brushes", "format": "text"}))["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nBRUSHES\nCharcoal: Art Brush\nDots: Scatter Brush\n") && !text.contains("3 pt. Round"), "{text}");
        assert!(!text.contains("OBJECTS"), "{text}");
        // Selection Only: the brushes the selected objects' strokes use.
        let selected = |s: &mut Session| brushes(s, json!({"selectionOnly": true, "category": "brushes"}));
        s.execute("select.set", &json!({"ids": [a, b, c]})).unwrap();
        assert_eq!(selected(&mut s)["brushNames"], json!(["Charcoal", "Dots"]));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let i = selected(&mut s);
        assert_eq!(
            (i["brushes"].clone(), i["sections"][0]["rows"].clone()),
            (json!([{"name": "Dots", "type": "scatter"}]), json!([["Dots", "Scatter Brush"]]))
        );
        let text = brushes(&mut s, json!({"selectionOnly": true, "category": "brushes", "format": "text"}))["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nBRUSHES\nDots: Scatter Brush\n") && !text.contains("Charcoal"), "{text}");
        // A stroke without a brush lists none.
        let plain = s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [plain]})).unwrap();
        assert_eq!(selected(&mut s)["brushNames"], json!([]));
    }

    /// Document Info › Brushes leaves out brushes on strokes that draw nothing (hidden, at an
    /// opacity of 0, with no width or paint, a guide's, a compound path member's, a path's with no segment, type's that
    /// draws no characters, a group's own), lists a brush the Brushes panel does
    /// not hold with no type, and gives a name two panel brushes share the type the renderer uses.
    #[test]
    fn info_brushes_skip_undrawn_strokes_and_name_missing_brushes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let square = |s: &mut Session, x: f64| {
            s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
        };
        let (hidden, thin, guide, missing) = (square(&mut s, 0.0), square(&mut s, 20.0), square(&mut s, 40.0), square(&mut s, 60.0));
        let (empty, faint) = (square(&mut s, 80.0), square(&mut s, 200.0));
        s.execute("brush.apply", &json!({"ids": [hidden, thin, guide, missing, empty, faint], "name": "Charcoal"})).unwrap();
        let (p, q) = (square(&mut s, 100.0), square(&mut s, 120.0));
        s.execute("select.set", &json!({"ids": [p, q]})).unwrap();
        let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        // A compound path's member: the compound path's own stroke draws it.
        let NodeKind::Compound { children, .. } = &mut d.node_mut(NodeId(compound)).unwrap().kind else { panic!("not a compound path") };
        let member = &mut std::sync::Arc::make_mut(&mut children[0]).appearance;
        member.set_stroke(Paint::solid(vectorcraft_color::Color::BLACK));
        member.stroke_mut().unwrap().brush = Some("Dots".into());
        d.node_mut(NodeId(hidden)).unwrap().appearance.stroke_mut().unwrap().visible = false;
        d.node_mut(NodeId(thin)).unwrap().appearance.stroke_mut().unwrap().width = 0.0;
        // A stroke at an opacity of 0 draws nothing.
        d.node_mut(NodeId(faint)).unwrap().appearance.stroke_mut().unwrap().opacity = 0.0;
        if let NodeKind::Path { guide: g, .. } = &mut d.node_mut(NodeId(guide)).unwrap().kind {
            *g = true;
        }
        // A path with no segment: its stroke follows nothing.
        if let NodeKind::Path { path, .. } = &mut d.node_mut(NodeId(empty)).unwrap().kind {
            *path = vectorcraft_geom::PathData::default();
        }
        d.node_mut(NodeId(missing)).unwrap().appearance.stroke_mut().unwrap().brush = Some("Gone".into());
        let i = s.execute("document.info", &json!({"category": "brushes"})).unwrap();
        assert_eq!(i["brushes"], json!([{"name": "Gone", "type": null}]));
        assert_eq!(i["sections"][0]["rows"], json!([["Gone", "missing from the Brushes panel"]]));
        // A stroke with no paint draws nothing.
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        d.node_mut(NodeId(missing)).unwrap().appearance.stroke_mut().unwrap().paint = Paint::None;
        assert_eq!(s.execute("document.info", &json!({"category": "brushes"})).unwrap()["brushes"], json!([]));
        // Two panel brushes with one name: the type is the one the renderer draws with, the first one's.
        let twin = square(&mut s, 140.0);
        s.execute("brush.apply", &json!({"ids": [twin], "name": "Charcoal"})).unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let mut lib = vectorcraft_brush::library(d);
        let kinds: Vec<vectorcraft_brush::BrushKind> =
            ["Charcoal", "Dots"].iter().map(|n| lib.iter().find(|b| b.name == *n).unwrap().kind.clone()).collect();
        lib.extend(kinds.into_iter().map(|kind| vectorcraft_brush::Brush { name: "Twin".into(), kind }));
        vectorcraft_brush::store(d, &lib);
        d.node_mut(NodeId(twin)).unwrap().appearance.stroke_mut().unwrap().brush = Some("Twin".into());
        assert_eq!(s.execute("document.info", &json!({"category": "brushes"})).unwrap()["brushes"], json!([{"name": "Twin", "type": "art"}]));
        // Type: a brushed stroke on drawn characters is listed, one on spaces alone is not.
        let blank = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "  "})).unwrap()["id"].as_u64().unwrap();
        let word = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        for (id, name) in [(blank, "Dots"), (word, "Arrow")] {
            let ap = &mut d.node_mut(NodeId(id)).unwrap().appearance;
            ap.set_stroke(Paint::solid(vectorcraft_color::Color::BLACK));
            ap.stroke_mut().unwrap().brush = Some(name.into());
        }
        let names = s.execute("document.info", &json!({"category": "brushes"})).unwrap()["brushNames"].clone();
        assert_eq!(names, json!(["Twin", "Arrow"]));
        // A group's own stroke: the group draws its members.
        let (m1, m2) = (square(&mut s, 160.0), square(&mut s, 180.0));
        s.execute("select.set", &json!({"ids": [m1, m2]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let ap = &mut d.node_mut(NodeId(group)).unwrap().appearance;
        ap.set_stroke(Paint::solid(vectorcraft_color::Color::BLACK));
        ap.stroke_mut().unwrap().brush = Some("Leaves".into());
        let names = s.execute("document.info", &json!({"category": "brushes"})).unwrap()["brushNames"].clone();
        assert_eq!(names, json!(["Twin", "Arrow"]));
    }

    /// Document Info › Repeat Objects lists each Repeat object by name with its kind and the number
    /// of instances it makes (for a grid, its rows and columns, followed by the number it draws when
    /// that is fewer), one in a group included. With Selection Only it lists the Repeat objects in
    /// the selection, each once when the selection holds a group and a Repeat object inside it.
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
        // A selection that holds the group and the Repeat object inside it lists that object once.
        for ids in [json!([group, mirror]), json!([mirror, group])] {
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            assert_eq!(names(&mut s), ["<Mirror Repeat>"], "{ids}");
        }
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

    /// Document Info › Pattern Objects lists each object filled or stroked with a pattern once,
    /// with the patterns it uses, one in a group and a type object whose characters use a pattern
    /// included; a pattern no art uses adds no row, and Objects counts the listed objects. With
    /// Selection Only it lists the patterned objects in the selection, and the report writes the
    /// same rows.
    #[test]
    fn info_lists_the_pattern_objects() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let square =
            |s: &mut Session, x: f64| s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let tile = square(&mut s, 0.0);
        s.execute("object.pattern.make", &json!({"ids": [tile], "name": "Dots", "edit": false})).unwrap();
        let spare = square(&mut s, 20.0);
        s.execute("object.pattern.make", &json!({"ids": [spare], "name": "Unused", "edit": false})).unwrap();
        let (a, b, c) = (square(&mut s, 100.0), square(&mut s, 200.0), square(&mut s, 300.0));
        s.execute("paint.setFill", &json!({"ids": [a], "swatch": "Dots"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [a], "swatch": "Dots"})).unwrap();
        s.execute("layer.setProps", &json!({"id": a, "name": "Both"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [b], "swatch": "Dots"})).unwrap();
        s.execute("select.set", &json!({"ids": [b, c]})).unwrap();
        s.execute("object.group", &json!({})).unwrap();
        // A type object whose characters are filled with the pattern.
        let t = s.execute("text.create", &json!({"x": 0, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [t], "swatch": "Dots"})).unwrap();
        s.execute("layer.setProps", &json!({"id": t, "name": "Type"})).unwrap();
        // A hidden pattern fill and a pattern stroke with no width paint nothing, so neither object is listed.
        let (d, e) = (square(&mut s, 100.0), square(&mut s, 200.0));
        s.execute("paint.setStroke", &json!({"ids": [d], "none": true})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [d], "swatch": "Dots"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [d], "index": 0, "visible": false})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [e], "none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [e], "swatch": "Dots"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [e], "index": 1, "weight": 0})).unwrap();
        // Nor does a pattern fill at an opacity of 0.
        let clear = square(&mut s, 220.0);
        s.execute("paint.setStroke", &json!({"ids": [clear], "none": true})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [clear], "swatch": "Dots"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [clear], "index": 0, "opacity": 0})).unwrap();
        // A type object with no characters paints nothing either.
        let empty = s.execute("text.create", &json!({"x": 0, "y": 300, "text": ""})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [empty], "swatch": "Dots"})).unwrap();
        // Its own fill (a copy of its characters' pattern fill) paints on its characters, so it paints nothing.
        s.execute("appearance.addFill", &json!({"ids": [empty]})).unwrap();
        // Nor does type of only spaces and a line break, or a guide made from a patterned path.
        let blank = s.execute("text.create", &json!({"x": 0, "y": 320, "text": " \n"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [blank], "swatch": "Dots"})).unwrap();
        let guide = square(&mut s, 300.0);
        s.execute("paint.setFill", &json!({"ids": [guide], "swatch": "Dots"})).unwrap();
        s.execute("select.set", &json!({"ids": [guide]})).unwrap();
        s.execute("view.guides.make", &json!({})).unwrap();
        // Nor does a compound path's member's own pattern fill: the compound path's own fill paints it.
        let (m1, m2) = (square(&mut s, 320.0), square(&mut s, 340.0));
        s.execute("select.set", &json!({"ids": [m1, m2]})).unwrap();
        let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [compound], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [compound], "none": true})).unwrap();
        let node = |s: &Session, id: &Value| s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().clone();
        let (member, paint) = (node(&s, &compound).children().unwrap()[0].id, node(&s, &a).appearance);
        // Nor does an image's pattern fill without an effect that gives it a shape.
        let pic = square(&mut s, 360.0);
        s.execute("select.set", &json!({"ids": [pic]})).unwrap();
        s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
        let image = s.doc().unwrap().selection.objects[0];
        s.edit("own paint", |d, _| {
            d.node_mut(member).unwrap().appearance = paint.clone();
            d.node_mut(image).unwrap().appearance = paint;
            Ok(())
        })
        .unwrap();
        // Underlined spaces draw a bar in the pattern, so that type object is listed.
        let bar = s.execute("text.create", &json!({"x": 0, "y": 340, "text": "  "})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [bar], "swatch": "Dots"})).unwrap();
        s.execute("select.set", &json!({"ids": [bar]})).unwrap();
        s.execute("text.setFormat", &json!({"underline": true})).unwrap();
        s.execute("layer.setProps", &json!({"id": bar, "name": "Bar"})).unwrap();
        // A list marker set in an inline graphic's run draws in that run's pattern, so that type object is listed.
        s.execute("shape.ellipse", &json!({"x": 300, "y": 300, "width": 10, "height": 10})).unwrap();
        s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        let listed = s.execute("text.create", &json!({"x": 0, "y": 360, "text": ""})).unwrap()["id"].clone();
        s.execute("text.insertInline", &json!({"id": listed, "at": 0, "symbol": "Mark"})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [listed], "list": "bullet"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [listed], "swatch": "Dots"})).unwrap();
        s.execute("layer.setProps", &json!({"id": listed, "name": "Listed"})).unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        assert_eq!(
            i["patternObjects"],
            json!([
                {"id": a, "name": "Both", "patterns": ["Dots"]},
                {"id": b, "name": "<Rectangle>", "patterns": ["Dots"]},
                {"id": t, "name": "Type", "patterns": ["Dots"]},
                {"id": bar, "name": "Bar", "patterns": ["Dots"]},
                {"id": listed, "name": "Listed", "patterns": ["Dots"]},
            ])
        );
        assert_eq!(i["patternNames"], json!(["Dots"]));
        // Objects counts the objects that Pattern Objects lists.
        assert_eq!(i["objects"]["patterns"].as_u64(), Some(5), "{}", i["objects"]);
        s.execute("select.set", &json!({"ids": [d, e, clear, empty, blank, guide, compound, image.0]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
        assert!(i["objects"]["patterns"].is_null() && i["patternObjects"] == json!([]), "{}", i["objects"]);
        let i = s.execute("document.info", &json!({"category": "patterns"})).unwrap();
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("patterns"), Some("Pattern Objects")));
        assert_eq!(
            i["sections"][0]["rows"],
            json!([["Both", "Dots"], ["<Rectangle>", "Dots"], ["Type", "Dots"], ["Bar", "Dots"], ["Listed", "Dots"]])
        );
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "patterns"})).unwrap();
        assert_eq!(i["sections"][0]["rows"], json!([["<Rectangle>", "Dots"]]));
        s.execute("select.set", &json!({"ids": [t]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "patterns"})).unwrap();
        assert_eq!((i["sections"][0]["rows"].clone(), i["patternNames"].clone()), (json!([["Type", "Dots"]]), json!(["Dots"])));
        let text = s.execute("document.info", &json!({"category": "patterns", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nPATTERN OBJECTS\nBoth: Dots\n<Rectangle>: Dots\nType: Dots\nBar: Dots\nListed: Dots\n"), "{text}");
        assert!(!text.contains("Unused"), "{text}");
    }

    /// Document Info › Gradient Objects lists each object filled or stroked with a gradient once,
    /// with the gradients it uses (a linked swatch by name, else the gradient's kind), one in a
    /// group and a type object whose characters use a gradient included; a gradient swatch no art
    /// uses adds no row, paint that draws nothing is left out by the rules Pattern Objects follows,
    /// and Objects counts the listed objects. With Selection Only it lists the objects in the
    /// selection, each once, and the report writes the same rows.
    #[test]
    fn info_lists_the_gradient_objects() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let square =
            |s: &mut Session, x: f64| s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let ramp = |kind: &str| json!({"kind": kind, "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        s.execute("swatch.new", &json!({"name": "Ramp", "gradient": ramp("linear")})).unwrap();
        s.execute("swatch.new", &json!({"name": "Spare", "gradient": ramp("radial")})).unwrap();
        let (a, b, c, f) = (square(&mut s, 100.0), square(&mut s, 200.0), square(&mut s, 300.0), square(&mut s, 0.0));
        s.execute("paint.setFill", &json!({"ids": [a], "swatch": "Ramp"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [a], "gradient": ramp("linear")})).unwrap();
        s.execute("layer.setProps", &json!({"id": a, "name": "Both"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [b], "gradient": ramp("radial")})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [f], "gradient": ramp("freeform")})).unwrap();
        s.execute("select.set", &json!({"ids": [b, c]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
        // A type object whose characters are filled with the gradient.
        let t = s.execute("text.create", &json!({"x": 0, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [t], "swatch": "Ramp"})).unwrap();
        s.execute("layer.setProps", &json!({"id": t, "name": "Type"})).unwrap();
        // A hidden gradient fill and a gradient stroke with no width paint nothing, so neither object is listed.
        let (d, e) = (square(&mut s, 100.0), square(&mut s, 200.0));
        s.execute("paint.setStroke", &json!({"ids": [d], "none": true})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [d], "swatch": "Ramp"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [d], "index": 0, "visible": false})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [e], "none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [e], "gradient": ramp("linear")})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [e], "index": 1, "weight": 0})).unwrap();
        // Type with no characters, or of only spaces and a line break, paints nothing, and neither
        // does a guide made from a path with a gradient.
        let empty = s.execute("text.create", &json!({"x": 0, "y": 300, "text": ""})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [empty], "swatch": "Ramp"})).unwrap();
        s.execute("appearance.addFill", &json!({"ids": [empty]})).unwrap();
        let blank = s.execute("text.create", &json!({"x": 0, "y": 320, "text": " \n"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [blank], "swatch": "Ramp"})).unwrap();
        let guide = square(&mut s, 300.0);
        s.execute("paint.setFill", &json!({"ids": [guide], "swatch": "Ramp"})).unwrap();
        s.execute("select.set", &json!({"ids": [guide]})).unwrap();
        s.execute("view.guides.make", &json!({})).unwrap();
        // Nor does a compound path's member's own gradient fill, or an image's gradient fill
        // without an effect that gives it a shape.
        let (m1, m2) = (square(&mut s, 320.0), square(&mut s, 340.0));
        s.execute("select.set", &json!({"ids": [m1, m2]})).unwrap();
        let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [compound], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [compound], "none": true})).unwrap();
        let node = |s: &Session, id: &Value| s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().clone();
        let (member, paint) = (node(&s, &compound).children().unwrap()[0].id, node(&s, &a).appearance);
        let pic = square(&mut s, 360.0);
        s.execute("select.set", &json!({"ids": [pic]})).unwrap();
        s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
        let image = s.doc().unwrap().selection.objects[0];
        s.edit("own paint", |d, _| {
            d.node_mut(member).unwrap().appearance = paint.clone();
            d.node_mut(image).unwrap().appearance = paint;
            Ok(())
        })
        .unwrap();
        // Underlined spaces draw a bar in the gradient, so that type object is listed.
        let bar = s.execute("text.create", &json!({"x": 0, "y": 340, "text": "  "})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [bar], "swatch": "Ramp"})).unwrap();
        s.execute("select.set", &json!({"ids": [bar]})).unwrap();
        s.execute("text.setFormat", &json!({"underline": true})).unwrap();
        s.execute("layer.setProps", &json!({"id": bar, "name": "Bar"})).unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        assert_eq!(
            i["gradientObjects"],
            json!([
                {"id": a, "name": "Both", "gradients": ["Ramp", "Linear gradient"]},
                {"id": b, "name": "<Rectangle>", "gradients": ["Radial gradient"]},
                {"id": f, "name": "<Rectangle>", "gradients": ["Freeform gradient"]},
                {"id": t, "name": "Type", "gradients": ["Ramp"]},
                {"id": bar, "name": "Bar", "gradients": ["Ramp"]},
            ])
        );
        assert_eq!(i["gradientNames"], json!(["Freeform gradient", "Linear gradient", "Radial gradient", "Ramp"]));
        // Objects counts the objects that Gradient Objects lists.
        assert_eq!(i["objects"]["gradients"].as_u64(), Some(5), "{}", i["objects"]);
        s.execute("select.set", &json!({"ids": [d, e, empty, blank, guide, compound, image.0]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
        assert!(i["objects"]["gradients"].is_null() && i["gradientObjects"] == json!([]), "{}", i["objects"]);
        let i = s.execute("document.info", &json!({"category": "gradients"})).unwrap();
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("gradients"), Some("Gradient Objects")));
        assert_eq!(
            i["sections"][0]["rows"],
            json!([
                ["Both", "Ramp, Linear gradient"],
                ["<Rectangle>", "Radial gradient"],
                ["<Rectangle>", "Freeform gradient"],
                ["Type", "Ramp"],
                ["Bar", "Ramp"]
            ])
        );
        // A selection that holds the group and the object inside it lists that object once.
        for ids in [json!([group, b]), json!([b, group])] {
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "gradients"})).unwrap();
            assert_eq!(i["sections"][0]["rows"], json!([["<Rectangle>", "Radial gradient"]]), "{ids}");
            assert_eq!(i["objects"]["gradients"].as_u64(), Some(1), "{ids}");
        }
        s.execute("select.set", &json!({"ids": [t]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "gradients"})).unwrap();
        assert_eq!((i["sections"][0]["rows"].clone(), i["gradientNames"].clone()), (json!([["Type", "Ramp"]]), json!(["Ramp"])));
        let text = s.execute("document.info", &json!({"category": "gradients", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(
            text.contains("\nGRADIENT OBJECTS\nBoth: Ramp, Linear gradient\n<Rectangle>: Radial gradient\n<Rectangle>: Freeform gradient\nType: Ramp\nBar: Ramp\n"),
            "{text}"
        );
        assert!(!text.contains("Spare"), "{text}");
    }

    /// Document Info › Spot Color Objects lists each object filled or stroked with a spot color once,
    /// with the spot colors it uses, as a solid color or in a gradient's stops, one in a group and a
    /// type object whose characters use one included. A spot swatch no art uses adds no row; a
    /// process color that looks the same, Registration and a tint of 0 are not spot colors; paint
    /// that draws nothing is left out by the rules Pattern Objects follows; and Objects counts the
    /// listed objects. With Selection Only it lists the objects in the selection, each once, and the
    /// report writes the same rows.
    #[test]
    fn info_lists_the_spot_color_objects() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        // An empty document lists none.
        let i = s.execute("document.info", &json!({"category": "spotColors"})).unwrap();
        assert_eq!((i["spotObjects"].clone(), i["spotColorNames"].clone(), i["spotColors"].clone()), (json!([]), json!([]), json!([])));
        assert!(i["objects"]["spots"].is_null() && i["sections"][0]["rows"] == json!([]), "{i}");
        let text = s.execute("document.info", &json!({"category": "spotColors", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nSPOT COLOR OBJECTS\nNone\n"), "{text}");
        let ink = |c: f64| json!({"model": "cmyk", "c": c, "m": 0.5, "y": 0.0, "k": 0.0});
        s.execute("swatch.new", &json!({"name": "Ink", "color": ink(1.0), "spot": true})).unwrap();
        s.execute("swatch.new", &json!({"name": "Gold", "color": ink(0.2), "spot": true})).unwrap();
        s.execute("swatch.new", &json!({"name": "Spare", "color": ink(0.4), "spot": true})).unwrap();
        // A global process color of the same color as Ink.
        s.execute("swatch.new", &json!({"name": "Look", "color": ink(1.0), "global": true})).unwrap();
        let square =
            |s: &mut Session, x: f64| s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let ramp = |a: &str, b: &str| json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000", "swatch": a}, {"offset": 1, "color": "#fff", "swatch": b}]});
        let (a, b, c, f, g, h) =
            (square(&mut s, 0.0), square(&mut s, 20.0), square(&mut s, 40.0), square(&mut s, 60.0), square(&mut s, 80.0), square(&mut s, 100.0));
        s.execute("paint.setFill", &json!({"ids": [a], "swatch": "Ink"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [a], "swatch": "Gold", "tint": 40})).unwrap();
        s.execute("layer.setProps", &json!({"id": a, "name": "Both"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [b], "swatch": "Look"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [b], "swatch": "Ink", "tint": 50})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [c], "swatch": "Ink", "tint": 0})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [f], "gradient": ramp("Gold", "Look")})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [g], "gradient": ramp("Look", "Look")})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [h], "swatch": "[Registration]"})).unwrap();
        for id in [&c, &g, &h] {
            s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [b, c]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
        // A type object whose characters are filled with a spot color.
        let t = s.execute("text.create", &json!({"x": 0, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [t], "swatch": "Ink"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [t], "none": true})).unwrap();
        s.execute("layer.setProps", &json!({"id": t, "name": "Type"})).unwrap();
        // A hidden spot fill and a spot stroke with no width paint nothing, so neither object is listed.
        let (d, e) = (square(&mut s, 120.0), square(&mut s, 140.0));
        s.execute("paint.setStroke", &json!({"ids": [d], "none": true})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [d], "swatch": "Ink"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [d], "index": 0, "visible": false})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [e], "none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [e], "swatch": "Ink"})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [e], "index": 1, "weight": 0})).unwrap();
        // Type with no characters, or of only spaces and a line break, paints nothing, and neither
        // does a guide made from a path with a spot color.
        let empty = s.execute("text.create", &json!({"x": 0, "y": 300, "text": ""})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [empty], "swatch": "Ink"})).unwrap();
        s.execute("appearance.addFill", &json!({"ids": [empty]})).unwrap();
        let blank = s.execute("text.create", &json!({"x": 0, "y": 320, "text": " \n"})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [blank], "swatch": "Ink"})).unwrap();
        let guide = square(&mut s, 300.0);
        s.execute("paint.setFill", &json!({"ids": [guide], "swatch": "Ink"})).unwrap();
        s.execute("select.set", &json!({"ids": [guide]})).unwrap();
        s.execute("view.guides.make", &json!({})).unwrap();
        // Nor does a compound path's member's own spot fill, or an image's spot fill without an
        // effect that gives it a shape.
        let (m1, m2) = (square(&mut s, 320.0), square(&mut s, 340.0));
        s.execute("select.set", &json!({"ids": [m1, m2]})).unwrap();
        let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [compound], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [compound], "none": true})).unwrap();
        let node = |s: &Session, id: &Value| s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().clone();
        let (member, paint) = (node(&s, &compound).children().unwrap()[0].id, node(&s, &a).appearance);
        let pic = square(&mut s, 360.0);
        s.execute("select.set", &json!({"ids": [pic]})).unwrap();
        s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
        let image = s.doc().unwrap().selection.objects[0];
        s.edit("own paint", |d, _| {
            d.node_mut(member).unwrap().appearance = paint.clone();
            d.node_mut(image).unwrap().appearance = paint;
            Ok(())
        })
        .unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        assert_eq!(
            i["spotObjects"],
            json!([
                {"id": a, "name": "Both", "spotColors": ["Ink", "Gold"]},
                {"id": b, "name": "<Rectangle>", "spotColors": ["Ink"]},
                {"id": f, "name": "<Rectangle>", "spotColors": ["Gold"]},
                {"id": t, "name": "Type", "spotColors": ["Ink"]},
            ])
        );
        assert_eq!((i["spotColorNames"].clone(), i["spotColors"].clone()), (json!(["Gold", "Ink"]), json!(["Gold", "Ink"])));
        // Objects counts the objects that Spot Color Objects lists.
        assert_eq!(i["objects"]["spots"].as_u64(), Some(4), "{}", i["objects"]);
        let objects = s.execute("document.info", &json!({"category": "objects"})).unwrap()["sections"][0]["rows"].clone();
        assert!(objects.as_array().unwrap().contains(&json!(["Spot Color Objects", "4"])), "{objects}");
        s.execute("select.set", &json!({"ids": [c, g, h, d, e, empty, blank, guide, compound, image.0]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
        assert!(i["objects"]["spots"].is_null() && i["spotObjects"] == json!([]) && i["spotColors"] == json!([]), "{i}");
        let i = s.execute("document.info", &json!({"category": "spotColors"})).unwrap();
        assert_eq!((i["sections"][0]["id"].as_str(), i["sections"][0]["title"].as_str()), (Some("spotColors"), Some("Spot Color Objects")));
        assert_eq!(i["sections"][0]["rows"], json!([["Both", "Ink, Gold"], ["<Rectangle>", "Ink"], ["<Rectangle>", "Gold"], ["Type", "Ink"]]));
        // A selection that holds the group and the object inside it lists that object once.
        for ids in [json!([group, b]), json!([b, group])] {
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "spotColors"})).unwrap();
            assert_eq!(i["sections"][0]["rows"], json!([["<Rectangle>", "Ink"]]), "{ids}");
            assert_eq!((i["objects"]["spots"].as_u64(), i["spotColorNames"].clone()), (Some(1), json!(["Ink"])), "{ids}");
        }
        s.execute("select.set", &json!({"ids": [t]})).unwrap();
        let i = s.execute("document.info", &json!({"selectionOnly": true, "category": "spotColors"})).unwrap();
        assert_eq!((i["sections"][0]["rows"].clone(), i["spotColors"].clone()), (json!([["Type", "Ink"]]), json!(["Ink"])));
        let text = s.execute("document.info", &json!({"category": "spotColors", "format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.contains("\nSPOT COLOR OBJECTS\nBoth: Ink, Gold\n<Rectangle>: Ink\n<Rectangle>: Gold\nType: Ink\n"), "{text}");
        assert!(!text.contains("Spare") && !text.contains("Look"), "{text}");
    }

    /// Spot Color Objects lists a grayscale image colorized with a spot color, and leaves out a
    /// gradient whose stops are all at an opacity of 0, which draws nothing.
    #[test]
    fn spot_color_objects_take_colorized_images_and_skip_clear_gradients() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("swatch.new", &json!({"name": "Ink", "color": {"model": "cmyk", "c": 0, "m": 1, "y": 1, "k": 0}, "spot": true})).unwrap();
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap())
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let image = s
            .edit("Place", |d, sel| {
                d.images.insert("gray".into(), vectorcraft_doc::ImageBlob::png(bytes));
                let id = d.alloc_id();
                let im = vectorcraft_doc::ImageObject {
                    key: "gray".into(),
                    width: 2,
                    height: 1,
                    xf: Default::default(),
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                };
                d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
                sel.set([id]);
                Ok(id)
            })
            .unwrap();
        s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
        assert!(s.doc().unwrap().doc.node(image).unwrap().extras().colorize.is_some());
        let clear = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 10, "height": 10})).unwrap()["id"].clone();
        let stops =
            json!([{"offset": 0, "color": "#000", "swatch": "Ink", "opacity": 0}, {"offset": 1, "color": "#fff", "swatch": "Ink", "opacity": 0}]);
        s.execute("paint.setFill", &json!({"ids": [clear], "gradient": {"kind": "linear", "stops": stops}})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [clear], "none": true})).unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        let name = s.doc().unwrap().doc.node(image).unwrap().display_name();
        assert_eq!(i["spotObjects"], json!([{"id": image.0, "name": name, "spotColors": ["Ink"]}]), "{i}");
        assert_eq!(i["objects"]["spots"].as_u64(), Some(1), "{}", i["objects"]);
    }

    /// Spot Color Objects leaves out a fill or stroke at an opacity of 0, which prints no ink, and
    /// keeps the object while another of its fills or strokes prints the spot color. Gradient
    /// Objects leaves out a clear gradient fill as well.
    #[test]
    fn spot_color_objects_skip_clear_fills_and_strokes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("swatch.new", &json!({"name": "Ink", "color": {"model": "cmyk", "c": 0, "m": 1, "y": 1, "k": 0}, "spot": true})).unwrap();
        let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("paint.setFill", &json!({"ids": [rect], "swatch": "Ink"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [rect], "swatch": "Ink"})).unwrap();
        let ramp = s.execute("shape.rectangle", &json!({"x": 50, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let stops = json!([{"offset": 0, "color": "#000", "swatch": "Ink"}, {"offset": 1, "color": "#fff", "swatch": "Ink"}]);
        s.execute("paint.setFill", &json!({"ids": [ramp], "gradient": {"kind": "linear", "stops": stops}})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [ramp], "none": true})).unwrap();
        let items = |s: &Session, id: &Value| s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().appearance.items.clone();
        let spots = |s: &mut Session| {
            let i = s.execute("document.info", &json!({})).unwrap();
            let mut ids: Vec<u64> = i["spotObjects"].as_array().unwrap().iter().map(|o| o["id"].as_u64().unwrap()).collect();
            ids.sort_unstable();
            (ids, i["objects"]["spots"].as_u64(), i["objects"]["gradients"].as_u64())
        };
        // The rectangle's fill at an opacity of 0: its stroke still prints the spot color.
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        let fill = items(&s, &rect).iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
        s.execute("appearance.setItem", &json!({"index": fill, "opacity": 0})).unwrap();
        assert_eq!(spots(&mut s), (vec![rect.as_u64().unwrap(), ramp.as_u64().unwrap()], Some(2), Some(1)));
        // Every fill and stroke of both objects at an opacity of 0.
        for id in [&rect, &ramp] {
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            for index in 0..items(&s, id).len() {
                s.execute("appearance.setItem", &json!({"index": index, "opacity": 0})).unwrap();
            }
        }
        assert_eq!(spots(&mut s), (vec![], None, None));
    }

    /// Document Info › Objects counts an object with a gradient or a pattern on both its fill and
    /// its stroke once, and with Selection Only a selection that holds a group and an object inside
    /// it counts that object once.
    #[test]
    fn info_counts_each_object_once() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let square =
            |s: &mut Session, x: f64| s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        let tile = square(&mut s, 0.0);
        s.execute("object.pattern.make", &json!({"ids": [tile], "name": "Dots", "edit": false})).unwrap();
        let (g, p) = (square(&mut s, 100.0), square(&mut s, 200.0));
        let gradient = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        s.execute("paint.setFill", &json!({"ids": [g], "gradient": gradient})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [g], "gradient": gradient})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [p], "swatch": "Dots"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [p], "swatch": "Dots"})).unwrap();
        let i = s.execute("document.info", &json!({})).unwrap();
        assert_eq!((i["objects"]["gradients"].as_u64(), i["objects"]["patterns"].as_u64()), (Some(1), Some(1)), "{}", i["objects"]);
        // A group of the two squares, selected together with one of them.
        s.execute("select.set", &json!({"ids": [g, p]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
        for ids in [json!([group, p]), json!([p, group])] {
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            let i = s.execute("document.info", &json!({"selectionOnly": true})).unwrap();
            let o = &i["objects"];
            assert_eq!(
                (o["paths"].as_u64(), o["groups"].as_u64(), o["gradients"].as_u64(), o["patterns"].as_u64()),
                (Some(2), Some(1), Some(1), Some(1)),
                "{ids}: {o}"
            );
            assert!((i["pathLength"].as_f64().unwrap() - 80.0).abs() < 1e-6, "{ids}: {}", i["pathLength"]);
        }
    }

    /// Document Info › Objects counts two objects that share an id (as a damaged file can hold) as
    /// two, in the whole document and in a selected group that holds both.
    #[test]
    fn info_counts_objects_that_share_an_id() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let square = |s: &mut Session, x: f64| {
            s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
        };
        let (a, b) = (square(&mut s, 0.0), square(&mut s, 100.0));
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [group]})).unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        d.node_mut(NodeId(b)).unwrap().id = NodeId(a);
        for p in [json!({}), json!({"selectionOnly": true})] {
            let i = s.execute("document.info", &p).unwrap();
            let o = &i["objects"];
            assert_eq!((o["paths"].as_u64(), o["groups"].as_u64()), (Some(2), Some(1)), "{p}: {o}");
            assert!((i["pathLength"].as_f64().unwrap() - 80.0).abs() < 1e-6, "{p}: {}", i["pathLength"]);
        }
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
