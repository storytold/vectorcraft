//! Colour management: Edit → Color Settings / Assign Profile, document colour mode conversion
//! through the CMS, gamut checks, View → Proof Setup / Proof Colors / Overprint Preview, the
//! Separations Preview panel, Overprint Black and spot swatches.
//!
//! Colour settings and the proof view are process-wide (`vectorcraft_color::cms::active`,
//! `vectorcraft_render::proof::view`). A document's assigned profiles are stored in
//! `Document::color_profiles` (files before format v3: `Document.unknown["colorProfiles"]`) and
//! take the place of the working spaces in the active settings while the document is in front
//! ([`use_front_profiles`]): when they are assigned, when the working settings change, and when the
//! document comes to the front or its assignment comes back (an open, a switch, a close, a revert,
//! an undo or a rollback, [`Session::sync_front_profiles`]); the working spaces stay as Edit → Color
//! Settings set them. A document drawn while another is in front draws through its own profiles
//! ([`doc_cms`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ColorSettings, Intent, Model, Policy, ProofTarget};
use vectorcraft_color::recolor::Neutral;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;
use vectorcraft_render::proof;

#[cfg(not(target_arch = "wasm32"))]
use super::EngineError;
use super::{CommandSpec, Result, Session, always, bad, cmd, color_value, has_doc, switch_or, targets, text_param};

pub mod proof_file;

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
            "{rgb?: profile, cmyk?: profile (built in: \"VectorCraft Generic CMYK (SWOP-like)\" (default), \"VectorCraft Coated CMYK (FOGRA51)\" and \"VectorCraft Uncoated CMYK (FOGRA52)\" (computed from Fogra's characterization data), \"Device CMYK (uncalibrated)\"), intent?: \"perceptual\"|\"relative\"|\"saturation\"|\"absolute\", bpc?: bool, rgbPolicy?: \"off\"|\"preserveEmbedded\"|\"convert\", cmykPolicy?: \"off\"|\"preserveEmbedded\"|\"convert\"|\"preserveNumbers\", askMismatchOpen?: bool, askMismatchPaste?: bool, askMissingOpen?: bool (all false by default)} set the working spaces, the color management policies and the Ask boxes → {rgb, cmyk, intent, bpc, rgbPolicy, cmykPolicy, askMismatchOpen, askMismatchPaste, askMissingOpen, profiles: [{name, kind, builtin}]}. With askMismatchOpen or askMissingOpen on, the desktop app's File › Open asks what to do when the profile of a document's color mode is not the working space, or it has none (document.open takes the answer as profileChoice; it never asks); with askMismatchPaste on, Paste asks whether colors copied from a document with another profile keep their numbers or convert (edit.paste takes it as profileMismatch; see clipboard.profileMismatch). A policy decides what opening a document (document.open, file.newFromTemplate, file.revert) does with an RGB or CMYK profile it is tagged with that is not the working space: preserveEmbedded (default) keeps it; off and preserveNumbers take it off and keep the color numbers; convert converts the colors of that model into the working space with the intent and bpc and takes it off. The open reply's warnings name each profile taken off or converted, and a document a policy changed opens with unsaved changes. The desktop app and vectorcraft-cli keep them in `Color Settings.json` next to the preferences and use them again at the next start (not with VECTORCRAFT_NO_PREFS); saved settings naming a profile that is gone leave the defaults",
            always,
            color_settings
        ),
        cmd!(
            "color.loadProfile",
            "Load Color Profile…",
            [],
            None,
            "{path: .icc/.icm file of at most 32 MB} register an ICC profile (native only). The desktop app and vectorcraft-cli keep a copy of the file in the `Color Profiles` folder next to the preferences and register it again at every start (not with VECTORCRAFT_NO_PREFS, or in a confined `mcp --automation-*-root` server: there it lasts until the process ends) → {name, kind, remembered: whether a copy was kept}",
            always,
            load_profile
        ),
        cmd!(
            "color.removeProfile",
            "Remove Color Profile…",
            [],
            None,
            "{name} forget a profile color.loadProfile loaded, and the copy kept of it (built-in profiles and the working spaces stay) → {removed: name}",
            always,
            remove_profile
        ),
        cmd!(
            "edit.assignProfile",
            "Assign Profile…",
            ["Edit"],
            None,
            "{rgb?: profile|null, cmyk?: profile|null} tag the document with profiles (color numbers are kept; null: no profile, Don't Color Manage This Document, so colors show through the working space; the working space's name: that profile stays assigned when the working space changes). Colors then show and convert through the profiles assigned; the working spaces stay as edit.colorSettings set them → {rgb, cmyk}",
            has_doc,
            assign_profile
        ),
        cmd!(
            "object.convertDocumentColorMode",
            "Convert Document Color Mode",
            [],
            None,
            "{…as file.documentColorMode} same as file.documentColorMode: {mode: \"cmyk\"|\"rgb\", convert?: true, intent?, grays?} → {changed} (an alias kept for older scripts; the command palette leaves it out)",
            has_doc,
            convert_mode
        ),
        cmd!(
            query "color.convert",
            "Convert Color",
            [],
            None,
            "{color (#rrggbbaa and #rgba are refused), to: \"rgb\"|\"cmyk\"|\"gray\"|\"lab\"|\"oklch\", intent?} → {model, values, hex, lab: [L,a,b], outOfGamut, deltaE}; oklch values are [L (0..100 %), C, H (0..360°)] of the color's display sRGB (CSS Color 4 OKLCH: https://www.w3.org/TR/css-color-4/#specifying-oklab-oklch), with H 0 for a gray; a color outside sRGB (a Lab color, a CMYK color or an RGB color in a wider working RGB space) is clipped to sRGB first",
            always,
            convert_color
        ),
        cmd!(
            query "color.gamutCheck",
            "Gamut Warning",
            [],
            None,
            "{ids?|id?, colors?: [color] (#rrggbbaa and #rgba are refused)} colors that can't be printed in the document's assigned CMYK profile, else the working CMYK (selection, or the whole document when nothing is selected) → {checked, outOfGamut: [{hex, deltaE, count}]}",
            has_doc,
            gamut_check
        ),
        cmd!(
            query "view.proofSetup",
            "Proof Setup",
            ["View", "Proof Setup"],
            None,
            "{target?: \"workingCmyk\"|\"cmyk:<profile>\"|\"rgb:<profile>\"|\"legacyMacRgb\"|\"srgb\"|\"monitorRgb\"|\"protanopia\"|\"deuteranopia\"|\"tritanopia\" (cmyk: and rgb: name a CMYK or RGB profile of color.loadProfile or the built-in ones: the device to simulate), intent? (used while preserveNumbers is off), simulatePaper?: bool, preserveNumbers?: bool (Preserve CMYK or RGB Numbers: the colors' numbers in the document's space go to the device unchanged), simulateBlack?: bool (Simulate Black Ink: the device's black shows as the dark gray it prints; simulatePaper does this too; CMYK devices only, as simulatePaper), proof?: bool (also turn Proof Colors on)} → proof state",
            always,
            proof_setup
        ),
        cmd!(
            query "view.proofSetup.save",
            "Save Proof Setup…",
            [],
            None,
            "{path?: file | name?: a setup in the Proof Setups folder next to the preferences (at most 100 characters and no path characters; exactly one of the two), target?, intent?, simulatePaper?, preserveNumbers?, simulateBlack? (as view.proofSetup; default: the current proof setup's)} write a custom proof setup to a VectorCraft proof setup file (.vcproof, JSON) without changing the proof view. The target must be a device: workingCmyk, cmyk:<profile> or rgb:<profile> → {path, target, intent, simulatePaper, preserveNumbers, simulateBlack}",
            always,
            proof_file::save
        ),
        cmd!(
            query "view.proofSetup.load",
            "Load Proof Setup…",
            [],
            None,
            "{path?: file | name?: a setup in the Proof Setups folder (exactly one of the two)} read a proof setup file of at most 64 KiB that view.proofSetup.save wrote, make it the proof setup and turn Proof Colors on → {path} and the proof state. A file whose device names a profile that is not loaded is refused and nothing changes",
            always,
            proof_file::load
        ),
        cmd!(
            query "view.proofSetup.read",
            "Read Proof Setup",
            [],
            None,
            "{path?: file | name?: a setup in the Proof Setups folder (exactly one of the two)} the settings of a proof setup file, read as view.proofSetup.load reads it, without changing the proof view → {path, target, intent, simulatePaper, preserveNumbers, simulateBlack}",
            always,
            proof_file::read
        ),
        cmd!(
            query "view.proofSetup.list",
            "Saved Proof Setups",
            [],
            None,
            "{} the proof setup files in the Proof Setups folder next to the preferences, by name, at most 100 → {folder (null without the folder), setups: [{name, path}], truncated}",
            always,
            proof_file::list
        ),
        cmd!(query "view.proofColors", "Proof Colors", ["View"], None, "{on?: bool (default: toggle)} → proof state", always, |s, p| {
            toggle_view(s, p, "view.proofColors", |v, on| v.proof_colors = on.unwrap_or(!v.proof_colors))
        }),
        cmd!(
            query "view.overprintPreview",
            "Overprint Preview",
            ["View"],
            Some("Cmd+Alt+Shift+Y"),
            "{on?: bool (default: toggle)} → proof state",
            always,
            |s, p| toggle_view(s, p, "view.overprintPreview", |v, on| v.overprint = on.unwrap_or(!v.overprint))
        ),
        cmd!(
            query "view.separationsPreview",
            "Separations Preview",
            [],
            None,
            "{on?: bool, plates?: [names], toggle?: plate, only?: plate, soloToggle?: plate (only that plate, or every plate when it is the only one shown)} Separations Preview: show the chosen plates (one plate = grayscale ink coverage) → {on, plates: [{name, spot, visible, rgb}]}",
            always,
            separations
        ),
        cmd!(query "color.plates", "Plates", [], None, "{} → {on, plates: [{name, spot, visible, rgb}]}", always, |s, _| Ok(plates_json(s))),
        cmd!(
            "edit.colors.overprintBlack",
            "Overprint Black…",
            ["Edit", "Edit Colors"],
            None,
            "{remove?: false, percentage?: 100, fill?: true, stroke?: true, includeCmyBlacks?: false, includeSpotBlacks?: false, ids?} make the black fills and/or strokes of the selection (or ids; groups: their contents; type: its characters; a grayscale or 1-bit image colorized by paint.setFill: its color, as a fill) overprint, or stop (remove). Black: K ≥ percentage with no C, M or Y (any with includeCmyBlacks), not linked to a spot swatch (unless includeSpotBlacks); a gradient is black when every stop is → {changed: objects}",
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
    match text_param(p, "intent", cmd)? {
        None => Ok(None),
        Some(i) => Intent::parse(i).map(Some).ok_or_else(|| bad(cmd, format!("unknown intent `{i}`"))),
    }
}

