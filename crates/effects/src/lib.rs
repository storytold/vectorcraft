//! VectorCraft live effects (Illustrator's Effect menu).
//!
//! Effects live in appearance stacks as [`Effect`] `{id, params, visible}` records. This crate
//! interprets them:
//!
//! - **Geometry effects** (Distort & Transform, Path, Convert to Shape, Round Corners, Scribble,
//!   Warp) rewrite a path: [`apply_geometry`] evaluates them in stack order.
//! - **Raster effects** (Drop Shadow, Inner/Outer Glow, Feather, Gaussian Blur, Rasterize, and the
//!   Photoshop-style filters of [`pixel`]: Radial Blur, Smart Blur, Color Halftone, Crystallize, Mezzotint, Pointillize, Halftone Pattern,
//!   Unsharp Mask and Glowing Edges) are described by
//!   [`raster_effects`] and painted by the renderer; [`outset`] says how far they reach beyond
//!   the geometry.
//! - **Stroke geometry** ([`stroke`]): arrowheads, dash patterns and width profiles, shared by the
//!   renderer, the exporters and Outline Stroke.
//! - [`effect_catalog`] lists every effect with its menu path, parameter documentation and
//!   default values. Missing parameters always fall back to those defaults.
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
mod lighting;
mod live;
mod marks;
pub mod pixel;
mod raster;
mod reshape;
mod revolve;
mod solids;
pub mod stroke;
mod stylize;
mod util;
mod warp;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_scale;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

