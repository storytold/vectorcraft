//! Brush libraries (Window → Brush Libraries, the Brushes panel's library button): read-only sets
//! of brushes the library panel shows. A library is a document holding the brushes with the
//! resources their art uses (patterns, symbols, swatches, graphic styles, images); saved, it is a
//! native `.vectorcraft` file, which File › Open also opens. The built-in libraries are generated
//! here ([`BUILTIN`]); User Defined ones are the `.vectorcraft` files in the user library folder
//! ([`Libraries`]), and Other Library… loads the brushes of any document VectorCraft opens.
//! `brush.addFromLibrary` copies brushes into the document; `brush.saveLibrary` writes the
//! document's.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use vectorcraft_brush::{self as brush, ArtBrush, Brush, BrushKind, Calligraphic, Colorization, PatternBrush, PatternFit, Scatter, Variation};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, PatternDef, Symbol};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, SubPath, shapes};

use super::brushsym::{apply_brush, brush_targets, roots, stroke_paint};
use super::clipboard::Clipboard;
use super::clipboard::resources::SwatchChoices;
use super::patterncmds::add_pattern;
use super::swatch::str_list;
use super::swatchlib::{FileData, LibraryFile, LibraryInfo, concealed_library_id, concealed_library_ids, shown_id, shown_library_id, stem};
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, str_param};

/// The extension of saved brush libraries: native documents.
pub const BRUSHES_EXT: &str = vectorcraft_format::EXTENSION;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "brush.libraries",
            "Brush Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"user\" (User Defined: the .vectorcraft files in the user library folder, rescanned now)|\"loaded\" (brush.loadLibrary), count, path?}], userFolder} every brush library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "brush.library",
            "Brush Library",
            [],
            None,
            "{library: id or name} → {id, name, category, brushes: [{name, type}] (as brush.list)}",
            always,
            get
        ),
        cmd!(
            "brush.addFromLibrary",
            "Add to Brushes",
            ["Window", "Brush Libraries"],
            None,
            "{library: id or name, name? | names?: [brush names in the library] (default: all of it), apply?: false (also apply the first one to paths, as brush.apply does), ids?|id? (the paths apply takes; default the selected ones), swatchConflict? (as edit.paste)} copy library brushes into the document's Brushes as one undo step (with the apply; named Apply Brush when no brush is added and it only applies one), with the resources their art uses (patterns, symbols, swatches, graphic styles, images). A brush the document already has (same name and definition) isn't added again; a name another brush has gets a number (\"Ribbon 2\"). The first one becomes the current brush → {library, names: [each brush's name in the document], added: [names added], existing: [names already there], applied: [ids of the paths it was applied to]}",
            has_doc,
            add
        ),
        cmd!(
            "brush.saveLibrary",
            "Save Brush Library…",
            ["Window", "Brushes"],
            None,
            "{path?, names?: [brush names] (default: all), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's brushes, with the resources their art uses, as a brush library: a .vectorcraft document holding only them → {path, count, library?: id when saved to the user folder}; without path or user → {dataBase64: the file's bytes, count}",
            has_doc,
            save
        ),
        cmd!(
            "brush.loadLibrary",
            "Other Library…",
            ["Window", "Brush Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load the brushes of any document VectorCraft opens (see document.formats; one without brushes of its own has the default ones), a saved brush library included, for the library panel (Window → Brush Libraries lists it until the app quits; a file of the user library folder, or a file with the same bytes as one there, opens as that User Defined library) → {library: id, name, count}",
            always,
            load
        ),
    ]
}

/// A brush library: its name, its brushes (in panel order) and a document holding them with the
/// resources their art uses; the document has no art of its own.
#[derive(Clone, Debug)]
pub struct BrushLibrary {
    pub name: String,
    pub brushes: Vec<Brush>,
    pub doc: Document,
}

impl Default for BrushLibrary {
    fn default() -> Self {
        Self { name: String::new(), brushes: vec![], doc: Document::new(100.0, 100.0) }
    }
}

impl BrushLibrary {
    pub fn len(&self) -> usize {
        self.brushes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.brushes.is_empty()
    }
    pub fn brush(&self, name: &str) -> Option<&Brush> {
        self.brushes.iter().find(|b| b.name == name)
    }

