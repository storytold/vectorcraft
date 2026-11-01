//! The Gradient panel's presets: ready-made gradients grouped by theme, applied to the gradient
//! behind the active proxy in one click, and the user's own gradients saved beside them.
//!
//! The built-in presets are the gradient swatches of the built-in swatch libraries
//! ([`vectorcraft_color::libraries`]: the app's own computed palettes), each listed under its
//! library's name. Saved presets are kept with the preferences ([`crate::Prefs::gradient_presets`]),
//! so every document has them. A preset is the colour ramp (type, stops, method, dither), not a
//! placement: applying one edits the gradient in place, as `paint.editGradient` does.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::libraries::{GRADIENT_LIBRARIES, SWATCH_LIBRARIES, builtin_library};
use vectorcraft_color::{Gradient, GradientPaint, Paint};
use vectorcraft_tools::params::{gradient_params, stops_json};

use super::appearance::{edits_stroke, item_target};
use super::gradient::active_gradient;
use super::{CommandSpec, Result, Session, always, bad, cmd, has_doc, text_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "gradient.presets.list",
            "Gradient Presets",
            [],
            None,
            "{} → {presets: [{name, library: the built-in library (theme) it comes from, null for a saved one, builtIn, gradient: {kind, stops, method?, dither?} (as paint.setFill's `gradient`)}]} the Gradient panel's presets: the built-in ones by library, then the saved ones; names are unique",
            always,
            list
        ),
        cmd!(
            "gradient.presets.apply",
            "Apply Gradient Preset",
            [],
            None,
            "{name: a preset's name (gradient.presets.list), stroke?, item?, ids?|id?, target? (as paint.editGradient)} give the gradient behind the active proxy (or the targeted fill or stroke) the preset's type, stops and method, and a fill its dither, keeping its placement; a solid or no paint becomes the gradient. One undo step, as paint.editGradient → {name, ids: the objects painted (none: only the default changed)}",
            has_doc,
            apply
        ),
        cmd!(
            query "gradient.presets.save",
            "Save Gradient Preset",
            [],
            None,
            "{name?: (default: a new \"Gradient Preset N\"; unique, any case, and not a built-in preset's)} save the gradient behind the active proxy (paint.toggleActive picks Fill or Stroke) as a preset, kept with the preferences: its type, stops, method and dither (stops linked to a swatch keep their colour, unlinked) → {name}",
            can_save,
            save
        ),
        cmd!(
            query "gradient.presets.delete",
            "Delete Gradient Preset",
            [],
            None,
            "{name?: a saved preset (default: the saved preset the active proxy's gradient is)} remove a saved preset; built-in presets can't be deleted, and painted art keeps its gradient → {deleted}",
            has_saved_presets,
            delete
        ),
    ]
}

/// A gradient saved to the Gradient panel's presets ([`crate::Prefs::gradient_presets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientPreset {
    pub name: String,
    pub gradient: Gradient,
}

/// A row of the Presets list: a built-in preset (with its library) or a saved one.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientPresetEntry {
    pub name: String,
    /// The built-in library (theme) it comes from; `None` for a saved preset.
    pub library: Option<&'static str>,
    pub gradient: Gradient,
}

/// The built-in presets: every built-in library's gradient swatches, the gradient libraries first.
fn builtin_presets() -> impl Iterator<Item = GradientPresetEntry> {
    GRADIENT_LIBRARIES.iter().chain(SWATCH_LIBRARIES).flat_map(|b| {
        let lib = builtin_library(b.id);
        let gradients: Vec<GradientPresetEntry> = lib
            .iter()
            .flat_map(|lib| lib.iter())
            .filter_map(|w| match &w.paint {
                Paint::Gradient(g) => Some(GradientPresetEntry { name: w.name.clone(), library: Some(b.name), gradient: g.gradient.clone() }),
                _ => None,
            })
            .collect();
        gradients
    })
}

/// Do `a` and `b` have the same colour ramp (type, method, dither and stops), swatch links aside?
fn same_ramp(a: &Gradient, b: &Gradient) -> bool {
    let stop = |x: &vectorcraft_color::GradientStop, y: &vectorcraft_color::GradientStop| {
        (x.offset, x.color, x.opacity, x.midpoint) == (y.offset, y.color, y.opacity, y.midpoint)
    };
    (a.kind, a.method, a.dither) == (b.kind, b.method, b.dither)
        && a.stops.len() == b.stops.len()
        && a.stops.iter().zip(&b.stops).all(|(x, y)| stop(x, y))
}

/// `g` without links to document swatches: each linked stop keeps the colour it shows.
fn unlinked(g: &Gradient) -> Gradient {
    let mut g = g.clone();
    for s in &mut g.stops {
        s.set_color(s.color, None);
    }
    g
}

impl Session {
    /// The Gradient panel's presets: the built-in ones by library, then the saved ones.
    pub fn gradient_presets(&self) -> Vec<GradientPresetEntry> {
        let saved =
            self.prefs.gradient_presets.iter().map(|p| GradientPresetEntry { name: p.name.clone(), library: None, gradient: p.gradient.clone() });
        builtin_presets().chain(saved).collect()
    }

