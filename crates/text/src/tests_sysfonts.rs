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

/// The bundled Source Sans 3 Regular with a `name` table of `records` (platform, language, name id,
/// text): Windows Unicode (UTF-16) or Mac Roman (ASCII here).
fn named(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
    let mut records = records.to_vec();
    records.sort_by_key(|r| (r.0, r.1, r.2));
    let (mut name, mut strings) = (vec![], vec![]);
    let n = records.len() as u16;
    for v in [0, n, 6 + 12 * n] {
        name.extend_from_slice(&v.to_be_bytes());
    }
    for &(platform, lang, id, s) in &records {
        let bytes: Vec<u8> = if platform == 3 { s.encode_utf16().flat_map(u16::to_be_bytes).collect() } else { s.as_bytes().to_vec() };
        let encoding = if platform == 3 { 1 } else { 0 };
        for v in [platform, encoding, lang, id, bytes.len() as u16, strings.len() as u16] {
            name.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend(bytes);
    }
    name.extend(strings);
    let base = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let font = skrifa::FontRef::new(&base).unwrap();
    let mut builder = write_fonts::FontBuilder::new();
    builder.add_raw(write_fonts::types::Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = write_fonts::types::Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    builder.build()
}

const EN: u16 = 0x0409;
const JA: u16 = 0x0411;

/// Copying a file a search offers into a folder the scan reads makes the font resolve exactly from
/// that file, and a file it doesn't offer doesn't: the search matches fonts as `resolve` finds
/// them. Each case resolves the font, copies the file into a scanned folder, rescans and resolves
/// again, as the app does.
#[test]
fn wanted_fonts_match_as_resolve_finds_them() {
    let one = named(&[(3, EN, 1, "Parity One"), (3, EN, 2, "Regular"), (3, EN, 6, "ParityOne-Regular")]);
    let two = named(&[
        (3, EN, 1, "Parity Two"),
        (3, EN, 2, "Regular"),
        (3, EN, 6, "ParityTwo-Regular"),
        (3, EN, 16, "Parity Two"),
        (3, JA, 16, "パリティ二"),
        (3, EN, 17, "Regular"),
    ]);
    // Laid out like Hiragino Sans W3: a legacy family per weight, paired with its legacy style, and
    // a PostScript name that reads as the legacy family.
    let three = |postscript: &str| {
        named(&[
            (1, 0, 1, "Parity Three"),
            (1, 0, 2, "W3"),
            (3, EN, 1, "Parity Three W3"),
            (3, EN, 2, "Regular"),
            (3, EN, 6, postscript),
            (3, EN, 16, "Parity Three"),
            (3, EN, 17, "W3"),
            (3, JA, 1, "パリティ三 W3"),
            (3, JA, 2, "Regular"),
            (3, JA, 16, "パリティ三"),
            (3, JA, 17, "W3"),
        ])
    };
    let (three, three_ps) = (three("ParityThree-W3"), three("PThree-W3"));
    // A PostScript name that is the family's name.
    let findme = named(&[(3, EN, 1, "Findme"), (3, EN, 2, "Regular"), (3, EN, 6, "Findme")]);
    let psdiff = named(&[(3, EN, 1, "Psdiff Sans"), (3, EN, 2, "Regular"), (3, EN, 6, "PsdiffSans-Regular")]);
    let variable = crate::test_fonts::variable_font().unwrap();
    let four = |style: &str, ja_style: &str| {
        named(&[
            (3, EN, 1, "Parity Four"),
            (3, EN, 2, style),
            (3, EN, 6, &format!("ParityFour-{style}")),
            (3, EN, 16, "Parity Four"),
            (3, EN, 17, style),
            (3, JA, 16, "パリティ四"),
            (3, JA, 17, ja_style),
        ])
    };
    let (four_regular, four_bold) = (four("Regular", "標準"), four("Bold", "太字"));
    let five = named(&[(3, EN, 1, "Parity Five"), (3, EN, 2, "Bold"), (3, EN, 6, "ParityFive-Bold"), (3, EN, 17, "Bold"), (3, JA, 17, "太字")]);
    // (family, style, Parity Four Regular installed, candidate file, provides it)
    let cases: Vec<(&str, &str, bool, &[u8], bool)> = vec![
        ("Parity One", "Regular", false, &one, true),
        ("parity one", "REGULAR", false, &one, true),
        ("Parity One", "Bold", false, &one, false),
        ("Parity One", "Regular", false, &four_bold, false),
        ("パリティ二", "Regular", false, &two, true),
        ("パリティ二", "Bold", false, &two, false),
        ("Parity Three W3", "Regular", false, &three_ps, true),
        ("Parity Three W3", "Bold", false, &three_ps, false),
        ("PThree-W3", "Bold", false, &three_ps, true),
        // The legacy family reads as the PostScript name, which names the W3 face.
        ("Parity Three W3", "Bold", false, &three, true),
        ("パリティ三", "W3", false, &three, true),
        ("パリティ三", "Bold", false, &three, false),
        ("Findme", "Bold", false, &findme, false),
        ("Find-me", "Regular", false, &findme, true),
        // A family's name goes before a PostScript name that reads the same.
        ("Find-me", "Bold", false, &findme, false),
        ("PsdiffSans-Regular", "Bold", false, &psdiff, true),
        ("Varitest Sans", "SemiBold", false, &variable, true),
        ("Varitest Sans", "Normal", false, &variable, true),
        ("VaritestSans-Bold", "Regular", false, &variable, true),
        ("Varitest Sans", "Black", false, &variable, false),
        ("Parity Five", "太字", false, &five, true),
        ("Parity Four", "Bold", true, &four_bold, true),
        ("パリティ四", "Bold", true, &four_bold, true),
        ("Parity Four", "太字", true, &four_bold, false),
        ("Parity Four", "Bold", true, &one, false),
    ];
    for (i, (family, style, installed, candidate, provides)) in cases.into_iter().enumerate() {
        let dir = std::env::temp_dir().join(format!("vc-sysfonts-{}-wanted-{i}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (base, new) = (dir.join("base"), dir.join("new"));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        if installed {
            std::fs::write(base.join("ParityFour-Regular.ttf"), &four_regular).unwrap();
        }
        let db = FontDb::with_font_dirs(vec![base, new.clone()]);
        let (face, m) = db.resolve(family, style).unwrap();
        assert_eq!(m, if installed { FontMatch::Style } else { FontMatch::Missing }, "case {i}: {family} {style}");
        let wanted = WantedFont { family: family.into(), style: style.into(), installed: (m == FontMatch::Style).then(|| face.family.clone()) };
        let file = new.join("Candidate.ttf");
        std::fs::write(&file, candidate).unwrap();
        db.load_system_fonts();
        let (face, m) = db.resolve(family, style).unwrap();
        let exact = m == FontMatch::Exact && face.path().map(|p| std::fs::canonicalize(p).unwrap()) == Some(std::fs::canonicalize(&file).unwrap());
        assert_eq!(exact, provides, "case {i}: resolve gave {face:?} {m:?} for {family} {style}");
        assert_eq!(WantedFonts::new(&[wanted]).provided_by(&file) == [0], provides, "case {i}: the search, for {family} {style}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A search reads only so much of a file: of a collection whose header gives 300 faces it reads
/// the first 64, and a cap on the bytes read ends it early.
#[test]
fn a_search_reads_at_most_its_share_of_a_font_file() {
    use crate::fontdb::file_face_names_within;
    let face = renamed("SourceSans3-Regular.ttf");
    let tables = u16::from_be_bytes([face[4], face[5]]) as usize;
    let len = |tag: &[u8]| {
        let r = (0..tables).map(|i| 12 + 16 * i).find(|&r| &face[r..r + 4] == tag).unwrap();
        u64::from(u32::from_be_bytes(face[r + 12..r + 16].try_into().unwrap()))
    };
    // What one face costs: its table directory and its name and OS/2 tables.
    let per_face = 12 + 16 * tables as u64 + len(b"name") + len(b"OS/2");
    let n = 300;
    let base = 12 + 4 * n;
    let mut ttc = b"ttcf".to_vec();
    ttc.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    ttc.extend_from_slice(&(n as u32).to_be_bytes());
    for _ in 0..n {
        ttc.extend_from_slice(&(base as u32).to_be_bytes());
    }
    // Every face is the one font, whose table offsets count from the start of the collection.
    let mut f = face.clone();
    for r in 0..tables {
        let at = 12 + r * 16 + 8;
        let off = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + base as u32;
        f[at..at + 4].copy_from_slice(&off.to_be_bytes());
    }
    ttc.extend(f);
    let dir = std::env::temp_dir().join(format!("vc-sysfonts-{}-many", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Many.ttc");
    std::fs::write(&file, ttc).unwrap();
    assert_eq!(file_face_names_within(&file, 256, u64::MAX).len(), 256);
    assert_eq!(file_face_names_within(&file, 64, u64::MAX).len(), 64);
    // The header, 64 offsets and three faces.
    assert_eq!(file_face_names_within(&file, 64, 12 + 4 * 64 + 3 * per_face).len(), 3);
    let wanted = WantedFonts::new(&[WantedFont { family: FAMILY.into(), style: "Regular".into(), installed: None }]);
    assert_eq!(wanted.provided_by(&file), [0]);
    assert!(wanted.provided_by(&dir.join("No Such.ttf")).is_empty());
}

#[test]
fn font_files_are_told_by_their_extension() {
    for name in ["a/B.TTF", "c.otf", "d.Ttc", "e.otc"] {
        assert!(is_font_file(Path::new(name)), "{name}");
    }
    for name in ["f.ttf.txt", "ttf", ".29457", "g.woff2", "h.fon"] {
        assert!(!is_font_file(Path::new(name)), "{name}");
    }
}

/// VectorCraft's own Fonts folder joins the scan once set, even before it exists; only the first
/// folder set counts.
#[test]
fn the_app_font_folder_is_read_by_the_scan() {
    let dir = std::env::temp_dir().join(format!("vc-sysfonts-{}-app-fonts", std::process::id()));
    set_app_font_dir(dir.clone());
    set_app_font_dir(dir.join("Other"));
    assert_eq!(app_font_dir(), Some(dir.as_path()));
    let dirs = system_font_dirs();
    assert_eq!(dirs.iter().filter(|d| **d == dir).count(), 1, "{dirs:?}");
    assert!(!dirs.contains(&dir.join("Other")));
}