    /// Document `src`'s brushes `names` (empty: all of them, in panel order) as a library named
    /// `name`: the brushes and every resource their art uses.
    pub fn from_document(src: &Document, names: &[String], name: String) -> std::result::Result<Self, String> {
        let all = brush::library(src);
        // Names looked up in sets: scanning the lists for each brush made a library of N brushes
        // cost N² comparisons.
        let have: HashSet<&str> = all.iter().map(|b| b.name.as_str()).collect();
        if let Some(n) = names.iter().find(|n| !have.contains(n.as_str())) {
            return Err(format!("no brush `{n}`"));
        }
        let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
        let picked = all.iter().filter(|b| names.is_empty() || wanted.contains(b.name.as_str()));
        let clip = Clipboard::gather(src, picked.map(|b| stroke(&b.name)).collect());
        let mut doc = clip.to_document();
        for l in &mut doc.layers {
            if let Some(ch) = Arc::make_mut(l).children_mut() {
                ch.clear();
            }
        }
        // The brushes picked and those their art paints with, in the panel order of the document
        // they came from.
        let kept: HashSet<&str> = clip.brushes.iter().map(|c| c.name.as_str()).collect();
        let brushes: Vec<Brush> = all.iter().filter(|b| kept.contains(b.name.as_str())).cloned().collect();
        brush::store(&mut doc, &brushes);
        doc.title = name.clone();
        Ok(Self { name, brushes, doc })
    }
}

impl LibraryFile for BrushLibrary {
    const EXTS: &'static [&'static str] = &[BRUSHES_EXT];
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        let doc = vectorcraft_format::load(bytes).map_err(|e| e.to_string())?;
        BrushLibrary::from_document(&doc, &[], stem.to_string())
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// User Defined and loaded brush libraries.
pub type Libraries = super::swatchlib::Libraries<BrushLibrary>;

/// A path stroked with brush `name` (what the resource gathering follows).
fn stroke(name: &str) -> Node {
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    if let Some(st) = ap.stroke_mut() {
        st.brush = Some(name.into());
    }
    let line = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false);
    Node::path(NodeId(0), PathData::single(line), ap)
}

// ---------- built-in libraries ----------

/// A library generated in code.
pub struct BuiltinBrushLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn() -> BrushLibrary,
}

/// The built-in brush libraries, in menu order: original brushes generated in code.
pub const BUILTIN: &[BuiltinBrushLibrary] = &[
    BuiltinBrushLibrary { id: "art-ribbons", name: "Art Ribbons", make: art_ribbons },
    BuiltinBrushLibrary { id: "calligraphic-nibs", name: "Calligraphic Nibs", make: calligraphic_nibs },
    BuiltinBrushLibrary { id: "pattern-borders", name: "Pattern Borders", make: pattern_borders },
    BuiltinBrushLibrary { id: "scatter-shapes", name: "Scatter Shapes", make: scatter_shapes },
    BuiltinBrushLibrary { id: "game-trails", name: "Game Trails", make: game_trails },
    BuiltinBrushLibrary { id: "energy-beams", name: "Energy Beams", make: energy_beams },
    BuiltinBrushLibrary { id: "dry-media", name: "Dry Media", make: dry_media },
];

/// Built-in library `id`, made once.
fn builtin_library(id: &str) -> Option<Arc<BrushLibrary>> {
    static MADE: LazyLock<Vec<Arc<BrushLibrary>>> = LazyLock::new(|| BUILTIN.iter().map(|b| Arc::new((b.make)())).collect());
    BUILTIN.iter().position(|b| b.id == id).and_then(|i| MADE.get(i).cloned())
}

/// Library `name` of `brushes`, `doc` holding the resources their art uses.
fn built(name: &str, mut doc: Document, brushes: Vec<Brush>) -> BrushLibrary {
    brush::store(&mut doc, &brushes);
    doc.title = name.into();
    BrushLibrary { name: name.into(), brushes, doc }
}

fn filled(p: PathData, paint: Paint) -> Node {
    Node::path(NodeId(0), p, Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], ..Default::default() })
}

fn black(p: PathData) -> Node {
    filled(p, Paint::solid(Color::BLACK))
}

fn group(children: Vec<Node>) -> Node {
    Node::group(NodeId(0), children.into_iter().map(Arc::new).collect())
}

fn polygon(pts: &[(f64, f64)]) -> PathData {
    let pts: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// A band along y = `f(x)` for x in 0..`len`, `thick` pt thick, in `n` steps.
fn band(len: f64, thick: f64, n: usize, f: impl Fn(f64) -> f64) -> PathData {
    let xs: Vec<f64> = (0..=n).map(|i| len * i as f64 / n as f64).collect();
    let mut pts: Vec<Point> = xs.iter().map(|&x| Point::new(x, f(x) - thick / 2.0)).collect();
    pts.extend(xs.iter().rev().map(|&x| Point::new(x, f(x) + thick / 2.0)));
    PathData::single(SubPath::polyline(&pts, true))
}

/// An even-odd ring of radii `outer` and `inner` around `c`.
fn ring(c: Point, outer: f64, inner: f64) -> Node {
    let disc = |r: f64| shapes::ellipse(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r));
    let children = [disc(outer), disc(inner)].into_iter().map(|p| Arc::new(Node::path(NodeId(0), p, Appearance::default()))).collect();
    let mut n = Node::new(NodeId(0), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    n.appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK)))], ..Default::default() }.into();
    n
}

