use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

const ZH: fn() -> Lang = || Lang::from_code("zh-hant").expect("zh-hant registered");

#[test]
fn hebrew_locale_file_flow_and_plural_categories() {
    let he = Lang::from_code("he").expect("Hebrew is registered");
    assert_eq!(he.name(), "עברית");
    assert!(!he.complete_menus(), "the first catalog is intentionally partial");
    for tag in ["he", "he-IL", "he_IL.UTF-8", "HE-il", "he-Hebr-IL"] {
        assert_eq!(lang_from_tag(tag), Some(he), "{tag}");
    }
    assert_eq!(Lang::from_pref("he"), he);
    for (english, expected) in [
        ("File", "קובץ"),
        ("Open…", "פתיחה…"),
        ("Save", "שמירה"),
        ("Save As…", "שמירה בשם…"),
        ("Close", "סגירה"),
        ("Edit", "עריכה"),
        ("Undo", "ביטול פעולה"),
        ("Redo", "ביצוע חוזר"),
        ("Export As SVG…", "ייצוא כ־SVG…"),
    ] {
        assert_eq!(tr(he, english), expected);
    }
    assert_eq!(tr(he, "untranslated VectorCraft sentinel"), "untranslated VectorCraft sentinel");
    assert_eq!([0, 1, 2, 3, 10, 20, 21].map(he.0.plural), [2, 0, 1, 2, 2, 2, 2]);
    for (n, expected) in [(0, "0 קבצים"), (1, "1 קובץ"), (2, "2 קבצים"), (21, "21 קבצים")] {
        assert_eq!(trn(he, n, "{n} file", "{n} files"), expected);
    }
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(rows.len() >= 80, "expected a useful initial file/edit catalog");
}

#[test]
fn hebrew_top_level_menus_are_source_linked_and_translated() {
    let he = Lang::from_code("he").expect("Hebrew registered");
    let titles: Vec<_> = crate::menus::menu_tree().into_iter().map(|(title, _)| title).filter(|title| *title != "VectorCraft").collect();
    assert_eq!(titles.len(), 9, "native source menu tree changed");
    for english in titles {
        assert_ne!(tr(he, english), english, "untranslated top-level menu: {english}");
        assert_eq!(tr(Lang::EN, english), english, "English menus must remain unchanged");
    }
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(rows.len() >= 208, "the seven additional menu titles must be registered");
}

#[test]
fn hebrew_new_document_dialog_controls_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let source = include_str!("../dialogs/new_document.rs");
    for english in [
        "RECENT",
        "SAVED",
        "BLANK DOCUMENT PRESETS",
        "Presets you save with the Save Preset button appear here.",
        "Documents you create appear here.",
        "PRESET DETAILS",
        "Document Name",
        "Save Preset",
        "Width",
        "Units",
        "Height",
        "Orientation",
        "Artboards",
        "Bleed",
        "Background Contents",
        "Create",
        "Save Document Preset",
        "Preset Name",
        "Cancel",
        "More Settings",
        "Number of Artboards:",
        "Change to Right-to-Left Layout",
        "Spacing:",
        "Columns:",
        "Done",
        "Portrait",
        "Landscape",
        "Advanced Options",
        "Color Mode",
        "Raster Effects",
        "Preview Mode",
    ] {
        assert!(source.contains(&format!(r#"tl!("{english}")"#)), "source key missing: {english}");
        assert_ne!(tr(he, english), english, "new document control untranslated: {english}");
    }
    assert_eq!(tr(he, "Save Preset"), "שמירת הגדרה קבועה");
    assert_eq!(tr(he, "Number of Artboards:"), "מספר משטחי יצירה:");
    assert_eq!(tr(he, "Change to Right-to-Left Layout"), "מעבר לפריסה מימין לשמאל");
    assert_eq!(tr(he, "File"), "קובץ", "existing File menu translation preserved");
    let (_, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn hebrew_save_options_dialog_controls_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let source = include_str!("../dialogs/save_options.rs");
    for english in [
        "Save a Copy",
        "Save as Template",
        "Save As",
        "Legacy .ai Options",
        "{format} Options",
        "Save each artboard to a separate file",
        "Include Linked Files",
        "Embed ICC Profiles",
        "Create PDF-Compatible File",
        "Use Compression",
        "Include Editing Data for Other Apps",
        "All",
        "Range:",
        "File Name:",
        "Format:",
        "A legacy .ai file holds one artboard: each artboard you check is saved as a separate file.",
        "Select All",
        "Clear",
        "{count} of {total} artboards",
        "Check an artboard to save.",
        "Version:",
        "PDF-compatible",
        "Legacy (version 7)",
        "Transparent Images:",
        "Flatten",
        "Omit",
        "Hidden Objects:",
        "Hidden Layer",
        "Make Visible",
        "Text:",
        "Editable",
        "Outlines",
        "VectorCraft {release} and later (version {n})",
    ] {
        assert!(source.contains(&format!(r#"tl!("{english}")"#)), "source key missing: {english}");
        assert_ne!(tr(he, english), english, "save dialog label untranslated: {english}");
    }
    assert_eq!(tr(he, "Save As"), "שמירה בשם");
    assert_eq!(tr(he, "File Name:"), "שם הקובץ:");
    assert_eq!(tr(he, "Include Linked Files"), "הכללת קבצים מקושרים");
    assert!(tr(he, "{format} Options").contains("{format}"));
    let artboards = tr(he, "{count} of {total} artboards");
    assert!(artboards.contains("{count}") && artboards.contains("{total}"));
    let release = tr(he, "VectorCraft {release} and later (version {n})");
    assert!(release.contains("{release}") && release.contains("{n}"));
    let (_, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
}

/// Native Save PDF and PDF-security controls must be readable in Hebrew, while
/// preserving their original English source identifiers and the fallback path.
#[test]
fn hebrew_save_pdf_options_and_security_controls_are_localized() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let source = include_str!("../dialogs/save_pdf.rs");
    for english in [
        "Save PDF",
        "New PDF Preset",
        "Edit PDF Preset",
        "Preset name:",
        "Standard:",
        "Compatibility:",
        "Preset:",
        "Save Preset…",
        "Save these settings as a preset",
        "Save as preset:",
        "Options",
        "Preserve editing capabilities",
        "Embed page thumbnails",
        "Optimize for fast web view",
        "View PDF after saving",
        "Create PDF layers from layers and sublayers",
        "Include non-printing layers",
        "Description",
        "Compression:",
        "Quality:",
        "Color images",
        "Grayscale images",
        "Monochrome images",
        "Compress text and line art",
        "Document open password",
        "Require a password to open the document",
        "Open password:",
        "Permissions",
        "Restrict printing, editing and other tasks",
        "Permissions password:",
        "Printing allowed:",
        "Changes allowed:",
        "Enable copying of text, images and other content",
        "Enable text access for screen readers",
        "Files of a PDF standard can't be password-protected.",
    ] {
        assert!(source.contains(&format!(r#"tl!("{english}")"#)), "source key missing: {english}");
        assert_ne!(tr(he, english), english, "Save PDF control untranslated: {english}");
    }
    assert_eq!(tr(he, "Save PDF"), "שמירת PDF");
    assert_eq!(tr(he, "Permissions"), "הרשאות");
    assert_eq!(tr(he, "Document open password"), "סיסמה לפתיחת המסמך");
    assert_eq!(tr(Lang::EN, "Save PDF"), "Save PDF");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(rows.len() >= 175, "new PDF rows absent");
}

#[test]
fn hindi_resolves_locales_and_uses_zero_one_plurals() {
    let hi = Lang::from_code("hi").expect("hi registered");
    for tag in ["hi", "hi-IN", "hi_IN.UTF-8", "HI-in", "hi-Deva-IN"] {
        assert_eq!(lang_from_tag(tag), Some(hi), "{tag}");
    }
    assert_eq!(hi.name(), "हिन्दी");
    assert_eq!([0, 1, 2, 21, u64::MAX].map(hi.0.plural), [0, 0, 1, 1, 1]);
    assert_eq!(tr(hi, "Save"), "सेव");
    assert_eq!(tr(hi, "Save As…"), "सेव ऐज़…");
    assert_eq!(tr(hi, "Path"), "पाथ");
    assert_eq!(tr(hi, "Selection"), "सिलेक्शन");
    assert_eq!(tr(hi, "Properties"), "प्रॉपर्टीज़");
    assert_eq!(tr(hi, "Options"), "ऑप्शन्स");
    assert_eq!(tr(hi, "Open"), "ओपन");
    let (entries, errors) = parse_entries(hi.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    for (ctx, src, translation) in entries.iter().filter(|(ctx, _, _)| ctx == "@plural" || ctx == "@msgplural") {
        let (one, other) = src.split_once('|').expect("one|other source");
        let forms: Vec<_> = translation.split('|').collect();
        assert_eq!(forms.len(), 2, "{ctx}: {src}");
        if ctx == "@plural" {
            for (n, index) in [(0, 0), (1, 0), (2, 1), (21, 1)] {
                assert_eq!(trn(hi, n, one, other), forms[index].replace("{n}", &n.to_string()));
            }
        }
    }
}

/// Locale tags name their language by region or script (in either case, with an encoding or
/// modifier), and a tag that names no language resolves to none.
#[test]
fn tags_map_to_languages() {
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("qaa-QM"), None, "the private-use tag names no language");
    // German: Germany, Austria, Switzerland and the rest share the one catalog.
    for tag in ["de", "de_DE.UTF-8", "de-AT", "de_CH", "de-LU", "de_DE.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(de()), "{tag}");
    }
    assert_eq!(lang_from_tag("ja_JP.UTF-8"), Lang::from_code("ja"));
    assert_eq!(lang_from_tag("ja"), Lang::from_code("ja"));
    // Spanish: every region (and Latin America as a whole) resolves to the one catalog.
    for tag in ["es", "es_ES.UTF-8", "es-MX", "es_AR", "es-419", "es-US"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("es"), "{tag}");
    }
    // French: France, Belgium, Canada, Switzerland and the rest share the one catalog.
    for tag in ["fr", "fr_FR.UTF-8", "fr-BE", "fr_CA", "fr-CH", "fr-LU"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("fr"), "{tag}");
    }
    other_tags_map_to_languages();
    // Traditional Chinese: by region, by script, and with a region after the script.
    assert_eq!(lang_from_tag("zh_TW.UTF-8"), Some(ZH()));
    assert_eq!(lang_from_tag("zh-TW"), Some(ZH()));
    assert_eq!(lang_from_tag("zh-HK"), Some(ZH()));
    assert_eq!(lang_from_tag("zh_MO"), Some(ZH()));
    assert_eq!(lang_from_tag("zh-Hant"), Some(ZH()));
    assert_eq!(lang_from_tag("zh-Hant-TW"), Some(ZH()));
    assert_eq!(lang_from_tag("zh-Hant-HK"), Some(ZH()));
    assert_eq!(lang_from_tag("zh_TW.UTF-8@radical"), Some(ZH()));
    // Simplified Chinese locales never pick up the Traditional catalog (they resolve to a
    // `zh-hans` catalog once one is registered, and to English until then).
    for tag in ["zh-CN", "zh_CN.UTF-8", "zh_SG", "zh-Hans", "zh-Hans-CN", "zh"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("zh-hans"), "{tag}");
    }
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
}

/// The Bosnian, Georgian, Ukrainian, Hungarian, Korean, Vietnamese, Swedish and Catalan tags of
/// [`tags_map_to_languages`].
fn other_tags_map_to_languages() {
    let bs = Lang::from_code("bs").expect("bs registered");
    for tag in ["bs", "bs-BA", "bs_BA.UTF-8", "BS-ba", "bs-Latn-BA"] {
        assert_eq!(lang_from_tag(tag), Some(bs), "{tag}");
    }
    for tag in ["ka", "ka-GE", "ka_GE.UTF-8", "KA-ge"] {
        assert_eq!(lang_from_tag(tag), Some(ka()), "{tag}");
    }
    for tag in ["uk", "uk-UA", "uk_UA.UTF-8", "UK-ua", "uk_UA.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(uk()), "{tag}");
    }
    for tag in ["hu", "hu-HU", "hu_HU.UTF-8", "HU-hu"] {
        assert_eq!(lang_from_tag(tag), Some(hu()), "{tag}");
    }
    for tag in ["ko", "ko-KR", "ko_KR.UTF-8", "KO-kr"] {
        assert_eq!(lang_from_tag(tag), Some(ko()), "{tag}");
    }
    for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vn"] {
        assert_eq!(lang_from_tag(tag), Some(vi()), "{tag}");
    }
    for tag in ["sv", "sv-SE", "sv-FI", "sv_SE.UTF-8", "SV-se"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("sv"), "{tag}");
    }
    // Catalan: Catalonia, Andorra, Northern Catalonia, the Valencian Country and the rest share
    // the one catalog.
    for tag in ["ca", "ca-ES", "ca_AD.UTF-8", "CA-es", "ca-FR", "ca-IT", "ca-ES-VALENCIA"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("ca"), "{tag}");
    }
}

/// Italian, Brazilian Portuguese, Finnish, Turkish, Catalan and Polish resolve from their locale
/// tags and preferences, and each language names itself in its own language.
#[test]
fn languages_resolve_from_their_tags_and_name_themselves() {
    let lang = |code: &str| Lang::from_code(code).expect("registered");
    for (code, tags) in [
        ("it", &["it", "it_IT.UTF-8", "it-CH", "it_SM"][..]),
        ("pt-br", &["pt-br", "pt_BR.UTF-8", "pt-BR", "pt_BR"]),
        ("fi", &["fi", "fi-FI", "fi_FI.UTF-8", "FI-fi"]),
        ("tr", &["tr", "tr-TR", "tr_TR.UTF-8", "TR-tr", "tr-CY"]),
        ("ca", &["ca", "ca-ES", "ca_AD.UTF-8", "CA-es", "ca-FR", "ca-IT", "ca-ES-VALENCIA"]),
        ("pl", &["pl", "pl-PL", "pl_PL.UTF-8", "PL-pl"]),
    ] {
        for tag in tags {
            assert_eq!(lang_from_tag(tag), Some(lang(code)), "{tag}");
        }
    }
    // Filipino (`fil`) and Swedish in Finland (`sv-FI`) don't pick Finnish.
    assert_ne!(lang_from_tag("fil"), Some(lang("fi")));
    assert_ne!(lang_from_tag("sv-FI"), Some(lang("fi")));
    for (pref, code) in [
        ("vi", "vi"),
        ("VI", "vi"),
        ("fi", "fi"),
        ("FI", "fi"),
        ("tr", "tr"),
        ("TR", "tr"),
        ("ca", "ca"),
        ("CA", "ca"),
        ("pt-br", "pt-br"),
        ("pt-BR", "pt-br"),
    ] {
        assert_eq!(Lang::from_pref(pref), lang(code), "{pref}");
    }
    for (code, name) in [("vi", "Tiếng Việt"), ("fi", "Suomi"), ("tr", "Türkçe"), ("ca", "Català"), ("pt-br", "Português (Brasil)")] {
        assert_eq!(lang(code).name(), name);
    }
}

/// The Catalan catalog registers under `ca`, claims complete menus and messages, and passes
/// locale resolution, vocabulary and count formatting checks.
#[test]
fn catalan_reads_as_catalan() {
    let ca = Lang::from_code("ca").expect("ca registered");
    assert_eq!(ca.name(), "Català");
    assert!(ca.complete_menus());
    assert_eq!(Lang::from_pref("ca"), ca);
    assert_eq!(Lang::from_pref("CA"), ca);
    for (en, want) in [
        ("Artboard Tool", "Eina de tauler"),
        ("Swatches", "Mostres"),
        ("Stroke", "Traç"),
        ("Fill", "Farciment"),
        ("Direct Selection Tool", "Eina de selecció directa"),
        ("Save As…", "Desa com a…"),
        ("Undo", "Desfés"),
        ("Close", "Tanca"),
    ] {
        assert_eq!(tr(ca, en), want);
    }
    for n in [0, 1, 3, 21] {
        let noun = if n == 1 { "capa" } else { "capes" };
        assert_eq!(trn(ca, n, "{n} Layer", "{n} Layers"), format!("{n} {noun}"));
    }
    let nested = "command `object.group` is not available right now: nothing selected";
    assert_eq!(message(ca, nested), "l’ordre `object.group` no està disponible ara mateix: no hi ha res seleccionat");
}

#[test]
fn font_height_reference_labels_are_in_complete_catalogs() {
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in
            ["Show Font Height Options", "Hide Font Height Options", "Set Font Height Reference", "Em Box", "Cap Height", "x-Height", "ICF Box"]
        {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// Preference choices and the marquee's E hint share the same translated labels.
#[test]
fn marquee_selection_labels_are_in_complete_catalogs() {
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in ["Marquee Selection", "Touching", "Fully Enclosed", "Drag Direction"] {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// The Character panel's Anti-aliasing Method label and its choices, with the Sharp method's
/// own `antiAlias` row apart from the plain "Sharp" corner row.
#[test]
fn anti_aliasing_method_labels_are_in_complete_catalogs() {
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        let cat = lang.catalog();
        for label in ["Anti-aliasing Method", "Crisp", "Strong"] {
            assert!(cat.plain(label).is_some(), "{} is missing {label}", lang.code);
        }
        assert!(cat.contextual("antiAlias", "Sharp").is_some(), "{} is missing the antiAlias Sharp row", lang.code);
    }
}

/// Each language counts with its own plural rule: the form a count picks (`one`, then `few`, then
/// `other`/`many`).
#[test]
fn each_language_counts_with_its_plural_rule() {
    const COUNTS: [u64; 8] = [0, 1, 2, 3, 5, 11, 21, 22];
    for (codes, forms) in [
        (&["de", "es", "it", "pt-br", "fi", "ca"][..], [1, 0, 1, 1, 1, 1, 1, 1]),
        (&["fr"], [0, 0, 1, 1, 1, 1, 1, 1]),
        (&["ja", "ka", "hu", "ko", "vi", "tr"], [0; 8]),
        (&["ru", "uk", "bs"], [2, 0, 1, 1, 2, 2, 0, 1]),
        (&["cs"], [2, 0, 1, 1, 2, 2, 2, 2]),
        (&["pl"], [2, 0, 1, 1, 2, 2, 2, 1]),
    ] {
        for code in codes {
            let rule = Lang::from_code(code).expect("registered").0.plural;
            assert_eq!(COUNTS.map(rule), forms, "{code}");
        }
    }
}

#[test]
fn candidates_walk_from_specific_to_general() {
    assert_eq!(candidates("pt_BR.UTF-8"), ["pt-br", "pt"]);
    assert_eq!(candidates("zh_TW"), ["zh-tw", "zh-hant", "zh"]);
    assert_eq!(candidates("zh-CN"), ["zh-cn", "zh-hans", "zh"]);
    assert_eq!(candidates("zh-Hant-HK"), ["zh-hant-hk", "zh-hant", "zh"]);
}

#[test]
fn os_language_lists_are_parsed() {
    assert_eq!(first_supported("(\n    \"zh-Hant-TW\",\n    \"en-US\"\n)\n"), Some(ZH()));
    assert_eq!(first_supported("(\n    \"qaa-QM\",\n    \"en-US\"\n)\n"), Some(Lang::EN), "the private-use tag falls through to the next entry");
    assert_eq!(first_supported("(\n    \"de-DE\",\n    \"en-US\"\n)\n"), Some(de()));
    assert_eq!(first_supported("(\n    \"fr-FR\",\n    \"en-US\"\n)\n"), Lang::from_code("fr"));
    assert_eq!(first_supported("("), None);
    // Windows: `reg query HKCU\Control Panel\International /v LocaleName`.
    let reg = "\r\nHKEY_CURRENT_USER\\Control Panel\\International\r\n    LocaleName    REG_SZ    zh-TW\r\n\r\n";
    assert_eq!(registry_locale(reg).as_deref(), Some("zh-TW"));
    assert_eq!(registry_locale(reg).as_deref().and_then(lang_from_tag), Some(ZH()));
    assert_eq!(registry_locale("ERROR: The system was unable to find the specified registry key or value."), None);
}

#[test]
fn preferences_resolve_with_fallback() {
    assert_eq!(Lang::from_pref("zh-Hans"), Lang::from_code("zh-hans").unwrap());
    assert_eq!(Lang::from_pref("zh-hant"), ZH());
    assert_eq!(Lang::from_pref("ZH-Hant"), ZH());
    assert_eq!(Lang::from_pref("en"), Lang::EN);
    assert_eq!(Lang::from_pref("UK"), uk());
    // `auto` and unknown codes follow the system (English under test).
    assert_eq!(Lang::from_pref("auto"), Lang::EN);
    assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
}

#[test]
fn current_language_is_a_frame_setting() {
    set_current(ZH());
    assert_eq!(current(), ZH());
    assert_eq!(t("Layers"), "圖層");
    set_current(Lang::EN);
    assert_eq!(current(), Lang::EN);
    assert_eq!(t("Layers"), "Layers");
}

/// Unit tests run on parallel threads, and each one draws in its own language: a language set on
/// another thread (by a test running beside this one) leaves this thread's text in English.
#[test]
fn a_language_set_by_another_test_leaves_this_one_in_english() {
    std::thread::spawn(|| set_current(ZH())).join().unwrap();
    assert_eq!(current(), Lang::EN);
    assert_eq!(t("Layers"), "Layers");
}

#[test]
fn lookups_fall_back_to_english() {
    assert_eq!(tr(Lang::EN, "Layers"), "Layers");
    let layers = ZH().catalog().plain("Layers");
    assert_eq!(tr(ZH(), "Layers"), layers.unwrap_or("Layers"));
    assert_eq!(tr(ZH(), "no such label"), "no such label");
    assert_eq!(tr_id(ZH(), "no.such.id", "Layers"), tr(ZH(), "Layers"));
    assert_eq!(tr_ctx(ZH(), "no such context", "Layers"), tr(ZH(), "Layers"));
    assert_eq!(has(ZH(), "Layers"), layers.is_some());
    assert!(!has(Lang::EN, "Layers"));
}

#[test]
fn catalog_kinds_are_parsed_and_looked_up() {
    let c = Catalog::parse("# c\n\tHello\t你好\n@id\tfile.save\t儲存\nmenu\tWindows\t視窗們\n@plural\t{n} file|{n} files\t{n} 個檔案\n\n");
    assert_eq!(c.plain("Hello"), Some("你好"));
    assert_eq!(c.id("file.save"), Some("儲存"));
    assert_eq!(c.contextual("menu", "Windows"), Some("視窗們"));
    assert_eq!(c.contextual("other", "Windows"), None);
    assert_eq!(c.plural("{n} file", "{n} files", 0), Some("{n} 個檔案"));
    assert_eq!(c.plural("{n} file", "{n} files", 5), Some("{n} 個檔案"), "an index past the forms clamps");
}

#[test]
fn malformed_lines_are_reported_not_fatal() {
    let (entries, errors) = parse_entries("\tok\t好\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n");
    assert_eq!(entries.len(), 1);
    assert_eq!(errors.len(), 4, "{errors:?}");
    assert_eq!(parse_entries("\ta\\tb\tx\\ny\\\\z\n").0[0], (String::new(), "a\tb".into(), "x\ny\\z".into()));
}

#[test]
fn plurals_and_placeholders() {
    assert_eq!(trn(Lang::EN, 1, "{n} item", "{n} items"), "1 item");
    assert_eq!(trn(Lang::EN, 0, "{n} item", "{n} items"), "0 items");
    assert_eq!(trn(Lang::EN, 7, "{n} item", "{n} items"), "7 items");
    // A catalog row's form takes the count; without a row the English form does.
    let zh_items = ZH().catalog().plural("{n} item", "{n} items", 0).unwrap_or("{n} items");
    assert_eq!(trn(ZH(), 7, "{n} item", "{n} items"), zh_items.replace("{n}", "7"));
    assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y"), ("c", "z")]), "y before x");
    assert_eq!(fmt("{missing}", &[]), "{missing}");
    assert_eq!(placeholders("a {x} b {y} {"), ["x", "y"]);
}

/// Every bundled catalog is well-formed and consistent with its sources.
#[test]
fn bundled_catalogs_are_consistent() {
    for l in &LANGUAGES {
        assert!(l.code == l.code.to_ascii_lowercase() && !l.name.is_empty(), "{}", l.code);
        let (entries, errors) = parse_entries(l.source);
        assert!(errors.is_empty(), "{}: {errors:?}", l.code);
        let mut seen = std::collections::HashSet::new();
        for (ctx, src, tr) in &entries {
            assert!(seen.insert((ctx.clone(), src.clone())), "{}: duplicate {ctx:?} {src:?}", l.code);
            if ctx == "@plural" || ctx == "@msgplural" {
                let one_other: Vec<&str> = src.split('|').collect();
                assert_eq!(one_other.len(), 2, "{}: plural source must be `one|other`: {src:?}", l.code);
                for form in tr.split('|') {
                    let mut want = placeholders(one_other[1]);
                    let mut got = placeholders(form);
                    want.sort_unstable();
                    got.sort_unstable();
                    assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
                }
                continue;
            }
            let mut want = placeholders(src);
            let mut got = placeholders(tr);
            want.sort_unstable();
            got.sort_unstable();
            assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
            if ctx.is_empty() {
                assert_eq!(src.ends_with('…'), tr.ends_with('…'), "{}: ellipsis mismatch: {src:?}", l.code);
            }
            if ctx == "@id" {
                // A command id, or an effect id: Effect menu items share `effect.apply` /
                // `effect.dialog`, so the effect's own id is what tells them apart.
                let known = crate::menus::UI_COMMANDS.iter().any(|c| c.0 == src)
                    || vectorcraft_engine::cmd::command_specs().iter().any(|c| c.id == src)
                    || vectorcraft_effects::effect_catalog().iter().any(|e| e.id == src);
                assert!(known, "{}: unknown command id {src:?}", l.code);
            }
        }
    }
}

/// A catalog that doesn't claim complete menus may still only hold strings the UI shows (menu
/// labels, command labels, `tl!` literals), so labels from other apps don't creep in.
#[test]
fn partial_catalogs_only_translate_strings_the_ui_shows() {
    let known: std::collections::BTreeSet<String> = crate::menus::menu_strings().into_iter().chain(tl_literals()).collect();
    for l in LANGUAGES.iter().filter(|l| !l.complete_menus && !l.source.is_empty()) {
        let (entries, _) = parse_entries(l.source);
        let unknown: Vec<_> = entries.iter().filter(|(ctx, src, _)| ctx.is_empty() && !known.contains(src)).map(|(_, src, _)| src).collect();
        assert!(unknown.is_empty(), "{}: not UI strings: {unknown:?}", l.code);
    }
}

/// The engine learns the language the UI is drawn in each frame: new type takes the Japanese
/// defaults while it is Japanese (#432). A Japanese frame draws in Japanese on this test's thread
/// only: the tests running beside it on other threads still draw in English.
#[test]
fn the_engine_follows_the_interface_language() {
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let frame = |app: &mut crate::VectorcraftApp| {
        ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).textures_delta.clear();
    };
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("en"), "tests resolve `auto` to English");
    assert!(!app.session.japanese_interface());
    app.session.prefs.interface_language = "ja".into();
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("ja"));
    assert!(app.session.japanese_interface());
    assert_eq!(current().code(), "ja");
    // A test running beside this one on another thread still draws in English.
    assert_eq!(std::thread::spawn(|| (current(), t("Layers"))).join().unwrap(), (Lang::EN, "Layers"));
    // With the preference back on `auto`, the next frame hands the engine `en` again.
    app.session.prefs.interface_language = "auto".into();
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("en"));
    assert!(!app.session.japanese_interface());
    assert_eq!(current(), Lang::EN);
}

/// `app.language` sets the `interfaceLanguage` preference, which is what persists; the checked
/// item follows the preference, and a bad code is an error, not a change.
#[test]
fn language_command_sets_the_preference() {
    use crate::menus::{checked, run_ui_command};
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(true));
    run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "ja"})).unwrap().unwrap();
    assert_eq!(app.session.prefs.interface_language, "ja");
    assert_eq!(app.ui_language(), Lang::from_code("ja").unwrap());
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "ja"})), Some(true));
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(false));
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "zh-hant"})), Some(false));
    let err = run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "xx"})).unwrap().unwrap_err();
    assert!(err.contains("auto") && err.contains("zh-hant"), "{err}");
    assert_eq!(app.session.prefs.interface_language, "ja");
    run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "auto"})).unwrap().unwrap();
    assert_eq!(app.session.prefs.interface_language, "auto");
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(true));
    // Every registered language is in the Mac App menu's Language, by its own name (the in-window
    // bar has no Language menu).
    let layout = crate::native_menu::mac_layout(&app, &crate::native_menu::from_tree(&app, &crate::menus::menu_tree()), Lang::EN);
    let app_menu = layout.bar.menus.into_iter().find(|m| m.title == "VectorCraft").expect("App menu");
    fn items(nodes: &[crate::native_menu::Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                crate::native_menu::Node::Item(it) if it.command == Some("app.language") => out.push(it.label.clone()),
                crate::native_menu::Node::Submenu { children, .. } => items(children, out),
                _ => {}
            }
        }
    }
    let mut labels = vec![];
    items(&app_menu.children, &mut labels);
    for l in Lang::all() {
        assert!(labels.iter().any(|x| x == l.name()), "{} missing from {labels:?}", l.code());
    }
}

