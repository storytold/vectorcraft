//! The Print dialog's model ([`PrintSettings`]): camelCase JSON whose every field is optional and
//! falls back to its default, the checks that refuse values out of range, and the inks of a
//! separation ([`PrintInk`]).

use serde::{Deserialize, Serialize};
use vectorcraft_color::cms::{self, Intent, ProfileKind};

use crate::settings::{Choice, choice, within};
use crate::{BleedSettings, MarkSettings, PdfError};

choice! {
    /// Which artboards print.
    PrintArtboards {
        All = "all", "All";
        /// The 1-based artboards [`PrintSettings::range`] names.
        Range = "range", "Range";
        /// All the art as one page, whatever the artboards.
        Ignore = "ignore", "Ignore Artboards";
    } default All
}

/// The paper (portrait sizes; [`Media::Custom`] is [`PrintSettings::width`] ×
/// [`PrintSettings::height`]).
///
/// `B4` and `B5` keep their ISO 216 dimensions (250 × 353 mm and 176 × 250 mm) so settings saved
/// before the Japanese papers existed keep their paper, and their dialogs say "(ISO)". Japanese
/// printing uses the JIS sizes, which are separate choices: `b4 (jis)` 257 × 364 mm, `b5 (jis)`
/// 182 × 257 mm and `b6 (jis)` 128 × 182 mm. The other Japanese papers are はがき
/// (`japanese postcard`, 100 × 148 mm), 往復はがき (`japanese double postcard`, 148 × 200 mm),
/// 封筒 長形3号/長形4号/角形2号/角形3号 and 四六判 (`yonroku-ban`, 127 × 188 mm).
///
/// Written out rather than through the `choice!` macro because a paper also comes from a person:
/// [`Media::from_name`] accepts a JSON id or a dialog label, any case and spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Media {
    #[default]
    Letter,
    Legal,
    Tabloid,
    A3,
    A4,
    A5,
    B4,
    B5,
    A6,
    B4Jis,
    B5Jis,
    B6Jis,
    JapanesePostcard,
    JapaneseDoublePostcard,
    JapaneseEnvelope3,
    JapaneseEnvelope4,
    Kaku2,
    Kaku3,
    YonrokuBan,
    Custom,
}

impl Media {
    /// The paper size in points, portrait (`None` for a custom size).
    pub fn size(self) -> Option<(f64, f64)> {
        let mm = |w: f64, h: f64| Some((w * 72.0 / 25.4, h * 72.0 / 25.4));
        match self {
            Self::Letter => Some((612.0, 792.0)),
            Self::Legal => Some((612.0, 1008.0)),
            Self::Tabloid => Some((792.0, 1224.0)),
            Self::A3 => mm(297.0, 420.0),
            Self::A4 => mm(210.0, 297.0),
            Self::A5 => mm(148.0, 210.0),
            Self::A6 => mm(105.0, 148.0),
            // ISO 216.
            Self::B4 => mm(250.0, 353.0),
            Self::B5 => mm(176.0, 250.0),
            // JIS B (JIS X 6310): the Japanese B sizes print a little larger than ISO's.
            Self::B4Jis => mm(257.0, 364.0),
            Self::B5Jis => mm(182.0, 257.0),
            Self::B6Jis => mm(128.0, 182.0),
            // はがき, 往復はがき, 封筒 長形3号/長形4号, 角形2号/角形3号, 四六判.
            Self::JapanesePostcard => mm(100.0, 148.0),
            Self::JapaneseDoublePostcard => mm(148.0, 200.0),
            Self::JapaneseEnvelope3 => mm(120.0, 235.0),
            Self::JapaneseEnvelope4 => mm(90.0, 225.0),
            Self::Kaku2 => mm(240.0, 332.0),
            Self::Kaku3 => mm(216.0, 277.0),
            Self::YonrokuBan => mm(127.0, 188.0),
            Self::Custom => None,
        }
    }