fn art(name: &str, art: Node, colorization: Colorization) -> Brush {
    Brush { name: name.into(), kind: BrushKind::Art(ArtBrush { art, colorization, ..Default::default() }) }
}

/// Art Ribbons: art brushes, 100 pt long, drawn here; Striped Ribbon is filled with the library's
/// own pattern, Pinstripes.
fn art_ribbons() -> BrushLibrary {
    let mut doc = Document::new(100.0, 100.0);
    let stripe = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 6.0, 1.5)), {
        let mut a = Appearance::default();
        a.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::from_hex("#2f80c9").unwrap_or(Color::BLACK)))));
        a
    });
    add_pattern(&mut doc, PatternDef { tile: Rect::new(0.0, 0.0, 6.0, 3.0), ..PatternDef::new("Pinstripes", vec![Arc::new(stripe)]) });
    let notched = polygon(&[(0.0, -3.0), (100.0, -3.0), (95.0, 0.0), (100.0, 3.0), (0.0, 3.0), (5.0, 0.0)]);
    let double = group(vec![black(shapes::rectangle(Rect::new(0.0, -2.5, 100.0, -1.5))), black(shapes::rectangle(Rect::new(0.0, 1.5, 100.0, 2.5)))]);
    let wave = band(100.0, 1.6, 64, |x| 2.5 * (x / 100.0 * 4.0 * std::f64::consts::TAU).sin());
    // Round and thick at the start, tapering to a point.
    let teardrop = {
        let n = 48;
        let half = |x: f64| 3.5 * (1.0 - x / 100.0).powf(0.8) * (x / 4.0).min(1.0).sqrt();
        let xs = |i: usize| 100.0 * i as f64 / n as f64;
        let mut pts: Vec<Point> = (0..=n).map(xs).map(|x| Point::new(x, -half(x))).collect();
        pts.extend((0..n).rev().map(xs).map(|x| Point::new(x, half(x))));
        PathData::single(SubPath::polyline(&pts, true))
    };
    let striped = {
        let mut n =
            filled(shapes::rectangle(Rect::new(0.0, -4.0, 100.0, 4.0)), Paint::Pattern { pattern: "Pinstripes".into(), xf: Default::default() });
        n.appearance.set_stroke(Paint::solid(Color::from_hex("#1d4f7c").unwrap_or(Color::BLACK)));
        n
    };
    let brushes = vec![
        art("Notched Ribbon", black(notched), Colorization::Tints),
        art("Double Line", double, Colorization::Tints),
        art("Wave", black(wave), Colorization::Tints),
        art("Teardrop", black(teardrop), Colorization::Tints),
        art("Striped Ribbon", striped, Colorization::None),
    ];
    built("Art Ribbons", doc, brushes)
}

fn nib(name: &str, angle: f64, roundness: f64, size: f64) -> Brush {
    Brush { name: name.into(), kind: BrushKind::Calligraphic(Calligraphic { angle, roundness, size, ..Default::default() }) }
}

/// Calligraphic Nibs: pen nibs of several shapes; Pressure Pen's size follows the pen pressure.
fn calligraphic_nibs() -> BrushLibrary {
    let pressure = Brush {
        name: "Pressure Pen".into(),
        kind: BrushKind::Calligraphic(Calligraphic {
            angle: 30.0,
            roundness: 60.0,
            size: 5.0,
            variation: [0.0, 0.0, 4.0],
            modes: Some([Variation::Fixed, Variation::Fixed, Variation::Pressure]),
        }),
    };
    let brushes = vec![
        nib("Fine Nib", 45.0, 25.0, 2.0),
        nib("Broad Nib", 45.0, 15.0, 9.0),
        nib("Chisel", 0.0, 8.0, 6.0),
        nib("Round Pen", 0.0, 100.0, 4.0),
        pressure,
    ];
    built("Calligraphic Nibs", Document::new(100.0, 100.0), brushes)
}

fn border(name: &str, side: Node, corner: Option<Node>, fit: PatternFit) -> Brush {
    let kind = BrushKind::Pattern(PatternBrush { side, outer_corner: corner.clone(), inner_corner: corner, fit, ..Default::default() });
    Brush { name: name.into(), kind }
}

