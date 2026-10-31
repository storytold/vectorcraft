//! Graphic style libraries: named, read-only sets of graphic styles (Window → Graphic Style
//! Libraries). The built-in ones are generated here in code ([`STYLE_LIBRARIES`]); user libraries
//! are `.vcstyles` files ([`write`], [`read`]): JSON holding the styles and the pattern
//! definitions they paint with.
//!
//! Library styles stand alone: no id, and no links to the swatches of the document they came from
//! ([`StyleLibrary::from_document`]).

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_geom::Point;

use crate::{
    Appearance, AppearanceItem, Arrowhead, Dash, Document, Effect, FillLayer, GraphicStyle, Knockout, LineCap, LineJoin, PatternDef, StrokeAlign,
    StrokeLayer, WidthProfile,
};

/// A graphic style library: its name, its styles and the patterns they paint with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleLibrary {
    pub name: String,
    #[serde(default)]
    pub styles: Vec<GraphicStyle>,
    /// The pattern swatches the styles' fills and strokes paint with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternDef>,
}

impl StyleLibrary {
    pub fn len(&self) -> usize {
        self.styles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
    pub fn style(&self, name: &str) -> Option<&GraphicStyle> {
        self.styles.iter().find(|g| g.name == name)
    }

    /// The document's styles `names` (empty: all of them, in panel order) as a library named
    /// `name`, standing alone ([`GraphicStyle::standalone`]) with the patterns they paint with.
    pub fn from_document(d: &Document, names: &[String], name: String) -> Result<Self, String> {
        if let Some(n) = names.iter().find(|n| d.graphic_style(n).is_none()) {
            return Err(format!("no graphic style `{n}`"));
        }
        let styles: Vec<GraphicStyle> =
            d.graphic_styles.iter().filter(|g| names.is_empty() || names.contains(&g.name)).map(GraphicStyle::standalone).collect();
        let mut patterns: Vec<PatternDef> = vec![];
        for p in styles.iter().flat_map(GraphicStyle::patterns) {
            if !patterns.iter().any(|x| x.name == p)
                && let Some(def) = d.pattern(p)
            {
                patterns.push(def.clone());
            }
        }
        Ok(Self { name, styles, patterns })
    }
}

impl GraphicStyle {
    /// Do both styles look the same (appearance and transparency; names and ids aside)?
    pub fn same_look(&self, other: &GraphicStyle) -> bool {
        (&self.appearance, self.opacity, self.blend, self.isolate, self.knockout, self.unit_box)
            == (&other.appearance, other.opacity, other.blend, other.isolate, other.knockout, other.unit_box)
    }

    /// The style as a library holds it: no id, and its paints unlinked from swatches.
    pub fn standalone(&self) -> GraphicStyle {
        let mut g = GraphicStyle { id: 0, ..self.clone() };
        for it in &mut g.appearance.items {
            match it.paint_mut() {
                // The colour shown stays, unlinked.
                Paint::Solid { swatch, tint, .. } => (*swatch, *tint) = (None, 1.0),
                Paint::Gradient(gp) => {
                    gp.swatch = None;
                    gp.gradient.stops.iter_mut().for_each(|st| st.set_color(st.color, None));
                }
                Paint::None | Paint::Pattern { .. } => {}
            }
        }
        g
    }

    /// The names of the patterns the style's fills and strokes paint with.
    pub fn patterns(&self) -> impl Iterator<Item = &str> {
        self.appearance.items.iter().filter_map(|it| match it.paint() {
            Paint::Pattern { pattern, .. } => Some(pattern.as_str()),
            _ => None,
        })
    }
}

// ---------- .vcstyles files ----------

/// The extension (and `format` header) of graphic style library files.
pub const STYLES_EXT: &str = "vcstyles";

/// The file: a header around the library.
#[derive(Serialize, Deserialize)]
struct NativeFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    library: StyleLibrary,
}

