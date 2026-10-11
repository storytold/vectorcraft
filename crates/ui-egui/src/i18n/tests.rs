use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

const ZH: fn() -> Lang = || Lang::from_code("zh-hant").expect("zh-hant registered");

#[test]
fn tags_map_to_languages() {
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("nl_NL"), None);
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
    for tag in ["uk", "uk-UA", "uk_UA.UTF-8", "UK-ua", "uk_UA.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(uk()), "{tag}");
    }
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
fn candidates_walk_from_specific_to_general() {
    assert_eq!(candidates("pt_BR.UTF-8"), ["pt-br", "pt"]);
    assert_eq!(candidates("zh_TW"), ["zh-tw", "zh-hant", "zh"]);
    assert_eq!(candidates("zh-CN"), ["zh-cn", "zh-hans", "zh"]);
    assert_eq!(candidates("zh-Hant-HK"), ["zh-hant-hk", "zh-hant", "zh"]);
}

#[test]
fn os_language_lists_are_parsed() {
    assert_eq!(first_supported("(\n    \"zh-Hant-TW\",\n    \"en-US\"\n)\n"), Some(ZH()));
    assert_eq!(first_supported("(\n    \"nl-NL\",\n    \"en-US\"\n)\n"), Some(Lang::EN));
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

/// The engine learns the language the UI is drawn in each frame: new type takes the Japanese
/// defaults while it is Japanese (#432). The frames stay in English: the drawing language is
/// process-wide, and a Japanese frame would translate the tests running beside this one.
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
    // Each frame hands the engine the language it draws in, whatever the engine had.
    app.session.ui_language = Some("ja".into());
    assert!(app.session.japanese_interface());
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("en"));
    // The language a frame would draw in follows the preference.
    app.session.prefs.interface_language = "ja".into();
    assert_eq!(app.ui_language().code(), "ja");
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

