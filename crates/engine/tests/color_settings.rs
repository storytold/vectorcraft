//! Edit → Color Settings and Assign Profile with non-default profiles. The Color Settings are
//! process-wide and every colour shown or rendered goes through them, so these tests run in their
//! own test binary (process) instead of next to the engine's unit tests, which would see the
//! switched working spaces mid-test; here they take turns.

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::{Color, cms};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::colormgmt::{PROFILES_KEY, ProfileChoice, ProfileIssue, doc_profiles};
use vectorcraft_engine::store::MAX_PRESET_FILE;
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_testkit::fixtures::{exec, id_of, rect, select, session};

/// The active Color Settings: one test at a time.
fn settings() -> MutexGuard<'static, ()> {
    static SETTINGS: Mutex<()> = Mutex::new(());
    SETTINGS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The working Color Settings as a test found them, put back when it ends (a failing one too)
/// before the next test takes its turn.
struct Restored(cms::ColorSettings, #[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Restored {
    fn drop(&mut self) {
        let _ = cms::set_active(&self.0);
    }
}

/// [`settings`], with the working Color Settings put back afterwards.
fn restored_settings() -> Restored {
    let g = settings();
    Restored(cms::working_settings(), g)
}

#[test]
fn color_settings_lists_profiles_and_validates() {
    let _g = settings();
    let mut s = session();
    let r = s.execute("edit.colorSettings", &json!({})).unwrap();
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::GENERIC_CMYK) && names.contains(&cms::SRGB) && names.contains(&cms::DEVICE_CMYK));
    assert!(s.execute("edit.colorSettings", &json!({"cmyk": "Missing.icc"})).is_err());
    assert_eq!(cms::active_settings().cmyk, r["cmyk"].as_str().unwrap(), "failed change leaves settings alone");
    let before = r["bpc"].as_bool().unwrap();
    let r2 = s.execute("edit.colorSettings", &json!({"bpc": !before})).unwrap();
    assert_eq!(r2["bpc"], !before);
    s.execute("edit.colorSettings", &json!({"bpc": before})).unwrap();
    assert!(s.execute("color.loadProfile", &json!({"path": "/nonexistent/x.icc"})).is_err());
}

#[test]
fn legacy_profile_name_resolves_in_commands_and_files() {
    let _g = settings();
    let (old, _) = cms::LEGACY_NAMES[0];
    let mut s = session();
    let before = cms::active_settings();
    let r = s.execute("edit.colorSettings", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB);
    assert_eq!(cms::active_settings().rgb, cms::WIDE_GAMUT_RGB);
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::WIDE_GAMUT_RGB) && !names.contains(&old));
    let r = s.execute("edit.assignProfile", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB, "stored under the current name");
    // A document saved by an older version names the profile the old way.
    let mut d = (*s.doc().unwrap().doc).clone();
    d.unknown.insert(PROFILES_KEY.into(), json!({"rgb": old, "cmyk": null}));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(doc_profiles(&back), (Some(cms::WIDE_GAMUT_RGB.to_string()), None));
    cms::set_active(&before).unwrap();
}

/// Assigning a profile other than the working space replaces the active color settings and raises
/// the other open documents' revisions.
#[test]
fn assigning_another_profile_replaces_the_active_color_settings() {
    let _g = settings();
    let mut s = session();
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let before = cms::active_settings();
    assert_ne!(before.cmyk, cms::DEVICE_CMYK);
    let active = cms::active();
    let other = s.documents()[0].revision;
    exec(&mut s, "edit.assignProfile", json!({"cmyk": cms::DEVICE_CMYK}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK);
    assert!(!std::sync::Arc::ptr_eq(&active, &cms::active()), "the active color settings were not replaced");
    assert!(s.documents()[0].revision > other, "the other document's revision did not change");
    cms::set_active(&before).unwrap();
}

/// Assign Profile tags the document and leaves the working spaces: Color Settings shows the
/// working spaces, a later change to it saves them and not the assigned profile, and taking the
/// profile off the document brings the working spaces back.
#[test]
fn assign_profile_leaves_the_working_spaces() {
    let _g = settings();
    let dir = fresh_dir("assign-profile");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    let file = dir.join("Color Settings.json");
    let mut s = session();
    s.attach_color_settings(file.clone());
    let working = cms::working_settings();
    assert_ne!(working.cmyk, cms::DEVICE_CMYK);
    exec(&mut s, "edit.assignProfile", json!({"cmyk": cms::DEVICE_CMYK}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "the document draws through its profile");
    assert_eq!(cms::working_settings(), working);
    assert!(!file.exists(), "Assign Profile saves no Color Settings");
    let r = exec(&mut s, "edit.colorSettings", json!({}));
    assert_eq!(r["cmyk"], working.cmyk.as_str(), "Color Settings shows the working CMYK space");
    exec(&mut s, "edit.colorSettings", json!({"intent": "perceptual"}));
    let saved: cms::ColorSettings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!((saved.cmyk.as_str(), saved.intent), (working.cmyk.as_str(), cms::Intent::Perceptual));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "the document still draws through its profile");
    exec(&mut s, "edit.assignProfile", json!({"cmyk": null}));
    assert_eq!(cms::active_settings(), cms::working_settings(), "no profile: the working spaces");
}

/// A loaded profile that is assigned but is not a working space can be removed: the working space
/// takes its place in the active settings, and the working spaces stay as they are.
#[test]
fn removing_an_assigned_profile_puts_the_working_space_back() {
    const NAME: &str = "Assigned CMYK";
    let _g = settings();
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![] };
    cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some(NAME.into())).unwrap();
    let mut s = session();
    let working = cms::working_settings();
    exec(&mut s, "edit.assignProfile", json!({"cmyk": NAME}));
    assert_eq!(cms::active_settings().cmyk, NAME);
    let revision = s.doc().unwrap().revision;
    let r = exec(&mut s, "color.removeProfile", json!({"name": NAME}));
    assert_eq!(r["removed"], NAME);
    assert_eq!(cms::active_settings(), working, "the working CMYK space is in use again");
    assert_eq!(cms::working_settings(), working);
    assert!(s.doc().unwrap().revision > revision, "the canvas redraws");
}

/// Loading the file of an assigned profile in use again keeps it in use, loading it after it was
/// removed puts it back in use, and the working spaces stay as they are.
#[test]
fn loading_an_assigned_profile_again_keeps_it_in_use() {
    const NAME: &str = "Assigned P";
    let _g = settings();
    let dir = fresh_dir("reload-assigned");
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("assigned.icc");
    std::fs::write(&path, icc_named(NAME)).unwrap();
    let mut s = session();
    let working = cms::working_settings();
    let load = |s: &mut Session| assert_eq!(exec(s, "color.loadProfile", json!({"path": path.to_str().unwrap()}))["name"], NAME);
    load(&mut s);
    exec(&mut s, "edit.assignProfile", json!({"rgb": NAME}));
    let before = cms::active();
    load(&mut s);
    assert_eq!(cms::active_settings().rgb, NAME, "the assigned profile stays in use");
    assert!(!std::sync::Arc::ptr_eq(&before, &cms::active()), "the new file is in use");
    exec(&mut s, "color.removeProfile", json!({"name": NAME}));
    assert_eq!(cms::active_settings(), working);
    load(&mut s);
    assert_eq!(cms::active_settings().rgb, NAME, "loaded again, the assigned profile is in use");
    assert_eq!(cms::working_settings(), working);
}

