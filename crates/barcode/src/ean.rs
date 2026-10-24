//! EAN-13, EAN-8, UPC-A and UPC-E (ISO/IEC 15420 and GS1 *General Specifications* §5.2).
//!
//! The four share one module alphabet of three number sets — A (odd parity), B (even parity) and
//! C, which is A inverted — and the same 101 / 01010 / 101 guard patterns. What tells them apart
//! is how much of the number they carry and how the digits are arranged around it:
//!
//! - EAN-13 carries twelve digits plus a check digit; the first digit is not encoded as a
//!   character at all but as the *parity pattern* of the six that follow.
//! - EAN-8 is the same with four digits a side and no parity trick.
//! - UPC-A is EAN-13 whose first digit is zero, and prints its first and last digits outside the
//!   symbol at a reduced size.
//! - UPC-E suppresses the zeros of a UPC-A whose company prefix starts with zero, down to six
//!   characters, and encodes the number system and the check digit as parity.

use crate::check;
use crate::linear::Linear;
use crate::{BarcodeSpec, Error, HriAlign, HriPart, Result, Symbol, Symbology, finish_linear};

/// Number set A: odd parity, used by the left-hand characters and by a UPC-E character that the
/// check digit's parity pattern calls odd. One bit per module, most significant bit leftmost.
const SET_A: [u8; 10] = [0b0001101, 0b0011001, 0b0010011, 0b0111101, 0b0100011, 0b0110001, 0b0101111, 0b0111011, 0b0110111, 0b0001011];

/// Number set B: even parity, the counterpart of set A used by the left-hand characters.
const SET_B: [u8; 10] = [0b0100111, 0b0110011, 0b0011011, 0b0100001, 0b0011101, 0b0111001, 0b0000101, 0b0010001, 0b0001001, 0b0010111];

/// Number set C: set A inverted, used by every right-hand character.
fn set_c(value: u8) -> u8 {
    !SET_A[usize::from(value) & 0b1111] & 0b1111111
}

/// Which of the three sets a character is drawn in. The set decides which way the ±1/13-module
/// adjustment of the characters 1, 2, 7 and 8 goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Set {
    A,
    B,
    C,
}

/// The six-character parity pattern of an EAN-13's first digit, one bit per character, set where
/// the character uses set B (GS1 §5.2.2.3.1).
const EAN13_PARITY: [u8; 10] = [0b000000, 0b001011, 0b001101, 0b001110, 0b010011, 0b011001, 0b011100, 0b010101, 0b010110, 0b011010];

/// The six-character parity pattern of a UPC-E, one bit per character, set where the character
/// uses set B. The row is the check digit; a number system of 1 inverts the whole pattern
/// (GS1 §5.2.2.3.4, ISO/IEC 15420).
const UPCE_PARITY: [u8; 10] = [0b111000, 0b110100, 0b110010, 0b110001, 0b101100, 0b100110, 0b100011, 0b101010, 0b101001, 0b100101];

const START_GUARD: [bool; 3] = [true, false, true];
const CENTRE_GUARD: [bool; 5] = [false, true, false, true, false];
/// UPC-E ends on 010101 rather than 101: it has no separate right guard, and its last character
/// already ends in the space the guard would begin with.
const UPCE_END_GUARD: [bool; 6] = [false, true, false, true, false, true];

/// Encode one of the four EAN/UPC symbologies.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    match spec.symbology {
        Symbology::Ean13 => ean13(spec, data),
        Symbology::Ean8 => ean8(spec, data),
        Symbology::UpcA => upc_a(spec, data),
        Symbology::UpcE => upc_e(spec, data),
        other => Err(Error::Data(format!("{} is not an EAN/UPC symbology", other.label()))),
    }
}

/// The geometry every EAN/UPC symbol is built with: the bar height, and the height the guard bars
/// and the UPC-A end characters reach, five modules further down (GS1 §5.2.2.4.2).
struct Frame {
    height: f64,
    guard_height: f64,
    /// A thirteenth of a module: the adjustment a bar of a character 1, 2, 7 or 8 takes.
    unit: f64,
}

