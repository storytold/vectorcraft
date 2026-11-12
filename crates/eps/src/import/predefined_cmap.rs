//! Strings in fonts over Adobe's predefined CMaps (<https://github.com/adobe-type-tools/cmap-resources>)
//! read as characters: the codes of a predefined Unicode CMap are the characters; those of the
//! others map to CIDs of a character collection, whose CID → Unicode CMap gives the characters.
//! Used by legacy `.ai` type in composite fonts and by type in CID-keyed fonts found by name.

use craft_pdf::interpret::hayro_cmap::{BfString, CMap, CMapName, load_embedded};

/// How a composite font's strings decode.
#[derive(Debug)]
pub(super) enum CMapCodes {
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
    pub(super) fn decode(&self, bytes: &[u8]) -> String {
        let mut text = String::new();
        self.each_code(bytes, |_, chars| text.extend(chars));
        text
    }

    /// The codes of string `bytes`, each with its value and the characters it stands for (as
    /// [`Self::decode`] reads them; none for a code the CMaps don't map, or for a byte that starts
    /// no code, which counts as a code of its own).
    pub(super) fn codes(&self, bytes: &[u8]) -> Decoded {
        let mut out = Decoded::default();
        self.each_code(bytes, |code, chars| out.push(code, chars));
        out
    }

    /// Calls `emit` with each code of string `bytes` in turn ([`Self::codes`]): its value and the
    /// characters it stands for.
    fn each_code(&self, bytes: &[u8], mut emit: impl FnMut(u32, &mut dyn Iterator<Item = char>)) {
        let value = |b: &[u8]| b.iter().fold(0u32, |c, &b| c << 8 | u32::from(b));
        let (codes, unicode, high_bit, space) = match self {
            Self::Unicode(form) => {
                let unit = match form {
                    Unicode::Utf16 => 2,
                    Unicode::Utf8 => 1,
                    Unicode::Utf32 => 4,
                };
                let (whole, rest) = bytes.split_at_checked(bytes.len() - bytes.len() % unit).unwrap_or((bytes, &[]));
                unicode_codes(whole, *form, |code, c| emit(value(code), &mut c.map(|c| if c == '\r' { '\n' } else { c }).into_iter()));
                if let Some(&last) = rest.last() {
                    emit(value(rest), &mut control(last).into_iter());
                }
                return;
            }
            Self::Cids { codes, unicode, high_bit, space } => (codes, unicode, *high_bit, space),
        };
        // A code's value is that of the string's bytes, also where they're read in their EUC form.
        let original = bytes;
        let value_at = |at: usize, len: usize| original.get(at..at + len).map_or(0, value);
        let euc: Vec<u8>;
        let bytes = if high_bit {
            euc = bytes.iter().map(|&b| if (0x21..0x7f).contains(&b) { b | 0x80 } else { b }).collect();
            euc.as_slice()
        } else {
            bytes
        };
        let mut at = 0;
        while let Some(&first) = bytes.get(at) {
            // VectorCraft reads a 0x0D byte as a paragraph break and a 0x09 byte as a tab in every
            // CMap, as in other fonts' strings, including the row-and-cell CMaps, whose code space
            // has no one-byte codes.
            if let Some(control) = control(first) {
                emit(value_at(at, 1), &mut Some(control).into_iter());
                at += 1;
                continue;
            }
            let rest = bytes.get(at..).unwrap_or_default();
            let code = |len: u8| rest.get(..usize::from(len)).map(value);
            let found = match space {
                // The code is as long as the code space range it falls in; a byte that starts no
                // code in the code space is skipped.
                Some(space) => code_len(space, rest).and_then(|len| code(len).map(|c| (len, codes.lookup_cid_code(c, len)))),
                None => (1..=4u8).find_map(|len| Some((len, Some(codes.lookup_cid_code(code(len)?, len)?)))),
            };
            let Some((len, cid)) = found else {
                emit(value_at(at, 1), &mut None.into_iter());
                at += 1;
                continue;
            };
            let value = value_at(at, usize::from(len));
            at += usize::from(len);
            // A code the CMap leaves out stands for no character.
            match cid.and_then(|cid| unicode.lookup_bf_string(cid)) {
                // A one-byte Roman code whose CID the table gives as a combining mark alone (0x7E in
                // the Mac Shift-JIS CMaps, U+0303): the ASCII character, as Adobe's cid2code.txt lists first.
                Some(BfString::Char('\u{300}'..='\u{36f}')) if len == 1 && (0x20..0x7f).contains(&first) => {
                    emit(value, &mut Some(char::from(first)).into_iter())
                }
                Some(BfString::Char(c)) if c != char::REPLACEMENT_CHARACTER => emit(value, &mut Some(c).into_iter()),
                Some(BfString::String(s)) => emit(value, &mut s.chars().filter(|c| *c != char::REPLACEMENT_CHARACTER)),
                _ => emit(value, &mut None.into_iter()),
            }
        }
    }
}

