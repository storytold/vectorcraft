//! Placing a Word document (`.docx`): its text set as area type with the character and paragraph
//! formatting VectorCraft has, read as ECMA-376 Part 1 describes WordprocessingML
//! (<https://ecma-international.org/publications-and-standards/standards/ecma-376/>), in its
//! Transitional and Strict namespaces.
//!
//! - The package is a ZIP archive ([`fileio::zip`]). `_rels/.rels` names the main document part
//!   (else `word/document.xml`), and that part's relationships name its styles and theme parts
//!   (else `styles.xml` and `theme/theme1.xml` next to it).
//! - Text: each paragraph (`w:p`) is a paragraph, and so is each table row, whose cells are
//!   separated by tabs. A text-wrapping break (`w:br`) and a carriage return (`w:cr`) start a new
//!   paragraph; page and column breaks are skipped. `w:tab` and `w:ptab` are tabs, and
//!   `w:noBreakHyphen` and `w:softHyphen` are their hyphens. Text in insertions, moves, hyperlinks,
//!   smart tags, content controls, custom XML and simple fields is read, and of a complex field,
//!   its result.
//! - Character formatting: the font (`w:rFonts`: `w:asciiTheme`, `w:ascii`, `w:hAnsiTheme`, then
//!   `w:hAnsi`; a theme font is the theme's major or minor Latin font), the size, bold, italic,
//!   underline (every style but `none`), strikethrough and double strikethrough, all caps, small
//!   caps, superscript and subscript (`w:vertAlign`), raised and lowered text (`w:position`) and
//!   the color.
//! - Paragraph formatting: alignment (`w:jc`), start and end indents (`w:start` or `w:left`,
//!   `w:end` or `w:right`: the right and left indents of a right-to-left paragraph), first-line and
//!   hanging indents, space before and after, and direction (`w:bidi`). A paragraph justified with
//!   a low, medium or high kashida length (`lowKashida`, `mediumKashida`, `highKashida`) is
//!   justified with Short, Medium or Long kashidas; `both` is justified without kashidas.
//! - Styles (§17.7.2): the document defaults, then the paragraph style (else the default paragraph
//!   style) and the styles it is based on, then the run's character style (else the default
//!   character style) and the styles it is based on, then direct formatting. Bold, italic, caps,
//!   small caps, strikethrough and hidden text are toggle properties (§17.7.3): a style that turns
//!   one on reverses the state the levels before it give, and direct formatting sets it.
//!
//! Hidden text (`w:vanish`), deleted text, field instructions, drawings, pictures, objects, symbol
//! characters (`w:sym`), note and comment references, headers, footers, notes, comments and list
//! numbers are left out. A package with more than [`MAX_FILES`] files, a part that unpacks to more
//! than [`MAX_PART`] bytes or nests elements more than [`MAX_NESTING`] deep, and a document with
//! more than [`MAX_PARAGRAPHS`] paragraphs, [`MAX_RUNS`] runs of one formatting or
//! [`text::MAX_TEXT`] bytes of text are refused. Tables, content controls and other containers
//! nested more than [`MAX_DEPTH`] deep are skipped, and so are fields nested deeper.

use std::collections::HashMap;

use roxmltree::Node;
use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{CharPosition, CharStyle, Justify, ParaDirection, ParaStyle, ScriptMetrics, TextRun};

use super::text::{self, Story};
use super::*;
use crate::cmd::fileio::zip::{self, Archive, ReadError};

/// Word documents as `file.place` reports them (not a format `document.open` reads).
pub(super) const FORMAT: fileio::Format = fileio::Format {
    id: "docx",
    label: "Word",
    extensions: fileio::DOCX_EXTS,
    mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    read: true,
    write: false,
    raster: false,
    options: &[],
};

/// The most files a Word document's package may hold.
const MAX_FILES: usize = 10_000;
/// The most bytes a part read from the package may unpack to.
const MAX_PART: usize = 16 << 20;
/// The most XML nodes a part read from the package may have.
const MAX_NODES: u32 = 2_000_000;
/// The deepest nesting of XML elements a part read from the package may have.
const MAX_NESTING: usize = 256;
/// The deepest nesting of tables, content controls and other containers read; deeper ones are
/// skipped, and so are fields nested deeper.
const MAX_DEPTH: usize = 64;
/// The most styles read from the styles part.
const MAX_STYLES: usize = 4096;
/// The most styles one style's `w:basedOn` chain takes in.
const MAX_BASED_ON: usize = 32;
/// The most fonts the document's formatting names.
const MAX_FONTS: usize = 4096;
/// The most bytes of a font name kept.
const MAX_NAME: usize = 256;
/// The most paragraphs placed type takes from one Word document.
const MAX_PARAGRAPHS: usize = 100_000;
/// The most runs of one character formatting placed type takes from one Word document.
const MAX_RUNS: usize = 100_000;

