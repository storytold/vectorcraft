//! Color Effects (`addon.color.*`), a built-in add-on: its live effects (Exposure, Desaturate,
//! Duotone and Invert Colors, in Effect › Color Effects) are the effects crate's; here, Apply as
//! Filter writes any colour adjustment into the colours themselves, Select by Color Kind selects the
//! objects that use spot colours, carry a live colour adjustment, or are images of a colour model,
//! Print Ink Fixes ([`print_fixes`]) prepares a CMYK document's inks for print, Find Nearest
//! Swatch ([`nearest`]) matches colours to a swatch library's, Create Swatch Legend
//! ([`legend`]) draws labelled chips of the document's swatches, and Apply Fill to Stroke and Apply
//! Stroke to Fill ([`copy_paint`]) copy each object's own paint across. Its panel and its submenu,
//! Object › Plug-ins › Color Effects, are the UI's.

pub mod legend;
mod nearest;
mod print_fixes;
#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Document, Node, NodeId, NodeKind};
use vectorcraft_render::effects;

use super::super::colorcmds::recolor_links;
use super::super::paint::{restore_unpainted_stroke, set_run_stroke};
use super::super::select::walk_selectable;
use super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, has_selection, paint_targets, switch_or, text_param};
use crate::{EngineError, Result, Session};

