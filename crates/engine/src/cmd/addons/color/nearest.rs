//! Find Nearest Swatch (`addon.color.nearestSwatch`, `addon.color.applyNearestSwatch`; Object ›
//! Plug-ins › Color Effects › Find Nearest Swatch…): the swatches of a library, a colour book the
//! user loaded or the document closest to each colour of the selection (CIEDE2000), and the
//! colours replaced by them as one undo step (#4923).

use serde_json::{Value, json};
use vectorcraft_color::cms::{Lab, delta_e2000};
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::swatches::color_name;
use vectorcraft_doc::{Document, NodeId, NodeKind};

use super::super::super::colorcmds::recolor_links;
use super::super::super::menucmds::squash;
use super::super::super::swatchlib::{DOCUMENT_SWATCHES, library_swatches};
use super::super::super::{bad, checked_ids_or_selection, color_value, index, switch_or, text_param};
use super::super::arrange::whole_in;
use crate::{Result, Session};

const FIND: &str = "addon.color.nearestSwatch";
const APPLY: &str = "addon.color.applyNearestSwatch";
/// The most colours of the selection looked up.
const MAX_COLORS: usize = 100;

/// The `library` param ([`DOCUMENT_SWATCHES`] when left out) and its colour swatches with their Lab
/// values.
fn candidates(s: &Session, p: &Value, cmd: &str) -> Result<(String, Vec<(Swatch, Lab)>)> {
    let key = text_param(p, "library", cmd)?.unwrap_or(DOCUMENT_SWATCHES).to_string();
    let swatches = library_swatches(s, &key).ok_or_else(|| bad(cmd, format!("no swatch library `{key}` (see swatch.library.list)")))?;
    let labs: Vec<(Swatch, Lab)> = swatches
        .into_iter()
        .filter(|w| !w.is_reserved())
        .filter_map(|w| {
            let lab = w.paint.color()?.to_lab();
            Some((w, lab))
        })
        .collect();
    if labs.is_empty() {
        return Err(bad(cmd, format!("`{key}` has no colours")));
    }
    Ok((key, labs))
}

/// The `count` swatches of `among` closest to `c`, closest first, with their difference (ΔE 2000).
fn nearest(among: &[(Swatch, Lab)], c: Color, count: usize) -> Vec<(&Swatch, f32)> {
    let lab = c.to_lab();
    let mut v: Vec<(&Swatch, f32)> = among.iter().map(|(w, l)| (w, delta_e2000(*l, lab))).collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v.truncate(count);
    v
}

/// The solid colours and gradient stops of the fills (`fill`) and strokes (`stroke`) of `ids`, of
/// everything inside them and of their type, once each and at most [`MAX_COLORS`].
fn art_colors(d: &Document, ids: &[NodeId], fill: bool, stroke: bool) -> Vec<Color> {
    let mut out: Vec<Color> = vec![];
    let mut add = |p: &Paint| {
        let colors: Vec<Color> = match p {
            Paint::Solid { color, .. } => vec![*color],
            Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.color).collect(),
            _ => vec![],
        };
        for c in colors {
            if out.len() < MAX_COLORS && !out.contains(&c) {
                out.push(c);
            }
        }
    };
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        n.walk(&mut |m| {
            for it in &m.appearance.items {
                if if it.is_fill() { fill } else { stroke } {
                    add(it.paint());
                }
            }
            if let NodeKind::Text(t) = &m.kind {
                for r in &t.runs {
                    if fill {
                        add(&r.style.fill);
                    }
                    if stroke {
                        add(&r.style.stroke);
                    }
                }
            }
        });
    }
    out
}

/// A swatch's name and values as the results give them.
fn described(w: &Swatch) -> Value {
    let c = w.paint.color().unwrap_or_default();
    json!({ "name": w.name, "values": color_name(c), "hex": c.to_hex(), "spot": w.spot })
}