/// An invisible rectangle that fixes a tile's width.
fn spacer(r: Rect) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::default())
}

/// Pattern Borders: pattern brushes of tiles drawn here; Beads has corner tiles.
fn pattern_borders() -> BrushLibrary {
    let zigzag = black(polygon(&[(0.0, 1.4), (5.0, -2.6), (10.0, 1.4), (10.0, 2.6), (5.0, -1.4), (0.0, 2.6)]));
    let beads = group(vec![black(shapes::rectangle(Rect::new(0.0, -0.3, 8.0, 0.3))), black(shapes::ellipse(Rect::new(1.5, -2.5, 6.5, 2.5)))]);
    let dash_dot = group(vec![
        spacer(Rect::new(0.0, -1.0, 12.0, 1.0)),
        black(shapes::rounded_rectangle(Rect::new(0.5, -0.75, 6.5, 0.75), 0.75)),
        black(shapes::ellipse(Rect::new(8.5, -1.0, 10.5, 1.0))),
    ]);
    let scallops = black(band(10.0, 1.0, 20, |x| -3.0 * (x / 10.0 * std::f64::consts::PI).sin()));
    let brushes = vec![
        border("Zigzag", zigzag, None, PatternFit::Stretch),
        border("Beads", beads, Some(ring(Point::ZERO, 3.5, 1.5)), PatternFit::Stretch),
        border("Dash Dot", dash_dot, None, PatternFit::AddSpace),
        border("Scallops", scallops, None, PatternFit::Stretch),
    ];
    built("Pattern Borders", Document::new(100.0, 100.0), brushes)
}

fn scatter(name: &str, art: Node, s: Scatter) -> Brush {
    Brush { name: name.into(), kind: BrushKind::Scatter(Scatter { art, ..s }) }
}

/// Scatter Shapes: scatter brushes of shapes drawn here; Sparkles scatters the library's own
/// symbol, Sparkle.
fn scatter_shapes() -> BrushLibrary {
    let mut doc = Document::new(100.0, 100.0);
    // A four-pointed gold sparkle in the ±10 pt box symbol instances assume.
    let points: Vec<Point> = (0..8)
        .map(|i| {
            let r = if i % 2 == 0 { 10.0 } else { 2.5 };
            let a = std::f64::consts::FRAC_PI_4 * i as f64;
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    let gold = Color::from_hex("#e0a526").unwrap_or(Color::BLACK);
    let sparkle = Node::path(doc.alloc_id(), PathData::single(SubPath::polyline(&points, true)), {
        let mut a = Appearance::default();
        a.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(gold))));
        a
    });
    doc.symbols.push(Symbol { dynamic: false, name: "Sparkle".into(), art: Arc::new(sparkle) });
    let instance = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Sparkle".into(), xf: Affine::scale(0.3) });
    let random = |a: f64, b: f64| (a, b);
    let base = Scatter { colorization: Colorization::Tints, ..Default::default() };
    let brushes = vec![
        scatter(
            "Stars",
            black(shapes::star(Point::ZERO, 4.0, 1.7, 5, 0.0)),
            Scatter {
                size: random(70.0, 130.0),
                spacing: random(150.0, 220.0),
                scatter: random(-80.0, 80.0),
                rotation: random(-180.0, 180.0),
                ..base.clone()
            },
        ),
        scatter("Rings", ring(Point::ZERO, 2.5, 1.5), Scatter { spacing: (140.0, 140.0), ..base.clone() }),
        scatter(
            "Triangles",
            black(shapes::polygon(Point::ZERO, 3.0, 3, 0.0)),
            Scatter { spacing: (120.0, 120.0), rotation: (90.0, 90.0), rotation_relative_to_path: true, ..base.clone() },
        ),
        scatter(
            "Sparkles",
            instance,
            Scatter {
                size: random(60.0, 140.0),
                spacing: random(110.0, 180.0),
                scatter: random(-60.0, 60.0),
                rotation: random(-45.0, 45.0),
                colorization: Colorization::None,
                ..base
            },
        ),
    ];
    built("Scatter Shapes", doc, brushes)
}

