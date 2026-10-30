use super::{Item, Sep, c, cp, json, sub, todo};

/// Type → Size presets.
pub(super) const TYPE_SIZES: [(&str, u32); 14] = [
    ("6 pt", 6),
    ("8 pt", 8),
    ("9 pt", 9),
    ("10 pt", 10),
    ("11 pt", 11),
    ("12 pt", 12),
    ("14 pt", 14),
    ("18 pt", 18),
    ("24 pt", 24),
    ("30 pt", 30),
    ("36 pt", 36),
    ("48 pt", 48),
    ("60 pt", 60),
    ("72 pt", 72),
];

/// Type → Type on a Path effects (label, `type.pathOptions` effect), also the choices of its
/// Options dialog.
pub(crate) const PATH_EFFECTS: &[(&str, &str)] =
    &[("Rainbow", "rainbow"), ("Skew", "skew"), ("3D Ribbon", "3dRibbon"), ("Stair Step", "stairStep"), ("Gravity", "gravity")];

pub(super) const INSERT_SPECIAL: &[(&str, &str)] = &[
    ("Bullet", "bullet"),
    ("Copyright Symbol", "copyright"),
    ("Ellipsis", "ellipsis"),
    ("Paragraph Symbol", "paragraph"),
    ("Registered Trademark Symbol", "registered"),
    ("Section Symbol", "section"),
    ("Trademark Symbol", "trademark"),
    ("Em Dash", "emDash"),
    ("En Dash", "enDash"),
    ("Discretionary Hyphen", "discretionaryHyphen"),
    ("Nonbreaking Hyphen", "nonBreakingHyphen"),
    ("Double Left Quotation Marks", "doubleLeftQuote"),
    ("Double Right Quotation Marks", "doubleRightQuote"),
    ("Single Left Quotation Mark", "singleLeftQuote"),
    ("Single Right Quotation Mark", "singleRightQuote"),
];

pub(super) const INSERT_WHITESPACE: &[(&str, &str)] = &[
    ("Em Space", "emSpace"),
    ("En Space", "enSpace"),
    ("Hair Space", "hairSpace"),
    ("Sixth Space", "sixthSpace"),
    ("Thin Space", "thinSpace"),
    ("Nonbreaking Space", "nonBreakingSpace"),
    ("Figure Space", "figureSpace"),
    ("Punctuation Space", "punctuationSpace"),
    ("Third Space", "thirdSpace"),
    ("Quarter Space", "quarterSpace"),
];

pub(super) const INSERT_BREAK: &[(&str, &str)] = &[("Forced Line Break", "forcedLineBreak"), ("Tab", "tab")];

pub(super) fn insert_items(list: &[(&'static str, &'static str)]) -> Vec<Item> {
    list.iter().map(|(l, c)| cp(l, "type.insert", json!({ "char": c }))).collect()
}

/// Type → Font: one item per available family, the installed fonts included. Built again only when
/// the fonts or Show Font Names in English change (the menu tree is built every frame); labels are
/// interned once each, so rebuilding doesn't allocate forever. The command still sets the English
/// family name ([`vectorcraft_text::FontDb::canonical`]).
pub(super) fn font_items(english_names: bool) -> Vec<Item> {
    static NAMES: std::sync::Mutex<std::collections::BTreeSet<&'static str>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    static ITEMS: std::sync::Mutex<(u64, bool, Vec<Item>)> = std::sync::Mutex::new((u64::MAX, true, Vec::new()));
    let db = vectorcraft_text::FontDb::global();
    // Read first: fonts that load meanwhile make the next frame build the list again.
    let generation = db.generation();
    let fams = db.menu_family_list();
    let (Ok(mut names), Ok(mut items)) = (NAMES.lock(), ITEMS.lock()) else { return vec![] };
    if items.0 != generation || items.1 != english_names {
        let list = fams
            .iter()
            .map(|f| {
                let english = f.as_str();
                let shown = db.family_display_name(english, english_names);
                let label: &'static str = match names.get(shown.as_str()) {
                    Some(n) => n,
                    None => {
                        let n: &'static str = Box::leak(shown.into_boxed_str());
                        names.insert(n);
                        n
                    }
                };
                let font: &'static str = match names.get(english) {
                    Some(n) => n,
                    None => {
                        let n: &'static str = Box::leak(english.to_string().into_boxed_str());
                        names.insert(n);
                        n
                    }
                };
                cp(label, "text.setStyle", json!({ "font": font }))
            })
            .collect();
        *items = (generation, english_names, list);
    }
    items.2.clone()
}

