//! The Swatches panel: new, delete and duplicate swatches, colour groups and sorting.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint, Swatch, SwatchGroup, tint_percent};
use vectorcraft_doc::pattern::pattern_paint;
use vectorcraft_doc::swatches::{LinkFn, color_name, node_colors};
use vectorcraft_doc::{Document, NodeId};

use super::paint::paint_from;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "swatch.new",
            "New Swatch",
            ["Window", "Swatches"],
            None,
            "{name?, color? | colors?: [colour] (one swatch each, in one undo step) | swatch? | gradient? | pattern?: name (default: the current fill), tint?: 0..100 (% of the global or spot `swatch`, or of the one the current fill links to), mode?: \"gray\"|\"rgb\"|\"hsb\"|\"lab\"|\"cmyk\"|\"web\" (convert the colour; lab: CIE L*a*b*, how spot colours are usually defined), global?, spot? (a spot colour, always global), group?: colour group name (solid colours only; default: ungrouped), none?: true (the None paint), paint?: paint.setFill params, keepModel?: bool (as paint.setFill)} save a colour, gradient or pattern as a swatch. A tint of a global or spot colour (below 100%, without mode, global or spot) becomes a tint swatch, \"Name 40%\", linked to its base: it follows edits to the base and applies as that tint of it. Names are unique (\"Sky 2\"); a colour's default name is its values (\"C=10 M=20 Y=30 K=0\", \"R=255 G=128 B=0\", \"Gray K=40\", \"L=55 a=60 b=40\") → {name, names: [every new swatch]}",
            has_doc,
            swatch_new
        ),
        cmd!(
            "swatch.delete",
            "Delete Swatch",
            ["Window", "Swatches"],
            None,
            "{name? | names?: [swatch or colour group names] (a group goes with its swatches), unlink?: true (art using a deleted global swatch keeps its colour, unlinked; false keeps the stale link)} delete in one undo step → {deleted, unlinked: paints unlinked}",
            has_doc,
            swatch_delete
        ),
        cmd!(
            "swatch.newGroup",
            "New Color Group",
            ["Window", "Swatches"],
            None,
            "{name?, swatches?: [names] (solid colours move into the group; gradients, patterns and None stay), colors?: [colour] (added as new swatches), keepModel?: bool (as paint.setFill), fromArtwork?: false (add the unique colours of the selected art: fills, strokes, text, gradient stops and mesh points; a colour linked to a global swatch moves that swatch into the group), toGlobal?: true (with fromArtwork: the new swatches are global and the selected art's matching unlinked colours link to them), includeTints?: false (with fromArtwork: tints of global swatches also get swatches of their own)} → {name, swatches: [names in the group], linked: paints linked}",
            has_doc,
            swatch_new_group
        ),
        cmd!("swatch.duplicate", "Duplicate Swatch", ["Window", "Swatches"], None, "{name}", has_doc, swatch_duplicate),
        cmd!("swatch.sortByName", "Sort by Name", ["Window", "Swatches"], None, "{}", has_doc, swatch_sort),
        cmd!(
            "swatch.edit",
            "Swatch Options",
            ["Window", "Swatches"],
            None,
            "{name, newName?, color? (e.g. {l, a, b} for a Lab colour), keepModel?: bool (as paint.setFill; the swatch's own color and a color given with mode keep their model), mode?: \"gray\"|\"rgb\"|\"hsb\"|\"lab\"|\"cmyk\"|\"web\" (convert the colour), global?, spot? (spot colours are always global), paint?: paint.setFill params ({color}, {gradient}, {swatch} or {pattern}) replacing the swatch's colour; gradient or pattern with one of the same kind} edit a swatch in any colour group, as one undo step. Fills, strokes, text, gradient stops and tint swatches linked to a global swatch take its new colour (at their own tint) and name; turning Global off unlinks them (they keep their colour). Colour, mode and spot apply to solid colours only; a tint swatch only takes a new name (edit its base) → {name, relinked: paints changed}",
            has_doc,
            swatch_edit
        ),
        cmd!(
            query "swatch.list",
            "Swatches",
            [],
            None,
            "{group?: name (only that colour group's swatches)} → {swatches: [{name, kind: \"none\"|\"color\"|\"gradient\"|\"pattern\", group, global, spot, color?, hex?, tintOf?: base swatch of a tint swatch, tint?: its %, gradient?, pattern?}] (the built-in [Registration] after None: it prints on every plate and can't be edited, moved or deleted), groups: [{name, swatches: [names]}]}",
            has_doc,
            swatch_list
        ),
        cmd!(
            "swatch.move",
            "Move Swatch",
            ["Window", "Swatches"],
            None,
            "{name? | names?: [swatch names, in order; or colour group names to reorder the groups], to?: index in the destination (they go before the swatch or group now there; default: the end), group?: destination colour group (default: the ungrouped swatches; groups hold solid colours only)} reorder swatches or move them into or out of colour groups, as one undo step → {moved}",
            has_doc,
            swatch_move
        ),
        cmd!(
            "swatch.addUsedColors",
            "Add Used Colors",
            ["Window", "Swatches"],
            None,
            "{selection?: false (only the selected art's colours; default: all the art's), global?: false (the new swatches are global and the matching unlinked colours in that art link to them)} add a swatch for each colour used (fills, strokes, text, gradient stops, mesh points) that no solid swatch has yet, as one undo step → {added: [names], linked}",
            has_doc,
            swatch_add_used
        ),
        cmd!(
            query "swatch.unused",
            "Select All Unused",
            ["Window", "Swatches"],
            None,
            "{} → {names: [the swatches nothing uses, in panel order]}: a colour is used when a paint links to it or an unlinked paint, gradient stop or mesh point has its colour; a gradient when a gradient links to it or has its stops; a pattern when it fills or strokes anything (art, symbols, pattern tiles and graphic styles count). None is never listed",
            has_doc,
            swatch_unused
        ),
        cmd!(
            "swatch.merge",
            "Merge Swatches",
            ["Window", "Swatches"],
            None,
            "{names: [two or more solid-colour swatches], name?: the merged swatch's name (one swatch kept)} keep the first (its name and colour) and delete the others; paints linked to them take its colour and link to it (or unlink when it isn't global), as one undo step → {name, merged: [deleted names], relinked}",
            has_doc,
            swatch_merge
        ),
        cmd!(
            "swatch.ungroup",
            "Ungroup Color Group",
            ["Window", "Swatches"],
            None,
            "{name: colour group} move its swatches to the end of the ungrouped swatches and remove the group, as one undo step → {swatches: [names]}",
            has_doc,
            swatch_ungroup
        ),
        cmd!(
            "swatch.sortByKind",
            "Sort by Kind",
            ["Window", "Swatches"],
            None,
            "{} order the swatches of each list by kind (None first, then process colours, spot colours, gradients, patterns; colour groups stay after them), keeping their order within a kind, as one undo step",
            has_doc,
            swatch_sort_by_kind
        ),
        cmd!(
            "swatch.spotOptions",
            "Spot Colors",
            ["Window", "Swatches"],
            None,
            "{useLab?: bool (omitted: unchanged)} the document's Spot Colors options: spot colours defined in Lab show, print and export from their Lab values (true, the default: PDF Separation spaces get a Lab alternate) or from their working-CMYK equivalents (false: a DeviceCMYK alternate, matching older files). Art, gradient stops and tint swatches linked to Lab spot swatches take the chosen colour at their tint, as one undo step → {useLab, relinked: paints changed}",
            has_doc,
            swatch_spot_options
        ),
        cmd!(
            "swatch.editGroup",
            "Edit Color Group",
            ["Window", "Swatches"],
            None,
            "{group: colour group, colors: [colour] (hex, key, {c,m,y,k} or {l,a,b}; the group's swatches take them in order, extra colours become new swatches, swatches past the end are removed; built-in swatches can't change; a color the swatch already has keeps its model), keepModel?: bool (as paint.setFill), rename?: new group name} rewrite a colour group as one undo step; art linked to its global swatches takes their new colours (and keeps its colour, unlinked, when its swatch goes) → {name, swatches: [names], relinked}",
            has_doc,
            swatch_edit_group
        ),
    ]
}