/// The true or false `p` gives for `key`, or `None` when `p` leaves it out or gives null. Any other
/// value is an error, as [`switch_or`] reads it.
fn on_param(p: &Value, key: &str, cmd: &str) -> Result<Option<bool>> {
    if p.get(key).is_none_or(Value::is_null) {
        return Ok(None);
    }
    switch_or(p, key, false, cmd).map(Some)
}

/// The policy `p` gives for `key`, one of `allowed`, or `None` when `p` leaves it out.
fn policy_param(p: &Value, key: &str, allowed: &[Policy], cmd: &str) -> Result<Option<Policy>> {
    let Some(v) = text_param(p, key, cmd)? else { return Ok(None) };
    match Policy::parse(v).filter(|x| allowed.contains(x)) {
        Some(x) => Ok(Some(x)),
        None => {
            let ids: Vec<&str> = allowed.iter().map(Policy::id).collect();
            Err(bad(cmd, format!("`{key}` must be one of {}", ids.join(", "))))
        }
    }
}

fn settings_json(st: &ColorSettings) -> Value {
    json!({
        "rgb": st.rgb,
        "cmyk": st.cmyk,
        "intent": st.intent.id(),
        "bpc": st.bpc,
        "rgbPolicy": st.rgb_policy.id(),
        "cmykPolicy": st.cmyk_policy.id(),
        "askMismatchOpen": st.ask_mismatch_open,
        "askMismatchPaste": st.ask_mismatch_paste,
        "askMissingOpen": st.ask_missing_open,
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

/// Make the active settings the working settings with the profiles the front document is assigned
/// in place of the working spaces (none, or no document open: the working spaces). The open
/// documents redraw when the active settings change.
pub(crate) fn use_front_profiles(s: &mut Session) {
    let (rgb, cmyk) = s.active().map(|d| doc_profiles(&d.doc)).unwrap_or_default();
    if cms::use_document_profiles(rgb.as_deref(), cmyk.as_deref()) {
        touch_all(s);
    }
}

fn color_settings(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.colorSettings";
    let mut st = cms::working_settings();
    let before = st.clone();
    if let Some(v) = text_param(p, "rgb", C)? {
        st.rgb = cms::canonical_name(v).to_string();
    }
    if let Some(v) = text_param(p, "cmyk", C)? {
        st.cmyk = cms::canonical_name(v).to_string();
    }
    if let Some(i) = intent_param(p, C)? {
        st.intent = i;
    }
    st.bpc = switch_or(p, "bpc", st.bpc, C)?;
    if let Some(v) = policy_param(p, "rgbPolicy", &Policy::RGB, C)? {
        st.rgb_policy = v;
    }
    if let Some(v) = policy_param(p, "cmykPolicy", &Policy::CMYK, C)? {
        st.cmyk_policy = v;
    }
    st.ask_mismatch_open = switch_or(p, "askMismatchOpen", st.ask_mismatch_open, C)?;
    st.ask_mismatch_paste = switch_or(p, "askMismatchPaste", st.ask_mismatch_paste, C)?;
    st.ask_missing_open = switch_or(p, "askMissingOpen", st.ask_missing_open, C)?;
    if st != before {
        cms::set_active(&st).map_err(|e| bad(C, e.to_string()))?;
        // Saved for the next start when the app attached a settings file (#6335).
        s.store_color_settings(&st);
        use_front_profiles(s);
        touch_all(s);
    }
    Ok(settings_json(&st))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_profile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.loadProfile";
    let path = text_param(p, "path", C)?.ok_or_else(|| bad(C, "missing `path`"))?;
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    let bytes = crate::store::read_capped(std::path::Path::new(path), crate::store::MAX_PROFILE_FILE).map_err(|e| bad(C, format!("{path}: {e}")))?;
    let info = cms::register_icc(&bytes, None).map_err(|e| bad(C, e.to_string()))?;
    // The bytes are kept, not the path: the file may move or go away.
    let remembered = s.store_profile(&info.name, &bytes);
    // A new file of a working space or of an assigned profile in use: the active settings still
    // hold the old one. A profile the front document is assigned, loaded after it was removed, is
    // in use again.
    let (working, active) = (cms::working_settings(), cms::active_settings());
    let (rgb, cmyk) = s.active().map(|d| doc_profiles(&d.doc)).unwrap_or_default();
    let front = [rgb, cmyk].iter().any(|n| n.as_deref() == Some(info.name.as_str()));
    if front || [&working.rgb, &working.cmyk, &active.rgb, &active.cmyk].contains(&&info.name) {
        cms::set_active(&working).map_err(|e| bad(C, e.to_string()))?;
        use_front_profiles(s);
        touch_all(s);
    } else if proof_targets(&info.name) {
        // A new file of the profile View → Proof Setup targets: redraw.
        touch_all(s);
    } else {
        // Documents behind the front one draw through their own profiles: those assigned it redraw.
        touch_assigned(s, &info.name);
    }
    Ok(json!({"name": info.name, "kind": info.kind, "remembered": remembered}))
}

/// Forget a profile `color.loadProfile` loaded, and the copy that is kept of it.
fn remove_profile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.removeProfile";
    let name = text_param(p, "name", C)?.ok_or_else(|| bad(C, "missing `name`"))?;
    let info = cms::profiles().into_iter().find(|i| i.name == name).ok_or_else(|| bad(C, format!("no profile named `{name}`")))?;
    if info.builtin {
        return Err(bad(C, format!("`{name}` is a built-in profile and stays")));
    }
    let working = cms::working_settings();
    if working.rgb == name || working.cmyk == name {
        return Err(bad(C, format!("`{name}` is a working space: choose another with edit.colorSettings first")));
    }
    let active = cms::active_settings();
    if !cms::unregister_icc(&info.name) {
        return Err(bad(C, format!("no profile named `{name}`")));
    }
    s.unstore_profile(&info.name);
    // An assigned profile in use: the working space takes its place, as for any document assigned
    // a profile that is not available.
    if active.rgb == name || active.cmyk == name {
        use_front_profiles(s);
    }
    // The profile View → Proof Setup targets: redraw.
    if proof_targets(&info.name) {
        touch_all(s);
    }
    // Documents behind the front one draw through their own profiles: those assigned it redraw.
    touch_assigned(s, &info.name);
    Ok(json!({"removed": info.name}))
}

/// Redraw the open documents assigned the profile `name`.
fn touch_assigned(s: &mut Session, name: &str) {
    for d in &mut s.docs {
        let (rgb, cmyk) = doc_profiles(&d.doc);
        if [rgb, cmyk].iter().any(|n| n.as_deref() == Some(name)) {
            d.revision += 1;
        }
    }
}

/// Whether View → Proof Setup targets the CMYK or RGB profile `name`.
fn proof_targets(name: &str) -> bool {
    matches!(proof::view().setup.target, ProofTarget::Cmyk(t) | ProofTarget::Rgb(t) if t == name)
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

/// The color-management context the document draws through: the working settings with its
/// assigned profiles in place of the working spaces, whether or not it is in front
/// ([`cms::for_document`]).
pub fn doc_cms(d: &vectorcraft_doc::Document) -> Arc<cms::Cms> {
    let (rgb, cmyk) = doc_profiles(d);
    cms::for_document(rgb.as_deref(), cmyk.as_deref())
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
        // The document draws through its profiles now, and a Document Color Mode later in the same
        // batch converts through them. The working spaces stay as they are.
        use_front_profiles(s);
    }
    Ok(json!({"rgb": rgb, "cmyk": cmyk}))
}

/// How Document Color Mode converts neutral RGB greys (R = G = B) to CMYK (the `grays` param).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grays {
    /// Through the CMYK profile like any colour: light grays in cyan, magenta and yellow only, darker
    /// grays in four inks, and a rich black.
    #[default]
    Profile,
    /// On the black plate only: K is the grey's ink percentage (as Convert to Grayscale and then
    /// to CMYK would give).
    Black,
}

impl Grays {
    /// The `grays` param of `cmd`: `"profile"` (default) or `"black"`.
    pub(crate) fn param(cmd: &str, p: &Value) -> Result<Self> {
        match text_param(p, "grays", cmd)?.map(str::to_ascii_lowercase).as_deref() {
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
    let mode = match p.get("mode").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
        Some("cmyk") => ColorMode::Cmyk,
        Some("rgb") => ColorMode::Rgb,
        _ => return Err(bad(C, "mode must be \"cmyk\" or \"rgb\"")),
    };
    let intent = intent_param(p, C)?;
    let grays = Grays::param(C, p)?;
    let convert = switch_or(p, "convert", true, C)?;
    // Choosing the mode the document already has (the checked menu item) is not an edit.
    if s.doc()?.doc.color_mode == mode {
        return Ok(json!({ "changed": 0 }));
    }
    let mut changed = 0usize;
    s.edit("Document Color Mode", |d, _| {
        changed = set_color_mode(d, mode, convert, intent, grays);
        Ok(())
    })?;
    if convert {
        if s.in_interaction() && s.batch_stash.is_none() {
            if let Some(it) = &mut s.doc_mut()?.interaction {
                it.deferred_color_mode = Some(DeferredColorMode { mode, intent, grays });
            }
        } else {
            let before = new_art_colors_before(s)?;
            convert_new_art_colors(s, mode, intent, grays)?;
            record_new_art_colors(s, before)?;
        }
    } else if s.batch_stash.is_some() || s.doc()?.undo_group.is_some() {
        let before = new_art_colors_before(s)?;
        record_new_art_colors(s, before)?;
    }
    Ok(json!({ "changed": changed }))
}

/// The validated conversion choices of the current tentative color-mode preview.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeferredColorMode {
    mode: ColorMode,
    intent: Option<Intent>,
    grays: Grays,
}

/// Complete the deferred new-art conversion before the interaction becomes an Undo step.
pub(crate) fn commit_new_art_colors(s: &mut Session) -> Result<()> {
    let pending =
        s.doc()?.interaction.as_ref().filter(|it| it.preview.is_some() && it.color_mode_paints.is_none()).and_then(|it| it.deferred_color_mode);
    if let Some(DeferredColorMode { mode, intent, grays }) = pending {
        let before = new_art_colors_before(s)?;
        convert_new_art_colors(s, mode, intent, grays)?;
        record_new_art_colors(s, before)?;
    }
    Ok(())
}

/// Record new-art paints `before` and the session's current ones with the step just made (or the
/// interaction or undo group in progress), so Undo puts `before` back and Redo the current ones.
pub(super) fn record_new_art_colors(s: &mut Session, before: vectorcraft_tools::PaintDefaults) -> Result<()> {
    let after = s.paint.clone();
    if before != after || s.batch_stash.is_some() || s.doc()?.undo_group.is_some() {
        let st = s.doc_mut()?;
        let slot = if let Some(it) = &mut st.interaction {
            Some(&mut it.color_mode_paints)
        } else {
            st.history.undo.last_mut().map(|e| &mut e.color_mode_paints)
        };
        if let Some(slot) = slot {
            match slot {
                Some((_, end)) => *end = after,
                slot => *slot = Some((before, after)),
            }
        }
        if st.interaction.is_none() {
            if let Some(e) = st.history.undo.last_mut() {
                e.recost_paints();
            }
            st.history.trim();
        }
    }
    Ok(())
}

/// The new-art paints an undo step that starts now puts back: those at the start of the undo group
/// or interaction in progress, else the session's current ones.
pub(super) fn new_art_colors_before(s: &Session) -> Result<vectorcraft_tools::PaintDefaults> {
    let st = s.doc()?;
    Ok(st.undo_group.as_ref().map(|g| &g.paint).or_else(|| st.interaction.as_ref().map(|it| &it.paint)).cloned().unwrap_or_else(|| s.paint.clone()))
}

/// Convert the fill and stroke that new art takes, and the colors of its template appearance
/// (paints and effect colors), as [`set_color_mode`] converted the active document's art. The
/// command records their original values for Undo. Previews defer this until acceptance;
/// a batch converts immediately and restores the values itself when it fails.
fn convert_new_art_colors(s: &mut Session, mode: ColorMode, intent: Option<Intent>, grays: Grays) -> Result<()> {
    let d = &s.doc()?.doc;
    let bases = global_bases(d);
    let conv = mode_converter(mode, intent, grays);
    let mut f = |col: &Color, link: proof::Link| mode_color(col, link, &bases, &conv);
    map_paint_colors(&mut s.paint.fill, &mut f);
    map_paint_colors(&mut s.paint.stroke, &mut f);
    if let Some(a) = s.paint.appearance.as_mut() {
        for it in &mut a.items {
            let paint = match it {
                vectorcraft_doc::AppearanceItem::Fill(l) => &mut l.paint,
                vectorcraft_doc::AppearanceItem::Stroke(l) => &mut l.paint,
            };
            map_paint_colors(paint, &mut f);
        }
        map_effect_colors(a, &mut f);
    }
    Ok(())
}

/// How Document Color Mode converts a color to `mode` through the color settings (`intent`,
/// default: theirs): Gray and Lab colors stay as they are; with `grays` on the black plate, an RGB
/// gray becomes K only in CMYK.
fn mode_converter(mode: ColorMode, intent: Option<Intent>, grays: Grays) -> impl Fn(&Color) -> Color {
    let c = cms::active();
    let intent = intent.unwrap_or(c.settings().intent);
    let model = if mode == ColorMode::Cmyk { Model::Cmyk } else { Model::Rgb };
    let k_only = model == Model::Cmyk && grays == Grays::Black;
    move |col: &Color| match col {
        Color::Gray { .. } | Color::Lab { .. } => *col,
        // Grey ink percentage, then K only (a Gray colour's CMYK).
        Color::Rgb { .. } if k_only && Neutral::of(col).is_some() => c.convert(&c.convert(col, Model::Gray, intent), Model::Cmyk, intent),
        _ => c.convert(col, model, intent),
    }
}

/// The name and color of each global swatch of `d`, for [`mode_color`].
fn global_bases(d: &vectorcraft_doc::Document) -> BTreeMap<String, Color> {
    let mut first = BTreeMap::new();
    for w in d.swatches_iter() {
        // An ineligible first record still shadows a later global solid of the same name.
        first.entry(w.name.as_str()).or_insert(w);
    }
    first
        .into_values()
        .filter_map(|w| {
            if !w.global {
                return None;
            }
            let Paint::Solid { color, swatch: None, .. } = &w.paint else { return None };
            Some((w.name.clone(), d.linked_color(*color, w.spot)))
        })
        .collect()
}

/// The color Document Color Mode gives `col` (swatch link `link`): a tint of a global swatch in
/// `bases` (already converted) is that swatch's color at its tint; any other color goes through
/// `conv` ([`mode_converter`]).
fn mode_color(col: &Color, link: proof::Link, bases: &BTreeMap<String, Color>, conv: &dyn Fn(&Color) -> Color) -> Color {
    match link.and_then(|(name, tint)| Some((bases.get(name)?, tint))) {
        Some((base, tint)) if tint < 1.0 => base.tinted(tint),
        _ => conv(col),
    }
}

/// Put `d` in color `mode` and, with `convert`, convert every color of its art (opacity masks,
/// effect colors and the series paints and label styles that graphs keep included), symbols,
/// graph designs, pattern tiles (the tile that pattern editing's Cancel puts back included),
/// graphic styles (their effects included), brushes (their art and a Hue Shift key color) and
/// swatches through the color settings (`intent`, default: theirs) → how many colors of the art
/// (opacity masks, effect colors and graphs' paints included), symbols, graph designs and pattern
/// tiles changed; the colors of graphic styles and brushes convert without being counted. Gray
/// colors stay Gray (they print on the black plate in either mode); RGB grays are colors like any
/// other and separate through the CMYK profile, unless `grays` puts them on the black plate
/// ([`mode_converter`]). Lab colors
/// (spot color definitions) are device independent and fit either mode. A tint of a global
/// swatch becomes the converted swatch's color at its tint. Document Color Mode and
/// `document.open {colorMode}` both convert this way.
pub(crate) fn set_color_mode(d: &mut vectorcraft_doc::Document, mode: ColorMode, convert: bool, intent: Option<Intent>, grays: Grays) -> usize {
    d.color_mode = mode;
    if !convert {
        return 0;
    }
    convert_colors(d, &mode_converter(mode, intent, grays))
}

/// Every color of `d` that [`set_color_mode`] converts, converted with `conv` → how many colors of
/// the art changed, as [`set_color_mode`] counts them.
fn convert_colors(d: &mut vectorcraft_doc::Document, conv: &dyn Fn(&Color) -> Color) -> usize {
    for sw in d.swatches_iter_mut() {
        match &mut sw.paint {
            Paint::Solid { color, .. } => *color = conv(color),
            Paint::Gradient(g) => {
                for st in &mut g.gradient.stops {
                    st.color = conv(&st.color);
                }
                if let Some(freeform) = &mut g.freeform {
                    for point in &mut freeform.points {
                        point.color = conv(&point.color);
                    }
                }
            }
            _ => {}
        }
    }
    // Tints are their converted swatch's colour at their tint (exact ink percentages).
    let bases = global_bases(d);
    let changed = std::cell::Cell::new(0usize);
    let mut count = |col: &Color, link: proof::Link| {
        let n = mode_color(col, link, &bases, &conv);
        if n != *col {
            changed.set(changed.get() + 1);
        }
        n
    };
    let patterns = d.patterns.iter_mut().chain(d.pattern_edit.iter_mut().filter_map(|e| e.original.as_mut()));
    for a in d
        .layers
        .iter_mut()
        .chain(d.symbols.iter_mut().map(|s| &mut s.art))
        .chain(d.graph_designs.iter_mut().map(|g| &mut g.art))
        .chain(patterns.flat_map(|p| &mut p.art))
    {
        map_art_colors(Arc::make_mut(a), &mut count);
    }
    // The reply counts the colors of the document's art only, not those of the graphic styles and brushes.
    let in_art = changed.get();
    for g in &mut d.graphic_styles {
        for it in &mut g.appearance.items {
            let paint = match it {
                vectorcraft_doc::AppearanceItem::Fill(l) => &mut l.paint,
                vectorcraft_doc::AppearanceItem::Stroke(l) => &mut l.paint,
            };
            map_paint_colors(paint, &mut count);
        }
        map_effect_colors(&mut g.appearance, &mut count);
    }
    // Convert each brush's art. A Hue Shift key color takes the converted color of the art color
    // it matches (a tint of a global swatch included), so the key still matches the converted art;
    // a key that matches no art color converts on its own.
    let in_styles = changed.get();
    let mut brushes = vectorcraft_brush::library(d);
    let mut keys = false;
    for b in &mut brushes {
        let old_key = match b.kind.colorization() {
            vectorcraft_brush::Colorization::HueShift { key } => Some(key),
            _ => None,
        };
        let mut new_key = None;
        for art in b.art_mut() {
            map_art_colors(art, &mut |col: &Color, link: proof::Link| {
                let n = count(col, link);
                if new_key.is_none() && old_key.is_some_and(|k| matches_key(col, &k)) {
                    new_key = Some(n);
                }
                n
            });
        }
        let colorization = match &mut b.kind {
            vectorcraft_brush::BrushKind::Art(a) => Some(&mut a.colorization),
            vectorcraft_brush::BrushKind::Scatter(s) => Some(&mut s.colorization),
            vectorcraft_brush::BrushKind::Pattern(p) => Some(&mut p.colorization),
            vectorcraft_brush::BrushKind::Calligraphic(_) | vectorcraft_brush::BrushKind::Bristle(_) => None,
        };
        if let Some(vectorcraft_brush::Colorization::HueShift { key }) = colorization {
            let n = new_key.unwrap_or_else(|| conv(key));
            keys |= n != *key;
            *key = n;
        }
    }
    if keys || changed.get() > in_styles {
        vectorcraft_brush::store(d, &brushes);
    }
    // This pass gives tint swatches their converted swatch's color at their tint; the passes above already did the same for the art's tints.
    d.map_solid_paints(&mut |c, link, tint| match link.as_ref().and_then(|name| bases.get(name)) {
        Some(base) if *tint < 1.0 => {
            *c = base.tinted(*tint);
            true
        }
        _ => false,
    });
    in_art
}

/// Apply the color management policies of Edit → Color Settings to `d`, a document being opened:
/// each RGB or CMYK profile it is tagged with that is not the working space of its model is kept
/// (Preserve Embedded Profiles), taken off with the color numbers kept (Off, Preserve Numbers), or
/// taken off after the colors of that model were converted from it into the working space with the
/// working intent (Convert to Working Space). A profile that is not available here is taken off
/// by Off and Preserve Numbers and kept by Convert to Working Space, which has nothing to convert
/// from. → a note for each profile, and whether `d` changed.
pub(crate) fn apply_open_policies(d: &mut vectorcraft_doc::Document) -> (Vec<String>, bool) {
    // Without a choice nothing can fail.
    apply_open_policies_choosing(d, None, &cms::working_settings()).unwrap_or_default()
}

/// What opening a document can ask about its profile (Edit › Color Settings' Ask When Opening
/// boxes): the profile of its color mode differs from the working space, or it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileIssue {
    /// Profile Mismatches: tagged with a profile other than the working space.
    Mismatch,
    /// Missing Profiles: not tagged with a profile.
    Missing,
}

