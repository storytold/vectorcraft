//! Number to string conversion (ECMA-262 5.1 section 9.8.1), for numeric property names, and the
//! formats of `Number.prototype`'s `toString(radix)`, `toFixed`, `toExponential` and
//! `toPrecision` (section 15.7.4). The fixed, exponential and precision formats round the
//! number's exact decimal value, ties away from zero, as the specification's "larger n" says.

/// The string ToString gives for the number `x`: the shortest decimal digits that read back as
/// `x`, laid out as section 9.8.1 says.
pub fn to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x < 0.0 {
        return format!("-{}", to_string(-x));
    }
    // Rust's `{:e}` writes the shortest digits that round-trip: `d.ddde±x`.
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = digits.len() as i64;
    let n = exp.parse::<i64>().unwrap_or(0) + 1;
    if k <= n && n <= 21 {
        let zeros = usize::try_from(n - k).unwrap_or(0);
        format!("{digits}{}", "0".repeat(zeros))
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at_checked(usize::try_from(n).unwrap_or(0)).unwrap_or((&digits, ""));
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        let zeros = usize::try_from(-n).unwrap_or(0);
        format!("0.{}{digits}", "0".repeat(zeros))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at_checked(1).unwrap_or((&digits, ""));
        if rest.is_empty() { format!("{first}e{sign}{}", e.abs()) } else { format!("{first}.{rest}e{sign}{}", e.abs()) }
    }
}

