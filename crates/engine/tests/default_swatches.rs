//! Saved application swatches replace the palette of regular new documents.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_brush::{ArtBrush, Brush, BrushKind, Colorization};
use vectorcraft_color::cms;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint, Swatch, SwatchGroup};
use vectorcraft_doc::{Appearance, Document, GraphicStyle, ImageBlob, ImageObject, LinkInfo, Node, NodeKind, PatternDef, Symbol, TextStyleDef};
use vectorcraft_engine::{Prefs, Session};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

struct ColorContext {
    working: cms::ColorSettings,
    active: cms::ColorSettings,
    _lock: MutexGuard<'static, ()>,
}

fn color_context() -> ColorContext {
    static LOCK: Mutex<()> = Mutex::new(());
    let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let context = ColorContext { working: cms::working_settings(), active: cms::active_settings(), _lock: lock };
    cms::set_active(&cms::ColorSettings::default()).unwrap();
    context
}

impl Drop for ColorContext {
    fn drop(&mut self) {
        cms::set_active(&self.working).unwrap();
        cms::use_document_profiles(Some(&self.active.rgb), Some(&self.active.cmyk));
    }
}

fn raw_palette() -> Document {
    let mut d = Document::new(80.0, 60.0);
    let brand = Color::lab(55.0, 38.0, -12.0);
    let tint = Paint::Solid { color: brand.tinted(0.4), swatch: Some("Brand".into()), tint: 0.4 };
    let mut stop = GradientStop::new(0.0, brand.tinted(0.3));
    stop.swatch = Some("Brand".into());
    stop.tint = 0.3;
    let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![stop, GradientStop::new(1.0, Color::rgb8(12, 34, 56))]));
    d.swatches = vec![
        Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false },
        Swatch { name: "RGB original".into(), paint: Paint::solid(Color::rgb(0.2, 0.4, 0.6)), global: false, spot: false },
        Swatch { name: "Brand ramp".into(), paint: Paint::Gradient(Box::new(gradient)), global: false, spot: false },
    ]
    .into();
    d.swatch_groups = vec![
        SwatchGroup {
            name: "Brand group".into(),
            swatches: vec![Swatch { name: "Brand".into(), paint: Paint::solid(brand), global: true, spot: true }],
        },
        SwatchGroup {
            name: "Print group".into(),
            swatches: vec![
                Swatch { name: "Brand 40%".into(), paint: tint, global: false, spot: false },
                Swatch { name: "CMYK original".into(), paint: Paint::solid(Color::cmyk(0.1, 0.2, 0.3, 0.4)), global: false, spot: false },
            ],
        },
        SwatchGroup { name: "Empty group".into(), swatches: vec![] },
    ]
    .into();
    d
}

fn restarted(s: &Session) -> Session {
    let saved = serde_json::to_vec(&s.prefs).unwrap();
    let prefs: Prefs = serde_json::from_slice(&saved).unwrap();
    let mut fresh = Session::new();
    fresh.apply_prefs(prefs);
    fresh
}

/// The baseline may fail at the missing command: this is a new application action.
#[test]
fn default_swatches_save_persists_raw_grouped_palette() {
    let _cms = color_context();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    let original = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, original, "saving defaults does not edit the source document");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let mut fresh = restarted(&s);
    fresh.execute("file.new", &json!({"width": 120, "height": 90, "colorMode": "cmyk", "profile": null})).unwrap();
    let new = &fresh.doc().unwrap().doc;
    assert_eq!(new.swatches, original.swatches, "raw values/models survive a different document color mode");
    assert_eq!(new.swatch_groups, original.swatch_groups, "group names, order and the empty group survive restart");
    assert!(new.swatch("[Registration]").is_some());
    assert!(new.swatch("White").is_none(), "the saved palette replaces bundled entries");
    assert_eq!(s.doc().unwrap().doc, original, "creating another session's document leaves the original alone");
}

/// Only this test's unique, freshly created directory may be removed on Drop.
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(&base).unwrap();
        let dir = base.join(format!("vc-swatch-defaults-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).canonicalize().unwrap();
        if let Ok(target) = self.0.canonicalize() {
            assert_eq!(target.parent(), Some(base.as_path()), "cleanup remains in the test temp directory");
            assert!(target.file_name().unwrap().to_string_lossy().starts_with("vc-swatch-defaults-"));
            let _ = std::fs::remove_dir_all(target);
        }
    }
}

fn png(pixel: [u8; 4]) -> Vec<u8> {
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba(pixel));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn rectangle(d: &mut Document, fill: Paint) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 3.0)), Appearance::basic(fill, Paint::None, 0.0))
}

fn image_node(d: &mut Document, link: Option<LinkInfo>) -> Node {
    Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key: "pixels".into(),
            width: 2,
            height: 2,
            xf: Affine::scale(2.0),
            link,
            placement: Default::default(),
            pixels: None,
        })),
    )
}

fn style(name: &str, id: u32, appearance: Appearance) -> GraphicStyle {
    serde_json::from_value(json!({"name": name, "id": id, "appearance": appearance})).unwrap()
}