/// `lib` as a `.vcstyles` file.
pub fn write(lib: &StyleLibrary) -> String {
    let file = NativeFile { format: STYLES_EXT.into(), version: 1, library: lib.clone() };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Read a `.vcstyles` file; an unnamed library is called `name`.
pub fn read(text: &str, name: &str) -> Result<StyleLibrary, String> {
    let f: NativeFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not a graphic style library: {e}"))?;
    if f.format != STYLES_EXT {
        return Err(format!("not a graphic style library (format `{}`)", f.format));
    }
    let mut lib = f.library;
    if lib.name.trim().is_empty() {
        lib.name = name.into();
    }
    Ok(lib)
}

/// Does `text` look like a `.vcstyles` file (its header names the format)?
pub fn sniff(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    let head = t.char_indices().nth(256).map_or(t, |(i, _)| &t[..i]);
    t.starts_with('{') && head.contains(&format!("\"{STYLES_EXT}\""))
}

// ---------- built-in libraries ----------

/// A built-in library: a stable id, its menu name and the function that makes its styles.
pub struct BuiltinStyleLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn() -> Vec<GraphicStyle>,
}

/// The built-in graphic style libraries, in menu order.
pub const STYLE_LIBRARIES: &[BuiltinStyleLibrary] = &[
    BuiltinStyleLibrary { id: "shadows-glows", name: "Shadows and Glows", make: shadows_glows },
    BuiltinStyleLibrary { id: "outlines-rules", name: "Outlines and Rules", make: outlines_rules },
    BuiltinStyleLibrary { id: "hand-drawn", name: "Hand-Drawn", make: hand_drawn },
    BuiltinStyleLibrary { id: "gradient-finishes", name: "Gradient Finishes", make: gradient_finishes },
    BuiltinStyleLibrary { id: "shape-effects", name: "Shape Effects", make: shape_effects },
    BuiltinStyleLibrary { id: "blends-transparency", name: "Blends and Transparency", make: blends_transparency },
];

/// Built-in library `id`, made once.
pub fn builtin_style_library(id: &str) -> Option<Arc<StyleLibrary>> {
    static LIBS: OnceLock<Vec<Arc<StyleLibrary>>> = OnceLock::new();
    let libs = LIBS.get_or_init(|| {
        STYLE_LIBRARIES.iter().map(|b| Arc::new(StyleLibrary { name: b.name.into(), styles: (b.make)(), patterns: vec![] })).collect()
    });
    STYLE_LIBRARIES.iter().position(|b| b.id == id).map(|i| libs[i].clone())
}

fn hex(h: &str) -> Color {
    Color::from_hex(h).unwrap_or(Color::BLACK)
}

fn solid(h: &str) -> Paint {
    Paint::solid(hex(h))
}

fn fill(h: &str) -> FillLayer {
    FillLayer::new(solid(h))
}

fn stroke(h: &str, width: f64) -> StrokeLayer {
    StrokeLayer::new(solid(h), width)
}

/// A stroke with round caps and joins.
fn round(h: &str, width: f64) -> StrokeLayer {
    StrokeLayer { cap: LineCap::Round, join: LineJoin::Round, ..stroke(h, width) }
}

/// Live effect `id` with `params` (the others keep the effect's defaults).
fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// A gradient of `(offset, colour, opacity)` stops.
fn gradient(kind: GradientKind, stops: &[(f32, &str, f32)]) -> Gradient {
    Gradient::new(kind, stops.iter().map(|&(offset, c, opacity)| GradientStop { opacity, ..GradientStop::new(offset, hex(c)) }).collect())
}

/// A linear gradient fitted to each object's box at `angle` degrees (90: upwards).
fn linear(angle: f64, stops: &[(f32, &str, f32)]) -> Paint {
    Paint::Gradient(Box::new(GradientPaint { angle, ..GradientPaint::new(gradient(GradientKind::Linear, stops)) }))
}