/// Chinese and Japanese characters (CJK punctuation, kana, ideographs, full-width forms).
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// Georgian letters (Mkhedruli, Mtavruli, Asomtavruli): only craft-fonts' Noto Sans Georgian has them.
#[cfg(feature = "interface-fonts")]
fn is_georgian(c: char) -> bool {
    matches!(c as u32, 0x10A0..=0x10FF | 0x1C90..=0x1CBF | 0x2D00..=0x2D2F)
}

#[cfg(feature = "interface-fonts")]
fn is_devanagari(c: char) -> bool {
    matches!(c as u32, 0x0900..=0x097F | 0x1CD0..=0x1CFF | 0xA8E0..=0xA8FF)
}

/// The UI fonts draw every character the catalogs use, in every family the UI draws text with
/// (the web build has no system fonts to fall back to). Chinese and Japanese characters come from
/// the craft-fonts build input, which is optional: without it they are left to the installed
/// fonts, and with it the Japanese catalog is checked against BIZ UDPGothic (the Traditional
/// Chinese one waits for a Traditional Chinese UI font in craft-fonts). Georgian letters are the
/// same: checked against Noto Sans Georgian when craft-fonts has it. It needs the interface's own
/// faces (`interface-fonts`): egui's built-in fonts lack some of the catalogs' symbols.
#[cfg(feature = "interface-fonts")]
#[test]
fn bundled_fonts_cover_the_catalogs() {
    let japanese_ui_font = vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.is_japanese());
    let georgian_ui_font = vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.is_georgian());
    let devanagari_ui_font = vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.scripts.contains(&"Deva"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    let mut missing = std::collections::BTreeMap::new();
    ctx.fonts_mut(|fonts| {
        let families = [
            egui::FontFamily::Proportional,
            egui::FontFamily::Name(crate::theme::FONT_UI.into()),
            egui::FontFamily::Name(crate::theme::FONT_UI_SEMIBOLD.into()),
            egui::FontFamily::Name(crate::theme::FONT_MONO.into()),
        ];
        for l in LANGUAGES.iter().filter(|l| !l.source.is_empty()) {
            let (entries, _) = parse_entries(l.source);
            let cjk_checked = japanese_ui_font && l.code == "ja";
            let chars: std::collections::BTreeSet<char> = entries
                .iter()
                .flat_map(|(_, _, tr)| tr.chars())
                .filter(|c| {
                    !c.is_whitespace()
                        && (cjk_checked || !is_cjk(*c))
                        && (georgian_ui_font || !is_georgian(*c))
                        && (devanagari_ui_font || !is_devanagari(*c))
                })
                .collect();
            for family in &families {
                let font = egui::FontId::new(13.0, family.clone());
                let gone: String = chars.iter().filter(|c| !fonts.has_glyph(&font, **c)).collect();
                if !gone.is_empty() {
                    missing.insert(format!("{} in {family:?}", l.code), gone);
                }
            }
        }
    });
    assert!(missing.is_empty(), "characters without a glyph: {missing:#?}");
}

/// The Traditional Chinese catalog holds no Simplified-only characters (a translation written
/// in the wrong script would read as a typo to every user).
#[test]
fn zh_hant_has_no_simplified_characters() {
    const SIMPLIFIED_ONLY: &str = "们这为来时间个说对会发现过还没动开关图层选设项编辑显视帮删预览导节线连择报错误处据库经体应该样点击确认标记录输进转换调约维护启闭锁义类网络页颜绘画宽长边缘缩镜复贴渐滤笔钢区饱参数变号单双属组齐径锚轮阴阳实际让从产东车门问闪并坚测试运术压缩叠饰够亚质纸张档缺省认识";
    let (entries, _) = parse_entries(ZH().0.source);
    let mut bad = Vec::new();
    for (ctx, src, tr) in entries {
        if let Some(c) = tr.chars().find(|c| SIMPLIFIED_ONLY.contains(*c)) {
            bad.push(format!("{ctx} {src:?} → {tr:?} has {c:?}"));
        }
    }
    assert!(bad.is_empty(), "simplified characters in zh-hant.tsv:\n{}", bad.join("\n"));
}

/// The Simplified Chinese catalog is written in Simplified characters and in the vocabulary of the
/// mainland: Taiwan's terms (and a converted Traditional row) would read as foreign to its users.
#[test]
fn zh_hans_is_simplified_and_mainland() {
    const TRADITIONAL_ONLY: &str = "們這為來時間個說對會發現過還沒動開關圖層選設項編輯顯視幫刪預覽導節線連擇報錯誤處據庫經體應該樣點擊確認標記錄輸進轉換調約維護啟閉鎖義類網絡頁顏繪畫寬長邊緣縮鏡複貼漸濾筆鋼區飽參數變號單雙屬組齊徑錨輪陰陽實際讓從產東車門問閃並堅測試運術壓縮疊飾夠亞質紙張檔認識";
    const TAIWAN_TERMS: &[&str] = &[
        "档案",
        "资料夹",
        "快速键",
        "按一下",
        "按两下",
        "描述档",
        "品质",
        "贴上",
        "功能表",
        "物件",
        "工作区域",
        "遮色片",
        "影像",
        "列印",
        "印表机",
        "字型",
        "视窗",
        "滑鼠",
        "游标",
        "程式",
        "软体",
        "偏好设定",
        "色票",
        "渐层",
        "笔刷",
        "尺标",
        "字元",
        "汇出",
        "汇入",
        "连结",
        "解析度",
        "点阵图",
        "介面",
        "自订",
        "对话方块",
        "预设值",
        "储存",
        "套用",
        "「",
        "」",
    ];
    let lang = Lang::from_code("zh-hans").expect("zh-hans registered");
    let (entries, _) = parse_entries(lang.0.source);
    let mut bad = Vec::new();
    for (ctx, src, tr) in entries {
        if let Some(c) = tr.chars().find(|c| TRADITIONAL_ONLY.contains(*c)) {
            bad.push(format!("{ctx} {src:?} → {tr:?} has {c:?}"));
        }
        if let Some(w) = TAIWAN_TERMS.iter().find(|w| tr.contains(**w)) {
            bad.push(format!("{ctx} {src:?} → {tr:?} has the Taiwan term {w:?}"));
        }
    }
    assert!(bad.is_empty(), "in zh-hans.tsv:\n{}", bad.join("\n"));
}

/// Whether `text` holds `term`. In a script that separates words with spaces the term must stand as
/// whole words, so "liderlik" does not hold "lider"; in Chinese, Japanese, Thai, Lao, Khmer and
/// Myanmar text any occurrence counts.
fn contains_term(text: &str, term: &str) -> bool {
    let unspaced = |c: char| matches!(c as u32, 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF);
    if term.chars().any(unspaced) {
        return text.contains(term);
    }
    text.match_indices(term).any(|(at, m)| {
        !text[..at].chars().next_back().is_some_and(char::is_alphanumeric) && !text[at + m.len()..].chars().next().is_some_and(char::is_alphanumeric)
    })
}

#[test]
fn contains_term_matches_whole_words_where_words_are_spaced() {
    assert!(contains_term("tepeden tepeye lider", "lider"));
    assert!(contains_term("boyut/lider:", "lider"));
    assert!(contains_term("réinitialiser l’approche", "approche"));
    assert!(!contains_term("tepeden tepeye liderlik", "lider"));
    assert!(!contains_term("automatické proklady:", "proklad"));
    assert!(contains_term("kích thước: giãn chữ:", "giãn chữ"));
    assert!(!contains_term("kích thước chữ: giãn:", "giãn chữ"));
    assert!(contains_term("自動行送り:", "行送り"));
    assert!(contains_term("大小：字距微調：", "字距"));
}

