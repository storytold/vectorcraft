//! The command registry. Ids follow Illustrator's menu structure.

pub(crate) mod appearance;
mod assets;
mod attributes;
mod brushsym;
mod buildcmds;
pub mod clipboard;
mod colorcmds;
pub mod colormgmt;
pub mod colortheme;
mod create;
mod css;
mod cut;
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
pub mod flatten;
mod fonts;
pub(crate) mod freeform;
pub(crate) mod gradient;
pub(crate) mod graph;
mod halftone;
pub mod help;
mod layer;
pub mod links;
mod live;
pub(crate) mod maskedit;
pub(crate) mod menucmds;
pub(crate) mod newart;
pub mod newdoc;
mod object;
mod opacitymask;
mod overprint;
mod package;
mod paint;
mod panelcmds;
mod path;
mod pathops;
mod patterncmds;
pub mod pdfcmds;
pub mod perspgrid;
mod place;
pub mod plugin;
pub mod prefscmds;
pub mod print;
pub(crate) mod printadvanced;
pub mod printpresets;
pub mod printtiling;
pub mod rasterfx;
mod recolor;
pub mod recovery;
pub mod registration;
mod select;
pub(crate) mod slices;
mod stroke;
mod style;
pub mod stylelib;
mod swatch;
pub mod swatchlib;
pub(crate) mod tabs;
mod textedit;
pub mod textstyles;
pub(crate) mod textwrap;
pub(crate) mod threads;
pub(crate) mod typecmd;
mod typemenu;
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
pub fn has_multi(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.len() < 2 { Err("select at least two objects".into()) } else { Ok(()) }
}
pub fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.undo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}
pub fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.redo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
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
        v.extend(object::specs());
        v.extend(path::specs());
        v.extend(select::specs());
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
        v.extend(textstyles::specs());
        v.extend(fonts::specs());
        v.extend(help::specs());
        v.extend(threads::specs());
        v.extend(textwrap::specs());
        v.extend(graph::specs());
        v.extend(rasterfx::specs());
        v.extend(views::specs());
        v.extend(maskedit::specs());
        v.extend(tabs::specs());
        v.extend(docmenu::specs());
        v.extend(docinfo::specs());
        v.extend(panelcmds::specs());
        v.extend(buildcmds::specs());
        v.extend(brushsym::specs());
        v.extend(patterncmds::specs());
        v.extend(prefscmds::specs());
        v.extend(distortcmds::specs());
        v.extend(paint::proxy_specs());
        v.extend(overprint::specs());
        v.extend(swatchlib::specs());
        v.extend(freeform::specs());
        v.extend(flatten::specs());
        v.extend(stylelib::specs());
        v.extend(expand::specs());
        v.extend(attributes::specs());
        v.extend(newart::specs());
        v.extend(colortheme::specs());
        v.extend(fileio::pdf::specs());
        v.extend(docsetup::specs());
        v.extend(newdoc::specs());
        v.extend(place::specs());
        v.extend(links::specs());
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
        v.extend(cut::specs());
        v.extend(halftone::specs());
        v.extend(perspgrid::specs());
        v.extend(registration::specs());
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
pub(crate) fn bool_or(p: &Value, key: &str, default: bool) -> bool {
    p.get(key).and_then(Value::as_bool).unwrap_or(default)
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
/// `{"c":..,"m":..,"y":..,"k":..}`, `{"gray":..}` or `{"l":..,"a":..,"b":..}` (CIE Lab: L 0..100, a and b about
/// −128..127).
pub fn color_value(v: &Value) -> Option<Color> {
    match v {
        // A hex, or a colour key ("cmyk 0 100 100 0", as Recolor Artwork lists colours).
        Value::String(s) => Color::from_hex(s).or_else(|| vectorcraft_color::recolor::ColorKey::parse(s).map(|k| k.color())),
        Value::Array(a) if a.len() >= 3 => Some(Color::rgb(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32)),
        Value::Object(o) => {
            if let (Some(c), Some(m), Some(y), Some(k)) = (o.get("c"), o.get("m"), o.get("y"), o.get("k")) {
                let f = |v: &Value| v.as_f64().map(|x| if x > 1.0 { x / 100.0 } else { x } as f32);
                return Some(Color::cmyk(f(c)?, f(m)?, f(y)?, f(k)?));
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
    if let Some(ids) = ids_param(p, "ids") {
        return Ok(ids);
    }
    if let Some(id) = id_param(p, "id") {
        return Ok(vec![id]);
    }
    Ok(s.doc()?.selection.objects.clone())
}

pub(crate) fn ok() -> Result<Value> {
    Ok(Value::Null)
}

/// Leaves whose appearance a paint command changes: groups and layers expand to their contents,
/// and compound paths own their children's appearance.
pub(crate) fn leaf_targets(s: &Session, ids: &[NodeId]) -> Result<Vec<NodeId>> {
    use vectorcraft_doc::NodeKind;
    let d = &s.doc()?.doc;
    let mut out = vec![];
    for id in ids {
        let Some(n) = d.node(*id) else { continue };
        match &n.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } => n.walk(&mut |c| {
                if !c.is_container() || matches!(c.kind, NodeKind::Compound { .. }) {
                    out.push(c.id)
                }
            }),
            _ => out.push(*id),
        }
    }
    let comp: Vec<NodeId> = out.iter().filter(|id| matches!(d.node(**id).map(|n| &n.kind), Some(NodeKind::Compound { .. }))).copied().collect();
    out.retain(|id| !comp.iter().any(|c| d.parent_of(*id) == Some(*c)));
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
    let t = given.unwrap_or_else(vectorcraft_doc::metadata::now_unix);
    s.note_journal(key, serde_json::json!(t));
    t
}

/// The Transparency panel's state ([`Session::transparency_info`]).
pub use opacitymask::TransparencyInfo;

/// Eyedropper Options (`Prefs::eyedropper`).
pub use xform::{AppearanceAttrs, EyedropperAttrs, EyedropperOptions, FillAttrs, StrokeAttrs};

/// The Attributes panel's state ([`Session::attributes_info`]).
pub use overprint::AttributesInfo;

/// Object → Flatten Transparency settings (`object.flattenTransparency`).
pub use flatten::FlattenOptions;

/// A built-in or saved flattener preset ([`crate::Prefs::flattener_presets`]).
pub use flatten::FlattenerPreset;

/// Command ids kept only so older scripts keep working: `(alias, the command it duplicates)`.
/// They run as before but the command palette leaves them out (it lists the command once).
pub const ALIASES: &[(&str, &str)] = &[("object.convertDocumentColorMode", "file.documentColorMode")];

/// Is `id` an [`ALIASES`] entry?
pub fn is_alias(id: &str) -> bool {
    ALIASES.iter().any(|(a, _)| *a == id)
}
