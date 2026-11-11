//! Preferences (Edit → Preferences): `prefs.get`, `prefs.set`, `prefs.reset`, `prefs.list`,
//! `prefs.export`, `prefs.import`.
//!
//! [`PREF_SPECS`] is the single source of truth for every preference: its category, section,
//! label and value kind. The Preferences dialog renders from it and `prefs.set` validates with it.

use serde_json::{Map, Value, json};

use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, str_param, switch_or, text_param};
use crate::Prefs;
use crate::units::Measure;
use vectorcraft_doc::{GridPrefs, Unit};

/// What kind of value a preference holds.
#[derive(Clone, Copy, Debug)]
pub enum PrefKind {
    Bool,
    /// A number in `min..=max` shown with `unit` ("pt", "px", "°", "%", "min", "").
    Num {
        min: f64,
        max: f64,
        unit: &'static str,
    },
    /// A length in points in `min..=max`, shown and typed in the unit of `measure` (also given
    /// as a string with a unit, "5 mm").
    Length {
        min: f64,
        max: f64,
        measure: Measure,
    },
    /// A whole number in `min..=max`.
    Int {
        min: i64,
        max: i64,
    },
    /// One of (value, label).
    Choice(&'static [(&'static str, &'static str)]),
    /// `#rrggbb`.
    Color,
    Text,
}

#[derive(Clone, Copy, Debug)]
pub struct PrefSpec {
    pub key: &'static str,
    pub category: &'static str,
    /// Group heading inside the category ("" = none).
    pub section: &'static str,
    pub label: &'static str,
    pub kind: PrefKind,
}

/// Categories in Illustrator's order.
pub const PREF_CATEGORIES: &[&str] = &[
    "General",
    "Selection & Anchor Display",
    "Type",
    "Units",
    "Guides & Grid",
    "Smart Guides",
    "Slices",
    "Hyphenation",
    "Plug-ins & Scratch Disks",
    "User Interface",
    "Performance",
    "File Handling",
    "Clipboard Handling",
    "Appearance of Black",
    "Devices",
];

/// Every unit as (`Unit::key`, `Unit::label`).
pub const UNITS: &[(&str, &str)] = &[
    ("points", "Points"),
    ("picas", "Picas"),
    ("inches", "Inches"),
    ("millimeters", "Millimeters"),
    ("centimeters", "Centimeters"),
    ("pixels", "Pixels"),
    ("feetInches", "Feet & Inches"),
    ("meters", "Meters"),
    ("yards", "Yards"),
    ("feet", "Feet"),
];
/// What Units › East Asian Type offers, as (`Unit::key`, `Unit::label`): [`Unit::EAST_ASIAN`].
pub const EAST_ASIAN_UNITS: &[(&str, &str)] = &{
    let mut out = [("", ""); Unit::EAST_ASIAN.len()];
    let mut i = 0;
    while i < out.len() {
        out[i] = (Unit::EAST_ASIAN[i].key(), Unit::EAST_ASIAN[i].label());
        i += 1;
    }
    out
};
const LINE_STYLE: &[(&str, &str)] = &[("lines", "Lines"), ("dots", "Dots")];
/// Performance › Graphics Processor (`gpuPreference`), read by the desktop app at startup. The
/// values are WebGPU's power preferences plus `automatic`; 0.5.0's default, `powerSaving`, reads as
/// `automatic` (#502).
pub const GPU_PREFERENCES: &[(&str, &str)] =
    &[("automatic", "Automatic"), ("lowPower", "Power Saving (integrated)"), ("highPerformance", "High Performance (discrete)")];
/// Performance › Linux Display Server (`linuxDisplayServer`), read by the desktop app at startup:
/// `x11` opens the window through XWayland on a Wayland session, where dropped files arrive
/// (winit 0.30 has no Wayland drag and drop, #1151); `automatic` and `x11WithPen` do the same only
/// when a pen is attached as the app starts: under XWayland a drawing tablet's pen draws as a mouse
/// (#2028, #4967).
const LINUX_DISPLAY_SERVERS: &[(&str, &str)] =
    &[("automatic", "Automatic"), ("x11", "X11 (XWayland)"), ("x11WithPen", "X11 (XWayland) When a Pen Is Attached")];
const BLACK: &[(&str, &str)] = &[("accurate", "Display All Blacks Accurately"), ("rich", "Display All Blacks as Rich Black")];
const BLACK_OUT: &[(&str, &str)] = &[("accurate", "Output All Blacks Accurately"), ("rich", "Output All Blacks as Rich Black")];

macro_rules! p {
    ($key:literal, $cat:literal, $sec:literal, $label:literal, bool) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Bool }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, num($min:expr, $max:expr, $unit:literal)) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Num { min: $min, max: $max, unit: $unit } }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, len($min:expr, $max:expr, $m:ident)) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Length { min: $min, max: $max, measure: Measure::$m } }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, int($min:expr, $max:expr)) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Int { min: $min, max: $max } }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, choice($c:expr)) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Choice($c) }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, color) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Color }
    };
    ($key:literal, $cat:literal, $sec:literal, $label:literal, text) => {
        PrefSpec { key: $key, category: $cat, section: $sec, label: $label, kind: PrefKind::Text }
    };
}

