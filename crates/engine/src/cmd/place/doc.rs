//! Placing a Word 97-2003 document (`.doc`): its main text set as area type with the character and
//! paragraph formatting VectorCraft has, read as \[MS-DOC\] describes the Word Binary File Format
//! (<https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/262caa01-60a5-479e-b710-1e966936a524>)
//! from the compound file \[MS-CFB\] describes
//! (<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/53989ce4-7b05-4f8d-829b-d08d6148375b>).
//!
//! - The `WordDocument` stream starts with the file information block (FIB, §2.5.1), which locates
//!   the structures read from the `0Table` or `1Table` stream (`fWhichTblStm`). Documents saved by
//!   Word 97 and later (`nFib` 0x00C1 and above) are read; older ones, encrypted or obfuscated ones
//!   and ones last saved with fast save (`fComplex`) are refused.
//! - Text: the main document's `ccpText` characters, from the pieces the Clx's PlcPcd lists
//!   (§2.9.38, §2.8.35) in their order, each in 16-bit Unicode or in compressed 8-bit text with the
//!   mapping §2.9.73 gives; 16-bit text with half of a surrogate pair is refused. A paragraph mark
//!   (U+000D) ends a paragraph, and so does a page or section break (U+000C) at the end of a
//!   paragraph's properties (PapxFkp); a line break
//!   (U+000B) starts a new paragraph in the same formatting. A table's cells (cell marks, U+0007)
//!   are separated by tabs and each row (a table terminating mark) is a paragraph. Tabs are tabs,
//!   and U+001E and U+001F are the nonbreaking and optional hyphens. Of a field (U+0013, U+0014 and
//!   U+0015), its result is read.
//! - Character formatting: the font (`sprmCRgFtc0`, named by the font table SttbfFfn), the size,
//!   bold, italic, underline (every style but none), strikethrough and double strikethrough, all
//!   caps, small caps, superscript and subscript, raised and lowered text (`sprmCHpsPos`) and the
//!   color (`sprmCIco`, `sprmCCv`).
//! - Paragraph formatting: alignment (`sprmPJc80`, then `sprmPJc`: start and end follow the
//!   direction, and the kashida alignments are justified with Short, Medium or Long kashidas), the
//!   left and right indents (the physical ones Word 97 writes, unless the logical ones later
//!   versions write are given), first-line indent, space before and after, and direction
//!   (`sprmPFBiDi`).
//! - Formatting is resolved as "Determining Formatting Properties" describes: the style sheet's
//!   default font and 10 pt, then the paragraph style (STSH, §2.9.271) and the styles it is based
//!   on, then the run's character style (`sprmCIstd`) and the styles it is based on, then the
//!   direct formatting of the run (Chpx) and of the paragraph (Papx). A toggle operand of 0x80 or
//!   0x81 takes or reverses the value the styles give.
//!
//! Hidden text, deleted revisions, field instructions, pictures and embedded objects, note and
//! comment references and the other stories (footnotes, endnotes, headers, footers, comments and
//! text boxes) are left out. The reply's warnings name each kind of content that was left out or
//! changed, once: pictures and objects, fields, fields left out (no result, or nested too deep), the
//! other stories, tracked changes and tables. A stream larger than [`MAX_STREAM`] bytes, a main
//! text longer than [`text::MAX_TEXT`] characters, more than [`MAX_PIECES`] pieces or
//! [`MAX_PAGES`] property pages, and placed type of more than [`MAX_PARAGRAPHS`] paragraphs or
//! [`MAX_RUNS`] runs are refused, and so is a structure that lies outside its stream.

use std::io::{Cursor, Read};

use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{CharPosition, CharStyle, Justify, ParaDirection, ParaStyle, ScriptMetrics, TextRun};

use super::text::{self, Story};
use super::{EngineError, Paint, Result, bad, fileio};

/// Word 97-2003 documents as `file.place` reports them (not a format `document.open` reads).
pub(super) const FORMAT: fileio::Format = fileio::Format {
    id: "doc",
    label: "Word 97-2003",
    extensions: fileio::DOC_EXTS,
    mime: "application/msword",
    read: true,
    write: false,
    raster: false,
    options: &[],
};

/// The first bytes of a compound file (\[MS-CFB\] §2.2).
const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// The most bytes a stream read from the compound file may hold.
const MAX_STREAM: u64 = 64 << 20;
/// The most pieces the main text may be made of.
const MAX_PIECES: usize = 100_000;
/// The most 512-byte pages of character or of paragraph properties read.
const MAX_PAGES: usize = 16_384;
/// The most styles read from the style sheet (`Stshif.cstd` is below 0x0FFE).
const MAX_STYLES: usize = 0x0FFE;
/// The most styles one style's chain of `istdBase` takes in.
const MAX_BASED_ON: usize = 32;
/// The most fonts read from the font table (SttbfFfn's `cData` is at most 0x7FF0).
const MAX_FONTS: usize = 0x7FF0;
/// The most bytes of a font name kept.
const MAX_NAME: usize = 256;
/// The most paragraphs placed type takes from one Word document.
const MAX_PARAGRAPHS: usize = 100_000;
/// The most runs of one character formatting placed type takes from one Word document.
const MAX_RUNS: usize = 100_000;
/// The deepest nesting of fields read; deeper ones are left out with their results.
const MAX_FIELDS: usize = 64;
/// `istdBase` of a style that is based on none.
const NO_STYLE: u16 = 0x0FFF;

/// The FibRgFcLcb97 pairs read (§2.5.6), by their index.
const STSHF: usize = 1;
const PLCF_BTE_CHPX: usize = 12;
const PLCF_BTE_PAPX: usize = 13;
const STTBF_FFN: usize = 15;
const CLX: usize = 33;
/// The number of pairs FibRgFcLcb97 holds.
const FC_LCB_97: u16 = 0x005D;

