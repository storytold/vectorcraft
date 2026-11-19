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

mod review_5972993766_adoption {
    use super::*;
    use vectorcraft_doc::{CharStyle, Effect, InlineArt, TextObject, TextRun};

    const BADGE: &str = "Review adopted Badge";
    const LOGO: &str = "Review mapped Logo";
    const TILE: &str = "Review adopted native tile";

    fn adoption_same_node(actual: &Node, expected: &Node) -> bool {
        let mut expected = expected.clone();
        expected.id = actual.id;
        serde_json::to_value(actual).unwrap() == serde_json::to_value(expected).unwrap()
    }

    fn adoption_named_symbol<'a>(d: &'a Document, name: &str) -> &'a Symbol {
        d.symbols.iter().find(|s| s.name == name).expect("CONTROL: fixture symbol exists")
    }

    fn adoption_seed_symbol(d: &mut Document, name: &str, aligned: bool) -> Symbol {
        let art = rectangle(d, Paint::solid(Color::rgb8(190, 20, 30)));
        let symbol = Symbol { name: name.into(), art: Arc::new(art), dynamic: false };
        d.symbols.push(symbol.clone());
        d.set_symbol_pixel_aligned(name, aligned);
        // A separate retained palette consumer keeps the destination definition and option alive.
        let user = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::translate((2.0, 3.0)) });
        let tile = format!("Existing {name} tile");
        d.patterns.push(PatternDef::new(&tile, vec![Arc::new(user)]));
        d.swatches.push(Swatch { name: tile.clone(), paint: Paint::Pattern { pattern: tile, xf: Affine::IDENTITY }, global: false, spot: false });
        symbol
    }

    fn adoption_assert_existing(d: &Document, expected: &Symbol, aligned: bool) {
        let actual = adoption_named_symbol(d, &expected.name);
        assert!(adoption_same_node(&actual.art, &expected.art), "CONTROL: existing destination symbol payload is unchanged");
        assert_eq!(actual.dynamic, expected.dynamic);
        assert_eq!(d.symbol_pixel_aligned(&expected.name), aligned, "CONTROL: existing destination option is unchanged");
        let tile = d.pattern(&format!("Existing {} tile", expected.name)).unwrap();
        assert!(
            matches!(&tile.art[0].kind, NodeKind::SymbolInstance { symbol, xf } if symbol == &expected.name && *xf == Affine::translate((2.0, 3.0))),
            "CONTROL: existing destination user is unchanged"
        );
    }

    fn adoption_capture_native(directory: &Path, label: &str, native: &Document, mut outer: Document) -> Vec<Document> {
        let path = directory.join(format!("{label}.vectorcraft"));
        write_native(&path, native);
        outer.fix_next_id();
        let mut source = Session::new();
        source.add_document(outer, None);
        let placed = source.execute("file.place", &json!({"path": path, "link": true})).expect("CONTROL: native link placement succeeds");
        assert_eq!(placed["linked"], true);
        let id = placed["ids"][0].as_u64().unwrap();
        assert!(matches!(document(&source).node(NodeId(id)).unwrap().kind, NodeKind::PlacedDocument(_)));
        source.execute("object.pattern.make", &json!({"ids": [id], "name": TILE, "edit": false})).unwrap();
        source.execute("select.set", &json!({"ids": [id]})).unwrap();
        source.execute("edit.clear", &json!({})).unwrap();
        let original = document(&source).clone();
        source.execute("swatch.defaults.save", &json!({})).expect("CONTROL: capture reaches retained-resource oracles");
        assert_eq!(document(&source), &original, "CONTROL: capture leaves its source document unchanged");
        let prefs = source.prefs.to_json();
        let bytes = vectorcraft_format::base64_decode(prefs["defaultSwatches"]["data"].as_str().unwrap()).unwrap();
        let capsule = vectorcraft_format::load_file(&bytes).expect("CONTROL: saved native capsule loads").doc;
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists(), "CONTROL: source file is unavailable before restart");
        let mut fresh = restarted(&source);
        fresh.execute("file.new", &json!({"profile": null})).expect("CONTROL: new document loads before resource oracles");
        vec![capsule, document(&fresh).clone()]
    }

    fn adoption_tile_user(d: &Document, inline: bool) -> Node {
        let Paint::Pattern { pattern, .. } = &d.swatch(TILE).expect("CONTROL: saved native tile swatch exists").paint else {
            panic!("CONTROL: saved tile remains a pattern")
        };
        let tile = d.pattern(pattern).expect("CONTROL: saved tile pattern resolves");
        let mut users = Vec::new();
        for root in &tile.art {
            root.walk(&mut |n| {
                if matches!(&n.kind, NodeKind::Text(_) if inline) || matches!(&n.kind, NodeKind::SymbolInstance { .. } if !inline) {
                    users.push(n.clone());
                }
            });
        }
        assert_eq!(users.len(), 1, "CONTROL: one native symbol consumer survives expansion");
        users.remove(0)
    }

    fn adoption_user_name(user: &Node) -> &str {
        match &user.kind {
            NodeKind::Text(t) => {
                assert_eq!(t.runs.len(), 1);
                &t.runs[0].inline.as_ref().expect("CONTROL: native inline graphic survives").symbol
            }
            NodeKind::SymbolInstance { symbol, .. } => symbol,
            _ => panic!("CONTROL: native symbol consumer kind is unchanged"),
        }
    }

    fn adoption_mapped_effect(art: &Node, paint: bool) -> &Effect {
        let effects = if paint { art.appearance.items[0].effects() } else { &art.appearance.effects };
        assert_eq!(effects.len(), 1, "CONTROL: one native mapped effect survives");
        &effects[0]
    }

    #[test]
    fn review_597299_adoption_mapped_inline_symbols_survive_capture_and_collision() {
        let _context = FixtureContext::hold();
        let directory = TestDirectory::new();
        let mut regressions = Vec::new();
        for paint in [false, true] {
            for collision in [false, true] {
                let mut native = Document::new(80.0, 60.0);
                let logo = rectangle(&mut native, Paint::solid(Color::rgb8(29, 71, 113)));
                native.symbols.push(Symbol { name: LOGO.into(), art: Arc::new(logo), dynamic: false });
                let mut badge = rectangle(&mut native, Paint::solid(Color::rgb8(90, 140, 40)));
                let effect =
                    Effect { id: vectorcraft_render::effects::ROTATE.into(), params: json!({"graphics": [{"symbol": LOGO}]}), visible: true };
                vectorcraft_render::effects::validate_three_d_at_in(Some(&native), &badge, &effect.id, &effect.params, (paint.then_some(0), 0))
                    .expect("CONTROL: the small native graphic is valid on this actual appearance location");
                if paint {
                    badge.appearance.items[0].effects_mut().push(effect)
                } else {
                    badge.appearance.effects.push(effect)
                }
                native.symbols.push(Symbol { name: BADGE.into(), art: Arc::new(badge), dynamic: false });
                let mut text = TextObject::point(Point::new(12.0, 16.0), "", CharStyle::default());
                text.runs = vec![TextRun::inline(InlineArt::new(BADGE), CharStyle::default())];
                let text = Node::new(native.alloc_id(), NodeKind::Text(Box::new(text)));
                put(&mut native, text);
                let path = directory.0.join(format!("mapped-input-{paint}-{collision}.vectorcraft"));
                let loaded = write_native(&path, &native);
                let expected_badge = adoption_named_symbol(&loaded, BADGE);
                let expected_logo = adoption_named_symbol(&loaded, LOGO);
                let input_effect = adoption_mapped_effect(&expected_badge.art, paint);
                assert_eq!(input_effect.params["graphics"][0]["symbol"], LOGO);
                vectorcraft_render::effects::validate_three_d_at_in(
                    Some(&loaded),
                    &expected_badge.art,
                    &input_effect.id,
                    &input_effect.params,
                    (paint.then_some(0), 0),
                )
                .expect("CONTROL: native reading retains valid supported mapped art and its effect location");
                assert_eq!(vectorcraft_render::effects::appearance_graphic_symbols(&expected_badge.art.appearance), vec![LOGO.to_string()]);
                assert_eq!(loaded.symbols.len(), 2, "CONTROL: Logo has no independent symbol-instance seed");
                let mut outer = Document::new(80.0, 60.0);
                outer.swatches.clear();
                outer.swatch_groups.clear();
                let existing = collision.then(|| adoption_seed_symbol(&mut outer, LOGO, true));
                for (stage, d) in
                    adoption_capture_native(&directory.0, &format!("mapped-{paint}-{collision}"), &loaded, outer).into_iter().enumerate()
                {
                    if let Some(existing) = &existing {
                        adoption_assert_existing(&d, existing, true);
                    }
                    let user = adoption_tile_user(&d, true);
                    let Some(adopted) = d.symbols.iter().find(|s| s.name == adoption_user_name(&user)) else {
                        regressions.push(format!("paint={paint}, collision={collision}, stage={stage}: inline Badge missing"));
                        continue;
                    };
                    let effect = adoption_mapped_effect(&adopted.art, paint);
                    let name = effect.params["graphics"][0]["symbol"].as_str().expect("CONTROL: native mapped symbol name survives");
                    let kept = d.symbols.iter().find(|s| s.name == name);
                    if kept.is_none_or(|s| !adoption_same_node(&s.art, &expected_logo.art)) || (name == LOGO) == collision {
                        regressions.push(format!(
                            "paint={paint}, collision={collision}, stage={stage}: mapped Logo missing or resolves to wrong source payload/name"
                        ));
                    }
                    let mut expected = expected_badge.art.as_ref().clone();
                    let effects = if paint { expected.appearance.items[0].effects_mut() } else { &mut expected.appearance.effects };
                    effects[0].params["graphics"][0]["symbol"] = json!(name);
                    assert!(
                        adoption_same_node(&adopted.art, &expected),
                        "CONTROL: only allocated ID and verified mapped resource name differ in native Badge"
                    );
                }
            }
        }
        assert!(regressions.is_empty(), "REGRESSION: native mapped-symbol dependency/remap failed: {regressions:?}");
    }

    #[test]
    fn review_597299_adoption_symbol_pixel_options_survive_capture_and_collision() {
        let _context = FixtureContext::hold();
        let directory = TestDirectory::new();
        let mut regressions = Vec::new();
        for aligned in [false, true] {
            for collision in [false, true] {
                let mut native = Document::new(80.0, 60.0);
                let badge = rectangle(&mut native, Paint::solid(Color::rgb8(29, 71, 113)));
                native.symbols.push(Symbol { name: BADGE.into(), art: Arc::new(badge), dynamic: false });
                native.set_symbol_pixel_aligned(BADGE, aligned);
                let instance = Node::new(native.alloc_id(), NodeKind::SymbolInstance { symbol: BADGE.into(), xf: Affine::translate((12.0, 16.0)) });
                put(&mut native, instance);
                let path = directory.0.join(format!("pixel-input-{aligned}-{collision}.vectorcraft"));
                let loaded = write_native(&path, &native);
                assert_eq!(loaded.symbol_pixel_aligned(BADGE), aligned, "CONTROL: native read retains the actual source option");
                let expected = adoption_named_symbol(&loaded, BADGE);
                let mut outer = Document::new(80.0, 60.0);
                outer.swatches.clear();
                outer.swatch_groups.clear();
                let existing = collision.then(|| adoption_seed_symbol(&mut outer, BADGE, !aligned));
                for (stage, d) in
                    adoption_capture_native(&directory.0, &format!("pixel-{aligned}-{collision}"), &loaded, outer).into_iter().enumerate()
                {
                    if let Some(existing) = &existing {
                        adoption_assert_existing(&d, existing, !aligned);
                    }
                    let user = adoption_tile_user(&d, false);
                    let name = adoption_user_name(&user);
                    let adopted = adoption_named_symbol(&d, name);
                    assert!(adoption_same_node(&adopted.art, &expected.art), "CONTROL: source Badge art is retained independently of its option");
                    assert_eq!(name == BADGE, !collision, "CONTROL: only a real name collision allocates an alias");
                    if d.symbol_pixel_aligned(name) != aligned {
                        regressions.push(format!(
                            "aligned={aligned}, collision={collision}, stage={stage}: native option lost at actual adopted name {name}"
                        ));
                    }
                }
            }
        }
        assert!(regressions.is_empty(), "REGRESSION: native adopted symbol pixel-alignment option failed: {regressions:?}");
    }
}

