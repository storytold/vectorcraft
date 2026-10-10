//! VectorCraft live effects (Illustrator's Effect menu).
//!
//! Effects live in appearance stacks as [`Effect`] `{id, params, visible}` records. This crate
//! interprets them:
//!
//! - **Geometry effects** (Distort & Transform, Path, Convert to Shape, Round Corners, Scribble,
//!   Warp) rewrite a path: [`apply_geometry`] evaluates them in stack order.
//! - **Raster effects** (Drop Shadow, Inner/Outer Glow, Feather, Gaussian Blur, and the
//!   Photoshop-style filters of [`pixel`]: Radial Blur, Smart Blur, Color Halftone, Crystallize, Mezzotint, Pointillize,
//!   Unsharp Mask and Glowing Edges) are described by
//!   [`raster_effects`] and painted by the renderer; [`outset`] says how far they reach beyond
//!   the geometry.
//! - **Stroke geometry** ([`stroke`]): arrowheads, dash patterns and width profiles, shared by the
//!   renderer, the exporters and Outline Stroke.
//! - [`effect_catalog`] lists every effect with its menu path, parameter documentation and the
//!   defaults of Illustrator's dialogs. Missing parameters always fall back to those defaults.
//! - [`reshape`] applies geometry effects to type, images, symbol instances and live objects
//!   through their outlines.
//! - [`evaluate_container`] turns a group's or layer's own fills, strokes and geometry effects into
//!   art painting its members (its raster effects then apply to the composite).
//! - [`clip_outline`] is the region a clip group clips to, shared by the renderer and the SVG and
//!   PDF writers.
//! - **Plug-in effects** (`plugin.<plug-in id>`, Effect › Plug-ins) are geometry effects run by an
//!   installed WebAssembly plug-in (`vectorcraft-plugins`); [`effect_info`] describes them like
//!   built-in ones, and an effect whose plug-in isn't installed leaves the geometry as it is.
//! - **Colour adjustments** (Brightness/Contrast, Curves, Levels, Hue/Saturation, Shift to Color,
//!   Temperature/Tint) recolour what an object paints with: [`adjust`] evaluates them for the
//!   renderer and the exporters.
//!
//! Everything is deterministic: "random" effects (Roughen, Tweak, Scribble) use a seeded hash
//! noise (`seed` parameter, default 0); the random Pixelate filters hash their pattern's cells,
//! anchored at the object's centre.
#![forbid(unsafe_code)]

mod adjust;
mod bake;
mod clip;
mod distort;
mod group;
mod live;
mod marks;
pub mod pixel;
mod raster;
mod reshape;
mod revolve;
pub mod stroke;
mod stylize;
mod util;
mod warp;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_scale;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{AppearanceItem, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

pub use adjust::{ADJUSTMENTS, ColorMap, ImageHook, adjust, adjust_in_document, color_map, curve_at, curve_points, has_adjustment, is_adjustment};
pub use bake::{StrokeArt, bake_appearance, bake_document, expand_art, expand_leaf, fresh_ids, needs_bake};
pub use clip::clip_outline;
pub use group::{
    OutlineHook, PATHFINDER_EFFECTS, evaluate_container, has_container_appearance, has_pathfinder, is_pathfinder, member_shapes, paints,
    pathfinder_children,
};
pub use live::{expand_live, expand_live_deep, expanded_live_group, text_outliner};
pub use marks::{CROP_MARKS, crop_marks_art, has_crop_marks};
pub use pixel::{PIXEL_EFFECTS, PixelFx, PixelSpace};
pub use raster::{RasterFx, outset, raster_effects};
pub use reshape::{expand_outlined, needs_outline, outline_art, outline_text, reshape};
pub use revolve::{REVOLVE, has_revolve, revolve_art, revolve_options, validate_revolve};
pub use warp::{WarpStyle, warp_point};

/// Catalogue entry for one effect.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectInfo {
    /// Stable id, e.g. `distort.roughen`.
    pub id: &'static str,
    /// Menu label (with the ellipsis when the effect has a dialog).
    pub label: &'static str,
    /// Effect-menu path, e.g. `["Effect", "Distort & Transform"]`.
    pub menu: &'static [&'static str],
    /// Human/agent-readable parameter documentation.
    pub params: &'static str,
    /// Dialog defaults.
    pub defaults: Value,
    /// Raster (painted) rather than geometry effect.
    pub raster: bool,
    /// The parameters that are distances in points (Scale Strokes & Effects scales them).
    pub lengths: Lengths,
}

/// Which of an effect's parameters are distances in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lengths {
    /// Always distances.
    pub always: &'static [&'static str],
    /// Distances while the effect's `relative` parameter is false (percentages of the object's
    /// size otherwise).
    pub absolute: &'static [&'static str],
}