/// A radial gradient placed in the unit box (each object gets it at the same place relative to its
/// bounds): centred at `(x, y)` with `radius`.
fn radial_at(x: f64, y: f64, radius: f64, stops: &[(f32, &str, f32)]) -> Paint {
    let geom = GradientGeom { start: Point::new(x, y), end: Point::new(x + radius, y), aspect: 1.0, focal: None };
    Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(gradient(GradientKind::Radial, stops)) }))
}

/// A style of fills and strokes `items` (painted first to last) and object `effects`.
fn style(name: &str, items: Vec<AppearanceItem>, effects: Vec<Effect>) -> GraphicStyle {
    GraphicStyle::new(name, Appearance { items, effects, contents_index: None })
}

use AppearanceItem::{Fill as F, Stroke as S};

fn shadows_glows() -> Vec<GraphicStyle> {
    let shadow = |opacity: f64, x: f64, y: f64, blur: f64, color: &str| {
        fx("stylize.dropShadow", json!({"mode": "multiply", "opacity": opacity, "x": x, "y": y, "blur": blur, "color": color}))
    };
    vec![
        style("Soft Shadow", vec![F(fill("#f4f1ea"))], vec![shadow(45.0, 4.0, 4.0, 6.0, "#1d1b2a")]),
        style(
            "Hard Shadow",
            vec![F(fill("#ffd166")), S(stroke("#1d1b2a", 2.0))],
            vec![fx("stylize.dropShadow", json!({"mode": "normal", "opacity": 100.0, "x": 5.0, "y": 5.0, "blur": 0.0, "color": "#1d1b2a"}))],
        ),
        style("Lifted Card", vec![F(fill("#ffffff")), S(stroke("#d9d9e3", 0.5))], vec![shadow(30.0, 0.0, 8.0, 12.0, "#2b2d42")]),
        style(
            "Halo",
            vec![F(fill("#2b2d42"))],
            vec![fx("stylize.outerGlow", json!({"mode": "normal", "opacity": 90.0, "blur": 8.0, "color": "#8ecae6"}))],
        ),
        style(
            "Neon Tube",
            vec![S(round("#ff4fd8", 4.0)), S(round("#ffe8fb", 1.5))],
            vec![fx("stylize.outerGlow", json!({"mode": "normal", "opacity": 85.0, "blur": 7.0, "color": "#ff4fd8"}))],
        ),
        style(
            "Inner Light",
            vec![F(fill("#3a86ff"))],
            vec![fx("stylize.innerGlow", json!({"mode": "screen", "opacity": 70.0, "blur": 9.0, "color": "#ffffff", "source": "center"}))],
        ),
        style(
            "Ember",
            vec![F(fill("#e85d04"))],
            vec![
                fx("stylize.innerGlow", json!({"mode": "screen", "opacity": 80.0, "blur": 6.0, "color": "#ffba08", "source": "edge"})),
                fx("stylize.outerGlow", json!({"mode": "normal", "opacity": 70.0, "blur": 5.0, "color": "#dc2f02"})),
            ],
        ),
        style("Feathered", vec![F(fill("#8338ec"))], vec![fx("stylize.feather", json!({"radius": 6.0}))]),
        style("Haze", vec![F(FillLayer { opacity: 0.8, ..fill("#06d6a0") })], vec![fx("blur.gaussian", json!({"radius": 3.0}))]),
    ]
}

