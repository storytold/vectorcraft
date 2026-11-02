//! Overprinting (Attributes panel → Overprint Fill / Overprint Stroke, Edit → Edit Colors →
//! Overprint Black): each fill and stroke, and each character's fill and stroke, can overprint
//! ([`vectorcraft_doc::FillLayer::overprint`]). Overprint Preview and Separations Preview show it.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::overprint::OverprintBlack;
use vectorcraft_doc::{Document, ImageMap, Node, NodeId, NodeKind};
use vectorcraft_geom::FillRule;

use super::appearance::{ItemTarget, item_target};
use super::edit::selected_roots;
use super::{CommandSpec, Result, Session, bad, bool_or, cmd, f64_or, has_doc, ids_param, targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.setOverprint",
            "Overprint",
            [],
            None,
            "{fill?: bool, stroke?: bool, item?: appearance item index|null, ids?|id?, target?: \"object\"|\"contents\"} make fills and/or strokes overprint (Overprint Preview, Separations Preview) or knock out: item's, else every fill or stroke of the painted objects (groups: their contents; type: its characters too). Without item and ids, the Appearance panel's active item stands in when it is of that kind → {changed: objects}",
            has_doc,
            set_overprint
        ),
        cmd!(
            query "attributes.info",
            "Attributes",
            [],
            None,
            "{ids?|id?, item?, target?: \"object\"|\"contents\"} → {ids, overprintFill, overprintStroke, showCenter, imageMap: \"none\"|\"rectangle\"|\"polygon\", url, note, fillRule: \"nonZero\"|\"evenOdd\" (of the paths and compound paths in them), reversed (their subpaths run counter-clockwise on screen: Reverse Path Direction On), recentUrls: [newest first]} the Attributes panel's values for `ids` or the selection, overprint aimed as object.setOverprint aims; a value is null where they differ (or nothing has a fill, stroke or path)",
            has_doc,
            |s, p| Ok(attributes(s, p)?.to_json())
        ),
    ]
}

/// The Attributes panel's values ([`Session::attributes_info`], `attributes.info`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttributesInfo {
    /// The targets (the selection by default).
    pub ids: Vec<NodeId>,
    /// Overprint Fill; `None` where the targets differ or have no fill.
    pub overprint_fill: Option<bool>,
    /// Overprint Stroke; `None` where the targets differ or have no stroke.
    pub overprint_stroke: Option<bool>,
    /// The rest of the panel ([`super::attributes`]); `None` where the targets differ.
    pub show_center: Option<bool>,
    pub image_map: Option<ImageMap>,
    pub url: Option<String>,
    pub note: Option<String>,
    /// Fill rule of the paths and compound paths in the targets.
    pub fill_rule: Option<FillRule>,
    /// Whether their subpaths run counter-clockwise on screen (Reverse Path Direction On).
    pub reversed: Option<bool>,
    /// [`Session::recent_urls`].
    pub recent_urls: Vec<String>,
}

impl AttributesInfo {
    fn to_json(&self) -> Value {
        json!({
            "ids": self.ids.iter().map(|id| id.0).collect::<Vec<_>>(),
            "overprintFill": self.overprint_fill,
            "overprintStroke": self.overprint_stroke,
            "showCenter": self.show_center,
            "imageMap": self.image_map,
            "url": self.url,
            "note": self.note,
            "fillRule": self.fill_rule.map(super::attributes::rule_name),
            "reversed": self.reversed,
            "recentUrls": self.recent_urls,
        })
    }
}

impl Session {
    /// The Attributes panel's values for the selection.
    pub fn attributes_info(&self) -> AttributesInfo {
        attributes(self, &Value::Null).unwrap_or_default()
    }
}

/// The overprint flags of `n` a fill (`fill`) or stroke edit aimed at item `index` covers: that
/// item's, or (`None`) every fill's or stroke's and, on type, its characters'.
fn flags(n: &Node, fill: bool, index: Option<usize>) -> Vec<bool> {
    let mut v: Vec<bool> = n
        .appearance
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| index.map_or(it.is_fill() == fill, |x| x == *i))
        .map(|(_, it)| it.overprint())
        .collect();
    if let (None, NodeKind::Text(t)) = (index, &n.kind) {
        v.extend(t.runs.iter().map(|r| if fill { r.style.overprint_fill } else { r.style.overprint_stroke }));
    }
    v
}

/// [`flags`], to change.
fn flags_mut(n: &mut Node, fill: bool, index: Option<usize>) -> Vec<&mut bool> {
    let mut v: Vec<&mut bool> = n
        .appearance
        .items
        .iter_mut()
        .enumerate()
        .filter(|(i, it)| index.map_or(it.is_fill() == fill, |x| x == *i))
        .map(|(_, it)| it.overprint_mut())
        .collect();
    if let (None, NodeKind::Text(t)) = (index, &mut n.kind) {
        v.extend(t.runs.iter_mut().map(|r| if fill { &mut r.style.overprint_fill } else { &mut r.style.overprint_stroke }));
    }
    v
}