impl Lengths {
    /// The distance parameters while the effect's `relative` parameter is `relative`.
    pub fn keys(&self, relative: bool) -> impl Iterator<Item = &'static str> + '_ {
        self.always.iter().chain(if relative { &[][..] } else { self.absolute }).copied()
    }
}

/// Is parameter `key` of effect `id` a distance in points while its `relative` parameter is
/// `relative`?
pub fn is_length(id: &str, key: &str, relative: bool) -> bool {
    catalog_index().get(id).is_some_and(|info| info.lengths.keys(relative).any(|k| k == key))
}

/// The distance parameters of effect `id` (none for unknown effects).
fn lengths_of(id: &str) -> Lengths {
    let always = |always| Lengths { always, absolute: &[] };
    match id {
        "convertToShape.rectangle" | "convertToShape.ellipse" => always(&["extraW", "extraH", "width", "height"]),
        "convertToShape.roundedRectangle" => always(&["extraW", "extraH", "width", "height", "radius"]),
        "distort.roughen" | "distort.zigZag" => Lengths { always: &[], absolute: &["size"] },
        "distort.tweak" => Lengths { always: &[], absolute: &["h", "v"] },
        "distort.transform" => always(&["moveH", "moveV"]),
        "path.offsetPath" => always(&["offset"]),
        "threeD.revolve" => always(&["offset"]),
        "path.outlineStroke" => always(&["width"]),
        "stylize.roundCorners" | "stylize.feather" | "blur.gaussian" | "blur.smart" | "sharpen.unsharpMask" => always(&["radius"]),
        "stylize.scribble" => always(&["overlap", "strokeWidth", "spacing", "variation"]),
        "stylize.dropShadow" => always(&["x", "y", "blur"]),
        "stylize.innerGlow" | "stylize.outerGlow" => always(&["blur"]),
        "stylize.glowingEdges" => always(&["edgeWidth", "smoothness"]),
        "pixelate.colorHalftone" => always(&["maxRadius"]),
        "pixelate.crystallize" | "pixelate.pointillize" => always(&["cellSize"]),
        "texture.craquelure" => always(&["crackSpacing"]),
        "texture.mosaicTiles" => always(&["tileSize"]),
        "texture.patchwork" => always(&["squareSize"]),
        "texture.stainedGlass" => always(&["cellSize"]),
        _ => Lengths::default(),
    }
}

const WARP_DOC: &str =
    "{bend: % (-100..100, 50), horizontal: % distortion (0), vertical: % distortion (0), orientation: \"horizontal\"|\"vertical\"}";
const DT: &[&str] = &["Effect", "Distort & Transform"];
const PATH: &[&str] = &["Effect", "Path"];
const SHAPE: &[&str] = &["Effect", "Convert to Shape"];
const STYLIZE: &[&str] = &["Effect", "Stylize"];
const WARP: &[&str] = &["Effect", "Warp"];
const BLUR: &[&str] = &["Effect", "Blur"];
const DISTORT: &[&str] = &["Effect", "Distort"];
const SHARPEN: &[&str] = &["Effect", "Sharpen"];
const PIXELATE: &[&str] = &["Effect", "Pixelate"];
const TEXTURE: &[&str] = &["Effect", "Texture"];
const VIDEO: &[&str] = &["Effect", "Video"];
const PATHFINDER: &[&str] = &["Effect", "Pathfinder"];
const PLUGINS: &[&str] = &["Effect", "Plug-ins"];
const ADJUST: &[&str] = &["Effect", "Color Adjustments"];

/// The warp styles in Illustrator's Style menu order: (id suffix, label).
pub const WARP_STYLES: [(&str, &str); 15] = [
    ("arc", "Arc…"),
    ("arcLower", "Arc Lower…"),
    ("arcUpper", "Arc Upper…"),
    ("arch", "Arch…"),
    ("bulge", "Bulge…"),
    ("shellLower", "Shell Lower…"),
    ("shellUpper", "Shell Upper…"),
    ("flag", "Flag…"),
    ("wave", "Wave…"),
    ("fish", "Fish…"),
    ("rise", "Rise…"),
    ("fisheye", "Fisheye…"),
    ("inflate", "Inflate…"),
    ("squeeze", "Squeeze…"),
    ("twist", "Twist…"),
];

