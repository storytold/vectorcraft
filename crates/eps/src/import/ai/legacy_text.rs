//! Type in the legacy format (version 7 specification §15): text objects `type To` … `TO`, their
//! text paths (`a b c d tx ty startPt Tp`, an area's frame, one for each linked area, or the path
//! type runs along, `TP`), the modal character and paragraph attributes
//! (`Tr Tf Ts Tz Tt TA Tl Ta Ti`, and §15.4.7 and §22's letter and word spacing `TC TW`, hanging
//! punctuation `Tq`, hyphenation `Th` and tab stops `Xb Xe XB`, and §15.3.4's writing direction `TV`,
//! Roman character rotation `Tv` and Kumi blocks `Ty TY`), the paint style, and the characters (`Tx`, `Tj`,
//! overflow `TX`; `\r` ends a paragraph). Type set in rendering modes 4 to 7 is a mask (§5.7): the
//! text object clips the objects of its clipping group. Final-form operators (the
//! computed positions, §15.1) are skipped: the type is laid out again. Strings are decoded by the
//! encoding pairs of the current font's `TZ`, then by the file's `TE` encoding unless that `TZ`'s
//! `useDefault` is 0, else as Windows-1252; a re-encoded font `/_name` is the font its `%AI3_BeginEncoding` or `TZ` names
//! (else `name`). Strings in a composite font named `base-CMap` (CJK type) whose `TZ` gives no
//! encoding pairs are decoded by that predefined CMap instead, and the font is `base`.

use std::sync::Arc;

use craft_pdf::interpret::hayro_cmap::{BfString, CMap, CMapName, load_embedded};
use kurbo::{Affine, BezPath, ParamCurve, ParamCurveArclen};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, Justify, NodeKind, ParaStyle, TabAlign, TabStop, TextKind, TextObject, TextRun};
use vectorcraft_geom::AnchorKind;

use super::super::obj::V;
use super::art::path_data;
use super::{FLIP, GState, HashSet, Kind, Node, NodeId, Reader};
use crate::latin;

/// Most characters of type read in one file.
const MAX_CHARS: usize = 1 << 22;
/// Largest type size and indent read (points).
const MAX_SIZE: f64 = 1e4;
/// Most re-encoded fonts read in one file, for their names and for their encodings each.
const MAX_FONTS: usize = 4096;
/// Most tab stops read in one tab definition.
const MAX_TABS: usize = 1024;
/// Largest tab stop position read (points), as `text.tabs.set` takes it.
const MAX_TAB_POSITION: f64 = 100_000.0;
/// Most tab stops copied in one text object into its runs of paragraphs with the same attributes
/// while it is read, and again, counted apart, into each of its paragraphs when it has more than one
/// run and into the frames of its linked areas after the first. The runs, paragraphs and frames past
/// it take the default stops, with a warning.
const MAX_STORY_TABS: usize = 1 << 20;
/// The warning when [`MAX_STORY_TABS`] leaves tab stops out.
const TABS_LEFT_OUT: &str = "tab stops past VectorCraft's limits were left out";
/// Most values read before one operator in a re-encoding section.
const MAX_ENCODING_VALUES: usize = 1024;
/// Most text paths read in one text object (its linked areas); the frames of others are left out,
/// with a warning.
const MAX_TEXT_PATHS: usize = 4096;
/// Most frames of linked areas times characters of their story: the story is laid out again
/// through its frames when the file opens. The frames of the areas past it are left out, with a
/// warning.
const MAX_THREAD_WORK: usize = MAX_CHARS;
/// The warning when [`MAX_TEXT_PATHS`] or [`MAX_THREAD_WORK`] leaves frames of linked areas out.
const LINKED_AREAS_LEFT_OUT: &str = "linked type areas past VectorCraft's limits were left out";
/// The warning when type set in a mask rendering mode can't be a clipping path: type in linked
/// areas (several frames), in wraparound text or in a compound path doesn't clip.
const MASK_NOT_CLIPPED: &str = "type used as a mask in linked areas, wraparound text or a compound path doesn't clip";
/// The warning when `TV` changes after a text object's first character: the object keeps the
/// direction of that character.
const DIRECTION_CHANGE_LEFT_OUT: &str = "type that changes writing direction inside a text object opens in the direction of its first character";

/// A text path (`Tp` … `TP`).
#[derive(Debug, Default)]
struct TextPath {
    /// `Tp`'s matrix: text space (y up) → art space.
    matrix: Option<Affine>,
    /// Where type on a path starts: a segment's index plus how far along it.
    start: f64,
    /// The frame or path, in art space, and the kind of the anchor each of its segments ends on.
    path: Option<(BezPath, Vec<AnchorKind>)>,
}

/// Paragraphs in a row that start with the same attributes.
#[derive(Debug)]
struct ParaRun {
    /// The attributes, as they were at the first character of each paragraph.
    style: ParaStyle,
    /// How many paragraphs.
    count: usize,
    /// [`TypeState::tabs_version`] when the last of them started.
    tabs_version: u64,
}

/// An overflow paragraph break (`(\r) TX`) read before any more type, with the state in force
/// where it was read.
#[derive(Debug)]
struct PendingBreak {
    /// `TV` at the break.
    vertical: bool,
    /// The attributes and [`TypeState::tabs_version`] of the empty paragraph the break ends, when
    /// it ends one that no character started.
    para: Option<(ParaStyle, u64)>,
}

/// A text object being read.
#[derive(Debug, Default)]
pub(super) struct LegacyText {
    /// `To`'s type: 0 point type, 1 area type, 2 type on a path.
    kind: i64,
    /// Its text paths in file order: area type has one for each linked area (§15.5.3).
    paths: Vec<TextPath>,
    /// Between `Tp` and `TP`: the path read is the text path.
    in_path: bool,
    /// It has more than [`MAX_TEXT_PATHS`] text paths: the paths read past them are left out.
    past_limit: bool,
    /// The story: the characters after every text path, in file order.
    runs: Vec<TextRun>,
    /// The paragraphs' attributes, as they were at the first character of each, by runs of
    /// paragraphs with the same attributes.
    paras: Vec<ParaRun>,
    /// The tab stops that the runs in `paras` hold.
    tab_stops: usize,
    /// [`MAX_STORY_TABS`] left tab stops out.
    tabs_left_out: bool,
    /// The current paragraph's attributes are in `paras`.
    para_taken: bool,
    /// An overflow paragraph break (`(\r) TX`) not yet followed by type: the story's end.
    pending_break: Option<PendingBreak>,
    /// Characters were set in a mask rendering mode (`Tr` 4 to 7): the text object is a clipping
    /// path (§5.7).
    clips: bool,
    /// `TV` at its first character: VectorCraft keeps one writing direction for each text object.
    vertical: bool,
    /// A later character was set in the other direction.
    direction_changed: bool,
    /// The state when it began (lock, visibility), and whether it began on a `%_` line.
    gs: GState,
    hidden: bool,
}

