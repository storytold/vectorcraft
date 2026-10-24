//! Page ranges as people type them (`"1-3, 5"`): artboards to export or print, PDF pages to
//! open, tiles to print.

/// A 1-based page/artboard range such as `"1-3, 5"` → 0-based indices in the order given (repeats
/// dropped). Each number must lie in `1..=count`; `"3-"` runs to the last one, `"-2"` from the first.
pub fn parse_range(s: &str, count: usize) -> Result<Vec<usize>, String> {
    let num = |t: &str, open: usize| -> Result<usize, String> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(open);
        }
        t.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| t.parse().ok())
            .flatten()
            .filter(|n| (1..=count).contains(n))
            .ok_or_else(|| format!("range `{s}`: `{t}` is not a number from 1 to {count}"))
    };
    let mut out = vec![];
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.replace('\u{2013}', "-").split_once('-') {
            Some((a, b)) if a.trim().is_empty() && b.trim().is_empty() => return Err(format!("range `{s}`: `{part}` names no number")),
            Some((a, b)) => (num(a, 1)?, num(b, count)?),
            None => {
                let n = num(part, 0)?;
                (n, n)
            }
        };
        if a > b {
            return Err(format!("range `{s}`: `{part}` runs backwards"));
        }
        for i in a - 1..b {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    if out.is_empty() {
        return Err(format!("range `{s}` is empty"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn sign_is_not_part_of_a_page_number() {
        assert!(parse_range("+2", 5).is_err());
        assert!(parse_range("1-+3", 5).is_err());
        assert!(parse_range("-+2", 5).is_err());
        assert_eq!(parse_range("1-3, 5", 5), Ok(vec![0, 1, 2, 4]));
    }
}
