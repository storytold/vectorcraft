//! Public-command witnesses for the new #7583 review findings.
//! CONTROL failures invalidate a fixture; REGRESSION failures are the final bug oracles.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_color::cms;
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint, Swatch};
use vectorcraft_doc::{Appearance, CompositeFont, Document, Node, NodeId, NodeKind, PatternDef, Symbol, SymbolOverride};
use vectorcraft_engine::{Prefs, Session};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_text::FontDb;

struct FixtureContext {
    fonts: Vec<CompositeFont>,
    working: cms::ColorSettings,
    active: cms::ColorSettings,
    _lock: MutexGuard<'static, ()>,
}

impl FixtureContext {
    fn hold() -> Self {
        static LOCK: Mutex<()> = Mutex::new(());
        let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let db = FontDb::global();
        let fonts = db.composite_font_names().iter().map(|name| (*db.composite_font(name).unwrap()).clone()).collect();
        let guard = Self { fonts, working: cms::working_settings(), active: cms::active_settings(), _lock: lock };
        cms::set_active(&cms::ColorSettings::default()).expect("CONTROL: builtin color context is available");
        guard
    }
}

impl Drop for FixtureContext {
    fn drop(&mut self) {
        let db = FontDb::global();
        for name in db.composite_font_names() {
            if CompositeFont::named(&self.fonts, &name).is_none() {
                db.remove_composite_font(&name);
            }
        }
        for font in &self.fonts {
            db.set_composite_font(font);
        }
        cms::set_active(&self.working).unwrap();
        cms::use_document_profiles(Some(&self.active.rgb), Some(&self.active.cmyk));
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(&base).unwrap();
        let name = format!("vc-review-7583-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = base.join(name);
        std::fs::create_dir(&path).expect("CONTROL: create this test's unique directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).canonicalize().unwrap();
        if let Ok(path) = self.0.canonicalize() {
            assert_eq!(path.parent(), Some(base.as_path()), "cleanup stays directly inside the test temporary root");
            assert!(path.file_name().unwrap().to_string_lossy().starts_with("vc-review-7583-"));
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn document(session: &Session) -> &Document {
    &session.doc().expect("CONTROL: an active document exists").doc
}

fn put(d: &mut Document, node: Node) {
    Arc::make_mut(&mut d.layers[0]).children_mut().unwrap().push(Arc::new(node));
    d.fix_next_id();
}

fn rectangle(d: &mut Document, fill: Paint) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 6.0)), Appearance::basic(fill, Paint::None, 0.0))
}

fn write_native(path: &Path, d: &Document) -> Document {
    let options = vectorcraft_format::SaveOptions { include_linked: true, precision: None, ..Default::default() };
    let bytes = vectorcraft_format::save_with(d, &options).expect("CONTROL: serialize the small typed native fixture");
    std::fs::write(path, &bytes).unwrap();
    vectorcraft_format::load(&bytes).expect("CONTROL: native format reads its own fixture bytes")
}

fn restarted(session: &Session) -> Session {
    let prefs: Prefs = serde_json::from_slice(&serde_json::to_vec(&session.prefs).unwrap()).unwrap();
    let mut fresh = Session::new();
    fresh.apply_prefs(prefs);
    fresh
}

fn font_document(font: &CompositeFont) -> Document {
    let mut session = Session::new();
    session.add_document(Document::new(80.0, 60.0), None);
    session.execute("type.compositeFonts.set", &json!({"fonts": [font]})).expect("CONTROL: ordinary source font is valid");
    session.execute("text.create", &json!({"x": 4, "y": 16, "text": "Ab", "font": font.name})).expect("CONTROL: source lettering is created");
    document(&session).clone()
}

fn placed_family(session: &Session, result: &Value) -> String {
    assert_eq!(result["linked"], false, "CONTROL: this is native adoption, not a linked-file placeholder");
    let id = NodeId(result["ids"][0].as_u64().expect("CONTROL: placement returns one root"));
    let root = document(session).node(id).expect("CONTROL: returned root exists");
    let mut families = Vec::new();
    root.walk(&mut |node| {
        if let NodeKind::Text(text) = &node.kind {
            families.extend(text.runs.iter().filter(|run| run.text == "Ab").map(|run| run.style.font_family.clone()));
        }
    });
    assert_eq!(families.len(), 1, "CONTROL: one original source lettering run is placed");
    families.pop().unwrap()
}

fn repeat_alias_case(conflict: bool) -> (String, String, usize, usize) {
    let directory = TestDirectory::new();
    let path = directory.0.join("lettering.vectorcraft");
    let incoming = CompositeFont::new("ReviewRepeatLettering", "Source Sans 3", "Regular");
    let loaded = write_native(&path, &font_document(&incoming));
    assert_eq!(CompositeFont::named(&loaded.composite_fonts, &incoming.name), Some(&incoming));
    let mut destination = incoming.clone();
    if conflict {
        destination.parts.roman.size = 125.0;
    }
    let mut session = Session::new();
    session.add_document(Document::new(100.0, 80.0), None);
    let mut destination_fonts = Vec::new();
    if !conflict {
        // An earlier equivalent font with a different name must not steal an exact-name match.
        destination_fonts.push(CompositeFont { name: "ReviewEquivalentOtherName".into(), ..destination.clone() });
    }
    destination_fonts.push(destination.clone());
    session.execute("type.compositeFonts.set", &json!({"fonts": destination_fonts})).unwrap();
    let existing = session.execute("text.create", &json!({"x": 2, "y": 12, "text": "Destination", "font": destination.name})).unwrap();
    let existing_id = NodeId(existing["id"].as_u64().unwrap());
    let existing_node = document(&session).node(existing_id).unwrap().clone();
    let first =
        session.execute("file.place", &json!({"path": path, "link": false, "at": [30, 25]})).expect("CONTROL: first ordinary import succeeds");
    let first_family = placed_family(&session, &first);
    let first_count = document(&session).composite_fonts.len();
    assert_eq!(first_count, 2, "CONTROL: first import creates only a needed definition or keeps the two existing named definitions");
    assert_eq!(CompositeFont::same_name(&first_family, &incoming.name), !conflict);
    let held = CompositeFont::named(&document(&session).composite_fonts, &first_family).unwrap();
    assert_eq!(held.parts, incoming.parts, "CONTROL: imported family means the source's parts");
    assert_eq!(held.custom, incoming.custom);
    let first_definition = held.clone();
    let second =
        session.execute("file.place", &json!({"path": path, "link": false, "at": [60, 45]})).expect("CONTROL: repeated small import succeeds");
    let second_family = placed_family(&session, &second);
    assert_eq!(
        CompositeFont::named(&document(&session).composite_fonts, &destination.name),
        Some(&destination),
        "CONTROL: destination definition is preserved"
    );
    assert_eq!(document(&session).node(existing_id), Some(&existing_node), "CONTROL: destination lettering is unchanged");
    assert_eq!(
        CompositeFont::named(&document(&session).composite_fonts, &first_family),
        Some(&first_definition),
        "CONTROL: first imported definition remains intact"
    );
    (first_family, second_family, first_count, document(&session).composite_fonts.len())
}

#[test]
fn review7583_repeat_native_import_reuses_existing_conflict_alias() {
    let _context = FixtureContext::hold();
    let (first, second, first_count, second_count) = repeat_alias_case(true);
    assert_eq!(
        (second, second_count),
        (first, first_count),
        "REGRESSION: repeating identical art reuses its existing alias without another definition"
    );
}

#[test]
fn review7583_repeat_equal_native_font_is_control() {
    let _context = FixtureContext::hold();
    let (first, second, first_count, second_count) = repeat_alias_case(false);
    assert_eq!((second, second_count), (first, first_count), "CONTROL: equal original-name definitions already reuse normally");
}

fn queue_names(reply: &Value) -> Vec<String> {
    reply["files"].as_array().unwrap().iter().map(|file| file["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn review7583_thumbnail_queue_skips_one_rejected_native_file_and_continues() {
    let _context = FixtureContext::hold();
    let directory = TestDirectory::new();
    let first = directory.0.join("first.vectorcraft");
    let rejected = directory.0.join("rejected.vectorcraft");
    let last = directory.0.join("last.vectorcraft");
    let mut valid = Document::new(20.0, 20.0);
    let art = rectangle(&mut valid, Paint::solid(Color::rgb8(30, 80, 120)));
    put(&mut valid, art);
    write_native(&first, &valid);
    write_native(&last, &valid);
    let font = CompositeFont::new("ReviewRejectedLettering", "Source Sans 3", "Regular");
    let mut bad = font_document(&font);
    bad.composite_fonts[0].parts.roman.size = 9.0;
    let expected_error = bad.composite_fonts[0].check().expect_err("CONTROL: small out-of-range Roman size is an ordinary typed validation error");
    let loaded = write_native(&rejected, &bad);
    assert_eq!(loaded.composite_fonts[0].parts.roman.size, 9.0, "CONTROL: native reading does not normalize away this validation input");
    assert_eq!(loaded.composite_fonts[0].check().unwrap_err(), expected_error);
    let mut session = Session::new();
    session.add_document(Document::new(80.0, 60.0), None);
    let info = session
        .execute("file.place.info", &json!({"path": rejected, "link": false}))
        .expect("CONTROL: metadata load succeeds before thumbnail adoption");
    assert!(info["width"].as_f64().unwrap() > 0.0 && info["height"].as_f64().unwrap() > 0.0);
    let thumbnail_error = session
        .execute("file.place.info", &json!({"path": rejected, "link": false, "thumbnail": 8}))
        .expect_err("CONTROL: public thumbnail reaches font validation in native build");
    assert!(
        thumbnail_error.to_string().contains(&expected_error),
        "CONTROL: error must be the chosen font check, not an unrelated renderer/read error: {thumbnail_error}"
    );
    let ordinary = session
        .execute("file.place.queue", &json!({"paths": [first, last], "link": false, "thumbnail": 8}))
        .expect("CONTROL: ordinary small native thumbnails render");
    assert_eq!(queue_names(&ordinary), ["first.vectorcraft", "last.vectorcraft"]);
    assert_eq!(ordinary["count"], 2);
    assert!(ordinary["skipped"].as_array().unwrap().is_empty());
    let metadata = session
        .execute("file.place.queue", &json!({"paths": [first, rejected, last], "link": false}))
        .expect("CONTROL: the same three files remain readable without thumbnail adoption");
    assert_eq!(metadata["count"], 3);
    let result = session.execute("file.place.queue", &json!({"paths": [first, rejected, last], "link": false, "thumbnail": 8}));
    let reply = match result {
        Ok(reply) => reply,
        Err(error) => panic!("REGRESSION: one rejected thumbnail aborts the valid/rejected/valid queue: {error}"),
    };
    assert_eq!(
        queue_names(&reply),
        ["first.vectorcraft", "last.vectorcraft"],
        "REGRESSION: valid files before and after the rejected file remain ordered"
    );
    assert_eq!(reply["count"], 2);
    let skipped = reply["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["name"], "rejected.vectorcraft");
    assert!(skipped[0]["error"].as_str().unwrap().contains(&expected_error));
    assert_eq!(session.tool_id(), "place", "CONTROL: the public queue loads the Place cursor");
    let options = session.tool_options();
    let queued = options["names"].as_array().expect("Place tool exposes queued file names");
    assert_eq!(options["count"], 2, "REGRESSION: a thumbnail rejection leaves no stale cursor entry");
    assert_eq!(queued.iter().map(|name| name.as_str().unwrap()).collect::<Vec<_>>(), ["first.vectorcraft", "last.vectorcraft"]);
}

const NATIVE_DOTS: &str = "ReviewNativeDots";
const NATIVE_TILE: &str = "Review native tile";

fn native_override_document() -> (Document, PatternDef) {
    let mut d = Document::new(40.0, 30.0);
    d.swatches.clear();
    d.swatch_groups.clear();
    let dots = rectangle(&mut d, Paint::solid(Color::rgb8(31, 81, 123)));
    let definition = PatternDef::new(NATIVE_DOTS, vec![Arc::new(dots)]);
    d.patterns.push(definition.clone());
    let base = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 6.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 1.0),
    );
    d.symbols.push(Symbol { name: "Review native dynamic".into(), art: Arc::new(base), dynamic: true });
    let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Review native dynamic".into(), xf: Affine::IDENTITY });
    instance.appearance = Appearance::basic(Paint::None, Paint::None, 0.0).into();
    instance.extras_mut().symbol_overrides.push(SymbolOverride {
        path: vec![],
        fill: Some(Paint::Pattern { pattern: NATIVE_DOTS.into(), xf: Affine::IDENTITY }),
        stroke: Some(Paint::Pattern { pattern: NATIVE_DOTS.into(), xf: Affine::scale(2.0) }),
        opacity: Some(0.65),
        blend: None,
    });
    put(&mut d, instance);
    (d, definition)
}

fn pattern_matches(actual: &PatternDef, expected: &PatternDef) -> bool {
    actual.tile == expected.tile
        && actual.art.len() == 1
        && expected.art.len() == 1
        && actual.art[0].appearance == expected.art[0].appearance
        && actual.art[0].kind == expected.art[0].kind
}

fn native_override_case(directory: &Path, collision: bool) -> Vec<String> {
    let path = directory.join(if collision { "native-collision.vectorcraft" } else { "native-blank.vectorcraft" });
    let (native, expected) = native_override_document();
    let loaded = write_native(&path, &native);
    assert!(pattern_matches(loaded.pattern(NATIVE_DOTS).unwrap(), &expected), "CONTROL: native input contains the intended pattern");
    assert!(loaded.swatch(NATIVE_DOTS).is_none(), "CONTROL: no visible swatch seeds this pattern dependency");
    let instance = loaded.layers[0].children().unwrap().first().unwrap();
    let NodeKind::SymbolInstance { symbol, .. } = &instance.kind else { panic!("CONTROL: input is a dynamic instance") };
    let base = loaded.symbols.iter().find(|entry| &entry.name == symbol).unwrap();
    assert!(base.dynamic);
    assert!(matches!(base.art.appearance.fill_paint(), Paint::Solid { .. }));
    assert!(matches!(base.art.appearance.stroke().unwrap().paint, Paint::Solid { .. }));
    assert!(instance.appearance.fill_paint().is_none(), "CONTROL: no instance stain hides the override");
    assert_eq!(instance.extras().symbol_overrides.len(), 1);
    let effective = vectorcraft_doc::symbol_overrides::instance_art(&base.art, instance);
    assert_eq!(Some(effective.appearance.fill_paint()), instance.extras().symbol_overrides[0].fill);
    assert_eq!(Some(effective.appearance.stroke().unwrap().paint.clone()), instance.extras().symbol_overrides[0].stroke);
    let mut outer = Document::new(80.0, 60.0);
    outer.swatches.clear();
    outer.swatch_groups.clear();
    let existing = if collision {
        let art = rectangle(&mut outer, Paint::solid(Color::rgb8(190, 20, 30)));
        let pattern = PatternDef::new(NATIVE_DOTS, vec![Arc::new(art)]);
        outer.patterns.push(pattern.clone());
        outer.swatches.push(Swatch {
            name: "Existing dots".into(),
            paint: Paint::Pattern { pattern: NATIVE_DOTS.into(), xf: Affine::IDENTITY },
            global: false,
            spot: false,
        });
        Some(pattern)
    } else {
        None
    };
    let mut source = Session::new();
    source.add_document(outer, None);
    let placed = source.execute("file.place", &json!({"path": path, "link": true})).expect("CONTROL: linked native placement succeeds");
    assert_eq!(placed["linked"], true);
    let id = placed["ids"][0].as_u64().unwrap();
    assert!(matches!(document(&source).node(NodeId(id)).unwrap().kind, NodeKind::PlacedDocument(_)));
    assert_eq!(document(&source).pattern(NATIVE_DOTS).is_some(), collision, "CONTROL: native-only pattern has not already been adopted");
    source.execute("object.pattern.make", &json!({"ids": [id], "name": NATIVE_TILE, "edit": false})).unwrap();
    source.execute("select.set", &json!({"ids": [id]})).unwrap();
    source.execute("edit.clear", &json!({})).unwrap();
    let original = document(&source).clone();
    source.execute("swatch.defaults.save", &json!({})).expect("CONTROL: capture completes before the retained-resource oracle");
    assert_eq!(document(&source), &original, "CONTROL: capture does not edit the source document");
    std::fs::remove_file(&path).expect("CONTROL: remove only this case's original native file");
    assert!(!path.exists());
    let mut fresh = restarted(&source);
    fresh
        .execute("file.new", &json!({"profile": null}))
        .expect("CONTROL: reopen completes after removing the source, before retained-resource oracles");
    let d = document(&fresh);
    if let Some(existing) = existing {
        assert!(
            pattern_matches(d.pattern(NATIVE_DOTS).expect("CONTROL: existing pattern survives"), &existing),
            "CONTROL: original colliding definition keeps its payload"
        );
    }
    let tile = match &d.swatch(NATIVE_TILE).expect("CONTROL: saved tile swatch exists").paint {
        Paint::Pattern { pattern, .. } => d.pattern(pattern).expect("CONTROL: saved tile resolves"),
        _ => panic!("CONTROL: saved tile remains a pattern"),
    };
    let mut instances = Vec::new();
    for root in &tile.art {
        root.walk(&mut |node| {
            if matches!(node.kind, NodeKind::SymbolInstance { .. }) {
                instances.push(node.clone());
            }
        });
    }
    assert_eq!(instances.len(), 1, "CONTROL: native expansion retained one original dynamic instance");
    let overrides = &instances[0].extras().symbol_overrides;
    assert_eq!(overrides.len(), 1, "CONTROL: the original override record remains");
    let mut regressions = Vec::new();
    for (kind, paint, expected_xf) in [("fill", &overrides[0].fill, Affine::IDENTITY), ("stroke", &overrides[0].stroke, Affine::scale(2.0))] {
        let valid = match paint {
            Some(Paint::Pattern { pattern, xf }) => *xf == expected_xf && d.pattern(pattern).is_some_and(|actual| pattern_matches(actual, &expected)),
            _ => false,
        };
        if !valid {
            regressions.push(format!("collision={collision}: {kind} override lost its native-only original pattern payload/placement"));
        }
    }
    regressions
}

#[test]
fn review7583_native_override_patterns_survive_linked_tile_capture_and_restart() {
    let _context = FixtureContext::hold();
    let directory = TestDirectory::new();
    let mut regressions = Vec::new();
    for collision in [false, true] {
        regressions.extend(native_override_case(&directory.0, collision));
    }
    assert!(regressions.is_empty(), "REGRESSION: native override fill/stroke dependency or remapping failed: {regressions:?}");
}

const RAMP: &str = "Review ramp";
const GRADIENT_ART: &str = "Review gradient art";

fn ramp(changed: bool) -> Gradient {
    let end = if changed { Color::rgb8(20, 180, 90) } else { Color::rgb8(180, 30, 80) };
    Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::rgb8(10, 40, 120)), GradientStop::new(1.0, end)])
}