/// Is the file `name` with `bytes` a Word 97-2003 document: a compound file named `.doc`, or one
/// that holds a `WordDocument` stream?
pub(super) fn is_doc(name: &str, bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
        && (fileio::DOC_EXTS.contains(&fileio::extension(name).as_str())
            || cfb::CompoundFile::open(Cursor::new(bytes)).is_ok_and(|f| f.is_stream("/WordDocument")))
}

/// The main text of a Word 97-2003 document's `bytes` (a compound file) with its character and
/// paragraph formatting, and a warning for each kind of content left out or changed.
pub(super) fn import(bytes: &[u8], cmd: &str) -> Result<(Story, Vec<String>)> {
    let mut file = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|_| damaged(cmd, "its compound file structure is damaged"))?;
    let wd = stream(&mut file, "WordDocument", cmd)?.ok_or_else(|| bad(cmd, "not a Word document: its compound file has no WordDocument stream"))?;
    let fib = Fib::read(&wd, cmd)?;
    let table_name = if fib.table_one { "1Table" } else { "0Table" };
    let table = stream(&mut file, table_name, cmd)?.ok_or_else(|| damaged(cmd, format!("it has no {table_name} stream")))?;
    let clx = fib.part(&table, CLX, cmd)?.ok_or_else(|| damaged(cmd, "it has no piece table"))?;
    let text = main_text(&wd, &pieces(clx, cmd)?, fib.ccp_text, cmd)?;
    let chpx = match fib.part(&table, PLCF_BTE_CHPX, cmd)? {
        Some(plc) => runs(&wd, plc, Fkp::Chpx, cmd)?,
        None => vec![],
    };
    let papx = match fib.part(&table, PLCF_BTE_PAPX, cmd)? {
        Some(plc) => runs(&wd, plc, Fkp::Papx, cmd)?,
        None => vec![],
    };
    let sheet = match fib.part(&table, STSHF, cmd)? {
        Some(stsh) => Sheet::read(stsh, cmd)?,
        None => Sheet::default(),
    };
    let fonts = match fib.part(&table, STTBF_FFN, cmd)? {
        Some(sttb) => fonts(sttb, cmd)?,
        None => vec![],
    };
    let mut r = Reader::new(&sheet, &fonts);
    r.read(&text, &chpx, &papx);
    let warnings = r.warnings(&fib);
    Ok((r.finish(cmd)?, warnings))
}

/// The error for a Word document whose structures can't be read.
fn damaged(cmd: &str, why: impl std::fmt::Display) -> EngineError {
    bad(cmd, format!("can't read this Word document: {why}"))
}

/// The stream `name` of the compound file; None when it has no such stream.
fn stream(file: &mut cfb::CompoundFile<Cursor<&[u8]>>, name: &str, cmd: &str) -> Result<Option<Vec<u8>>> {
    if !file.is_stream(name) {
        return Ok(None);
    }
    let s = file.open_stream(name).map_err(|_| damaged(cmd, format!("its {name} stream is damaged")))?;
    if s.len() > MAX_STREAM {
        return Err(bad(cmd, format!("the Word document is too large to place (its {name} stream holds more than {} MB)", MAX_STREAM >> 20)));
    }
    let mut out = vec![];
    s.take(MAX_STREAM).read_to_end(&mut out).map_err(|_| damaged(cmd, format!("its {name} stream is damaged")))?;
    Ok(Some(out))
}

/// The little-endian 16-bit integer at `at`.
fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at.checked_add(2)?).and_then(|s| s.try_into().ok()).map(u16::from_le_bytes)
}

/// The little-endian 32-bit integer at `at`.
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).and_then(|s| s.try_into().ok()).map(u32::from_le_bytes)
}

/// `len` bytes of `b` from `at`.
fn bytes_at(b: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    b.get(at..at.checked_add(len)?)
}

/// What the reader takes from the file information block.
struct Fib {
    /// The table stream is `1Table` (else `0Table`).
    table_one: bool,
    /// The main document's length in characters.
    ccp_text: u32,
    /// The lengths of the footnotes, headers and footers, comments, endnotes, text boxes and
    /// header text boxes (`ccpFtn`, `ccpHdd`, `ccpAtn`, `ccpEdn`, `ccpTxbx`, `ccpHdrTxbx`).
    stories: [u32; 6],
    /// FibRgFcLcb's offset and size pairs.
    fc_lcb: Vec<(u32, u32)>,
}

impl Fib {
    /// The FIB at the start of the WordDocument stream `wd` (§2.5.1).
    fn read(wd: &[u8], cmd: &str) -> Result<Self> {
        if u16_at(wd, 0) != Some(0xA5EC) {
            return Err(bad(cmd, "not a Word document: its WordDocument stream doesn't start with a file information block"));
        }
        let field = |at: usize| u16_at(wd, at).ok_or_else(|| damaged(cmd, "its file information block is cut short"));
        if field(2)? < 0x00C1 {
            return Err(bad(
                cmd,
                "the Word document was saved by Word 6.0 or Word 95, which VectorCraft doesn't read; save it as a Word 97-2003 document",
            ));
        }
        let flags = field(10)?;
        // fEncrypted (fObfuscated only says how).
        if flags & 0x0100 != 0 {
            return Err(bad(cmd, "the Word document is encrypted or protected with a password; save it without one to place it"));
        }
        // fComplex: the last save was an incremental (fast) save.
        if flags & 0x0004 != 0 {
            return Err(bad(
                cmd,
                "the Word document was last saved with fast save, which VectorCraft doesn't read; save it again with fast save turned off",
            ));
        }
        // csw (14 16-bit values) and cslw (22 32-bit values) are fixed.
        if field(32)? != 0x000E || field(62)? != 0x0016 {
            return Err(damaged(cmd, "its file information block is damaged"));
        }
        let count = field(152)?;
        if count < FC_LCB_97 {
            return Err(damaged(cmd, "its file information block is damaged"));
        }
        let pair = |i: usize| Some((u32_at(wd, 154 + 8 * i)?, u32_at(wd, 158 + 8 * i)?));
        let fc_lcb =
            (0..usize::from(count)).map(pair).collect::<Option<Vec<_>>>().ok_or_else(|| damaged(cmd, "its file information block is cut short"))?;
        // FibRgLw97 starts at 64: ccpText is its fourth value.
        let lw = |i: usize| u32_at(wd, 64 + 4 * i).unwrap_or_default();
        Ok(Self { table_one: flags & 0x0200 != 0, ccp_text: lw(3), stories: [lw(4), lw(5), lw(7), lw(8), lw(9), lw(10)], fc_lcb })
    }