/// WordprocessingML's namespaces: Transitional, then Strict.
const W: [&str; 2] = ["http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"];

/// Is the file `name` with `bytes` a Word document: a ZIP archive named `.docx`, or one that holds
/// `word/document.xml`?
pub(super) fn is_docx(name: &str, bytes: &[u8]) -> bool {
    zip::is_zip(bytes)
        && (fileio::DOCX_EXTS.contains(&fileio::extension(name).as_str())
            || Archive::open(bytes, MAX_FILES).is_ok_and(|z| z.contains("word/document.xml")))
}

/// The text of a Word document's `bytes` (a ZIP package) with its character and paragraph
/// formatting.
pub(super) fn import(bytes: &[u8], cmd: &str) -> Result<Story> {
    let zip = Archive::open(bytes, MAX_FILES).map_err(|e| package_error(cmd, e))?;
    let main = relationship(&zip, "", "officeDocument", cmd)?.unwrap_or_else(|| "word/document.xml".into());
    let Some(xml) = part(&zip, &main, cmd)? else { return Err(bad(cmd, "not a Word document: its package has no main document part")) };
    let doc = parse(&xml, &main, cmd)?;
    if local(doc.root_element()) != Some("document") {
        return Err(bad(cmd, "not a Word document: its main part is not a WordprocessingML document"));
    }
    let folder = main.rfind('/').and_then(|i| main.get(..=i)).unwrap_or_default();
    let mut r = Reader::default();
    let theme = relationship(&zip, &main, "theme", cmd)?.unwrap_or_else(|| format!("{folder}theme/theme1.xml"));
    if let Some(xml) = part(&zip, &theme, cmd)? {
        r.theme_fonts(&parse(&xml, &theme, cmd)?);
    }
    let styles = relationship(&zip, &main, "styles", cmd)?.unwrap_or_else(|| format!("{folder}styles.xml"));
    if let Some(xml) = part(&zip, &styles, cmd)? {
        r.styles(&parse(&xml, &styles, cmd)?);
    }
    if let Some(body) = doc.root_element().children().find(|n| local(*n) == Some("body")) {
        r.block(body, 0);
    }
    r.finish(cmd)
}

/// The error for a package (or a part of it) the ZIP reader can't read.
fn package_error(cmd: &str, e: ReadError) -> EngineError {
    bad(
        cmd,
        match e {
            ReadError::NotZip => "not a Word document: it isn't a ZIP package".to_string(),
            ReadError::TooManyFiles => format!("the Word document's package has more than {MAX_FILES} files"),
            ReadError::Zip64 => "the Word document's package uses ZIP64 records, which VectorCraft doesn't read".to_string(),
            ReadError::Encrypted(name) => format!("`{name}` in the Word document is encrypted"),
            ReadError::Method(name, m) => format!("`{name}` in the Word document uses compression method {m}, which VectorCraft doesn't read"),
            ReadError::TooLarge(name) => format!("`{name}` in the Word document is too large to place (at most {} MB unpacked)", MAX_PART >> 20),
            ReadError::Damaged(name) if name.is_empty() => "the Word document's package is damaged".to_string(),
            ReadError::Damaged(name) => format!("the Word document's package is damaged: `{name}` can't be read"),
        },
    )
}

/// The part `name` of the package as text (UTF-8, or UTF-16 with a byte-order mark); None: the
/// package has no such part.
fn part(zip: &Archive, name: &str, cmd: &str) -> Result<Option<String>> {
    let Some(bytes) = zip.read(name, MAX_PART).map_err(|e| package_error(cmd, e))? else { return Ok(None) };
    let utf16 = |rest: &[u8], le: bool| -> String {
        let units = rest.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
        char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    let text = match &bytes[..] {
        [0xEF, 0xBB, 0xBF, rest @ ..] => std::str::from_utf8(rest).map(str::to_string),
        [0xFF, 0xFE, rest @ ..] => Ok(utf16(rest, true)),
        [0xFE, 0xFF, rest @ ..] => Ok(utf16(rest, false)),
        _ => std::str::from_utf8(&bytes).map(str::to_string),
    };
    text.map(Some).map_err(|_| bad(cmd, format!("`{name}` in the Word document is not UTF-8 or UTF-16 text")))
}

/// The XML of the part `name`.
fn parse<'a>(xml: &'a str, name: &str, cmd: &str) -> Result<roxmltree::Document<'a>> {
    // roxmltree parses nested elements recursively.
    if nesting(xml) > MAX_NESTING {
        return Err(bad(cmd, format!("`{name}` in the Word document nests elements more than {MAX_NESTING} deep")));
    }
    let opts = roxmltree::ParsingOptions { nodes_limit: MAX_NODES, ..roxmltree::ParsingOptions::default() };
    roxmltree::Document::parse_with_options(xml, opts).map_err(|e| bad(cmd, format!("`{name}` in the Word document is not well-formed XML: {e}")))
}