fn swatch_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.new";
    let mode = str_param(p, "mode");
    let paints = match (p.get("colors").and_then(Value::as_array), str_param(p, "pattern")) {
        (Some(cs), _) => {
            let stored = |c: Color| if mode.is_some() { c } else { super::paint::new_color(s, p, c) };
            let cs: Vec<Paint> = cs
                .iter()
                .map(|c| color_value(c).map(|c| Paint::solid(stored(c))))
                .collect::<Option<_>>()
                .ok_or_else(|| bad(C, "bad colour in `colors`"))?;
            if cs.is_empty() {
                return Err(bad(C, "`colors` is empty"));
            }
            cs
        }
        (None, Some(name)) if s.doc()?.doc.pattern(name).is_some() => vec![pattern_paint(name)],
        (None, Some(name)) => return Err(bad(C, format!("no pattern `{name}`"))),
        (None, None) => {
            let paint = match mode {
                Some(_) => paint_from(s, &keeping_model(p))?,
                None => paint_from(s, p)?,
            }
            .unwrap_or_else(|| s.paint.fill.clone());
            // `tint` alone: that tint of the global colour the current fill links to.
            match (super::paint::tint_param(p, C)?, &paint) {
                (Some(t), Paint::Solid { swatch: Some(l), .. }) if p.get("swatch").is_none() => {
                    vec![s.doc()?.doc.tint_paint(l, t).ok_or_else(|| bad(C, format!("`{l}` isn't a global or spot colour")))?]
                }
                (Some(_), _) if p.get("swatch").is_none() => return Err(bad(C, "`tint` needs a global or spot colour")),
                _ => vec![paint],
            }
        }
    };
    let spot = bool_or(p, "spot", false);
    let global = spot || bool_or(p, "global", false);
    let group = str_param(p, "group");
    let paints = paints
        .into_iter()
        .map(|paint| match paint {
            // A tint stays linked to its base: a tint swatch.
            Paint::Solid { swatch: Some(_), tint, .. } if tint < 1.0 && mode.is_none() && !global => Ok(paint),
            // The swatch holds the colour itself, not a link to the swatch it may have come from.
            Paint::Solid { color, .. } => Ok(Paint::solid(match mode {
                Some(m) => convert_to_mode(color, m, C)?,
                None => color,
            })),
            Paint::None => Err(bad(C, "a swatch needs a colour, gradient or pattern")),
            _ if spot || mode.is_some() || group.is_some() => Err(bad(C, "spot, mode and group apply to solid colours only")),
            other => Ok(other),
        })
        .collect::<Result<Vec<_>>>()?;
    // A name applies to a single new swatch.
    let requested = name_param(p, "name").filter(|_| paints.len() == 1);
    let names = s.edit("New Swatch", |d, _| {
        let mut names = vec![];
        for paint in paints {
            let name = match &requested {
                Some(n) => d.free_swatch_name(n),
                None => d.new_swatch_name(&paint),
            };
            // A tint swatch's kind is its base's.
            let tint = matches!(paint, Paint::Solid { swatch: Some(_), .. });
            let swatch = Swatch { name: name.clone(), paint, global: global && !tint, spot: spot && !tint };
            match group {
                Some(g) => {
                    d.swatch_groups.iter_mut().find(|x| x.name == g).ok_or_else(|| bad(C, format!("no colour group `{g}`")))?.swatches.push(swatch)
                }
                None => d.swatches.push(swatch),
            }
            names.push(name);
        }
        Ok(names)
    })?;
    Ok(json!({ "name": names[0], "names": names }))
}