    /// The paper an id or a dialog label names, ignoring case and anything but letters and digits:
    /// `"b5 (jis)"`, `"B5 (JIS)"` and `"b5jis"` all mean [`Media::B5Jis`], and `"b5"` stays the ISO
    /// size that settings and presets already use. `None` for a name that is no paper.
    pub fn from_name(name: &str) -> Option<Self> {
        let norm = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect::<String>();
        let n = norm(name);
        if n.is_empty() {
            return None;
        }
        let at = |names: &'static [&'static str]| names.iter().position(|name| norm(name) == n);
        at(Self::IDS).or_else(|| at(Self::LABELS)).and_then(|i| Self::ALL.get(i).copied())
    }
}

impl Choice for Media {
    const ALL: &'static [Self] = &[
        Self::Letter,
        Self::Legal,
        Self::Tabloid,
        Self::A3,
        Self::A4,
        Self::A5,
        Self::B4,
        Self::B5,
        Self::A6,
        Self::B4Jis,
        Self::B5Jis,
        Self::B6Jis,
        Self::JapanesePostcard,
        Self::JapaneseDoublePostcard,
        Self::JapaneseEnvelope3,
        Self::JapaneseEnvelope4,
        Self::Kaku2,
        Self::Kaku3,
        Self::YonrokuBan,
        Self::Custom,
    ];
    const IDS: &'static [&'static str] = &[
        "letter",
        "legal",
        "tabloid",
        "a3",
        "a4",
        "a5",
        "b4",
        "b5",
        "a6",
        "b4 (jis)",
        "b5 (jis)",
        "b6 (jis)",
        "japanese postcard",
        "japanese double postcard",
        "japanese envelope #3",
        "japanese envelope #4",
        "kaku #2",
        "kaku #3",
        "yonroku-ban",
        "custom",
    ];
    const LABELS: &'static [&'static str] = &[
        "Letter",
        "Legal",
        "Tabloid",
        "A3",
        "A4",
        "A5",
        "B4 (ISO)",
        "B5 (ISO)",
        "A6",
        "B4 (JIS)",
        "B5 (JIS)",
        "B6 (JIS)",
        "Japanese Postcard",
        "Japanese Double Postcard",
        "Japanese Envelope #3",
        "Japanese Envelope #4",
        "Kaku #2",
        "Kaku #3",
        "Yonroku-ban",
        "Custom",
    ];
}

impl Media {
    /// The label shown in the Print dialogs.
    pub fn label(self) -> &'static str {
        Self::ALL.iter().position(|m| *m == self).and_then(|i| Self::LABELS.get(i)).copied().unwrap_or_default()
    }

    /// The JSON id.
    pub fn id(self) -> &'static str {
        Self::ALL.iter().position(|m| *m == self).and_then(|i| Self::IDS.get(i)).copied().unwrap_or_default()
    }
}

impl Serialize for Media {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for Media {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let name = String::deserialize(deserializer)?;
        Self::from_name(&name).ok_or_else(|| D::Error::custom(format!("unknown paper `{name}` ({})", Self::IDS.join("|"))))
    }
}

choice! {
    /// How the paper is turned.
    Orientation {
        Portrait = "portrait", "Portrait";
        Landscape = "landscape", "Landscape";
        /// Portrait, the art upside down.
        PortraitFlipped = "portraitFlipped", "Portrait (flipped)";
        /// Landscape, the art upside down.
        LandscapeFlipped = "landscapeFlipped", "Landscape (flipped)";
    } default Portrait
}

impl Orientation {
    pub fn landscape(self) -> bool {
        matches!(self, Self::Landscape | Self::LandscapeFlipped)
    }

    pub fn flipped(self) -> bool {
        matches!(self, Self::PortraitFlipped | Self::LandscapeFlipped)
    }
}

choice! {
    /// Which layers print (template layers never do).
    PrintLayers {
        VisiblePrintable = "visiblePrintable", "Visible & Printable Layers";
        Visible = "visible", "Visible Layers";
        /// Hidden layers and layers whose Print option is off too.
        All = "all", "All Layers";
    } default VisiblePrintable
}

