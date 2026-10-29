//! The command registry. Ids follow Illustrator's menu structure.

pub(crate) mod appearance;
mod assets;
mod attributes;
pub mod brushlib;
mod brushsym;
mod buildcmds;
pub mod clipboard;
mod colorcmds;
pub mod colormgmt;
pub mod colortheme;
pub(crate) mod create;
mod css;
mod cut;
pub(crate) mod dimension;
pub(crate) mod distortcmds;
mod docinfo;
mod docmenu;
pub(crate) mod docsetup;
mod draw2;
mod edit;
mod effectcmd;
pub mod expand;
pub(crate) mod fileinfo;
pub mod fileio;
pub mod findfiles;
pub mod flatten;
pub mod fontfiles;
mod fonts;
pub(crate) mod freeform;
pub mod globaledit;
pub(crate) mod gradient;
pub(crate) mod graph;
mod halftone;
pub mod help;
pub mod importmatches;
pub(crate) mod inline;
mod interactivity;
mod intertwine;
mod layer;
mod layerpanel;
pub mod library;
pub mod linkfiles;
pub mod links;
mod live;
pub(crate) mod maskedit;
pub(crate) mod menucmds;
pub(crate) mod newart;
pub mod newdoc;
mod object;
pub(crate) mod objects_on_path;
mod opacitymask;
mod overprint;
pub mod package;
mod paint;
mod panelcmds;
pub mod params;
mod path;
pub(crate) mod pathops;
mod patterncmds;
pub mod pdfcmds;
pub mod perspgrid;
pub mod place;
pub mod plugin;
pub mod prefscmds;
pub mod print;
pub(crate) mod printadvanced;
pub mod printpresets;
pub mod printtiling;
pub mod rasterfx;
mod recolor;
pub mod recovery;
pub mod scripts;
mod select;
pub(crate) mod shaper;
pub mod shortcutset;
pub(crate) mod slices;
mod stroke;
mod style;
pub mod stylelib;
mod swatch;
pub mod swatchlib;
pub mod symbollib;
pub(crate) mod tabs;
mod textedit;
pub mod textstyles;
pub(crate) mod textwrap;
pub(crate) mod threads;
pub(crate) mod typecmd;
mod typemenu;
mod variablecmds;
pub(crate) mod views;
pub mod wand;
pub mod webexport;
mod xform;

use serde::Serialize;
use serde_json::Value;
use vectorcraft_color::Color;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Affine, Point};

use crate::{EngineError, Result, Session};

pub type Run = fn(&mut Session, &Value) -> Result<Value>;
pub type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Object", "Arrange"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut (`Cmd+Shift+]`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Human/agent-readable parameter description.
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the journal (false for queries and selection-only helpers).
    pub journal: bool,
}

/// Serializable command metadata.
#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: Vec<&'static str>,
    pub shortcut: Option<&'static str>,
    pub params: &'static str,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl CommandSpec {
    pub fn info(&self, s: &Session) -> CommandInfo {
        let e = (self.enabled)(s);
        CommandInfo {
            id: self.id,
            label: self.label,
            menu: self.menu.to_vec(),
            shortcut: self.shortcut,
            params: self.params,
            enabled: e.is_ok(),
            disabled_reason: e.err(),
        }
    }
}

// ---------- enablement predicates ----------

pub fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
pub fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
pub fn has_selection(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.is_empty() { Err("nothing selected".into()) } else { Ok(()) }
}
/// Objects are selected, or the Artboard tool is chosen (Copy and Cut take its artboard).
pub fn has_selection_or_artboard_tool(s: &Session) -> std::result::Result<(), String> {
    if s.tool_id() == "artboard" { has_doc(s) } else { has_selection(s) }
}
/// Objects or ruler guides are selected (what Delete and the arrow keys act on).
pub fn has_selection_or_guides(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.has_objects_or_guides() { Ok(()) } else { Err("nothing selected".into()) }
}
pub fn has_anchors(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.anchors.values().all(|a| a.is_empty()) { Err("no anchor points selected".into()) } else { Ok(()) }
}
pub fn has_multi(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.len() < 2 { Err("select at least two objects".into()) } else { Ok(()) }
}
/// A document is open and it is not in isolation mode: Object › Lock's and Object › Hide's items,
/// Unlock All and Show All are off in isolation mode. Pattern editing mode and opacity mask
/// editing isolate their art too, and do not count as isolation mode here while their own layer
/// is the isolated one. An object isolated inside them counts.
pub fn outside_isolation(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let editing_layer = |l| st.doc.pattern_edit.as_ref().is_some_and(|e| e.layer == l) || st.doc.mask_edit.as_ref().is_some_and(|e| e.layer == l);
    if st.isolation.is_some_and(|l| !editing_layer(l)) { Err("exit isolation mode first".into()) } else { Ok(()) }
}
/// Objects are selected ([`has_selection`]) and the document is not in isolation mode
/// ([`outside_isolation`]).
pub fn has_selection_outside_isolation(s: &Session) -> std::result::Result<(), String> {
    outside_isolation(s)?;
    has_selection(s)
}
pub fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.undo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}
pub fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.redo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
}
/// The active document has a step to undo or to redo (Clear History).
pub fn can_clear_history(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.undo.is_empty() || !d.history.redo.is_empty()).map(|_| ()).ok_or_else(|| "no history to clear".into())
}
pub fn has_clipboard(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.clipboard.is_empty() { Err("clipboard is empty".into()) } else { Ok(()) }
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
    (query $id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: false }
    };
}
pub(crate) use cmd;

pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(edit::specs());
        v.extend(clipboard::specs());
        v.extend(recolor::specs());
        v.extend(fileio::specs());
        v.extend(create::specs());
        v.extend(dimension::specs());
        v.extend(object::specs());
        v.extend(path::specs());
        v.extend(select::specs());
        v.extend(globaledit::specs());
        v.extend(wand::specs());
        v.extend(paint::specs());
        // Split out of `paint` (M3.8); kept next to it so lists in registry order (the command
        // palette, `engine.commands`) show them where they always were.
        v.extend(stroke::specs());
        v.extend(appearance::specs());
        v.extend(style::specs());
        v.extend(swatch::specs());
        v.extend(gradient::specs());
        v.extend(opacitymask::specs());
        v.extend(layer::specs());
        v.extend(layerpanel::specs());
        v.extend(draw2::specs());
        v.extend(xform::specs());
        v.extend(effectcmd::specs());
        v.extend(pathops::specs());
        v.extend(typecmd::specs());
        v.extend(menucmds::specs());
        v.extend(live::specs());
        v.extend(colorcmds::specs());
        v.extend(colormgmt::specs());
        v.extend(typemenu::specs());
        v.extend(textedit::specs());
        v.extend(inline::specs());
        v.extend(textstyles::specs());
        v.extend(fonts::specs());
        v.extend(fontfiles::specs());
        v.extend(help::specs());
        v.extend(threads::specs());
        v.extend(textwrap::specs());
        v.extend(graph::specs());
        v.extend(rasterfx::specs());
        v.extend(views::specs());
        v.extend(maskedit::specs());
        v.extend(tabs::specs());
        v.extend(docmenu::specs());
        v.extend(variablecmds::specs());
        v.extend(docinfo::specs());
        v.extend(panelcmds::specs());
        v.extend(buildcmds::specs());
        v.extend(shaper::specs());
        v.extend(brushsym::specs());
        v.extend(patterncmds::specs());
        v.extend(intertwine::specs());
        v.extend(objects_on_path::specs());
        v.extend(prefscmds::specs());
        v.extend(distortcmds::specs());
        v.extend(paint::proxy_specs());
        v.extend(overprint::specs());
        v.extend(swatchlib::specs());
        v.extend(freeform::specs());
        v.extend(flatten::specs());
        v.extend(stylelib::specs());
        v.extend(symbollib::specs());
        v.extend(brushlib::specs());
        v.extend(library::specs());
        v.extend(expand::specs());
        v.extend(attributes::specs());
        v.extend(interactivity::specs());
        v.extend(newart::specs());
        v.extend(colortheme::specs());
        v.extend(fileio::pdf::specs());
        v.extend(docsetup::specs());
        v.extend(newdoc::specs());
        v.extend(place::specs());
        v.extend(links::specs());
        v.extend(linkfiles::specs());
        v.extend(pdfcmds::specs());
        v.extend(fileio::dxf::specs());
        v.extend(fileio::eps::specs());
        v.extend(package::specs());
        v.extend(slices::specs());
        v.extend(print::specs());
        v.extend(recovery::specs());
        v.extend(fileio::dxfimport::specs());
        v.extend(webexport::specs());
        v.extend(assets::specs());
        v.extend(css::specs());
        v.extend(printpresets::specs());
        v.extend(printtiling::specs());
        v.extend(plugin::specs());
        v.extend(scripts::specs());
        v.extend(cut::specs());
        v.extend(halftone::specs());
        v.extend(perspgrid::specs());
        v.extend(shortcutset::specs());
        v.extend(importmatches::specs());
        v
    })
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    command_specs().iter().find(|c| c.id == id)
}