    /// The structure FibRgFcLcb's pair `index` locates in the table stream `table`; None when it
    /// is empty.
    fn part<'a>(&self, table: &'a [u8], index: usize, cmd: &str) -> Result<Option<&'a [u8]>> {
        let Some(&(fc, lcb)) = self.fc_lcb.get(index).filter(|(_, lcb)| *lcb > 0) else { return Ok(None) };
        bytes_at(table, fc as usize, lcb as usize).map(Some).ok_or_else(|| damaged(cmd, "a table it lists lies outside its table stream"))
    }
}

/// A piece of the text (Pcd): characters `cp..end`, stored from file position `fc`.
struct Piece {
    cp: u32,
    end: u32,
    fc: u32,
    /// Compressed 8-bit text (else 16-bit Unicode).
    compressed: bool,
}

/// The pieces the Clx `clx` lists (§2.9.38), in character order.
fn pieces(clx: &[u8], cmd: &str) -> Result<Vec<Piece>> {
    let broken = || damaged(cmd, "its piece table is damaged");
    let mut at = 0usize;
    // Prc records (property modifiers for pieces) come first.
    while clx.get(at) == Some(&0x01) {
        let size = u16_at(clx, at + 1).map(|v| v as i16).filter(|v| *v >= 0).ok_or_else(broken)?;
        at += 3 + size as usize;
    }
    if clx.get(at) != Some(&0x02) {
        return Err(broken());
    }
    let lcb = u32_at(clx, at + 1).ok_or_else(broken)? as usize;
    let plc = bytes_at(clx, at + 5, lcb).ok_or_else(broken)?;
    if lcb < 4 || !(lcb - 4).is_multiple_of(12) {
        return Err(broken());
    }
    let n = (lcb - 4) / 12;
    if n > MAX_PIECES {
        return Err(bad(cmd, format!("the Word document's text is made of more than {MAX_PIECES} pieces, too many to place")));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (cp, end) = (u32_at(plc, 4 * i).ok_or_else(broken)?, u32_at(plc, 4 * i + 4).ok_or_else(broken)?);
        // A Pcd: 2 bytes of flags, then FcCompressed (fc in bits 0 to 29, fCompressed in bit 30).
        let raw = u32_at(plc, 4 * (n + 1) + 8 * i + 2).ok_or_else(broken)?;
        if end <= cp || out.last().is_some_and(|p: &Piece| p.end != cp) || (i == 0 && cp != 0) {
            return Err(broken());
        }
        let compressed = raw & 0x4000_0000 != 0;
        let fc = raw & 0x3FFF_FFFF;
        out.push(Piece { cp, end, fc: if compressed { fc / 2 } else { fc }, compressed });
    }
    Ok(out)
}

/// A character of the main text: the character, the file position of its bytes, and their number.
type Char = (char, u32, u8);

/// The first `ccp` characters of the pieces, read from the WordDocument stream `wd`.
fn main_text(wd: &[u8], pieces: &[Piece], ccp: u32, cmd: &str) -> Result<Vec<Char>> {
    if ccp as usize > text::MAX_TEXT {
        return Err(bad(cmd, format!("the Word document has too much text to place as type (at most {} MB)", text::MAX_TEXT >> 20)));
    }
    if pieces.last().is_none_or(|p| p.end < ccp) {
        return Err(damaged(cmd, "its piece table doesn't cover its text"));
    }
    let outside = || damaged(cmd, "a piece of its text lies outside its WordDocument stream");
    // Each character position's UTF-16 code unit, file position and byte count, in character
    // order; a surrogate pair may be split between two pieces.
    let mut units: Vec<(u16, u32, u8)> = Vec::with_capacity(ccp as usize);
    for p in pieces.iter().take_while(|p| p.cp < ccp) {
        let n = (p.end.min(ccp) - p.cp) as usize;
        if p.compressed {
            let b = bytes_at(wd, p.fc as usize, n).ok_or_else(outside)?;
            // Every character compressed text maps to is in the Basic Multilingual Plane.
            units.extend(b.iter().zip(p.fc..).map(|(&c, fc)| (u16::try_from(u32::from(compressed(c))).unwrap_or(0xFFFD), fc, 1)));
            continue;
        }
        let b = bytes_at(wd, p.fc as usize, n.checked_mul(2).ok_or_else(outside)?).ok_or_else(outside)?;
        units.extend(b.as_chunks::<2>().0.iter().zip((p.fc..).step_by(2)).map(|(&u, fc)| (u16::from_le_bytes(u), fc, 2)));
    }
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while let Some(&(u, fc, width)) = units.get(i) {
        let low = units.get(i + 1).map(|&(low, _, w)| (low, w)).filter(|(low, _)| (0xD800..0xDC00).contains(&u) && (0xDC00..0xE000).contains(low));
        let c = match low {
            Some((low, _)) => char::decode_utf16([u, low]).next().and_then(|c| c.ok()),
            None => char::from_u32(u32::from(u)),
        };
        let c = c.ok_or_else(|| damaged(cmd, "its text holds half of a UTF-16 surrogate pair"))?;
        out.push((c, fc, low.map_or(width, |(_, w)| width + w)));
        i += if low.is_some() { 2 } else { 1 };
    }
    Ok(out)
}