/// A pattern reaches the image ONLY through brush art, outside visit_stored's own roots.
fn resource_palette(missing_link: &Path) -> (Document, Vec<u8>) {
    let mut d = raw_palette();
    d.color_profiles.rgb = Some(cms::WIDE_GAMUT_RGB.into());
    d.color_profiles.cmyk = Some(cms::GENERIC_CMYK.into());
    let bytes = png([17, 85, 153, 255]);
    d.images.insert("pixels".into(), ImageBlob { mime: "image/png".into(), bytes: Arc::new(bytes.clone()), proxy: None });
    let image = image_node(&mut d, Some(LinkInfo::new(missing_link.to_string_lossy().to_string())));
    vectorcraft_brush::store(
        &mut d,
        &[Brush { name: "Ink".into(), kind: BrushKind::Art(ArtBrush { art: image, colorization: Colorization::None, ..Default::default() }) }],
    );
    let linked = d.tint_paint("Brand", 0.6).unwrap();
    let appearance = Appearance::basic(Paint::None, linked, 1.0);
    d.graphic_styles.push(style("Line style", 7, appearance.clone()));
    let mut line = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 4.0)), appearance);
    line.appearance.stroke_mut().unwrap().brush = Some("Ink".into());
    line.extras_mut().graphic_style = Some(7);
    let badge = rectangle(&mut d, Paint::solid(Color::rgb8(29, 71, 113)));
    d.symbols.push(Symbol { name: "Badge".into(), art: Arc::new(badge), dynamic: false });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Badge".into(), xf: Affine::translate((2.0, 1.0)) });
    let group = Node::group(d.alloc_id(), vec![Arc::new(line), Arc::new(instance)]);
    d.patterns.push(PatternDef::new("Dots", vec![Arc::new(group)]));
    d.swatches.push(Swatch {
        name: "Dots".into(),
        paint: Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY },
        global: false,
        spot: false,
    });
    let canvas = rectangle(&mut d, Paint::solid(Color::rgb8(211, 19, 93)));
    Arc::make_mut(&mut d.layers[0]).children_mut().unwrap().push(Arc::new(canvas));
    d.fix_next_id();
    (d, bytes)
}

fn linked(color: Color, name: &str, tint: f32) -> Paint {
    Paint::Solid { color: color.tinted(tint), swatch: Some(name.into()), tint }
}

/// The category has resource-name/id conflicts plus palette-dependent defaults.
fn customized_profile() -> Document {
    let mut d = Document::new(50.0, 50.0);
    let blue = Color::cmyk(0.8, 0.2, 0.0, 0.1);
    let old = Color::rgb(0.1, 0.7, 0.2);
    d.swatches = vec![
        Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false },
        Swatch { name: "Brand".into(), paint: Paint::solid(blue), global: true, spot: true },
        Swatch { name: "Old only".into(), paint: Paint::solid(old), global: true, spot: false },
    ]
    .into();
    d.swatch_groups = vec![SwatchGroup { name: "Profile group".into(), swatches: vec![] }].into();
    d.spot_use_lab = false;
    d.color_profiles.cmyk = Some(cms::DEVICE_CMYK.into());
    d.ruler_origin = Point::new(5.0, 7.0);
    let matching = rectangle(&mut d, linked(blue, "Brand", 0.4));
    d.symbols.push(Symbol { name: "Profile matching".into(), art: Arc::new(matching), dynamic: false });
    let removed = rectangle(&mut d, linked(old, "Old only", 0.7));
    d.symbols.push(Symbol { name: "Profile removed".into(), art: Arc::new(removed), dynamic: false });
    let badge = rectangle(&mut d, Paint::solid(Color::rgb8(255, 0, 0)));
    d.symbols.push(Symbol { name: "Badge".into(), art: Arc::new(badge), dynamic: false });
    d.images.insert("pixels".into(), ImageBlob { mime: "image/png".into(), bytes: Arc::new(png([255, 0, 0, 255])), proxy: None });
    let image = image_node(&mut d, None);
    d.patterns.push(PatternDef::new("Dots", vec![Arc::new(image)]));
    let brush_art = rectangle(&mut d, Paint::solid(Color::rgb8(255, 0, 0)));
    let hue_art = rectangle(&mut d, linked(blue, "Brand", 1.0));
    // This ordinary RGB key matches the CMYK art in the profile's own assigned space.
    // Keeping the old active Generic CMYK during the comparison would miss that match.
    let target_settings = cms::ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..Default::default() };
    let target_cms = cms::Cms::new(&target_settings).unwrap();
    let hue_key = target_cms.convert(&blue, cms::Model::Rgb, target_settings.intent);
    let old_settings = cms::ColorSettings::default();
    let old_cms = cms::Cms::new(&old_settings).unwrap();
    assert_ne!(
        old_cms.convert(&blue, cms::Model::Rgb, old_settings.intent),
        hue_key,
        "the two builtin CMYK contexts make a consequential difference"
    );
    vectorcraft_brush::store(
        &mut d,
        &[
            Brush { name: "Ink".into(), kind: BrushKind::Art(ArtBrush { art: brush_art, ..Default::default() }) },
            Brush {
                name: "Profile hue".into(),
                kind: BrushKind::Art(ArtBrush { art: hue_art, colorization: Colorization::HueShift { key: hue_key }, ..Default::default() }),
            },
        ],
    );
    d.graphic_styles.push(style("Line style", 7, Appearance::basic(Paint::solid(blue), Paint::None, 0.0)));
    d.para_styles.push(TextStyleDef {
        name: "Profile paragraph".into(),
        attrs: json!({"fill": linked(blue, "Brand", 0.4), "stroke": linked(old, "Old only", 0.7)}).as_object().unwrap().clone(),
    });
    d.fix_next_id();
    d
}

fn save_profile(dir: &Path, d: &Document) {
    let bytes = vectorcraft_format::save_with(d, &vectorcraft_format::SaveOptions { include_linked: true, ..Default::default() }).unwrap();
    std::fs::write(dir.join("Print.vectorcraft"), bytes).unwrap();
}

fn assert_palette(actual: &Document, saved: &Document) {
    let mut swatches = (*saved.swatches).clone();
    for w in &mut swatches {
        if let Paint::Pattern { pattern, .. } = &mut w.paint {
            let Paint::Pattern { pattern: target, .. } = &actual.swatch(&w.name).unwrap().paint else { panic!("pattern swatch lost") };
            assert!(actual.pattern(target).is_some(), "the remapped pattern must resolve");
            *pattern = target.clone();
        }
    }
    assert_eq!(&*actual.swatches, &swatches, "only a pattern's internal resource name may change");
    assert_eq!(actual.swatch_groups, saved.swatch_groups, "groups, empty groups and raw linked colors remain exact");
    assert!(actual.swatch("[Registration]").is_some());
}