impl Frame {
    fn new(spec: &BarcodeSpec) -> Self {
        let height = spec.resolved_height_mm();
        Self { height, guard_height: height + 5.0 * spec.resolved_module_mm(), unit: 1.0 / 13.0 }
    }
}

/// The ±1/13-module adjustment of one bar of a character, in modules.
///
/// GS1 §5.2.2.4.1 widens or narrows each bar and each space of the characters 1, 2, 7 and 8, in
/// opposite directions, so that a character keeps its seven modules — it has two bars and two
/// spaces. In set A the characters 1 and 2 narrow and 7 and 8 widen; in sets B and C the other
/// way round.
fn bar_adjustment(set: Set, value: u8) -> f64 {
    let small = matches!(value, 1 | 2);
    let large = matches!(value, 7 | 8);
    if !small && !large {
        return 0.0;
    }
    let wider = (set == Set::A) == large;
    if wider { 1.0 } else { -1.0 }
}

/// Write one seven-module character, adjusted and, where the standard asks for it, with its bars
/// running down to the guard bars' height.
fn character(l: &mut Linear, bits: u8, value: u8, set: Set, frame: &Frame, extended: bool) {
    let adjust = bar_adjustment(set, value) * frame.unit;
    let height = if extended { frame.guard_height } else { frame.height };
    let dark = |i: u32| (bits >> (6 - i)) & 1 == 1;
    let mut i = 0u32;
    while i < 7 {
        let is_bar = dark(i);
        let mut run = 1u32;
        while i + run < 7 && dark(i + run) == is_bar {
            run += 1;
        }
        let width = (f64::from(run) + if is_bar { adjust } else { -adjust }).max(0.05);
        if is_bar {
            l.bar(width, 0.0, height);
        } else {
            l.gap(width);
        }
        i += run;
    }
}

/// Write a guard pattern, whose bars always reach the guard bars' height.
fn guard(l: &mut Linear, bits: &[bool], frame: &Frame) {
    l.modules(bits, 0.0, frame.guard_height);
}

/// The set a character of the left half is drawn in, from its parity bit.
fn left_set(parity: u8, index: usize) -> Set {
    if (parity >> (5 - index)) & 1 == 1 { Set::B } else { Set::A }
}

/// The module alphabet lookup for a set.
fn bits_of(set: Set, value: u8) -> u8 {
    match set {
        Set::A => SET_A[usize::from(value) & 0b1111],
        Set::B => SET_B[usize::from(value) & 0b1111],
        Set::C => set_c(value),
    }
}

/// One digit of the human readable line, centred under the seven modules of its character.
fn digit_part(cell_start: f64, d: u8) -> HriPart {
    HriPart { text: char::from(b'0' + (d % 10)).to_string(), at_mm: cell_start + 3.5, align: HriAlign::Center, width_mm: 7.0, full_size: true }
}

/// A digit of the human readable line that sits outside the symbol, at the reduced size
/// GS1 §5.2.4 asks for: at most four modules wide, its height reduced in proportion.
fn outside_part(text: String, at: f64, align: HriAlign) -> HriPart {
    HriPart { text, at_mm: at, align, width_mm: 4.0, full_size: false }
}

fn ean13(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let (d, _) = check::digits(data, 13, "EAN-13")?;
    let frame = Frame::new(spec);
    let parity = EAN13_PARITY[usize::from(d.first().copied().unwrap_or(0)) & 0b1111];
    let mut l = Linear::new();
    let mut parts = vec![HriPart {
        text: d.first().map_or_else(String::new, |v| char::from(b'0' + v).to_string()),
        // GS1 §5.2.4: printed to the left of the start guard pattern, in line with the other
        // digits. One module of clear space keeps it off the bars.
        at_mm: -1.0,
        align: HriAlign::Right,
        width_mm: 7.0,
        full_size: true,
    }];
    guard(&mut l, &START_GUARD, &frame);
    for i in 0..6 {
        let value = d.get(i + 1).copied().unwrap_or(0);
        let set = left_set(parity, i);
        parts.push(digit_part(l.cursor(), value));
        character(&mut l, bits_of(set, value), value, set, &frame, false);
    }
    guard(&mut l, &CENTRE_GUARD, &frame);
    for i in 0..6 {
        let value = d.get(i + 7).copied().unwrap_or(0);
        parts.push(digit_part(l.cursor(), value));
        character(&mut l, bits_of(Set::C, value), value, Set::C, &frame, false);
    }
    guard(&mut l, &START_GUARD, &frame);
    Ok(finish_linear(spec, l, parts))
}

