//! Placing an RTF file: its text set as area type with the character and paragraph formatting
//! VectorCraft has, read as the Rich Text Format (RTF) Specification 1.9.1 describes the format
//! (<https://www.microsoft.com/en-us/download/details.aspx?id=10725>).
//!
//! - Text: `\par`, `\line` and `\sect` start a new paragraph, and so does `\row` (the end of a
//!   table row, whose cells, ended by `\cell` or `\nestcell`, are separated by tabs). `\tab` is a
//!   tab, `\uN` a Unicode character (the `\ucN` fallback characters after it are skipped), and
//!   `\'hh` a byte in Windows-1252, or in Mac Roman after `\mac` or `\ansicpg10000`. Of a `\upr`
//!   group's ANSI and Unicode (`\*\ud`) groups, the Unicode one is read. The control
//!   symbols `\~`, `\-` and `\_` and the control words for quotes, dashes, bullets and special
//!   spaces are their characters.
//! - Character formatting, reset by `\plain`: the font table's font (`\f`, else `\deff`), the size
//!   (`\fs`), bold, italic, underline (every underline style), strikethrough, all caps, small caps,
//!   superscript and subscript, baseline shift (`\up`, `\dn`) and the color table's color (`\cf`).
//! - Paragraph formatting, reset by `\pard`: alignment (`\ql`, `\qc`, `\qr`, `\qj`, `\qd`), left,
//!   right and first-line indents, space before and after, and direction (`\rtlpar`, `\ltrpar`).
//!   `\ql` and `\qr` are the left and right edges in either direction, and a justified (`\qj`)
//!   paragraph marked right to left (`\rtlpar`) ends its last line on the right.
//!
//! Other control words are skipped, and so are hidden text (`\v`) and the groups of destinations
//! that are not the document's text: every `\*` destination, the style sheet, document
//! information, pictures, objects, shapes, headers, footers, notes, note separators, the text for
//! readers without nested tables (`\nonesttables`) and field instructions (a field's result is
//! read).
//! Groups nested deeper than [`MAX_GROUPS`] share the formatting of the deepest one read, and a
//! file with more than [`MAX_PARAGRAPHS`] paragraphs or [`MAX_RUNS`] runs of one formatting is
//! refused.

use std::collections::HashMap;

use vectorcraft_doc::{CharPosition, CharStyle, Justify, ParaDirection, ParaStyle, ScriptMetrics, TextRun};

use super::text::{self, Story};
use super::*;

/// RTF files as `file.place` reports them (not a format `document.open` reads).
pub(super) const FORMAT: fileio::Format = fileio::Format {
    id: "rtf",
    label: "RTF",
    extensions: fileio::RTF_EXTS,
    mime: "application/rtf",
    read: true,
    write: false,
    raster: false,
    options: &[],
};

/// The deepest group nesting read; deeper groups share the formatting of the deepest one read.
const MAX_GROUPS: usize = 512;
/// The most fonts and colors the font and color tables hold.
const MAX_TABLE: usize = 4096;
/// The reader adds no more characters to a font name once it holds this many bytes.
const MAX_NAME: usize = 256;
/// The most paragraphs placed type takes from one RTF file.
const MAX_PARAGRAPHS: usize = 100_000;
/// The most runs of one character formatting placed type takes from one RTF file.
const MAX_RUNS: usize = 100_000;

/// Does `bytes` hold an RTF file (`{\rtf` after any white space)?
pub(super) fn is_rtf(bytes: &[u8]) -> bool {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    bytes.get(start..).is_some_and(|b| b.starts_with(b"{\\rtf"))
}

