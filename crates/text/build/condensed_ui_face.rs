//! The condensed craft-fonts face document tab titles draw in, and its row in the checkout's opt-in
//! Latin catalog (`fonts/latin-manifest.txt`). Shared by `build.rs` and the crate's tests
//! (`#[path]`-included by both, so it has no dependencies).

/// The condensed face's family and style.
pub const FACE: (&str, &str) = ("Noto Sans", "Condensed SemiBold");

/// The file and scripts fields of [`FACE`]'s row in the catalog `text`: `None` when no row names
/// the face, and an error when the row that names it does not have the catalog's 8 fields or leaves
/// the file or scripts empty.
pub fn row(text: &str) -> Result<Option<(&str, &str)>, String> {
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let fields: Vec<&str> = line.split(" | ").map(str::trim).collect();
        match fields.as_slice() {
            [family, style, file, scripts, _, _, _, _] if (*family, *style) == FACE && !file.is_empty() && !scripts.is_empty() => {
                return Ok(Some((*file, *scripts)));
            }
            [family, style, ..] if (*family, *style) == FACE => return Err(format!("malformed manifest line: {line}")),
            _ => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The face's row as storytold/craft-fonts#72 adds it to the catalog is found, rows for other
    /// faces are not taken for it, and a row naming it without its file or scripts, or with one of
    /// them empty, is an error.
    #[test]
    fn the_catalog_row_names_the_face() {
        let catalog = "# Latin presentation fonts for the Crafting Apps (opt-in).\n\
            Inter | SemiBold | fonts/inter/Inter-SemiBold.ttf | Latn | OFL-1.1 | fonts/inter/OFL.txt | 78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3 | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip#extras/ttf/Inter-SemiBold.ttf\n\
            Noto Sans | Condensed SemiBold | fonts/noto-sans-condensed/NotoSans-CondensedSemiBold.ttf | Latn | OFL-1.1 | fonts/noto-sans-condensed/OFL.txt | b41ecd0838ecd17b706432a01c0d63f8bf977d851cc2cf37242324233edb9294 | https://github.com/notofonts/latin-greek-cyrillic/releases/download/NotoSans-v2.015/NotoSans-v2.015.zip#NotoSans/unhinted/ttf/NotoSans-CondensedSemiBold.ttf\n";
        assert_eq!(row(catalog), Ok(Some(("fonts/noto-sans-condensed/NotoSans-CondensedSemiBold.ttf", "Latn"))));
        let rest = "OFL-1.1 | fonts/a/OFL.txt | 00 | https://example.com/a.zip";
        assert_eq!(row(&format!("Noto Sans | SemiBold | fonts/a.ttf | Latn | {rest}\n")), Ok(None), "another style");
        assert_eq!(row(&format!("# Noto Sans | Condensed SemiBold | fonts/a.ttf | Latn | {rest}\n")), Ok(None), "a comment");
        assert_eq!(row("Inter | Bold\n"), Ok(None), "a short row for another face");
        assert!(row(&format!("Noto Sans | Condensed SemiBold | fonts/a.ttf | {rest}\n")).is_err(), "a row without its scripts");
        assert!(row(&format!("Noto Sans | Condensed SemiBold | fonts/a.ttf |  | {rest}\n")).is_err(), "a row with empty scripts");
    }
}
