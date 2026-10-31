use super::{Result, Session, TEMPLATE_FORMATS, UNSUPPORTED, Unsupported, Value, bad, dxf, eps, json, metafile, native, pdf, svg, text, unsupported};

/// One option a format's encoder reads from the export params.
#[derive(Clone, Copy, Debug)]
pub struct FormatOption {
    pub name: &'static str,
    /// JSON type: `number`, `integer`, `boolean`, `string`, `array` or `object`.
    pub ty: &'static str,
    /// The default as a JSON literal (`"1"`, `"false"`, `"null"`).
    pub default: &'static str,
    pub description: &'static str,
}

/// One file format: how `document.open` recognises it and what `document.export` writes.
#[derive(Clone, Copy, Debug)]
pub struct Format {
    /// The `format` param value (`svg`, `png`, …).
    pub id: &'static str,
    /// Open/save dialog filter name.
    pub label: &'static str,
    /// Lower-case extensions without the dot; the first is the one exports use.
    pub extensions: &'static [&'static str],
    pub mime: &'static str,
    pub read: bool,
    pub write: bool,
    /// Pixels (export `scale` applies) rather than vectors.
    pub raster: bool,
    /// Options the encoder reads (writable formats).
    pub options: &'static [FormatOption],
}

impl Format {
    pub fn to_json(&self) -> Value {
        let options: serde_json::Map<String, Value> = self
            .options
            .iter()
            .map(|o| {
                let default: Value = serde_json::from_str(o.default).unwrap_or(Value::Null);
                (o.name.to_string(), json!({"type": o.ty, "default": default, "description": o.description}))
            })
            .collect();
        json!({
            "id": self.id,
            "label": self.label,
            "extensions": self.extensions,
            "mime": self.mime,
            "read": self.read,
            "write": self.write,
            "raster": self.raster,
            "options": options,
        })
    }
}

pub(super) const ARTBOARD: FormatOption = FormatOption {
    name: "artboard",
    ty: "integer",
    default: "0",
    description: "0-based artboard to export (`artboards: [i]` or `range: \"n\"` naming one artboard work too)",
};

pub(super) const ARTBOARDS: FormatOption = FormatOption {
    name: "artboards",
    ty: "array",
    default: "null",
    description: "0-based artboards (PDF and AI: one page each, default all; SVG, and raster formats with useArtboards: one file each)",
};

pub(super) const RANGE: FormatOption = FormatOption {
    name: "range",
    ty: "string",
    default: "null",
    description: "1-based artboards such as \"1-3, 5\", or \"all\" (wins over artboards and artboard)",
};

const SCALE: FormatOption = FormatOption { name: "scale", ty: "number", default: "1", description: "pixels per point (0.01–64)" };

const WIDTH: FormatOption = FormatOption {
    name: "width",
    ty: "number",
    default: "null",
    description: "the width of each file in pixels: each artboard, rect or art bounds gets its own scale (0.01–64 pixels per point, stored in the file except in GIF, WebP and Targa); wins over height, ppi and scale",
};

const HEIGHT: FormatOption = FormatOption {
    name: "height",
    ty: "number",
    default: "null",
    description: "the height of each file in pixels, as width; wins over ppi and scale",
};

const QUALITY: FormatOption =
    FormatOption { name: "quality", ty: "integer", default: "90", description: "JPEG quality 0–100 (the JPEG Options dialog shows 0–10)" };

pub(super) const USE_ARTBOARDS: FormatOption = FormatOption {
    name: "useArtboards",
    ty: "boolean",
    default: "null",
    description: "true: every chosen artboard (default all), one file each named <file>-<artboard>.<ext> (PDF and AI: one page each); false: the bounds of the visible art",
};

const RECT: FormatOption = FormatOption {
    name: "rect",
    ty: "array",
    default: "null",
    description: "[x, y, width, height] in document points, y down: exports that area of the page (no artboard needed, the document is not changed); not with artboard, artboards, range or useArtboards",
};

const PPI: FormatOption = FormatOption {
    name: "ppi",
    ty: "number",
    default: "72",
    description: "resolution in pixels per inch (72 = one pixel per point; 150, 300…), stored in the file except in GIF, WebP and Targa; wins over scale",
};

