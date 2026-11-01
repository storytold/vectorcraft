//! The 8-bit encoding of type in legacy `.ai` files: Windows-1252, its glyphs named as the
//! WinAnsiEncoding of the PDF specification names them (ISO 32000-1, Annex D.2). The writer
//! installs it with `TE` (version 7 specification §3.3.1) and encodes strings in it; the reader
//! decodes strings by the `TE` a file has (its glyph names, which may be another platform's).

/// The glyph names of codes 128–255 (`.notdef` where Windows-1252 has no character).
const HIGH: [&str; 128] = [
    "Euro",
    ".notdef",
    "quotesinglbase",
    "florin",
    "quotedblbase",
    "ellipsis",
    "dagger",
    "daggerdbl",
    "circumflex",
    "perthousand",
    "Scaron",
    "guilsinglleft",
    "OE",
    ".notdef",
    "Zcaron",
    ".notdef",
    ".notdef",
    "quoteleft",
    "quoteright",
    "quotedblleft",
    "quotedblright",
    "bullet",
    "endash",
    "emdash",
    "tilde",
    "trademark",
    "scaron",
    "guilsinglright",
    "oe",
    ".notdef",
    "zcaron",
    "Ydieresis",
    "space",
    "exclamdown",
    "cent",
    "sterling",
    "currency",
    "yen",
    "brokenbar",
    "section",
    "dieresis",
    "copyright",
    "ordfeminine",
    "guillemotleft",
    "logicalnot",
    "hyphen",
    "registered",
    "macron",
    "degree",
    "plusminus",
    "twosuperior",
    "threesuperior",
    "acute",
    "mu",
    "paragraph",
    "periodcentered",
    "cedilla",
    "onesuperior",
    "ordmasculine",
    "guillemotright",
    "onequarter",
    "onehalf",
    "threequarters",
    "questiondown",
    "Agrave",
    "Aacute",
    "Acircumflex",
    "Atilde",
    "Adieresis",
    "Aring",
    "AE",
    "Ccedilla",
    "Egrave",
    "Eacute",
    "Ecircumflex",
    "Edieresis",
    "Igrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Eth",
    "Ntilde",
    "Ograve",
    "Oacute",
    "Ocircumflex",
    "Otilde",
    "Odieresis",
    "multiply",
    "Oslash",
    "Ugrave",
    "Uacute",
    "Ucircumflex",
    "Udieresis",
    "Yacute",
    "Thorn",
    "germandbls",
    "agrave",
    "aacute",
    "acircumflex",
    "atilde",
    "adieresis",
    "aring",
    "ae",
    "ccedilla",
    "egrave",
    "eacute",
    "ecircumflex",
    "edieresis",
    "igrave",
    "iacute",
    "icircumflex",
    "idieresis",
    "eth",
    "ntilde",
    "ograve",
    "oacute",
    "ocircumflex",
    "otilde",
    "odieresis",
    "divide",
    "oslash",
    "ugrave",
    "uacute",
    "ucircumflex",
    "udieresis",
    "yacute",
    "thorn",
    "ydieresis",
];

/// Codes 128–159 of Windows-1252 (the others are Latin-1's); its five gaps are C1 controls.
const C1: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜',
    '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Glyph names other platforms' encodings use that Windows-1252 has no code for (Mac Roman's).
const OTHER_NAMES: &[(&str, char)] = &[
    ("notequal", '≠'),
    ("infinity", '∞'),
    ("lessequal", '≤'),
    ("greaterequal", '≥'),
    ("partialdiff", '∂'),
    ("summation", '∑'),
    ("product", '∏'),
    ("pi", 'π'),
    ("integral", '∫'),
    ("Omega", 'Ω'),
    ("radical", '√'),
    ("approxequal", '≈'),
    ("Delta", '∆'),
    ("lozenge", '◊'),
    ("fraction", '⁄'),
    ("fi", 'ﬁ'),
    ("fl", 'ﬂ'),
    ("dotlessi", 'ı'),
    ("breve", '˘'),
    ("dotaccent", '˙'),
    ("ring", '˚'),
    ("hungarumlaut", '˝'),
    ("ogonek", '˛'),
    ("caron", 'ˇ'),
    ("nbspace", '\u{A0}'),
    ("quotesingle", '\''),
    ("grave", '`'),
];

/// Character `b` of Windows-1252.
pub(crate) fn decode(b: u8) -> char {
    match b {
        0x80..=0x9F => C1.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}

/// The code of `c` in Windows-1252: the printable characters, tab and the line breaks; None for
/// the others (and the C1 controls).
pub(crate) fn encode(c: char) -> Option<u8> {
    match c {
        '\t' | '\n' | '\r' | ' '..='~' | '\u{A0}'..='\u{FF}' => u8::try_from(u32::from(c)).ok(),
        _ => C1.iter().position(|&x| x == c && !x.is_control()).and_then(|i| u8::try_from(0x80 + i).ok()),
    }
}

/// The `TE` that installs the encoding: its codes 39 and 96 as in ASCII (PostScript's standard
/// encoding has quotes there) and 128–255 by name.
pub(crate) fn te() -> String {
    let mut s = String::from("[39/quotesingle 96/grave 128");
    for (i, name) in HIGH.iter().enumerate() {
        s.push('/');
        s.push_str(name);
        if i % 8 == 7 && i + 1 < HIGH.len() {
            s.push('\n');
        }
    }
    s.push_str("\nTE\n");
    s
}

/// The character glyph `name` stands for at code `code` of an encoding: that code's Windows-1252
/// character when the name is the one this encoding gives it; else by the name.
pub(crate) fn glyph(code: u8, name: &str) -> Option<char> {
    if code >= 0x80 && HIGH.get(usize::from(code - 0x80)) == Some(&name) && name != ".notdef" {
        return Some(decode(code));
    }
    if let Some(i) = HIGH.iter().position(|n| *n == name && *n != ".notdef" && *n != "space") {
        return u8::try_from(0x80 + i).ok().map(decode);
    }
    if let Some((_, c)) = OTHER_NAMES.iter().find(|(n, _)| *n == name) {
        return Some(*c);
    }
    match name {
        "space" => Some(' '),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_encodable_character_decodes_as_itself() {
        for b in 0..=255u8 {
            let c = decode(b);
            if let Some(e) = encode(c) {
                assert_eq!(e, b, "{c:?}");
            }
        }
        assert_eq!(encode('é'), Some(0xE9));
        assert_eq!(encode('€'), Some(0x80));
        assert_eq!(encode('—'), Some(0x97));
        assert_eq!(encode('Ω'), None);
        assert_eq!(encode('\u{81}'), None);
    }

    #[test]
    fn glyph_names_decode_by_code_and_by_name() {
        assert_eq!(glyph(0xE9, "eacute"), Some('é'));
        assert_eq!(glyph(0xA0, "space"), Some('\u{A0}'));
        // Mac Roman puts é at 0x8E.
        assert_eq!(glyph(0x8E, "eacute"), Some('é'));
        assert_eq!(glyph(0xAD, "notequal"), Some('≠'));
        assert_eq!(glyph(0x41, "nosuchglyph"), None);
        assert!(te().starts_with("[39/quotesingle 96/grave 128/Euro/.notdef/"));
        assert!(te().ends_with("/ydieresis\nTE\n"));
    }
}
