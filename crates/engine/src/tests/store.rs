//! The stores of PDF presets and ICC profiles (#2168, #2174): what one session saves, the next one
//! finds; damaged or oversized files are skipped, never fatal.

use serde_json::{Value, json};
use vectorcraft_color::cms;

use crate::Session;
use crate::store::{MAX_PRESET_FILE, file_stem};

fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = vectorcraft_testkit::temp_dir(&format!("store-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn saved_names(s: &mut Session) -> Vec<String> {
    let v = s.execute("pdf.preset.list", &json!({})).unwrap();
    v["presets"].as_array().unwrap().iter().filter(|p| p["builtIn"] == false).map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn saved_presets_come_back_in_the_next_session_and_renames_and_deletes_follow() {
    let dir = fresh_dir("presets").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Press", "description": "for the printer", "preset": "Press Quality"})).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web"})).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web", "newName": "Screen"})).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Old"})).unwrap();
    a.execute("pdf.preset.delete", &json!({"name": "Old"})).unwrap();

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir.clone());
    assert_eq!(saved_names(&mut b), ["Press", "Screen"]);
    let press = b.prefs.pdf_presets.iter().find(|p| p.name == "Press").unwrap();
    assert_eq!(press.description, "for the printer");
    assert_eq!(press.settings, b.pdf_presets().into_iter().find(|p| p.name == "Press Quality").unwrap().settings);
    // The file of a name that is gone is gone.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert!(!dir.join(format!("{}.vcpdfpresets", file_stem("Old"))).exists());
}

#[test]
fn a_rename_that_cannot_be_written_keeps_the_preset_under_its_old_name() {
    let dir = fresh_dir("rename-fails").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Web", "description": "for screens"})).unwrap();
    // The test puts a folder where the new name's file goes, so the write fails as it would on a full disk.
    std::fs::create_dir(dir.join(format!("{}.vcpdfpresets", file_stem("Screen")))).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web", "newName": "Screen"})).unwrap();

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir.clone());
    assert_eq!(saved_names(&mut b), ["Web"]);
    assert_eq!(b.prefs.pdf_presets[0].description, "for screens");
    assert!(dir.join(format!("{}.vcpdfpresets", file_stem("Web"))).is_file());
}

#[test]
fn a_rename_that_only_changes_case_keeps_one_file() {
    let dir = fresh_dir("rename-case").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Web"})).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web", "newName": "WEB", "description": "renamed"})).unwrap();

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir.clone());
    assert_eq!(saved_names(&mut b), ["WEB"]);
    assert_eq!(b.prefs.pdf_presets[0].description, "renamed");
    assert_eq!(file_names(&dir), [format!("{}.vcpdfpresets", file_stem("WEB"))]);
}

fn file_names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
}

/// Windows does not allow replacing a read-only file or writing into it, so the write fails.
#[cfg(windows)]
#[test]
fn a_rename_that_only_changes_case_and_cannot_be_written_keeps_the_old_file() {
    let dir = fresh_dir("rename-case-fails").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Web", "description": "for screens"})).unwrap();
    let old = dir.join(format!("{}.vcpdfpresets", file_stem("Web")));
    let writable = std::fs::metadata(&old).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&old, read_only).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web", "newName": "WEB"})).unwrap();
    std::fs::set_permissions(&old, writable).unwrap();

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir.clone());
    assert_eq!(saved_names(&mut b), ["Web"]);
    assert_eq!(b.prefs.pdf_presets[0].description, "for screens");
    assert_eq!(file_names(&dir), [format!("{}.vcpdfpresets", file_stem("Web"))]);
}

/// Where file names keep case apart, a folder named like the new file makes the rename fail.
#[cfg(not(any(windows, target_vendor = "apple")))]
#[test]
fn a_rename_that_only_changes_case_and_cannot_be_moved_keeps_the_old_preset() {
    let dir = fresh_dir("rename-case-blocked").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Web", "description": "for screens"})).unwrap();
    std::fs::create_dir(dir.join(format!("{}.vcpdfpresets", file_stem("WEB")))).unwrap();
    a.execute("pdf.preset.save", &json!({"name": "Web", "newName": "WEB", "description": "renamed"})).unwrap();

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir.clone());
    assert_eq!(saved_names(&mut b), ["Web"]);
    assert_eq!(b.prefs.pdf_presets[0].description, "for screens");
}

#[test]
fn a_folder_that_does_not_exist_yet_takes_the_presets_the_preferences_held() {
    let dir = fresh_dir("seed").join("PDF Presets");
    let mut a = Session::new();
    a.execute("pdf.preset.save", &json!({"name": "From Preferences"})).unwrap();
    a.attach_pdf_preset_store(dir.clone());
    assert!(dir.is_dir(), "written at once");
    let mut b = Session::new();
    b.attach_pdf_preset_store(dir);
    assert_eq!(saved_names(&mut b), ["From Preferences"]);
}