/// Every supported effect with its dialog defaults.
pub fn effect_catalog() -> Vec<EffectInfo> {
    let g = |id, label, menu, params, defaults| EffectInfo { id, label, menu, params, defaults, raster: false, lengths: lengths_of(id) };
    let r = |id, label, menu, params, defaults| EffectInfo { id, label, menu, params, defaults, raster: true, lengths: lengths_of(id) };
    let mut v = vec![
        g(
            REVOLVE,
            "Revolve…",
            &["Effect", "3D and Materials"],
            "{angle: degrees (0..360, 360), offset: pt (0..100000, 0), edge: left|right, rotationX: degrees (0), rotationY: degrees (0), rotationZ: degrees (0), perspective: % (0..100, 0), segments: integer (8..128, 64), shade: bool (true), lightAzimuth: degrees (-45), lightElevation: degrees (45), lightIntensity: % (80), ambient: % (25), expandVisibleOnly: bool (true, trim covered opaque solid surfaces only on Expand Appearance)}. Open paths create uncapped surfaces; intersecting profiles use approximate painter visibility.",
            json!({"angle":360.0,"offset":0.0,"edge":"left","rotationX":0.0,"rotationY":0.0,"rotationZ":0.0,"perspective":0.0,"segments":64,"shade":true,"lightAzimuth":-45.0,"lightElevation":45.0,"lightIntensity":80.0,"ambient":25.0,"expandVisibleOnly":true}),
        ),
        g(
            "convertToShape.rectangle",
            "Rectangle…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width: pt (absolute, 100), height: pt (absolute, 100)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0}),
        ),
        g(
            "convertToShape.roundedRectangle",
            "Rounded Rectangle…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width, height (absolute), radius: pt (9)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0, "radius": 9.0}),
        ),
        g(
            "convertToShape.ellipse",
            "Ellipse…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width, height (absolute)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0}),
        ),
        g(
            "distort.freeDistort",
            "Free Distort…",
            DT,
            "{corners: [[x,y]×4] new positions of the bounding-box corners TL, TR, BR, BL in unit box coordinates (identity [[0,0],[1,0],[1,1],[0,1]])}",
            json!({"corners": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]}),
        ),
        g("distort.puckerBloat", "Pucker & Bloat…", DT, "{amount: % (-200 pucker … 200 bloat, default 0)}", json!({"amount": 0.0})),
        g(
            "distort.roughen",
            "Roughen…",
            DT,
            "{size: 0..100 (5), relative: bool (true: % of size; false: pt), detail: per inch 0..100 (10), points: \"smooth\"|\"corner\", seed: int (0)}",
            json!({"size": 5.0, "relative": true, "detail": 10.0, "points": "smooth", "seed": 0}),
        ),
        g(
            "distort.transform",
            "Transform…",
            DT,
            "{scaleH: % (100), scaleV: % (100), moveH: pt (0; right = +), moveV: pt (0; down = +), rotate: deg (0; counter-clockwise), copies: 0..1000 (0; each copy transforms the last again), reflectX: bool (false; flips left to right), reflectY: bool (false; flips top to bottom), reference: 0..8 (4; the point of the bounds' 9-point grid it scales, rotates and reflects about: 0 top left, 4 centre, 8 bottom right), random: bool (false; each scale goes a random share of the way from 100 % to its value, each move and the angle a random share of theirs, differently for each object and the same on every redraw)}",
            json!({"scaleH": 100.0, "scaleV": 100.0, "moveH": 0.0, "moveV": 0.0, "rotate": 0.0, "copies": 0, "reflectX": false, "reflectY": false, "reference": 4, "random": false}),
        ),
        g(
            "distort.tweak",
            "Tweak…",
            DT,
            "{h: amount (10), v: amount (10), relative: bool (true: % of size), anchors: bool (true), in: bool (true), out: bool (true), seed: int (0)}",
            json!({"h": 10.0, "v": 10.0, "relative": true, "anchors": true, "in": true, "out": true, "seed": 0}),
        ),
        g("distort.twist", "Twist…", DT, "{angle: deg (-3600..3600, 10)}", json!({"angle": 10.0})),
        g(
            "distort.zigZag",
            "Zig Zag…",
            DT,
            "{size: (10), relative: bool (false: pt; true: % of size), ridges: per segment 0..100 (4), points: \"smooth\"|\"corner\"}",
            json!({"size": 10.0, "relative": false, "ridges": 4, "points": "smooth"}),
        ),
        g(
            "path.offsetPath",
            "Offset Path…",
            PATH,
            "{offset: pt (10; negative insets), joins: \"miter\"|\"round\"|\"bevel\", miterLimit: (4)}",
            json!({"offset": 10.0, "joins": "miter", "miterLimit": 4.0}),
        ),
        g(
            "path.outlineStroke",
            "Outline Stroke",
            PATH,
            "{width?: pt (defaults to the stroke's weight)} the outline of the stroke the effect is on (on a fill or the object: the top stroke), with its caps, joins, dashes, alignment, width profile and arrowheads",
            json!({}),
        ),
        g("stylize.roundCorners", "Round Corners…", STYLIZE, "{radius: pt (10)}", json!({"radius": 10.0})),
        g(
            "stylize.scribble",
            "Scribble…",
            STYLIZE,
            "{angle: deg (30), overlap: pt (0), strokeWidth: pt (3), curviness: % (5), spacing: pt (5), variation: pt (0.5), seed: int (0)} (simplified: a single hatching scribble outlined to a filled shape)",
            json!({"angle": 30.0, "overlap": 0.0, "strokeWidth": 3.0, "curviness": 5.0, "spacing": 5.0, "variation": 0.5, "seed": 0}),
        ),
        r(
            "stylize.dropShadow",
            "Drop Shadow…",
            STYLIZE,
            "{mode: blend mode (\"multiply\"), opacity: % (75), x: pt (7), y: pt (7), blur: pt (5), color: \"#rrggbb\" (\"#000000\")}",
            json!({"mode": "multiply", "opacity": 75.0, "x": 7.0, "y": 7.0, "blur": 5.0, "color": "#000000"}),
        ),
        r(
            "stylize.innerGlow",
            "Inner Glow…",
            STYLIZE,
            "{mode: blend mode (\"screen\"), opacity: % (75), blur: pt (5), color: \"#rrggbb\" (\"#ffffff\"), source: \"center\"|\"edge\" (\"edge\")}",
            json!({"mode": "screen", "opacity": 75.0, "blur": 5.0, "color": "#ffffff", "source": "edge"}),
        ),
        r(
            "stylize.outerGlow",
            "Outer Glow…",
            STYLIZE,
            "{mode: blend mode (\"screen\"), opacity: % (75), blur: pt (5), color: \"#rrggbb\" (\"#ffff00\")}",
            json!({"mode": "screen", "opacity": 75.0, "blur": 5.0, "color": "#ffff00"}),
        ),
        r("stylize.feather", "Feather…", STYLIZE, "{radius: pt (5)}", json!({"radius": 5.0})),
        r("blur.gaussian", "Gaussian Blur…", BLUR, "{radius: pt (5)}", json!({"radius": 5.0})),
        r(
            "blur.radial",
            "Radial Blur…",
            BLUR,
            "{amount: 1..100 (10; spin: an arc of `amount`°, zoom: the content scaled by up to about ±amount/2 %), method: \"spin\"|\"zoom\" (\"spin\"), quality: \"draft\"|\"good\"|\"best\" (\"good\"; 16, 64 or 256 samples)} blurs around the object's centre",
            json!({"amount": 10.0, "method": "spin", "quality": "good"}),
        ),
        r(
            "blur.smart",
            "Smart Blur…",
            BLUR,
            "{radius: pt 0.1..100 (3), threshold: levels 0.1..100 (25; only colours closer than this blur together, so edges stay sharp), quality: \"low\"|\"medium\"|\"high\" (\"medium\"; 5, 7 or 9 samples across)} (Normal mode)",
            json!({"radius": 3.0, "threshold": 25.0, "quality": "medium"}),
        ),
        r(
            "distort.diffuseGlow",
            "Diffuse Glow…",
            DISTORT,
            "{graininess: 0..10 (6; white grain, thicker in the glow), glowAmount: 0..20 (10), clearAmount: 0..20 (15; the higher, the more of the image stays clear of glow)} renders the object as if seen through a soft diffusion filter: its highlights glow white under see-through white grain",
            json!({"graininess": 6.0, "glowAmount": 10.0, "clearAmount": 15.0}),
        ),
        r(
            "distort.glass",
            "Glass…",
            DISTORT,
            "{distortion: 0..20 (5), smoothness: 1..15 (3), texture: \"blocks\"|\"canvas\"|\"frosted\"|\"tinyLens\" (\"frosted\"; surfaces made in code), scaling: % 50..200 (100), invert: bool (false; turns the surface's heights over)} makes the object look as if seen through glass",
            json!({"distortion": 5.0, "smoothness": 3.0, "texture": "frosted", "scaling": 100.0, "invert": false}),
        ),
        r(
            "distort.oceanRipple",
            "Ocean Ripple…",
            DISTORT,
            "{rippleSize: 1..15 (9), rippleMagnitude: 0..20 (9)} adds randomly spaced ripples, as if the object were under water",
            json!({"rippleSize": 9.0, "rippleMagnitude": 9.0}),
        ),
        r(
            "sharpen.unsharpMask",
            "Unsharp Mask…",
            SHARPEN,
            "{amount: % 1..500 (50), radius: pt 0.1..250 (1; σ of the blur edges are found against), threshold: levels 0..255 (0; smaller differences are left alone)}",
            json!({"amount": 50.0, "radius": 1.0, "threshold": 0.0}),
        ),
        r(
            "stylize.glowingEdges",
            "Glowing Edges…",
            STYLIZE,
            "{edgeWidth: pt 1..14 (2), edgeBrightness: 0..20 (6), smoothness: pt 1..15 (5)} finds alpha-weighted Sobel edges and draws bright coloured outlines on black",
            json!({"edgeWidth": 2.0, "edgeBrightness": 6.0, "smoothness": 5.0}),
        ),
        r(
            "pixelate.colorHalftone",
            "Color Halftone…",
            PIXELATE,
            "{maxRadius: pt 4..127 (8; the radius of a dot at full strength, which fills its square cell), channel1: screen angle deg -360..360 (108), channel2: deg (162), channel3: deg (90), channel4: deg (45)} screens each colour channel (red, green, blue: channels 1 to 3; in CMYK documents cyan, magenta, yellow, black: 1 to 4) at its angle into dots whose area follows the channel's mean over their cell",
            json!({"maxRadius": 8.0, "channel1": 108.0, "channel2": 162.0, "channel3": 90.0, "channel4": 45.0}),
        ),
        r(
            "pixelate.crystallize",
            "Crystallize…",
            PIXELATE,
            "{cellSize: pt 3..300 (10)} redraws the object as polygon crystals of solid colour around random points about cellSize apart",
            json!({"cellSize": 10.0}),
        ),
        r(
            "pixelate.mezzotint",
            "Mezzotint…",
            PIXELATE,
            "{type: \"fineDots\"|\"mediumDots\"|\"grainyDots\"|\"coarseDots\"|\"shortLines\"|\"mediumLines\"|\"longLines\"|\"shortStrokes\"|\"mediumStrokes\"|\"longStrokes\" (\"fineDots\")} turns each colour channel fully on or off against a random pattern of dots, lines or strokes: fully saturated colours",
            json!({"type": "fineDots"}),
        ),
        r(
            "pixelate.pointillize",
            "Pointillize…",
            PIXELATE,
            "{cellSize: pt 3..300 (5)} redraws the object as randomly placed dots of its colours on a white canvas",
            json!({"cellSize": 5.0}),
        ),
        r(
            "texture.craquelure",
            "Craquelure…",
            TEXTURE,
            "{crackSpacing: pt 2..100 (15; how far apart the cracks are), crackDepth: 0..10 (6; how deep and wide the cracks are, and how high the plates and the tones stand), crackBrightness: 0..10 (9; how brightly the plaster is lit)} paints the object on relief plaster cracked into plates and along the contours of its tones",
            json!({"crackSpacing": 15.0, "crackDepth": 6.0, "crackBrightness": 9.0}),
        ),
        r(
            "texture.grain",
            "Grain…",
            TEXTURE,
            "{intensity: 0..100 (40), contrast: 0..100 (50; the image's contrast, 50 leaves it as it is), grainType: \"regular\"|\"soft\"|\"sprinkles\"|\"clumped\"|\"contrasty\"|\"enlarged\"|\"stippled\"|\"horizontal\"|\"vertical\"|\"speckle\" (\"regular\")} adds grain in 1 pt grains (2 pt Enlarged, clumps Clumped, streaks Horizontal and Vertical); Sprinkles and Stippled use the background colour, white",
            json!({"intensity": 40.0, "contrast": 50.0, "grainType": "regular"}),
        ),
        r(
            "texture.mosaicTiles",
            "Mosaic Tiles…",
            TEXTURE,
            "{tileSize: pt 2..100 (12), groutWidth: 1..15 (3; the grout is half a point wide per step), lightenGrout: 0..10 (9; how light the grout is)} lays the object in irregular tiles bevelled at their edges, with sunken grout between them",
            json!({"tileSize": 12.0, "groutWidth": 3.0, "lightenGrout": 9.0}),
        ),
        r(
            "texture.patchwork",
            "Patchwork…",
            TEXTURE,
            "{squareSize: pt 0..10 (4; 0 draws 1 pt squares), relief: 0..25 (8)} redraws the object in squares of the colour around their centres, raised to heights that follow its highlights and shadows, a little more or less at random, lit from the top left",
            json!({"squareSize": 4.0, "relief": 8.0}),
        ),
        r(
            "texture.stainedGlass",
            "Stained Glass…",
            TEXTURE,
            "{cellSize: pt 2..50 (10), borderThickness: 1..20 (4; the lead between the panes, half a point wide per step, in the foreground colour: black), lightIntensity: 0..10 (3; a light behind the object's centre, fading out towards its corners)} redraws the object as single-coloured panes around random points about cellSize apart",
            json!({"cellSize": 10.0, "borderThickness": 4.0, "lightIntensity": 3.0}),
        ),
        r(
            "texture.texturizer",
            "Texturizer…",
            TEXTURE,
            "{texture: \"brick\"|\"burlap\"|\"canvas\"|\"sandstone\" (\"canvas\"; surfaces made in code), scaling: % 50..200 (100), relief: 0..50 (4), lightDirection: \"bottom\"|\"bottomLeft\"|\"left\"|\"topLeft\"|\"top\"|\"topRight\"|\"right\"|\"bottomRight\" (\"top\"), invert: bool (false; turns the surface's heights over)} paints the object on a surface in relief",
            json!({"texture": "canvas", "scaling": 100.0, "relief": 4.0, "lightDirection": "top", "invert": false}),
        ),
        r(
            "video.deinterlace",
            "De-Interlace…",
            VIDEO,
            "{eliminate: \"odd\"|\"even\" (\"odd\"; which field lines are taken out), create: \"duplication\"|\"interpolation\" (\"duplication\"; a line taken out becomes a copy of the line above, or the average of the lines above and below)} the field lines are the rows of the document's raster grid (Document Raster Effects Settings › Resolution), numbered from 1 at the top of the page",
            json!({"eliminate": "odd", "create": "duplication"}),
        ),
        r(
            "video.ntscColors",
            "NTSC Colors",
            VIDEO,
            "{} makes colours a television signal can't carry (luma plus chroma past 110 % of white, or luma minus chroma below −20 %) less saturated, keeping their brightness",
            json!({}),
        ),
    ];
    v.extend([
        g(
            "adjust.brightnessContrast",
            "Brightness/Contrast…",
            ADJUST,
            "{brightness: -100..100 (0; bends the tones, black and white stay), contrast: -100..100 (0)} recolours the object's fills, strokes, type, meshes and embedded images (each colour keeps its model)",
            json!({"brightness": 0.0, "contrast": 0.0}),
        ),
        g(
            "adjust.curves",
            "Curves…",
            ADJUST,
            "{points: \"x,y x,y …\" or [[x, y], …] (input, output 0..255; a smooth monotone curve through them, flat past the ends; \"0,0 128,128 255,255\"), channel: \"rgb\"|\"red\"|\"green\"|\"blue\" (\"rgb\")} recolours as Brightness/Contrast does",
            json!({"points": "0,0 128,128 255,255", "channel": "rgb"}),
        ),
        g(
            "adjust.hueSaturation",
            "Hue/Saturation…",
            ADJUST,
            "{hue: deg -180..180 (0), saturation: -100..100 (0), lightness: -100..100 (0), colorize: bool (false; true: every colour takes hue `hue` (0..360) at saturation (100 + saturation) / 2 %)} recolours as Brightness/Contrast does",
            json!({"hue": 0.0, "saturation": 0.0, "lightness": 0.0, "colorize": false}),
        ),
        g(
            "adjust.levels",
            "Levels…",
            ADJUST,
            "{inputBlack: 0..255 (0), inputWhite: 0..255 (255), gamma: 0.1..10 (1; above 1 lightens the midtones), outputBlack: 0..255 (0), outputWhite: 0..255 (255), channel: \"rgb\"|\"red\"|\"green\"|\"blue\" (\"rgb\")} recolours as Brightness/Contrast does",
            json!({"inputBlack": 0.0, "inputWhite": 255.0, "gamma": 1.0, "outputBlack": 0.0, "outputWhite": 255.0, "channel": "rgb"}),
        ),
        g(
            "adjust.shiftToColor",
            "Shift to Color…",
            ADJUST,
            "{color: \"#rrggbb\" (\"#ff8000\"), amount: 0..100 % (50), preserveLightness: bool (true: colours take the target's hue and saturation and keep their lightness; false: they mix with it)} recolours as Brightness/Contrast does",
            json!({"color": "#ff8000", "amount": 50.0, "preserveLightness": true}),
        ),
        g(
            "adjust.temperatureTint",
            "Temperature/Tint…",
            ADJUST,
            "{temperature: -100 (cooler) .. 100 (warmer) (0), tint: -100 (greener) .. 100 (more magenta) (0)} recolours as Brightness/Contrast does",
            json!({"temperature": 0.0, "tint": 0.0}),
        ),
    ]);
    for (suffix, label) in WARP_STYLES {
        let id: &'static str = match suffix {
            "arc" => "warp.arc",
            "arcLower" => "warp.arcLower",
            "arcUpper" => "warp.arcUpper",
            "arch" => "warp.arch",
            "bulge" => "warp.bulge",
            "shellLower" => "warp.shellLower",
            "shellUpper" => "warp.shellUpper",
            "flag" => "warp.flag",
            "wave" => "warp.wave",
            "fish" => "warp.fish",
            "rise" => "warp.rise",
            "fisheye" => "warp.fisheye",
            "inflate" => "warp.inflate",
            "squeeze" => "warp.squeeze",
            _ => "warp.twist",
        };
        v.push(g(id, label, WARP, WARP_DOC, json!({"bend": 50.0, "horizontal": 0.0, "vertical": 0.0, "orientation": "horizontal"})));
    }
    for (id, label, _) in PATHFINDER_EFFECTS {
        v.push(g(id, label, PATHFINDER, "{} (groups and layers: live Pathfinder over the members)", json!({})));
    }
    v.push(g(
        CROP_MARKS,
        "Crop Marks",
        &["Effect"],
        "{style?: \"roman\"|\"japanese\" (default: the japaneseCropMarks preference when applied)} trim marks in [Registration] around the object's bounds, following it",
        json!({}),
    ));
    v
}