fn outlines_rules() -> Vec<GraphicStyle> {
    let dashed =
        |h: &str, width: f64, pattern: Vec<f64>| StrokeLayer { dash: Some(Dash { pattern, offset: 0.0, align_corners: true }), ..round(h, width) };
    let keyline = |h: &str, width: f64, offset: f64| StrokeLayer {
        effects: vec![fx("path.offsetPath", json!({"offset": offset, "joins": "round"}))],
        ..stroke(h, width)
    };
    vec![
        style("Double Rule", vec![F(fill("#ffffff")), S(stroke("#14213d", 6.0)), S(stroke("#ffffff", 3.0))], vec![]),
        style("Triple Rule", vec![F(fill("#fefae0")), S(stroke("#283618", 10.0)), S(stroke("#fefae0", 6.0)), S(stroke("#283618", 2.0))], vec![]),
        style("Dashed Border", vec![F(fill("#e9f5f2")), S(dashed("#264653", 2.0, vec![6.0, 4.0]))], vec![]),
        style("Dotted Border", vec![F(fill("#fff4e6")), S(dashed("#e76f51", 3.0, vec![0.0, 6.0]))], vec![]),
        style("Offset Keyline", vec![F(fill("#e9c46a")), S(keyline("#264653", 1.5, 4.0))], vec![]),
        style("Inset Keyline", vec![F(fill("#2a9d8f")), S(keyline("#ffffff", 1.5, -4.0))], vec![]),
        style("Outside Stroke", vec![F(fill("#f4a261")), S(StrokeLayer { align: StrokeAlign::Outside, ..stroke("#264653", 4.0) })], vec![]),
        style("Tapered Ink", vec![S(StrokeLayer { profile: Some(WidthProfile::lens()), ..round("#1b1b1b", 5.0) })], vec![]),
        style("Pointer", vec![S(StrokeLayer { end_arrow: Some(Arrowhead::Triangle), ..round("#e76f51", 2.0) })], vec![]),
        style("Highlighter", vec![S(StrokeLayer { opacity: 0.6, blend: BlendMode::Multiply, ..round("#ffd60a", 10.0) })], vec![]),
    ]
}

fn hand_drawn() -> Vec<GraphicStyle> {
    let rough = |h: &str, width: f64, seed: u32| StrokeLayer {
        effects: vec![fx("distort.roughen", json!({"size": 1.5, "relative": true, "detail": 8.0, "points": "smooth", "seed": seed}))],
        ..round(h, width)
    };
    let scribble = |h: &str, angle: f64, opacity: f32| FillLayer {
        opacity,
        effects: vec![fx("stylize.scribble", json!({"angle": angle, "strokeWidth": 1.5, "spacing": 4.0, "curviness": 5.0, "variation": 0.5}))],
        ..fill(h)
    };
    vec![
        style("Sketchy Line", vec![F(fill("#fefae0")), S(rough("#1d3557", 1.25, 1)), S(rough("#1d3557", 0.75, 7))], vec![]),
        style("Scribble Fill", vec![F(scribble("#e63946", 30.0, 1.0)), S(rough("#1d3557", 1.25, 3))], vec![]),
        style("Crosshatch", vec![F(scribble("#457b9d", 45.0, 0.8)), F(scribble("#457b9d", -45.0, 0.8)), S(rough("#1d3557", 1.0, 5))], vec![]),
        style(
            "Wobble",
            vec![F(fill("#a8dadc")), S(round("#1d3557", 2.0))],
            vec![fx("distort.tweak", json!({"h": 3.0, "v": 3.0, "relative": true, "seed": 5}))],
        ),
        style(
            "Zig Zag Edge",
            vec![
                F(fill("#f1faee")),
                S(StrokeLayer {
                    effects: vec![fx("distort.zigZag", json!({"size": 3.0, "relative": false, "ridges": 6, "points": "corner"}))],
                    ..stroke("#e63946", 1.5)
                }),
            ],
            vec![],
        ),
        style(
            "Wavy Edge",
            vec![F(fill("#bde0fe")), S(round("#3a5a9b", 1.5))],
            vec![fx("distort.zigZag", json!({"size": 3.0, "relative": false, "ridges": 5, "points": "smooth"}))],
        ),
        style("Puffy", vec![F(fill("#ffc8dd")), S(round("#c9184a", 1.5))], vec![fx("distort.puckerBloat", json!({"amount": 25.0}))]),
        style("Starburst", vec![F(fill("#ffbe0b")), S(stroke("#fb5607", 1.0))], vec![fx("distort.puckerBloat", json!({"amount": -40.0}))]),
    ]
}