/// The kinds Select by Color Kind selects (`addon.color.select`).
pub const SELECT_KINDS: [&str; 5] = ["spotColors", "colorEffects", "rgbImages", "cmykImages", "grayImages"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.color.select",
            "Select by Color Kind",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{kind: \"spotColors\" (objects painted with a spot colour swatch, solid or in a gradient's stops) | \"colorEffects\" (objects carrying a visible colour adjustment effect, on themselves or on a fill or stroke) | \"rgbImages\" | \"cmykImages\" | \"grayImages\" (embedded or linked images whose pixels are RGB, CMYK, or grayscale or 1-bit)} select those objects among the ones Select All reaches, inside groups too; locked and hidden ones are left out → {count}",
            has_doc,
            select
        ),
        cmd!(
            "addon.color.filter",
            "Apply as Filter",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{effect: a colour adjustment's effect id (adjust.brightnessContrast, adjust.curves, adjust.hueSaturation, adjust.levels, adjust.shiftToColor, adjust.temperatureTint, addon.color.exposure, addon.color.desaturate, addon.color.duotone or addon.color.invert), params?: {…} (as effect.apply; missing keys take the dialog defaults), fill?: true, stroke?: true, includeImages?: true, includePatterns?: true, ids?: [id…] (default: the selection)} recolour the objects and everything inside them as the live effect would draw them, but in their colours, as edit.colors.invert does (fills, strokes, type, gradient meshes, embedded images and pattern tiles), as one undo step; their effects stay. Duotone links a colour to its one tone's swatch as the live effect does. On a path directly in a graph series, the graph also stores the new paint of the topmost visible fill and stroke (docs/mcp.md) → {changed}",
            has_selection,
            filter
        ),
        cmd!(
            "addon.color.printFixes",
            "Print Ink Fixes",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{scope?: \"selection\" (the selection, or `ids`; by default, the document while nothing is selected) | \"document\" (every layer), inkLimit?: total ink % 1..400 (0 or left out: off), excludePureBlack?: true (black-only and grey colours keep their ink), removeWhiteOverprint?: false (white fills and strokes stop overprinting), overprintBlack?: black % 1..100 (black-only and grey fills and strokes this dark or darker overprint, as edit.colors.overprintBlack; 0 or left out: off), textOnly?: false (Overprint Black on type only), fill?: true, stroke?: true, ids?: [id…]} prepare a CMYK document's inks for print as one undo step: CMYK colours (solid, gradient stops, gradient meshes) with more total ink than inkLimit are scaled down to it (colours of spot swatches and Registration stay), white overprint is removed and black overprint set. Refused in RGB documents (File › Document Color Mode › CMYK Color first). On a path directly in a graph series, the graph also stores a paint the ink limit changes on the topmost visible fill or stroke (docs/mcp.md) → {changed, inkLimited: paints and meshes scaled, whiteOverprintRemoved: objects, blackOverprinted: objects}",
            has_doc,
            print_fixes::run
        ),
        cmd!(
            query "addon.color.nearestSwatch",
            "Find Nearest Swatch",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{library?: a swatch library's id or name (swatch.library.list: the built-in ones, and the colour books and libraries the user saved or loaded) or \"document\" (the document's swatches; the default), color?: colour (look this colour up, #rrggbbaa and #rgba refused; default: the selection's), fill?: true, stroke?: true (the selected objects' fills and strokes, their type's and those of the objects inside them: solid colours and gradient stops, once each, the first 100), count?: 1..50 (5), ids?: [id…] (default: the selection)} list the library's colour swatches closest to each colour, closest first, by CIEDE2000 difference (ΔE; under 1 hardly tells apart, about 2 to 3 close). Tint swatches, None and Registration aren't matched → {library, colors: [{values, hex, matches: [{name, values, hex, spot, deltaE}]}]}",
            has_doc,
            nearest::find
        ),
        cmd!(
            "addon.color.applyNearestSwatch",
            "Apply Nearest Swatch",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{library?: as addon.color.nearestSwatch (default: \"document\"), fill?: true, stroke?: true, ids?: [id…] (default: the selection)} give each solid colour and gradient stop of the selected objects' fills and strokes (their type's and those of the objects inside them; gradient meshes too) the colour of its nearest swatch in the library, as addon.color.nearestSwatch finds it, as one undo step: a global or spot swatch's colour links to it. A library's swatches the colours take are added to the document's first, as swatch.library.add adds them (a spot colour stays spot); images and patterns keep their colours. On a path directly in a graph series, the graph also stores the new paint of the topmost visible fill and stroke (docs/mcp.md) → {changed: paints changed, swatches: [the document swatches the colours took], added: [swatches added to the document]}",
            has_selection,
            nearest::apply
        ),
        cmd!(
            "addon.color.fillToStroke",
            "Apply Fill to Stroke",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{ids?|id?, skipNone?: bool} each target's top stroke takes its own top fill, as it is (a colour in its model, a global or spot colour linked at its tint, a gradient or pattern with its placement, None); a stroke given paint at width 0 gets 1 pt, as Swap Fill and Stroke does. Type: each run's stroke takes its own fill, and while the Type tool has characters selected (no ids), only those. A graph's labels keep the paint when the graph is drawn again. On a path directly in a graph series, the graph also stores the paint and weight of the topmost visible stroke when the command wrote that row (exceptions: docs/mcp.md). skipNone: leave alone a target whose fill is None. A target whose stroke already has that paint is left as it is, and when nothing changes no undo step is recorded. The default stroke takes the default fill the same way → {objects}",
            has_doc,
            |s, p| copy_paint(s, p, true)
        ),
        cmd!(
            "addon.color.strokeToFill",
            "Apply Stroke to Fill",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{ids?|id?, skipNone?: bool} each target's top fill takes its own top stroke's paint, as addon.color.fillToStroke does the other way; the stroke itself is unchanged. The default fill takes the default stroke → {objects}",
            has_doc,
            |s, p| copy_paint(s, p, false)
        ),
    ]
    .into_iter()
    .chain(legend::specs())
    .collect()
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.color.select";
    let kind = text_param(p, "kind", C)?.ok_or_else(|| bad(C, "missing `kind`"))?;
    if !SELECT_KINDS.contains(&kind) {
        return Err(bad(C, "`kind` must be spotColors, colorEffects, rgbImages, cmykImages or grayImages"));
    }
    let st = s.doc()?;
    let mut ids: Vec<NodeId> = vec![];
    walk_selectable(&st.doc, st.isolation, |n| {
        if is_of_kind(&st.doc, n, kind) {
            ids.push(n.id);
            // An object taken whole: what is inside it isn't taken again.
            return false;
        }
        true
    });
    s.select(|_, sel| sel.set(ids))?;
    Ok(json!({ "count": s.doc()?.selection.objects.len() }))
}

/// Is `n` an object of Select by Color Kind's `kind`? Containers count for colour effects only:
/// for the other kinds, the objects inside them are looked at one by one.
fn is_of_kind(d: &Document, n: &Node, kind: &str) -> bool {
    match kind {
        "colorEffects" => effects::has_adjustment(n),
        "spotColors" if !n.is_container() => {
            let mut spot = false;
            vectorcraft_doc::swatches::node_colors(n, &mut |_, swatch| {
                spot |= swatch.is_some_and(|name| d.swatch(name).is_some_and(|w| w.spot));
            });
            spot
        }
        _ => {
            let NodeKind::Image(img) = &n.kind else { return false };
            let Some(blob) = d.images.get(&img.key) else { return false };
            let model = if blob.is_cmyk() {
                "cmykImages"
            } else if is_gray(&blob.bytes) {
                "grayImages"
            } else {
                "rgbImages"
            };
            model == kind
        }
    }
}

/// Are the encoded image `bytes` grayscale or 1-bit? Reads only the header.
fn is_gray(bytes: &[u8]) -> bool {
    use image::ImageDecoder as _;
    let decoder = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok().and_then(|r| r.into_decoder().ok());
    decoder.is_some_and(|d| matches!(d.color_type(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16))
}

fn filter(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.color.filter";
    let id = text_param(p, "effect", C)?.ok_or_else(|| bad(C, "missing `effect`"))?;
    if !effects::is_adjustment(id) {
        return Err(bad(C, format!("`{id}` is not a colour adjustment")));
    }
    let params = match p.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err(bad(C, "`params` must be an object")),
    };
    let fx = effects::new_effect(id, &params).ok_or_else(|| bad(C, format!("unknown effect `{id}`")))?;
    let ids = checked_ids_or_selection(s, p, C)?;
    // Settings that change nothing still make the pass, which then leaves every colour as it was.
    let Some(map) = effects::color_map(&[fx]) else { return Ok(json!({ "changed": 0 })) };
    let mut given = p.clone();
    if let Some(o) = given.as_object_mut() {
        o.insert("ids".into(), json!(ids.iter().map(|i| i.0).collect::<Vec<_>>()));
    }
    let label = effects::effect_info(id).map_or("Apply as Filter", |e| e.label.trim_end_matches('…'));
    let kinds = effects::addons::duotone::swatch_kinds(&s.doc()?.doc);
    let swatches = |name: &str| kinds.get(name).copied();
    recolor_links(s, &given, C, label, &|c| map.apply(c), &|c, link, tint| map.apply_link(c, link, tint, &swatches))
}

/// Apply Fill to Stroke (`to_stroke`) or Apply Stroke to Fill: each target copies one of its own
/// paints onto the other, so a mixed selection keeps each object's colour (#5530). Aimed and
/// undone like Swap Fill and Stroke; characters selected with the Type tool are the only ones that
/// change, as for `paint.setFill`. A graph's series marks and labels give the graph the paint in
/// the same undo step, as `paint.setFill` does, so drawing the graph again keeps it. When nothing
/// changes, the document and its undo history stay as they are.
fn copy_paint(s: &mut Session, p: &Value, to_stroke: bool) -> Result<Value> {
    let (cmd, label) =
        if to_stroke { ("addon.color.fillToStroke", "Apply Fill to Stroke") } else { ("addon.color.strokeToFill", "Apply Stroke to Fill") };
    let skip_none = switch_or(p, "skipNone", false, cmd)?;
    // One text style: its stroke takes its fill, or the other way.
    let copy_run = |st: &mut CharStyle| {
        let from = if to_stroke { st.fill.clone() } else { st.stroke.clone() };
        if (skip_none && from.is_none()) || from == if to_stroke { st.stroke.clone() } else { st.fill.clone() } {
            return;
        }
        if to_stroke {
            set_run_stroke(st, from);
        } else {
            st.fill = from;
        }
    };
    if p.get("ids").is_none()
        && p.get("id").is_none()
        && let Some((id, a, b)) = super::super::textedit::editing_range(s)?
        && a != b
    {
        // After Select All in threaded text, every frame's text takes it.
        let story = super::super::textedit::editing_story(s);
        let ranges: Vec<(NodeId, usize, usize)> =
            if story.is_empty() { vec![(id, a, b)] } else { story.iter().map(|f| (*f, 0, usize::MAX)).collect() };
        let ids: Vec<NodeId> = ranges.iter().map(|(f, _, _)| *f).collect();
        let before = s.doc()?.doc.clone();
        let n = super::super::overprint::edit_counted(s, label, |d| {
            for &(id, a, b) in &ranges {
                let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
                vectorcraft_text::edit::style_range(&mut t.runs, a, b, copy_run);
                super::super::typecmd::refresh_bounds(t);
            }
            Ok(super::super::appearance::count_changed(d, &before, &ids))
        })?;
        return Ok(json!({"objects": n}));
    }
    // A graph's series group stands for its marks, as for `paint.setFill`.
    let ids = super::super::graph::paint_targets(&s.doc()?.doc, &paint_targets(s, p)?);
    let mut objects = 0;
    if !ids.is_empty() {
        let before = s.doc()?.doc.clone();
        super::super::overprint::edit_counted(s, label, |d| {
            // Each object with the row the paint went to, for the paint a graph stores for its series.
            let mut written = Vec::new();
            for id in &ids {
                let Some(node) = d.node_mut(*id) else { continue };
                if let NodeKind::Text(t) = &mut node.kind {
                    for r in &mut t.runs {
                        copy_run(&mut r.style);
                    }
                    super::super::typecmd::refresh_bounds(t);
                    continue;
                }
                let ap = &mut node.appearance;
                let (from, to) = if to_stroke { (ap.fill_paint(), ap.stroke_paint()) } else { (ap.stroke_paint(), ap.fill_paint()) };
                // A missing item reads as None: copying None onto it would add an item that
                // changes nothing.
                if skip_none && from.is_none() {
                    continue;
                }
                if from != to {
                    if to_stroke {
                        restore_unpainted_stroke(ap, None, &from);
                        ap.set_stroke(from);
                    } else {
                        ap.set_fill(from);
                    }
                }
                // A row that already had the paint counts as written with it, as for `paint.setFill`.
                written.extend(ap.items.iter().rposition(|it| it.is_fill() != to_stroke).map(|at| (*id, at)));
            }
            objects = super::super::appearance::count_changed(d, &before, &ids);
            // The graph keeps the paint its series marks and labels now have.
            let series = super::super::appearance::capture_series_paint_writes(&before, d, &ids, &written);
            let labels = super::super::graph::capture_label_styles(d, &ids, &copy_run);
            Ok(objects + usize::from(series || labels))
        })?;
    }
    // The defaults for new art change the same way, as Swap Fill and Stroke changes them.
    let from = if to_stroke { s.paint.fill.clone() } else { s.paint.stroke.clone() };
    if !(skip_none && from.is_none()) {
        if to_stroke {
            if s.paint.stroke.is_none() && !from.is_none() && s.paint.stroke_width == 0.0 {
                s.paint.stroke_width = 1.0;
            }
            s.paint.stroke = from;
        } else {
            s.paint.fill = from;
        }
    }
    Ok(json!({"objects": objects}))
}
