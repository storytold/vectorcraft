//! Colour management: Edit → Color Settings / Assign Profile, document colour mode conversion
//! through the CMS, gamut checks, View → Proof Setup / Proof Colors / Overprint Preview, the
//! Separations Preview panel, Overprint Black and spot swatches.
//!
//! Colour settings and the proof view are process-wide (`vectorcraft_color::cms::active`,
//! `vectorcraft_render::proof::view`). A document's assigned profiles are stored in
//! `Document::color_profiles` (files before format v3: `Document.unknown["colorProfiles"]`) and
//! become active when assigned.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ColorSettings, Intent, Model, ProofTarget};
use vectorcraft_color::recolor::Neutral;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;
use vectorcraft_render::proof;

use super::*;

/// `Document.unknown` key under which files before format v3 kept the assigned profiles (migrated
/// to `Document::color_profiles` on load).
pub const PROFILES_KEY: &str = vectorcraft_doc::profiles::LEGACY_PROFILES_KEY;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.colorSettings",
            "Color Settings…",
            ["Edit"],
            Some("Cmd+Shift+K"),
            "{rgb?: profile, cmyk?: profile, intent?: \"perceptual\"|\"relative\"|\"saturation\"|\"absolute\", bpc?: bool} set the working spaces → {rgb, cmyk, intent, bpc, profiles: [{name, kind, builtin}]}",
            always,
            color_settings
        ),
        cmd!(
            "color.loadProfile",
            "Load Color Profile…",
            [],
            None,
            "{path: .icc/.icm file} register an ICC profile (native only) → {name, kind}",
            always,
            load_profile
        ),
        cmd!(
            "edit.assignProfile",
            "Assign Profile…",
            ["Edit"],
            None,
            "{rgb?: profile|null, cmyk?: profile|null} tag the document with profiles (colour numbers are kept) → {rgb, cmyk}",
            has_doc,
            assign_profile
        ),
        cmd!(
            "object.convertDocumentColorMode",
            "Convert Document Color Mode",
            [],
            None,
            "same as file.documentColorMode: {mode: \"cmyk\"|\"rgb\", convert?: true, intent?, grays?} → {changed} (an alias kept for older scripts; the command palette leaves it out)",
            has_doc,
            convert_mode
        ),
        cmd!(
            query "color.convert",
            "Convert Color",
            [],
            None,
            "{color, to: \"rgb\"|\"cmyk\"|\"gray\"|\"lab\", intent?} → {model, values, hex, lab: [L,a,b], outOfGamut, deltaE}",
            always,
            convert_color
        ),
        cmd!(
            query "color.gamutCheck",
            "Gamut Warning",
            [],
            None,
            "{ids?, colors?: [color]} colours that can't be printed in the working CMYK (selection, or the whole document when nothing is selected) → {checked, outOfGamut: [{hex, deltaE, count}]}",
            has_doc,
            gamut_check
        ),
        cmd!(
            query "view.proofSetup",
            "Proof Setup",
            ["View", "Proof Setup"],
            None,
            "{target?: \"workingCmyk\"|\"cmyk:<profile>\"|\"legacyMacRgb\"|\"srgb\"|\"monitorRgb\"|\"protanopia\"|\"deuteranopia\", intent?, simulatePaper?, proof?: bool (also turn Proof Colors on)} → proof state",
            always,
            proof_setup
        ),
        cmd!(query "view.proofColors", "Proof Colors", ["View"], None, "{on?: bool (default: toggle)} → proof state", always, |s, p| {
            toggle_view(s, p, |v, on| v.proof_colors = on.unwrap_or(!v.proof_colors))
        }),
        cmd!(
            query "view.overprintPreview",
            "Overprint Preview",
            ["View"],
            Some("Cmd+Alt+Shift+Y"),
            "{on?: bool (default: toggle)} → proof state",
            always,
            |s, p| toggle_view(s, p, |v, on| v.overprint = on.unwrap_or(!v.overprint))
        ),
        cmd!(
            query "view.separationsPreview",
            "Separations Preview",
            [],
            None,
            "{on?: bool, plates?: [names], toggle?: plate, only?: plate} Separations Preview: show the chosen plates (one plate = greyscale ink coverage) → {on, plates: [{name, spot, visible, rgb}]}",
            always,
            separations
        ),
        cmd!(query "color.plates", "Plates", [], None, "{} → {on, plates: [{name, spot, visible, rgb}]}", always, |s, _| Ok(plates_json(s))),
        cmd!(
            "edit.colors.overprintBlack",
            "Overprint Black…",
            ["Edit", "Edit Colors"],
            None,
            "{remove?: false, percentage?: 100, fill?: true, stroke?: true, includeCmyBlacks?: false, includeSpotBlacks?: false, ids?} make the black fills and/or strokes of the selection (or ids; groups: their contents; type: its characters) overprint, or stop (remove). Black: K ≥ percentage with no C, M or Y (any with includeCmyBlacks), not linked to a spot swatch (unless includeSpotBlacks); a gradient is black when every stop is → {changed: objects}",
            has_doc,
            super::overprint::overprint_black
        ),
        cmd!(
            "swatch.setSpot",
            "Spot Color",
            [],
            None,
            "{name, spot?: true} make a swatch a spot colour (prints on its own plate; spot swatches are global) → {name, spot}",
            has_doc,
            set_spot
        ),
    ]
}