impl LegacyText {
    /// A paragraph starts, with the attributes `para` at [`TypeState::tabs_version`]
    /// `tabs_version`. When they are those of the paragraph before it, it joins that paragraph's
    /// run; else a new run starts, with the tab stops that [`MAX_STORY_TABS`] leaves room for.
    fn begin_para(&mut self, para: &mut ParaStyle, tabs_version: u64) {
        if let Some(run) = self.paras.last_mut() {
            // No tab definition since the run's last paragraph started: the stops are the same.
            let same = if run.tabs_version == tabs_version { same_but_tabs(&mut run.style, para) } else { run.style == *para };
            if same {
                run.count += 1;
                run.tabs_version = tabs_version;
                return;
            }
        }
        let style = within_tabs(para, &mut self.tab_stops, &mut self.tabs_left_out);
        self.paras.push(ParaRun { style, count: 1, tabs_version });
    }
}

/// Are `a` and `b` the same, leaving their tab stops out?
fn same_but_tabs(a: &mut ParaStyle, b: &mut ParaStyle) -> bool {
    let (a_tabs, b_tabs) = (std::mem::take(&mut a.tabs), std::mem::take(&mut b.tabs));
    let same = a == b;
    (a.tabs, b.tabs) = (a_tabs, b_tabs);
    same
}

/// A copy of `para`, with its tab stops when `copied` (the tab stops copied so far) has room for
/// them within [`MAX_STORY_TABS`], else with the default stops and `left_out` set.
fn within_tabs(para: &ParaStyle, copied: &mut usize, left_out: &mut bool) -> ParaStyle {
    let mut style = para.clone();
    if MAX_STORY_TABS.saturating_sub(*copied) >= style.tabs.len() {
        *copied += style.tabs.len();
    } else {
        *left_out = true;
        style.tabs = Vec::new();
    }
    style
}

/// The attributes of each paragraph of the runs `runs`, in order. A paragraph keeps its tab stops
/// while `copied` (the tab stops copied so far) has room for them within [`MAX_STORY_TABS`]; the
/// paragraphs past it take the default stops, and `left_out` is set.
fn each_paragraph(runs: Vec<ParaRun>, copied: &mut usize, left_out: &mut bool) -> Vec<ParaStyle> {
    let mut each = Vec::with_capacity(runs.iter().map(|r| r.count).sum());
    for run in runs {
        let stops = run.style.tabs.len();
        let room = MAX_STORY_TABS.saturating_sub(*copied);
        let kept = room.checked_div(stops).map_or(run.count, |n| run.count.min(n));
        *copied += kept * stops;
        let bare = (kept < run.count).then(|| ParaStyle { tabs: Vec::new(), ..run.style.clone() });
        each.extend(std::iter::repeat_n(run.style, kept));
        if let Some(bare) = bare {
            *left_out = true;
            each.extend(std::iter::repeat_n(bare, run.count - kept));
        }
    }
    each
}

/// The modal type attributes of the legacy format, which last from one text object to the next.
#[derive(Debug)]
pub(super) struct TypeState {
    style: CharStyle,
    para: ParaStyle,
    /// How many tab definitions set `para`'s tab stops: paragraphs that start at the same count
    /// start with the same stops.
    tabs_version: u64,
    /// `Tr`.
    render: i64,
    /// `TV`: vertical writing.
    vertical: bool,
    /// `TY`: a Kumi block recorded for later use, which horizontal type keeps as its mark.
    kumi_recorded: bool,
    /// The encoding strings are in, by code (`TE`); None where it says nothing.
    encoding: Vec<Option<char>>,
    /// Re-encoded fonts' original names (`%AI3_BeginEncoding: _new old`, or `TZ`'s).
    fonts: Vec<(String, String)>,
    /// Re-encoded fonts' own encodings (`TZ`), by the new font's name: what its encoding pairs
    /// change, by code (None where they say nothing), and whether `TE`'s encoding applies to it.
    font_encodings: Vec<(String, Vec<Option<char>>, bool)>,
    /// The new fonts' names whose `TZ` gives encoding pairs, which no composite font has.
    paired_fonts: Vec<String>,
    /// The font `Tf` set last, as the file names it.
    font: Option<String>,
    /// How strings in that font decode, when it is a composite font ([`composite_font`]).
    cmap: Option<Arc<CMapCodes>>,
    /// The predefined CMaps read so far, by name (`None` when one couldn't be read).
    cmaps: Vec<(String, Option<Arc<CMapCodes>>)>,
    /// Between `Xb` and `XB`: the tab stops of the tab definition read so far.
    tabs: Option<Vec<TabStop>>,
    chars: usize,
}

impl Default for TypeState {
    fn default() -> Self {
        Self {
            style: CharStyle::default(),
            para: ParaStyle::default(),
            tabs_version: 0,
            render: 0,
            vertical: false,
            kumi_recorded: false,
            encoding: vec![None; 256],
            fonts: vec![],
            font_encodings: vec![],
            paired_fonts: vec![],
            font: None,
            cmap: None,
            cmaps: vec![],
            tabs: None,
            chars: 0,
        }
    }
}

impl TypeState {
    /// `%AI3_BeginEncoding: _new old` (`line` the comment).
    pub(super) fn encoding_comment(&mut self, line: &str) {
        let Some((_, names)) = line.split_once(':') else { return };
        let mut words = names.split_whitespace();
        if let (Some(new), Some(old)) = (words.next(), words.next()) {
            self.font_names(new.trim_start_matches('/'), old.trim_start_matches('/'));
        }
    }

    /// Re-encoded font `new` is font `old`. The first statement for a name is used.
    fn font_names(&mut self, new: &str, old: &str) {
        if self.fonts.len() < MAX_FONTS && !self.fonts.iter().any(|(n, _)| n == new) {
            self.fonts.push((new.to_string(), old.to_string()));
        }
    }

    /// A string's bytes, in the current font.
    fn decode(&self, bytes: &[u8]) -> String {
        if let Some(cmap) = &self.cmap {
            return cmap.decode(bytes);
        }
        let own = self.font.as_ref().and_then(|f| self.font_encodings.iter().find(|(n, _, _)| n == f));
        let (own, te) = own.map_or((None, true), |(_, e, te)| (Some(e.as_slice()), *te));
        bytes
            .iter()
            .map(|&b| {
                if b == b'\r' {
                    return '\n';
                }
                own.and_then(|e| e.get(usize::from(b)).copied().flatten())
                    .or_else(|| if te { self.encoding.get(usize::from(b)).copied().flatten() } else { None })
                    .unwrap_or_else(|| latin::decode(b))
            })
            .collect()
    }

    /// How strings in a font with predefined CMap `cmap` decode; `None` when the CMap can't be read.
    fn cmap_codes(&mut self, cmap: &str) -> Option<Arc<CMapCodes>> {
        if let Some(form) = unicode_cmap(cmap) {
            return Some(Arc::new(CMapCodes::Unicode(form)));
        }
        if let Some((_, codes)) = self.cmaps.iter().find(|(n, _)| n == cmap) {
            return codes.clone();
        }
        let codes = cid_codes(cmap).map(Arc::new);
        self.cmaps.push((cmap.to_string(), codes.clone()));
        codes
    }
}