fn gradient_finishes() -> Vec<GraphicStyle> {
    let rim = |h: &str| S(stroke(h, 1.0));
    vec![
        style(
            "Glossy",
            vec![
                F(FillLayer::new(linear(-90.0, &[(0.0, "#7ab8ff", 1.0), (1.0, "#1d4ed8", 1.0)]))),
                F(FillLayer::new(linear(-90.0, &[(0.0, "#ffffff", 0.7), (0.5, "#ffffff", 0.0)]))),
                rim("#1e3a8a"),
            ],
            vec![],
        ),
        style(
            "Brushed Metal",
            vec![
                F(FillLayer::new(linear(
                    0.0,
                    &[(0.0, "#bfbfbf", 1.0), (0.3, "#f5f5f5", 1.0), (0.55, "#a6a6a6", 1.0), (0.8, "#e6e6e6", 1.0), (1.0, "#8c8c8c", 1.0)],
                ))),
                rim("#595959"),
            ],
            vec![],
        ),
        style(
            "Spotlight",
            vec![F(FillLayer::new(radial_at(0.35, 0.3, 0.9, &[(0.0, "#ffffff", 1.0), (0.35, "#ffbe0b", 1.0), (1.0, "#fb5607", 1.0)])))],
            vec![],
        ),
        style(
            "Sunset Band",
            vec![F(FillLayer::new(linear(-90.0, &[(0.0, "#ffbe0b", 1.0), (0.35, "#fb5607", 1.0), (0.7, "#ff006e", 1.0), (1.0, "#8338ec", 1.0)])))],
            vec![],
        ),
        style(
            "Sea Glass",
            vec![
                F(FillLayer {
                    opacity: 0.85,
                    ..FillLayer::new(linear(-45.0, &[(0.0, "#caf0f8", 1.0), (0.5, "#48cae4", 1.0), (1.0, "#0077b6", 1.0)]))
                }),
                rim("#03045e"),
            ],
            vec![],
        ),
        style(
            "Pearl",
            vec![
                F(FillLayer::new(radial_at(0.4, 0.35, 0.8, &[(0.0, "#ffffff", 1.0), (0.5, "#e8e1f0", 1.0), (1.0, "#b8b0c8", 1.0)]))),
                S(stroke("#8a7fa0", 0.75)),
            ],
            vec![],
        ),
        style(
            "Chrome Rim",
            vec![F(fill("#22223b")), S(StrokeLayer::new(linear(-90.0, &[(0.0, "#f8f9fa", 1.0), (0.5, "#6c757d", 1.0), (1.0, "#dee2e6", 1.0)]), 4.0))],
            vec![],
        ),
        style(
            "Glow Orb",
            vec![F(FillLayer::new(radial_at(0.5, 0.5, 0.5, &[(0.0, "#ffffff", 1.0), (0.4, "#80ffdb", 1.0), (1.0, "#5390d9", 1.0)])))],
            vec![fx("stylize.outerGlow", json!({"mode": "normal", "opacity": 60.0, "blur": 6.0, "color": "#80ffdb"}))],
        ),
    ]
}