/// A document behind the front one draws through its own assigned profile: loading that profile's
/// file again or removing it redraws the document, and the front document stays as it was.
#[test]
fn loading_or_removing_a_back_documents_profile_redraws_it() {
    const NAME: &str = "Back RGB P";
    let _g = settings();
    let dir = fresh_dir("back-assigned");
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("back.icc");
    std::fs::write(&path, icc_named(NAME)).unwrap();
    let mut s = session();
    let load = |s: &mut Session| assert_eq!(exec(s, "color.loadProfile", json!({"path": path.to_str().unwrap()}))["name"], NAME);
    load(&mut s);
    exec(&mut s, "edit.assignProfile", json!({"rgb": NAME}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let revisions = |s: &Session| s.documents().iter().map(|d| d.revision).collect::<Vec<_>>();
    let before = revisions(&s);
    load(&mut s);
    let loaded = revisions(&s);
    assert!(loaded[0] > before[0], "the back document assigned the profile redraws after a load");
    assert_eq!(loaded[1], before[1], "the front document does not use it");
    exec(&mut s, "color.removeProfile", json!({"name": NAME}));
    let removed = revisions(&s);
    assert!(removed[0] > loaded[0], "the back document assigned the profile redraws after a removal");
    assert_eq!(removed[1], loaded[1], "the front document does not use it");
}

/// A batch that assigns a profile and then changes the color mode, as Document Setup's OK does,
/// converts the colors through the profile it assigns.
#[test]
fn a_batch_converts_through_the_profile_it_assigns() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let blue = |s: &mut Session| {
        let id = rect(s, 10.0, 10.0, 50.0, 50.0);
        exec(s, "paint.setFill", json!({"ids": [id.0], "color": "#3366cc"}));
        id
    };
    let fill = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color();
    let mut working = session();
    let w = blue(&mut working);
    exec(&mut working, "file.documentColorMode", json!({"mode": "cmyk"}));
    let mut tagged = session();
    let a = blue(&mut tagged);
    let steps = json!([
        {"command": "edit.assignProfile", "params": {"cmyk": cms::DEVICE_CMYK}},
        {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
    ]);
    exec(&mut tagged, "command.batch", json!({"commands": steps}));
    let converted = fill(&tagged, a);
    assert_ne!(converted, fill(&working, w), "converted through the working CMYK space");
    let mut device = session();
    let b = blue(&mut device);
    exec(&mut device, "edit.colorSettings", json!({"cmyk": cms::DEVICE_CMYK}));
    exec(&mut device, "file.documentColorMode", json!({"mode": "cmyk"}));
    assert_eq!(converted, fill(&device, b));
}

/// The profiles a document is assigned are in use while it is in front: a new or opened document
/// assigned none draws through the working spaces, switching back to the assigned document puts its
/// profile in use again, and closing a document puts the profiles of the document left in front, or
/// with none open the working spaces, in use.
#[test]
fn assigned_profiles_follow_the_front_document() {
    let _g = settings();
    let dir = fresh_dir("front-profiles");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let tagged = dir.join("tagged.vectorcraft");
    let mut s = session();
    let working = cms::working_settings();
    assert_ne!(working.cmyk, cms::DEVICE_CMYK);
    exec(&mut s, "edit.assignProfile", json!({"cmyk": cms::DEVICE_CMYK}));
    exec(&mut s, "file.saveAs", json!({"path": tagged.to_str().unwrap()}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    assert_eq!(cms::active_settings(), working, "a new document draws through the working spaces");
    exec(&mut s, "document.activate", json!({"index": 0}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "back in front, the document draws through its profile");
    assert!(s.set_active(1));
    assert_eq!(cms::active_settings(), working, "a tab switch to the untagged document");
    exec(&mut s, "file.close", json!({}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "the document left in front draws through its profile");
    exec(&mut s, "file.close", json!({}));
    assert_eq!(cms::active_settings(), working, "no document open: the working spaces");
    exec(&mut s, "document.open", json!({"path": tagged.to_str().unwrap()}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "an opened document draws through its saved profile");
}

/// Undo and Redo of Assign Profile, and File › Revert to a file assigned no profile, put the
/// profiles the document has then in use.
#[test]
fn undo_redo_and_revert_put_the_document_profiles_in_use() {
    let _g = settings();
    let dir = fresh_dir("undo-profiles");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let untagged = dir.join("untagged.vectorcraft");
    let mut s = session();
    let working = cms::working_settings();
    exec(&mut s, "file.saveAs", json!({"path": untagged.to_str().unwrap()}));
    exec(&mut s, "edit.assignProfile", json!({"cmyk": cms::DEVICE_CMYK}));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK);
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(doc_profiles(&s.doc().unwrap().doc), (None, None));
    assert_eq!(cms::active_settings(), working, "undone: the working spaces");
    exec(&mut s, "edit.redo", json!({}));
    assert_eq!(doc_profiles(&s.doc().unwrap().doc), (None, Some(cms::DEVICE_CMYK.to_string())));
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "redone: the assigned profile");
    exec(&mut s, "file.revert", json!({}));
    assert_eq!(doc_profiles(&s.doc().unwrap().doc), (None, None));
    assert_eq!(cms::active_settings(), working, "reverted: the working spaces");
}

/// A batch that assigns a profile and then fails puts the document back without it, and the
/// working spaces back in use.
#[test]
fn a_failed_batch_puts_the_profiles_in_use_back() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let working = cms::working_settings();
    let steps = json!([
        {"command": "edit.assignProfile", "params": {"cmyk": cms::DEVICE_CMYK}},
        {"command": "document.activate", "params": {"index": 99}},
    ]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert_eq!(doc_profiles(&s.doc().unwrap().doc), (None, None));
    assert_eq!(cms::active_settings(), working);
}

/// A batch step that switches documents makes the next step convert through the profiles of the
/// document it switched to, not those of the document the batch began in.
#[test]
fn a_batch_converts_the_document_it_switches_to_through_its_profiles() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let blue = |s: &mut Session| {
        let id = rect(s, 10.0, 10.0, 50.0, 50.0);
        exec(s, "paint.setFill", json!({"ids": [id.0], "color": "#3366cc"}));
        id
    };
    let fill = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color();
    let mut working = session();
    let w = blue(&mut working);
    exec(&mut working, "file.documentColorMode", json!({"mode": "cmyk"}));
    let mut s = session();
    let b = blue(&mut s);
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    exec(&mut s, "edit.assignProfile", json!({"cmyk": cms::DEVICE_CMYK}));
    let steps = json!([
        {"command": "document.activate", "params": {"index": 0}},
        {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
    ]);
    exec(&mut s, "command.batch", json!({"commands": steps}));
    assert_eq!(s.active_index(), Some(0));
    assert_eq!(fill(&s, b), fill(&working, w), "converted through the working CMYK space");
}

/// View › Proof Setup › Working CMYK simulates the working CMYK space of Color Settings in a
/// document assigned another CMYK profile, in Proof Colors and in Separations Preview, and a change
/// of the working CMYK space shows in the next proof while the document keeps its profile.
#[test]
fn working_cmyk_proofs_a_tagged_document_through_the_working_space() {
    const NAME: &str = "Tagged CMYK";
    let _g = settings();
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![] };
    cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some(NAME.into())).unwrap();
    let proof = cms::ProofSetup::default();
    assert_eq!(proof.target, cms::ProofTarget::WorkingCmyk);
    let (blue, inks) = ([0.2, 0.4, 0.8], [0.6, 0.3, 0.0, 0.1]);
    let mut s = session();
    exec(&mut s, "edit.assignProfile", json!({"cmyk": NAME}));
    let (active, working) = (cms::active(), cms::working());
    assert_eq!(active.settings().cmyk, NAME);
    assert_eq!(active.proof_srgb(blue, &proof), working.proof_srgb(blue, &proof), "Proof Colors");
    assert_eq!(active.proof_cmyk_to_srgb(inks, &proof), working.proof_cmyk_to_srgb(inks, &proof), "Separations Preview");
    let _ = active.proof_lut(&proof);
    exec(&mut s, "edit.colorSettings", json!({"cmyk": cms::DEVICE_CMYK}));
    let (active, working) = (cms::active(), cms::working());
    assert_eq!(active.settings().cmyk, NAME, "the document keeps its profile");
    let lut = active.proof_lut(&proof);
    let fresh = cms::ProofLut::build(|rgb| working.proof_srgb(rgb, &proof));
    for px in [[51, 102, 204], [230, 30, 30], [20, 160, 40], [128, 128, 128]] {
        assert_eq!(lut.apply8(px), fresh.apply8(px), "the proof of {px:?} through the new working CMYK space");
    }
}

/// Separations Preview follows the Color Settings and the profiles loaded: a renderer draws a gray
/// square on the Black plate through the new working CMYK profile, and the four process plates
/// through a proof profile loaded again under the same name, as a new renderer draws them.
#[test]
fn separations_preview_draws_through_new_color_settings() {
    let _g = settings();
    let mut s = session();
    let gray = rect(&mut s, 20.0, 20.0, 160.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [gray.0], "color": "#808080"}));
    // The proof and Overprint Preview options the canvas takes from the view state the
    // commands set.
    let canvas = || RenderOptions {
        proof: vectorcraft_render::proof::active_proof(),
        overprint_preview: vectorcraft_render::proof::overprint_preview_on(),
        background: Some([255, 255, 255, 255]),
        ..Default::default()
    };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &canvas()).pixels;
    exec(&mut s, "view.separationsPreview", json!({"only": "Black"}));
    let mut r = Renderer::new();
    let before = frame(&mut r, &s);
    let old = cms::active_settings();
    s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let fresh = frame(&mut Renderer::new(), &s);
    let warm = frame(&mut r, &s);
    cms::set_active(&old).unwrap();
    assert!(fresh != before, "the new profile changes the plate");
    assert!(warm == fresh);
    let name = "Separations Proof CMYK";
    cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some(name.into())).unwrap();
    exec(&mut s, "view.proofSetup", json!({"target": format!("cmyk:{name}")}));
    exec(&mut s, "view.separationsPreview", json!({"plates": ["Cyan", "Magenta", "Yellow", "Black"]}));
    let before = frame(&mut r, &s);
    cms::register_icc(&cms::icc_bytes(cms::DEVICE_CMYK).unwrap(), Some(name.into())).unwrap();
    let fresh = frame(&mut Renderer::new(), &s);
    let warm = frame(&mut r, &s);
    exec(&mut s, "view.proofSetup", json!({"target": "workingCmyk"}));
    exec(&mut s, "view.separationsPreview", json!({"on": false}));
    assert!(fresh != before, "the profile loaded again changes the plates");
    assert!(warm == fresh);
}

/// Raster effects follow the Color Settings: a blurred CMYK fill, and a white square's drop shadow
/// in a CMYK color, are drawn again through the new working CMYK profile, in a frame drawn while
/// interacting (as during a dialog's preview) and after it.
#[test]
fn raster_effects_draw_through_new_color_settings() {
    let _g = settings();
    let mut s = session();
    let blurred = rect(&mut s, 20.0, 20.0, 160.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blurred.0], "color": {"c": 0, "m": 60, "y": 100, "k": 0}}));
    exec(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "blur.gaussian", "params": {"radius": 2}}));
    let lit = rect(&mut s, 220.0, 20.0, 120.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [lit.0], "color": "#ffffff"}));
    let cyan = json!({"model": "cmyk", "c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0});
    let shadow = json!({"x": 8, "y": 8, "blur": 1, "opacity": 100, "mode": "normal", "color": cyan});
    exec(&mut s, "effect.apply", json!({"ids": [lit.0], "effect": "stylize.dropShadow", "params": shadow}));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let live = RenderOptions { interacting: true, ..opts.clone() };
    let frame = |r: &mut Renderer, s: &Session, o: &RenderOptions| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, o).pixels;
    let mut r = Renderer::new();
    let before = frame(&mut r, &s, &opts);
    let old = cms::active_settings();
    s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let fresh = frame(&mut Renderer::new(), &s, &opts);
    assert!(fresh != before, "the new profile changes the frame");
    assert!(frame(&mut r, &s, &live) == fresh, "while interacting");
    assert!(frame(&mut r, &s, &opts) == fresh, "after it");
    cms::set_active(&old).unwrap();
}

/// Pattern tiles follow the Color Settings: a fill of a pattern in a CMYK color is drawn through
/// the new working CMYK profile in the next frame, as a renderer on a new thread (with no tiles of
/// its own yet) draws it.
#[test]
fn pattern_tiles_draw_through_new_color_settings() {
    use std::sync::Arc;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::Rect;
    use vectorcraft_geom::shapes::rectangle;
    let _g = settings();
    let old = cms::active_settings();
    let mut d = Document::new(200.0, 100.0);
    let cyan = Appearance::basic(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)), Paint::None, 0.0);
    let square = Node::path(d.alloc_id(), rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), cyan);
    let mut def = PatternDef::new("Checks", vec![Arc::new(square)]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(def);
    let checks = Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0);
    let filled = Node::path(d.alloc_id(), rectangle(Rect::new(10.0, 10.0, 190.0, 90.0)), checks);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, filled).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, d: &Document| r.render(d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    let before = frame(&mut r, &d);
    session().execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let warm = frame(&mut r, &d);
    let fresh = std::thread::scope(|t| t.spawn(|| frame(&mut Renderer::new(), &d)).join().unwrap());
    cms::set_active(&old).unwrap();
    assert!(fresh != before, "the new profile changes the frame");
    assert!(warm == fresh, "drawn as a renderer with no tiles draws it");
}

/// Live objects follow the Color Settings: in the next frame, a blend of an RGB red and a CMYK cyan,
/// the same blend in an envelope, an envelope that distorts the appearance of a CMYK cyan rectangle
/// with Revolve, and a gradient mesh in CMYK colors draw through the new working CMYK profile, and
/// so do the blend and the mesh with a drop shadow.
#[test]
fn live_objects_draw_through_new_color_settings() {
    let _g = settings();
    let old = cms::active_settings();
    let cyan = json!({"c": 100, "m": 0, "y": 0, "k": 0});
    let blend = |s: &mut Session| {
        let a = rect(s, 20.0, 40.0, 60.0, 120.0);
        exec(s, "paint.setFill", json!({"ids": [a.0], "color": "#ff0000"}));
        let b = rect(s, 300.0, 40.0, 60.0, 120.0);
        exec(s, "paint.setFill", json!({"ids": [b.0], "color": cyan}));
        id_of(&exec(s, "object.blend.make", json!({"ids": [a.0, b.0], "steps": 5})))
    };
    let mut blended = session();
    blend(&mut blended);
    let mut enveloped = session();
    let b = blend(&mut enveloped);
    exec(&mut enveloped, "object.envelope.makeWithWarp", json!({"ids": [b.0], "style": "arc", "bend": 30}));
    let mut revolved = session();
    let profile = rect(&mut revolved, 150.0, 60.0, 60.0, 120.0);
    exec(&mut revolved, "paint.setFill", json!({"ids": [profile.0], "color": cyan}));
    exec(&mut revolved, "effect.apply", json!({"ids": [profile.0], "effect": "threeD.revolve"}));
    let e = id_of(&exec(&mut revolved, "object.envelope.makeWithWarp", json!({"ids": [profile.0], "style": "arc", "bend": 30})));
    select(&mut revolved, &[e]);
    exec(&mut revolved, "object.envelope.options", json!({"distortAppearance": true}));
    let mesh = |s: &mut Session| {
        let m = rect(s, 20.0, 20.0, 360.0, 260.0);
        exec(s, "paint.setFill", json!({"ids": [m.0], "color": cyan}));
        exec(s, "object.mesh.create", json!({"ids": [m.0], "rows": 2, "cols": 2}));
        exec(s, "object.mesh.setPointColor", json!({"id": m.0, "index": 4, "color": {"c": 0, "m": 100, "y": 0, "k": 0}}));
        m
    };
    let mut meshed = session();
    mesh(&mut meshed);
    let shadow = json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
    let mut shadowed_blend = session();
    let b = blend(&mut shadowed_blend);
    exec(&mut shadowed_blend, "effect.apply", json!({"ids": [b.0], "effect": "stylize.dropShadow", "params": shadow}));
    let mut shadowed_mesh = session();
    let m = mesh(&mut shadowed_mesh);
    exec(&mut shadowed_mesh, "effect.apply", json!({"ids": [m.0], "effect": "stylize.dropShadow", "params": shadow}));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &opts).pixels;
    for (what, mut s) in [
        ("the blend", blended),
        ("the envelope", enveloped),
        ("the revolved rectangle", revolved),
        ("the mesh", meshed),
        ("the blend with a drop shadow", shadowed_blend),
        ("the mesh with a drop shadow", shadowed_mesh),
    ] {
        let mut r = Renderer::new();
        let before = frame(&mut r, &s);
        s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
        let fresh = frame(&mut Renderer::new(), &s);
        let warm = frame(&mut r, &s);
        cms::set_active(&old).unwrap();
        assert!(fresh != before, "{what}: the new profile changes the frame");
        assert!(warm == fresh, "{what}: drawn as a new renderer draws it");
    }
}