/// How a composite font's strings decode.
#[derive(Debug)]
enum CMapCodes {
    /// By a predefined CMap's codes to CIDs, then by its character collection's CID → Unicode CMap;
    /// with `high_bit`, each byte 0x21 to 0x7E gets its high bit set first. `space` is the CMap's
    /// code space ([`code_space`]).
    Cids { codes: Box<CMap>, unicode: Box<CMap>, high_bit: bool, space: Option<CodeSpace> },
    /// A predefined Unicode CMap: the codes are the characters.
    Unicode(Unicode),
}

/// A CMap's code space: ranges of codes, each given by its lowest and highest code, both with the
/// code's length in bytes.
type CodeSpace = &'static [(&'static [u8], &'static [u8])];

/// What control byte `b` is in a composite font's string: `\r` a paragraph break, `\t` a tab.
fn control(b: u8) -> Option<char> {
    match b {
        b'\r' => Some('\n'),
        b'\t' => Some('\t'),
        _ => None,
    }
}

impl CMapCodes {
    /// The characters of string `bytes`, leaving out codes the CMaps don't map; a `\r` is a
    /// paragraph break and a `\t` a tab (also as a byte left over after the last whole code of a
    /// UTF-16 or UTF-32 string).
    fn decode(&self, bytes: &[u8]) -> String {
        let (codes, unicode, high_bit, space) = match self {
            Self::Unicode(form) => {
                let unit = match form {
                    Unicode::Utf16 => 2,
                    Unicode::Utf8 => 1,
                    Unicode::Utf32 => 4,
                };
                let (whole, rest) = bytes.split_at_checked(bytes.len() - bytes.len() % unit).unwrap_or((bytes, &[]));
                let mut text = unicode_text(whole, *form).replace('\r', "\n");
                text.extend(rest.last().copied().and_then(control));
                return text;
            }
            Self::Cids { codes, unicode, high_bit, space } => (codes, unicode, *high_bit, space),
        };
        let euc: Vec<u8>;
        let bytes = if high_bit {
            euc = bytes.iter().map(|&b| if (0x21..0x7f).contains(&b) { b | 0x80 } else { b }).collect();
            euc.as_slice()
        } else {
            bytes
        };
        let mut out = String::new();
        let mut at = 0;
        while let Some(&first) = bytes.get(at) {
            // VectorCraft reads a 0x0D byte as a paragraph break and a 0x09 byte as a tab in every
            // CMap, as in other fonts' strings, including the row-and-cell CMaps, whose code space
            // has no one-byte codes.
            if let Some(control) = control(first) {
                out.push(control);
                at += 1;
                continue;
            }
            let rest = bytes.get(at..).unwrap_or_default();
            let code = |len: u8| rest.get(..usize::from(len)).map(|b| b.iter().fold(0u32, |c, &b| c << 8 | u32::from(b)));
            let found = match space {
                // The code is as long as the code space range it falls in; a byte that starts no
                // code in the code space is skipped.
                Some(space) => {
                    let Some(len) = code_len(space, rest) else {
                        at += 1;
                        continue;
                    };
                    code(len).map(|c| (len, codes.lookup_cid_code(c, len)))
                }
                None => (1..=4u8).find_map(|len| Some((len, Some(codes.lookup_cid_code(code(len)?, len)?)))),
            };
            let Some((len, cid)) = found else {
                at += 1;
                continue;
            };
            at += usize::from(len);
            // A code the CMap leaves out is dropped.
            let Some(cid) = cid else { continue };
            match unicode.lookup_bf_string(cid) {
                // A one-byte Roman code whose CID the table gives as a combining mark alone (0x7E in
                // the Mac Shift-JIS CMaps, U+0303): the ASCII character, as Adobe's cid2code.txt lists first.
                Some(BfString::Char('\u{300}'..='\u{36f}')) if len == 1 && (0x20..0x7f).contains(&first) => out.push(char::from(first)),
                Some(BfString::Char(c)) if c != char::REPLACEMENT_CHARACTER => out.push(c),
                Some(BfString::String(s)) => out.extend(s.chars().filter(|c| *c != char::REPLACEMENT_CHARACTER)),
                _ => {}
            }
        }
        out
    }
}

/// How a predefined Unicode CMap's codes encode their characters.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Unicode {
    /// Two bytes a character (`UCS2`) or a UTF-16 surrogate pair, the high byte first.
    Utf16,
    Utf8,
    /// Four bytes, the high byte first.
    Utf32,
}

/// The encoding of predefined Unicode CMap `name`: `Uni` and a character collection, then `-UCS2`,
/// `-UTF8`, `-UTF16` or `-UTF32` (and `-HW` for half-width forms), then `-H` or `-V`
/// (`UniJIS-UCS2-H`, `UniGB-UTF16-V`: <https://github.com/adobe-type-tools/cmap-resources>).
fn unicode_cmap(name: &str) -> Option<Unicode> {
    let rest = name.strip_suffix("-H").or_else(|| name.strip_suffix("-V"))?;
    let rest = rest.strip_suffix("-HW").unwrap_or(rest);
    let (collection, form) = rest.rsplit_once('-')?;
    let collections = ["UniJIS", "UniJISPro", "UniJIS2004", "UniJISX0213", "UniJISX02132004", "UniHojo", "UniGB", "UniCNS", "UniKS", "UniAKR"];
    if !collections.contains(&collection) {
        return None;
    }
    match form {
        "UCS2" | "UTF16" => Some(Unicode::Utf16),
        "UTF8" => Some(Unicode::Utf8),
        "UTF32" => Some(Unicode::Utf32),
        _ => None,
    }
}

/// The characters string `s` encodes in Unicode form `form` (invalid codes left out).
fn unicode_text(s: &[u8], form: Unicode) -> String {
    match form {
        Unicode::Utf16 => {
            let units = s.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b));
            char::decode_utf16(units).filter_map(Result::ok).collect()
        }
        Unicode::Utf8 => s.utf8_chunks().map(|chunk| chunk.valid()).collect(),
        Unicode::Utf32 => s.as_chunks::<4>().0.iter().filter_map(|b| char::from_u32(u32::from_be_bytes(*b))).collect(),
    }
}

/// Predefined CMap `name` among those VectorCraft carries, leaving out the Identity CMaps, whose
/// codes are CIDs of no known character collection.
fn predefined(name: &str) -> Option<CMapName<'_>> {
    match CMapName::from_bytes(name.as_bytes()) {
        CMapName::Custom(_) | CMapName::IdentityH | CMapName::IdentityV => None,
        name => Some(name),
    }
}