/// The catalogue built once, indexed by id (lookups run per effect per rendered frame).
fn catalog_index() -> &'static std::collections::HashMap<&'static str, EffectInfo> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, EffectInfo>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| effect_catalog().into_iter().map(|e| (e.id, e)).collect())
}

/// Catalogue entry for `id` (built-in, or an installed plug-in effect).
pub fn effect_info(id: &str) -> Option<EffectInfo> {
    catalog_index().get(id).cloned().or_else(|| plugin_info(id))
}

/// The catalogue entry of the installed effect plug-in with effect id `id` (`plugin.<id>`).
fn plugin_info(id: &str) -> Option<EffectInfo> {
    use vectorcraft_plugins::registry::intern;
    let p = vectorcraft_plugins::effect::installed(id)?;
    let m = p.manifest();
    let dots = if m.params.is_empty() || m.name.ends_with('…') { "" } else { "…" };
    Some(EffectInfo {
        id: intern(id),
        label: intern(&format!("{}{dots}", m.name)),
        menu: PLUGINS,
        params: intern(&m.params_doc()),
        defaults: Value::Object(m.defaults()),
        raster: false,
        lengths: Lengths::default(),
    })
}

/// The installed effect plug-ins as catalogue entries (Effect › Plug-ins), by plug-in id.
pub fn plugin_effects() -> Vec<EffectInfo> {
    vectorcraft_plugins::registry::list().iter().filter_map(|p| plugin_info(&vectorcraft_plugins::effect::effect_id(p.id()))).collect()
}

