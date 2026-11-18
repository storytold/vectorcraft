//! The shipped code that records a plug-in grant: only the engine's install approval and its load
//! of the plug-in folder's grants file make a `GrantToken` and call `registry::grant` or
//! `registry::install_checked`. A new call site fails this test until it is added to the list. It
//! checks the source, not what native code can do at run time.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `(text, file, how many times)` for every place shipped code may name it.
const EXPECTED: &[(&str, &str, usize)] = &[
    ("GrantToken::after_engine_approval", "crates/engine/src/cmd/plugin.rs", 1),
    ("GrantToken::from_engine_grants_file", "crates/engine/src/store.rs", 2),
    ("registry::grant", "crates/engine/src/store.rs", 2),
    ("registry::install_checked", "crates/engine/src/cmd/plugin.rs", 1),
];

/// The names the audit counts.
const NAMES: [&str; 4] = ["GrantToken::after_engine_approval", "GrantToken::from_engine_grants_file", "registry::grant", "registry::install_checked"];

/// Every `.rs` file under `dir`, without test files and folders.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "tests" | "benches" | "examples" | "target" | "fuzz") {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.starts_with("tests_") && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

/// The lines of `text` that ship: not comments, and nothing from a top-level `#[cfg(test)] mod x {`
/// on.
fn shipped(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let opens_test_module = |i: usize| {
        lines[i] == "#[cfg(test)]"
            && lines[i + 1..].iter().find(|l| !l.trim().is_empty()).is_some_and(|l| l.starts_with("mod ") && l.trim_end().ends_with('{'))
    };
    let end = (0..lines.len()).find(|&i| opens_test_module(i)).unwrap_or(lines.len());
    lines[..end].iter().copied().filter(|l| !l.trim_start().starts_with("//")).collect()
}

/// How many times `name` occurs in `line` as a whole path (`registry::grant`, not `registry::granted`).
fn count(line: &str, name: &str) -> usize {
    line.match_indices(name).filter(|(at, _)| !line[at + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')).count()
}

#[test]
fn grant_token_call_sites_are_limited() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for top in ["crates", "apps", "xtask"] {
        sources(&root.join(top), &mut files);
    }
    assert!(files.len() > 100, "found only {} source files under {}", files.len(), root.display());
    let mut found: BTreeMap<(&str, String), usize> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let rel = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        for line in shipped(&text) {
            for name in NAMES {
                let n = count(line, name);
                if n > 0 {
                    *found.entry((name, rel.clone())).or_default() += n;
                }
            }
        }
    }
    let expected: BTreeMap<(&str, String), usize> = EXPECTED.iter().map(|&(name, file, n)| ((name, file.to_string()), n)).collect();
    assert_eq!(found, expected, "shipped code that makes a grant token or records a grant");
}

/// The audit's own filters: a test module and comments are left out, a longer name isn't counted.
#[test]
fn the_audit_skips_test_code_and_comments() {
    let text = "fn a() { registry::grant(t); }\n// registry::grant(t)\nfn b() { registry::granted(id); }\n\n#[cfg(test)]\n\nmod tests {\n    registry::grant(t);\n}\n";
    let lines = shipped(text);
    assert_eq!(lines.iter().map(|l| count(l, "registry::grant")).sum::<usize>(), 1, "{lines:?}");
}