fn intent_param(p: &Value, cmd: &str) -> Result<Option<Intent>> {
    match str_param(p, "intent") {
        None => Ok(None),
        Some(i) => Intent::parse(i).map(Some).ok_or_else(|| bad(cmd, format!("unknown intent `{i}`"))),
    }
}

fn settings_json(st: &ColorSettings) -> Value {
    json!({
        "rgb": st.rgb,
        "cmyk": st.cmyk,
        "intent": st.intent.id(),
        "bpc": st.bpc,
        "profiles": cms::profiles().iter().map(|p| json!({"name": p.name, "kind": p.kind, "builtin": p.builtin})).collect::<Vec<_>>(),
    })
}

/// Colour settings/proof changes don't edit the document, but the canvas must redraw: bump the
/// revisions (which never mark documents dirty).
fn touch_all(s: &mut Session) {
    for d in &mut s.docs {
        d.revision += 1;
    }
}

fn color_settings(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.colorSettings";
    let mut st = cms::active_settings();
    let before = st.clone();
    if let Some(v) = str_param(p, "rgb") {
        st.rgb = cms::canonical_name(v).to_string();
    }
    if let Some(v) = str_param(p, "cmyk") {
        st.cmyk = cms::canonical_name(v).to_string();
    }
    if let Some(i) = intent_param(p, C)? {
        st.intent = i;
    }
    st.bpc = bool_or(p, "bpc", st.bpc);
    if st != before {
        cms::set_active(&st).map_err(|e| bad(C, e.to_string()))?;
        touch_all(s);
    }
    Ok(settings_json(&st))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_profile(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.loadProfile";
    let path = str_param(p, "path").ok_or_else(|| bad(C, "missing `path`"))?;
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    let info = cms::load_icc_file(std::path::Path::new(path)).map_err(|e| bad(C, e.to_string()))?;
    Ok(json!({"name": info.name, "kind": info.kind}))
}

#[cfg(target_arch = "wasm32")]
fn load_profile(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("color.loadProfile", "loading profiles from disk isn't available in the browser"))
}

/// The document's assigned profiles (`None` = working space).
pub fn doc_profiles(d: &vectorcraft_doc::Document) -> (Option<String>, Option<String>) {
    let get = |n: &Option<String>| n.as_deref().map(|n| cms::canonical_name(n).to_string());
    (get(&d.color_profiles.rgb), get(&d.color_profiles.cmyk))
}

