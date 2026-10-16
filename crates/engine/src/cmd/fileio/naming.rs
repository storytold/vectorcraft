//! File-name templates for batch exports (`document.exportForScreens {name}`): the same pattern
//! gives every exported file a consistent name, such as `COMPANY-Logo-<filename>-####-YYYYMMDD`.
//!
//! Tokens:
//! - `<filename>` (or `<document>`): the document's name without its extension;
//! - `<name>` (or `<artboard>`, `<item>`): the artboard's, asset's, layer's or selection's name;
//! - `<index>`: the item's place in the export, from 1;
//! - a run of `#`: the counter, zero-padded to the run's length (`####` → `0001`);
//! - `<format>` (or `<ext>`): the file's extension; `<scale>`: the row's size (`2x`, `100w`);
//!   `<suffix>`: the row's suffix (otherwise added at the end);
//! - the date and time: `YYYYMMDD`, `YYYY-MM-DD` and `YYYY` as written, and `<date>`
//!   (YYYYMMDD), `<time>` (hhmmss), `<YYYY>`, `<MM>`, `<DD>`, `<hh>`, `<mm>`, `<ss>`.
//!
//! A trailing `.ext` (or `.<ext>`, `.<format>`) is dropped: the extension comes from the format.
//! Anything that could start a folder (`/`, `\`, `:`) becomes `-`.

/// What a template's tokens stand for, for one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameParts<'a> {
    pub filename: &'a str,
    pub name: &'a str,
    pub index: usize,
    pub counter: u64,
    pub format: &'a str,
    pub scale: &'a str,
    pub suffix: &'a str,
    /// `[year, month, day, hour, minute, second]`.
    pub when: [i64; 6],
}

/// The bracketed tokens and what each stands for.
fn token(name: &str, p: &NameParts<'_>) -> Option<String> {
    let [y, mo, d, h, mi, s] = p.when;
    Some(match name {
        "filename" | "document" => p.filename.to_string(),
        "name" | "artboard" | "item" => p.name.to_string(),
        "index" => p.index.to_string(),
        "format" | "ext" => p.format.to_string(),
        "scale" => p.scale.to_string(),
        "suffix" => p.suffix.to_string(),
        "date" => format!("{y:04}{mo:02}{d:02}"),
        "time" => format!("{h:02}{mi:02}{s:02}"),
        "YYYY" => format!("{y:04}"),
        "MM" => format!("{mo:02}"),
        "DD" => format!("{d:02}"),
        "hh" => format!("{h:02}"),
        "mm" => format!("{mi:02}"),
        "ss" => format!("{s:02}"),
        _ => return None,
    })
}

/// Text with `/`, `\` and `:` turned into `-`, so it can't start a folder.
fn clean(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect()
}

/// Does `template` place the row's suffix itself (so it isn't added at the end)?
pub fn has_suffix_token(template: &str) -> bool {
    template.contains("<suffix>")
}

/// The file name (without extension) `template` gives for `p`.
pub fn render(template: &str, p: &NameParts<'_>) -> String {
    let mut t = template.trim();
    for ext in [".ext", ".<ext>", ".<format>"] {
        if let Some(stripped) = t.strip_suffix(ext) {
            t = stripped;
        }
    }
    let [y, mo, d, ..] = p.when;
    let mut out = String::with_capacity(t.len() + 16);
    let mut rest = t;
    while let Some(c) = rest.chars().next() {
        // Bracketed tokens; an unknown one stays as written.
        if c == '<'
            && let Some(end) = rest.find('>')
            && let Some(value) = rest.get(1..end).and_then(|name| token(name, p))
        {
            out.push_str(&clean(&value));
            rest = rest.get(end + 1..).unwrap_or("");
            continue;
        }
        if c == '#' {
            let run = rest.chars().take_while(|c| *c == '#').count();
            out.push_str(&format!("{:0run$}", p.counter));
            rest = rest.get(run..).unwrap_or("");
            continue;
        }
        // The date spelled out, longest first.
        let date = [("YYYYMMDD", format!("{y:04}{mo:02}{d:02}")), ("YYYY-MM-DD", format!("{y:04}-{mo:02}-{d:02}")), ("YYYY", format!("{y:04}"))];
        if let Some((pat, value)) = date.iter().find(|(pat, _)| rest.starts_with(pat)) {
            out.push_str(value);
            rest = rest.get(pat.len()..).unwrap_or("");
            continue;
        }
        out.push(if matches!(c, '/' | '\\' | ':') { '-' } else { c });
        rest = rest.get(c.len_utf8()..).unwrap_or("");
    }
    out.trim().to_string()
}

/// `YYYY-MM-DD` (or `YYYYMMDD`) as `[year, month, day, 0, 0, 0]`; `None` when it isn't a date.
pub fn parse_date(s: &str) -> Option<[i64; 6]> {
    let digits: String = s.trim().chars().filter(|c| *c != '-').collect();
    if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |a: usize, b: usize| digits.get(a..b).and_then(|t| t.parse::<i64>().ok());
    let (y, m, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some([y, m, d, 0, 0, 0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> NameParts<'static> {
        NameParts { filename: "Brand", name: "Dark", index: 3, counter: 7, format: "png", scale: "2x", suffix: "@2x", when: [2026, 10, 16, 9, 5, 0] }
    }

    #[test]
    fn the_example_from_the_request() {
        assert_eq!(render("COMPANY-Logo-<filename>-####-YYYYMMDD.ext", &parts()), "COMPANY-Logo-Brand-0007-20261016");
    }

    #[test]
    fn every_token() {
        let p = parts();
        assert_eq!(render("<document>_<name>_<artboard>_<item>", &p), "Brand_Dark_Dark_Dark");
        assert_eq!(render("<index>-#-##-<format>-<ext>-<scale><suffix>", &p), "3-7-07-png-png-2x@2x");
        assert_eq!(render("<date>T<time> <YYYY>/<MM>/<DD> <hh>:<mm>:<ss>", &p), "20261016T090500 2026-10-16 09-05-00");
        assert_eq!(render("YYYY-MM-DD YYYY", &p), "2026-10-16 2026");
        // Words are left alone: only the date patterns and bracketed tokens are replaced.
        assert_eq!(render("SUMMER-DD-MM", &p), "SUMMER-DD-MM");
        assert_eq!(render("<unknown>-x", &p), "<unknown>-x");
        assert_eq!(render("logo.<format>", &p), "logo", "the extension comes from the format");
        assert!(has_suffix_token("a<suffix>") && !has_suffix_token("a"));
    }

    #[test]
    fn no_name_climbs_out_of_its_folder() {
        let p = NameParts { name: "../../etc", ..parts() };
        let n = render("a/b\\c:<name>", &p);
        assert!(!n.contains('/') && !n.contains('\\') && !n.contains(':'), "{n}");
        // A long counter is padded, a short run is not cut.
        assert_eq!(render("########", &NameParts { counter: 123, ..parts() }), "00000123");
        assert_eq!(render("#", &NameParts { counter: 12345, ..parts() }), "12345");
    }

    #[test]
    fn dates_parse() {
        assert_eq!(parse_date("2026-10-16"), Some([2026, 10, 16, 0, 0, 0]));
        assert_eq!(parse_date("20261016"), Some([2026, 10, 16, 0, 0, 0]));
        for bad in ["", "2026-13-01", "2026-1-1", "today", "2026-10-1x"] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }
}
