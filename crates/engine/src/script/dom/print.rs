//! Printing in the script object model: `Document.print`, `PrintOptions` and its option classes,
//! `app.printPresetsList`, `app.printerList` and `app.PPDFileList`, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintJobOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintColorManagementOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintColorSeparationOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintCoordinateOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintFlattenerOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintFontOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintPageMarksOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintPaperOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PrintPostScriptOptions/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Application/> describe.
//!
//! An option object is an object of a host class whose properties the run keeps. A `PrintOptions`
//! holds its option objects by reference, so a change made to one after it was assigned is read
//! when `print` is called. Every value is checked when it is assigned; `print` maps the whole tree
//! into `file.print`'s settings before anything is written.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `print` writes a file: `jobOptions.file` names it, a `File` the script may write
//!   ([`super::super::files`]). PostScript is written when the name ends with `.ps` or
//!   `postScriptOptions.postScriptLevel` is set, a print-ready PDF otherwise. The PostScript is
//!   level 2 when `postScriptLevel` is not assigned, the default it reads as. Printing to a printer
//!   from a script is not available yet, so `print` without a file throws an error.
//! - Only the properties a script assigned apply; the others keep the settings `print` starts
//!   from: the preset's when `printPreset` is assigned, otherwise the document's print settings
//!   (`print.setup`), which `print` leaves as they are. A property that was not assigned reads as
//!   the default its help page gives.
//! - A property the help pages list that is not mapped accepts the default its help page gives
//!   and throws an error that says it is not available yet at any other value.
//! - `fitToPage` set to true wins over the scale factors and the tiling mode.
//! - With `printAllArtboards` set to false, `artboardRange` picks the artboards; without a range
//!   every artboard prints. With `printAllArtboards` not assigned, an assigned `artboardRange`
//!   replaces the range of the settings `print` starts from, which prints when those settings
//!   print a range.
//! - An assigned `printArea` prints by artboard when the settings `print` starts from ignore the
//!   artboards: the artboards an assigned `artboardRange` names, otherwise each artboard.
//! - The format follows the name `jobOptions.file` gives, also when that name is a link to a file
//!   with another extension.
//! - With one of `horizontalScale` and `verticalScale` assigned, the other keeps its value in the
//!   settings `print` starts from.
//! - `bitmapResolution` is `file.print`'s `advanced.bitmapResolution`: 0 prints at the document's
//!   raster effects resolution, and 1 to 9600 ppi at that resolution; `print` throws, before
//!   anything is written, for a value between 0 and 1.
//! - `print` throws, before anything is written, when an assigned `artboardRange` names an artboard
//!   the document does not have, also with `printAllArtboards` true; when `printPreset` or
//!   `flattenerPreset` is not the name of a preset (any case); when `colorManagementOptions.name` is
//!   not the name of an RGB or CMYK color profile (any case); and when `paperOptions.name` is not
//!   the id or label of a paper `file.print` knows, or names a paper other than `custom` while
//!   `width` or `height` is assigned above 0. An assigned name that is empty or only spaces,
//!   or that holds `/`, `\` or a control character (also at either end), throws too. Each checked
//!   name goes to `file.print` without the spaces around it. `printerName` and `PPDName` take only the empty
//!   string.
//! - A run makes at most [`MAX_OPTION_OBJECTS`] option objects.
//! - `app.printerList` is not available yet, and `app.PPDFileList` is empty: VectorCraft prints
//!   without PPD files.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_color::cms;
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember, host_ref, host_ref_parts};

use super::save_open::{exactly, file_path, plain, refused};
use super::{
    Answer, Doc, Host, Link, MAX_LISTED, MAX_TEXT_BYTES, arg, command, current, float, get, given, later, method, named, num, on_doc, text, this_doc,
    whole, with_doc,
};
use crate::cmd::flatten::FlattenOptions;

/// The most option objects one run makes.
pub const MAX_OPTION_OBJECTS: usize = 256;
/// The largest scale factor `file.print` takes, in percent.
pub const MAX_PRINT_SCALE: f64 = 1000.0;
/// The error of `print` without `jobOptions.file`.
pub const NO_PRINTER: &str = "Printing to a printer is not available to scripts yet: set jobOptions.file to print to a file";
/// The error for an option object from an earlier run.
pub const STALE_OPTIONS: &str = "Stale print options";

/// The option objects of the run: each one's class, as an index into [`CLASSES`], and the
/// properties the script assigned.
#[derive(Default)]
pub(in crate::script) struct State {
    objects: RefCell<Vec<Options>>,
}