fn ean8(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let (d, _) = check::digits(data, 8, "EAN-8")?;
    let frame = Frame::new(spec);
    let mut l = Linear::new();
    let mut parts = Vec::new();
    guard(&mut l, &START_GUARD, &frame);
    for i in 0..4 {
        let value = d.get(i).copied().unwrap_or(0);
        parts.push(digit_part(l.cursor(), value));
        character(&mut l, bits_of(Set::A, value), value, Set::A, &frame, false);
    }
    guard(&mut l, &CENTRE_GUARD, &frame);
    for i in 0..4 {
        let value = d.get(i + 4).copied().unwrap_or(0);
        parts.push(digit_part(l.cursor(), value));
        character(&mut l, bits_of(Set::C, value), value, Set::C, &frame, false);
    }
    guard(&mut l, &START_GUARD, &frame);
    Ok(finish_linear(spec, l, parts))
}

fn upc_a(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let (d, _) = check::digits(data, 12, "UPC-A")?;
    let frame = Frame::new(spec);
    let mut l = Linear::new();
    guard(&mut l, &START_GUARD, &frame);
    let mut cells = Vec::new();
    for i in 0..6 {
        let value = d.get(i).copied().unwrap_or(0);
        cells.push(l.cursor());
        // The first character's bars run down with the guard bars (GS1 §5.2.2.4.2), so the digit
        // printed outside the symbol has something to line up with.
        character(&mut l, bits_of(Set::A, value), value, Set::A, &frame, i == 0);
    }
    guard(&mut l, &CENTRE_GUARD, &frame);
    for i in 0..6 {
        let value = d.get(i + 6).copied().unwrap_or(0);
        cells.push(l.cursor());
        character(&mut l, bits_of(Set::C, value), value, Set::C, &frame, i == 5);
    }
    guard(&mut l, &START_GUARD, &frame);

    // The first and last digits sit outside the symbol, their inner edges five modules clear of
    // the guard bars, and none of the twelve is centred under a character it does not have.
    let text = |i: usize| d.get(i).map_or_else(String::new, |v| char::from(b'0' + v).to_string());
    let mut parts = vec![outside_part(text(0), -5.0, HriAlign::Right)];
    parts.extend(cells.iter().skip(1).take(5).zip(d.iter().skip(1)).map(|(&x, &v)| digit_part(x, v)));
    parts.extend(cells.iter().skip(6).take(5).zip(d.iter().skip(6)).map(|(&x, &v)| digit_part(x, v)));
    parts.push(outside_part(text(11), 100.0, HriAlign::Left));

    Ok(finish_linear(spec, l, parts))
}

fn upc_e(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let (number_system, digits6, check_digit) = upc_e_parts(data)?;
    let frame = Frame::new(spec);
    let parity = UPCE_PARITY[usize::from(check_digit) & 0b1111] ^ if number_system == 1 { 0b111111 } else { 0 };
    let mut l = Linear::new();
    guard(&mut l, &START_GUARD, &frame);
    let mut parts = vec![outside_part(number_system.to_string(), -5.0, HriAlign::Right)];
    for (i, &value) in digits6.iter().enumerate() {
        let set = left_set(parity, i);
        parts.push(digit_part(l.cursor(), value));
        character(&mut l, bits_of(set, value), value, set, &frame, false);
    }
    guard(&mut l, &UPCE_END_GUARD, &frame);
    // UPC-E puts its check digit three modules right of the symbol rather than five (GS1 §5.2.4).
    parts.push(outside_part(check_digit.to_string(), 54.0, HriAlign::Left));
    Ok(finish_linear(spec, l, parts))
}