fn review_batch_font_registry() -> Vec<CompositeFont> {
    let db = FontDb::global();
    let mut fonts: Vec<_> = db.composite_font_names().iter().map(|name| (*db.composite_font(name).unwrap()).clone()).collect();
    fonts.sort_by(|a, b| a.name.cmp(&b.name));
    fonts
}

fn review_batch_font_destination(font: &CompositeFont) -> (Session, NodeId, Node) {
    let mut old = font.clone();
    old.parts.roman.size = 100.0;
    let mut session = Session::new();
    session.add_document(Document::new(100.0, 80.0), None);
    session.execute("type.compositeFonts.set", &json!({"fonts": [old]})).unwrap();
    let result = session.execute("text.create", &json!({"x": 2, "y": 12, "text": "Destination", "font": font.name})).unwrap();
    let id = NodeId(result["id"].as_u64().unwrap());
    let node = document(&session).node(id).unwrap().clone();
    (session, id, node)
}

fn review_batch_font_geometry_contrast(source: &Document, font: &CompositeFont) {
    let mut text = None;
    source.layers[0].walk(&mut |node| {
        if let NodeKind::Text(value) = &node.kind {
            assert!(text.is_none(), "CONTROL: one source lettering object");
            text = Some((**value).clone());
        }
    });
    let text = text.unwrap();
    let db = FontDb::global();
    let intended = {
        let _loan = db.lend_document_composite_fonts(std::slice::from_ref(font));
        vectorcraft_text::layout(db, &text)
    };
    let fallback = {
        let _loan = db.lend_document_composite_fonts(&[]);
        vectorcraft_text::layout(db, &text)
    };
    assert!(!intended.glyphs.is_empty() && !fallback.glyphs.is_empty(), "CONTROL: both layouts produce real glyphs");
    assert!(
        (intended.bounds.width() - fallback.bounds.width()).abs() > 0.1,
        "CONTROL: the valid 125% composite has observably different geometry from a missing-family fallback"
    );
}