/// The text of an RTF file's `bytes` with its character and paragraph formatting.
pub(super) fn import(bytes: &[u8], cmd: &str) -> Result<Story> {
    if bytes.len() > text::MAX_TEXT {
        return Err(bad(cmd, format!("the RTF file is too large to place as type (at most {} MB)", text::MAX_TEXT >> 20)));
    }
    if !is_rtf(bytes) {
        return Err(bad(cmd, "not an RTF file: it doesn't start with {\\rtf"));
    }
    let mut r = Reader::default();
    let mut i = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    let mut starred = false;
    while let Some(&b) = bytes.get(i) {
        if r.full {
            break;
        }
        i += 1;
        match b {
            b'{' => {
                starred = false;
                r.open();
            }
            b'}' => {
                starred = false;
                // The end of the outermost group is the end of the document.
                if !r.close() {
                    break;
                }
            }
            b'\\' => {
                let Some(&next) = bytes.get(i) else { break };
                if next.is_ascii_alphabetic() {
                    let (word, n, end) = control_word(bytes, i);
                    i = end;
                    if word == "bin" {
                        // Binary data: never text, and one character of a `\uN` fallback.
                        i = i.saturating_add(usize::try_from(n.unwrap_or(0)).unwrap_or(0)).min(bytes.len());
                        r.skip = r.skip.saturating_sub(1);
                        continue;
                    }
                    r.word(word, n, std::mem::take(&mut starred));
                    continue;
                }
                i += 1;
                match next {
                    b'*' => starred = true,
                    b'\'' => {
                        if let Some(v) = bytes.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                            i += 2;
                            r.byte(v);
                        }
                    }
                    b'\r' | b'\n' => r.word("par", None, false),
                    b'\t' => r.put('\t'),
                    b'~' => r.put('\u{A0}'),
                    b'-' => r.put('\u{AD}'),
                    b'_' => r.put('\u{2011}'),
                    b'\\' | b'{' | b'}' => r.put(char::from(next)),
                    // Other control symbols (index entries, formula characters) count as one
                    // character of a `\uN` fallback.
                    _ => r.skip = r.skip.saturating_sub(1),
                }
            }
            // Line breaks in the file are not text.
            b'\r' | b'\n' => {}
            b'\t' => r.put('\t'),
            0..=0x1F => {}
            _ => r.byte(b),
        }
    }
    r.finish(cmd)
}

/// The control word whose letters start at `bytes[at]`: its name (at most 32 letters), its numeric
/// parameter, and where the text after it starts (a space after it belongs to the word).
fn control_word(bytes: &[u8], at: usize) -> (&str, Option<i32>, usize) {
    let rest = bytes.get(at..).unwrap_or_default();
    let letters = rest.iter().take(32).take_while(|b| b.is_ascii_alphabetic()).count();
    let word = rest.get(..letters).and_then(|w| std::str::from_utf8(w).ok()).unwrap_or_default();
    let mut end = at + letters;
    let negative = bytes.get(end) == Some(&b'-') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit);
    if negative {
        end += 1;
    }
    let digits = bytes.get(end..).unwrap_or_default().iter().take(10).take_while(|b| b.is_ascii_digit()).count();
    let n = bytes
        .get(end..end + digits)
        .filter(|d| !d.is_empty())
        .and_then(|d| std::str::from_utf8(d).ok())
        .and_then(|d| d.parse::<i64>().ok())
        .map(|v| (if negative { -v } else { v }).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
    end += digits;
    if bytes.get(end) == Some(&b' ') {
        end += 1;
    }
    (word, n, end)
}

/// What the characters of the current group are part of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dest {
    /// The document's text.
    Body,
    /// The font table: font names.
    Fonts,
    /// The color table: `\red`, `\green` and `\blue` entries ended by `;`.
    Colors,
    /// Anything else: skipped.
    Skip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// Character formatting (what `\plain` resets).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Chars {
    /// The font table's font number; None: the default font (`\deff`).
    font: Option<i32>,
    /// Size in half points.
    half_points: i32,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    caps: bool,
    small_caps: bool,
    script: Script,
    /// Baseline shift in half points (up positive).
    shift: i32,
    /// The color table's color number; a number without an entry, or whose entry has no values, gives the default color.
    color: i32,
    /// Hidden text (`\v`): not placed.
    hidden: bool,
}

impl Default for Chars {
    fn default() -> Self {
        Self {
            font: None,
            half_points: 24,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            caps: false,
            small_caps: false,
            script: Script::Normal,
            shift: 0,
            color: 0,
            hidden: false,
        }
    }
}

/// Paragraph formatting (what `\pard` resets); lengths in twips.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Para {
    justify: Justify,
    left: i32,
    right: i32,
    first: i32,
    before: i32,
    after: i32,
    rtl: Option<bool>,
}

/// The state a group keeps: it is restored when the group ends.
#[derive(Clone, Copy, Debug)]
struct Group {
    dest: Dest,
    chars: Chars,
    para: Para,
    /// How many fallback characters follow each `\uN`.
    uc: usize,
    /// A `\upr` group: of its two groups, the ANSI one is skipped and the `\*\ud` one is read.
    upr: bool,
}

impl Default for Group {
    fn default() -> Self {
        Self { dest: Dest::Body, chars: Chars::default(), para: Para::default(), uc: 1, upr: false }
    }
}