/// Each catalog names leading with one term: every word of the Character panel's Leading field
/// (the text before " (", ignoring case) appears as a whole word (see [`contains_term`]) in the
/// Auto Leading, Size/Leading, Top-to-Top Leading and Bottom-to-Bottom Leading rows outside their
/// placeholders.
/// A catalog without the Leading row, or without one of the others, is skipped for that check.
#[test]
fn leading_term_agrees_within_each_catalog() {
    let mut bad = Vec::new();
    for l in LANGUAGES.iter().filter(|l| !l.source.is_empty()) {
        let (entries, _) = parse_entries(l.source);
        let row = |src: &str| entries.iter().find(|(ctx, s, _)| ctx.is_empty() && s == src).map(|(_, _, tr)| tr.to_lowercase());
        let Some(leading) = row("Leading").map(|tr| tr.split(" (").next().unwrap_or_default().trim().to_string()) else {
            continue;
        };
        for src in ["Auto Leading:", "Size/Leading", "Top-to-Top Leading", "Bottom-to-Bottom Leading"] {
            if let Some(text) = row(src) {
                let bare = placeholders(&text).iter().fold(text.clone(), |t, p| t.replace(&format!("{{{p}}}"), " "));
                for word in leading.split_whitespace().filter(|w| !contains_term(&bare, w)) {
                    bad.push(format!("{}: {src:?} → {text:?} lacks {word:?} from the Leading field {leading:?}", l.code));
                }
            }
        }
    }
    assert!(bad.is_empty(), "rows that name leading differently from the Leading field:\n{}", bad.join("\n"));
}

/// No translated word mixes Latin, Cyrillic and Greek letters, such as a Latin "A" typed in place
/// of a Cyrillic "А". A word written in one script, such as "PDF" or "X" in a Russian row, is
/// allowed.
#[test]
fn no_translated_word_mixes_latin_cyrillic_and_greek_letters() {
    let script = |c: char| match c as u32 {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Some("Latin"),
        0x370..=0x3FF | 0x1F00..=0x1FFF => Some("Greek"),
        0x400..=0x52F => Some("Cyrillic"),
        _ => None,
    };
    let mut bad = Vec::new();
    for l in LANGUAGES.iter().filter(|l| !l.source.is_empty()) {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in entries {
            for word in tr.split(|c: char| !c.is_alphabetic()) {
                let scripts: std::collections::BTreeSet<_> = word.chars().filter_map(script).collect();
                if scripts.len() > 1 {
                    bad.push(format!("{}: {ctx} {src:?} → {tr:?} has {word:?} ({scripts:?})", l.code));
                }
            }
        }
    }
    assert!(bad.is_empty(), "words that mix scripts:\n{}", bad.join("\n"));
}

/// Each catalog gives the Character panel's Kerning, Leading and Tracking fields three different
/// names, and every word of the Tracking and Kerning names appears in the Reset Tracking and
/// Kerning row (ignoring case). Each word is matched once, the longer name first, so a name that
/// contains the other name does not count for both. The Info panel's type line labels
/// `{tracking}` with the whole Tracking name: the text between the placeholder before it and
/// `{tracking}` holds the name (see [`contains_term`]) once any Kerning or Leading name that
/// contains it has been taken out.
/// A catalog without one of these rows is skipped for the checks that need it.
#[test]
fn character_panel_terms_agree_within_each_catalog() {
    let mut bad = Vec::new();
    for l in LANGUAGES.iter().filter(|l| !l.source.is_empty()) {
        let (entries, _) = parse_entries(l.source);
        let row = |src: &str| entries.iter().find(|(ctx, s, _)| ctx.is_empty() && s == src).map(|(_, _, tr)| tr.to_lowercase());
        let kerning = row("Kerning");
        let leading = row("Leading");
        let tracking = row("Tracking");
        for (a, b) in [(&kerning, &leading), (&kerning, &tracking), (&leading, &tracking)] {
            if let (Some(a), Some(b)) = (a, b)
                && a == b
            {
                bad.push(format!("{}: two Character panel fields are both {a:?}", l.code));
            }
        }
        let mut uses = |row_src: &str, terms: &[&Option<String>]| {
            let Some(text) = row(row_src) else { return };
            let mut bare = placeholders(&text).iter().fold(text.clone(), |t, p| t.replace(&format!("{{{p}}}"), " "));
            let mut terms: Vec<&String> = terms.iter().filter_map(|t| t.as_ref()).collect();
            terms.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
            for term in terms {
                for word in term.split_whitespace() {
                    if bare.contains(word) {
                        bare = bare.replacen(word, " ", 1);
                    } else {
                        bad.push(format!("{}: {row_src:?} → {text:?} lacks {word:?} from the field {term:?}", l.code));
                    }
                }
            }
        };
        uses("Reset Tracking and Kerning", &[&tracking, &kerning]);
        if let (Some(text), Some(tracking)) = (row("Size: {size}  Tracking: {tracking}"), &tracking) {
            let before = text.split("{tracking}").next().unwrap_or_default();
            let mut label = before.rsplit('}').next().unwrap_or_default().to_string();
            for other in [&kerning, &leading].into_iter().flatten().filter(|o| *o != tracking && o.contains(tracking.as_str())) {
                label = label.replace(other.as_str(), " ");
            }
            if !contains_term(&label, tracking) {
                bad.push(format!("{}: the Info row {text:?} labels {{tracking}} {label:?}, not the field {tracking:?}", l.code));
            }
        }
    }
    assert!(bad.is_empty(), "Character panel terms that disagree:\n{}", bad.join("\n"));
}

/// Every `tl!("…")` literal in this crate's sources (test modules aside).
pub fn tl_literals() -> std::collections::BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut literals = std::collections::BTreeSet::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && name != "lib.rs" && !name.starts_with("tests") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                // Code up to the first test item; test-only imports (`#[cfg(test)] use …;`) don't end it.
                let code = text
                    .match_indices("#[cfg(test)]")
                    .find(|&(at, m)| {
                        let item = text[at + m.len()..].trim_start();
                        let item = ["pub(crate) ", "pub(super) ", "pub "].iter().find_map(|p| item.strip_prefix(p)).unwrap_or(item);
                        !item.starts_with("use ")
                    })
                    .map_or(text.as_str(), |(at, _)| &text[..at]);
                let mut rest = code;
                while let Some(at) = rest.find("tl!(\"") {
                    rest = &rest[at + 5..];
                    let mut end = 0;
                    let bytes = rest.as_bytes();
                    while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
                        end += 1;
                    }
                    let lit = rest.get(..end).unwrap_or("").replace("\\\"", "\"");
                    if rest.get(end + 1..end + 2) == Some(")") {
                        literals.insert(lit);
                    }
                }
            }
        }
    }
    literals
}

/// Crates whose undo steps the History panel lists.
const UNDO_CRATES: &[&str] = &["engine", "tools", "ui-egui"];

/// Calls whose first argument names an undo step.
const UNDO_MARKERS: &[&str] = &[".edit(", "begin_interaction(", "Action::Begin(", "begin_named_undo_group(Some("];

/// The call whose first argument is the template of an undo step's name (`Session::edit_named`).
const NAMED_MARKER: &str = ".edit_named(";

/// A function in a source file, as [`undo_labels`] reads it.
struct SourceFn<'a> {
    file: usize,
    name: &'a str,
    /// Declared `pub` (`pub(crate)`, `pub(super)`), so other files call it too.
    public: bool,
    /// Position of its `label` parameter among a call's arguments (`self` not counted).
    label: Option<usize>,
    body: &'a str,
}

/// Every undo step name in [`UNDO_CRATES`] (test modules aside) that the sources write as a
/// literal: the first argument at each of [`UNDO_MARKERS`], the argument a function whose `label`
/// parameter reaches one of them takes at each of its calls, and the value of each `let label = …`
/// in a function that passes `label` on. A literal counts where it is the value (`"Move"`,
/// `if copy { "Copy" } else { "Move" }`, a match arm's result). A call of a function named
/// `…label` (`pf_label(op)`, `kind.label()`) gives the literals that function returns in the same
/// file. A name built with `format!` or taken from data (an effect's, a plug-in's) is not read.
pub fn undo_labels() -> std::collections::BTreeSet<String> {
    undo_scan().labels
}

/// What [`undo_scan`] finds in [`UNDO_CRATES`].
struct UndoScan {
    /// The step names written as literals ([`undo_labels`]).
    labels: std::collections::BTreeSet<String>,
    /// The templates step names are made from with `Session::edit_named` (its first argument).
    templates: std::collections::BTreeSet<String>,
    /// The format strings of the step names built with `format!` where [`undo_labels`] reads a
    /// name.
    formatted: std::collections::BTreeSet<String>,
}

/// The undo step names and templates in [`UNDO_CRATES`] (see [`undo_labels`]).
fn undo_scan() -> UndoScan {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = std::collections::BTreeSet::new();
    let mut templates = std::collections::BTreeSet::new();
    let mut formatted = std::collections::BTreeSet::new();
    for krate in UNDO_CRATES {
        let files = undo_sources(&crates.join(krate).join("src"));
        let fns: Vec<SourceFn> = files.iter().enumerate().flat_map(|(i, t)| source_fns(i, t)).collect();
        // Functions whose `label` parameter reaches a marker, directly or through one another.
        let mut forwards = vec![false; fns.len()];
        loop {
            let before = forwards.clone();
            for (i, f) in fns.iter().enumerate().filter(|(_, f)| f.label.is_some()) {
                let passed = |g: &SourceFn| call_args(f.body, g).iter().any(|a| bare(a) == "label");
                forwards[i] |= marker_args(f.body).iter().any(|a| bare(a) == "label") || fns.iter().zip(&before).any(|(g, &on)| on && passed(g));
            }
            if forwards == before {
                break;
            }
        }
        let forwarders: Vec<&SourceFn> = fns.iter().zip(&forwards).filter(|(_, on)| **on).map(|(f, _)| f).collect();
        // Each expression that gives a step's name, with the file it is in.
        let mut exprs: Vec<(usize, &str)> = vec![];
        for (i, t) in files.iter().enumerate() {
            exprs.extend(marker_args(t).into_iter().map(|a| (i, a)));
            for g in forwarders.iter().filter(|g| g.public || g.file == i) {
                exprs.extend(call_args(t, g).into_iter().map(|a| (i, a)));
            }
        }
        for f in &fns {
            let mut passed = marker_args(f.body);
            passed.extend(forwarders.iter().flat_map(|g| call_args(f.body, g)));
            if !passed.iter().any(|a| bare(a).starts_with("label")) {
                continue;
            }
            for (at, m) in f.body.match_indices("let label =") {
                let rest = &f.body[at + m.len()..];
                exprs.push((f.file, rest[..top_level(rest).1].trim()));
            }
        }
        for (file, e) in exprs {
            label_literals(e, file, &fns, &mut out, &mut formatted);
        }
        for t in &files {
            let named = t.match_indices(NAMED_MARKER).filter_map(|(at, _)| arguments(&t[at + NAMED_MARKER.len()..]).first().copied());
            templates.extend(named.filter_map(|a| a.strip_prefix('"').and_then(string_literal)));
        }
    }
    UndoScan { labels: out, templates, formatted }
}

/// The sources of a crate's `src` directory, test modules and files and comments aside.
fn undo_sources(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.starts_with("tests") && !name.ends_with("_tests.rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.push(without_comments(without_test_module(&text)));
            }
        }
    }
    out
}

/// `text` with each comment removed: a `//` comment, and a `/* … */` comment (nested ones
/// included), which becomes a space. A `//` or `/*` inside a string or character literal stays.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(text, i)).flatten() {
            out.push_str(&text[i..end]);
            i = end;
        } else if text[i..].starts_with("//") {
            i = text[i..].find('\n').map_or(text.len(), |n| i + n);
        } else if text[i..].starts_with("/*") {
            i = block_comment_end(text, i);
            out.push(' ');
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Where the `/* … */` comment starting at `text[at]` ends, past the comments nested in it; the
/// end of `text` when the comment is not closed.
fn block_comment_end(text: &str, at: usize) -> usize {
    let (mut depth, mut i) = (0usize, at);
    while let Some(c) = text[i..].chars().next() {
        if text[i..].starts_with("/*") {
            depth += 1;
            i += 2;
        } else if text[i..].starts_with("*/") {
            depth = depth.saturating_sub(1);
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += c.len_utf8();
        }
    }
    text.len()
}

/// Where the string or character literal starting at `text[at]` (a `"` or `'`) ends, raw strings
/// (`r#"…"#`) included; `None` for a lifetime's `'`.
fn quoted_end(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at + 1..];
    if text[at..].starts_with('\'') {
        let mut it = rest.char_indices();
        let (_, c) = it.next()?;
        if c == '\\' {
            it.next()?;
            return it.find(|(_, c)| *c == '\'').map(|(n, _)| at + 1 + n + 1);
        }
        return it.next().filter(|(_, c)| *c == '\'').map(|(n, _)| at + 1 + n + 1);
    }
    let hashes = text[..at].chars().rev().take_while(|c| *c == '#').count();
    if text[..at - hashes].ends_with('r') {
        let close = format!("\"{}", "#".repeat(hashes));
        return rest.find(&close).map(|n| at + 1 + n + close.len());
    }
    let mut it = rest.char_indices();
    while let Some((n, c)) = it.next() {
        match c {
            '\\' => {
                it.next();
            }
            '"' => return Some(at + 1 + n + 1),
            _ => {}
        }
    }
    None
}

/// In `s` (the text after an opening bracket), the offset of each top-level `,`, and where the
/// list ends: at the bracket that closes it, a top-level `;`, or the end of `s`.
fn top_level(s: &str) -> (Vec<usize>, usize) {
    let (mut commas, mut depth, mut i) = (vec![], 0usize, 0);
    while let Some(c) = s[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(s, i)).flatten() {
            i = end;
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return (commas, i),
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => return (commas, i),
            ',' if depth == 0 => commas.push(i),
            _ => {}
        }
        i += c.len_utf8();
    }
    (commas, s.len())
}

/// The length of the block at the start of `s` (which starts with `{`), braces included.
fn block_len(s: &str) -> usize {
    let mut at = 1;
    while at < s.len() {
        let end = at + top_level(&s[at..]).1;
        if !s[end..].starts_with(';') {
            return (end + 1).min(s.len());
        }
        at = end + 1;
    }
    s.len()
}

/// The arguments of the call whose `(` comes just before `s`, trimmed.
fn arguments(s: &str) -> Vec<&str> {
    let (commas, end) = top_level(s);
    let mut out = vec![];
    let mut start = 0;
    for at in commas.into_iter().chain([end]) {
        out.push(s[start..at].trim());
        start = at + 1;
    }
    out
}

/// The functions defined in a source file, with their bodies.
fn source_fns(file: usize, text: &str) -> Vec<SourceFn<'_>> {
    let mut out = vec![];
    for (at, _) in text.match_indices("fn ") {
        if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let after = &text[at + 3..];
        let name_len = after.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(after.len());
        let (name, mut rest) = after.split_at(name_len);
        if rest.starts_with('<') {
            let mut depth = 0;
            let close = rest
                .char_indices()
                .find(|&(n, c)| {
                    depth += match c {
                        '<' => 1,
                        '>' if !rest[..n].ends_with('-') => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .map(|(n, _)| n);
            rest = close.map_or("", |n| &rest[n + 1..]);
        }
        let Some(params) = rest.strip_prefix('(') else { continue };
        let names: Vec<&str> =
            arguments(params).into_iter().map(|p| p.split(':').next().unwrap_or("").trim().trim_start_matches("mut ").trim()).collect();
        let after_params = &params[top_level(params).1..];
        let (Some(open), semi) = (after_params.find('{'), after_params.find(';')) else { continue };
        if semi.is_some_and(|s| s < open) {
            continue;
        }
        let body = &after_params[open..];
        let body = &body[..block_len(body)];
        let skip = usize::from(names.first().is_some_and(|n| n.ends_with("self")));
        let line = &text[text[..at].rfind('\n').map_or(0, |n| n + 1)..at];
        out.push(SourceFn { file, name, public: line.contains("pub"), label: names.iter().position(|n| *n == "label").map(|k| k - skip), body });
    }
    out
}

/// The first argument at each of [`UNDO_MARKERS`] in `text`.
fn marker_args(text: &str) -> Vec<&str> {
    UNDO_MARKERS.iter().flat_map(|m| text.match_indices(m).filter_map(move |(at, _)| arguments(&text[at + m.len()..]).first().copied())).collect()
}

/// The argument at `g`'s `label` position at each call of `g` in `text`.
fn call_args<'a>(text: &'a str, g: &SourceFn) -> Vec<&'a str> {
    let Some(k) = g.label else { return vec![] };
    let call = format!("{}(", g.name);
    text.match_indices(&call)
        .filter(|(at, _)| !text[..*at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') && !text[..*at].ends_with("fn "))
        .filter_map(|(at, _)| arguments(&text[at + call.len()..]).get(k).copied())
        .collect()
}

/// An expression without a leading `&` and trailing conversions (`.into()`, `.to_string()`).
fn bare(e: &str) -> &str {
    let mut e = e.trim().trim_start_matches('&');
    while let Some(s) = [".into()", ".to_string()", ".clone()", ".as_str()", ".to_owned()"].iter().find_map(|t| e.strip_suffix(t)) {
        e = s.trim_end();
    }
    e
}

/// Add the step names expression `e` in source file `file` gives (see [`undo_labels`]) to `out`,
/// and the format string of a name it builds with `format!` to `formatted`.
fn label_literals(
    e: &str,
    file: usize,
    fns: &[SourceFn],
    out: &mut std::collections::BTreeSet<String>,
    formatted: &mut std::collections::BTreeSet<String>,
) {
    let mut e = bare(e);
    if let Some(args) = e.strip_prefix("format!(") {
        formatted.extend(args.trim_start().strip_prefix('"').and_then(string_literal));
        return;
    }
    if let Some(body) = e.strip_prefix('|').and_then(|r| r.find('|').map(|n| &r[n + 1..])) {
        e = body.trim();
    }
    let values = string_literals(e, |before, after| {
        let before = before.trim_end();
        let value = before.is_empty() || ["{", "=>", "else", "Some(", "unwrap_or("].iter().any(|t| before.ends_with(t));
        value && !after.trim_start().starts_with("=>") && !after.trim_start().starts_with('|')
    });
    if !values.is_empty() {
        out.extend(values);
        return;
    }
    let Some(inner) = e.strip_suffix(')') else { return };
    let Some(open) = inner.rfind('(').filter(|n| !inner[n + 1..].contains(')')) else { return };
    let name_at = inner[..open].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |n| n + 1);
    let name = &inner[name_at..open];
    let method = inner[..name_at].ends_with('.');
    if name.ends_with("label") && (name != "label" || method) {
        for f in fns.iter().filter(|f| f.file == file && f.name == name) {
            out.extend(string_literals(f.body, |_, after| !after.trim_start().starts_with("=>") && !after.trim_start().starts_with('|')));
        }
    }
}

/// The string literals in `e` that `keep` accepts, given the text before and after each.
fn string_literals(e: &str, keep: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(c) = e[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(e, i)).flatten() {
            if c == '"'
                && let Some(lit) = string_literal(&e[i + 1..])
                && keep(&e[..i], &e[end..])
            {
                out.push(lit);
            }
            i = end;
        } else {
            i += c.len_utf8();
        }
    }
    out
}