impl ProfileIssue {
    pub fn id(self) -> &'static str {
        match self {
            ProfileIssue::Mismatch => "mismatch",
            ProfileIssue::Missing => "missing",
        }
    }
}

/// The answer to a [`ProfileQuestion`] (`document.open`'s `profileChoice`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileChoice {
    /// Mismatch: keep the profile (as Preserve Embedded Profiles).
    UseEmbedded,
    /// Mismatch: convert the colors of that model into the working space and take the profile off
    /// (as Convert to Working Space).
    Convert,
    /// Mismatch: take the profile off and keep the color numbers (as Off).
    Discard,
    /// Missing: leave the document untagged (it follows the working space).
    Leave,
    /// Missing: tag the document with the working space of its color mode.
    AssignWorking,
    /// Missing: tag the document with this profile.
    Assign(String),
}

impl ProfileChoice {
    /// The ids `profileChoice` takes.
    pub const IDS: [&'static str; 6] = ["useEmbedded", "convert", "discard", "leave", "assignWorking", "assign"];

    pub fn id(&self) -> &'static str {
        match self {
            ProfileChoice::UseEmbedded => "useEmbedded",
            ProfileChoice::Convert => "convert",
            ProfileChoice::Discard => "discard",
            ProfileChoice::Leave => "leave",
            ProfileChoice::AssignWorking => "assignWorking",
            ProfileChoice::Assign(_) => "assign",
        }
    }