/// Adobe's predefined CMaps that VectorCraft doesn't carry, by name without `-H` or `-V`
/// (<https://github.com/adobe-type-tools/cmap-resources>): the nearest carried CMap with the same
/// code space, and whether each byte of a row-and-cell code gets its high bit set first (the EUC
/// form of the same code; `GB-H`, `CNS1-H` and `KSC-H` have no one-byte codes, their code space
/// is `<2121>` to `<7E7E>`); `None` for those no carried CMap reads.
fn stand_in(stem: &str) -> Option<Option<(&'static str, bool)>> {
    Some(match stem {
        "RKSJ" | "78-RKSJ" | "78ms-RKSJ" => Some(("90ms-RKSJ-H", false)),
        "78-EUC" => Some(("EUC-H", false)),
        "78" | "Add" | "Ext" | "NWP" => Some(("H", false)),
        "GB" => Some(("GB-EUC-H", true)),
        "CNS1" => Some(("CNS-EUC-H", true)),
        "KSC" => Some(("KSC-EUC-H", true)),
        "B5" => Some(("ETen-B5-H", false)),
        "ETHK-B5" | "HKdla-B5" | "HKdlb-B5" | "HKgccs-B5" | "HKm314-B5" | "HKm471-B5" => Some(("HKscs-B5-H", false)),
        "GBT" | "GBT-EUC" | "GBTpc-EUC" | "CNS2" | "KSC-Johab" => None,
        _ => return None,
    })
}

/// Predefined CMap `cmap`'s codes to CIDs, and its character collection's CID → Unicode CMap. A
/// vertical CMap's codes are read with its horizontal CMap when VectorCraft carries that: the two
/// differ only in the CIDs of vertical glyph forms. A CMap VectorCraft doesn't carry is read with
/// its [`stand_in`].
fn cid_codes(cmap: &str) -> Option<CMapCodes> {
    let read = |name: &str| CMap::parse(load_embedded(predefined(name)?)?, load_embedded);
    let horizontal = cmap.strip_suffix('V').map(|stem| format!("{stem}H"));
    let stem = cmap.strip_suffix("-H").or_else(|| cmap.strip_suffix("-V"));
    let (codes, high_bit) = horizontal.iter().map(String::as_str).chain([cmap]).find_map(|n| Some((read(n)?, false))).or_else(|| {
        let (name, high_bit) = stand_in(stem?).flatten()?;
        Some((read(name)?, high_bit))
    })?;
    let table = codes.metadata().character_collection.as_ref()?.family.ucs2_cmap()?;
    let unicode = CMap::parse(load_embedded(table)?, load_embedded)?;
    // A row-and-cell code read in its EUC form falls in the EUC CMap's code space.
    let space = if high_bit { stand_in(stem?).flatten().and_then(|(name, _)| code_space(name)) } else { code_space(cmap) };
    Some(CMapCodes::Cids { codes: Box::new(codes), unicode: Box::new(unicode), high_bit, space })
}

/// The code space of predefined CMap `cmap`, as Adobe's CMap files give it
/// (<https://github.com/adobe-type-tools/cmap-resources>); a CMap's vertical form has its
/// horizontal form's. `None` for a CMap not listed here.
fn code_space(cmap: &str) -> Option<CodeSpace> {
    const RKSJ: CodeSpace = &[(b"\x00", b"\x80"), (b"\xa0", b"\xdf"), (b"\x81\x40", b"\x9f\xfc"), (b"\xe0\x40", b"\xfc\xfc")];
    const RKSJ_MAC: CodeSpace = &[(b"\x00", b"\x80"), (b"\xa0", b"\xdf"), (b"\xfd", b"\xff"), (b"\x81\x40", b"\x9f\xfc"), (b"\xe0\x40", b"\xfc\xfc")];
    const EUC_JP: CodeSpace = &[(b"\x00", b"\x80"), (b"\x8e\xa0", b"\x8e\xdf"), (b"\xa1\xa1", b"\xfe\xfe")];
    const ROW_CELL: CodeSpace = &[(b"\x21\x21", b"\x7e\x7e")];
    const EUC: CodeSpace = &[(b"\x00", b"\x80"), (b"\xa1\xa1", b"\xfe\xfe")];
    const GB_PC: CodeSpace = &[(b"\x00", b"\x80"), (b"\xfd", b"\xff"), (b"\xa1\xa1", b"\xfc\xfe")];
    const GBK: CodeSpace = &[(b"\x00", b"\x80"), (b"\x81\x40", b"\xfe\xfe")];
    const GBK2K: CodeSpace = &[(b"\x00", b"\x7f"), (b"\x81\x40", b"\xfe\xfe"), (b"\x81\x30\x81\x30", b"\xfe\x39\xfe\x39")];
    const B5_PC: CodeSpace = &[(b"\x00", b"\x80"), (b"\xfd", b"\xff"), (b"\xa1\x40", b"\xfc\xfe")];
    const B5: CodeSpace = &[(b"\x00", b"\x80"), (b"\xa1\x40", b"\xfe\xfe")];
    const HKSCS: CodeSpace = &[(b"\x00", b"\x80"), (b"\x87\x40", b"\xfe\xfe")];
    const HKDLB: CodeSpace = &[(b"\x00", b"\x80"), (b"\x8e\x40", b"\xfe\xfe")];
    const HKGCCS: CodeSpace = &[(b"\x00", b"\x80"), (b"\x8a\x40", b"\xfe\xfe")];
    const CNS_EUC: CodeSpace = &[(b"\x00", b"\x80"), (b"\xa1\xa1", b"\xfe\xfe"), (b"\x8e\xa1\xa1\xa1", b"\x8e\xa3\xfe\xfe")];
    const KSC_PC: CodeSpace = &[(b"\x00", b"\x84"), (b"\xfe", b"\xff"), (b"\xa1\x41", b"\xfd\xfe")];
    const UHC: CodeSpace = &[(b"\x00", b"\x80"), (b"\x81\x41", b"\xfe\xfe")];
    let stem = match cmap {
        "H" | "V" => "",
        _ => cmap.strip_suffix("-H").or_else(|| cmap.strip_suffix("-V"))?,
    };
    let stem = stem.strip_suffix("-HW").unwrap_or(stem);
    Some(match stem {
        "83pv-RKSJ" | "90pv-RKSJ" => RKSJ_MAC,
        _ if stem.ends_with("RKSJ") => RKSJ,
        "EUC" | "78-EUC" => EUC_JP,
        "" | "78" | "Add" | "Ext" | "NWP" => ROW_CELL,
        "GB-EUC" | "GBT-EUC" | "KSC-EUC" => EUC,
        "GBpc-EUC" | "GBTpc-EUC" => GB_PC,
        "GBK-EUC" | "GBKp-EUC" => GBK,
        "GBK2K" => GBK2K,
        "B5pc" => B5_PC,
        "B5" | "ETen-B5" | "ETenms-B5" | "HKdla-B5" | "HKm314-B5" | "HKm471-B5" => B5,
        "HKscs-B5" | "ETHK-B5" => HKSCS,
        "HKdlb-B5" => HKDLB,
        "HKgccs-B5" => HKGCCS,
        "CNS-EUC" => CNS_EUC,
        "KSCpc-EUC" => KSC_PC,
        "KSCms-UHC" => UHC,
        _ => return None,
    })
}