/// How deep the elements of `xml` nest: start tags less end tags, at most, outside comments,
/// CDATA sections, processing instructions, declarations and quoted attribute values.
pub(crate) fn nesting(xml: &str) -> usize {
    let b = xml.as_bytes();
    let (mut depth, mut most, mut i) = (0usize, 0usize, 0usize);
    // The index just past the first `end` at or after `from` (the end of the text if none).
    let past = |from: usize, end: &[u8]| {
        b.get(from..).and_then(|rest| rest.windows(end.len()).position(|w| w == end)).map_or(b.len(), |at| from + at + end.len())
    };
    while let Some(at) = b.get(i..).and_then(|rest| rest.iter().position(|&c| c == b'<')) {
        let start = i + at;
        let rest = b.get(start..).unwrap_or_default();
        if rest.starts_with(b"<!--") {
            i = past(start + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = past(start + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = past(start + 2, b"?>");
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") {
            depth = depth.saturating_sub(usize::from(rest.starts_with(b"</")));
            i = past(start + 2, b">");
        } else {
            // A start tag: up to its `>` outside quotes; `/>` closes it at once.
            let mut j = start + 1;
            let mut quote = None;
            while let Some(&c) = b.get(j) {
                match (quote, c) {
                    (None, b'"' | b'\'') => quote = Some(c),
                    (Some(q), _) if c == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if b.get(j.saturating_sub(1)) != Some(&b'/') {
                depth += 1;
                most = most.max(depth);
            }
            i = j + 1;
        }
    }
    most
}

/// The part that the first relationship of part `source` (`""`: the package) whose type ends in
/// `/{kind}` points to; None when the package has no relationships part for `source` or it names
/// no such part.
fn relationship(zip: &Archive, source: &str, kind: &str, cmd: &str) -> Result<Option<String>> {
    let (folder, file) = source.rfind('/').map_or(("", source), |i| (source.get(..=i).unwrap_or_default(), source.get(i + 1..).unwrap_or_default()));
    let rels = format!("{folder}_rels/{file}.rels");
    let Some(xml) = part(zip, &rels, cmd)? else { return Ok(None) };
    let doc = parse(&xml, &rels, cmd)?;
    let target = doc
        .root_element()
        .children()
        .filter(|n| n.tag_name().name() == "Relationship" && n.attribute("TargetMode") != Some("External"))
        .find(|n| n.attribute("Type").is_some_and(|t| t.rsplit('/').next() == Some(kind)))
        .and_then(|n| n.attribute("Target"));
    Ok(target.map(|t| resolve(folder, t)))
}

/// The part name `target` gives, relative to the folder `folder` (`""` or ending in `/`).
fn resolve(folder: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("{folder}{target}"),
    };
    let mut parts: Vec<&str> = vec![];
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// `n`'s local name when it is a WordprocessingML element.
fn local<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    (n.is_element() && n.tag_name().namespace().is_some_and(|ns| W.contains(&ns))).then(|| n.tag_name().name())
}

/// `n`'s first WordprocessingML child element named `name`.
fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| local(*c) == Some(name))
}

/// `n`'s WordprocessingML attribute `name`.
fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name && a.namespace().is_some_and(|ns| W.contains(&ns))).map(|a| a.value())
}

/// An on/off property (ST_OnOff): on unless `w:val` is `false`, `0` or `off`.
fn on(n: Node) -> bool {
    !matches!(attr(n, "val"), Some("false" | "0" | "off"))
}

/// A measure `v` in units of `1 / per_pt` point: a number in those units, or a number with a unit
/// (`mm`, `cm`, `in`, `pt`, `pc`, `pi`).
fn measure(v: &str, per_pt: f64) -> Option<f64> {
    const UNITS: [(&str, f64); 6] = [("mm", 72.0 / 25.4), ("cm", 72.0 / 2.54), ("in", 72.0), ("pt", 1.0), ("pc", 12.0), ("pi", 12.0)];
    let v = v.trim();
    let n = match UNITS.iter().find_map(|(unit, pt)| v.strip_suffix(unit).map(|n| (n, pt))) {
        Some((n, pt)) => n.trim().parse::<f64>().ok()? * pt * per_pt,
        None => v.parse::<f64>().ok()?,
    };
    n.is_finite().then_some(n)
}

/// Twentieths of a point.
fn twips(n: Node, name: &str) -> Option<f64> {
    attr(n, name).and_then(|v| measure(v, 20.0))
}