fn ramp_swatch(gradient: Gradient) -> Swatch {
    Swatch { name: RAMP.into(), paint: Paint::Gradient(Box::new(GradientPaint::new(gradient))), global: false, spot: false }
}

fn gradient_profile_with(gradient: Gradient) -> (Document, GradientPaint) {
    let mut d = Document::new(80.0, 60.0);
    d.swatches = vec![ramp_swatch(gradient.clone())].into();
    d.swatch_groups.clear();
    let mut paint = GradientPaint::new(gradient);
    paint.swatch = Some(RAMP.into());
    paint.geom = Some(GradientGeom { start: Point::new(1.5, 2.5), end: Point::new(7.5, 4.5), aspect: 1.0, focal: None });
    paint.angle = 17.0;
    let mut art = rectangle(&mut d, Paint::Gradient(Box::new(paint.clone())));
    art.name = Some(GRADIENT_ART.into());
    put(&mut d, art);
    (d, paint)
}

fn gradient_profile() -> (Document, GradientPaint) {
    gradient_profile_with(ramp(false))
}

fn gradient_art(d: &Document) -> GradientPaint {
    let mut paints = Vec::new();
    d.visit_stored(|node, _| {
        if node.name.as_deref() == Some(GRADIENT_ART)
            && let Paint::Gradient(gradient) = node.appearance.fill_paint()
        {
            paints.push(*gradient);
        }
    });
    assert_eq!(paints.len(), 1, "CONTROL: one named original gradient object remains");
    paints.pop().unwrap()
}