/// Game Trails: art brushes for motion lines and impact streaks (original game HUD / VFX art).
fn game_trails() -> BrushLibrary {
    let speed = {
        let n = 32;
        let half = |x: f64| 2.2 * (1.0 - x / 100.0).powf(1.2).max(0.08);
        let xs = |i: usize| 100.0 * i as f64 / n as f64;
        let mut pts: Vec<Point> = (0..=n).map(xs).map(|x| Point::new(x, -half(x))).collect();
        pts.extend((0..n).rev().map(xs).map(|x| Point::new(x, half(x))));
        PathData::single(SubPath::polyline(&pts, true))
    };
    let dash = group(vec![
        black(shapes::rounded_rectangle(Rect::new(0.0, -1.0, 14.0, 1.0), 1.0)),
        black(shapes::rounded_rectangle(Rect::new(22.0, -1.0, 36.0, 1.0), 1.0)),
        black(shapes::rounded_rectangle(Rect::new(44.0, -1.0, 58.0, 1.0), 1.0)),
        black(shapes::rounded_rectangle(Rect::new(66.0, -1.0, 80.0, 1.0), 1.0)),
        black(shapes::rounded_rectangle(Rect::new(88.0, -1.0, 100.0, 1.0), 1.0)),
    ]);
    let burst = {
        // A wedge that flares from a tip at x=0 to a wide end at x=100.
        let tip = Point::new(0.0, 0.0);
        let top = Point::new(100.0, -6.0);
        let bot = Point::new(100.0, 6.0);
        PathData::single(SubPath::polyline(&[tip, top, bot], true))
    };
    let zigzag = band(100.0, 1.8, 40, |x| {
        let t = (x / 12.5).floor();
        let local = (x / 12.5) - t;
        if (t as i32) % 2 == 0 { -3.0 + 6.0 * local } else { 3.0 - 6.0 * local }
    });
    let brushes = vec![
        art("Speed Line", black(speed), Colorization::Tints),
        art("Dash Trail", dash, Colorization::Tints),
        art("Burst Ray", black(burst), Colorization::Tints),
        art("Zigzag", black(zigzag), Colorization::Tints),
    ];
    built("Game Trails", Document::new(100.0, 100.0), brushes)
}

/// Energy Beams: art brushes for laser, plasma and charge strokes (original game VFX art).
fn energy_beams() -> BrushLibrary {
    let laser = {
        let n = 32;
        let half = |x: f64| 1.6 * (1.0 - (x / 100.0 - 0.5).abs() * 1.6).max(0.12);
        let xs = |i: usize| 100.0 * i as f64 / n as f64;
        let mut pts: Vec<Point> = (0..=n).map(xs).map(|x| Point::new(x, -half(x))).collect();
        pts.extend((0..n).rev().map(xs).map(|x| Point::new(x, half(x))));
        PathData::single(SubPath::polyline(&pts, true))
    };
    let plasma = band(100.0, 2.2, 48, |x| 4.0 * (x / 100.0 * std::f64::consts::PI).sin());
    let charge = band(100.0, 1.6, 56, |x| 3.5 * (x / 100.0 * std::f64::consts::TAU).sin());
    let spark = group(vec![
        black(shapes::rounded_rectangle(Rect::new(0.0, -1.2, 10.0, 1.2), 0.8)),
        black(shapes::rounded_rectangle(Rect::new(18.0, -0.9, 28.0, 0.9), 0.7)),
        black(shapes::rounded_rectangle(Rect::new(36.0, -1.4, 48.0, 1.4), 0.9)),
        black(shapes::rounded_rectangle(Rect::new(56.0, -0.8, 66.0, 0.8), 0.6)),
        black(shapes::rounded_rectangle(Rect::new(74.0, -1.1, 86.0, 1.1), 0.8)),
        black(shapes::rounded_rectangle(Rect::new(92.0, -0.7, 100.0, 0.7), 0.5)),
    ]);
    let brushes = vec![
        art("Laser Bolt", black(laser), Colorization::Tints),
        art("Plasma Arc", black(plasma), Colorization::Tints),
        art("Charge Wave", black(charge), Colorization::Tints),
        art("Spark Trail", spark, Colorization::Tints),
    ];
    built("Energy Beams", Document::new(100.0, 100.0), brushes)
}

/// The fixed-seed number source of the Dry Media grain: integer steps and plain arithmetic, so the
/// art is the same on every run and every platform.
struct Grain(u64);