pub const PREF_SPECS: &[PrefSpec] = &[
    // General
    p!("keyboardIncrement", "General", "", "Keyboard Increment", len(0.001, 1296.0, General)),
    p!("constrainAngle", "General", "", "Constrain Angle", num(-360.0, 360.0, "°")),
    p!("cornerRadius", "General", "", "Corner Radius", len(0.0, 1296.0, General)),
    p!("pasteOffset", "General", "", "Paste Offset", len(0.0, 1296.0, General)),
    p!("disableAutoAddDelete", "General", "Options", "Disable Auto Add/Delete", bool),
    p!("usePreciseCursors", "General", "Options", "Use Precise Cursors", bool),
    p!("showToolTips", "General", "Options", "Show Tool Tips", bool),
    p!("showExternalScriptWarning", "General", "Options", "Show External Script Warning", bool),
    p!("antiAliasedArtwork", "General", "Options", "Anti-aliased Artwork", bool),
    p!("selectSameTintPercent", "General", "Options", "Select Same Tint %", bool),
    p!("showHomeScreen", "General", "Options", "Show The Home Screen When No Documents Are Open", bool),
    p!("usePreviewBounds", "General", "Options", "Use Preview Bounds", bool),
    p!("displayPrintSize", "General", "Options", "Display Print Size at 100% Zoom", bool),
    p!("doubleClickToIsolate", "General", "Options", "Double Click To Isolate", bool),
    p!("transformPatternTiles", "General", "Options", "Transform Pattern Tiles", bool),
    p!("scaleCorners", "General", "Options", "Scale Corners", bool),
    p!("scaleStrokes", "General", "Options", "Scale Strokes & Effects", bool),
    p!("zoomWithMouseWheel", "General", "Options", "Zoom with Mouse Wheel", bool),
    p!("ctrlWheelScrollsSideways", "General", "Options", "Scroll Sideways with Cmd/Ctrl + Wheel", bool),
    p!("scrubNumericFields", "General", "Options", "Scrub Numeric Fields by Dragging", bool),
    p!("trackpadRotateView", "General", "Options", "Trackpad Gesture to Rotate View", bool),
    p!("holdToolKeys", "General", "Options", "Hold a Tool Shortcut to Use the Tool Temporarily", bool),
    // Selection & Anchor Display
    p!("selectionTolerance", "Selection & Anchor Display", "Selection", "Tolerance", num(1.0, 8.0, "px")),
    p!(
        "marqueeSelection",
        "Selection & Anchor Display",
        "Selection",
        "Marquee Selection",
        choice(&[("touching", "Touching"), ("enclosed", "Fully Enclosed"), ("directional", "Drag Direction")])
    ),
    p!("objectSelectionByPathOnly", "Selection & Anchor Display", "Selection", "Object Selection by Path Only", bool),
    p!("snapToPointTolerance", "Selection & Anchor Display", "Selection", "Snap to Point", num(1.0, 8.0, "px")),
    p!("ctrlClickSelectsBehind", "Selection & Anchor Display", "Selection", "Command Click to Select Objects Behind", bool),
    p!("zoomToSelection", "Selection & Anchor Display", "Selection", "Zoom to Selection", bool),
    p!("moveLockedWithArtboard", "Selection & Anchor Display", "Selection", "Move Locked and Hidden Artwork with Artboard", bool),
    p!("anchorSize", "Selection & Anchor Display", "Anchor Points, Handle, and Bounding Box Display", "Size", int(1, 7)),
    p!(
        "handleStyle",
        "Selection & Anchor Display",
        "Anchor Points, Handle, and Bounding Box Display",
        "Handles",
        choice(&[("solid", "Solid"), ("hollow", "Hollow"), ("large", "Large")])
    ),
    p!(
        "highlightAnchorsOnHover",
        "Selection & Anchor Display",
        "Anchor Points, Handle, and Bounding Box Display",
        "Highlight anchors on mouse over",
        bool
    ),
    p!(
        "showHandlesMultipleAnchors",
        "Selection & Anchor Display",
        "Anchor Points, Handle, and Bounding Box Display",
        "Show handles when multiple anchors are selected",
        bool
    ),
    p!(
        "hideCornerWidgetAbove",
        "Selection & Anchor Display",
        "Anchor Points, Handle, and Bounding Box Display",
        "Hide Corner Widget for angles greater than",
        num(0.0, 180.0, "°")
    ),
    p!("penRubberBand", "Selection & Anchor Display", "Enable Rubber Band for", "Pen Tool", bool),
    p!("curvatureRubberBand", "Selection & Anchor Display", "Enable Rubber Band for", "Curvature Tool", bool),
    // Type
    p!("typeSizeIncrement", "Type", "", "Size/Leading", len(0.001, 1296.0, Type)),
    p!("trackingIncrement", "Type", "", "Tracking", num(1.0, 1000.0, "/1000 em")),
    p!("baselineShiftIncrement", "Type", "", "Baseline Shift", len(0.001, 1296.0, Type)),
    p!("showEastAsianOptions", "Type", "Language Options", "Show East Asian Options", bool),
    p!("showIndicOptions", "Type", "Language Options", "Show Indic Options", bool),
    p!("typeSelectionByPathOnly", "Type", "Options", "Type Object Selection by Path Only", bool),
    p!("fontNamesInEnglish", "Type", "Options", "Show Font Names in English", bool),
    p!("autoSizeAreaType", "Type", "Options", "Auto Size New Area Type", bool),
    p!("fontPreview", "Type", "Options", "Enable in-menu font previews", bool),
    p!("fontPreviewSize", "Type", "Options", "Font Preview Size", choice(&[("small", "Small"), ("medium", "Medium"), ("large", "Large")])),
    p!("recentFontsCount", "Type", "Options", "Number of Recent Fonts", int(1, 15)),
    p!("missingGlyphProtection", "Type", "Options", "Enable Missing Glyph Protection", bool),
    p!("highlightAlternateGlyphs", "Type", "Options", "Highlight Alternate Glyphs", bool),
    p!("highlightSubstitutedFonts", "Type", "Options", "Highlight Substituted Fonts", bool),
    p!("placeholderText", "Type", "Options", "Fill New Type Objects With Placeholder Text", bool),
    p!("typedLists", "Type", "Options", "Start Lists From Typed Markers", bool),
    p!("fontsFolder", "Type", "Options", "Additional Fonts Folder", text),
    // Units
    p!("unitsGeneral", "Units", "", "General", choice(UNITS)),
    p!("unitsStroke", "Units", "", "Stroke", choice(UNITS)),
    p!("unitsType", "Units", "", "Type", choice(UNITS)),
    p!("unitsAsianType", "Units", "", "East Asian Type", choice(EAST_ASIAN_UNITS)),
    p!("numbersWithoutUnitsArePoints", "Units", "", "Numbers Without Units Are Points", bool),
    p!("identifyObjectsBy", "Units", "", "Identify Objects By", choice(&[("objectName", "Object Name"), ("xmlId", "XML ID")])),
    // Guides & Grid
    p!("guideColor", "Guides & Grid", "Guides", "Color", color),
    p!("guideStyle", "Guides & Grid", "Guides", "Style", choice(LINE_STYLE)),
    p!("lockGuidesByDefault", "Guides & Grid", "Guides", "Lock Guides by Default", bool),
    p!("cmdRulerDragArtboardGuides", "Guides & Grid", "Guides", "Artboard Guides with Cmd/Ctrl + Ruler Drag", bool),
    p!("gridColor", "Guides & Grid", "Grid", "Color", color),
    p!("gridStyle", "Guides & Grid", "Grid", "Style", choice(LINE_STYLE)),
    p!("gridlineEvery", "Guides & Grid", "Grid", "Gridline every", len(1.0, 16383.0, General)),
    p!("gridSubdivisions", "Guides & Grid", "Grid", "Subdivisions", int(1, 1000)),
    p!("gridsInBack", "Guides & Grid", "Grid", "Grids In Back", bool),
    p!("showPixelGrid", "Guides & Grid", "Grid", "Show Pixel Grid (Above 600% Zoom)", bool),
    // Smart Guides
    p!("smartGuideColor", "Smart Guides", "Display Options", "Color", color),
    p!("alignmentGuides", "Smart Guides", "Display Options", "Alignment Guides", bool),
    p!("objectHighlighting", "Smart Guides", "Display Options", "Object Highlighting", bool),
    p!("transformToolsGuides", "Smart Guides", "Display Options", "Transform Tools", bool),
    p!("constructionGuides", "Smart Guides", "Display Options", "Construction Guides", bool),
    p!(
        "constructionAngles",
        "Smart Guides",
        "Display Options",
        "Angles",
        choice(&[
            ("90° & 45° Angles", "90° & 45° Angles"),
            ("30° Angles", "30° Angles"),
            ("45° Angles", "45° Angles"),
            ("60° Angles", "60° Angles"),
            ("90° Angles", "90° Angles"),
            ("90° & 45° & 30° Angles", "90° & 45° & 30° Angles"),
        ])
    ),
    p!("anchorPathLabels", "Smart Guides", "Display Options", "Anchor/Path Labels", bool),
    p!("measurementLabels", "Smart Guides", "Display Options", "Measurement Labels", bool),
    p!("spacingGuides", "Smart Guides", "Display Options", "Spacing Guides", bool),
    p!("distanceGuides", "Smart Guides", "Display Options", "Invoke Distance Guides", bool),
    p!("snapToLastLocation", "Smart Guides", "Display Options", "Snap to Last Location", bool),
    p!("snappingTolerance", "Smart Guides", "", "Snapping Tolerance", num(0.0, 40.0, "pt")),
    p!("snapTangent", "Smart Guides", "Geometric Guides", "Snap to Tangent", bool),
    p!("snapPerpendicular", "Smart Guides", "Geometric Guides", "Snap to Perpendicular", bool),
    // Slices
    p!("showSliceNumbers", "Slices", "", "Show Slice Numbers", bool),
    p!("sliceLineColor", "Slices", "", "Line Color", color),
    // Hyphenation
    p!(
        "hyphenationLanguage",
        "Hyphenation",
        "",
        "Default Language",
        choice(&[
            ("en-US", "English: USA"),
            ("en-GB", "English: UK"),
            ("de", "German"),
            ("de-CH", "German: Swiss"),
            ("fr", "French"),
            ("it", "Italian"),
            ("es", "Spanish"),
            ("ca", "Catalan"),
            ("pt", "Portuguese"),
            ("nl", "Dutch"),
            ("da", "Danish"),
            ("sv", "Swedish"),
            ("fi", "Finnish"),
            ("nb", "Norwegian"),
            ("pl", "Polish"),
            ("cs", "Czech"),
            ("ru", "Russian"),
            ("bg", "Bulgarian"),
            ("sr-Cyrl", "Serbian: Cyrillic"),
            ("sr-Latn", "Serbian: Latin"),
            ("hr", "Croatian"),
            ("tr", "Turkish"),
            ("ja", "Japanese"),
            ("zh", "Chinese"),
            ("hi", "Hindi"),
            ("mr", "Marathi"),
            ("gu", "Gujarati"),
            ("bn", "Bengali"),
            ("pa", "Punjabi"),
            ("ta", "Tamil"),
            ("te", "Telugu"),
            ("kn", "Kannada"),
            ("ml", "Malayalam"),
            ("or", "Odia"),
            ("as", "Assamese"),
            ("sa", "Sanskrit"),
            ("ur", "Urdu")
        ])
    ),
    p!("hyphenationExceptions", "Hyphenation", "", "Exceptions (comma separated)", text),
    // Plug-ins & Scratch Disks
    p!("pluginsFolder", "Plug-ins & Scratch Disks", "Plug-ins", "Additional Plug-ins Folder", text),
    p!("scratchPrimary", "Plug-ins & Scratch Disks", "Scratch Disks", "Primary", text),
    p!("scratchSecondary", "Plug-ins & Scratch Disks", "Scratch Disks", "Secondary", text),
    // User Interface
    p!(
        "uiBrightness",
        "User Interface",
        "",
        "Brightness",
        choice(&[("dark", "Dark"), ("mediumDark", "Medium Dark"), ("mediumLight", "Medium Light"), ("light", "Light")])
    ),
    p!("uiColorScheme", "User Interface", "", "Color Scheme", choice(&[("modern", "Modern"), ("classic", "Classic")])),
    p!(
        "uiAccent",
        "User Interface",
        "",
        "Accent Color",
        choice(&[("coral", "Coral"), ("blue", "Blue"), ("violet", "Violet"), ("green", "Green"), ("amber", "Amber"), ("pink", "Pink")])
    ),
    p!(
        "canvasColor",
        "User Interface",
        "",
        "Canvas Color",
        choice(&[
            ("matchUi", "Match User Interface Brightness"),
            ("white", "White"),
            ("lightGray", "Light Gray"),
            ("mediumGray", "Medium Gray"),
            ("darkGray", "Dark Gray")
        ])
    ),
    p!("autoCollapseIconPanels", "User Interface", "", "Auto-Collapse Iconic Panels", bool),
    p!("toolGroupLabels", "User Interface", "", "Show Tool Group Labels", bool),
    p!("openDocumentsAsTabs", "User Interface", "", "Open Documents As Tabs", bool),
    p!("largeTabs", "User Interface", "", "Large Tabs", bool),
    p!("systemTitleBar", "User Interface", "", "System Title Bar", bool),
    p!("appTitleBar", "User Interface", "", "VectorCraft Title Bar on Every Desktop", bool),
    p!("uiScaling", "User Interface", "UI Scaling", "Scale", num(0.75, 2.0, "×")),
    p!("scaleCursorWithUi", "User Interface", "UI Scaling", "Scale Cursor Proportional to UI", bool),
    // `auto` or a language code the shell registers (`zh-hant`); the shell shows it as a dropdown.
    p!("interfaceLanguage", "User Interface", "Language", "Language", text),
    // Performance
    p!("gpuPerformance", "Performance", "GPU Performance", "GPU Performance", bool),
    p!("animatedZoom", "Performance", "GPU Performance", "Animated Zoom", bool),
    p!("gpuPreference", "Performance", "GPU Performance", "Graphics Processor", choice(GPU_PREFERENCES)),
    p!("historyStates", "Performance", "", "History States", int(5, 1000)),
    p!("realTimeDrawing", "Performance", "", "Real-time Drawing and Editing", bool),
    p!("renderThreads", "Performance", "", "Render Threads (-1 = Automatic)", int(-1, 64)),
    p!("linuxDisplayServer", "Performance", "", "Linux Display Server", choice(LINUX_DISPLAY_SERVERS)),
    // File Handling
    p!("backgroundSave", "File Handling", "", "Enable Background Save", bool),
    p!("backgroundExport", "File Handling", "", "Enable Background Export", bool),
    p!("autosaveRecovery", "File Handling", "Data Recovery", "Automatically Save Recovery Data", bool),
    p!("autosaveInterval", "File Handling", "Data Recovery", "Every (minutes)", int(1, 60)),
    p!("recoveryFolder", "File Handling", "Data Recovery", "Folder", text),
    p!("recoveryOffForComplex", "File Handling", "Data Recovery", "Turn off Data Recovery for complex documents", bool),
    p!("recentFilesCount", "File Handling", "Files", "Number of Recent Files to Display", int(0, 30)),
    p!("lowResProxyEps", "File Handling", "Files", "Use Low Resolution Proxy for Linked EPS", bool),
    p!("antiAliasedBitmaps", "File Handling", "Files", "Display Bitmaps as Anti-aliased Images in Pixel Preview", bool),
    p!(
        "updateLinks",
        "File Handling",
        "Files",
        "Update Links",
        choice(&[("automatically", "Automatically"), ("manually", "Manually"), ("askWhenModified", "Ask When Modified")])
    ),
    // Clipboard Handling
    p!("copyAsSvg", "Clipboard Handling", "On Copy", "Include SVG Code", bool),
    p!("copyAsPdf", "Clipboard Handling", "On Copy", "PDF", bool),
    p!("copyAicb", "Clipboard Handling", "On Copy", "Legacy vector clipboard (no transparency)", bool),
    p!(
        "aicbMode",
        "Clipboard Handling",
        "On Copy",
        "Legacy vector clipboard",
        choice(&[("preservePaths", "Preserve Paths"), ("preserveAppearance", "Preserve Appearance and Overprints")])
    ),
    p!(
        "pasteTextFormatting",
        "Clipboard Handling",
        "On Paste",
        "When pasting text",
        choice(&[("keep", "Keep Formatting"), ("plain", "Keep Plain Text")])
    ),
    // Appearance of Black
    p!("blackOnScreen", "Appearance of Black", "", "On Screen", choice(BLACK)),
    p!("blackOutput", "Appearance of Black", "", "Printing / Exporting", choice(BLACK_OUT)),
    // Devices
    p!("touchWorkspace", "Devices", "", "Enable Touch Workspace", bool),
    p!("touchGestures", "Devices", "", "Enable Touch Gestures", bool),
    // Graphic Styles panel menu
    p!("overrideCharColor", "Type", "Graphic Styles", "Override Character Color", bool),
    // Object → Pattern → Tile Edge Color
    p!("patternTileEdgeColor", "Guides & Grid", "Pattern Editing", "Tile Edge Color", color),
    // Object → Create Trim Marks, Effect → Crop Marks
    p!("japaneseCropMarks", "General", "Options", "Use Japanese Crop Marks", bool),
    // Appearance panel menu
    p!("newArtBasic", "General", "Appearance Panel", "New Art Has Basic Appearance", bool),
    // File Handling (continued)
    p!("templatesFolder", "File Handling", "Files", "Templates Folder", text),
    // Off: raster images open in PhotoCraft where it is installed (#5236).
    p!("editOriginalSystemDefaults", "File Handling", "Files", "Use System Defaults for Edit Original", bool),
    p!("appendConverted", "File Handling", "Files", "Mark Older Files as [Converted] When Opened", bool),
    // Native saves (`document.save {compress}`)
    p!("useCompression", "File Handling", "Files", "Use Compression", bool),
    // The W/H link of the Transform panel, the Properties panel and the Control bar
    p!("constrainProportions", "General", "Transform Panel", "Constrain Width and Height Proportions", bool),
    p!("singleCornerRadius", "General", "Transform Panel", "Single Corner Radius", bool),
    // The Transform panel menu: what the panel transforms (`default`: as General › Transform
    // Pattern Tiles, until a choice in the menu), and symbols' registration point
    p!(
        "transformPanelPatterns",
        "General",
        "Transform Panel",
        "Transform",
        choice(&[("default", "Default"), ("object", "Transform Object Only"), ("pattern", "Transform Pattern Only"), ("both", "Transform Both")])
    ),
    p!("useRegistrationPointForSymbol", "General", "Transform Panel", "Use Registration Point for Symbol", bool),
    // The Align panel menu's Align to Glyph Bounds: Align and Distribute measure type by its glyphs' ink.
    p!("alignGlyphBoundsPoint", "Type", "Align to Glyph Bounds", "Point Text", bool),
    p!("alignGlyphBoundsArea", "Type", "Align to Glyph Bounds", "Area Text", bool),
    // The bounding box and the Transform panel measure point type by its glyphs' ink (also in the Transform panel menu).
    p!("typeGlyphBounds", "Type", "Glyph Bounds", "Measure Type by Glyph Bounds", bool),
    // Artboard Options › Global: the canvas darkens the area outside the artboards while the Artboard tool is active.
    p!("fadeRegionOutsideArtboard", "General", "Artboard Options", "Fade Region Outside Artboard", bool),
    p!("artboardUpdateWhileDragging", "General", "Artboard Options", "Update While Dragging", bool),
];