/// The art drawn for 3D effects and color adjustments follows the Color Settings: in the next
/// frame, a CMYK cyan rectangle with Revolve, with Extrude & Bevel, with Inflate, with Rotate, with
/// Brightness/Contrast, and a group holding it with Brightness/Contrast, draw through the new
/// working CMYK profile.
#[test]
fn effect_art_draws_through_new_color_settings() {
    let _g = settings();
    let old = cms::active_settings();
    let cyan = json!({"c": 100, "m": 0, "y": 0, "k": 0});
    let brighter = json!({"brightness": 30});
    let cases = [
        ("the revolved rectangle", "threeD.revolve", json!({}), false),
        ("the extruded rectangle", "threeD.extrude", json!({}), false),
        ("the inflated rectangle", "threeD.inflate", json!({}), false),
        ("the rotated rectangle", "threeD.rotate", json!({}), false),
        ("the adjusted rectangle", "adjust.brightnessContrast", brighter.clone(), false),
        ("the adjusted group", "adjust.brightnessContrast", brighter, true),
    ];
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &opts).pixels;
    for (what, effect, params, grouped) in cases {
        let mut s = session();
        let shape = rect(&mut s, 150.0, 60.0, 60.0, 120.0);
        exec(&mut s, "paint.setFill", json!({"ids": [shape.0], "color": cyan}));
        let target = if grouped { id_of(&exec(&mut s, "object.group", json!({"ids": [shape.0]}))) } else { shape };
        exec(&mut s, "effect.apply", json!({"ids": [target.0], "effect": effect, "params": params}));
        let mut r = Renderer::new();
        let before = frame(&mut r, &s);
        s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
        let fresh = frame(&mut Renderer::new(), &s);
        let warm = frame(&mut r, &s);
        cms::set_active(&old).unwrap();
        assert!(fresh != before, "{what}: the new profile changes the frame");
        assert!(warm == fresh, "{what}: drawn as a new renderer draws it");
    }
}

/// #2276: with a working RGB space other than sRGB, the Eyedropper takes an image's sRGB pixel
/// into the working space, so the sampled fill shows as the pixel it was picked from.
#[test]
fn the_eyedropper_takes_an_image_pixel_into_the_working_rgb_space() {
    use vectorcraft_color::Color;
    use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeKind};
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let _g = settings();
    let old = cms::active_settings();
    let mut s = session();
    let img = image::RgbaImage::from_raw(1, 1, vec![83, 170, 223, 255]).unwrap();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let xf = Affine::translate((300.0, 0.0)) * Affine::scale(100.0);
        let im = ImageObject { key: "px".into(), width: 1, height: 1, xf, link: None, placement: Default::default(), pixels: None };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).map_err(|e| vectorcraft_engine::EngineError::Other(e.to_string()))
    })
    .unwrap();
    let dst = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    s.execute("edit.colorSettings", &json!({"rgb": cms::WIDE_GAMUT_RGB})).unwrap();
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 350.0, 50.0), ViewInfo::default()).unwrap();
    let got = s.doc().unwrap().doc.node(dst).unwrap().appearance.fill_paint().color();
    let shown = got.map(|c| c.to_rgba8(1.0));
    cms::set_active(&old).unwrap();
    assert!(matches!(got, Some(Color::Rgb { .. })), "{got:?}");
    let [r, g, b, _] = shown.unwrap_or_default();
    assert!(r.abs_diff(83) <= 1 && g.abs_diff(170) <= 1 && b.abs_diff(223) <= 1, "{shown:?} for 83/170/223");
}

/// With a working RGB space other than sRGB, a freeform gradient of RGB points expands to mesh
/// points with the points' own RGB values, and points in different models expand to mesh points
/// that show the colors the gradient shows there.
#[test]
fn a_freeform_gradient_expands_with_its_rgb_values_in_the_working_rgb_space() {
    use vectorcraft_color::freeform::spread_scale;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::NodeKind;
    use vectorcraft_geom::Rect;
    let _g = settings();
    let old = cms::active_settings();
    cms::set_active(&cms::ColorSettings { rgb: cms::WIDE_GAMUT_RGB.into(), ..old.clone() }).unwrap();
    // Each mesh point's color, the display color it shows and the display color the gradient
    // shows at its place.
    let expand = |colors: [serde_json::Value; 4]| {
        let mut s = session();
        let id = rect(&mut s, 20.0, 20.0, 160.0, 160.0);
        select(&mut s, &[id]);
        exec(&mut s, "paint.setFill", json!({"gradient": {"kind": "freeform"}}));
        exec(&mut s, "paint.setStroke", json!({"none": true}));
        for (i, c) in colors.into_iter().enumerate() {
            exec(&mut s, "paint.freeform.setPoint", json!({"index": i, "color": c, "stroke": false}));
        }
        let b = Rect::new(20.0, 20.0, 180.0, 180.0);
        let field = match s.doc().unwrap().doc.node(id).map(|n| n.appearance.fill_paint()) {
            Some(Paint::Gradient(g)) => g.freeform_on(b).field(spread_scale(b)),
            p => panic!("a gradient fill: {p:?}"),
        };
        exec(&mut s, "object.expand", json!({}));
        let d = &s.doc().unwrap();
        let mesh = d.selection.objects.first().and_then(|id| d.doc.node(*id)).map(|n| n.kind.clone());
        let Some(NodeKind::Mesh(m)) = mesh else { panic!("a mesh: {mesh:?}") };
        m.points.iter().map(|p| (p.color, p.color.to_rgb(), field.sample(p.p).0)).collect::<Vec<_>>()
    };
    let rgb = expand([json!("#3399e6"), json!("#3399e6"), json!("#3399e6"), json!("#3399e6")]);
    let mixed = expand([json!("#3399e6"), json!("#3399e6"), json!({"gray": 0.5}), json!({"gray": 0.5})]);
    cms::set_active(&old).unwrap();
    let want = Color::rgb8(0x33, 0x99, 0xe6);
    for (c, ..) in &rgb {
        let (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) = (*c, want) else { panic!("an RGB point: {c:?}") };
        assert!((r - r2).abs() < 1e-5 && (g - g2).abs() < 1e-5 && (b - b2).abs() < 1e-5, "{c:?} for {want:?}");
    }
    for (c, shown, field) in &mixed {
        let close = shown.iter().zip(field).all(|(a, b)| (a - b).abs() < 2.0 / 255.0);
        assert!(matches!(c, Color::Rgb { .. }) && close, "{c:?} shows {shown:?} for {field:?}");
    }
}

/// With a working RGB space other than sRGB, RGB freeform points mix in that space on the canvas
/// and in a writer, as Object > Expand mixes them.
#[test]
fn freeform_rgb_points_mix_in_the_working_rgb_space() {
    use vectorcraft_color::freeform::spread_scale;
    use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, Paint};
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::{Point, Rect, shapes};
    let _g = settings();
    let old = cms::active_settings();
    cms::set_active(&cms::ColorSettings { rgb: cms::WIDE_GAMUT_RGB.into(), ..old.clone() }).unwrap();
    let box_ = Rect::new(0.0, 0.0, 100.0, 100.0);
    let (a, b) = (Color::rgb(0.9, 0.1, 0.1), Color::rgb(0.1, 0.2, 0.9));
    let p = |x, c| FreeformPoint::new(Point::new(x, 50.5), c);
    let f = Freeform { points: vec![p(20.5, a), p(80.5, b)], ..Default::default() };
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
    g.freeform = Some(f.clone());
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    d.insert(Some(d.layers[0].id), 0, Node::path(id, shapes::rectangle(box_), Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0)))
        .unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let canvas = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts).pixel(50, 50);
    let svg = vectorcraft_svg::export_full(&d, &vectorcraft_svg::ExportOptions::default(), None).svg;
    let data = svg.split("data:image/png;base64,").nth(1).unwrap().split('"').next().unwrap();
    let png = craft_image_codecs::decode(&vectorcraft_testkit::format::base64_decode(data).unwrap())
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    let written = png.get_pixel(png.width() / 2, png.height() / 2).0;
    let want = Color::rgb(0.5, 0.15, 0.5).to_rgba8(1.0);
    let display = f.field(spread_scale(box_)).sample(Point::new(50.5, 50.5)).0.map(|v| (v * 255.0).round() as u8);
    cms::set_active(&old).unwrap();
    let near = |got: [u8; 4]| got.iter().zip(want).take(3).all(|(g, w)| g.abs_diff(w) <= 3);
    assert!(near(canvas), "canvas: {canvas:?} vs {want:?}");
    assert!(near(written), "SVG: {written:?} vs {want:?}");
    assert!(display.iter().zip(want).any(|(g, w)| g.abs_diff(w) > 8), "the display mix differs: {display:?} vs {want:?}");
}

/// Loading a new file of the working RGB profile (named after its description, as before) takes
/// effect at once: CMYK colors convert into the new file and the canvas redraws, without choosing
/// the working spaces again.
#[test]
fn loading_the_working_rgb_profile_again_takes_effect_at_once() {
    let _g = settings();
    let old = cms::active_settings();
    // A built-in RGB profile's file renamed `name`: padded with spaces, which names drop.
    let renamed = |builtin: &str, name: &str| {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let mut bytes = cms::icc_bytes(builtin).unwrap().to_vec();
        let (from, to) = (utf16(builtin), utf16(&format!("{name:<width$}", width = builtin.len())));
        let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
        bytes[at..at + to.len()].copy_from_slice(&to);
        bytes
    };
    let name = "Reload RGB";
    let dir = vectorcraft_testkit::TempDir::new("reload-working-rgb");
    let load = |s: &mut Session, builtin: &str| {
        let path = dir.join(format!("{}.icc", builtin.len()));
        std::fs::write(&path, renamed(builtin, name)).unwrap();
        let r = s.execute("color.loadProfile", &json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(r["name"], name);
    };
    let cmyk = cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some("Reload Working CMYK".into())).unwrap().name;
    let cyan = [1.0, 0.0, 0.0, 0.0];
    let convert = || cms::active().cmyk_to_rgb(cyan, old.intent);
    let mut s = session();
    load(&mut s, cms::DISPLAY_P3);
    s.execute("edit.colorSettings", &json!({"rgb": name, "cmyk": cmyk})).unwrap();
    let in_p3 = convert();
    let revision = s.doc().unwrap().revision;
    load(&mut s, cms::PROPHOTO_RGB);
    let after = convert();
    let redrawn = s.doc().unwrap().revision > revision;
    // What a CMYK profile loaded afresh gives in the new file.
    let fresh = cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some("Reload Working CMYK 2".into())).unwrap().name;
    let want = cms::Cms::new(&cms::ColorSettings { cmyk: fresh, ..cms::active_settings() }).unwrap().cmyk_to_rgb(cyan, old.intent);
    cms::set_active(&old).unwrap();
    assert!(in_p3.iter().zip(want).any(|(a, b)| (a - b).abs() > 0.05), "Display P3 {in_p3:?} vs ProPhoto {want:?}");
    assert!(after.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4), "after the reload {after:?}, want {want:?}");
    assert!(redrawn, "the canvas redraws");
}