/// Byte `b` of compressed text (§2.9.73): the byte as a Unicode character, except the ones that
/// stand for these.
fn compressed(b: u8) -> char {
    const MAPPED: [(u8, char); 24] = [
        (0x82, '\u{201A}'),
        (0x83, '\u{0192}'),
        (0x84, '\u{201E}'),
        (0x85, '\u{2026}'),
        (0x86, '\u{2020}'),
        (0x87, '\u{2021}'),
        (0x88, '\u{02C6}'),
        (0x89, '\u{2030}'),
        (0x8A, '\u{0160}'),
        (0x8B, '\u{2039}'),
        (0x8C, '\u{0152}'),
        (0x91, '\u{2018}'),
        (0x92, '\u{2019}'),
        (0x93, '\u{201C}'),
        (0x94, '\u{201D}'),
        (0x95, '\u{2022}'),
        (0x96, '\u{2013}'),
        (0x97, '\u{2014}'),
        (0x98, '\u{02DC}'),
        (0x99, '\u{2122}'),
        (0x9A, '\u{0161}'),
        (0x9B, '\u{203A}'),
        (0x9C, '\u{0153}'),
        (0x9F, '\u{0178}'),
    ];
    MAPPED.iter().find(|(k, _)| *k == b).map_or(char::from(b), |(_, c)| *c)
}

/// Which property pages a bin table points to.
#[derive(Clone, Copy, PartialEq)]
enum Fkp {
    /// ChpxFkp (§2.9.33): character properties.
    Chpx,
    /// PapxFkp (§2.9.174): paragraph properties.
    Papx,
}

/// The file positions `start..end` and the properties a property page gives them.
struct Run {
    start: u32,
    end: u32,
    /// The paragraph style (Papx only).
    istd: u16,
    grpprl: Vec<u8>,
}

/// The runs of the property pages the bin table `plc` (PlcBteChpx or PlcBtePapx) points to, in
/// the WordDocument stream `wd`, ordered by file position.
fn runs(wd: &[u8], plc: &[u8], kind: Fkp, cmd: &str) -> Result<Vec<Run>> {
    let broken = || damaged(cmd, "its character or paragraph properties are damaged");
    if plc.len() < 4 || !(plc.len() - 4).is_multiple_of(8) {
        return Err(broken());
    }
    let n = (plc.len() - 4) / 8;
    if n > MAX_PAGES {
        return Err(bad(cmd, format!("the Word document has more than {MAX_PAGES} pages of properties, too many to place")));
    }
    let mut out = vec![];
    for i in 0..n {
        // PnFkpChpx and PnFkpPapx: the page number in bits 0 to 21.
        let pn = u32_at(plc, 4 * (n + 1) + 4 * i).ok_or_else(broken)? & 0x003F_FFFF;
        let page = bytes_at(wd, pn as usize * 512, 512).ok_or_else(broken)?;
        page_runs(page, kind, &mut out).ok_or_else(broken)?;
    }
    out.sort_by_key(|r| r.start);
    Ok(out)
}

/// The runs of one 512-byte property page; None when it is damaged.
fn page_runs(page: &[u8], kind: Fkp, out: &mut Vec<Run>) -> Option<()> {
    let count = usize::from(*page.get(511)?);
    let most = if kind == Fkp::Chpx { 0x65 } else { 0x1D };
    if count == 0 || count > most {
        return None;
    }
    for i in 0..count {
        let (start, end) = (u32_at(page, 4 * i)?, u32_at(page, 4 * i + 4)?);
        // ChpxFkp's rgb: one byte each; PapxFkp's rgbx: 13-byte BxPap, its first byte the offset.
        let entry = 4 * (count + 1) + if kind == Fkp::Chpx { i } else { 13 * i };
        let offset = usize::from(*page.get(entry)?) * 2;
        let (istd, grpprl) = match (offset, kind) {
            (0, _) => (0, vec![]),
            (_, Fkp::Chpx) => {
                let cb = usize::from(*page.get(offset)?);
                (0, bytes_at(page, offset + 1, cb).filter(|_| offset + 1 + cb <= 511)?.to_vec())
            }
            (_, Fkp::Papx) => {
                // PapxInFkp: 2 × cb − 1 bytes, or (cb = 0) 2 × cb′ bytes after cb′.
                let (at, len) = match usize::from(*page.get(offset)?) {
                    0 => (offset + 2, usize::from(*page.get(offset + 1)?) * 2),
                    cb => (offset + 1, 2 * cb - 1),
                };
                let grp = bytes_at(page, at, len).filter(|_| at + len <= 511 && len >= 2)?;
                (u16_at(grp, 0)?, grp.get(2..)?.to_vec())
            }
        };
        if end <= start {
            return None;
        }
        out.push(Run { start, end, istd, grpprl });
    }
    Some(())
}

/// The run of `runs` (ordered by file position) that holds file position `fc`.
fn run_at(runs: &[Run], fc: u32) -> Option<usize> {
    let i = runs.partition_point(|r| r.start <= fc).checked_sub(1)?;
    runs.get(i).filter(|r| fc < r.end).map(|_| i)
}

/// The size of the operand of `sprm` (§2.2.5.1) whose bytes start `rest`.
fn operand_len(sprm: u16, rest: &[u8]) -> Option<usize> {
    Some(match sprm >> 13 {
        0 | 1 => 1,
        2 | 4 | 5 => 2,
        3 => 4,
        7 => 3,
        // sprmTDefTable: a 2-byte size of the rest, plus 1.
        _ if sprm == 0xD608 => usize::from(u16_at(rest, 0)?) + 1,
        // sprmPChgTabs with cb 255: the deleted tabs with their close tolerances, then the added
        // tabs with their descriptors.
        _ if sprm == 0xC615 && rest.first() == Some(&255) => {
            let deleted = usize::from(*rest.get(1)?);
            let added = usize::from(*rest.get(2 + 4 * deleted)?);
            3 + 4 * deleted + 3 * added
        }
        _ => usize::from(*rest.first()?) + 1,
    })
}