/// The `names` list and `name` parameters, without repeats; at least one is required.
pub(super) fn names_param(p: &Value, cmd: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = vec![];
    for n in str_list(p, "names").into_iter().chain(str_param(p, "name").map(str::to_string)) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if names.is_empty() {
        return Err(bad(cmd, "give `name` or `names`"));
    }
    Ok(names)
}

fn swatch_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.delete";
    let names = names_param(p, C)?;
    let d = &s.doc()?.doc;
    // Global swatches going away (named ones and those in named groups): their links are dropped.
    let mut global: Vec<String> = vec![];
    for n in &names {
        let swatches: Vec<&Swatch> = match d.swatch_groups.iter().find(|g| g.name == *n) {
            Some(g) => g.swatches.iter().collect(),
            None => vec![d.swatch(n).ok_or_else(|| bad(C, format!("no swatch or colour group `{n}`")))?],
        };
        if swatches.iter().any(|w| w.is_reserved()) {
            return Err(bad(C, format!("`{n}` can't be deleted")));
        }
        global.extend(swatches.iter().filter(|w| w.global).map(|w| w.name.clone()));
    }
    if !bool_or(p, "unlink", true) {
        global.clear();
    }
    let mut unlink =
        |_: &mut Color, link: &mut Option<String>, _: &mut f32| link.as_ref().is_some_and(|l| global.contains(l)) && link.take().is_some();
    let unlinked = s.edit("Delete Swatch", |d, _| {
        d.swatch_groups.retain(|g| !names.contains(&g.name));
        for n in &names {
            d.remove_swatch(n);
        }
        Ok(d.map_solid_paints(&mut unlink))
    })?;
    map_default_paints(s, &mut unlink);
    Ok(json!({"deleted": names, "unlinked": unlinked}))
}

/// The colours of the art `ids` for a new colour group, in paint order: the swatches that linked
/// colours belong to (added to `swatches` once each) and the other colours (added to `colors` once
/// each). A tint (a linked colour that differs from its swatch's) brings its swatch, and with
/// `tints` its own colour too. Links to missing swatches count as unlinked.
fn artwork_colors(d: &Document, ids: &[NodeId], tints: bool, swatches: &mut Vec<String>, colors: &mut Vec<Color>) {
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        node_colors(n, &mut |c, link| {
            let sw = link.and_then(|l| d.swatch(l)).filter(|w| w.paint.color().is_some());
            if let Some(w) = sw
                && !swatches.contains(&w.name)
            {
                swatches.push(w.name.clone());
            }
            let own = sw.is_none_or(|w| tints && w.paint.color() != Some(*c));
            if own && !colors.contains(c) {
                colors.push(*c);
            }
        });
    }
}

/// Link the solid paints in the subtrees of `ids` whose colour is one of `links`' to that swatch,
/// unless they already link to an existing swatch. Returns the number of paints linked.
fn link_colors(d: &mut Document, ids: &[NodeId], links: &[(Color, String)]) -> usize {
    if links.is_empty() {
        return 0;
    }
    let live: Vec<String> = d.swatches_iter().map(|w| w.name.clone()).collect();
    d.map_solid_paints_in(ids, &mut |c, link, _| {
        if link.as_ref().is_some_and(|l| live.contains(l)) {
            return false;
        }
        match links.iter().find(|(lc, _)| lc == c) {
            Some((_, n)) => {
                *link = Some(n.clone());
                true
            }
            None => false,
        }
    })
}

