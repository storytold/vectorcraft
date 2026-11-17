//! Code 128 (ISO/IEC 15417) and its GS1 form, GS1-128.
//!
//! Every symbol character is eleven modules wide — six elements, three bars and three spaces,
//! whose widths run from one to four modules — except the stop, which is thirteen. That leaves
//! 107 values: 0 to 102 for data and the control characters, and 103 to 106 for the three start
//! characters and the stop.
//!
//! The code sets decide what a value means. A and B each carry 96 characters (A has the control
//! characters and upper case, B has both cases), C carries two digits to a value, and the control
//! characters switch between them: a symbol may start in any set and change set as it goes, which
//! is how a twelve-digit article number takes six characters instead of twelve.
//!
//! [`encode`] picks the sets itself; the `Code128A`, `Code128B` and `Code128C` symbologies hold
//! the data to one set instead, which is what a system expecting a fixed set wants.

use crate::linear::Linear;
use crate::{BarcodeSpec, Error, HriAlign, HriPart, Result, Symbol, Symbology, finish_linear};

/// Every symbol character, as the six widths of its elements, bar first. The last entry is the
/// stop character, which has a seventh element: the two-module bar that ends the symbol.
const PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213", "221312", "231212", "112232", "122132",
    "122231", "113222", "123122", "123221", "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221", "312212",
    "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313", "231113", "231311",
    "112133", "112331", "132131", "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131", "311123", "311321",
    "331121", "312113", "312311", "332111", "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214",
    "112412", "122114", "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242", "121142", "121241", "114212",
    "124112", "124211", "411212", "421112", "421211", "212141", "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113",
    "411311", "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// The three start characters.
const START_A: u8 = 103;
const START_B: u8 = 104;
const START_C: u8 = 105;
/// The stop character.
const STOP: u8 = 106;
/// The values that are control characters rather than data in sets A and B.
const CODE_C: u8 = 99;
const CODE_B: u8 = 100;
const CODE_A: u8 = 101;
const FNC1: u8 = 102;
const SHIFT: u8 = 98;

/// Where a GS1 element string needs an FNC1: after a variable-length field and before the next
/// one. A character no caller can type, so the symbol's own data can never be mistaken for one.
const GS1_SEPARATOR: char = '\u{1D}';

/// Which of the three code sets a Code 128 symbol is working in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSet {
    A,
    B,
    C,
}

impl CodeSet {
    fn start(self) -> u8 {
        match self {
            CodeSet::A => START_A,
            CodeSet::B => START_B,
            CodeSet::C => START_C,
        }
    }

    fn switch_to(self) -> u8 {
        match self {
            CodeSet::A => CODE_A,
            CodeSet::B => CODE_B,
            CodeSet::C => CODE_C,
        }
    }

    /// The value that carries `c` in this set, or None when the set has no room for it.
    fn value_of(self, c: char) -> Option<u8> {
        let n = c as u32;
        match self {
            CodeSet::A => match n {
                32..=95 => Some((n - 32) as u8),
                0..=31 => Some((n + 64) as u8),
                _ => None,
            },
            CodeSet::B => {
                if (32..=127).contains(&n) {
                    Some((n - 32) as u8)
                } else {
                    None
                }
            }
            // Set C carries digit pairs, which the caller forms.
            CodeSet::C => None,
        }
    }

    /// Whether the set can carry `c` at all, which is what decides a shift or a switch.
    fn holds(self, c: char) -> bool {
        self.value_of(c).is_some()
    }
}

/// How many digits run from `from`.
fn digit_run(chars: &[char], from: usize) -> usize {
    chars.iter().skip(from).take_while(|c| c.is_ascii_digit()).count()
}

/// How many characters from `from` the set `set` holds before one it does not.
fn run_in(chars: &[char], from: usize, set: CodeSet) -> usize {
    chars.iter().skip(from).take_while(|c| set.holds(**c)).count()
}