/// Loading a new file of the CMYK profile View → Proof Setup targets, or removing that profile,
/// gives the documents new revisions, as a working space's new file does; loading or removing
/// another profile does not.
#[test]
fn loading_or_removing_the_proof_profile_redraws() {
    let _g = settings();
    // A built-in CMYK profile's file renamed `name`: padded with spaces, which names drop.
    let renamed = |builtin: &str, name: &str| {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let mut bytes = cms::icc_bytes(builtin).unwrap().to_vec();
        let (from, to) = (utf16(builtin), utf16(&format!("{name:<width$}", width = builtin.len())));
        let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
        bytes[at..at + to.len()].copy_from_slice(&to);
        bytes
    };
    let dir = fresh_dir("reload-proof-cmyk");
    std::fs::create_dir_all(&dir).unwrap();
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    let load = |s: &mut Session, name: &str| {
        let path = dir.join(format!("{}.icc", name.len()));
        std::fs::write(&path, renamed(cms::GENERIC_CMYK, name)).unwrap();
        let r = s.execute("color.loadProfile", &json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(r["name"], name);
        s.doc().unwrap().revision
    };
    let (proofed, other) = ("Reload Proof CMYK", "Reload Other CMYK 2");
    let _view = ProofReset([proofed, other]);
    let mut s = session();
    load(&mut s, proofed);
    load(&mut s, other);
    exec(&mut s, "view.proofSetup", json!({"target": format!("cmyk:{proofed}"), "proof": true}));
    let revision = s.doc().unwrap().revision;
    assert_eq!(load(&mut s, other), revision, "another profile's file");
    let reloaded = load(&mut s, proofed);
    assert!(reloaded > revision, "the proof profile's file");
    exec(&mut s, "color.removeProfile", json!({"name": other}));
    assert_eq!(s.doc().unwrap().revision, reloaded, "another profile removed");
    exec(&mut s, "color.removeProfile", json!({"name": proofed}));
    assert!(s.doc().unwrap().revision > reloaded, "the proof profile removed");
}

/// Sets the default proof view and unregisters the profiles when dropped, also after a failed
/// assertion.
struct ProofReset([&'static str; 2]);

impl Drop for ProofReset {
    fn drop(&mut self) {
        vectorcraft_render::proof::set_view(Default::default());
        for name in self.0 {
            cms::unregister_icc(name);
        }
    }
}

/// A folder of its own for the test `tag`, empty, removed when dropped.
fn fresh_dir(tag: &str) -> vectorcraft_testkit::TempDir {
    vectorcraft_testkit::TempDir::new(&format!("color-settings-{tag}"))
}

/// An ICC file of an RGB profile named `name` (a built-in one with its description replaced by a
/// name of the same length).
#[allow(clippy::unwrap_used, reason = "a test helper: a missing built-in profile fails the test")]
fn icc_named(name: &str) -> Vec<u8> {
    let builtin = cms::DISPLAY_P3;
    assert_eq!(builtin.len(), name.len());
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let mut bytes = cms::icc_bytes(builtin).unwrap().to_vec();
    let (from, to) = (utf16(builtin), utf16(name));
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
    bytes[at..at + to.len()].copy_from_slice(&to);
    bytes
}

/// Puts the default Color Settings back, unregisters `profile` and deletes `paths` when dropped,
/// also after a failed assertion, so the next test starts from the defaults.
struct ColorSettingsReset {
    profile: Option<&'static str>,
    paths: Vec<std::path::PathBuf>,
}

impl Drop for ColorSettingsReset {
    /// The defaults back, the profile unregistered and the paths deleted.
    fn drop(&mut self) {
        let _ = cms::set_active(&cms::ColorSettings::default());
        if let Some(name) = self.profile {
            cms::unregister_icc(name);
        }
        for path in &self.paths {
            let _ = std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path));
        }
    }
}

/// #6335: Edit › Color Settings come back in the next session: the working spaces (a loaded
/// profile among them, registered again by the profile store first), the intent and black point
/// compensation. A session nobody attached a file to keeps nothing, and nothing is written before
/// the settings change.
#[test]
fn color_settings_come_back_in_the_next_session() {
    let _g = settings();
    let root = fresh_dir("color-settings");
    let (profiles, file) = (root.join("Color Profiles"), root.join("Color Settings.json"));
    let loaded = "StoreRGB-6";
    let icc_dir = vectorcraft_testkit::TempDir::new("store-color-settings");
    let icc = icc_dir.join("loaded.icc");
    let _reset = ColorSettingsReset { profile: Some(loaded), paths: vec![root.clone(), icc.clone()] };
    std::fs::write(&icc, icc_named(loaded)).unwrap();

    let mut a = Session::new();
    a.attach_profile_store(profiles.clone());
    a.attach_color_settings(file.clone());
    a.execute("color.loadProfile", &json!({"path": icc.to_str().unwrap()})).unwrap();
    a.execute("edit.colorSettings", &json!({})).unwrap();
    assert!(!file.exists(), "nothing to keep before the settings change");
    a.execute("edit.colorSettings", &json!({"rgb": loaded, "cmyk": cms::DEVICE_CMYK, "intent": "perceptual", "bpc": false})).unwrap();
    let chosen = cms::ColorSettings {
        rgb: loaded.into(),
        cmyk: cms::DEVICE_CMYK.into(),
        intent: cms::Intent::Perceptual,
        bpc: false,
        ..cms::ColorSettings::default()
    };
    assert_eq!(cms::active_settings(), chosen);

    // The next start begins with the defaults and without the loaded profile, which the profile
    // store registers again before the settings name it.
    cms::set_active(&cms::ColorSettings::default()).unwrap();
    assert!(cms::unregister_icc(loaded));
    let mut b = Session::new();
    b.attach_profile_store(profiles);
    b.attach_color_settings(file.clone());
    assert_eq!(cms::active_settings(), chosen, "the saved settings are the active ones again");

    // Without a file, a change is kept only for the process.
    let mut c = Session::new();
    c.execute("edit.colorSettings", &json!({"bpc": true})).unwrap();
    let saved: cms::ColorSettings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved, chosen, "the file is left alone");
}

/// #6335: saved Color Settings that can't be used (a profile that is gone, a damaged or oversized
/// file) leave the defaults and the file as it is; the next change writes over it.
#[test]
fn saved_color_settings_that_cannot_be_used_leave_the_defaults() {
    let _g = settings();
    let dir = fresh_dir("color-settings-bad");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    let file = dir.join("Color Settings.json");
    std::fs::create_dir_all(&dir).unwrap();
    let gone = json!({"rgb": cms::SRGB, "cmyk": "A Profile That Is Gone", "intent": "perceptual", "bpc": false}).to_string().into_bytes();
    let oversized = vec![b' '; MAX_PRESET_FILE as usize + 1];
    for bytes in [gone, b"not json".to_vec(), oversized] {
        cms::set_active(&cms::ColorSettings::default()).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        let mut s = Session::new();
        s.attach_color_settings(file.clone());
        assert_eq!(cms::active_settings(), cms::ColorSettings::default());
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "left as it is");
        s.execute("edit.colorSettings", &json!({"bpc": false})).unwrap();
        let saved: cms::ColorSettings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(!saved.bpc && saved.cmyk == cms::GENERIC_CMYK, "written over by the next change: {saved:?}");
    }
}

/// With a working RGB space other than sRGB, `color.contrast` writes the background in working RGB,
/// the form `against` takes: a given brown comes back as it was given, and the background read
/// behind the selection, given back, answers the same ratio.
#[test]
fn contrast_writes_the_background_as_against_takes_it_with_another_working_space() {
    let _g = settings();
    let before = cms::active_settings();
    let mut s = session();
    exec(&mut s, "edit.colorSettings", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    let given = s.execute("color.contrast", &json!({"color": "#000000", "against": "#806040"}));
    exec(&mut s, "file.new", json!({"width": 200, "height": 200}));
    exec(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 80.3, "height": 160}));
    exec(&mut s, "paint.setFill", json!({"color": "#2050a0"}));
    exec(&mut s, "object.lock", json!({}));
    exec(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}));
    exec(&mut s, "paint.setFill", json!({"color": "#808080"}));
    let below = s.execute("color.contrast", &json!({}));
    let again = below.as_ref().ok().and_then(|b| s.execute("color.contrast", &json!({"against": b["against"].clone()})).ok());
    cms::set_active(&before).unwrap();
    let given = given.unwrap();
    assert_eq!(given["against"], "#806040", "{given}");
    let (below, again) = (below.unwrap(), again.unwrap());
    assert_eq!(below["against"], again["against"], "{below} {again}");
    assert!((below["ratio"].as_f64().unwrap() - again["ratio"].as_f64().unwrap()).abs() < 1e-9, "{below} {again}");
}

/// View › Proof Setup › Customize's Preserve CMYK Numbers proofs each color from its own inks:
/// with Device CMYK as the working space, C50 M50 Y50 and K50 show as the same gray, and on
/// VectorCraft Generic CMYK they print, and so proof, as different grays; a pattern's tile art
/// proofs the same way. Without it the proof starts from the drawn pixels, which are the same.
#[test]
fn preserve_cmyk_numbers_proofs_each_color_from_its_inks() {
    let _g = restored_settings();
    let mut s = session();
    s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let k50 = json!({"c": 0, "m": 0, "y": 0, "k": 50});
    let cmy = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    exec(&mut s, "paint.setFill", json!({"ids": [cmy.0], "color": {"c": 50, "m": 50, "y": 50, "k": 0}}));
    let k = rect(&mut s, 100.0, 0.0, 100.0, 100.0);
    exec(&mut s, "paint.setFill", json!({"ids": [k.0], "color": k50}));
    // A pattern whose tile is all K50, filling a band below the squares.
    let tile = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [tile.0], "color": k50}));
    exec(&mut s, "object.pattern.make", json!({"ids": [tile.0], "name": "K50 Tile", "edit": false}));
    let band = rect(&mut s, 0.0, 130.0, 200.0, 40.0);
    exec(&mut s, "paint.setFill", json!({"ids": [band.0], "swatch": "K50 Tile"}));
    let proof = |preserve_numbers| vectorcraft_render::proof::ProofSetup {
        target: vectorcraft_render::proof::ProofTarget::Cmyk(cms::GENERIC_CMYK.into()),
        preserve_numbers,
        ..Default::default()
    };
    let doc = s.doc().unwrap().doc.clone();
    let pixel = |px: &[u8], x: usize, y: usize| [px[(y * 200 + x) * 4], px[(y * 200 + x) * 4 + 1], px[(y * 200 + x) * 4 + 2]];
    // The pixels at the centers of the two squares and in the patterned band.
    let shown = |proof: Option<vectorcraft_render::proof::ProofSetup>| {
        let opts = RenderOptions { proof, background: Some([255, 255, 255, 255]), ..Default::default() };
        let px = Renderer::new().render(&doc, 200, 200, Affine::IDENTITY, &opts).pixels;
        (pixel(&px, 50, 50), pixel(&px, 150, 50), pixel(&px, 55, 155))
    };
    let (a, b, _) = shown(None);
    let (pa, pb, _) = shown(Some(proof(false)));
    let (ca, cb, cp) = shown(Some(proof(true)));
    let want = |c: Color| cms::active().proof_color(&c, &proof(true)).map(|v| (v * 255.0).round() as i32);
    let cmy_want = want(Color::cmyk(0.5, 0.5, 0.5, 0.0));
    let k_want = want(Color::cmyk(0.0, 0.0, 0.0, 0.5));
    // With Simulate Paper Color the art sits on the device's paper, not on white.
    let paper = vectorcraft_render::proof::ProofSetup { simulate_paper: true, ..proof(true) };
    let tint = cms::active().proof_srgb([1.0; 3], &paper).map(|v| (v * 255.0).round() as i32);
    let opts = RenderOptions { proof: Some(paper), background: Some([255, 255, 255, 255]), ..Default::default() };
    let bare = pixel(&Renderer::new().render(&doc, 200, 200, Affine::IDENTITY, &opts).pixels, 50, 115);
    assert_eq!(a, b, "the two builds show as one gray in Device CMYK");
    assert_eq!(pa, pb, "proofed from the pixels, they stay one gray");
    assert!(ca != cb, "proofed from their inks, they print differently: {ca:?} vs {cb:?}");
    let near = |got: [u8; 3], want: [i32; 3]| got.iter().zip(want).all(|(g, w)| (i32::from(*g) - w).abs() <= 2);
    assert!(near(ca, cmy_want) && near(cb, k_want), "{ca:?} {cmy_want:?}, {cb:?} {k_want:?}");
    assert!(near(cp, k_want), "the pattern's tile proofs from its inks: {cp:?} vs {k_want:?}");
    assert!(tint != [255; 3] && near(bare, tint), "the bare page shows the paper: {bare:?} vs {tint:?}");
}