fn swatch_new_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.newGroup";
    let mut names = str_list(p, "swatches");
    let colors: Vec<Color> = p
        .get("colors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(color_value).map(|c| super::paint::new_color(s, p, c)).collect())
        .unwrap_or_default();
    let from_art = bool_or(p, "fromArtwork", false);
    let to_global = from_art && bool_or(p, "toGlobal", true);
    let tints = bool_or(p, "includeTints", false);
    let requested = name_param(p, "name");
    let (name, members, linked) = s.edit("New Color Group", |d, sel| {
        // New swatches: (colour, global).
        let mut new: Vec<(Color, bool)> = colors.iter().map(|c| (*c, false)).collect();
        if from_art {
            if sel.objects.is_empty() {
                return Err(bad(C, "select artwork to make a group of its colours"));
            }
            let mut art = vec![];
            artwork_colors(d, &sel.objects, tints, &mut names, &mut art);
            new.extend(art.into_iter().map(|c| (c, to_global)));
        }
        let name = d.free_swatch_name(requested.as_deref().unwrap_or("Color Group"));
        let mut group = SwatchGroup { name: name.clone(), swatches: vec![] };
        // Colour groups hold solid colours only.
        for n in &names {
            if d.swatch(n).is_some_and(|sw| matches!(sw.paint, Paint::Solid { .. }))
                && let Some(sw) = d.remove_swatch(n)
            {
                group.swatches.push(sw);
            }
        }
        // The global swatches made from the art's colours, which its unlinked paints link to.
        let mut links: Vec<(Color, String)> = vec![];
        for (c, global) in new {
            let nm = unique_name(&color_name(c), |n| n == name || d.swatch_name_taken(n) || group.swatches.iter().any(|sw| sw.name == n));
            if global {
                links.push((c, nm.clone()));
            }
            group.swatches.push(Swatch { name: nm, paint: Paint::solid(c), global, spot: false });
        }
        let members: Vec<String> = group.swatches.iter().map(|w| w.name.clone()).collect();
        d.swatch_groups.push(group);
        let linked = link_colors(d, &sel.objects, &links);
        Ok((name, members, linked))
    })?;
    Ok(json!({"name": name, "swatches": members, "linked": linked}))
}

/// The name of the item that items moving to index `to` of `list` go before: the first one at or
/// after `to` that isn't moving (`None`: the end).
fn anchor<T>(list: &[T], to: Option<usize>, moving: &[String], name: impl Fn(&T) -> &str) -> Option<String> {
    list.iter().skip(to?).map(&name).find(|n| !moving.iter().any(|m| m == n)).map(str::to_string)
}

/// Insert `items` into `list` before the item named `anchor` (at the end without one).
fn insert_before<T>(list: &mut Vec<T>, items: Vec<T>, anchor: Option<&str>, name: impl Fn(&T) -> &str) {
    let at = anchor.and_then(|a| list.iter().position(|x| name(x) == a)).unwrap_or(list.len());
    list.splice(at..at, items);
}

fn swatch_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.move";
    let names = names_param(p, C)?;
    let to = p.get("to").and_then(Value::as_u64).map(|v| v as usize);
    let group = str_param(p, "group").map(str::to_string);
    let moved = names.len();
    s.edit("Move Swatch", |d, _| {
        let is_group = |d: &Document, n: &str| d.swatch_groups.iter().any(|g| g.name == n);
        if names.iter().all(|n| is_group(d, n)) {
            if group.is_some() {
                return Err(bad(C, "colour groups can't go into colour groups"));
            }
            let at = anchor(&d.swatch_groups, to, &names, |g| &g.name);
            let mut items = vec![];
            for n in &names {
                if let Some(i) = d.swatch_groups.iter().position(|g| g.name == *n) {
                    items.push(d.swatch_groups.remove(i));
                }
            }
            insert_before(&mut d.swatch_groups, items, at.as_deref(), |g| &g.name);
            return Ok(());
        }
        for n in &names {
            let sw = d.swatch(n).ok_or_else(|| bad(C, format!("no swatch `{n}` (swatches and colour groups move separately)")))?;
            if sw.is_reserved() {
                return Err(bad(C, format!("`{n}` can't be moved")));
            }
            if group.is_some() && sw.paint.color().is_none() {
                return Err(bad(C, format!("`{n}`: colour groups hold solid colours only")));
            }
        }
        let dest = match &group {
            Some(g) => Some(d.swatch_groups.iter().position(|x| x.name == *g).ok_or_else(|| bad(C, format!("no colour group `{g}`")))?),
            None => None,
        };
        fn list(d: &mut Document, dest: Option<usize>) -> &mut Vec<Swatch> {
            match dest {
                Some(i) => &mut d.swatch_groups[i].swatches,
                None => &mut d.swatches,
            }
        }
        let at = anchor(list(d, dest), to, &names, |w| &w.name);
        let items: Vec<Swatch> = names.iter().filter_map(|n| d.remove_swatch(n)).collect();
        insert_before(list(d, dest), items, at.as_deref(), |w| &w.name);
        Ok(())
    })?;
    Ok(json!({ "moved": moved }))
}

fn swatch_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("swatch.duplicate", "missing name"))?.to_string();
    let new = s.edit("Duplicate Swatch", |d, _| {
        let src = d.swatch(&name).cloned().ok_or_else(|| EngineError::Other(format!("no swatch `{name}`")))?;
        if src.is_reserved() {
            return Err(bad("swatch.duplicate", format!("`{name}` can't be duplicated")));
        }
        let nm = d.free_swatch_name(&format!("{name} copy"));
        let copy = Swatch { name: nm.clone(), ..src };
        if let Some(pos) = d.swatches.iter().position(|sw| sw.name == name) {
            d.swatches.insert(pos + 1, copy);
        } else if let Some(g) = d.swatch_groups.iter_mut().find(|g| g.swatches.iter().any(|sw| sw.name == name)) {
            let pos = g.swatches.iter().position(|sw| sw.name == name).unwrap_or(0);
            g.swatches.insert(pos + 1, copy);
        }
        Ok(nm)
    })?;
    Ok(json!({"name": new}))
}