/// For translators, `cargo test -p vectorcraft-ui-egui dump_source_strings` writes every English
/// interface string to `VECTORCRAFT_I18N_DUMP=<file>`, and every status and error message as a
/// catalog template to `VECTORCRAFT_I18N_DUMP_MESSAGES=<file>`, one per line.
#[test]
fn dump_source_strings() {
    if let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP") {
        let mut all = crate::menus::menu_strings();
        all.extend(tl_literals());
        all.extend(undo_labels());
        all.extend(undo_scan().templates);
        let text: String = all.iter().map(|s| format!("{}\n", s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t"))).collect();
        std::fs::write(path, text).expect("write dump");
    }
    if let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP_MESSAGES") {
        std::fs::write(path, message_literals().iter().map(|m| format!("{m}\n")).collect::<String>()).expect("write dump");
    }
}

fn cs() -> Lang {
    Lang::from_code("cs").expect("cs registered")
}

/// Languages whose catalogs leave [`MENU_KEEP_AS_IS`] in English.
const KEEPS_MENU_NAMES: [&str; 20] =
    ["cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "fi", "tr", "ca"];

/// Menu labels the menu-complete catalogs (Czech, German, Spanish, Italian, Japanese, Brazilian Portuguese) show as they are: the product name, a format name, the built-in workspace
/// names and the perspective grid presets (names, shown untranslated wherever else they appear).
/// Each language's own name in the Language menu is left alone too.
const MENU_KEEP_AS_IS: &[&str] = &[
    "VectorCraft",
    "OpenType",
    "Essentials",
    "Essentials Classic",
    "Automation",
    "Layout",
    "Painting",
    "Printing and Proofing",
    "Tracing",
    "Touch",
    "Typography",
    "Web",
    "[1P-Normal View]",
    "[1P-Low View]",
    "[1P-High View]",
    "[2P-Normal View]",
    "[2P-Low View]",
    "[2P-High View]",
    "[3P-Normal View]",
    "[3P-Low View]",
];

/// Every menu title, submenu, item and header label with its command and params (`""`/null for
/// the ones that run nothing).
fn menu_labels() -> Vec<(&'static str, &'static str, serde_json::Value)> {
    use crate::menus::Item;
    fn walk(items: &[Item], out: &mut Vec<(&'static str, &'static str, serde_json::Value)>) {
        for i in items {
            match i {
                Item::Cmd(l, id, p) => out.push((l, id, p.clone())),
                Item::Todo(l, _) | Item::Header(l) => out.push((l, "", serde_json::Value::Null)),
                Item::Sub(l, children) => {
                    out.push((l, "", serde_json::Value::Null));
                    walk(children, out);
                }
                Item::Sep => {}
            }
        }
    }
    let mut labels = vec![];
    for (title, items) in crate::menus::menu_tree() {
        labels.push((title, "", serde_json::Value::Null));
        walk(&items, &mut labels);
    }
    labels
}

/// The labels toggling items switch to ("Show Guides" once guides are hidden…), from a document
/// with every toggle flipped.
fn toggled_labels() -> Vec<String> {
    use serde_json::json;
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("object.envelope.makeWithWarp", json!({})).unwrap();
    let toggles = [
        "view.outline",
        "view.edges",
        "view.cornerWidget",
        "view.textThreads",
        "type.hiddenCharacters",
        "view.gradientAnnotator",
        "view.livePaintGaps",
        "view.artboards",
        "view.template",
        "view.rulers",
        "view.videoRulers",
        "view.boundingBox",
        "view.transparencyGrid",
        "view.guides",
        "view.grid",
        "view.guides.lock",
        "view.slices.hide",
        "view.printTiling",
        "perspective.grid.show",
        "perspective.grid.rulers",
        "perspective.grid.lock",
        "object.envelope.editContents",
    ];
    let mut labels = vec![];
    for id in toggles {
        let before = crate::menus::dynamic_label(&app, id, "");
        app.run(id, json!({})).unwrap();
        let after = crate::menus::dynamic_label(&app, id, "");
        assert_ne!(before, after, "{id} didn't toggle its label");
        labels.extend([before, after]);
    }
    labels
}

/// Prints, per language, the strings its catalog has no row for: menu strings (titles, submenus,
/// items, headers, command labels and menu paths), `tl!` literals, undo step names and their
/// templates, the menu labels with the Show/Hide pairs and the context menu, render-site labels,
/// status and error messages, PostScript error meanings and default swatch names. The interface
/// shows a missing string in English, so a gap never fails this test; its assertions catch a scan
/// that stopped finding strings. `cargo xtask ci` and `cargo xtask i18n-report` run it with
/// `--nocapture`.
#[test]
fn untranslated_report() {
    use std::collections::{BTreeMap, BTreeSet};
    // For each kind of string, the languages checked and what each one lacks.
    let mut gaps: Vec<(&str, BTreeMap<&str, BTreeSet<String>>)> = Vec::new();
    let mut add = |code: &'static str, what: &'static str, missing: BTreeSet<String>| {
        let at = gaps.iter().position(|(w, _)| *w == what).unwrap_or_else(|| {
            gaps.push((what, BTreeMap::new()));
            gaps.len() - 1
        });
        gaps[at].1.insert(code, missing);
    };

    let strings = crate::menus::menu_strings();
    assert!(strings.len() > 500, "menu scan found only {} strings", strings.len());
    let literals = tl_literals();
    assert!(literals.len() > 300, "scan found only {} tl! literals", literals.len());
    // Undo step names: the recorded names, every effect's name without its ellipsis, and the
    // Liquify tools' step names (`{} Tool`). A name's row is its plain row or the row of the same
    // name with an ellipsis (`panels::history::step_name`); a template made with
    // `Session::edit_named` needs a plain row, which `panels::history::entry_name` fills.
    let scan = undo_scan();
    let mut undo = scan.labels;
    assert!(undo.len() > 300, "scan found only {} undo labels", undo.len());
    undo.extend(vectorcraft_effects::effect_catalog().iter().map(|e| e.label.trim_end_matches('…').to_string()));
    let liquify = ["warp", "twirl", "pucker", "bloat", "scallop", "crystallize", "wrinkle"];
    undo.extend(liquify.iter().filter_map(|k| vectorcraft_tools::distort::liquify::LiquifyKind::parse(k)).map(|k| format!("{} Tool", k.label())));
    assert_eq!(scan.formatted.iter().collect::<Vec<_>>(), ["{} Tool"], "step names built with format! (make them with Session::edit_named)");
    assert!(scan.templates.len() >= 3, "scan found only the templates {:?}", scan.templates);
    let messages = message_literals();
    assert!(messages.len() > 300, "scan found only {} messages", messages.len());
    let (swatches, groups) = vectorcraft_color::default_swatches(vectorcraft_color::cms::Model::Rgb);
    // The greys' `K=` values read the same in any language.
    let swatch_names: BTreeSet<String> = swatches
        .iter()
        .map(|s| s.name.clone())
        .chain(groups.iter().flat_map(|g| std::iter::once(g.name.clone()).chain(g.swatches.iter().map(|s| s.name.clone()))))
        .chain([vectorcraft_color::swatch::REGISTRATION.to_string()])
        .filter(|n| !n.starts_with("K="))
        .collect();

    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        let cat = l.catalog();
        let lang = Lang::from_code(l.code).expect("registered");
        // Languages that keep the product, workspace and perspective preset names in English.
        let kept = |s: &str| KEEPS_MENU_NAMES.contains(&l.code) && MENU_KEEP_AS_IS.contains(&s);
        add(l.code, "menu strings", strings.iter().filter(|s| !kept(s) && cat.plain(s).is_none()).cloned().collect::<BTreeSet<_>>());
        add(l.code, "tl! strings", literals.iter().filter(|s| cat.plain(s).is_none()).cloned().collect::<BTreeSet<_>>());
        add(l.code, "undo labels", undo.iter().filter(|s| crate::panels::history::step_name(lang, s).is_none()).cloned().collect::<BTreeSet<_>>());
        add(l.code, "undo label templates", scan.templates.iter().filter(|t| cat.plain(t).is_none()).cloned().collect::<BTreeSet<_>>());
        let (entries, _) = parse_entries(l.source);
        let rows: BTreeSet<(&str, &str)> = entries.iter().map(|(ctx, src, _)| (ctx.as_str(), src.as_str())).collect();
        let render_site = RENDER_SITE_ROWS.iter().filter(|row| !rows.contains(row)).map(|(ctx, src)| format!("{ctx}: {src}"));
        add(l.code, "render-site labels", render_site.collect());
        add(l.code, "swatch names", swatch_names.iter().filter(|n| cat.contextual("swatch", n).is_none()).cloned().collect::<BTreeSet<_>>());
    }

    // The menu labels as the menus show them, with what menu strings don't hold: the labels
    // toggling items switch to, the canvas context menu, the macOS menu bar's own labels and its
    // Settings submenu's Preferences pages.
    let labels = menu_labels();
    let language_name = |l: &str| Lang::all().any(|lang| lang.name() == l);
    // Font names and sizes, and installed plug-ins' own names, are not interface text.
    let interface = |(label, id, p): &(&str, &str, serde_json::Value)| {
        *id != "text.setStyle"
            && !crate::menus::lists_plugin(id, p)
            && strings.contains(*label)
            && !MENU_KEEP_AS_IS.contains(label)
            && !language_name(label)
    };
    let mut shown: BTreeSet<String> = labels.iter().filter(|l| interface(l)).map(|(l, ..)| l.to_string()).collect();
    shown.extend(toggled_labels());
    shown.extend(crate::menus::CONTEXT_LABELS.iter().map(|l| l.to_string()));
    shown.extend(crate::native_menu::MAC_LABELS.iter().map(|l| l.to_string()));
    shown.extend(vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES.iter().map(|l| l.to_string()));
    for code in KEEPS_MENU_NAMES {
        let lang = Lang::from_code(code).expect("registered");
        // Labels the menu strings above already listed aren't listed twice.
        let missing = shown.iter().filter(|l| !has(lang, l) && (!lang.complete_menus() || !strings.contains(*l)));
        add(code, "menu labels", missing.cloned().collect::<BTreeSet<_>>());
    }

    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        let rows: BTreeSet<&str> = entries.iter().filter(|(ctx, _, _)| ctx == "@msg").map(|(_, src, _)| src.as_str()).collect();
        add(code, "messages", messages.iter().filter(|m| !rows.contains(m.as_str())).cloned().collect::<BTreeSet<_>>());
        let meanings = vectorcraft_eps::POSTSCRIPT_ERRORS.iter().filter(|(_, m)| message(lang, m) == *m).map(|(_, m)| m.to_string());
        add(code, "PostScript error meanings", meanings.collect());
    }

    // A string that every language checked lacks is listed once, under "every language".
    let mut by_language: BTreeMap<&str, Vec<(&str, Vec<&String>)>> = BTreeMap::new();
    let mut everywhere = vec![];
    for (what, langs) in &gaps {
        let first = langs.values().next();
        let common: BTreeSet<&String> = first.into_iter().flatten().filter(|m| langs.values().all(|set| set.contains(*m))).collect();
        if !common.is_empty() {
            everywhere.push((*what, langs.len(), common.iter().copied().collect::<Vec<_>>()));
        }
        for (code, missing) in langs {
            let rest: Vec<&String> = missing.iter().filter(|m| !common.contains(m)).collect();
            if !rest.is_empty() {
                by_language.entry(code).or_default().push((*what, rest));
            }
        }
    }
    let total: usize = gaps.iter().flat_map(|(_, langs)| langs.values()).map(BTreeSet::len).sum();
    let mut out = format!("Untranslated strings, shown in English until a catalog row is added: {total} missing rows\n");
    let list = |out: &mut String, what: &str, missing: &[&String]| {
        *out += &format!("  {what} ({}):\n", missing.len());
        for m in missing {
            *out += &format!("    {}\n", m.replace('\n', "\\n"));
        }
    };
    if !everywhere.is_empty() {
        out += "\nevery language:\n";
        for (what, langs, missing) in &everywhere {
            list(&mut out, &format!("{what}, all {langs} languages checked"), missing);
        }
    }
    for (code, kinds) in &by_language {
        out += &format!("\n{code}:\n");
        for (what, missing) in kinds {
            list(&mut out, what, missing);
        }
    }
    println!("{out}");
}

/// The names [`MENU_KEEP_AS_IS`] lists are menu labels, and no catalog that keeps them in English
/// translates them.
#[test]
fn kept_menu_names_are_menu_labels_left_in_english() {
    let labels = menu_labels();
    for keep in MENU_KEEP_AS_IS {
        assert!(labels.iter().any(|(l, ..)| l == keep), "`{keep}` isn't a menu label");
        for code in KEEPS_MENU_NAMES {
            assert!(!has(Lang::from_code(code).expect("registered"), keep), "{code}: `{keep}` is both kept and translated");
        }
    }
}

/// Georgian interface text has no capitals: no row uses Mtavruli.
#[test]
fn georgian_rows_have_no_mtavruli() {
    let (entries, _) = parse_entries(ka().0.source);
    let caps: Vec<_> = entries.iter().filter(|(_, _, tr)| tr.chars().any(|c| matches!(c, '\u{1C90}'..='\u{1CBF}'))).collect();
    assert!(caps.is_empty(), "Mtavruli in ka.tsv: {caps:?}");
}

fn de() -> Lang {
    Lang::from_code("de").expect("de registered")
}

fn ka() -> Lang {
    Lang::from_code("ka").expect("ka registered")
}

fn hu() -> Lang {
    Lang::from_code("hu").expect("hu registered")
}

fn ko() -> Lang {
    Lang::from_code("ko").expect("ko registered")
}

fn vi() -> Lang {
    Lang::from_code("vi").expect("vi registered")
}

fn uk() -> Lang {
    Lang::from_code("uk").expect("uk registered")
}

/// Every Ukrainian plural has all three forms; exercise different endings through real lookups.
#[test]
fn ukrainian_plurals_have_three_forms() {
    let (entries, _) = parse_entries(uk().0.source);
    let mut count = 0;
    for (ctx, src, translated) in entries {
        if ctx != "@plural" {
            continue;
        }
        count += 1;
        let (one, other) = src.split_once('|').expect("one|other source");
        let forms: Vec<_> = translated.split('|').collect();
        assert_eq!(forms.len(), 3, "{src}");
        for (n, form) in [
            (1, 0),
            (2, 1),
            (4, 1),
            (0, 2),
            (5, 2),
            (11, 2),
            (12, 2),
            (14, 2),
            (21, 0),
            (24, 1),
            (111, 2),
            (114, 2),
            (121, 0),
            (122, 1),
            (u64::MAX, 2),
        ] {
            assert_eq!(trn(uk(), n, one, other), forms[form].replace("{n}", &n.to_string()), "{src}: {n}");
        }
    }
    assert!(count >= 13, "Ukrainian plural rows missing");
}

/// Catalogs written with spaces between words keep a fragment's leading and trailing spaces: the
/// hint bar and a few labels are joined from pieces (" to finish", "Press ").
#[test]
fn spaced_catalogs_keep_the_spaces_around_fragments() {
    let edge = |s: &str| (s.len() - s.trim_start_matches(' ').len(), s.len() - s.trim_end_matches(' ').len());
    for l in LANGUAGES.iter().filter(|l| !l.source.is_empty() && !l.source.chars().any(is_cjk)) {
        let (entries, _) = parse_entries(l.source);
        let bad: Vec<_> = entries.iter().filter(|(ctx, src, tr)| ctx.is_empty() && edge(src) != edge(tr)).map(|(_, src, _)| src).collect();
        assert!(bad.is_empty(), "{}: spaces differ around {bad:?}", l.code);
    }
}

#[test]
fn czech_plurals_have_three_forms() {
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 11, 21].into_iter().map(plural_czech).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2]);
    // Every Czech plural row has the three forms, and each count picks its own.
    let (entries, _) = parse_entries(cs().0.source);
    let mut count = 0;
    for (ctx, src, translated) in entries {
        if ctx != "@plural" {
            continue;
        }
        count += 1;
        let (one, other) = src.split_once('|').expect("one|other source");
        let forms: Vec<_> = translated.split('|').collect();
        assert_eq!(forms.len(), 3, "{src}");
        for (n, form) in [(1, 0), (2, 1), (4, 1), (0, 2), (5, 2), (11, 2), (21, 2), (u64::MAX, 2)] {
            assert_eq!(trn(cs(), n, one, other), forms[form].replace("{n}", &n.to_string()), "{src}: {n}");
        }
    }
    assert!(count >= 13, "Czech plural rows missing");
}

#[test]
fn russian_plurals_have_three_forms() {
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 11, 21, 22, 25, 111, 121].into_iter().map(plural_east_slavic).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 0, 1, 2, 2, 0]);
}

/// Czech, German, Spanish, French, Italian, Hungarian, Polish and Swedish letters (and the punctuation their text uses) come from each family's own first font, not
/// from a fallback further down the stack. (`has_glyph` can't tell: it counts characters of the
/// face that draws missing glyphs, the first one, as missing.)
#[test]
fn latin_script_glyphs_are_available_without_system_fonts() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    ctx.fonts_mut(|fonts| {
        let families: Vec<_> = fonts.definitions().families.iter().map(|(f, stack)| (f.clone(), stack.first().cloned())).collect();
        for (family, first) in families {
            let first = first.unwrap();
            // Public 0.37 view: the family's characters with the fonts that have each (the old
            // `fonts.font(...).characters()` reached a now-private field).
            let chars = fonts.characters(&family);
            for ch in "áčďéěíňóřšťúůýžÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ„“‚‘…–ñÑüÜ¿¡”àèìòùÀÈÌÒÙ«»’âêîôûçëïÿœæÂÊÎÔÛÇËÏŒÆäöÄÖßőűŐŰąćęłńśźżĄĆĘŁŃŚŹŻåÅ\u{a0}".chars()
            {
                assert!(chars.get(&ch).is_some_and(|fonts| fonts.contains(&first)), "{first} ({family:?}) has no {ch}");
            }
        }
    });
}

/// Russian and Ukrainian letters (and their punctuation) come from each family's own first font, not
/// from a fallback further down the stack.
#[test]
fn cyrillic_glyphs_are_available_without_system_fonts() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    ctx.fonts_mut(|fonts| {
        let families: Vec<_> = fonts.definitions().families.iter().map(|(f, stack)| (f.clone(), stack.first().cloned())).collect();
        for (family, first) in families {
            let first = first.unwrap();
            // Public 0.37 view: the family's characters with the fonts that have each (the old
            // `fonts.font(...).characters()` reached a now-private field).
            let chars = fonts.characters(&family);
            for ch in "абвгдеёжзийклмнопрстуфхцчшщъыьэюяАБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯґєіїҐЄІЇ«»„“…–’".chars()
            {
                assert!(chars.get(&ch).is_some_and(|fonts| fonts.contains(&first)), "{first} ({family:?}) has no {ch}");
            }
        }
    });
}