/// The data characters of a symbol, as code values, together with the set it starts in.
///
/// `fixed` pins the whole symbol to one set, which is what the three Code 128 variants do; `gs1`
/// puts an FNC1 in the first position, which is what makes a symbol a GS1-128.
fn values(data: &str, fixed: Option<CodeSet>, gs1: bool) -> Result<(CodeSet, Vec<u8>)> {
    // GS1 writes its application identifiers in brackets for people to read; the symbol carries
    // the bare digits and the FNC1s that separate them.
    let data = &if gs1 { gs1_data(data)? } else { data.to_string() };
    if !data.is_ascii() {
        return Err(Error::Data("Code 128 carries ASCII only".into()));
    }
    let chars: Vec<char> = data.chars().collect();
    let mut out = Vec::new();
    if gs1 {
        // The first FNC1 is what marks a symbol as GS1-128; it is not a data character, so it
        // comes before the start character's position in the checksum's weight order.
        out.push(FNC1);
    }
    if let Some(set) = fixed {
        if set == CodeSet::C {
            if !chars.len().is_multiple_of(2) || chars.is_empty() {
                return Err(Error::Data("Code 128 C carries digit pairs, so it needs an even number of digits".into()));
            }
            for pair in chars.chunks(2) {
                out.push(digit_pair(pair)?);
            }
        } else {
            for &c in &chars {
                if c == GS1_SEPARATOR {
                    out.push(FNC1);
                    continue;
                }
                out.push(set.value_of(c).ok_or_else(|| {
                    Error::Data(format!("{} cannot carry {}", if set == CodeSet::A { "Code 128 A" } else { "Code 128 B" }, c.escape_debug()))
                })?);
            }
        }
        return Ok((set, out));
    }

    // The set the symbol starts in: C for a leading run of digits, else A when the first
    // character needs it, else B (ISO/IEC 15417 Annex B's recommendation).
    let lead = digit_run(&chars, 0);
    let mut set = if lead >= 4 && lead.is_multiple_of(2) {
        CodeSet::C
    } else if chars.first().is_some_and(|c| !CodeSet::B.holds(*c)) {
        CodeSet::A
    } else {
        CodeSet::B
    };
    if lead >= 4 && !lead.is_multiple_of(2) {
        // An odd leading run: one digit in set B, then the rest as pairs in C.
        set = CodeSet::B;
    }
    let start_set = set;
    let mut i = 0;
    while i < chars.len() {
        // An FNC1 costs one value and ends a run of digits, so set C can carry one and go on with
        // the pairs after it.
        if chars.get(i) == Some(&GS1_SEPARATOR) {
            out.push(FNC1);
            i += 1;
            continue;
        }
        let digits = digit_run(&chars, i);
        // Six digits are worth a switch in the middle of a symbol; four at the end, where there is
        // no later data to amortise the switch against.
        let worth_switching = digits >= 6 || (digits >= 4 && i + digits == chars.len());
        if set != CodeSet::C && worth_switching {
            if !digits.is_multiple_of(2) {
                // An odd run: its first digit goes in the set the symbol is in, the rest as pairs.
                // Switching without it would leave a Start B followed by set-C values, which reads
                // as other characters.
                let c = chars.get(i).copied().unwrap_or(' ');
                out.push(set.value_of(c).ok_or_else(|| Error::Data(format!("Code 128 cannot carry {}", c.escape_debug())))?);
                i += 1;
            }
            out.push(CodeSet::C.switch_to());
            set = CodeSet::C;
            continue;
        }
        if set == CodeSet::C {
            if digits >= 2 {
                let pair = chars.get(i..i + 2).unwrap_or(&[]);
                out.push(digit_pair(pair)?);
                i += 2;
            } else {
                // Fewer than two digits left: leave C for the set that can carry what remains.
                let next = chars.get(i).copied().unwrap_or(' ');
                let to = if CodeSet::B.holds(next) { CodeSet::B } else { CodeSet::A };
                out.push(to.switch_to());
                set = to;
            }
            continue;
        }
        let c = chars.get(i).copied().unwrap_or(' ');
        if let Some(v) = set.value_of(c) {
            out.push(v);
            i += 1;
            continue;
        }
        let other = if set == CodeSet::A { CodeSet::B } else { CodeSet::A };
        if other.holds(c) {
            // One character out of place: the Shift character carries it without leaving the set.
            let run = run_in(&chars, i, other);
            if run == 1 {
                out.push(SHIFT);
                out.push(other.value_of(c).unwrap_or(0));
                i += 1;
            } else {
                out.push(other.switch_to());
                set = other;
            }
            continue;
        }
        return Err(Error::Data(format!("Code 128 cannot carry {}", c.escape_debug())));
    }
    Ok((start_set, out))
}

