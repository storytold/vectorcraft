//! Fill and stroke paint (the toolbar proxies and their defaults) and the Transparency panel.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind};

use super::appearance::{ItemTarget, appearance_targets, edit_items, item_target};
use super::gradient::{item_paint_bounds, place_paint, place_run_paint, run_paint_mut, run_stroke_weight, unplaced};
use super::*;
use crate::{DocState, EngineError};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.setFill",
            "Fill",
            [],
            None,
            "{color?: \"#rrggbb\"|[r,g,b]|{c,m,y,k}|{gray}|{l,a,b} (CIE Lab), none?: true, swatch?: name (a global or spot colour stays linked, so swatch edits recolour it; a tint swatch links to its base at its tint; a gradient swatch is recorded as the gradient's swatch and fits each object, keeping its aspect; the built-in \"[Registration]\" prints on every plate), tint?: 0..100 (% of a global or spot `swatch`; default 100, or a tint swatch's own), gradient?: {kind?: linear|radial|freeform, stops?: [{offset 0..1, color, opacity? 0..1 (or 0..100), midpoint? 0.13..0.87, swatch?: global or spot colour (or tint) swatch the stop links to (its colour comes from the swatch), tint?: 0..100}] (at least 2; default white→black), angle?: deg, start?: [x,y], end?: [x,y] (the vector in document coordinates, both or neither; type objects keep it in text space), aspect?: % (radial; without start/end the gradient is placed on each object's bounds), focal?: [x,y] (radial, with start/end: the focal point, where the first stop sits), swatch?: linked gradient swatch name}, item?: appearance item index|null (omitted: the Appearance panel's active item if it is a fill, else the top fill), ids?, focus?: true (false keeps the active proxy), keepModel?: false (in a CMYK document, RGB colours and gradient stops are stored as CMYK unless true; Gray stays Gray)} sets the selection's fill and the default (new art fits a gradient to itself)",
            has_doc,
            |s, p| set_paint(s, p, true)
        ),
        cmd!("paint.setStroke", "Stroke", [], None, "same as paint.setFill, for the stroke (item?: the stroke item to set)", has_doc, |s, p| {
            set_paint(s, p, false)
        }),
        cmd!(
            "paint.swap",
            "Swap Fill and Stroke",
            [],
            Some("Shift+X"),
            "{ids?} swap the fill and stroke of the selection (type too) and of the defaults",
            has_doc,
            swap
        ),
        cmd!(
            "paint.default",
            "Default Fill and Stroke",
            [],
            Some("D"),
            "{ids?} white fill and 1 pt black stroke for the selection and the defaults; type gets black fill and no stroke",
            has_doc,
            default_paint
        ),
        cmd!(
            "paint.toggleActive",
            "Toggle Fill/Stroke Focus",
            [],
            Some("X"),
            "{fill?: bool (true: bring the Fill proxy forward, false: the Stroke proxy; omitted: swap which is in front)} → {fillActive}",
            always,
            |s, p| {
                s.fill_active = p.get("fill").and_then(Value::as_bool).unwrap_or(!s.fill_active);
                Ok(json!({ "fillActive": s.fill_active }))
            }
        ),
        cmd!("paint.none", "None", [], Some("/"), "{} set the active proxy (fill or stroke) to None", has_doc, |s, _| {
            let f = s.fill_active;
            set_paint(s, &json!({"none": true}), f)
        }),
        cmd!(
            "transparency.set",
            "Transparency",
            ["Window", "Transparency"],
            None,
            "{ids?|id?, opacity?: 0..100, blend?: name, isolate?, knockout?: \"on\"|\"off\"|\"neutral\"|bool (true = on, false = neutral), knockoutShape?: bool, item?: appearance item index|null} for `ids`, the selection, or the object whose opacity mask is being edited; opacity and blend go to the targeted fill/stroke item (omitted: the Appearance panel's active item, else the objects)",
            has_doc,
            transparency
        ),
    ]
}