/// A string's characters, and its codes: each code's value and how many of the characters it
/// stands for, in order.
#[derive(Debug, Default)]
pub(super) struct Decoded {
    pub text: String,
    pub codes: Vec<(u32, usize)>,
}

impl Decoded {
    /// Code `code`, standing for characters `chars`.
    pub(super) fn push(&mut self, code: u32, chars: impl IntoIterator<Item = char>) {
        let before = self.text.len();
        self.text.extend(chars);
        let n = self.text.get(before..).map_or(0, |added| added.chars().count());
        self.codes.push((code, n));
    }
}

/// How strings in a font over predefined CMap `cmap` decode; `None` when the CMap can't be read.
pub(super) fn codes(cmap: &str) -> Option<CMapCodes> {
    match unicode_cmap(cmap) {
        Some(form) => Some(CMapCodes::Unicode(form)),
        None => cid_codes(cmap),
    }
}

/// How a predefined Unicode CMap's codes encode their characters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Unicode {
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

/// Calls `out` with each code of string `s` in Unicode form `form` in turn, and the character it
/// encodes (none for an invalid code: a lone surrogate, a byte that starts no UTF-8 character, a
/// value past U+10FFFF).
fn unicode_codes(s: &[u8], form: Unicode, mut out: impl FnMut(&[u8], Option<char>)) {
    match form {
        Unicode::Utf16 => {
            let unit = |i: usize| s.get(i..i + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_be_bytes);
            let mut at = 0;
            while let Some(u) = unit(at) {
                let next = unit(at + 2);
                let pair = next.filter(|n| (0xd800..0xdc00).contains(&u) && (0xdc00..0xe000).contains(n));
                let len = if pair.is_some() { 4 } else { 2 };
                let c = char::decode_utf16([u].into_iter().chain(pair)).next().and_then(Result::ok);
                out(s.get(at..at + len).unwrap_or_default(), c);
                at += len;
            }
        }
        Unicode::Utf8 => {
            let mut at = 0;
            for chunk in s.utf8_chunks() {
                for c in chunk.valid().chars() {
                    out(s.get(at..at + c.len_utf8()).unwrap_or_default(), Some(c));
                    at += c.len_utf8();
                }
                for _ in chunk.invalid() {
                    out(s.get(at..at + 1).unwrap_or_default(), None);
                    at += 1;
                }
            }
        }
        Unicode::Utf32 => {
            for b in s.as_chunks::<4>().0 {
                out(b.as_slice(), char::from_u32(u32::from_be_bytes(*b)));
            }
        }
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

/// Whether `cmap` names a predefined CMap: a predefined Unicode CMap ([`unicode_cmap`]), one of the
/// predefined CMaps VectorCraft carries ([`predefined`]) or one of the others ([`stand_in`]), in
/// its horizontal (`-H`) or vertical (`-V`) form. The Identity CMaps are not among them.
pub(super) fn known(cmap: &str) -> bool {
    let vertical_of_predefined = || cmap.strip_suffix('V').is_some_and(|stem| predefined(&format!("{stem}H")).is_some());
    let other = || cmap.strip_suffix("-H").or_else(|| cmap.strip_suffix("-V")).is_some_and(|stem| stand_in(stem).is_some());
    unicode_cmap(cmap).is_some() || predefined(cmap).is_some() || vertical_of_predefined() || other()
}

/// Whether predefined CMap `cmap` is a vertical CMap (`-V`, or the row-and-cell `V`).
pub(super) fn vertical(cmap: &str) -> bool {
    cmap == "V" || cmap.ends_with("-V")
}

/// A composite font's name `base-CMap`: the base font's name and its CMap's name, when that is a
/// predefined CMap ([`known`]). A name ending in an Identity CMap is not read as one, nor is one
/// that ends in an unknown CMap name with an encoding part (`UniFoo-UTF8-H`), which would otherwise
/// end in `H`.
pub(super) fn composite_font(name: &str) -> Option<(&str, &str)> {
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
        if known(cmap) {
            return Some((base, cmap));
        }
    }
    None
}
