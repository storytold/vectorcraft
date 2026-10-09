//! Installed system fonts: found by family name by every lookup, whatever ran before it (#130).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::fontdb::fonts_outside;

const FAMILY: &str = "Sysfont Sans3";

/// A bundled Source Sans 3 file renamed [`FAMILY`] (as long as the original name).
fn renamed(file: &str) -> Vec<u8> {
    renamed_to(file, FAMILY)
}

/// A bundled Source Sans 3 file renamed `family`, which is as long as the original name.
fn renamed_to(file: &str, family: &str) -> Vec<u8> {
    assert_eq!(family.len(), "Source Sans 3".len());
    let mut data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

/// The fonts in `faces` as one collection (TTC) file.
fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"ttcf".to_vec();
    out.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
    let mut base = 12 + 4 * faces.len();
    for f in faces {
        out.extend_from_slice(&(base as u32).to_be_bytes());
        base += f.len();
    }
    for f in faces {
        // Table offsets count from the start of the collection.
        let start = out.len() as u32;
        let mut f = f.clone();
        let tables = u16::from_be_bytes([f[4], f[5]]) as usize;
        for r in 0..tables {
            let at = 12 + r * 16 + 8;
            let off = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + start;
            f[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        out.extend_from_slice(&f);
    }
    out
}

/// A font folder (with a subfolder, a damaged font and a file that isn't a font) holding
/// [`FAMILY`] Regular and Bold.
fn font_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-sysfonts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Sub")).unwrap();
    std::fs::write(dir.join("Sysfont-Regular.ttf"), renamed("SourceSans3-Regular.ttf")).unwrap();
    std::fs::write(dir.join("Sub/Sysfont-Bold.TTF"), renamed("SourceSans3-Bold.ttf")).unwrap();
    std::fs::write(dir.join("damaged.ttf"), b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
    std::fs::write(dir.join("readme.txt"), FAMILY).unwrap();
    dir
}

fn has(list: &[String], family: &str) -> bool {
    list.iter().any(|f| f == family)
}

#[test]
fn every_lookup_by_name_finds_installed_fonts_in_a_fresh_database() {
    let dir = font_dir("fresh");
    // Each lookup is the first thing a new database is asked. A folder listed twice is read once.
    let db = || FontDb::with_font_dirs(vec![dir.clone(), dir.join("Sub/..")]);
    let f = db().face("sysfont sans3", "Bold").unwrap();
    assert_eq!((f.family.as_str(), f.style.as_str()), (FAMILY, "Bold"));
    assert_eq!(f.path().map(std::fs::canonicalize).unwrap().unwrap(), std::fs::canonicalize(dir.join("Sub/Sysfont-Bold.TTF")).unwrap());
    assert!(db().has_family(FAMILY));
    assert!(has(&db().families(), FAMILY));
    assert_eq!(db().styles(FAMILY), ["Regular", "Bold"]);
    assert_eq!(db().load_system_fonts(), 2);
    assert_eq!(db().find_family("SysfontSans3").as_deref(), Some(FAMILY), "a PostScript-style name");
    // Missing fonts stay missing.
    let db = db();
    assert!(!db.has_family("No Such Font"));
    assert_eq!(db.face("No Such Font", "Regular").unwrap().family, FALLBACK_FAMILY);
}

#[test]
fn the_scan_reads_collections_and_loads_fonts_only_when_used() {
    let dir = font_dir("collection");
    std::fs::remove_file(dir.join("Sysfont-Regular.ttf")).unwrap();
    std::fs::remove_file(dir.join("Sub/Sysfont-Bold.TTF")).unwrap();
    std::fs::write(dir.join("Sysfont.ttc"), collection(&[renamed("SourceSans3-Regular.ttf"), renamed("SourceSans3-Bold.ttf")])).unwrap();
    let db = FontDb::with_font_dirs(vec![dir.clone()]);
    assert_eq!(db.styles(FAMILY), ["Regular", "Bold"]);
    // Cataloged, not loaded.
    assert!(!db.is_loaded(FAMILY));
    let bold = db.face(FAMILY, "Bold").unwrap();
    assert_eq!((bold.style.as_str(), bold.face_index()), ("Bold", 1));
    assert_eq!(db.find(FAMILY, "Regular").unwrap().face_index(), 0, "the collection's faces load together");
}

#[test]
fn fonts_without_outlines_the_engine_draws_are_left_out() {
    let dir = font_dir("outlines");
    // The Bold's TrueType outlines under another tag, as in fonts that keep their outlines in
    // Apple's `hvgl` table or hold bitmaps only. The tag keeps the table directory sorted.
    let bold = dir.join("Sub/Sysfont-Bold.TTF");
    let mut data = std::fs::read(&bold).unwrap();
    let tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let rec = (0..tables).map(|i| 12 + 16 * i).find(|&r| &data[r..r + 4] == b"glyf").unwrap();
    data[rec..rec + 4].copy_from_slice(b"gxyz");
    std::fs::write(&bold, data).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    assert_eq!(db.styles(FAMILY), ["Regular"]);
    assert_eq!(db.load_system_fonts(), 1);
}

#[test]
fn rescanning_finds_fonts_installed_since() {
    let dir = font_dir("rescan");
    let later = dir.join("Later");
    let db = FontDb::with_font_dirs(vec![later.clone()]);
    assert!(!db.has_family(FAMILY));
    let (generation, families) = (db.generation(), db.family_list());
    std::fs::create_dir_all(&later).unwrap();
    std::fs::copy(dir.join("Sysfont-Regular.ttf"), later.join("Sysfont-Regular.ttf")).unwrap();
    assert!(!db.has_family(FAMILY), "the folders are scanned once, not on every miss");
    assert_eq!(db.load_system_fonts(), 1);
    assert!(db.has_family(FAMILY) && has(&db.family_list(), FAMILY) && !has(&families, FAMILY));
    assert_ne!(db.generation(), generation);
}

#[test]
fn a_background_scan_serves_the_first_lookup() {
    let dir = font_dir("background");
    let db: &'static FontDb = Box::leak(Box::new(FontDb::with_font_dirs(vec![dir])));
    db.scan_in_background();
    // Waits for the scan when it is still running.
    assert!(has(&db.families(), FAMILY));
    assert_eq!(db.load_system_fonts(), 2, "a rescan catalogs the two faces again");
}

#[test]
fn the_family_list_is_shared_until_fonts_change() {
    let db = FontDb::with_font_dirs(vec![]);
    let a = db.family_list();
    assert!(Arc::ptr_eq(&a, &db.family_list()));
    assert!(db.add_font(renamed("SourceSans3-Regular.ttf")) > 0);
    let b = db.family_list();
    assert!(!Arc::ptr_eq(&a, &b) && has(&b, FAMILY) && !has(&a, FAMILY));
}

#[test]
fn font_lists_leave_out_hidden_system_families_which_still_resolve() {
    // macOS names the faces it keeps for its own interface with a leading "." (".SF NS").
    const HIDDEN: &str = ".Sysfont Sans";
    const DOTTED: &str = "Sysfont.Sans3";
    let dir = font_dir("hidden");
    std::fs::write(dir.join("Hidden.ttf"), renamed_to("SourceSans3-Regular.ttf", HIDDEN)).unwrap();
    std::fs::write(dir.join("Dotted.ttf"), renamed_to("SourceSans3-Regular.ttf", DOTTED)).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let listed = db.menu_family_list();
    assert!(has(&listed, FAMILY) && has(&listed, FALLBACK_FAMILY));
    assert!(!has(&listed, HIDDEN), "a leading dot hides a family: {listed:?}");
    assert!(has(&listed, DOTTED), "a dot elsewhere doesn't");
    assert_eq!(listed.len() + 1, db.family_list().len(), "only the hidden family is left out");
    assert!(Arc::ptr_eq(&listed, &db.menu_family_list()), "shared until the fonts change");
    // Documents and fallbacks that name it still find it.
    assert!(has(&db.families(), HIDDEN) && db.has_family(HIDDEN));
    assert_eq!(db.find_family(HIDDEN).as_deref(), Some(HIDDEN));
    assert_eq!(db.face(HIDDEN, "Regular").unwrap().family, HIDDEN);
    // Loading it changed the fonts: the list is built again, still without it.
    let again = db.menu_family_list();
    assert!(!Arc::ptr_eq(&listed, &again) && *listed == *again);
}

#[test]
fn installed_styles_of_a_loaded_family_load_when_asked_for() {
    let dir = font_dir("styles");
    let db = FontDb::with_font_dirs(vec![dir]);
    // The family is loaded with only its Regular face (as when a bundled family is also installed).
    assert_eq!(db.add_font(renamed("SourceSans3-Regular.ttf")), 1);
    assert_eq!(db.face(FAMILY, "Bold").unwrap().style, "Bold");
    // A style nobody has still gets the closest one.
    assert_eq!(db.face(FAMILY, "Black").unwrap().style, "Bold");
}

/// Documents name fonts by PostScript name, and families can hold hyphens: the installed face of
/// that exact name gives the family and style, however the name splits.
#[test]
fn installed_faces_are_found_by_postscript_name() {
    let dir = font_dir("postscript");
    let db = FontDb::with_font_dirs(vec![dir]);
    // The renamed Source Sans 3 files keep their PostScript names.
    assert_eq!(db.by_postscript_name("SourceSans3-Bold"), Some((FAMILY.to_string(), "Bold".to_string())));
    assert_eq!(db.by_postscript_name("sourcesans3-regular"), Some((FAMILY.to_string(), "Regular".to_string())), "any case");
    assert_eq!(db.by_postscript_name("Rounded-X-Mplus-1c-black"), None);
}

/// Windows lists fonts installed as shortcuts, or by programs into their own folders, by their
/// files' full paths: those files are scanned too (#443).
#[test]
fn a_font_file_named_by_itself_is_scanned() {
    let dir = font_dir("file");
    let db = FontDb::with_font_dirs(vec![dir.join("Sub/Sysfont-Bold.TTF"), dir.join("No Such Font.ttf")]);
    assert_eq!(db.styles(FAMILY), ["Bold"]);
    assert_eq!(db.load_system_fonts(), 1);
}

#[test]
fn registered_fonts_outside_the_font_folders_are_their_full_paths() {
    let dir = std::env::temp_dir().join("Fonts");
    let elsewhere = std::env::temp_dir().join("Downloads").join("Montserrat-Regular.ttf");
    let registered = [
        // A file in the Windows font folder, by name.
        "arial.ttf".into(),
        // A file in a font folder (ignoring case).
        dir.join("Lato-Regular.ttf").to_string_lossy().to_uppercase(),
        elsewhere.to_string_lossy().into_owned(),
        elsewhere.to_string_lossy().into_owned(),
    ];
    assert_eq!(fonts_outside(registered, &[dir]), [elsewhere]);
}

/// Fonts installed while the app runs are found when it comes back to the front, without
/// scanning every time (#443).
#[test]
fn the_database_tells_when_fonts_were_installed_or_removed_since_its_scan() {
    let dir = font_dir("changed");
    let db = FontDb::with_font_dirs(vec![dir.clone(), dir.join("Later")]);
    assert!(!db.installed_fonts_changed(), "nothing to compare with before the first scan");
    assert!(!db.has_family("Other Sans 33"));
    assert!(!db.installed_fonts_changed());
    // Past the file system's clock tick, so the folders' times differ from the scan's.
    let tick = || std::thread::sleep(std::time::Duration::from_millis(50));
    tick();
    std::fs::write(dir.join("Sub/Other.ttf"), renamed_to("SourceSans3-Regular.ttf", "Other Sans 33")).unwrap();
    assert!(db.installed_fonts_changed(), "a font installed into a subfolder");
    assert_eq!(db.load_system_fonts(), 3);
    assert!(db.has_family("Other Sans 33") && !db.installed_fonts_changed());
    tick();
    std::fs::create_dir_all(dir.join("Later")).unwrap();
    assert!(db.installed_fonts_changed(), "a font folder that didn't exist");
    db.load_system_fonts();
    tick();
    std::fs::remove_file(dir.join("Sysfont-Regular.ttf")).unwrap();
    assert!(db.installed_fonts_changed(), "a font removed");
    assert_eq!(db.load_system_fonts(), 2);
    assert_eq!(db.styles(FAMILY), ["Bold"]);
}

/// A font a font service loads from a file without an extension, outside the font folders.
fn service_font() -> PathBuf {
    std::env::temp_dir().join(format!("vc-sysfonts-{}-service", std::process::id())).join(".29457")
}

/// The fonts a font service (Adobe Fonts on Windows, through DirectWrite) loads in place are
/// cataloged: named by the platform's lister, read whatever their file is called (#579).
#[test]
fn fonts_the_platform_lists_outside_the_font_folders_are_cataloged() {
    let file = service_font();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, renamed_to("SourceSans3-Regular.ttf", "Service Sans3")).unwrap();
    // The lister's files outside the font folders are scanned with them, each once (a collection
    // lists its file once per face).
    fn lister() -> Vec<String> {
        vec![service_font().to_string_lossy().into_owned(); 2]
    }
    set_platform_font_files(lister);
    let dirs = system_font_dirs();
    assert_eq!(dirs.iter().filter(|d| **d == file).count(), 1, "{dirs:?}");
    // A file named by itself is read without a font's extension, one in a folder isn't.
    let db = FontDb::with_font_dirs(vec![file.clone()]);
    assert_eq!(db.styles("Service Sans3"), ["Regular"]);
    assert!(!FontDb::with_font_dirs(vec![file.parent().unwrap().to_path_buf()]).has_family("Service Sans3"));
}