/// A list-of-strings parameter (missing or non-string entries are skipped).
pub(super) fn str_list(p: &Value, key: &str) -> Vec<String> {
    p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// A trimmed, non-empty name parameter.
fn name_param(p: &Value, key: &str) -> Option<String> {
    str_param(p, key).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

/// `c` converted to a Swatch Options colour mode through the colour settings (HSB is an RGB view;
/// Web snaps each channel to a multiple of 0x33).
fn convert_to_mode(c: Color, mode: &str, cmd: &str) -> Result<Color> {
    use vectorcraft_color::cms::Model;
    let cms = vectorcraft_color::cms::active();
    let to = |m: Model| cms.convert(&c, m, cms.settings().intent);
    Ok(match mode.to_ascii_lowercase().as_str() {
        "gray" | "grayscale" => to(Model::Gray),
        "rgb" | "hsb" => to(Model::Rgb),
        "cmyk" => to(Model::Cmyk),
        "lab" => to(Model::Lab),
        "web" => {
            let snap = |v: f32| (v.clamp(0.0, 1.0) * 5.0).round() / 5.0;
            let [r, g, b] = to(Model::Rgb).to_rgb_uncalibrated();
            Color::rgb(snap(r), snap(g), snap(b))
        }
        other => return Err(bad(cmd, format!("unknown mode `{other}` (gray, rgb, hsb, lab, cmyk or web)"))),
    })
}

/// Paint params `p` with `keepModel: true`, for a color given with a `mode`: [`convert_to_mode`]
/// converts it, and the document's color mode does not convert it first.
fn keeping_model(p: &Value) -> Value {
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.insert("keepModel".into(), Value::Bool(true));
    }
    q
}

/// How a swatch edit reaches the solid paints linked to swatch `from`.
struct Relink {
    from: String,
    /// The swatch's name after the edit.
    to: String,
    /// The colour paints linked to the global swatch now show ([`Document::linked_color`]); they
    /// take it at their tint.
    color: Option<Color>,
    /// false: drop the links (the swatch stopped being global); paints keep their colour.
    keep: bool,
}

impl Relink {
    /// Apply to one linked colour; true when it changed.
    fn apply(&self, c: &mut Color, link: &mut Option<String>, tint: &mut f32) -> bool {
        if link.as_deref() != Some(self.from.as_str()) {
            return false;
        }
        if !self.keep {
            *link = None;
            return true;
        }
        let renamed = self.to != self.from;
        if renamed {
            *link = Some(self.to.clone());
        }
        let new = self.color.map(|n| n.tinted(*tint));
        let recolored = new.is_some_and(|n| n != *c);
        if let Some(n) = new {
            *c = n;
        }
        renamed || recolored
    }

    fn defaults(&self, s: &mut Session) {
        map_default_paints(s, &mut |c, l, t| self.apply(c, l, t));
    }
}

/// Apply a swatch-link rewrite to the default fill and stroke for new art, except during a live
/// preview (Cancel rolls the document back, not the session).
pub(super) fn map_default_paints(s: &mut Session, f: &mut LinkFn) {
    if s.in_interaction() {
        return;
    }
    for p in [&mut s.paint.fill, &mut s.paint.stroke] {
        p.map_links(f);
    }
}

fn swatch_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.edit";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let d = &s.doc()?.doc;
    let sw = d.swatch(&name).cloned().ok_or_else(|| bad(C, format!("no swatch `{name}`")))?;
    if sw.is_reserved() {
        return Err(bad(C, format!("`{name}` can't be edited")));
    }
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    let solid = sw.paint.color();
    if solid.is_none() && (given("color") || given("mode") || bool_or(p, "spot", false)) {
        return Err(bad(C, "colour, mode and spot apply to solid-colour swatches only"));
    }
    if let Some((base, _)) = sw.tint_of()
        && ["color", "mode", "paint", "global", "spot"].iter().any(|k| given(k))
    {
        return Err(bad(C, format!("`{name}` is a tint of `{base}`: edit `{base}` (a tint swatch only takes a new name)")));
    }
    // A replacement paint of the swatch's kind (a dropped colour or edited gradient).
    let paint = match p.get("paint").filter(|v| !v.is_null()) {
        Some(v) => match (if given("mode") { paint_from(s, &keeping_model(v))? } else { paint_from(s, v)? }, &sw.paint) {
            (Some(Paint::Solid { color, .. }), Paint::Solid { .. }) => Some(Paint::solid(color)),
            (Some(Paint::Gradient(g)), Paint::Gradient(_)) => {
                Some(Paint::Gradient(Box::new(vectorcraft_color::GradientPaint { geom: None, swatch: None, ..*g })))
            }
            (Some(new @ Paint::Pattern { .. }), Paint::Pattern { .. }) => Some(new),
            _ => return Err(bad(C, format!("`{name}` takes a paint of its own kind (colour, gradient or pattern)"))),
        },
        None => None,
    };
    let mut color = match p.get("color").filter(|v| !v.is_null()) {
        Some(v) => {
            let c = color_value(v).ok_or_else(|| bad(C, format!("bad color {v}")))?;
            // The swatch's own color, and a color given with a mode, keep their model.
            Some(if given("mode") || solid == Some(c) { c } else { super::paint::new_color(s, p, c) })
        }
        None => paint.as_ref().and_then(Paint::color).or(solid),
    };
    if let (Some(c), Some(m)) = (color, str_param(p, "mode")) {
        color = Some(convert_to_mode(c, m, C)?);
    }
    let spot = bool_or(p, "spot", sw.spot);
    let global = spot || bool_or(p, "global", sw.global);
    let to = name_param(p, "newName").filter(|n| *n != name).map_or_else(|| name.clone(), |n| d.free_swatch_name(&n));
    let relink = Relink { from: name.clone(), to: to.clone(), color: color.filter(|_| global).map(|c| d.linked_color(c, spot)), keep: global };
    let relinked = s.edit("Swatch Options", |d, _| {
        let w = d.swatch_mut(&name).ok_or_else(|| bad(C, format!("no swatch `{name}`")))?;
        w.name = to.clone();
        w.spot = spot;
        w.global = global;
        match (color, &mut w.paint) {
            (Some(c), Paint::Solid { color: wc, .. }) => *wc = c,
            (_, wp) => {
                if let Some(new) = paint {
                    *wp = new;
                }
            }
        }
        Ok(d.map_solid_paints(&mut |c, l, t| relink.apply(c, l, t)))
    })?;
    relink.defaults(s);
    Ok(json!({"name": to, "relinked": relinked}))
}