/// Half points.
fn half_points(n: Node) -> Option<f64> {
    attr(n, "val").and_then(|v| measure(v, 2.0))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// The run properties one level of the style hierarchy sets (None: it doesn't set that one).
#[derive(Clone, Copy, Debug, Default)]
struct RunProps {
    /// The font's number in [`Reader::fonts`].
    font: Option<usize>,
    half_points: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    caps: Option<bool>,
    small_caps: Option<bool>,
    strike: Option<bool>,
    dstrike: Option<bool>,
    hidden: Option<bool>,
    underline: Option<bool>,
    script: Option<Script>,
    /// Raised (positive) or lowered text in half points.
    shift: Option<f64>,
    /// Some(None): automatic (the default color).
    color: Option<Option<[u8; 3]>>,
}

/// Character formatting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Chars {
    font: Option<usize>,
    /// None: VectorCraft's default size.
    half_points: Option<f64>,
    bold: bool,
    italic: bool,
    caps: bool,
    small_caps: bool,
    strike: bool,
    dstrike: bool,
    hidden: bool,
    underline: bool,
    script: Script,
    shift: f64,
    color: Option<[u8; 3]>,
}

impl Chars {
    /// `p` applied over this formatting: as a style's (a toggle property turned on reverses its
    /// state, turned off leaves it) or as direct formatting (it sets each property).
    fn apply(&mut self, p: &RunProps, style: bool) {
        let toggle = |state: &mut bool, v: Option<bool>| match v {
            Some(true) if style => *state = !*state,
            Some(v) if !style => *state = v,
            _ => {}
        };
        toggle(&mut self.bold, p.bold);
        toggle(&mut self.italic, p.italic);
        toggle(&mut self.caps, p.caps);
        toggle(&mut self.small_caps, p.small_caps);
        toggle(&mut self.strike, p.strike);
        toggle(&mut self.hidden, p.hidden);
        self.font = p.font.or(self.font);
        self.half_points = p.half_points.or(self.half_points);
        self.dstrike = p.dstrike.unwrap_or(self.dstrike);
        self.underline = p.underline.unwrap_or(self.underline);
        self.script = p.script.unwrap_or(self.script);
        self.shift = p.shift.unwrap_or(self.shift);
        self.color = p.color.unwrap_or(self.color);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Align {
    #[default]
    Left,
    Center,
    Right,
    /// Justified, the last line at the start (the right of a right-to-left paragraph), with the
    /// kashida length of `lowKashida`, `mediumKashida` and `highKashida` (§17.18.44).
    Both(Kashida),
    /// Justified, the last line too.
    Distribute,
    Start,
    End,
}

/// The paragraph properties one level of the style hierarchy sets; lengths in twips.
#[derive(Clone, Copy, Debug, Default)]
struct ParaProps {
    align: Option<Align>,
    /// The indent at the paragraph's start (the left of a left-to-right paragraph).
    start: Option<f64>,
    /// The indent at the paragraph's end.
    end: Option<f64>,
    first: Option<f64>,
    before: Option<f64>,
    after: Option<f64>,
    bidi: Option<bool>,
}

/// Paragraph formatting; lengths in twips.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Para {
    align: Align,
    start: f64,
    end: f64,
    first: f64,
    before: f64,
    after: f64,
    rtl: Option<bool>,
}

impl Para {
    fn apply(&mut self, p: &ParaProps) {
        self.align = p.align.unwrap_or(self.align);
        self.start = p.start.unwrap_or(self.start);
        self.end = p.end.unwrap_or(self.end);
        self.first = p.first.unwrap_or(self.first);
        self.before = p.before.unwrap_or(self.before);
        self.after = p.after.unwrap_or(self.after);
        self.rtl = p.bidi.or(self.rtl);
    }
}

/// A style of the styles part.
struct Style {
    based_on: Option<String>,
    run: RunProps,
    para: ParaProps,
}

#[derive(Default)]
struct Reader {
    /// The theme's major and minor Latin fonts.
    theme: [Option<String>; 2],
    fonts: Vec<String>,
    font_numbers: HashMap<String, usize>,
    styles: HashMap<String, Style>,
    default_run: RunProps,
    default_para: ParaProps,
    /// The default paragraph style's id.
    default_style: Option<String>,
    /// The default character style's id.
    default_char_style: Option<String>,
    /// The paragraph being read: its formatting, and its run formatting before character styles
    /// and direct formatting.
    para: Para,
    base: Chars,
    /// A paragraph that has ended (its formatting and its mark's): its break goes before the
    /// next text.
    pending: Option<(Para, Chars)>,
    /// The last cell's paragraph, which its row's break ends.
    cell_end: Option<(Para, Chars)>,
    /// A cell just ended: a tab goes before the next character of its row.
    cell_tab: bool,
    /// The complex fields open: true while their instructions are being read.
    fields: Vec<bool>,
    /// Fields begun past [`MAX_DEPTH`] and not yet ended.
    field_overflow: usize,
    /// The text, in runs of one character formatting; paragraphs end with `\n`.
    spans: Vec<(String, Chars)>,
    /// The bytes of text in `spans`.
    len: usize,
    /// The formatting of each paragraph that has ended.
    paras: Vec<Para>,
    /// Past [`MAX_PARAGRAPHS`], [`MAX_RUNS`] or [`text::MAX_TEXT`].
    full: bool,
}

impl Reader {
    /// The major and minor fonts of a theme part's font scheme.
    fn theme_fonts(&mut self, doc: &roxmltree::Document) {
        for (slot, scheme) in self.theme.iter_mut().zip(["majorFont", "minorFont"]) {
            *slot = doc
                .descendants()
                .find(|n| n.tag_name().name() == scheme)
                .and_then(|n| n.children().find(|c| c.tag_name().name() == "latin"))
                .and_then(|n| n.attribute("typeface"))
                .map(font_name)
                .filter(|f| !f.is_empty());
        }
    }

