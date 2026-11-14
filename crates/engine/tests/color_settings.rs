//! Edit → Color Settings and Assign Profile with non-default profiles. The Color Settings are
//! process-wide and every colour shown or rendered goes through them, so these tests run in their
//! own test binary (process) instead of next to the engine's unit tests, which would see the
//! switched working spaces mid-test; here they take turns.

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::cms;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::colormgmt::{PROFILES_KEY, doc_profiles};
use vectorcraft_engine::store::MAX_PRESET_FILE;
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_testkit::fixtures::{exec, id_of, rect, select, session};

/// The active Color Settings: one test at a time.
fn settings() -> MutexGuard<'static, ()> {
    static SETTINGS: Mutex<()> = Mutex::new(());
    SETTINGS.lock().unwrap_or_else(|e| e.into_inner())
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
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
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
    let png = image::load_from_memory(&vectorcraft_testkit::format::base64_decode(data).unwrap()).unwrap().to_rgba8();
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
    let chosen = cms::ColorSettings { rgb: loaded.into(), cmyk: cms::DEVICE_CMYK.into(), intent: cms::Intent::Perceptual, bpc: false };
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