/// Evaluate the target's assigned/working CMS explicitly, independently of global_color and
/// whichever document happened to be active while file.new assembled this document.
fn expected_brand(doc: &Document) -> Color {
    let raw = Color::lab(55.0, 38.0, -12.0);
    if doc.spot_use_lab {
        return raw;
    }
    let mut settings = cms::working_settings();
    if let Some(rgb) = &doc.color_profiles.rgb {
        settings.rgb = rgb.clone();
    }
    if let Some(cmyk) = &doc.color_profiles.cmyk {
        settings.cmyk = cmyk.clone();
    }
    cms::Cms::new(&settings).unwrap().convert(&raw, cms::Model::Cmyk, settings.intent)
}

fn assert_matching(paint: &Paint, doc: &Document) {
    let Paint::Solid { color, swatch, tint } = paint else { panic!("expected linked solid paint") };
    assert_eq!(swatch.as_deref(), Some("Brand"));
    assert_eq!(*tint, 0.4);
    assert_eq!(*color, expected_brand(doc).tinted(0.4), "matching profile references follow the replacement palette through its own CMS");
}

fn assert_removed(paint: &Paint) {
    let Paint::Solid { color, swatch, .. } = paint else { panic!("expected an unlinked solid paint") };
    assert!(swatch.is_none(), "a removed name cannot remain dangling");
    assert_eq!(*color, Color::rgb(0.1, 0.7, 0.2).tinted(0.7), "unlink retains its cached color");
}

fn assert_resources(doc: &Document, bytes: &[u8], profile: &Document) {
    let Paint::Pattern { pattern, .. } = &doc.swatch("Dots").unwrap().paint else { panic!("saved pattern absent") };
    assert_ne!(pattern, "Dots", "a different existing profile pattern is not overwritten");
    let retained = doc.pattern("Dots").unwrap();
    let mut expected = profile.pattern("Dots").unwrap().clone();
    // The profile fixture has one image root. Stored-ID collision repair can allocate it a
    // fresh ID; compare every other field while separately checking all stored IDs are unique.
    assert_eq!(retained.art.len(), 1);
    assert!(matches!(retained.art[0].kind, NodeKind::Image(_)));
    Arc::make_mut(&mut expected.art[0]).id = retained.art[0].id;
    assert_eq!(retained, &expected, "the retained profile pattern keeps its full contents");
    assert_eq!(doc.images.get("pixels"), profile.images.get("pixels"));
    let mut brush_names = vec![];
    let mut symbol_names = vec![];
    let mut styles = vec![];
    for root in &doc.pattern(pattern).unwrap().art {
        root.walk(&mut |node| {
            if let Some(name) = node.appearance.stroke().and_then(|s| s.brush.as_ref()) {
                brush_names.push(name.clone());
            }
            if let NodeKind::SymbolInstance { symbol, .. } = &node.kind {
                symbol_names.push(symbol.clone());
            }
            if let Some(id) = node.extras().graphic_style {
                styles.push(id);
            }
        });
    }
    assert!(
        !brush_names.is_empty() && !symbol_names.is_empty() && !styles.is_empty(),
        "the dependency graph is present, not a flattened placeholder"
    );
    let library = vectorcraft_brush::library(doc);
    for name in brush_names {
        assert_ne!(name, "Ink");
        let brush = library.iter().find(|b| b.name == name).unwrap();
        let mut found = 0;
        for root in brush.art() {
            root.walk(&mut |n| {
                if let NodeKind::Image(im) = &n.kind {
                    let blob = doc.images.get(&im.key).unwrap();
                    assert_eq!(blob.mime, "image/png");
                    assert_eq!(blob.bytes.as_slice(), bytes, "native preference storage retains full image bytes");
                    assert!(im.link.is_none(), "the saved palette is self-contained");
                    found += 1;
                }
            });
        }
        assert_eq!(found, 1);
    }
    for name in symbol_names {
        assert_ne!(name, "Badge");
        assert!(doc.symbols.iter().any(|s| s.name == name));
    }
    for id in styles {
        assert_ne!(id, 7);
        assert!(doc.graphic_styles.iter().any(|s| s.id == id));
    }
    let mut ids = HashSet::new();
    doc.visit_stored(|node, _| assert!(ids.insert(node.id), "stored node IDs remain unique"));
}