fn shape_effects() -> Vec<GraphicStyle> {
    let basic = |f: &str, s: &str, w: f64| vec![F(fill(f)), S(round(s, w))];
    vec![
        style("Rounded", basic("#90be6d", "#43aa8b", 2.0), vec![fx("stylize.roundCorners", json!({"radius": 10.0}))]),
        style(
            "Pill Badge",
            vec![F(fill("#f94144"))],
            vec![fx("convertToShape.roundedRectangle", json!({"relative": true, "extraW": 12.0, "extraH": 6.0, "radius": 40.0}))],
        ),
        style(
            "Round Badge",
            vec![F(fill("#577590")), S(stroke("#ffffff", 2.0))],
            vec![fx("convertToShape.ellipse", json!({"relative": true, "extraW": 10.0, "extraH": 10.0}))],
        ),
        style("Arc", basic("#f8961e", "#f3722c", 1.5), vec![fx("warp.arc", json!({"bend": 30.0}))]),
        style("Flag Wave", basic("#277da1", "#1d4e89", 1.5), vec![fx("warp.flag", json!({"bend": 25.0}))]),
        style("Bulge", basic("#f9c74f", "#f8961e", 1.5), vec![fx("warp.bulge", json!({"bend": 30.0}))]),
        style("Twisted", basic("#4d908e", "#264653", 1.5), vec![fx("distort.twist", json!({"angle": 30.0}))]),
        style(
            "Echo",
            vec![F(FillLayer { opacity: 0.55, ..fill("#43aa8b") }), S(stroke("#264653", 1.0))],
            vec![fx("distort.transform", json!({"copies": 3, "moveH": 4.0, "moveV": 4.0}))],
        ),
        style(
            "Spiral Copies",
            vec![F(FillLayer { opacity: 0.35, ..fill("#f94144") }), S(stroke("#9d0208", 0.75))],
            vec![fx("distort.transform", json!({"copies": 5, "rotate": 12.0, "scaleH": 90.0, "scaleV": 90.0}))],
        ),
    ]
}