    /// The question kind this choice answers.
    pub fn issue(&self) -> ProfileIssue {
        match self {
            ProfileChoice::UseEmbedded | ProfileChoice::Convert | ProfileChoice::Discard => ProfileIssue::Mismatch,
            _ => ProfileIssue::Missing,
        }
    }

    /// The choice `id` (`profile`, which only `assign` takes, for `assign`), or an error naming
    /// `cmd`.
    pub fn parse(id: &str, profile: Option<&str>, cmd: &str) -> Result<Self> {
        let key = "profileChoice";
        let choice = match id {
            "useEmbedded" => ProfileChoice::UseEmbedded,
            "convert" => ProfileChoice::Convert,
            "discard" => ProfileChoice::Discard,
            "leave" => ProfileChoice::Leave,
            "assignWorking" => ProfileChoice::AssignWorking,
            "assign" => match profile.filter(|p| !p.is_empty()) {
                Some(p) => ProfileChoice::Assign(cms::canonical_name(p).to_string()),
                None => {
                    let path = "assignProfile";
                    return Err(bad(cmd, format!("missing `{path}`")));
                }
            },
            _ => return Err(bad(cmd, format!("`{key}` must be one of {}", Self::IDS.join(", ")))),
        };
        // `assignProfile` goes with `assign` only.
        if profile.is_some() && !matches!(choice, ProfileChoice::Assign(_)) {
            let path = "assignProfile";
            return Err(bad(cmd, format!("`{path}` goes with `{key}`: \"assign\" only")));
        }
        Ok(choice)
    }