pub use adjust::{
    ADJUSTMENTS, CHANNELS, ColorMap, ImageHook, adjust, adjust_in_document, color_map, curve_at, curve_points, has_adjustment, is_adjustment,
    is_channel,
};
pub use bake::{StrokeArt, bake_appearance, bake_document, expand_art, expand_leaf, fresh_ids, needs_bake};
pub use clip::clip_outline;
pub use group::{
    OutlineHook, PATHFINDER_EFFECTS, evaluate_container, has_container_appearance, has_pathfinder, is_pathfinder, member_shapes, paints,
    pathfinder_children,
};
pub use live::{expand_live, expand_live_deep, expanded_live_group, text_outliner};
pub use marks::{CROP_MARKS, crop_marks_art, has_crop_marks};
pub use pixel::{PIXEL_EFFECTS, PixelFx, PixelSpace};
pub use raster::{RASTERIZE, RasterFx, Rasterize, outset, raster_effects};
pub use reshape::{expand_outlined, has_gradient_copies, has_item_geometry, image_paint, needs_outline, outline_art, outline_text, reshape};
pub use revolve::{REVOLVE, has_revolve, revolve_art, revolve_options, validate_revolve};
pub use solids::{
    EXTRUDE, INFLATE, ROTATE, ROTATION_PRESETS, SOLIDS, extrude_options, has_solid, inflate_options, is_three_d, solid_art, validate_solid,
    view_options,
};
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
        "threeD.extrude" => always(&["depth", "bevelWidth", "bevelHeight"]),
        "threeD.inflate" => always(&["depth"]),
        "path.outlineStroke" => always(&["width"]),
        "stylize.roundCorners" | "stylize.feather" | "blur.gaussian" | "blur.smart" | "sharpen.unsharpMask" => always(&["radius"]),
        "stylize.scribble" => always(&["overlap", "overlapVariation", "strokeWidth", "spacing", "spacingVariation"]),
        "stylize.dropShadow" => always(&["x", "y", "blur"]),
        "stylize.innerGlow" | "stylize.outerGlow" => always(&["blur"]),
        "stylize.glowingEdges" => always(&["edgeWidth", "smoothness"]),
        "brushStrokes.accentedEdges" => always(&["edgeWidth"]),
        "brushStrokes.angledStrokes" | "brushStrokes.crosshatch" | "brushStrokes.inkOutlines" => always(&["strokeLength"]),
        "brushStrokes.spatter" => always(&["sprayRadius"]),
        "brushStrokes.sprayedStrokes" => always(&["strokeLength", "sprayRadius"]),
        "brushStrokes.sumiE" => always(&["strokeWidth"]),
        "pixelate.colorHalftone" => always(&["maxRadius"]),
        "pixelate.crystallize" | "pixelate.pointillize" => always(&["cellSize"]),
        "texture.craquelure" => always(&["crackSpacing"]),
        "texture.mosaicTiles" => always(&["tileSize"]),
        "texture.patchwork" => always(&["squareSize"]),
        "texture.stainedGlass" => always(&["cellSize"]),
        "sketch.graphicPen" => always(&["strokeLength"]),
        "sketch.halftonePattern" => always(&["size"]),
        "sketch.notePaper" => always(&["relief"]),
        RASTERIZE => always(&["addAround"]),
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
const ARTISTIC: &[&str] = &["Effect", "Artistic"];
const BRUSH_STROKES: &[&str] = &["Effect", "Brush Strokes"];
const DISTORT: &[&str] = &["Effect", "Distort"];
const SHARPEN: &[&str] = &["Effect", "Sharpen"];
const SKETCH: &[&str] = &["Effect", "Sketch"];
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
            "{angle: degrees (0..360, 360), offset: pt (0..100000, 0), edge: left|right, rotationX: degrees (0), rotationY: degrees (0), rotationZ: degrees (0), perspectiveAngle: degrees (0..160, 0; 0 is none, wider brings the eye nearer), perspective: % (0..100; the earlier form, read as the perspectiveAngle that shows the same view), segments: integer (8..128, 64), shade: bool (true), lightColor: #rrggbb (white, #ffffff; legacy directional light color), lights: null or array of 1..10 objects (color: #rrggbb, intensity: % 0..100, rotation: degrees -180..180, height: degrees 0..90, softness: % 0..100, behind: bool; overrides the single-light fields; ambient is shared), lightRotation: degrees (-180..180, 125.26; round the object on the page, 0 on its right, counterclockwise), lightHeight: degrees (0..90, 30; 0 beside the object, 90 straight in front), lightBehind: bool (false; true lights it from behind), lightIntensity: % (0..100, 80), lightSoftness: % (0..100, 0; how far round the object the light reaches), ambient: % (0..200, 25; Ambient Light), lightAzimuth: degrees (the earlier form: with lightElevation, read as the lightRotation, lightHeight and lightBehind of the same light), lightElevation: degrees (the earlier form, with lightAzimuth), cap: bool (true, close circular ends of open profiles; partial-sweep cut sides stay open), expandVisibleOnly: bool (true, trim covered opaque solid surfaces only on Expand Appearance)}. Shaded surfaces use smooth vector gradients automatically; intersecting profiles use approximate painter visibility.",
            json!({"angle":360.0,"offset":0.0,"edge":"left","rotationX":0.0,"rotationY":0.0,"rotationZ":0.0,"perspectiveAngle":0.0,"segments":64,"shade":true,"lightColor":"#ffffff","lights":null,"lightRotation":125.26438968275465,"lightHeight":30.0,"lightBehind":false,"lightIntensity":80.0,"lightSoftness":0.0,"ambient":25.0,"cap":true,"expandVisibleOnly":true}),
        ),
        g(
            EXTRUDE,
            "Extrude & Bevel…",
            &["Effect", "3D and Materials"],
            "{depth: pt (0..2000, 50), twist: degrees (0..360, 0; the back turned against the front), taper: % (1..100, 100; the back's size), cap: bool (true: solid; false: hollow), bevel: none|classic|rounded|cove|step|ogee (none), bevelWidth: pt (0..1000, 4), bevelHeight: pt (0..1000, 4), bevelRepeat: integer (1..32, 1), bevelSpace: % (0..90, 0; flat between repeats), bevelInside: bool (false: added around the shape; true: carved into it), bevelBothSides: bool (false), rotationX: degrees (-180..180, -21.88), rotationY: degrees (-180..180, -23.4), rotationZ: degrees (-180..180, 9.06; the defaults are the Off-Axis Front position), perspectiveAngle: degrees (0..160, 0; 0 is none, wider brings the eye nearer), perspective: % (0..100; the earlier form, read as the perspectiveAngle that shows the same view), shade: bool (true), lightColor: #rrggbb (white, #ffffff; legacy directional light color), lights: null or array of 1..10 objects (color: #rrggbb, intensity: % 0..100, rotation: degrees -180..180, height: degrees 0..90, softness: % 0..100, behind: bool; overrides the single-light fields; ambient is shared), lightRotation: degrees (-180..180, 125.26; round the object on the page, 0 on its right, counterclockwise), lightHeight: degrees (0..90, 30; 0 beside the object, 90 straight in front), lightBehind: bool (false; true lights it from behind), lightIntensity: % (0..100, 80), lightSoftness: % (0..100, 0; how far round the object the light reaches), ambient: % (0..200, 25; Ambient Light), lightAzimuth: degrees (the earlier form: with lightElevation, read as the lightRotation, lightHeight and lightBehind of the same light), lightElevation: degrees (the earlier form, with lightAzimuth)}. The front face is the art, extruded backwards about the centre of its bounds; painted with the top fill (else the top stroke).",
            json!({"depth":50.0,"twist":0.0,"taper":100.0,"cap":true,"bevel":"none","bevelWidth":4.0,"bevelHeight":4.0,"bevelRepeat":1,"bevelSpace":0.0,"bevelInside":false,"bevelBothSides":false,"rotationX":-21.88,"rotationY":-23.4,"rotationZ":9.06,"perspectiveAngle":0.0,"shade":true,"lightColor":"#ffffff","lights":null,"lightRotation":125.26438968275465,"lightHeight":30.0,"lightBehind":false,"lightIntensity":80.0,"lightSoftness":0.0,"ambient":25.0}),
        ),
        g(
            INFLATE,
            "Inflate…",
            &["Effect", "3D and Materials"],
            "{depth: pt (0..2000, 0; flat depth between front and back), twist: degrees (0..360, 0; the back turned against the front), taper: % (1..100, 100; the back's size), cap: bool (true: solid; false: no back, a shell open behind), volume: % (0..100, 50; 50 rounds the edge like a half sphere), bothSides: bool (false: flat back), rotationX: degrees (-180..180, -21.88), rotationY: degrees (-180..180, -23.4), rotationZ: degrees (-180..180, 9.06; the defaults are the Off-Axis Front position), perspectiveAngle: degrees (0..160, 0; 0 is none, wider brings the eye nearer), perspective: % (0..100; the earlier form, read as the perspectiveAngle that shows the same view), shade: bool (true), lightColor: #rrggbb (white, #ffffff; legacy directional light color), lights: null or array of 1..10 objects (color: #rrggbb, intensity: % 0..100, rotation: degrees -180..180, height: degrees 0..90, softness: % 0..100, behind: bool; overrides the single-light fields; ambient is shared), lightRotation: degrees (-180..180, 125.26; round the object on the page, 0 on its right, counterclockwise), lightHeight: degrees (0..90, 30; 0 beside the object, 90 straight in front), lightBehind: bool (false; true lights it from behind), lightIntensity: % (0..100, 80), lightSoftness: % (0..100, 0; how far round the object the light reaches), ambient: % (0..200, 25; Ambient Light), lightAzimuth: degrees (the earlier form: with lightElevation, read as the lightRotation, lightHeight and lightBehind of the same light), lightElevation: degrees (the earlier form, with lightAzimuth)}. Painted with the top fill (else the top stroke).",
            json!({"depth":0.0,"twist":0.0,"taper":100.0,"cap":true,"volume":50.0,"bothSides":false,"rotationX":-21.88,"rotationY":-23.4,"rotationZ":9.06,"perspectiveAngle":0.0,"shade":true,"lightColor":"#ffffff","lights":null,"lightRotation":125.26438968275465,"lightHeight":30.0,"lightBehind":false,"lightIntensity":80.0,"lightSoftness":0.0,"ambient":25.0}),
        ),
        g(
            ROTATE,
            "Rotate…",
            &["Effect", "3D and Materials"],
            "{rotationX: degrees (-180..180, -21.88), rotationY: degrees (-180..180, -23.4), rotationZ: degrees (-180..180, 9.06; the defaults are the Off-Axis Front position), perspectiveAngle: degrees (0..160, 0; 0 is none, wider brings the eye nearer), perspective: % (0..100; the earlier form, read as the perspectiveAngle that shows the same view), shade: bool (true), lightColor: #rrggbb (white, #ffffff; legacy directional light color), lights: null or array of 1..10 objects (color: #rrggbb, intensity: % 0..100, rotation: degrees -180..180, height: degrees 0..90, softness: % 0..100, behind: bool; overrides the single-light fields; ambient is shared), lightRotation: degrees (-180..180, 125.26; round the object on the page, 0 on its right, counterclockwise), lightHeight: degrees (0..90, 30; 0 beside the object, 90 straight in front), lightBehind: bool (false; true lights it from behind), lightIntensity: % (0..100, 80), lightSoftness: % (0..100, 0; how far round the object the light reaches), ambient: % (0..200, 25; Ambient Light), lightAzimuth: degrees (the earlier form: with lightElevation, read as the lightRotation, lightHeight and lightBehind of the same light), lightElevation: degrees (the earlier form, with lightAzimuth)}. Turns the flat art in 3D about the centre of its bounds; every fill and stroke keeps its paint, shaded as one plane.",
            json!({"rotationX":-21.88,"rotationY":-23.4,"rotationZ":9.06,"perspectiveAngle":0.0,"shade":true,"lightColor":"#ffffff","lights":null,"lightRotation":125.26438968275465,"lightHeight":30.0,"lightBehind":false,"lightIntensity":80.0,"lightSoftness":0.0,"ambient":25.0}),
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
            "{corners: [[x,y]×4] (new positions of the bounding-box corners TL, TR, BR, BL in unit box coordinates; identity [[0,0],[1,0],[1,1],[0,1]])}",
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
            "path.outlineObject",
            "Outline Object",
            PATH,
            "{} type is drawn, exported and expanded as its glyph outlines and stays editable, and the effects after it reshape those outlines; paths and images do not change",
            json!({}),
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
            "{angle: deg (30; stored above -180 and up to 180: 270 becomes -90, -180 becomes 180), overlap: pt (0; Path Overlap: a positive value extends each line beyond the path, a negative one ends it inside), overlapVariation: pt (0..100, 0.5; how far each line's ends move along it), strokeWidth: pt (3), curviness: % (5), curvinessVariation: % (0..100, 0; how far each turn's curviness moves from curviness), spacing: pt (5), spacingVariation: pt (0..100, 0.5; how far each line's ends move across it), variation: pt (the earlier form, read as overlapVariation and spacingVariation unless those are given), seed: int (0)} (simplified: a single hatching scribble outlined to a filled shape)",
            json!({"angle": 30.0, "overlap": 0.0, "overlapVariation": 0.5, "strokeWidth": 3.0, "curviness": 5.0, "curvinessVariation": 0.0, "spacing": 5.0, "spacingVariation": 0.5, "seed": 0}),
        ),
        r(
            "stylize.dropShadow",
            "Drop Shadow…",
            STYLIZE,
            "{mode: blend mode (\"multiply\"), opacity: % (75), x: pt (7), y: pt (7), blur: pt (5), shade: \"color\"|\"darkness\" (\"color\"), color: \"#rrggbb\" (\"#000000\"), darkness: % (100)} (shade \"darkness\": the shadow is the object's own colors with `darkness` percent of black added instead of `color`: in a CMYK document their cyan, magenta and yellow stay and their black K becomes K + darkness × (1 − K); in an RGB document each channel is multiplied by 1 − darkness)",
            json!({"mode": "multiply", "opacity": 75.0, "x": 7.0, "y": 7.0, "blur": 5.0, "shade": "color", "color": "#000000", "darkness": 100.0}),
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
            "artistic.cutout",
            "Cutout…",
            ARTISTIC,
            "{levels: 2..8 (4; steps per colour channel: more levels, more flat colours), edgeSimplicity: 0..10 (4; simpler, smoother edges), edgeFidelity: 1..3 (2; keeps more of the edges' detail)} redraws the object as flat pieces of coloured paper, each piece painted in the average colour that falls into it, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"levels": 4.0, "edgeSimplicity": 4.0, "edgeFidelity": 2.0}),
        ),
        r(
            "artistic.filmGrain",
            "Film Grain…",
            ARTISTIC,
            "{grain: 0..20 (4; monochrome noise in shadows and midtones), highlightArea: 0..20 (0; widens the bright highlight area), intensity: 0..10 (10; highlight strength and quieter highlight grain)}. Smooth document-space grain follows translations; minification uses approximate footprint filtering. Coverage and pure white are preserved; black takes light speckles, and saturated colour channels may clip. Grain 0 with either highlightArea 0 or intensity 0 leaves the art unchanged. Original visual approximation; chosen VectorCraft defaults.",
            json!({"grain": 4.0, "highlightArea": 0.0, "intensity": 10.0}),
        ),
        r(
            "artistic.neonGlow",
            "Neon Glow…",
            ARTISTIC,
            "{glowSize: -24..24 (5; above 0 the light areas glow, below 0 the dark ones; farther from 0 spreads the glow wider), glowBrightness: 0..50 (15; a stronger glow over dimmer greys), color: \"#rrggbb\" (\"#4da6ff\"; the glow's colour)} redraws the object in greys lit by a coloured glow, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"glowSize": 5.0, "glowBrightness": 15.0, "color": "#4da6ff"}),
        ),
        r(
            "artistic.plasticWrap",
            "Plastic Wrap…",
            ARTISTIC,
            "{highlightStrength: 0..20 (20; brighter specular highlights), detail: 0..15 (12; finer wrinkles in document space), smoothness: 1..15 (15; softer folds)} coats the object in shiny plastic, keeping its coverage. highlightStrength 0 leaves the art unchanged. Original visual approximation; chosen VectorCraft defaults.",
            json!({"highlightStrength": 20.0, "detail": 12.0, "smoothness": 15.0}),
        ),
        r(
            "artistic.posterEdges",
            "Poster Edges…",
            ARTISTIC,
            "{edgeThickness: 0..10 (2; wider lines), edgeIntensity: 0..10 (1; darker lines, on fainter edges too), posterization: 0..6 (2; posterization + 2 levels per colour channel)} reduces the object's colours and draws black lines on its edges, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"edgeThickness": 2.0, "edgeIntensity": 1.0, "posterization": 2.0}),
        ),
        r(
            "artistic.smudgeStick",
            "Smudge Stick…",
            ARTISTIC,
            "{strokeLength: 0..10 (2; length of the short diagonal strokes), highlightArea: 0..20 (12; widens the area counted as light), intensity: 0..10 (10; how far the light areas are lightened)} smudges the dark areas along short diagonal strokes; the lighter areas become brighter and lose detail. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"strokeLength": 2.0, "highlightArea": 12.0, "intensity": 10.0}),
        ),
        r(
            "artistic.sponge",
            "Sponge…",
            ARTISTIC,
            "{brushSize: 0..10 (2; size of the sponge's pores), definition: 0..25 (12; how strongly lighter and darker blobs contrast), smoothness: 1..15 (5; softer blob edges)} paints the object as if dabbed with a sponge, in highly textured areas of contrasting colour. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"brushSize": 2.0, "definition": 12.0, "smoothness": 5.0}),
        ),
        r(
            "artistic.watercolor",
            "Watercolor…",
            ARTISTIC,
            "{brushDetail: 1..14 (9; the larger, the more detail stays), shadowIntensity: 0..10 (1; deepens the shadows), texture: 1..3 (1; paper grain, whole numbers)} paints the object in a watercolor style: detail simplified, colour saturated and darker where tones change at edges. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"brushDetail": 9.0, "shadowIntensity": 1.0, "texture": 1.0}),
        ),
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
            "{radius: pt 0.1..100 (3), threshold: levels 0.1..100 (25; only colours closer than this blur together, so edges stay sharp), quality: \"low\"|\"medium\"|\"high\" (\"medium\"; 5, 7 or 9 samples across), blurMode: \"normal\"|\"edgeOnly\"|\"overlayEdge\" (\"normal\"; edgeOnly: the edges in white on black, overlayEdge: the edges in white over the blur)}. A pixel is on an edge when one of the pixels the search reads within the radius (all of them for a small radius, a grid of samples for a wide one) or one of its four nearest pixels differs from it by more than the threshold in a channel and its channels (alpha included) add up to at least that pixel's, so a step gives a line about as wide as the radius, and at least one pixel wide, on the side with the larger sum; the white and the black keep each pixel's coverage",
            json!({"radius": 3.0, "threshold": 25.0, "quality": "medium", "blurMode": "normal"}),
        ),
        r(
            "brushStrokes.accentedEdges",
            "Accented Edges…",
            BRUSH_STROKES,
            "{edgeWidth: pt 1..14 (2), edgeBrightness: 0..50 (38; high: white chalk, low: black ink), smoothness: 1..15 (5; softens the object first and leaves its faint edges out)} accentuates the object's edges",
            json!({"edgeWidth": 2.0, "edgeBrightness": 38.0, "smoothness": 5.0}),
        ),
        r(
            "brushStrokes.angledStrokes",
            "Angled Strokes…",
            BRUSH_STROKES,
            "{directionBalance: 0..100 (50; 0: every stroke a left diagonal, 100: every stroke a right diagonal, between: light areas right, dark areas left), strokeLength: pt 3..50 (15), sharpness: 0..10 (3)} repaints the object with diagonal strokes, the light areas' going the opposite way to the dark areas'",
            json!({"directionBalance": 50.0, "strokeLength": 15.0, "sharpness": 3.0}),
        ),
        r(
            "brushStrokes.crosshatch",
            "Crosshatch…",
            BRUSH_STROKES,
            "{strokeLength: pt 3..50 (9), sharpness: 0..20 (6), strength: 1..3 (1; passes of hatching)} keeps the object's detail under pencil hatching along both diagonals and roughens its edges",
            json!({"strokeLength": 9.0, "sharpness": 6.0, "strength": 1.0}),
        ),
        r(
            "brushStrokes.darkStrokes",
            "Dark Strokes…",
            BRUSH_STROKES,
            "{balance: 0..10 (5; the higher, the more of the object counts as dark), blackIntensity: 0..10 (6), whiteIntensity: 0..10 (2)} paints the dark areas closer to black with short strokes and the light areas with long white strokes",
            json!({"balance": 5.0, "blackIntensity": 6.0, "whiteIntensity": 2.0}),
        ),
        r(
            "brushStrokes.inkOutlines",
            "Ink Outlines…",
            BRUSH_STROKES,
            "{strokeLength: pt 1..50 (4), darkIntensity: 0..50 (20), lightIntensity: 0..50 (10)} redraws the object in pen and ink: fine outlines over its edges and ink strokes in its shadows",
            json!({"strokeLength": 4.0, "darkIntensity": 20.0, "lightIntensity": 10.0}),
        ),
        r(
            "brushStrokes.spatter",
            "Spatter…",
            BRUSH_STROKES,
            "{sprayRadius: pt 0..25 (10; colours scattered up to 0.4 × this), smoothness: 1..15 (5; larger, smoother grains)} renders the object as if sprayed with a spatter airbrush",
            json!({"sprayRadius": 10.0, "smoothness": 5.0}),
        ),
        r(
            "brushStrokes.sprayedStrokes",
            "Sprayed Strokes…",
            BRUSH_STROKES,
            "{strokeLength: pt 0..20 (12), sprayRadius: pt 0..25 (7; strokes scattered up to 0.3 × this), strokeDirection: \"rightDiagonal\"|\"horizontal\"|\"leftDiagonal\"|\"vertical\" (\"rightDiagonal\")} repaints the object with sprayed, angled strokes of its main colours",
            json!({"strokeLength": 12.0, "sprayRadius": 7.0, "strokeDirection": "rightDiagonal"}),
        ),
        r(
            "brushStrokes.sumiE",
            "Sumi-e…",
            BRUSH_STROKES,
            "{strokeWidth: pt 3..15 (10), strokePressure: 0..15 (2; more ink), contrast: 0..40 (16)} paints the object in the Japanese style, as if with a wet brush full of black ink on rice paper: soft, blurred edges and rich blacks, the colours keeping their hue and the object its shape",
            json!({"strokeWidth": 10.0, "strokePressure": 2.0, "contrast": 16.0}),
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
            "{distortion: 0..20 (5), smoothness: 1..15 (3), texture: \"blocks\"|\"canvas\"|\"frosted\"|\"tinyLens\"|\"loaded\" (\"frosted\"; \"loaded\" draws the image in textureImage and draws Frosted when there is none; the others are surfaces made in code), textureImage?: {path} | {name?, dataBase64} | string (Load Texture…: an image file that File › Place reads, within the renderer's pixel allowance, stored as its gray levels in a grayscale PNG in base64; a file sets texture to \"loaded\" unless texture is given, and the stored string, as effect.list gives it, is kept as it is), scaling: % 50..200 (100), invert: bool (false; turns the surface's heights over)} makes the object look as if seen through glass. A loaded texture lies one image pixel per point at 100 %, centered on the object's center and repeated; each pixel's luma is the surface's height, from 0 for black to 1 pt for white, and a transparent pixel counts as middle gray",
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
            "sketch.basRelief",
            "Bas Relief…",
            SKETCH,
            "{detail: 1..15 (13; the larger, the finer the features that stay), smoothness: 1..15 (3; rounds the carving), lightDirection: \"bottom\"|\"bottomLeft\"|\"left\"|\"topLeft\"|\"top\"|\"topRight\"|\"right\"|\"bottomRight\" (\"bottom\"; where the light comes from)} carves the object in low relief in black and white: surfaces facing the light turn white, those turned away black, flat areas grey. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"detail": 13.0, "smoothness": 3.0, "lightDirection": "bottom"}),
        ),
        r(
            "sketch.chalkCharcoal",
            "Chalk & Charcoal…",
            SKETCH,
            "{chalkArea: 0..20 (6; chalk in light areas), charcoalArea: 0..20 (6; charcoal in dark areas), strokePressure: 0..5 (2)} redraws the object with chalk highlights and charcoal strokes on white paper, keeping its coverage. Original visual approximation from public documentation; slider ranges and defaults are VectorCraft choices, not verified proprietary values.",
            json!({"chalkArea": 6.0, "charcoalArea": 6.0, "strokePressure": 2.0}),
        ),
        r(
            "sketch.charcoal",
            "Charcoal…",
            SKETCH,
            "{thickness: 1..7 (1; width of the bold edges and of the strokes' spacing), detail: 0..5 (1; the larger, the finer the edges), lightDarkBalance: 0..100 (50; how far into the lights the charcoal reaches)} redraws the object in black charcoal on white paper: bold edges and diagonal strokes over the midtones. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"thickness": 1.0, "detail": 1.0, "lightDarkBalance": 50.0}),
        ),
        r(
            "sketch.chrome",
            "Chrome…",
            SKETCH,
            "{detail: 0..10 (4; finer surface texture), smoothness: 0..10 (4; softer relief before the highlights and shadows)} treats the object as polished chrome lit from the top left: bright highlights on high points and dark shadows in low ones, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"detail": 4.0, "smoothness": 4.0}),
        ),
        r(
            "sketch.graphicPen",
            "Graphic Pen…",
            SKETCH,
            "{strokeLength: pt 1..15 (5; the longest ink stroke), lightDarkBalance: 0..100 (50; more black when higher), strokeDirection: \"rightDiagonal\"|\"horizontal\"|\"leftDiagonal\"|\"vertical\" (\"rightDiagonal\")} redraws the object in fine linear black ink on white paper, with strokes denser in darker areas. Control names and ranges follow the public documentation; the defaults are unverified.",
            json!({"strokeLength": 5.0, "lightDarkBalance": 50.0, "strokeDirection": "rightDiagonal"}),
        ),
        r(
            "sketch.halftonePattern",
            "Halftone Pattern…",
            SKETCH,
            "{size: pt 1..12 (1; how far apart the screen's dots, rings or lines are), contrast: 0..50 (5; 50 draws pure black and white), patternType: \"circle\"|\"dot\"|\"line\" (\"dot\"; rings round the object's centre, dots on a square grid, horizontal lines)} redraws the object in black ink on white paper through a halftone screen whose ink covers as much as the colour is dark, keeping its coverage. Original visual approximation from the public Sketch effects reference; slider ranges follow the documented control names, numeric defaults are chosen for VectorCraft and are not verified against proprietary defaults (the public help pages give no numbers).",
            json!({"size": 1.0, "contrast": 5.0, "patternType": "dot"}),
        ),
        r(
            "sketch.notePaper",
            "Note Paper…",
            SKETCH,
            "{imageBalance: 0..50 (25; lower tones darken the emboss, higher lighten it), graininess: 0..10 (0; fibrous paper texture), relief: 0..50 (4; emboss strength in points)} embosses the object's tones on warm paper, keeping its coverage. Original visual approximation from the public Sketch effects reference; slider ranges follow the documented control names, numeric defaults are chosen for VectorCraft and are not verified against proprietary defaults (the public help pages give no numbers).",
            json!({"imageBalance": 25.0, "graininess": 0.0, "relief": 4.0}),
        ),
        r(
            "sketch.photocopy",
            "Photocopy…",
            SKETCH,
            "{detail: 1..24 (7; the larger, the finer the features that copy), darkness: 1..50 (25; the larger, the more of the dark copies)} copies the object in black on white: large dark areas copy only around their edges and midtones go to black or white. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"detail": 7.0, "darkness": 25.0}),
        ),
        r(
            "sketch.plaster",
            "Plaster…",
            SKETCH,
            "{imageBalance: 1..50 (20; higher values sink more of the image), smoothness: 1..15 (8; higher values soften the relief), lightDirection: \"bottom\"|\"bottomLeft\"|\"left\"|\"topLeft\"|\"top\"|\"topRight\"|\"right\"|\"bottomRight\" (\"top\")} molds the object in relief plaster and lights it from that direction; dark areas are raised and light areas recessed, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"imageBalance": 20.0, "smoothness": 8.0, "lightDirection": "top"}),
        ),
        r(
            "sketch.reticulation",
            "Reticulation…",
            SKETCH,
            "{density: 0..50 (12; finer grain at higher values), foregroundLevel: 0..50 (6; pulls shadows toward black ink), backgroundLevel: 0..50 (6; pulls highlights toward white paper)} redraws the object in black ink on white paper through a high-contrast film-grain screen, keeping its coverage. Original visual approximation; chosen VectorCraft defaults.",
            json!({"density": 12.0, "foregroundLevel": 6.0, "backgroundLevel": 6.0}),
        ),
        r(
            "sketch.stamp",
            "Stamp…",
            SKETCH,
            "{lightDarkBalance: 0..50 (25; the larger, the more of the object prints black), smoothness: 1..50 (5; rounds the shapes)} simplifies the object to solid black and white, as a rubber stamp would print it. Coverage is kept. Original visual approximation; chosen VectorCraft defaults.",
            json!({"lightDarkBalance": 25.0, "smoothness": 5.0}),
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
    v.push(r(
        RASTERIZE,
        "Rasterize…",
        &["Effect"],
        "{colorModel: \"document\" (the document's mode, RGB or CMYK)|\"grayscale\"|\"bitmap\" (\"document\"), useDocumentResolution: bool (true: Document Raster Effects Settings › Resolution), resolution: ppi 1..2400 (72; with useDocumentResolution false), background: \"white\"|\"transparent\" (\"transparent\"), antiAlias: \"none\"|\"art\"|\"type\" (\"art\"; none: hard edges), clippingMask: bool (false; the image clipped to the art's outline), addAround: pt 0..1000 (0; room around the art)} draws the object as an image of square pixels over its bounds and the room around them, leaving its paths, type and other art as they are; when it is the object's only raster effect, Expand Appearance makes the image Object › Rasterize makes",
        json!({"colorModel": "document", "useDocumentResolution": true, "resolution": 72.0, "background": "transparent", "antiAlias": "art", "clippingMask": false, "addAround": 0.0}),
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

/// `params` layered over the defaults of `id` (unknown keys are kept), in their current form (see
/// [`upgrade_params`]). The 3D effects' earlier `perspective` is read as `perspectiveAngle`, and
/// Scribble's earlier `variation` is read as `overlapVariation` and `spacingVariation`.
pub fn merged_params(id: &str, params: &Value) -> Value {
    let mut out = match catalog_index().get(id).map(|e| &e.defaults) {
        Some(Value::Object(m)) => m.clone(),
        Some(_) => Map::new(),
        None => vectorcraft_plugins::effect::installed(id).map(|p| p.manifest().defaults()).unwrap_or_default(),
    };
    // Upgraded before the defaults are under it: a default angle is no angle given.
    if let Value::Object(p) = upgrade_params(id, params) {
        out.extend(p);
    }
    Value::Object(out)
}

/// `params` for effect `id` in their current form. The 3D effects took Perspective as a 0–100 %
/// strength, `perspective`, before the documented 0°–160° angle, `perspectiveAngle`, and the light
/// as `lightAzimuth` and `lightElevation` before the documented Rotation and Height
/// (`lightRotation`, `lightHeight`, `lightBehind`). Scribble took one `variation` before its
/// separate `overlapVariation` and `spacingVariation`. Earlier values (from a file or a script of
/// then) become the current ones that show exactly the same, unless those are given too, which win.
/// Scribble's angle is also stored above -180° and up to 180° (270° becomes -90°).
pub fn upgrade_params(id: &str, params: &Value) -> Value {
    match params {
        Value::Object(m) => {
            let mut m = m.clone();
            upgrade_perspective(id, &mut m);
            upgrade_light(id, &mut m);
            upgrade_scribble(id, &mut m);
            Value::Object(m)
        }
        other => other.clone(),
    }
}

fn upgrade_light(id: &str, m: &mut Map<String, Value>) {
    if !is_three_d(id) {
        return;
    }
    let (azimuth, elevation) = (m.remove("lightAzimuth"), m.remove("lightElevation"));
    if azimuth.is_none() && elevation.is_none() || m.contains_key("lightRotation") || m.contains_key("lightHeight") {
        return;
    }
    let degrees = |v: Option<Value>, default: f64| v.and_then(|v| v.as_f64()).unwrap_or(default);
    let (rotation, height, behind) = vectorcraft_three_d::light_from_azimuth_elevation(degrees(azimuth, -45.0), degrees(elevation, 45.0));
    m.insert("lightRotation".into(), json!(rotation));
    m.insert("lightHeight".into(), json!(height));
    m.insert("lightBehind".into(), json!(behind));
}

fn upgrade_perspective(id: &str, m: &mut Map<String, Value>) {
    if !is_three_d(id) {
        return;
    }
    let Some(strength) = m.remove("perspective") else { return };
    if !m.contains_key("perspectiveAngle") {
        let angle = vectorcraft_three_d::perspective_from_strength(strength.as_f64().unwrap_or(0.0) / 100.0);
        m.insert("perspectiveAngle".into(), json!(angle));
    }
}

fn upgrade_scribble(id: &str, m: &mut Map<String, Value>) {
    if id != "stylize.scribble" {
        return;
    }
    if let Some(variation) = m.remove("variation") {
        for key in ["overlapVariation", "spacingVariation"] {
            m.entry(key).or_insert_with(|| variation.clone());
        }
    }
    if let Some(angle) = m.get("angle").and_then(Value::as_f64).filter(|a| a.is_finite() && (*a <= -180.0 || *a > 180.0)) {
        let wrapped = angle.rem_euclid(360.0);
        m.insert("angle".into(), json!(if wrapped > 180.0 { wrapped - 360.0 } else { wrapped }));
    }
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
    matches!(id, "stylize.dropShadow" | "stylize.innerGlow" | "stylize.outerGlow" | "stylize.feather" | "blur.gaussian" | RASTERIZE)
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
        && !is_three_d(id)
        && (catalog_index().contains_key(id) || vectorcraft_plugins::effect::plugin_id(id).is_some())
}

/// Any visible geometry effect in the list?
pub fn has_geometry(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible && is_geometry(&e.id))
}

/// Does `n` have a visible plug-in effect, its own or a fill's or stroke's? Such an effect draws
/// what the plug-in installed under its id draws ([`plugin_revision`]).
pub fn has_plugin_effect(n: &Node) -> bool {
    let plugin = |e: &Effect| e.visible && vectorcraft_plugins::effect::plugin_id(&e.id).is_some();
    n.appearance.effects.iter().any(plugin) || n.appearance.items.iter().any(|i| i.effects().iter().any(plugin))
}

/// The plug-in registry's revision, which changes whenever a plug-in is installed or removed.
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision()
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
    /// The object's appearance, which live-effect plug-ins are given read only (their fills and
    /// strokes).
    pub appearance: Option<&'a Appearance>,
    /// The index in `appearance.items` of the fill or stroke whose own effects these are; `None`:
    /// the object's effects.
    pub item: Option<usize>,
}

impl<'a> GeomContext<'a> {
    /// The context of `n`'s object-level effects.
    pub fn of(n: &'a Node) -> Self {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::NonZero,
        };
        let stroke = n.appearance.stroke().filter(|s| !s.paint.is_none() && s.width > 0.0);
        Self { stroke, rule, seed: n.id.0, appearance: Some(&n.appearance), item: None }
    }

    /// The context of the effects on `item`, one of the same object's appearance items: a
    /// stroke's own effects outline that stroke, and a plug-in is told which item it is on.
    pub fn item(self, item: &'a AppearanceItem) -> Self {
        let at = self.appearance.and_then(|a| a.items.iter().position(|i| std::ptr::eq(i, item)));
        match item {
            AppearanceItem::Stroke(s) => Self { stroke: Some(s), item: at, ..self },
            AppearanceItem::Fill(_) => Self { item: at, ..self },
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
        let paints = vectorcraft_plugins::effect::Paints { appearance: ctx.appearance, item: ctx.item };
        return vectorcraft_plugins::effect::apply(plugin, p, path, b, ctx.rule, paints).unwrap_or_else(|| path.clone());
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
        // Outline Object returns the geometry unchanged. Type reaches every geometry effect as its
        // glyph outlines (`reshape::reshape`), so type that carries it is drawn as those outlines.
        "path.outlineObject" => path.clone(),
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