#[test]
fn default_swatches_full_resources_replace_customized_category_after_restart() {
    let _cms = color_context();
    let dir = TestDirectory::new();
    let profile = customized_profile();
    save_profile(&dir.0, &profile);
    let (palette, bytes) = resource_palette(&dir.0.join("previously-loaded-image-now-missing.png"));
    let mut source = Session::new();
    source.add_document(palette, None);
    let before = source.doc().unwrap().doc.clone();
    let history = source.doc().unwrap().history.undo.len();
    let selection = source.doc().unwrap().selection.clone();
    let revision = source.doc().unwrap().revision;
    let dirty = source.doc().unwrap().is_dirty();
    let source_cms = cms::active_settings();
    let working = cms::working_settings();
    source.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(source.doc().unwrap().doc, before);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().revision, revision);
    assert_eq!(source.doc().unwrap().is_dirty(), dirty);
    assert_eq!(cms::active_settings(), source_cms, "capture restores the source's assigned context");
    assert_eq!(cms::working_settings(), working, "capture does not replace working Color Settings");
    let mut target = restarted(&source);
    target.attach_new_doc_profiles(dir.0.clone());
    target.add_document(raw_palette(), None);
    let existing = target.doc().unwrap().doc.clone();
    assert_ne!(
        cms::active_settings().cmyk,
        profile.color_profiles.cmyk.as_deref().unwrap(),
        "the old active document differs from the future target CMS"
    );
    let result = target.execute("file.new", &json!({"profile": "Print", "colorMode": "rgb", "width": 120, "height": 90})).unwrap();
    assert_eq!(result["profile"], "Print", "the customized category actually participates");
    let d = &target.doc().unwrap().doc;
    assert_palette(d, &before);
    assert_resources(d, &bytes, &profile);
    assert_eq!(d.ruler_origin, profile.ruler_origin);
    assert_eq!(d.spot_use_lab, profile.spot_use_lab);
    assert_eq!(d.color_profiles, profile.color_profiles, "assigned profile settings survive palette replacement");
    assert_eq!(cms::active_settings().cmyk, cms::DEVICE_CMYK, "the completed new document is frontmost");
    assert_eq!(cms::working_settings(), working, "target conversion must not edit working Color Settings");
    assert_eq!((d.artboards[0].rect.width(), d.artboards[0].rect.height()), (120.0, 90.0));
    assert!(d.layers.iter().all(|n| n.children().is_none_or(Vec::is_empty)), "carrier/source canvas art is not installed");
    let matching = d.symbols.iter().find(|s| s.name == "Profile matching").unwrap();
    assert_matching(&matching.art.appearance.fill().unwrap().paint, d);
    let removed = d.symbols.iter().find(|s| s.name == "Profile removed").unwrap();
    assert_removed(&removed.art.appearance.fill().unwrap().paint);
    let para = d.para_styles.iter().find(|s| s.name == "Profile paragraph").unwrap();
    assert_matching(&serde_json::from_value::<Paint>(para.attrs["fill"].clone()).unwrap(), d);
    assert_removed(&serde_json::from_value::<Paint>(para.attrs["stroke"].clone()).unwrap());
    let brushes = vectorcraft_brush::library(d);
    let hue = brushes.iter().find(|b| b.name == "Profile hue").unwrap();
    let BrushKind::Art(art) = &hue.kind else { panic!("profile Art brush changed kind") };
    let Colorization::HueShift { key } = &art.colorization else { panic!("profile brush lost HueShift") };
    let Paint::Solid { color, swatch, tint } = &art.art.appearance.fill().unwrap().paint else { panic!("profile HueShift art lost its color") };
    assert_eq!(swatch.as_deref(), Some("Brand"));
    assert_eq!(*tint, 1.0);
    assert_eq!(*color, expected_brand(d), "profile brush art follows the saved global color");
    assert_eq!(*key, *color, "HueShift key moves with its recolored linked art");
    assert_eq!(target.documents()[0].doc, existing, "existing documents do not receive application defaults retroactively");
    assert_eq!(source.doc().unwrap().doc, before);
}

