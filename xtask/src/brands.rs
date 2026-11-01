//! `cargo xtask brands`: no other vendor's product or company names in what users and agents read.
//!
//! VectorCraft names its own features; where a comparison is needed, text says "the reference
//! app". Scanned:
//! * string literals in Rust sources (command labels and params docs, `UI_COMMANDS`, menus, panels,
//!   MCP tool definitions…). Comments and test code (`tests*.rs`, `*_tests.rs`, `tests/` and
//!   `benches/` folders, items under `#[cfg(test)]`) are skipped;
//! * the `description` of every `Cargo.toml`;
//! * the cursor manifest `crates/tools/src/cursors.toml` (designers read it);
//! * every text file under `packaging/`, and under `docs/` except Markdown pages: Markdown docs
//!   (like `README.md` and `ROADMAP.md`) may name the reference app and compare against it.
//!
//! A line that has to keep an old name (an alias that old files or preferences still use) or a
//! name another project defines (a shared MIME type) says `brand-ok` in a comment on that line or,
//! in a file listed in `MARK_ABOVE`, on a `#` comment line just above it.

use std::path::Path;

/// Vendor product and company names, lowercase, matched case-insensitively as whole words.
const DENY: &[&str] = &[
    "adobe",
    "illustrator",
    "photoshop",
    "acrobat",
    "indesign",
    "lightroom",
    "dreamweaver",
    "creative cloud",
    "typekit",
    "kuler",
    "behance",
    "myriad",
    "pantone",
    "aicb",
];

/// Marks a line that keeps a legacy name on purpose.
const ALLOW: &str = "brand-ok";

/// This file lists the names it rejects.
const SELF: &str = "xtask/src/brands.rs";

/// The cursor manifest, which designers read.
const CURSOR_MANIFEST: &str = "crates/tools/src/cursors.toml";

/// Packaging files whose comments take whole lines (a desktop entry has no end-of-line comments):
/// in these, a `#` line marked `brand-ok` also covers the line just below it.
const MARK_ABOVE: &[&str] = &["packaging/linux/ai.storyteller.vectorcraft.desktop"];

/// A denied name found in a file.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    /// 1-based.
    pub line: usize,
    pub term: &'static str,
    /// The literal or line it was found in.
    pub context: String,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan(root, &files);
    if hits.is_empty() {
        println!("brands: no vendor names in user-visible text");
        return Ok(());
    }
    let list: Vec<String> = hits.iter().map(|h| format!("{}:{}: \"{}\" in {}", h.path, h.line, h.term, short(&h.context))).collect();
    Err(format!(
        "{} vendor name(s) in user-visible text (say \"the reference app\" or name the feature; mark a required legacy alias `{ALLOW}`):\n  {}",
        hits.len(),
        list.join("\n  ")
    ))
}

/// Every hit in `files` (paths relative to `root`).
pub fn scan(root: &Path, files: &[String]) -> Vec<Hit> {
    let mut hits = vec![];
    for path in files {
        let Some(kind) = kind_of(path) else { continue };
        // Binary files (images under docs/) are not text and have nothing to read.
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        let found = match kind {
            Kind::Rust => scan_rust(&text),
            Kind::Manifest => scan_lines(&text, |l| l.trim_start().starts_with("description"), false),
            Kind::Text => scan_lines(&text, |_| true, MARK_ABOVE.contains(&path.as_str())),
        };
        hits.extend(found.into_iter().map(|(line, term, context)| Hit { path: path.clone(), line, term, context }));
    }
    hits
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Rust,
    Manifest,
    Text,
}

fn kind_of(path: &str) -> Option<Kind> {
    if (path.starts_with("docs/") && !path.ends_with(".md")) || path.starts_with("packaging/") || path == CURSOR_MANIFEST {
        Some(Kind::Text)
    } else if path == "Cargo.toml" || path.ends_with("/Cargo.toml") {
        Some(Kind::Manifest)
    } else if path.ends_with(".rs") && ["crates/", "apps/", "xtask/"].iter().any(|d| path.starts_with(d)) && path != SELF && !is_test_path(path) {
        Some(Kind::Rust)
    } else {
        None
    }
}

