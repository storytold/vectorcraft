//! Preferences (Edit → Preferences): `prefs.get`, `prefs.set`, `prefs.reset`, `prefs.list`.
//!
//! [`PREF_SPECS`] is the single source of truth for every preference: its category, section,
//! label and value kind. The Preferences dialog renders from it and `prefs.set` validates with it.

use serde_json::{Map, Value, json};

use super::*;
use crate::Prefs;
use crate::units::Measure;
use vectorcraft_doc::Unit;

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

/// Categories in Illustrator's order (Plug-ins & Scratch Disks is "Performance & Storage" here).
pub const PREF_CATEGORIES: &[&str] = &[
    "General",
    "Selection & Anchor Display",
    "Type",
    "Units",
    "Guides & Grid",
    "Smart Guides",
    "Slices",
    "Hyphenation",
    "Performance & Storage",
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
const LINE_STYLE: &[(&str, &str)] = &[("lines", "Lines"), ("dots", "Dots")];
/// Performance › Graphics Processor (`gpuPreference`), read by the desktop app at startup. The
/// values are WebGPU's power preferences plus `automatic`; 0.5.0's default, `powerSaving`, reads as
/// `automatic` (#502).
pub const GPU_PREFERENCES: &[(&str, &str)] =
    &[("automatic", "Automatic"), ("lowPower", "Power Saving (integrated)"), ("highPerformance", "High Performance (discrete)")];
/// User Interface › Appearance Mode, and the brightnesses each mode shows.
pub const APPEARANCE_MODES: &[(&str, &str)] = &[("auto", "Sync with system"), ("dark", "Dark"), ("light", "Light")];
pub const DARK_THEMES: &[(&str, &str)] = &[("dark", "Dark"), ("mediumDark", "Medium Dark")];
pub const LIGHT_THEMES: &[(&str, &str)] = &[("mediumLight", "Medium Light"), ("light", "Light")];
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
    p!("scrubNumericFields", "General", "Options", "Scrub Numeric Fields by Dragging", bool),
    // Selection & Anchor Display
    p!("selectionTolerance", "Selection & Anchor Display", "Selection", "Tolerance", num(1.0, 8.0, "px")),
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
    p!("placeholderText", "Type", "Options", "Fill New Type Objects With Placeholder Text", bool),
    p!("fontsFolder", "Type", "Options", "Additional Fonts Folder", text),
    // Units
    p!("unitsGeneral", "Units", "", "General", choice(UNITS)),
    p!("unitsStroke", "Units", "", "Stroke", choice(UNITS)),
    p!("unitsType", "Units", "", "Type", choice(UNITS)),
    p!("unitsAsianType", "Units", "", "East Asian Type", choice(UNITS)),
    p!("numbersWithoutUnitsArePoints", "Units", "", "Numbers Without Units Are Points", bool),
    p!("identifyObjectsBy", "Units", "", "Identify Objects By", choice(&[("objectName", "Object Name"), ("xmlId", "XML ID")])),
    // Guides & Grid
    p!("guideColor", "Guides & Grid", "Guides", "Color", color),
    p!("guideStyle", "Guides & Grid", "Guides", "Style", choice(LINE_STYLE)),
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
    p!("snappingTolerance", "Smart Guides", "", "Snapping Tolerance", num(0.0, 40.0, "pt")),
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
            ("English: USA", "English: USA"),
            ("English: UK", "English: UK"),
            ("French", "French"),
            ("German", "German"),
            ("Spanish", "Spanish"),
            ("Italian", "Italian"),
            ("Dutch", "Dutch"),
            ("Portuguese", "Portuguese")
        ])
    ),
    p!("hyphenationExceptions", "Hyphenation", "", "Exceptions (comma separated)", text),
    // Performance & Storage
    p!("pluginsFolder", "Performance & Storage", "Plug-ins", "Additional Plug-ins Folder", text),
    p!("scratchPrimary", "Performance & Storage", "Scratch Disks", "Primary", text),
    p!("scratchSecondary", "Performance & Storage", "Scratch Disks", "Secondary", text),
    // User Interface
    p!(
        "uiBrightness",
        "User Interface",
        "",
        "Brightness",
        choice(&[("dark", "Dark"), ("mediumDark", "Medium Dark"), ("mediumLight", "Medium Light"), ("light", "Light")])
    ),
    p!("appearanceMode", "User Interface", "", "Appearance Mode", choice(APPEARANCE_MODES)),
    p!("darkTheme", "User Interface", "", "Dark Theme", choice(DARK_THEMES)),
    p!("lightTheme", "User Interface", "", "Light Theme", choice(LIGHT_THEMES)),
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
    p!("appendConverted", "File Handling", "Files", "Mark Older Files as [Converted] When Opened", bool),
    // Native saves (`document.save {compress}`)
    p!("useCompression", "File Handling", "Files", "Use Compression", bool),
    // The W/H link of the Transform panel, the Properties panel and the Control bar
    p!("constrainProportions", "General", "Transform Panel", "Constrain Width and Height Proportions", bool),
];

pub fn spec(key: &str) -> Option<&'static PrefSpec> {
    PREF_SPECS.iter().find(|s| s.key == key)
}

/// Preferences kept as one object with a command of their own instead of [`PREF_SPECS`] rows (they
/// aren't in the Preferences dialog): the Eyedropper Options (`eyedropper.setOptions`) and the
/// Perspective Grid Options (`perspective.widget.options`). `prefs.get`
/// and `prefs.set` take them by key (a partial object updates what it names) and `prefs.reset`
/// without a category resets them.
pub const PREF_GROUPS: &[&str] = &["eyedropper", "perspectiveWidget"];