choice! {
    /// The point of the printed area that sits on the same point of the imageable area.
    Origin {
        TopLeft = "topLeft", "Top Left";
        Top = "top", "Top";
        TopRight = "topRight", "Top Right";
        Left = "left", "Left";
        Center = "center", "Center";
        Right = "right", "Right";
        BottomLeft = "bottomLeft", "Bottom Left";
        Bottom = "bottom", "Bottom";
        BottomRight = "bottomRight", "Bottom Right";
    } default Center
}

impl Origin {
    /// Where the point lies across and down a box (0, ½ or 1).
    pub fn factors(self) -> (f64, f64) {
        match self {
            Self::TopLeft => (0.0, 0.0),
            Self::Top => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::Left => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::Right => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::Bottom => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }
}

choice! {
    /// How the art is sized on the paper.
    PrintScaling {
        None = "none", "Do Not Scale";
        /// As large as fits the imageable area with its marks, keeping its proportions.
        Fit = "fit", "Fit to Page";
        /// [`PrintSettings::scale`].
        Custom = "custom", "Custom Scale";
        /// Pages the size of the paper, overlapping by [`PrintSettings::overlap`].
        TileFull = "tileFull", "Tile Full Pages";
        /// Pages the size of the imageable area.
        TileImageable = "tileImageable", "Tile Imageable Areas";
    } default None
}

impl PrintScaling {
    pub fn tiles(self) -> bool {
        matches!(self, Self::TileFull | Self::TileImageable)
    }
}

choice! {
    /// Composite colour, or one page per ink.
    OutputMode {
        Composite = "composite", "Composite";
        Separations = "separations", "Separations";
    } default Composite
}

choice! {
    /// Which side of the film the image is read from: Down mirrors the page.
    Emulsion {
        Up = "up", "Up (right reading)";
        Down = "down", "Down (right reading)";
    } default Up
}

choice! {
    /// Positive, or a film negative (the page inverted).
    PrintImage {
        Positive = "positive", "Positive";
        Negative = "negative", "Negative";
    } default Positive
}

choice! {
    /// Which fonts are sent to the printer.
    FontDownload {
        None = "none", "None";
        Subset = "subset", "Subset";
        Complete = "complete", "Complete";
    } default Subset
}

/// File → Print settings (the dialog's sections; what [`vectorcraft_doc::Document::print_setup`]
/// stores).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintSettings {
    // General.
    /// Copies of the whole job (1–999).
    pub copies: u32,
    /// Copies as whole sets (1 2 3, 1 2 3), not page by page (1 1, 2 2, 3 3).
    pub collate: bool,
    /// Last page first.
    pub reverse: bool,
    pub artboards: PrintArtboards,
    /// 1-based artboards with [`PrintArtboards::Range`] (`"1-3, 5"`).
    pub range: String,
    /// Leave out artboards with no art that prints.
    pub skip_blank: bool,
    pub media: Media,
    /// Custom paper size (pt).
    pub width: f64,
    pub height: f64,
    pub orientation: Orientation,
    /// Turn the paper to the art's orientation, artboard by artboard (the orientation is ignored).
    pub auto_rotate: bool,
    /// Turn the page a quarter turn on the paper (roll-fed devices).
    pub transverse: bool,
    pub print_layers: PrintLayers,
    pub placement: Placement,
    pub scaling: PrintScaling,
    /// Width and height in percent, with custom scale and tiling.
    pub scale: PrintScale,
    /// How much tiles overlap (pt).
    pub overlap: f64,
    /// 1-based tiles to print (`"1-3, 5"`, numbered across then down); empty: every tile.
    pub tile_range: String,
    /// The device's unprintable margin around the paper (pt): the imageable area is inside it.
    pub margin: f64,
    // Marks and Bleed.
    pub marks: MarkSettings,
    pub bleed: BleedSettings,
    pub output: PrintOutput,
    pub graphics: PrintGraphics,
    pub color: PrintColor,
    /// Where the Print Tiling tool put the pages.
    pub tile_origin: TileOrigin,
    // Advanced.
    pub advanced: PrintAdvanced,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            copies: 1,
            collate: true,
            reverse: false,
            artboards: PrintArtboards::All,
            range: String::new(),
            skip_blank: false,
            media: Media::Letter,
            width: 612.0,
            height: 792.0,
            orientation: Orientation::Portrait,
            auto_rotate: true,
            transverse: false,
            print_layers: PrintLayers::VisiblePrintable,
            placement: Placement::default(),
            scaling: PrintScaling::None,
            scale: PrintScale::default(),
            overlap: 0.0,
            tile_range: String::new(),
            margin: 0.0,
            marks: MarkSettings::default(),
            // The document's bleed (Document Setup) unless other values are given.
            bleed: BleedSettings { use_document: true, ..Default::default() },
            output: PrintOutput::default(),
            graphics: PrintGraphics::default(),
            color: PrintColor::default(),
            tile_origin: TileOrigin::default(),
            advanced: PrintAdvanced::default(),
        }
    }
}