pub(super) fn swatch_json(sw: &Swatch, group: Option<&str>) -> Value {
    let mut v = json!({"name": sw.name, "group": group, "global": sw.global, "spot": sw.spot});
    match &sw.paint {
        Paint::None => v["kind"] = json!("none"),
        Paint::Solid { color, swatch, tint } => {
            v["kind"] = json!("color");
            v["color"] = json!(color);
            v["hex"] = json!(color.to_hex());
            if let Some(base) = swatch {
                v["tintOf"] = json!(base);
                v["tint"] = json!(tint_percent(*tint));
            }
        }
        Paint::Gradient(g) => {
            v["kind"] = json!("gradient");
            v["gradient"] = json!(g.gradient.kind.label().to_lowercase());
        }
        Paint::Pattern { pattern, .. } => {
            v["kind"] = json!("pattern");
            v["pattern"] = json!(pattern);
        }
    }
    v
}

fn swatch_list(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let only = str_param(p, "group");
    let groups: Vec<&SwatchGroup> = match only {
        Some(g) => vec![d.swatch_groups.iter().find(|x| x.name == g).ok_or_else(|| bad("swatch.list", format!("no colour group `{g}`")))?],
        None => d.swatch_groups.iter().collect(),
    };
    // The built-in Registration swatch lists after None, as the panel shows it.
    let none = d.swatches.iter().take_while(|w| w.paint.is_none()).count();
    let (specials, rest) = d.swatches.split_at(none);
    let all = specials.iter().chain([vectorcraft_color::swatch::registration()]).chain(rest);
    let ungrouped = all.filter(|_| only.is_none()).map(|w| swatch_json(w, None));
    let grouped = groups.iter().flat_map(|g| g.swatches.iter().map(|w| swatch_json(w, Some(&g.name))));
    let swatches: Vec<Value> = ungrouped.chain(grouped).collect();
    let groups: Vec<Value> =
        groups.iter().map(|g| json!({"name": g.name, "swatches": g.swatches.iter().map(|w| &w.name).collect::<Vec<_>>()})).collect();
    Ok(json!({"swatches": swatches, "groups": groups}))
}

fn swatch_sort(s: &mut Session, _: &Value) -> Result<Value> {
    s.edit("Sort Swatches", |d, _| {
        // [None] and other bracketed specials stay first, like the reference app.
        let key = |sw: &Swatch| (!sw.name.starts_with('['), sw.name.to_lowercase());
        d.swatches.sort_by_key(key);
        for g in &mut d.swatch_groups {
            g.swatches.sort_by_key(key);
        }
        Ok(())
    })?;
    ok()
}

fn swatch_add_used(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.addUsedColors";
    let global = bool_or(p, "global", false);
    let only_selection = bool_or(p, "selection", false);
    let (added, linked) = s.edit("Add Used Colors", |d, sel| {
        let ids: Vec<NodeId> = if only_selection { sel.objects.clone() } else { d.layers.iter().map(|l| l.id).collect() };
        if only_selection && ids.is_empty() {
            return Err(bad(C, "select artwork to add its colours"));
        }
        let (mut linked_to, mut colors) = (vec![], vec![]);
        artwork_colors(d, &ids, false, &mut linked_to, &mut colors);
        // Colours a solid swatch already has are skipped.
        colors.retain(|c| !d.swatches_iter().any(|w| w.paint.color() == Some(*c)));
        let mut links = vec![];
        for c in colors {
            let name = d.free_swatch_name(&color_name(c));
            d.swatches.push(Swatch { name: name.clone(), paint: Paint::solid(c), global, spot: false });
            links.push((c, name));
        }
        let linked = if global { link_colors(d, &ids, &links) } else { 0 };
        Ok((links.into_iter().map(|(_, n)| n).collect::<Vec<_>>(), linked))
    })?;
    Ok(json!({"added": added, "linked": linked}))
}