const BACKGROUND: FormatOption = FormatOption {
    name: "background",
    ty: "string",
    default: "\"transparent\"",
    description: "transparent, white, black or a colour such as \"#ff8800\" (JPEG: white when transparent)",
};

const ANTI_ALIAS: FormatOption = FormatOption {
    name: "antiAlias",
    ty: "string",
    default: "\"art\"",
    description: "none (hard pixel edges), art (smooth edges) or type (smooth, text snapped to whole pixels)",
};

const INTERLACED: FormatOption =
    FormatOption { name: "interlaced", ty: "boolean", default: "false", description: "Adam7 interlacing (the image builds up while it loads)" };

const COLOR_MODEL: FormatOption = FormatOption {
    name: "colorModel",
    ty: "string",
    default: "\"rgb\"",
    description: "rgb, cmyk (ink amounts in the working CMYK space: CMYK colours keep their inks) or gray",
};

const METHOD: FormatOption = FormatOption {
    name: "method",
    ty: "string",
    default: "\"baseline\"",
    description: "baseline (standard), optimized (smaller Huffman tables) or progressive (builds up in scans)",
};

const SCANS: FormatOption = FormatOption { name: "scans", ty: "integer", default: "3", description: "progressive scans, 3–5" };

const EMBED_ICC: FormatOption = FormatOption {
    name: "embedIcc",
    ty: "boolean",
    default: "true",
    description: "embed the colour profile: sRGB (RGB), the working CMYK space (CMYK) or gray with sRGB's tone curve; PNG: mark the file as sRGB (an sRGB chunk)",
};

const IMAGE_MAP: FormatOption = FormatOption {
    name: "imageMap",
    ty: "string",
    default: "\"none\"",
    description: "none, client (an HTML page with <map>, <stem>.html) or server (an NCSA <stem>.map): the areas of objects with a URL and an Image Map shape (attributes.set)",
};

const COLORS: FormatOption =
    FormatOption { name: "colors", ty: "integer", default: "256", description: "most palette entries, 2–256 (the transparent one included)" };

const REDUCTION: FormatOption = FormatOption {
    name: "reduction",
    ty: "string",
    default: "\"selective\"",
    description: "the palette: perceptual, selective (also keeps rarer colours, snaps near web colours), adaptive (most used), web (web-safe), blackWhite or gray; art with that many colours or fewer keeps them exactly",
};

const DITHER: FormatOption = FormatOption {
    name: "dither",
    ty: "string",
    default: "\"diffusion\"",
    description: "none, diffusion (Floyd–Steinberg), pattern (8×8 ordered) or noise",
};

const DITHER_AMOUNT: FormatOption = FormatOption { name: "ditherAmount", ty: "integer", default: "100", description: "dither strength 0–100" };

const TRANSPARENCY: FormatOption = FormatOption {
    name: "transparency",
    ty: "boolean",
    default: "true",
    description: "pixels under half opacity become one transparent entry (false: everything is blended over the matte)",
};

const MATTE: FormatOption = FormatOption {
    name: "matte",
    ty: "string",
    default: "\"white\"",
    description: "the colour partly transparent edges are blended over (\"#rrggbb\", white, black), or none (they keep their colour)",
};

const LOSSLESS: FormatOption = FormatOption {
    name: "lossless",
    ty: "boolean",
    default: "true",
    description: "lossless WebP; false asks for lossy, which isn't available yet (written lossless, with a warning)",
};

const WEBP_QUALITY: FormatOption =
    FormatOption { name: "quality", ty: "integer", default: "90", description: "lossy quality 0–100 (for when lossy WebP is available)" };

/// The options of the palette formats (PNG-8, GIF).
const PALETTE_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    RECT,
    PPI,
    SCALE,
    WIDTH,
    HEIGHT,
    BACKGROUND,
    ANTI_ALIAS,
    INTERLACED,
    COLORS,
    REDUCTION,
    DITHER,
    DITHER_AMOUNT,
    TRANSPARENCY,
    MATTE,
];