fn is_test_path(path: &str) -> bool {
    let mut parts = path.split('/').rev();
    let file = parts.next().unwrap_or_default();
    file == "tests.rs" || file.starts_with("tests_") || file.ends_with("_tests.rs") || parts.any(|d| d == "tests" || d == "benches")
}

/// (line, term, line text) for each denied name on the lines `keep` selects, but those marked
/// `brand-ok` on the line or, with `mark_above`, on a `#` comment line just above.
fn scan_lines(text: &str, keep: impl Fn(&str) -> bool, mark_above: bool) -> Vec<(usize, &'static str, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let marked_above =
        |i: usize| mark_above && i.checked_sub(1).and_then(|j| lines.get(j)).is_some_and(|l| l.trim_start().starts_with('#') && l.contains(ALLOW));
    lines
        .iter()
        .enumerate()
        .filter(|&(i, l)| keep(l) && !l.contains(ALLOW) && !marked_above(i))
        .flat_map(|(i, l)| terms_in(l, DENY).into_iter().map(move |t| (i + 1, t, l.trim().to_string())))
        .collect()
}

/// (first line, term, literal) for each denied name in a Rust string literal outside comments and
/// test code.
fn scan_rust(src: &str) -> Vec<(usize, &'static str, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = vec![];
    for lit in rust_literals(src) {
        if lines.get(lit.first - 1..lit.last.min(lines.len())).is_some_and(|ls| ls.iter().any(|l| l.contains(ALLOW))) {
            continue;
        }
        for t in terms_in(&lit.text, DENY) {
            out.push((lit.first, t, lit.text.clone()));
        }
    }
    out
}