#[test]
fn default_swatches_factory_is_distinct_from_unset_for_customized_categories() {
    let _cms = color_context();
    let dir = TestDirectory::new();
    let profile = customized_profile();
    save_profile(&dir.0, &profile);
    let mut s = Session::new();
    s.attach_new_doc_profiles(dir.0.clone());
    s.execute("file.new", &json!({"profile": "Print", "colorMode": "rgb"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.swatches, profile.swatches, "unset keeps the legacy custom-category palette");
    assert_eq!(s.doc().unwrap().doc.swatch_groups, profile.swatch_groups);
    let existing = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(s.prefs.to_json()["defaultSwatches"]["kind"], "custom");
    s.execute("swatch.defaults.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.to_json()["defaultSwatches"]["kind"], "factory");
    assert_eq!(s.doc().unwrap().doc, existing);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let mut fresh = restarted(&s);
    fresh.attach_new_doc_profiles(dir.0.clone());
    fresh.execute("file.new", &json!({"profile": "Print", "colorMode": "rgb"})).unwrap();
    let d = &fresh.doc().unwrap().doc;
    let (swatches, groups) = vectorcraft_color::default_swatches(d.color_mode.model());
    assert_eq!(&*d.swatches, &swatches, "Factory replaces the customized category palette after restart");
    assert_eq!(&*d.swatch_groups, &groups);
    assert_eq!(d.ruler_origin, profile.ruler_origin, "Factory does not reset unrelated profile settings");
    let before = fresh.prefs.to_json();
    fresh.execute("swatch.defaults.reset", &json!({})).unwrap();
    assert_eq!(fresh.prefs.to_json(), before, "Factory reset is idempotent");
    let mut empty = Session::new();
    empty.execute("swatch.defaults.reset", &json!({})).unwrap();
    assert_eq!(empty.prefs.to_json()["defaultSwatches"]["kind"], "factory", "reset needs no open document");
}

#[test]
fn default_swatches_second_save_replaces_and_templates_keep_their_palette() {
    let _cms = color_context();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    let first = s.doc().unwrap().doc.clone();
    let mut second = Document::new(90.0, 70.0);
    second.swatches = vec![
        Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false },
        Swatch { name: "Second palette".into(), paint: Paint::solid(Color::gray(0.25)), global: false, spot: false },
    ]
    .into();
    second.swatch_groups = vec![SwatchGroup { name: "Second empty group".into(), swatches: vec![] }].into();
    s.add_document(second.clone(), None);
    let history = s.doc().unwrap().history.undo.len();
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.as_ref(), &second);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert_eq!(s.documents()[0].doc, first);
    let mut fresh = restarted(&s);
    fresh.execute("file.new", &json!({})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &second);
    assert!(fresh.doc().unwrap().doc.swatch("Brand").is_none(), "saving twice replaces, not appends");
    let bytes = vectorcraft_format::save_with(&raw_palette(), &vectorcraft_format::SaveOptions::default()).unwrap();
    let data = vectorcraft_format::base64_encode(&bytes);
    let ordinary = fresh.doc().unwrap().doc.clone();
    fresh.execute("file.newFromTemplate", &json!({"name": "original.vctemplate", "dataBase64": data.clone()})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &raw_palette());
    assert_eq!(fresh.documents()[0].doc, ordinary);
    fresh.execute("document.open", &json!({"name": "original.vectorcraft", "dataBase64": data})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &raw_palette());
}

#[test]
fn default_swatches_missing_full_brush_image_keeps_the_previous_saved_palette() {
    let _cms = color_context();
    let dir = TestDirectory::new();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    let saved = s.prefs.to_json();
    let (mut bad, _) = resource_palette(&dir.0.join("missing-required-full-image.png"));
    let blob = bad.images.get_mut("pixels").unwrap();
    blob.proxy = Some(blob.bytes.clone());
    assert!(blob.is_proxy(), "this ordinary missing link has only its saved preview");
    s.add_document(bad, None);
    let original = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let active_cms = cms::active_settings();
    let working = cms::working_settings();
    assert!(s.execute("swatch.defaults.save", &json!({})).is_err(), "cannot promise a complete saved palette from a missing full brush image");
    assert_eq!(s.prefs.to_json(), saved, "failed capture keeps the previous good application default");
    assert_eq!(s.doc().unwrap().doc, original);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert_eq!(cms::active_settings(), active_cms, "failed capture restores the source's assigned context");
    assert_eq!(cms::working_settings(), working);
    let mut fresh = restarted(&s);
    fresh.execute("file.new", &json!({})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &raw_palette());
}

#[test]
fn default_swatches_unreadable_saved_palette_adds_no_partial_document() {
    let _cms = color_context();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    // A single unusable persisted palette, not a parser/stress input campaign.
    let mut values = s.prefs.to_json();
    values["defaultSwatches"] = json!({"kind": "custom", "data": vectorcraft_format::base64_encode(b"unreadable saved palette")});
    s.apply_prefs(serde_json::from_value(values).unwrap());
    let original = s.doc().unwrap().doc.clone();
    let active = s.active_index();
    let count = s.documents().len();
    let history = s.doc().unwrap().history.undo.len();
    assert!(s.execute("file.new", &json!({"width": 123, "height": 87})).is_err());
    assert_eq!(s.documents().len(), count, "apply failure does not insert a partial document");
    assert_eq!(s.active_index(), active);
    assert_eq!(s.doc().unwrap().doc, original);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("swatch.defaults.reset", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (swatches, groups) = vectorcraft_color::default_swatches(d.color_mode.model());
    assert_eq!(&*d.swatches, &swatches, "Factory recovers without decoding the unusable Custom data");
    assert_eq!(&*d.swatch_groups, &groups);
}

/// A saved pattern keeps its document-local composite family without replacing a category's.
#[test]
fn default_swatches_pattern_composite_family_survives_restart_and_profile_collision() {
    use vectorcraft_doc::{CharStyle, CompositeFont, NodeId};
    use vectorcraft_text::FontDb;

    struct FontContext(Vec<CompositeFont>);
    impl Drop for FontContext {
        fn drop(&mut self) {
            let db = FontDb::global();
            for name in db.composite_font_names() {
                if CompositeFont::named(&self.0, &name).is_none() {
                    db.remove_composite_font(&name);
                }
            }
            for font in &self.0 {
                db.set_composite_font(font);
            }
        }
    }
    fn pattern_family(d: &Document) -> (String, String) {
        let Paint::Pattern { pattern, .. } = &d.swatch("Letter tile").unwrap().paint else { panic!("pattern swatch lost") };
        let mut runs = vec![];
        for root in &d.pattern(pattern).unwrap().art {
            root.walk(&mut |n| {
                if let NodeKind::Text(text) = &n.kind {
                    runs.extend(
                        text.runs.iter().filter(|r| r.text == "Ab").map(|r| (r.style.font_family.clone(), r.style.style_name.clone().unwrap())),
                    );
                }
            });
        }
        assert_eq!(runs.len(), 1, "the original lettering lives only in this pattern");
        runs.pop().unwrap()
    }
    fn assert_pattern_family(d: &Document, source_font: &CompositeFont, collision: bool) {
        let (family, style_name) = pattern_family(d);
        let held =
            CompositeFont::named(&d.composite_fonts, &family).expect("saved pattern must carry its document-defined composite family after restart");
        assert_eq!(held.parts, source_font.parts, "the pattern keeps the source definition, not a category's same-named definition");
        assert_eq!(CompositeFont::same_name(&held.name, &source_font.name), !collision);
        let style = d.char_styles.iter().find(|s| s.name == style_name).unwrap();
        assert_eq!(style.attrs["font_family"].as_str(), Some(family.as_str()), "the referenced character style follows the imported family");
        let mut label_families = vec![];
        d.visit_stored(|node, _| {
            if let Some(graph) = node.extras().graph.as_deref() {
                label_families.extend(graph.label_styles.values.iter().map(|s| s.font_family.clone()));
            }
        });
        assert_eq!(label_families, [family], "the graph's stored label style follows the same family for regeneration");
        assert!(CompositeFont::named(&d.composite_fonts, "PaletteUnused").is_none(), "unused source definitions are not new-document settings");
    }

    let _cms = color_context();
    let db = FontDb::global();
    let _fonts = FontContext(db.composite_font_names().iter().map(|n| (*db.composite_font(n).unwrap()).clone()).collect());
    let dir = TestDirectory::new();
    let mut palette_font = CompositeFont::new("PaletteFamily", "Source Sans 3", "Regular");
    palette_font.parts.roman.size = 110.0;
    let unused_font = CompositeFont::new("PaletteUnused", "Source Sans 3", "Regular");
    let profile_font = CompositeFont::new("palettefamily", "Source Serif 4", "Regular");

    let mut profile_session = Session::new();
    profile_session.add_document(Document::new(80.0, 60.0), None);
    profile_session.execute("type.compositeFonts.set", &json!({"fonts": [profile_font.clone()]})).unwrap();
    let id = profile_session.execute("text.create", &json!({"x": 5, "y": 15, "text": "Profile", "font": profile_font.name})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let mut profile = (*profile_session.doc().unwrap().doc).clone();
    let lettering = profile.node(NodeId(id)).unwrap().clone();
    Arc::make_mut(&mut profile.layers[0]).children_mut().unwrap().clear();
    profile.symbols.push(Symbol { name: "Profile lettering".into(), art: Arc::new(lettering), dynamic: false });
    profile.fix_next_id();
    save_profile(&dir.0, &profile);

    let mut source = Session::new();
    source.add_document(raw_palette(), None);
    source.execute("type.compositeFonts.set", &json!({"fonts": [palette_font.clone(), unused_font]})).unwrap();
    let text = source.execute("text.create", &json!({"x": 5, "y": 15, "text": "Ab", "font": palette_font.name})).unwrap()["id"].as_u64().unwrap();
    source.execute("charStyle.new", &json!({"name": "Palette lettering", "attrs": {"font_family": palette_font.name}})).unwrap();
    source.execute("charStyle.apply", &json!({"name": "Palette lettering", "id": text})).unwrap();
    let graph = source
        .execute(
            "graph.create",
            &json!({"type": "column", "x": 0, "y": 0, "width": 30, "height": 20, "series": ["s"], "categories": ["a"], "rows": [[1]]}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    source
        .edit("Format the pattern graph's saved labels", |d, _| {
            let node = d.node_mut(NodeId(graph)).unwrap();
            let mut extras = node.extras_mut();
            extras.graph.as_deref_mut().unwrap().label_styles.values =
                Some(CharStyle { font_family: palette_font.name.clone(), ..Default::default() });
            Ok(())
        })
        .unwrap();
    source.execute("graph.setData", &json!({"id": graph, "rows": [[2]]})).unwrap();
    source.execute("object.pattern.make", &json!({"ids": [text, graph], "name": "Letter tile", "edit": false})).unwrap();
    source.execute("select.set", &json!({"ids": [text, graph]})).unwrap();
    source.execute("edit.clear", &json!({})).unwrap();
    let original = source.doc().unwrap().doc.clone();
    let selection = source.doc().unwrap().selection.clone();
    let history = source.doc().unwrap().history.undo.len();
    let names = db.composite_font_names();
    let held = (*db.composite_font(&palette_font.name).unwrap()).clone();
    source.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(db.composite_font_names(), names, "private capture leaves no temporary registered family");
    assert_eq!(*db.composite_font(&palette_font.name).unwrap(), held, "private capture restores the source family");
    assert_eq!(source.doc().unwrap().doc, original);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);

    let mut plain = restarted(&source);
    plain.execute("file.new", &json!({"profile": null})).unwrap();
    assert_pattern_family(&plain.doc().unwrap().doc, &palette_font, false);
    assert_palette(&plain.doc().unwrap().doc, &original);

    let mut customized = restarted(&source);
    customized.attach_new_doc_profiles(dir.0.clone());
    customized.execute("file.new", &json!({"profile": "Print"})).unwrap();
    let doc = &customized.doc().unwrap().doc;
    assert_pattern_family(doc, &palette_font, true);
    assert_palette(doc, &original);
    assert_eq!(
        CompositeFont::named(&doc.composite_fonts, &profile_font.name),
        Some(&profile_font),
        "the customized category's definition is not replaced"
    );
    let NodeKind::Text(actual_profile_text) = &doc.symbols.iter().find(|s| s.name == "Profile lettering").unwrap().art.kind else {
        panic!("profile lettering lost")
    };
    let NodeKind::Text(expected_profile_text) = &profile.symbols[0].art.kind else { panic!("profile fixture lost its type") };
    assert_eq!(actual_profile_text.runs, expected_profile_text.runs, "unrelated profile type keeps its text, formatting and original family");
    assert_eq!(actual_profile_text.xf, expected_profile_text.xf);
    assert_eq!(source.doc().unwrap().doc, original);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);

    // The same document-local family survives the normal missing-full-brush-image error path.
    let saved = source.prefs.to_json();
    let mut bad = (*original).clone();
    let bytes = Arc::new(png([19, 47, 83, 255]));
    let preview = ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) };
    assert!(preview.is_proxy(), "the missing linked image has only its saved preview");
    bad.images.insert("pixels".into(), preview);
    let image = image_node(&mut bad, Some(LinkInfo::new(dir.0.join("missing-composite-brush-image.png").to_string_lossy().to_string())));
    vectorcraft_brush::store(&mut bad, &[Brush { name: "Missing ink".into(), kind: BrushKind::Art(ArtBrush { art: image, ..Default::default() }) }]);
    let mut line = rectangle(&mut bad, Paint::None);
    line.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0).into();
    line.appearance.stroke_mut().unwrap().brush = Some("Missing ink".into());
    bad.patterns.iter_mut().find(|p| p.name == "Letter tile").unwrap().art.push(Arc::new(line));
    let mut failing = restarted(&source);
    failing.add_document(bad, None);
    let before = failing.doc().unwrap().doc.clone();
    let before_selection = failing.doc().unwrap().selection.clone();
    let before_history = failing.doc().unwrap().history.undo.len();
    let held_names = db.composite_font_names();
    let held_font = (*db.composite_font(&palette_font.name).unwrap()).clone();
    assert!(failing.execute("swatch.defaults.save", &json!({})).is_err(), "the linked brush has only an unavailable full image's saved preview");
    assert_eq!(failing.prefs.to_json(), saved, "failure keeps the previous complete palette");
    assert_eq!(failing.doc().unwrap().doc, before);
    assert_eq!(failing.doc().unwrap().selection, before_selection);
    assert_eq!(failing.doc().unwrap().history.undo.len(), before_history);
    assert_eq!(db.composite_font_names(), held_names, "failed private capture leaves no temporary registered family");
    assert_eq!(*db.composite_font(&palette_font.name).unwrap(), held_font, "failed private capture restores the source family");
}