/// A language chosen in an older version (saved with the UI state as `"language"`) carries over to
/// the `interfaceLanguage` preference, and isn't written back.
#[test]
fn a_language_saved_by_an_older_version_carries_over() {
    for (saved, want) in [("ja", "ja"), ("cs", "cs"), ("pt-br", "pt-br"), ("en", "auto"), ("xx", "auto")] {
        let ui: crate::state::UiState = serde_json::from_value(serde_json::json!({"language": saved})).unwrap();
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.ui = ui;
        crate::prefs_dialog::restore(&mut app);
        assert_eq!(app.session.prefs.interface_language, want, "saved {saved}");
        assert!(serde_json::to_value(&app.ui).unwrap().get("language").is_none());
    }
    // A preference already set wins over the old field.
    let ui: crate::state::UiState =
        serde_json::from_value(serde_json::json!({"language": "ja", "engine_prefs": {"interfaceLanguage": "cs"}})).unwrap();
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.ui = ui;
    crate::prefs_dialog::restore(&mut app);
    assert_eq!(app.session.prefs.interface_language, "cs");
}

#[test]
fn messages_translate_by_template_and_their_parts() {
    let c = Catalog::parse(concat!(
        "@msg\tnothing selected\tnada seleccionado\n",
        "@msg\t{path}: {e}\t{path}: {e}\n",
        "@msg\tCouldn't open {name}: {e}\tNo se pudo abrir {name}: {e}\n",
        "@msg\tcommand `{0}` is not available right now: {1}\tel comando `{0}` no está disponible ahora: {1}\n",
        "@msg\t{a}{b}\tnever\n",
        "@msg\t{n} of {total}\t{total} contiene {n}\n",
    ));
    assert_eq!(c.message("nothing selected").map(|m| m.0), Some("nada seleccionado"));
    // The template with the most literal text wins over the generic `{path}: {e}`.
    let (tr, caps) = c.message("Couldn't open a: b.svg: damaged").unwrap();
    assert_eq!(tr, "No se pudo abrir {name}: {e}");
    assert_eq!(caps, [("name", "a"), ("e", "b.svg: damaged")], "a placeholder takes the shortest text that lets the rest match");
    assert_eq!(c.message("x of y").unwrap().1, [("n", "x"), ("total", "y")]);
    assert!(c.message("of y").is_none(), "a placeholder is never empty");
    assert!(c.message("something else").is_none());
    assert!(c.message(&"x".repeat(5000)).is_none());

    let es = Lang::from_code("es").unwrap();
    assert_eq!(message(es, "no such message"), "no such message");
    // A reason inside a message is translated too, and parts without entries stay as they are.
    let nested = "command `object.group` is not available right now: nothing selected";
    let shown = message(es, nested);
    assert!(shown.contains("`object.group`") && !shown.contains("nothing selected") && !shown.contains("not available"), "{shown}");
    assert_eq!(message(Lang::EN, nested), nested);
}

/// The note `recolor.apply` and the Edit Colors commands give when they leave out locked or hidden
/// objects is translated whole, its counts included, in every complete-message language.
#[test]
fn recolor_notes_on_objects_left_unchanged_translate_with_their_counts() {
    let notes = [
        ("Recolor completed; 2 locked objects and 1 hidden object were left unchanged", ["2", "1"]),
        ("Recolor completed; 1 locked object and 3 hidden objects were left unchanged", ["1", "3"]),
        ("Recolor completed; 1 locked object was left unchanged", ["1", "1"]),
        ("Recolor completed; 5 hidden objects were left unchanged", ["5", "5"]),
    ];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (note, counts) in notes {
            let shown = message(lang, note);
            // Whole words, so that a translation such as Catalan's "objecte" is not read as English.
            let english = ["completed", "locked", "hidden", "object", "objects", "unchanged"];
            assert!(shown.split(|c: char| !c.is_alphabetic()).all(|w| !english.contains(&w)), "{code}: {shown}");
            assert!(counts.iter().all(|n| shown.contains(n)), "{code}: {shown}");
        }
    }
}

#[test]
fn package_status_message_templates_parse() {
    let c = Catalog::parse(concat!(
        "@msg\tPackaged {n} file(s) in {folder}\tpackaged plain\n",
        "@msg\tPackaged {n} file(s) in {folder} {note}\tpackaged noted\n",
        "@msg\t{count} file(s) in {folder}. Show the package folder?\tdetail plain\n",
        "@msg\t{count} file(s) in {folder} {note}. Show the package folder?\tdetail noted\n",
    ));
    assert!(c.message("Packaged 1 file(s) in /tmp/foo").is_some());
    assert!(c.message("Packaged 1 file(s) in /tmp/foo (1 linked file(s) not found)").is_some());
    assert!(c.message("2 file(s) in /tmp/foo. Show the package folder?").is_some());
    assert!(c.message("2 file(s) in /tmp/foo (1 linked file(s) not found). Show the package folder?").is_some());
    assert!(Catalog::parse("@msg\tPackaged {n} file(s) in {folder}{note}\tbad\n").message("Packaged 1 file(s) in /a b").is_none());
}

/// A `@msgplural` row translates both English forms of a message with a count, and the count
/// picks the language's form.
#[test]
fn messages_with_a_count_take_the_plural_form() {
    let c = Catalog::parse("@msgplural\tSaved {n} file|Saved {n} files\tone {n}|few {n}|many {n}\n");
    for s in ["Saved 1 file", "Saved 4 files"] {
        let (forms, caps) = c.message_forms(s).unwrap();
        assert_eq!(forms, ["one {n}", "few {n}", "many {n}"], "{s}");
        assert_eq!(caps.len(), 1, "{s}");
    }
    assert!(c.message_forms("Saved 1 files!").is_none());
    let cs = cs();
    assert_eq!(message(cs, "Packaged 1 file"), "Zabalen 1 soubor");
    assert_eq!(message(cs, "Packaged 4 files"), "Zabaleny 4 soubory");
    assert_eq!(message(cs, "Packaged 21 files"), "Zabaleno 21 souborů");
}

/// The import's note for warped type that opened as drawn art is translated in every complete
/// catalog. It is built from a constant, which the scan of message literals doesn't see.
#[test]
fn warped_type_note_is_translated() {
    let note = "warped text objects that came in as drawn art, not as editable type: 3";
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let shown = message(lang, note);
        assert_ne!(shown, note, "{code}");
        assert!(shown.contains('3'), "{code}: {shown}");
    }
}

/// The Gamut Check status line names a count of 1 in the singular and other counts in the plural,
/// and every complete catalog translates it, in the form its plural rule picks.
#[test]
fn gamut_check_status_takes_the_plural_form() {
    use crate::panels::separations::gamut_check_status;
    assert_eq!(gamut_check_status(0), "All colors are within the CMYK gamut");
    assert_eq!(gamut_check_status(1), "1 color is out of the CMYK gamut");
    assert_eq!(gamut_check_status(3), "3 colors are out of the CMYK gamut");
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for n in [0, 1, 3] {
            let s = gamut_check_status(n);
            assert_ne!(message(lang, &s), s, "{code}: {s}");
        }
    }
    assert_eq!(message(de(), &gamut_check_status(1)), "1 Farbe liegt außerhalb des CMYK-Farbumfangs");
    assert_eq!(message(de(), &gamut_check_status(3)), "3 Farben liegen außerhalb des CMYK-Farbumfangs");
    let cs = cs();
    assert_eq!(message(cs, &gamut_check_status(1)), "1 barva je mimo gamut CMYK");
    assert_eq!(message(cs, &gamut_check_status(3)), "3 barvy jsou mimo gamut CMYK");
    assert_eq!(message(cs, &gamut_check_status(5)), "5 barev je mimo gamut CMYK");
}

/// The link, font and export status messages with a count, in English for a count of 1 and of 2.
const COUNTED_STATUS_MESSAGES: &[(&str, &str)] = &[
    ("1 linked file can't be read here: its preview shows", "2 linked files can't be read here: their previews show"),
    ("1 image couldn't be decoded and shows as a crossed box", "2 images couldn't be decoded and show as crossed boxes"),
    ("1 linked image changed on disk: Update Links shows the new version", "2 linked images changed on disk: Update Links shows the new versions"),
    ("Updated 1 linked file changed on disk", "Updated 2 linked files changed on disk"),
    ("Exported /out/a.svg with 1 note: a note", "Exported /out/a.svg with 2 notes: a note"),
    ("Embedded 1 image; 1 missing: relink them first", "Embedded 2 images; 1 missing: relink them first"),
    ("Embedded 1 image", "Embedded 2 images"),
    ("Relinked 1 image", "Relinked 2 images"),
    ("Relinked 1 image; not in that folder: a.png", "Relinked 2 images; not in that folder: a.png"),
    ("Relinked 1 image; a.png couldn't be relinked: a reason", "Relinked 2 images; a.png couldn't be relinked: a reason"),
    ("Updated 1 image", "Updated 2 images"),
    ("Replaced 1 image", "Replaced 2 images"),
    ("Broke 1 link", "Broke 2 links"),
    ("Broke 1 link; 1 can't be read: relink or update them first", "Broke 2 links; 1 can't be read: relink or update them first"),
    ("1 font file is now in /fonts", "2 font files are now in /fonts"),
    ("1 font file is now in /fonts; 1 couldn't be added: a reason", "2 font files are now in /fonts; 1 couldn't be added: a reason"),
    ("1 font isn't available here: its text shows in another font", "2 fonts aren't available here: their text shows in another font"),
    ("Copied the CSS of 1 object", "Copied the CSS of 2 objects"),
    ("Downloaded 1 file", "Downloaded 2 files"),
    ("Exported 1 file to /out", "Exported 2 files to /out"),
    ("Loaded “Tidy”: 1 step was left out", "Loaded “Tidy”: 2 steps were left out"),
];

/// The link, font, export and action set status messages name a count of 1 in the singular and other counts in
/// the plural, and every complete catalog translates both, in the form its plural rule picks.
#[test]
fn counted_status_messages_take_the_plural_form() {
    use crate::panels::links::{relinked_status, updated_status};
    assert_eq!((relinked_status(1), relinked_status(2)), ("Relinked 1 image".to_string(), "Relinked 2 images".to_string()));
    assert_eq!((updated_status(1), updated_status(2)), ("Updated 1 image".to_string(), "Updated 2 images".to_string()));
    assert_eq!(crate::dialogs::missing_links::changed_on_disk_status(1), COUNTED_STATUS_MESSAGES[2].0);
    assert_eq!(crate::dialogs::missing_links::changed_on_disk_status(2), COUNTED_STATUS_MESSAGES[2].1);
    assert_eq!(crate::io::exported_with_notes("/out/a.svg", 1, "a note"), COUNTED_STATUS_MESSAGES[4].0);
    assert_eq!(crate::io::exported_with_notes("/out/a.svg", 2, "a note"), COUNTED_STATUS_MESSAGES[4].1);
    // A value that holds placeholder text is not filled in again.
    assert_eq!(crate::io::exported_with_notes("/out/{first}.svg", 1, "{n}"), "Exported /out/{first}.svg with 1 note: {n}");
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for s in COUNTED_STATUS_MESSAGES.iter().flat_map(|(one, other)| [one, other]) {
            assert_ne!(message(lang, s), *s, "{code}: {s}");
        }
    }
    let cs = cs();
    assert_eq!(message(cs, &relinked_status(1)), "Znovu propojen 1 obrázek");
    assert_eq!(message(cs, &relinked_status(3)), "Znovu propojeny 3 obrázky");
    assert_eq!(message(cs, &relinked_status(5)), "Znovu propojeno 5 obrázků");
    assert_eq!(message(de(), &updated_status(1)), "1 Bild aktualisiert");
    assert_eq!(message(de(), &updated_status(2)), "2 Bilder aktualisiert");
    let noted = crate::io::exported_with_notes("/out/{n}.svg", 1, "{what}");
    assert_eq!(message(de(), &noted), "/out/{n}.svg mit 1 Hinweis exportiert: {what}");
    let ru = Lang::from_code("ru").expect("registered");
    assert_eq!(message(ru, COUNTED_STATUS_MESSAGES[19].0), "Экспортирован 1 файл в /out");
    assert_eq!(message(ru, "Exported 3 files to /out"), "Экспортировано 3 файла в /out");
    assert_eq!(message(ru, "Exported 5 files to /out"), "Экспортировано 5 файлов в /out");
}

/// The open, save and recovery status messages with a count, in English for a count of 1 and of 2.
const OPEN_SAVE_COUNTED_MESSAGES: &[(&str, &str)] = &[
    ("Opened with 1 note: a note", "Opened with 2 notes: a note"),
    ("Saved /out/a.pdf with 1 note: a note", "Saved /out/a.pdf with 2 notes: a note"),
    ("Discarded 1 recovered document", "Discarded 2 recovered documents"),
    ("Restored 1 recovered document: save it to keep it", "Restored 2 recovered documents: save them to keep them"),
];

/// The open, save and recovery status messages and the Global Edit options' Results line name a
/// count of 1 in the singular and other counts in the plural, and every complete catalog
/// translates both, in the form its plural rule picks.
#[test]
fn open_save_and_recovery_messages_take_the_plural_form() {
    use crate::io::{opened_with_notes, saved_status};
    use crate::panels::properties::global_edit_results;
    use crate::recovery::recovered_status;
    let saved = |path: &str, n: usize| saved_status(path, &serde_json::json!({ "warnings": vec!["a note"; n] }));
    let made = [
        (opened_with_notes(1, "a note"), opened_with_notes(2, "a note")),
        (saved("/out/a.pdf", 1), saved("/out/a.pdf", 2)),
        (recovered_status(true, 1), recovered_status(true, 2)),
        (recovered_status(false, 1), recovered_status(false, 2)),
    ];
    for ((one, other), (want_one, want_other)) in made.iter().zip(OPEN_SAVE_COUNTED_MESSAGES) {
        assert_eq!((one.as_str(), other.as_str()), (*want_one, *want_other));
    }
    // A path that holds placeholder text is not filled in again.
    assert_eq!(saved("/out/{first}.pdf", 1), "Saved /out/{first}.pdf with 1 note: a note");
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for s in OPEN_SAVE_COUNTED_MESSAGES.iter().flat_map(|(one, other)| [one, other]) {
            assert_ne!(message(lang, s), *s, "{code}: {s}");
        }
    }
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        let lang = Lang::from_code(l.code).expect("registered");
        for n in [1, 2] {
            assert_ne!(trn(lang, n, "Result: {n}", "Results: {n}"), trn(Lang::EN, n, "Result: {n}", "Results: {n}"), "{}: {n}", l.code);
        }
    }
    assert_eq!((global_edit_results(1), global_edit_results(2)), ("Result: 1".to_string(), "Results: 2".to_string()));
    set_current(de());
    assert_eq!((global_edit_results(1), global_edit_results(2)), ("Ergebnis: 1".to_string(), "Ergebnisse: 2".to_string()));
    set_current(Lang::EN);
    assert_eq!(message(de(), OPEN_SAVE_COUNTED_MESSAGES[0].0), "Mit 1 Hinweis geöffnet: a note");
    assert_eq!(message(de(), OPEN_SAVE_COUNTED_MESSAGES[0].1), "Mit 2 Hinweisen geöffnet: a note");
    assert_eq!(message(de(), &saved("/out/{n}.pdf", 2)), "/out/{n}.pdf mit 2 Hinweisen gespeichert: a note");
    let cs = cs();
    assert_eq!(message(cs, &recovered_status(true, 1)), "Zahozen 1 obnovený dokument");
    assert_eq!(message(cs, &recovered_status(true, 3)), "Zahozeny 3 obnovené dokumenty");
    assert_eq!(message(cs, &recovered_status(true, 5)), "Zahozeno 5 obnovených dokumentů");
    let ru = Lang::from_code("ru").expect("registered");
    assert_eq!(message(ru, &opened_with_notes(1, "x")), "Открыто с 1 примечанием: x");
    assert_eq!(message(ru, &opened_with_notes(3, "x")), "Открыто с 3 примечаниями: x");
}

/// The `{n}` of a message with a count matches only a number, so a path that holds the text after
/// the path keeps its count and its whole path. A `{n}` in a message without a count is any text.
#[test]
fn a_count_placeholder_matches_only_a_number() {
    use crate::io::saved_status;
    let saved = |path: &str, n: usize| saved_status(path, &serde_json::json!({ "warnings": vec!["a note"; n] }));
    let path = "/out/art with notes.pdf";
    assert_eq!(message(de(), &saved(path, 1)), "/out/art with notes.pdf mit 1 Hinweis gespeichert: a note");
    assert_eq!(message(de(), &saved(path, 2)), "/out/art with notes.pdf mit 2 Hinweisen gespeichert: a note");
    let c = Catalog::parse("@msgplural\tSaved {path} with {n} note|Saved {path} with {n} notes\tone|other\n");
    let (_, caps) = c.message_forms("Saved /a with 1 b with 2 notes").expect("matches");
    assert_eq!(caps, [("path", "/a with 1 b"), ("n", "2")]);
    assert!(c.message_forms("Saved /a with b notes").is_none());
    assert_eq!(message(de(), "no brush `with 2`"), "kein Pinsel `with 2`");
}