pub(super) fn find(s: &mut Session, p: &Value) -> Result<Value> {
    let count = whole_in(p, "count", (1, 50), FIND)?.map_or(5, index);
    let (fill, stroke) = (switch_or(p, "fill", true, FIND)?, switch_or(p, "stroke", true, FIND)?);
    let colors = match p.get("color").filter(|v| !v.is_null()) {
        Some(v) => vec![color_value(v).ok_or_else(|| bad(FIND, "`color` is not a colour"))?],
        None => {
            let ids = checked_ids_or_selection(s, p, FIND)?;
            if ids.is_empty() {
                return Err(bad(FIND, "select objects or give `color`"));
            }
            art_colors(&s.doc()?.doc, &ids, fill, stroke)
        }
    };
    let (library, among) = candidates(s, p, FIND)?;
    let found: Vec<Value> = colors
        .iter()
        .map(|c| {
            let matches: Vec<Value> = nearest(&among, *c, count)
                .into_iter()
                .map(|(w, de)| {
                    let mut m = described(w);
                    m["deltaE"] = json!((f64::from(de) * 100.0).round() / 100.0);
                    m
                })
                .collect();
            json!({ "values": color_name(*c), "hex": c.to_hex(), "matches": matches })
        })
        .collect();
    Ok(json!({ "library": library, "colors": found }))
}

pub(super) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let (fill, stroke) = (switch_or(p, "fill", true, APPLY)?, switch_or(p, "stroke", true, APPLY)?);
    let ids = checked_ids_or_selection(s, p, APPLY)?;
    if ids.is_empty() {
        return Err(bad(APPLY, "nothing selected"));
    }
    let (library, among) = candidates(s, p, APPLY)?;
    // The swatch each colour takes, once each.
    let colors = art_colors(&s.doc()?.doc, &ids, fill, stroke);
    let mut used: Vec<&Swatch> = vec![];
    for c in &colors {
        if let Some((w, _)) = nearest(&among, *c, 1).first()
            && !used.iter().any(|u| u.name == w.name)
        {
            used.push(w);
        }
    }
    if used.is_empty() {
        return Ok(json!({ "changed": 0, "swatches": [], "added": [] }));
    }
    let undo = s.doc()?.history.undo.len();
    // A library's swatches join the document's first, as Add to Swatches adds them, so spot and
    // global colours stay linked to theirs.
    let mut added = vec![];
    let mut names: Vec<String> = used.iter().map(|w| w.name.clone()).collect();
    if library != DOCUMENT_SWATCHES {
        let r = s.execute("swatch.library.add", &json!({ "library": library, "names": names }))?;
        let listed = |k: &str| r.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let in_doc: Vec<String> = listed("added").iter().chain(&listed("existing")).filter_map(Value::as_str).map(str::to_string).collect();
        let d = &s.doc()?.doc;
        // Each library swatch's copy: the document swatch with its paint among those added or kept.
        names = used
            .iter()
            .map(|w| in_doc.iter().find(|n| d.swatch(n).is_some_and(|x| x.paint == w.paint)).cloned().unwrap_or_else(|| w.name.clone()))
            .collect();
        added = listed("added");
    }
    // Each used swatch's paint as art takes it: linked to a global or spot swatch.
    let d = &s.doc()?.doc;
    let paints: Vec<(Lab, Paint)> = used
        .iter()
        .zip(&names)
        .filter_map(|(w, name)| {
            let lab = w.paint.color()?.to_lab();
            let paint = d.swatch(name).filter(|x| x.global).and_then(|_| d.tint_paint(name, 1.0)).unwrap_or_else(|| w.paint.clone());
            Some((lab, paint))
        })
        .collect();
    // shortcut: a colour past the MAX_COLORS looked up takes the closest of the swatches those
    // took; look up every colour if documents with more colours need their own matches.
    let to = |c: Color| -> Option<&Paint> {
        let lab = c.to_lab();
        paints.iter().min_by(|a, b| delta_e2000(a.0, lab).total_cmp(&delta_e2000(b.0, lab))).map(|(_, p)| p)
    };
    let links = |c: &mut Color, link: &mut Option<String>, tint: &mut f32| {
        let Some(Paint::Solid { color, swatch, .. }) = to(*c) else { return false };
        if *c == *color && *link == *swatch {
            return false;
        }
        (*c, *link, *tint) = (*color, swatch.clone(), 1.0);
        true
    };
    let given = json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "fill": fill, "stroke": stroke, "includeImages": false, "includePatterns": false,
    });
    let r = recolor_links(s, &given, "Apply Nearest Swatch", &|c| to(c).and_then(Paint::color).unwrap_or(c), &links)?;
    squash(s, undo, "Apply Nearest Swatch");
    Ok(json!({ "changed": r["changed"], "swatches": names, "added": added }))
}

#[cfg(test)]
mod tests;
