//! `cargo xtask web-fonts`: the Trunk `post_build` hook (`apps/*-web/Trunk.toml`). Copies the
//! craft-fonts faces listed in `crates/text/web-fonts.txt` from `$CRAFT_FONTS_DIR` into the site,
//! each at `fonts/<first 16 hex digits of its SHA-256>/<file>` with its licence beside it: the URLs
//! `crates/text/build.rs` puts in `WEB_FONTS` (both use `crates/text/build/web_fonts.rs`).
//!
//! A no-op without `CRAFT_FONTS_DIR`. A listed face the checkout lacks (an older craft-fonts) is
//! skipped with a warning, as `build.rs` does; a file whose SHA-256 doesn't match the manifest is an
//! error (a corrupt checkout). Rust rather than a shell script, so the hook runs wherever cargo does.

use std::path::{Path, PathBuf};

use crate::sha256;

#[allow(dead_code)] // build.rs uses the rest (sri, base64)
#[path = "../../crates/text/build/web_fonts.rs"]
mod list;

/// The hook: reads `CRAFT_FONTS_DIR` and Trunk's `TRUNK_STAGING_DIR`.
pub fn run(root: &Path) -> Result<(), String> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    run_with(root, var("CRAFT_FONTS_DIR"), var("TRUNK_STAGING_DIR"))
}

fn run_with(root: &Path, craft_fonts: Option<PathBuf>, staging: Option<PathBuf>) -> Result<(), String> {
    let Some(dir) = craft_fonts else { return Ok(()) };
    // Relative like build.rs: from the workspace root, not the directory Trunk runs hooks in.
    let dir = if dir.is_absolute() { dir } else { root.join(dir) };
    let staging = staging.ok_or("TRUNK_STAGING_DIR is unset: run this as the Trunk post_build hook")?;
    let (copied, skipped) = copy(&root.join("crates/text/web-fonts.txt"), &dir, &staging)?;
    if !skipped.is_empty() {
        eprintln!("warning: web build won't fetch {} (not in CRAFT_FONTS_DIR={}; an older craft-fonts?)", skipped.join(", "), dir.display());
    }
    println!("web-fonts: copied {copied} font(s) into {}", staging.join("fonts").display());
    Ok(())
}

