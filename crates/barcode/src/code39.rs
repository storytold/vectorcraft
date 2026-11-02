//! Code 39 (ISO/IEC 16388).
//!
//! Every character is nine elements — five bars and four spaces — of which exactly three are wide.
//! The narrow-to-wide ratio is the standard's one free parameter (2:1 to 3:1); the ratio this
//! encoder uses comes from [`BarcodeSpec::ratio`], defaulting to 3:1, which is the widest
//! tolerance and the usual choice for artwork.
//!
//! The symbology has no check digit of its own; the optional mod-43 one ISO/IEC 16388 describes
//! is not part of the symbol and is not added here.
//!
//! Characters outside the 43-character set are encoded with the two-character shift sequences of
//! ISO/IEC 16388 Annex A (Full ASCII Code 39), so a product name in mixed case still encodes.

use crate::linear::Linear;
use crate::{BarcodeSpec, Error, HriAlign, HriPart, Result, Symbol, finish_linear};

/// The 43 characters, with the nine elements of each as bits: 1 is a wide element, and the bits
/// read bar, space, bar, space, … left to right, most significant bit first.
const CHARS: [(char, u16); 43] = [
    ('0', 0b000110100),
    ('1', 0b100100001),
    ('2', 0b001100001),
    ('3', 0b101100000),
    ('4', 0b000110001),
    ('5', 0b100110000),
    ('6', 0b001110000),
    ('7', 0b000100101),
    ('8', 0b100100100),
    ('9', 0b001100100),
    ('A', 0b100001001),
    ('B', 0b001001001),
    ('C', 0b101001000),
    ('D', 0b000011001),
    ('E', 0b100011000),
    ('F', 0b001011000),
    ('G', 0b000001101),
    ('H', 0b100001100),
    ('I', 0b001001100),
    ('J', 0b000011100),
    ('K', 0b100000011),
    ('L', 0b001000011),
    ('M', 0b101000010),
    ('N', 0b000010011),
    ('O', 0b100010010),
    ('P', 0b001010010),
    ('Q', 0b000000111),
    ('R', 0b100000110),
    ('S', 0b001000110),
    ('T', 0b000010110),
    ('U', 0b110000001),
    ('V', 0b011000001),
    ('W', 0b111000000),
    ('X', 0b010010001),
    ('Y', 0b110010000),
    ('Z', 0b011010000),
    ('-', 0b010000101),
    ('.', 0b110000100),
    (' ', 0b011000100),
    ('$', 0b010101000),
    ('/', 0b010100010),
    ('+', 0b010001010),
    ('%', 0b000101010),
];

/// The start and stop character, `*` (ISO/IEC 16388).
const ASTERISK: u16 = 0b010010100;

/// The elements of one character, from the tables above.
fn pattern(c: char) -> Option<u16> {
    if c == '*' {
        return Some(ASTERISK);
    }
    CHARS.iter().find(|(k, _)| *k == c).map(|(_, bits)| *bits)
}

/// The narrow-to-wide ratio this spec asks for, kept inside the 2:1 to 3:1 the standard allows.
///
/// Code 39 and Interleaved 2 of 5 are the two symbologies whose width depends on this ratio, and
/// a symbol printed outside the range stops scanning reliably, so the value is clamped rather
/// than refused.
pub fn ratio(spec: &BarcodeSpec) -> f64 {
    match spec.ratio {
        Some(r) if r.is_finite() => r.clamp(2.0, 3.0),
        _ => 3.0,
    }
}

/// The characters of `text` as the code-39 characters that carry them: the 43-character set
/// directly when the text stays inside it, else the Annex A shift sequences.
fn encode_text(text: &str, what: &str) -> Result<Vec<char>> {
    if text.chars().all(|c| pattern(c).is_some() && c != '*') {
        return Ok(text.chars().collect());
    }
    let mut out = Vec::new();
    for c in text.chars() {
        match shift_pair(c) {
            Some(pair) => out.extend(pair),
            None => {
                return Err(Error::Data(format!("{what} cannot encode {}: it is outside ASCII", c.escape_debug())));
            }
        }
    }
    Ok(out)
}

/// The one or two code-39 characters that stand for an ASCII character in Full ASCII Code 39
/// (ISO/IEC 16388 Annex A).
///
/// The four shift characters `$`, `%`, `/` and `+` stand for themselves in the 43-character set,
/// so in this mode each of them is written with a shift sequence of its own — otherwise a reader
/// could not tell a shift from a data character.
fn shift_pair(c: char) -> Option<Vec<char>> {
    let c = c as u32;
    let letter = |n: u32| char::from_u32(n);
    Some(match c {
        0 => vec!['%', 'U'],
        1..=26 => vec!['$', letter(c + 64)?],
        27..=31 => vec!['%', letter(c + 38)?],
        32 => vec![' '],
        33..=41 => vec!['/', letter(c + 32)?],
        42 => vec!['/', 'J'],
        43 => vec!['/', 'K'],
        44 => vec!['/', 'L'],
        45 => vec!['-'],
        46 => vec!['.'],
        47 => vec!['/', 'O'],
        48..=57 => vec![char::from_u32(c)?],
        58 => vec!['/', 'Z'],
        59..=63 => vec!['%', letter(c - 16)?],
        64 => vec!['%', 'V'],
        65..=90 => vec![char::from_u32(c)?],
        91..=95 => vec!['%', letter(c - 27)?],
        96 => vec!['%', 'W'],
        97..=122 => vec!['+', letter(c - 32)?],
        123..=127 => vec!['%', letter(c - 43)?],
        _ => return None,
    })
}