/// With Preserve CMYK Numbers, an image in an opacity mask keeps its pixels as they are, so the
/// mask covers the same in a pattern's tile as outside one.
#[test]
fn preserve_cmyk_numbers_leaves_mask_images_in_pattern_tiles_as_they_are() {
    let _g = restored_settings();
    let mut s = session();
    // A K100 square masked by a blue image, the same at (0, 0) and as a pattern's tile.
    let masked = |s: &mut Session, x: f64| {
        let obj = rect(s, x, 0.0, 20.0, 20.0);
        exec(s, "paint.setFill", json!({"ids": [obj.0], "color": {"c": 0, "m": 0, "y": 0, "k": 100}}));
        let blue = rect(s, x, 0.0, 20.0, 20.0);
        exec(s, "paint.setFill", json!({"ids": [blue.0], "color": "#0000ff"}));
        select(s, &[blue]);
        exec(s, "object.rasterize", json!({"ppi": 72}));
        let image = s.doc().unwrap().selection.objects[0];
        select(s, &[obj, image]);
        exec(s, "transparency.makeOpacityMask", json!({"clip": true}));
        obj
    };
    masked(&mut s, 0.0);
    let tile = masked(&mut s, 300.0);
    exec(&mut s, "object.pattern.make", json!({"ids": [tile.0], "name": "Masked Tile", "edit": false}));
    let band = rect(&mut s, 0.0, 100.0, 200.0, 40.0);
    exec(&mut s, "paint.setFill", json!({"ids": [band.0], "swatch": "Masked Tile"}));
    // An unmasked K100 square, for the color the masked one covers the page with.
    let solid = rect(&mut s, 100.0, 0.0, 20.0, 20.0);
    exec(&mut s, "paint.setFill", json!({"ids": [solid.0], "color": {"c": 0, "m": 0, "y": 0, "k": 100}}));
    let proof = vectorcraft_render::proof::ProofSetup {
        target: vectorcraft_render::proof::ProofTarget::Cmyk(cms::GENERIC_CMYK.into()),
        preserve_numbers: true,
        ..Default::default()
    };
    let doc = s.doc().unwrap().doc.clone();
    let pixel = |px: &[u8], x: usize, y: usize| [px[(y * 200 + x) * 4], px[(y * 200 + x) * 4 + 1], px[(y * 200 + x) * 4 + 2]];
    // The masked square, the middle of a tile (the tiles start at the art's left and top edges,
    // every 20 points), the unmasked square and the bare page.
    let shown = |proof: Option<vectorcraft_render::proof::ProofSetup>| {
        let opts = RenderOptions { proof, background: Some([255, 255, 255, 255]), ..Default::default() };
        let px = Renderer::new().render(&doc, 200, 200, Affine::IDENTITY, &opts).pixels;
        (pixel(&px, 10, 10), pixel(&px, 50, 130), pixel(&px, 110, 10), pixel(&px, 190, 60))
    };
    let (plain, plain_tile, plain_k, plain_page) = shown(None);
    let (proofed, proofed_tile, proofed_k, proofed_page) = shown(Some(proof));
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= 2);
    assert!(plain != [255; 3] && near(plain, plain_tile), "the tile shows the masked square: {plain:?} vs {plain_tile:?}");
    assert!(near(proofed, proofed_tile), "the tile's mask covers as the square's does: {proofed:?} vs {proofed_tile:?}");
    // The proof changes the ink's color but not how much of the page the mask lets it cover.
    let channel = |v: [u8; 3], i: usize| f32::from(v[i]);
    let cover = (0..3).map(|i| (channel(plain_page, i) - channel(plain, i)) / (channel(plain_page, i) - channel(plain_k, i))).sum::<f32>() / 3.0;
    assert!(cover > 0.02 && cover < 0.98, "the mask covers part of the square: {cover}");
    let want: [u8; 3] =
        std::array::from_fn(|i| (channel(proofed_page, i) + cover * (channel(proofed_k, i) - channel(proofed_page, i))).round() as u8);
    assert!(near(proofed, want), "the proofed square keeps the mask's coverage: {proofed:?} vs {want:?}");
}

/// Edit › Color Settings' color management policies decide what opening a document tagged with
/// profiles other than the working spaces does with them: Preserve Embedded Profiles keeps them,
/// Off and Preserve Numbers take them off and keep the color numbers, and Convert to Working Space
/// converts the colors into the working spaces, keeping how they look, and takes them off. A
/// document a policy changed opens with unsaved changes, and the open reply names each profile.
/// A profile that is the working space stays, and File › Revert and File › New from Template apply
/// the policies too.
#[test]
fn color_management_policies_keep_take_off_or_convert_a_tagged_profile() {
    let _g = settings();
    let dir = fresh_dir("policies");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let tagged = dir.join("tagged.vectorcraft");
    let mut s = session();
    let (blue, ink) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 20.0, 0.0, 10.0, 10.0));
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "paint.setFill", json!({"ids": [ink.0], "color": {"c": 20, "m": 40, "y": 60, "k": 10}}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB, "cmyk": cms::DEVICE_CMYK}));
    // An artboard background, a character style's fill and a freeform gradient swatch, colors
    // outside the art.
    let paper = vectorcraft_color::Color::Rgb { r: 0.9, g: 0.8, b: 0.6 };
    let free = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    select(&mut s, &[free]);
    exec(&mut s, "paint.setFill", json!({"gradient": {"kind": "freeform"}}));
    exec(&mut s, "paint.freeform.setPoint", json!({"index": 0, "color": "#e6cc99", "stroke": false}));
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        d.artboards[0].color = Some(paper);
        let mut style = vectorcraft_doc::TextStyleDef { name: "Inked".into(), attrs: Default::default() };
        style.attrs.insert("fill".into(), serde_json::to_value(vectorcraft_color::Paint::solid(paper)).unwrap());
        d.char_styles.push(style);
        let paint = d.node(free).unwrap().appearance.fill().unwrap().paint.clone();
        d.swatches.push(vectorcraft_color::Swatch { name: "Free".into(), paint, global: false, spot: false });
    }
    exec(&mut s, "file.saveAs", json!({"path": tagged.to_str().unwrap()}));
    let settings_colors = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let style = serde_json::from_value::<vectorcraft_color::Paint>(d.char_styles[0].attrs["fill"].clone()).unwrap();
        (d.artboards[0].color.unwrap(), style.color().unwrap())
    };
    // The first point of the freeform swatch and of the freeform fill.
    let free_points = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let first = |p: &vectorcraft_color::Paint| match p {
            vectorcraft_color::Paint::Gradient(g) => g.freeform.as_ref().map(|f| f.points[0].color),
            _ => None,
        };
        let swatch = d.swatches.iter().find(|w| w.name == "Free").and_then(|w| first(&w.paint)).unwrap();
        (swatch, first(&d.node(free).unwrap().appearance.fill().unwrap().paint).unwrap())
    };
    let (free0, _) = free_points(&s);
    let fill = |s: &Session, id| s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap();
    let (blue0, ink0) = (fill(&s, blue), fill(&s, ink));
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), Some(cms::DEVICE_CMYK));
    let notes = |r: &serde_json::Value| r["warnings"].as_array().unwrap().iter().filter(|w| w.as_str().unwrap().contains("profile")).count();
    // A session that opened the file under the policies, and the open reply.
    let open = |rgb: &str, cmyk: &str| {
        let mut s = session();
        exec(&mut s, "edit.colorSettings", json!({"rgbPolicy": rgb, "cmykPolicy": cmyk}));
        let r = exec(&mut s, "document.open", json!({"path": tagged.to_str().unwrap()}));
        (r, s)
    };

    let (r, kept) = open("preserveEmbedded", "preserveEmbedded");
    assert_eq!(doc_profiles(&kept.doc().unwrap().doc), (Some(cms::WIDE_GAMUT_RGB.into()), Some(cms::DEVICE_CMYK.into())));
    assert_eq!((fill(&kept, blue), fill(&kept, ink), notes(&r)), (blue0, ink0, 0));
    assert!(!kept.doc().unwrap().is_dirty());

    for (rgb, cmyk) in [("off", "off"), ("off", "preserveNumbers")] {
        let (r, off) = open(rgb, cmyk);
        assert_eq!(doc_profiles(&off.doc().unwrap().doc), (None, None), "{cmyk}");
        assert_eq!((fill(&off, blue), fill(&off, ink), notes(&r)), (blue0, ink0, 2), "{cmyk}: the numbers stay");
        assert!(off.doc().unwrap().is_dirty(), "{cmyk}: the document changed");
    }

    let (r, converted) = open("convert", "convert");
    assert_eq!(doc_profiles(&converted.doc().unwrap().doc), (None, None));
    assert_eq!(notes(&r), 2);
    let working = cms::working();
    let (blue1, ink1) = (fill(&converted, blue), fill(&converted, ink));
    assert!(blue1 != blue0 && ink1 != ink0, "the numbers change: {blue1:?} {ink1:?}");
    assert!(cms::delta_e2000(source.lab(&blue0), working.lab(&blue1)) < 1.0, "{blue0:?} → {blue1:?}");
    assert!(cms::delta_e2000(source.lab(&ink0), working.lab(&ink1)) < 3.0, "{ink0:?} → {ink1:?}");
    assert!(converted.doc().unwrap().is_dirty());
    let (board, styled) = settings_colors(&converted);
    assert_eq!(board, styled, "the artboard and the style convert alike");
    assert!(board != paper && cms::delta_e2000(source.lab(&paper), working.lab(&board)) < 1.0, "{paper:?} → {board:?}");
    assert_eq!(settings_colors(&kept), (paper, paper), "kept with the profile");
    let (swatch_point, fill_point) = free_points(&converted);
    assert_eq!(swatch_point, fill_point, "the freeform swatch and fill convert alike");
    assert!(swatch_point != free0 && cms::delta_e2000(source.lab(&free0), working.lab(&swatch_point)) < 1.0, "{free0:?} → {swatch_point:?}");

    // A color mode asked for on open converts the colors the policy converted, through the working
    // spaces, also when the document in front draws through another profile.
    let mut s2 = session();
    exec(&mut s2, "edit.assignProfile", json!({"rgb": cms::DISPLAY_P3}));
    exec(&mut s2, "edit.colorSettings", json!({"rgbPolicy": "convert", "cmykPolicy": "convert"}));
    exec(&mut s2, "document.open", json!({"path": tagged.to_str().unwrap(), "colorMode": "cmyk"}));
    let intent = working.settings().intent;
    let want = working.convert(&blue1, cms::Model::Cmyk, intent);
    let got = fill(&s2, blue);
    assert!(cms::delta_e2000(working.lab(&want), working.lab(&got)) < 0.01, "{want:?} vs {got:?}");

    // A profile that is the working space is kept whatever the policy.
    exec(&mut s, "edit.colorSettings", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    let (r, same) = open("off", "off");
    assert_eq!(doc_profiles(&same.doc().unwrap().doc), (Some(cms::WIDE_GAMUT_RGB.into()), None));
    assert_eq!(notes(&r), 1);
    exec(&mut s, "edit.colorSettings", json!({"rgb": cms::SRGB}));

    // File › Revert reads the file again under the policies.
    let (_, mut reverted) = open("preserveEmbedded", "preserveEmbedded");
    exec(&mut reverted, "paint.setFill", json!({"ids": [blue.0], "color": "#ff0000"}));
    exec(&mut reverted, "edit.colorSettings", json!({"cmykPolicy": "preserveNumbers"}));
    let r = exec(&mut reverted, "file.revert", json!({"confirmed": true}));
    assert_eq!(doc_profiles(&reverted.doc().unwrap().doc), (Some(cms::WIDE_GAMUT_RGB.into()), None));
    assert_eq!((fill(&reverted, blue), notes(&r)), (blue0, 1));

    // File › New from Template applies the policies as opening does.
    let mut from_template = session();
    exec(&mut from_template, "edit.colorSettings", json!({"rgbPolicy": "convert", "cmykPolicy": "off"}));
    let r = exec(&mut from_template, "file.newFromTemplate", json!({"path": tagged.to_str().unwrap()}));
    assert_eq!(doc_profiles(&from_template.doc().unwrap().doc), (None, None));
    assert_eq!(notes(&r), 2);
    assert_eq!(fill(&from_template, ink), ink0, "Off keeps the CMYK numbers");
    let blue2 = fill(&from_template, blue);
    assert!(blue2 != blue0 && cms::delta_e2000(source.lab(&blue0), working.lab(&blue2)) < 1.0, "{blue0:?} → {blue2:?}");
    assert!(from_template.doc().unwrap().is_dirty());
}