/// Commands of the Fill/Stroke proxies, registered at the end of the command list.
pub fn proxy_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.invert",
            "Invert",
            [],
            None,
            "{stroke?: bool (default: the active proxy), ids?} invert the active proxy's colours of the selection (or ids; groups recolour their contents) keeping each colour's model, as edit.colors.invert does; the Appearance panel's active fill/stroke item (appearance.setActiveItem) is inverted instead when it is of that kind and no ids are given; with nothing selected, invert the default → {changed}",
            has_doc,
            |s, p| proxy_recolor(s, p, "paint.invert", "Invert", &super::colorcmds::invert)
        ),
        cmd!(
            "paint.complement",
            "Complement",
            [],
            None,
            "{stroke?: bool (default: the active proxy), ids?} replace the active proxy's colours by their complements ((highest + lowest) − each component, over RGB or CMY; grey unchanged) keeping each colour's model; honours the Appearance panel's active item as paint.invert does; with nothing selected, the default → {changed}",
            has_doc,
            |s, p| proxy_recolor(s, p, "paint.complement", "Complement", &|c: Color| c.complement_keep_model())
        ),
        cmd!(
            "paint.lastColor",
            "Apply Last Color",
            [],
            Some(","),
            "{stroke?: bool (default: the active proxy), ids?} apply the last solid colour used (paint.recent lastColor) to the active proxy of the selection and the default",
            has_doc,
            |s, p| {
                let paint = Paint::solid(s.last_solid);
                apply_to_proxy(s, p, paint)
            }
        ),
        cmd!(
            "paint.lastGradient",
            "Apply Last Gradient",
            [],
            Some("."),
            "{stroke?: bool (default: the active proxy), ids?} apply the last gradient used (paint.recent lastGradient), fitted to each object, to the active proxy of the selection and the default",
            has_doc,
            |s, p| {
                let paint = super::gradient::unplaced(&Paint::Gradient(Box::new(s.last_gradient.clone())));
                apply_to_proxy(s, p, paint)
            }
        ),
        cmd!(
            query "paint.recent",
            "Recent Colors",
            [],
            None,
            "{} → {colors: [{hex, color, swatch?: the global or spot swatch it came from, tint?: %}] newest first (fed by every paint command and the eyedropper), lastColor: {hex, color}, lastGradient: gradient paint}",
            always,
            |s, _| {
                let c = |c: &Color| json!({"hex": c.to_hex(), "color": c});
                let recent = s.recent_colors.iter().enumerate().map(|(i, col)| {
                    let mut v = c(col);
                    if let Some(Some((name, tint))) = s.recent_links.get(i) {
                        v["swatch"] = json!(name);
                        v["tint"] = json!(tint * 100.0);
                    }
                    v
                });
                Ok(json!({
                    "colors": recent.collect::<Vec<_>>(),
                    "lastColor": c(&s.last_solid),
                    "lastGradient": s.last_gradient,
                }))
            }
        ),
        cmd!(
            query "paint.proxies",
            "Fill and Stroke",
            [],
            None,
            "{} → {fill, stroke: the paints the Fill/Stroke proxies show (the Appearance panel's active item for the proxy of its kind, else the first selected object's, a group's first painted object's, type: its first run's; else the defaults for new art), fillActive, fillMixed, strokeMixed: the selected objects' (or a selected group's contents') fills (strokes) differ, shown as a \"?\" proxy}",
            always,
            proxies
        ),
    ]
}

impl Session {
    /// How many colours the Recent Colors rows keep.
    pub const RECENT_MAX: usize = 10;

    /// Remember an applied paint: a solid colour becomes the last colour and the newest recent
    /// colour, a gradient the last gradient. During a live preview this waits for the commit.
    pub(crate) fn remember_paint(&mut self, p: &Paint) {
        if self.in_interaction() {
            self.pending_paint = Some(p.clone());
        } else {
            self.remember_paint_now(p);
        }
    }

    /// The fill and stroke the Fill/Stroke proxies show: the Appearance panel's active item (in the
    /// first selected object's own stack) for the proxy of its kind, else the first selected
    /// object's (a group's first painted object's; type: its first run's), else the defaults for
    /// new art.
    pub fn proxy_paints(&self) -> (Paint, Paint) {
        let show = |stroke: bool| match self.proxy_source(stroke) {
            Some((n, item)) => proxy_paint(n, stroke, item),
            None if stroke => self.paint.stroke.clone(),
            None => self.paint.fill.clone(),
        };
        (show(false), show(true))
    }

    /// The object whose fill (or stroke) the proxy shows, with the Appearance panel's active item
    /// when it stands for it (see [`Session::proxy_paints`]); None: the defaults for new art.
    pub(crate) fn proxy_source(&self, stroke: bool) -> Option<(&Node, Option<usize>)> {
        let st = self.active()?;
        let first = st.doc.node(*st.selection.subjects().first()?)?;
        let item = self.appearance_item();
        if first.appearance.item_of_kind(item, !stroke).is_some() {
            return Some((first, item));
        }
        let mut nodes = vec![];
        painted(first, true, &mut nodes);
        nodes.first().map(|n| (*n, None))
    }