/// The scan reads the message of an error made with one of the engine's [`ERROR_HELPERS`] or with
/// an error enum's variant, `EngineError::Disabled`'s reason included: a literal or a `format!`
/// string, on the call's line or wrapped over several lines, in the same template form as the
/// catalogs' rows. It does not read a call whose name ends in a helper's name (`is_bad(`), a method
/// call (`.err(`), `std::io::Error::new`, one of [`NAME_VARIANTS`], `EngineError::Disabled`'s
/// command id or a call in a comment. A `//` or `/*` inside a string or a raw string, or after a
/// character literal such as `'"'`, does not start a comment. Without the helpers, the scan reads
/// only the four variants in this source.
#[test]
fn message_scan_reads_error_helpers_and_variants() {
    let code = r##"
fn width(p: &Value, cmd: &str, key: &str) -> Result<f64> {
    let w = p.get(key).and_then(Value::as_f64).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
    if w < 0.0 {
        return Err(bad(cmd, "the width can't be negative"));
    }
    let e = bad(
        "object.example",
        format!(
            "`{}` must be between {lo:.1} and {hi}, not {w}",
            key
        ),
    );
    let _ = bad(&format!("{cmd}, {key}"), "a comma in the first argument");
    let _ = is_bad(cmd, "not an error helper");
    let _ = r.err("not an error helper either");
    let _ = err(format!("can't open `{name}`: damaged"));
    let _ = PsError::Limit(format!("the {what} is nested too deep"));
    let _ = Error::Manifest("the manifest has no name".into());
    let _ = std::io::Error::new(kind, "not an error variant");
    let _ = PsError::Ps("not an error message", op);
    let _ = EngineError::UnknownCommand("not.an.error.message".into());
    let _ = EngineError::Disabled("not.a.reason".into(), "there is nothing to clear".into());
    let _ = EngineError::Disabled(C.into(), format!("the {kind} panel is locked"));
    // return Err(bad(cmd, "not an error: a commented-out call"));
    /* let _ = err("not an error: a call in a block comment");
       /* nested */ let _ = err("not an error: after a nested block comment"); */
    let _ = ('"', '\'', b'"', bad(cmd, "the width // the height"));
    let _ = (r#"a "/* raw string"#, |s: &'a str| -> Result<&'a str> { Err(bad(cmd, "the height is too large")) });
    Err(error("Select only dimensions"))
}
"##;
    let engine: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "engine").map(|&(_, call, before)| (call, before)).collect();
    let found = source_messages(code, &engine);
    for want in [
        "`{key}` must be a number",
        "the width can't be negative",
        "`{_1}` must be between {lo} and {hi}, not {w}",
        "a comma in the first argument",
        "can't open `{name}`: damaged",
        "the {what} is nested too deep",
        "the manifest has no name",
        "there is nothing to clear",
        "the {kind} panel is locked",
        "the width // the height",
        "the height is too large",
        "Select only dimensions",
    ] {
        assert!(found.iter().any(|m| m == want), "{want} not in {found:#?}");
    }
    let named = |m: &String| m.contains("not an error") || m.contains("not.an.error") || m.contains("not.a.reason");
    assert!(!found.iter().any(named), "{found:#?}");
    let elsewhere = source_messages(code, &[]);
    let in_variants = ["nested", "manifest", "nothing to clear", "panel is locked"];
    assert!(elsewhere.len() == 4 && elsewhere.iter().all(|m| in_variants.iter().any(|w| m.contains(w))), "{elsewhere:#?}");
    // A message read this way matches its row: `{key}` keeps its name.
    let c = Catalog::parse("@msg\t`{key}` must be a number\t`{key}` ok\n");
    assert_eq!(c.message("`width` must be a number").map(|m| m.1), Some(vec![("key", "width")]));
}

/// The warnings of an Image Trace with keepColors, which the status bar shows and the scan above
/// doesn't read, are translated in each language of [`COMPLETE_MESSAGES`], with the colors kept.
#[test]
fn the_keep_colors_trace_warnings_are_translated() {
    use serde_json::json;
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    // A white field with a black square in it.
    let img = image::RgbaImage::from_fn(40, 40, |x, y| {
        image::Rgba(if (10..30).contains(&x) && (10..30).contains(&y) { [0, 0, 0, 255] } else { [255, 255, 255, 255] })
    });
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    let mut warnings = vec![];
    for (params, shown) in [
        (json!({"mode": "color", "palette": "limited", "colors": 2, "keepColors": ["#ff00ff", "#00ffff"]}), "#ff00ff, #00ffff"),
        (json!({"mode": "color", "palette": "limited", "colors": 2, "keepColors": ["#000000", "#00ffff"]}), "#00ffff"),
        (json!({"mode": "color", "palette": "limited", "colors": 2, "keepColors": ["#000000"], "ignoreWhite": true, "ignoreColor": "#000000"}), ""),
        (json!({"mode": "logo", "keepColors": ["#000000"]}), ""),
    ] {
        s.execute("file.place", &json!({"name": "square.png", "dataBase64": data, "link": false})).unwrap();
        let r = s.execute("imageTrace.makeAndExpand", &json!({ "params": params })).unwrap();
        let w = r["warnings"].as_array().and_then(|w| w.first()).and_then(|w| w.as_str()).unwrap_or_else(|| panic!("{params}: {r}"));
        warnings.push((w.to_string(), shown));
    }
    // Listed colors that were traced and produced no paths (no square image gives that).
    warnings.push(("Nothing was traced: the colors that match keepColors produce no paths.".to_string(), ""));
    for (warning, shown) in &warnings {
        for code in COMPLETE_MESSAGES {
            let tr = message(Lang::from_code(code).unwrap(), warning);
            assert!(tr != *warning && tr.contains(shown) && tr.contains("keepColors"), "{code}: {tr}");
        }
    }
}

/// The parameter errors of the blend spine commands (Move Spine Point, Reshape Spine Segment, Add
/// Spine Point and Delete Spine Point, which Direct Selection, the Pen and the Anchor Point tool
/// send) say what is wrong.
#[test]
fn blend_spine_param_errors_say_what_is_wrong() {
    use serde_json::json;
    use vectorcraft_engine::{EngineError, Session};
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let rectangle =
        |s: &mut Session, x: u64, y: u64| s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap()["id"].clone();
    let blend = |s: &mut Session, y: u64| {
        let keys = [0, 100].map(|x| rectangle(s, x, y));
        s.execute("object.blend.make", &json!({"ids": keys, "steps": 1})).unwrap()["id"].clone()
    };
    // `replaced` gives a new blend a new spine. `ring`'s spine is a closed path of two points with
    // both keys on the first, and `dot`'s spine is a single point.
    let replaced = |s: &mut Session, y: u64, anchors: serde_json::Value, closed: bool| {
        let b = blend(s, y);
        let path = s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [b, path]})).unwrap();
        s.execute("object.blend.replaceSpine", &json!({})).unwrap();
        b
    };
    let ring = replaced(&mut s, 200, json!([{"x": 0, "y": 205}, {"x": 100, "y": 205}]), true);
    let dot = replaced(&mut s, 400, json!([{"x": 0, "y": 405}]), false);
    let other = rectangle(&mut s, 0, 600);
    let b = blend(&mut s, 0);
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    for (cmd, p, want) in [
        ("object.blend.spine.moveAnchor", json!({}), "missing number `anchor`"),
        ("object.blend.spine.moveAnchor", json!({"anchor": -1, "x": 0, "y": 0}), "index must be ≥ 0"),
        ("object.blend.spine.moveAnchor", json!({"anchor": 0, "x": 1e12, "y": 0}), "x and y must be finite"),
        ("object.blend.spine.moveAnchor", json!({"anchor": 0, "x": 0, "y": 0, "handle": "up"}), "handle must be in or out"),
        ("object.blend.spine.moveAnchor", json!({"anchor": 5, "x": 0, "y": 0}), "no such spine point"),
        (
            "object.blend.spine.moveAnchor",
            json!({"anchor": 0, "x": 0, "y": 0, "with": "1"}),
            "`with` must be a list of at most 10000 spine point indexes",
        ),
        ("object.blend.spine.moveAnchor", json!({"anchor": 0, "x": 0, "y": 0, "with": [1], "handle": "in"}), "`with` moves points, not a handle"),
        ("object.blend.spine.moveAnchor", json!({"id": other, "anchor": 0, "x": 0, "y": 0}), "not a blend"),
        ("object.blend.spine.reshapeSegment", json!({"segment": 0, "dx": 1e12}), "t, dx and dy must be finite"),
        ("object.blend.spine.reshapeSegment", json!({"segment": 5, "dy": 10}), "no such spine segment"),
        ("object.blend.spine.removeAnchor", json!({"anchor": 0}), "a key object sits on that point"),
        ("object.blend.spine.removeAnchor", json!({"id": ring, "anchor": 1}), "a spine keeps at least two points"),
        ("object.blend.spine.addAnchor", json!({"id": dot, "x": 0, "y": 0}), "the blend has no spine"),
    ] {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{cmd} {p}: {e}");
    }
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 0, "x": 0, "y": 0})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == "select a blend or give `id`"), "{e}");
}

/// Import Data Sets' refusal of two header cells that name one variable is translated in every
/// language that translates every message.
#[test]
fn dataset_import_repeated_column_error_is_translated() {
    use serde_json::json;
    use vectorcraft_engine::{EngineError, Session};
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let e = s.execute("dataset.import", &json!({"csv": "City,City\nParis,Rome"})).unwrap_err();
    let want = "columns 1 and 2 both name the variable `City`";
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{e}");
    let e = e.to_string();
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let shown = message(lang, &e);
        assert!(!shown.contains(want), "{code}: {shown}");
    }
}

/// The errors of a command's parameter schema are translated in every language that translates
/// every message, with the parameter's path and the refused value kept.
#[test]
fn parameter_schema_errors_are_translated() {
    use serde_json::json;
    use vectorcraft_engine::Session;
    let scanned = message_literals();
    for template in [
        "the params must be an object, not {got}",
        "unknown parameter `{path}`; the object takes no keys",
        "unknown parameter `{path}`; it takes: {keys}",
        "missing `{path}`",
        "`{path}` must be {expected}, not {got}",
    ] {
        assert!(scanned.contains(template), "{template}");
    }
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let errors = [
        (s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": -1})).unwrap_err().to_string(), "`zoom`", "must be", "-1"),
        (s.execute("view.saved.new", &json!({"center": [0, 0]})).unwrap_err().to_string(), "`zoom`", "missing", ""),
        (s.execute("view.saved.list", &json!(5)).unwrap_err().to_string(), "5", "must be", ""),
    ];
    s.execute("view.saved.new", &json!({"name": "A", "center": [0, 0], "zoom": 1})).unwrap();
    let nested = [
        (s.execute("view.saved.edit", &json!({"edits": [{"name": "A"}, {"name": "A", "delete": 1}]})).unwrap_err().to_string(), "`edits[1].delete`"),
        (s.execute("view.saved.edit", &json!({"edits": [{"name": "A", "x": 1}]})).unwrap_err().to_string(), "`edits[0].x`"),
    ];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, kept, english, value) in &errors {
            let shown = message(lang, e);
            assert!(shown.contains(kept) && shown.contains(value) && !shown.contains(english), "{code}: {shown}");
        }
        for (e, kept) in &nested {
            let shown = message(lang, e);
            assert!(shown.contains(kept) && !shown.contains("unknown parameter") && !shown.contains("must be"), "{code}: {shown}");
        }
    }
}

/// The warning for a PDF/X file that leaves out the document's bleed is translated in every catalog,
/// alone and in the status bar's note.
#[test]
fn pdfx_bleed_warning_is_translated() {
    let w = vectorcraft_engine::cmd::fileio::pdf::BLEED_LEFT_OUT;
    let status = format!("Saved /tmp/a.pdf with 1 note: {w}");
    // A catalog's `{a}: {b}` row can translate the colon alone, so the check looks for the English words.
    for lang in Lang::all().filter(|l| l.code() != Lang::EN.code()) {
        for shown in [message(lang, w), message(lang, &status)] {
            assert!(!shown.contains("leaves out") && !shown.contains("turn on"), "{}: {shown}", lang.code());
        }
    }
}

/// The errors of saving, renaming and deleting a tracing preset are translated in every language
/// that translates every message.
#[test]
fn saved_tracing_preset_errors_are_translated() {
    use serde_json::json;
    use vectorcraft_engine::{EngineError, Session};
    let mut s = Session::new();
    s.execute("imageTrace.presets.save", &json!({"name": "Mine"})).unwrap();
    s.execute("imageTrace.presets.save", &json!({"name": "Other"})).unwrap();
    let mut errors = vec![];
    for (cmd, p, want) in [
        ("imageTrace.presets.save", json!({"name": ""}), "`name` is empty"),
        ("imageTrace.presets.save", json!({"name": "Mine", "newName": ""}), "`newName` is empty"),
        ("imageTrace.presets.save", json!({"name": "Other", "newName": "Mine"}), "a preset named `Mine` exists"),
        ("imageTrace.presets.save", json!({"name": "Default"}), "`Default` is a built-in preset and can't change: save it under another name"),
        ("imageTrace.presets.delete", json!({"name": "Default"}), "`Default` is a built-in preset and stays"),
        ("imageTrace.presets.delete", json!({"name": "Gone"}), "no saved preset named `Gone`"),
    ] {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{cmd} {p}: {e}");
        errors.push((e.to_string(), want));
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, want) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(want), "{code}: {shown}");
        }
    }
}

/// The notes on names in a `.ai`'s editing data that aren't UTF-8 are translated in every language
/// that translates every message, keeping the name, the encoding and the param.
#[test]
fn legacy_name_notes_are_translated() {
    let notes = [
        "the name `·¹ÀÌ¾î 1` isn't UTF-8 and was read as windows-1252: if it looks wrong, open the file with another legacyNameEncoding",
        "the name `레\u{fffd} 1` isn't UTF-8, and parts of it aren't valid windows-949 either: open the file with another legacyNameEncoding",
        "more names that aren't UTF-8 were read as macintosh (7)",
    ];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for note in notes {
            let shown = message(lang, note);
            assert_ne!(shown, note, "{code}");
            assert!(shown.contains("UTF-8"), "{code}: {shown}");
        }
        let shown = message(lang, notes[0]);
        assert!(shown.contains("`·¹ÀÌ¾î 1`") && shown.contains("windows-1252") && shown.contains("legacyNameEncoding"), "{code}: {shown}");
        assert!(message(lang, notes[2]).contains("macintosh") && message(lang, notes[2]).contains('7'), "{code}");
    }
}

/// Export Data Sets' refusals of more data sets or variables than Import Data Sets reads are
/// translated in every language that translates every message.
#[test]
fn dataset_export_limit_errors_are_translated() {
    use serde_json::json;
    use vectorcraft_engine::{EngineError, Session};
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let rows = "x\n".repeat(1000);
    s.execute("dataset.import", &json!({"csv": format!("V1\n{rows}")})).unwrap();
    s.execute("dataset.new", &json!({"name": "Set 1001"})).unwrap();
    let mut errors = vec![];
    let e = s.execute("dataset.export", &json!({})).unwrap_err();
    let want = "more than 1000 data sets: Import Data Sets reads at most 1000";
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{e}");
    errors.push((e.to_string(), want));
    s.execute("dataset.delete", &json!({"names": ["Set 1001"]})).unwrap();
    for i in 2..=257 {
        s.execute("variable.define", &json!({"name": format!("V{i}"), "kind": "text"})).unwrap();
    }
    let e = s.execute("dataset.export", &json!({})).unwrap_err();
    let want = "more than 256 variables: Import Data Sets reads at most 256";
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{e}");
    errors.push((e.to_string(), want));
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, want) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(want), "{code}: {shown}");
        }
    }
}

/// The status bar's line for an EPS file whose PostScript stops on an error, opened partly or
/// refused, and the refusal's dialog keep the error's name as PostScript spells it and its meaning,
/// which each language shows as its catalog translates the meaning.
#[test]
fn postscript_errors_keep_their_name_and_meaning() {
    let meaning = "a value has the wrong type for the operator";
    for (body, opens) in [("0 0 10 10 rectfill 1 (a) add", true), ("1 (a) add", false)] {
        let bytes = format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}\n%%EOF\n").into_bytes();
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.services.read = Some(Box::new(move |_: &str| Ok(bytes.clone())));
        assert_eq!(crate::io::open_reporting(&mut app, "/in/a.eps").is_ok(), opens, "{}", app.ui.status);
        let mut shown_texts = vec![app.ui.status.clone()];
        if !opens {
            shown_texts.push(app.ui.dialog.take().expect("a dialog says why").str("detail"));
        }
        for text in shown_texts {
            assert!(text.contains(&format!("(typecheck in `add`; {meaning})")), "{text}");
            for lang in Lang::all() {
                let shown = message(lang, &text);
                let meaning_shown = shown.contains(&message(lang, meaning)) || shown.contains(meaning);
                let ok = shown.contains("typecheck") && shown.contains("`add`") && meaning_shown;
                assert!(ok, "{}: {shown}", lang.code());
            }
        }
    }
}

/// Import Data Sets' errors for a refused cell are translated in every language that translates
/// every message, with the line and the variable kept.
#[test]
fn dataset_import_cell_errors_are_translated() {
    use serde_json::json;
    use vectorcraft_engine::{EngineError, Session};
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Sales", "kind": "graph"})).unwrap();
    let mut errors = vec![];
    for (csv, want) in [
        ("Show\nmaybe".to_string(), "line 2: value for `Show` must be true or false"),
        (format!("Sales\n\"{}\"", "1,".repeat(140_000)), "line 2: value for `Sales` is longer than 262144 bytes"),
    ] {
        let e = s.execute("dataset.import", &json!({"csv": csv})).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg == want), "{e}");
        errors.push((e.to_string(), want));
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, want) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(want) && shown.contains('2') && shown.contains('`'), "{code}: {shown}");
        }
    }
}

/// The errors of the parameter checks that many commands share (a number given as text, a missing
/// number or index, a fraction for a whole number, `ids` that aren't a list of object ids, an
/// `ids` entry or an `id` that is negative, an object id that names no object, a date that isn't
/// Unix seconds) fill their templates, and each language shows the value in the message, such as a
/// quoted `"60 mm"`, as given: in its row's translation, or in English where it has none.
#[test]
fn shared_param_check_errors_show_their_values_as_given() {
    use serde_json::json;
    use vectorcraft_engine::EngineError;
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    // A command, its params, the English template of its error and the template's values.
    type Case = (&'static str, serde_json::Value, &'static str, &'static [(&'static str, &'static str)]);
    let cases: [Case; 11] = [
        ("object.move", json!({"dx": "60 mm"}), "`{key}` must be a number, not {v}", &[("key", "dx"), ("v", r#""60 mm""#)]),
        ("object.rotate", json!({"angle": true}), "`{key}` must be a number, not {v}", &[("key", "angle"), ("v", "true")]),
        ("object.setBounds", json!({"width": [1, 2]}), "`{key}` must be a number, not {v}", &[("key", "width"), ("v", "[1,2]")]),
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 10}), "missing number `{key}`", &[("key", "height")]),
        ("appearance.setItem", json!({}), "missing integer `{key}`", &[("key", "index")]),
        ("object.transformEach", json!({"reference": 2.5}), "`{key}` must be a whole number", &[("key", "reference")]),
        ("edit.clear", json!({"ids": "all"}), "{key} must be an array of non-negative integer object ids", &[("key", "ids")]),
        (
            "edit.clear",
            json!({"ids": [-1]}),
            "`{key}` entry {index} is {value}, not a non-negative integer object id",
            &[("key", "ids"), ("index", "0"), ("value", "-1")],
        ),
        ("paint.invert", json!({"id": -1}), "invalid object id {value}: expected a non-negative integer", &[("value", "-1")]),
        ("edit.clear", json!({"ids": [99999]}), "no such object id {value}", &[("value", "99999")]),
        ("file.new", json!({"created": "yesterday"}), "{key} must be Unix seconds (an integer) or null", &[("key", "created")]),
    ];
    for (cmd, p, template, values) in cases {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { msg, .. } if *msg == fmt(template, values)), "{cmd} {p}: {e}");
        for lang in Lang::all() {
            let (entries, _) = parse_entries(lang.0.source);
            let row = entries.iter().find(|(ctx, src, _)| ctx == "@msg" && src == template);
            let shown = message(lang, &e.to_string());
            let as_given = shown.contains(&fmt(template, values)) || row.is_some_and(|(_, _, tr)| shown.contains(&fmt(tr, values)));
            assert!(as_given, "{}: {shown}", lang.code());
        }
    }
}