const LZW: FormatOption = FormatOption { name: "lzw", ty: "boolean", default: "true", description: "LZW compression (lossless, smaller files)" };

const BYTE_ORDER: FormatOption = FormatOption {
    name: "byteOrder",
    ty: "string",
    default: "\"little\"",
    description: "the order of the bytes in the file's numbers: little (II, as on PCs) or big (MM)",
};

const TIFF_OPTIONS: &[FormatOption] =
    &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, RECT, PPI, SCALE, WIDTH, HEIGHT, BACKGROUND, ANTI_ALIAS, COLOR_MODEL, LZW, BYTE_ORDER, EMBED_ICC];

const BMP_COLOR_MODEL: FormatOption =
    FormatOption { name: "colorModel", ty: "string", default: "\"rgb\"", description: "rgb or gray (greys at every depth)" };

const BMP_DEPTH: FormatOption = FormatOption {
    name: "depth",
    ty: "integer",
    default: "24",
    description: "bits per pixel: 1 (black and white), 4 or 8 (a palette by reduction and dither), 16, 24, or 32 (keeps transparency); the others are flattened on white",
};

const FILE_FORMAT: FormatOption = FormatOption {
    name: "fileFormat",
    ty: "string",
    default: "\"windows\"",
    description: "windows, or os2 (1, 4, 8 or 24 bits, uncompressed, rows bottom-up)",
};

const RLE: FormatOption =
    FormatOption { name: "rle", ty: "boolean", default: "false", description: "run-length encode a 4- or 8-bit windows bitmap (RLE4, RLE8)" };

const FLIP_ROWS: FormatOption = FormatOption {
    name: "flipRows",
    ty: "boolean",
    default: "false",
    description: "rows top-down (a negative height) instead of bottom-up; not with rle or os2",
};

const BMP_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    RECT,
    PPI,
    SCALE,
    WIDTH,
    HEIGHT,
    BACKGROUND,
    ANTI_ALIAS,
    BMP_COLOR_MODEL,
    BMP_DEPTH,
    FILE_FORMAT,
    RLE,
    FLIP_ROWS,
    REDUCTION,
    DITHER,
    DITHER_AMOUNT,
];

const TGA_DEPTH: FormatOption = FormatOption {
    name: "depth",
    ty: "integer",
    default: "24",
    description: "bits per pixel: 16 (one-bit alpha), 24 (flattened on white) or 32 (keeps transparency)",
};

const TGA_OPTIONS: &[FormatOption] = &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, RECT, PPI, SCALE, WIDTH, HEIGHT, BACKGROUND, ANTI_ALIAS, TGA_DEPTH];

const PSD_LAYERS: FormatOption = FormatOption {
    name: "layers",
    ty: "boolean",
    default: "true",
    description: "write layers: each top-level layer a pixel layer with its opacity and blend mode (a background colour is a Background layer); false: one flat image, on white where transparent",
};

const MAX_EDITABILITY: FormatOption = FormatOption {
    name: "maxEditability",
    ty: "boolean",
    default: "false",
    description: "with layers: layers and sublayers become groups and every object a layer of its own, named as the Layers panel names it (text by its text); clipping, masked or knockout layers stay one layer",
};

const PSD_HIDDEN_LAYERS: FormatOption = FormatOption {
    name: "hiddenLayers",
    ty: "boolean",
    default: "false",
    description: "with layers: hidden layers (and hidden objects) are written as hidden layers instead of left out",
};

const PSD_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    RECT,
    PPI,
    SCALE,
    WIDTH,
    HEIGHT,
    BACKGROUND,
    ANTI_ALIAS,
    COLOR_MODEL,
    PSD_LAYERS,
    MAX_EDITABILITY,
    PSD_HIDDEN_LAYERS,
    EMBED_ICC,
];

/// A format `document.open` reads but nothing writes yet.
const fn reader(id: &'static str, label: &'static str, extensions: &'static [&'static str], mime: &'static str, raster: bool) -> Format {
    Format { id, label, extensions, mime, read: true, write: false, raster, options: &[] }
}