/// The length of the code string `bytes` starts with: the length of the code space range whose
/// lowest and highest codes hold each of its bytes between them. `None` when no range does.
fn code_len(space: CodeSpace, bytes: &[u8]) -> Option<u8> {
    space
        .iter()
        .filter(|(lo, hi)| bytes.get(..lo.len()).is_some_and(|b| b.iter().zip(lo.iter().zip(hi.iter())).all(|(b, (lo, hi))| (lo..=hi).contains(&b))))
        .map(|(lo, _)| lo.len())
        .min()
        .and_then(|len| u8::try_from(len).ok())
}

/// Encoding pairs (§3.3.1): codes, each followed by the glyph names from it on, into `table`.
fn encoding_pairs(table: &mut [Option<char>], vals: &[V]) {
    let mut code: Option<usize> = None;
    for v in vals {
        match v {
            V::Num(n) if n.is_finite() && (0.0..256.0).contains(n) => code = Some(*n as usize),
            V::Name(name) => {
                if let Some(c) = code {
                    if let (Some(slot), Ok(b)) = (table.get_mut(c), u8::try_from(c)) {
                        *slot = latin::glyph(b, name);
                    }
                    code = Some(c + 1);
                }
            }
            _ => {}
        }
    }
}

/// A composite font's name `base-CMap`: the base font's name and its CMap's name, when that is a
/// predefined Unicode CMap ([`unicode_cmap`]), one of the predefined CMaps VectorCraft carries
/// ([`predefined`]) or one of the others ([`stand_in`]), in its horizontal (`-H`) or vertical
/// (`-V`) form. A name ending in an Identity CMap is not read as one, nor is one that ends in an
/// unknown CMap name with an encoding part (`UniFoo-UTF8-H`), which would otherwise end in `H`.
fn composite_font(name: &str) -> Option<(&str, &str)> {
    if !(name.ends_with("-H") || name.ends_with("-V")) {
        return None;
    }
    // The longest CMap name first: `83pv-RKSJ-H` before `H`.
    // No predefined CMap name is longer than 32 bytes.
    for (at, _) in name.match_indices('-').filter(|(at, _)| name.len() - at <= 33) {
        let (Some(base), Some(cmap)) = (name.get(..at).filter(|b| !b.is_empty()), name.get(at + 1..)) else { continue };
        if matches!(cmap, "Identity-H" | "Identity-V") {
            return None;
        }
        let encoding_part =
            base.rsplit('-').next().is_some_and(|part| ["UCS2", "UTF8", "UTF16", "UTF32", "HW", "EUC", "RKSJ", "B5", "UHC", "Johab"].contains(&part));
        if matches!(cmap, "H" | "V") && encoding_part {
            return None;
        }
        let vertical_of_predefined = || cmap.strip_suffix('V').is_some_and(|stem| predefined(&format!("{stem}H")).is_some());
        let other = || cmap.strip_suffix("-H").or_else(|| cmap.strip_suffix("-V")).is_some_and(|stem| stand_in(stem).is_some());
        if unicode_cmap(cmap).is_some() || predefined(cmap).is_some() || vertical_of_predefined() || other() {
            return Some((base, cmap));
        }
    }
    None
}