    pub(crate) fn remember_paint_now(&mut self, p: &Paint) {
        match p {
            Paint::Solid { color, swatch, tint } => {
                self.last_solid = *color;
                let link = swatch.clone().map(|n| (n, *tint));
                // Keep the links beside their colours (older sessions may have fewer).
                self.recent_links.resize(self.recent_colors.len(), None);
                let keep: Vec<bool> = self.recent_colors.iter().zip(&self.recent_links).map(|(c, l)| !(c == color && *l == link)).collect();
                let mut k = keep.iter();
                self.recent_colors.retain(|_| k.next().copied().unwrap_or(true));
                let mut k = keep.iter();
                self.recent_links.retain(|_| k.next().copied().unwrap_or(true));
                self.recent_colors.insert(0, *color);
                self.recent_links.insert(0, link);
                self.recent_colors.truncate(Self::RECENT_MAX);
                self.recent_links.truncate(Self::RECENT_MAX);
            }
            Paint::Gradient(g) => self.last_gradient = (**g).clone(),
            Paint::None | Paint::Pattern { .. } => {}
        }
    }
}

/// The fill or stroke a proxy shows for `n` ([`Node::proxy_paint`]: fill or stroke `item` when it
/// is of that kind, type's first run, else the topmost one).
pub(crate) fn proxy_paint(n: &Node, stroke: bool, item: Option<usize>) -> Paint {
    match n.proxy_paint(stroke, item) {
        Some((p, ..)) => p.clone(),
        // Type without runs shows the default character style.
        None if matches!(n.kind, NodeKind::Text(_)) => {
            let st = CharStyle::default();
            if stroke { st.stroke } else { st.fill }
        }
        None => Paint::None,
    }
}

/// Do two paints look the same in a proxy? Solid colours compare without their swatch link and
/// gradients without their per-object geometry.
fn same_in_proxy(a: &Paint, b: &Paint) -> bool {
    match (a, b) {
        (Paint::Solid { color: x, .. }, Paint::Solid { color: y, .. }) => x == y,
        (Paint::Gradient(x), Paint::Gradient(y)) => x.gradient == y.gradient,
        _ => a == b,
    }
}

/// The objects whose paints the proxies show for a selected `n`, the ones the paint commands
/// change ([`leaf_targets`]): groups and layers stand for their contents (and so do blends and
/// other containers inside them), a compound path for its parts.
pub(crate) fn painted<'a>(n: &'a Node, top: bool, out: &mut Vec<&'a Node>) {
    let expand = match n.kind {
        NodeKind::Group { .. } | NodeKind::Layer { .. } => true,
        NodeKind::Compound { .. } => false,
        _ => !top && n.is_container(),
    };
    if expand {
        n.children().into_iter().flatten().for_each(|c| painted(c, false, out));
    } else {
        out.push(n);
    }
}

impl DocState {
    /// Whether the selected objects' fills and strokes differ as the proxies show them (a "?"
    /// proxy; a group whose contents differ counts). `item` (the Appearance panel's active item)
    /// stands for the proxy of its kind, as in [`Session::proxy_paints`]: that fill or stroke of
    /// each selected object's own stack is compared. One walk of the document, so callers that ask
    /// every frame cache it by revision.
    pub fn proxy_mixed(&self, item: Option<usize>) -> (bool, bool) {
        if self.selection.is_empty() {
            return (false, false);
        }
        let ids: std::collections::HashSet<NodeId> = self.selection.objects.iter().copied().collect();
        let (mut selected, mut nodes) = (vec![], vec![]);
        self.doc.walk(|n| {
            if ids.contains(&n.id) {
                selected.push(n);
                painted(n, true, &mut nodes);
            }
        });
        let first = self.selection.objects.first().and_then(|id| self.doc.node(*id));
        let differ = |stroke: bool| {
            let item = first.and_then(|f| f.appearance.item_of_kind(item, !stroke));
            let set = if item.is_some() { &selected } else { &nodes };
            let mut paints = set.iter().map(|n| proxy_paint(n, stroke, item));
            let Some(p0) = paints.next() else { return false };
            paints.any(|p| !same_in_proxy(&p0, &p))
        };
        (differ(false), differ(true))
    }
}