fn review_batch_font_steps(path: &Path) -> Vec<Value> {
    vec![
        json!({"command": "file.place", "params": {"path": path, "link": false, "at": [50, 40]}}),
        json!({"command": "type.createOutlines", "params": {}}),
    ]
}

fn review_batch_font_rollback(session: &mut Session, path: &Path, font: &CompositeFont, alias: &str) {
    let before = document(session).clone();
    let selection = session.doc().unwrap().selection.clone();
    let history = session.doc().unwrap().history.clone();
    let registry = review_batch_font_registry();
    let mut bad = font.clone();
    bad.parts.roman.size = 9.0;
    let expected = bad.check().expect_err("CONTROL: the final batch step has a real font validation error");
    let mut steps = review_batch_font_steps(path);
    steps.push(json!({"command": "type.compositeFonts.set", "params": {"fonts": [bad]}}));
    let error = session.execute("command.batch", &json!({"commands": steps})).expect_err("CONTROL: the last step rejects the batch");
    assert!(error.to_string().contains(&expected), "CONTROL: intended validation failure: {error}");
    assert_eq!(document(session), &before, "CONTROL: failed batch restores the whole document");
    let state = session.doc().unwrap();
    assert_eq!(state.selection, selection, "CONTROL: failed batch restores selection");
    assert_eq!((state.history.undo.len(), state.history.redo.len()), (history.undo.len(), history.redo.len()));
    for (actual, expected) in state.history.undo.iter().chain(&state.history.redo).zip(history.undo.iter().chain(&history.redo)) {
        assert_eq!(actual.label, expected.label);
        assert_eq!(actual.doc, expected.doc, "CONTROL: original history document meanings remain");
        assert_eq!(actual.selection, expected.selection);
        assert_eq!(actual.bytes, expected.bytes);
    }
    assert_eq!(review_batch_font_registry(), registry, "CONTROL: failed batch restores all registered font meanings");
    assert!(FontDb::global().composite_font(alias).is_none(), "CONTROL: rollback leaves no unused imported alias");
}