fn assign_profile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.assignProfile";
    let (mut rgb, mut cmyk) = doc_profiles(&s.doc()?.doc);
    let pick = |key: &str, cur: &mut Option<String>, kind: cms::ProfileKind| -> Result<bool> {
        match p.get(key) {
            None => Ok(false),
            Some(Value::Null) => Ok(cur.take().is_some()),
            Some(Value::String(n)) => {
                let Some(info) = cms::profile(n).filter(|k| k.kind == kind) else {
                    return Err(bad(C, format!("unknown {key} profile `{n}`")));
                };
                let changed = cur.as_deref() != Some(info.name.as_str());
                *cur = Some(info.name);
                Ok(changed)
            }
            Some(_) => Err(bad(C, format!("`{key}` must be a profile name or null"))),
        }
    };
    let changed = pick("rgb", &mut rgb, cms::ProfileKind::Rgb)? | pick("cmyk", &mut cmyk, cms::ProfileKind::Cmyk)?;
    if changed {
        let profiles = vectorcraft_doc::ColorProfiles { rgb: rgb.clone(), cmyk: cmyk.clone() };
        s.edit("Assign Profile", |d, _| {
            d.color_profiles = profiles;
            Ok(())
        })?;
        let mut st = cms::active_settings();
        if let Some(r) = &rgb {
            st.rgb = r.clone();
        }
        if let Some(c) = &cmyk {
            st.cmyk = c.clone();
        }
        cms::set_active(&st).map_err(|e| bad(C, e.to_string()))?;
        touch_all(s);
    }
    Ok(json!({"rgb": rgb, "cmyk": cmyk}))
}

/// How Document Color Mode converts neutral RGB greys (R = G = B) to CMYK (the `grays` param).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grays {
    /// Through the CMYK profile like any colour: four-colour greys and a rich black.
    #[default]
    Profile,
    /// On the black plate only: K is the grey's ink percentage (as Convert to Grayscale and then
    /// to CMYK would give).
    Black,
}

impl Grays {
    /// The `grays` param of `cmd`: `"profile"` (default) or `"black"`.
    pub(crate) fn param(cmd: &str, p: &Value) -> Result<Self> {
        match str_param(p, "grays").map(str::to_ascii_lowercase).as_deref() {
            None | Some("profile") => Ok(Grays::Profile),
            Some("black") => Ok(Grays::Black),
            Some(g) => Err(bad(cmd, format!("grays must be \"profile\" or \"black\", not `{g}`"))),
        }
    }
}

/// File → Document Color Mode (`file.documentColorMode` and its alias
/// `object.convertDocumentColorMode`): set the mode and, unless `convert: false`, convert every
/// colour through the colour settings.
pub(crate) fn convert_mode(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.documentColorMode";
    let mode = match str_param(p, "mode").map(str::to_ascii_lowercase).as_deref() {
        Some("cmyk") => ColorMode::Cmyk,
        Some("rgb") => ColorMode::Rgb,
        _ => return Err(bad(C, "mode must be \"cmyk\" or \"rgb\"")),
    };
    let intent = intent_param(p, C)?;
    let grays = Grays::param(C, p)?;
    // Choosing the mode the document already has (the checked menu item) is not an edit.
    if s.doc()?.doc.color_mode == mode {
        return Ok(json!({ "changed": 0 }));
    }
    let convert = bool_or(p, "convert", true);
    let mut changed = 0usize;
    s.edit("Document Color Mode", |d, _| {
        changed = set_color_mode(d, mode, convert, intent, grays);
        Ok(())
    })?;
    Ok(json!({ "changed": changed }))
}

