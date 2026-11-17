//! Interleaved 2 of 5, in its ITF-14 form and without it (ISO/IEC 16390 and GS1 §5.3).
//!
//! Digits are encoded in pairs: the first digit's five elements are bars, the second's five are
//! the spaces between them, so a pair is ten elements wide. Each digit has exactly two wide
//! elements and three narrow ones, which is where the symbology's name comes from.
//!
//! ITF-14 is Interleaved 2 of 5 with GS1's rules on top: fourteen digits (a GTIN with its check
//! digit), a wide-to-narrow ratio around 2.5:1, 10X quiet zones, and the bearer bars of
//! GS1 §5.3.1.5 that butt against the top and bottom of the symbol so a worn printing plate
//! cannot spread the bars.

use crate::check;
use crate::linear::Linear;
use crate::{BarcodeSpec, Error, HriAlign, HriPart, Result, Symbol, Symbology, finish_linear};

/// The five elements of each digit: 1 is a wide element, and the bits read left to right.
const DIGITS: [u8; 10] = [0b00110, 0b10001, 0b01001, 0b11000, 0b00101, 0b10100, 0b01100, 0b00011, 0b10010, 0b01010];

/// The wide-to-narrow ratio an ITF symbol uses.
///
/// Neither standard fixes one: ISO/IEC 16390 allows 2:1 to 3:1 and GS1 §5.3.1.3 gives 2.25:1 to
/// 3.0:1 with 2.5:1 as its worked example, which is the default here.
pub fn itf_ratio(spec: &BarcodeSpec) -> f64 {
    match spec.ratio {
        Some(r) if r.is_finite() => r.clamp(2.0, 3.0),
        _ => 2.5,
    }
}

/// Encode an Interleaved 2 of 5 symbol.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let itf14 = spec.symbology == Symbology::Itf14;
    let digits: Vec<u8> = if itf14 {
        let (d, _) = check::digits(data, 14, "ITF-14")?;
        d
    } else {
        if !data.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::Data("ITF-25 takes digits only".into()));
        }
        if !data.len().is_multiple_of(2) || data.is_empty() {
            return Err(Error::Data("Interleaved 2 of 5 encodes digit pairs, so it needs an even number of digits".into()));
        }
        data.bytes().map(|b| b - b'0').collect()
    };
    if !digits.len().is_multiple_of(2) {
        return Err(Error::Data("Interleaved 2 of 5 encodes digit pairs, so it needs an even number of digits".into()));
    }

    let wide = itf_ratio(spec);
    let narrow = 1.0;
    let height = spec.resolved_height_mm();
    let width_of = |bit_set: bool| if bit_set { wide } else { narrow };

    let mut l = Linear::new();
    // The start pattern: four narrow elements, bar first.
    l.bar(narrow, 0.0, height);
    l.gap(narrow);
    l.bar(narrow, 0.0, height);
    l.gap(narrow);
    for pair in digits.chunks(2) {
        let (bars, spaces) = match (pair.first(), pair.get(1)) {
            (Some(&a), Some(&b)) => (bits(a), bits(b)),
            _ => return Err(Error::Data("Interleaved 2 of 5 encodes digit pairs".into())),
        };
        // Ten elements to a pair: the first digit's five bars with the second digit's five
        // spaces interleaved between them, so a pair ends on a space like it starts on a bar.
        for i in 0..5 {
            l.bar(width_of(bars >> (4 - i) & 1 == 1), 0.0, height);
            l.gap(width_of(spaces >> (4 - i) & 1 == 1));
        }
    }
    // The stop pattern: a wide bar, a narrow space and a narrow bar.
    l.bar(wide, 0.0, height);
    l.gap(narrow);
    l.bar(narrow, 0.0, height);

    let text: String = digits.iter().map(|d| char::from(b'0' + d % 10)).collect();
    let modules = l.width_modules();
    Ok(finish_linear(spec, l, centred(&text, modules)))
}

/// The five bits of a digit, or 0 for a value that is not a digit (which [`encode`] has already
/// refused).
fn bits(d: u8) -> u8 {
    if d > 99 {
        return 0;
    }
    // A pair's two digits came from characters, so they are always below ten.
    DIGITS[usize::from(d) % 10]
}

/// The human readable line of a symbol whose digits run the width of the bar pattern.
fn centred(data: &str, modules: f64) -> Vec<HriPart> {
    vec![HriPart { text: data.to_string(), at_mm: modules / 2.0, align: HriAlign::Center, width_mm: modules, full_size: true }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(data: &str, symbology: Symbology) -> Symbol {
        match crate::encode(&BarcodeSpec::new(symbology, data)) {
            Ok(crate::Code::Linear(l)) => l,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_digit_has_two_wide_elements() {
        for (d, bits) in DIGITS.iter().enumerate() {
            assert_eq!(bits.count_ones(), 2, "digit {d}");
            assert_eq!(bits >> 5, 0, "digit {d} has more than five elements");
        }
    }

    /// GS1's own worked example (§5.3.1.3): seven digit pairs, a 2.5:1 ratio, X = 1.016 mm and
    /// 10X quiet zones give a 142.75 mm symbol.
    #[test]
    fn an_itf14_is_the_width_gs1_works_out() {
        let s = symbol("15400141288763", Symbology::Itf14);
        assert!((s.module_mm - 1.016).abs() < 1e-9);
        assert!((s.width_mm - 142.75).abs() < 0.05, "{}", s.width_mm);
    }

    /// The bearer bars run the full width at the top and bottom, 4.8 mm thick (GS1 §5.3.1.5),
    /// and the human readable line clears them by 1.02 mm.
    #[test]
    fn an_itf14_has_bearer_bars() {
        let s = symbol("15400141288763", Symbology::Itf14);
        assert!((s.bearer_mm - 4.8).abs() < 1e-9);
        let top = s.bars.iter().filter(|b| b.y.abs() < 1e-9).map(|b| b.width).fold(0.0, f64::max);
        assert!((top - s.width_mm).abs() < 1e-6, "the bearer bar spans the quiet zones too");
        let hri = s.human_readable.as_ref().expect("a human readable line");
        let bars_bottom = 4.8 + 32.0;
        assert!((hri.top_mm - (bars_bottom + 4.8 + 1.02)).abs() < 1e-6, "{}", hri.top_mm);
        assert_eq!(hri.parts.first().map(|p| p.text.as_str()), Some("15400141288763"));
    }

    /// Without the bearered form the symbol is just the bars, and the check digit is added.
    #[test]
    fn a_plain_itf_has_no_bearer_bars() {
        let s = symbol("1234", Symbology::Itf25);
        assert_eq!(s.bearer_mm, 0.0);
        assert!(crate::encode(&BarcodeSpec::new(Symbology::Itf25, "12345")).is_err());
        // ITF-14 takes thirteen digits and works the check digit out.
        assert_eq!(symbol("1540014128876", Symbology::Itf14).modules, symbol("15400141288763", Symbology::Itf14).modules);
    }
}