// These fixtures have ordinary existing global-stop links. Compare their cached colors
// in the document's public global-color context, rather than raw Lab versus CMYK caches.
fn logical_gradient_in_document(d: &Document, gradient: &Gradient) -> Gradient {
    let mut normalized = gradient.clone();
    for stop in &mut normalized.stops {
        if let Some(name) = stop.swatch.as_deref()
            && let Some(base) = d.global_color(name)
        {
            stop.color = base.tinted(stop.tint);
        }
    }
    normalized
}

fn valid_gradient_link(d: &Document, paint: &GradientPaint) -> bool {
    match paint.swatch.as_deref() {
        None => true,
        Some(name) => match d.swatch(name).map(|swatch| &swatch.paint) {
            Some(Paint::Gradient(swatch)) => logical_gradient_in_document(d, &swatch.gradient) == logical_gradient_in_document(d, &paint.gradient),
            _ => false,
        },
    }
}

fn created_from_profile(profile: &Document, palette: Document) -> Document {
    const RESOURCE: &str = "Review retained category gradient";
    let directory = TestDirectory::new();
    let mut category = profile.clone();
    assert!(category.symbols.is_empty(), "CONTROL: no other symbol resource seeds this fixture");
    assert_eq!(category.layers.len(), 1, "CONTROL: one ordinary source layer");
    let art = category.layers[0]
        .children()
        .unwrap()
        .iter()
        .find(|node| node.name.as_deref() == Some(GRADIENT_ART))
        .cloned()
        .expect("CONTROL: original gradient becomes the retained category resource");
    category.symbols.push(Symbol { name: RESOURCE.into(), art, dynamic: false });
    // Category profiles intentionally discard layer art, while keeping symbol definitions.
    Arc::make_mut(&mut category.layers[0]).children_mut().unwrap().clear();
    let loaded = write_native(&directory.0.join("Print.vectorcraft"), &category);
    assert_eq!(loaded.symbols.len(), 1, "CONTROL: native profile actually carries the symbol paint consumer");
    let mut source = Session::new();
    source.add_document(palette, None);
    source.execute("swatch.defaults.save", &json!({})).expect("CONTROL: save a small ordinary replacement palette");
    let mut fresh = restarted(&source);
    fresh.attach_new_doc_profiles(directory.0.clone());
    // Match the fixture's RGB mode, so category conversion is not the payload-change oracle.
    fresh.execute("file.new", &json!({"profile": "Print", "colorMode": "rgb"})).expect("CONTROL: normal customized-category new document is created");
    let d = document(&fresh);
    assert_eq!(d.color_mode, category.color_mode, "CONTROL: no unrelated profile color-mode conversion");
    assert!(d.symbols.iter().any(|symbol| symbol.name == RESOURCE), "CONTROL: file.new retains the intended resource");
    assert!(
        d.layers.iter().all(|layer| layer.children().is_some_and(|children| children.is_empty())),
        "CONTROL: category starts with empty layer art"
    );
    d.clone()
}