fn blends_transparency() -> Vec<GraphicStyle> {
    let with =
        |g: GraphicStyle, opacity: f32, blend: BlendMode, isolate: bool, knockout: Knockout| GraphicStyle { opacity, blend, isolate, knockout, ..g };
    let one = |name: &str, h: &str| style(name, vec![F(fill(h))], vec![]);
    // A fill blending with the fills under it, so the style shows on any backdrop.
    let over = |paint: Paint, blend: BlendMode| F(FillLayer { blend, ..FillLayer::new(paint) });
    vec![
        with(one("Multiply Tint", "#ffbe0b"), 1.0, BlendMode::Multiply, false, Knockout::Neutral),
        with(one("Darken Veil", "#4cc9f0"), 0.85, BlendMode::Darken, false, Knockout::Neutral),
        with(style("Ghosted", vec![F(fill("#7209b7")), S(stroke("#3a0ca3", 1.0))], vec![]), 0.4, BlendMode::Normal, false, Knockout::Neutral),
        with(one("Invert", "#ffffff"), 1.0, BlendMode::Difference, false, Knockout::Neutral),
        style("Two Tone", vec![F(fill("#4361ee")), F(FillLayer { opacity: 0.6, blend: BlendMode::Multiply, ..fill("#f72585") })], vec![]),
        style(
            "Overlay Glaze",
            vec![F(fill("#f72585")), over(linear(-90.0, &[(0.0, "#ffffff", 1.0), (1.0, "#000000", 1.0)]), BlendMode::Overlay)],
            vec![],
        ),
        style(
            "Screen Light",
            vec![F(fill("#3a0ca3")), over(radial_at(0.5, 0.4, 0.6, &[(0.0, "#4cc9f0", 1.0), (1.0, "#000000", 1.0)]), BlendMode::Screen)],
            vec![],
        ),
        with(
            style(
                "Soft Light Wash",
                vec![F(fill("#0096c7")), over(linear(0.0, &[(0.0, "#ffffff", 1.0), (1.0, "#03045e", 1.0)]), BlendMode::SoftLight)],
                vec![],
            ),
            1.0,
            BlendMode::Normal,
            true,
            Knockout::Neutral,
        ),
        with(style("Knockout Group", vec![F(fill("#4895ef")), S(stroke("#3f37c9", 1.0))], vec![]), 0.7, BlendMode::Normal, true, Knockout::On),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_libraries_are_non_empty_with_unique_names() {
        let mut ids: Vec<&str> = STYLE_LIBRARIES.iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), STYLE_LIBRARIES.len(), "library ids are unique");
        for b in STYLE_LIBRARIES {
            let lib = builtin_style_library(b.id).unwrap();
            assert_eq!(lib.name, b.name);
            assert!(lib.len() >= 6, "{}", b.id);
            let mut names: Vec<&str> = lib.styles.iter().map(|g| g.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), lib.len(), "style names in {} are unique", b.id);
            assert!(lib.styles.iter().all(|g| g.id == 0 && g.unit_box && !g.appearance.items.is_empty()));
        }
        assert!(Arc::ptr_eq(&builtin_style_library("hand-drawn").unwrap(), &builtin_style_library("hand-drawn").unwrap()), "made once");
        assert!(builtin_style_library("nope").is_none());
    }

    #[test]
    fn vcstyles_round_trip() {
        let mut lib = (*builtin_style_library("gradient-finishes").unwrap()).clone();
        lib.styles.extend(builtin_style_library("blends-transparency").unwrap().styles.iter().cloned());
        let text = write(&lib);
        assert!(sniff(&text) && !sniff("{\"format\": \"vectorcraft\"}") && !sniff("GIMP Palette"));
        assert_eq!(read(&text, "Fallback").unwrap(), lib);
        // An unnamed library takes the file's name; other JSON isn't a library.
        let unnamed = write(&StyleLibrary { name: " ".into(), ..lib.clone() });
        assert_eq!(read(&unnamed, "Mine").unwrap().name, "Mine");
        assert!(read("{\"format\": \"vcswatches\", \"version\": 1, \"name\": \"x\"}", "x").is_err());
    }

    #[test]
    fn a_library_from_a_document_stands_alone_and_keeps_its_patterns() {
        let mut d = Document::new(100.0, 100.0);
        let red = Paint::Solid { color: Color::rgb(1.0, 0.0, 0.0), swatch: Some("Red".into()), tint: 0.5 };
        let dots = Paint::Pattern { pattern: "Dots".into(), xf: Default::default() };
        d.patterns.push(PatternDef::new("Dots", vec![]));
        let mut g = GraphicStyle::new("Linked", Appearance::basic(red, dots, 2.0));
        g.id = 9;
        d.graphic_styles.push(g);
        let lib = StyleLibrary::from_document(&d, &["Linked".into()], "Mine".into()).unwrap();
        assert_eq!(lib.len(), 1);
        let s = &lib.styles[0];
        assert_eq!((s.id, s.appearance.fill_paint()), (0, Paint::solid(Color::rgb(1.0, 0.0, 0.0))), "unlinked, no id");
        assert_eq!(lib.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Dots"]);
        assert!(s.same_look(&GraphicStyle { name: "Other".into(), id: 3, ..s.clone() }));
        assert!(!s.same_look(&GraphicStyle { opacity: 0.5, ..s.clone() }));
        assert_eq!(StyleLibrary::from_document(&d, &[], "All".into()).unwrap().len(), d.graphic_styles.len());
        assert!(StyleLibrary::from_document(&d, &["nope".into()], "x".into()).is_err());
        // Gradients lose their swatch and their stops' links too.
        let mut gp = GradientPaint::new(gradient(GradientKind::Linear, &[(0.0, "#ff0000", 1.0), (1.0, "#0000ff", 1.0)]));
        gp.swatch = Some("Sunset".into());
        gp.gradient.stops[0].set_color(hex("#ff0000"), Some(("Red".into(), 0.5)));
        let g = GraphicStyle::new("G", Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 1.0)).standalone();
        let Paint::Gradient(gp) = g.appearance.fill_paint() else { panic!("a gradient") };
        assert!(gp.swatch.is_none() && gp.gradient.stops.iter().all(|s| s.swatch.is_none() && s.tint == 1.0));
    }
}
