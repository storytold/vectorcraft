use std::sync::Arc;
use std::time::SystemTime;

use craft::behavior::{EditEvent, Mixed, Preview};

use crate::VectorcraftApp;

/// An app with a new document holding type set in each of `fonts`, one text object each.
fn app_with_type(fonts: &[&str]) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({})).unwrap();
    for (i, font) in fonts.iter().enumerate() {
        app.run("text.create", serde_json::json!({"x": 20, "y": 40 + 30 * i, "text": "Type", "font": font})).unwrap();
    }
    app
}

/// The Document set's families, sorted.
fn used(app: &mut VectorcraftApp) -> Vec<String> {
    let mut names = super::document_family_set(app).to_vec();
    names.sort();
    names
}

/// A pick of `after` over `before`, as the picker raises it.
fn set(before: &str, after: &str) -> EditEvent<String> {
    EditEvent::set(Mixed::Same(before.to_string()), after.to_string(), "Font Family".into(), None)
}

/// The shared picker narrows to variable families, including when Favorites is also on.
#[test]
fn the_variable_filter_lists_only_variable_families() {
    use craft::font_manager::picker::{FamilyInfos, FontFilters, FontPickerSettings, family_rows};
    use craft::font_manager::{Catalog, Source};
    use vectorcraft_text::test_fonts::{HEIGHT_FAMILY, TWO_AXIS_FAMILY, height_font, two_axis_font};

    let catalog = Arc::new(Catalog::of_files(Vec::new()));
    for font in [height_font(true), two_axis_font()] {
        catalog.add_bytes(font.unwrap(), Source::Bundled, &Default::default()).unwrap();
    }
    let infos = FamilyInfos::of(&catalog);
    let mut settings = FontPickerSettings::default();
    settings.set_favorite(HEIGHT_FAMILY, true);
    let listed = |variable, favorites, settings: &FontPickerSettings| {
        let mut filters = FontFilters::default();
        filters.variable = variable;
        filters.favorites = favorites;
        family_rows(&catalog, &infos, "", Some(settings), &filters, &[], "en").into_iter().map(|row| row.family).collect::<Vec<_>>()
    };

    let all = listed(false, false, &settings);
    assert_eq!(all, [TWO_AXIS_FAMILY, HEIGHT_FAMILY]);
    assert_eq!(listed(true, false, &settings), [TWO_AXIS_FAMILY]);
    assert!(listed(true, true, &settings).is_empty(), "the only favorite is static");
    settings.set_favorite(TWO_AXIS_FAMILY, true);
    assert_eq!(listed(true, true, &settings), [TWO_AXIS_FAMILY]);
    assert_eq!(listed(false, false, &settings), all);
}

/// The Document set agrees with the families the document's type uses (hidden and locked type included, a missing
/// family left out of nothing: it is named as it is), and gives the same set again when nothing changed.
#[test]
fn the_document_filter_lists_the_families_the_documents_type_uses() {
    let mut app = app_with_type(&["Source Serif 4", "Inter"]);
    app.run("object.hide", serde_json::json!({})).unwrap();
    app.run("text.create", serde_json::json!({"x": 20, "y": 120, "text": "Locked", "font": "Source Sans 3"})).unwrap();
    app.run("object.lock", serde_json::json!({})).unwrap();
    app.run("text.create", serde_json::json!({"x": 20, "y": 150, "text": "Missing", "font": "Missing Family 7059"})).unwrap();
    let db = vectorcraft_text::FontDb::global();
    // Tests running alongside register composite fonts and load fonts in the process-wide database;
    // their changes wait until this test ends, so both listings of every family are the same.
    let _steady = vectorcraft_text::test_fonts::steady_fonts(db);
    let used_by_doc = super::document_families(db, &app.session.active().unwrap().doc);
    let mut names: Vec<_> = used_by_doc.iter().cloned().collect();
    names.sort();
    assert_eq!(names, ["inter", "missing family 7059", "source sans 3", "source serif 4"], "each run's family, as named");
    assert_eq!(used(&mut app), names, "the set the picker reads is the document's");
    assert_eq!(used(&mut app), names, "the same set back when nothing changed");
}