#[derive(Default)]
struct Reader {
    group: Group,
    stack: Vec<Group>,
    /// Groups opened past [`MAX_GROUPS`] and not yet closed.
    overflow: usize,
    /// `\'hh` bytes are Mac Roman (else Windows-1252).
    mac: bool,
    default_font: i32,
    fonts: HashMap<i32, String>,
    /// The font the font table is naming.
    font: i32,
    font_name: String,
    colors: Vec<Option<[u8; 3]>>,
    rgb: [Option<u8>; 3],
    /// Fallback characters of a `\uN` still to skip.
    skip: usize,
    /// The high surrogate of a character written as two `\uN`.
    high: Option<u16>,
    /// A cell just ended: a tab goes before the next character of its row.
    cell_tab: bool,
    /// The text, in runs of one character formatting; paragraphs end with `\n`.
    spans: Vec<(String, Chars)>,
    /// The formatting of each paragraph that has ended.
    paras: Vec<Para>,
    /// The formatting of the paragraph being read, as at its last character or paragraph
    /// formatting word.
    last: Para,
    /// Past [`MAX_PARAGRAPHS`] or [`MAX_RUNS`].
    full: bool,
}

impl Reader {
    fn open(&mut self) {
        self.skip = 0;
        if self.stack.len() >= MAX_GROUPS {
            self.overflow += 1;
        } else {
            self.stack.push(self.group);
            if std::mem::take(&mut self.group.upr) {
                self.group.dest = Dest::Skip;
            }
        }
    }

    /// Ends a group; false when it was the outermost one.
    fn close(&mut self) -> bool {
        self.skip = 0;
        if self.overflow > 0 {
            self.overflow -= 1;
            return true;
        }
        if self.group.dest == Dest::Fonts {
            self.commit_font();
        }
        match self.stack.pop() {
            Some(g) => {
                self.group = g;
                !self.stack.is_empty()
            }
            None => false,
        }
    }

    fn commit_font(&mut self) {
        let name = std::mem::take(&mut self.font_name);
        let name = name.trim();
        if !name.is_empty() && self.fonts.len() < MAX_TABLE {
            self.fonts.insert(self.font, name.to_string());
        }
    }

    fn commit_color(&mut self) {
        let rgb = std::mem::take(&mut self.rgb);
        if self.colors.len() < MAX_TABLE {
            self.colors.push(rgb.iter().any(Option::is_some).then(|| rgb.map(|v| v.unwrap_or(0))));
        }
    }

    /// Byte `b` of the file's character set.
    fn byte(&mut self, b: u8) {
        self.put(if self.mac { text::mac_roman(b) } else { text::windows_1252(b) });
    }