fn replacement_gradient_case(changed: bool) -> (Document, GradientPaint) {
    let (profile, original) = gradient_profile();
    assert!(valid_gradient_link(&profile, &original), "CONTROL: the original named link matches its profile palette");
    let mut palette = Document::new(80.0, 60.0);
    palette.swatches = vec![ramp_swatch(ramp(changed))].into();
    palette.swatch_groups.clear();
    let d = created_from_profile(&profile, palette);
    let paint = gradient_art(&d);
    let Paint::Gradient(replacement) = &d.swatch(RAMP).expect("CONTROL: replacement ramp exists").paint else {
        panic!("CONTROL: replacement is still a gradient")
    };
    assert_eq!(replacement.gradient, ramp(changed), "CONTROL: the selected saved palette is actually installed");
    assert_eq!(replacement.gradient == original.gradient, !changed, "CONTROL: changed and equal cases have the intended logical definitions");
    assert_eq!(paint.geom, original.geom, "CONTROL: replacing a palette does not move the original gradient vector");
    assert_eq!(paint.angle, original.angle);
    assert_eq!(paint.freeform, original.freeform);
    (d, paint)
}

#[test]
fn review7583_gradient_replacement_never_keeps_an_invalid_named_link() {
    let _context = FixtureContext::hold();
    let (d, paint) = replacement_gradient_case(true);
    // The first oracle identifies the invalid link without depending on the repair choice.
    assert!(valid_gradient_link(&d, &paint), "REGRESSION: a gradient still names the replacement swatch while carrying different stops");
    // Root selected the existing unlink-on-changed-payload contract after reviewing its helpers.
    assert!(paint.swatch.is_none(), "REGRESSION: a stale palette link is cleared");
    assert_eq!(paint.gradient, ramp(false), "REGRESSION: unlinking preserves all original stops and their metadata");
}