/// Type kept in a graph design counts as the document's type, as type in a symbol does.
#[test]
fn the_document_filter_counts_type_in_graph_designs() {
    let mut app = app_with_type(&["Inter"]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Bar", "font": "JetBrains Mono"})).unwrap()["id"].as_u64().unwrap();
    let art = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    let mut doc = (*app.session.active().unwrap().doc).clone();
    let db = vectorcraft_text::FontDb::global();
    assert!(!super::document_families(db, &doc).contains("jetbrains mono"));
    doc.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Bars".into(), art: std::sync::Arc::new(art) });
    let mut names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    names.sort();
    assert_eq!(names, ["inter", "jetbrains mono"]);
}

/// Type in the art of a stored scatter brush counts as the document's type, as a scatter stroke
/// draws it, after a malformed library entry too. Type in an art or pattern brush's art does not,
/// as those brushes leave type out when they bend their art.
#[test]
fn the_document_filter_counts_type_in_scatter_brushes() {
    let mut app = app_with_type(&["Inter"]);
    let mut art = |font: &str| {
        let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Dot", "font": font})).unwrap()["id"].as_u64().unwrap();
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
        app.run("edit.undo", serde_json::json!({})).unwrap();
        node
    };
    let (scatter, bent, tiled) = (art("JetBrains Mono"), art("Source Serif 4"), art("Source Sans 3"));
    let brush = |name: &str, kind| serde_json::to_value(vectorcraft_brush::Brush { name: name.into(), kind }).unwrap();
    let library = serde_json::json!([
        {"name": "Broken", "type": "scatter", "art": 7},
        brush("Art", vectorcraft_brush::BrushKind::Art(vectorcraft_brush::ArtBrush { art: bent, ..Default::default() })),
        brush("Pattern", vectorcraft_brush::BrushKind::Pattern(vectorcraft_brush::PatternBrush { side: tiled, ..Default::default() })),
        brush("Dots", vectorcraft_brush::BrushKind::Scatter(vectorcraft_brush::Scatter { art: scatter, ..Default::default() })),
    ]);
    let mut doc = (*app.session.active().unwrap().doc).clone();
    doc.unknown.insert(vectorcraft_brush::DOC_KEY.into(), library);
    let db = vectorcraft_text::FontDb::global();
    let mut names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    names.sort();
    assert_eq!(names, ["inter", "jetbrains mono"]);
}

/// A scatter brush made of type keeps its family in the Document filter after the type is deleted;
/// deleting the brush takes the family out, and undoing the deletion brings it back.
#[test]
fn the_document_filter_follows_a_scatter_brushs_edits_and_undo() {
    let mut app = app_with_type(&["Inter"]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Dot", "font": "JetBrains Mono"})).unwrap()["id"].clone();
    let name = app.run("brush.new", serde_json::json!({"type": "scatter", "name": "Type Dots"})).unwrap()["name"].clone();
    app.run("edit.clear", serde_json::json!({"ids": [id]})).unwrap();
    // The set is cached in `app.canvas.document_families`, so each edit has to get past what the cache keeps.
    assert_eq!(used(&mut app), ["inter", "jetbrains mono"], "the brush's type");
    app.run("brush.delete", serde_json::json!({"name": name})).unwrap();
    assert_eq!(used(&mut app), ["inter"], "the brush deleted");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(used(&mut app), ["inter", "jetbrains mono"], "the deletion undone");
}

/// A run whose family and style were split from a PostScript name (as an importer without the font
/// splits one) counts under the family of the installed font of that name, found with its style.
#[test]
fn the_document_filter_finds_a_postscript_name_split_into_family_and_style() {
    let mut app = app_with_type(&[]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Split", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    let mut art = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &mut art.kind else { panic!("type") };
    for r in &mut t.runs {
        // SourceSans3-Semibold, split after "Semi".
        r.style.font_family = "SourceSans3Semi".into();
        r.style.font_style = "bold".into();
    }
    let mut doc = (*app.session.active().unwrap().doc).clone();
    doc.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Split".into(), art: std::sync::Arc::new(art) });
    let db = vectorcraft_text::FontDb::global();
    let names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    assert_eq!(names, ["source sans 3"]);
}

/// The one filter store: a filter set through one picker is on in the next one, and `store` keeps it in
/// `app.ui.font_filters`, where every picker's binding reads it.
#[test]
fn a_filter_chosen_in_one_panels_menu_is_on_in_the_others() {
    let mut app = app_with_type(&[]);
    let before = super::picker_settings(&app);
    let mut first = super::hold(&mut app);
    assert!(!first.filters.favorites && !first.filters.variable, "off by default");
    first.filters.favorites = true;
    first.filters.variable = true;
    first.store(&mut app, &before);
    assert!(app.ui.font_filters.favorites && app.ui.font_filters.variable, "kept in the shared store");
    let second = super::hold(&mut app);
    assert!(second.filters.favorites && second.filters.variable, "on in the next picker");
    let mut third = super::hold(&mut app);
    third.filters.clear();
    third.store(&mut app, &before);
    assert!(!super::hold(&mut app).filters.any(), "cleared everywhere");
}

/// A font previewed from the picker does not change what the Document set lists: the family the previewed text had
/// stays listed, though no other type uses it. A pick follows the document, and so does undo.
#[test]
fn a_preview_keeps_the_document_filters_families() {
    let owner = egui::Id::unique("font-test");
    let mut app = app_with_type(&["Source Serif 4", "Inter"]);
    assert_eq!(used(&mut app), ["inter", "source serif 4"]);
    // The selected text, the only type in Inter, previewed in Source Serif 4.
    super::apply_preview(&mut app, owner, Preview::Shows("Source Serif 4".into()));
    assert!(app.session.active().unwrap().interaction.is_some(), "the preview is open");
    assert_eq!(used(&mut app), ["inter", "source serif 4"], "the preview leaves the set as it was");
    super::apply_preview(&mut app, owner, Preview::Shows("Inter".into()));
    assert_eq!(used(&mut app), ["inter", "source serif 4"], "previewing back changes nothing");
    super::apply_family(&mut app, set("Inter", "Source Serif 4"), None);
    assert_eq!(used(&mut app), ["source serif 4"], "the pick is an edit: Inter is no longer used");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(used(&mut app), ["inter", "source serif 4"], "undo lists it again");
}

/// A document without type has an empty Document set.
#[test]
fn the_document_filter_on_an_empty_document_or_none() {
    let mut app = app_with_type(&[]);
    assert!(used(&mut app).is_empty());
}

/// A fixed time the Recently Added tests count from.
fn clock() -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000)
}

const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Runs `run` until the process-wide font database held still through it: tests running alongside add fonts to it,
/// and the cache under test is keyed by its generation.
fn while_fonts_hold_still<T>(mut run: impl FnMut() -> T) -> T {
    let db = vectorcraft_text::FontDb::global();
    for _ in 0..20 {
        // The font folders' first scan changes the generation: it runs before the generation is read.
        db.family_list();
        let generation = db.generation();
        let out = run();
        if db.generation() == generation {
            return out;
        }
    }
    panic!("the fonts kept changing");
}

/// Seeds the Recently Added cache with `seeded`, found at `found_at` and good until `recheck_at`.
fn seed_recently_added(app: &mut VectorcraftApp, found_at: SystemTime, recheck_at: Option<SystemTime>) {
    let generation = vectorcraft_text::FontDb::global().generation();
    app.canvas.recently_added = Some((generation, found_at, Arc::from(vec!["Seeded".to_string()]), recheck_at));
}

/// The cached list is kept until its recheck time and the database is read again from then on.
#[test]
fn the_recently_added_filter_drops_families_as_their_30_days_end() {
    let (cached, later) = while_fonts_hold_still(|| {
        let mut app = app_with_type(&[]);
        seed_recently_added(&mut app, clock(), Some(clock() + DAY));
        (super::recently_added_set(&mut app, clock()).to_vec(), super::recently_added_set(&mut app, clock() + DAY).to_vec())
    });
    assert_eq!(cached, ["Seeded"], "kept before its recheck time");
    assert!(!later.contains(&"Seeded".to_string()), "read again at its recheck time: {later:?}");
}

/// When the clock goes back before the time the list was found for (the system clock is set back), the
/// families are found again rather than the list kept.
#[test]
fn the_recently_added_filter_finds_its_families_again_when_the_clock_goes_back() {
    let (cached, earlier) = while_fonts_hold_still(|| {
        let mut app = app_with_type(&[]);
        seed_recently_added(&mut app, clock(), None);
        (super::recently_added_set(&mut app, clock()).to_vec(), super::recently_added_set(&mut app, clock() - DAY).to_vec())
    });
    assert_eq!(cached, ["Seeded"]);
    assert!(!earlier.contains(&"Seeded".to_string()), "found again a day earlier: {earlier:?}");
}

/// The size a picker writes becomes Font Preview Size, which sizes every picker's rows, and the host is asked to
/// write the preferences file. The document is left as it was: a font previewed on the selected type stays
/// previewed, and the change adds no undo step.
#[test]
fn a_preview_size_asked_for_sets_the_preference_and_keeps_the_preview() {
    // PLAN: assumed `craft::font_manager::FontPreviewSize` is the picker's size enum
    use craft::font_manager::picker::FontPreviewSize;
    let owner = egui::Id::unique("font-test");
    let mut app = app_with_type(&[]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Size", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", serde_json::json!({"ids": [id]})).unwrap();
    let font = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    super::apply_preview(&mut app, owner, Preview::Shows("Source Serif 4".into()));
    assert_eq!(font(&app), "Source Serif 4");
    assert!(!app.take_prefs_changed(), "nothing to write yet");
    let undo_depth = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let depth = undo_depth(&app);
    // The size the rows have: nothing changes.
    let before = super::picker_settings(&app);
    super::store_picker_settings(&mut app, &before, before.clone());
    assert!(!app.take_prefs_changed(), "nothing to write for the size the rows have");
    let mut after = before.clone();
    after.preview = FontPreviewSize::Large;
    super::store_picker_settings(&mut app, &before, after);
    assert_eq!(app.session.prefs.font_preview_size, "large");
    assert_eq!(undo_depth(&app), depth, "the size is not an undo step");
    assert!(app.take_prefs_changed(), "the host is asked to write the preferences file");
    assert_eq!(app.ui.engine_prefs["fontPreviewSize"], "large");
    assert!(app.session.in_interaction(), "the preview is still open");
    assert_eq!(font(&app), "Source Serif 4", "the preview stays on the type");
    super::apply_preview(&mut app, owner, Preview::Ended);
    assert_eq!(font(&app), "Inter");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert!(
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_none(),
        "undo takes the type away: no step was added for the size"
    );
}