/// Every format VectorCraft reads or writes. Append-only: new formats go at the end.
pub const FORMATS: &[Format] = &[
    Format {
        id: "vectorcraft",
        label: "VectorCraft",
        extensions: &[vectorcraft_format::EXTENSION, vectorcraft_format::LEGACY_EXTENSION],
        mime: "application/json",
        read: true,
        write: true,
        raster: false,
        options: native::OPTIONS,
    },
    Format { id: "svg", label: "SVG", extensions: &["svg"], mime: "image/svg+xml", read: true, write: true, raster: false, options: svg::OPTIONS },
    Format {
        id: "svgz",
        label: "SVG Compressed",
        extensions: &["svgz"],
        mime: "image/svg+xml",
        read: true,
        write: true,
        raster: false,
        options: svg::OPTIONS,
    },
    Format { id: "pdf", label: "PDF", extensions: &["pdf"], mime: "application/pdf", read: true, write: true, raster: false, options: pdf::OPTIONS },
    Format {
        id: "ai",
        label: "PDF-compatible .ai",
        extensions: &["ai"],
        mime: "application/pdf",
        read: true,
        // Save As writes it; exports write the same file.
        write: true,
        raster: false,
        options: native::AI_OPTIONS,
    },
    Format {
        id: "ait",
        label: "PDF-compatible .ait template",
        extensions: &["ait"],
        mime: "application/pdf",
        read: true,
        // Save as Template writes it, with the .ai file's options; exports don't.
        write: false,
        raster: false,
        options: native::AI_OPTIONS,
    },
    Format {
        id: "png",
        label: "PNG",
        extensions: &["png"],
        mime: "image/png",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, RECT, PPI, SCALE, WIDTH, HEIGHT, BACKGROUND, ANTI_ALIAS, INTERLACED, EMBED_ICC],
    },
    Format {
        id: "jpg",
        label: "JPEG",
        extensions: &["jpg", "jpeg"],
        mime: "image/jpeg",
        read: true,
        write: true,
        raster: true,
        options: &[
            ARTBOARD,
            ARTBOARDS,
            RANGE,
            USE_ARTBOARDS,
            RECT,
            PPI,
            SCALE,
            WIDTH,
            HEIGHT,
            BACKGROUND,
            ANTI_ALIAS,
            QUALITY,
            COLOR_MODEL,
            METHOD,
            SCANS,
            EMBED_ICC,
            IMAGE_MAP,
        ],
    },
    Format { id: "gif", label: "GIF", extensions: &["gif"], mime: "image/gif", read: true, write: true, raster: true, options: PALETTE_OPTIONS },
    Format {
        id: "webp",
        label: "WebP",
        extensions: &["webp"],
        mime: "image/webp",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, RECT, PPI, SCALE, WIDTH, HEIGHT, BACKGROUND, ANTI_ALIAS, LOSSLESS, WEBP_QUALITY],
    },
    Format {
        id: "tiff",
        label: "TIFF",
        extensions: &["tif", "tiff"],
        mime: "image/tiff",
        read: true,
        write: true,
        raster: true,
        options: TIFF_OPTIONS,
    },
    Format { id: "bmp", label: "BMP", extensions: &["bmp"], mime: "image/bmp", read: true, write: true, raster: true, options: BMP_OPTIONS },
    Format {
        id: "template",
        label: "VectorCraft Template",
        extensions: &["vctemplate"],
        mime: "application/json",
        read: true,
        write: true,
        raster: false,
        options: native::OPTIONS,
    },
    Format { id: "png8", label: "PNG-8", extensions: &["png"], mime: "image/png", read: false, write: true, raster: true, options: PALETTE_OPTIONS },
    Format { id: "txt", label: "Text", extensions: TEXT_EXTS, mime: "text/plain", read: false, write: true, raster: false, options: text::OPTIONS },
    Format { id: "dxf", label: "DXF", extensions: &["dxf"], mime: "image/vnd.dxf", read: true, write: true, raster: false, options: dxf::OPTIONS },
    Format {
        id: "eps",
        label: "EPS",
        extensions: &["eps", "epsf"],
        mime: "application/postscript",
        read: true,
        write: true,
        raster: false,
        options: eps::OPTIONS,
    },
    Format { id: "emf", label: "EMF", extensions: &["emf"], mime: "image/emf", read: true, write: true, raster: false, options: metafile::OPTIONS },
    Format { id: "wmf", label: "WMF", extensions: &["wmf"], mime: "image/wmf", read: true, write: true, raster: false, options: metafile::OPTIONS },
    Format {
        id: "tga",
        label: "Targa",
        extensions: &["tga", "vda", "icb", "vst"],
        mime: "image/x-tga",
        read: true,
        write: true,
        raster: true,
        options: TGA_OPTIONS,
    },
    Format { id: "psd", label: "PSD", extensions: &["psd"], mime: "image/x-psd", read: true, write: true, raster: true, options: PSD_OPTIONS },
    // Affinity Photo's `.afphoto` opens by its content but isn't listed: Finder shouldn't offer a
    // vector app for a raster editor's documents.
    reader("affinity", "Affinity", &["af", "afdesign", "afpub"], "application/x-affinity", false),
    // Photoshop's large document format: read like a PSD (its merged image), never written.
    reader("psb", "PSB", &["psb"], "image/x-psb", true),
    // PostScript programs, read as EPS files are; Print to PostScript writes them, exports don't.
    reader("ps", "PostScript", &["ps"], "application/postscript", false),
    // ZSoft PCX images, read by `pcxread` and stored as PNG; exports don't write them.
    reader("pcx", "PCX", &["pcx"], "image/x-pcx", true),
];