fn write_linked_native_fixture(path: &Path, document: &Document, include_linked: bool) -> Document {
    let bytes =
        vectorcraft_format::save_with(document, &vectorcraft_format::SaveOptions { include_linked, precision: None, ..Default::default() }).unwrap();
    std::fs::write(path, &bytes).unwrap();
    vectorcraft_format::load(&bytes).unwrap()
}

fn linked_native_palette(path: &Path, name: &str) -> Session {
    let mut source = Session::new();
    source.add_document(raw_palette(), None);
    let placed = source.execute("file.place", &json!({"path": path, "link": true})).unwrap();
    assert_eq!(placed["linked"], true, "this is the supported linked-native placement route");
    let id = placed["ids"][0].as_u64().unwrap();
    assert!(matches!(&source.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind, NodeKind::PlacedDocument(_)));
    source.execute("object.pattern.make", &json!({"ids": [id], "name": name, "edit": false})).unwrap();
    source.execute("select.set", &json!({"ids": [id]})).unwrap();
    source.execute("edit.clear", &json!({})).unwrap();
    let mut count = 0;
    source.doc().unwrap().doc.visit_stored(|node, _| {
        if let NodeKind::PlacedDocument(placed) = &node.kind {
            count += 1;
            let blob = &source.doc().unwrap().doc.images[&placed.key];
            assert!(!blob.is_proxy(), "the placed native file itself has full stored bytes");
            assert!(vectorcraft_format::load(&blob.bytes).is_ok());
        }
    });
    assert_eq!(count, 1, "the native placement is stored only in the pattern");
    source
}