/// Where the printed area (the artboard with its bleed and marks) sits on the paper.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Placement {
    pub origin: Origin,
    /// Moves it right and down from there (pt).
    pub x: f64,
    pub y: f64,
}

/// Custom scale, in percent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintScale {
    pub width: f64,
    pub height: f64,
}

impl Default for PrintScale {
    fn default() -> Self {
        Self { width: 100.0, height: 100.0 }
    }
}

/// The Output section.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintOutput {
    pub mode: OutputMode,
    pub emulsion: Emulsion,
    pub image: PrintImage,
    /// Separate spot colours into process inks: no spot plates.
    pub spots_to_process: bool,
    /// Per-ink options, by ink name; inks not listed print with their defaults.
    pub inks: Vec<InkSettings>,
}

/// One ink's options in the ink list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InkSettings {
    /// "Cyan", "Magenta", "Yellow", "Black" or a spot swatch's name.
    pub name: String,
    /// Print this ink's plate.
    pub print: bool,
    /// Halftone screen ruling (lpi); `None`: the ink's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f64>,
    /// Halftone screen angle (degrees); `None`: the ink's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
}

impl Default for InkSettings {
    fn default() -> Self {
        Self { name: String::new(), print: true, frequency: None, angle: None }
    }
}

/// An ink of a separation, with its options resolved ([`super::inks`]).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintInk {
    pub name: String,
    pub spot: bool,
    pub print: bool,
    pub frequency: f64,
    pub angle: f64,
}

/// The default halftone screen ruling (lpi).
pub const DEFAULT_FREQUENCY: f64 = 60.0;

impl PrintInk {
    /// The default screen angle of ink `name`: the classic process angles (Cyan 15°, Magenta 75°,
    /// Yellow 0°, Black 45°), 45° for spot inks.
    pub fn default_angle(name: &str) -> f64 {
        match name {
            "Cyan" => 15.0,
            "Magenta" => 75.0,
            "Yellow" => 0.0,
            _ => 45.0,
        }
    }
}

/// The Graphics section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintGraphics {
    /// Let the device pick the flatness curves are drawn with.
    pub auto_flatness: bool,
    /// The flatness without it (device pixels, 0.2–100: higher is faster and coarser).
    pub flatness: f64,
    pub fonts: FontDownload,
}

impl Default for PrintGraphics {
    fn default() -> Self {
        Self { auto_flatness: true, flatness: 1.0, fonts: FontDownload::Subset }
    }
}

/// The Color Management section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintColor {
    /// How colours outside the press gamut are separated.
    pub intent: Intent,
    /// CMYK colours print with their own values; off, they go through the colour settings too.
    pub preserve_numbers: bool,
    /// The printer profile (an RGB or CMYK profile by name): composite output converts colours to
    /// it with the intent (CMYK colours keep their numbers with `preserve_numbers`), and
    /// separations separate with it when it is a CMYK one. Empty: colours print as they are.
    pub profile: String,
}