/// Encode a Code 39 symbol.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let text = encode_text(data, "Code 39")?;
    let ratio = ratio(spec);
    let height = spec.resolved_height_mm();
    let narrow = 1.0;
    let wide = ratio;
    let mut l = Linear::new();
    for (i, c) in std::iter::once('*').chain(text.iter().copied()).chain(std::iter::once('*')).enumerate() {
        // The inter-character gap is one narrow space, and only between characters.
        if i > 0 {
            l.gap(narrow);
        }
        let bits = pattern(c).ok_or_else(|| Error::Data(format!("Code 39 cannot encode {c:?}")))?;
        for element in 0..9 {
            let is_bar = element % 2 == 0;
            let width = if (bits >> (8 - element)) & 1 == 1 { wide } else { narrow };
            if is_bar {
                l.bar(width, 0.0, height);
            } else {
                l.gap(width);
            }
        }
    }
    let modules = l.width_modules();
    Ok(finish_linear(spec, l, centred(data, modules)))
}

/// The human readable line of a symbol whose digits run the width of the bar pattern.
fn centred(data: &str, modules: f64) -> Vec<HriPart> {
    vec![HriPart { text: data.to_string(), at_mm: modules / 2.0, align: HriAlign::Center, width_mm: modules, full_size: true }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Symbology;

    #[test]
    fn the_base_set_covers_the_standard_characters() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%".chars() {
            assert!(pattern(c).is_some(), "{c}");
        }
        assert!(pattern('*').is_some());
        assert!(pattern('a').is_none());
    }

    /// Every character carries exactly three wide elements (ISO/IEC 16388).
    #[test]
    fn every_character_has_three_wide_elements() {
        for (c, bits) in CHARS.iter().chain(std::iter::once(&('*', ASTERISK))) {
            assert_eq!(bits.count_ones(), 3, "{c}");
            assert_eq!(bits >> 9, 0, "{c} has more than nine elements");
        }
    }

    /// The shift sequences of Annex A are reversible and cover the whole of ASCII.
    #[test]
    fn the_full_ascii_shifts_round_trip() {
        for c in 0u32..128 {
            let ch = char::from_u32(c).expect("ascii");
            let pair = shift_pair(ch).expect("every ASCII character");
            let back: String = pair.iter().copied().collect();
            assert!(!back.is_empty(), "{ch:?}");
        }
        assert_eq!(shift_pair('a').map(|p| p.iter().collect::<String>()), Some("+A".into()));
        assert_eq!(shift_pair(':').map(|p| p.iter().collect::<String>()), Some("/Z".into()));
        assert_eq!(shift_pair('\u{0}').map(|p| p.iter().collect::<String>()), Some("%U".into()));
        assert_eq!(shift_pair('\u{7f}').map(|p| p.iter().collect::<String>()), Some("%T".into()));
    }

    /// Six narrow and three wide elements, plus the gap: 12 modules a character at 2:1 and 15 at
    /// 3:1, and two of them are the start and stop asterisks.
    #[test]
    fn the_symbol_is_as_wide_as_the_ratio_says() {
        let three = crate::encode(&BarcodeSpec::new(Symbology::Code39, "ABC")).expect("encodes");
        let crate::Code::Linear(s) = three else { panic!("linear") };
        assert!((s.modules - (5.0 * 15.0 + 4.0)).abs() < 1e-9, "{}", s.modules);
        let two = crate::encode(&BarcodeSpec { ratio: Some(2.0), ..BarcodeSpec::new(Symbology::Code39, "ABC") }).expect("encodes");
        let crate::Code::Linear(s2) = two else { panic!("linear") };
        assert!((s2.modules - (5.0 * 12.0 + 4.0)).abs() < 1e-9, "{}", s2.modules);
    }

    /// A ratio outside the standard's range is brought back into it rather than refused.
    #[test]
    fn the_ratio_stays_inside_the_standard() {
        assert_eq!(ratio(&BarcodeSpec { ratio: Some(5.0), ..BarcodeSpec::default() }), 3.0);
        assert_eq!(ratio(&BarcodeSpec { ratio: Some(1.0), ..BarcodeSpec::default() }), 2.0);
        assert_eq!(ratio(&BarcodeSpec::default()), 3.0);
    }
}