/// Every menu string (top-level titles, submenu names, item labels, section headers, UI and
/// engine command labels and menu paths) has an entry in each language that claims complete menus.
#[test]
fn complete_languages_translate_every_menu_string() {
    let strings = crate::menus::menu_strings();
    assert!(strings.len() > 500, "menu scan found only {} strings", strings.len());
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        let cat = l.catalog();
        // Languages that keep the product, workspace and perspective preset names in English.
        let kept = |s: &str| KEEPS_MENU_NAMES.contains(&l.code) && MENU_KEEP_AS_IS.contains(&s);
        let missing: Vec<_> = strings.iter().filter(|s| !kept(s) && cat.plain(s).is_none()).collect();
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

/// Languages whose catalogs leave [`MENU_KEEP_AS_IS`] in English.
const KEEPS_MENU_NAMES: [&str; 9] = ["cs", "de", "es", "fr", "it", "ja", "pt-br", "ru", "uk"];

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

/// Czech, German, Spanish, Italian, Japanese and Brazilian Portuguese cover every menu label, the Show/Hide pairs
/// and the canvas context menu included (panels and dialogs not yet).
#[test]
fn menu_catalogs_translate_every_menu_label() {
    let labels = menu_labels();
    // What the menus show (recent files and fonts, user presets and libraries show their names).
    let shown = crate::menus::menu_strings();
    let language_name = |l: &str| Lang::all().any(|lang| lang.name() == l);
    // Font names and sizes, and installed plug-ins' own names, are not interface text.
    let interface = |(label, id, p): &(&str, &str, serde_json::Value)| {
        let plugin = *id == "plugin.dialog" || p.get("effect").and_then(serde_json::Value::as_str).is_some_and(|e| e.starts_with("plugin."));
        *id != "text.setStyle" && !plugin && shown.contains(*label) && !MENU_KEEP_AS_IS.contains(label) && !language_name(label)
    };
    let mut all: Vec<String> = labels.iter().filter(|l| interface(l)).map(|(l, ..)| l.to_string()).collect();
    all.extend(toggled_labels());
    all.extend(crate::menus::CONTEXT_LABELS.iter().map(|l| l.to_string()));
    // The macOS menu bar's own labels, and its Settings submenu's Preferences pages.
    all.extend(crate::native_menu::MAC_LABELS.iter().map(|l| l.to_string()));
    all.extend(vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES.iter().map(|l| l.to_string()));
    all.sort();
    all.dedup();
    for code in KEEPS_MENU_NAMES {
        let lang = Lang::from_code(code).expect("registered");
        let missing: Vec<&String> = all.iter().filter(|l| !has(lang, l)).collect();
        assert!(missing.is_empty(), "{code}: untranslated menu labels: {missing:?}");
        for keep in MENU_KEEP_AS_IS {
            assert!(labels.iter().any(|(l, ..)| l == keep), "`{keep}` isn't a menu label");
            assert!(!has(lang, keep), "{code}: `{keep}` is both kept and translated");
        }
    }
    assert_eq!(tr(cs(), "File"), "Soubor");
    assert_eq!(tr(Lang::from_code("ja").expect("ja"), "File"), "ファイル");
    // Italian: Italy, Switzerland and San Marino share the one catalog.
    for tag in ["it", "it_IT.UTF-8", "it-CH", "it_SM"] {
        assert_eq!(lang_from_tag(tag), Lang::from_code("it"), "{tag}");
    }
    assert_eq!(tr(Lang::from_code("pt-br").expect("pt-br"), "File"), "Arquivo");
    assert_eq!(tr(es(), "File"), "Archivo");
    assert_eq!(tr(de(), "File"), "Datei");
    assert_eq!(tr(it(), "Edit"), "Modifica");
}

fn de() -> Lang {
    Lang::from_code("de").expect("de registered")
}

fn es() -> Lang {
    Lang::from_code("es").expect("es registered")
}

fn fr() -> Lang {
    Lang::from_code("fr").expect("fr registered")
}

fn it() -> Lang {
    Lang::from_code("it").expect("it registered")
}

fn ru() -> Lang {
    Lang::from_code("ru").expect("ru registered")
}

fn uk() -> Lang {
    Lang::from_code("uk").expect("uk registered")
}

/// Spanish uses the vector-illustration vocabulary its users know, has two plural forms like
/// English, and reads the same in the menus and in the panels.
#[test]
fn spanish_reads_as_spanish() {
    for (en, want) in [
        ("Artboard Tool", "Herramienta Mesa de trabajo"),
        ("Swatches", "Muestras"),
        ("Pathfinder", "Buscatrazos"),
        ("Stroke", "Trazo"),
        ("Fill", "Relleno"),
        ("Direct Selection Tool", "Herramienta Selección directa"),
        ("Save As…", "Guardar como…"),
        ("Undo", "Deshacer"),
    ] {
        assert_eq!(tr(es(), en), want);
    }
    assert_eq!(trn(es(), 1, "{n} Layer", "{n} Layers"), "1 capa");
    assert_eq!(trn(es(), 0, "{n} Layer", "{n} Layers"), "0 capas");
    assert_eq!(trn(es(), 3, "{n} Layer", "{n} Layers"), "3 capas");
}

/// German uses the vector-illustration vocabulary its users know, has two plural forms like
/// English (zero takes the plural), translates the reason inside a message too, and tells the
/// interface theme ("Erscheinungsbild") from the Appearance panel ("Aussehen").
#[test]
fn german_reads_as_german() {
    for (en, want) in [
        ("Artboard Tool", "Zeichenflächen-Werkzeug"),
        ("Swatches", "Farbfelder"),
        ("Pathfinder", "Pathfinder"),
        ("Stroke", "Kontur"),
        ("Fill", "Fläche"),
        ("Direct Selection Tool", "Direktauswahl-Werkzeug"),
        ("Save As…", "Speichern unter…"),
        ("Undo", "Rückgängig"),
    ] {
        assert_eq!(tr(de(), en), want);
    }
    assert_eq!(trn(de(), 1, "{n} Layer", "{n} Layers"), "1 Ebene");
    assert_eq!(trn(de(), 0, "{n} Layer", "{n} Layers"), "0 Ebenen");
    assert_eq!(trn(de(), 3, "{n} Layer", "{n} Layers"), "3 Ebenen");
    assert_eq!(tr_ctx(de(), "axis", "Both"), "Beide");
    assert_eq!(tr(de(), "Appearance"), "Aussehen");
    assert_eq!(tr_ctx(de(), "theme", "Appearance"), "Erscheinungsbild");
    let nested = "command `object.group` is not available right now: nothing selected";
    assert_eq!(message(de(), nested), "der Befehl `object.group` ist derzeit nicht verfügbar: nichts ausgewählt");
    // Text styles are "…format" in German: the kind is joined to it ("Neues Zeichenformat erstellen").
    assert_eq!(fmt(tr(de(), "Create New {kind} Style"), &[("kind", tr(de(), "Character"))]), "Neues Zeichenformat erstellen");
    assert_eq!(fmt(tr(de(), "Show {kind} Brushes"), &[("kind", tr(de(), "Calligraphic"))]), "Kalligrafiepinsel einblenden");
}

/// French uses the vector-illustration vocabulary its users know, puts zero in the singular (« 0
/// calque »), and reads the same in the menus and in the panels.
#[test]
fn french_reads_as_french() {
    for (en, want) in [
        ("Artboard Tool", "Outil Plan de travail"),
        ("Swatches", "Nuancier"),
        ("Pathfinder", "Pathfinder"),
        ("Stroke", "Contour"),
        ("Fill", "Fond"),
        ("Direct Selection Tool", "Outil Sélection directe"),
        ("Save As…", "Enregistrer sous…"),
        ("Undo", "Annuler"),
    ] {
        assert_eq!(tr(fr(), en), want);
    }
    assert_eq!(trn(fr(), 0, "{n} Layer", "{n} Layers"), "0 calque");
    assert_eq!(trn(fr(), 1, "{n} Layer", "{n} Layers"), "1 calque");
    assert_eq!(trn(fr(), 2, "{n} Layer", "{n} Layers"), "2 calques");
}

/// Italian uses the vector-illustration vocabulary its users know, has two plural forms like
/// English (zero takes the plural), and reads the same in the menus and in the panels.
#[test]
fn italian_reads_as_italian() {
    for (en, want) in [
        ("Artboard Tool", "Strumento Tavola da disegno"),
        ("Swatches", "Campioni"),
        ("Pathfinder", "Elaborazione tracciati"),
        ("Stroke", "Traccia"),
        ("Fill", "Riempimento"),
        ("Direct Selection Tool", "Strumento Selezione diretta"),
        ("Save As…", "Salva con nome…"),
        ("Undo", "Annulla"),
    ] {
        assert_eq!(tr(it(), en), want);
    }
    assert_eq!(trn(it(), 1, "{n} Layer", "{n} Layers"), "1 livello");
    assert_eq!(trn(it(), 0, "{n} Layer", "{n} Layers"), "0 livelli");
    assert_eq!(trn(it(), 3, "{n} Layer", "{n} Layers"), "3 livelli");
}

/// Russian uses the vector-illustration vocabulary its users know, has the three plural forms
/// (one/few/many), and reads the same in the menus and in the panels.
#[test]
fn russian_reads_as_russian() {
    for (en, want) in [
        ("Artboard Tool", "Инструмент «Монтажная область»"),
        ("Swatches", "Образцы"),
        ("Pathfinder", "Обработка контуров"),
        ("Stroke", "Обводка"),
        ("Fill", "Заливка"),
        ("Direct Selection Tool", "Инструмент «Прямое выделение»"),
        ("Save As…", "Сохранить как…"),
        ("Undo", "Отменить"),
    ] {
        assert_eq!(tr(ru(), en), want);
    }
    assert_eq!(trn(ru(), 1, "{n} Layer", "{n} Layers"), "1 слой");
    assert_eq!(trn(ru(), 2, "{n} Layer", "{n} Layers"), "2 слоя");
    assert_eq!(trn(ru(), 5, "{n} Layer", "{n} Layers"), "5 слоёв");
    assert_eq!(trn(ru(), 11, "{n} Layer", "{n} Layers"), "11 слоёв");
    assert_eq!(trn(ru(), 21, "{n} Layer", "{n} Layers"), "21 слой");
    assert_eq!(trn(ru(), 22, "{n} Layer", "{n} Layers"), "22 слоя");
}

#[test]
fn ukrainian_reads_as_ukrainian() {
    for (en, want) in [
        ("Artboard Tool", "Інструмент «Монтажна область»"),
        ("Swatches", "Зразки"),
        ("Pathfinder", "Обробка контурів"),
        ("Stroke", "Обведення"),
        ("Fill", "Заливка"),
        ("Direct Selection Tool", "Інструмент «Пряме виділення»"),
        ("Save As…", "Зберегти як…"),
        ("Undo", "Скасувати"),
    ] {
        assert_eq!(tr(uk(), en), want);
    }
    assert_eq!(tr_ctx(uk(), "axis", "Both"), "Обидві");
    for (n, word) in [
        (0, "шарів"),
        (1, "шар"),
        (2, "шари"),
        (5, "шарів"),
        (11, "шарів"),
        (21, "шар"),
        (22, "шари"),
        (111, "шарів"),
        (112, "шарів"),
        (121, "шар"),
        (124, "шари"),
    ] {
        assert_eq!(trn(uk(), n, "{n} Layer", "{n} Layers"), format!("{n} {word}"));
    }
    let nested = "command `object.group` is not available right now: nothing selected";
    assert_eq!(message(uk(), nested), "команда `object.group` зараз недоступна: нічого не виділено");
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
}

#[test]
fn russian_plurals_have_three_forms() {
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 11, 21, 22, 25, 111, 121].into_iter().map(plural_east_slavic).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 0, 1, 2, 2, 0]);
}

