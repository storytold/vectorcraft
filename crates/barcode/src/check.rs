//! The check digits the symbologies use.
//!
//! The GS1 mod-10 check digit (GS1 *General Specifications* §7.10, ISO/IEC 15420 §5.2.2.2) is
//! shared by EAN-13, EAN-8, UPC-A, UPC-E and ITF-14: number the digits from the right, give the
//! rightmost an odd position, weight the odd positions by 3 and the even ones by 1, and take the
//! amount that brings the total up to the next multiple of ten. The other symbologies carry their
//! checksums in their own modules.

use crate::{Error, Result};

/// Whether the data a caller gave already ended in its check digit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckDigit {
    /// The caller gave it, and it was right.
    Given,
    /// The encoder worked it out and added it.
    Added,
}

/// The digits of `data`, or an error naming what is wrong with it.
///
/// `len` is the length the symbology wants *with* the check digit; the caller may give either
/// `len - 1` or `len` digits. A check digit that disagrees with the data is refused rather than
/// quietly replaced: a mistyped product number must not print as a different valid one.
pub fn digits(data: &str, len: usize, what: &str) -> Result<(Vec<u8>, CheckDigit)> {
    if !data.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::Data(format!("{what} takes digits only")));
    }
    let mut out: Vec<u8> = data.bytes().map(|b| b - b'0').collect();
    match out.len() {
        n if n == len - 1 => {
            let c = mod10(&out);
            out.push(c);
            Ok((out, CheckDigit::Added))
        }
        n if n == len => {
            let body = out.get(..len - 1).ok_or_else(|| Error::Data(format!("{what} takes {len} digits")))?;
            let want = mod10(body);
            match out.last() {
                Some(&given) if given == want => Ok((out, CheckDigit::Given)),
                Some(&given) => Err(Error::Data(format!("{what}: the check digit should be {want}, not {given}"))),
                None => Err(Error::Data(format!("{what} takes {len} digits"))),
            }
        }
        n => Err(Error::Data(format!("{what} takes {} or {len} digits, not {n}", len - 1))),
    }
}

/// The GS1 mod-10 check digit of `body`, which must not include one.
pub fn mod10(body: &[u8]) -> u8 {
    let sum: u32 = body.iter().rev().enumerate().map(|(i, &d)| u32::from(d) * if i % 2 == 0 { 3 } else { 1 }).sum();
    ((10 - sum % 10) % 10) as u8
}

/// The digits of an integer value, most significant first.
#[cfg(test)]
pub fn digit_string(value: &[u8]) -> String {
    value.iter().map(|d| char::from(b'0' + d.min(&9))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked examples GS1 prints for the check digit (§7.10) and the ones the symbology
    /// standards use.
    #[test]
    fn check_digits_match_the_standards_examples() {
        // 629104150021 -> 3 (GS1's own worked GTIN-13 example).
        assert_eq!(mod10(&[6, 2, 9, 1, 0, 4, 1, 5, 0, 0, 2, 1]), 3);
        // 400638133393 -> 1, the classic EAN-13 example.
        assert_eq!(mod10(&[4, 0, 0, 6, 3, 8, 1, 3, 3, 3, 9, 3]), 1);
        // 501234567890 -> 0.
        assert_eq!(mod10(&[5, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0]), 0);
    }

    #[test]
    fn a_given_check_digit_is_kept_and_a_wrong_one_refused() {
        let (d, how) = digits("4006381333931", 13, "EAN-13").expect("valid");
        assert_eq!((digit_string(&d).as_str(), how), ("4006381333931", CheckDigit::Given));
        let (d, how) = digits("400638133393", 13, "EAN-13").expect("short form");
        assert_eq!((digit_string(&d).as_str(), how), ("4006381333931", CheckDigit::Added));
        let err = digits("4006381333932", 13, "EAN-13").expect_err("wrong check digit");
        assert!(err.to_string().contains("should be 1"), "{err}");
        assert!(digits("40063813339", 13, "EAN-13").is_err());
        assert!(digits("400638133393X", 13, "EAN-13").is_err());
    }
}