impl Session {
    /// [`DocState::proxy_mixed`] of the active document for the Appearance panel's active item.
    pub fn proxy_mixed(&self) -> (bool, bool) {
        self.active().map_or((false, false), |st| st.proxy_mixed(self.appearance_item()))
    }
}

fn proxies(s: &mut Session, _: &Value) -> Result<Value> {
    let (fill, stroke) = s.proxy_paints();
    let (fill_mixed, stroke_mixed) = s.proxy_mixed();
    Ok(json!({"fill": fill, "stroke": stroke, "fillActive": s.fill_active, "fillMixed": fill_mixed, "strokeMixed": stroke_mixed}))
}

/// `stroke` param, defaulting to the proxy that is in front.
fn stroke_param(s: &Session, p: &Value) -> bool {
    p.get("stroke").and_then(Value::as_bool).unwrap_or(!s.fill_active)
}

/// The model RGB colours given in `p` are stored in: the document's in a CMYK document (`None` in
/// an RGB one, or with `keepModel`).
fn new_color_model(s: &Session, p: &Value) -> Option<vectorcraft_color::cms::Model> {
    let mode = s.active()?.doc.color_mode;
    (mode != vectorcraft_doc::ColorMode::Rgb && !bool_or(p, "keepModel", false)).then(|| mode.model())
}

/// Parse a paint from params (color / none / swatch / gradient). None = no paint keys given. A
/// colour or gradient given in RGB takes the document's colour model ([`new_color_model`]).
pub(crate) fn paint_from(s: &Session, p: &Value) -> Result<Option<Paint>> {
    let model = new_color_model(s, p);
    let new_color = |c: Color| match (c, model) {
        (Color::Rgb { .. }, Some(m)) => c.in_model(m),
        _ => c,
    };
    if bool_or(p, "none", false) {
        return Ok(Some(Paint::None));
    }
    if let Some(name) = str_param(p, "swatch") {
        let d = &s.doc()?.doc;
        // A pattern definition works as its swatch even without a swatch entry.
        if d.swatch(name).is_none() && d.pattern(name).is_some() {
            return Ok(Some(vectorcraft_doc::pattern::pattern_paint(name)));
        }
        let sw = d.swatch(name).ok_or_else(|| EngineError::Other(format!("no swatch `{name}`")))?;
        let tint = tint_param(p, "paint")?;
        return match &sw.paint {
            Paint::Solid { .. } => swatch_solid(d, name, tint).map(Some).map_err(|e| bad("paint", e)),
            _ if tint.is_some() => Err(bad("paint", "`tint` applies to global and spot colours")),
            Paint::Gradient(g) => Ok(Some(Paint::Gradient(Box::new(GradientPaint { swatch: Some(name.to_string()), ..(**g).clone() })))),
            other => Ok(Some(other.clone())),
        };
    }
    if let Some(g) = p.get("gradient") {
        let g = super::gradient::link_stops(&s.doc()?.doc, g).map_err(|e| bad("paint", e))?;
        let mut gp = super::gradient::parse_gradient(&g).map_err(|e| bad("paint", e))?;
        gp.gradient.stops.iter_mut().filter(|st| st.swatch.is_none()).for_each(|st| st.color = new_color(st.color));
        return Ok(Some(Paint::Gradient(Box::new(gp))));
    }
    if let Some(c) = p.get("color") {
        let c = color_value(c).ok_or_else(|| bad("paint", format!("bad color {c}")))?;
        return Ok(Some(Paint::solid(new_color(c))));
    }
    Ok(None)
}

/// The `tint` param (a percentage) as 0..1.
pub(crate) fn tint_param(p: &Value, cmd: &str) -> Result<Option<f32>> {
    p.get("tint")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().map(|t| (t / 100.0).clamp(0.0, 1.0) as f32).ok_or_else(|| bad(cmd, "`tint` must be a number (%)")))
        .transpose()
}

/// The solid paint colour swatch `name` applies at `tint` (0..1): a global or spot colour links to
/// itself (default 100%), a tint swatch to its base (default: its own tint), a process colour is
/// just its colour (it has no tints).
pub(crate) fn swatch_solid(d: &Document, name: &str, tint: Option<f32>) -> std::result::Result<Paint, String> {
    let sw = d.swatch(name).ok_or_else(|| format!("no swatch `{name}`"))?;
    if sw.paint.color().is_none() {
        return Err(format!("`{name}` isn't a colour swatch"));
    }
    match (d.swatch_link(name), tint) {
        // A tint swatch whose base is gone keeps its colour.
        (Some((base, own)), _) => Ok(d.tint_paint(&base, tint.unwrap_or(own)).unwrap_or_else(|| sw.paint.clone())),
        (None, None) => Ok(sw.paint.clone()),
        (None, Some(_)) => Err(format!("`{name}` is a process colour: tints apply to global and spot colours")),
    }
}