/// Calls `f` with each Prl of `grpprl`: its sprm and its operand.
fn each_prl(grpprl: &[u8], mut f: impl FnMut(u16, &[u8])) {
    let mut at = 0;
    while let Some(sprm) = u16_at(grpprl, at) {
        let rest = grpprl.get(at + 2..).unwrap_or_default();
        let Some((n, op)) = operand_len(sprm, rest).and_then(|n| Some((n, rest.get(..n)?))) else { return };
        // A variable operand's bytes follow its size.
        let op = if sprm >> 13 == 6 && !matches!(sprm, 0xD608 | 0xC615) { op.get(1..).unwrap_or_default() } else { op };
        f(sprm, op);
        at += 2 + n;
    }
}

/// A style of the style sheet.
struct Style {
    /// 1: paragraph style, 2: character style.
    stk: u8,
    base: u16,
    papx: Vec<u8>,
    chpx: Vec<u8>,
}

/// The style sheet: its styles by istd and its default font.
#[derive(Default)]
struct Sheet {
    styles: Vec<Option<Style>>,
    /// `Stshif.ftcAsci`.
    font: Option<u16>,
}

impl Sheet {
    /// The style sheet STSH (§2.9.271).
    fn read(stsh: &[u8], cmd: &str) -> Result<Self> {
        let broken = || damaged(cmd, "its style sheet is damaged");
        let size = usize::from(u16_at(stsh, 0).ok_or_else(broken)?);
        let stshi = bytes_at(stsh, 2, size).ok_or_else(broken)?;
        let count = usize::from(u16_at(stshi, 0).ok_or_else(broken)?).min(MAX_STYLES);
        // cbSTDBaseInFile: StdfBase, and StdfPost2000 when it is 18.
        let base_size = usize::from(u16_at(stshi, 2).ok_or_else(broken)?).max(10);
        let font = u16_at(stshi, 12);
        let mut at = 2 + size;
        let mut styles = Vec::with_capacity(count);
        for _ in 0..count {
            let Some(len) = u16_at(stsh, at) else { break };
            let std = bytes_at(stsh, at + 2, usize::from(len)).ok_or_else(broken)?;
            styles.push(if std.is_empty() { None } else { Some(style(std, base_size).ok_or_else(broken)?) });
            // LPStd records start on even offsets.
            at += 2 + usize::from(len) + usize::from(len) % 2;
        }
        Ok(Self { styles, font })
    }

    /// Style `istd` of kind `stk` and the styles it is based on, the first of the chain first.
    fn chain(&self, istd: u16, stk: u8) -> Vec<&Style> {
        let mut seen: Vec<u16> = vec![];
        let mut chain = vec![];
        let mut next = istd;
        while next != NO_STYLE && !seen.contains(&next) && seen.len() < MAX_BASED_ON {
            let Some(s) = self.styles.get(usize::from(next)).and_then(Option::as_ref).filter(|s| s.stk == stk) else { break };
            seen.push(next);
            chain.push(s);
            next = s.base;
        }
        chain.reverse();
        chain
    }
}

/// A style definition (STD, §2.9.258) whose Stdf takes `base_size` bytes; None when it is damaged.
fn style(std: &[u8], base_size: usize) -> Option<Style> {
    let kind = u16_at(std, 2)?;
    let (stk, base) = ((kind & 0x000F) as u8, kind >> 4);
    let cupx = usize::from(u16_at(std, 4)? & 0x000F);
    // xstzName: a length, the characters and a terminating zero.
    let name = usize::from(u16_at(std, base_size)?);
    let mut at = base_size + 2 + 2 * name + 2;
    let mut upx = vec![];
    for _ in 0..cupx {
        let len = usize::from(u16_at(std, at)?);
        upx.push(bytes_at(std, at + 2, len)?);
        // Each LPUpx is padded to an even length.
        at += 2 + len + len % 2;
    }
    let (papx, chpx) = match (stk, upx.as_slice()) {
        // UpxPapx: the istd, then the paragraph properties; then UpxChpx.
        (1, [p, c, ..]) => (p.get(2..)?.to_vec(), c.to_vec()),
        (2, [c, ..]) => (vec![], c.to_vec()),
        _ => (vec![], vec![]),
    };
    Some(Style { stk, base, papx, chpx })
}

/// The font names of the font table SttbfFfn (§2.9.286).
fn fonts(sttb: &[u8], cmd: &str) -> Result<Vec<String>> {
    let broken = || damaged(cmd, "its font table is damaged");
    let count = usize::from(u16_at(sttb, 0).ok_or_else(broken)?).min(MAX_FONTS);
    let mut at = 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let len = usize::from(*sttb.get(at).ok_or_else(broken)?);
        let ffn = bytes_at(sttb, at + 1, len).ok_or_else(broken)?;
        // FFN: 39 bytes of family, weight, character set, PANOSE and signature, then the name.
        let units = ffn.get(39..).unwrap_or_default().as_chunks::<2>().0.iter().map(|&u| u16::from_le_bytes(u)).take_while(|&u| u != 0);
        let name: String = char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
        out.push(font_name(&name));
        at += 1 + len;
    }
    Ok(out)
}

/// Font name `name` trimmed, and cut at the last character that ends within [`MAX_NAME`] bytes.
fn font_name(name: &str) -> String {
    let name = name.trim();
    let end = (0..=MAX_NAME.min(name.len())).rev().find(|&i| name.is_char_boundary(i)).unwrap_or_default();
    name.get(..end).unwrap_or_default().to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// Character formatting.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Chars {
    /// The font's number in the font table.
    font: Option<u16>,
    half_points: u16,
    bold: bool,
    italic: bool,
    caps: bool,
    small_caps: bool,
    strike: bool,
    dstrike: bool,
    hidden: bool,
    underline: bool,
    script: Script,
    /// Raised (positive) or lowered text in half points.
    shift: i16,
    color: Option<[u8; 3]>,
    /// Deleted and inserted revisions.
    deleted: bool,
    inserted: bool,
    /// A special character (sprmCFSpec): a picture, a note reference or a symbol.
    special: bool,
}