    /// The document defaults and the styles of the styles part.
    fn styles(&mut self, doc: &roxmltree::Document) {
        for n in doc.root_element().children() {
            match local(n) {
                Some("docDefaults") => {
                    let props = |outer: &str, inner: &str| child(n, outer).and_then(|d| child(d, inner));
                    if let Some(rpr) = props("rPrDefault", "rPr") {
                        self.default_run = self.run_props(rpr);
                    }
                    if let Some(ppr) = props("pPrDefault", "pPr") {
                        self.default_para = para_props(ppr);
                    }
                }
                Some("style") if self.styles.len() < MAX_STYLES => {
                    let Some(id) = attr(n, "styleId") else { continue };
                    let default = match attr(n, "type") {
                        Some("paragraph") => Some(&mut self.default_style),
                        Some("character") => Some(&mut self.default_char_style),
                        _ => None,
                    };
                    if let Some(default) = default.filter(|_| attr(n, "default").is_some_and(|v| matches!(v, "1" | "true" | "on"))) {
                        default.get_or_insert_with(|| id.to_string());
                    }
                    let run = child(n, "rPr").map(|r| self.run_props(r)).unwrap_or_default();
                    let para = child(n, "pPr").map(para_props).unwrap_or_default();
                    let based_on = child(n, "basedOn").and_then(|b| attr(b, "val")).map(str::to_string);
                    self.styles.entry(id.to_string()).or_insert(Style { based_on, run, para });
                }
                _ => {}
            }
        }
    }

    /// Style `id` and the styles it is based on, the first of the chain first.
    fn chain(&self, id: Option<&str>) -> Vec<&Style> {
        let mut ids: Vec<&str> = vec![];
        let mut chain = vec![];
        let mut next = id;
        while let Some(id) = next {
            if ids.contains(&id) || ids.len() >= MAX_BASED_ON {
                break;
            }
            let Some(style) = self.styles.get(id) else { break };
            ids.push(id);
            chain.push(style);
            next = style.based_on.as_deref();
        }
        chain.reverse();
        chain
    }

    /// The number of the font a `w:rFonts` element names.
    fn font(&mut self, n: Node) -> Option<usize> {
        let theme = |v: &str| self.theme.get(usize::from(v.starts_with("minor"))).and_then(Option::as_deref);
        let name = attr(n, "asciiTheme")
            .and_then(theme)
            .or_else(|| attr(n, "ascii"))
            .or_else(|| attr(n, "hAnsiTheme").and_then(theme))
            .or_else(|| attr(n, "hAnsi"))
            .map(font_name)?;
        if name.is_empty() {
            return None;
        }
        if let Some(&i) = self.font_numbers.get(&name) {
            return Some(i);
        }
        if self.fonts.len() >= MAX_FONTS {
            return None;
        }
        self.fonts.push(name.clone());
        self.font_numbers.insert(name, self.fonts.len() - 1);
        Some(self.fonts.len() - 1)
    }

    /// The run properties of a `w:rPr` element.
    fn run_props(&mut self, rpr: Node) -> RunProps {
        let mut p = RunProps::default();
        for n in rpr.children() {
            match local(n) {
                Some("rFonts") => p.font = self.font(n),
                Some("b") => p.bold = Some(on(n)),
                Some("i") => p.italic = Some(on(n)),
                Some("caps") => p.caps = Some(on(n)),
                Some("smallCaps") => p.small_caps = Some(on(n)),
                Some("strike") => p.strike = Some(on(n)),
                Some("dstrike") => p.dstrike = Some(on(n)),
                Some("vanish") => p.hidden = Some(on(n)),
                Some("u") => p.underline = Some(attr(n, "val") != Some("none")),
                Some("vertAlign") => {
                    p.script = match attr(n, "val") {
                        Some("superscript") => Some(Script::Super),
                        Some("subscript") => Some(Script::Sub),
                        Some("baseline") => Some(Script::Normal),
                        _ => p.script,
                    }
                }
                Some("position") => p.shift = half_points(n).or(p.shift),
                Some("sz") => p.half_points = half_points(n).filter(|v| *v > 0.0).or(p.half_points),
                Some("color") => {
                    let hex = |v: &str| -> Option<[u8; 3]> {
                        let v = v.trim();
                        if v.len() != 6 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
                            return None;
                        }
                        let byte = |i: usize| v.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
                        Some([byte(0)?, byte(2)?, byte(4)?])
                    };
                    p.color = match attr(n, "val") {
                        Some("auto") => Some(None),
                        Some(v) => hex(v).map(Some).or(p.color),
                        None => p.color,
                    }
                }
                _ => {}
            }
        }
        p
    }