/// Every extension `document.open` reads (the "All readable files" filter of open dialogs).
pub const OPEN_EXTS: &[&str] = &[
    "vectorcraft",
    "drawcraft",
    "svg",
    "svgz",
    "pdf",
    "ai",
    "ait",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "tif",
    "tiff",
    "bmp",
    "tga",
    "vda",
    "icb",
    "vst",
    "pcx",
    "psd",
    "psb",
    "vctemplate",
    "dxf",
    "emf",
    "wmf",
    "eps",
    "epsf",
    "ps",
    "af",
    "afdesign",
    "afpub",
];

/// Extensions in [`OPEN_EXTS`] the packaging declares no type for: Photoshop documents belong to
/// raster editors, so Finder and file managers don't offer a vector app for them, and there is no
/// `.epsf`, `.ps`, Targa or PCX type (File › Open and Place still read them all). On Linux the
/// desktop entry's EPS type (shared-mime-info's) also covers `.epsf`, so file managers offer
/// VectorCraft for it.
pub const UNASSOCIATED_EXTS: &[&str] = &["psd", "psb", "epsf", "ps", "tga", "vda", "icb", "vst", "pcx"];

/// The extension that picks each writable format when exporting (the format's first; PNG-8 shares
/// `.png` with PNG, so `.png` comes once), in [`FORMATS`] order.
pub fn export_extensions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    let added = plugin_formats();
    for e in FORMATS.iter().chain(added.iter().copied()).filter(|f| f.write).filter_map(|f| f.extensions.first()) {
        if !v.contains(e) {
            v.push(e);
        }
    }
    v
}

/// Text files: File → Place sets them as area type (Text Import Options).
pub const TEXT_EXTS: &[&str] = &["txt"];

/// RTF files: File → Place sets them as area type with their formatting.
pub const RTF_EXTS: &[&str] = &["rtf"];

/// Word documents: File → Place sets them as area type with their formatting.
pub const DOCX_EXTS: &[&str] = &["docx"];

/// Every extension File → Place reads: [`OPEN_EXTS`], [`TEXT_EXTS`], [`RTF_EXTS`] and
/// [`DOCX_EXTS`].
pub const PLACE_EXTS: &[&str] = &[
    "vectorcraft",
    "drawcraft",
    "svg",
    "svgz",
    "pdf",
    "ai",
    "ait",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "tif",
    "tiff",
    "bmp",
    "tga",
    "vda",
    "icb",
    "vst",
    "pcx",
    "psd",
    "psb",
    "vctemplate",
    "dxf",
    "emf",
    "wmf",
    "eps",
    "epsf",
    "ps",
    "af",
    "afdesign",
    "afpub",
    "txt",
    "rtf",
    "docx",
];