#[test]
fn review7583_gradient_equal_definition_and_native_placement_are_controls() {
    let _context = FixtureContext::hold();
    let (equal, paint) = replacement_gradient_case(false);
    assert!(valid_gradient_link(&equal, &paint));
    assert_eq!(paint.swatch.as_deref(), Some(RAMP), "CONTROL: an equal definition retains its legitimate link");
    let directory = TestDirectory::new();
    let path = directory.0.join("gradient.vectorcraft");
    let (profile, _) = gradient_profile();
    write_native(&path, &profile);
    let mut destination = Document::new(80.0, 60.0);
    destination.swatches = vec![ramp_swatch(ramp(true))].into();
    destination.swatch_groups.clear();
    let mut session = Session::new();
    session.add_document(destination, None);
    session
        .execute("file.place", &json!({"path": path, "link": false, "at": [40, 30]}))
        .expect("CONTROL: ordinary native placement with a same-name ramp collision succeeds");
    let placed = gradient_art(document(&session));
    assert_eq!(placed.gradient, ramp(false), "CONTROL: placement preserves the incoming art's gradient payload");
    assert!(valid_gradient_link(document(&session), &placed), "CONTROL: normal placement renames or unlinks an incompatible palette link");
}

#[test]
fn review7583_gradient_stop_edit_is_an_existing_unlink_control() {
    let _context = FixtureContext::hold();
    let (profile, original) = gradient_profile();
    let id = profile.layers[0].children().unwrap()[0].id.0;
    let mut session = Session::new();
    session.add_document(profile, None);
    session
        .execute(
            "paint.editGradient",
            &json!({"ids": [id], "keepModel": true, "stops": [{"offset": 0, "color": "#123456"}, {"offset": 1, "color": "#fedcba"}]}),
        )
        .expect("CONTROL: normal public gradient stop edit succeeds");
    let edited = gradient_art(document(&session));
    assert!(edited.swatch.is_none(), "CONTROL: the existing gradient-edit contract unlinks a changed payload");
    assert_ne!(edited.gradient, original.gradient);
    assert_eq!(edited.geom, original.geom);
}