/// Czech, German, Spanish, French and Italian letters (and the punctuation their text uses) come from each family's own first font, not
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
            let mut font = fonts.fonts.font(&family);
            let chars = font.characters();
            for ch in "áčďéěíňóřšťúůýžÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ„“‚‘…–ñÑüÜ¿¡”àèìòùÀÈÌÒÙ«»’âêîôûçëïÿœæÂÊÎÔÛÇËÏŒÆäöÄÖß\u{a0}".chars()
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
            let mut font = fonts.fonts.font(&family);
            let chars = font.characters();
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

/// Each language that translates status and error messages ([`COMPLETE_MESSAGES`]) has an `@msg`
/// row for every message in the sources that reach the status bar, so a new one can't ship
/// untranslated by accident (`VECTORCRAFT_I18N_DUMP_MESSAGES=<file>` lists them).
#[test]
fn complete_languages_translate_every_message() {
    let messages = message_literals();
    assert!(messages.len() > 300, "scan found only {} messages", messages.len());
    if let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP_MESSAGES") {
        std::fs::write(path, messages.iter().map(|m| format!("{m}\n")).collect::<String>()).expect("write dump");
    }
    for code in COMPLETE_MESSAGES {
        let lang = Lang::from_code(code).expect("registered");
        let (entries, _) = parse_entries(lang.0.source);
        let rows: std::collections::HashSet<&str> = entries.iter().filter(|(ctx, _, _)| ctx == "@msg").map(|(_, src, _)| src.as_str()).collect();
        let missing: Vec<_> = messages.iter().filter(|m| !rows.contains(m.as_str())).collect();
        assert!(missing.is_empty(), "{code}: {} untranslated messages: {missing:#?}", missing.len());
    }
}