/// The policies are checked, come back in the reply and in `Color Settings.json`, and a saved file
/// that gives the RGB policy Preserve Numbers, a CMYK policy only, leaves the defaults.
#[test]
fn color_management_policies_are_checked_and_kept() {
    let _g = settings();
    let dir = fresh_dir("policies-kept");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    let file = dir.join("Color Settings.json");
    let mut s = session();
    s.attach_color_settings(file.clone());
    for bad in [json!({"rgbPolicy": "preserveNumbers"}), json!({"cmykPolicy": "keep"}), json!({"rgbPolicy": 1})] {
        assert!(s.execute("edit.colorSettings", &bad).is_err(), "{bad}");
    }
    assert!(!file.exists(), "a refused change keeps nothing");
    let r = exec(&mut s, "edit.colorSettings", json!({"rgbPolicy": "convert", "cmykPolicy": "preserveNumbers"}));
    assert_eq!((r["rgbPolicy"].as_str(), r["cmykPolicy"].as_str()), (Some("convert"), Some("preserveNumbers")));
    let saved: cms::ColorSettings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!((saved.rgb_policy, saved.cmyk_policy), (cms::Policy::Convert, cms::Policy::PreserveNumbers));
    cms::set_active(&cms::ColorSettings::default()).unwrap();
    std::fs::write(
        &file,
        json!({"rgb": cms::SRGB, "cmyk": cms::GENERIC_CMYK, "intent": "relative", "bpc": true, "rgbPolicy": "preserveNumbers"}).to_string(),
    )
    .unwrap();
    let mut next = Session::new();
    next.attach_color_settings(file);
    assert_eq!(cms::active_settings(), cms::ColorSettings::default());
}

/// `color.parse` takes `oklch()` only while the working RGB space is sRGB; a hex color still gives
/// its numbers as the document's RGB numbers in another working space.
#[test]
fn color_parse_takes_oklch_only_in_an_srgb_working_space() {
    let _g = settings();
    let old = cms::active_settings();
    let mut s = session();
    s.execute("edit.colorSettings", &json!({"rgb": cms::WIDE_GAMUT_RGB})).unwrap();
    let oklch = s.execute("color.parse", &json!({"text": "oklch(0.6 0.1 30)"})).map_err(|e| e.to_string());
    let hex = s.execute("color.parse", &json!({"text": "#33669980"})).map_err(|e| e.to_string());
    cms::set_active(&old).unwrap();
    assert!(oklch.as_ref().unwrap_err().ends_with("OKLCH needs sRGB as the working RGB space"), "{oklch:?}");
    let hex = hex.unwrap();
    assert_eq!((&hex["hex"], &hex["opacity"]), (&json!("#336699"), &json!(50.2)));
    assert!(s.execute("color.parse", &json!({"text": "oklch(0.6 0.1 30)"})).is_ok(), "sRGB again");
}

/// The Ask boxes of Edit › Color Settings are checked, come back in the reply and are kept in
/// `Color Settings.json`.
#[test]
fn ask_boxes_are_checked_and_kept() {
    let _g = settings();
    let dir = fresh_dir("ask-kept");
    let _reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    let file = dir.join("Color Settings.json");
    let mut s = session();
    s.attach_color_settings(file.clone());
    let r = exec(&mut s, "edit.colorSettings", json!({}));
    assert_eq!(
        (r["askMismatchOpen"].as_bool(), r["askMismatchPaste"].as_bool(), r["askMissingOpen"].as_bool()),
        (Some(false), Some(false), Some(false))
    );
    for bad in [json!({"askMismatchOpen": "yes"}), json!({"askMismatchPaste": 1}), json!({"askMissingOpen": []})] {
        assert!(s.execute("edit.colorSettings", &bad).is_err(), "{bad}");
    }
    assert!(!file.exists(), "a refused change keeps nothing");
    let r = exec(&mut s, "edit.colorSettings", json!({"askMismatchOpen": true, "askMissingOpen": true}));
    assert_eq!(
        (r["askMismatchOpen"].as_bool(), r["askMismatchPaste"].as_bool(), r["askMissingOpen"].as_bool()),
        (Some(true), Some(false), Some(true))
    );
    let saved: cms::ColorSettings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert!(saved.ask_mismatch_open && !saved.ask_mismatch_paste && saved.ask_missing_open);
}

/// Two saved documents with a blue rectangle (`blue`, filled `blue0`): `tagged`, assigned Wide
/// Gamut RGB, and `plain`, untagged. The files go when it drops, and the Color Settings go back to
/// the defaults.
struct ProfileFiles {
    _reset: ColorSettingsReset,
    _dir: vectorcraft_testkit::TempDir,
    tagged: std::path::PathBuf,
    plain: std::path::PathBuf,
    blue: vectorcraft_doc::NodeId,
    blue0: vectorcraft_color::Color,
}

#[allow(clippy::unwrap_used, reason = "a test helper: a file that can't be written fails the test")]
fn profile_files(tag: &str) -> ProfileFiles {
    let dir = fresh_dir(tag);
    let reset = ColorSettingsReset { profile: None, paths: vec![dir.clone()] };
    std::fs::create_dir_all(&dir).unwrap();
    let (tagged, plain) = (dir.join("tagged.vectorcraft"), dir.join("plain.vectorcraft"));
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "file.saveAs", json!({"path": plain.to_str().unwrap()}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    exec(&mut s, "file.saveAs", json!({"path": tagged.to_str().unwrap()}));
    let blue0 = fill_of(&s, blue);
    ProfileFiles { _reset: reset, _dir: dir, tagged, plain, blue, blue0 }
}

/// The fill color of object `id` in the active document.
#[allow(clippy::unwrap_used, reason = "a test helper: a missing object or fill fails the test")]
fn fill_of(s: &Session, id: vectorcraft_doc::NodeId) -> vectorcraft_color::Color {
    s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap()
}

/// `document.open` of `path` with the params `extra` in a new session → the reply, and the session.
#[allow(clippy::unwrap_used, reason = "a test helper: params that are not an object fail the test")]
fn open_with(path: &std::path::Path, extra: serde_json::Value) -> (Result<serde_json::Value, String>, Session) {
    let mut s = session();
    let mut p = json!({"path": path.to_str().unwrap()});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute("document.open", &p).map_err(|e| e.to_string());
    (r, s)
}

/// `document.open`'s `profileChoice` answers what Color Settings' Profile Mismatches box asks: for
/// a document tagged with another profile than the working space of its color mode it keeps the
/// profile, converts the colors or discards the profile in place of the policy. A missing choice
/// for it is not used.
#[test]
fn profile_choice_answers_a_mismatch() {
    let _g = settings();
    let ProfileFiles { tagged, blue, blue0, .. } = &profile_files("profile-mismatch");
    let (tagged, blue, blue0) = (tagged.as_path(), *blue, *blue0);
    let fill = |s: &Session| fill_of(s, blue);
    let open = open_with;
    let mut s = session();

    // Each choice in place of the policy (Preserve Embedded Profiles).
    let (r, kept) = open(tagged, json!({"profileChoice": "useEmbedded"}));
    let r = r.unwrap();
    assert_eq!(r["profileChoice"], json!({"issue": "mismatch", "model": "rgb", "choice": "useEmbedded"}));
    assert_eq!((doc_profiles(&kept.doc().unwrap().doc).0, fill(&kept)), (Some(cms::WIDE_GAMUT_RGB.into()), blue0));
    let (r, discarded) = open(tagged, json!({"profileChoice": "discard"}));
    let r = r.unwrap();
    assert_eq!((doc_profiles(&discarded.doc().unwrap().doc).0, fill(&discarded)), (None, blue0), "the numbers stay");
    assert!(discarded.doc().unwrap().is_dirty());
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("as chosen when it was opened")), "{r}");
    let (r, converted) = open(tagged, json!({"profileChoice": "convert"}));
    assert_eq!(r.unwrap()["profileChoice"]["choice"], "convert");
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    let blue1 = fill(&converted);
    assert_eq!(doc_profiles(&converted.doc().unwrap().doc).0, None);
    assert!(blue1 != blue0 && cms::delta_e2000(source.lab(&blue0), cms::working().lab(&blue1)) < 1.0, "{blue0:?} → {blue1:?}");
    // Under policy Off, Use Embedded keeps the profile.
    exec(&mut s, "edit.colorSettings", json!({"rgbPolicy": "off"}));
    let (_, kept) = open(tagged, json!({"profileChoice": "useEmbedded"}));
    assert_eq!(doc_profiles(&kept.doc().unwrap().doc).0, Some(cms::WIDE_GAMUT_RGB.into()));
    // A missing choice for a mismatched document is not used: the policy applies.
    let (r, off) = open(tagged, json!({"profileChoice": "leave"}));
    assert!(r.unwrap().get("profileChoice").is_none());
    assert_eq!(doc_profiles(&off.doc().unwrap().doc).0, None);
}

/// `document.open`'s `profileChoice` answers what the Missing Profiles box asks: an untagged
/// document is left, assigned the working space or assigned a profile, its numbers kept. A
/// mismatch choice for it is not used, and wrong choices are refused.
#[test]
fn profile_choice_answers_a_missing_profile() {
    let _g = settings();
    let ProfileFiles { tagged, plain, blue, blue0, .. } = &profile_files("profile-missing");
    let (tagged, plain, blue, blue0) = (tagged.as_path(), plain.as_path(), *blue, *blue0);
    let fill = |s: &Session| fill_of(s, blue);
    let open = open_with;
    let (r, left) = open(plain, json!({"profileChoice": "leave"}));
    assert_eq!(r.unwrap()["profileChoice"], json!({"issue": "missing", "model": "rgb", "choice": "leave"}));
    assert_eq!(doc_profiles(&left.doc().unwrap().doc).0, None);
    assert!(!left.doc().unwrap().is_dirty());
    let (_, working) = open(plain, json!({"profileChoice": "assignWorking"}));
    assert_eq!(doc_profiles(&working.doc().unwrap().doc).0.as_deref(), Some(cms::canonical_name(cms::SRGB)));
    assert!(working.doc().unwrap().is_dirty());
    let (_, p3) = open(plain, json!({"profileChoice": "assign", "assignProfile": cms::DISPLAY_P3}));
    assert_eq!((doc_profiles(&p3.doc().unwrap().doc).0, fill(&p3)), (Some(cms::DISPLAY_P3.into()), blue0), "assigning keeps the numbers");
    // A mismatch choice for an untagged document is not used.
    let (r, plain_kept) = open(plain, json!({"profileChoice": "convert"}));
    assert!(r.unwrap().get("profileChoice").is_none());
    assert_eq!(doc_profiles(&plain_kept.doc().unwrap().doc).0, None);

    for bad in [
        json!({"profileChoice": "bogus"}),
        json!({"profileChoice": "assign"}),
        json!({"profileChoice": "discard", "assignProfile": cms::DISPLAY_P3}),
        json!({"assignProfile": cms::DISPLAY_P3}),
        json!({"profileChoice": 3}),
    ] {
        assert!(open(tagged, bad.clone()).0.is_err(), "{bad}");
    }
    let (r, none) = open(plain, json!({"profileChoice": "assign", "assignProfile": cms::DEVICE_CMYK}));
    assert!(r.unwrap_err().contains("unknown rgb profile"), "a CMYK profile can't tag an RGB document");
    assert_eq!(none.documents().len(), 1, "nothing opened");
}