/// Copy the faces `list_path` names from the craft-fonts checkout `dir` into `site` (at their
/// `fonts/…` URLs). Returns how many were copied and the listed faces the checkout lacks.
fn copy(list_path: &Path, dir: &Path, site: &Path) -> Result<(usize, Vec<String>), String> {
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let list = read(list_path)?;
    let manifest = read(&dir.join("fonts/manifest.txt"))?;
    let rows: Vec<Vec<&str>> =
        manifest.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split(" | ").map(str::trim).collect()).collect();
    let (mut copied, mut skipped) = (0, Vec::new());
    for e in list::parse_list(&list)? {
        let Some(row) = rows.iter().find(|r| r.first() == Some(&e.family) && r.get(1) == Some(&e.style)) else {
            skipped.push(format!("{} {}", e.family, e.style));
            continue;
        };
        let (Some(file), Some(licence), Some(sha)) = (row.get(2), row.get(5), row.get(6)) else {
            return Err(format!("malformed manifest row for {} {}", e.family, e.style));
        };
        let bytes = std::fs::read(dir.join(file)).map_err(|err| format!("{file}: {err}"))?;
        let got = sha256::hex(&bytes);
        if got != *sha {
            return Err(format!("{file}: sha256 {got}, manifest says {sha}"));
        }
        let dest = site.join(list::url(sha, file));
        let folder = dest.parent().ok_or_else(|| format!("{}: no parent folder", dest.display()))?;
        std::fs::create_dir_all(folder).map_err(|err| format!("{}: {err}", folder.display()))?;
        std::fs::write(&dest, &bytes).map_err(|err| format!("{}: {err}", dest.display()))?;
        let licence_name = Path::new(licence).file_name().ok_or_else(|| format!("{licence}: not a file"))?;
        // The manifest is supplier data: the licence must be a file inside the checkout.
        let root = std::fs::canonicalize(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let src = std::fs::canonicalize(dir.join(licence)).map_err(|err| format!("{licence}: {err}"))?;
        if !src.starts_with(&root) || !src.is_file() {
            return Err(format!("{licence}: not a file inside the font checkout"));
        }
        std::fs::copy(&src, folder.join(licence_name)).map_err(|err| format!("{licence}: {err}"))?;
        copied += 1;
    }
    Ok((copied, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake craft-fonts checkout with one face, and a list naming it plus one it lacks.
    fn fixture(name: &str, manifest_sha: Option<&str>) -> (PathBuf, PathBuf, PathBuf, String) {
        let base = std::env::temp_dir().join(format!("xtask-web-fonts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("craft-fonts");
        std::fs::create_dir_all(dir.join("fonts/a")).unwrap();
        std::fs::write(dir.join("fonts/a/A-Regular.ttf"), b"font bytes").unwrap();
        std::fs::write(dir.join("fonts/a/OFL.txt"), b"licence").unwrap();
        let sha = sha256::hex(b"font bytes");
        let row_sha = manifest_sha.map_or(sha.clone(), str::to_string);
        std::fs::write(
            dir.join("fonts/manifest.txt"),
            format!("# family | style | file | scripts | licence | licence file | sha256 | source\nA Sans | Regular | fonts/a/A-Regular.ttf | Arab | OFL-1.1 | fonts/a/OFL.txt | {row_sha} | https://example.invalid\n"),
        )
        .unwrap();
        let list = base.join("web-fonts.txt");
        std::fs::write(&list, "startup | A Sans | Regular\nbackground | Missing Sans | Bold\n").unwrap();
        (list, dir, base.join("site"), sha)
    }

    #[test]
    fn copies_listed_faces_with_their_licence_to_their_urls() {
        let (list, dir, site, sha) = fixture("copy", None);
        let (copied, skipped) = copy(&list, &dir, &site).unwrap();
        assert_eq!(copied, 1);
        let at = site.join("fonts").join(&sha[..16]);
        assert_eq!(std::fs::read(at.join("A-Regular.ttf")).unwrap(), b"font bytes");
        assert_eq!(std::fs::read(at.join("OFL.txt")).unwrap(), b"licence");
        assert_eq!(format!("fonts/{}/A-Regular.ttf", &sha[..16]), list::url(&sha, "fonts/a/A-Regular.ttf"));
        assert_eq!(skipped, ["Missing Sans Bold"], "a face the checkout lacks is skipped, not an error");
    }

    #[test]
    fn a_file_that_does_not_match_its_manifest_hash_is_an_error() {
        let (list, dir, site, _) = fixture("mismatch", Some(&"0".repeat(64)));
        let e = copy(&list, &dir, &site).unwrap_err();
        assert!(e.contains("A-Regular.ttf") && e.contains("sha256"), "{e}");
    }

    #[test]
    fn a_licence_path_that_leaves_the_checkout_is_refused() {
        let (list, dir, site, _) = fixture("escape", None);
        std::fs::write(dir.parent().unwrap().join("secret.txt"), b"secret").unwrap();
        let m = dir.join("fonts/manifest.txt");
        let text = std::fs::read_to_string(&m).unwrap().replace("fonts/a/OFL.txt", "../secret.txt");
        std::fs::write(&m, text).unwrap();
        let e = copy(&list, &dir, &site).unwrap_err();
        assert!(e.contains("inside the font checkout"), "{e}");
    }

    #[test]
    fn without_craft_fonts_the_hook_does_nothing() {
        let (_, _, site, _) = fixture("noop", None);
        run_with(Path::new("/nonexistent"), None, None).unwrap();
        assert!(!site.exists());
    }
}