/// Dialog defaults for `id` (`None` for unknown effects).
pub fn default_params(id: &str) -> Option<Value> {
    match catalog_index().get(id) {
        Some(e) => Some(e.defaults.clone()),
        None => plugin_info(id).map(|e| e.defaults),
    }
}

/// `params` layered over the defaults of `id` (unknown keys are kept).
pub fn merged_params(id: &str, params: &Value) -> Value {
    let mut out = match catalog_index().get(id).map(|e| &e.defaults) {
        Some(Value::Object(m)) => m.clone(),
        Some(_) => Map::new(),
        None => vectorcraft_plugins::effect::installed(id).map(|p| p.manifest().defaults()).unwrap_or_default(),
    };
    if let Value::Object(p) = params {
        for (k, v) in p {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// A new effect record with default parameters (overridden by `params`).
pub fn new_effect(id: &str, params: &Value) -> Option<Effect> {
    if !catalog_index().contains_key(id) {
        vectorcraft_plugins::effect::installed(id)?;
    }
    Some(Effect { id: id.to_string(), params: merged_params(id, params), visible: true })
}

/// Scale the distance parameters of `e` by `s` (Scale Strokes & Effects; see
/// [`EffectInfo::lengths`]). A distance left at its default is written out scaled; one the
/// effect doesn't have (Outline Stroke's weight following its stroke) stays absent.
pub fn scale_effect(e: &mut Effect, s: f64) {
    let Some(info) = catalog_index().get(e.id.as_str()) else { return };
    let merged = merged_params(&e.id, &e.params);
    let relative = util::flag(&merged, "relative", false);
    for k in info.lengths.keys(relative) {
        let v = util::num(&merged, k, f64::NAN);
        if v.is_finite() {
            if !e.params.is_object() {
                e.params = Value::Object(Map::new());
            }
            e.params[k] = json!(v * s);
        }
    }
}

/// Is `id` a raster (painted) effect?
pub fn is_raster(id: &str) -> bool {
    matches!(id, "stylize.dropShadow" | "stylize.innerGlow" | "stylize.outerGlow" | "stylize.feather" | "blur.gaussian")
        || PIXEL_EFFECTS.contains(&id)
}

/// Does `id` change geometry? (Crop Marks adds art of its own instead, [`crop_marks_art`]; colour
/// adjustments recolour, [`adjust`].) Plug-in effects do, installed or not (a missing plug-in
/// leaves the geometry as it is).
pub fn is_geometry(id: &str) -> bool {
    !is_raster(id)
        && !is_pathfinder(id)
        && !is_adjustment(id)
        && id != CROP_MARKS
        && id != REVOLVE
        && (catalog_index().contains_key(id) || vectorcraft_plugins::effect::plugin_id(id).is_some())
}

/// Any visible geometry effect in the list?
pub fn has_geometry(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible && is_geometry(&e.id))
}

/// Context the geometry effects may need from the object's appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeomContext<'a> {
    /// The stroke `path.outlineStroke` outlines (its weight, caps, joins, dashes, alignment,
    /// profile and arrowheads): the stroke the effect sits on, else the object's top painted
    /// stroke. `None` outlines a plain 1 pt stroke.
    pub stroke: Option<&'a StrokeLayer>,
    /// The object's fill rule (inside and outside alignment).
    pub rule: FillRule,
    /// The seed of the Transform effect's Random: the object's id, so each object varies its own
    /// way and keeps its result on every redraw.
    pub seed: u64,
}

impl<'a> GeomContext<'a> {
    /// The context of `n`'s object-level effects.
    pub fn of(n: &'a Node) -> Self {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::NonZero,
        };
        Self { stroke: n.appearance.stroke().filter(|s| !s.paint.is_none() && s.width > 0.0), rule, seed: n.id.0 }
    }

    /// The context of the effects on `item`, one of the same object's appearance items: a
    /// stroke's own effects outline that stroke.
    pub fn item(self, item: &'a AppearanceItem) -> Self {
        match item {
            AppearanceItem::Stroke(s) => Self { stroke: Some(s), ..self },
            AppearanceItem::Fill(_) => self,
        }
    }
}