/// The exact decimal digits of a positive finite `x`, without trailing zeros, and the position of
/// the decimal point: `x = 0.d1d2… × 10^point`.
fn exact_digits(x: f64) -> (Vec<u8>, i64) {
    // A double's exact decimal expansion has at most 767 significant digits.
    let sci = format!("{x:.800e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    (digits, exp.parse::<i64>().unwrap_or(0) + 1)
}

/// The shortest digits that read back as a positive finite `x`, and the decimal point's position.
fn shortest_digits(x: f64) -> (Vec<u8>, i64) {
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    (mantissa.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect(), exp.parse::<i64>().unwrap_or(0) + 1)
}

/// Rounds `digits` (a number `0.d1d2… × 10^point`) to its first `keep` digits, a half rounding
/// up. Returns exactly `keep` digits and the new point; `keep` may be 0 or negative, which leaves
/// either nothing or a single `1` one place up.
fn round_digits(digits: &[u8], point: i64, keep: i64) -> (Vec<u8>, i64) {
    if keep < 0 {
        return (Vec::new(), point);
    }
    let k = usize::try_from(keep).unwrap_or(usize::MAX);
    if digits.len() <= k {
        let mut out = digits.to_vec();
        out.resize(k, 0);
        return (out, point);
    }
    let mut out: Vec<u8> = digits.get(..k).unwrap_or_default().to_vec();
    if digits.get(k).is_some_and(|&d| d >= 5) {
        let mut i = out.len();
        loop {
            if i == 0 {
                out.insert(0, 1);
                out.truncate(k.max(1));
                return (out, point + 1);
            }
            i -= 1;
            match out.get_mut(i) {
                Some(d) if *d == 9 => *d = 0,
                Some(d) => {
                    *d += 1;
                    break;
                }
                None => break,
            }
        }
    }
    (out, point)
}

fn digits_text(digits: &[u8]) -> String {
    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// `Number.prototype.toFixed` (section 15.7.4.5) for `f` digits after the point, 0 to 20.
pub(crate) fn to_fixed(x: f64, f: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.abs() >= 1e21 {
        return to_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let x = x.abs();
    let fi = i64::try_from(f).unwrap_or(0);
    let n = if x == 0.0 {
        vec![0]
    } else {
        let (digits, point) = exact_digits(x);
        let (mut n, new_point) = round_digits(&digits, point, point + fi);
        if n.is_empty() {
            n.push(0);
        } else if new_point > point {
            // A carry past the first digit leaves one digit more than kept.
            n.push(0);
            n.truncate(usize::try_from(new_point + fi).unwrap_or(n.len()));
        }
        n
    };
    let mut m = digits_text(&n);
    let m_trim = m.trim_start_matches('0');
    m = if m_trim.is_empty() { "0".into() } else { m_trim.into() };
    if f == 0 {
        return format!("{sign}{m}");
    }
    if m.len() <= f {
        m = format!("{}{m}", "0".repeat(f + 1 - m.len()));
    }
    let (int, frac) = m.split_at(m.len() - f);
    format!("{sign}{int}.{frac}")
}

/// `Number.prototype.toExponential` (section 15.7.4.6) for `f` digits after the point (0 to 20),
/// or as many as the number needs when `f` is `None`.
pub(crate) fn to_exponential(x: f64, f: Option<usize>) -> String {
    if !x.is_finite() {
        return to_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let x = x.abs();
    let (digits, e) = if x == 0.0 {
        (vec![0; f.unwrap_or(0) + 1], 0)
    } else {
        match f {
            None => {
                let (d, point) = shortest_digits(x);
                (d, point - 1)
            }
            Some(f) => {
                let (d, point) = exact_digits(x);
                let (d, point) = round_digits(&d, point, i64::try_from(f).unwrap_or(0) + 1);
                (d, point - 1)
            }
        }
    };
    let text = digits_text(&digits);
    let (first, rest) = text.split_at(1.min(text.len()));
    let mantissa = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
    let esign = if e < 0 { '-' } else { '+' };
    format!("{sign}{mantissa}e{esign}{}", e.abs())
}

/// `Number.prototype.toPrecision` (section 15.7.4.7) for `p` significant digits, 1 to 21.
pub(crate) fn to_precision(x: f64, p: usize) -> String {
    if !x.is_finite() {
        return to_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let x = x.abs();
    let pi = i64::try_from(p).unwrap_or(1);
    let (digits, e) = if x == 0.0 {
        (vec![0; p], 0)
    } else {
        let (d, point) = exact_digits(x);
        let (d, point) = round_digits(&d, point, pi);
        (d, point - 1)
    };
    let m = digits_text(&digits);
    if e < -6 || e >= pi {
        let (first, rest) = m.split_at(1.min(m.len()));
        let mantissa = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
        let esign = if e < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}e{esign}{}", e.abs());
    }
    if e == pi - 1 {
        return format!("{sign}{m}");
    }
    if e >= 0 {
        let (int, frac) = m.split_at(usize::try_from(e + 1).unwrap_or(0).min(m.len()));
        return format!("{sign}{int}.{frac}");
    }
    format!("{sign}0.{}{m}", "0".repeat(usize::try_from(-(e + 1)).unwrap_or(0)))
}

/// `Number.prototype.toString(radix)` (section 15.7.4.2) for a radix of 2 to 36 other than 10:
/// the integer part exactly, and fraction digits until the number would read back unchanged.
pub(crate) fn to_radix_string(x: f64, radix: u32) -> String {
    if !x.is_finite() || x == 0.0 {
        return to_string(x);
    }
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let r = f64::from(radix);
    let sign = if x < 0.0 { "-" } else { "" };
    let x = x.abs();
    let mut integer = x.floor();
    let mut fraction = x - integer;
    // Half the distance to the next double: digits past this resolution are not needed.
    let next = f64::from_bits(x.to_bits() + 1);
    let mut delta = (0.5 * (next - x)).max(f64::from_bits(1));
    let mut frac_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= r;
            delta *= r;
            let digit = fraction.floor();
            frac_digits.push(digit as u8);
            fraction -= digit;
            if (fraction > 0.5 || (fraction == 0.5 && (digit as u8) & 1 == 1)) && fraction + delta > 1.0 {
                // Round up, carrying into the integer part when every digit is the largest.
                loop {
                    match frac_digits.last_mut() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(d) if u32::from(*d) + 1 == radix => {
                            frac_digits.pop();
                        }
                        Some(d) => {
                            *d += 1;
                            break;
                        }
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut int_digits: Vec<u8> = Vec::new();
    // Digits below the double's precision are zeros.
    while (integer / r) >= 9_007_199_254_740_992.0 {
        integer /= r;
        int_digits.push(0);
    }
    loop {
        let rem = integer % r;
        int_digits.push(rem as u8);
        integer = (integer - rem) / r;
        if integer <= 0.0 {
            break;
        }
    }
    let mut out = String::from(sign);
    out.extend(int_digits.iter().rev().map(|&d| char::from(CHARS.get(usize::from(d)).copied().unwrap_or(b'0'))));
    if !frac_digits.is_empty() {
        out.push('.');
        out.extend(frac_digits.iter().map(|&d| char::from(CHARS.get(usize::from(d)).copied().unwrap_or(b'0'))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{to_exponential, to_fixed, to_precision, to_radix_string, to_string};

    #[test]
    fn fixed_exponential_and_precision_round_the_exact_value_with_ties_away_from_zero() {
        let fixed: &[(f64, usize, &str)] = &[
            (0.5, 0, "1"),
            (2.5, 0, "3"),
            (1.45, 1, "1.4"),
            (1.255, 2, "1.25"),
            (1.005, 2, "1.00"),
            (0.1, 20, "0.10000000000000000555"),
            (123.456, 2, "123.46"),
            (-1.5, 0, "-2"),
            (-0.0000001, 2, "-0.00"),
            (-0.0, 2, "0.00"),
            (0.0, 0, "0"),
            (99.995, 2, "100.00"),
            (9.5, 0, "10"),
            (1e21, 2, "1e+21"),
            (1000000000000000128.0, 0, "1000000000000000128"),
            (f64::NAN, 2, "NaN"),
        ];
        for (x, f, want) in fixed {
            assert_eq!(to_fixed(*x, *f), *want, "({x}).toFixed({f})");
        }
        let exp: &[(f64, Option<usize>, &str)] = &[
            (123456.0, Some(2), "1.23e+5"),
            (123456.0, None, "1.23456e+5"),
            (0.00015, Some(1), "1.5e-4"),
            (1.25, Some(1), "1.3e+0"),
            (0.0, Some(2), "0.00e+0"),
            (0.0, None, "0e+0"),
            (-6.9e-11, Some(1), "-6.9e-11"),
            (9.99, Some(1), "1.0e+1"),
            (f64::INFINITY, Some(2), "Infinity"),
        ];
        for (x, f, want) in exp {
            assert_eq!(to_exponential(*x, *f), *want, "({x}).toExponential({f:?})");
        }
        let prec: &[(f64, usize, &str)] = &[
            (123.456, 4, "123.5"),
            (0.000123, 2, "0.00012"),
            (0.0000001234, 2, "1.2e-7"),
            (123456.0, 2, "1.2e+5"),
            (123456.0, 6, "123456"),
            (0.0, 3, "0.00"),
            (1.0, 1, "1"),
            (99.5, 2, "1.0e+2"),
            (1e21, 21, "1.00000000000000000000e+21"),
        ];
        for (x, p, want) in prec {
            assert_eq!(to_precision(*x, *p), *want, "({x}).toPrecision({p})");
        }
    }

    #[test]
    fn radix_strings_give_exact_integers_and_enough_fraction_digits() {
        let cases: &[(f64, u32, &str)] = &[
            (255.0, 16, "ff"),
            (255.0, 2, "11111111"),
            (-255.0, 36, "-73"),
            (0.5, 2, "0.1"),
            (0.5, 16, "0.8"),
            (3.75, 2, "11.11"),
            (2f64.powi(60), 16, "1000000000000000"),
            (1e21, 36, "5v1j4f4ds7c000"),
            (0.1, 3, "0.0022002200220022002200220022002201"),
        ];
        for (x, r, want) in cases {
            assert_eq!(to_radix_string(*x, *r), *want, "({x}).toString({r})");
        }
    }

    #[test]
    fn numbers_print_as_section_9_8_1_lays_them_out() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (123.456, "123.456"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.1 + 0.2, "0.30000000000000004"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
        ];
        for (x, want) in cases {
            assert_eq!(to_string(*x), *want, "{x:?}");
        }
    }
}