/// One dialog filter per readable format.
fn format_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    FORMATS.iter().filter(|f| f.read).map(|f| (f.label, f.extensions))
}

/// Open-dialog filters: "All readable files" first, then one per readable format, then swatch
/// libraries (which open in the library panel), Libraries panel libraries, flattener, PDF and print
/// presets (imported), scripts (checked without running them) and plug-ins (installed).
pub fn open_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("All readable files", OPEN_EXTS))
        .chain(format_filters())
        .chain(std::iter::once(("Swatch libraries", crate::cmd::swatchlib::LIBRARY_EXTS)))
        .chain(std::iter::once(("Libraries", crate::cmd::library::LIBRARY_EXTS)))
        .chain(std::iter::once(("Flattener presets", crate::cmd::flatten::PRESET_EXTS)))
        .chain(std::iter::once(("PDF presets", crate::cmd::pdfcmds::PRESET_EXTS)))
        .chain(std::iter::once(("Print presets", crate::cmd::printpresets::PRESET_EXTS)))
        .chain(std::iter::once(("Shortcut sets", crate::cmd::shortcutset::EXTS)))
        .chain(std::iter::once(("Scripts", crate::cmd::scripts::EXTS)))
        .chain(std::iter::once(("Plug-ins", crate::cmd::plugin::EXTS)))
}

/// Open-dialog filters for Swatch Libraries and Graphic Style Libraries Other Library… and Locate
/// Library…. The library formats come first so Linux file portals (which grey out files that don't
/// match the default filter) still list `.ase` and the other library extensions; then the same
/// filters as [`open_filters`] so a document can be picked for its swatches or graphic styles.
pub fn library_open_filters(
    library_label: &'static str,
    library_exts: &'static [&'static str],
) -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once((library_label, library_exts)).chain(open_filters())
}

/// File → Place dialog filters: "All placeable files", then one per placeable format, then text,
/// RTF and Word.
pub fn place_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("All placeable files", PLACE_EXTS))
        .chain(format_filters().filter(|(_, exts)| exts.iter().all(|ext| PLACE_EXTS.contains(ext))))
        .chain(std::iter::once(("Text", TEXT_EXTS)))
        .chain(std::iter::once(("RTF", RTF_EXTS)))
        .chain(std::iter::once(("Word", DOCX_EXTS)))
}

/// A format by id or extension (any case, leading dot allowed; `jpeg` finds `jpg`): a built-in
/// one, else one an exporter plug-in adds ([`plugin_formats`]). A built-in format keeps its
/// extension, so an exporter that lists it too is found by its format id.
pub fn format(id_or_ext: &str) -> Option<&'static Format> {
    let k = id_or_ext.trim_start_matches('.').to_ascii_lowercase();
    FORMATS.iter().find(|f| f.id == k).or_else(|| FORMATS.iter().find(|f| f.extensions.contains(&k.as_str()))).or_else(|| {
        let added = plugin_formats();
        added.iter().find(|f| f.id.eq_ignore_ascii_case(&k)).or_else(|| added.iter().find(|f| f.extensions.contains(&k.as_str()))).copied()
    })
}

/// The formats the installed exporter plug-ins add ([`vectorcraft_plugins::export`]), by plug-in
/// id: write only, with the plug-in's name as the label and its extensions. Each is made once for
/// each installation of its plug-in, its strings interned, so formats live as long as the catalogues
/// that hold them.
pub fn plugin_formats() -> Vec<&'static Format> {
    use vectorcraft_plugins::{export, registry};
    static MADE: std::sync::Mutex<Vec<(u64, &'static Format)>> = std::sync::Mutex::new(Vec::new());
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    export::installed()
        .iter()
        .map(|p| {
            if let Some((_, f)) = made.iter().find(|(serial, _)| *serial == p.serial()) {
                return *f;
            }
            let m = p.manifest();
            let extensions: Vec<&'static str> = m.extensions.iter().map(|e| registry::intern(e)).collect();
            let f: &'static Format = Box::leak(Box::new(Format {
                id: registry::intern(&export::format_id(&m.id)),
                label: registry::intern(&m.name),
                extensions: Box::leak(extensions.into_boxed_slice()),
                mime: "application/octet-stream",
                read: false,
                write: true,
                raster: false,
                options: &[],
            }));
            made.push((p.serial(), f));
            f
        })
        .collect()
}