fn palette_link_ids(document: &Document) -> Vec<u64> {
    let mut ids = vec![];
    document.visit_stored(|node, _| {
        if matches!(&node.kind, NodeKind::PlacedDocument(_)) || matches!(&node.kind, NodeKind::Image(image) if image.link.is_some()) {
            ids.push(node.id.0);
        }
    });
    ids
}

fn assert_native_ink_resource(document: &Document) {
    let mut names = vec![];
    document.visit_stored(|node, _| {
        for item in &node.appearance.items {
            if let vectorcraft_doc::AppearanceItem::Stroke(stroke) = item
                && let Some(brush) = &stroke.brush
            {
                names.push(brush.clone());
            }
        }
    });
    assert_eq!(names, ["Native ink"], "the expanded pattern still uses its original custom brush");
    let library = vectorcraft_brush::library(document);
    let brush = library
        .iter()
        .find(|brush| brush.name == names[0])
        .expect("a brush adopted while embedding the linked native file must remain in the saved palette");
    let BrushKind::Art(art) = &brush.kind else { panic!("the imported Art brush changed kind") };
    assert_eq!(art.art.appearance.fill().unwrap().paint, Paint::solid(Color::rgb8(31, 81, 123)));
}

/// Materialize only for inspection after deleting the originals. This accepts a valid fully
/// self-contained nested capsule too; it does not require capture to flatten its representation.
fn materialize_saved_palette_links(source: &mut Session) {
    for _ in 0..3 {
        let ids = palette_link_ids(&source.doc().unwrap().doc);
        if ids.is_empty() {
            return;
        }
        let result = source.execute("links.embed", &json!({"ids": ids})).unwrap();
        assert!(
            result["missing"].as_array().unwrap().is_empty(),
            "a saved default must supply full nested resources after the original files are removed: {result}"
        );
    }
    assert!(palette_link_ids(&source.doc().unwrap().doc).is_empty(), "the two-level fixture was completely inspected");
}

#[test]
fn default_swatches_linked_native_imported_brush_survives_capture() {
    let _cms = color_context();
    let directory = TestDirectory::new();
    let native_path = directory.0.join("brushed.vectorcraft");
    let mut native = Document::new(20.0, 20.0);
    let brush_art = rectangle(&mut native, Paint::solid(Color::rgb8(31, 81, 123)));
    vectorcraft_brush::store(
        &mut native,
        &[Brush { name: "Native ink".into(), kind: BrushKind::Art(ArtBrush { art: brush_art, ..Default::default() }) }],
    );
    let mut stroked = Node::path(
        native.alloc_id(),
        shapes::rectangle(Rect::new(2.0, 2.0, 12.0, 10.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0),
    );
    stroked.appearance.stroke_mut().unwrap().brush = Some("Native ink".into());
    Arc::make_mut(&mut native.layers[0]).children_mut().unwrap().push(Arc::new(stroked));
    native.fix_next_id();
    write_linked_native_fixture(&native_path, &native, true);
    let mut source = linked_native_palette(&native_path, "Native brush tile");
    let original = source.doc().unwrap().doc.clone();
    let selection = source.doc().unwrap().selection.clone();
    let history = source.doc().unwrap().history.undo.len();
    assert!(
        !vectorcraft_brush::library(&original).iter().any(|brush| brush.name == "Native ink"),
        "linked placement has not adopted the custom native brush yet"
    );

    // Prove the public embed/adopt seam produces the referenced definition before capture.
    let mut control = Session::new();
    control.add_document((*original).clone(), None);
    let ids = palette_link_ids(&control.doc().unwrap().doc);
    let result = control.execute("links.embed", &json!({"ids": ids})).unwrap();
    assert!(result["missing"].as_array().unwrap().is_empty());
    assert_native_ink_resource(&control.doc().unwrap().doc);

    source.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(source.doc().unwrap().doc, original);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);
    std::fs::remove_file(&native_path).unwrap();
    let mut fresh = restarted(&source);
    fresh.execute("file.new", &json!({"profile": null})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &original);
    materialize_saved_palette_links(&mut fresh);
    assert_native_ink_resource(&fresh.doc().unwrap().doc);
    assert_eq!(source.doc().unwrap().doc, original);
}