/// Evaluate the visible geometry effects of `effects` on `path`, in order. `bounds` is the
/// reference box of the first effect (usually the object's geometric bounds); each later effect
/// uses the bounds of its input. Raster and unknown effects are skipped.
pub fn apply_geometry(effects: &[Effect], path: &PathData, bounds: Rect) -> PathData {
    apply_geometry_with(effects, path, bounds, &GeomContext::default())
}

/// [`apply_geometry`] with an explicit [`GeomContext`].
pub fn apply_geometry_with(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext) -> PathData {
    let mut cur = path.clone();
    let mut b = bounds;
    let mut first = true;
    for e in effects.iter().filter(|e| e.visible && is_geometry(&e.id)) {
        if !first {
            b = cur.bounds().unwrap_or(b);
        }
        first = false;
        if cur.is_empty() {
            break;
        }
        let p = merged_params(&e.id, &e.params);
        cur = apply_one(&e.id, &p, &cur, b, ctx);
    }
    cur
}

/// Convenience wrapper on kurbo paths.
pub fn apply_geometry_bez(effects: &[Effect], path: &BezPath, bounds: Rect, ctx: &GeomContext) -> BezPath {
    apply_geometry_with(effects, &PathData::from_bezpath(path), bounds, ctx).to_bezpath()
}

