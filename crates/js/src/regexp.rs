//! The early check of a regular expression literal's pattern (ECMA-262 5.1 section 15.10.1, with
//! the lenient forms of Annex B that scripts rely on: a `]`, `{` or `}` that starts no class or
//! quantifier is a literal character, a lookahead may take a quantifier, and a class range with a
//! class escape at one end is a set of literal characters).
//!
//! The check reads the pattern once, front to back, counting open groups instead of recursing.

/// Checks `pattern`, the text between a literal's slashes. Groups may nest `max_depth` deep.
pub fn check(pattern: &[u16], max_depth: usize) -> Result<(), String> {
    let mut i = 0;
    let mut depth = 0usize;
    // The term before the current position may take a quantifier.
    let mut quantifiable = false;
    while let Some(&unit) = pattern.get(i) {
        let c = char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}');
        match c {
            '\\' => {
                let next = pattern.get(i + 1).ok_or("\\ at end of pattern")?;
                quantifiable = !matches!(char::from_u32(u32::from(*next)), Some('b' | 'B'));
                i += 2;
            }
            '(' => {
                if pattern.get(i + 1) == Some(&u16::from(b'?')) {
                    match pattern.get(i + 2).and_then(|&u| char::from_u32(u32::from(u))) {
                        Some(':' | '=' | '!') => i += 3,
                        _ => return Err("invalid group".into()),
                    }
                } else {
                    i += 1;
                }
                depth += 1;
                if depth > max_depth {
                    return Err(crate::error::TOO_DEEP.into());
                }
                quantifiable = false;
            }
            ')' => {
                depth = depth.checked_sub(1).ok_or("unmatched ')'")?;
                quantifiable = true;
                i += 1;
            }
            '[' => {
                i = class(pattern, i + 1)?;
                quantifiable = true;
            }
            '*' | '+' | '?' => {
                if !quantifiable {
                    return Err("nothing to repeat".into());
                }
                i = lazy(pattern, i + 1);
                quantifiable = false;
            }
            '{' => match braces(pattern, i + 1) {
                Some((min, max, end)) => {
                    if !quantifiable {
                        return Err("nothing to repeat".into());
                    }
                    if max.is_some_and(|max| max < min) {
                        return Err("numbers out of order in {} quantifier".into());
                    }
                    i = lazy(pattern, end);
                    quantifiable = false;
                }
                None => {
                    quantifiable = true;
                    i += 1;
                }
            },
            '^' | '$' | '|' => {
                quantifiable = false;
                i += 1;
            }
            _ => {
                quantifiable = true;
                i += 1;
            }
        }
    }
    if depth > 0 {
        return Err("unterminated group".into());
    }
    Ok(())
}

/// After a quantifier ending before `i`: past the `?` that makes it lazy, if there is one.
fn lazy(pattern: &[u16], i: usize) -> usize {
    if pattern.get(i) == Some(&u16::from(b'?')) { i + 1 } else { i }
}

/// A `{n}`, `{n,}` or `{n,m}` quantifier whose `{` is before `i`: its bounds and the position
/// after its `}`. `None` when the text there is not one.
fn braces(pattern: &[u16], i: usize) -> Option<(u64, Option<u64>, usize)> {
    let (min, mut i) = digits(pattern, i)?;
    let max = if pattern.get(i) == Some(&u16::from(b',')) {
        i += 1;
        match digits(pattern, i) {
            Some((max, end)) => {
                i = end;
                Some(max)
            }
            None => None,
        }
    } else {
        Some(min)
    };
    (pattern.get(i) == Some(&u16::from(b'}'))).then_some((min, max, i + 1))
}

/// Decimal digits at `i`: their value (saturated) and the position after them.
fn digits(pattern: &[u16], mut i: usize) -> Option<(u64, usize)> {
    let start = i;
    let mut value: u64 = 0;
    while let Some(d) = pattern.get(i).and_then(|&u| char::from_u32(u32::from(u))).and_then(|c| c.to_digit(10)) {
        value = value.saturating_mul(10).saturating_add(u64::from(d));
        i += 1;
    }
    (i > start).then_some((value, i))
}

/// Reads a character class whose `[` is before `i` and returns the position after its `]`.
fn class(pattern: &[u16], mut i: usize) -> Result<usize, String> {
    if pattern.get(i) == Some(&u16::from(b'^')) {
        i += 1;
    }
    // The last class atom's character (`None` for a class escape such as `\d`), when a `-` after it
    // may make a range.
    let mut prev: Option<Option<u32>> = None;
    loop {
        let unit = *pattern.get(i).ok_or("unterminated character class")?;
        if unit == u16::from(b']') {
            return Ok(i + 1);
        }
        let (atom, next) = class_atom(pattern, i)?;
        i = next;
        let dash = pattern.get(i) == Some(&u16::from(b'-')) && pattern.get(i + 1).is_some_and(|&u| u != u16::from(b']'));
        match prev.take() {
            Some(low) => {
                if let (Some(low), Some(high)) = (low, atom)
                    && low > high
                {
                    return Err("range out of order in character class".into());
                }
            }
            None if dash => {
                prev = Some(atom);
                i += 1;
            }
            None => {}
        }
    }
}

/// A class atom at `i`: its character (`None` for a class escape) and the position after it.
fn class_atom(pattern: &[u16], i: usize) -> Result<(Option<u32>, usize), String> {
    let unit = u32::from(*pattern.get(i).ok_or("unterminated character class")?);
    if unit != u32::from(b'\\') {
        return Ok((Some(unit), i + 1));
    }
    let next = u32::from(*pattern.get(i + 1).ok_or("\\ at end of pattern")?);
    let c = char::from_u32(next).unwrap_or('\u{fffd}');
    let hex = |n: usize| -> Option<u32> {
        (0..n).try_fold(0, |v, k| pattern.get(i + 2 + k).and_then(|&u| crate::chars::hex_value(u32::from(u))).map(|d| v * 16 + d))
    };
    Ok(match c {
        'd' | 'D' | 's' | 'S' | 'w' | 'W' => (None, i + 2),
        'b' => (Some(0x08), i + 2),
        'f' => (Some(0x0C), i + 2),
        'n' => (Some(0x0A), i + 2),
        'r' => (Some(0x0D), i + 2),
        't' => (Some(0x09), i + 2),
        'v' => (Some(0x0B), i + 2),
        'x' => hex(2).map_or((Some(next), i + 2), |v| (Some(v), i + 4)),
        'u' => hex(4).map_or((Some(next), i + 2), |v| (Some(v), i + 6)),
        'c' => match pattern.get(i + 2).map(|&u| u32::from(u)) {
            Some(l) if char::from_u32(l).is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') => (Some(l % 32), i + 3),
            _ => (Some(u32::from(b'\\')), i + 1),
        },
        '0'..='7' => {
            let (value, end) = crate::regexp_vm::octal(pattern, i + 1);
            (Some(u32::from(value)), end)
        }
        _ => (Some(next), i + 2),
    })
}