/// The raster effects' submenus of the Effect menu (the Photoshop-style effects, below the
/// vector effects): (submenu, the catalogue's menu path of its effects).
/// The raster Stylize submenu shares its path with the vector one and lists only the effects at
/// that path that [`vectorcraft_effects::PIXEL_EFFECTS`] names.
const RASTER_MENUS: [(&str, &[&str]); 10] = [
    ("Artistic", &["Effect", "Artistic"]),
    ("Blur", &["Effect", "Blur"]),
    ("Brush Strokes", &["Effect", "Brush Strokes"]),
    ("Distort", &["Effect", "Distort"]),
    ("Pixelate", &["Effect", "Pixelate"]),
    ("Sharpen", &["Effect", "Sharpen"]),
    ("Sketch", &["Effect", "Sketch"]),
    ("Stylize", &["Effect", "Stylize"]),
    ("Texture", &["Effect", "Texture"]),
    ("Video", &["Effect", "Video"]),
];

/// The Effect menu, built from the effects catalogue (vector effects), plus raster effects.
pub(super) fn effect_menu() -> Vec<Item> {
    let cat = vectorcraft_effects::effect_catalog();
    let mut out = vec![
        c("Apply Last Effect", "effect.applyLast"),
        c("Last Effect…", "effect.last"),
        Sep,
        c("Document Raster Effects Settings…", "document.rasterEffectsSettings"),
        Sep,
        Item::Header("Vector Effects"),
    ];
    // Submenus (and Crop Marks and Rasterize…, items of their own) in the reference app's order.
    let order = [
        "3D and Materials",
        "Color Adjustments",
        "Color Effects",
        "Convert to Shape",
        "Crop Marks",
        "Distort & Transform",
        "Path",
        "Pathfinder",
        "Rasterize…",
        "Stylize",
        "SVG Filters",
        "Warp",
    ];
    let top_level = |e: &vectorcraft_effects::EffectInfo| e.menu == ["Effect"] && order.contains(&e.label);
    // An effect goes in the vector submenu its path names unless it is a Photoshop-style effect,
    // which goes in the raster submenu of the same name when `RASTER_MENUS` has one (Stylize ›
    // Glowing Edges).
    let in_vector_menu =
        |e: &vectorcraft_effects::EffectInfo| order.iter().any(|s| e.menu == ["Effect", *s]) && !vectorcraft_effects::PIXEL_EFFECTS.contains(&e.id);
    let item = |e: &vectorcraft_effects::EffectInfo| match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
        // No options (Effect → Pathfinder, Crop Marks): apply directly, like Illustrator.
        true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
        false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
    };
    // The effects whose catalogue menu path is `path`.
    // `vector` picks the ones for the vector submenu or the ones for the raster submenu.
    let items_at =
        |path: &[&str], vector: bool| -> Vec<Item> { cat.iter().filter(|e| e.menu == path && in_vector_menu(e) == vector).map(item).collect() };
    for sub_name in order {
        if let Some(e) = cat.iter().find(|e| top_level(e) && e.label == sub_name) {
            out.push(item(e));
            continue;
        }
        let items = items_at(&["Effect", sub_name], true);
        if items.is_empty() {
            let placeholder = match sub_name {
                "3D and Materials" => vec![todo("Extrude & Bevel…"), todo("Revolve…"), todo("Inflate…"), todo("Rotate…"), todo("Materials…")],
                "SVG Filters" => vec![todo("Apply SVG Filter…")],
                _ => continue,
            };
            out.push(sub(sub_name, placeholder));
        } else {
            let mut items = items;
            if sub_name == "3D and Materials" {
                // In the panel's order of 3D types, then Materials (not built yet).
                let rank = |label: &str| ["Extrude & Bevel…", "Revolve…", "Inflate…", "Rotate…"].iter().position(|l| *l == label).unwrap_or(9);
                items.sort_by_key(|i| match i {
                    Item::Cmd(label, ..) => rank(label),
                    _ => 9,
                });
                items.push(todo("Materials…"));
            }
            out.push(sub(sub_name, items));
        }
    }
    // Live-effect plug-ins close the vector effects.
    out.extend(crate::dialogs::plugin::effect_menu());
    let raster: Vec<Item> = RASTER_MENUS
        .iter()
        .filter_map(|(name, path)| {
            let items = items_at(path, false);
            (!items.is_empty()).then(|| sub(name, items))
        })
        .collect();
    if !raster.is_empty() {
        out.push(Sep);
        out.push(Item::Header("Raster Effects"));
        out.push(c("Effect Gallery…", "ui.effectGalleryDialog"));
        out.extend(raster);
    }
    // Anything not placed above (future effects) still shows up.
    let placed = |e: &vectorcraft_effects::EffectInfo| top_level(e) || in_vector_menu(e) || RASTER_MENUS.iter().any(|(_, path)| e.menu == *path);
    for e in cat.iter().filter(|e| !placed(e)) {
        out.push(Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })));
    }
    out
}