/// Put `d` in colour `mode` and, with `convert`, convert every colour of its art, symbols,
/// pattern tiles and swatches through the colour settings (`intent`, default: theirs) → how many
/// colours of the art, symbols and patterns changed. Gray colours stay Gray (they print on the
/// black plate in either mode); RGB greys are colours like any other and separate through the
/// CMYK profile, unless `grays` puts them on the black plate. Lab colours (spot colour
/// definitions) are device independent and fit either mode. Document Color Mode and
/// `document.open {colorMode}` both convert this way.
pub(crate) fn set_color_mode(d: &mut vectorcraft_doc::Document, mode: ColorMode, convert: bool, intent: Option<Intent>, grays: Grays) -> usize {
    d.color_mode = mode;
    if !convert {
        return 0;
    }
    let c = cms::active();
    let intent = intent.unwrap_or(c.settings().intent);
    let model = if mode == ColorMode::Cmyk { Model::Cmyk } else { Model::Rgb };
    let k_only = model == Model::Cmyk && grays == Grays::Black;
    let conv = |col: &Color| match col {
        Color::Gray { .. } | Color::Lab { .. } => *col,
        // Grey ink percentage, then K only (a Gray colour's CMYK).
        Color::Rgb { .. } if k_only && Neutral::of(col).is_some() => c.convert(&c.convert(col, Model::Gray, intent), Model::Cmyk, intent),
        _ => c.convert(col, model, intent),
    };
    let mut changed = 0usize;
    let mut count = |col: &Color, _: proof::Link| {
        let n = conv(col);
        if n != *col {
            changed += 1;
        }
        n
    };
    proof::map_document_colors(d, &mut count);
    for a in d.patterns.iter_mut().flat_map(|p| &mut p.art) {
        proof::map_node_colors(Arc::make_mut(a), &mut count);
    }
    for sw in d.swatches_iter_mut() {
        match &mut sw.paint {
            Paint::Solid { color, .. } => *color = conv(color),
            Paint::Gradient(g) => {
                for st in &mut g.gradient.stops {
                    st.color = conv(&st.color);
                }
            }
            _ => {}
        }
    }
    // Tints are their converted swatch's colour at their tint (exact ink percentages).
    let bases: Vec<(String, Color)> = d.swatches_iter().filter_map(|w| Some((w.name.clone(), d.global_color(&w.name)?))).collect();
    d.map_solid_paints(&mut |c, link, tint| match bases.iter().find(|(n, _)| Some(n) == link.as_ref()) {
        Some((_, base)) if *tint < 1.0 => {
            *c = base.tinted(*tint);
            true
        }
        _ => false,
    });
    changed
}

fn values(c: &Color) -> Vec<f32> {
    match *c {
        Color::Rgb { r, g, b } => vec![r, g, b],
        Color::Cmyk { c, m, y, k } => vec![c, m, y, k],
        Color::Gray { k } => vec![k],
        Color::Lab { l, a, b } => vec![l, a, b],
    }
}

fn convert_color(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.convert";
    let col = p.get("color").and_then(color_value).ok_or_else(|| bad(C, "missing or invalid `color`"))?;
    let c = cms::active();
    let intent = intent_param(p, C)?.unwrap_or(c.settings().intent);
    let lab = c.lab(&col);
    let (model, vals) = match str_param(p, "to").unwrap_or("rgb") {
        "rgb" => ("rgb", values(&c.convert(&col, Model::Rgb, intent))),
        "cmyk" => ("cmyk", values(&c.convert(&col, Model::Cmyk, intent))),
        "gray" | "grayscale" => ("gray", values(&c.convert(&col, Model::Gray, intent))),
        "lab" => ("lab", vec![lab.l, lab.a, lab.b]),
        other => return Err(bad(C, format!("unknown model `{other}`"))),
    };
    let de = c.gamut_error(&col);
    Ok(json!({
        "model": model,
        "values": vals,
        "hex": col.to_hex(),
        "lab": [lab.l, lab.a, lab.b],
        "outOfGamut": de > cms::GAMUT_THRESHOLD,
        "deltaE": de,
    }))
}

fn gamut_check(s: &mut Session, p: &Value) -> Result<Value> {
    let mut cols: Vec<Color> = vec![];
    if let Some(a) = p.get("colors").and_then(Value::as_array) {
        cols.extend(a.iter().filter_map(color_value));
    } else {
        let st = s.doc()?;
        let ids = targets(s, p)?;
        let mut collect = |n: &vectorcraft_doc::Node| {
            let mut n = n.clone();
            proof::map_node_colors(&mut n, &mut |c, _| {
                cols.push(*c);
                *c
            });
        };
        if ids.is_empty() {
            for l in &st.doc.layers {
                collect(l);
            }
        } else {
            for id in ids {
                if let Some(n) = st.doc.node(id) {
                    collect(n);
                }
            }
        }
    }
    let c = cms::active();
    let mut out: BTreeMap<String, (f32, usize)> = BTreeMap::new();
    for col in &cols {
        let de = c.gamut_error(col);
        if de > cms::GAMUT_THRESHOLD {
            out.entry(col.to_hex()).or_insert((de, 0)).1 += 1;
        }
    }
    Ok(json!({
        "checked": cols.len(),
        "outOfGamut": out.iter().map(|(h, (de, n))| json!({"hex": h, "deltaE": de, "count": n})).collect::<Vec<_>>(),
    }))
}