/// The fill and stroke overprint params given under `keys` (fill's, stroke's): (fill?, on).
fn kinds(p: &Value, cmd: &str, keys: [&str; 2]) -> Result<Vec<(bool, bool)>> {
    [(true, keys[0]), (false, keys[1])]
        .into_iter()
        .filter_map(|(fill, key)| {
            p.get(key).map(|v| v.as_bool().map(|on| (fill, on)).ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))))
        })
        .collect()
}

/// Each kind's item target and objects: (fill?, item, ids).
fn aims(s: &Session, p: &Value, cmd: &str, kinds: impl IntoIterator<Item = bool>) -> Result<Vec<(bool, ItemTarget, Vec<NodeId>)>> {
    let item = item_target(s, p, cmd)?;
    kinds
        .into_iter()
        .map(|fill| {
            let it = item.of_kind(s, fill);
            Ok((fill, it, it.targets(s, p)?))
        })
        .collect()
}

/// Run `f` on a copy of the document and commit it as one undo step (`label`) only when `f`
/// reports changes; returns their count.
pub(super) fn edit_counted(s: &mut Session, label: &str, f: impl FnOnce(&mut Document) -> Result<usize>) -> Result<usize> {
    let mut d = (*s.doc()?.doc).clone();
    let n = f(&mut d)?;
    if n > 0 {
        s.edit(label, |doc, _| {
            *doc = d;
            Ok(())
        })?;
    }
    Ok(n)
}

/// One overprint edit: (fill?, item, objects), on.
pub(super) type OverprintJob = ((bool, ItemTarget, Vec<NodeId>), bool);

/// The overprint edits `p` asks for under `keys` (the fill's and the stroke's), aimed as
/// `object.setOverprint` aims.
pub(super) fn overprint_jobs(s: &Session, p: &Value, cmd: &str, keys: [&str; 2]) -> Result<Vec<OverprintJob>> {
    let kinds = kinds(p, cmd, keys)?;
    Ok(aims(s, p, cmd, kinds.iter().map(|k| k.0))?.into_iter().zip(kinds.iter().map(|k| k.1)).collect())
}

/// Apply `jobs` to `d`, adding the objects changed to `changed`.
pub(super) fn apply_overprint(d: &mut Document, jobs: &[OverprintJob], cmd: &str, changed: &mut BTreeSet<NodeId>) -> Result<()> {
    for ((fill, item, ids), on) in jobs {
        for id in ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let index = item.resolve(&n.appearance, *fill, cmd)?;
            for f in flags_mut(n, *fill, index) {
                if *f != *on {
                    *f = *on;
                    changed.insert(*id);
                }
            }
        }
    }
    Ok(())
}

fn set_overprint(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.setOverprint";
    let jobs = overprint_jobs(s, p, C, ["fill", "stroke"])?;
    if jobs.is_empty() {
        return Err(bad(C, "give `fill` and/or `stroke`"));
    }
    let changed = edit_counted(s, "Overprint", |d| {
        let mut changed = BTreeSet::new();
        apply_overprint(d, &jobs, C, &mut changed)?;
        Ok(changed.len())
    })?;
    Ok(json!({ "changed": changed }))
}

fn attributes(s: &Session, p: &Value) -> Result<AttributesInfo> {
    const C: &str = "attributes.info";
    let d = &s.doc()?.doc;
    let mut info = AttributesInfo { ids: targets(s, p)?, recent_urls: s.recent_urls.clone(), ..Default::default() };
    super::attributes::fill_info(d, &mut info);
    for (fill, item, ids) in aims(s, p, C, [true, false])? {
        let mut all =
            ids.iter().filter_map(|id| d.node(*id)).filter_map(|n| Some(flags(n, fill, item.resolve(&n.appearance, fill, C).ok()?))).flatten();
        let same = all.next().and_then(|first| all.all(|b| b == first).then_some(first));
        if fill {
            info.overprint_fill = same;
        } else {
            info.overprint_stroke = same;
        }
    }
    Ok(info)
}

/// Overprint Black on `n` and its descendants; returns the objects changed.
fn overprint_black_tree(n: &mut Node, op: &OverprintBlack, spots: &[String]) -> usize {
    let mut changed = usize::from(op.apply(n, spots));
    for c in n.children_mut().into_iter().flatten() {
        changed += overprint_black_tree(Arc::make_mut(c), op, spots);
    }
    changed
}

pub(crate) fn overprint_black(s: &mut Session, p: &Value) -> Result<Value> {
    let op = OverprintBlack {
        remove: bool_or(p, "remove", false),
        min_k: (f64_or(p, "percentage", 100.0) / 100.0).clamp(0.0, 1.0) as f32,
        fill: bool_or(p, "fill", true),
        stroke: bool_or(p, "stroke", true),
        rich: bool_or(p, "includeCmyBlacks", false),
        spot: bool_or(p, "includeSpotBlacks", false),
    };
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    let changed = edit_counted(s, "Overprint Black", |d| {
        let spots = d.spot_names();
        Ok(ids.iter().map(|id| d.node_mut(*id).map_or(0, |n| overprint_black_tree(n, &op, &spots))).sum())
    })?;
    Ok(json!({ "changed": changed }))
}