/// The app's half of the open prompts: a file read with asking stops at the question when its box
/// is on (never without asking), and opens as answered, or as the policy says when it opens
/// unanswered. A refused answer leaves it waiting.
#[test]
fn a_read_waits_for_the_profile_answer() {
    let _g = settings();
    let ProfileFiles { tagged, plain, blue, blue0, .. } = &profile_files("profile-wait");
    let (blue, blue0) = (*blue, *blue0);
    let fill = |s: &Session| fill_of(s, blue);
    let mut s = session();
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    let blue1 = source.convert_into(&cms::working(), &blue0, cms::working().settings().intent);
    let bytes = std::fs::read(tagged).unwrap();
    let name = tagged.to_str().unwrap();
    let read =
        |ask: bool| vectorcraft_engine::cmd::fileio::read_for_open_asking(name, &bytes, Some(name.into()), &json!({}), false, false, ask).unwrap();
    assert!(read(true).question().is_none(), "the box is off");
    exec(&mut s, "edit.colorSettings", json!({"askMismatchOpen": true, "rgbPolicy": "convert"}));
    assert!(read(false).question().is_none(), "document.open never asks");
    let mut waiting = read(true);
    let q = waiting.question().unwrap();
    assert_eq!((q.issue, q.profile.as_deref(), q.default.id()), (ProfileIssue::Mismatch, Some(cms::WIDE_GAMUT_RGB), "convert"));
    assert_eq!(waiting.name(), "tagged.vectorcraft");
    waiting.answer(ProfileChoice::Discard).unwrap();
    assert!(waiting.question().is_none(), "answered");
    let mut opened = session();
    let r = vectorcraft_engine::cmd::fileio::open_read(&mut opened, waiting).unwrap();
    assert_eq!(r["profileChoice"]["choice"], "discard");
    assert_eq!((doc_profiles(&opened.doc().unwrap().doc).0, fill(&opened)), (None, blue0));
    // Unanswered, it opens as the policy (Convert to Working Space) says.
    let mut opened = session();
    vectorcraft_engine::cmd::fileio::open_read(&mut opened, read(true)).unwrap();
    assert_eq!((doc_profiles(&opened.doc().unwrap().doc).0, fill(&opened)), (None, blue1));
    // A missing answer to a mismatch is not used: the policy decides.
    let mut waiting = read(true);
    waiting.answer(ProfileChoice::Leave).unwrap();
    let mut opened = session();
    let r = vectorcraft_engine::cmd::fileio::open_read(&mut opened, waiting).unwrap();
    assert!(r.get("profileChoice").is_none());
    assert_eq!(fill(&opened), blue1);
    // Missing Profiles: Ask When Opening, starting on assigning the working space under Convert.
    exec(&mut s, "edit.colorSettings", json!({"askMismatchOpen": false, "askMissingOpen": true}));
    assert!(read(true).question().is_none(), "the mismatch box is off now");
    let plain_bytes = std::fs::read(plain).unwrap();
    let mut waiting =
        vectorcraft_engine::cmd::fileio::read_for_open_asking("plain.vectorcraft", &plain_bytes, None, &json!({}), false, false, true).unwrap();
    let q = waiting.question().unwrap();
    assert_eq!((q.issue, q.profile.as_deref(), q.default.id()), (ProfileIssue::Missing, None, "assignWorking"));
    assert!(waiting.answer(ProfileChoice::Assign(cms::DEVICE_CMYK.into())).is_err(), "not an RGB profile");
    assert!(waiting.question().is_some(), "still waiting after a refused answer");
    waiting.answer(ProfileChoice::Assign(cms::DISPLAY_P3.into())).unwrap();
    let mut opened = session();
    vectorcraft_engine::cmd::fileio::open_read(&mut opened, waiting).unwrap();
    assert_eq!(doc_profiles(&opened.doc().unwrap().doc).0, Some(cms::DISPLAY_P3.into()));
}

/// An answer is taken only while the question it answers stands: once Color Settings change the
/// working space so that the question goes away or names another working space, the answer is
/// refused and the file keeps waiting. Asked again, it takes the answer to the question it asks now.
#[test]
fn a_profile_answer_waits_for_the_question_it_answers() {
    let _g = settings();
    let ProfileFiles { tagged, blue, blue0, .. } = &profile_files("profile-changed");
    let (blue, blue0) = (*blue, *blue0);
    let mut s = session();
    exec(&mut s, "edit.colorSettings", json!({"askMismatchOpen": true}));
    let bytes = std::fs::read(tagged).unwrap();
    let name = tagged.to_str().unwrap();
    let mut waiting = vectorcraft_engine::cmd::fileio::read_for_open_asking(name, &bytes, Some(name.into()), &json!({}), false, false, true).unwrap();
    assert_eq!(waiting.question().unwrap().working, cms::canonical_name(cms::SRGB));
    // The working space becomes the file's profile: the question is gone.
    exec(&mut s, "edit.colorSettings", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    assert!(waiting.answer(ProfileChoice::Discard).is_err(), "the question it answers is gone");
    // Another working space: the question names it.
    exec(&mut s, "edit.colorSettings", json!({"rgb": cms::DISPLAY_P3}));
    let err = waiting.answer(ProfileChoice::Discard).unwrap_err().to_string();
    assert!(err.contains("the profile question changed since it was asked"), "{err}");
    assert_eq!(waiting.ask().unwrap().working, cms::canonical_name(cms::DISPLAY_P3));
    waiting.answer(ProfileChoice::Discard).unwrap();
    let mut opened = session();
    let r = vectorcraft_engine::cmd::fileio::open_read(&mut opened, waiting).unwrap();
    assert_eq!(r["profileChoice"]["choice"], "discard");
    assert_eq!((doc_profiles(&opened.doc().unwrap().doc).0, fill_of(&opened, blue)), (None, blue0));
}

/// Swatch conflicts are checked against the colors as the paste brings them: a global swatch whose
/// name and color match the destination's conflicts once the paste converts it, and pasting back
/// into its own document with conversion compares its swatches too.
#[test]
fn swatch_conflicts_follow_the_profile_conversion() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    exec(&mut s, "swatch.new", json!({"name": "Brand", "color": "#3366cc", "global": true}));
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [r.0], "swatch": "Brand"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    select(&mut s, &[r]);
    exec(&mut s, "edit.copy", json!({}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    exec(&mut s, "swatch.new", json!({"name": "Brand", "color": "#3366cc", "global": true}));
    let conflicts = |s: &mut Session, p: serde_json::Value| exec(s, "clipboard.conflicts", p)["swatches"].as_array().unwrap().len();
    assert_eq!(conflicts(&mut s, json!({})), 0, "the numbers are kept: the same color");
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert"})), 1, "converted, it differs");
    assert!(s.execute("clipboard.conflicts", &json!({"profileMismatch": "keep"})).is_err());
    // Back in the first document, under another working space, a converting paste compares too.
    exec(&mut s, "document.activate", json!({"index": 0}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": null}));
    assert_eq!(conflicts(&mut s, json!({})), 0);
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert"})), 1);
}

/// A paste is a mismatch only for the models the copied colors use: CMYK art from a document with
/// other RGB and CMYK profiles is a CMYK mismatch alone, and so is an artboard copied without art
/// whose background is CMYK.
#[test]
fn a_paste_mismatches_only_the_models_its_colors_use() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let ink = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [ink.0], "color": {"c": 20, "m": 40, "y": 60, "k": 10}}));
    exec(&mut s, "paint.setStroke", json!({"ids": [ink.0], "none": true}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB, "cmyk": cms::DEVICE_CMYK}));
    select(&mut s, &[ink]);
    exec(&mut s, "edit.copy", json!({}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let models = |s: &mut Session| -> Vec<String> {
        let r = exec(s, "clipboard.profileMismatch", json!({}));
        r["mismatches"].as_array().unwrap().iter().map(|m| m["model"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(models(&mut s), ["cmyk"]);
    exec(&mut s, "document.activate", json!({"index": 0}));
    exec(&mut s, "artboard.setProps", json!({"index": 0, "color": {"c": 0, "m": 100, "y": 0, "k": 0}}));
    exec(&mut s, "artboard.copy", json!({"index": 0}));
    exec(&mut s, "document.activate", json!({"index": 1}));
    assert_eq!(models(&mut s), ["cmyk"]);
}

/// An artboard copied without art from a document with another profile is a mismatch through its
/// background color, which converts with a paste that converts.
#[test]
fn a_pasted_artboard_background_converts() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    exec(&mut s, "artboard.setProps", json!({"index": 0, "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    exec(&mut s, "artboard.copy", json!({"index": 0}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    assert_eq!(exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"][0]["model"], "rgb");
    let r = exec(&mut s, "edit.paste", json!({"profileMismatch": "convert"}));
    let i = r["artboards"][0].as_u64().unwrap() as usize;
    let paper = s.doc().unwrap().doc.artboards[i].color.unwrap();
    let blue0 = vectorcraft_color::Color::from_hex("#3366cc").unwrap();
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    assert!(paper != blue0 && cms::delta_e2000(source.lab(&blue0), cms::working().lab(&paper)) < 1.0, "{blue0:?} → {paper:?}");
}

/// Paste compares the profiles the copied colors showed through with the destination's: a model
/// whose profile differs and whose colors the objects use is a mismatch (clipboard.profileMismatch).
/// Its colors keep their numbers or convert, as `profileMismatch` says, else as the policy says;
/// the reply lists what was done.
#[test]
fn paste_keeps_or_converts_colors_from_another_profile() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB, "cmyk": cms::DEVICE_CMYK}));
    select(&mut s, &[blue]);
    exec(&mut s, "edit.copy", json!({}));
    let blue0 = vectorcraft_color::Color::from_hex("#3366cc").unwrap();
    // Back in the same document: nothing to compare.
    assert_eq!(exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"], json!([]));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let r = exec(&mut s, "clipboard.profileMismatch", json!({}));
    // Only RGB: the objects use no CMYK color.
    let srgb = cms::canonical_name(cms::SRGB);
    assert_eq!(
        r["mismatches"],
        json!([{"model": "rgb", "clipboard": cms::WIDE_GAMUT_RGB, "document": srgb, "default": "preserveNumbers", "convertible": true}])
    );
    let pasted = |s: &mut Session, p: serde_json::Value| {
        let r = exec(s, "edit.paste", p);
        let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
        let c = s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap();
        (r, c)
    };
    let (r, kept) = pasted(&mut s, json!({}));
    assert_eq!(kept, blue0, "the policy (Preserve Embedded Profiles) keeps the numbers");
    assert_eq!(r["profileMismatch"][0]["choice"], "preserveNumbers");
    let (r, moved) = pasted(&mut s, json!({"profileMismatch": "convert"}));
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    assert!(moved != blue0 && cms::delta_e2000(source.lab(&blue0), cms::working().lab(&moved)) < 1.0, "{blue0:?} → {moved:?}");
    exec(&mut s, "edit.colorSettings", json!({"rgbPolicy": "convert"}));
    assert_eq!(exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"][0]["default"], "convert");
    let (_, by_policy) = pasted(&mut s, json!({}));
    assert_eq!(by_policy, moved, "Convert to Working Space converts");
    let (_, numbers) = pasted(&mut s, json!({"profileMismatch": "preserveNumbers"}));
    assert_eq!(numbers, blue0);
    for bad in [json!({"profileMismatch": "keep"}), json!({"profileMismatch": true})] {
        assert!(s.execute("edit.paste", &bad).is_err(), "{bad}");
    }
    // A drag from the first document follows the policy too.
    let source = s.documents()[0].uid;
    let r = exec(&mut s, "edit.dropSelection", json!({"sourceDocument": source, "ids": [blue.0], "grab": [5, 5], "at": [50, 50]}));
    let dropped = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
    let c = s.doc().unwrap().doc.node(dropped).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap();
    assert_eq!(c, moved, "dropped colors convert under Convert to Working Space");
    // A destination assigned the source's profile has nothing to compare.
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    assert_eq!(exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"], json!([]));
    let (r, same) = pasted(&mut s, json!({}));
    assert!(r.get("profileMismatch").is_none());
    assert_eq!(same, blue0);
}

/// A drag between documents is a paste for Profile Mismatches: clipboard.profileMismatch with
/// `sourceDocument` and `ids` lists what the drop brings, edit.dropSelection takes `profileMismatch`
/// and reports what it did, and a wrong choice drops nothing.
#[test]
fn a_drop_takes_the_profile_mismatch_choice() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    let source = s.documents()[0].uid;
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let blue0 = vectorcraft_color::Color::from_hex("#3366cc").unwrap();
    assert_eq!(exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"], json!([]), "the clipboard is empty");
    let dragged = json!({"sourceDocument": source, "ids": [blue.0]});
    let srgb = cms::canonical_name(cms::SRGB);
    assert_eq!(
        exec(&mut s, "clipboard.profileMismatch", dragged.clone())["mismatches"],
        json!([{"model": "rgb", "clipboard": cms::WIDE_GAMUT_RGB, "document": srgb, "default": "preserveNumbers", "convertible": true}])
    );
    for bad in [json!({"sourceDocument": source}), json!({"ids": [blue.0]}), json!({"sourceDocument": 999_999, "ids": [blue.0]})] {
        assert!(s.execute("clipboard.profileMismatch", &bad).is_err(), "{bad}");
    }
    let dropped = |s: &mut Session, choice: Option<&str>| {
        let mut p = json!({"sourceDocument": source, "ids": [blue.0], "grab": [5, 5], "at": [50, 50]});
        if let Some(c) = choice {
            p["profileMismatch"] = json!(c);
        }
        let r = exec(s, "edit.dropSelection", p);
        let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
        (r, s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap())
    };
    let (r, kept) = dropped(&mut s, None);
    assert_eq!((kept, &r["profileMismatch"][0]["choice"]), (blue0, &json!("preserveNumbers")), "the policy keeps the numbers");
    let (r, moved) = dropped(&mut s, Some("convert"));
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    let from = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    assert!(moved != blue0 && cms::delta_e2000(from.lab(&blue0), cms::working().lab(&moved)) < 1.0, "{blue0:?} → {moved:?}");
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    let before = objects(&s);
    let bad = json!({"sourceDocument": source, "ids": [blue.0], "grab": [5, 5], "at": [50, 50], "profileMismatch": "keep"});
    assert!(s.execute("edit.dropSelection", &bad).is_err());
    assert_eq!(objects(&s), before, "a refused choice drops nothing");
}

/// `assignProfile` beside another choice than `assign` is refused with a message that says it goes
/// with `assign` only.
#[test]
fn assign_profile_goes_with_assign_only() {
    let _g = settings();
    let ProfileFiles { tagged, .. } = &profile_files("profile-assign-only");
    let err = open_with(tagged, json!({"profileChoice": "discard", "assignProfile": cms::DISPLAY_P3})).0.unwrap_err();
    assert!(err.contains("`assignProfile` goes with `profileChoice`: \"assign\" only"), "{err}");
}

/// `profileMismatch: {rgb?, cmyk?}` gives each model its own choice, and a model left out follows
/// its policy; a key other than rgb and cmyk, or a value other than the two choices, is refused and
/// pastes nothing.
#[test]
fn paste_takes_a_choice_for_each_model() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let both = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [both.0], "color": "#3366cc"}));
    exec(&mut s, "paint.setStroke", json!({"ids": [both.0], "color": {"c": 20, "m": 40, "y": 60, "k": 10}}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB, "cmyk": cms::DEVICE_CMYK}));
    let stroke_of = |s: &Session, id: vectorcraft_doc::NodeId| {
        s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.stroke()).and_then(|f| f.paint.color()).unwrap()
    };
    let ink0 = stroke_of(&s, both);
    select(&mut s, &[both]);
    exec(&mut s, "edit.copy", json!({}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    exec(&mut s, "edit.colorSettings", json!({"cmykPolicy": "convert"}));
    let blue0 = vectorcraft_color::Color::from_hex("#3366cc").unwrap();
    let pasted = |s: &mut Session, choice: serde_json::Value| {
        let r = exec(s, "edit.paste", json!({ "profileMismatch": choice }));
        let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
        let fill = s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill()).and_then(|f| f.paint.color()).unwrap();
        let picked = |model: &str| r["profileMismatch"].as_array().unwrap().iter().find(|m| m["model"] == model).map(|m| m["choice"].clone());
        (fill, stroke_of(s, id), picked("rgb"), picked("cmyk"))
    };
    // The CMYK policy (Convert to Working Space) converts the stroke from Device CMYK into the
    // working CMYK space, as a paste that leaves both models to their policies does.
    let (_, ink, rgb_policy, cmyk_policy) = pasted(&mut s, serde_json::Value::Null);
    assert_eq!((rgb_policy, &cmyk_policy), (Some(json!("preserveNumbers")), &Some(json!("convert"))));
    let device = cms::for_document(None, Some(cms::DEVICE_CMYK));
    assert!(ink != ink0 && cms::delta_e2000(device.lab(&ink0), cms::working().lab(&ink)) < 2.0, "{ink0:?} → {ink:?}");
    let converted_ink = ink;
    let (fill, ink, rgb, cmyk) = pasted(&mut s, json!({"rgb": "convert"}));
    assert_eq!((rgb, &cmyk, ink), (Some(json!("convert")), &cmyk_policy, converted_ink), "CMYK follows its policy");
    let source = cms::for_document(Some(cms::WIDE_GAMUT_RGB), None);
    assert!(fill != blue0 && cms::delta_e2000(source.lab(&blue0), cms::working().lab(&fill)) < 1.0, "{blue0:?} → {fill:?}");
    let (fill, ink, rgb, cmyk) = pasted(&mut s, json!({"cmyk": "preserveNumbers"}));
    assert_eq!((fill, ink, rgb, cmyk), (blue0, ink0, Some(json!("preserveNumbers")), Some(json!("preserveNumbers"))), "the inks keep their values");
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    let before = objects(&s);
    for bad in [json!({"lab": "convert"}), json!({"rgb": "keep"}), json!({"rgb": 1}), json!([]), json!(true)] {
        assert!(s.execute("edit.paste", &json!({ "profileMismatch": bad })).is_err(), "{bad}");
        assert!(s.execute("clipboard.conflicts", &json!({ "profileMismatch": bad })).is_err(), "{bad}");
    }
    assert_eq!(objects(&s), before, "a refused choice pastes nothing");
}