/// The number system digit, the six encoded digits and the check digit of a UPC-E, from whatever
/// a caller typed: the six digits alone, those with a number system, or all eight.
fn upc_e_parts(data: &str) -> Result<(u8, [u8; 6], u8)> {
    if !data.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::Data("UPC-E takes digits only".into()));
    }
    let all: Vec<u8> = data.bytes().map(|b| b - b'0').collect();
    let (number_system, six) = match all.len() {
        6 => (0u8, all.get(..6).unwrap_or(&[])),
        7 | 8 => (all.first().copied().unwrap_or(0), all.get(1..7).unwrap_or(&[])),
        n => return Err(Error::Data(format!("UPC-E takes 6, 7 or 8 digits, not {n}"))),
    };
    if number_system > 1 {
        return Err(Error::Data("a UPC-E number system is 0 or 1".into()));
    }
    let mut digits6 = [0u8; 6];
    for (slot, &v) in digits6.iter_mut().zip(six) {
        *slot = v;
    }
    let expanded = expand(number_system, &digits6);
    // GS1 §7.11's Test 1: the six digits must suppress back out of the UPC-A they expand to,
    // otherwise they name a number the symbol does not actually carry.
    if compress(&expanded).as_ref() != Some(&digits6) {
        return Err(Error::Data("UPC-E: those six digits do not name a UPC-A that suppresses to them".into()));
    }
    let want = check::mod10(&expanded);
    if all.len() == 8 {
        match all.get(7) {
            Some(&given) if given == want => {}
            Some(&given) => return Err(Error::Data(format!("UPC-E: the check digit should be {want}, not {given}"))),
            None => return Err(Error::Data("UPC-E takes 6, 7 or 8 digits".into())),
        }
    }
    Ok((number_system, digits6, want))
}

/// Expand a UPC-E to the eleven digits of the UPC-A it stands for, by the zero-suppression rules
/// of GS1 §7.11 (ISO/IEC 15420): the last encoded digit says where the run of zeros it stands for
/// belongs.
pub fn expand(number_system: u8, d: &[u8; 6]) -> Vec<u8> {
    let mut out = vec![number_system];
    match d[5] {
        0..=2 => out.extend([d[0], d[1], d[5], 0, 0, 0, 0, d[2], d[3], d[4]]),
        3 => out.extend([d[0], d[1], d[2], 0, 0, 0, 0, 0, d[3], d[4]]),
        4 => out.extend([d[0], d[1], d[2], d[3], 0, 0, 0, 0, 0, d[4]]),
        _ => out.extend([d[0], d[1], d[2], d[3], d[4], 0, 0, 0, 0, d[5]]),
    }
    out
}