fn view_json(v: &proof::ProofView) -> Value {
    json!({
        "target": v.setup.target.id(),
        "intent": v.setup.intent.id(),
        "simulatePaper": v.setup.simulate_paper,
        "proofColors": v.proof_colors,
        "overprintPreview": v.overprint,
        "separations": v.separations,
    })
}

fn proof_setup(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.proofSetup";
    let mut v = proof::view();
    if let Some(t) = str_param(p, "target") {
        let t = ProofTarget::parse(t).ok_or_else(|| bad(C, format!("unknown proof target `{t}`")))?;
        if let ProofTarget::Cmyk(name) = &t
            && !cms::profile(name).is_some_and(|k| k.kind == cms::ProfileKind::Cmyk)
        {
            return Err(bad(C, format!("unknown CMYK profile `{name}`")));
        }
        v.setup.target = t;
    }
    if let Some(i) = intent_param(p, C)? {
        v.setup.intent = i;
    }
    v.setup.simulate_paper = bool_or(p, "simulatePaper", v.setup.simulate_paper);
    if let Some(on) = p.get("proof").and_then(Value::as_bool) {
        v.proof_colors = on;
    }
    proof::set_view(v.clone());
    touch_all(s);
    Ok(view_json(&v))
}

fn toggle_view(s: &mut Session, p: &Value, f: impl FnOnce(&mut proof::ProofView, Option<bool>)) -> Result<Value> {
    let mut v = proof::view();
    f(&mut v, p.get("on").and_then(Value::as_bool));
    proof::set_view(v.clone());
    touch_all(s);
    Ok(view_json(&v))
}

fn plates_json(s: &Session) -> Value {
    let v = proof::view();
    let plates = s.active().map(|d| proof::plates(&d.doc)).unwrap_or_default();
    json!({
        "on": v.separations.is_some(),
        "plates": plates.iter().map(|pl| json!({
            "name": pl.name,
            "spot": pl.spot,
            "visible": v.separations.as_ref().is_none_or(|vis| vis.contains(&pl.name)),
            "rgb": pl.rgb,
        })).collect::<Vec<_>>(),
    })
}

fn separations(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.separationsPreview";
    let mut v = proof::view();
    let all: Vec<String> = s
        .active()
        .map(|d| proof::plates(&d.doc).into_iter().map(|p| p.name).collect())
        .unwrap_or_else(|| cms::PROCESS_PLATES.iter().map(|s| s.to_string()).collect());
    let check = |n: &str| if all.iter().any(|a| a == n) { Ok(n.to_string()) } else { Err(bad(C, format!("unknown plate `{n}`"))) };
    if let Some(on) = p.get("on").and_then(Value::as_bool) {
        v.separations = if on { Some(v.separations.take().unwrap_or_else(|| all.clone())) } else { None };
    }
    if let Some(a) = p.get("plates").and_then(Value::as_array) {
        v.separations = Some(a.iter().filter_map(Value::as_str).map(check).collect::<Result<Vec<_>>>()?);
    }
    if let Some(n) = str_param(p, "only") {
        v.separations = Some(vec![check(n)?]);
    }
    if let Some(n) = str_param(p, "toggle") {
        let n = check(n)?;
        let vis = v.separations.get_or_insert_with(|| all.clone());
        if let Some(i) = vis.iter().position(|x| *x == n) {
            vis.remove(i);
        } else {
            vis.push(n);
        }
    }
    proof::set_view(v);
    touch_all(s);
    Ok(plates_json(s))
}

fn set_spot(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.setSpot";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let spot = bool_or(p, "spot", true);
    let sw = s.doc()?.doc.swatch(&name).ok_or_else(|| bad(C, format!("no swatch `{name}`")))?;
    if !matches!(sw.paint, Paint::Solid { .. }) {
        return Err(bad(C, "only solid-colour swatches can be spot colours"));
    }
    if sw.spot != spot {
        let n = name.clone();
        s.edit("Swatch Options", |d, _| {
            if let Some(w) = d.swatch_mut(&n) {
                w.spot = spot;
                if spot {
                    w.global = true;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"name": name, "spot": spot}))
}