fn swatch_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    // Links with their tints, so a tint swatch is used where its tint of its base is.
    let (mut links, mut colors, mut gradients, mut patterns, mut gradient_links) = (vec![], vec![], vec![], vec![], vec![]);
    let mut color = |c: &Color, link: Option<&String>, tint: f32| match link {
        Some(l) if d.swatch(l).is_some() => links.push((l.clone(), tint)),
        _ => colors.push(*c),
    };
    d.visit_paints(&mut |p| match p {
        Paint::Solid { color: c, swatch, tint } => color(c, swatch.as_ref(), *tint),
        Paint::Gradient(g) => {
            g.gradient.stops.iter().for_each(|s| color(&s.color, s.swatch.as_ref(), s.tint));
            gradient_links.extend(g.swatch.clone());
            gradients.push(g.gradient.clone());
        }
        Paint::Pattern { pattern, .. } => patterns.push(pattern.clone()),
        Paint::None => {}
    });
    let used = |w: &Swatch| {
        gradient_links.contains(&w.name)
            || links.iter().any(|(l, t)| match w.tint_of() {
                Some((base, tint)) => l == base && *t == tint,
                None => *l == w.name,
            })
            || match &w.paint {
                Paint::None => true,
                Paint::Solid { color, .. } => colors.contains(color),
                Paint::Gradient(g) => gradients.contains(&g.gradient),
                Paint::Pattern { pattern, .. } => patterns.contains(pattern),
            }
    };
    let names: Vec<&str> = d.swatches_iter().filter(|w| !used(w)).map(|w| w.name.as_str()).collect();
    Ok(json!({ "names": names }))
}

fn swatch_merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.merge";
    let names = names_param(p, C)?;
    let d = &s.doc()?.doc;
    for n in &names {
        let sw = d.swatch(n).ok_or_else(|| bad(C, format!("no swatch `{n}`")))?;
        if sw.is_reserved() {
            return Err(bad(C, format!("`{n}` can't be merged")));
        }
        if sw.paint.color().is_none() {
            return Err(bad(C, format!("`{n}` isn't a solid colour")));
        }
    }
    let (keep, merged) = names.split_first().ok_or_else(|| bad(C, "give two or more swatches"))?;
    if merged.is_empty() {
        return Err(bad(C, "give two or more swatches"));
    }
    let kept = d.swatch(keep).ok_or_else(|| bad(C, format!("no swatch `{keep}`")))?;
    let (color, link) = (d.linked_color(kept.paint.color().unwrap_or_default(), kept.spot), kept.global.then(|| keep.clone()));
    // Tints keep their tint (of the kept colour).
    let mut relink = |c: &mut Color, l: &mut Option<String>, t: &mut f32| {
        if !l.as_ref().is_some_and(|l| merged.contains(l)) {
            return false;
        }
        *c = color.tinted(*t);
        *l = link.clone();
        true
    };
    let relinked = s.edit("Merge Swatches", |d, _| {
        for n in merged {
            d.remove_swatch(n);
        }
        Ok(d.map_solid_paints(&mut relink))
    })?;
    map_default_paints(s, &mut relink);
    Ok(json!({"name": keep, "merged": merged, "relinked": relinked}))
}

fn swatch_ungroup(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.ungroup";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let moved = s.edit("Ungroup Color Group", |d, _| {
        let i = d.swatch_groups.iter().position(|g| g.name == name).ok_or_else(|| bad(C, format!("no colour group `{name}`")))?;
        let g = d.swatch_groups.remove(i);
        let names: Vec<String> = g.swatches.iter().map(|w| w.name.clone()).collect();
        d.swatches.extend(g.swatches);
        Ok(names)
    })?;
    Ok(json!({ "swatches": moved }))
}

fn swatch_sort_by_kind(s: &mut Session, _: &Value) -> Result<Value> {
    let rank = |w: &Swatch| match &w.paint {
        Paint::None => 0,
        Paint::Solid { .. } if !w.spot => 1,
        Paint::Solid { .. } => 2,
        Paint::Gradient(_) => 3,
        Paint::Pattern { .. } => 4,
    };
    s.edit("Sort Swatches", |d, _| {
        // A stable sort: swatches keep their order within a kind.
        d.swatches.sort_by_key(rank);
        for g in &mut d.swatch_groups {
            g.swatches.sort_by_key(rank);
        }
        Ok(())
    })?;
    ok()
}

fn swatch_spot_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.spotOptions";
    let was = s.doc()?.doc.spot_use_lab;
    let use_lab = match p.get("useLab").filter(|v| !v.is_null()) {
        Some(v) => v.as_bool().ok_or_else(|| bad(C, "`useLab` must be true or false"))?,
        None => was,
    };
    if use_lab == was {
        return Ok(json!({"useLab": use_lab, "relinked": 0}));
    }
    let (relinked, bases) = s.edit("Spot Colors", |d, _| {
        d.spot_use_lab = use_lab;
        // Each Lab spot swatch with the colour its linked paints now show.
        let bases: Vec<(String, Color)> = d
            .swatches_iter()
            .filter(|w| w.spot && matches!(w.paint, Paint::Solid { color: Color::Lab { .. }, swatch: None, .. }))
            .filter_map(|w| Some((w.name.clone(), d.global_color(&w.name)?)))
            .collect();
        let relinked = d.map_solid_paints(&mut follow_bases(&bases));
        Ok((relinked, bases))
    })?;
    map_default_paints(s, &mut follow_bases(&bases));
    Ok(json!({"useLab": use_lab, "relinked": relinked}))
}