/// The characters a GS1-128 carries for `data` written as application identifiers in brackets: the
/// identifiers and their data, with an FNC1 between a variable-length field and the field after it
/// (GS1 General Specifications §7.8.4). Text with no brackets is taken as the bare data.
fn gs1_data(data: &str) -> Result<String> {
    if !data.contains('(') {
        return Ok(data.to_string());
    }
    let mut elements: Vec<(&str, &str)> = Vec::new();
    let mut rest = data;
    while let Some(body) = rest.strip_prefix('(') {
        let close = body.find(')').ok_or_else(|| Error::Data("a GS1 application identifier is written (…)".into()))?;
        let ai = &body[..close];
        if !(2..=4).contains(&ai.len()) || !ai.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::Data(format!("`({ai})` is not an application identifier")));
        }
        let after = &body[close + 1..];
        let end = after.find('(').unwrap_or(after.len());
        let value = &after[..end];
        // A fixed-length identifier is delimited by its own length, so a value of another length,
        // or one that is not digits, is data no scanner can read back as written: refuse it rather
        // than print it.
        if let Some(n) = gs1_fixed_length(ai) {
            let length = value.chars().count();
            if length != n {
                return Err(Error::Data(format!("({ai}) carries {n} characters, not {length}")));
            }
            if !value.chars().all(|c| c.is_ascii_digit()) {
                return Err(Error::Data(format!("({ai}) carries digits")));
            }
        }
        elements.push((ai, value));
        rest = &after[end..];
    }
    if !rest.is_empty() {
        return Err(Error::Data(format!("`{rest}` is outside an application identifier")));
    }
    let mut out = String::new();
    for (n, (ai, value)) in elements.iter().enumerate() {
        out.push_str(ai);
        out.push_str(value);
        // A fixed-length field is delimited by its own length; a variable-length one is not, and
        // needs the separator before whatever follows it.
        if n + 1 < elements.len() && gs1_fixed_length(ai).is_none() {
            out.push(GS1_SEPARATOR);
        }
    }
    Ok(out)
}

/// The data length a GS1 application identifier fixes, or `None` for a variable-length one — the
/// identifiers packaging uses, from GS1 General Specifications Table 7.8.4-1. Every one of them
/// carries digits.
fn gs1_fixed_length(ai: &str) -> Option<usize> {
    match ai {
        "00" => Some(18),
        "01" | "02" => Some(14),
        "11" | "12" | "13" | "15" | "16" | "17" => Some(6),
        "20" => Some(2),
        // 310n to 369n: a measure whose last identifier digit is its number of decimals.
        _ if ai.len() == 4 && matches!(&ai[..2], "31" | "32" | "33" | "34" | "35" | "36") => Some(6),
        // 410 to 417: a GLN. Matched by their three digits, not by the `41` prefix, which is no
        // identifier of its own.
        "410" | "411" | "412" | "413" | "414" | "415" | "416" | "417" => Some(13),
        "422" => Some(3),
        "7001" => Some(13),
        "7003" => Some(10),
        "7006" => Some(6),
        "7040" => Some(4),
        _ => None,
    }
}

/// The value of a two-digit pair in set C.
fn digit_pair(pair: &[char]) -> Result<u8> {
    let a = pair.first().and_then(|c| c.to_digit(10));
    let b = pair.get(1).and_then(|c| c.to_digit(10));
    match (a, b) {
        (Some(a), Some(b)) => Ok((a * 10 + b) as u8),
        _ => Err(Error::Data("Code 128 C carries digits only".into())),
    }
}

/// The mod-103 check character: the start value plus every value times its position, counting
/// from one (ISO/IEC 15417).
fn check_value(start: u8, values: &[u8]) -> u8 {
    let sum: u32 = u32::from(start) + values.iter().enumerate().map(|(i, v)| u32::from(*v) * (i as u32 + 1)).sum::<u32>();
    (sum % 103) as u8
}

/// Encode a Code 128 symbol.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let gs1 = spec.symbology == Symbology::Ean128;
    let fixed = match spec.symbology {
        Symbology::Code128A => Some(CodeSet::A),
        Symbology::Code128B => Some(CodeSet::B),
        Symbology::Code128C => Some(CodeSet::C),
        _ => None,
    };
    let (start_set, coded) = values(data, fixed, gs1)?;
    let shown = if gs1 { data.replace(['(', ')'], "") } else { data.to_string() };
    let start = start_set.start();
    let check = check_value(start, &coded);

    let height = spec.resolved_height_mm();
    let mut l = Linear::new();
    write_value(&mut l, start, height);
    for v in coded.iter().chain(std::iter::once(&check)) {
        write_value(&mut l, *v, height);
    }
    write_value(&mut l, STOP, height);
    let modules = l.width_modules();
    Ok(finish_linear(spec, l, centred(&shown, modules)))
}