#[test]
fn damaged_unsafe_and_oversized_preset_files_are_skipped() {
    let dir = fresh_dir("damaged").join("PDF Presets");
    let mut a = Session::new();
    a.attach_pdf_preset_store(dir.clone());
    a.execute("pdf.preset.save", &json!({"name": "Good"})).unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(dir.join(name), bytes).unwrap();
    write("a-not-json.vcpdfpresets", b"{ not json");
    write("b-other-format.vcpdfpresets", br#"{"format": "something else", "presets": []}"#);
    let huge = vec![b' '; MAX_PRESET_FILE as usize + 1];
    write("c-huge.vcpdfpresets", &huge);
    write("d-readme.txt", b"ignored");
    let good = std::fs::read_to_string(dir.join(format!("{}.vcpdfpresets", file_stem("Good")))).unwrap();
    // A built-in name, an empty name, a second "good" in another case, and values no export accepts.
    let mut file: Value = serde_json::from_str(&good).unwrap();
    let mut preset = file["presets"][0].clone();
    for name in ["High Quality Print", "  ", "GOOD"] {
        preset["name"] = json!(name);
        file["presets"].as_array_mut().unwrap().push(preset.clone());
    }
    preset["name"] = json!("Bad Values");
    preset["settings"]["advanced"]["fontSubsetPercent"] = json!(500);
    file["presets"].as_array_mut().unwrap().push(preset);
    write("e-mixed.vcpdfpresets", file.to_string().as_bytes());

    let mut b = Session::new();
    b.attach_pdf_preset_store(dir);
    assert_eq!(saved_names(&mut b), ["Good"]);
}

/// An ICC file of an RGB profile named `name` (a built-in one with its description replaced by a
/// name of the same length).
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

#[test]
fn stored_profiles_register_again_and_removing_one_removes_its_copy() {
    let dir = fresh_dir("profiles").join("Color Profiles");
    std::fs::create_dir_all(&dir).unwrap();
    let name = "StoreRGB-1";
    let file = format!("{}.icc", file_stem(name));
    std::fs::write(dir.join("junk.icc"), b"not a profile").unwrap();
    std::fs::write(dir.join("empty.icc"), b"").unwrap();
    std::fs::write(dir.join(&file), icc_named(name)).unwrap();
    assert!(cms::profile(name).is_none());

    let mut s = Session::new();
    s.attach_profile_store(dir.clone());
    assert!(cms::profile(name).is_some_and(|p| !p.builtin), "registered from the store");

    // A profile loaded from elsewhere is copied in, whatever the file is called.
    let other = "StoreRGB-2";
    let loose = vectorcraft_testkit::temp_dir("store-loose").join("whatever.icm");
    std::fs::write(&loose, icc_named(other)).unwrap();
    let r = s.execute("color.loadProfile", &json!({"path": loose.to_str().unwrap()})).unwrap();
    assert_eq!((r["name"].as_str(), r["remembered"].as_bool()), (Some(other), Some(true)));
    std::fs::remove_file(&loose).unwrap();
    assert_eq!(std::fs::read(dir.join(format!("{}.icc", file_stem(other)))).unwrap(), icc_named(other));

    let r = s.execute("color.removeProfile", &json!({"name": name})).unwrap();
    assert_eq!(r["removed"], name);
    assert!(cms::profile(name).is_none() && !dir.join(&file).exists());
    assert!(s.execute("color.removeProfile", &json!({"name": name})).is_err(), "already gone");
    assert!(s.execute("color.removeProfile", &json!({"name": cms::SRGB})).is_err(), "built-in profiles stay");
    s.execute("color.removeProfile", &json!({"name": other})).unwrap();
}

/// Puts the default Color Settings back, unregisters `profile` and deletes `paths` when dropped,
/// also after a failed assertion, so the next test holding `GLOBAL` starts from a clean state.
struct ColorSettingsReset {
    profile: Option<&'static str>,
    paths: Vec<std::path::PathBuf>,
}

impl Drop for ColorSettingsReset {
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
    let _g = super::colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = fresh_dir("color-settings");
    let (profiles, file) = (root.join("Color Profiles"), root.join("Color Settings.json"));
    let loaded = "StoreRGB-6";
    let icc = vectorcraft_testkit::temp_dir("store-color-settings").join("loaded.icc");
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

    // The next start begins with the defaults.
    cms::set_active(&cms::ColorSettings::default()).unwrap();
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
    let _g = super::colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
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