/// Validate a value for preference group `key` against `current`.
fn validate_group(key: &str, current: &Value, v: &Value) -> std::result::Result<Value, String> {
    match key {
        "eyedropper" => {
            let cur: super::EyedropperOptions = serde_json::from_value(current.clone()).unwrap_or_default();
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

/// The appearance mode (`dark` or `light`) a brightness belongs to, and its theme preference key.
pub fn brightness_family(id: &str) -> Option<(&'static str, &'static str)> {
    if DARK_THEMES.iter().any(|(v, _)| *v == id) {
        Some(("dark", "darkTheme"))
    } else if LIGHT_THEMES.iter().any(|(v, _)| *v == id) {
        Some(("light", "lightTheme"))
    } else {
        None
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

impl Prefs {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Set several validated values at once; on any error nothing changes. The legacy
    /// `uiBrightness` also selects its appearance mode and theme slot, unless the same call sets
    /// those explicitly (as the Preferences dialog does).
    pub fn set_values(&mut self, values: &Map<String, Value>) -> std::result::Result<(), String> {
        let mut obj = self.to_json();
        for (k, v) in values {
            obj[k.as_str()] = if PREF_GROUPS.contains(&k.as_str()) { validate_group(k, &obj[k.as_str()], v)? } else { validate(k, v)? };
        }
        if values.contains_key("uiBrightness")
            && let Some(b) = obj.get("uiBrightness").and_then(Value::as_str)
            && let Some((mode, slot)) = brightness_family(b)
        {
            let b = b.to_string();
            if !values.contains_key("appearanceMode") {
                obj["appearanceMode"] = json!(mode);
            }
            if !values.contains_key(slot) {
                obj[slot] = json!(b);
            }
        }
        *self = serde_json::from_value(obj).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Show brightness `id` (`dark`, `mediumDark`, `mediumLight`, `light`): the UI Brightness menu
    /// fixes the appearance mode to its family and stores it as that family's theme.
    pub fn select_brightness(&mut self, id: &str) -> std::result::Result<(), String> {
        let (mode, _) = brightness_family(id).ok_or_else(|| format!("unknown brightness `{id}`"))?;
        self.ui_brightness = id.to_string();
        self.appearance_mode = mode.to_string();
        if mode == "dark" {
            self.dark_theme = id.to_string();
        } else {
            self.light_theme = id.to_string();
        }
        Ok(())
    }

    /// Preferences as saved (`to_json`), read leniently: unknown keys are ignored and missing ones
    /// take their defaults. Files saved before appearance modes had only `uiBrightness`: they keep
    /// that brightness as a fixed Dark or Light mode instead of switching to another look.
    pub fn from_saved(mut v: Value) -> Prefs {
        if let Some(o) = v.as_object_mut()
            && !o.contains_key("appearanceMode")
            && let Some(b) = o.get("uiBrightness").and_then(Value::as_str).map(str::to_string)
            && let Some((mode, slot)) = brightness_family(&b)
        {
            o.insert("appearanceMode".into(), json!(mode));
            o.insert(slot.into(), json!(b));
        }
        serde_json::from_value(v).unwrap_or_default()
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
}

impl Session {
    /// Replace the preferences and push the ones with engine-side consumers into open documents
    /// and the renderer (grid, history depth, render threads).
    pub fn apply_prefs(&mut self, p: Prefs) {
        let grid_changed = p.gridline_every != self.prefs.gridline_every || p.grid_subdivisions != self.prefs.grid_subdivisions;
        let history_changed = p.history_states != self.prefs.history_states;
        let tile_edge_changed = p.pattern_tile_edge_color != self.prefs.pattern_tile_edge_color;
        let fonts_folder_changed = p.fonts_folder != self.prefs.fonts_folder;
        let hyphen_exceptions_changed = p.hyphenation_exceptions != self.prefs.hyphenation_exceptions;
        self.prefs = p;
        if fonts_folder_changed {
            let folder = self.prefs.fonts_folder.trim();
            vectorcraft_text::set_user_font_dirs(if folder.is_empty() { vec![] } else { vec![folder.into()] });
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
                let over = st.history.undo.len().saturating_sub(st.history.limit);
                st.history.undo.drain(..over);
            }
            // Documents in pattern editing mode redraw their tile edge.
            if tile_edge_changed && st.doc.pattern_edit.is_some() {
                st.revision += 1;
            }
            if grid_changed {
                let d = std::sync::Arc::make_mut(&mut st.doc);
                d.grid.spacing = self.prefs.gridline_every;
                d.grid.subdivisions = self.prefs.grid_subdivisions;
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
            "{key, value} or {values: {key: value, …}} set preferences (validated; see prefs.list; lengths in pt or strings with a unit; unitsGeneral also sets the active document's units, one undo step; the eyedropper group takes a partial object, as eyedropper.setOptions)",
            always,
            set
        ),
        cmd!(query "prefs.reset", "Reset Preferences", [], None, "{category?} reset one category (or everything) to defaults", always, reset),
        cmd!(query "prefs.list", "List Preferences", [], None, "{} → [{key, category, section, label, kind, value, …}] every preference", always, list),
    ]
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
    let mut next = s.prefs.clone();
    next.set_values(&values).map_err(|e| bad("prefs.set", e))?;
    s.apply_prefs(next);
    // Units ▸ General is also the open document's units (as Document Setup sets them).
    if values.contains_key("unitsGeneral") && s.active().is_some() {
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