/// Suppress a UPC-A's zeros back to six digits, or None when it has none to suppress.
///
/// The tests are the four expansion cases read backwards, longest run of zeros first, and each
/// also checks the value the case has to place in the last position.
pub fn compress(upc_a: &[u8]) -> Option<[u8; 6]> {
    let d: &[u8] = upc_a.get(1..11)?;
    let zero = |from: usize, to: usize| d.get(from..to).is_some_and(|s| s.iter().all(|&v| v == 0));
    let at = |i: usize| d.get(i).copied().unwrap_or(0);
    if zero(3, 8) {
        return Some([at(0), at(1), at(2), at(8), at(9), 3]);
    }
    // A last digit of five or more is read before the four case, which is the order GS1 §7.11's
    // Test 1 asks about them in: without it, a number whose fifth digit is also zero would
    // compress two ways.
    if zero(5, 9) && at(9) >= 5 {
        return Some([at(0), at(1), at(2), at(3), at(4), at(9)]);
    }
    if zero(4, 9) {
        return Some([at(0), at(1), at(2), at(3), at(9), 4]);
    }
    if zero(3, 7) && at(2) <= 2 {
        return Some([at(0), at(1), at(7), at(8), at(9), at(2)]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(s: Symbology, data: &str) -> Symbol {
        match crate::encode(&BarcodeSpec::new(s, data)) {
            Ok(crate::Code::Linear(l)) => l,
            other => panic!("{other:?}"),
        }
    }

    /// The module counts GS1 §5.2.2.4.5 fixes, the quiet zones excluded.
    #[test]
    fn symbols_are_the_length_the_standard_gives() {
        assert!((symbol(Symbology::Ean13, "4006381333931").modules - 95.0).abs() < 1e-9);
        assert!((symbol(Symbology::Ean8, "96385074").modules - 67.0).abs() < 1e-9);
        assert!((symbol(Symbology::UpcA, "036000291452").modules - 95.0).abs() < 1e-9);
        assert!((symbol(Symbology::UpcE, "01234565").modules - 51.0).abs() < 1e-9);
    }

    /// The nominal size of GS1 §5.2.2.4: 0.33 mm modules, an EAN-13 37.29 mm wide and 25.93 mm
    /// high including its human readable line, whose digits are 2.75 mm and start 23.18 mm down.
    #[test]
    fn an_ean13_is_the_nominal_size() {
        let s = symbol(Symbology::Ean13, "4006381333931");
        assert!((s.module_mm - 0.33).abs() < 1e-9);
        assert!((s.width_mm - 37.29).abs() < 0.02, "{}", s.width_mm);
        assert!((s.height_mm - 25.93).abs() < 0.02, "{}", s.height_mm);
        let hri = s.human_readable.as_ref().expect("a human readable line");
        assert!((hri.height_mm - 2.75).abs() < 1e-9);
        assert!((hri.top_mm - 23.18).abs() < 0.02, "{}", hri.top_mm);
        assert_eq!(hri.parts.len(), 13);
        // The guard bars reach five modules below the others.
        let tallest = s.bars.iter().map(|b| b.height).fold(0.0, f64::max);
        assert!((tallest - (22.85 + 1.65)).abs() < 1e-6, "{tallest}");
    }

    /// Every character is seven modules wide, adjustment and all.
    #[test]
    fn the_adjustment_keeps_every_character_seven_modules_wide() {
        for value in 0..10u8 {
            for set in [Set::A, Set::B, Set::C] {
                let adjust = bar_adjustment(set, value);
                // Two bars and two spaces: the two adjustments cancel.
                assert!((2.0 * adjust + 2.0 * -adjust).abs() < 1e-12);
            }
        }
        assert_eq!(bar_adjustment(Set::A, 1), -1.0);
        assert_eq!(bar_adjustment(Set::A, 7), 1.0);
        assert_eq!(bar_adjustment(Set::B, 1), 1.0);
        assert_eq!(bar_adjustment(Set::C, 8), -1.0);
        assert_eq!(bar_adjustment(Set::A, 5), 0.0);
    }

    #[test]
    fn a_check_digit_is_added_when_it_is_missing() {
        assert_eq!(symbol(Symbology::Ean13, "400638133393").modules, symbol(Symbology::Ean13, "4006381333931").modules);
        assert!(crate::encode(&BarcodeSpec::new(Symbology::Ean13, "4006381333932")).is_err());
    }

    /// Suppressing a UPC-A's zeros and expanding the result gives the same UPC-A back, for every
    /// number with a run of zeros in its middle — and a UPC-A without one has no UPC-E at all.
    #[test]
    fn a_upc_a_survives_the_round_trip_through_its_upc_e() {
        let mut compressed = 0;
        for a in 0u8..10 {
            for b in 0u8..10 {
                for last in 0u8..10 {
                    let upc_a = [0, 1, 2, a, 0, 0, 0, 0, b, last, 0];
                    if let Some(six) = compress(&upc_a) {
                        assert_eq!(expand(0, &six), upc_a, "{upc_a:?} -> {six:?}");
                        compressed += 1;
                    }
                }
            }
        }
        assert!(compressed > 50, "only {compressed} of the numbers compressed");
        // A UPC-A whose middle carries no run of zeros has no UPC-E.
        assert_eq!(compress(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0]), None);
    }

    /// A number system of 1 inverts the parity pattern of the six characters (ISO/IEC 15420).
    #[test]
    fn the_number_system_inverts_the_parity_pattern() {
        let zero = symbol(Symbology::UpcE, "01234565");
        let one = symbol(Symbology::UpcE, "11234562");
        let xs = |s: &Symbol| s.bars.iter().map(|b| (b.x * 1000.0).round()).collect::<Vec<_>>();
        assert_eq!(zero.bars.len(), one.bars.len());
        assert_ne!(xs(&zero), xs(&one));
    }
}