pub fn spec(key: &str) -> Option<&'static PrefSpec> {
    PREF_SPECS.iter().find(|s| s.key == key)
}

/// Preferences kept as one object with a command of their own instead of [`PREF_SPECS`] rows (they
/// aren't in the Preferences dialog): the Eyedropper Options (`eyedropper.setOptions`), the
/// Perspective Grid Options (`perspective.widget.options`), the Pathfinder Options
/// (`object.pathfinder.options`) and the Snap to Glyph options (`view.snapToGlyph.options`).
/// `prefs.get` and `prefs.set` take them by key (a partial object updates what it names) and
/// `prefs.reset` without a category resets them.
pub const PREF_GROUPS: &[&str] = &["eyedropper", "perspectiveWidget", "pathfinder", "glyphSnap"];

/// Validate a value for preference group `key` against `current`.
fn validate_group(key: &str, current: &Value, v: &Value) -> std::result::Result<Value, String> {
    if !v.is_object() {
        return Err(format!("`{key}` must be an object"));
    }
    match key {
        "eyedropper" => {
            let cur: super::EyedropperOptions = serde_json::from_value(current.clone()).unwrap_or_default();
            cur.merged(v).map(|o| json!(o))
        }
        "pathfinder" => {
            let cur: super::PathfinderOptions = serde_json::from_value(current.clone()).unwrap_or_default();
            cur.merged(v).map(|o| json!(o))
        }
        "glyphSnap" => {
            let cur: vectorcraft_tools::GlyphSnap = serde_json::from_value(current.clone()).unwrap_or_default();
            cur.merged(v).map(|o| json!(o))
        }
        "perspectiveWidget" => {
            let mut merged = current.clone();
            if let (Some(o), Some(p)) = (merged.as_object_mut(), v.as_object()) {
                o.extend(p.clone());
            }
            serde_json::from_value::<vectorcraft_tools::distort::perspective::widget::WidgetOptions>(merged)
                .map(|o| json!(o))
                .map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown preference `{key}`")),
    }
}

/// Validate (and normalize) a value for `key`. Numbers may be given as strings; booleans as
/// "true"/"false"; choices by value or label (case-insensitive).
pub fn validate(key: &str, v: &Value) -> std::result::Result<Value, String> {
    let sp = spec(key).ok_or_else(|| format!("unknown preference `{key}`"))?;
    let num =
        || v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c == '°').trim().parse().ok()));
    match sp.kind {
        PrefKind::Bool => match v {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(s) if s == "true" || s == "false" => Ok(json!(s == "true")),
            _ => Err(format!("`{key}` must be true or false")),
        },
        PrefKind::Num { min, max, .. } => in_range(key, num(), min, max),
        PrefKind::Length { min, max, .. } => in_range(key, v.as_f64().or_else(|| v.as_str().and_then(|s| Unit::Points.parse(s))), min, max),
        PrefKind::Int { min, max } => match num() {
            Some(x) if x.fract() == 0.0 && (min as f64..=max as f64).contains(&x) => Ok(json!(x as i64)),
            Some(x) => Err(format!("`{key}` must be a whole number between {min} and {max} (got {x})")),
            None => Err(format!("`{key}` must be a whole number")),
        },
        PrefKind::Choice(opts) => {
            let s = v.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
            opts.iter()
                .find(|(val, label)| val.eq_ignore_ascii_case(s) || label.eq_ignore_ascii_case(s))
                .map(|(val, _)| json!(val))
                .ok_or_else(|| format!("`{key}` must be one of {}", opts.iter().map(|o| o.0).collect::<Vec<_>>().join(", ")))
        }
        PrefKind::Color => {
            let s = v.as_str().unwrap_or("");
            let hex = s.trim_start_matches('#');
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(json!(format!("#{}", hex.to_ascii_lowercase())))
            } else {
                Err(format!("`{key}` must be a #rrggbb colour"))
            }
        }
        PrefKind::Text if key == "interfaceLanguage" => {
            let s = v.as_str().map(str::trim).unwrap_or("");
            let ok = !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            if ok { Ok(json!(s.to_ascii_lowercase())) } else { Err(format!("`{key}` must be `auto` or a language code such as `en` or `zh-hant`")) }
        }
        PrefKind::Text => match v {
            Value::String(s) => Ok(json!(s)),
            _ => Err(format!("`{key}` must be a string")),
        },
    }
}