struct Options {
    class: usize,
    set: Vec<(&'static str, Data)>,
}

const PRINT_COLOR_PROFILE: &[&str] = &["CUSTOMPROFILE", "PRINTERPROFILE", "OLDSTYLEPROFILE", "SOURCEPROFILE"];
const PRINT_COLOR_INTENT: &[&str] = &["ABSOLUTECOLORIMETRIC", "PERCEPTUALINTENT", "RELATIVECOLORIMETRIC", "SATURATIONINTENT"];
const PRINT_COLOR_SEPARATION_MODE: &[&str] = &["COMPOSITE", "HOSTBASEDSEPARATION", "INRIPSEPARATION"];
const PRINT_ORIENTATION: &[&str] = &["AUTOROTATE", "LANDSCAPE", "PORTRAIT", "REVERSELANDSCAPE", "REVERSEPORTRAIT"];
const PRINT_POSITION: &[&str] = &[
    "TRANSLATEBOTTOM",
    "TRANSLATEBOTTOMLEFT",
    "TRANSLATEBOTTOMRIGHT",
    "TRANSLATECENTER",
    "TRANSLATELEFT",
    "TRANSLATERIGHT",
    "TRANSLATETOP",
    "TRANSLATETOPLEFT",
    "TRANSLATETOPRIGHT",
];
const PRINT_TILING: &[&str] = &["TILEFULLPAGES", "TILESINGLEFULLPAGE", "TILEIMAGEABLEAREAS"];
const PRINT_ARTWORK_DESIGNATION: &[&str] = &["ALLLAYERS", "VISIBLELAYERS", "VISIBLEPRINTABLELAYERS"];
const PRINTING_BOUNDS: &[&str] = &["ARTBOARDBOUNDS", "ARTWORKBOUNDS"];
const PRINT_FONT_DOWNLOAD_MODE: &[&str] = &["DOWNLOADNONE", "DOWNLOADCOMPLETE", "DOWNLOADSUBSET"];
const FONT_SUBSTITUTION_POLICY: &[&str] = &["SUBSTITUTEDEVICE", "SUBSTITUTEOBLIQUE", "SUBSTITUTETINT"];
const PAGE_MARKS_TYPES: &[&str] = &["Japanese", "Roman"];
const PDF_OVERPRINT: &[&str] = &["DISCARDPDFOVERPRINT", "PRESERVEPDFOVERPRINT"];
const POSTSCRIPT_IMAGE_COMPRESSION: &[&str] = &["IMAGECOMPRESSIONNONE", "JPEG", "RLE"];
const POSTSCRIPT_LEVEL: &[&str] = &["PSLEVEL1", "PSLEVEL2", "PSLEVEL3"];

/// The enumerations these members use, as `dom`'s are made. `PDFOverprint` comes from
/// [`super::save_open::ENUMS`].
pub(super) const ENUMS: &[(&str, &[&str])] = &[
    ("PrintColorProfile", PRINT_COLOR_PROFILE),
    ("PrintColorIntent", PRINT_COLOR_INTENT),
    ("PrintColorSeparationMode", PRINT_COLOR_SEPARATION_MODE),
    ("PrintOrientation", PRINT_ORIENTATION),
    ("PrintPosition", PRINT_POSITION),
    ("PrintTiling", PRINT_TILING),
    ("PrintArtworkDesignation", PRINT_ARTWORK_DESIGNATION),
    ("PrintingBounds", PRINTING_BOUNDS),
    ("PrintFontDownloadMode", PRINT_FONT_DOWNLOAD_MODE),
    ("FontSubstitutionPolicy", FONT_SUBSTITUTION_POLICY),
    ("PageMarksTypes", PAGE_MARKS_TYPES),
    ("PostScriptImageCompressionType", POSTSCRIPT_IMAGE_COMPRESSION),
    ("PrinterPostScriptLevelEnum", POSTSCRIPT_LEVEL),
];

/// What a property takes.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    /// A finite number from the first to the second.
    Number(f64, f64),
    /// A whole number from the first to the second.
    Whole(u64, u64),
    Text,
    /// A value of the enumeration (its name and members); only the members in the last list are
    /// available.
    Choice(&'static str, &'static [&'static str], &'static [&'static str]),
    /// An option object of the class named.
    Child(&'static str),
    File,
}

/// The default a property reads as before it is assigned.
#[derive(Clone, Copy)]
enum Start {
    None,
    Bool(bool),
    Num(f64),
    Text(&'static str),
    /// An enumerated value, by its member name.
    Member(&'static str),
}

impl Start {
    fn value(self, kind: Kind) -> Data {
        match (self, kind) {
            (Start::Bool(b), _) => Data::Bool(b),
            (Start::Num(n), _) => num(n),
            (Start::Text(s), _) => text(s),
            (Start::Member(m), Kind::Choice(e, ..)) => text(format!("{e}.{m}")),
            _ => Data::Undefined,
        }
    }

    fn matches(self, kind: Kind, d: &Data) -> bool {
        match (self, d) {
            (Start::Num(a), Data::Number(b)) => (a - b).abs() < 1e-9,
            (Start::None, _) => false,
            _ => self.value(kind) == *d,
        }
    }
}

/// A property: its name, what it takes, its default, and whether `print` maps it.
struct Field {
    name: &'static str,
    kind: Kind,
    start: Start,
    mapped: bool,
}

const fn f(name: &'static str, kind: Kind, start: Start) -> Field {
    Field { name, kind, start, mapped: true }
}

const fn kept(name: &'static str, kind: Kind, start: Start) -> Field {
    Field { name, kind, start, mapped: false }
}

struct Class {
    name: &'static str,
    fields: &'static [Field],
}

const fn choice(e: &'static str, all: &'static [&'static str], ok: &'static [&'static str]) -> Kind {
    Kind::Choice(e, all, ok)
}

const PRINT_OPTIONS: Class = Class {
    name: "PrintOptions",
    fields: &[
        f("colorManagementOptions", Kind::Child("PrintColorManagementOptions"), Start::None),
        f("colorSeparationOptions", Kind::Child("PrintColorSeparationOptions"), Start::None),
        f("coordinateOptions", Kind::Child("PrintCoordinateOptions"), Start::None),
        f("flattenerOptions", Kind::Child("PrintFlattenerOptions"), Start::None),
        f("flattenerPreset", Kind::Text, Start::None),
        f("fontOptions", Kind::Child("PrintFontOptions"), Start::None),
        f("jobOptions", Kind::Child("PrintJobOptions"), Start::None),
        f("pageMarksOptions", Kind::Child("PrintPageMarksOptions"), Start::None),
        f("paperOptions", Kind::Child("PrintPaperOptions"), Start::None),
        f("postScriptOptions", Kind::Child("PrintPostScriptOptions"), Start::None),
        kept("PPDName", Kind::Text, Start::Text("")),
        kept("printerName", Kind::Text, Start::Text("")),
        f("printPreset", Kind::Text, Start::None),
    ],
};

const JOB: Class = Class {
    name: "PrintJobOptions",
    fields: &[
        f("artboardRange", Kind::Text, Start::Text("1-")),
        f("bitmapResolution", Kind::Number(0.0, vectorcraft_pdf::MAX_BITMAP_RESOLUTION), Start::Num(0.0)),
        f("collate", Kind::Bool, Start::Bool(false)),
        f("copies", Kind::Whole(1, 999), Start::Num(1.0)),
        f(
            "designation",
            choice("PrintArtworkDesignation", PRINT_ARTWORK_DESIGNATION, PRINT_ARTWORK_DESIGNATION),
            Start::Member("VISIBLEPRINTABLELAYERS"),
        ),
        f("file", Kind::File, Start::None),
        kept("name", Kind::Text, Start::None),
        f("printAllArtboards", Kind::Bool, Start::Bool(true)),
        f("printArea", choice("PrintingBounds", PRINTING_BOUNDS, &["ARTBOARDBOUNDS"]), Start::Member("ARTBOARDBOUNDS")),
        f("printAsBitmap", Kind::Bool, Start::Bool(false)),
        f("reversePages", Kind::Bool, Start::Bool(false)),
    ],
};

const COLOR: Class = Class {
    name: "PrintColorManagementOptions",
    fields: &[
        f("colorProfileMode", choice("PrintColorProfile", PRINT_COLOR_PROFILE, &["CUSTOMPROFILE", "SOURCEPROFILE"]), Start::Member("SOURCEPROFILE")),
        f("intent", choice("PrintColorIntent", PRINT_COLOR_INTENT, PRINT_COLOR_INTENT), Start::Member("RELATIVECOLORIMETRIC")),
        f("name", Kind::Text, Start::None),
    ],
};

const SEPARATION: Class = Class {
    name: "PrintColorSeparationOptions",
    fields: &[
        f(
            "colorSeparationMode",
            choice("PrintColorSeparationMode", PRINT_COLOR_SEPARATION_MODE, &["COMPOSITE", "HOSTBASEDSEPARATION"]),
            Start::Member("COMPOSITE"),
        ),
        f("convertSpotColors", Kind::Bool, Start::Bool(false)),
        kept("inkList", Kind::Text, Start::None),
        f("overPrintBlack", Kind::Bool, Start::Bool(false)),
    ],
};

const COORDINATE: Class = Class {
    name: "PrintCoordinateOptions",
    fields: &[
        f("emulsion", Kind::Bool, Start::Bool(false)),
        f("fitToPage", Kind::Bool, Start::Bool(false)),
        f("horizontalScale", Kind::Number(1.0, 10_000.0), Start::Num(100.0)),
        f("orientation", choice("PrintOrientation", PRINT_ORIENTATION, PRINT_ORIENTATION), Start::Member("PORTRAIT")),
        f("position", choice("PrintPosition", PRINT_POSITION, PRINT_POSITION), Start::Member("TRANSLATECENTER")),
        f("tiling", choice("PrintTiling", PRINT_TILING, PRINT_TILING), Start::Member("TILESINGLEFULLPAGE")),
        f("verticalScale", Kind::Number(1.0, 10_000.0), Start::Num(100.0)),
    ],
};

const FLATTENER: Class = Class {
    name: "PrintFlattenerOptions",
    fields: &[
        kept("clipComplexRegions", Kind::Bool, Start::Bool(false)),
        kept("convertStrokesToOutlines", Kind::Bool, Start::Bool(false)),
        kept("convertTextToOutlines", Kind::Bool, Start::Bool(false)),
        kept("flatteningBalance", Kind::Number(0.0, 100.0), Start::Num(100.0)),
        kept("gradientResolution", Kind::Number(1.0, 9600.0), Start::Num(300.0)),
        f("overprint", choice("PDFOverprint", PDF_OVERPRINT, PDF_OVERPRINT), Start::Member("PRESERVEPDFOVERPRINT")),
        kept("rasterizationResolution", Kind::Number(1.0, 9600.0), Start::Num(300.0)),
    ],
};

const FONT: Class = Class {
    name: "PrintFontOptions",
    fields: &[
        kept("downloadFonts", choice("PrintFontDownloadMode", PRINT_FONT_DOWNLOAD_MODE, &[]), Start::Member("DOWNLOADSUBSET")),
        kept("fontSubstitution", choice("FontSubstitutionPolicy", FONT_SUBSTITUTION_POLICY, &[]), Start::Member("SUBSTITUTEOBLIQUE")),
    ],
};

const MARKS: Class = Class {
    name: "PrintPageMarksOptions",
    fields: &[
        kept("bleedOffsetRect", Kind::Text, Start::None),
        f("colorBars", Kind::Bool, Start::Bool(false)),
        kept("marksOffsetRect", Kind::Text, Start::None),
        f("pageInfoMarks", Kind::Bool, Start::Bool(false)),
        f("pageMarksType", choice("PageMarksTypes", PAGE_MARKS_TYPES, PAGE_MARKS_TYPES), Start::Member("Roman")),
        f("registrationMarks", Kind::Bool, Start::Bool(false)),
        f("trimMarks", Kind::Bool, Start::Bool(false)),
        f("trimMarksWeight", Kind::Number(0.0, 1000.0), Start::Num(0.125)),
    ],
};

const PAPER: Class = Class {
    name: "PrintPaperOptions",
    fields: &[
        f("height", Kind::Number(0.0, 14_400.0), Start::Num(0.0)),
        f("name", Kind::Text, Start::None),
        kept("offset", Kind::Number(0.0, 14_400.0), Start::Num(0.0)),
        f("transverse", Kind::Bool, Start::Bool(false)),
        f("width", Kind::Number(0.0, 14_400.0), Start::Num(0.0)),
    ],
};

const POSTSCRIPT: Class = Class {
    name: "PrintPostScriptOptions",
    fields: &[
        kept("binaryPrinting", Kind::Bool, Start::Bool(false)),
        kept("compatibleShading", Kind::Bool, Start::Bool(false)),
        kept("forceContinuousTone", Kind::Bool, Start::Bool(false)),
        kept("imageCompression", choice("PostScriptImageCompressionType", POSTSCRIPT_IMAGE_COMPRESSION, &[]), Start::Member("IMAGECOMPRESSIONNONE")),
        f("negativePrinting", Kind::Bool, Start::Bool(false)),
        f("postScriptLevel", choice("PrinterPostScriptLevelEnum", POSTSCRIPT_LEVEL, &["PSLEVEL2", "PSLEVEL3"]), Start::Member("PSLEVEL2")),
        kept("shadingResolution", Kind::Number(1.0, 9600.0), Start::Num(300.0)),
    ],
};

/// The option classes, `PrintOptions` first.
const CLASSES: &[Class] = &[PRINT_OPTIONS, JOB, COLOR, SEPARATION, COORDINATE, FLATTENER, FONT, MARKS, PAPER, POSTSCRIPT];

fn class_index(name: &str) -> Option<usize> {
    CLASSES.iter().position(|c| c.name == name)
}

fn not_available(class: &str, name: &str) -> HostError {
    HostError::Error(format!("{class}.{name} is not available yet"))
}

/// A new option object of class `class`.
fn create(h: &Host, class: usize) -> Answer {
    let mut objects = h.dom.printing.objects.borrow_mut();
    if objects.len() >= MAX_OPTION_OBJECTS {
        return Err(HostError::Range(format!("a script can make at most {MAX_OPTION_OBJECTS} print option objects in a run")));
    }
    objects.push(Options { class, set: Vec::new() });
    let slot = u64::try_from(objects.len() - 1).unwrap_or(u64::MAX);
    Ok(host_ref(CLASSES[class].name, &[float(h.run), float(slot)]))
}

/// The slot of the option object `d` of class `class`, made in this run.
fn slot_of(h: &Host, d: &Data, class: usize) -> std::result::Result<usize, HostError> {
    let name = CLASSES[class].name;
    let wrong = || HostError::Type(format!("this is not a {name}"));
    let (c, handle) = host_ref_parts(d).ok_or_else(wrong)?;
    if c != name {
        return Err(wrong());
    }
    let [run, slot] = handle.as_slice() else { return Err(wrong()) };
    let stale = || HostError::Error(STALE_OPTIONS.into());
    if whole(*run) != Some(h.run) {
        return Err(stale());
    }
    let slot = whole(*slot).and_then(|s| usize::try_from(s).ok()).ok_or_else(stale)?;
    match h.dom.printing.objects.borrow().get(slot) {
        Some(o) if o.class == class => Ok(slot),
        _ => Err(stale()),
    }
}

/// The value `d` assigned to `field` of `class` stores, or `None` when it unsets it.
fn checked(h: &Host, class: &Class, field: &Field, d: &Data) -> std::result::Result<Option<Data>, HostError> {
    let what = format!("{}.{}", class.name, field.name);
    if matches!(d, Data::Undefined) || (matches!(d, Data::Null) && matches!(field.kind, Kind::Child(_) | Kind::File)) {
        return Ok(None);
    }
    if !field.mapped {
        return if field.start.matches(field.kind, d) { Ok(None) } else { Err(not_available(class.name, field.name)) };
    }
    match field.kind {
        Kind::Bool => match d {
            Data::Bool(_) => {}
            _ => return Err(HostError::Type(format!("{what} must be true or false"))),
        },
        Kind::Number(lo, hi) => match d {
            Data::Number(n) if n.is_finite() => {
                if !(lo..=hi).contains(n) {
                    return Err(HostError::Range(format!("{what} must be from {lo} to {hi}")));
                }
            }
            _ => return Err(HostError::Type(format!("{what} must be a finite number"))),
        },
        Kind::Whole(lo, hi) => match d {
            Data::Number(n) => {
                if !whole(*n).is_some_and(|n| (lo..=hi).contains(&n)) {
                    return Err(HostError::Range(format!("{what} must be a whole number from {lo} to {hi}")));
                }
            }
            _ => return Err(HostError::Type(format!("{what} must be a number"))),
        },
        Kind::Text => match d {
            Data::String(s) if s.len() > MAX_TEXT_BYTES => return Err(HostError::Range(format!("{what} is too long"))),
            Data::String(_) => {}
            _ => return Err(HostError::Type(format!("{what} must be a string"))),
        },
        Kind::Choice(e, all, ok) => {
            let bad = || HostError::Type(format!("{what} must be a {e} value"));
            let Data::String(s) = d else { return Err(bad()) };
            let m = s.strip_prefix(e).and_then(|r| r.strip_prefix('.')).filter(|m| all.contains(m)).ok_or_else(bad)?;
            if !ok.contains(&m) {
                return Err(HostError::Error(format!("{e}.{m} is not available yet")));
            }
        }
        Kind::Child(c) => {
            let index = class_index(c).ok_or_else(|| HostError::Type(format!("{what} must be a {c}")))?;
            slot_of(h, d, index).map_err(|e| match e {
                HostError::Type(_) => HostError::Type(format!("{what} must be a {c}")),
                e => e,
            })?;
        }
        Kind::File => {
            if vectorcraft_js::file_ref_path(d).is_none() {
                return Err(HostError::Type(format!("{what} must be a File")));
            }
        }
    }
    Ok(Some(d.clone()))
}

fn read(h: &Host, class: usize, field: usize, this: &Data) -> Answer {
    let slot = slot_of(h, this, class)?;
    let spec = &CLASSES[class].fields[field];
    let stored = h.dom.printing.objects.borrow()[slot].set.iter().find(|(k, _)| *k == spec.name).map(|(_, v)| v.clone());
    if let Some(v) = stored {
        return Ok(v);
    }
    match spec.kind {
        Kind::Child(c) => {
            let index = class_index(c).ok_or_else(|| HostError::Type(format!("no option class {c}")))?;
            let child = create(h, index)?;
            h.dom.printing.objects.borrow_mut()[slot].set.push((spec.name, child.clone()));
            Ok(child)
        }
        kind => Ok(spec.start.value(kind)),
    }
}

fn write(h: &Host, class: usize, field: usize, this: &Data, d: &Data) -> Answer {
    let slot = slot_of(h, this, class)?;
    let c = &CLASSES[class];
    let spec = &c.fields[field];
    let value = checked(h, c, spec, d)?;
    let mut objects = h.dom.printing.objects.borrow_mut();
    let set = &mut objects[slot].set;
    set.retain(|(k, _)| *k != spec.name);
    if let Some(v) = value {
        set.push((spec.name, v));
    }
    Ok(Data::Undefined)
}

fn property(link: &Link, class: usize, field: usize) -> HostMember {
    let (l, m) = (link.clone(), link.clone());
    let get: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16)?;
        read(&h, class, field, arg(&a, 0))
    });
    let set: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&m)?;
        h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
        write(&h, class, field, arg(&a, 0), arg(&a, 1))
    });
    HostMember::Property { get, set }
}

// ---- Mapping to file.print ----

/// What a `PrintOptions` asks `file.print` for.
#[derive(Default)]
struct Job {
    preset: Option<String>,
    file: Option<Data>,
    level: Option<u64>,
    /// The assigned `artboardRange`, checked whether or not it picks the artboards.
    range: Option<String>,
    /// The assigned names [`check_names`] checks before they go into `settings`: the flattener
    /// preset, the color profile and the paper.
    flattener: Option<String>,
    profile: Option<String>,
    paper: Option<String>,
    /// A paper `width` or `height` above 0 is assigned.
    paper_size: bool,
    /// `colorProfileMode` is `SOURCEPROFILE`: the colors print as they are, whatever `name` says.
    source: bool,
    /// `printArea` is assigned: the artboards print even when the base settings ignore them.
    print_area: bool,
    /// The groups of `settings`: the top level and each nested object by its key.
    top: Map<String, Value>,
    groups: Vec<(&'static str, Map<String, Value>)>,
}

impl Job {
    fn group(&mut self, key: &'static str) -> &mut Map<String, Value> {
        let i = match self.groups.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                self.groups.push((key, Map::new()));
                self.groups.len() - 1
            }
        };
        &mut self.groups[i].1
    }

    fn settings(&mut self) -> Map<String, Value> {
        let mut s = std::mem::take(&mut self.top);
        for (k, g) in self.groups.drain(..) {
            if !g.is_empty() {
                s.insert(k.into(), Value::Object(g));
            }
        }
        s
    }
}

/// The assigned properties of the option object in `slot`.
fn assigned(h: &Host, slot: usize) -> Vec<(&'static str, Data)> {
    h.dom.printing.objects.borrow()[slot].set.clone()
}

/// The member name of a checked enumerated value (`"PrintTiling.TILEFULLPAGES"` → `TILEFULLPAGES`).
fn member_of(d: &Data) -> &str {
    match d {
        Data::String(s) => s.rsplit_once('.').map_or("", |(_, m)| m),
        _ => "",
    }
}

fn flag(d: &Data) -> bool {
    matches!(d, Data::Bool(true))
}

fn number_of(d: &Data) -> f64 {
    match d {
        Data::Number(n) => *n,
        _ => 0.0,
    }
}

fn string_of(d: &Data) -> String {
    match d {
        Data::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// The slot of the child a stored `Kind::Child` value names.
fn child_slot(h: &Host, d: &Data, class: &str) -> std::result::Result<usize, HostError> {
    let index = class_index(class).ok_or_else(|| HostError::Type(format!("no option class {class}")))?;
    slot_of(h, d, index)
}

fn map_options(h: &Host, slot: usize, job: &mut Job) -> std::result::Result<(), HostError> {
    for (k, v) in assigned(h, slot) {
        match k {
            "flattenerPreset" => job.flattener = Some(string_of(&v)),
            "printPreset" => job.preset = Some(string_of(&v)),
            "jobOptions" => map_job(h, child_slot(h, &v, JOB.name)?, job),
            "colorManagementOptions" => map_color(h, child_slot(h, &v, COLOR.name)?, job),
            "colorSeparationOptions" => map_separation(h, child_slot(h, &v, SEPARATION.name)?, job),
            "coordinateOptions" => map_coordinate(h, child_slot(h, &v, COORDINATE.name)?, job)?,
            "flattenerOptions" => map_flattener(h, child_slot(h, &v, FLATTENER.name)?, job),
            "fontOptions" => {
                child_slot(h, &v, FONT.name)?;
            }
            "pageMarksOptions" => map_marks(h, child_slot(h, &v, MARKS.name)?, job),
            "paperOptions" => map_paper(h, child_slot(h, &v, PAPER.name)?, job),
            "postScriptOptions" => map_postscript(h, child_slot(h, &v, POSTSCRIPT.name)?, job),
            _ => {}
        }
    }
    Ok(())
}

fn map_job(h: &Host, slot: usize, job: &mut Job) {
    let set = assigned(h, slot);
    let value = |name: &str| set.iter().find(|(k, _)| *k == name).map(|(_, v)| v);
    for (k, v) in &set {
        let s = &mut job.top;
        match *k {
            "copies" => {
                s.insert("copies".into(), json!(whole(number_of(v)).unwrap_or(1)));
            }
            "collate" => {
                s.insert("collate".into(), json!(flag(v)));
            }
            "reversePages" => {
                s.insert("reverse".into(), json!(flag(v)));
            }
            "designation" => {
                let layers = match member_of(v) {
                    "ALLLAYERS" => "all",
                    "VISIBLELAYERS" => "visible",
                    _ => "visiblePrintable",
                };
                s.insert("printLayers".into(), json!(layers));
            }
            "printAllArtboards" if flag(v) => {
                s.insert("artboards".into(), json!("all"));
            }
            "printAllArtboards" => match value("artboardRange").map(string_of).filter(|r| !r.trim().is_empty()) {
                Some(range) => {
                    s.insert("artboards".into(), json!("range"));
                    s.insert("range".into(), json!(range));
                }
                None => {
                    s.insert("artboards".into(), json!("all"));
                }
            },
            "printAsBitmap" => {
                job.group("advanced").insert("printAsBitmap".into(), json!(flag(v)));
            }
            "bitmapResolution" => {
                job.group("advanced").insert("bitmapResolution".into(), json!(number_of(v)));
            }
            "file" => job.file = Some(v.clone()),
            "artboardRange" => {
                job.range = Some(string_of(v)).filter(|r| !r.trim().is_empty());
                // Without `printAllArtboards`, the range replaces the base range and the base
                // settings still decide whether a range prints.
                if let Some(range) = job.range.as_ref().filter(|_| value("printAllArtboards").is_none()) {
                    s.insert("range".into(), json!(range));
                }
            }
            "printArea" => job.print_area = true,
            _ => {}
        }
    }
}

fn map_color(h: &Host, slot: usize, job: &mut Job) {
    let set = assigned(h, slot);
    let source = set.iter().any(|(k, v)| *k == "colorProfileMode" && member_of(v) == "SOURCEPROFILE");
    job.profile = set.iter().find(|(k, _)| *k == "name").map(|(_, v)| string_of(v));
    job.source = source;
    let g = job.group("color");
    for (k, v) in &set {
        if *k == "intent" {
            let intent = match member_of(v) {
                "ABSOLUTECOLORIMETRIC" => "absoluteColorimetric",
                "PERCEPTUALINTENT" => "perceptual",
                "SATURATIONINTENT" => "saturation",
                _ => "relativeColorimetric",
            };
            g.insert("intent".into(), json!(intent));
        }
    }
    if source {
        g.insert("profile".into(), json!(""));
    }
}

fn map_separation(h: &Host, slot: usize, job: &mut Job) {
    let g = job.group("output");
    for (k, v) in assigned(h, slot) {
        match k {
            "colorSeparationMode" => {
                g.insert("mode".into(), json!(if member_of(&v) == "HOSTBASEDSEPARATION" { "separations" } else { "composite" }));
            }
            "convertSpotColors" => {
                g.insert("spotsToProcess".into(), json!(flag(&v)));
            }
            "overPrintBlack" => {
                g.insert("overprintBlack".into(), json!(flag(&v)));
            }
            _ => {}
        }
    }
}

fn map_coordinate(h: &Host, slot: usize, job: &mut Job) -> std::result::Result<(), HostError> {
    let set = assigned(h, slot);
    let value = |name: &str| set.iter().find(|(k, _)| *k == name).map(|(_, v)| v);
    let (hs, vs) = (value("horizontalScale").map(number_of), value("verticalScale").map(number_of));
    for (name, n) in [("horizontalScale", hs), ("verticalScale", vs)] {
        if n.is_some_and(|n| n > MAX_PRINT_SCALE) {
            return Err(HostError::Error(format!("PrintCoordinateOptions.{name} above {MAX_PRINT_SCALE} is not available yet")));
        }
    }
    let tiling = value("tiling").map(|v| match member_of(v) {
        "TILEFULLPAGES" => "tileFull",
        "TILEIMAGEABLEAREAS" => "tileImageable",
        _ => "none",
    });
    let fit = value("fitToPage").map(flag);
    let scaled = hs.is_some() || vs.is_some();
    let scaling = if fit == Some(true) {
        Some("fit")
    } else if let Some(t) = tiling.filter(|t| *t != "none") {
        Some(t)
    } else if scaled {
        Some("custom")
    } else if fit.is_some() || tiling.is_some() {
        Some("none")
    } else {
        None
    };
    if let Some(scaling) = scaling {
        job.top.insert("scaling".into(), json!(scaling));
    }
    if scaled && fit != Some(true) {
        // Only the assigned factors are sent; with one of them, the other keeps its value in the
        // base settings (the preset's or the saved ones).
        let g = job.group("scale");
        if let Some(w) = hs {
            g.insert("width".into(), json!(w));
        }
        if let Some(hh) = vs {
            g.insert("height".into(), json!(hh));
        }
        // Linked only when the factors are equal, as `file.print` compares them.
        let linked = matches!((hs, vs), (Some(w), Some(hh)) if w == hh);
        g.insert("linked".into(), json!(linked));
    }
    for (k, v) in &set {
        match *k {
            "emulsion" => {
                job.group("output").insert("emulsion".into(), json!(if flag(v) { "down" } else { "up" }));
            }
            "orientation" => {
                let orientation = match member_of(v) {
                    "AUTOROTATE" => None,
                    "LANDSCAPE" => Some("landscape"),
                    "REVERSEPORTRAIT" => Some("portraitFlipped"),
                    "REVERSELANDSCAPE" => Some("landscapeFlipped"),
                    _ => Some("portrait"),
                };
                job.top.insert("autoRotate".into(), json!(orientation.is_none()));
                if let Some(o) = orientation {
                    job.top.insert("orientation".into(), json!(o));
                }
            }
            "position" => {
                let origin = match member_of(v) {
                    "TRANSLATEBOTTOM" => "bottom",
                    "TRANSLATEBOTTOMLEFT" => "bottomLeft",
                    "TRANSLATEBOTTOMRIGHT" => "bottomRight",
                    "TRANSLATELEFT" => "left",
                    "TRANSLATERIGHT" => "right",
                    "TRANSLATETOP" => "top",
                    "TRANSLATETOPLEFT" => "topLeft",
                    "TRANSLATETOPRIGHT" => "topRight",
                    _ => "center",
                };
                job.group("placement").insert("origin".into(), json!(origin));
            }
            _ => {}
        }
    }
    Ok(())
}

fn map_flattener(h: &Host, slot: usize, job: &mut Job) {
    for (k, v) in assigned(h, slot) {
        if k == "overprint" {
            let keep = member_of(&v) == "PRESERVEPDFOVERPRINT";
            job.group("advanced").insert("overprints".into(), json!(if keep { "preserve" } else { "discard" }));
        }
    }
}

fn map_marks(h: &Host, slot: usize, job: &mut Job) {
    let g = job.group("marks");
    for (k, v) in assigned(h, slot) {
        let (key, value) = match k {
            "colorBars" => ("colorBars", json!(flag(&v))),
            "pageInfoMarks" => ("pageInfo", json!(flag(&v))),
            "registrationMarks" => ("registration", json!(flag(&v))),
            "trimMarks" => ("trim", json!(flag(&v))),
            "pageMarksType" => ("kind", json!(if member_of(&v) == "Japanese" { "japanese" } else { "roman" })),
            "trimMarksWeight" => ("weight", json!(number_of(&v))),
            _ => continue,
        };
        g.insert(key.into(), value);
    }
}

fn map_paper(h: &Host, slot: usize, job: &mut Job) {
    let set = assigned(h, slot);
    let size = |name: &str| set.iter().find(|(k, _)| *k == name).map(|(_, v)| number_of(v)).filter(|n| *n > 0.0);
    let (width, height) = (size("width"), size("height"));
    job.paper = set.iter().find(|(k, _)| *k == "name").map(|(_, v)| string_of(v));
    job.paper_size = width.is_some() || height.is_some();
    let s = &mut job.top;
    if job.paper.is_none() && (width.is_some() || height.is_some()) {
        s.insert("media".into(), json!("custom"));
    }
    if let Some(w) = width {
        s.insert("width".into(), json!(w));
    }
    if let Some(hh) = height {
        s.insert("height".into(), json!(hh));
    }
    if let Some((_, v)) = set.iter().find(|(k, _)| *k == "transverse") {
        s.insert("transverse".into(), json!(flag(v)));
    }
}

fn map_postscript(h: &Host, slot: usize, job: &mut Job) {
    for (k, v) in assigned(h, slot) {
        match k {
            "negativePrinting" => {
                job.group("output").insert("image".into(), json!(if flag(&v) { "negative" } else { "positive" }));
            }
            "postScriptLevel" => job.level = Some(if member_of(&v) == "PSLEVEL2" { 2 } else { 3 }),
            _ => {}
        }
    }
}

/// `document.print(options?)`: `file.print` of the document to the file `jobOptions.file` names,
/// with the assigned options over the document's print settings.
fn doc_print(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let mut job = Job::default();
    if let Some(o) = given(arg(a, 1)).filter(|d| !matches!(d, Data::Null)) {
        let slot = slot_of(h, o, 0).map_err(|e| match e {
            HostError::Type(_) => HostError::Type("print's options must be a PrintOptions object".into()),
            e => e,
        })?;
        map_options(h, slot, &mut job)?;
    }
    let Some(file) = job.file.take() else { return Err(HostError::Error(NO_PRINTER.into())) };
    check_names(h, doc, &mut job)?;
    let path = file_path(&file, "print")?;
    let real = h.disk.borrow().save_target(path).map_err(|m| refused(m, path))?;
    let mut p = Map::new();
    if let Some(preset) = job.preset.take() {
        p.insert("preset".into(), json!(preset));
    }
    // The format follows the name `jobOptions.file` gives, not the real path a link leads to.
    let postscript = job.level.is_some() || crate::cmd::fileio::extension(path) == "ps";
    p.insert("format".into(), json!(if postscript { "postscript" } else { "pdf" }));
    // PostScript prints at the level the options read as: an unassigned `postScriptLevel` is `PSLEVEL2`.
    if postscript {
        p.insert("level".into(), json!(job.level.unwrap_or(2)));
    }
    let settings = job.settings();
    if !settings.is_empty() {
        p.insert("settings".into(), Value::Object(settings));
    }
    p.insert("path".into(), json!(plain(real)));
    let printed = exactly(h, || on_doc(h, doc.uid, false, |h| unfenced(h, PRINT, &Value::Object(p))))?;
    // What the job lost or how its options apply: one Script Console line each.
    let warnings = match &printed {
        Data::Object(fields) => fields.iter().find(|(k, _)| k == "warnings").map(|(_, v)| v),
        _ => None,
    };
    if let Some(Data::Array(warnings)) = warnings {
        for w in warnings.iter().take(MAX_LISTED) {
            if let Data::String(w) = w {
                super::super::each_line(&format!("Document.print: {w}"), |line, cut| h.line(super::super::console::Entry::Output(line), cut));
            }
        }
    }
    Ok(Data::Undefined)
}

/// Checks, before anything is written, the artboard range against the document's artboards and
/// each assigned name against the list it must come from, and puts the checked names into the
/// job: a print or flattener preset by its name (any case), an RGB or CMYK color profile by its
/// name (any case) and a paper by its id or label. An empty name, or one with a path separator or
/// a control character, throws.
fn check_names(h: &Host, doc: Doc, job: &mut Job) -> std::result::Result<(), HostError> {
    let (count, saved_ignore) = with_doc(h, doc, |_, st| {
        let mode = st.doc.print_setup.as_ref().and_then(|v| v.get("artboards")).and_then(Value::as_str);
        (st.doc.artboards.len(), mode == Some("ignore"))
    });
    if let Some(range) = &job.range {
        vectorcraft_doc::range::parse_range(range, count).map_err(|e| HostError::Range(format!("PrintJobOptions.artboardRange: {e}")))?;
    }
    let s = h.session.borrow();
    let mut base_ignore = saved_ignore;
    if let Some(name) = job.preset.take() {
        let n = plain_name("PrintOptions.printPreset", "print preset", &name)?;
        let preset = s.print_preset_settings(n).ok_or_else(|| no_such("PrintOptions.printPreset", "print preset", &name))?;
        base_ignore = matches!(preset.artboards, vectorcraft_pdf::PrintArtboards::Ignore);
        job.preset = Some(n.to_string());
    }
    // An assigned `printArea` prints by artboard when the base settings ignore the artboards: the
    // assigned `artboardRange` when there is one, otherwise every artboard.
    if job.print_area && base_ignore && !job.top.contains_key("artboards") {
        let mode = if job.top.contains_key("range") { "range" } else { "all" };
        job.top.insert("artboards".into(), json!(mode));
    }
    if let Some(name) = job.flattener.take() {
        let n = plain_name("PrintOptions.flattenerPreset", "flattener preset", &name)?;
        if FlattenOptions::find_preset(n, &s.prefs.flattener_presets).is_none() {
            return Err(no_such("PrintOptions.flattenerPreset", "flattener preset", &name));
        }
        job.group("advanced").insert("flattenerPreset".into(), json!(n));
    }
    if let Some(name) = job.profile.take() {
        let what = "PrintColorManagementOptions.name";
        let n = plain_name(what, "color profile", &name)?;
        let n = cms::LEGACY_NAMES.iter().find(|(old, _)| old.eq_ignore_ascii_case(n)).map_or(n, |(_, new)| new);
        let profile = cms::profiles()
            .into_iter()
            .find(|p| p.kind != cms::ProfileKind::Gray && p.name.eq_ignore_ascii_case(n))
            .ok_or_else(|| no_such(what, "color profile", &name))?;
        if !job.source {
            job.group("color").insert("profile".into(), json!(profile.name));
        }
    }
    if let Some(name) = job.paper.take() {
        let n = plain_name("PrintPaperOptions.name", "paper", &name)?;
        let media = vectorcraft_pdf::Media::from_name(n).ok_or_else(|| no_such("PrintPaperOptions.name", "paper", &name))?;
        if job.paper_size && media != vectorcraft_pdf::Media::Custom {
            return Err(HostError::Error(format!("PrintPaperOptions.width and height apply only to custom paper, not {name}")));
        }
        job.top.insert("media".into(), json!(media.id()));
    }
    Ok(())
}

/// The error for a name that is not on its list.
fn no_such(what: &str, kind: &str, name: &str) -> HostError {
    HostError::Error(format!("{what} names no {kind}: {name}"))
}

/// `name` without the spaces around it, or an error when it is empty or holds a path separator or
/// a control character.
fn plain_name<'a>(what: &str, kind: &str, name: &'a str) -> std::result::Result<&'a str, HostError> {
    // Control characters are looked for before the trim, which would drop a tab or a line break.
    if name.chars().any(char::is_control) {
        return Err(no_such(what, kind, name));
    }
    let n = name.trim();
    if n.is_empty() {
        return Err(HostError::Error(format!("{what} must name a {kind}")));
    }
    if n.contains(['/', '\\']) {
        return Err(no_such(what, kind, name));
    }
    Ok(n)
}

const PRINT: &str = "file.print";

/// Runs `id`, which scripts may not run through `vc.run`, for a member that checked its params:
/// only `id` passes the fence, and only for this one command.
fn unfenced(h: &Host, id: &'static str, params: &Value) -> Answer {
    h.session.borrow_mut().script_unfenced = Some(id);
    let r = command(h, id, params);
    h.session.borrow_mut().script_unfenced = None;
    r
}

/// `app.printPresetsList`: the names of the print presets, `[Default]` first.
fn app_presets(h: &Host, _: &[Data]) -> Answer {
    let presets = h.session.borrow().print_presets();
    Ok(Data::Array(presets.into_iter().take(MAX_LISTED).map(|p| text(p.name)).collect()))
}

fn app_ppd_files(_: &Host, _: &[Data]) -> Answer {
    Ok(Data::Array(Vec::new()))
}

/// The members these add to `Application`.
pub(super) fn application_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut members = named(vec![("printPresetsList", get(l, app_presets)), ("PPDFileList", get(l, app_ppd_files))]);
    members.push(later(l, "Application", "printerList"));
    members
}

/// The members these add to `Document`.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("print", method(l, doc_print))])
}

/// The option classes: each `new Name()` gives a new option object with nothing assigned.
pub(super) fn classes(link: &Link) -> Vec<HostClass> {
    CLASSES
        .iter()
        .enumerate()
        .map(|(index, c)| {
            let l = link.clone();
            let construct: HostFunction = Rc::new(move |_: Vec<Data>| {
                let h = current(&l)?;
                h.charge_call(64)?;
                create(&h, index)
            });
            let mut members = vec![("typename".to_string(), super::typename_of(c.name))];
            members.extend(c.fields.iter().enumerate().map(|(i, field)| (field.name.to_string(), property(link, index, i))));
            HostClass { name: c.name.into(), parent: None, members, index: None, construct: Some(construct) }
        })
        .collect()
}
