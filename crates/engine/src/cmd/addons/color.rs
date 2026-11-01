//! Color Effects (`addon.color.*`), a built-in add-on: its live effects (Exposure, Desaturate,
//! Duotone and Invert Colors, in Effect › Color Effects) are the effects crate's; here, Apply as
//! Filter writes any colour adjustment into the colours themselves, Select by Color Kind selects the
//! objects that use spot colours, carry a live colour adjustment, or are images of a colour model,
//! and Print Ink Fixes ([`print_fixes`]) prepares a CMYK document's inks for print. Its panel and
//! its submenu, Object › Plug-ins › Color Effects, are the UI's.

mod print_fixes;
#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_render::effects;

use super::super::colorcmds::recolor_links;
use super::super::select::walk_selectable;
use super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, has_selection, text_param};
use crate::{Result, Session};

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
            "{effect: a colour adjustment's effect id (adjust.brightnessContrast, adjust.curves, adjust.hueSaturation, adjust.levels, adjust.shiftToColor, adjust.temperatureTint, addon.color.exposure, addon.color.desaturate, addon.color.duotone or addon.color.invert), params?: {…} (as effect.apply; missing keys take the dialog defaults), fill?: true, stroke?: true, includeImages?: true, includePatterns?: true, ids?: [id…] (default: the selection)} recolour the objects and everything inside them as the live effect would draw them, but in their colours, as edit.colors.invert does (fills, strokes, type, gradient meshes, embedded images and pattern tiles), as one undo step; their effects stay. Duotone links a colour to its one tone's swatch as the live effect does → {changed}",
            has_selection,
            filter
        ),
        cmd!(
            "addon.color.printFixes",
            "Print Ink Fixes",
            ["Object", "Plug-ins", "Color Effects"],
            None,
            "{scope?: \"selection\" (the selection, or `ids`; by default, the document while nothing is selected) | \"document\" (every layer), inkLimit?: total ink % 1..400 (0 or left out: off), excludePureBlack?: true (black-only and grey colours keep their ink), removeWhiteOverprint?: false (white fills and strokes stop overprinting), overprintBlack?: black % 1..100 (black-only and grey fills and strokes this dark or darker overprint, as edit.colors.overprintBlack; 0 or left out: off), textOnly?: false (Overprint Black on type only), fill?: true, stroke?: true, ids?: [id…]} prepare a CMYK document's inks for print as one undo step: CMYK colours (solid, gradient stops, gradient meshes) with more total ink than inkLimit are scaled down to it (colours of spot swatches and Registration stay), white overprint is removed and black overprint set. Refused in RGB documents (File › Document Color Mode › CMYK Color first) → {changed, inkLimited: paints and meshes scaled, whiteOverprintRemoved: objects, blackOverprinted: objects}",
            has_doc,
            print_fixes::run
        ),
    ]
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
    recolor_links(s, &given, label, &|c| map.apply(c), &|c, link, tint| map.apply_link(c, link, tint, &swatches))
}