/// An operator's `N` operands, the last `N` values before it, when each is a finite number.
fn nums<const N: usize>(vals: &[V]) -> Option<[f64; N]> {
    let operands = vals.get(vals.len().checked_sub(N)?..)?;
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(operands) {
        *o = v.num().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// A flag operand: 0 or 1.
fn flag(v: f64) -> Option<bool> {
    match v {
        0.0 => Some(false),
        1.0 => Some(true),
        _ => None,
    }
}

impl Reader<'_> {
    /// `[encodingPairs TE`: codes, each followed by the glyph names from it on (§3.3.1).
    pub(super) fn te(&mut self, vals: &[V]) {
        encoding_pairs(&mut self.type_state.encoding, vals);
    }

    /// `[encodingPairs newFontName oldFontName direction fontScript useDefault TZ`, with a Multiple
    /// Master weight vector `[w0 … wn]` after it, and without the encoding pairs or the direction
    /// in some versions (§3.3.1): the new font's own encoding pairs, which strings in it are
    /// decoded by first. `useDefault` 1 (or no number after the names) decodes the codes the pairs
    /// leave out by `TE`'s encoding, and 0 leaves `TE`'s encoding out for the font. The font names
    /// are the last two names; only numbers and the weight vector follow them. The first `TZ` for
    /// a font is used.
    pub(super) fn tz(&mut self, vals: &[V]) {
        let mut names = vals.iter().enumerate().filter_map(|(i, v)| if let V::Name(n) = v { Some((i, n)) } else { None }).rev();
        let (Some((old_at, old)), Some((new_at, new))) = (names.next(), names.next()) else { return };
        if new_at + 1 != old_at {
            return;
        }
        let tail = vals.get(old_at + 1..).unwrap_or_default();
        let weights = matches!(tail.last(), Some(V::Arr(_)));
        let numbers = tail.get(..tail.len() - usize::from(weights)).unwrap_or_default();
        if !numbers.iter().all(|v| matches!(v, V::Num(_))) {
            return;
        }
        let te = !matches!(numbers.last(), Some(V::Num(n)) if *n == 0.0);
        let st = &mut self.type_state;
        st.font_names(new, old);
        if st.font_encodings.iter().any(|(n, _, _)| n == new) || st.font_encodings.len() >= MAX_FONTS {
            return;
        }
        // The pairs start after the `[` that opens them.
        let pairs = vals.get(..new_at).unwrap_or_default();
        let pairs = pairs.iter().rposition(|v| matches!(v, V::Mark)).map_or(pairs, |m| pairs.get(m + 1..).unwrap_or_default());
        let mut table = vec![None; 256];
        encoding_pairs(&mut table, pairs);
        if pairs.iter().any(|v| matches!(v, V::Name(_))) {
            st.paired_fonts.push(new.clone());
        }
        st.font_encodings.push((new.clone(), table, te));
    }

    /// What is between `%AI3_BeginEncoding` and `%AI3_EndEncoding` (§3.3): its `TZ` is read, and
    /// every other operator is skipped with its operands.
    pub(super) fn encoding_section(&mut self, section: &[u8]) {
        let mut lex = super::Lexer::new(section);
        let mut vals: Vec<V> = Vec::new();
        let mut marks: Vec<usize> = Vec::new();
        // `[`s past the value limit, which are not kept: as many `]`s after them are skipped.
        let mut dropped: usize = 0;
        while let Some(t) = lex.next_token() {
            let v = match t.tok {
                super::Tok::Num(n) => V::Num(n),
                super::Tok::Name(n) => V::Name(super::obj::text(n)),
                super::Tok::Str(s) | super::Tok::Hex(s) => V::Str(s),
                super::Tok::Word(b"[") => {
                    // Each `[` kept is a value, so the open arrays are no more than the values.
                    if vals.len() >= MAX_ENCODING_VALUES {
                        dropped = dropped.saturating_add(1);
                        continue;
                    }
                    marks.push(vals.len());
                    V::Mark
                }
                super::Tok::Word(b"]") => {
                    if dropped > 0 {
                        dropped -= 1;
                        continue;
                    }
                    let Some(m) = marks.pop() else { continue };
                    let items: Vec<V> = vals.split_off(m.min(vals.len())).into_iter().skip(1).collect();
                    V::Arr(items)
                }
                super::Tok::Word(w) => {
                    if w == b"TZ" {
                        self.tz(&vals);
                    }
                    vals.clear();
                    marks.clear();
                    dropped = 0;
                    continue;
                }
                _ => continue,
            };
            if vals.len() < MAX_ENCODING_VALUES {
                vals.push(v);
            }
        }
    }

    /// Is a text object's path being read (so the path painted is its frame or path)?
    pub(super) fn in_text_path(&self) -> bool {
        self.text.as_ref().is_some_and(|t| t.in_path)
    }

    /// `type To`: a text object begins.
    pub(super) fn begin_text(&mut self, vals: &[V], hidden: bool) {
        self.end_path_quietly();
        if self.pattern.is_some() {
            self.unreadable("type in a pattern", false);
        }
        let kind = nums::<1>(vals).map_or(0, |[k]| k as i64);
        // A tab definition left open by an earlier text object (an `Xb` with no `XB`) ends here.
        self.type_state.tabs = None;
        self.text = Some(LegacyText { kind, gs: self.gs.clone(), hidden, ..LegacyText::default() });
    }

    /// `a b c d tx ty startPt Tp`: the text path begins.
    pub(super) fn begin_text_path(&mut self, vals: &[V]) {
        self.end_path_quietly();
        let Some(t) = self.text.as_mut() else { return };
        // Seven operands, or the matrix alone.
        let (m, start) = match vals.len() {
            7 => (vals.get(..6).and_then(nums::<6>), vals.get(6).and_then(V::num)),
            6 => (nums::<6>(vals), None),
            _ => (None, None),
        };
        let matrix = m.map(Affine::new).filter(|m| {
            let det = m.determinant();
            det.is_finite() && det.abs() > 1e-12
        });
        let start = start.filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0);
        t.past_limit |= t.paths.len() >= MAX_TEXT_PATHS;
        if !t.past_limit {
            t.paths.push(TextPath { matrix, start, path: None });
        }
        t.in_path = true;
    }

    /// The path painted (or `TP`, ending the text path): the text path's frame or path.
    pub(super) fn text_path(&mut self) {
        let path = std::mem::take(&mut self.path);
        let kinds = std::mem::take(&mut self.path_kinds);
        self.end_path_quietly();
        if let Some(p) = self.text.as_mut().filter(|t| !t.past_limit).and_then(|t| t.paths.last_mut())
            && p.path.is_none()
            && !path.elements().is_empty()
        {
            p.path = Some((path, kinds));
        }
    }

    /// `TP`.
    pub(super) fn end_text_path(&mut self) {
        self.text_path();
        if let Some(t) = self.text.as_mut() {
            t.in_path = false;
        }
    }

    /// A character or paragraph attribute. An attribute whose operands are missing, are not
    /// numbers, or are flags other than 0 and 1 stays as it was.
    pub(super) fn text_attr(&mut self, w: &str, vals: &[V]) {
        let st = &mut self.type_state;
        let n0 = nums::<1>(vals).map(|[v]| v);
        match w {
            // `render Tr`: a mode from 0 to 9 (§15.4.5.1).
            "Tr" => {
                if let Some(v) = n0.filter(|v| v.fract() == 0.0 && (0.0..=9.0).contains(v)) {
                    st.render = v as i64;
                }
            }
            "Tf" => {
                // `/name size Tf`, or `/name size ascent descent Tf`.
                let Some(at) = vals.iter().position(|v| matches!(v, V::Name(_))) else { return };
                if let Some(V::Name(name)) = vals.get(at) {
                    st.font = Some(name.clone());
                    let full =
                        st.fonts.iter().find(|(new, _)| new == name).map_or_else(|| name.trim_start_matches('_').to_string(), |(_, old)| old.clone());
                    // A font whose `TZ` gives encoding pairs is re-encoded, which a composite font
                    // can't be; a `TZ` without pairs only names the font.
                    let reencoded = st.paired_fonts.contains(name);
                    let (name, cmap) = match composite_font(&full).filter(|_| !reencoded) {
                        Some((base, cmap)) => (base, st.cmap_codes(cmap)),
                        None => (full.as_str(), None),
                    };
                    st.cmap = cmap;
                    let (family, style) = super::super::super::text::family_style(name);
                    st.style.font_family = family;
                    st.style.font_style = style;
                    st.style.font_version = None;
                }
                if let Some(size) = vals.get(at + 1).and_then(V::num).filter(|s| s.is_finite() && *s > 0.0) {
                    st.style.size = size.min(MAX_SIZE);
                }
            }
            "Ts" => {
                if let Some(v) = n0 {
                    st.style.baseline_shift = v.clamp(-MAX_SIZE, MAX_SIZE);
                }
            }
            "Tz" => {
                // `parallelScale perpendicularScale Tz`; before version 7, `scale Tz`.
                let scale = |v: f64| v.clamp(1.0, 10_000.0);
                let scales = match vals.len() {
                    2 => nums::<2>(vals),
                    1 => n0.map(|h| [h, 100.0]),
                    _ => None,
                };
                if let Some([h, v]) = scales {
                    (st.style.h_scale, st.style.v_scale) = (scale(h), scale(v));
                }
            }
            "Tt" => {
                if let Some(v) = n0 {
                    st.style.tracking = v.clamp(-10_000.0, 10_000.0);
                }
            }
            "TA" => {
                if let Some(auto) = n0.and_then(flag) {
                    st.style.kerning = if auto { None } else { Some(0.0) };
                }
            }
            "Tl" => {
                // `leading paragraphLeading Tl`; before version 7, `leading Tl`.
                let leading = match vals.len() {
                    2 => nums::<2>(vals),
                    1 => n0.map(|l| [l, 0.0]),
                    _ => None,
                };
                if let Some([l, before]) = leading {
                    let l = l.clamp(0.0, MAX_SIZE);
                    // The automatic leading, 120% of the size, stays automatic.
                    st.style.leading = if (l - st.style.size * 1.2).abs() < 1e-3 { None } else { Some(l) };
                    st.para.space_before = before.clamp(-MAX_SIZE, MAX_SIZE);
                }
            }
            "Ta" => {
                let justify = match n0 {
                    Some(0.0) => Some(Justify::Left),
                    Some(1.0) => Some(Justify::Center),
                    Some(2.0) => Some(Justify::Right),
                    Some(3.0) => Some(Justify::JustifyLeft),
                    Some(4.0) => Some(Justify::JustifyAll),
                    _ => None,
                };
                if let Some(j) = justify {
                    st.para.justify = j;
                }
            }
            "Ti" => {
                if let Some(i) = nums::<3>(vals) {
                    let [first, left, right] = i.map(|v| v.clamp(-MAX_SIZE, MAX_SIZE));
                    (st.para.first_line_indent, st.para.left_indent, st.para.right_indent) = (first, left, right);
                }
            }
            // `min opt max TC` (letter spacing) and `min opt max TW` (word spacing), in percent of
            // a space's width, as the paragraph's Justification options take them.
            "TC" | "TW" => {
                if let Some(v) = nums::<3>(vals) {
                    let j = &mut st.para.justification;
                    if w == "TC" {
                        j.letter_spacing = v;
                    } else {
                        j.word_spacing = v;
                    }
                    j.keep_in_range();
                }
            }
            // `hangingQuotes Tq`.
            "Tq" => {
                if let Some(on) = n0.and_then(flag) {
                    st.para.roman_hanging = on;
                }
            }
            // `hyphenate limitHyphLines minLeadHyphen minTailHyphen maxHyphLines Th`, kept to the
            // ranges `text.setFormat` takes.
            "Th" => {
                let Some([on, limit, lead, tail, max]) = nums::<5>(vals) else { return };
                let (Some(on), Some(limit)) = (flag(on), flag(limit)) else { return };
                let letters = |v: f64, lo: f64, hi: f64| v.round().clamp(lo, hi) as u32;
                st.para.hyphenate = on;
                let h = &mut st.para.hyphenation;
                h.after_first = letters(lead, 1.0, 15.0);
                h.before_last = letters(tail, 1.0, 15.0);
                h.hyphen_limit = if limit { letters(max, 0.0, 25.0) } else { 0 };
            }
            // `direction TV` (§15.3.4): 0 horizontal, 1 vertical.
            "TV" => {
                if let Some(on) = n0.and_then(flag) {
                    st.vertical = on;
                }
            }
            // `rotation Tv`: in vertical writing, 0 rotates Roman characters 90 degrees clockwise and
            // 1 keeps them upright.
            "Tv" => {
                if let Some(on) = n0.and_then(flag) {
                    st.style.standard_vertical_roman = on;
                }
            }
            // `kumi Ty`: 1 begins a Kumi block and 0 ends it.
            "Ty" => {
                if let Some(on) = n0.and_then(flag) {
                    st.style.tate_chu_yoko = on;
                }
            }
            // `kumi TY`: as `Ty`, for a block that does not appear in the set text.
            "TY" => {
                if let Some(on) = n0.and_then(flag) {
                    st.kumi_recorded = on;
                }
            }
            // A tab definition (§22.6): `numDots numTabs Xb` begins it, each
            // `leader decimal tabType tabPosition Xe` describes one tab stop, and `XB` ends it. The
            // stops it describes replace the paragraph's tab stops, sorted by position, one stop at
            // each position; a definition with none (or whose descriptions were all skipped) leaves
            // the default stops. The dot-leader distance, the count and the two reserved operands are
            // not read. A description whose type is not 1 to 4 (left, center, right, decimal) or
            // whose position is not a number from 0 to [`MAX_TAB_POSITION`] is skipped, and so is one
            // outside a tab definition or past the first [`MAX_TABS`] of one. A text object's `To`
            // ends a definition that an earlier object left open.
            "Xb" => st.tabs = Some(vec![]),
            "Xe" => {
                let Some(tabs) = st.tabs.as_mut().filter(|t| t.len() < MAX_TABS) else { return };
                let Some([kind, position]) = nums::<2>(vals) else { return };
                let align = match kind {
                    1.0 => TabAlign::Left,
                    2.0 => TabAlign::Center,
                    3.0 => TabAlign::Right,
                    4.0 => TabAlign::Decimal,
                    _ => return,
                };
                if (0.0..=MAX_TAB_POSITION).contains(&position) {
                    tabs.push(TabStop { position, align, leader: String::new(), align_on: '.' });
                }
            }
            "XB" => {
                if let Some(mut tabs) = st.tabs.take() {
                    tabs.sort_by(|a, b| a.position.total_cmp(&b.position));
                    tabs.dedup_by(|a, b| (a.position - b.position).abs() < 1e-6);
                    st.para.tabs = tabs;
                    st.tabs_version = st.tabs_version.wrapping_add(1);
                }
            }
            _ => {}
        }
    }

    /// `(…) Tx` (`Tj`, overflow `TX`): characters in the current style and paint.
    pub(super) fn text_body(&mut self, w: &str, vals: &[V]) {
        let Some(bytes) = vals.iter().rev().find_map(|v| if let V::Str(s) = v { Some(s.clone()) } else { None }) else { return };
        let st = &mut self.type_state;
        let Some(t) = self.text.as_mut() else { return };
        let decoded = st.decode(&bytes);
        if decoded.is_empty() {
            return;
        }
        // Overflow text ending the story with a paragraph break (`(\r) TX`) adds no paragraph.
        if w == "TX" && decoded == "\n" {
            if t.pending_break.is_none() {
                let para = (!t.para_taken).then(|| (st.para.clone(), st.tabs_version));
                t.pending_break = Some(PendingBreak { vertical: st.vertical, para });
            }
            return;
        }
        // As many characters as the file's allowance has left.
        let room = MAX_CHARS.saturating_sub(st.chars);
        if room == 0 {
            return;
        }
        let decoded: String = decoded.chars().take(room).collect();
        st.chars += decoded.chars().count();
        // A break read before these characters keeps the direction and paragraph attributes it was
        // read with, and is the story's first character when nothing came before it.
        let brk = t.pending_break.take();
        if t.runs.is_empty() {
            t.vertical = brk.as_ref().map_or(st.vertical, |b| b.vertical);
        }
        t.direction_changed |= st.vertical != t.vertical || brk.as_ref().is_some_and(|b| b.vertical != t.vertical);
        let gs = &self.gs;
        let mut style = st.style.clone();
        style.tate_chu_yoko |= st.kumi_recorded && !t.vertical;
        // Characters in modes 4 to 7 make the whole text object the mask, not each run (§5.7); 7
        // paints nothing, nor does 3 (invisible).
        t.clips |= (4..=7).contains(&st.render);
        let (fill, stroke) = match st.render {
            0 | 4 => (true, false),
            1 | 5 => (false, true),
            2 | 6 => (true, true),
            _ => (false, false),
        };
        style.fill = if fill { gs.fill.clone() } else { Paint::None };
        style.stroke = if stroke { gs.stroke.clone() } else { Paint::None };
        style.stroke_width = if stroke { gs.line_width() } else { 0.0 };
        (style.stroke_cap, style.stroke_join, style.stroke_miter_limit, style.stroke_dash) = (gs.cap, gs.join, gs.miter, gs.dash.clone());
        (style.overprint_fill, style.overprint_stroke) = (gs.overprint_fill, gs.overprint_stroke);
        let mut text = String::new();
        if let Some(brk) = brk {
            if let Some((mut para, tabs_version)) = brk.para {
                t.begin_para(&mut para, tabs_version);
            }
            text.push('\n');
            t.para_taken = false;
        }
        for c in decoded.chars() {
            if c != '\n' && !t.para_taken {
                t.begin_para(&mut st.para, st.tabs_version);
                t.para_taken = true;
            }
            text.push(c);
            if c == '\n' {
                if !t.para_taken {
                    t.begin_para(&mut st.para, st.tabs_version);
                }
                t.para_taken = false;
            }
        }
        match t.runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => t.runs.push(TextRun::new(text, style)),
        }
    }

    /// `TO`: the text object ends. Area type with several linked areas becomes one frame for each
    /// area, threaded in file order: the first frame holds the story, and the flow from area to
    /// area (final-form, §15.1) is laid out again. The frames are added last to first, so the
    /// object's dictionary that follows names the first one ([`Reader::each_linked_frame`]).
    /// Point type is read at its first text path; area type and type on a path at the first one
    /// that has a frame or path (else at the first one, as point type).
    pub(super) fn end_text(&mut self, hidden: bool) -> Result<(), String> {
        let Some(mut t) = self.text.take() else { return Ok(()) };
        if t.runs.is_empty() {
            return Ok(());
        }
        let to_doc = self.space();
        let chars: usize = t.runs.iter().map(|r| r.text.chars().count()).sum();
        let most = (MAX_THREAD_WORK / chars.max(1)).max(1);
        let all_areas = t.paths.iter().filter(|p| p.path.is_some()).count();
        // Area type with linked areas, also when the limits leave one frame of it.
        let linked = t.kind == 1 && (all_areas > 1 || t.past_limit);
        let areas: Vec<&TextPath> = t.paths.iter().filter(|p| p.path.is_some()).take(most).collect();
        let placed: Vec<(Affine, TextKind)> = match t.kind {
            1 if areas.len() > 1 => areas.into_iter().map(|p| placed(1, Some(p))).collect(),
            0 => vec![placed(0, t.paths.first())],
            kind => vec![placed(kind, areas.first().copied().or(t.paths.first()))],
        };
        if t.kind == 1 && (t.past_limit || all_areas > placed.len()) {
            self.warn(LINKED_AREAS_LEFT_OUT);
        }
        if t.direction_changed {
            self.warn(DIRECTION_CHANGE_LEFT_OUT);
        }
        let mut paras = std::mem::take(&mut t.paras);
        if paras.is_empty() {
            paras.push(ParaRun { style: self.type_state.para.clone(), count: 1, tabs_version: 0 });
        }
        let last_para = paras.last().map(|r| r.style.clone()).unwrap_or_default();
        let last_style = t.runs.last().map(|r| r.style.clone()).unwrap_or_default();
        let mut story = Some((std::mem::take(&mut t.runs), paras));
        // The tab stops copied into the paragraphs and frames.
        let mut copied = 0;
        let mut nodes = Vec::with_capacity(placed.len());
        for (m, kind) in placed {
            let mut obj = TextObject::point(kurbo::Point::ZERO, "", last_style.clone());
            obj.kind = kind;
            obj.vertical = t.vertical;
            obj.xf = to_doc * m * FLIP;
            match story.take() {
                Some((runs, paras)) => {
                    obj.runs = runs;
                    // Paragraphs that all have the same attributes share one copy of them.
                    match <[ParaRun; 1]>::try_from(paras) {
                        Ok([one]) => obj.set_all_paras(one.style),
                        Err(paras) => obj.set_paragraph_styles(each_paragraph(paras, &mut copied, &mut t.tabs_left_out)),
                    }
                }
                None => obj.set_all_paras(within_tabs(&last_para, &mut copied, &mut t.tabs_left_out)),
            }
            nodes.push(self.new_node(NodeKind::Text(Box::new(obj)), &t.gs));
        }
        if t.tabs_left_out {
            self.warn(TABS_LEFT_OUT);
        }
        let frames: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
        // Type used as a mask clips the objects of its clipping group, as a `W` path does. A
        // clipping path is one object, and wraparound text and compound paths don't clip their
        // objects with type, so the frames of linked areas and type in those don't clip, with a
        // warning. The frames up to the clipping group or opacity mask's art are checked, a group in
        // either included.
        let in_wrap_or_compound = self
            .frames
            .iter()
            .rev()
            .take_while(|f| !matches!(f.kind, Kind::Clip | Kind::Mask | Kind::Layer(_)))
            .any(|f| matches!(f.kind, Kind::Wrap | Kind::Compound));
        let can_clip = !linked && nodes.len() == 1 && !in_wrap_or_compound;
        if t.clips && !can_clip {
            self.warn(MASK_NOT_CLIPPED);
        }
        for node in nodes.into_iter().rev() {
            if t.clips
                && can_clip
                && let Some(f) = self.frames.last_mut()
            {
                f.clips.push(f.children.len());
            }
            self.add(node, t.hidden || hidden)?;
        }
        if frames.len() > 1 {
            self.doc.text_threads.push(frames);
        }
        Ok(())
    }

    /// Call `f` on the other frames of the threaded text object just read, whose first frame is
    /// the last object ([`Reader::end_text`]): the opacity mask and text wrap of the object's
    /// dictionary apply to every frame.
    pub(super) fn each_linked_frame(&mut self, mut f: impl FnMut(&mut Node)) {
        if !self.after_object {
            return;
        }
        let Some(first) = self.frames.last().and_then(|fr| fr.children.last()).map(|n| n.id) else { return };
        let Some(thread) = self.doc.text_threads.last().filter(|t| t.first() == Some(&first)) else { return };
        let rest: HashSet<NodeId> = thread.iter().skip(1).copied().collect();
        let Some(fr) = self.frames.last_mut() else { return };
        for n in fr.children.iter_mut().rev().skip(1).take(rest.len()).filter(|n| rest.contains(&n.id)) {
            f(n);
        }
    }
}