// ---------- param helpers ----------

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
pub(crate) fn f64_or(p: &Value, key: &str, default: f64) -> f64 {
    p.get(key).and_then(Value::as_f64).unwrap_or(default)
}
pub(crate) fn f64_req(p: &Value, key: &str, cmd: &str) -> Result<f64> {
    p.get(key).and_then(Value::as_f64).ok_or_else(|| bad(cmd, format!("missing number `{key}`")))
}
/// The number `p` gives for `key`, or `default` when `p` leaves it out or gives null. Any other
/// value, such as `"40"`, is an error: "`{key}` must be a number, not {v}".
pub(crate) fn number_or(p: &Value, key: &str, default: f64, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().ok_or_else(|| bad(cmd, format!("`{key}` must be a number, not {v}"))),
    }
}
/// Error unless each of `keys` that `p` gives is a number (or null), as [`number_or`] reads them: an
/// optional number read with [`f64_or`] would otherwise fall back to its default for a string such as
/// `"60 mm"` (#1952).
pub(crate) fn check_numbers(p: &Value, keys: &[&str], cmd: &str) -> Result<()> {
    for key in keys {
        number_or(p, key, 0.0, cmd)?;
    }
    Ok(())
}
pub(crate) fn bool_or(p: &Value, key: &str, default: bool) -> bool {
    p.get(key).and_then(Value::as_bool).unwrap_or(default)
}
/// The true or false `p` gives for `key`, or `default` when `p` leaves it out or gives null. Any
/// other value, such as `"false"`, is an error: "`{key}` must be true or false".
pub(crate) fn switch_or(p: &Value, key: &str, default: bool, cmd: &str) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))),
    }
}
/// Error unless each of `keys` that `p` gives is true or false (or null), as [`switch_or`] reads
/// them.
pub(crate) fn check_switches(p: &Value, keys: &[&str], cmd: &str) -> Result<()> {
    for key in keys {
        switch_or(p, key, false, cmd)?;
    }
    Ok(())
}
pub(crate) fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}
pub(crate) fn id_param(p: &Value, key: &str) -> Option<NodeId> {
    p.get(key).and_then(Value::as_u64).map(NodeId)
}
pub(crate) fn ids_param(p: &Value, key: &str) -> Option<Vec<NodeId>> {
    p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(NodeId).collect())
}
/// The objects `p` gives `command`: its `ids`, else its `id` as `ids` of one (as [`targets`] reads
/// them), each checked ([`checked_id`]): a malformed one, or one naming no object, is an error,
/// never the selection. `None` when it gives neither (or gives them as `null`).
pub(crate) fn given_objects(s: &Session, p: &Value, command: &str) -> Result<Option<Vec<NodeId>>> {
    if p.get("ids").is_some_and(|v| !v.is_null()) {
        return checked_ids_param(s, p, "ids", command).map(Some);
    }
    match p.get("id").filter(|v| !v.is_null()) {
        None => Ok(None),
        // A step reference a batch couldn't resolve, as `targets` reports it, under the command.
        Some(Value::String(r)) if r.starts_with('$') => {
            Err(bad(command, format!("`id` is the step reference {r:?}, which only `run` and `command_batch` resolve")))
        }
        Some(v) => Ok(Some(vec![checked_id(s, v, command)?])),
    }
}
/// `v` as an index, a count or a byte offset: past `usize::MAX` reads as `usize::MAX`, past the end of any list.
pub(crate) fn index(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}
/// Validate all explicit targets before a command changes the document or selection.
pub(crate) fn checked_ids_param(s: &Session, p: &Value, key: &str, command: &str) -> Result<Vec<NodeId>> {
    let values =
        p.get(key).and_then(Value::as_array).ok_or_else(|| bad(command, format!("{key} must be an array of non-negative integer object ids")))?;
    values.iter().map(|value| checked_id(s, value, command)).collect()
}

/// The objects a command acts on: `ids` when given (every one checked, and those inside another
/// given one dropped), else the selected objects.
pub(crate) fn checked_ids_or_selection(s: &Session, p: &Value, command: &str) -> Result<Vec<NodeId>> {
    if p.get("ids").is_some() { Ok(edit::outermost(&s.doc()?.doc, checked_ids_param(s, p, "ids", command)?)) } else { edit::selected_roots(s) }
}

pub(crate) fn checked_id(s: &Session, value: &Value, command: &str) -> Result<NodeId> {
    let id = value.as_u64().map(NodeId).ok_or_else(|| bad(command, format!("invalid object id {value}: expected a non-negative integer")))?;
    if s.doc()?.doc.node(id).is_none() {
        return Err(bad(command, format!("no such object id {value}")));
    }
    Ok(id)
}