impl Grain {
    /// The next number in 0..1.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    /// The next number in `a`..`b`.
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

/// `floor` at x = `x0` and `x1`, rising smoothly to 1 within `ramp` pt of each.
fn ends(x: f64, x0: f64, x1: f64, ramp: f64, floor: f64) -> f64 {
    let t = ((x - x0).min(x1 - x) / ramp).clamp(0.0, 1.0);
    floor + (1.0 - floor) * t * t * (3.0 - 2.0 * t)
}

/// A closed band from x = `x0` to `x1` in `n` steps, at most `half` pt either side of y = 0 times
/// `taper(x)`; each edge point is pulled in by up to `rough` of that.
fn rough_band(g: &mut Grain, (x0, x1): (f64, f64), n: usize, half: f64, rough: f64, taper: impl Fn(f64) -> f64) -> PathData {
    let xs: Vec<f64> = (0..=n).map(|i| x0 + (x1 - x0) * i as f64 / n as f64).collect();
    let mut pts: Vec<Point> = xs.iter().map(|&x| Point::new(x, -half * taper(x) * (1.0 - rough * g.next()))).collect();
    let bottom: Vec<Point> = xs.iter().rev().map(|&x| Point::new(x, half * taper(x) * (1.0 - rough * g.next()))).collect();
    pts.extend(bottom);
    PathData::single(SubPath::polyline(&pts, true))
}

/// A small four-cornered fleck around (`cx`, `cy`), at most `w` × `h` pt either side; one corner
/// in each direction, so its outline never crosses itself.
fn fleck(g: &mut Grain, cx: f64, cy: f64, w: f64, h: f64) -> PathData {
    let mut side = |a: f64, b: f64| g.range(a, b);
    let pts = [
        (cx - w * side(0.6, 1.0), cy + h * side(-0.3, 0.3)),
        (cx + w * side(-0.3, 0.3), cy - h * side(0.6, 1.0)),
        (cx + w * side(0.6, 1.0), cy + h * side(-0.3, 0.3)),
        (cx + w * side(-0.3, 0.3), cy + h * side(0.6, 1.0)),
    ];
    polygon(&pts)
}

/// `body` with `holes` (inside it, apart from each other) cut out, even-odd, in gray `k`: what
/// lies under the stroke shows through the holes.
fn holed(body: PathData, holes: Vec<PathData>, k: f32) -> Node {
    let children = std::iter::once(body).chain(holes).map(|p| Arc::new(Node::path(NodeId(0), p, Appearance::default()))).collect();
    let mut n = Node::new(NodeId(0), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    n.appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::gray(k))))], ..Default::default() }.into();
    n
}

/// Path `p` filled with gray `k` (1 is black).
fn ink(p: PathData, k: f32) -> Node {
    filled(p, Paint::solid(Color::gray(k)))
}

/// Slivers `w` × `h` pt (ranges) in `count` slots along x = 6..94 inside a band `half` pt wide
/// times `taper`. Each stays within `reach` of the band's half width from y = 0, taken where the
/// band is narrowest over the sliver and the 3 pt either side of it, so with `reach` below the
/// band's roughness margin no sliver touches the band's edge or another sliver.
fn slivers(g: &mut Grain, count: usize, half: f64, reach: f64, (w, h): ((f64, f64), (f64, f64)), taper: impl Fn(f64) -> f64) -> Vec<PathData> {
    let slot = 88.0 / count as f64;
    (0..count)
        .map(|i| {
            let cx = 6.0 + slot * (i as f64 + 0.5);
            let (w, h) = (g.range(w.0, w.1), g.range(h.0, h.1));
            let room = reach * half * taper(cx - w - 3.0).min(taper(cx + w + 3.0));
            let cy = g.range(-1.0, 1.0) * (room - h).max(0.0);
            fleck(g, cx, cy, w, h)
        })
        .collect()
}