pub(crate) fn apply_one(id: &str, p: &Value, path: &PathData, b: Rect, ctx: &GeomContext) -> PathData {
    use util::*;
    if let Some(plugin) = vectorcraft_plugins::effect::plugin_id(id) {
        return vectorcraft_plugins::effect::apply(plugin, p, path, b, ctx.rule).unwrap_or_else(|| path.clone());
    }
    match id {
        "distort.freeDistort" => distort::free_distort(path, b, p),
        "distort.puckerBloat" => distort::pucker_bloat(path, b, num(p, "amount", 0.0)),
        "distort.roughen" => distort::roughen(path, b, p),
        "distort.transform" => distort::transform(path, b, p, ctx.seed),
        "distort.tweak" => distort::tweak(path, b, p),
        "distort.twist" => distort::twist(path, b, num(p, "angle", 10.0)),
        "distort.zigZag" => distort::zig_zag(path, b, p),
        "path.offsetPath" => {
            let off = num(p, "offset", 10.0).clamp(-1e5, 1e5);
            if off.abs() < 1e-9 {
                return path.clone();
            }
            vectorcraft_pathops::offset_path(path, off, join(p, "joins"), num(p, "miterLimit", 4.0).clamp(1.0, 500.0))
        }
        "path.outlineStroke" => {
            let fallback;
            let base = match ctx.stroke {
                Some(s) => s,
                None => {
                    fallback = StrokeLayer::new(vectorcraft_doc::color::Paint::None, 1.0);
                    &fallback
                }
            };
            let st = match p.get("width").and_then(Value::as_f64) {
                Some(w) => std::borrow::Cow::Owned(StrokeLayer { width: w, ..base.clone() }),
                None => std::borrow::Cow::Borrowed(base),
            };
            if st.width.is_nan() || st.width <= 0.0 {
                return path.clone();
            }
            stroke::outline_region(path, ctx.rule, &st)
        }
        "convertToShape.rectangle" | "convertToShape.roundedRectangle" | "convertToShape.ellipse" => stylize::convert_to_shape(id, b, p),
        "stylize.roundCorners" => stylize::round_corners(path, num(p, "radius", 10.0)),
        "stylize.scribble" => stylize::scribble(path, b, p),
        _ => match id.strip_prefix("warp.").and_then(WarpStyle::from_id) {
            Some(style) => warp::warp(path, b, style, p),
            None => path.clone(),
        },
    }
}