/// `v` as exactly `N` finite numbers (`[x, y]`, `[x, y, width, height]`…): `None` when it isn't an
/// array of that length or an entry isn't a number, so nothing malformed is left out or padded.
pub(crate) fn finite_numbers<const N: usize>(v: &Value) -> Option<[f64; N]> {
    let a = v.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (o, n) in out.iter_mut().zip(a) {
        *o = n.as_f64().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// `v` as a list of point indices: `None` unless it is an array of non-negative integers, so no
/// malformed index is dropped or wrapped round.
pub(crate) fn point_indices(v: &Value) -> Option<Vec<usize>> {
    v.as_array()?.iter().map(|i| usize::try_from(i.as_u64()?).ok()).collect()
}

pub(crate) fn point_param(p: &Value, key: &str) -> Option<Point> {
    let a = p.get(key)?.as_array()?;
    Some(Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}
/// `[a, b, c, d, e, f]` affine coefficients.
pub fn matrix_param(p: &Value, key: &str) -> Option<Affine> {
    let a = p.get(key)?.as_array()?;
    if a.len() != 6 {
        return None;
    }
    let mut c = [0.0; 6];
    for (i, v) in a.iter().enumerate() {
        c[i] = v.as_f64()?;
    }
    Some(Affine::new(c))
}
/// A colour from `"#rrggbb"`, a colour key (`"cmyk 0 100 100 0"`, `"lab 55 60 40"`), `[r,g,b]` (0..1),
/// `{"c":..,"m":..,"y":..,"k":..}` (percent when any component is above 1, else 0..1), `{"gray":..}` or `{"l":..,"a":..,"b":..}` (CIE Lab: L 0..100, a and b about
/// −128..127).
pub fn color_value(v: &Value) -> Option<Color> {
    match v {
        // A hex, or a colour key ("cmyk 0 100 100 0", as Recolor Artwork lists colours).
        Value::String(s) => Color::from_hex(s).or_else(|| vectorcraft_color::recolor::ColorKey::parse(s).map(|k| k.color())),
        Value::Array(a) if a.len() >= 3 => Some(Color::rgb(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32)),
        Value::Object(o) => {
            if let (Some(c), Some(m), Some(y), Some(k)) = (o.get("c"), o.get("m"), o.get("y"), o.get("k")) {
                // One scale for the whole colour: percent when any component is above 1, else 0..1.
                // Deciding per component read {c: 18, k: 0.79} as K 79 %.
                let v = [c.as_f64()?, m.as_f64()?, y.as_f64()?, k.as_f64()?];
                let scale = if v.iter().any(|x| *x > 1.0) { 100.0 } else { 1.0 };
                let f = |x: f64| (x / scale) as f32;
                return Some(Color::cmyk(f(v[0]), f(v[1]), f(v[2]), f(v[3])));
            }
            if let Some(g) = o.get("gray") {
                let g = g.as_f64()?;
                return Some(Color::gray(if g > 1.0 { g / 100.0 } else { g } as f32));
            }
            if let (Some(l), Some(a), Some(b), None) = (o.get("l"), o.get("a"), o.get("b"), o.get("model")) {
                return Some(Color::lab(l.as_f64()? as f32, a.as_f64()? as f32, b.as_f64()? as f32));
            }
            serde_json::from_value(v.clone()).ok()
        }
        _ => None,
    }
}

/// Objects a command targets: explicit `ids` param or the selection.
pub(crate) fn targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    // Objects given are used as given: a value that isn't an object id fails rather than the
    // command acting on the selection instead (#785).
    let object_id = |v: &Value| v.as_u64().map(NodeId).ok_or_else(|| EngineError::Other(format!("object ids are non-negative integers, not {v}")));
    match p.get("ids").filter(|v| !v.is_null()) {
        Some(Value::Array(a)) => return a.iter().map(object_id).collect(),
        Some(v) => return Err(EngineError::Other(format!("`ids` must be an array of object ids, not {v}"))),
        None => {}
    }
    match p.get("id").filter(|v| !v.is_null()) {
        Some(v @ Value::Number(_)) => Ok(vec![object_id(v)?]),
        // A step reference a batch couldn't resolve (other strings are other ids, such as an
        // effect's).
        Some(Value::String(r)) if r.starts_with('$') => {
            Err(EngineError::Other(format!("`id` is the step reference {r:?}, which only `run` and `command_batch` resolve")))
        }
        _ => Ok(s.doc()?.selection.objects.clone()),
    }
}

pub(crate) fn ok() -> Result<Value> {
    Ok(Value::Null)
}

/// Leaves whose appearance a paint command changes: groups and layers expand to their contents,
/// and compound paths own their children's appearance, so a member stands for its compound path
/// (#3692), which is listed once.
pub(crate) fn leaf_targets(s: &Session, ids: &[NodeId]) -> Result<Vec<NodeId>> {
    use vectorcraft_doc::NodeKind;
    let d = &s.doc()?.doc;
    let compound = |id: NodeId| d.parent_of(id).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. })));
    let mut out = vec![];
    for id in ids {
        let Some(n) = d.node(*id) else { continue };
        match &n.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } => n.walk(&mut |c| {
                if !c.is_container() || matches!(c.kind, NodeKind::Compound { .. }) {
                    out.push(c.id)
                }
            }),
            _ => out.push(compound(*id).unwrap_or(*id)),
        }
    }
    let comp: Vec<NodeId> = out.iter().filter(|id| matches!(d.node(**id).map(|n| &n.kind), Some(NodeKind::Compound { .. }))).copied().collect();
    out.retain(|id| !comp.iter().any(|c| d.parent_of(*id) == Some(*c)));
    let mut seen = std::collections::HashSet::new();
    out.retain(|id| seen.insert(*id));
    Ok(out)
}

