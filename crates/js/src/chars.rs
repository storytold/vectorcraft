//! Character classes of ECMA-262 5.1 section 7: white space, line terminators and the characters
//! identifiers are made of.

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

/// A character an identifier may start with: a letter (categories Lu, Ll, Lt, Lm, Lo and Nl),
/// `$` or `_` (section 7.6). `\u` escapes are decoded before this check.
pub fn is_id_start(c: u32) -> bool {
    match c {
        0x24 | 0x5F => true,
        0x41..=0x5A | 0x61..=0x7A => true,
        0..=0x7F => false,
        // U+2E2F VERTICAL TILDE is a letter (Lm) that Unicode leaves out of identifiers.
        0x2E2F => false,
        _ => category(c).is_some_and(|g| {
            matches!(g, G::UppercaseLetter | G::LowercaseLetter | G::TitlecaseLetter | G::ModifierLetter | G::OtherLetter | G::LetterNumber)
        }),
    }
}

/// A character an identifier may hold after its first: those it may start with, decimal digits
/// (Nd), combining marks (Mn, Mc), connector punctuation (Pc), and the zero width non-joiner and
/// joiner.
pub fn is_id_part(c: u32) -> bool {
    match c {
        0x30..=0x39 => true,
        0x200C | 0x200D => true,
        0x2E2F => false,
        _ if is_id_start(c) => true,
        0..=0x7F => false,
        _ => category(c).is_some_and(|g| matches!(g, G::NonspacingMark | G::SpacingMark | G::DecimalNumber | G::ConnectorPunctuation)),
    }
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