    /// `document.open`'s `profileChoice` and `assignProfile` in `p` (`None` when left out).
    pub(crate) fn param(p: &Value, cmd: &str) -> Result<Option<Self>> {
        let profile = text_param(p, "assignProfile", cmd)?;
        match text_param(p, "profileChoice", cmd)? {
            Some(id) => Self::parse(id, profile, cmd).map(Some),
            None if profile.is_some() => {
                let path = "profileChoice";
                Err(bad(cmd, format!("missing `{path}`")))
            }
            None => Ok(None),
        }
    }
}

/// What opening document `d` could ask about the profile of its color mode, with the working
/// settings `st`: none when it is tagged with the working space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileQuestion {
    pub issue: ProfileIssue,
    /// The model of the document's color mode.
    pub model: ColorMode,
    /// The profile the document is tagged with (none when it is missing).
    pub profile: Option<String>,
    /// The working space of that model.
    pub working: String,
    /// What the policy of that model does, the choice the prompt starts on.
    pub default: ProfileChoice,
}

impl ProfileQuestion {
    /// The question for document `d` under the working settings `st` (whether or not they ask it).
    pub fn of(d: &vectorcraft_doc::Document, st: &ColorSettings) -> Option<Self> {
        let (rgb, cmyk) = doc_profiles(d);
        let (tag, working, policy) = match d.color_mode {
            ColorMode::Rgb => (rgb, cms::canonical_name(&st.rgb).to_string(), st.rgb_policy),
            ColorMode::Cmyk => (cmyk, cms::canonical_name(&st.cmyk).to_string(), st.cmyk_policy),
        };
        let (issue, default) = match &tag {
            Some(t) if *t == working => return None,
            Some(_) => (
                ProfileIssue::Mismatch,
                match policy {
                    Policy::PreserveEmbedded => ProfileChoice::UseEmbedded,
                    Policy::Convert => ProfileChoice::Convert,
                    Policy::Off | Policy::PreserveNumbers => ProfileChoice::Discard,
                },
            ),
            // VectorCraft's own rule: Convert to Working Space starts on assigning the working
            // space, the other policies on leaving the document as it is.
            None => (ProfileIssue::Missing, if policy == Policy::Convert { ProfileChoice::AssignWorking } else { ProfileChoice::Leave }),
        };
        Some(Self { issue, model: d.color_mode, profile: tag, working, default })
    }

    /// Do the settings `st` ask it (the Ask When Opening box of its kind is on)?
    pub fn asked(&self, st: &ColorSettings) -> bool {
        match self.issue {
            ProfileIssue::Mismatch => st.ask_mismatch_open,
            ProfileIssue::Missing => st.ask_missing_open,
        }
    }

    /// The model's name as the UI shows it ("RGB", "CMYK").
    pub fn model_label(&self) -> &'static str {
        match self.model {
            ColorMode::Rgb => "RGB",
            ColorMode::Cmyk => "CMYK",
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "issue": self.issue.id(),
            "model": self.model_label().to_ascii_lowercase(),
            "profile": self.profile,
            "working": self.working,
            "default": self.default.id(),
        })
    }
}

/// [`apply_open_policies`] with the answer `choice` to the document's [`ProfileQuestion`]: a
/// mismatch choice takes the place of the policy of the document's color mode, and a missing
/// choice tags the document after the policies ran. A choice for the other kind of question, or
/// for a document with none, is not used. → as [`apply_open_policies`]; an error when the profile
/// to assign is not a profile of that model. `working` is the settings snapshot the question is
/// built from and the policies are read from.
pub(crate) fn apply_open_policies_choosing(
    d: &mut vectorcraft_doc::Document,
    choice: Option<&ProfileChoice>,
    working: &ColorSettings,
) -> Result<(Vec<String>, bool)> {
    let question = ProfileQuestion::of(d, working);
    let choice = choice.filter(|c| question.as_ref().is_some_and(|q| q.issue == c.issue()));
    // A mismatch answer stands in for the policy of the document's color mode.
    let chosen = |model: ColorMode, policy: Policy| -> (Policy, bool) {
        match choice {
            Some(c) if d.color_mode == model => match c {
                ProfileChoice::UseEmbedded => (Policy::PreserveEmbedded, true),
                ProfileChoice::Convert => (Policy::Convert, true),
                ProfileChoice::Discard => (Policy::Off, true),
                _ => (policy, false),
            },
            _ => (policy, false),
        }
    };
    let (rgb_policy, rgb_chosen) = chosen(ColorMode::Rgb, working.rgb_policy);
    let (cmyk_policy, cmyk_chosen) = chosen(ColorMode::Cmyk, working.cmyk_policy);
    // The profile a missing answer assigns, checked before anything changes.
    let assign = match (choice, &question) {
        (Some(ProfileChoice::AssignWorking), Some(q)) => Some(q.working.clone()),
        (Some(ProfileChoice::Assign(name)), Some(q)) => {
            let kind = if q.model == ColorMode::Rgb { cms::ProfileKind::Rgb } else { cms::ProfileKind::Cmyk };
            let (key, n) = (q.model_label().to_ascii_lowercase(), name);
            let info = cms::profile(name).filter(|p| p.kind == kind).ok_or_else(|| bad("document.open", format!("unknown {key} profile `{n}`")))?;
            Some(info.name)
        }
        _ => None,
    };
    let (rgb, cmyk) = doc_profiles(d);
    let mut notes = Vec::new();
    // What each model's policy does with its profile: `None` keeps it; `Some` takes it off, after
    // converting the colors from the profile it holds. `by_choice`: the prompt's answer decided.
    let mut decide =
        |label: &str, tag: Option<String>, working: &str, policy: Policy, by_choice: bool, kind: cms::ProfileKind| -> Option<Option<String>> {
            let tag = tag.filter(|t| t != working)?;
            let available = cms::profile(&tag).is_some_and(|p| p.kind == kind);
            match policy {
                Policy::PreserveEmbedded => None,
                Policy::Off | Policy::PreserveNumbers if by_choice => {
                    notes.push(format!("the {label} profile “{tag}” the file is tagged with was discarded, as chosen when it was opened"));
                    Some(None)
                }
                Policy::Off | Policy::PreserveNumbers => {
                    notes.push(format!(
                        "the {label} profile “{tag}” the file is tagged with was not used: Color Settings' {label} policy is {}",
                        policy.label()
                    ));
                    Some(None)
                }
                Policy::Convert if available => {
                    notes.push(format!(
                        "the {label} colors were converted from the profile “{tag}” the file is tagged with into the working space “{working}”"
                    ));
                    Some(Some(tag))
                }
                Policy::Convert => {
                    notes.push(format!("the {label} profile “{tag}” the file is tagged with is not available, so its colors were not converted"));
                    None
                }
            }
        };
    let rgb_do = decide("RGB", rgb, &working.rgb, rgb_policy, rgb_chosen, cms::ProfileKind::Rgb);
    let cmyk_do = decide("CMYK", cmyk, &working.cmyk, cmyk_policy, cmyk_chosen, cms::ProfileKind::Cmyk);
    let mut changed = false;
    if let (Some(name), Some(q)) = (assign, &question) {
        let label = q.model_label();
        notes.push(format!("the document was assigned the {label} profile “{name}”, as chosen when it was opened"));
        match q.model {
            ColorMode::Rgb => d.color_profiles.rgb = Some(name),
            ColorMode::Cmyk => d.color_profiles.cmyk = Some(name),
        }
        changed = true;
    }
    if rgb_do.is_none() && cmyk_do.is_none() {
        return Ok((notes, changed));
    }
    let (from_rgb, from_cmyk) = (rgb_do.clone().flatten(), cmyk_do.clone().flatten());
    if from_rgb.is_some() || from_cmyk.is_some() {
        let source = cms::for_document(from_rgb.as_deref(), from_cmyk.as_deref());
        let dest = cms::working();
        let intent = working.intent;
        let (rgb_on, cmyk_on) = (from_rgb.is_some(), from_cmyk.is_some());
        let conv = |c: &Color| match c {
            Color::Rgb { .. } if rgb_on => source.convert_into(&dest, c, intent),
            Color::Cmyk { .. } if cmyk_on => source.convert_into(&dest, c, intent),
            _ => *c,
        };
        convert_colors(d, &conv);
        convert_setting_colors(d, &conv);
    }
    if rgb_do.is_some() {
        d.color_profiles.rgb = None;
    }
    if cmyk_do.is_some() {
        d.color_profiles.cmyk = None;
    }
    Ok((notes, true))
}