fn set_paint(s: &mut Session, p: &Value, fill: bool) -> Result<Value> {
    let cmd = if fill { "paint.setFill" } else { "paint.setStroke" };
    if p.get("tint").is_some() && p.get("swatch").is_none() {
        return Err(bad(cmd, "`tint` goes with a global or spot `swatch`"));
    }
    let paint = paint_from(s, p)?.ok_or_else(|| bad(cmd, "give color, none, swatch or gradient"))?;
    apply_paint(s, p, paint, fill)
}

/// [`apply_paint`] on the proxy named by `stroke` (default: the active one), focusing it.
fn apply_to_proxy(s: &mut Session, p: &Value, paint: Paint) -> Result<Value> {
    let fill = !stroke_param(s, p);
    apply_paint(s, p, paint, fill)
}

/// Set the fill (or stroke) of the targets of `p` (the `item` and `ids` params, see
/// `paint.setFill`) and of the defaults for new art, focus its proxy unless `focus` is false, and
/// remember the paint.
fn apply_paint(s: &mut Session, p: &Value, paint: Paint, fill: bool) -> Result<Value> {
    let cmd = if fill { "paint.setFill" } else { "paint.setStroke" };
    // New art gets the paint fitted to itself, not placed where this one is.
    if fill {
        s.paint.fill = unplaced(&paint);
    } else {
        s.paint.stroke = unplaced(&paint);
    }
    if bool_or(p, "focus", true) {
        s.fill_active = fill;
    }
    let item = item_target(s, p, cmd)?.of_kind(s, fill);
    let ids = item.targets(s, p)?;
    edit_items(s, &ids, item, cmd, if fill { "Fill Color" } else { "Stroke Color" }, fill, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            let (xf, lb) = (t.xf, t.local_bounds());
            for r in &mut t.runs {
                if !fill && !paint.is_none() {
                    run_stroke_weight(&mut r.style);
                }
                let (cur, b) = run_paint_mut(r, !fill, lb);
                *cur = place_run_paint(&paint, p, xf, b);
            }
            return Ok(());
        }
        // A missing top fill or stroke is created first: a new stroke's weight sizes the box its
        // gradient fits.
        if n.appearance.paint_at(index, fill).is_none() {
            n.appearance.set_paint_at(index, fill, Paint::None);
        }
        let placed = place_paint(&paint, p, item_paint_bounds(n, index, fill));
        n.appearance.set_paint_at(index, fill, placed);
        Ok(())
    })?;
    s.remember_paint(&paint);
    ok()
}

/// A type run's stroke; a painted stroke on a run without one gets 1 pt.
fn set_run_stroke(style: &mut CharStyle, paint: Paint) {
    if !paint.is_none() {
        run_stroke_weight(style);
    }
    style.stroke = paint;
}

fn swap(s: &mut Session, p: &Value) -> Result<Value> {
    std::mem::swap(&mut s.paint.fill, &mut s.paint.stroke);
    let ids = paint_targets(s, p)?;
    if !ids.is_empty() {
        s.edit("Swap Fill and Stroke", |d, _| {
            for id in &ids {
                let Some(n) = d.node_mut(*id) else { continue };
                if let NodeKind::Text(t) = &mut n.kind {
                    for r in &mut t.runs {
                        let fill = std::mem::take(&mut r.style.fill);
                        r.style.fill = std::mem::take(&mut r.style.stroke);
                        set_run_stroke(&mut r.style, fill);
                    }
                    continue;
                }
                let f = n.appearance.fill_paint();
                let st = n.appearance.stroke_paint();
                n.appearance.set_fill(st);
                n.appearance.set_stroke(f);
            }
            Ok(())
        })?;
    }
    ok()
}

fn default_paint(s: &mut Session, p: &Value) -> Result<Value> {
    super::newart::reset(s);
    let ids = paint_targets(s, p)?;
    if !ids.is_empty() {
        s.edit("Default Fill and Stroke", |d, _| {
            for id in &ids {
                let Some(n) = d.node_mut(*id) else { continue };
                match &mut n.kind {
                    NodeKind::Text(t) => {
                        for r in &mut t.runs {
                            r.style.fill = Paint::solid(Color::BLACK);
                            r.style.stroke = Paint::None;
                        }
                    }
                    _ => n.appearance = Appearance::default_art(),
                }
            }
            Ok(())
        })?;
    }
    ok()
}