    /// Character `c` read from the file: skipped when it is a `\uN` fallback character.
    fn put(&mut self, c: char) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        self.high = None;
        self.text(c);
    }

    /// Character `c` in the current destination.
    fn text(&mut self, c: char) {
        match self.group.dest {
            Dest::Body if self.group.chars.hidden => {}
            Dest::Body => {
                if std::mem::take(&mut self.cell_tab) {
                    self.push('\t');
                }
                self.push(c);
            }
            Dest::Fonts if c == ';' => self.commit_font(),
            Dest::Fonts if self.font_name.len() < MAX_NAME => self.font_name.push(c),
            Dest::Colors if c == ';' => self.commit_color(),
            Dest::Fonts | Dest::Colors | Dest::Skip => {}
        }
    }

    /// Adds `c` to the document's text in the current character formatting.
    fn push(&mut self, c: char) {
        let chars = self.group.chars;
        let runs = self.spans.len();
        match self.spans.last_mut() {
            Some((s, f)) if *f == chars => s.push(c),
            _ if runs >= MAX_RUNS => {
                self.full = true;
                return;
            }
            _ => self.spans.push((c.to_string(), chars)),
        }
        self.last = self.group.para;
    }

    /// Ends the paragraph being read (unless its mark is hidden text).
    fn end_paragraph(&mut self) {
        if self.group.dest != Dest::Body || self.group.chars.hidden {
            return;
        }
        self.cell_tab = false;
        if self.paras.len() >= MAX_PARAGRAPHS {
            self.full = true;
            return;
        }
        self.paras.push(self.group.para);
        self.push('\n');
    }

    /// Unicode character (or UTF-16 code unit) `n`, a negative `n` standing for `n + 65536`.
    fn unicode(&mut self, n: i32) {
        let unit = n.rem_euclid(0x10000) as u16;
        match (self.high.take(), unit) {
            (Some(high), 0xDC00..=0xDFFF) => {
                if let Some(Ok(c)) = char::decode_utf16([high, unit]).next() {
                    self.text(c);
                }
            }
            (_, 0xD800..=0xDBFF) => self.high = Some(unit),
            (_, 0xDC00..=0xDFFF) => {}
            (_, unit) => {
                if let Some(c) = char::from_u32(u32::from(unit)) {
                    self.text(c);
                }
            }
        }
        self.skip = self.group.uc;
    }

    /// Control word `w` with parameter `n`; `starred` when `\*` came before it.
    fn word(&mut self, w: &str, n: Option<i32>, starred: bool) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        if w != "u" {
            self.high = None;
        }
        // The Unicode group of a `\upr` group is read in place of its ANSI group.
        if w == "ud"
            && self.overflow == 0
            && let Some(parent) = self.stack.last().filter(|g| g.upr)
        {
            self.group.dest = parent.dest;
            return;
        }
        // An ignorable destination (`\*`): VectorCraft reads none of them.
        if starred {
            self.group.dest = Dest::Skip;
            return;
        }
        if self.group.dest == Dest::Skip {
            return;
        }
        // `\b`, `\b1`: on; `\b0`: off.
        let on = n != Some(0);
        let v = n.unwrap_or(0);
        let para = self.group.para;
        let c = &mut self.group.chars;
        let p = &mut self.group.para;
        match w {
            "fonttbl" => self.group.dest = Dest::Fonts,
            "colortbl" => self.group.dest = Dest::Colors,
            "upr" => self.group.upr = true,
            "stylesheet" | "info" | "pict" | "object" | "shp" | "nonshppict" | "header" | "headerl" | "headerr" | "headerf" | "footer"
            | "footerl" | "footerr" | "footerf" | "footnote" | "annotation" | "fldinst" | "pn" | "listtable" | "listoverridetable" | "revtbl"
            | "rsidtbl" | "filetbl" | "bkmkstart" | "bkmkend" | "xe" | "tc" | "txe" | "template" | "ftnsep" | "ftnsepc" | "ftncn" | "aftnsep"
            | "aftnsepc" | "aftncn" | "nonesttables" => self.group.dest = Dest::Skip,
            "ansi" | "pc" | "pca" => self.mac = false,
            "mac" => self.mac = true,
            "ansicpg" => self.mac = v == 10000,
            "deff" => self.default_font = v,
            "f" if self.group.dest == Dest::Fonts => {
                self.commit_font();
                self.font = v;
            }
            "f" => c.font = Some(v),
            "red" | "green" | "blue" if self.group.dest == Dest::Colors => {
                let i = match w {
                    "red" => 0,
                    "green" => 1,
                    _ => 2,
                };
                if let Some(slot) = self.rgb.get_mut(i) {
                    *slot = Some(v.clamp(0, 255) as u8);
                }
            }
            "uc" => self.group.uc = usize::try_from(v).unwrap_or(0),
            "u" => self.unicode(v),
            "par" | "line" | "sect" => self.end_paragraph(),
            "row" | "nestrow" => {
                self.cell_tab = false;
                self.end_paragraph();
            }
            "cell" | "nestcell" if self.group.dest == Dest::Body && !c.hidden => {
                if self.cell_tab {
                    self.push('\t');
                }
                self.cell_tab = true;
            }
            "tab" => self.text('\t'),
            "emdash" => self.text('\u{2014}'),
            "endash" => self.text('\u{2013}'),
            "bullet" => self.text('\u{2022}'),
            "lquote" => self.text('\u{2018}'),
            "rquote" => self.text('\u{2019}'),
            "ldblquote" => self.text('\u{201C}'),
            "rdblquote" => self.text('\u{201D}'),
            "emspace" => self.text('\u{2003}'),
            "enspace" => self.text('\u{2002}'),
            "qmspace" => self.text('\u{2005}'),
            "zwj" => self.text('\u{200D}'),
            "zwnj" => self.text('\u{200C}'),
            "ltrmark" => self.text('\u{200E}'),
            "rtlmark" => self.text('\u{200F}'),
            "plain" => *c = Chars::default(),
            "fs" => c.half_points = n.unwrap_or(24),
            "b" => c.bold = on,
            "i" => c.italic = on,
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulhwave" | "ulldash" | "ulth" | "ulthd" | "ulthdash" | "ulthdashd"
            | "ulthdashdd" | "ulthldash" | "ululdbwave" | "ulw" | "ulwave" => c.underline = on,
            "ulnone" => c.underline = false,
            "strike" | "striked" => c.strike = on,
            "caps" => c.caps = on,
            "scaps" => c.small_caps = on,
            "super" => c.script = Script::Super,
            "sub" => c.script = Script::Sub,
            "nosupersub" => c.script = Script::Normal,
            "up" => c.shift = n.unwrap_or(6),
            "dn" => c.shift = n.unwrap_or(6).saturating_neg(),
            "cf" => c.color = v,
            "v" => c.hidden = on,
            "pard" => *p = Para::default(),
            "ql" => p.justify = Justify::Left,
            "qc" => p.justify = Justify::Center,
            "qr" => p.justify = Justify::Right,
            "qj" => p.justify = Justify::JustifyLeft,
            "qd" => p.justify = Justify::JustifyAll,
            "li" => p.left = v,
            "ri" => p.right = v,
            "fi" => p.first = v,
            "sb" => p.before = v,
            "sa" => p.after = v,
            "rtlpar" => p.rtl = Some(true),
            "ltrpar" => p.rtl = Some(false),
            _ => {}
        }
        // Paragraph formatting after a paragraph's last character still applies to it.
        if self.group.para != para && self.group.dest == Dest::Body {
            self.last = self.group.para;
        }
    }

    /// The character attributes of formatting `c`.
    fn style(&self, c: &Chars) -> CharStyle {
        let mut s = CharStyle::default();
        if let Some(name) = self.fonts.get(&c.font.unwrap_or(self.default_font)) {
            s.font_family.clone_from(name);
        }
        s.font_style = match (c.bold, c.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        }
        .into();
        s.size = (f64::from(c.half_points) / 2.0).clamp(0.1, 1296.0);
        s.baseline_shift = (f64::from(c.shift) / 2.0).clamp(-1296.0, 1296.0);
        if let Some(Some([r, g, b])) = usize::try_from(c.color).ok().and_then(|i| self.colors.get(i)) {
            s.fill = Paint::solid(vectorcraft_color::Color::rgb8(*r, *g, *b));
        }
        s.underline = c.underline;
        s.strikethrough = c.strike;
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
        let too_many = || {
            bad(
                cmd,
                format!(
                    "the RTF file has too many paragraphs or formatting changes to place as type (at most {MAX_PARAGRAPHS} paragraphs and {MAX_RUNS} runs)"
                ),
            )
        };
        if self.full {
            return Err(too_many());
        }
        self.paras.push(self.last);
        let mut all: String = self.spans.iter().map(|(s, _)| s.as_str()).collect();
        while self.paras.len() > 1
            && let Some(at) = all.rfind('\n')
            && all.get(at + 1..).is_some_and(|tail| tail.trim().is_empty())
        {
            all.truncate(at);
            self.paras.pop();
        }
        if self.paras.len() > MAX_PARAGRAPHS {
            return Err(too_many());
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
        let twips = |v: i32| (f64::from(v) / 20.0).clamp(-1296.0, 1296.0);
        let paras = self
            .paras
            .iter()
            .map(|p| ParaStyle {
                // `\qj` in an `\rtlpar` paragraph justifies it with its last line on the right.
                justify: match p.justify {
                    Justify::JustifyLeft if p.rtl == Some(true) => Justify::JustifyRight,
                    j => j,
                },
                left_indent: twips(p.left),
                right_indent: twips(p.right),
                first_line_indent: twips(p.first),
                space_before: twips(p.before),
                space_after: twips(p.after),
                direction: p.rtl.map(|rtl| if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight }),
                ..ParaStyle::default()
            })
            .collect();
        Ok(Story { runs, paras })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(rtf: &str) -> Story {
        import(rtf.as_bytes(), "file.place").unwrap()
    }

    fn text_of(s: &Story) -> String {
        s.runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn unicode_escapes_skip_their_fallback_characters() {
        // `\u` then 8364: the euro sign.
        let s = read(concat!(r"{\rtf1\ansi\uc1 caf\u233?\uc2 \u", "8364", r"\'80\'80 \uc1\u-10179?\u-8704? end\'93q\'94}"));
        assert_eq!(text_of(&s), "café€ 😀 end“q”");
        assert_eq!(text_of(&read(r"{\rtf1\uc1\u233\bin1 ?X}")), "éX", "binary data is one fallback character");
        let mac = read(r"{\rtf1\mac \'8e\'a5}");
        assert_eq!(text_of(&mac), "é•", "Mac Roman");
    }

    #[test]
    fn upr_groups_read_their_unicode_text() {
        let s = read(concat!(r"{\rtf1 a{\upr{\f0 b?}{\*\ud{\f0 c\u", "945", r"?}}}d}"));
        assert_eq!(text_of(&s), "acαd");
        let fonts = read(r"{\rtf1{\fonttbl{\upr{\f0 Ansi Name;}{\*\ud{\f0 Unicode Name;}}}}x}");
        assert_eq!(fonts.runs.first().map(|r| r.style.font_family.as_str()), Some("Unicode Name"));
    }

    #[test]
    fn destinations_that_are_not_text_are_skipped() {
        let s = read(concat!(
            r"{\rtf1{\fonttbl{\f0 Arial;}}{\*\generator Some Writer;}{\info{\title T}}{\stylesheet{\s1 Heading;}}",
            r"{\header H}A{\*\bkmkstart b}{\field{\*\fldinst HYPERLINK x}{\fldrslt link}}{\pict\pngblip 89504e47}\bin2 {}B}",
            "trailing"
        ));
        assert_eq!(text_of(&s), "AlinkB");
    }

    #[test]
    fn hidden_text_and_the_text_for_older_readers_are_skipped() {
        let s = read(r"{\rtf1 One{\v  hidden\par more\cell\par}\sect Two {\nonesttables flat copy}three}");
        assert_eq!(text_of(&s), "One\nTwo three");
    }

    #[test]
    fn cells_are_tabs_and_rows_paragraphs() {
        let s = read(r"{\rtf1\trowd\cellx100\cellx200 a\cell b\cell\row\trowd c\cell\cell d\cell\row}");
        assert_eq!(text_of(&s), "a\tb\nc\t\td");
    }

    #[test]
    fn paragraph_formatting_after_the_last_character_applies() {
        let s = read(r"{\rtf1 One\par Two\qc\li400}");
        let justify: Vec<Justify> = s.paras.iter().map(|p| p.justify).collect();
        assert_eq!(justify, [Justify::Left, Justify::Center]);
        assert_eq!(s.paras.last().map(|p| p.left_indent), Some(20.0));
    }

    #[test]
    fn justified_right_to_left_paragraphs_end_their_last_line_on_the_right() {
        let s = read(concat!(
            r"{\rtf1\pard\rtlpar\qj a\par\pard\qj\rtlpar b\par\pard\qj c\par\pard\rtlpar\ltrpar\qj d\par",
            r"\pard\rtlpar\ql e\par\pard\rtlpar\qr f\par\pard\rtlpar\qc g\par\pard\rtlpar\qd h}"
        ));
        let justify: Vec<Justify> = s.paras.iter().map(|p| p.justify).collect();
        use Justify::*;
        assert_eq!(justify, [JustifyRight, JustifyRight, JustifyLeft, JustifyLeft, Left, Right, Center, JustifyAll]);
        assert_eq!(s.paras.first().and_then(|p| p.direction), Some(ParaDirection::RightToLeft));
    }

    #[test]
    fn deep_nesting_and_stray_braces_end_cleanly() {
        let deep = format!("{{\\rtf1 {}x{}}}", "{".repeat(5000), "}".repeat(5000));
        assert_eq!(text_of(&read(&deep)), "x");
        assert_eq!(text_of(&read(r"{\rtf1 unclosed {\b group")), "unclosed group");
        assert!(import(b"{\\rt", "file.place").is_err(), "not RTF");
        assert!(import(b"Plain words", "file.place").is_err(), "not RTF");
    }

    #[test]
    fn too_many_paragraphs_or_runs_are_refused() {
        let most = format!("{{\\rtf1 {}}}", "a\\par ".repeat(MAX_PARAGRAPHS));
        assert_eq!(read(&most).paras.len(), MAX_PARAGRAPHS);
        let paras = format!("{{\\rtf1 {}a}}", "a\\par ".repeat(MAX_PARAGRAPHS));
        assert!(import(paras.as_bytes(), "file.place").is_err());
        let marks = format!("{{\\rtf1 {}}}", "a\\par ".repeat(MAX_PARAGRAPHS + 1));
        assert!(import(marks.as_bytes(), "file.place").is_err());
        let runs = format!("{{\\rtf1 {}}}", "{\\b a}b".repeat(MAX_RUNS / 2 + 1));
        assert!(import(runs.as_bytes(), "file.place").is_err());
    }
}