/// The fill color of the first object a paste reply `r` names, in the active document.
#[allow(clippy::unwrap_used, reason = "a test helper: a reply without objects fails the test")]
fn pasted_fill(s: &Session, r: &serde_json::Value) -> vectorcraft_color::Color {
    fill_of(s, vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap()))
}

/// A document assigned a profile that was removed since draws through the working space, and its
/// copied colors come from the working space too: pasted into a document with another profile, they
/// convert from it as `profileMismatch` says.
#[test]
fn a_removed_assigned_profile_pastes_as_the_working_space() {
    const NAME: &str = "Removed Wide RGB";
    let _g = settings();
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![] };
    cms::register_icc(&cms::icc_bytes(cms::WIDE_GAMUT_RGB).unwrap(), Some(NAME.into())).unwrap();
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": NAME}));
    exec(&mut s, "color.removeProfile", json!({"name": NAME}));
    select(&mut s, &[blue]);
    exec(&mut s, "edit.copy", json!({}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::DISPLAY_P3}));
    let m = exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"][0].clone();
    assert_eq!((&m["clipboard"], &m["convertible"]), (&json!(cms::canonical_name(cms::SRGB)), &json!(true)), "{m}");
    let r = exec(&mut s, "edit.paste", json!({"profileMismatch": "convert"}));
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    let (blue0, moved) = (vectorcraft_color::Color::from_hex("#3366cc").unwrap(), pasted_fill(&s, &r));
    let p3 = cms::for_document(Some(cms::DISPLAY_P3), None);
    assert!(moved != blue0 && cms::delta_e2000(cms::working().lab(&blue0), p3.lab(&moved)) < 1.0, "{blue0:?} → {moved:?}");
}

/// Colors copied from a document whose profile was removed after the copy can't convert: the
/// mismatch says so (convertible false, and default preserveNumbers under a converting policy), and
/// a paste that asks to convert keeps their numbers.
#[test]
fn colors_from_a_profile_removed_after_the_copy_keep_their_numbers() {
    const NAME: &str = "Copied Wide RGB";
    let _g = settings();
    let _reset = ColorSettingsReset { profile: Some(NAME), paths: vec![] };
    cms::register_icc(&cms::icc_bytes(cms::WIDE_GAMUT_RGB).unwrap(), Some(NAME.into())).unwrap();
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": NAME}));
    select(&mut s, &[blue]);
    exec(&mut s, "edit.copy", json!({}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    exec(&mut s, "edit.colorSettings", json!({"rgbPolicy": "convert"}));
    let m = exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"][0].clone();
    assert_eq!((&m["default"], &m["convertible"]), (&json!("convert"), &json!(true)), "both profiles are here: {m}");
    exec(&mut s, "color.removeProfile", json!({"name": NAME}));
    let m = exec(&mut s, "clipboard.profileMismatch", json!({}))["mismatches"][0].clone();
    assert_eq!((&m["clipboard"], &m["default"], &m["convertible"]), (&json!(NAME), &json!("preserveNumbers"), &json!(false)), "{m}");
    let r = exec(&mut s, "edit.paste", json!({"profileMismatch": "convert"}));
    assert_eq!(r["profileMismatch"][0]["choice"], "preserveNumbers");
    assert_eq!(pasted_fill(&s, &r), vectorcraft_color::Color::from_hex("#3366cc").unwrap());
}

/// Paste on All Artboards leaves copied artboards behind, so their backgrounds raise no mismatch for
/// it: clipboard.profileMismatch with allArtboards lists none, and the paste reports none. A plain
/// paste, which brings the artboard, still lists the background's model.
#[test]
fn paste_on_all_artboards_leaves_artboard_backgrounds_out() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let ink = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [ink.0], "color": {"c": 20, "m": 40, "y": 60, "k": 10}}));
    exec(&mut s, "paint.setStroke", json!({"ids": [ink.0], "none": true}));
    exec(&mut s, "artboard.setProps", json!({"index": 0, "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    exec(&mut s, "artboard.copy", json!({"index": 0}));
    exec(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let models = |s: &mut Session, p: serde_json::Value| exec(s, "clipboard.profileMismatch", p)["mismatches"].as_array().unwrap().len();
    assert_eq!(models(&mut s, json!({})), 1, "a plain paste brings the RGB background");
    assert_eq!(models(&mut s, json!({"allArtboards": true})), 0);
    assert!(s.execute("clipboard.profileMismatch", &json!({"allArtboards": "yes"})).is_err());
    let r = exec(&mut s, "edit.pasteOnAllArtboards", json!({"profileMismatch": "convert"}));
    assert!(r.get("profileMismatch").is_none(), "{r}");
}

/// The swatch conflicts Paste on All Artboards raises leave copied artboard backgrounds out, as the
/// paste does: pasting back into the document the objects came from, where only the RGB background
/// of the copied artboard now has another profile, converts nothing for it, so its edited CMYK
/// swatch raises no conflict. A plain paste, which brings the background and converts it, compares
/// the swatches and lists the edited one.
#[test]
fn swatch_conflicts_on_all_artboards_leave_artboard_backgrounds_out() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    exec(&mut s, "swatch.new", json!({"name": "Brand", "color": {"c": 20, "m": 40, "y": 60, "k": 10}, "global": true}));
    let ink = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [ink.0], "swatch": "Brand"}));
    exec(&mut s, "paint.setStroke", json!({"ids": [ink.0], "none": true}));
    exec(&mut s, "artboard.setProps", json!({"index": 0, "color": "#3366cc"}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    exec(&mut s, "artboard.copy", json!({"index": 0}));
    exec(&mut s, "edit.assignProfile", json!({"rgb": null}));
    exec(&mut s, "swatch.edit", json!({"name": "Brand", "color": {"c": 60, "m": 40, "y": 20, "k": 0}}));
    let conflicts = |s: &mut Session, p: serde_json::Value| exec(s, "clipboard.conflicts", p)["swatches"].as_array().unwrap().len();
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert"})), 1, "a plain paste converts the RGB background");
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert", "allArtboards": true})), 0);
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert", "allArtboards": false})), 1);
    assert_eq!(conflicts(&mut s, json!({"profileMismatch": "convert", "allArtboards": null})), 1, "null is the default");
    for bad in [json!("yes"), json!(1)] {
        assert!(s.execute("clipboard.conflicts", &json!({ "allArtboards": bad })).is_err(), "{bad}");
    }
    let r = exec(&mut s, "edit.pasteOnAllArtboards", json!({"profileMismatch": "convert"}));
    assert!(r.get("profileMismatch").is_none(), "{r}");
}

/// A converting paste back into the document the objects came from (its profile changed since the
/// copy) still takes its image blobs as the document's own: an image whose blob under its key
/// changed since keeps that key, and so shows the document's blob.
#[test]
fn a_converting_paste_into_its_own_document_keeps_its_image_keys() {
    let _g = settings();
    let _reset = ColorSettingsReset { profile: None, paths: vec![] };
    let mut s = session();
    let blue = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blue.0], "color": "#3366cc"}));
    let blob = |b: u8| vectorcraft_doc::ImageBlob::png(vec![b; 8]);
    let image = s
        .edit("Place", |d, _| {
            d.images.insert("pic".into(), blob(1));
            let id = d.alloc_id();
            let im = vectorcraft_doc::ImageObject {
                key: "pic".into(),
                width: 4,
                height: 4,
                xf: Affine::translate((20.0, 0.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(im))))?;
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[blue, image]);
    exec(&mut s, "edit.copy", json!({}));
    s.edit("Replace Image", |d, _| {
        d.images.insert("pic".into(), blob(2));
        Ok(())
    })
    .unwrap();
    exec(&mut s, "edit.assignProfile", json!({"rgb": cms::WIDE_GAMUT_RGB}));
    let r = exec(&mut s, "edit.paste", json!({"profileMismatch": "convert"}));
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    let doc = &s.doc().unwrap().doc;
    let keys: Vec<String> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|id| match &doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap()))?.kind {
            vectorcraft_doc::NodeKind::Image(im) => Some(im.key.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(keys, ["pic"]);
    assert_eq!(doc.images.len(), 1, "no copy of the old blob");
}