/// The colors of `d`'s art and resources ([`convert_colors`]) and of its character and paragraph
/// styles, converted with `conv`; not its artboards' backgrounds or Document Setup's colors.
pub(crate) fn convert_art_and_style_colors(d: &mut vectorcraft_doc::Document, conv: &dyn Fn(&Color) -> Color) {
    convert_colors(d, conv);
    convert_style_colors(d, conv);
}

/// The RGB and CMYK profiles the colors of `d` show through, by name: those it is assigned, else
/// the working spaces. An assigned profile that is not available (removed since) is the working
/// space, as the document draws through it ([`cms::for_document`]).
pub(crate) fn effective_profiles(d: &vectorcraft_doc::Document) -> [String; 2] {
    let working = cms::working_settings();
    let (rgb, cmyk) = doc_profiles(d);
    let available = |name: Option<String>, kind: cms::ProfileKind| name.filter(|n| cms::profile(n).is_some_and(|p| p.kind == kind));
    [
        available(rgb, cms::ProfileKind::Rgb).unwrap_or_else(|| cms::canonical_name(&working.rgb).to_string()),
        available(cmyk, cms::ProfileKind::Cmyk).unwrap_or_else(|| cms::canonical_name(&working.cmyk).to_string()),
    ]
}

/// What Paste does with the colors of a model when the document they were copied from showed them
/// through another profile than the document they go into (`profileMismatch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteProfile {
    /// The colors keep their numbers.
    PreserveNumbers,
    /// The colors convert from the source profile into the destination's, which keeps their
    /// appearance.
    Convert,
}

impl PasteProfile {
    pub fn id(self) -> &'static str {
        match self {
            PasteProfile::PreserveNumbers => "preserveNumbers",
            PasteProfile::Convert => "convert",
        }
    }

    /// What a model's policy does to pasted colors (VectorCraft's own mapping): Convert to Working
    /// Space converts them, the other policies keep their numbers.
    pub fn of(policy: Policy) -> Self {
        if policy == Policy::Convert { PasteProfile::Convert } else { PasteProfile::PreserveNumbers }
    }

    /// The `profileMismatch` param of paste command `cmd` (`None` when left out): one choice for
    /// both models, or `{rgb?, cmyk?}` with a choice for each (a model left out follows its policy).
    pub(crate) fn param(p: &Value, cmd: &str) -> Result<Option<PasteChoice>> {
        let key = "profileMismatch";
        let refuse = || bad(cmd, format!("`{key}` must be one of {}", "preserveNumbers, convert, {rgb?, cmyk?} of those"));
        let one = |v: &Value| match v.as_str() {
            Some("preserveNumbers") => Ok(PasteProfile::PreserveNumbers),
            Some("convert") => Ok(PasteProfile::Convert),
            _ => Err(refuse()),
        };
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(o)) => {
                if o.keys().any(|k| k != "rgb" && k != "cmyk") {
                    return Err(refuse());
                }
                let model = |m: &str| o.get(m).map(one).transpose();
                Ok(Some(PasteChoice { rgb: model("rgb")?, cmyk: model("cmyk")? }))
            }
            Some(v) => {
                let c = one(v)?;
                Ok(Some(PasteChoice { rgb: Some(c), cmyk: Some(c) }))
            }
        }
    }
}

/// What Paste does with the mismatched colors of each model (`profileMismatch`); a model without a
/// choice follows its policy ([`PasteProfile::of`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PasteChoice {
    pub rgb: Option<PasteProfile>,
    pub cmyk: Option<PasteProfile>,
}

impl PasteChoice {
    /// The choice for the colors of `model`, if any.
    pub fn of(&self, model: ColorMode) -> Option<PasteProfile> {
        match model {
            ColorMode::Rgb => self.rgb,
            ColorMode::Cmyk => self.cmyk,
        }
    }
}

/// A model whose colors a paste brings from another profile than the destination's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteMismatch {
    pub model: ColorMode,
    /// The profile the colors showed through where they were copied.
    pub from: String,
    /// The profile they show through in the document they go into.
    pub to: String,
    /// What the model's policy does with them; Preserve Numbers when they can't convert.
    pub default: PasteProfile,
    /// Both profiles are available, so the colors can convert; otherwise they keep their numbers
    /// whatever the choice.
    pub convertible: bool,
}

impl PasteMismatch {
    pub fn to_json(&self) -> Value {
        let model = if self.model == ColorMode::Rgb { "rgb" } else { "cmyk" };
        json!({ "model": model, "clipboard": self.from, "document": self.to, "default": self.default.id(), "convertible": self.convertible })
    }
}

/// The models whose colors pasting `clip` into `d` brings from another profile: RGB, CMYK or both,
/// each only when the copied objects or the resources they carry hold a color of that model. The
/// background colors of copied artboards count only with `boards` (Paste on All Artboards doesn't
/// paste the artboards).
pub(crate) fn paste_mismatches(clip: &super::clipboard::Clipboard, d: &vectorcraft_doc::Document, boards: bool) -> Vec<PasteMismatch> {
    let Some([from_rgb, from_cmyk]) = &clip.source_profiles else { return vec![] };
    let [to_rgb, to_cmyk] = effective_profiles(d);
    let working = cms::working_settings();
    let mut out: Vec<PasteMismatch> =
        [(ColorMode::Rgb, from_rgb, to_rgb, working.rgb_policy), (ColorMode::Cmyk, from_cmyk, to_cmyk, working.cmyk_policy)]
            .into_iter()
            .filter(|(_, from, to, _)| cms::canonical_name(from) != cms::canonical_name(to))
            .map(|(model, from, to, policy)| {
                let kind = if model == ColorMode::Rgb { cms::ProfileKind::Rgb } else { cms::ProfileKind::Cmyk };
                // Both profiles must be here to convert between them.
                let here = |name: &str| cms::profile(name).is_some_and(|p| p.kind == kind);
                let convertible = here(from) && here(&to);
                let default = if convertible { PasteProfile::of(policy) } else { PasteProfile::PreserveNumbers };
                PasteMismatch { model, from: from.clone(), to, default, convertible }
            })
            .collect();
    if out.is_empty() {
        return out;
    }
    // Without the artboards when their backgrounds don't paste.
    let without_boards;
    let clip = if !boards && !clip.artboards.is_empty() {
        without_boards = super::clipboard::Clipboard { artboards: vec![], ..clip.clone() };
        &without_boards
    } else {
        clip
    };
    // Which models the pasted colors use.
    let seen = std::cell::Cell::new((false, false));
    let _ = clip.with_colors(&|c: &Color| {
        let (rgb, cmyk) = seen.get();
        seen.set((rgb || matches!(c, Color::Rgb { .. }), cmyk || matches!(c, Color::Cmyk { .. })));
        *c
    });
    let (rgb, cmyk) = seen.get();
    out.retain(|m| if m.model == ColorMode::Rgb { rgb } else { cmyk });
    out
}