/// `x` (`None`: not a number) as the value of number preference `key` in `min..=max`.
fn in_range(key: &str, x: Option<f64>, min: f64, max: f64) -> std::result::Result<Value, String> {
    match x {
        Some(x) if x.is_finite() && (min..=max).contains(&x) => Ok(json!(x)),
        Some(x) => Err(format!("`{key}` must be between {min} and {max} (got {x})")),
        None => Err(format!("`{key}` must be a number")),
    }
}

/// A check of a folder path: `Err` says why the folder can't be the preference's value.
pub type FolderCheck = fn(&str) -> std::result::Result<(), String>;

/// The check a folder preference with a Choose… button gives a folder before it becomes the
/// preference's value: Data Recovery keeps its copies in its folder, and the Additional Plug-ins
/// Folder is read for plug-ins. `None` for any other preference.
pub fn folder_check(key: &str) -> Option<FolderCheck> {
    match key {
        "recoveryFolder" => Some(writable_folder),
        "pluginsFolder" => Some(plugins_folder),
        _ => None,
    }
}

/// `Ok` when `path` is a folder whose contents can be listed, as the plug-ins in it are found by
/// listing it. A folder outside the automation roots in force is refused before it is looked at.
fn plugins_folder(path: &str) -> std::result::Result<(), String> {
    crate::file_access::check_read(path)?;
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    std::fs::read_dir(dir).map(drop).map_err(|e| format!("Plug-ins can't be loaded from {path}: {e}"))
}

