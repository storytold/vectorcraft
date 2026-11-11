//! Character classes of ECMA-262 5.1 section 7: white space, line terminators and the characters
//! identifiers are made of. Identifiers follow the Unicode `ID_Start` and `ID_Continue` rule of
//! later editions.

use std::cmp::Ordering;

use unicode_general_category::{GeneralCategory as G, get_general_category};

/// A line terminator: LF, CR, U+2028 or U+2029 (section 7.3).
pub fn is_line_terminator(c: u32) -> bool {
    matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029)
}

/// White space (section 7.2): tab, vertical tab, form feed, space, no-break space, the byte order
/// mark and the other space separators (Unicode category Zs).
pub fn is_whitespace(c: u32) -> bool {
    matches!(c, 0x09 | 0x0B | 0x0C | 0x20 | 0xA0 | 0xFEFF | 0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000)
}

/// A character an identifier may start with: a Unicode `ID_Start` character, `$` or `_`
/// (the Names and Keywords clause of current ECMA-262 editions). `ID_Start` is the letters (categories Lu, Ll, Lt, Lm, Lo and Nl)
/// and `Other_ID_Start`, less `Pattern_Syntax` and `Pattern_White_Space` (Unicode Standard Annex
/// #31). The categories come from Unicode 16.0. `\u` escapes are decoded before this check.
pub fn is_id_start(c: u32) -> bool {
    match c {
        0x24 | 0x5F => true,
        0x41..=0x5A | 0x61..=0x7A => true,
        0..=0x7F => false,
        _ if is_other_id_start(c) => true,
        _ => {
            category(c).is_some_and(|g| {
                matches!(g, G::UppercaseLetter | G::LowercaseLetter | G::TitlecaseLetter | G::ModifierLetter | G::OtherLetter | G::LetterNumber)
            }) && !is_pattern_syntax(c)
        }
    }
}

/// A character an identifier may hold after its first: a Unicode `ID_Continue` character or `$`.
/// `ID_Continue` adds to `ID_Start` the decimal digits (Nd), combining marks (Mn, Mc), connector
/// punctuation (Pc, which holds `_`) and `Other_ID_Continue` (which holds the zero width
/// non-joiner and joiner), less `Pattern_Syntax` and `Pattern_White_Space`.
pub fn is_id_part(c: u32) -> bool {
    match c {
        0x30..=0x39 => true,
        _ if is_id_start(c) => true,
        0..=0x7F => false,
        _ if is_other_id_continue(c) => true,
        _ => {
            category(c).is_some_and(|g| matches!(g, G::NonspacingMark | G::SpacingMark | G::DecimalNumber | G::ConnectorPunctuation))
                && !is_pattern_syntax(c)
        }
    }
}

/// `Other_ID_Start` (Unicode PropList): characters kept in `ID_Start` though their category is
/// not a letter.
fn is_other_id_start(c: u32) -> bool {
    matches!(c, 0x1885 | 0x1886 | 0x2118 | 0x212E | 0x309B | 0x309C)
}

/// `Other_ID_Continue` (Unicode PropList 16.0): characters kept in `ID_Continue` though their
/// category is not a letter, mark, digit or connector.
fn is_other_id_continue(c: u32) -> bool {
    matches!(c, 0xB7 | 0x387 | 0x1369..=0x1371 | 0x19DA | 0x200C | 0x200D | 0x30FB | 0xFF65)
}

/// Non-ASCII `Pattern_Syntax` ranges (Unicode PropList). Unicode does not change this set, and no
/// identifier character belongs to it. `Pattern_White_Space` outside ASCII (U+0085, U+200E, U+200F,
/// U+2028 and U+2029) holds no letter, mark, digit or connector, so the category checks leave it out.
const PATTERN_SYNTAX: [(u32, u32); 23] = [
    (0x00A1, 0x00A7),
    (0x00A9, 0x00A9),
    (0x00AB, 0x00AC),
    (0x00AE, 0x00AE),
    (0x00B0, 0x00B1),
    (0x00B6, 0x00B6),
    (0x00BB, 0x00BB),
    (0x00BF, 0x00BF),
    (0x00D7, 0x00D7),
    (0x00F7, 0x00F7),
    (0x2010, 0x2027),
    (0x2030, 0x203E),
    (0x2041, 0x2053),
    (0x2055, 0x205E),
    (0x2190, 0x245F),
    (0x2500, 0x2775),
    (0x2794, 0x2BFF),
    (0x2E00, 0x2E7F),
    (0x3001, 0x3003),
    (0x3008, 0x3020),
    (0x3030, 0x3030),
    (0xFD3E, 0xFD3F),
    (0xFE45, 0xFE46),
];

/// Whether a non-ASCII code point is `Pattern_Syntax`.
fn is_pattern_syntax(c: u32) -> bool {
    PATTERN_SYNTAX
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                Ordering::Less
            } else if lo > c {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// The Unicode general category of a code point; `None` for a lone surrogate.
fn category(c: u32) -> Option<G> {
    char::from_u32(c).map(get_general_category)
}

/// The value of a hexadecimal digit.
pub fn hex_value(c: u32) -> Option<u32> {
    match c {
        0x30..=0x39 => Some(c - 0x30),
        0x41..=0x46 => Some(c - 0x41 + 10),
        0x61..=0x66 => Some(c - 0x61 + 10),
        _ => None,
    }
}

/// The value of the digit values `digits` in `radix` (8 or 16), rounded once to the nearest number
/// with ties to even (sections 7.8.3 and 9.3.1), or `None` when there are no digits. Digits are
/// kept exactly in a `u128` while they fit; past that, each later digit counts only for its
/// position and for whether any of the dropped bits is set.
pub fn radix_value(digits: impl IntoIterator<Item = u32>, radix: u32) -> Option<f64> {
    let bits = radix.trailing_zeros();
    let mut kept: u128 = 0;
    let mut dropped_bits: u32 = 0;
    let mut sticky = false;
    let mut any = false;
    for d in digits {
        any = true;
        if kept.leading_zeros() >= bits {
            kept = kept << bits | u128::from(d);
        } else {
            dropped_bits = dropped_bits.saturating_add(bits);
            sticky |= d != 0;
        }
    }
    if !any {
        return None;
    }
    // `kept` holds at least 125 bits once a digit is dropped, so its lowest bit lies below the
    // rounding bit and stands in for every dropped bit.
    let mut value = (kept | u128::from(sticky)) as f64;
    while dropped_bits > 0 && value.is_finite() {
        let step = dropped_bits.min(64);
        value *= (1_u128 << step) as f64;
        dropped_bits -= step;
    }
    Some(value)
}

pub fn is_decimal_digit(c: u32) -> bool {
    (0x30..=0x39).contains(&c)
}

pub fn is_octal_digit(c: u32) -> bool {
    (0x30..=0x37).contains(&c)
}