#[test]
fn review7583_equivalent_lab_spot_gradient_is_compared_after_color_refresh_control() {
    let _context = FixtureContext::hold();
    const SPOT: &str = "Review gradient spot";
    let base = Color::lab(55.0, 38.0, -12.0);
    let mut stop = GradientStop::new(0.0, base.tinted(0.4));
    stop.swatch = Some(SPOT.into());
    stop.tint = 0.4;
    let logical = Gradient::new(GradientKind::Linear, vec![stop, GradientStop::new(1.0, Color::rgb8(10, 30, 80))]);
    let spot = Swatch { name: SPOT.into(), paint: Paint::solid(base), global: true, spot: true };
    let (mut profile, original) = gradient_profile_with(logical.clone());
    profile.swatches = vec![spot.clone(), ramp_swatch(logical.clone())].into();
    profile.spot_use_lab = false;
    profile.color_profiles.cmyk = Some(cms::DEVICE_CMYK.into());
    let mut palette = Document::new(80.0, 60.0);
    palette.swatches = vec![spot, ramp_swatch(logical.clone())].into();
    palette.swatch_groups.clear();
    let d = created_from_profile(&profile, palette);
    assert!(!d.spot_use_lab, "CONTROL: category keeps its ordinary Spot Colors policy");
    cms::use_document_profiles(d.color_profiles.rgb.as_deref(), d.color_profiles.cmyk.as_deref());
    let paint = gradient_art(&d);
    let refreshed = d.global_color(SPOT).expect("CONTROL: ordinary linked Lab spot resolves").tinted(0.4);
    assert_ne!(refreshed, logical.stops[0].color, "CONTROL: this context distinguishes a refreshed cache from raw Lab");
    assert_eq!(paint.gradient.stops[0].color, refreshed, "CONTROL: ordinary existing stop-color refresh actually ran");
    let Paint::Gradient(saved) = &d.swatch(RAMP).unwrap().paint else { panic!("CONTROL: saved gradient remains a gradient") };
    assert_eq!(saved.gradient, logical, "CONTROL: authoritative saved palette keeps raw logical values");
    assert!(valid_gradient_link(&d, &paint), "CONTROL: the two caches describe the same refreshed logical definition");
    assert_eq!(paint.swatch.as_deref(), Some(RAMP), "CONTROL: equivalent refreshed payload retains its named link");
    assert_eq!(paint.geom, original.geom);
    assert_eq!(paint.angle, original.angle);
    assert_eq!(paint.freeform, original.freeform);
    assert_eq!(paint.gradient.stops[0].swatch.as_deref(), Some(SPOT));
    assert_eq!(paint.gradient.stops[0].tint, 0.4);
    assert!(paint.gradient.stops.iter().all(|stop| stop.plates.is_none()), "CONTROL: this does not exercise or change mixed-ink policy");
}