/// The `terms` (lowercase) that occur in `text` as whole words, ignoring case.
fn terms_in(text: &str, terms: &[&'static str]) -> Vec<&'static str> {
    let lower = text.to_ascii_lowercase();
    let b = lower.as_bytes();
    let word = |c: Option<&u8>| c.is_some_and(u8::is_ascii_alphanumeric);
    terms
        .iter()
        .copied()
        .filter(|t| lower.match_indices(t).any(|(i, _)| !word(i.checked_sub(1).and_then(|j| b.get(j))) && !word(b.get(i + t.len()))))
        .collect()
}

fn short(s: &str) -> String {
    let s = s.replace('\n', " ");
    match s.char_indices().nth(100) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// A Rust string literal: its first and last line (1-based) and its text (escapes as spaces).
#[derive(Debug)]
struct Lit {
    first: usize,
    last: usize,
    text: String,
}

/// String literals of a Rust source, skipping comments and items under `#[cfg(test)]`.
fn rust_literals(src: &str) -> Vec<Lit> {
    let b = src.as_bytes();
    let mut out = vec![];
    let (mut i, mut line) = (0usize, 1usize);
    let mut depth = 0usize;
    // `#[cfg(test)]` seen, its item not opened yet; brace depth the test item was opened at.
    let (mut test_attr, mut test_depth) = (false, None::<usize>);
    while i < b.len() {
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut nest = 0usize;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        nest += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        nest -= 1;
                        i += 2;
                        if nest == 0 {
                            break;
                        }
                    } else {
                        line += usize::from(b[i] == b'\n');
                        i += 1;
                    }
                }
            }
            b'"' => {
                let first = line;
                let (text, end) = quoted(b, i + 1, &mut line);
                i = end;
                if test_depth.is_none() {
                    out.push(Lit { first, last: line, text });
                }
            }
            b'\'' => i = after_char_or_lifetime(src, i),
            b'#' if b[i..].starts_with(b"#[cfg(test)]") => {
                test_attr = true;
                i += "#[cfg(test)]".len();
            }
            b'{' => {
                if test_attr && test_depth.is_none() {
                    test_depth = Some(depth);
                }
                test_attr = false;
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if test_depth == Some(depth) {
                    test_depth = None;
                }
                i += 1;
            }
            b';' => {
                test_attr = false;
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let hashes = b[i..].iter().take_while(|&&h| h == b'#').count();
                // Raw strings (r"…", r#"…"#, br…, cr…); b"…" and c"…" are read as plain strings next.
                if matches!(&src[start..i], "r" | "br" | "cr") && b.get(i + hashes) == Some(&b'"') {
                    let first = line;
                    let body = i + hashes + 1;
                    let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
                    let end = (body..b.len()).find(|&j| b[j..].starts_with(&close)).unwrap_or(b.len());
                    let text = String::from_utf8_lossy(&b[body..end]).into_owned();
                    line += text.matches('\n').count();
                    i = (end + close.len()).min(b.len());
                    if test_depth.is_none() {
                        out.push(Lit { first, last: line, text });
                    }
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Read a `"…"` literal whose body starts at `i`: (text with escapes as spaces, index after the
/// closing quote).
fn quoted(b: &[u8], mut i: usize, line: &mut usize) -> (String, usize) {
    let mut text = Vec::new();
    while i < b.len() && b[i] != b'"' {
        if b[i] == b'\\' {
            match b.get(i + 1) {
                Some(&c @ (b'"' | b'\\' | b'\'')) => text.push(c),
                Some(b'\n') => {
                    *line += 1;
                    text.push(b' ');
                }
                _ => text.push(b' '),
            }
            i += 2;
        } else {
            *line += usize::from(b[i] == b'\n');
            text.push(b[i]);
            i += 1;
        }
    }
    (String::from_utf8_lossy(&text).into_owned(), (i + 1).min(b.len()))
}

/// Skip a char literal (`'x'`, `'\n'`, `'\u{…}'`) at `i`, or just the quote of a lifetime/label.
fn after_char_or_lifetime(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    if b.get(i + 1) == Some(&b'\\') {
        let close = (i + 3..b.len()).find(|&j| b[j] == b'\'').unwrap_or(b.len());
        return (close + 1).min(b.len());
    }
    match src[i + 1..].chars().next() {
        Some(c) if b.get(i + 1 + c.len_utf8()) == Some(&b'\'') => i + 2 + c.len_utf8(),
        _ => i + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_hits(src: &str) -> Vec<(usize, &'static str)> {
        scan_rust(src).into_iter().map(|(l, t, _)| (l, t)).collect()
    }

    #[test]
    fn rust_literals_skip_comments_identifiers_tests_and_marked_lines() {
        let src = r##"// Illustrator in a comment
/// Adobe in a doc comment
/* block Adobe /* nested */ Photoshop */
fn f() { let a = "Made for Illustrator users"; let k = "copyAicb"; let q = '"'; let l: &'static str = r#"raw "Acrobat" text"#; }
const OLD: &str = "Illustrator Defaults"; // brand-ok: legacy alias
fn g() -> &'static str { "line\nAICB" }
#[cfg(test)]
mod tests {
    fn t() { let x = "Adobe"; if true { let y = "Photoshop"; } }
}
#[cfg(test)]
mod other;
fn h() { let s = b"InDesign"; let m = "multi
line Pantone"; }
"##;
        assert_eq!(rust_hits(src), [(4, "illustrator"), (4, "acrobat"), (6, "aicb"), (13, "indesign"), (13, "pantone")]);
        let lits = rust_literals(src);
        let multi = lits.iter().find(|l| l.text.contains("Pantone")).unwrap();
        assert_eq!((multi.first, multi.last), (13, 14));
    }

    #[test]
    fn whole_words_only() {
        assert_eq!(terms_in("Adobe's app", DENY), ["adobe"]);
        assert_eq!(terms_in("an ILLUSTRATOR-style UI", DENY), ["illustrator"]);
        assert!(terms_in("illustrators illustration copyAicb aicbMode", DENY).is_empty());
    }

    #[test]
    fn docs_packaging_and_manifests() {
        let scan = |text: &str, keep: fn(&str) -> bool, mark_above: bool| -> Vec<(usize, &'static str)> {
            scan_lines(text, keep, mark_above).into_iter().map(|(l, t, _)| (l, t)).collect()
        };
        let lines = |text: &str, keep: fn(&str) -> bool| scan(text, keep, false);
        assert_eq!(lines("Keywords=vector;illustrator;\n", |_| true), [(1, "illustrator")]);
        assert_eq!(lines("Like Adobe apps\nnot here: Adobe # brand-ok\n", |_| true), [(1, "adobe")]);
        let desktop = "# shared type (brand-ok)\nMimeType=application/illustrator;\n# a comment\nMimeType=application/illustrator;\n";
        assert_eq!(scan(desktop, |_| true, true), [(4, "illustrator")], "a `#` line marks the line below it only");
        assert_eq!(lines(desktop, |_| true), [(2, "illustrator"), (4, "illustrator")], "only `MARK_ABOVE` files take a mark from the line above");
        assert_eq!(scan("x = 1 # brand-ok\nIllustrator\n", |_| true, true), [(2, "illustrator")], "a code line's comment marks that line only");
        let manifest = "name = \"adobe-thing\"\ndescription = \"Illustrator-style UI\"\n";
        assert_eq!(lines(manifest, |l| l.trim_start().starts_with("description")), [(2, "illustrator")]);
    }

    #[test]
    fn scope() {
        assert_eq!(kind_of("crates/engine/src/cmd/file.rs"), Some(Kind::Rust));
        assert_eq!(kind_of("crates/engine/src/tests_file.rs"), None);
        assert_eq!(kind_of("crates/tools/src/text_tests.rs"), None);
        assert_eq!(kind_of("crates/color/src/cms/tests.rs"), None);
        assert_eq!(kind_of("crates/engine/tests/command_sweep.rs"), None);
        assert_eq!(kind_of(SELF), None);
        assert_eq!(kind_of("crates/ui-egui/Cargo.toml"), Some(Kind::Manifest));
        assert_eq!(kind_of("docs/brand/LICENSE-brand.txt"), Some(Kind::Text));
        assert_eq!(kind_of("packaging/linux/nfpm.yaml"), Some(Kind::Text));
        assert_eq!(kind_of(CURSOR_MANIFEST), Some(Kind::Text));
        for skipped in ["README.md", "ROADMAP.md", "CLAUDE.md", "AGENTS.md", ".github/workflows/release.yml", "ASSETS.md", "docs/mcp.md"] {
            assert_eq!(kind_of(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn injected_strings_fail_the_gate() {
        let dir = std::env::temp_dir().join(format!("vc-brands-{}", std::process::id()));
        let file = |p: &str, text: &str| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        file("crates/x/src/lib.rs", "pub const LABEL: &str = \"Export for Illustrator\"; // a comment: Adobe\n");
        file("docs/x.md", "Works like Photoshop.\n");
        file("docs/x.txt", "Works like Photoshop.\n");
        file("packaging/x.desktop", "Keywords=vector;illustrator;\n");
        let marked = "# shared type (brand-ok)\nMimeType=application/illustrator;\n";
        file(MARK_ABOVE[0], marked);
        file("packaging/x.yaml", marked);
        file("README.md", "Adobe trademark notice\n");
        let files: Vec<String> =
            ["crates/x/src/lib.rs", "docs/x.md", "docs/x.txt", "packaging/x.desktop", MARK_ABOVE[0], "packaging/x.yaml", "README.md"]
                .map(String::from)
                .to_vec();
        let hits = scan(&dir, &files);
        let _ = std::fs::remove_dir_all(&dir);
        let found: Vec<(&str, usize, &str)> = hits.iter().map(|h| (h.path.as_str(), h.line, h.term)).collect();
        assert_eq!(
            found,
            [
                ("crates/x/src/lib.rs", 1, "illustrator"),
                ("docs/x.txt", 1, "photoshop"),
                ("packaging/x.desktop", 1, "illustrator"),
                ("packaging/x.yaml", 2, "illustrator")
            ]
        );
    }

    #[test]
    fn the_tree_is_clean() {
        let root = crate::root();
        // Outside a git checkout there is no file list to check.
        let Ok(files) = crate::repo_files(&root) else { return };
        let hits = scan(&root, &files);
        assert!(hits.is_empty(), "{hits:#?}");
    }
}