/// The lower-case extension of a file name or path (empty when it has none).
pub fn extension(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// A file name or path without its folder and extension (`/a/Poster.svg` → `Poster`).
pub fn file_stem(name: &str) -> String {
    std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string())
}

/// What a `format` export of `doc`, whose file is `path`, writes when it isn't `doc` itself: a
/// document that only has the Untitled-N name New gave it is titled after its file, as its tab is
/// (the title PDF, PNG and SVG metadata carry). A title of its own stays, and native files keep
/// the stored title.
pub fn has_own_title(doc: &vectorcraft_doc::Document) -> bool {
    !doc.title.trim().is_empty() && !doc.title.strip_prefix("Untitled-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub fn titled_for_export(doc: &vectorcraft_doc::Document, path: Option<&str>, format: &str) -> Option<vectorcraft_doc::Document> {
    let path = path.filter(|_| !has_own_title(doc) && !matches!(format, "vectorcraft" | "template" | "ai"))?;
    let mut d = doc.clone();
    d.title = file_stem(path);
    Some(d)
}

/// The format a file name's extension names.
pub fn format_for_name(name: &str) -> Option<&'static Format> {
    Some(extension(name)).filter(|e| !e.is_empty()).and_then(|e| format(&e))
}

/// The format to write: `format` (an id or extension), else the path's extension, else PNG. For a
/// `.ait` template it returns an error that names `file.saveAsTemplate`, which writes it.
pub fn writable_format(format_param: Option<&str>, path: Option<&str>) -> std::result::Result<&'static Format, String> {
    // Formats that can't be written say what to use instead.
    let unknown = |what: String, key: &str| unsupported(key).map_or(what, |u| u.hint.to_string());
    let f = match (format_param, path.map(extension).filter(|e| !e.is_empty())) {
        (Some(f), _) => format(f).ok_or_else(|| unknown(format!("unknown format `{f}` (see document.formats)"), f))?,
        (None, Some(e)) => format(&e).ok_or_else(|| unknown(format!("unknown extension `.{e}`: pass `format` (see document.formats)"), &e))?,
        (None, None) => format("png").ok_or("no PNG encoder")?,
    };
    if f.write {
        Ok(f)
    } else if TEMPLATE_FORMATS.contains(&f.id) {
        Err(format!("{} files are written by file.saveAsTemplate", f.label))
    } else {
        Err(format!("{} files can be opened but not written (see document.formats)", f.label))
    }
}

/// [`writable_format`] for command `cmd`.
pub(super) fn writable(cmd: &str, format_param: Option<&str>, path: Option<&str>) -> Result<&'static Format> {
    writable_format(format_param, path).map_err(|e| bad(cmd, e))
}

pub(super) fn formats(_: &mut Session, _: &Value) -> Result<Value> {
    let added = plugin_formats();
    let all = || FORMATS.iter().chain(added.iter().copied());
    let ids = |pick: fn(&Format) -> bool| all().filter(|f| pick(f)).map(|f| f.id).collect::<Vec<_>>();
    let listed = |f: &Format| {
        let mut v = f.to_json();
        if let Some(id) = vectorcraft_plugins::export::plugin_id(f.id) {
            v["plugin"] = json!(id);
        }
        v
    };
    Ok(json!({
        "formats": all().map(listed).collect::<Vec<_>>(),
        "readable": ids(|f| f.read),
        "writable": ids(|f| f.write),
        "openExtensions": OPEN_EXTS,
        "unsupported": UNSUPPORTED.iter().map(Unsupported::to_json).collect::<Vec<_>>(),
    }))
}