    /// The paragraphs, tables and containers of them that are `n`'s children.
    fn block(&mut self, n: Node, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in n.children() {
            if self.full {
                return;
            }
            match local(c) {
                Some("p") => self.paragraph(c, depth),
                Some("tbl") => self.table(c, depth),
                Some("sdt") => {
                    if let Some(content) = child(c, "sdtContent") {
                        self.block(content, depth + 1);
                    }
                }
                Some("customXml" | "ins" | "moveTo") => self.block(c, depth + 1),
                _ => {}
            }
        }
    }

    /// A table: each row a paragraph, its cells separated by tabs.
    fn table(&mut self, tbl: Node, depth: usize) {
        for row in items(tbl, "tr", depth) {
            for cell in items(row, "tc", depth) {
                self.flush();
                self.block(cell, depth + 1);
                // The cell's last paragraph goes on in the next cell, after a tab.
                let end = self.pending.take();
                if self.cell_tab
                    && let Some((_, mark)) = end
                {
                    self.push('\t', mark);
                }
                self.cell_end = end.or(self.cell_end);
                self.cell_tab = true;
            }
            self.cell_tab = false;
            self.pending = self.cell_end.take();
        }
    }

    /// A paragraph.
    fn paragraph(&mut self, p: Node, depth: usize) {
        self.flush();
        let ppr = child(p, "pPr");
        let style = ppr.and_then(|n| child(n, "pStyle")).and_then(|n| attr(n, "val")).map(str::to_string).or_else(|| self.default_style.clone());
        let mut para = Para::default();
        para.apply(&self.default_para);
        let mut base = Chars::default();
        base.apply(&self.default_run, true);
        for s in self.chain(style.as_deref()) {
            para.apply(&s.para);
            base.apply(&s.run, true);
        }
        if let Some(n) = ppr {
            para.apply(&para_props(n));
        }
        (self.para, self.base) = (para, base);
        self.inline(p, depth + 1);
        // The paragraph mark's run properties are direct formatting over the paragraph style's.
        let mut mark = base;
        if let Some(rpr) = ppr.and_then(|n| child(n, "rPr")) {
            let props = self.run_props(rpr);
            mark.apply(&props, false);
        }
        // A line break at the paragraph's end is written before the paragraph ends, and a hidden
        // paragraph mark joins the paragraph to the next.
        if !mark.hidden {
            self.flush();
            self.pending = Some((para, mark));
        }
    }