/// Dry Media: art brushes of grainy dry drawing marks, chalk, pencil, crayon and charcoal (original
/// vector grain from fixed seeds; every brush 100 pt long, black and grays that take the stroke
/// color as tints, gaps cut out so the paper shows).
pub(crate) fn dry_media() -> BrushLibrary {
    // Chalk Grain, 100 × 8 pt: a soft band with narrow scratches and chips along its edges.
    let chalk = {
        let mut g = Grain(0x0c4a_1c0a);
        let taper = |x: f64| ends(x, 0.0, 100.0, 10.0, 0.35);
        let body = rough_band(&mut g, (0.0, 100.0), 36, 4.0, 0.25, taper);
        let scratches = slivers(&mut g, 14, 4.0, 0.7, ((1.2, 2.4), (0.15, 0.3)), taper);
        let mut parts = vec![holed(body, scratches, 0.8)];
        for i in 0..18 {
            let cx = 3.0 + 94.0 * (i as f64 + g.range(0.2, 0.8)) / 18.0;
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let cy = side * 4.0 * taper(cx) * g.range(0.7, 0.95);
            let (w, h) = (g.range(0.3, 0.7), g.range(0.25, 0.5));
            let f = fleck(&mut g, cx, cy, w, h);
            parts.push(ink(f, 1.0));
        }
        group(parts)
    };
    // Graphite Pencil, 100 × 2.5 pt: a solid tapered core with fine slanted hatching at its edges.
    let pencil = {
        let mut g = Grain(0x009e_4c11);
        let core = rough_band(&mut g, (0.0, 100.0), 40, 0.8, 0.12, |x| ends(x, 0.0, 100.0, 18.0, 0.25));
        let mut parts = vec![ink(core, 1.0)];
        for i in 0..24 {
            let x0 = 2.0 + 92.0 * (i as f64 + g.range(0.0, 0.4)) / 24.0;
            let len = g.range(1.5, 3.0);
            let t = g.range(0.12, 0.2);
            let slant = g.range(0.1, 0.3);
            let y0 = if i % 2 == 0 { -g.range(0.85, 1.05) } else { g.range(0.85, 1.25) - t - slant };
            parts.push(ink(polygon(&[(x0, y0 + slant), (x0 + len, y0), (x0 + len, y0 + t), (x0, y0 + slant + t)]), 0.6));
        }
        group(parts)
    };
    // Wax Crayon, 100 × 6 pt: a dense band with small gaps in its middle and short overlapping
    // marks along both edges.
    let crayon = {
        let mut g = Grain(0x00c0_a7e5);
        let taper = |x: f64| ends(x, 0.0, 100.0, 6.0, 0.5);
        let body = rough_band(&mut g, (0.0, 100.0), 30, 2.6, 0.2, taper);
        let gaps = slivers(&mut g, 16, 2.6, 0.34, ((0.6, 1.6), (0.15, 0.3)), taper);
        let mut parts = vec![holed(body, gaps, 0.9)];
        for i in 0..36 {
            let w = g.range(1.0, 2.2);
            let cx = (1.0 + w).max(98.0 * (i as f64 + g.range(0.0, 1.0)) / 36.0).min(99.0 - w);
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let h = g.range(0.4, 0.6);
            // Clear of the gaps, which stay within 0.9 pt of the middle.
            let inner = 0.9 + h;
            let cy = side * g.range(inner, (2.4 * taper(cx)).max(inner));
            let f = fleck(&mut g, cx, cy, w, h);
            parts.push(ink(f, 1.0));
        }
        group(parts)
    };
    // Charcoal Dust, 100 × 7 pt: a dark band broken into pieces, with flecks of dust either side.
    let dust = {
        let mut g = Grain(0xc4a2_c0a1);
        let mut parts = vec![];
        let mut x0 = 0.0;
        while x0 < 100.0 {
            let x1 = (x0 + g.range(14.0, 24.0)).min(100.0);
            let x1 = if 100.0 - x1 < 8.0 { 100.0 } else { x1 };
            let piece = rough_band(&mut g, (x0, x1), 10, 2.0, 0.3, |x| ends(x, 0.0, 100.0, 8.0, 0.4) * ends(x, x0, x1, 2.0, 0.3));
            parts.push(ink(piece, 1.0));
            x0 = x1 + g.range(1.5, 3.0);
        }
        for i in 0..30 {
            let cx = 2.0 + 96.0 * (i as f64 + g.range(0.2, 0.8)) / 30.0;
            let side = if g.next() < 0.5 { -1.0 } else { 1.0 };
            let s = g.range(0.15, 0.3);
            let cy = side * g.range(2.3, 3.5 - s);
            let f = fleck(&mut g, cx, cy, s, s);
            parts.push(ink(f, if i % 3 == 0 { 0.7 } else { 1.0 }));
        }
        group(parts)
    };
    let brushes = vec![
        art("Chalk Grain", chalk, Colorization::Tints),
        art("Graphite Pencil", pencil, Colorization::Tints),
        art("Wax Crayon", crayon, Colorization::Tints),
        art("Charcoal Dust", dust, Colorization::Tints),
    ];
    built("Dry Media", Document::new(100.0, 100.0), brushes)
}

// ---------- commands ----------

fn builtin_info(b: &BuiltinBrushLibrary) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category: "builtIn" }
}