fn review_batch_font_paths(session: &Session, outlined: &Value) -> Vec<vectorcraft_geom::PathData> {
    let mut paths = vec![];
    let ids = outlined["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1, "CONTROL: only the placed source lettering was outlined");
    let node = document(session).node(NodeId(ids[0].as_u64().unwrap())).unwrap();
    node.walk(&mut |node| {
        if let NodeKind::Path { path, .. } = &node.kind {
            paths.push(path.clone());
        }
    });
    assert!(!paths.is_empty(), "CONTROL: outlining produced actual path geometry");
    paths
}

fn review_batch_font_keeps_destination(session: &Session, id: NodeId, node: &Node, font: &CompositeFont) {
    let mut old = font.clone();
    old.parts.roman.size = 100.0;
    assert_eq!(document(session).node(id), Some(node), "CONTROL: existing destination lettering is exact");
    assert_eq!(CompositeFont::named(&document(session).composite_fonts, &font.name), Some(&old));
}

#[test]
fn review_597299_batch_adoption_registers_fonts_before_outlining() {
    let _context = FixtureContext::hold();
    let directory = TestDirectory::new();
    let path = directory.0.join("batch-lettering.vectorcraft");
    let mut incoming = CompositeFont::new("ReviewBatchStepLettering", "Source Sans 3", "Regular");
    incoming.parts.roman.size = 125.0;
    incoming.check().expect("CONTROL: the source definition is valid");
    let source = write_native(&path, &font_document(&incoming));
    review_batch_font_geometry_contrast(&source, &incoming);
    let (mut batch, old_id, old_node) = review_batch_font_destination(&incoming);
    let alias = format!("{} 2", incoming.name);
    let db = FontDb::global();
    assert!(!db.has_installed_family(&alias) && db.composite_font(&alias).is_none(), "CONTROL: the first collision alias is cold");
    review_batch_font_rollback(&mut batch, &path, &incoming, &alias);
    let cold_registry = review_batch_font_registry();
    let result = batch.execute("command.batch", &json!({"commands": review_batch_font_steps(&path)})).unwrap();
    assert_eq!(result["results"][0]["linked"], false, "CONTROL: the real native placement is editable");
    let actual = review_batch_font_paths(&batch, &result["results"][1]);
    review_batch_font_keeps_destination(&batch, old_id, &old_node, &incoming);
    let imported = CompositeFont::named(&document(&batch).composite_fonts, &alias).expect("CONTROL: the conflict creates the fresh alias");
    assert_eq!(imported.parts, incoming.parts);
    assert_eq!(imported.custom, incoming.custom);
    assert!(!cold_registry.iter().any(|font| CompositeFont::same_name(&font.name, &imported.name)));
    assert_eq!(*db.composite_font(&alias).unwrap(), *imported, "CONTROL: the outer command eventually registers the imported meaning");
    // Only now run the ordinary control: it cannot prewarm the batch's alias.
    let (mut ordinary, old_id, old_node) = review_batch_font_destination(&incoming);
    let placed = ordinary.execute("file.place", &json!({"path": path, "link": false, "at": [50, 40]})).unwrap();
    assert_eq!(placed_family(&ordinary, &placed), alias);
    assert_eq!(*db.composite_font(&alias).unwrap(), *imported, "CONTROL: the ordinary placement registers the correct alias before outlining");
    let outlined = ordinary.execute("type.createOutlines", &json!({})).unwrap();
    let expected = review_batch_font_paths(&ordinary, &outlined);
    review_batch_font_keeps_destination(&ordinary, old_id, &old_node, &incoming);
    assert_eq!(actual, expected, "REGRESSION: the batch must outline in the adopted composite, before its top-level sync");
}

#[test]
fn review_597299_cancelled_previews_cannot_choose_application_swatch_defaults() {
    let _context = FixtureContext::hold();
    // Ordinary application choices remain supported even with an interaction open.
    // These are immediate commands, not preview calls or scripts.
    let mut control = Session::new();
    control.add_document(Document::new(80.0, 60.0), None);
    control.execute("swatch.new", &json!({"name": "Ordinary saved choice", "color": "#2468ac"})).unwrap();
    let ordinary_palette = document(&control).swatches.clone();
    let ordinary_groups = document(&control).swatch_groups.clone();
    control.begin_interaction("Ordinary Save control").unwrap();
    control.execute("swatch.defaults.save", &json!({})).expect("CONTROL: immediate Save is allowed during an interaction");
    control.cancel_interaction().unwrap();
    control.execute("file.new", &json!({"profile": null})).unwrap();
    assert_eq!(document(&control).swatches, ordinary_palette, "CONTROL: immediate Save selects the actual palette");
    assert_eq!(document(&control).swatch_groups, ordinary_groups);
    control.begin_interaction("Ordinary Reset control").unwrap();
    control.execute("swatch.defaults.reset", &json!({})).expect("CONTROL: immediate Reset is allowed during an interaction");
    control.cancel_interaction().unwrap();
    control.execute("file.new", &json!({"profile": null})).unwrap();
    assert!(document(&control).swatch("Ordinary saved choice").is_none(), "CONTROL: immediate Reset restores factory defaults");

    for batch in [false, true] {
        for command in ["swatch.defaults.save", "swatch.defaults.reset"] {
            let mut session = Session::new();
            session.add_document(Document::new(80.0, 60.0), None);
            session.execute("swatch.new", &json!({"name": "Previously chosen palette", "color": "#2468ac"})).unwrap();
            session.execute("swatch.defaults.save", &json!({})).unwrap();
            let chosen_palette = document(&session).swatches.clone();
            let chosen_groups = document(&session).swatch_groups.clone();
            let chosen_prefs = session.prefs.to_json();
            // The current document now differs from the earlier explicit application choice.
            session.execute("swatch.new", &json!({"name": "Tentative current palette", "color": "#ac6824"})).unwrap();
            let art = session.execute("shape.rectangle", &json!({"x": 2, "y": 3, "width": 12, "height": 9})).unwrap();
            session.execute("select.set", &json!({"ids": [art["id"]]})).unwrap();
            let original = session.doc().unwrap().doc.clone();
            let selection = session.doc().unwrap().selection.clone();
            let undo = session.doc().unwrap().history.undo.len();
            let redo = session.doc().unwrap().history.redo.len();
            assert!(undo > 0 && !selection.objects.is_empty(), "CONTROL: cancellation preserves real selected art and existing history");
            assert_ne!(document(&session).swatches, chosen_palette, "CONTROL: Save would select a genuinely different palette");
            assert_eq!(session.prefs.to_json(), chosen_prefs, "CONTROL: ordinary document edits do not select application defaults");
            let (id, params) =
                if batch { ("command.batch", json!({"commands": [{"command": command, "params": {}}]})) } else { (command, json!({})) };
            session.begin_interaction("Cancellable application-default preview").unwrap();
            let preview = session.preview(id, &params);
            session.cancel_interaction().unwrap();
            // Check cancellation's real state FIRST, before checking the new refusal policy.
            // Before must expose the leaked preference, not just an expected-error assertion.
            assert_eq!(session.doc().unwrap().doc, original, "{command}, batch={batch}: cancellation preserves the document");
            assert_eq!(session.doc().unwrap().selection, selection);
            assert_eq!(session.doc().unwrap().history.undo.len(), undo);
            assert_eq!(session.doc().unwrap().history.redo.len(), redo);
            assert!(!session.in_interaction());
            assert_eq!(
                session.prefs.to_json(),
                chosen_prefs,
                "REGRESSION: {command}, batch={batch}: a cancelled preview must retain the earlier application palette"
            );
            session.execute("file.new", &json!({"profile": null})).unwrap();
            assert_eq!(
                document(&session).swatches,
                chosen_palette,
                "REGRESSION: {command}, batch={batch}: the next document retains the earlier choice"
            );
            assert_eq!(document(&session).swatch_groups, chosen_groups);
            assert!(document(&session).swatch("Tentative current palette").is_none());
            assert!(preview.is_err(), "{command}, batch={batch}: application preference changes do not support preview");
            assert!(preview.unwrap_err().to_string().contains("cannot be previewed"));
        }
    }
}
