use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

const ZH: fn() -> Lang = || Lang::from_code("zh-hant").expect("zh-hant registered");

#[test]
fn tags_map_to_languages() {
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("fr_FR"), None);
    assert_eq!(lang_from_tag("ja_JP.UTF-8"), Lang::from_code("ja"));
    assert_eq!(lang_from_tag("ja"), Lang::from_code("ja"));
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
        assert_ne!(lang_from_tag(tag), Some(ZH()), "{tag}");
    }
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
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
    assert_eq!(first_supported("(\n    \"fr-FR\",\n    \"en-US\"\n)\n"), Some(Lang::EN));
    assert_eq!(first_supported("("), None);
    // Windows: `reg query HKCU\Control Panel\International /v LocaleName`.
    let reg = "\r\nHKEY_CURRENT_USER\\Control Panel\\International\r\n    LocaleName    REG_SZ    zh-TW\r\n\r\n";
    assert_eq!(registry_locale(reg).as_deref(), Some("zh-TW"));
    assert_eq!(registry_locale(reg).as_deref().and_then(lang_from_tag), Some(ZH()));
    assert_eq!(registry_locale("ERROR: The system was unable to find the specified registry key or value."), None);
}

#[test]
fn preferences_resolve_with_fallback() {
    assert_eq!(Lang::from_pref("zh-hant"), ZH());
    assert_eq!(Lang::from_pref("ZH-Hant"), ZH());
    assert_eq!(Lang::from_pref("en"), Lang::EN);
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

#[test]
fn lookups_fall_back_to_english() {
    assert_eq!(tr(Lang::EN, "Layers"), "Layers");
    assert_eq!(tr(ZH(), "Layers"), "圖層");
    assert_eq!(tr(ZH(), "no such label"), "no such label");
    assert_eq!(tr_id(ZH(), "no.such.id", "Layers"), "圖層");
    assert_eq!(tr_ctx(ZH(), "no such context", "Layers"), "圖層");
    assert!(has(ZH(), "Layers"));
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
    assert_eq!(trn(ZH(), 1, "{n} item", "{n} items"), "1 個項目");
    assert_eq!(trn(ZH(), 7, "{n} item", "{n} items"), "7 個項目");
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
            if ctx == "@plural" {
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
                let known =
                    crate::menus::UI_COMMANDS.iter().any(|c| c.0 == src) || vectorcraft_engine::cmd::command_specs().iter().any(|c| c.id == src);
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

/// VectorCraft › Language (`app.language`) sets the `interfaceLanguage` preference, which is what
/// persists; the checked item follows the preference, and a bad code is an error, not a change.
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
    // Every registered language is in the menu, by its own name.
    let labels: Vec<String> =
        crate::menus::menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some("app.language")).map(|e| e.label).collect();
    for l in Lang::all() {
        assert!(labels.iter().any(|x| x == l.name()), "{} missing from {labels:?}", l.code());
    }
}

/// Chinese and Japanese characters (CJK punctuation, kana, ideographs, full-width forms).
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// The UI fonts draw every character the catalogs use, in every family the UI draws text with
/// (the web build has no system fonts to fall back to). Chinese and Japanese characters come from
/// the craft-fonts build input, which is optional: without it they are left to the installed
/// fonts, and with it the Japanese catalog is checked against BIZ UDPGothic (the Traditional
/// Chinese one waits for a Traditional Chinese UI font in craft-fonts).
#[test]
fn bundled_fonts_cover_the_catalogs() {
    let japanese_ui_font = vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.is_japanese());
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
            let chars: std::collections::BTreeSet<char> =
                entries.iter().flat_map(|(_, _, tr)| tr.chars()).filter(|c| !c.is_whitespace() && (cjk_checked || !is_cjk(*c))).collect();
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

/// Every menu string (top-level titles, submenu names, item labels, section headers, UI and
/// engine command labels and menu paths) has an entry in each language that claims complete menus.
#[test]
fn complete_languages_translate_every_menu_string() {
    let strings = crate::menus::menu_strings();
    assert!(strings.len() > 500, "menu scan found only {} strings", strings.len());
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        let cat = l.catalog();
        let missing: Vec<_> = strings.iter().filter(|s| cat.plain(s).is_none()).collect();
        assert!(missing.is_empty(), "{}: {} untranslated menu strings: {missing:#?}", l.code, missing.len());
    }
}