/// Recolour the colours linked to the swatches of `bases` (name, colour) to that colour at their
/// tint; true when a colour changed.
fn follow_bases(bases: &[(String, Color)]) -> impl FnMut(&mut Color, &mut Option<String>, &mut f32) -> bool + '_ {
    |c, link, tint| {
        let Some((_, base)) = bases.iter().find(|(n, _)| link.as_ref() == Some(n)) else { return false };
        let new = base.tinted(*tint);
        std::mem::replace(c, new) != new
    }
}

/// A colour group rewrite: its name afterwards and the swatch links it changed.
pub(super) struct GroupEdit {
    name: String,
    swatches: Vec<String>,
    relinks: Vec<Relink>,
    relinked: usize,
}

impl GroupEdit {
    /// Apply the link changes to the default fill and stroke for new art.
    pub(super) fn defaults(&self, s: &mut Session) {
        self.relinks.iter().for_each(|r| r.defaults(s));
    }
}

/// Rewrite colour group `group` with `colors`: its swatches take them in order (keeping their
/// names, kinds and global or spot flags), extra colours are added as new swatches and swatches
/// past the end are removed. Paints linked to its global swatches follow (a removed swatch's are
/// unlinked); `rename` renames the group. A built-in swatch (None, Registration) it would change or
/// remove is refused.
pub(super) fn rewrite_group(d: &mut Document, group: &str, colors: &[Color], rename: Option<&str>, cmd: &str) -> Result<GroupEdit> {
    let gi = d.swatch_groups.iter().position(|g| g.name == group).ok_or_else(|| bad(cmd, format!("no colour group `{group}`")))?;
    let name = match rename.map(str::trim).filter(|n| !n.is_empty() && *n != group) {
        Some(n) => d.free_swatch_name(n),
        None => group.to_string(),
    };
    let old = &d.swatch_groups[gi].swatches;
    if let Some(w) = old.iter().enumerate().find(|(i, w)| w.is_reserved() && colors.get(*i).is_none_or(|c| w.paint.color() != Some(*c))).map(|x| x.1)
    {
        return Err(bad(cmd, format!("`{}` can't be edited or removed", w.name)));
    }
    let mut relinks = vec![];
    let mut swatches = std::mem::take(&mut d.swatch_groups[gi].swatches);
    for gone in swatches.split_off(colors.len().min(swatches.len())) {
        if gone.global {
            relinks.push(Relink { from: gone.name.clone(), to: gone.name, color: None, keep: false });
        }
    }
    for (w, c) in swatches.iter_mut().zip(colors) {
        if w.paint.color() != Some(*c) {
            // A tint swatch becomes a colour of its own.
            w.paint = Paint::solid(*c);
            if w.global {
                // Art linked to a Lab spot shows it as the Spot Colors options say.
                relinks.push(Relink { from: w.name.clone(), to: w.name.clone(), color: Some(d.linked_color(*c, w.spot)), keep: true });
            }
        }
    }
    for c in colors.iter().skip(swatches.len()) {
        let nm = unique_name(&color_name(*c), |n| n == name || d.swatch_name_taken(n) || swatches.iter().any(|w| w.name == n));
        swatches.push(Swatch { name: nm, paint: Paint::solid(*c), global: false, spot: false });
    }
    let g = &mut d.swatch_groups[gi];
    (g.name, g.swatches) = (name.clone(), swatches);
    let names = g.swatches.iter().map(|w| w.name.clone()).collect();
    let relinked = if relinks.is_empty() { 0 } else { d.map_solid_paints(&mut |c, l, t| relinks.iter().any(|r| r.apply(c, l, t))) };
    Ok(GroupEdit { name, swatches: names, relinks, relinked })
}

fn swatch_edit_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.editGroup";
    let group = str_param(p, "group").ok_or_else(|| bad(C, "missing `group`"))?;
    let colors: Vec<Color> = p
        .get("colors")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(C, "missing `colors`"))?
        .iter()
        .map(|v| color_value(v).ok_or_else(|| bad(C, format!("bad colour {v}"))))
        .collect::<Result<_>>()?;
    // A color its swatch already has keeps its model. The others take the document's.
    let old: Vec<Option<Color>> =
        s.doc()?.doc.swatch_groups.iter().find(|g| g.name == group).map(|g| g.swatches.iter().map(|w| w.paint.color()).collect()).unwrap_or_default();
    let colors: Vec<Color> =
        colors.into_iter().enumerate().map(|(i, c)| if old.get(i) == Some(&Some(c)) { c } else { super::paint::new_color(s, p, c) }).collect();
    let rename = str_param(p, "rename");
    let e = s.edit("Edit Color Group", |d, _| rewrite_group(d, group, &colors, rename, C))?;
    e.defaults(s);
    Ok(json!({"name": e.name, "swatches": e.swatches, "relinked": e.relinked}))
}