/// The clipboard `clip` as pasting it into `d` brings it: for each mismatched model
/// ([`paste_mismatches`]), its colors keep their numbers or convert from the source profile into
/// the destination's with the working intent, as `choice` says for that model (default: the
/// model's policy, [`PasteProfile::of`]). A source or destination profile that is not available
/// keeps the numbers. `boards`: as [`paste_mismatches`].
/// → the converted clipboard (`None` when nothing converts; its resources paste as another
/// document's) and a line for each mismatched model (`{model, clipboard, document, choice}`).
pub(crate) fn paste_profiles(
    clip: &super::clipboard::Clipboard,
    d: &vectorcraft_doc::Document,
    choice: Option<PasteChoice>,
    boards: bool,
) -> (Option<super::clipboard::Clipboard>, Vec<Value>) {
    let mismatches = paste_mismatches(clip, d, boards);
    let mut report = Vec::new();
    let (mut rgb_on, mut cmyk_on) = (false, false);
    for m in &mismatches {
        let picked = if m.convertible { choice.and_then(|c| c.of(m.model)).unwrap_or(m.default) } else { PasteProfile::PreserveNumbers };
        if picked == PasteProfile::Convert {
            match m.model {
                ColorMode::Rgb => rgb_on = true,
                ColorMode::Cmyk => cmyk_on = true,
            }
        }
        let mut line = m.to_json();
        if let Some(o) = line.as_object_mut() {
            o.remove("default");
            o.remove("convertible");
            o.insert("choice".into(), json!(picked.id()));
        }
        report.push(line);
    }
    if !rgb_on && !cmyk_on {
        return (None, report);
    }
    let Some([from_rgb, from_cmyk]) = &clip.source_profiles else { return (None, report) };
    let source = cms::for_document(Some(from_rgb), Some(from_cmyk));
    let dest = doc_cms(d);
    let intent = cms::working_settings().intent;
    let conv = |c: &Color| match c {
        Color::Rgb { .. } if rgb_on => source.convert_into(&dest, c, intent),
        Color::Cmyk { .. } if cmyk_on => source.convert_into(&dest, c, intent),
        _ => *c,
    };
    let mut converted = clip.with_colors(&conv);
    // Converted colors are not the source document's own: its resources of the same names are
    // compared with them, as another document's are.
    converted.colors_converted = true;
    (Some(converted), report)
}

/// The colors of `d` outside its art that [`convert_colors`] leaves, converted with `conv`: each
/// artboard's background, the transparency grid and simulated paper colors of Document Setup, and
/// the fill and stroke of the character and paragraph styles.
fn convert_setting_colors(d: &mut vectorcraft_doc::Document, conv: &dyn Fn(&Color) -> Color) {
    for c in d.artboards.iter_mut().filter_map(|a| a.color.as_mut()) {
        *c = conv(c);
    }
    for c in &mut d.setup.grid_colors {
        *c = conv(c);
    }
    convert_style_colors(d, conv);
}

/// The fill and stroke colors of `d`'s character and paragraph styles, converted with `conv`.
fn convert_style_colors(d: &mut vectorcraft_doc::Document, conv: &dyn Fn(&Color) -> Color) {
    // A tint of a global swatch is the converted swatch's color at its tint, which
    // [`convert_colors`] already gave it.
    let bases = global_bases(d);
    let mut f = |c: &Color, link: proof::Link| mode_color(c, link, &bases, conv);
    for styles in [&mut d.char_styles, &mut d.para_styles] {
        if !styles.iter().any(|s| s.attrs.contains_key("fill") || s.attrs.contains_key("stroke")) {
            continue;
        }
        for def in styles.iter_mut() {
            for key in ["fill", "stroke"] {
                let Some(value) = def.attrs.get_mut(key) else { continue };
                let Ok(mut paint) = serde_json::from_value::<Paint>(value.clone()) else { continue };
                map_paint_colors(&mut paint, &mut f);
                if let Ok(v) = serde_json::to_value(&paint) {
                    *value = v;
                }
            }
        }
    }
}

/// [`proof::map_node_colors`] over `n`'s subtree, then [`map_more_colors`].
fn map_art_colors(n: &mut vectorcraft_doc::Node, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    proof::map_node_colors(n, f);
    map_more_colors(n, f);
}

