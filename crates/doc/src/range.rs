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
    // Past each run of indices picked so far, the index after it: an interval skips the runs it
    // overlaps instead of walking them again, so repeating `1-` costs no more than writing it once.
    let mut next = std::collections::HashMap::new();
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
        let mut i = unpicked(&mut next, a - 1);
        while i < b {
            out.push(i);
            next.insert(i, i + 1);
            i = unpicked(&mut next, i + 1);
        }
    }
    if out.is_empty() {
        return Err(format!("range `{s}` is empty"));
    }
    Ok(out)
}

/// The first index from `i` on not picked yet, `next` linking each picked index onward (the links
/// followed are then pointed straight at it).
fn unpicked(next: &mut std::collections::HashMap<usize, usize>, i: usize) -> usize {
    let mut end = i;
    while let Some(&j) = next.get(&end) {
        end = j;
    }
    let mut at = i;
    while at != end {
        match next.insert(at, end) {
            Some(j) => at = j,
            None => break,
        }
    }
    end
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

    #[test]
    fn repeats_and_overlaps_are_dropped_in_order() {
        assert_eq!(parse_range("3-5, 1-4, 2, 6-, -1", 7), Ok(vec![2, 3, 4, 0, 1, 5, 6]));
        assert_eq!(parse_range("2,2,1-2", 4), Ok(vec![1, 0]));
        assert_eq!(parse_range("4, 2, 1-", 5), Ok(vec![3, 1, 0, 2, 4]));
    }

    #[test]
    fn a_range_repeated_many_times_is_read_once() {
        // 100 000 pages named 50 000 times: walking the pages for each would take billions of steps.
        let s = "1-,".repeat(50_000);
        let pages = parse_range(&s, 100_000).unwrap();
        assert_eq!(pages.len(), 100_000);
        assert!(pages.iter().enumerate().all(|(i, p)| i == *p));
    }
}