/// Every brush library, in menu order: built-in, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    BUILTIN.iter().map(builtin_info).chain(s.brush_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<BrushLibrary>)> {
    if let Some(found) = s.brush_libraries.get(key) {
        return Some(found);
    }
    if let Some(found) = concealed_library_id(s, key).and_then(|id| s.brush_libraries.get(&id)) {
        return Some(found);
    }
    if let Some(b) = BUILTIN.iter().find(|b| b.id == key) {
        return Some((builtin_info(b), builtin_library(key)?));
    }
    let id = libraries(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    library(s, &id)
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<BrushLibrary>)> {
    let key = str_param(p, "library").ok_or_else(|| bad(cmd, "missing `library` (see brush.libraries)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no brush library `{key}` (see brush.libraries)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.brush_libraries.rescan();
    let concealed = concealed_library_ids(s, []);
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            let (id, path) = (shown_id(&concealed, &l.id), s.brush_libraries.shown_path(&l.id));
            json!({"id": id, "name": l.name, "category": l.category, "count": count, "path": path})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.brush_libraries.shown_user_dir() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "brush.library")?;
    let brushes: Vec<Value> = lib.brushes.iter().map(|b| json!({"name": b.name, "type": b.kind.type_id()})).collect();
    Ok(json!({"id": shown_library_id(s, &info.id), "name": info.name, "category": info.category, "brushes": brushes}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.addFromLibrary";
    let (info, lib) = library_param(s, p, C)?;
    let mut names = str_list(p, "names");
    names.extend(str_param(p, "name").map(str::to_string));
    if let Some(n) = names.iter().find(|n| lib.brush(n).is_none()) {
        return Err(bad(C, format!("no brush `{n}` in `{}`", lib.name)));
    }
    let mut picked: Vec<String> = if names.is_empty() { lib.brushes.iter().map(|b| b.name.clone()).collect() } else { names };
    // Each once, in the order asked for.
    let mut seen = HashSet::new();
    picked.retain(|n| seen.insert(n.clone()));
    let apply = bool_or(p, "apply", false);
    if apply && picked.is_empty() {
        return Err(bad(C, format!("`{}` has no brushes to apply", lib.name)));
    }
    let choices = SwatchChoices::parse(C, p)?;
    let clip = Clipboard::gather(&lib.doc, picked.iter().map(|n| stroke(n)).collect());
    // The typing `s.edit` would settle first is settled before the probe below, so the document
    // the edit installs is the one it starts from.
    if !s.previewing && super::typing_in_progress(s) {
        super::settle_interaction(s)?;
    }
    let st = s.doc()?;
    let before: HashSet<String> = brush::library(&st.doc).into_iter().map(|b| b.name).collect();
    // A library brush's name in the document: its own, or the one the import gave it.
    let target = |renamed: &[Value], n: &str| -> String {
        let to = renamed.iter().find(|r| r["kind"] == "brush" && r["from"] == n).and_then(|r| r["to"].as_str());
        to.unwrap_or(n).to_string()
    };
    // Where the brushes land, worked out on a copy that the edit then installs: nothing to add and
    // nothing to apply is no edit, and a click that only applies brushes the document has is
    // named for that.
    let mut probe = (*st.doc).clone();
    let imported = clip.import_into(&mut probe, &choices, false)?;
    let doc_names: Vec<String> = picked.iter().map(|n| target(&imported.renamed, n)).collect();
    let paths = if apply { brush_targets(&st.doc, &roots(s, p)?) } else { vec![] };
    let paint = stroke_paint(s);
    if imported.added > 0 || !paths.is_empty() {
        let label = if imported.added > 0 { "Add to Brushes" } else { "Apply Brush" };
        s.edit(label, |d, _| {
            *d = probe;
            if let Some(first) = doc_names.first() {
                d.unknown.insert(brush::CURRENT_KEY.into(), json!(first));
                apply_brush(d, &paths, first, &paint);
            }
            Ok(())
        })?;
    } else if let Some(first) = doc_names.first() {
        s.execute("brush.setCurrent", &json!({ "name": first }))?;
    }
    let (added, existing): (Vec<&String>, Vec<&String>) = doc_names.iter().partition(|n| !before.contains(*n));
    let applied: Vec<u64> = paths.iter().map(|id| id.0).collect();
    Ok(json!({"library": shown_library_id(s, &info.id), "names": doc_names, "added": added, "existing": existing, "applied": applied}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.saveLibrary";
    let st = s.doc()?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = BrushLibrary::from_document(&st.doc, &str_list(p, "names"), name).map_err(|e| bad(C, e))?;
    if lib.is_empty() {
        return Err(bad(C, "the document has no brushes to save"));
    }
    let mut out = json!({ "count": lib.len() });
    let bytes = vectorcraft_format::save_file(&lib.doc);
    s.brush_libraries.write(p, &lib.name, BRUSHES_EXT, FileData::Binary(bytes), &mut out, C)?;
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.loadLibrary";
    let (info, lib) = s.brush_libraries.load(p, C, |bytes, file| {
        let loaded = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?;
        if loaded.preview_only {
            return Err(bad(C, "only this Affinity file's embedded preview could be read; use File › Open to see it with its warning"));
        }
        BrushLibrary::from_document(&loaded.doc, &[], stem(file).to_string()).map_err(|e| bad(C, e))
    })?;
    Ok(json!({"library": shown_library_id(s, &info.id), "name": info.name, "count": lib.len()}))
}