/// `Ok` when `path` is a folder that can be listed and that a folder can be created in, as the
/// recovery store lists its folders there and makes its own: the folder is listed, and an empty
/// folder is made there and removed. A folder that can't be removed again is an error too. A
/// folder outside the automation roots in force is refused before it is looked at.
fn writable_folder(path: &str) -> std::result::Result<(), String> {
    crate::file_access::check_write(path)?;
    crate::file_access::check_read(path)?;
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let refused = |e: std::io::Error| format!("Data Recovery can't keep copies in {path}: {e}");
    std::fs::read_dir(dir).map_err(refused)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_nanos());
    let check = dir.join(format!(".vectorcraft-write-check-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&check).map_err(refused)?;
    std::fs::remove_dir(&check).map_err(refused)
}

impl Prefs {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Set several validated values at once; on any error nothing changes.
    pub fn set_values(&mut self, values: &Map<String, Value>) -> std::result::Result<(), String> {
        let mut obj = self.to_json();
        for (k, v) in values {
            obj[k.as_str()] = if PREF_GROUPS.contains(&k.as_str()) { validate_group(k, &obj[k.as_str()], v)? } else { validate(k, v)? };
        }
        *self = serde_json::from_value(obj).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The folders the app reads (fonts, plug-ins) or writes (Data Recovery copies, the Templates
    /// folder it makes for its dialogs) on its own, with the access each needs.
    fn folders_mut(&mut self) -> [(&mut String, &'static [crate::file_access::Access]); 4] {
        use crate::file_access::Access;
        [
            (&mut self.fonts_folder, &[Access::Read][..]),
            (&mut self.plugins_folder, &[Access::Read][..]),
            // Data Recovery lists and restores what is in its folder too.
            (&mut self.recovery_folder, &[Access::Write, Access::Read][..]),
            (&mut self.templates_folder, &[Access::Write][..]),
        ]
    }

    /// Put back the folders `next` sets in place of these preferences' that the automation roots
    /// in force refuse: automation may only point them inside its roots ([`crate::file_access`]).
    /// → why each was refused. Nothing is refused without roots, nor a folder cleared or kept.
    fn keep_folders(&self, next: &mut Prefs) -> Vec<String> {
        let Some(roots) = crate::file_access::current() else { return vec![] };
        let mut before = self.clone();
        let mut refused = vec![];
        for ((old, _), (new, access)) in before.folders_mut().into_iter().zip(next.folders_mut()) {
            let folder = new.trim();
            if folder.is_empty() || folder == old.trim() {
                continue;
            }
            if let Err(e) = access.iter().try_for_each(|a| roots.check(folder, *a)) {
                refused.push(e);
                *new = old.clone();
            }
        }
        refused
    }

    /// The [`folder_check`] of each folder preference in `values`. An empty value (the default
    /// folder) and a folder the preference already holds are not checked. The web has no folders to
    /// check.
    fn check_folders(&self, values: &Map<String, Value>) -> std::result::Result<(), String> {
        if cfg!(target_arch = "wasm32") {
            return Ok(());
        }
        let saved = self.to_json();
        for (key, v) in values {
            let (Some(check), Some(path)) = (folder_check(key), v.as_str()) else { continue };
            let path = path.trim();
            if path.is_empty() || saved[key.as_str()].as_str().is_some_and(|s| s.trim() == path) {
                continue;
            }
            check(path)?;
        }
        Ok(())
    }

    /// Values of one category (`None` = all) reset to defaults.
    pub fn reset(&mut self, category: Option<&str>) {
        let d = Prefs::default().to_json();
        let mut obj = self.to_json();
        for sp in PREF_SPECS.iter().filter(|s| category.is_none_or(|c| c.eq_ignore_ascii_case(s.category))) {
            obj[sp.key] = d[sp.key].clone();
        }
        if category.is_none() {
            for g in PREF_GROUPS {
                obj[*g] = d[*g].clone();
            }
        }
        if let Ok(p) = serde_json::from_value(obj) {
            *self = p;
        }
    }

    /// The document grid Guides & Grid › Gridline every and Subdivisions set.
    pub fn grid(&self) -> GridPrefs {
        GridPrefs { spacing: self.gridline_every, subdivisions: self.grid_subdivisions }
    }
}

impl Session {
    /// Replace the preferences and push the ones with engine-side consumers into open documents
    /// and the renderer (grid, history depth, render threads).
    pub fn apply_prefs(&mut self, mut p: Prefs) {
        // However they are set (a dialog driven by automation too), the folders stay inside the
        // automation roots in force: a refused one keeps its value.
        for e in self.prefs.keep_folders(&mut p) {
            log::warn!("{e}");
        }
        // The edit the active tool computes on another thread is applied first (the Preferences
        // dialog applies its settings here, outside a command). A failure to apply it was the
        // edit's own, and the preferences apply all the same.
        if self.depth == 0 {
            let _ = self.finish_tool_edit_later();
        }
        let grid_changed = p.gridline_every != self.prefs.gridline_every || p.grid_subdivisions != self.prefs.grid_subdivisions;
        let history_changed = p.history_states != self.prefs.history_states;
        let tile_edge_changed = p.pattern_tile_edge_color != self.prefs.pattern_tile_edge_color;
        let fonts_folder_changed = p.fonts_folder != self.prefs.fonts_folder;
        if p.plugins_folder.trim().is_empty() && !self.prefs.plugins_folder.trim().is_empty() {
            super::plugin::forget_prefs_folder(&self.prefs.plugins_folder);
        }
        let hyphen_exceptions_changed = p.hyphenation_exceptions != self.prefs.hyphenation_exceptions;
        let corner_radius_changed = p.corner_radius != self.prefs.corner_radius;
        let lock_guides_changed = p.lock_guides_by_default != self.prefs.lock_guides_by_default;
        if p.trace_presets != self.prefs.trace_presets {
            self.trace_preset_changes.replaced();
        }
        self.prefs = p;
        // Guides & Grid › Lock Guides by Default sets the lock when it changes, the saved value at
        // start included. Otherwise the lock stays as View › Guides › Lock Guides left it.
        if lock_guides_changed {
            self.set_guides_locked(self.prefs.lock_guides_by_default);
        }
        if corner_radius_changed {
            // General › Corner Radius is the Rounded Rectangle tool's radius (#394): a tool already
            // made (active, or lent a selection tool for a drag) takes the new value.
            let r = serde_json::Value::from(self.prefs.corner_radius);
            for t in std::iter::once(&mut self.tool).chain(self.lender.as_mut()) {
                if t.id() == "roundedRectangle" {
                    t.set_option("cornerRadius", &r);
                }
            }
        }
        if fonts_folder_changed {
            let folder = self.prefs.fonts_folder.trim();
            // A folder an automation caller chose is read without following its links out of it.
            let confined = crate::file_access::current().is_some();
            vectorcraft_text::set_user_font_dirs_confined(if folder.is_empty() { vec![] } else { vec![folder.into()] }, confined);
            // Once the fonts were scanned, scan again now: the folder's fonts appear (or go) in the
            // font menus, and type in them lays out again. The first scan reads it anyway.
            if vectorcraft_text::FontDb::global().installed_fonts_changed() {
                // Its result only counts the faces cataloged.
                let _ = super::fonts::rescan(self, &serde_json::Value::Null);
            }
        }
        if hyphen_exceptions_changed {
            // Preferences › Hyphenation › Exceptions (#394): the layout reads the process-wide list.
            vectorcraft_text::set_hyphenation_exceptions(&self.prefs.hyphenation_exceptions);
        }
        vectorcraft_render::set_default_threads(u16::try_from(self.prefs.render_threads).ok());
        for st in &mut self.docs {
            if history_changed {
                st.history.limit = self.prefs.history_states as usize;
                st.history.trim();
            }
            // Documents in pattern editing mode redraw their tile edge.
            if tile_edge_changed && st.doc.pattern_edit.is_some() {
                st.revision += 1;
            }
            if grid_changed {
                let d = std::sync::Arc::make_mut(&mut st.doc);
                d.grid = self.prefs.grid();
                st.revision += 1;
            }
            // Hyphenation exceptions change every text object's line breaks.
            if hyphen_exceptions_changed {
                st.revision += 1;
            }
        }
        // Plug-ins in a newly set Additional Plug-ins Folder are installed.
        super::plugin::sync_prefs(self);
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "prefs.get", "Get Preferences", [], None, "{key?} → the value of one preference (or preference group: eyedropper), or {prefs: {…all…}}", always, get),
        cmd!(
            query "prefs.set",
            "Set Preference",
            [],
            None,
            "{key, value} or {values: {key: value, …}, documentUnits?: bool=true} set preferences (validated; see prefs.list; lengths in pt or strings with a unit; unitsGeneral also sets the active document's units, one undo step, unless documentUnits is false; the eyedropper group takes a partial object, as eyedropper.setOptions, and a group given anything but an object is an error; a new recoveryFolder must be a folder that can be listed and written in, and a new pluginsFolder a folder that can be listed: otherwise an error, and nothing changes; an empty value is not checked)",
            always,
            set
        ),
        cmd!(query "prefs.reset", "Reset Preferences", [], None, "{category?} reset one category (or everything) to defaults", always, reset),
        cmd!(query "prefs.list", "List Preferences", [], None, "{} → [{key, category, section, label, kind, value, …}] every preference", always, list),
        cmd!(
            query "prefs.export",
            "Export Preferences",
            [],
            None,
            "{path?} write the preferences as a JSON file {format: \"vectorcraft-preferences\", version: 1, prefs: {key: value, …}}: every preference prefs.list lists and the eyedropper, perspectiveWidget, pathfinder and glyphSnap groups, without the folder preferences fontsFolder, pluginsFolder, recoveryFolder and templatesFolder and without saved presets; a file over 1 MB, which prefs.import would refuse, is an error and is not written → {path, count}; without path → {data: the file's text, count}",
            always,
            export
        ),
        cmd!(
            query "prefs.import",
            "Import Preferences",
            [],
            None,
            "{path? | data?: the file's text} apply the preferences of a file prefs.export wrote (at most 1 MB). Every value is checked first, as prefs.set checks it: another format, a newer version, an unknown preference, a value of the wrong type or out of range, or a folder preference is an error, and nothing changes. Preferences the file leaves out keep their values; the open document's units stay as they are; no undo step; not a command.batch step → {imported: count}",
            always,
            import
        ),
    ]
}

/// The `format` of a preferences file (`prefs.export`, `prefs.import`).
pub const PREFS_FORMAT: &str = "vectorcraft-preferences";

/// The `version` `prefs.export` writes; `prefs.import` refuses a newer one.
pub const PREFS_VERSION: u64 = 1;

/// The largest preferences file `prefs.import` reads.
const MAX_PREFS_FILE_BYTES: usize = 1 << 20;

/// Folder preferences, which name folders on one computer: a preferences file leaves them out.
const FOLDER_PREFS: &[&str] = &["fontsFolder", "pluginsFolder", "recoveryFolder", "templatesFolder"];

/// The preferences `prefs.export` writes: every [`PREF_SPECS`] key but the [`FOLDER_PREFS`], and
/// the [`PREF_GROUPS`].
fn exported(prefs: &crate::Prefs) -> Map<String, Value> {
    let all = prefs.to_json();
    let keys = PREF_SPECS.iter().map(|s| s.key).filter(|k| !FOLDER_PREFS.contains(k)).chain(PREF_GROUPS.iter().copied());
    keys.map(|k| (k.to_string(), all[k].clone())).collect()
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "prefs.export";
    let path = text_param(p, "path", C)?;
    let prefs = exported(&s.prefs);
    let count = prefs.len();
    let file = json!({ "format": PREFS_FORMAT, "version": PREFS_VERSION, "prefs": prefs });
    let text = serde_json::to_string_pretty(&file).map_err(|e| bad(C, e.to_string()))?;
    // A file `prefs.import` would refuse is not written.
    if text.len() > MAX_PREFS_FILE_BYTES {
        return Err(bad(C, "the file is larger than 1 MB"));
    }
    match path {
        Some(path) => {
            super::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "count": count}))
        }
        None => Ok(json!({"data": text, "count": count})),
    }
}