/// Write one symbol character's six, or for the stop seven, elements.
fn write_value(l: &mut Linear, value: u8, height: f64) {
    let Some(runs) = PATTERNS.get(usize::from(value)) else { return };
    for (i, ch) in runs.chars().enumerate() {
        let width = ch.to_digit(10).map_or(1.0, f64::from);
        match i % 2 {
            0 => l.bar(width, 0.0, height),
            _ => l.gap(width),
        }
    }
}

/// The human readable line of a symbol whose digits run the width of the bar pattern.
fn centred(data: &str, modules: f64) -> Vec<HriPart> {
    vec![HriPart { text: data.to_string(), at_mm: modules / 2.0, align: HriAlign::Center, width_mm: modules, full_size: true }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(data: &str, symbology: Symbology) -> String {
        crate::testutil::module_row(&crate::testutil::linear(symbology, data))
    }

    /// The pattern table is the one ISO/IEC 15417 prints: 107 entries, six elements each except
    /// the stop's seven, and every element one to four modules.
    #[test]
    fn the_pattern_table_is_the_standard_one() {
        assert_eq!(PATTERNS.len(), 107);
        for (i, p) in PATTERNS.iter().enumerate() {
            let expect = if i == usize::from(STOP) { 7 } else { 6 };
            assert_eq!(p.len(), expect, "entry {i}");
            assert!(p.chars().all(|c| matches!(c, '1'..='4')), "entry {i}");
            let total: u32 = p.chars().filter_map(|c| c.to_digit(10)).sum();
            let want = if i == usize::from(STOP) { 13 } else { 11 };
            assert_eq!(total, want, "entry {i}");
        }
    }

    /// A symbol is 11 modules a character plus the 13-module stop, and the quiet zones are 10X.
    #[test]
    fn the_symbol_is_the_width_the_standard_gives() {
        // Start B, "A", "B" and the check character: four eleven-module characters, then the
        // thirteen-module stop.
        assert_eq!(modules("AB", Symbology::Code128).len(), 4 * 11 + 13);
        // Six digits in set C: start C, three pairs, the check character and the stop.
        assert_eq!(modules("123456", Symbology::Code128).len(), 5 * 11 + 13);
    }

    /// The check character is the start value plus each value times its position, mod 103.
    #[test]
    fn the_check_character_is_mod_103() {
        // "123456" is three values in set C: 105 + 12×1 + 34×2 + 56×3 = 353, and 353 mod 103
        // is 44.
        let (start_set, coded) = values("123456", None, false).expect("encodes");
        assert_eq!(start_set, CodeSet::C);
        assert_eq!(coded, vec![12, 34, 56]);
        assert_eq!(check_value(START_C, &coded), 44);
        // "AIM1234" is seven values in set B: 104 + 33×1 + 41×2 + 45×3 + 17×4 + 18×5 + 19×6
        // + 20×7 = 766, and 766 mod 103 is 45.
        let (start_set, coded) = values("AIM1234", Some(CodeSet::B), false).expect("encodes");
        assert_eq!(start_set, CodeSet::B);
        assert_eq!(check_value(START_B, &coded), 45);
    }

    /// A long digit run switches to set C, and a short one does not.
    #[test]
    fn set_c_is_used_where_it_is_worth_it() {
        let (_, v) = values("AB123456", None, false).expect("encodes");
        assert!(v.contains(&CODE_C), "{v:?}");
        let (_, v) = values("AB123", None, false).expect("encodes");
        assert!(!v.contains(&CODE_C), "a run of three digits is not worth a switch: {v:?}");
    }

    /// One character outside the set a symbol is working in costs a Shift, not a switch.
    #[test]
    fn a_single_odd_character_is_shifted() {
        let (_, v) = values("abcd\te", None, false).expect("encodes");
        assert!(v.contains(&SHIFT), "{v:?}");
        assert!(!v.contains(&CODE_A), "{v:?}");
    }

    #[test]
    fn a_gs1_symbol_starts_with_fnc1_and_drops_the_brackets() {
        let (_, bracketed) = values("(01)12345678901231", None, true).expect("encodes");
        assert_eq!(bracketed.first(), Some(&FNC1), "a GS1-128 starts with FNC1");
        // The brackets are only for people: the digits themselves reach the symbol.
        let digits: Vec<u8> = "0112345678901231".bytes().map(|b| b - b'0').collect();
        assert!(digits.chunks(2).all(|pair| bracketed.contains(&(pair[0] * 10 + pair[1]))), "{bracketed:?}");
    }

    #[test]
    fn the_fixed_sets_refuse_what_they_cannot_carry() {
        assert!(values("abc", Some(CodeSet::A), false).is_err());
        assert!(values("ABC", Some(CodeSet::C), false).is_err());
        assert!(values("123", Some(CodeSet::C), false).is_err());
        assert!(values("ABC", Some(CodeSet::B), false).is_ok());
    }

    /// An odd run of digits carries its first digit in the set the symbol is in: switching to C
    /// without it leaves a Start B followed by pair values, which reads as other characters.
    #[test]
    fn an_odd_run_of_digits_keeps_its_first_digit() {
        let (set, v) = values("12345", None, false).expect("encodes");
        assert_eq!(set, CodeSet::B);
        assert_eq!(v, vec![17, CodeSet::C.switch_to(), 23, 45]);
        // The same in the middle of a symbol: seven digits carry the first, then pair the rest.
        let (_, v) = values("AB1234567CD", None, false).expect("encodes");
        assert_eq!(&v[..7], &[33, 34, 17, CodeSet::C.switch_to(), 23, 45, 67], "{v:?}");
    }

    /// A GS1 element string puts FNC1 after a variable-length field and before the next one, so a
    /// scanner knows where the lot number ends (GS1 General Specifications §7.8.4).
    #[test]
    fn gs1_separates_a_variable_field_from_the_next() {
        let (_, v) = values("(10)LOT1(21)SER9", None, true).expect("encodes");
        assert_eq!(v.first(), Some(&FNC1));
        assert_eq!(v.iter().filter(|&&x| x == FNC1).count(), 2, "the leading one and the separator: {v:?}");
        // A fixed-length field is delimited by its own length: no FNC1 after it.
        let (_, v) = values("(01)12345678901231(10)AB", None, true).expect("encodes");
        assert_eq!(v.iter().filter(|&&x| x == FNC1).count(), 1, "{v:?}");
        // Text with no brackets is the bare data, FNC1-marked as before.
        let (_, v) = values("0112345678901231", None, true).expect("encodes");
        assert_eq!(v.iter().filter(|&&x| x == FNC1).count(), 1, "{v:?}");
    }

    #[test]
    fn an_element_outside_its_brackets_is_refused() {
        assert!(values("(10)LOT", None, true).is_ok());
        assert!(values("(10", None, true).is_err());
        assert!(values("10)LOT", None, true).is_ok(), "no bracket at the start is bare data");
        assert!(values("(1x)LOT", None, true).is_err());
    }

    /// A fixed-length identifier carries exactly its own number of digits: `(01)` is a GTIN-14,
    /// and a shorter one would leave a scanner reading the next field as its digits.
    #[test]
    fn a_fixed_length_field_of_the_wrong_length_is_refused() {
        assert!(values("(01)12345678901231(10)AB", None, true).is_ok());
        let err = values("(01)123(10)AB", None, true).expect_err("13 digits short");
        assert!(err.to_string().contains("14"), "{err}");
        assert!(values("(11)260101(10)AB", None, true).is_ok(), "a date is six characters");
        assert!(values("(11)2601(10)AB", None, true).is_err());
        // A fixed-length field carries digits: a letter is data no scanner can read back.
        assert!(values("(01)1234567890123A", None, true).is_err());
        assert!(values("(01)1234-678901231", None, true).is_err());
        // 410 to 417 are GLNs, matched by three digits rather than by a `41` prefix.
        assert!(values("(410)1234567890123", None, true).is_ok());
        let (_, v) = values("(410)1234567890123(10)AB", None, true).expect("a GLN, then a lot");
        assert_eq!(v.iter().filter(|&&x| x == FNC1).count(), 1, "a fixed-length field needs no separator: {v:?}");
        assert!(values("(410)123", None, true).is_err());
        // A variable-length field takes what it is given.
        assert!(values("(10)1(21)2", None, true).is_ok());
    }
}