/// [`leaf_targets`] of a command's [`targets`] (`ids`, `id` or the selection).
pub(crate) fn paint_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    leaf_targets(s, &targets(s, p)?)
}

/// `base` if `taken` doesn't claim it, else the first free "base 2", "base 3", …
pub(crate) fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

pub(crate) use edit::{settle_interaction, typing_in_progress};
pub(crate) use object::bounds_rewrite;
pub(crate) use vectorcraft_doc::range::parse_range;

/// A date param in Unix seconds that defaults to the clock: `None` when absent (now, see
/// [`clock_date`]), `Some(None)` for null (no date), else `Some(Some(seconds))`.
pub(crate) fn date_param(p: &Value, key: &str, cmd: &str) -> Result<Option<Option<i64>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v.as_i64().map(|t| Some(Some(t))).ok_or_else(|| bad(cmd, format!("{key} must be Unix seconds (an integer) or null"))),
    }
}

/// The date a command stamps: `given` (see [`date_param`]), else now (none on the web, which has
/// no clock). It joins the running command's journal entry as `key`, so a replay of the journal
/// gives the same date whenever it runs (an action leaves it out: [`crate::Session::journal_for_action`]).
pub(crate) fn clock_date(s: &mut Session, key: &str, given: Option<Option<i64>>) -> Option<i64> {
    let t = given.unwrap_or_else(vectorcraft_doc::metadata::output_time);
    s.note_journal(key, serde_json::json!(t));
    t
}

/// The Transparency panel's state ([`Session::transparency_info`]).
pub use opacitymask::TransparencyInfo;

/// Eyedropper Options (`Prefs::eyedropper`).
pub use xform::{AppearanceAttrs, EyedropperAttrs, EyedropperOptions, FillAttrs, StrokeAttrs};

/// Pathfinder Options (`Prefs::pathfinder`).
pub use pathops::{PATHFINDER_PRECISION_RANGE, PathfinderOptions};

/// The Attributes panel's state ([`Session::attributes_info`]).
pub use overprint::AttributesInfo;

/// Object → Flatten Transparency settings (`object.flattenTransparency`).
pub use flatten::FlattenOptions;

/// A built-in or saved flattener preset ([`crate::Prefs::flattener_presets`]).
pub use flatten::FlattenerPreset;

/// A saved Image Trace preset ([`crate::Prefs::trace_presets`]), and a change to the saved presets.
pub use buildcmds::{TracePreset, TracePresetChange};

/// The saved Image Trace presets' revision and latest changes ([`crate::Session`] keeps them).
pub(crate) use buildcmds::TracePresetChanges;

/// The shapes a group selected alone gives the Pathfinder panel.
pub use pathops::shape_count;

/// The gaps View › Show Live Paint Gaps shows on a Live Paint group.
pub use buildcmds::live_paint_gaps;

/// Command ids kept only so older scripts keep working: `(alias, the command it duplicates)`.
/// They run as before but the command palette leaves them out (it lists the command once).
pub const ALIASES: &[(&str, &str)] = &[("object.convertDocumentColorMode", "file.documentColorMode")];

/// Is `id` an [`ALIASES`] entry?
pub fn is_alias(id: &str) -> bool {
    ALIASES.iter().any(|(a, _)| *a == id)
}