    /// The runs of a paragraph, and the containers of runs in it.
    fn inline(&mut self, n: Node, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in n.children() {
            match local(c) {
                Some("r") => self.run(c),
                Some("hyperlink" | "ins" | "moveTo" | "smartTag" | "customXml" | "fldSimple" | "dir" | "bdo") => self.inline(c, depth + 1),
                Some("sdt") => {
                    if let Some(content) = child(c, "sdtContent") {
                        self.inline(content, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// A run.
    fn run(&mut self, r: Node) {
        let rpr = child(r, "rPr");
        let mut chars = self.base;
        let style = rpr.and_then(|n| child(n, "rStyle")).and_then(|n| attr(n, "val")).or(self.default_char_style.as_deref());
        for s in self.chain(style) {
            chars.apply(&s.run, true);
        }
        if let Some(n) = rpr {
            let props = self.run_props(n);
            chars.apply(&props, false);
        }
        for c in r.children() {
            match local(c) {
                Some("t") => {
                    // Comments and processing instructions may split the text.
                    for ch in c.children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars) {
                        self.put(ch, chars);
                    }
                }
                Some("tab" | "ptab") => self.put('\t', chars),
                Some("noBreakHyphen") => self.put('\u{2011}', chars),
                Some("softHyphen") => self.put('\u{AD}', chars),
                Some("br") if matches!(attr(c, "type"), None | Some("textWrapping")) => self.line_break(chars),
                Some("cr") => self.line_break(chars),
                Some("fldChar") => match attr(c, "fldCharType") {
                    Some("begin") if self.fields.len() < MAX_DEPTH => self.fields.push(true),
                    Some("begin") => self.field_overflow += 1,
                    Some("separate") if self.field_overflow == 0 => {
                        if let Some(instructions) = self.fields.last_mut() {
                            *instructions = false;
                        }
                    }
                    Some("end") if self.field_overflow > 0 => self.field_overflow -= 1,
                    Some("end") => {
                        self.fields.pop();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Text is not read: hidden, or a field's instructions.
    fn skipped(&self, chars: &Chars) -> bool {
        chars.hidden || self.field_overflow > 0 || self.fields.contains(&true)
    }

    /// Character `c` of the document's text in formatting `chars`.
    fn put(&mut self, c: char, chars: Chars) {
        if self.skipped(&chars) {
            return;
        }
        self.flush();
        if std::mem::take(&mut self.cell_tab) {
            self.push('\t', chars);
        }
        self.push(c, chars);
    }

    /// A line break: the rest of the paragraph is a new paragraph in the same formatting.
    fn line_break(&mut self, chars: Chars) {
        if self.skipped(&chars) {
            return;
        }
        self.flush();
        self.pending = Some((self.para, chars));
    }

    /// Writes the break of the paragraph that has ended.
    fn flush(&mut self) {
        let Some((para, mark)) = self.pending.take() else { return };
        if self.paras.len() >= MAX_PARAGRAPHS {
            self.full = true;
            return;
        }
        self.paras.push(para);
        // A cell that starts with a break is still separated from the cell before it.
        if std::mem::take(&mut self.cell_tab) {
            self.push('\t', mark);
        }
        self.push('\n', mark);
    }

    /// Adds `c` to the text in formatting `chars`.
    fn push(&mut self, c: char, chars: Chars) {
        self.len += c.len_utf8();
        if self.len > text::MAX_TEXT {
            self.full = true;
            return;
        }
        let runs = self.spans.len();
        match self.spans.last_mut() {
            Some((s, f)) if *f == chars => s.push(c),
            _ if runs >= MAX_RUNS => self.full = true,
            _ => self.spans.push((c.to_string(), chars)),
        }
    }

    /// The character attributes of formatting `c`.
    fn style(&self, c: &Chars) -> CharStyle {
        let mut s = CharStyle::default();
        if let Some(name) = c.font.and_then(|i| self.fonts.get(i)) {
            s.font_family.clone_from(name);
        }
        s.font_style = match (c.bold, c.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        }
        .into();
        if let Some(v) = c.half_points {
            s.size = (v / 2.0).clamp(0.1, 1296.0);
        }
        s.baseline_shift = (c.shift / 2.0).clamp(-1296.0, 1296.0);
        if let Some([r, g, b]) = c.color {
            s.fill = Paint::solid(vectorcraft_color::Color::rgb8(r, g, b));
        }
        s.underline = c.underline;
        s.strikethrough = c.strike || c.dstrike;
        s.all_caps = c.caps;
        // Document Setup's proportions replace these when the type is placed.
        s.small_caps = c.small_caps.then_some(70.0);
        s.position = match c.script {
            Script::Normal => CharPosition::Normal,
            Script::Super => CharPosition::Superscript(ScriptMetrics::DEFAULT),
            Script::Sub => CharPosition::Subscript(ScriptMetrics::DEFAULT),
        };
        s
    }

    /// The text read, without blank paragraphs at its end, and each paragraph's attributes.
    fn finish(mut self, cmd: &str) -> Result<Story> {
        if self.full {
            return Err(bad(
                cmd,
                format!(
                    "the Word document has too much text or formatting to place as type (at most {MAX_PARAGRAPHS} paragraphs, {MAX_RUNS} runs and {} MB of text)",
                    text::MAX_TEXT >> 20
                ),
            ));
        }
        let last = self.pending.take().map_or(self.para, |(para, _)| para);
        self.paras.push(last);
        let mut all: String = self.spans.iter().map(|(s, _)| s.as_str()).collect();
        while self.paras.len() > 1
            && let Some(at) = all.rfind('\n')
            && all.get(at + 1..).is_some_and(|tail| tail.trim().is_empty())
        {
            all.truncate(at);
            self.paras.pop();
        }
        let mut keep = all.len();
        let mut runs: Vec<TextRun> = vec![];
        for (s, chars) in &self.spans {
            let piece = s.get(..keep.min(s.len())).unwrap_or_default();
            keep -= piece.len();
            if piece.is_empty() {
                continue;
            }
            let style = self.style(chars);
            match runs.last_mut() {
                Some(r) if r.style == style => r.text.push_str(piece),
                _ => runs.push(TextRun::new(piece, style)),
            }
        }
        let points = |v: f64| (v / 20.0).clamp(-1296.0, 1296.0);
        let paras = self
            .paras
            .iter()
            .map(|p| {
                let rtl = p.rtl == Some(true);
                let (left, right) = if rtl { (p.end, p.start) } else { (p.start, p.end) };
                ParaStyle {
                    justify: match p.align {
                        Align::Left => Justify::Left,
                        Align::Center => Justify::Center,
                        Align::Right => Justify::Right,
                        Align::Both(_) if rtl => Justify::JustifyRight,
                        Align::Both(_) => Justify::JustifyLeft,
                        Align::Distribute => Justify::JustifyAll,
                        Align::Start => Justify::Auto,
                        Align::End if rtl => Justify::Left,
                        Align::End => Justify::Right,
                    },
                    left_indent: points(left),
                    right_indent: points(right),
                    first_line_indent: points(p.first),
                    space_before: points(p.before),
                    space_after: points(p.after),
                    direction: p.rtl.map(|rtl| if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight }),
                    kashida: if let Align::Both(k) = p.align { k } else { Kashida::None },
                    ..ParaStyle::default()
                }
            })
            .collect();
        Ok(Story { runs, paras })
    }
}

/// Font name `name` trimmed, and cut at the last character that ends within [`MAX_NAME`] bytes.
fn font_name(name: &str) -> String {
    let name = name.trim();
    let end = (0..=MAX_NAME.min(name.len())).rev().find(|&i| name.is_char_boundary(i)).unwrap_or_default();
    name.get(..end).unwrap_or_default().to_string()
}

/// The paragraph properties of a `w:pPr` element.
fn para_props(ppr: Node) -> ParaProps {
    let mut p = ParaProps::default();
    for n in ppr.children() {
        match local(n) {
            Some("jc") => {
                p.align = match attr(n, "val") {
                    Some("left") => Some(Align::Left),
                    Some("center") => Some(Align::Center),
                    Some("right") => Some(Align::Right),
                    Some("both") => Some(Align::Both(Kashida::None)),
                    Some("lowKashida") => Some(Align::Both(Kashida::Short)),
                    Some("mediumKashida") => Some(Align::Both(Kashida::Medium)),
                    Some("highKashida") => Some(Align::Both(Kashida::Long)),
                    Some("distribute" | "thaiDistribute") => Some(Align::Distribute),
                    Some("start") => Some(Align::Start),
                    Some("end") => Some(Align::End),
                    _ => p.align,
                }
            }
            Some("ind") => {
                // `w:left` and `w:right` (Transitional) are the start and end indents too.
                p.start = twips(n, "start").or_else(|| twips(n, "left")).or(p.start);
                p.end = twips(n, "end").or_else(|| twips(n, "right")).or(p.end);
                // A hanging indent wins over a first-line indent.
                p.first = twips(n, "hanging").map(|v| -v).or_else(|| twips(n, "firstLine")).or(p.first);
            }
            Some("spacing") => {
                p.before = twips(n, "before").or(p.before);
                p.after = twips(n, "after").or(p.after);
            }
            Some("bidi") => p.bidi = Some(on(n)),
            _ => {}
        }
    }
    p
}

/// `n`'s children named `name`, and those inside content controls and custom XML among them.
fn items<'a, 'i>(n: Node<'a, 'i>, name: &str, depth: usize) -> Vec<Node<'a, 'i>> {
    let mut out = vec![];
    if depth > MAX_DEPTH {
        return out;
    }
    for c in n.children() {
        match local(c) {
            Some(l) if l == name => out.push(c),
            Some("sdt") => {
                if let Some(content) = child(c, "sdtContent") {
                    out.extend(items(content, name, depth + 1));
                }
            }
            Some("customXml") => out.extend(items(c, name, depth + 1)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_read_numbers_and_units() {
        assert_eq!(measure("240", 20.0), Some(240.0));
        assert_eq!(measure("12pt", 20.0), Some(240.0));
        assert_eq!(measure("1in", 2.0), Some(144.0));
        assert_eq!(measure("2.54cm", 1.0).map(|v| (v * 1e9).round() / 1e9), Some(72.0));
        assert_eq!(measure("1e999", 20.0), None);
        assert_eq!(measure("wide", 20.0), None);
    }

    #[test]
    fn nesting_counts_elements_outside_markup_that_is_not_an_element() {
        assert_eq!(nesting(r#"<?xml version="1.0"?><a><b x="1>2" y='</c>'><c/></b><!-- <d><e> --><![CDATA[<f><g>]]></a>"#), 2);
        assert_eq!(nesting("<a><b><c>"), 3, "unclosed elements count");
        assert_eq!(nesting("</a></b><a>"), 1);
        assert_eq!(nesting("<a x=\"unterminated"), 1);
        assert_eq!(nesting(""), 0);
    }

    #[test]
    fn targets_resolve_against_their_folder() {
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve("word/", "styles.xml"), "word/styles.xml");
        assert_eq!(resolve("word/", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve("word/", "/word/theme/./theme1.xml"), "word/theme/theme1.xml");
    }
}