    /// The preset called `name` (any case).
    pub fn gradient_preset(&self, name: &str) -> Option<GradientPresetEntry> {
        let name = name.trim();
        self.gradient_presets().into_iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// The name Save Gradient Preset suggests: the first free "Gradient Preset N".
    pub fn next_gradient_preset_name(&self) -> String {
        let presets = self.gradient_presets();
        (1..).map(|n| format!("Gradient Preset {n}")).find(|n| !presets.iter().any(|e| e.name.eq_ignore_ascii_case(n))).unwrap_or_default()
    }

    /// The name of the preset `g` is (its colour ramp, swatch links aside): a built-in one first,
    /// then a saved one. Cheap enough for every frame: nothing is copied but the name found.
    pub fn gradient_preset_name(&self, g: &Gradient) -> Option<String> {
        let builtin = GRADIENT_LIBRARIES.iter().chain(SWATCH_LIBRARIES).find_map(|b| {
            let lib = builtin_library(b.id)?;
            lib.iter().find(|w| matches!(&w.paint, Paint::Gradient(x) if same_ramp(&x.gradient, g))).map(|w| w.name.clone())
        });
        builtin.or_else(|| self.prefs.gradient_presets.iter().find(|p| same_ramp(&p.gradient, g)).map(|p| p.name.clone()))
    }

    /// The saved preset the gradient behind the active proxy is (its colour ramp, links aside).
    pub fn active_saved_gradient_preset(&self) -> Option<&str> {
        let g = active_gradient(self)?;
        self.prefs.gradient_presets.iter().find(|p| same_ramp(&p.gradient, &g.gradient)).map(|p| p.name.as_str())
    }
}

fn can_save(s: &Session) -> std::result::Result<(), String> {
    active_gradient(s).map(|_| ()).ok_or_else(|| "the active paint is not a gradient".into())
}

fn has_saved_presets(s: &Session) -> std::result::Result<(), String> {
    if s.prefs.gradient_presets.is_empty() { Err("no saved gradient presets".into()) } else { Ok(()) }
}

/// `p`'s `name`: a string, required.
fn name_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    text_param(p, "name", cmd)?.ok_or_else(|| bad(cmd, "missing `name` (see gradient.presets.list)"))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows: Vec<Value> = s
        .gradient_presets()
        .into_iter()
        .map(|e| json!({"name": e.name, "library": e.library, "builtIn": e.library.is_some(), "gradient": gradient_params(&GradientPaint::new(e.gradient))}))
        .collect();
    Ok(json!({ "presets": rows }))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gradient.presets.apply";
    let name = name_param(p, C)?;
    let preset = s.gradient_preset(name).ok_or_else(|| bad(C, format!("no gradient preset `{name}` (see gradient.presets.list)")))?;
    let stroke = edits_stroke(s, p, item_target(s, p, C)?, !s.fill_active, C)?;
    let g = &preset.gradient;
    let mut edit = p.clone();
    let Some(o) = edit.as_object_mut() else { return Err(bad(C, "params must be an object")) };
    o.remove("name");
    o.insert("stroke".into(), json!(stroke));
    o.insert("kind".into(), json!(g.kind.label().to_lowercase()));
    o.insert("stops".into(), stops_json(&g.stops));
    o.insert("method".into(), json!(g.method.label().to_lowercase()));
    // Dither is a fill's (paint.editGradient refuses it on a stroke).
    if !stroke {
        o.insert("dither".into(), json!(g.dither));
    }
    let out = s.execute("paint.editGradient", &edit)?;
    Ok(json!({"name": preset.name, "ids": out.get("ids").cloned().unwrap_or_else(|| json!([]))}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gradient.presets.save";
    let g = active_gradient(s).ok_or_else(|| bad(C, "the active paint is not a gradient"))?;
    let name = match text_param(p, "name", C)?.map(str::trim) {
        None => s.next_gradient_preset_name(),
        Some("") => return Err(bad(C, "the name is empty")),
        Some(n) if s.gradient_preset(n).is_some() => return Err(bad(C, format!("a gradient preset is already called {n}"))),
        Some(n) => n.to_string(),
    };
    s.prefs.gradient_presets.push(GradientPreset { name: name.clone(), gradient: unlinked(&g.gradient) });
    Ok(json!({ "name": name }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gradient.presets.delete";
    let name = match text_param(p, "name", C)? {
        Some(n) => n.trim().to_string(),
        None => s
            .active_saved_gradient_preset()
            .map(str::to_string)
            .ok_or_else(|| bad(C, "give a name, or paint the active proxy with a saved preset"))?,
    };
    if builtin_presets().any(|e| e.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(C, format!("{name} is built in and can't be deleted")));
    }
    let i = s
        .prefs
        .gradient_presets
        .iter()
        .position(|g| g.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| bad(C, format!("no saved gradient preset called {name}")))?;
    Ok(json!({ "deleted": s.prefs.gradient_presets.remove(i).name }))
}
