use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

const ZH: fn() -> Lang = || Lang::from_code("zh-hant").expect("zh-hant registered");

#[test]
fn gujarati_resolves_locales_and_takes_the_singular_for_zero_and_one() {
    // The catalog is present even when this web build cannot display it.
    let gu = Lang(LANGUAGES.iter().find(|l| l.code == "gu").expect("gu registered"));
    let selectable = gu.0.available().then_some(gu);
    assert_eq!(Lang::from_code("gu"), selectable);
    for tag in ["gu", "gu-IN", "gu_IN.UTF-8", "GU-in", "gu-Gujr-IN"] {
        assert_eq!(lang_from_tag(tag), selectable, "{tag}");
    }
    assert_eq!(gu.name(), "ગુજરાતી");
    assert_eq!([0, 1, 2, 21].map(gu.0.plural), [0, 0, 1, 1]);
    assert_eq!(tr(gu, "Save"), "સાચવો");
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

/// Every Hindi row uses the terms `docs/localization-hi.md` sets for what its English names:
/// सेव for saving, स्वॉच for a swatch, क्रॉप for cropping, रिज़ॉल्यूशन for a resolution and पाथ for
/// a path, never पाठ (a reading), and a number of points is a number (संख्या) in points (पॉइंट).
/// Rows also never use a word's unrelated sense: a ruler is not शासक (a sovereign), a text character
/// is not चरित्र (a personality), a vector point or a printer's mark is not अंक (a digit), fills are
/// not भरता (the verb), Desired is not देसी (local), Even is not शाम (evening), and so on.
#[test]
fn hindi_rows_use_the_glossary_term_for_what_their_english_names() {
    let hi = Lang::from_code("hi").expect("hi registered");
    let (entries, _) = parse_entries(hi.0.source);
    let mut wrong = Vec::new();
    for (_, src, translation) in &entries {
        let english = src.to_lowercase();
        let names = |words: &[&str]| english.split(|c: char| !c.is_ascii_alphabetic()).any(|w| words.contains(&w));
        let banned: &[(bool, &[&str])] = &[
            (names(&["save", "saves", "saved", "saving"]), &["बचा", "बचत", "सहेज"]),
            (names(&["swatch", "swatches"]), &["घड़ी"]),
            (names(&["crop"]), &["फसल"]),
            (names(&["resolution", "resolutions"]), &["संकल्प"]),
            (names(&["path", "paths"]), &["पथ"]),
            (true, &["पाठ", "कई अंक"]),
            (names(&["ruler", "rulers"]), &["शासक"]),
            (names(&["character", "characters"]), &["चरित्र"]),
            (names(&["point", "points", "mark", "marks"]), &["अंक"]),
            (names(&["fill", "fills"]), &["भरता"]),
            (names(&["desired"]), &["देसी", "इच्छा"]),
            (names(&["even"]), &["शाम"]),
            (names(&["force", "forced"]), &["सेना"]),
            (names(&["same"]), &["समीर"]),
            (names(&["thumbnail", "thumbnails"]), &["हिंदुस्तान"]),
            (names(&["contrast"]), &["न्यायालय"]),
            (names(&["function"]), &["समारोह"]),
            (names(&["kern"]), &["बर्न"]),
            (names(&["imageable"]), &["कल्पनाशील"]),
            (names(&["constrain"]), &["कंस्ट्रक्शन"]),
            (names(&["complement"]), &["संगम"]),
            (names(&["concave"]), &["अवतरण"]),
            (names(&["triad"]), &["त्रिद"]),
            (names(&["trapped", "traps"]), &["निशानेबाज़", "जाल"]),
            (names(&["alternate", "alternates"]), &["अल्टरनेटर"]),
            (names(&["highlight"]), &["उच्च प्रकाश"]),
        ];
        for &(applies, words) in banned {
            if let Some(word) = words.iter().find(|w| applies && translation.contains(**w)) {
                wrong.push(format!("{word} in {src:?} -> {translation:?}"));
            }
        }
        if english.contains("number of points") && !(translation.contains("संख्या") && translation.contains("पॉइंट")) {
            wrong.push(format!("no number in points in {src:?} -> {translation:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} Hindi rows use the wrong term:\n{}", wrong.len(), wrong.join("\n"));
}

/// Every Hindi row that names a variable says वेरिएबल, never चर, परिवर्तनीय or वैरिएबल, and joining
/// paths, points, type or lines is जोड़ें, never शामिल हों, which stays for joining the community.
#[test]
fn hindi_rows_use_one_term_for_a_variable_and_for_joining_paths() {
    let hi = Lang::from_code("hi").expect("hi registered");
    let (entries, _) = parse_entries(hi.0.source);
    let mut wrong = Vec::new();
    for (_, src, translation) in &entries {
        let english = src.to_lowercase();
        let words: Vec<&str> = english.split(|c: char| !c.is_ascii_alphabetic()).collect();
        let hindi: Vec<&str> = translation.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation() || "।…".contains(c)).collect();
        let names = |list: &[&str]| words.iter().any(|w| list.contains(w));
        let other_variable = hindi.iter().any(|w| ["चर", "चरों", "परिवर्तनीय"].contains(w)) || translation.contains("वैरिएबल");
        if names(&["variable", "variables"]) && (other_variable || !translation.contains("वेरिएबल")) {
            wrong.push(format!("variable in {src:?} -> {translation:?}"));
        }
        if names(&["join", "joining"]) && !names(&["discord"]) && translation.contains("शामिल") {
            wrong.push(format!("join in {src:?} -> {translation:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} Hindi rows use the wrong term:\n{}", wrong.len(), wrong.join("\n"));
    assert_eq!(tr(hi, "Variable"), "वेरिएबल");
    assert_eq!(tr(hi, "Load Variable Library…"), "वेरिएबल लाइब्रेरी लोड करें…");
    assert_eq!(tr(hi, "Join"), "जोड़ें");
    assert_eq!(tr(hi, "Close or Join"), "बंद करें या जोड़ें");
    assert_eq!(message(hi, "join needs open paths"), "जोड़ने के लिए खुले पाथ चाहिए");
    assert_eq!(message(hi, "Join: a subpath went missing"), "जोड़ते समय एक सबपाथ नहीं मिला");
    assert_eq!(tr(hi, "Join our Discord"), "हमारे डिसकॉर्ड में शामिल हों");
}

/// Hindi messages and labels keep the condition, quantity and clauses of their English.
#[test]
fn hindi_rows_keep_the_meaning_of_their_english() {
    let hi = Lang::from_code("hi").expect("hi registered");
    assert_eq!(message(hi, "`shift` must be a number of points"), "`shift` पॉइंट में एक संख्या होनी चाहिए");
    assert_eq!(
        message(hi, "`spacing` must be a number of points, 0 or more, or \"auto\""),
        "`spacing` पॉइंट में 0 या उससे बड़ी संख्या, या \"auto\" होनी चाहिए"
    );
    assert_eq!(tr(hi, "Check an artboard to save."), "सेव करने के लिए किसी आर्टबोर्ड पर चेक लगाएँ।");
    assert_eq!(tr(hi, "Save the applied preferences to a JSON file"), "लागू प्रेफ़रेंसेज़ को JSON फ़ाइल में सेव करें");
    assert_eq!(message(hi, "the script is not UTF-8 text"), "स्क्रिप्ट UTF-8 टेक्स्ट नहीं है");
    assert_eq!(message(hi, "only solid-colour swatches can be spot colours"), "केवल ठोस रंग वाले स्वॉच स्पॉट रंग हो सकते हैं");
    assert_eq!(message(hi, "select area type"), "एरिया टाइप सिलेक्ट करें");
    assert_eq!(message(hi, "12 MB of images, at most 10 MB"), "इमेज के 12 MB, अधिकतम 10 MB");
    let all =
        message(hi, "SVG keeps 1 of the document's 3 artboards: the file reopens with one. Turn on Preserve Editing Capabilities to keep them all.");
    assert!(all.contains("3 आर्टबोर्ड में से 1") && all.contains("सभी आर्टबोर्ड रखने के लिए"), "{all}");
    let left_out = message(hi, "the file was read up to an error (bad xref): what follows it was left out");
    assert!(left_out.contains("उसके बाद का हिस्सा छोड़ दिया गया"), "{left_out}");
    assert_eq!(message(hi, "unknown align `left`"), "अज्ञात संरेखण `left`");
    assert_eq!(tr(hi, "Delete Library Group"), "लाइब्रेरी ग्रुप हटाएँ");
    assert_eq!(message(hi, "not editing an opacity mask"), "ओपेसिटी मास्क संपादित नहीं किया जा रहा है");
    assert_eq!(
        message(hi, "a page thumbnail must be RGB pixels, 1 to 256 a side (got 0 × 4)"),
        "पेज थंबनेल RGB पिक्सल का होना चाहिए, हर तरफ़ 1 से 256 (मिला 0 × 4)"
    );
    assert_eq!(message(hi, "invalid angle"), "अमान्य कोण");
    assert_eq!(tr(hi, "Desired"), "वांछित");
    let range = "न्यूनतम, वांछित से अधिक नहीं होना चाहिए और वांछित, अधिकतम से अधिक नहीं होना चाहिए";
    assert_eq!(tr(hi, "Minimum must not exceed Desired, nor Desired Maximum."), format!("{range}।"));
    assert_eq!(message(hi, "`wordSpacing`: Minimum must not exceed Desired, nor Desired Maximum"), format!("`wordSpacing`: {range}"));
}

/// Hindi messages keep every `a|b|c` list of accepted values exactly as their English writes it.
#[test]
fn hindi_messages_keep_their_lists_of_accepted_values() {
    let hi = Lang::from_code("hi").expect("hi registered");
    let (entries, _) = parse_entries(hi.0.source);
    let mut wrong = Vec::new();
    for (_, src, translation) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg") {
        let lists = src.split(|c: char| !(c.is_ascii_alphabetic() || c == '|')).filter(|w| w.contains('|') && w.split('|').all(|v| !v.is_empty()));
        for list in lists.filter(|list| !translation.contains(*list)) {
            wrong.push(format!("{list} in {src:?} -> {translation:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} Hindi messages change a list of values:\n{}", wrong.len(), wrong.join("\n"));
}

/// Hindi rows leave no English connecting word in their sentences. Text in backticks, quotes,
/// braces and brackets is a name, value or placeholder and keeps its English.
#[test]
fn hindi_rows_keep_no_english_words_in_their_sentences() {
    const ENGLISH: [&str; 13] = ["or", "and", "the", "must", "give", "see", "use", "most", "is", "of", "be", "not", "an"];
    let hi = Lang::from_code("hi").expect("hi registered");
    let (entries, _) = parse_entries(hi.0.source);
    let mut wrong = Vec::new();
    for (_, src, translation) in &entries {
        let mut prose = String::new();
        let mut close = None;
        for c in translation.chars() {
            match (close, c) {
                (Some(end), _) if c == end => close = None,
                (Some(_), _) => {}
                (None, '`') => close = Some('`'),
                (None, '"') => close = Some('"'),
                (None, '“') => close = Some('”'),
                (None, '{') => close = Some('}'),
                (None, '[') => close = Some(']'),
                (None, _) => prose.push(c),
            }
        }
        if let Some(word) = prose.split(|c: char| !c.is_ascii_alphabetic()).find(|w| ENGLISH.contains(w)) {
            wrong.push(format!("{word} in {src:?} -> {translation:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} Hindi rows keep an English word:\n{}", wrong.len(), wrong.join("\n"));
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

#[test]
fn gujarati_web_selection_requires_a_bundled_interface_font() {
    let gu = LANGUAGES.iter().find(|l| l.code == "gu").expect("Gujarati catalog");
    assert!(gu.available_with_fonts(false, false), "native installed-font fallback");
    assert!(!gu.available_with_fonts(true, false), "web must not offer missing glyphs");
    assert!(gu.available_with_fonts(true, true));
    assert!(LANGUAGES[0].available_with_fonts(true, false), "English fallback");
    assert_eq!(Lang::from_code("gu").is_some(), gu.available());
    assert_eq!(Lang::for_picker().iter().any(|l| l.code() == "gu"), gu.available());
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

/// The Objects on Path Options section's Detach button, its undo step and the message for a
/// Detach with no object chosen.
#[test]
fn objects_on_path_detach_strings_are_in_complete_catalogs() {
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in ["Detach", "Detach Object from Path"] {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
    let choose = "choose the object to detach with its Select/Move widget";
    assert!(message_literals().contains(choose));
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        assert_ne!(message(lang, choose), choose, "{code}");
    }
}

/// The Window › Brush Libraries menu, built from the built-in library table, names each library.
#[test]
fn built_in_brush_library_names_are_in_complete_catalogs() {
    let mut bad = vec![];
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        let missing: Vec<&str> =
            vectorcraft_engine::cmd::brushlib::BUILTIN.iter().map(|b| b.name).filter(|l| lang.catalog().plain(l).is_none()).collect();
        if !missing.is_empty() {
            bad.push(format!("{} is missing {missing:?}", lang.code));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A built-in brush library's panel lists its brushes in the UI language, so every brush of every
/// built-in library has a row in each complete catalog.
#[test]
fn built_in_library_brush_names_are_in_complete_catalogs() {
    use vectorcraft_engine::cmd::brushlib;
    let session = vectorcraft_engine::Session::new();
    let mut names: Vec<String> = brushlib::BUILTIN
        .iter()
        .filter_map(|b| brushlib::library(&session, b.id))
        .flat_map(|(_, lib)| lib.brushes.iter().map(|b| b.name.clone()).collect::<Vec<_>>())
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 43, "{names:?}");
    let mut bad = vec![];
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        let missing: Vec<&String> = names.iter().filter(|n| lang.catalog().plain(n).is_none()).collect();
        if !missing.is_empty() {
            bad.push(format!("{} is missing {missing:?}", lang.code));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
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

/// Preferences › Performance › Use Monitor Profile for Canvas and the note under it.
#[test]
fn monitor_profile_labels_are_in_complete_catalogs() {
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in [
            "Use Monitor Profile for Canvas",
            "The canvas is drawn in sRGB.",
            "Canvas display profile:",
            "The system matches the window to the display's colors.",
            "No usable display profile was found: the canvas is drawn in sRGB.",
        ] {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// The Control bar panel menu's items, its groups of options and the edges, and the label of the
/// command that turns a group on or off.
#[test]
fn control_bar_panel_menu_labels_are_in_complete_catalogs() {
    const LABELS: [&str; 18] = [
        "Snapping",
        "Anchor Points",
        "Tool Options",
        "Object Options",
        "Image Options",
        "Fill and Stroke",
        "Opacity",
        "Graphic Style",
        "Character",
        "Paragraph",
        "Vertical Alignment",
        "Align and Distribute",
        "Transform",
        "Artboard Options",
        "Document Options",
        "Dock to Top",
        "Dock to Bottom",
        "Control Bar Options",
    ];
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in LABELS {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// Document Info's labels: every category title, which the panel and its menu show, every count
/// label of the Objects category, and the Document category's fixed row labels (the artboard rows
/// show artboard names and are left out).
#[test]
fn document_info_labels_are_in_complete_catalogs() {
    let mut session = vectorcraft_engine::Session::new();
    session.execute("file.new", &serde_json::json!({"width": 100, "height": 100})).unwrap();
    session.execute("shape.rectangle", &serde_json::json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let info = session.execute("document.info", &serde_json::json!({})).unwrap();
    let sections = info["sections"].as_array().unwrap();
    let titles: Vec<&str> = sections.iter().filter_map(|s| s["title"].as_str()).collect();
    let categories: Vec<&str> = vectorcraft_engine::cmd::CATEGORIES.iter().map(|&(_, title)| title).collect();
    assert_eq!(titles, categories);
    let mut labels = categories;
    labels.extend(vectorcraft_engine::cmd::OBJECT_LABELS.iter().map(|&(_, label)| label));
    let objects = sections.iter().find(|s| s["id"] == "objects").unwrap()["rows"].as_array().unwrap();
    assert!(objects.iter().any(|r| r[0] == "Path Length"), "{objects:?}");
    labels.push("Path Length");
    let document = sections.iter().find(|s| s["id"] == "document").unwrap()["rows"].as_array().unwrap();
    let artboards = document.iter().position(|r| r[0] == "Artboards").unwrap();
    let names = artboards + 1..=artboards + document[artboards][1].as_str().unwrap().parse::<usize>().unwrap();
    let fixed: Vec<&str> = document.iter().enumerate().filter(|(k, _)| !names.contains(k)).filter_map(|(_, r)| r[0].as_str()).collect();
    assert_eq!(fixed.len(), 9, "{fixed:?}");
    labels.extend(fixed);
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in &labels {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// The values Document Info makes and shows in the interface language: the font states and
/// embedding permissions, the templates of the font, image, raster effects and Repeat object
/// values, the names of gradients named by their kind, and the counted instances, rows and columns.
#[test]
fn document_info_values_are_in_complete_catalogs() {
    const VALUES: [&str; 20] = [
        "built in",
        "embedding not allowed",
        "embedding for editing",
        "embedding for preview and print",
        "embedding allowed",
        "{file}; {embedding}",
        "substituted: shown in {family} {style} ({detail})",
        "missing: shown in {family} {style}",
        "missing",
        "{width} × {height} px",
        "{size} — {file}",
        "{n} ppi",
        "{kind}, {instances}",
        "{kind}, {rows} × {cols}",
        "{grid}, {instances}",
        "Linear gradient",
        "Radial gradient",
        "Conic gradient",
        "Diamond gradient",
        "Freeform gradient",
    ];
    const COUNTED: [(&str, &str); 3] = [("{n} instance", "{n} instances"), ("{n} row", "{n} rows"), ("{n} column", "{n} columns")];
    let gradient_kinds: Vec<String> = vectorcraft_color::GradientKind::ALL.iter().map(|k| format!("{} gradient", k.label())).collect();
    assert!(gradient_kinds.iter().all(|k| VALUES.contains(&k.as_str())), "{gradient_kinds:?}");
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for value in VALUES {
            assert!(lang.catalog().plain(value).is_some(), "{} is missing {value}", lang.code);
        }
        for (one, other) in COUNTED {
            assert!(lang.catalog().plural(one, other, 0).is_some(), "{} is missing the row for {other}", lang.code);
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

/// The Painted and Sketched graphic style library's name and its styles' names have a row in every
/// bundled catalog.
#[test]
fn painted_and_sketched_style_names_are_in_every_catalog() {
    let lib = vectorcraft_doc::style_libs::builtin_style_library("painted-sketched").expect("built in");
    let labels: Vec<&str> = std::iter::once(lib.name.as_str()).chain(lib.styles.iter().map(|g| g.name.as_str())).collect();
    assert_eq!(labels.len(), 8);
    for lang in LANGUAGES.iter().filter(|lang| !lang.source.is_empty()) {
        for label in &labels {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
    }
}

/// The names the EPS options' menus show (preview, overprints, flattener preset and PostScript
/// level), and Save As's Preview label.
#[test]
fn eps_option_menu_names_are_in_complete_catalogs() {
    use vectorcraft_engine::cmd::FlattenOptions;
    use vectorcraft_eps::{Level, Overprint, Preview};
    let names: Vec<&str> = Preview::ALL
        .map(Preview::label)
        .into_iter()
        .chain(Overprint::ALL.map(Overprint::label))
        .chain(Level::ALL.map(Level::label))
        .chain(FlattenOptions::PRESETS.into_iter().filter_map(FlattenOptions::preset_label))
        .chain(["Preview:"])
        .collect();
    for lang in LANGUAGES.iter().filter(|lang| lang.complete_menus) {
        for label in &names {
            assert!(lang.catalog().plain(label).is_some(), "{} is missing {label}", lang.code);
        }
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

/// The Script Console's counted phrases have a row in every complete catalog, and a count picks
/// the language's own form.
#[test]
fn script_console_counts_take_each_language_s_form() {
    const COUNTED: [(&str, &str); 3] =
        [("{n} host call", "{n} host calls"), ("{n} earlier line dropped", "{n} earlier lines dropped"), ("{n} line cut", "{n} lines cut")];
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        for (one, other) in COUNTED {
            assert!(l.catalog().plural(one, other, 0).is_some(), "{} is missing the row for {other}", l.code);
        }
    }
    assert_eq!([1, 2].map(|n| trn(de(), n, COUNTED[0].0, COUNTED[0].1)), ["1 Host-Aufruf", "2 Host-Aufrufe"]);
    let ru = Lang::from_code("ru").expect("registered");
    assert_eq!([1, 3, 5].map(|n| trn(ru, n, COUNTED[2].0, COUNTED[2].1)), ["1 строка обрезана", "3 строки обрезаны", "5 строк обрезано"]);
    let fr = Lang::from_code("fr").expect("registered");
    assert_eq!([0, 1].map(|n| trn(fr, n, COUNTED[1].0, COUNTED[1].1)), ["0 ligne antérieure supprimée", "1 ligne antérieure supprimée"]);
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

#[cfg(feature = "interface-fonts")]
fn is_gujarati(c: char) -> bool {
    matches!(c as u32, 0x0A80..=0x0AFF)
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
    let gujarati_ui_font = vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.scripts.contains(&"Gujr"));
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
                        && (gujarati_ui_font || !is_gujarati(*c))
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
/// catalog template that [`untranslated_report`] checks to `VECTORCRAFT_I18N_DUMP_MESSAGES=<file>`,
/// one per line.
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
        std::fs::write(path, message_dump()).expect("write dump");
    }
}

/// The text [`dump_source_strings`] writes to `VECTORCRAFT_I18N_DUMP_MESSAGES`.
fn message_dump() -> String {
    reported_messages().iter().map(|m| format!("{m}\n")).collect()
}

/// The translators' message dump lists every message the untranslated report checks, the
/// phrases a parameter schema's error builds included.
#[test]
fn message_dump_lists_the_reported_messages() {
    let dump = message_dump();
    let lines: std::collections::BTreeSet<&str> = dump.lines().collect();
    for phrase in SCHEMA_PHRASES {
        assert!(lines.contains(phrase), "the dump skips {phrase:?}");
    }
    assert!(lines.contains("the color book is cut short"));
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
    let messages = reported_messages();
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

/// The status and error messages [`untranslated_report`] checks: the message literals of
/// [`MESSAGE_CRATES`] and the phrases a parameter schema's error builds at run time
/// ([`SCHEMA_PHRASES`]), which no literal holds.
fn reported_messages() -> std::collections::BTreeSet<String> {
    let mut messages = message_literals();
    messages.extend(SCHEMA_PHRASES.iter().map(|p| p.to_string()));
    messages
}

/// The untranslated report checks the messages of the effects and of the crates whose errors the
/// engine and the effects pass on, and the phrases a parameter schema's error builds.
#[test]
fn untranslated_report_checks_every_message_crate_and_schema_phrase() {
    let reported = reported_messages();
    let passed_on = [
        ("effects", "the SVG filter has no primitives"),
        ("color", "not a swatch library (.vcswatches or .gpl)"),
        ("render", "invalid scale {scale}"),
        ("three-d", "3D effects require one to ten lights"),
        ("pathops", "the shapes are too degenerate to combine"),
    ];
    for (krate, m) in passed_on {
        assert!(message_literals_in(&[krate]).contains(m), "{krate}: {m}");
        assert!(reported.contains(m), "the report skips {krate}: {m}");
    }
    for phrase in SCHEMA_PHRASES {
        assert!(reported.contains(*phrase), "the report skips the schema phrase {phrase:?}");
    }
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

/// A placeholder that occurs twice in a template matches the same text both times, so a template
/// that ends with the name it starts with does not swallow a line that wraps its message.
#[test]
fn a_repeated_placeholder_matches_the_same_text() {
    let c = Catalog::parse(concat!(
        "@msg\t{name}: the next answer is for {_1}, not {name}\tnext {name} {_1}\n",
        "@msg\t{file}: line {line}: {e} (thrown)\tthrown {file} {line} {e}\n",
    ));
    let (form, caps) = c.message("confirm: the next answer is for prompt, not confirm").expect("matches");
    assert_eq!((form, caps), ("next {name} {_1}", vec![("name", "confirm"), ("_1", "prompt"), ("name", "confirm")]));
    let line = "t.js: line 3: Error: confirm: the next answer is for prompt, not confirm (thrown)";
    let (form, caps) = c.message(line).expect("matches");
    assert_eq!(form, "thrown {file} {line} {e}");
    assert_eq!(caps.last(), Some(&("e", "Error: confirm: the next answer is for prompt, not confirm")));
    assert!(c.message("alert: the next answer is for prompt, not confirm").is_none());
}

/// A template that starts with text matches a message it wraps before a template that starts
/// with a placeholder, even one with more literal text, so the wrapped reason is translated by
/// its own row.
#[test]
fn templates_that_start_with_text_match_first() {
    let c = Catalog::parse(concat!(
        "@msg\tplug-in manifest: {0}\twrapped {0}\n",
        "@msg\t{_1} is {_2} bytes (limit {_3})\tsized {_1}\n",
        "@msg\tmanifest is {_1} bytes (limit {_2})\tmanifest {_1}\n",
    ));
    let (form, caps) = c.message("plug-in manifest: manifest is 9 bytes (limit 8)").expect("matches");
    assert_eq!((form, caps), ("wrapped {0}", vec![("0", "manifest is 9 bytes (limit 8)")]));
    assert_eq!(c.message("a.wasm is 9 bytes (limit 8)").map(|(f, _)| f), Some("sized {_1}"));
}

/// `{{` and `}}` in a message row are literal braces, in the English source and in the
/// translation, as in a Rust format string; only the other `{name}` are placeholders.
#[test]
fn message_rows_read_doubled_braces_as_literal_braces() {
    let c = Catalog::parse(concat!(
        "@msg\t`{_1}` runs with plugin.run {{\"id\": \"{_2}\"}}\t`{_1}`: plugin.run {{\"id\": \"{_2}\"}} (fr)\n",
        "@msg\tbraces {{}} and {_1}\tbraces {{}} with {_1}\n",
        "@msg\tonly {{literal}} braces\tliteral {{braces}}\n",
        "@msg\tnot an RTF file: it doesn't start with {\\\\rtf\tno RTF: {\\\\rtf\n",
    ));
    let (form, caps) = c.message(r#"`a.b` runs with plugin.run {"id": "plugin:a.b"}"#).expect("matches");
    assert_eq!(caps, [("_1", "a.b"), ("_2", "plugin:a.b")]);
    assert_eq!(fill_once(form, &caps), r#"`a.b`: plugin.run {"id": "plugin:a.b"} (fr)"#);
    assert!(c.message(r#"`a.b` runs with plugin.run {{"id": "plugin:a.b"}}"#).is_none());
    let (form, caps) = c.message("braces {} and x").expect("matches");
    assert_eq!(fill_once(form, &caps), "braces {} with x");
    let (form, caps) = c.message("only {literal} braces").expect("matches");
    assert_eq!(fill_once(form, &caps), "literal {braces}");
    // A `{` with no `}` after it is literal text too.
    assert_eq!(c.message(r"not an RTF file: it doesn't start with {\rtf").map(|(f, _)| f), Some(r"no RTF: {\rtf"));
    assert_eq!(placeholders(r#"{{"id": "{_2}"}} {{x}} {n}"#), ["_2", "n"]);
}

/// A format string's `{{` and `}}` stay literal braces in its catalog key, and only its real
/// placeholders are numbered.
#[test]
fn message_keys_keep_doubled_braces() {
    assert_eq!(
        as_template(r#"`{}` is an importer: open a file with document.open {{"importer": "{}"}}"#),
        r#"`{_1}` is an importer: open a file with document.open {{"importer": "{_2}"}}"#
    );
    assert_eq!(as_template("{k} must be {{size?, position?}}"), "{k} must be {{size?, position?}}");
    assert_eq!(as_template(r"it doesn't start with {\rtf"), r"it doesn't start with {\rtf");
}

/// `plugin.run` of an importer, an exporter or a live effect names the command that runs it with
/// the JSON that picks it, and every complete-message catalog translates that error with the
/// plug-in's id and the JSON kept.
#[test]
fn plugin_run_of_another_kind_is_translated_with_its_json() {
    use serde_json::json;
    use vectorcraft_plugins::wat as tpl;
    let mut s = vectorcraft_engine::Session::new();
    let plugins = [
        ("test.i18n.reader", tpl::importer("test.i18n.reader", r#"["ti18n"]"#, "{}"), "is an importer"),
        ("test.i18n.writer", tpl::exporter("test.i18n.writer", r#"["ti18n"]"#, "{}"), "is an exporter"),
        ("test.i18n.effect", tpl::manifest("test.i18n.effect", "effect", "{}"), "is a live effect"),
    ];
    for (_, manifest, _) in &plugins {
        let wasm = wat::parse_str(tpl::module(manifest, tpl::ECHO, "", 1)).unwrap();
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    }
    s.execute("file.new", &json!({})).unwrap();
    let errors: Vec<(String, &str, &str)> =
        plugins.iter().map(|(id, _, english)| (s.execute("plugin.run", &json!({"id": id})).unwrap_err().to_string(), *id, *english)).collect();
    for (id, _, _) in &plugins {
        s.execute("plugin.remove", &json!({"id": id})).unwrap();
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, id, english) in &errors {
            assert!(e.contains(english), "{e}");
            let json = &e[e.rfind('{').unwrap()..];
            assert!(json.ends_with('}') && json.contains(id), "{e}");
            let shown = message(lang, e);
            assert!(!shown.contains(english) && shown.contains(json) && shown.contains(&format!("`{id}`")), "{code}: {e} -> {shown}");
        }
    }
    assert!(
        message(de(), &errors[0].0).ends_with(r#"Öffnen Sie eine Datei mit document.open {"importer": "plugin.test.i18n.reader"}"#),
        "{}",
        message(de(), &errors[0].0)
    );
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

/// The plug-in host's status and error messages (manifests, modules, runs and the output of
/// importers, exporters and effects) have an `@msg` row in every complete-message catalog.
#[test]
fn plugin_messages_are_translated_in_every_complete_catalog() {
    let messages = message_literals_in(&["plugins"]);
    assert!(messages.len() > 100, "scan found only {} plug-in messages", messages.len());
    let mut missing = vec![];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        let rows: std::collections::BTreeSet<&str> = entries.iter().filter(|(ctx, _, _)| ctx == "@msg").map(|(_, src, _)| src.as_str()).collect();
        missing.extend(messages.iter().filter(|m| !rows.contains(m.as_str())).map(|m| format!("{code}: {m}")));
    }
    assert!(missing.is_empty(), "{} plug-in messages have no @msg row:\n{}", missing.len(), missing.join("\n"));
}

/// Errors the plug-in host returns are translated whole in every complete-message catalog, the
/// reason inside the error's kind included, and keep their numbers, JSON shapes, permission names
/// and nested reasons as written.
#[test]
fn plugin_errors_translate_with_their_values() {
    use vectorcraft_plugins::{Error, Manifest, manifest::MAX_MANIFEST_BYTES};
    let err = |r: Result<(), Error>| r.err().map(|e| e.to_string()).unwrap_or_default();
    let not_json = err(vectorcraft_plugins::export::decode(b"{} x").map(drop));
    let reason = not_json.strip_prefix("plug-in output: not JSON: ").expect("not JSON error").to_string();
    let errors = [
        (err(Manifest::parse(b"\xff").map(drop)), "manifest is not UTF-8", vec!["UTF-8"]),
        (err(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).map(drop)), "manifest is", vec!["65537", "65536"]),
        (err(vectorcraft_plugins::export::decode(b"{}").map(drop)), "expected {", vec![r#"{"dataBase64": "…"}"#]),
        (err(vectorcraft_plugins::objects::decode(b"{}").map(drop)), "expected {", vec![r#"{"objects": [...]}"#]),
        (not_json, "not JSON", vec![reason.as_str()]),
        (Error::NeedsApproval("doc.read, doc.write".into()).to_string(), "needs approval", vec!["doc.read, doc.write"]),
    ];
    for (e, english, _) in &errors {
        assert!(e.contains(english), "{e}");
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, english, kept) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(english), "{code}: {e} -> {shown}");
            assert!(kept.iter().all(|k| shown.contains(k)), "{code}: {e} -> {shown}");
        }
    }
    assert_eq!(message(de(), &errors[1].0), "Plug-in-Manifest: das Manifest ist 65537 Byte groß (Grenze 65536)");
    let fr = Lang::from_code("fr").expect("registered");
    assert!(message(fr, &errors[3].0).ends_with(r#"attendu : {"objects": [...]}"#), "{}", message(fr, &errors[3].0));
}

/// The reasons the plug-in host puts inside its errors (what is wrong with a parameter's spec or
/// dialog keys, the value a parameter needs, what is wrong with an output object) are translated
/// with the error in every complete-message catalog, so no status line mixes two languages.
#[test]
fn plugin_error_reasons_are_translated_with_the_error() {
    use serde_json::json;
    use vectorcraft_plugins::{Error, Manifest};
    let err = |r: Result<(), Error>| r.err().map(|e| e.to_string()).unwrap_or_default();
    let manifest = |params: &str| Manifest::parse(format!(r#"{{"id":"a","name":"x","kind":"filter","params":{{{params}}}}}"#).as_bytes());
    let m = manifest(r#""angles":{"type":"numberList","min":0,"max":360,"default":[45],"minItems":1,"maxItems":3}"#).expect("manifest");
    let errors = [
        (err(manifest(r#""r":{"type":"number","min":5,"max":1,"default":1}"#).map(drop)), "bad range"),
        (err(manifest(r#""r":{"type":"number","min":0,"max":1,"default":1,"label":" "}"#).map(drop)), "printable characters"),
        (err(m.resolve_params(&json!({"angles": "big"})).map(drop)), "a list of numbers"),
        (err(vectorcraft_plugins::objects::decode(br#"{"objects":[{"opacity":"full"}]}"#).map(drop)), "must be a number"),
    ];
    for (e, english) in &errors {
        assert!(e.contains(english), "{e}");
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, english) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(english), "{code}: {e} -> {shown}");
        }
    }
    assert_eq!(message(de(), &errors[0].0), "Plug-in-Manifest: Parameter `r`: ungültiger Bereich");
    assert_eq!(message(de(), &errors[2].0), "Plug-in-Parameter: `angles` muss eine Liste von Zahlen sein");
    assert!(message(de(), &errors[3].0).ends_with("objects[0]: `opacity` muss eine Zahl sein"), "{}", message(de(), &errors[3].0));
}

/// The 3D effects' source files under `crates/effects/src`. Their refusals reach the status bar
/// through the effect commands, and every `format!` text in them outside tests is a refusal.
const THREE_D_SOURCES: &[&str] = &[
    "solids.rs",
    "solids/classic.rs",
    "solids/graphics.rs",
    "solids/shadow.rs",
    "solids/source.rs",
    "solids/wireframe.rs",
    "revolve.rs",
    "revolve/classic.rs",
    "lighting.rs",
];

/// The values 3D refusals put in their templates that are messages of their own: the name of an
/// effect that has no 3D type of its own, and the objects a group's 3D effect can't build from.
const THREE_D_PARTS: &[&str] = &["This effect", "clipping paths", "other objects"];

/// The 3D effects' refusals as catalog templates: the messages of [`THREE_D_SOURCES`] that
/// [`source_messages`] finds, and the `format!` texts their helpers return.
fn three_d_refusals() -> std::collections::BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../effects/src");
    let texts: Vec<String> = THREE_D_SOURCES.iter().map(|f| std::fs::read_to_string(dir.join(f)).expect("a 3D source file")).collect();
    let mut consts = std::collections::BTreeMap::new();
    for text in &texts {
        for (name, value) in str_consts(without_test_module(text)) {
            consts.entry(name).or_insert_with(Vec::new).push(value);
        }
    }
    texts.iter().flat_map(|text| source_messages(without_test_module(text), &[("format!(", 0)], &consts)).collect()
}

/// The names 3D refusals give their effect (`{label}`): each 3D effect's menu label without its
/// ellipsis, and [`THREE_D_PARTS`].
fn three_d_parts() -> Vec<String> {
    let labels = vectorcraft_effects::effect_catalog().into_iter().filter(|e| vectorcraft_effects::is_three_d(e.id));
    labels.map(|e| e.label.trim_end_matches('…').to_string()).chain(THREE_D_PARTS.iter().map(|p| p.to_string())).collect()
}

/// The 3D effects' refusals, and the effect names and objects they name, have an `@msg` row in
/// every complete-message catalog.
#[test]
fn three_d_refusals_are_translated_in_every_complete_catalog() {
    let refusals = three_d_refusals();
    assert!(refusals.len() > 55, "scan found only {} 3D refusals", refusals.len());
    let helper_made = [
        "{label} can't build a solid from an image: trace it first with Object › Image Trace › Make.",
        "{_1} can't build solids from art nested more than {MAX_DEPTH} levels deep",
        "the object has no 3D effect (Extrude & Bevel, Revolve, Inflate or Rotate)",
    ];
    for m in helper_made {
        assert!(refusals.contains(m), "the scan reads helpers' texts and constants: {m}");
    }
    let parts = three_d_parts();
    for label in ["Extrude & Bevel", "Revolve", "Inflate", "Rotate", "Extrude & Bevel (Classic)", "Revolve (Classic)", "Rotate (Classic)"] {
        assert!(parts.iter().any(|p| p == label), "{label} is a 3D effect's name");
    }
    let mut missing = vec![];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        let rows: std::collections::BTreeSet<&str> = entries.iter().filter(|(ctx, _, _)| ctx == "@msg").map(|(_, src, _)| src.as_str()).collect();
        missing.extend(refusals.iter().chain(&parts).filter(|m| !rows.contains(m.as_str())).map(|m| format!("{code}: {m}")));
    }
    assert!(missing.is_empty(), "{} 3D messages have no @msg row:\n{}", missing.len(), missing.join("\n"));
}

/// A 3D effect's refusal shows in the interface language whole, with the effect's translated
/// name, and with its numbers, the parameter keys it names and a symbol's name as given; the
/// error the command returns stays English.
#[test]
fn three_d_refusals_translate_with_their_values() {
    use serde_json::json;
    use std::sync::Arc;
    use vectorcraft_doc::{Node, NodeId, NodeKind};
    use vectorcraft_effects::{CLASSIC_EXTRUDE, EXTRUDE};
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = s.execute("path.create", &json!({"d": "M10 10L60 10L60 60L10 60Z"})).unwrap()["id"].as_u64().unwrap();
    let rect = s.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
    let group = Node::group(NodeId(id + 1), vec![Arc::new(rect.clone())]);
    let instance = Node::new(NodeId(id + 2), NodeKind::SymbolInstance { symbol: "Mine 7".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let refuse = |r: Result<(), String>| {
        let e = r.expect_err("refused");
        vectorcraft_engine::EngineError::BadParams { cmd: "effect.apply".into(), msg: e }.to_string()
    };
    let errors = [
        (refuse(vectorcraft_effects::validate_three_d_at(&group, EXTRUDE, &json!({}), (Some(0), 0))), "goes on the group itself", vec![]),
        (
            refuse(vectorcraft_effects::validate_three_d(&rect, EXTRUDE, &json!({"lights": [{"intensity": 500}]}))),
            "must be a number from",
            vec!["0", "100"],
        ),
        (
            refuse(vectorcraft_effects::validate_three_d(&rect, EXTRUDE, &json!({"graphics": [{"symbol": "S", "face": "side"}]}))),
            "must be front or back",
            vec!["`graphics`", "front", "back"],
        ),
        (refuse(vectorcraft_effects::validate_three_d(&rect, CLASSIC_EXTRUDE, &json!({"depth": "deep"}))), "must be a number", vec!["`depth`"]),
        (refuse(vectorcraft_effects::validate_three_d(&instance, EXTRUDE, &json!({}))), "the symbol's art isn't available", vec!["Mine 7"]),
        (refuse(vectorcraft_effects::three_d_wireframe_in(None, &rect).map(drop)), "has no 3D effect", vec![]),
    ];
    for (e, english, _) in &errors {
        assert!(e.starts_with("invalid parameters for `effect.apply`: ") && e.contains(english), "{e}");
        assert_eq!(message(Lang::EN, e), *e);
    }
    assert!(errors[1].0.contains("light intensity must be a number from 0 to 100"), "{}", errors[1].0);
    assert!(errors[4].0.contains("Extrude & Bevel can't build a solid from the instance of symbol \"Mine 7\""), "{}", errors[4].0);
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (e, english, kept) in &errors {
            let shown = message(lang, e);
            assert!(!shown.contains(english) && shown.contains("`effect.apply`"), "{code}: {e} -> {shown}");
            assert!(kept.iter().all(|k| shown.contains(k)), "{code}: {e} -> {shown}");
        }
        let symbol = message(lang, &errors[4].0);
        let label = message(lang, "Extrude & Bevel");
        assert!(label != "Extrude & Bevel" && symbol.contains(&label), "{code}: the effect's name is translated: {symbol}");
    }
    assert_eq!(message(de(), &errors[1].0), "ungültige Parameter für `effect.apply`: Licht: intensity muss eine Zahl von 0 bis 100 sein");
}

/// A value a message quotes is a name and is shown as written, even when a catalog row translates
/// the same text: a symbol called Rotate stays Rotate while the effect's name is translated.
#[test]
fn quoted_values_in_a_message_stay_as_written() {
    let s = "Extrude & Bevel can't build a solid from the instance of symbol \"Rotate\": the symbol's art isn't available";
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let shown = message(lang, s);
        let rotate = message(lang, "Rotate");
        assert!(rotate != "Rotate" && shown.contains("Rotate") && !shown.contains(&rotate), "{code}: {shown}");
        assert!(shown.contains(&message(lang, "Extrude & Bevel")), "{code}: {shown}");
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
    (
        "1 linked image changed on disk: Update Links shows the new version. Opened with 1 note: a note",
        "1 linked image changed on disk: Update Links shows the new version. Opened with 2 notes: a note",
    ),
];

/// The open, save and recovery status messages and the Global Edit options' Results line name a
/// count of 1 in the singular and other counts in the plural, and every complete catalog
/// translates both, in the form its plural rule picks.
#[test]
fn open_save_and_recovery_messages_take_the_plural_form() {
    use crate::dialogs::missing_links::changed_on_disk_status;
    use crate::io::{opened_after_notice, opened_with_notes, saved_status};
    use crate::panels::properties::global_edit_results;
    use crate::recovery::recovered_status;
    let saved = |path: &str, n: usize| saved_status(path, &serde_json::json!({ "warnings": vec!["a note"; n] }));
    let notice = changed_on_disk_status(1);
    let made = [
        (opened_with_notes(1, "a note"), opened_with_notes(2, "a note")),
        (saved("/out/a.pdf", 1), saved("/out/a.pdf", 2)),
        (recovered_status(true, 1), recovered_status(true, 2)),
        (recovered_status(false, 1), recovered_status(false, 2)),
        (opened_after_notice(&notice, 1, "a note"), opened_after_notice(&notice, 2, "a note")),
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
        // The line about linked files and the open's line are each shown as the catalog has them.
        for n in [1, 2] {
            let shown = message(lang, &opened_after_notice(&notice, n, "a note"));
            let (link, open) = (message(lang, &notice), message(lang, &opened_with_notes(n, "a note")));
            assert!(link != notice && shown.starts_with(&link) && shown.ends_with(&open), "{code}: {shown}");
        }
    }
    let unreadable = "2 linked files can't be read here: their previews show";
    assert_eq!(
        message(de(), &opened_after_notice(unreadable, 3, "a note")),
        format!("{}. Mit 3 Hinweisen geöffnet: a note", message(de(), unreadable))
    );
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
    let found = source_messages(code, &engine, &Default::default());
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
    let elsewhere = source_messages(code, &[], &Default::default());
    let in_variants = ["nested", "manifest", "nothing to clear", "panel is locked"];
    assert!(elsewhere.len() == 4 && elsewhere.iter().all(|m| in_variants.iter().any(|w| m.contains(w))), "{elsewhere:#?}");
    // A message read this way matches its row: `{key}` keeps its name.
    let c = Catalog::parse("@msg\t`{key}` must be a number\t`{key}` ok\n");
    assert_eq!(c.message("`width` must be a number").map(|m| m.1), Some(vec![("key", "width")]));
}

/// The scan reads a message that an error or a status takes from a `&str` constant, by its name or
/// by its path, with `.into()`, `.to_string()` or nothing after it, and the constant declared in
/// another file. It does not read a constant that no error takes, a call of a name that a constant
/// also has, or a constant in a comment.
#[test]
fn message_scan_reads_messages_held_in_constants() {
    let declared = r#"
pub(crate) const TOO_LARGE: &str = "the selection is too large to hand over";
pub const SLOW: &'static str = "time budget";
const UNUSED: &str = "not an error: a constant no error takes";
const fn NOT_A_STR() -> u32 { 0 }
// const COMMENTED: &str = "not an error: a constant in a comment";
"#;
    let code = r#"
fn run() -> Result<()> {
    let _ = Error::Limit(crate::runtime::TOO_LARGE.into());
    let _ = Err(Error::Limit(SLOW.to_string()));
    let _ = Err(COMMENTED);
    let _ = Err(SLOW(3));
    Ok(())
}
"#;
    let mut consts = std::collections::BTreeMap::new();
    for (name, value) in str_consts(declared) {
        consts.entry(name).or_insert_with(Vec::new).push(value);
    }
    assert_eq!(consts.keys().collect::<Vec<_>>(), ["SLOW", "TOO_LARGE", "UNUSED"]);
    let found: std::collections::BTreeSet<String> = source_messages(code, &[], &consts).into_iter().collect();
    assert_eq!(found.iter().collect::<Vec<_>>(), ["the selection is too large to hand over", "time budget"]);
    assert!(source_messages(code, &[], &Default::default()).is_empty());
}

/// The scan reads the message of each of the plug-in host's [`ERROR_HELPERS`]: a literal, a
/// `format!` string with or without a `&` before it, after the location of `out_err` (a
/// `format!` string too), the number and range of `in_range`, the number and location of `unit`,
/// and the block of `.read_packed`, a method. It does not read a value that isn't a literal, a
/// key's name alone, a call with too few arguments, a helper's declaration or a call whose name
/// ends in a helper's name.
#[test]
fn message_scan_reads_plugin_helpers() {
    let code = r#"
fn check(k: &str) -> Result<()> {
    let bad = |what: &str| Err(Error::Manifest(format!("parameter `{k}`: {what}")));
    let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
    let _ = bad("bad range");
    let _ = bad(&format!("`maxLength` must be 1-{MAX_TEXT}, and the default no longer"));
    let _ = wrong("a number");
    let _ = wrong(&format!("a list of {min_items} to {max_items} numbers"));
    let _ = out_err(at, "`opacity` must be a number");
    let _ = out_err(&format!("{at}.{k}"), format!("unknown key `{k}`; the keys here are {}", keys.join(", ")));
    let _ = out_err(at, e);
    let _ = live.read_packed(packed, "the output", limits.max_output_bytes);
    let _ = in_range(l, 0.0, 100.0, at, "a color component");
    let _ = unit(*x, &format!("{at}.plates"), "a plate");
    let _ = unit(s.tint, &format!("{at}.tint"), "`tint`");
    let _ = unit(r);
    let _ = not_bad("not an error helper");
    Err(wrong())
}
fn wrong(&self) -> String { String::new() }
"#;
    let plugins: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "plugins").map(|&(_, call, before)| (call, before)).collect();
    let found: std::collections::BTreeSet<String> = source_messages(code, &plugins, &Default::default()).into_iter().collect();
    let want = [
        "`maxLength` must be 1-{MAX_TEXT}, and the default no longer",
        "`opacity` must be a number",
        "`{k}` must be {what}",
        "a color component",
        "a list of {min_items} to {max_items} numbers",
        "a number",
        "a plate",
        "bad range",
        "parameter `{k}`: {what}",
        "the output",
        "unknown key `{k}`; the keys here are {_1}",
    ];
    assert_eq!(found.iter().collect::<Vec<_>>(), want);
}

/// A constant's name reads the text the file declares under it, else every text the files of its
/// own crate declare under it, else every text any scanned crate declares under it: an engine
/// reader's error does not read the Scripts menu's headers, which are constants of the same names
/// in another crate.
#[test]
fn message_scan_reads_constants_of_the_file_and_its_crate() {
    use std::collections::{BTreeMap, BTreeSet};
    let jxl = str_consts(r#"const TRUNCATED: &str = "the JPEG XL file is cut short"; const LIMIT: &str = "the image is too large";"#);
    let pcx = str_consts(r#"const TRUNCATED: &str = "the PCX file is cut short";"#);
    let menu = str_consts(r#"const TRUNCATED: &str = "Some scripts were not listed"; const SHARED: &str = "a message of another crate";"#);
    let gather = |files: &[&Vec<(String, String)>]| {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in files.iter().copied().flatten() {
            map.entry(name.clone()).or_default().push(value.clone());
        }
        map
    };
    let engine = gather(&[&jxl, &pcx]);
    let everywhere = gather(&[&jxl, &pcx, &menu]);
    let code =
        "fn f() -> Result<()> { Err(TRUNCATED.into()) }\nfn g() -> Result<()> { Err(LIMIT.into()) }\nfn h() -> Result<()> { Err(SHARED.into()) }";
    let found = |own: &[(String, String)], in_crate: &BTreeMap<String, Vec<String>>| -> BTreeSet<String> {
        source_messages(code, &[], &scoped_consts(own, in_crate, &everywhere)).into_iter().collect()
    };
    let in_jxl = ["a message of another crate", "the JPEG XL file is cut short", "the image is too large"];
    assert_eq!(found(&jxl, &engine).iter().collect::<Vec<_>>(), in_jxl);
    let in_engine = ["a message of another crate", "the JPEG XL file is cut short", "the PCX file is cut short", "the image is too large"];
    assert_eq!(found(&[], &engine).iter().collect::<Vec<_>>(), in_engine);
    assert_eq!(found(&[], &BTreeMap::new()).len(), 5);
    let scanned = message_literals();
    for header in [crate::scripts_menu::TRUNCATED, crate::scripts_menu::UNREADABLE] {
        assert!(!scanned.contains(header), "{header}");
    }
}

/// The scan reads the errors the script interpreter throws at a script with a method of any
/// receiver, `&format!` strings included, and not a call of a longer name.
#[test]
fn message_scan_reads_script_interpreter_errors() {
    let code = r#"
fn f(i: &mut Interp, interp: &mut Interp) -> JsResult<Value> {
    let _ = i.type_error(&format!("{what}: the object is not a File or a Folder"));
    let _ = interp.throw(ErrorType::Range, "File.seek: the mode must be 0, 1 or 2");
    let _ = self.error("the path has an empty step");
    let _ = self.throw_host(t, "not an error: a longer name");
    let _ = throw_type_error(interp, "not an error: a function");
    Ok(Value::Undefined)
}
"#;
    let js: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "js").map(|&(_, call, before)| (call, before)).collect();
    let found = source_messages(code, &js, &Default::default());
    assert_eq!(found, ["{what}: the object is not a File or a Folder", "File.seek: the mode must be 0, 1 or 2", "the path has an empty step"]);
}

/// The scan reads the message a `.map_err` closure returns, a literal, a `format!` string or a
/// constant, whatever the closure's parameter, from each branch of an `if` and each arm of a
/// `match`, and given to a variant of an enum whose name ends in `Fail`, and not a closure that
/// passes the error on, a `json!` value's keys, nor the steps of a script result's path
/// ([`NOT_MESSAGES`]).
#[test]
fn message_scan_reads_map_err_closures() {
    let code = r#"
fn f(text: &str, n: usize) -> Result<u32, String> {
    let doc = parse(text).map_err(|e| format!("the filter isn't valid XML: {e}"))?;
    let a = at(doc).map_err(|place| format!("[{i}]{place}"))?;
    let b = at(doc).map_err(|place| format!(".{k}{place}"))?;
    let c = at(doc).map_err(|e| format!("{name}: {e}"))?;
    let n = u32::try_from(n).map_err(|_| "too many blocks".to_string())?;
    let m = u16::try_from(n).map_err(move |(a, b)| LIMIT.into())?;
    let k = read(doc).map_err(|e| e.to_string())?;
    let j = read(doc).map_err(|e| bad(C, e))?;
    let i = read(doc).map_err(|e| if e == "{x}" { CUT_SHORT.into() } else { e })?;
    let h = read(doc).map_err(|e| match e {
        Error::Io(_) => "the file can't be read".to_string(),
        other => fail(json!({"objects": [other.to_string()]})),
    })?;
    let g = i32::try_from(n).map_err(|_| CompileFail::Syntax("the pattern is too large".into()))?;
    let f = read(doc).map_err(|e| {
        self.published = None;
        format!("the clipboard can't be read: {e}")
    })?;
    Ok(n)
}
"#;
    let consts =
        [("LIMIT", "the name is too long"), ("CUT_SHORT", "the book is cut short")].map(|(k, v)| (k.to_string(), vec![v.to_string()])).into();
    let want = [
        "the pattern is too large",
        "the filter isn't valid XML: {e}",
        "{name}: {e}",
        "too many blocks",
        "the name is too long",
        "the book is cut short",
        "the file can't be read",
        "the clipboard can't be read: {e}",
    ];
    assert_eq!(source_messages(code, &[], &consts), want);
    assert!(message_literals_in(&["color"]).contains("the color book is cut short"));
    assert!(message_literals_in(&["color"]).contains("not a swatch library: {e}"));
    assert!(message_literals_in(&["js"]).contains("regular expression too large"));
    assert!(message_literals_in(&["effects"]).contains("the SVG filter isn't valid XML: {e}"));
}

/// The scan reads the messages of an error handler that a `.map_err(name)` names: a closure bound
/// to that name, up to its `;`, and a function of that name, each of its statements.
#[test]
fn message_scan_reads_named_map_err_handlers() {
    let code = r#"
fn encode(img: &Image) -> Result<Vec<u8>, String> {
    let fail = |e: gif::EncodingError| format!("GIF encoding failed: {e}");
    let gone = move |_| GONE.to_string();
    let size = if img.is_empty() { WRONG.into() } else { 0 };
    let mut enc = Encoder::new(img).map_err(fail)?;
    enc.write(img).map_err(fail)?;
    enc.check().map_err(gone)?;
    enc.close().map_err(unreadable)?;
    enc.into_inner().map_err(Error::Io)
}

fn unreadable(e: impl std::fmt::Display) -> String {
    let e = e.to_string();
    format!("the file can't be read: {e}")
}
"#;
    let consts = [("GONE", "the encoder is gone"), ("WRONG", "the image is wrong")].map(|(k, v)| (k.to_string(), vec![v.to_string()])).into();
    let want = ["GIF encoding failed: {e}", "the encoder is gone", "the file can't be read: {e}"];
    assert_eq!(source_messages(code, &[], &consts), want);
    let render = message_literals_in(&["render"]);
    assert!(render.contains("GIF encoding failed: {e}") && render.contains("JPEG encoding failed: {e}"));
    assert!(message_literals_in(&["engine"]).contains("Data Recovery can't keep copies in {path}: {e}"));
}

/// The scan stops at a test module whose `cfg` is `test` or `all(test, …)`, and reads on past a
/// declaration of a test file and a module that is not only for tests.
#[test]
fn message_scan_skips_test_modules_with_any_test_cfg() {
    let code = "fn a() {}\n#[cfg(test)]\nmod tests_x;\n#[cfg(any(test, feature = \"x\"))]\nmod extra {}\nfn b() {}\n";
    for module in ["#[cfg(test)]\nmod tests {", "#[cfg(all(test, not(target_arch = \"wasm32\")))]\nmod tests {", "#[cfg(all( test , unix))] mod t {"]
    {
        let text = format!("{code}{module} fn c() {{}} }}\n");
        assert_eq!(without_test_module(&text), code, "{module}");
    }
    assert_eq!(without_test_module(code), code);
}

/// The limits a plug-in run can exceed, and the error of a removal that leaves the plug-in's grant
/// in the plug-in store, are translated in every language that translates every message, as the
/// status bar shows them.
#[test]
fn plugin_limit_errors_are_translated() {
    use vectorcraft_plugins::Error;
    let scanned = message_literals();
    let grant = "the plug-in's grant can't be removed from the plug-in store";
    let templates = [
        "instruction budget",
        "time budget",
        "memory budget (the selection is too large to hand over)",
        "output size ({what} is {len} bytes, limit {max})",
        grant,
    ];
    for template in templates {
        assert!(scanned.contains(template), "{template}");
    }
    let limits = [
        "instruction budget",
        "time budget",
        "memory budget (the selection is too large to hand over)",
        "output size (the result is 9000 bytes, limit 4096)",
    ];
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for limit in limits {
            let shown = message(lang, limit);
            assert_ne!(shown, limit, "{code}");
            let whole = message(lang, &Error::Limit(limit.into()).to_string());
            assert!(whole.contains(&shown) && !whole.contains("exceeded"), "{code}: {whole}");
        }
        let shown = message(lang, limits[3]);
        assert!(shown.contains("9000") && shown.contains("4096") && !shown.contains("output size"), "{code}: {shown}");
        assert_ne!(message(lang, grant), grant, "{code}");
    }
}

/// The reason Object › Path's commands are unavailable while the selection holds no path has its
/// own row in every language that translates every message, and shows translated both alone and
/// inside the error a disabled command returns.
#[test]
fn selected_path_reason_is_translated() {
    use serde_json::json;
    use vectorcraft_engine::EngineError;
    let reason = "select a path";
    assert!(message_literals().contains(reason));
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Type"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let error = match s.execute("path.reverse", &json!({})) {
        Err(e @ EngineError::Disabled(..)) => e.to_string(),
        other => panic!("a selection of type alone: {other:?}"),
    };
    assert_eq!(error, "command `path.reverse` is not available right now: select a path");
    assert_eq!(message(Lang::EN, &error), error);
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        let row = entries.iter().find(|(ctx, src, _)| ctx == "@msg" && src == reason).map(|(_, _, tr)| tr.as_str());
        let Some(row) = row else { panic!("{code}: no `@msg` row for {reason:?}") };
        assert_ne!(row, reason, "{code}");
        assert_eq!(message(lang, reason), row, "{code}");
        let shown = message(lang, &error);
        assert!(shown.contains(row) && !shown.contains(reason) && shown.contains("`path.reverse`"), "{code}: {shown}");
    }
}

/// The errors a script's `File` and `Folder` objects, `vc.run` and `$.evalFile` raise are
/// translated in every language of [`COMPLETE_MESSAGES`], and so is the status bar's line for a run
/// that ends with one: the script's name, its place, the command ids and what the script itself
/// threw stay as they are.
#[test]
fn script_errors_are_translated() {
    use serde_json::json;
    let scanned = message_literals();
    let templates = [
        "Script file access denied",
        "Script file exceeds 8 MiB",
        "Script file is not open",
        "Script file cannot be decoded",
        "Script file is not a regular file",
        "Script file does not exist",
        "Script file could not be read",
        "Script folder has more than 2,048 entries",
        "Script file open mode must be r, w, e or a",
        "Script file writing is not available",
        "Script file dialogs are not available",
        "Script files cannot be opened in another application",
        "Script file access is unavailable on the web",
        "Script has 256 files open",
        "Script file path exceeds 32,767 characters",
        "Script folder mask takes too many steps to match",
        "the script opened more file data than one run may open (32 MiB)",
        "File.seek: takes a position and a mode",
        "vc.run: give a command id, as a string",
        "vc.run: takes a command id and an object of parameters",
        "vc.run: the parameters must be an object",
        "vc.run: the result{place} is too large for a script",
        "$.evalFile requires a file path",
        "$.evalFile: the timeout must be a finite number of milliseconds, 0 or more",
        "$.evalFile: takes a file path and a timeout",
    ];
    for template in templates {
        assert!(scanned.contains(template), "{template}");
    }
    let messages: Vec<String> = templates.iter().map(|t| t.replace("{place}", ".layers[2]")).collect();
    // The status bar's lines for runs that end with one of these errors, each with the message it holds.
    let mut s = vectorcraft_engine::Session::new();
    let mut lines = vec![];
    for (source, inner) in [
        ("vc.run()", messages[18].as_str()),
        ("vc.run('view.capsLock', {}, 1)", &messages[19]),
        ("vc.run('view.capsLock', 1)", &messages[20]),
        ("$.evalFile()", &messages[22]),
        ("$.evalFile('a.jsx', -1)", &messages[23]),
        ("$.evalFile('a.jsx', 0, 1)", &messages[24]),
    ] {
        let e = s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap_err().to_string();
        assert!(e.starts_with("t.js: line ") && e.contains(inner) && e.ends_with(" (thrown)"), "{source}: {e}");
        lines.push((e, Some(inner.to_string())));
    }
    // A `File` gives why it could not open as its `error`, which the script throws.
    let source = "var f = new File('/vc-no-such-folder/a.txt');\nif (!f.open('r')) throw new Error(f.error);";
    let e = s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap_err().to_string();
    let inner = messages[..17].iter().find(|m| e.ends_with(&format!(": Error: {m} (thrown)"))).unwrap_or_else(|| panic!("{e}"));
    lines.push((e, Some(inner.clone())));
    let thrown = s.execute("script.run", &json!({"source": "throw new Error('mine')", "name": "t.js"})).unwrap_err().to_string();
    assert_eq!(thrown, "t.js: line 1, column 1: Error: mine (thrown)");
    lines.push((thrown, None));
    // A script that does not compile, and an error raised in code `eval` compiled, which has no place.
    let syntax = s.execute("script.run", &json!({"source": "var = 1;", "name": "t.js"})).unwrap_err().to_string();
    assert!(syntax.starts_with("t.js: line 1, column ") && syntax.ends_with(" (syntax)"), "{syntax}");
    lines.push((syntax, None));
    let source = format!("eval(\"throw new Error('{}')\");", messages[18]);
    let placeless = s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap_err().to_string();
    assert_eq!(placeless, format!("t.js: Error: {} (thrown)", messages[18]));
    lines.push((placeless, Some(messages[18].clone())));
    lines.push((format!("t.js: line 7, column 2: {} (stopped)", messages[16]), Some(messages[16].clone())));
    lines.push((format!("t.js: line 7, column 2 of lib.jsx: {} (internal)", messages[16]), Some(messages[16].clone())));
    lines.push((format!("t.js: {} (internal)", messages[16]), Some(messages[16].clone())));
    // A run that ends at a limit, in a file another one included, with files left written.
    lines.push((format!("t.js: line 4, column 9 of lib.jsx: {} (limit)", messages[16]), Some(messages[16].clone())));
    lines.push((format!("t.js: {} (limit)", messages[16]), Some(messages[16].clone())));
    lines.push((format!("t.js: line 3, column 1 of lib.jsx: TypeError: {} (thrown)", messages[0]), Some(messages[0].clone())));
    lines.push((format!("t.js: TypeError: {} (thrown)", messages[0]), Some(messages[0].clone())));
    let kept = format!("t.js: line 2, column 5: Error: {} (thrown) (these files stay written: /tmp/a.txt, /tmp/b.txt)", messages[13]);
    lines.push((kept, Some(messages[13].clone())));
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for m in &messages {
            let shown = message(lang, m);
            assert_ne!(shown, *m, "{code}");
            assert!(["vc.run", "$.evalFile", "File.seek"].iter().all(|id| !m.contains(id) || shown.contains(id)), "{code}: {shown}");
        }
        assert!(message(lang, &messages[21]).contains(".layers[2]"), "{code}");
        for (line, inner) in &lines {
            let shown = message(lang, line);
            let kinds = ["(thrown)", "(limit)", "(syntax)", "(stopped)", "(internal)", "stay written"];
            assert!(kinds.iter().all(|k| !shown.contains(k)), "{code}: {shown}");
            let numbers: Vec<&str> = line.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).collect();
            assert!(shown.starts_with("t.js") && numbers.iter().all(|n| shown.contains(n)), "{code}: {line} -> {shown}");
            match inner {
                Some(inner) => assert!(shown.contains(&message(lang, inner)), "{code}: {line} -> {shown}"),
                None if line.ends_with("(syntax)") => assert_ne!(shown, *line, "{code}"),
                None => assert!(shown.contains("Error") && shown.contains("mine"), "{code}: {shown}"),
            }
            for kept in ["lib.jsx", "/tmp/a.txt, /tmp/b.txt"] {
                assert!(!line.contains(kept) || shown.contains(kept), "{code}: {shown}");
            }
        }
    }
}

/// The errors a script's `$` object, its `alert`, `confirm` and `prompt` dialogs, `vc.document`,
/// `vc.log`, the methods of its `File` and `Folder` objects and a run's limits give, and the errors
/// of the script engine commands, are translated in every language of [`COMPLETE_MESSAGES`], on
/// their own and in the status bar's line for a run that ends with one. Names, numbers and the
/// ids of functions stay as they are.
#[test]
fn script_host_errors_are_translated() {
    use serde_json::json;
    let scanned = message_literals();
    let templates = [
        "$.{name}: give a name, as a string",
        "$.locale: give a locale such as \"en\" or \"de_DE\", or null for the interface's",
        "$.localize: give true or false",
        "$.flags: only 512, which turns localization on, or 0 can be set",
        "$.flags: give a number",
        "$.appEncoding: scripts here read and write UTF-8 only",
        "$.appEncoding: give an encoding's name, as a string",
        "$.memCache: the memory cache size cannot be set",
        "$.setenv: give a value, as a string",
        "$.setenv: only {_1} can be set",
        "$.setenv: a value has at most {MAX_ENV_BYTES} letters, digits and _ . - @",
        "$.sleep: give a number of milliseconds from 0 to {MAX_SLEEP_MS}",
        "$.sleep: give a number of milliseconds",
        "$.sleep: a run cannot wait inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code",
        "$.bp: scripts here have no debugger",
        "$.colorPicker: scripts cannot show a dialog yet",
        "{name}: give a message",
        "{name}: give the text the field starts with",
        "{name}: the message is longer than 16 KiB",
        "{name}: the title is longer than 256 bytes",
        "{name}: the text the field starts with is longer than 16 KiB",
        "{name}: a dialog cannot open inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code",
        "{name}: the next answer in `answers` is for {_1}, not {name}",
        "{name}: `answers` has no answer left for this dialog",
        "vc.document{place} is too large for a script",
        "vc.log: the log of this run is full",
        "this function's script run has ended",
        "the script made more host calls than one run may make",
        "the script passed more data to and from commands than one run may pass",
        "the script added more objects and data to the documents than one run may add",
        "{what}: the object is not a File or a Folder",
        "{what}: the count must be a finite number, 0 or more",
        "File.seek: the position must be a finite number",
        "File.seek: the mode must be 0, 1 or 2",
        "script file is compiled: {value}",
        "`name` must be the name of a script engine",
        "the main script engine is new for each run and is never kept, so it cannot be dropped",
        "there is no script engine named {name}",
    ];
    for template in templates {
        assert!(scanned.contains(template), "{template}");
    }
    let mut s = vectorcraft_engine::Session::new();
    // (what the line holds, what that is as the script's error, what the catalogs must keep)
    let mut lines: Vec<(String, String, Vec<&str>)> = vec![];
    for (source, inner, kept) in [
        ("$.getenv(1)", "$.getenv: give a name, as a string", vec!["$.getenv"]),
        ("$.setenv('LANG', 1)", "$.setenv: give a value, as a string", vec!["$.setenv"]),
        ("$.setenv('HOME', 'x')", "$.setenv: only LANG, LC_MESSAGES, LC_ALL, OS can be set", vec!["$.setenv", "LANG, LC_MESSAGES, LC_ALL, OS"]),
        ("$.setenv('LANG', 'a b')", "$.setenv: a value has at most 64 letters, digits and _ . - @", vec!["$.setenv", "64", "_ . - @"]),
        ("$.sleep('x')", "$.sleep: give a number of milliseconds", vec!["$.sleep"]),
        ("$.sleep(5000)", "$.sleep: give a number of milliseconds from 0 to 1000", vec!["$.sleep", "1000"]),
        ("$.flags = 3", "$.flags: only 512, which turns localization on, or 0 can be set", vec!["$.flags", "512"]),
        ("$.flags = 'x'", "$.flags: give a number", vec!["$.flags"]),
        ("$.localize = 1", "$.localize: give true or false", vec!["$.localize"]),
        ("$.locale = 1", templates[1], vec!["$.locale", "\"en\"", "\"de_DE\""]),
        ("$.appEncoding = 'latin1'", templates[5], vec!["$.appEncoding", "UTF-8"]),
        ("$.appEncoding = 1", templates[6], vec!["$.appEncoding"]),
        ("$.memCache = 1", templates[7], vec!["$.memCache"]),
        ("$.bp()", templates[14], vec!["$.bp"]),
        ("$.colorPicker()", templates[15], vec!["$.colorPicker"]),
        ("File.prototype.seek.call({})", "seek: the object is not a File or a Folder", vec!["seek", "File"]),
        ("new File('a.txt').seek(NaN)", templates[32], vec!["File.seek"]),
        ("new File('a.txt').seek(0, 5)", templates[33], vec!["File.seek", "0, 1", "2"]),
        ("new File('a.txt').read(-1)", "File.read: the count must be a finite number, 0 or more", vec!["File.read", "0"]),
    ] {
        let e = s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap_err().to_string();
        assert!(e.starts_with("t.js: ") && e.contains(&format!(": {inner} (")), "{source}: {e}");
        lines.push((e, inner.to_string(), kept));
    }
    let in_callback = templates[21].replace("{name}", "alert");
    for (inner, kept) in [
        ("alert: give a message", vec!["alert"]),
        ("prompt: give the text the field starts with", vec!["prompt"]),
        ("confirm: the message is longer than 16 KiB", vec!["confirm", "16"]),
        ("alert: the title is longer than 256 bytes", vec!["alert", "256"]),
        ("prompt: the text the field starts with is longer than 16 KiB", vec!["prompt", "16"]),
        (in_callback.as_str(), vec!["alert", "$.evalFile"]),
        ("confirm: the next answer in `answers` is for prompt, not confirm", vec!["`answers`", "prompt", "confirm"]),
        ("alert: `answers` has no answer left for this dialog", vec!["`answers`", "alert"]),
        (templates[13], vec!["$.sleep", "$.evalFile"]),
        ("vc.document.layers[0] is too large for a script", vec!["vc.document.layers[0]"]),
        (templates[25], vec!["vc.log"]),
        (templates[26], vec![]),
        (templates[27], vec![]),
        (templates[28], vec![]),
        (templates[29], vec![]),
        ("script file is compiled: lib.jsxbin", vec!["lib.jsxbin"]),
    ] {
        let line = format!("t.js: line 3, column 7: Error: {inner} (thrown)");
        lines.push((line, inner.to_string(), kept));
    }
    let mut commands = vec![];
    for (id, params) in
        [("script.engine.drop", json!({})), ("script.engine.drop", json!({"name": "main"})), ("script.engine.drop", json!({"name": "kept"}))]
    {
        commands.push(s.execute(id, &params).unwrap_err().to_string());
    }
    assert!(commands[0].contains(templates[35]) && commands[1].contains(templates[36]), "{commands:?}");
    assert!(commands[2].contains("there is no script engine named \"kept\""), "{commands:?}");
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        for (line, inner, kept) in &lines {
            let shown = message(lang, inner);
            assert_ne!(shown, *inner, "{code}");
            assert!(kept.iter().all(|k| shown.contains(k)), "{code}: {inner} -> {shown}");
            let whole = message(lang, line);
            let kinds = ["(thrown)", "(limit)", "(syntax)", "(stopped)", "(internal)"];
            assert!(whole.starts_with("t.js") && whole.contains(&shown) && kinds.iter().all(|k| !whole.contains(k)), "{code}: {line} -> {whole}");
        }
        for e in &commands {
            let shown = message(lang, e);
            assert_ne!(shown, *e, "{code}");
            assert!(
                !shown.contains("must be the name") && !shown.contains("is never kept") && !shown.contains("there is no script engine"),
                "{code}: {shown}"
            );
        }
        assert!(message(lang, &commands[2]).contains("\"kept\""), "{code}");
    }
}

/// A manifest whose `commands` value has the wrong shape is refused with an error that every
/// language of [`COMPLETE_MESSAGES`] translates whole, the command's name and the place in the
/// manifest included.
#[test]
fn wrongly_shaped_plugin_commands_errors_are_translated() {
    let many: Vec<String> = (0..=32).map(|i| format!(r#"{{"id":"c{i}","label":"C"}}"#)).collect();
    let commands = [
        r#"{"id":"x"}"#.to_string(),
        r#""x""#.to_string(),
        "[1]".to_string(),
        r#"[{"id":1,"label":"A"}]"#.to_string(),
        r#"[{"id":"x","label":true}]"#.to_string(),
        r#"[{"label":"B"}]"#.to_string(),
        r#"[{"id":"x"}]"#.to_string(),
        r#"[{"id":"x","id":"y","label":"A"}]"#.to_string(),
        format!("[{}]", many.join(",")),
    ];
    let english = ["must", "list", "missing", "field", "duplicate", "object", "line", "column", "each", "with", "invalid", "expected"];
    for value in &commands {
        let manifest = format!(r#"{{"id":"a","name":"x","kind":"filter","commands":{value}}}"#);
        let e = vectorcraft_plugins::manifest::Manifest::parse(manifest.as_bytes()).unwrap_err().to_string();
        let numbers: Vec<&str> = e.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).collect();
        for code in COMPLETE_MESSAGES {
            let shown = message(Lang::from_code(code).expect("registered"), &e);
            // Backticked names stay in English; the words around them are translated.
            let words: String = shown.split('`').step_by(2).collect();
            assert!(words.split(|c: char| !c.is_alphabetic()).all(|w| !english.contains(&w)), "{code}: {e} -> {shown}");
            assert!(numbers.iter().all(|n| shown.contains(n)), "{code}: {e} -> {shown}");
        }
    }
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
        (s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": -1})).unwrap_err().to_string(), "`zoom`", "a number > 0", "-1"),
        (s.execute("view.saved.new", &json!({"center": 5, "zoom": 1})).unwrap_err().to_string(), "`center`", "two numbers", "5"),
        (
            s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 1, "rotation": "x"})).unwrap_err().to_string(),
            "`rotation`",
            "a number (deg)",
            "\"x\"",
        ),
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

/// The phrases a parameter schema's error uses for what a value must be, as `@msg` sources: one
/// for each type, and for a number each range and unit form.
const SCHEMA_PHRASES: &[&str] = &[
    "a number",
    "a number >= {_1}",
    "a number > {_1}",
    "a number <= {_1}",
    "a number < {_1}",
    "a number {_1} to {_2}",
    "a number >= {_1} and < {_2}",
    "a number > {_1} and <= {_2}",
    "a number > {_1} and < {_2}",
    "a number ({_1})",
    "a number >= {_1} ({_2})",
    "a number > {_1} ({_2})",
    "a number <= {_1} ({_2})",
    "a number < {_1} ({_2})",
    "a number {_1} to {_2} ({_3})",
    "a number >= {_1} and < {_2} ({_3})",
    "a number > {_1} and <= {_2} ({_3})",
    "a number > {_1} and < {_2} ({_3})",
    "an integer",
    "an integer >= {_1}",
    "an integer <= {_1}",
    "an integer {_1} to {_2}",
    "true or false",
    "a string",
    "one of {_1}",
    "an object id (a non-negative integer)",
    "a list of object ids",
    "a color (a string, [r, g, b] or an object)",
    "[x, y] (two numbers)",
    "an object",
    "a list",
];

/// What a value of each parameter type must be is translated in every language that translates
/// every message, for every range and unit of a number: the matched row's placeholders take the
/// bounds, the unit and the names exactly as the English gives them, and the whole error keeps
/// the parameter's path and the refused value.
#[test]
fn parameter_schema_expected_phrases_are_translated() {
    use serde_json::{Value, json};
    use vectorcraft_apischema::{Bound, Param, ParamSchema, Problem, Type};
    let number = |min, max, unit| Type::Number { min, max, unit };
    let (incl, excl) = (|x| Some(Bound::Inclusive(x)), |x| Some(Bound::Exclusive(x)));
    // (type, a value it refuses, the placeholder values of the phrase's row)
    let cases: Vec<(Type, Value, Vec<&str>)> = vec![
        (Type::NUMBER, json!("x"), vec![]),
        (number(incl(-2.5), None, None), json!(-3), vec!["-2.5"]),
        (number(excl(0.0), None, None), json!(0), vec!["0"]),
        (number(None, incl(1.5), None), json!(2), vec!["1.5"]),
        (number(None, excl(100.0), None), json!(100), vec!["100"]),
        (number(incl(-1.0), incl(1.0), None), json!(2), vec!["-1", "1"]),
        (number(incl(0.0), excl(360.0), None), json!(360), vec!["0", "360"]),
        (number(excl(0.0), incl(1.0), None), json!(0), vec!["0", "1"]),
        (number(excl(-0.5), excl(0.5), None), json!(1), vec!["-0.5", "0.5"]),
        (number(None, None, Some("deg")), json!("x"), vec!["deg"]),
        (number(incl(0.25), None, Some("pt")), json!(0), vec!["0.25", "pt"]),
        (number(excl(0.0), None, Some("pt")), json!(0), vec!["0", "pt"]),
        (number(None, incl(-1.0), Some("mm")), json!(0), vec!["-1", "mm"]),
        (number(None, excl(10.0), Some("px")), json!(10), vec!["10", "px"]),
        (number(incl(0.0), incl(100.0), Some("%")), json!(101), vec!["0", "100", "%"]),
        (number(incl(-180.0), excl(180.0), Some("deg")), json!(180), vec!["-180", "180", "deg"]),
        (number(excl(0.0), incl(1000.0), Some("pt")), json!(0), vec!["0", "1000", "pt"]),
        (number(excl(-1.0), excl(1.0), Some("in")), json!(1), vec!["-1", "1", "in"]),
        (Type::Integer { min: None, max: None }, json!(1.5), vec![]),
        (Type::Integer { min: Some(1), max: None }, json!(0), vec!["1"]),
        (Type::Integer { min: None, max: Some(-1) }, json!(0), vec!["-1"]),
        (Type::Integer { min: Some(1), max: Some(25) }, json!(26), vec!["1", "25"]),
        (Type::Boolean, json!(1), vec![]),
        (Type::String, json!(1), vec![]),
        (Type::Enum(&["left", "center", "right"]), json!("top"), vec![r#""left", "center", "right""#]),
        (Type::Enum(&["only"]), json!("other"), vec![r#""only""#]),
        (Type::Id, json!(-1), vec![]),
        (Type::Ids, json!(5), vec![]),
        (Type::Color, json!(5), vec![]),
        (Type::Point, json!(5), vec![]),
        (Type::Object(&[]), json!(5), vec![]),
        (Type::Array(&Type::NUMBER), json!(5), vec![]),
    ];
    let one = |name: &'static str, ty: Type| ParamSchema::new(Box::leak(Box::new([Param::new(name, ty)])), "");
    let inner: &'static [Param] = Box::leak(Box::new([Param::new("zoom", number(excl(0.0), None, None))]));
    let nested = one("edits", Type::Array(Box::leak(Box::new(Type::Object(inner)))));
    let mut errors: Vec<_> = cases.iter().map(|(ty, bad, caps)| (one("x", *ty).validate(&json!({ "x": bad })), caps.clone())).collect();
    errors.push((nested.validate(&json!({"edits": [{"zoom": 1}, {"zoom": -1}]})), vec!["0"]));
    let mut seen = std::collections::BTreeSet::new();
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        for phrase in SCHEMA_PHRASES {
            assert!(entries.iter().any(|(ctx, src, _)| ctx == "@msg" && src == phrase), "{code}: no row for {phrase:?}");
        }
        for (result, caps) in &errors {
            let Err(e) = result else { panic!("accepted: {result:?}") };
            let Problem::WrongType { expected, got } = &e.problem else { panic!("{e}") };
            seen.insert(expected.clone());
            let (_, matched) = lang.catalog().message(expected).unwrap_or_else(|| panic!("{code}: no row matches {expected:?}"));
            let values: Vec<&str> = matched.iter().map(|(_, v)| *v).collect();
            assert_eq!(&values, caps, "{code}: {expected:?}");
            let phrase = message(lang, expected);
            assert_ne!(&phrase, expected, "{code}");
            let mut from = 0;
            for value in caps {
                let at = phrase[from..].find(value).unwrap_or_else(|| panic!("{code}: {value:?} in {expected:?} -> {phrase:?}"));
                from += at + value.len();
            }
            let shown = message(lang, &e.to_string());
            let path = format!("`{}`", e.path);
            let kept = caps.iter().all(|value| shown.contains(value));
            assert!(kept && shown.contains(&path) && shown.contains(got.as_str()), "{code}: {shown}");
            assert!(!holds_word(&shown, expected) && !shown.contains("must be") && !shown.contains(", not "), "{code}: {shown}");
        }
    }
    // A catalog's `{key}` must be one of {_1}` row would take `"a", "b", not "c"` as its list.
    let enum_error = one("x", Type::Enum(&["a", "b"])).validate(&json!({"x": "c"})).map_or_else(|e| e.to_string(), |_| String::new());
    assert_eq!(enum_error, r#"`x` must be one of "a", "b", not "c""#);
    for lang in Lang::all().filter(|l| l.code() != Lang::EN.code() && !l.0.source.is_empty()) {
        let shown = message(lang, &enum_error);
        assert!(!shown.contains(", not ") && shown.contains(r#""a", "b""#) && shown.contains(r#""c""#), "{}: {shown}", lang.code());
    }
    for phrase in
        ["a number > -0.5 and < 0.5", "a number 0 to 100 (%)", "an integer 1 to 25", r#"one of "left", "center", "right""#, "[x, y] (two numbers)"]
    {
        assert!(seen.contains(phrase), "{phrase:?} not in {seen:?}");
    }
}

/// A JSON object or array inside a translated message stays as it is, and an exact message that
/// starts with a bracket is still translated.
#[test]
fn bracketed_message_parts_translate_only_as_exact_messages() {
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let point = message(lang, "[x, y] (two numbers)");
        assert_ne!(point, "[x, y] (two numbers)", "{code}");
        assert!(point.contains("[x, y]"), "{code}: {point}");
        for got in ["[1]", "[1, 2, 3]", r#"{"objects": [1]}"#, r#"[{"a": 1}, "b: c", …"#] {
            let shown = message(lang, &format!("`center` must be [x, y] (two numbers), not {got}"));
            assert!(shown.contains(&point) && shown.contains(got), "{code}: {shown}");
            let shown = message(lang, &format!("`p` must be an object, not {got}"));
            assert!(shown.contains(got) && !shown.contains("an object"), "{code}: {shown}");
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
    ("bevel must be none or one of {_1}", &["bevel", "none"]),
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
    ("light behind must be true or false", &["behind"]),
    ("light color must use opaque #rrggbb", &["color"]),
    ("lightColor must be an opaque RGB color in #rrggbb form", &["lightColor"]),
    ("lights must contain one to ten lights", &["lights"]),
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
    ("Revolve cap must be a boolean", &["cap"]),
    ("Revolve edge must be left or right", &["edge", "left", "right"]),
    ("scale must be a number from {_1} to {_2}, not {_3}", &["scale"]),
    ("scale must be a number from {_1} to {_2}, not {s}", &["scale"]),
    ("shadow must be true or false", &["shadow"]),
    ("shadowPosition must be behind or below", &["shadowPosition", "behind", "below"]),
    ("show must be true or false", &["show"]),
    ("span: \"selection\" places a linear, radial, conic or diamond gradient, not a freeform one", &["span"]),
    ("thickness must be from 0.01 to 5000 pt", &["thickness"]),
    ("title can't be empty", &["title"]),
    (
        "unknown light setting: use color, intensity, rotation, height, softness or behind",
        &["color", "intensity", "rotation", "height", "softness", "behind"],
    ),
    ("view is required", &["view"]),
    ("width must be positive", &["width"]),
    ("zone must be a string", &["zone"]),
    ("zone must be tab, center, left, right, top or bottom", &["zone", "tab", "center", "left", "right", "top", "bottom"]),
    ("`dither` applies to gradient fills (stroke: false)", &["stroke"]),
    ("`strokeMode` applies to strokes (stroke: true)", &["stroke"]),
    ("`{GRAPHICS}` entries take symbol, face, x, y, scale and rotation, not `{k}`", &["symbol", "face", "x", "y", "scale", "rotation"]),
    ("`{GRAPHICS}` face must be front or back", &["face", "front", "back"]),
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

/// The command ids that `s` names outside its backticked spans, such as `assets.list` or
/// `window.panel.dock`: words of two or more dot-separated parts, each an ASCII letter or digit
/// run that starts with a lowercase letter.
fn command_ids(s: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    for part in s.split('`').step_by(2) {
        for word in part.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_')) {
            let word = word.trim_end_matches('.');
            let segments: Vec<&str> = word.split('.').collect();
            let dotted = segments.len() > 1
                && segments
                    .iter()
                    .all(|seg| seg.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && seg.chars().all(|c| c.is_ascii_alphanumeric()));
            if dotted && word != "e.g" && word != "i.e" {
                ids.push(word);
            }
        }
    }
    ids
}

#[test]
fn command_ids_finds_dotted_ids_outside_backticks() {
    assert_eq!(command_ids("no slice {n} in the image (see slice.list)"), ["slice.list"]);
    assert_eq!(command_ids("use window.panel.dock."), ["window.panel.dock"]);
    assert_eq!(command_ids("output.destination: {e}"), ["output.destination"]);
    assert_eq!(command_ids("(assets.list.old देखें)"), ["assets.list.old"]);
    assert_eq!(command_ids("see `plugin.list`, e.g. 1.5 or {name}.svg"), Vec::<&str>::new());
}

/// A translated status or error message keeps each command id its English text names without
/// backticks, spelled as the English spells it, since that is the id a script or the command
/// palette runs.
#[test]
fn translated_messages_keep_the_command_ids_their_english_names() {
    let mut missing = Vec::new();
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg" || ctx == "@msgplural") {
            let plural = ctx == "@msgplural";
            let english = if plural { src.rsplit('|').next().unwrap_or(src) } else { src.as_str() };
            let forms: Vec<&str> = if plural { tr.split('|').collect() } else { vec![tr.as_str()] };
            for form in forms {
                let kept: Vec<&str> = command_ids(form).into_iter().chain(backticked(form)).collect();
                for id in command_ids(english) {
                    if !kept.contains(&id) {
                        missing.push(format!("{}: {id} in {src:?} -> {form:?}", l.code));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} rows drop a command id their English names:\n{}", missing.len(), missing.join("\n"));
}

/// Whether `word` is an ASCII name a command reads: a letter followed by letters, digits or
/// underscores.
fn is_ascii_name(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The field names and accepted values that `s` spells out of its backticked spans: each name of a
/// comma-separated list in square brackets, such as `[x, y, width, height]` or
/// `[[subpath, anchor]…]`; each `a|b|c` list of values, whole; and the values an `unknown …` or
/// `bad model …` message lists in parentheses, such as `(none, standard or wedges)`, up to the first
/// item that is not a single name, so `(high, medium, low or a saved one: see …)` gives `high`,
/// `medium` and `low`, and `(or give its ids)` gives none.
fn field_names_and_values(s: &str) -> Vec<String> {
    let outside = s.split('`').step_by(2).collect::<Vec<_>>().join(" ");
    let mut found = Vec::new();
    for piece in outside.split('[').skip(1) {
        let Some(end) = piece.find(']') else { continue };
        let names: Vec<&str> = piece[..end].split(',').map(str::trim).collect();
        if names.len() > 1 && names.iter().all(|name| is_ascii_name(name)) {
            found.extend(names.into_iter().map(str::to_string));
        }
    }
    let lists = outside.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '|'));
    found.extend(lists.filter(|w| w.contains('|') && w.split('|').all(is_ascii_name)).map(str::to_string));
    for (at, _) in outside.match_indices('(') {
        let before = &outside[..at];
        if !(holds_ascii_word(before, "unknown") || before.contains("bad model")) {
            continue;
        }
        let Some(end) = outside[at..].find(')') else { continue };
        let items = outside[at + 1..at + end].split(", ").flat_map(|item| item.split(" or "));
        let values: Vec<&str> = items.map(str::trim).take_while(|item| is_ascii_name(item)).collect();
        if values.len() > 1 {
            found.extend(values.into_iter().map(str::to_string));
        }
    }
    found
}

#[test]
fn field_names_and_values_finds_bracketed_names_and_listed_values() {
    assert_eq!(field_names_and_values("`{key}` must be [x, y, width, height]: four numbers"), ["x", "y", "width", "height"]);
    assert_eq!(field_names_and_values("missing anchors [[subpath, anchor]…]"), ["subpath", "anchor"]);
    assert_eq!(field_names_and_values("arrowScale must be [start, end]"), ["start", "end"]);
    assert_eq!(field_names_and_values("unknown pieLegend {v} (none, standard or wedges)"), ["none", "standard", "wedges"]);
    assert_eq!(field_names_and_values("unknown side `{other}` (left|right|top|bottom)"), ["left|right|top|bottom"]);
    assert_eq!(field_names_and_values("bad model {other} (keep, new or document)"), ["keep", "new", "document"]);
    assert_eq!(
        field_names_and_values("unknown preset `{name}` (high, medium, low or a saved one: see flattener.presets.list)"),
        ["high", "medium", "low"]
    );
    assert_eq!(field_names_and_values("unknown object (or give its ids)"), Vec::<String>::new());
    assert_eq!(field_names_and_values("unknown color `{c}` (#rrggbb or a preset name)"), Vec::<String>::new());
    assert_eq!(field_names_and_values("unknown resolution (72 screen, 150 medium, 300 high)"), Vec::<String>::new());
    assert_eq!(field_names_and_values("the art is [1, 2] (left, right) and `[a, b]`"), Vec::<String>::new());
}

/// A translated status or error message keeps each field name and accepted value its English text
/// spells out (see [`field_names_and_values`]), as the English spells it: the message tells a user
/// what to pass, and a command reads only the English names and values.
#[test]
fn translated_messages_keep_the_field_names_and_values_their_english_lists() {
    let mut missing = Vec::new();
    for l in &LANGUAGES {
        let (entries, _) = parse_entries(l.source);
        for (ctx, src, tr) in entries.iter().filter(|(ctx, _, _)| ctx == "@msg" || ctx == "@msgplural") {
            let plural = ctx == "@msgplural";
            let english = if plural { src.rsplit('|').next().unwrap_or(src) } else { src.as_str() };
            let forms: Vec<&str> = if plural { tr.split('|').collect() } else { vec![tr.as_str()] };
            for form in forms {
                for name in field_names_and_values(english) {
                    if !holds_ascii_word(form, &name) {
                        missing.push(format!("{}: {name} in {src:?} -> {form:?}", l.code));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} rows drop a field name or value their English lists:\n{}", missing.len(), missing.join("\n"));
}

/// Languages whose catalogs translate status and error messages; [`untranslated_report`] lists
/// the messages each one lacks.
const COMPLETE_MESSAGES: &[&str] = &[
    "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr", "ca",
    "nl",
];

/// Crates whose error and status messages reach the status bar, the crates whose errors the
/// engine and the effects pass on included (effects, swatch libraries, raster sizes and encoders,
/// 3D solids, path operations).
const MESSAGE_CRATES: &[&str] = &[
    "ui-egui", "engine", "doc", "format", "svg", "pdf", "eps", "text", "plugins", "metafile", "cad", "trace", "js", "effects", "color", "render",
    "three-d", "pathops",
];

/// Where a message literal starts: a status, an error value, a `thiserror` message.
const MESSAGE_MARKERS: &[&str] = &[".status(", "ui.status = ", "status = ", "Other(", "Err(", "ok_or(", "ok_or_else(|| ", "#[error("];

/// Helpers that make an error from a message, by crate, with the number of arguments before the
/// message: the engine's `bad(cmd, msg)` (`EngineError::BadParams`), `err(msg)` (`cmd/fileio`)
/// and `error(msg)` (`cmd/dimension.rs`), the script host's `fail(msg)` (`script/files.rs`,
/// `script/dollar.rs`) and `value(msg)`, which gives the messages the script `File` object's own
/// code throws (`script/files.rs`), and `.over(limit)` for a limit a run passes (`script/mod.rs`),
/// the PDF writer's `bad(why)` (`forms.rs`) and `failed(why)` (`encrypt.rs`, `linearize.rs`), the
/// plug-in host's `bad(what)` (a parameter's spec and dialog keys), `wrong(what)` (a parameter's
/// value), `out_err(at, msg)` (plug-in output), `in_range(x, lo, hi, at, what)` and
/// `unit(x, at, what)` (a painted effect's numbers) and `.read_packed(packed, what, max)` (the block
/// a plug-in returned), and the script interpreter's methods that throw an error at a script:
/// `.type_error(msg)`, `.throw(kind, msg)` and the XPath evaluator's `.error(msg)`. A helper that
/// starts with `.` is a method of any receiver.
const ERROR_HELPERS: &[(&str, &str, usize)] = &[
    ("engine", "bad(", 1),
    ("engine", "err(", 0),
    ("engine", "error(", 0),
    ("engine", "fail(", 0),
    ("engine", "value(", 0),
    ("engine", ".over(", 0),
    ("pdf", "bad(", 0),
    ("pdf", "failed(", 0),
    ("plugins", "bad(", 0),
    ("plugins", "wrong(", 0),
    ("plugins", "out_err(", 1),
    ("plugins", "in_range(", 4),
    ("plugins", "unit(", 2),
    ("plugins", ".read_packed(", 1),
    ("js", ".type_error(", 0),
    ("js", ".throw(", 1),
    ("js", ".error(", 0),
];

/// Error variants whose first argument is a name, not a message: a command id
/// (`EngineError::UnknownCommand`) and a PostScript error name (`PsError::Ps`).
const NAME_VARIANTS: &[&str] = &["UnknownCommand", "Ps"];

/// Error variants whose message follows other arguments, with the number of arguments before it:
/// `EngineError::Disabled(command id, reason)`.
const LATER_MESSAGE_VARIANTS: &[(&str, usize)] = &[("Disabled", 1)];

/// Messages that are signals, not text for people, the name of a plug-in export that an error
/// message holds (`vc_manifest points outside memory`), and the steps of the path to a script
/// result's part that an error message holds (`vc.run: the result{place} is too large for a script`).
const NOT_MESSAGES: &[&str] = &["quit", "cancelled", "vc_manifest", "[{i}]{place}", ".{k}{place}"];

/// Every status and error message literal in [`MESSAGE_CRATES`] (test modules aside), as catalog
/// templates: `{}` becomes `{_1}`, `{_2}` … and format specs are dropped (`{n:?}` → `{n}`).
pub fn message_literals() -> std::collections::BTreeSet<String> {
    message_literals_in(MESSAGE_CRATES)
}

/// The message literals of [`message_literals`] in the crates `names` alone.
fn message_literals_in(names: &[&str]) -> std::collections::BTreeSet<String> {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = std::collections::BTreeSet::new();
    let mut files = vec![];
    let mut stack: Vec<_> = names.iter().map(|c| crates.join(c).join("src")).collect();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs")
                && !name.starts_with("tests")
                && !name.ends_with("_tests.rs")
                && !(d == crates.join("ui-egui").join("src") && name == "control.rs")
            {
                // Control replies are English protocol data, never status-bar messages
                // (see i18n's module contract). Engine messages are scanned in their own crates.
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                files.push((path, text));
            }
        }
    }
    let crate_of = |path: &std::path::Path| names.iter().copied().find(|c| path.starts_with(crates.join(c))).unwrap_or("");
    let declared: Vec<_> = files.iter().map(|(path, text)| (crate_of(path), str_consts(without_test_module(text)))).collect();
    let mut by_crate: std::collections::BTreeMap<&str, std::collections::BTreeMap<String, Vec<String>>> = std::collections::BTreeMap::new();
    let mut everywhere: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for (krate, consts) in &declared {
        let in_crate = by_crate.entry(*krate).or_default();
        for (name, value) in consts {
            in_crate.entry(name.clone()).or_default().push(value.clone());
            everywhere.entry(name.clone()).or_default().push(value.clone());
        }
    }
    let none = std::collections::BTreeMap::new();
    let crate_consts: std::collections::BTreeMap<&str, _> =
        by_crate.iter().map(|(krate, in_crate)| (*krate, scoped_consts(&[], in_crate, &everywhere).into_owned())).collect();
    for ((path, text), (krate, own)) in files.iter().zip(&declared) {
        let helpers: Vec<_> =
            ERROR_HELPERS.iter().filter(|(c, _, _)| path.starts_with(crates.join(c))).map(|&(_, call, before)| (call, before)).collect();
        let consts = scoped_consts(own, crate_consts.get(krate).unwrap_or(&none), &none);
        out.extend(source_messages(without_test_module(text), &helpers, &consts));
    }
    out
}

/// The `&str` constants a file can name, from the constants it declares (`own`), those the files
/// of its crate declare (`in_crate`) and those every scanned crate declares (`everywhere`): a name
/// the file declares has the file's own text only, a name its crate declares has every text the
/// crate's files give it, and any other name has every text the scanned crates give it.
fn scoped_consts<'a>(
    own: &[(String, String)],
    in_crate: &'a std::collections::BTreeMap<String, Vec<String>>,
    everywhere: &std::collections::BTreeMap<String, Vec<String>>,
) -> std::borrow::Cow<'a, std::collections::BTreeMap<String, Vec<String>>> {
    let elsewhere = everywhere.keys().any(|name| !in_crate.contains_key(name));
    let own_differs = own.iter().any(|(name, _)| {
        let mine: Vec<&String> = own.iter().filter(|(n, _)| n == name).map(|(_, v)| v).collect();
        in_crate.get(name).is_none_or(|values| values.iter().collect::<Vec<_>>() != mine)
    });
    if !elsewhere && !own_differs {
        return std::borrow::Cow::Borrowed(in_crate);
    }
    let mut consts = everywhere.clone();
    for (name, values) in in_crate {
        consts.insert(name.clone(), values.clone());
    }
    for (name, _) in own {
        consts.remove(name);
    }
    for (name, value) in own {
        consts.entry(name.clone()).or_default().push(value.clone());
    }
    std::borrow::Cow::Owned(consts)
}

/// The `&str` constants `code` declares outside its comments (`const NAME: &str = "…";`), with
/// their text, escapes resolved.
fn str_consts(code: &str) -> Vec<(String, String)> {
    let code = &without_comments(code);
    code.match_indices("const ")
        .filter(|&(at, _)| !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        .filter_map(|(at, m)| {
            let rest = &code[at + m.len()..];
            let (name, rest) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')?);
            let rest = rest.trim_start().strip_prefix(':')?.trim_start();
            let rest = rest.strip_prefix("&'static str").or_else(|| rest.strip_prefix("&str"))?.trim_start();
            let value = string_literal(rest.strip_prefix('=')?.trim_start().strip_prefix('"')?)?;
            Some((name.to_string(), value))
        })
        .collect()
}

/// The text of the constant that `rest` starts with, by name or by path (`TIME_BUDGET`,
/// `crate::runtime::SELECTION_TOO_LARGE.into()`), when `consts` holds it: every text, when crates
/// declare constants of that name with different texts.
fn named_const<'a>(rest: &str, consts: &'a std::collections::BTreeMap<String, Vec<String>>) -> &'a [String] {
    let path = &rest[..rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':').unwrap_or(rest.len())];
    let after = &rest[path.len()..];
    let used = [")", ",", ".into()", ".to_string()", ".to_owned()"].iter().any(|end| after.starts_with(end));
    let name = path.rsplit("::").next().unwrap_or("");
    let upper = name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if used && upper { consts.get(name).map_or(&[], Vec::as_slice) } else { &[] }
}

/// The message literals in `code` outside its comments, as catalog templates (see
/// [`message_literals`]): the literal after each of [`MESSAGE_MARKERS`], the message argument of an
/// error enum's variant, an enum whose name ends in `Error` or `Fail` (the first argument, as in
/// `PsError::Limit(…)`, `Error::Manifest(…)` and `CompileFail::Syntax(…)`, or the one
/// [`LATER_MESSAGE_VARIANTS`] gives; not [`NAME_VARIANTS`]),
/// the messages a `.map_err` closure returns (`.map_err(|e| format!("…: {e}"))`, see
/// [`closure_returns`]) or the handler a `.map_err(name)` names returns (see [`handler_returns`]),
/// and the message argument of each call in `helpers` (a call and the number of arguments before
/// its message), a literal, a `format!` string or the name of one of `consts` (see
/// [`str_consts`]), each one with or without a `&` before it. A key's name alone (`` `tint` ``)
/// is not a message.
fn source_messages(code: &str, helpers: &[(&str, usize)], consts: &std::collections::BTreeMap<String, Vec<String>>) -> Vec<String> {
    let code = &without_comments(code);
    let after_markers = MESSAGE_MARKERS.iter().flat_map(|marker| code.match_indices(marker).map(|(at, _)| &code[at + marker.len()..]));
    let in_variants = ["Error::", "Fail::"].into_iter().flat_map(|enum_end| code.match_indices(enum_end)).filter_map(|(at, m)| {
        let rest = &code[at + m.len()..];
        let (variant, args) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len()));
        let named = variant.starts_with(|c: char| c.is_ascii_uppercase()) && !NAME_VARIANTS.contains(&variant);
        let before = LATER_MESSAGE_VARIANTS.iter().find(|(v, _)| *v == variant).map_or(0, |&(_, n)| n);
        if named { skip_args(args.strip_prefix('(')?, before) } else { None }
    });
    let in_map_err = code.match_indices(".map_err(").flat_map(|(at, m)| {
        let rest = code[at + m.len()..].trim_start();
        let params = rest.strip_prefix("move").map_or(rest, str::trim_start).strip_prefix('|');
        params.and_then(|p| p.find('|').map(|end| &p[end + 1..])).map(closure_returns).unwrap_or_default()
    });
    let mut handlers = vec![];
    for (at, m) in code.match_indices(".map_err(") {
        let rest = &code[at + m.len()..];
        let name = &rest[..rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len())];
        if name.starts_with(|c: char| c.is_ascii_lowercase()) && rest[name.len()..].starts_with(')') && !handlers.contains(&name) {
            handlers.push(name);
        }
    }
    let in_handlers = handlers.into_iter().flat_map(|name| handler_returns(code, name));
    let in_helpers = helpers.iter().flat_map(|&(call, before)| {
        code.match_indices(call)
            .filter(|&(at, _)| call.starts_with('.') || !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.'))
            .filter_map(move |(at, _)| skip_args(&code[at + call.len()..], before))
    });
    let mut out = vec![];
    for rest in after_markers.chain(in_variants).chain(in_map_err).chain(in_handlers).chain(in_helpers) {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('&').map_or(rest, str::trim_start);
        let rest = rest.strip_prefix("format!(").map_or(rest, str::trim_start);
        let lits = match rest.strip_prefix('"') {
            Some(quoted) => string_literal(quoted).into_iter().collect(),
            None => named_const(rest, consts).to_vec(),
        };
        for lit in lits {
            let has_word = lit.as_bytes().windows(3).any(|w| w.iter().all(u8::is_ascii_lowercase));
            let key_name = lit.len() > 1 && lit.starts_with('`') && lit.ends_with('`') && !lit.contains(' ');
            if has_word && !key_name && !NOT_MESSAGES.contains(&lit.as_str()) {
                out.push(as_template(&lit));
            }
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

/// Where each value the error handler `name` of a `.map_err(name)` in `code` can return starts:
/// the closure the file binds to that name (`let fail = |e| format!("…: {e}");`, up to its `;`)
/// or the body of the function of that name (`fn unreadable(e: …) -> String { … }`), read as
/// [`closure_returns`] reads a closure.
fn handler_returns<'a>(code: &'a str, name: &str) -> Vec<&'a str> {
    let (bound, declared) = (format!("let {name} = "), format!("fn {name}("));
    let mut out = vec![];
    for (at, m) in code.match_indices(bound.as_str()).chain(code.match_indices(declared.as_str())) {
        if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = &code[at + m.len()..];
        if m == bound {
            let rest = rest.strip_prefix("move").map_or(rest, str::trim_start);
            if let Some(body) = rest.strip_prefix('|').and_then(|p| p.find('|').map(|end| &p[end + 1..])) {
                out.extend(closure_returns(statement(body)));
            }
        } else if let Some(open) = rest.find('{') {
            out.extend(closure_returns(&rest[open + 1..]));
        }
    }
    out
}

/// The statement `code` starts with, up to its `;` or the bracket that closes the block it is in.
fn statement(code: &str) -> &str {
    let (mut depth, mut chars) = (0usize, code.char_indices());
    while let Some((at, c)) = chars.next() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return &code[..at],
            },
            ';' if depth == 0 => return &code[..at],
            '"' => loop {
                match chars.next() {
                    Some((_, '\\')) => {
                        chars.next();
                    }
                    Some((_, '"')) | None => break,
                    _ => {}
                }
            },
            _ => {}
        }
    }
    code
}

/// Where each value a closure can return starts, `body` being the text after its parameters: the
/// body itself, each block or `if`/`else` branch in it (`{ ACB_CUT_SHORT.into() }`, not the
/// `{` of a `json!({…})` value), each `match` arm (`=>`) and each statement after a `;` (the
/// last expression of a block or a function body), up to the bracket that closes the call the
/// closure is given to or the function.
fn closure_returns(body: &str) -> Vec<&str> {
    let mut out = vec![body];
    let (mut depth, mut chars) = (0usize, body.char_indices());
    while let Some((at, c)) = chars.next() {
        match c {
            '(' | '[' => depth += 1,
            '{' => {
                depth += 1;
                if !body[..at].trim_end().ends_with('(') {
                    out.push(&body[at + 1..]);
                }
            }
            ')' | ']' | '}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => break,
            },
            ';' => out.push(&body[at + 1..]),
            '=' if body[at + 1..].starts_with('>') => out.push(&body[at + 2..]),
            '"' => loop {
                match chars.next() {
                    Some((_, '\\')) => {
                        chars.next();
                    }
                    Some((_, '"')) | None => break,
                    _ => {}
                }
            },
            _ => {}
        }
    }
    out
}

/// Source up to its inline test module (`#[cfg(test)] mod tests {`, or a `cfg(all(test, …))` one);
/// declarations of test files (`#[cfg(test)] mod tests_x;`) don't end it.
fn without_test_module(text: &str) -> &str {
    for (at, m) in text.match_indices("#[cfg(") {
        let rest = &text[at + m.len()..];
        let Some(end) = rest.find(")]") else { continue };
        let cond: String = rest[..end].chars().filter(|c| !c.is_whitespace()).collect();
        if cond != "test" && !cond.starts_with("all(test,") {
            continue;
        }
        let after = rest[end + 2..].trim_start();
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

/// A format string as a catalog template (see [`message_literals`]). `{{` and `}}` stay as they
/// are, the catalogs' literal braces.
fn as_template(s: &str) -> String {
    let mut out = String::new();
    let mut unnamed = 0;
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(escaped) = after.strip_prefix('{') {
            out.push_str("{{");
            rest = escaped;
            continue;
        }
        let Some(b) = after.find('}') else {
            out.push_str(&rest[a..]);
            return out;
        };
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