/// A text object of type `kind` on text path `p` (none: at the origin): the matrix from text space
/// (y up) to art space, and the kind of type with its frame or path in text space (y down).
fn placed(kind: i64, p: Option<&TextPath>) -> (Affine, TextKind) {
    let m = p.and_then(|p| p.matrix).unwrap_or(Affine::IDENTITY);
    // Art space → text space (y down).
    let to_text = FLIP * m.inverse();
    let kind = match (kind, p.and_then(|p| p.path.as_ref().map(|path| (path, p.start)))) {
        (1, Some(((path, kinds), _))) => TextKind::Area { frame: path_data(&(to_text * path.clone()), kinds) },
        (2, Some(((path, kinds), start))) => {
            TextKind::OnPath { path: path_data(&(to_text * path.clone()), kinds), start: start_fraction(path, start), end: None }
        }
        _ => TextKind::Point,
    };
    (m, kind)
}

/// Where type on `path` starts (`index.fraction`: a segment and how far along it, §15.5.2), as a
/// fraction of its length.
fn start_fraction(path: &BezPath, start: f64) -> f64 {
    const ACCURACY: f64 = 1e-3;
    let segments: Vec<kurbo::PathSeg> = path.segments().collect();
    let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if !(total.is_finite() && total > 0.0 && start.is_finite()) {
        return 0.0;
    }
    let index = start.floor().max(0.0) as usize;
    let Some(seg) = segments.get(index) else { return 1.0 };
    let before: f64 = lengths.iter().take(index).sum();
    let along = seg.subsegment(0.0..(start - start.floor()).clamp(0.0, 1.0)).arclen(ACCURACY);
    ((before + along) / total).clamp(0.0, 1.0)
}