impl Default for Chars {
    fn default() -> Self {
        Self {
            font: None,
            // The default size: 20 half points.
            half_points: 20,
            bold: false,
            italic: false,
            caps: false,
            small_caps: false,
            strike: false,
            dstrike: false,
            hidden: false,
            underline: false,
            script: Script::Normal,
            shift: 0,
            color: None,
            deleted: false,
            inserted: false,
            special: false,
        }
    }
}

/// The colors of `sprmCIco` (Ico, §2.9.119), 1 to 16; 0 is automatic.
const ICO: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 0xFF],
    [0, 0xFF, 0xFF],
    [0, 0xFF, 0],
    [0xFF, 0, 0xFF],
    [0xFF, 0, 0],
    [0xFF, 0xFF, 0],
    [0xFF, 0xFF, 0xFF],
    [0, 0, 0x80],
    [0, 0x80, 0x80],
    [0, 0x80, 0],
    [0x80, 0, 0x80],
    [0x80, 0, 0],
    [0x80, 0x80, 0],
    [0x80, 0x80, 0x80],
    [0xC0, 0xC0, 0xC0],
];

impl Chars {
    /// The character properties of `grpprl` applied over this formatting; a toggle operand of
    /// 0x80 or 0x81 takes or reverses the value of `style`, the formatting its styles give.
    fn apply(&mut self, grpprl: &[u8], style: &Chars) {
        each_prl(grpprl, |sprm, op| {
            let byte = op.first().copied();
            let word = u16_at(op, 0);
            let toggle = |state: &mut bool, of_style: bool| match byte {
                Some(0) => *state = false,
                Some(1) => *state = true,
                Some(0x80) => *state = of_style,
                Some(0x81) => *state = !of_style,
                _ => {}
            };
            match sprm {
                0x0800 => toggle(&mut self.deleted, style.deleted),
                0x0801 => toggle(&mut self.inserted, style.inserted),
                0x0835 => toggle(&mut self.bold, style.bold),
                0x0836 => toggle(&mut self.italic, style.italic),
                0x0837 => toggle(&mut self.strike, style.strike),
                0x083A => toggle(&mut self.small_caps, style.small_caps),
                0x083B => toggle(&mut self.caps, style.caps),
                0x083C => toggle(&mut self.hidden, style.hidden),
                0x0855 => toggle(&mut self.special, style.special),
                0x2A53 => toggle(&mut self.dstrike, style.dstrike),
                0x2A3E => self.underline = byte.is_some_and(|k| k != 0),
                0x2A42 => self.color = byte.and_then(|i| usize::from(i).checked_sub(1)).and_then(|i| ICO.get(i)).copied(),
                0x4A43 => self.half_points = word.filter(|v| (2..=3276).contains(v)).unwrap_or(self.half_points),
                0x4845 => self.shift = word.map_or(self.shift, |v| v as i16),
                0x2A48 => {
                    self.script = match byte {
                        Some(1) => Script::Super,
                        Some(2) => Script::Sub,
                        _ => Script::Normal,
                    }
                }
                0x4A4F => self.font = word.or(self.font),
                // COLORREF: red, green, blue, then 0xFF for the automatic color.
                0x6870 => {
                    if let [r, g, b, auto] = *op {
                        self.color = (auto != 0xFF).then_some([r, g, b]);
                    }
                }
                _ => {}
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Align {
    #[default]
    Left,
    Center,
    Right,
    /// Justified, the last line at the start, with the kashida length of the kashida alignments.
    Both(Kashida),
    /// Justified, the last line too.
    Distribute,
    Start,
    End,
}

/// Paragraph formatting; lengths in twips.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Para {
    align: Align,
    /// The physical left and right indents (`sprmPDxaLeft80`, `sprmPDxaRight80`).
    left: Option<i16>,
    right: Option<i16>,
    /// The logical start and end indents (`sprmPDxaLeft`, `sprmPDxaRight`).
    start: Option<i16>,
    end: Option<i16>,
    first: i16,
    before: u16,
    after: u16,
    rtl: Option<bool>,
    /// The paragraph's mark ends a table row (`sprmPFTtp`, or `sprmPFInnerTtp` in a nested table).
    row_end: bool,
    /// The paragraph's mark ends a cell of a nested table (`sprmPFInnerTableCell`).
    cell_end: bool,
}

impl Para {
    /// The paragraph properties of `grpprl` applied over this formatting.
    fn apply(&mut self, grpprl: &[u8]) {
        each_prl(grpprl, |sprm, op| {
            let byte = op.first().copied();
            let word = u16_at(op, 0);
            match sprm {
                0x2403 => {
                    self.align = match byte {
                        Some(1) => Align::Center,
                        Some(2) => Align::Right,
                        Some(3..=5) => Align::Both(Kashida::None),
                        Some(_) => Align::Left,
                        None => self.align,
                    }
                }
                0x2461 => {
                    self.align = match byte {
                        Some(0) => Align::Start,
                        Some(1) => Align::Center,
                        Some(2) => Align::End,
                        Some(3) => Align::Both(Kashida::None),
                        Some(5) => Align::Both(Kashida::Medium),
                        Some(7) => Align::Both(Kashida::Long),
                        Some(8) => Align::Both(Kashida::Short),
                        Some(4 | 9) => Align::Distribute,
                        _ => self.align,
                    }
                }
                0x840F => self.left = word.map(|v| v as i16),
                0x840E => self.right = word.map(|v| v as i16),
                0x845E => self.start = word.map(|v| v as i16),
                0x845D => self.end = word.map(|v| v as i16),
                0x8411 | 0x8460 => self.first = word.map_or(self.first, |v| v as i16),
                0xA413 => self.before = word.unwrap_or(self.before),
                0xA414 => self.after = word.unwrap_or(self.after),
                0x2441 => self.rtl = byte.map(|v| v != 0),
                0x2417 | 0x244C => self.row_end = byte.is_some_and(|v| v != 0),
                0x244B => self.cell_end = byte.is_some_and(|v| v != 0),
                _ => {}
            }
        });
    }

    /// The paragraph attributes of this formatting.
    fn style(&self) -> ParaStyle {
        let rtl = self.rtl == Some(true);
        let (physical_start, physical_end) = if rtl { (self.right, self.left) } else { (self.left, self.right) };
        let (start, end) = (self.start.or(physical_start).unwrap_or_default(), self.end.or(physical_end).unwrap_or_default());
        let (left, right) = if rtl { (end, start) } else { (start, end) };
        let points = |v: f64| (v / 20.0).clamp(-1296.0, 1296.0);
        ParaStyle {
            justify: match self.align {
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
            left_indent: points(f64::from(left)),
            right_indent: points(f64::from(right)),
            first_line_indent: points(f64::from(self.first)),
            space_before: points(f64::from(self.before)),
            space_after: points(f64::from(self.after)),
            direction: self.rtl.map(|rtl| if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight }),
            kashida: if let Align::Both(k) = self.align { k } else { Kashida::None },
            ..ParaStyle::default()
        }
    }
}

/// The kinds of content a warning names.
#[derive(Clone, Copy, Default)]
struct Seen {
    pictures: bool,
    /// Fields with a result.
    fields: bool,
    /// Fields with no result, or nested past [`MAX_FIELDS`].
    unread_fields: bool,
    revisions: bool,
    tables: bool,
}

struct Reader<'a> {
    sheet: &'a Sheet,
    fonts: &'a [String],
    /// The paragraph being read.
    para: Para,
    /// A paragraph that has ended (its formatting and its mark's): its break goes before the
    /// next text.
    pending: Option<(Para, Chars)>,
    /// The last cell's paragraph, which its row's break ends.
    cell_end: Option<(Para, Chars)>,
    /// A cell just ended: a tab goes before the next character of its row.
    cell_tab: bool,
    /// The fields open: true while their instructions are being read.
    fields: Vec<bool>,
    /// Fields begun past [`MAX_FIELDS`] and not yet ended.
    field_overflow: usize,
    /// The text, in runs of one character formatting; paragraphs end with `\n`.
    spans: Vec<(String, Chars)>,
    /// The bytes of text in `spans`.
    len: usize,
    /// The formatting of each paragraph that has ended.
    paras: Vec<Para>,
    /// Past [`MAX_PARAGRAPHS`], [`MAX_RUNS`] or [`text::MAX_TEXT`].
    full: bool,
    seen: Seen,
}

impl<'a> Reader<'a> {
    /// A reader of text formatted with the styles of `sheet` and the fonts of `fonts`.
    fn new(sheet: &'a Sheet, fonts: &'a [String]) -> Self {
        Self {
            sheet,
            fonts,
            para: Para::default(),
            pending: None,
            cell_end: None,
            cell_tab: false,
            fields: vec![],
            field_overflow: 0,
            spans: vec![],
            len: 0,
            paras: vec![],
            full: false,
            seen: Seen::default(),
        }
    }

    /// The text `text`, with the character properties of `chpx` and the paragraph properties of
    /// `papx`.
    fn read(&mut self, text: &[Char], chpx: &[Run], papx: &[Run]) {
        let mut start = 0;
        while start < text.len() && !self.full {
            // A paragraph runs to its mark: a paragraph or cell mark, or a page or section break at
            // the end of its paragraph properties.
            let ends = |&(c, fc, width): &Char| {
                matches!(c, '\r' | '\u{7}')
                    || (c == '\u{C}' && run_at(papx, fc).and_then(|i| papx.get(i)).is_some_and(|r| r.end == fc + u32::from(width)))
            };
            let end = text.get(start..).and_then(|rest| rest.iter().position(ends)).map_or(text.len(), |at| start + at + 1);
            let segment = text.get(start..end).unwrap_or_default();
            let mark_fc = segment.last().map(|&(_, fc, _)| fc).unwrap_or_default();
            let (para, base) = self.paragraph(run_at(papx, mark_fc).and_then(|i| papx.get(i)));
            self.para = para;
            let mut cached: Option<(Option<usize>, Chars)> = None;
            for (k, ch) in segment.iter().enumerate() {
                let run = run_at(chpx, ch.1);
                let chars = match cached {
                    Some((r, chars)) if r == run => chars,
                    _ => self.chars(base, run.and_then(|i| chpx.get(i)).map_or(&[][..], |r| r.grpprl.as_slice())),
                };
                cached = Some((run, chars));
                self.character(ch.0, chars, k + 1 == segment.len() && ends(ch));
            }
            start = end;
        }
    }

    /// A paragraph's formatting and its text's formatting before character styles and direct
    /// formatting, from its properties `run`.
    fn paragraph(&self, run: Option<&Run>) -> (Para, Chars) {
        let mut para = Para::default();
        let mut chars = Chars { font: self.sheet.font, ..Chars::default() };
        for s in self.sheet.chain(run.map_or(0, |r| r.istd), 1) {
            para.apply(&s.papx);
            let style = chars;
            chars.apply(&s.chpx, &style);
        }
        if let Some(r) = run {
            para.apply(&r.grpprl);
        }
        (para, chars)
    }

    /// The formatting of text whose paragraph gives `base` and whose properties are `grpprl`: its
    /// character style (`sprmCIstd`), then its direct formatting.
    fn chars(&self, base: Chars, grpprl: &[u8]) -> Chars {
        let mut style = base;
        each_prl(grpprl, |sprm, op| {
            if sprm == 0x4A30
                && let Some(istd) = u16_at(op, 0)
            {
                style = base;
                for s in self.sheet.chain(istd, 2) {
                    let before = style;
                    style.apply(&s.chpx, &before);
                }
            }
        });
        let mut chars = style;
        chars.apply(grpprl, &style);
        chars
    }

    /// Character `c` of the main text in formatting `chars`; `mark`: it ends its paragraph's
    /// properties (a page or section break that does is a paragraph's end).
    fn character(&mut self, c: char, chars: Chars, mark: bool) {
        if chars.inserted || chars.deleted {
            self.seen.revisions = true;
        }
        if chars.deleted {
            return;
        }
        match c {
            '\u{13}' => {
                if self.fields.len() < MAX_FIELDS && self.field_overflow == 0 {
                    self.fields.push(true);
                } else {
                    self.seen.unread_fields = true;
                    self.field_overflow += 1;
                }
                return;
            }
            '\u{14}' if self.field_overflow == 0 => {
                if let Some(instructions) = self.fields.last_mut() {
                    *instructions = false;
                    self.seen.fields = true;
                }
                return;
            }
            '\u{15}' => {
                if self.field_overflow > 0 {
                    self.field_overflow -= 1;
                } else if self.fields.pop() == Some(true) {
                    // A field that ends without a separator has no result.
                    self.seen.unread_fields = true;
                }
                return;
            }
            _ => {}
        }
        if self.field_overflow > 0 || self.fields.contains(&true) || chars.hidden {
            return;
        }
        let para = self.para;
        match c {
            '\r' if para.row_end => self.row_end(para, chars),
            '\r' if para.cell_end => self.cell(para, chars),
            '\r' => self.end_paragraph(para, chars),
            '\u{7}' if para.row_end => self.row_end(para, chars),
            '\u{7}' => self.cell(para, chars),
            '\u{C}' if mark => self.end_paragraph(para, chars),
            '\u{B}' => self.end_paragraph(para, chars),
            '\u{1}' | '\u{8}' => self.seen.pictures = true,
            '\u{1E}' => self.put('\u{2011}', chars),
            '\u{1F}' => self.put('\u{AD}', chars),
            '\t' => self.put('\t', chars),
            // Note references, separators and symbols.
            '(' if chars.special => {}
            c if c.is_control() => {}
            c => self.put(c, chars),
        }
    }

    /// The end of a paragraph in formatting `para` whose mark is in formatting `mark`.
    fn end_paragraph(&mut self, para: Para, mark: Chars) {
        self.flush();
        self.pending = Some((para, mark));
    }

    /// The end of a table cell: its paragraph goes on in the next cell, after a tab.
    fn cell(&mut self, para: Para, mark: Chars) {
        self.seen.tables = true;
        self.flush();
        if self.cell_tab {
            self.push('\t', mark);
        }
        self.cell_end = Some((para, mark));
        self.cell_tab = true;
    }

    /// The end of a table row: its cells' paragraph ends.
    fn row_end(&mut self, para: Para, mark: Chars) {
        self.seen.tables = true;
        self.flush();
        self.cell_tab = false;
        self.pending = Some(self.cell_end.take().unwrap_or((para, mark)));
    }

    /// Character `c` of the document's text in formatting `chars`.
    fn put(&mut self, c: char, chars: Chars) {
        self.flush();
        if std::mem::take(&mut self.cell_tab) {
            self.push('\t', chars);
        }
        self.push(c, chars);
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

    /// A warning for each kind of content that was left out or changed: what the text held, and
    /// the other stories `fib` gives a length.
    fn warnings(&self, fib: &Fib) -> Vec<String> {
        let mut out = vec![];
        if self.seen.pictures {
            out.push("the Word document's pictures and embedded objects were left out".to_string());
        }
        if self.seen.fields {
            out.push("the Word document's fields were placed as the text of their results".to_string());
        }
        // A field still in its instructions when the text ends has no result either.
        if self.seen.unread_fields || self.fields.contains(&true) {
            out.push("the Word document's fields without a result that could be read were left out".to_string());
        }
        if self.seen.revisions {
            out.push("the Word document's tracked changes were accepted: deleted text was left out and inserted text kept".to_string());
        }
        if self.seen.tables {
            out.push("the Word document's tables were placed as paragraphs with their cells separated by tabs".to_string());
        }
        let names = ["footnotes", "headers and footers", "comments", "endnotes", "text boxes"];
        let [ftn, hdd, atn, edn, txbx, hdr_txbx] = fib.stories;
        let present = [ftn, hdd, atn, edn, txbx.max(hdr_txbx)];
        let stories: Vec<&str> = names.iter().zip(present).filter(|(_, n)| *n > 0).map(|(name, _)| *name).collect();
        if let Some((last, rest)) = stories.split_last() {
            let list = if rest.is_empty() { (*last).to_string() } else { format!("{} and {last}", rest.join(", ")) };
            out.push(format!("the Word document's {list} were left out"));
        }
        out
    }

    /// The character attributes of formatting `c`.
    fn style(&self, c: &Chars) -> CharStyle {
        let mut s = CharStyle::default();
        if let Some(name) = c.font.and_then(|i| self.fonts.get(usize::from(i))).filter(|n| !n.is_empty()) {
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
        let paras = self.paras.iter().map(Para::style).collect();
        Ok(Story { runs, paras })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_text_maps_the_bytes_the_specification_lists() {
        let decoded: String = [b'a', 0x80, 0x82, 0x93, 0x94, 0x9E, 0x9F, 0xE9].map(compressed).iter().collect();
        assert_eq!(decoded, "a\u{80}‚“”\u{9E}Ÿé", "not Windows-1252: 0x80 and 0x9E are themselves");
    }

    #[test]
    fn operands_take_the_size_their_sprm_gives() {
        let mut seen = vec![];
        // sprmCFBold 1, sprmCHps 24, sprmCCv, a variable operand of 2 bytes, then a cut-off Prl.
        let grpprl = [0x35, 0x08, 0x01, 0x43, 0x4A, 24, 0, 0x70, 0x68, 1, 2, 3, 0, 0x2A, 0xC6, 2, 9, 9, 0x43, 0x4A, 1];
        each_prl(&grpprl, |sprm, op| seen.push((sprm, op.to_vec())));
        assert_eq!(seen, [(0x0835, vec![1]), (0x4A43, vec![24, 0]), (0x6870, vec![1, 2, 3, 0]), (0xC62A, vec![9, 9])]);
    }
}