impl Default for PrintColor {
    fn default() -> Self {
        Self { intent: Intent::RelativeColorimetric, preserve_numbers: true, profile: String::new() }
    }
}

choice! {
    /// What composite output does with overprinting fills and strokes (separations always
    /// honour them).
    PrintOverprints {
        Preserve = "preserve", "Preserve";
        /// They knock out.
        Discard = "discard", "Discard";
        /// Composited as Overprint Preview shows them.
        Simulate = "simulate", "Simulate";
    } default Preserve
}

/// The Advanced section (with [`PrintSettings::margin`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintAdvanced {
    /// Composite pages print as one image each of the art, at the document's raster effects
    /// resolution (the app renders them before printing).
    pub print_as_bitmap: bool,
    pub overprints: PrintOverprints,
    /// The transparency flattener preset transparency is flattened with (a built-in or saved
    /// one, by name; the app flattens before printing). Empty: transparency prints live.
    pub flattener_preset: String,
}

/// Where the Print Tiling tool put the pages (View → Show Print Tiling).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TileOrigin {
    /// The tool placed the pages: the top-left corner of the first page's imageable area is at
    /// (`x`, `y`) from the top-left corner of the art that prints (the artboard, or all the art
    /// with artboards ignored), in document points. It replaces the placement, and tiles start
    /// there (and go on to the left and up as far as the art does). Off: the placement places the
    /// art and tiles start at the top-left corner of the art with its bleed and marks.
    pub placed: bool,
    pub x: f64,
    pub y: f64,
}

/// Most copies of a job.
pub const MAX_COPIES: u32 = 999;

impl PrintSettings {
    /// Refuse values out of range (ranges and tile overlap are checked against the document when
    /// it prints).
    pub fn check(&self) -> Result<(), PdfError> {
        within("copies", self.copies as f64, 1.0, MAX_COPIES as f64, "")?;
        if self.media == Media::Custom {
            within("width", self.width, 72.0, 14_400.0, " pt")?;
            within("height", self.height, 72.0, 14_400.0, " pt")?;
        }
        within("placement.x", self.placement.x, -14_400.0, 14_400.0, " pt")?;
        within("placement.y", self.placement.y, -14_400.0, 14_400.0, " pt")?;
        within("scale.width", self.scale.width, 1.0, 1000.0, "%")?;
        within("scale.height", self.scale.height, 1.0, 1000.0, "%")?;
        within("overlap", self.overlap, 0.0, 720.0, " pt")?;
        within("margin", self.margin, 0.0, 144.0, " pt")?;
        within("tileOrigin.x", self.tile_origin.x, -14_400.0, 14_400.0, " pt")?;
        within("tileOrigin.y", self.tile_origin.y, -14_400.0, 14_400.0, " pt")?;
        self.marks.check()?;
        self.bleed.check()?;
        within("graphics.flatness", self.graphics.flatness, 0.2, 100.0, "")?;
        let profile = self.color.profile.trim();
        if !profile.is_empty() && cms::profile(profile).is_none_or(|p| p.kind == ProfileKind::Gray) {
            return Err(PdfError::BadSetting(format!("color.profile: no RGB or CMYK profile is called “{profile}”")));
        }
        for ink in &self.output.inks {
            if let Some(f) = ink.frequency {
                within(&format!("output.inks[{}].frequency", ink.name), f, 1.0, 1000.0, " lpi")?;
            }
            if let Some(a) = ink.angle {
                within(&format!("output.inks[{}].angle", ink.name), a, -360.0, 360.0, "°")?;
            }
        }
        Ok(())
    }

    /// The paper size in points, portrait as chosen (before orientation).
    pub fn paper(&self) -> (f64, f64) {
        self.media.size().unwrap_or((self.width, self.height))
    }
}