/// `f` applied to the colors in `n`'s subtree that [`proof::map_node_colors`] does not visit: the
/// effect colors ([`map_effect_colors`]), the Hue Shift key colors of strokes' own brush settings
/// ([`map_stroke_brush_keys`]), the series paints and label styles a graph keeps
/// ([`map_graph_colors`]) and the art of every opacity mask.
fn map_more_colors(n: &mut vectorcraft_doc::Node, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    map_effect_colors(&mut n.appearance, f);
    map_stroke_brush_keys(&mut n.appearance, f);
    if let Some(e) = n.extras_if_any() {
        if let Some(g) = e.graph.as_deref_mut() {
            map_graph_colors(g, f);
        }
        if let Some(mask) = e.mask.as_deref_mut() {
            map_art_colors(Arc::make_mut(&mut mask.art), f);
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut().filter(|c| !c.is_ruler_guide()) {
            map_more_colors(Arc::make_mut(c), f);
        }
    }
}

/// `f` applied to the Hue Shift key color of each stroke's own brush settings in appearance `a`
/// (Options of Selected Object, `colorization.key` of [`StrokeLayer::brush_options`]); a key
/// that does not read is left as it is.
///
/// [`StrokeLayer::brush_options`]: vectorcraft_doc::StrokeLayer::brush_options
fn map_stroke_brush_keys(a: &mut vectorcraft_doc::Appearance, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    for it in &mut a.items {
        let vectorcraft_doc::AppearanceItem::Stroke(st) = it else { continue };
        let Some(c) = st.brush_options.get_mut("colorization").and_then(Value::as_object_mut) else { continue };
        let Some(key) = c.get("key").and_then(|k| serde_json::from_value::<Color>(k.clone()).ok()) else { continue };
        let n = f(&key, None);
        if n != key {
            c.insert("key".into(), json!(n));
        }
    }
}

/// Whether art color `c` is the Hue Shift key color `key`, with the tolerance the brush's
/// colorization matches it with.
fn matches_key(c: &Color, key: &Color) -> bool {
    c.to_rgb().iter().zip(key.to_rgb()).map(|(a, b)| (a - b).abs()).sum::<f32>() < 0.02
}

/// `f` applied to the `color` of each effect in appearance `a` (the object's effects and those of
/// each fill and stroke) that has one ([`super::params::effect_color`]). An effect that draws its
/// default color stores the new color as its own. The light colors of the 3D effects stay as they
/// are.
fn map_effect_colors(a: &mut vectorcraft_doc::Appearance, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    let items = a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
    for e in a.effects.iter_mut().chain(items) {
        if !(e.params.is_object() || e.params.is_null()) {
            continue;
        }
        let Some(color) = super::params::effect_color(&e.id, &e.params) else { continue };
        let n = f(&color, None);
        if n == color {
            continue;
        }
        match &mut e.params {
            Value::Object(m) => {
                m.insert("color".into(), json!(n));
            }
            p => *p = json!({ "color": n }),
        }
    }
}

/// `f` applied to the series paints, the label styles and the line rows that graph `g` keeps, which
/// its marks, labels and lines take each time it is drawn again.
fn map_graph_colors(g: &mut vectorcraft_doc::GraphSpec, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    for p in g.series_paints.iter_mut() {
        for paint in [&mut p.fill, &mut p.stroke].into_iter().flatten() {
            map_paint_colors(paint, f);
        }
    }
    let styles = &mut g.label_styles;
    // A legend fill of the labels' own stays theirs when `f` makes it black. The default black legend fill stays black,
    // so the labels on dark pie wedges are still drawn white.
    styles.mark_own_legend_fill();
    let own = styles.legend_fill_is_own();
    for (st, fill) in [(&mut styles.values, true), (&mut styles.categories, true), (&mut styles.legend, own)] {
        let Some(st) = st else { continue };
        if fill {
            map_paint_colors(&mut st.fill, f);
        }
        map_paint_colors(&mut st.stroke, f);
    }
    for r in [&mut styles.values_rows, &mut styles.categories_rows, &mut styles.legend_rows, &mut styles.axis_rows, &mut styles.ring_rows]
        .into_iter()
        .flatten()
    {
        for paint in [&mut r.paint.fill, &mut r.paint.stroke].into_iter().flatten() {
            map_paint_colors(paint, f);
        }
    }
}

/// `f` applied to solid, gradient-stop and freeform-control colors, with any swatch link.
fn map_paint_colors(p: &mut Paint, f: &mut dyn FnMut(&Color, proof::Link) -> Color) {
    match p {
        Paint::Solid { color, swatch, tint } => *color = f(color, swatch.as_deref().map(|s| (s, *tint))),
        Paint::Gradient(g) => {
            for st in &mut g.gradient.stops {
                st.color = f(&st.color, st.swatch.as_deref().map(|s| (s, st.tint)));
            }
            if let Some(freeform) = &mut g.freeform {
                for point in &mut freeform.points {
                    point.color = f(&point.color, None);
                }
            }
        }
        _ => {}
    }
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
    let (model, vals) = match text_param(p, "to", C)?.unwrap_or("rgb") {
        "rgb" => ("rgb", json!(values(&c.convert(&col, Model::Rgb, intent)))),
        "cmyk" => ("cmyk", json!(values(&c.convert(&col, Model::Cmyk, intent)))),
        "gray" | "grayscale" => ("gray", json!(values(&c.convert(&col, Model::Gray, intent)))),
        "lab" => ("lab", json!([lab.l, lab.a, lab.b])),
        "oklch" => {
            // RGB numbers past 0..1 ([2, 2, 2]) display clipped, as `hex` reports them.
            let [l, ch, h] = vectorcraft_color::oklab::to_oklch(c.display_rgb(&col).map(|v| v.clamp(0.0, 1.0)));
            ("oklch", json!([l * 100.0, ch, h]))
        }
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
    let colors = p.get("colors").filter(|v| !v.is_null());
    if let Some(v) = colors.filter(|v| !v.is_array()) {
        return Err(bad("color.gamutCheck", format!("`colors` must be an array of colors, not {v}")));
    }
    if let Some(a) = colors.and_then(Value::as_array) {
        for v in a {
            cols.push(color_value(v).ok_or_else(|| bad("color.gamutCheck", format!("bad color {v}")))?);
        }
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
        "preserveNumbers": v.setup.preserve_numbers,
        "simulateBlack": v.setup.simulate_black,
        "proofColors": v.proof_colors,
        "overprintPreview": v.overprint,
        "separations": v.separations,
    })
}

fn proof_setup(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.proofSetup";
    let mut v = proof::view();
    setup_params(p, &mut v.setup, C)?;
    if let Some(on) = on_param(p, "proof", C)? {
        v.proof_colors = on;
    }
    store_view(s, v.clone());
    Ok(view_json(&v))
}

/// Take the proof setup settings `p` gives (`target`, `intent`, `simulatePaper`, `preserveNumbers`
/// and `simulateBlack`, as `view.proofSetup` reads them) into `setup`. A `cmyk:` or `rgb:` target
/// must name a CMYK or RGB profile that is loaded.
fn setup_params(p: &Value, setup: &mut cms::ProofSetup, cmd: &str) -> Result<()> {
    if let Some(t) = text_param(p, "target", cmd)? {
        let t = ProofTarget::parse(t).ok_or_else(|| bad(cmd, format!("unknown proof target `{t}`")))?;
        if let ProofTarget::Cmyk(name) = &t
            && !cms::profile(name).is_some_and(|k| k.kind == cms::ProfileKind::Cmyk)
        {
            return Err(bad(cmd, format!("unknown CMYK profile `{name}`")));
        }
        if let ProofTarget::Rgb(name) = &t
            && !cms::profile(name).is_some_and(|k| k.kind == cms::ProfileKind::Rgb)
        {
            return Err(bad(cmd, format!("unknown RGB profile `{name}`")));
        }
        setup.target = t;
    }
    if let Some(i) = intent_param(p, cmd)? {
        setup.intent = i;
    }
    setup.simulate_paper = switch_or(p, "simulatePaper", setup.simulate_paper, cmd)?;
    setup.preserve_numbers = switch_or(p, "preserveNumbers", setup.preserve_numbers, cmd)?;
    setup.simulate_black = switch_or(p, "simulateBlack", setup.simulate_black, cmd)?;
    Ok(())
}

/// Make `v` the proof view and redraw every document. When `v` equals the current view, the view
/// and the revisions stay as they are; Separations Preview's visible plates are compared as a set.
fn store_view(s: &mut Session, v: proof::ProofView) {
    let shown = |v: &proof::ProofView| proof::ProofView {
        separations: v.separations.as_ref().map(|vis| {
            let mut vis = vis.clone();
            vis.sort();
            vis.dedup();
            vis
        }),
        ..v.clone()
    };
    if shown(&v) != shown(&proof::view()) {
        proof::set_view(v);
        touch_all(s);
    }
}

fn toggle_view(s: &mut Session, p: &Value, cmd: &str, f: impl FnOnce(&mut proof::ProofView, Option<bool>)) -> Result<Value> {
    let on = on_param(p, "on", cmd)?;
    let mut v = proof::view();
    f(&mut v, on);
    store_view(s, v.clone());
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
    if let Some(on) = on_param(p, "on", C)? {
        v.separations = if on { Some(v.separations.take().unwrap_or_else(|| all.clone())) } else { None };
    }
    match p.get("plates") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => {
            let mut vis: Vec<String> = Vec::new();
            for n in a.iter().filter_map(Value::as_str) {
                let n = check(n)?;
                // A plate named twice is shown once.
                if !vis.contains(&n) {
                    vis.push(n);
                }
            }
            v.separations = Some(vis);
        }
        Some(_) => {
            let key = "plates";
            return Err(bad(C, format!("`{key}` must be an array of these names: {}", all.join(", "))));
        }
    }
    if let Some(n) = text_param(p, "only", C)? {
        v.separations = Some(vec![check(n)?]);
    }
    // The panel's Alt-click on a plate's eye: that plate alone, or every plate when it already
    // shows alone among the active document's plates.
    if let Some(n) = text_param(p, "soloToggle", C)? {
        let n = check(n)?;
        let alone = v.separations.as_ref().is_some_and(|vis| all.iter().filter(|a| vis.contains(a)).eq([&n]));
        v.separations = Some(if alone { all.clone() } else { vec![n] });
    }
    if let Some(n) = text_param(p, "toggle", C)? {
        let n = check(n)?;
        let vis = v.separations.get_or_insert_with(|| all.clone());
        if let Some(i) = vis.iter().position(|x| *x == n) {
            vis.remove(i);
        } else {
            vis.push(n);
        }
    }
    store_view(s, v);
    Ok(plates_json(s))
}

fn set_spot(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.setSpot";
    let name = text_param(p, "name", C)?.ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let spot = switch_or(p, "spot", true, C)?;
    let sw = s.doc()?.doc.swatch(&name).ok_or_else(|| bad(C, format!("no swatch `{name}`")))?;
    if !matches!(sw.paint, Paint::Solid { .. }) {
        return Err(bad(C, "only solid-colour swatches can be spot colours"));
    }
    if sw.spot != spot {
        let n = name.clone();
        let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
        s.edit("Swatch Options", |d, _| {
            if let Some(w) = d.swatch_mut(&n) {
                w.spot = spot;
                if spot {
                    w.global = true;
                }
            }
            super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad(C, e))
        })?;
    }
    Ok(json!({"name": name, "spot": spot}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policies come from the settings snapshot the question was built from, not from the
    /// working settings as they are when the policies run.
    #[test]
    fn open_policies_use_the_snapshot_they_are_given() {
        let tagged = |d: &mut vectorcraft_doc::Document| d.color_profiles.rgb = Some(cms::canonical_name(cms::WIDE_GAMUT_RGB).to_string());
        let snapshot = ColorSettings { rgb_policy: Policy::Off, ..ColorSettings::default() };
        let mut d = vectorcraft_doc::Document::new(100.0, 100.0);
        tagged(&mut d);
        let (notes, changed) = apply_open_policies_choosing(&mut d, None, &snapshot).unwrap();
        assert!(changed, "the Off policy of the snapshot takes the profile off");
        assert_eq!(d.color_profiles.rgb, None);
        assert!(notes.iter().any(|n| n.contains("policy is")), "{notes:?}");
    }
}
