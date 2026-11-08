//! Number to string conversion (ECMA-262 5.1 section 9.8.1), for numeric property names.

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

#[cfg(test)]
mod tests {
    use super::to_string;

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
