//! `web-fonts.txt` (the craft-fonts faces web builds fetch instead of embedding) and the
//! `WEB_FONTS` fields built from it. Shared by `build.rs` and the crate's tests (`#[path]`-included
//! by both, so it has no dependencies), and by `xtask/src/web_fonts.rs`, which copies the files to [`url`].

/// One line of `web-fonts.txt`: `when | family | style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listed<'a> {
    /// `startup` (fetched before the app starts).
    pub startup: bool,
    /// `demand` (fetched when the interface needs a character of one of the face's scripts).
    /// Neither this nor `startup`: `background`.
    pub demand: bool,
    pub family: &'a str,
    pub style: &'a str,
}

/// Parse `web-fonts.txt`. Blank lines and `#` comments are skipped; line numbers are 1-based.
pub fn parse_list(text: &str) -> Result<Vec<Listed<'_>>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [when, family, style] = fields.as_slice() else {
            return Err(format!("web-fonts.txt line {}: expected `when | family | style`", i + 1));
        };
        let (startup, demand) = match *when {
            "startup" => (true, false),
            "background" => (false, false),
            "demand" => (false, true),
            other => return Err(format!("web-fonts.txt line {}: `{other}` is not startup, background or demand", i + 1)),
        };
        if family.is_empty() || style.is_empty() {
            return Err(format!("web-fonts.txt line {}: empty family or style", i + 1));
        }
        out.push(Listed { startup, demand, family, style });
    }
    Ok(out)
}

/// Standard base64 (RFC 4648, with padding).
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        for k in 0..4 {
            if k <= chunk.len() {
                let six = ((n >> (18 - 6 * k)) & 63) as usize;
                s.push(TABLE.get(six).map_or('=', |&c| char::from(c)));
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The Subresource Integrity value (`sha256-<base64>`) for a 64-digit hex SHA-256; `None` if `hex`
/// isn't one.
pub fn sri(hex: &str) -> Option<String> {
    if hex.len() != 64 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..32).map(|i| hex.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
    Some(format!("sha256-{}", base64(&bytes?)))
}

/// The site-relative URL a face is served at: `fonts/<first 16 hex digits of its SHA-256>/<file
/// name>`. Relative, so the site works under any path; content-addressed, so it can be cached
/// forever.
pub fn url(sha256: &str, file: &str) -> String {
    // Either separator: on Windows a backslash in supplier data would otherwise pick a folder.
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    format!("fonts/{}/{name}", sha256.get(..16).unwrap_or(sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, want) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(input.as_bytes()), want, "{input:?}");
        }
    }

    #[test]
    fn sri_of_the_empty_files_sha256() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(sri(empty).as_deref(), Some("sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="));
        assert_eq!(sri("abc"), None);
        assert_eq!(sri(&"zz".repeat(32)), None);
    }

    #[test]
    fn url_is_relative_and_content_addressed() {
        let sha = "63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e";
        assert_eq!(url(sha, "fonts/noto-sans-arabic/NotoSansArabic.ttf"), "fonts/63111b5b2e074dd4/NotoSansArabic.ttf");
        assert_eq!(url(sha, "fonts\\..\\..\\NotoSansArabic.ttf"), "fonts/63111b5b2e074dd4/NotoSansArabic.ttf");
        assert!(!url(sha, "x.ttf").starts_with('/'));
    }

    #[test]
    fn list_parsing() {
        let l =
            parse_list("# c\n\nstartup | Noto Sans Arabic | Regular\nbackground | Amiri | Bold Italic\ndemand | NanumGothic | Regular\n").unwrap();
        assert_eq!(
            l,
            [
                Listed { startup: true, demand: false, family: "Noto Sans Arabic", style: "Regular" },
                Listed { startup: false, demand: false, family: "Amiri", style: "Bold Italic" },
                Listed { startup: false, demand: true, family: "NanumGothic", style: "Regular" }
            ]
        );
        assert!(parse_list("soon | A | B").unwrap_err().contains("line 1"));
        assert!(parse_list("startup | A").is_err());
        assert!(parse_list("startup |  | B").is_err());
    }
}