/// The parameter errors of Align, Distribute Spacing, Reflect and Shear, whose text the scan above
/// doesn't read, are translated whole in each language of [`COMPLETE_MESSAGES`].
#[test]
fn align_reflect_and_shear_param_errors_are_translated() {
    use serde_json::json;
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({})).unwrap();
    for x in [0, 50] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap();
    }
    s.execute("select.all", &json!({})).unwrap();
    for (cmd, p) in [
        ("object.align", json!({})),
        ("object.align", json!({"horizontal": "middle"})),
        ("object.align", json!({"vertical": "centre"})),
        ("object.align", json!({"horizontal": "left", "to": "page"})),
        ("object.align", json!({"horizontal": "left", "bounds": "visual"})),
        ("object.distributeSpacing", json!({"axis": "diagonal"})),
        ("object.distributeSpacing", json!({"spacing": "5"})),
        ("object.reflect", json!({"axis": "diagonal"})),
        ("object.shear", json!({"axis": "diagonal"})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err();
        let vectorcraft_engine::EngineError::BadParams { msg, .. } = &e else { panic!("{cmd} {p}: {e}") };
        for code in COMPLETE_MESSAGES {
            let shown = message(Lang::from_code(code).unwrap(), &e.to_string());
            assert!(!shown.contains(msg.as_str()), "{code}: {shown}");
        }
    }
}

/// Languages whose catalogs cover every status and error message.
const COMPLETE_MESSAGES: &[&str] = &["de", "es", "fr", "it", "ja", "ru", "uk"];

/// Crates whose error and status messages reach the status bar.
const MESSAGE_CRATES: &[&str] = &["ui-egui", "engine", "doc", "format", "svg", "pdf", "eps", "text", "plugins", "metafile", "cad", "trace"];

/// Where a message literal starts: a status, an error value, a `thiserror` message.
const MESSAGE_MARKERS: &[&str] = &[".status(", "ui.status = ", "status = ", "Other(", "Err(", "ok_or(", "ok_or_else(|| ", "#[error("];

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
            } else if name.ends_with(".rs") && !name.starts_with("tests") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let code = without_test_module(&text);
                for marker in MESSAGE_MARKERS {
                    for (at, _) in code.match_indices(marker) {
                        let rest = code[at + marker.len()..].trim_start();
                        let rest = rest.strip_prefix("format!(").map_or(rest, str::trim_start);
                        let Some(lit) = rest.strip_prefix('"').and_then(string_literal) else { continue };
                        let has_word = lit.as_bytes().windows(3).any(|w| w.iter().all(u8::is_ascii_lowercase));
                        if has_word && !NOT_MESSAGES.contains(&lit.as_str()) {
                            out.insert(as_template(&lit));
                        }
                    }
                }
            }
        }
    }
    out
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