#[test]
fn default_swatches_linked_native_nested_image_is_complete_or_capture_fails() {
    let _cms = color_context();
    let directory = TestDirectory::new();
    let image_path = directory.0.join("original-banner.png");
    let inner_path = directory.0.join("inner.vectorcraft");
    let outer_path = directory.0.join("outer.vectorcraft");
    // Just over the normal 256px proxy threshold; a 2x2 image would keep full bytes.
    let pixels = image::RgbaImage::from_fn(300, 4, |x, y| image::Rgba([(x % 251) as u8, (y * 53) as u8, 137, 255]));
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
    let full_png = encoded.into_inner();
    std::fs::write(&image_path, &full_png).unwrap();

    let mut inner = Session::new();
    inner.add_document(Document::new(320.0, 20.0), None);
    inner.execute("file.place", &json!({"path": image_path, "link": true, "at": [150, 2]})).unwrap();
    let saved_inner = write_linked_native_fixture(&inner_path, &inner.doc().unwrap().doc, false);
    let mut proxies = 0;
    saved_inner.visit_stored(|node, _| {
        if let NodeKind::Image(image) = &node.kind {
            assert!(image.link.is_some());
            assert!(saved_inner.images[&image.key].is_proxy(), "the native file genuinely stores only the linked image's proxy");
            assert_eq!((image.width, image.height), (300, 4));
            proxies += 1;
        }
    });
    assert_eq!(proxies, 1);

    let mut outer = Session::new();
    outer.add_document(Document::new(320.0, 20.0), None);
    outer.execute("file.place", &json!({"path": inner_path, "link": true, "at": [160, 10]})).unwrap();
    let saved_outer = write_linked_native_fixture(&outer_path, &outer.doc().unwrap().doc, true);
    let mut nested = 0;
    saved_outer.visit_stored(|node, _| {
        if let NodeKind::PlacedDocument(placed) = &node.kind {
            let blob = &saved_outer.images[&placed.key];
            assert!(!blob.is_proxy(), "A carries full B native bytes, not just a placement preview");
            let nested_file = vectorcraft_format::load(&blob.bytes).unwrap();
            assert!(nested_file.images.values().any(ImageBlob::is_proxy), "B's full native payload still contains the saved proxy-only image");
            nested += 1;
        }
    });
    assert_eq!(nested, 1);

    let mut good = Session::new();
    good.add_document(raw_palette(), None);
    good.execute("swatch.defaults.save", &json!({})).unwrap();
    let good_prefs = good.prefs.to_json();
    let mut source = linked_native_palette(&outer_path, "Nested image tile");
    source.apply_prefs(good.prefs.clone());
    let original = source.doc().unwrap().doc.clone();
    let selection = source.doc().unwrap().selection.clone();
    let history = source.doc().unwrap().history.undo.len();

    std::fs::remove_file(&image_path).unwrap();
    assert!(
        source.execute("swatch.defaults.save", &json!({})).is_err(),
        "full A and B native blobs cannot make a missing proxy-only image a complete saved default"
    );
    assert_eq!(source.prefs.to_json(), good_prefs, "missing nested content keeps the previous complete default");
    assert_eq!(source.doc().unwrap().doc, original);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);

    std::fs::write(&image_path, &full_png).unwrap();
    source.execute("swatch.defaults.save", &json!({})).unwrap();
    assert_eq!(source.doc().unwrap().doc, original);
    assert_eq!(source.doc().unwrap().selection, selection);
    assert_eq!(source.doc().unwrap().history.undo.len(), history);
    for path in [&image_path, &inner_path, &outer_path] {
        std::fs::remove_file(path).unwrap();
    }
    let mut fresh = restarted(&source);
    fresh.execute("file.new", &json!({"profile": null})).unwrap();
    assert_palette(&fresh.doc().unwrap().doc, &original);
    materialize_saved_palette_links(&mut fresh);
    let document = &fresh.doc().unwrap().doc;
    let mut images = 0;
    document.visit_stored(|node, _| {
        if let NodeKind::Image(image) = &node.kind {
            let blob = &document.images[&image.key];
            assert!(image.link.is_none());
            assert!(!blob.is_proxy());
            assert_eq!(blob.bytes.as_ref(), &full_png, "the saved palette retains the original full PNG, not a preview");
            images += 1;
        }
    });
    assert_eq!(images, 1, "the original image remains reachable through the pattern");
    assert_eq!(source.doc().unwrap().doc, original);
}

#[test]
fn default_swatches_scripts_refuse_global_choices_directly_and_in_batches() {
    let _cms = color_context();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    s.execute("shape.rectangle", &json!({"x": 2, "y": 3, "width": 12, "height": 9})).unwrap();
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    let prefs = s.prefs.to_json();
    let original = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let redo = s.doc().unwrap().history.redo.len();
    assert!(undo > 0, "the script must preserve an existing document undo step");

    let mut source = String::new();
    let mut expected = Vec::new();
    for batch in [false, true] {
        for command in ["swatch.defaults.save", "swatch.defaults.reset"] {
            let (id, params) =
                if batch { ("command.batch", json!({"commands": [{"command": command, "params": {}}]})) } else { (command, json!({})) };
            // Catch each refusal so the actual script exercises all four routes before assertions.
            source.push_str(&format!(
                "try {{ vc.run({}, {}); vc.log('allowed'); }} catch (e) {{ vc.log(e.message); }}\n",
                serde_json::to_string(id).unwrap(),
                params
            ));
            expected.push(format!("a script cannot run `{command}`"));
        }
    }
    let report = s.execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(report["ok"], true);
    let log = report["log"].as_array().unwrap();
    assert_eq!(log.len(), expected.len());
    for (message, refusal) in log.iter().zip(expected) {
        assert!(message.as_str().unwrap().contains(&refusal), "the actual script must refuse this global choice: {report}");
    }
    assert_eq!(s.prefs.to_json(), prefs, "refused script commands keep the previous application default");
    assert_eq!(s.doc().unwrap().doc, original);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.doc().unwrap().history.redo.len(), redo);
}

#[test]
fn default_swatches_script_document_edits_and_existing_pref_refusal_remain_supported() {
    let _cms = color_context();
    let mut s = Session::new();
    s.add_document(raw_palette(), None);
    s.execute("swatch.defaults.save", &json!({})).unwrap();
    let defaults = s.prefs.to_json()["defaultSwatches"].clone();
    let original = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let source = concat!(
        "try { vc.run('prefs.reset', {}); } catch (e) { vc.log(e.message); }",
        "vc.run('shape.rectangle', {x:10,y:8,width:12,height:9});",
        "vc.run('command.batch', {commands:[{command:'shape.rectangle',params:{x:30,y:8,width:12,height:9}}]});"
    );
    let report = s.execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(report["ok"], true);
    assert!(report["log"][0].as_str().unwrap().contains("a script cannot run `prefs.reset`"));
    assert_eq!(report["changed"].as_array().unwrap().len(), 1, "ordinary direct and batched document edits still commit");
    assert_ne!(s.doc().unwrap().doc, original);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "the document script remains one undo step");
    assert_eq!(s.prefs.to_json()["defaultSwatches"], defaults);
}