/// The spans of `s` between pairs of backticks, such as a parameter key or a command id; none when
/// its backticks don't pair up.
fn backticked(s: &str) -> Vec<&str> {
    let parts: Vec<&str> = s.split('`').collect();
    if parts.len().is_multiple_of(2) {
        return Vec::new();
    }
    parts.into_iter().skip(1).step_by(2).filter(|t| !t.is_empty()).collect()
}

#[test]
fn backticked_finds_the_spans_between_paired_backticks() {
    assert_eq!(backticked("`ids` must be an array, see `plugin.list`"), ["ids", "plugin.list"]);
    assert_eq!(backticked("no backticks"), Vec::<&str>::new());
    assert_eq!(backticked("a lone ` backtick"), Vec::<&str>::new());
}

/// A translated row keeps each backticked span of its English text, in backticks and as the
/// English spells it: the span names a parameter key, a command id or a placeholder, which the
/// command reads as written.
#[test]
fn translations_keep_the_backticked_names_of_their_english() {
    let mut missing = Vec::new();
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in &entries {
            let plural = ctx == "@plural" || ctx == "@msgplural";
            let english = if plural { src.rsplit('|').next().unwrap_or(src) } else { src.as_str() };
            let forms: Vec<&str> = if plural { tr.split('|').collect() } else { vec![tr.as_str()] };
            for form in forms {
                for token in backticked(english) {
                    if !form.contains(&format!("`{token}`")) {
                        missing.push(format!("{}: `{token}` in {src:?} -> {form:?}", l.code));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} rows drop a backticked name:\n{}", missing.len(), missing.join("\n"));
}

/// The spans of `s` between pairs of straight double quotes outside its backticked spans, such as
/// a value a parameter takes; none when those quotes don't pair up.
fn double_quoted(s: &str) -> Vec<&str> {
    let outside: Vec<&str> = s.split('`').step_by(2).collect();
    let quotes: usize = outside.iter().map(|part| part.matches('"').count()).sum();
    if !quotes.is_multiple_of(2) {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut open = false;
    for part in outside {
        for (i, piece) in part.split('"').enumerate() {
            if i > 0 {
                open = !open;
            }
            if open && !piece.is_empty() {
                spans.push(piece);
            }
        }
    }
    spans
}

/// Whether `text` holds `word` with no letter or digit right before or after it.
fn holds_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[at + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

#[test]
fn double_quoted_finds_the_spans_between_paired_quotes() {
    assert_eq!(double_quoted(r#"span must be "object" or "selection", got {v}"#), ["object", "selection"]);
    assert_eq!(double_quoted(r#"`{"a": 1}` is "left""#), ["left"]);
    assert_eq!(double_quoted(r#"a lone " quote"#), Vec::<&str>::new());
    assert!(holds_word("“auto” veya “optical”", "auto"));
    assert!(!holds_word(r#""content""#, "contents"));
    assert!(!holds_word(r#""none""#, "one"));
}

/// A translated status or error message keeps each value its English text puts in double quotes,
/// spelled as the English spells it, since the value is what a command reads; the quote marks may
/// be the language's own. Menu and control labels quote names of the interface, which translate.
#[test]
fn translated_messages_keep_the_double_quoted_values_of_their_english() {
    let mut missing = Vec::new();
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg" || ctx == "@msgplural") {
            let plural = ctx == "@msgplural";
            let english = if plural { src.rsplit('|').next().unwrap_or(src) } else { src.as_str() };
            let forms: Vec<&str> = if plural { tr.split('|').collect() } else { vec![tr.as_str()] };
            for form in forms {
                for value in double_quoted(english) {
                    if !holds_word(form, value) {
                        missing.push(format!("{}: \"{value}\" in {src:?} -> {form:?}", l.code));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} rows drop a double-quoted value:\n{}", missing.len(), missing.join("\n"));
}

/// Whether `text` holds `word` with no ASCII letter, digit or underscore right before or after it,
/// so `true` counts in `true이거나` but not in `untrue`.
fn holds_ascii_word(text: &str, word: &str) -> bool {
    let joins = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !joins(c));
        let after = text[at + word.len()..].chars().next().is_none_or(|c| !joins(c));
        before && after
    })
}

/// The literal values `true`, `false` and `null` that `s` names outside its backticked and
/// double-quoted spans.
fn bare_literals(s: &str) -> Vec<&'static str> {
    let mut outside = String::new();
    let mut quoted = false;
    for part in s.split('`').step_by(2) {
        for (i, piece) in part.split('"').enumerate() {
            if i > 0 {
                quoted = !quoted;
            }
            if !quoted {
                outside.push_str(piece);
                outside.push(' ');
            }
        }
    }
    ["true", "false", "null"].into_iter().filter(|lit| holds_ascii_word(&outside, lit)).collect()
}

#[test]
fn bare_literals_finds_true_false_and_null_outside_backticks_and_quotes() {
    assert_eq!(bare_literals("clip must be true or false"), ["true", "false"]);
    assert_eq!(bare_literals("`leading` must be from 0.1 to 5000, or null for Auto"), ["null"]);
    assert_eq!(bare_literals(r#"`true` is "false", not untrue"#), Vec::<&str>::new());
    assert!(holds_ascii_word("true이거나", "true"));
    assert!(!holds_ascii_word("isTrue or falsey", "false"));
}

/// A translated status or error message keeps each `true`, `false` and `null` its English text
/// names without backticks, spelled as the English spells it, since that is the value a command
/// reads.
#[test]
fn translated_messages_keep_the_bare_literals_of_their_english() {
    let mut missing = Vec::new();
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg" || ctx == "@msgplural") {
            let plural = ctx == "@msgplural";
            let english = if plural { src.rsplit('|').next().unwrap_or(src) } else { src.as_str() };
            let forms: Vec<&str> = if plural { tr.split('|').collect() } else { vec![tr.as_str()] };
            for form in forms {
                for lit in bare_literals(english) {
                    if !holds_ascii_word(form, lit) {
                        missing.push(format!("{}: {lit} in {src:?} -> {form:?}", l.code));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} rows drop a bare true, false or null:\n{}", missing.len(), missing.join("\n"));
}

/// Messages that name parameter keys and values without backticks, each with the names a
/// translation keeps as written in the English.
const NAMED_KEYS: &[(&str, &[&str])] = &[
    ("a DXF drawing has one origin: pass useArtboards: true for one drawing per artboard", &["useArtboards"]),
    ("a name names one asset: give multiple: false, or one object", &["multiple"]),
    ("a theme is already called {name} (replace: \"{name}\" overwrites it)", &["replace"]),
    ("action is required", &["action"]),
    ("anchor must name a registered panel", &["anchor"]),
    ("antiAlias `{id}`: none, art or type", &["antiAlias", "none", "art", "type"]),
    ("axis must be \"vertical\", \"horizontal\" or an angle in degrees, not `{v}`", &["axis"]),
    ("balance must be between 0 and 100", &["balance"]),
    ("before must name a panel", &["before"]),
    ("brightness must be dark|mediumDark|mediumLight|light", &["brightness", "dark", "mediumDark", "mediumLight", "light"]),
    ("clip must be true or false", &["clip"]),
    ("collapsed must be true or false", &["collapsed"]),
    ("column must be true or a number from 1", &["column"]),
    ("column: true floats the whole column and takes no onto, above or below", &["column", "onto", "above", "below"]),
    ("control must be fillStroke, coloring, drawMode or screenMode", &["control", "fillStroke", "coloring", "drawMode", "screenMode"]),
    ("decimals must be {_1}–{_2} (got {_3})", &["decimals"]),
    ("detect must be true or false", &["detect"]),
    ("give origin: [x, y], or reset: true", &["origin", "reset"]),
    ("global must be true or false", &["global"]),
    ("group must be true or false", &["group"]),
    ("icon must be true or false", &["icon"]),
    ("included must be true or false", &["included"]),
    ("index must be a number or first, previous, next or last", &["index", "first", "previous", "next", "last"]),
    ("index must be a number or first, previous, next or last, not `{other}`", &["index", "first", "previous", "next", "last"]),
    ("index must be ≥ 0", &["index"]),
    ("kerning must be a number, \"auto\", or \"optical\"", &["kerning"]),
    ("lang must be one of {_1}", &["lang"]),
    ("layers must be true or false", &["layers"]),
    ("member is for order front, forward, backward or back", &["member", "order", "front", "forward", "backward", "back"]),
    ("mode is set, add or toggle, not `{other}`", &["mode", "set", "add", "toggle"]),
    ("names is empty", &["names"]),
    ("on must be true or false", &["on"]),
    ("onto must name a floating panel", &["onto"]),
    ("others must be true or false", &["others"]),
    ("page must be a whole number from 1", &["page"]),
    ("panel is not in a dock group", &["panel"]),
    ("panel must name a registered panel", &["panel"]),
    ("paths must be strings", &["paths"]),
    ("percent must be above 0 and at most 6400, not {p}", &["percent"]),
    ("plane must be left, right or ground", &["plane", "left", "right", "ground"]),
    ("precision must be from 0.001 to 10 pt", &["precision"]),
    ("rect is [x, y, width, height] in points", &["rect"]),
    ("scale must be a number from {_1} to {_2}, not {_3}", &["scale"]),
    ("scale must be a number from {_1} to {_2}, not {s}", &["scale"]),
    ("show must be true or false", &["show"]),
    ("span: \"selection\" places a linear or radial gradient, not a freeform one", &["span"]),
    ("thickness must be from 0.01 to 5000 pt", &["thickness"]),
    ("title can't be empty", &["title"]),
    ("view is required", &["view"]),
    ("width must be positive", &["width"]),
    ("zone must be a string", &["zone"]),
    ("zone must be tab, center, left, right, top or bottom", &["zone", "tab", "center", "left", "right", "top", "bottom"]),
    ("`dither` applies to gradient fills (stroke: false)", &["stroke"]),
    ("`strokeMode` applies to strokes (stroke: true)", &["stroke"]),
];

/// Every catalog's translation of a [`NAMED_KEYS`] message keeps each name the table lists, as
/// a whole word, and every message in the table is still in a catalog.
#[test]
fn translated_messages_keep_the_keys_and_values_their_english_names() {
    let mut missing = Vec::new();
    let mut seen = vec![false; NAMED_KEYS.len()];
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (_, src, tr) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg") {
            let Some(at) = NAMED_KEYS.iter().position(|(english, _)| english == src) else { continue };
            seen[at] = true;
            for name in NAMED_KEYS[at].1 {
                if !holds_ascii_word(tr, name) {
                    missing.push(format!("{}: {name} in {src:?} -> {tr:?}", l.code));
                }
            }
        }
    }
    let stale: Vec<&str> = NAMED_KEYS.iter().zip(&seen).filter(|(_, seen)| !**seen).map(|((english, _), _)| *english).collect();
    assert!(stale.is_empty(), "no catalog translates {stale:?}");
    assert!(missing.is_empty(), "{} rows drop a key or value their English names:\n{}", missing.len(), missing.join("\n"));
}

/// Languages whose catalogs translate status and error messages; [`untranslated_report`] lists
/// the messages each one lacks.
const COMPLETE_MESSAGES: &[&str] = &[
    "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr", "ca",
];

/// Crates whose error and status messages reach the status bar.
const MESSAGE_CRATES: &[&str] = &["ui-egui", "engine", "doc", "format", "svg", "pdf", "eps", "text", "plugins", "metafile", "cad", "trace"];

/// Where a message literal starts: a status, an error value, a `thiserror` message.
const MESSAGE_MARKERS: &[&str] = &[".status(", "ui.status = ", "status = ", "Other(", "Err(", "ok_or(", "ok_or_else(|| ", "#[error("];

/// Helpers that make an error from a message, by crate, with the number of arguments before the
/// message: the engine's `bad(cmd, msg)` (`EngineError::BadParams`), `err(msg)` (`cmd/fileio`)
/// and `error(msg)` (`cmd/dimension.rs`), and the PDF writer's `bad(why)` (`forms.rs`) and
/// `failed(why)` (`encrypt.rs`, `linearize.rs`).
const ERROR_HELPERS: &[(&str, &str, usize)] =
    &[("engine", "bad(", 1), ("engine", "err(", 0), ("engine", "error(", 0), ("pdf", "bad(", 0), ("pdf", "failed(", 0)];

/// Error variants whose first argument is a name, not a message: a command id
/// (`EngineError::UnknownCommand`) and a PostScript error name (`PsError::Ps`).
const NAME_VARIANTS: &[&str] = &["UnknownCommand", "Ps"];

/// Error variants whose message follows other arguments, with the number of arguments before it:
/// `EngineError::Disabled(command id, reason)`.
const LATER_MESSAGE_VARIANTS: &[(&str, usize)] = &[("Disabled", 1)];

/// Messages that are signals, not text for people.
const NOT_MESSAGES: &[&str] = &["quit", "cancelled"];

/// Every status and error message literal in [`MESSAGE_CRATES`] (test modules aside), as catalog
/// templates: `{}` becomes `{_1}`, `{_2}` … and format specs are dropped (`{n:?}` → `{n}`).
pub fn message_literals() -> std::collections::BTreeSet<String> {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = std::collections::BTreeSet::new();
    let mut stack: Vec<_> = MESSAGE_CRATES.iter().map(|c| crates.join(c).join("src")).collect();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.starts_with("tests") && !(d == crates.join("ui-egui").join("src") && name == "control.rs") {
                // Control replies are English protocol data, never status-bar messages
                // (see i18n's module contract). Engine messages are scanned in their own crates.
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let helpers: Vec<_> =
                    ERROR_HELPERS.iter().filter(|(c, _, _)| path.starts_with(crates.join(c))).map(|&(_, call, before)| (call, before)).collect();
                out.extend(source_messages(without_test_module(&text), &helpers));
            }
        }
    }
    out
}

/// The message literals in `code` outside its comments, as catalog templates (see
/// [`message_literals`]): the literal after each of [`MESSAGE_MARKERS`], the message argument of an
/// error enum's variant (the first, as in `PsError::Limit(…)` and `Error::Manifest(…)`, or the one
/// [`LATER_MESSAGE_VARIANTS`] gives; not [`NAME_VARIANTS`]),
/// and the message argument of each call in `helpers` (a call and the number of arguments before
/// its message), a literal or a `format!` string.
fn source_messages(code: &str, helpers: &[(&str, usize)]) -> Vec<String> {
    let code = &without_comments(code);
    let after_markers = MESSAGE_MARKERS.iter().flat_map(|marker| code.match_indices(marker).map(|(at, _)| &code[at + marker.len()..]));
    let in_variants = code.match_indices("Error::").filter_map(|(at, m)| {
        let rest = &code[at + m.len()..];
        let (variant, args) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len()));
        let named = variant.starts_with(|c: char| c.is_ascii_uppercase()) && !NAME_VARIANTS.contains(&variant);
        let before = LATER_MESSAGE_VARIANTS.iter().find(|(v, _)| *v == variant).map_or(0, |&(_, n)| n);
        if named { skip_args(args.strip_prefix('(')?, before) } else { None }
    });
    let in_helpers = helpers.iter().flat_map(|&(call, before)| {
        code.match_indices(call)
            .filter(|&(at, _)| !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.'))
            .filter_map(move |(at, _)| skip_args(&code[at + call.len()..], before))
    });
    let mut out = vec![];
    for rest in after_markers.chain(in_variants).chain(in_helpers) {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("format!(").map_or(rest, str::trim_start);
        let Some(lit) = rest.strip_prefix('"').and_then(string_literal) else { continue };
        let has_word = lit.as_bytes().windows(3).any(|w| w.iter().all(u8::is_ascii_lowercase));
        if has_word && !NOT_MESSAGES.contains(&lit.as_str()) {
            out.push(as_template(&lit));
        }
    }
    out
}

/// The arguments of a call whose opening parenthesis was just read, past the first `n` of them;
/// `None` when `n` is 1 or more and the call has `n` arguments or fewer.
fn skip_args(args: &str, n: usize) -> Option<&str> {
    if n == 0 {
        return Some(args);
    }
    let (mut left, mut depth, mut chars) = (n, 0usize, args.char_indices());
    while let Some((at, c)) = chars.next() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            '"' => loop {
                match chars.next()?.1 {
                    '\\' => {
                        chars.next()?;
                    }
                    '"' => break,
                    _ => {}
                }
            },
            ',' if depth == 0 => {
                left -= 1;
                if left == 0 {
                    return Some(&args[at + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Source up to its inline test module (`#[cfg(test)] mod tests {`); declarations of test files
/// (`#[cfg(test)] mod tests_x;`) don't end it.
fn without_test_module(text: &str) -> &str {
    for (at, m) in text.match_indices("#[cfg(test)]") {
        let after = text[at + m.len()..].trim_start();
        if let Some(rest) = after.strip_prefix("mod ")
            && rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').trim_start().starts_with('{')
        {
            return &text[..at];
        }
    }
    text
}

/// The text of a string literal whose opening quote was just read, escapes resolved.
fn string_literal(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut it = s.chars();
    loop {
        match it.next()? {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}

/// A format string as a catalog template (see [`message_literals`]).
fn as_template(s: &str) -> String {
    let mut out = String::new();
    let mut unnamed = 0;
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        let Some(b) = after.find('}') else { break };
        let name = after[..b].split(':').next().unwrap_or("");
        if name.is_empty() {
            unnamed += 1;
            out.push_str(&format!("{{_{unnamed}}}"));
        } else {
            out.push_str(&format!("{{{name}}}"));
        }
        rest = &after[b + 1..];
    }
    out.push_str(rest);
    out
}

/// Render-site labels looked up in a context fall back to the plain row and then to English, and
/// their templates fill in English as in any language.
#[test]
fn render_site_labels_fall_back_to_english() {
    for (_, source) in RENDER_SITE_ROWS.iter().filter(|(ctx, _)| *ctx == "file-filter") {
        assert_eq!(tr_ctx(Lang::EN, "file-filter", source), *source);
    }
    for source in ["SVG", "PNG", "VectorCraft", "unregistered filter"] {
        assert_eq!(tr_ctx(cs(), "file-filter", source), tr(cs(), source));
    }
    assert_eq!(fmt(tr(Lang::EN, "Preset {number}"), &[("number", "12")]), "Preset 12");
    for kind in ["Linear", "Radial", "Freeform"] {
        assert_eq!(fmt(tr(Lang::EN, "{kind} Gradient"), &[("kind", tr_ctx(Lang::EN, "gradient-kind", kind))]), format!("{kind} Gradient"));
    }
    assert_eq!(tr_ctx(Lang::EN, "mark-style", "Japanese"), "Japanese");
    for n in [0, 1, 2, 5] {
        let commits = if n == 1 { "commit" } else { "commits" };
        assert_eq!(trn(Lang::EN, n, "{n} commit", "{n} commits"), format!("{n} {commits}"));
    }
}

/// (context, English source) of the labels [`untranslated_report`] lists for a language without
/// their rows: the file dialogs' filter names, the status bar's timings, New Document's preset
/// name, the contributor tooltips, the gradient kind in a swatch's description, the printer mark
/// styles and the label of the 3D gizmo's light.
const RENDER_SITE_ROWS: &[(&str, &str)] = &[
    ("file-filter", "All readable files"),
    ("file-filter", "All placeable files"),
    ("file-filter", "Swatch libraries"),
    ("file-filter", "Flattener presets"),
    ("file-filter", "PDF presets"),
    ("file-filter", "Print presets"),
    ("file-filter", "Export presets"),
    ("file-filter", "SVG Compressed"),
    ("file-filter", "PDF-compatible .ai"),
    ("file-filter", "VectorCraft Template"),
    ("file-filter", "Templates"),
    ("anchor", "Points:"),
    ("diagnostics", "render"),
    ("diagnostics", "ui"),
    ("", "Preset {number}"),
    ("", "PRs"),
    ("", "@{login}: {prs}, {commits}, +{added} / −{deleted} lines (Δ {delta}), +{binary_added} / −{binary_deleted} binary assets, {first} – {last}"),
    ("@plural", "{n} PR|{n} PRs"),
    ("@plural", "{n} commit|{n} commits"),
    ("gradient-kind", "Radial"),
    ("mark-style", "Japanese"),
    ("", "Roman"),
    ("lighting", "Light"),
];

fn pl() -> Lang {
    Lang::from_code("pl").expect("pl registered")
}

#[test]
fn polish_plurals_have_three_forms() {
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 112, 122].into_iter().map(plural_polish).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2, 2, 2, 1, 2, 2, 2, 1]);
    // Every Polish plural row has the three forms, and each count picks its own.
    let (entries, _) = parse_entries(pl().0.source);
    let mut count = 0;
    for (ctx, src, tr) in &entries {
        if ctx != "@plural" {
            continue;
        }
        count += 1;
        let forms: Vec<&str> = tr.split('|').collect();
        assert_eq!(forms.len(), 3, "{src}");
        let (one, other) = src.split_once('|').unwrap();
        for (n, form) in [(1, 0), (3, 1), (5, 2), (21, 2), (22, 1)] {
            assert_eq!(trn(pl(), n, one, other), forms[form].replace("{n}", &n.to_string()), "{src}: {n}");
        }
    }
    assert!(count >= 13, "Polish plural rows missing");
}

/// Every catalog names each numeral system of Character panel menu › Digits, in the `digits`
/// context (where the language's name would read wrong) or with its plain row (#4942).
#[test]
fn every_catalog_names_the_digits_menus_numeral_systems() {
    let mut missing = Vec::new();
    for l in LANGUAGES.iter().filter(|l| !l.source.is_empty()) {
        let cat = l.catalog();
        for (label, _) in crate::panels::character::DIGITS {
            if cat.contextual("digits", label).is_none() && cat.plain(label).is_none() {
                missing.push(format!("{}: {label}", l.code));
            }
        }
    }
    assert!(missing.is_empty(), "Digits menu labels without a row:\n{}", missing.join("\n"));
}

/// Source-linked Hebrew labels for real Native Save Changes, Export As, and Import PDF dialogs.
#[test]
fn hebrew_native_save_export_import_dialogs_are_localized() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    for (filename, labels) in [
        (
            "../dialogs/save_changes.rs",
            &["Do you want to save the changes you made to “{name}”?", "Your changes will be lost if you don't save them."][..],
        ),
        (
            "../dialogs/export_as.rs",
            &[
                "Export As",
                "Use Artboards",
                "The plug-in writes the whole document.",
                "One page per artboard.",
                "One file per artboard: <name>-<artboard>.{ext}",
                "The bounds of the visible art.",
            ][..],
        ),
        (
            "../dialogs/import_pdf.rs",
            &[
                "Place PDF",
                "Import PDF",
                "“{file}” is protected by a password.",
                "First Page",
                "Previous Page",
                "of {count}",
                "Next Page",
                "Last Page",
                "Crop To",
                "Pages",
                "e.g. 1-3, 5",
                "Password",
            ][..],
        ),
    ] {
        let source = match filename {
            "../dialogs/save_changes.rs" => include_str!("../dialogs/save_changes.rs"),
            "../dialogs/export_as.rs" => include_str!("../dialogs/export_as.rs"),
            _ => include_str!("../dialogs/import_pdf.rs"),
        };
        for english in labels {
            assert!(source.contains(&format!(r#"tl!("{english}")"#)), "{filename}: missing source key {english}");
            assert_ne!(tr(he, english), *english, "untranslated: {english}");
            assert_eq!(tr(Lang::EN, english), *english, "English changed: {english}");
        }
    }
    for english in ["Don't Save", "Export…"] {
        let source = if english == "Don't Save" { include_str!("../dialogs/save_changes.rs") } else { include_str!("../dialogs/export_as.rs") };
        assert!(source.contains(&format!(r#"Some("{english}")"#)));
        assert_ne!(tr(he, english), english);
    }
    for key in [
        "Do you want to save the changes you made to “{name}”?",
        "One file per artboard: <name>-<artboard>.{ext}",
        "“{file}” is protected by a password.",
        "of {count}",
    ] {
        assert_eq!(placeholders(key), placeholders(tr(he, key)));
    }
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(rows.len() >= 201, "new Hebrew catalog rows absent");
}

/// The vector editing UI must show native Hebrew for common artboard, path, layer,
/// fill, stroke and clipping controls, while preserving source command identifiers.
#[test]
fn hebrew_vector_artboard_path_and_color_controls_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "invalid Hebrew VectorCraft TSV: {errors:?}");
    assert_eq!(rows.len(), 681, "source-bound Hebrew catalog row count changed");
    for key in [
        "Artboard Tool",
        "Artboard Options…",
        "Anchor Point",
        "Anchor Points",
        "Delete Anchor Point",
        "Direct Selection Tool",
        "Clipping Mask",
        "Clipping Masks",
        "Compound Path",
        "Convert To Outlines",
        "Brush Libraries",
        "Color Picker",
        "Color Management",
        "Crop Image",
        "Delete Layer",
        "Duplicate Artboard",
        "Ellipse Tool",
        "Fill & Stroke",
        "Free Transform Tool",
        "Gradient Tool",
        "Group Selection Tool",
        "Hide Guides",
        "Make Live Paint",
    ] {
        let translated = tr(he, key);
        assert_ne!(translated, key, "untranslated vector control: {key:?}");
        assert!(translated.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c)), "{key:?}: no Hebrew letters");
        assert_eq!(tr(Lang::EN, key), key, "source command text must remain English");
    }
    assert_eq!(tr(he, "Clipping Mask"), "מסכת חיתוך");
    assert_eq!(tr(he, "Artboard Tool"), "כלי משטח יצירה");
    assert_eq!(tr(he, "Color Picker"), "בורר צבע");
    assert_eq!(tr(he, "Untranslated native path sentinel"), "Untranslated native path sentinel");
}

/// Font, character/paragraph and alignment panels are visible in Hebrew;
/// original command IDs, file names and English tool names are untouched.
#[test]
fn hebrew_typography_font_path_and_alignment_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rows.len(), 917, "current source catalog count");
    for source in [
        "Font",
        "Font Family",
        "Font Size",
        "Fonts in Document",
        "Glyphs",
        "Kerning",
        "Leading",
        "Tracking",
        "Character Style",
        "Paragraph Styles",
        "Type Tool",
        "Type on a Path",
        "Type on a Path Options…",
        "Align to Artboard",
        "Align to Key Object",
        "Show Hidden Characters",
        "Text Wrap Options…",
        "Stroke Weight",
        "Snap to Grid",
        "Pathfinder",
        "Layer Options…",
        "Show Rulers",
    ] {
        let translated = tr(he, source);
        assert_ne!(translated, source, "untranslated native source key {source:?}");
        assert!(translated.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c)));
        assert_eq!(tr(Lang::EN, source), source, "English source changed");
    }
    assert_eq!(tr(he, "Font Size"), "גודל גופן");
    assert_eq!(tr(he, "Type on a Path"), "טקסט על נתיב");
    assert_eq!(tr(he, "Kerning"), "צימוד אותיות");
    assert_eq!(tr(he, "Untranslated customer font name"), "Untranslated customer font name");
}

/// File/package/export and print dialogs must expose Hebrew source labels
/// without modifying file paths, user data, English commands or plural rules.
#[test]
fn hebrew_document_export_print_and_missing_states_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rows.len(), 1209, "Hebrew source catalog record count");
    for source in [
        "Document Setup",
        "Export Selection",
        "Export PDF",
        "Export for Screens",
        "Export Assets",
        "Open Document",
        "Linked File Not Found",
        "No document open",
        "No fonts are available.",
        "Missing Fonts",
        "Package",
        "Package created",
        "Print Preview",
        "Print Presets…",
        "Printer Setup…",
        "Save Document",
        "Save PDF Preset",
        "Save Recovery Data",
        "Open Location after Export",
        "Embed ICC Profile",
        "New Layer…",
        "Replace Font",
    ] {
        let translated = tr(he, source);
        assert_ne!(translated, source, "native source UI still English: {source:?}");
        assert!(translated.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c)));
        assert_eq!(tr(Lang::EN, source), source, "source English changed");
    }
    assert_eq!(tr(he, "Linked File Not Found"), "לא נמצא הקובץ המקושר");
    assert_eq!(tr(he, "Print Preview"), "תצוגה מקדימה להדפסה");
    assert_eq!(tr(he, "Package created"), "האריזה נוצרה");
    assert_eq!(tr(he, "User library file name"), "User library file name");
}

/// Shape, transform, opacity and swatch controls must be presented in Hebrew
/// while preserving original command names, English fallback and user data.
#[test]
fn hebrew_shape_transform_and_color_panels_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rows.len(), 1386, "Hebrew catalog source records");
    for source in [
        "Arrange",
        "Blend Tool",
        "Blending Mode",
        "Curvature Tool",
        "Distribute Objects:",
        "Divide",
        "Envelope Distort",
        "Expand Appearance",
        "Intersect",
        "Merge Layers",
        "Pathfinders:",
        "Rotate View",
        "Scale Strokes & Effects",
        "Shape Builder",
        "Shear Tool",
        "Simplify…",
        "Subtract",
        "Transform Each…",
        "Transform Panel",
        "Ungroup",
        "Unite",
        "Color wheel",
        "Opacity Mask",
        "Recolor Artwork",
        "Swatches",
        "Transparency Grid",
    ] {
        let translated = tr(he, source);
        assert_ne!(translated, source, "missing Hebrew visible UI: {source:?}");
        assert!(translated.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c)));
        assert_eq!(tr(Lang::EN, source), source, "source English must be immutable");
    }
    assert_eq!(tr(he, "Shape Builder"), "בונה צורות");
    assert_eq!(tr(he, "Opacity Mask"), "מסכת אטימות");
    assert_eq!(tr(he, "Unite"), "איחוד");
    assert_eq!(tr(he, "User-authored shape name"), "User-authored shape name");
}

/// Image tracing, perspective, artboard and history operations are visible in
/// Hebrew without changing file paths, dynamic labels or source English IDs.
#[test]
fn hebrew_image_trace_perspective_and_artboard_controls_are_translated() {
    let he = Lang::from_code("he").expect("Hebrew locale registered");
    let (rows, errors) = parse_entries(he.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rows.len(), 1386, "current source-linked Hebrew rows");
    for source in [
        "Image Trace",
        "Image Trace Presets",
        "Pen Tool",
        "Pencil Tool",
        "Perspective Grid",
        "Perspective Grid Tool",
        "Perspective Grid Presets…",
        "Zoom to Selection",
        "Zoom with Mouse Wheel",
        "Select an image to trace",
        "Select by Link Status…",
        "Asset Export Settings",
        "Close Path",
        "History Options…",
        "New Document from State",
        "Lock Guides",
        "Show Rulers (⌘R)",
        "Undo {name}",
        "Redo {name}",
        "Artboard {n} of {total}",
        "Show {kind} Brushes",
        "Show Perspective Grid",
        "Document Health Check",
        "File SHA-256: {hash}",
        "Unlock Selected",
        "Hide VectorCraft",
        "Selection Tools",
    ] {
        let translated = tr(he, source);
        assert_ne!(translated, source, "untranslated native source label: {source:?}");
        assert!(translated.chars().any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c)));
        assert_eq!(tr(Lang::EN, source), source, "original English must stay unchanged");
    }
    assert_eq!(tr(he, "Image Trace"), "מעקב תמונה");
    assert_eq!(tr(he, "Pen Tool"), "כלי עט");
    assert_eq!(tr(he, "Perspective Grid"), "רשת פרספקטיבה");
    assert_eq!(tr(he, "Locked"), "נעול");
    assert_eq!(tr(he, "User canvas object name"), "User canvas object name");
}

#[test]
fn hebrew_parallel_round3_selection_values() {
    let he = Lang::from_code("he").expect("Hebrew locale available");
    for (source, expected) in [
        ("Align to Pixel Grid", "יישור לרשת הפיקסלים"),
        ("Clear Selection", "ניקוי הבחירה"),
        ("Deselect", "ביטול הבחירה"),
        ("Isolate Selected Object", "בידוד האובייקט שנבחר"),
        ("Isolate Selected Group", "בידוד הקבוצה שנבחרה"),
        ("Copy Selected Style", "העתקת הסגנון שנבחר"),
        ("Connect Selected End Points", "חיבור נקודות הקצה שנבחרו"),
        ("Flip Along Horizontal Axis", "היפוך לאורך הציר האופקי"),
        ("Flip Along Vertical Axis", "היפוך לאורך הציר האנכי"),
        ("Free Transform", "שינוי צורה חופשי"),
    ] {
        assert_eq!(
            tr(he, source),
            expected,
            "Hebrew UI regression for {source}"
        );
    }
}

#[test]
fn hebrew_parallel_round3_typography_values() {
    let he = Lang::from_code("he").expect("Hebrew locale available");
    for (source, expected) in [
        ("Add Fonts", "הוספת גופנים"),
        ("Baseline Shift", "הסטת קו בסיס"),
        ("Font Family:", "משפחת גופנים:"),
        ("Font Style:", "סגנון גופן:"),
        ("Find Font", "חיפוש גופן"),
        ("First Line Indent:", "כניסת שורה ראשונה:"),
        ("Break Text into Lines", "פיצול טקסט לשורות"),
        ("Break Text into Words", "פיצול טקסט למילים"),
        ("Character Color", "צבע תו"),
        (
            "Left-to-Right Paragraph Direction",
            "כיוון פסקה משמאל לימין",
        ),
    ] {
        assert_eq!(
            tr(he, source),
            expected,
            "Hebrew UI regression for {source}"
        );
    }
}

#[test]
fn hebrew_parallel_round3_files_values() {
    let he = Lang::from_code("he").expect("Hebrew locale available");
    for (source, expected) in [
        (
            "Export All Open Documents as PDF",
            "ייצוא כל המסמכים הפתוחים ל־PDF",
        ),
        (
            "Export All Open Documents as SVG",
            "ייצוא כל המסמכים הפתוחים ל־SVG",
        ),
        ("Go to Artboard", "מעבר למשטח עבודה"),
        (
            "Couldn't save recovery data: {e}",
            "לא ניתן לשמור נתוני שחזור: \u{2068}{e}\u{2069}",
        ),
        ("Cancel Export", "ביטול הייצוא"),
        ("No artboards match.", "לא נמצאו משטחי עבודה תואמים."),
        ("No importer plug-ins installed", "לא מותקנים תוספים לייבוא"),
        ("Next Document", "המסמך הבא"),
        ("Next Artboard", "משטח העבודה הבא"),
        ("Find Linked Files", "איתור קבצים מקושרים"),
    ] {
        assert_eq!(
            tr(he, source),
            expected,
            "Hebrew UI regression for {source}"
        );
    }
}

#[test]
fn hebrew_parallel_round5_view_values() {
    let he = Lang::from_code("he").expect("Hebrew locale available");
    for (source, expected) in [
        ("Hide Gradient Annotator", "הסתרת סימוני מעבר הצבע"),
        ("Hide Print Tiling", "הסתרת חלוקת ההדפסה לאריחים"),
        ("Hide Slices", "הסתרת פרוסות"),
        ("Hide Template", "הסתרת התבנית"),
        ("Hide Thumbnail", "הסתרת תמונה ממוזערת"),
        ("Hide Thumbnails", "הסתרת תמונות ממוזערות"),
        ("Hide Video Rulers", "הסתרת סרגלי וידאו"),
        ("More Align options", "אפשרויות יישור נוספות"),
        ("Reset Rotate View", "איפוס סיבוב התצוגה"),
        ("Rearrange All Artboards", "סידור מחדש של כל משטחי העבודה"),
    ] {
        assert_eq!(tr(he, source), expected, "Hebrew source mismatch: {source}");
    }
}

#[test]
fn hebrew_parallel_round5_color_values() {
    let he = Lang::from_code("he").expect("Hebrew locale available");
    for (source, expected) in [
        ("Apply Fill to Stroke", "החלת המילוי על קו המתאר"),
        ("Apply Stroke to Fill", "החלת קו המתאר על המילוי"),
        ("Apply Last Color", "החלת הצבע האחרון"),
        ("Apply Last Gradient", "החלת מעבר הצבע האחרון"),
        ("Enable Opacity Mask", "הפעלת מסכת אטימות"),
        ("Disable Opacity Mask", "השבתת מסכת אטימות"),
        ("Make Opacity Mask", "יצירת מסכת אטימות"),
        ("Copy Color Value (Hex)", "העתקת ערך צבע (HEX)"),
        ("Fill Tint", "גוון מילוי"),
        ("Auto Color", "צביעה אוטומטית"),
    ] {
        assert_eq!(tr(he, source), expected, "Hebrew source mismatch: {source}");
    }
}