/// The preferences in the bytes of a preferences file, unchecked: `Err` says why the file is not
/// one this version reads.
fn read_prefs_file(bytes: &[u8]) -> std::result::Result<Map<String, Value>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a preferences file: {e}"))?;
    let Some(file) = v.as_object() else { return Err("not a preferences file: no JSON object".into()) };
    match file.get("format").and_then(Value::as_str) {
        Some(PREFS_FORMAT) => {}
        Some(format) => return Err(format!("not a preferences file (format `{format}`)")),
        None => return Err("not a preferences file: no `format`".into()),
    }
    match file.get("version").and_then(Value::as_u64) {
        Some(v) if v > PREFS_VERSION => return Err(format!("the file was saved by a newer version of VectorCraft (version {v})")),
        Some(1..) => {}
        _ => return Err("not a preferences file: no `version`".into()),
    }
    let Some(prefs) = file.get("prefs").and_then(Value::as_object) else { return Err("not a preferences file: no `prefs`".into()) };
    if prefs.is_empty() {
        return Err("the file holds no preferences".into());
    }
    if let Some(key) = prefs.keys().find(|k| FOLDER_PREFS.contains(&k.as_str())) {
        return Err(format!("`{key}` names a folder, which preferences files leave out"));
    }
    Ok(prefs.clone())
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "prefs.import";
    let too_large = || bad(C, "the file is larger than 1 MB");
    let bytes = match (text_param(p, "path", C)?, text_param(p, "data", C)?) {
        (Some(path), None) => super::fileio::read_file_head(path, MAX_PREFS_FILE_BYTES + 1)?,
        (None, Some(text)) if text.len() > MAX_PREFS_FILE_BYTES => return Err(too_large()),
        (None, Some(text)) => text.as_bytes().to_vec(),
        _ => return Err(bad(C, "give `path` or `data`")),
    };
    if bytes.len() > MAX_PREFS_FILE_BYTES {
        return Err(too_large());
    }
    let values = read_prefs_file(&bytes).map_err(|e| bad(C, e))?;
    let mut next = s.prefs.clone();
    next.set_values(&values).map_err(|e| bad(C, e))?;
    s.apply_prefs(next);
    Ok(json!({ "imported": values.len() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let all = s.prefs.to_json();
    match str_param(p, "key") {
        Some(k) if spec(k).is_some() || PREF_GROUPS.contains(&k) => Ok(all[k].clone()),
        Some(k) => Err(bad("prefs.get", format!("unknown preference `{k}`"))),
        None => Ok(json!({ "prefs": all })),
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let mut values = p.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(k) = str_param(p, "key") {
        values.insert(k.to_string(), p.get("value").cloned().ok_or_else(|| bad("prefs.set", "missing `value`"))?);
    }
    if values.is_empty() {
        return Err(bad("prefs.set", "give {key, value} or {values}"));
    }
    let document_units = switch_or(p, "documentUnits", true, "prefs.set")?;
    let mut next = s.prefs.clone();
    next.set_values(&values).map_err(|e| bad("prefs.set", e))?;
    if let Some(e) = s.prefs.keep_folders(&mut next).into_iter().next() {
        return Err(EngineError::Other(e));
    }
    s.prefs.check_folders(&values).map_err(EngineError::Other)?;
    s.apply_prefs(next);
    // Units ▸ General is also the open document's units (as Document Setup sets them).
    if document_units && values.contains_key("unitsGeneral") && s.active().is_some() {
        s.set_document_units(s.default_units())?;
    }
    let all = s.prefs.to_json();
    Ok(Value::Object(values.keys().map(|k| (k.clone(), all[k.as_str()].clone())).collect()))
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let cat = str_param(p, "category");
    if let Some(c) = cat
        && !PREF_CATEGORIES.iter().any(|x| x.eq_ignore_ascii_case(c))
    {
        return Err(bad("prefs.reset", format!("unknown category `{c}`")));
    }
    let mut next = s.prefs.clone();
    next.reset(cat);
    s.apply_prefs(next);
    Ok(json!({ "prefs": s.prefs.to_json() }))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let all = s.prefs.to_json();
    Ok(Value::Array(
        PREF_SPECS
            .iter()
            .map(|sp| {
                let mut o = json!({"key": sp.key, "category": sp.category, "section": sp.section, "label": sp.label, "value": all[sp.key]});
                match sp.kind {
                    PrefKind::Bool => o["kind"] = json!("bool"),
                    PrefKind::Num { min, max, unit } => {
                        o["kind"] = json!("number");
                        o["min"] = json!(min);
                        o["max"] = json!(max);
                        o["unit"] = json!(unit);
                    }
                    PrefKind::Length { min, max, measure } => {
                        o["kind"] = json!("number");
                        o["min"] = json!(min);
                        o["max"] = json!(max);
                        o["unit"] = json!("pt");
                        o["measure"] = json!(measure.name());
                    }
                    PrefKind::Int { min, max } => {
                        o["kind"] = json!("integer");
                        o["min"] = json!(min);
                        o["max"] = json!(max);
                    }
                    PrefKind::Choice(c) => {
                        o["kind"] = json!("choice");
                        o["options"] = json!(c.iter().map(|x| x.0).collect::<Vec<_>>());
                    }
                    PrefKind::Color => o["kind"] = json!("color"),
                    PrefKind::Text => o["kind"] = json!("text"),
                }
                o
            })
            .collect(),
    ))
}