/// Every `tl!("literal")` in the shell has an entry in each language that claims complete menus
/// (so a new label can't ship untranslated by accident). Literals that are deliberately shown as
/// they are (names, units) are listed in `KEEP_AS_IS`.
#[test]
fn every_tl_literal_is_translated() {
    const KEEP_AS_IS: &[&str] = &[];
    let literals = tl_literals();
    assert!(literals.len() > 300, "scan found only {} literals", literals.len());
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        let cat = l.catalog();
        let missing: Vec<_> = literals.iter().filter(|s| !KEEP_AS_IS.contains(&s.as_str()) && cat.plain(s).is_none()).collect();
        assert!(missing.is_empty(), "{}: {} untranslated tl! strings: {missing:#?}", l.code, missing.len());
    }
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
                stack.push(path);
            } else if name.ends_with(".rs") && name != "lib.rs" && !name.starts_with("tests") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let code = text.split("#[cfg(test)]").next().unwrap_or("");
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

/// `VECTORCRAFT_I18N_DUMP=<file> cargo test -p vectorcraft-ui-egui dump_source_strings` writes every
/// English source string a complete catalog has to cover, one per line, for translators.
#[test]
fn dump_source_strings() {
    let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP") else { return };
    let mut all = crate::menus::menu_strings();
    all.extend(tl_literals());
    let text: String = all.iter().map(|s| format!("{}\n", s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t"))).collect();
    std::fs::write(path, text).expect("write dump");
}

fn cs() -> Lang {
    Lang::from_code("cs").expect("cs registered")
}

/// Menu labels Czech shows as they are: the product name, a format name, the built-in workspace
/// names and the perspective grid presets (names, shown untranslated wherever else they appear).
/// Each language's own name in the Language menu is left alone too.
const CZECH_KEEP_AS_IS: &[&str] = &[
    "Vector W3K2",
    "Summa",
    "Zünd",
    "OpenType",
    "Essentials",
    "Essentials Classic",
    "Automation",
    "Layout",
    "Painting",
    "Printing and Proofing",
    "Tracing",
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
        "view.artboards",
        "view.rulers",
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

/// Czech covers every menu label, the Show/Hide pairs included (panels and dialogs not yet).
#[test]
fn czech_translates_every_menu_label() {
    let labels = menu_labels();
    // What the menus show (recent files and fonts, user presets and libraries show their names).
    let shown = crate::menus::menu_strings();
    let language_name = |l: &str| Lang::all().any(|lang| lang.name() == l);
    // Font names and sizes, and installed plug-ins' own names, are not interface text.
    let interface = |(label, id, p): &(&str, &str, serde_json::Value)| {
        let plugin = *id == "plugin.dialog" || p.get("effect").and_then(serde_json::Value::as_str).is_some_and(|e| e.starts_with("plugin."));
        *id != "text.setStyle" && !plugin && shown.contains(*label) && !CZECH_KEEP_AS_IS.contains(label) && !language_name(label)
    };
    let mut missing: Vec<String> = labels.iter().filter(|l| interface(l)).map(|(l, ..)| l.to_string()).collect();
    missing.extend(toggled_labels());
    missing.extend(crate::menus::CONTEXT_LABELS.iter().map(|l| l.to_string()));
    missing.retain(|l| !has(cs(), l));
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "untranslated Czech menu labels: {missing:?}");
    for keep in CZECH_KEEP_AS_IS {
        assert!(labels.iter().any(|(l, ..)| l == keep), "`{keep}` isn't a menu label");
        assert!(!has(cs(), keep), "`{keep}` is both kept and translated");
    }
    assert_eq!(tr(cs(), "File"), "Soubor");
}

#[test]
fn czech_plurals_have_three_forms() {
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 11, 21].into_iter().map(plural_czech).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2]);
}

/// Czech letters (and the punctuation Czech text uses) come from each family's own first font, not
/// from a fallback further down the stack. (`has_glyph` can't tell: it counts characters of the
/// face that draws missing glyphs, the first one, as missing.)
#[test]
fn czech_glyphs_are_available_without_system_fonts() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    ctx.fonts_mut(|fonts| {
        let families: Vec<_> = fonts.definitions().families.iter().map(|(f, stack)| (f.clone(), stack.first().cloned())).collect();
        for (family, first) in families {
            let first = first.unwrap();
            let mut font = fonts.fonts.font(&family);
            let chars = font.characters();
            for ch in "áčďéěíňóřšťúůýžÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ„“‚‘…–".chars() {
                assert!(chars.get(&ch).is_some_and(|fonts| fonts.contains(&first)), "{first} ({family:?}) has no {ch}");
            }
        }
    });
}

/// A language chosen in an older version (saved with the UI state as `"language"`) carries over to
/// the `interfaceLanguage` preference, and isn't written back.
#[test]
fn a_language_saved_by_an_older_version_carries_over() {
    for (saved, want) in [("ja", "ja"), ("cs", "cs"), ("en", "auto"), ("xx", "auto")] {
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