/// Invert / Complement (`cmd`, undone as `label`): recolour the active proxy of the targets (or the
/// default when there are none) and remember the first target's new paint. The Appearance panel's
/// active item of the proxy's kind is recoloured in each selected object's own stack instead.
fn proxy_recolor(s: &mut Session, p: &Value, cmd: &str, label: &str, f: &dyn Fn(Color) -> Color) -> Result<Value> {
    let stroke = stroke_param(s, p);
    let ids = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => super::edit::selected_roots(s)?,
    };
    if ids.is_empty() {
        let def = if stroke { &mut s.paint.stroke } else { &mut s.paint.fill };
        let changed = super::colorcmds::map_paint(def, f);
        let shown = def.clone();
        s.remember_paint(&shown);
        return Ok(json!({ "changed": changed as usize }));
    }
    let item = item_target(s, p, cmd)?.of_kind(s, !stroke);
    let (r, shown_item) = match item {
        ItemTarget::Item { index, .. } => {
            let ids = item.targets(s, p)?;
            let mut changed = 0;
            edit_items(s, &ids, item, cmd, label, !stroke, |n, i| {
                let paint =
                    if stroke { n.appearance.stroke_at_mut(i).map(|l| &mut l.paint) } else { n.appearance.fill_at_mut(i).map(|l| &mut l.paint) };
                changed += paint.is_some_and(|paint| super::colorcmds::map_paint(paint, f)) as usize;
                Ok(())
            })?;
            (json!({ "changed": changed }), Some(index))
        }
        // The proxy's colours only: images and pattern tiles stay as they are.
        ItemTarget::Top => {
            let q = json!({"fill": !stroke, "stroke": stroke, "includeImages": false, "includePatterns": false, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()});
            (super::colorcmds::recolor(s, &q, label, f)?, None)
        }
    };
    let shown_ids = if shown_item.is_some() { ids } else { leaf_targets(s, &ids)? };
    if let Some(shown) = shown_ids.first().and_then(|id| s.doc().ok()?.doc.node(*id).map(|n| proxy_paint(n, stroke, shown_item))) {
        s.remember_paint(&shown);
    }
    Ok(r)
}

fn transparency(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.set";
    let ids = super::opacitymask::transparency_targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(C, "select objects or give ids"));
    }
    // While an opacity mask is edited the selection is its art, so the Appearance panel's active
    // item (a row of that art) doesn't stand for the masked object.
    let editing_mask = p.get("ids").is_none() && p.get("id").is_none() && s.doc()?.doc.mask_edit.is_some();
    let item = if editing_mask && p.get("item").is_none() { ItemTarget::Top } else { item_target(s, p, C)? };
    let mut q = p.as_object().cloned().unwrap_or_default();
    q.remove("item");
    q.remove("id");
    q.insert("ids".into(), json!(ids.iter().map(|id| id.0).collect::<Vec<_>>()));
    let ItemTarget::Item { index, explicit } = item else {
        return s.execute("object.setProps", &Value::Object(q));
    };
    // Opacity and blend belong to the targeted fill/stroke; isolate/knockout/knockoutShape stay object-level.
    let item_ids = if editing_mask { ids } else { appearance_targets(s, p)? };
    if explicit {
        let d = &s.doc()?.doc;
        if !item_ids.iter().any(|id| d.node(*id).is_some_and(|n| index < n.appearance.items.len())) {
            return Err(bad(C, format!("no appearance item {index}")));
        }
    }
    let (opacity, blend) = (q.remove("opacity"), q.remove("blend"));
    if let Some(b) = &blend
        && b.as_str().and_then(BlendMode::parse).is_none()
    {
        return Err(bad(C, format!("unknown blend mode {b}")));
    }
    if let Some(k) = q.get("knockout")
        && vectorcraft_doc::Knockout::from_value(k).is_none()
    {
        return Err(bad(C, format!("unknown knockout state {k}")));
    }
    if opacity.is_some() || blend.is_some() {
        let ids: Vec<u64> = item_ids.iter().map(|id| id.0).collect();
        s.execute("appearance.setItem", &json!({ "index": index, "ids": ids, "opacity": opacity, "blend": blend }))?;
    }
    if q.keys().all(|k| k == "ids") {
        return ok();
    }
    s.execute("object.setProps", &Value::Object(q))
}
