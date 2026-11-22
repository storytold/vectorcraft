//! The optional craft-fonts build input (https://github.com/storytold/craft-fonts,
//! `docs/integration.md`): with `CRAFT_FONTS_DIR=<checkout>` set, every font in its
//! `fonts/manifest.txt` is embedded as `CRAFT_FONTS`; unset, `CRAFT_FONTS` is empty and the app
//! uses its own and the installed fonts. `CRAFT_FONTS_REQUIRED=1` turns a bad checkout into a build
//! error (release builds). Nothing is fetched: the build reads only the local checkout.
//!
//! Web (wasm32) builds embed only the UI font, BIZ UDPGothic Regular (~4.7 MB): the browser has no
//! system fonts, and all four fonts (~24 MB) would push the `.wasm` past static hosts' per-file
//! limits (Cloudflare Pages: 25 MiB).
//!
//! It also writes `WEB_FONTS`: the faces listed in `web-fonts.txt`, which web builds fetch from
//! `fonts/<sha16>/` beside the wasm instead of embedding (`cargo xtask web-fonts` copies them
//! there; `apps/vectorcraft-web/src/fonts.rs` fetches them). A list line the manifest lacks is left
//! out with a warning, never an error, and never costs the desktop its fonts.
//!
//! On native targets it writes `CONDENSED_UI_FONT`: the condensed face document tab titles draw in
//! (`CONDENSED_UI_FACE`), the one row it reads from the opt-in `fonts/latin-manifest.txt`. A
//! checkout without that row leaves it `None` with a warning, never an error.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "build/web_fonts.rs"]
mod web_fonts;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    println!("cargo::rerun-if-changed=web-fonts.txt");
    println!("cargo::rerun-if-changed=build/web_fonts.rs");
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let (mut fonts, mut web, mut condensed) = (String::new(), String::new(), String::from("None"));
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) {
        let dir = from_workspace(dir);
        match craft_fonts(&dir) {
            Ok(entries) => fonts = entries,
            Err(e) if required => println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display()),
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
        // Separately: a bad web list must not cost the desktop build its craft fonts. A listed face
        // the checkout lacks (an older craft-fonts pin) is left out with a warning, never an error,
        // so the apps don't depend on which craft-fonts commit CI pins: that build just fetches
        // fewer fonts.
        match web_font_entries(&dir) {
            Ok((entries, missing)) => {
                web = entries;
                if !missing.is_empty() {
                    let msg = format!("{} not in CRAFT_FONTS_DIR={} (an older craft-fonts?)", missing.join(", "), dir.display());
                    println!("cargo::warning=web build won't fetch: {msg}");
                }
            }
            Err(e) if required => println!("cargo::error=web-fonts.txt: {e}"),
            Err(e) => println!("cargo::warning=web build fetches no craft fonts: web-fonts.txt: {e}"),
        }
        if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a != "wasm32") {
            match condensed_ui_font(&dir) {
                Ok(entry) => condensed = format!("Some({entry})"),
                Err(e) => println!("cargo::warning=document tabs draw in the interface face: CRAFT_FONTS_DIR={}: {e}", dir.display()),
            }
        }
    }
    let src = format!(
        "pub static CRAFT_FONTS: &[CraftFont] = &[\n{fonts}];\npub static WEB_FONTS: &[WebFont] = &[\n{web}];\npub static CONDENSED_UI_FONT: Option<CraftFont> = {condensed};\n"
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    // Unchanged output keeps its mtime. Rewritten just before this crate compiles, it can read as
    // newer than that compile where cargo's recorded times lose their sub-second part (ecryptfs
    // home directories), and the next build would compile this crate and everything above it again.
    if std::fs::read_to_string(&out).is_ok_and(|old| old == src) {
        return;
    }
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// A relative `CRAFT_FONTS_DIR` is taken from the workspace root (where `cargo` is usually run),
/// not from this crate's directory, which is where build scripts run.
fn from_workspace(dir: PathBuf) -> PathBuf {
    if dir.is_absolute() {
        return dir;
    }
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    crate_dir.join("../..").join(dir)
}

/// The one craft-fonts face web builds embed (family, style).
const WEB_FONT: (&str, &str) = ("BIZ UDPGothic", "Regular");

/// One `CraftFont { .. }` initialiser per manifest line.
fn craft_fonts(dir: &Path) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let root = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if wasm && !(*family == WEB_FONT.0 && *style == WEB_FONT.1) {
            continue;
        }
        let _ = writeln!(out, "    {},", craft_font(dir, &root, family, style, file, scripts)?);
    }
    Ok(out)
}

/// The `CraftFont { .. }` initialiser of one manifest row of the checkout `dir` (canonical `root`).
fn craft_font(dir: &Path, root: &Path, family: &str, style: &str, file: &str, scripts: &str) -> Result<String, String> {
    let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
    // The manifest is supplier data: embed only regular files inside the checkout.
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("{file}: not a file inside the font checkout"));
    }
    println!("cargo::rerun-if-changed={}", path.display());
    let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
    Ok(format!(
        "CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }}",
        scripts.join(", "),
        path.display().to_string(),
    ))
}

/// The condensed craft-fonts face (family, style) document tab titles draw in, from the opt-in
/// Latin catalog.
const CONDENSED_UI_FACE: (&str, &str) = ("Noto Sans", "Condensed SemiBold");

/// The `CraftFont { .. }` initialiser of [`CONDENSED_UI_FACE`]'s row in `fonts/latin-manifest.txt`.
fn condensed_ui_font(dir: &Path) -> Result<String, String> {
    let manifest = dir.join("fonts/latin-manifest.txt");
    // A checkout without the catalog is watched through its folder, so the build runs again once
    // the catalog appears.
    let watched = if manifest.is_file() { manifest.clone() } else { dir.join("fonts") };
    println!("cargo::rerun-if-changed={}", watched.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e} (an older craft-fonts?)", manifest.display()))?;
    let root = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let row = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(" | ").map(str::trim).collect::<Vec<_>>())
        .find(|f| f.first() == Some(&CONDENSED_UI_FACE.0) && f.get(1) == Some(&CONDENSED_UI_FACE.1));
    let Some([family, style, file, scripts, ..]) = row.as_deref() else {
        return Err(format!("{} {} not in {} (an older craft-fonts?)", CONDENSED_UI_FACE.0, CONDENSED_UI_FACE.1, manifest.display()));
    };
    craft_font(dir, &root, family, style, file, scripts)
}

/// One `WebFont { .. }` initialiser per `web-fonts.txt` line, from its craft-fonts manifest row, and
/// the listed faces the manifest lacks (an older craft-fonts checkout), which are left out.
fn web_font_entries(dir: &Path) -> Result<(String, Vec<String>), String> {
    let list_path = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("web-fonts.txt");
    let list = std::fs::read_to_string(&list_path).map_err(|e| format!("{}: {e}", list_path.display()))?;
    let manifest_path = dir.join("fonts/manifest.txt");
    // WEB_FONTS follows the manifest (URLs and hashes), on every target.
    println!("cargo::rerun-if-changed={}", manifest_path.display());
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let rows: Vec<Vec<&str>> =
        manifest.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split(" | ").map(str::trim).collect()).collect();
    let (mut out, mut missing) = (String::new(), Vec::new());
    for e in web_fonts::parse_list(&list)? {
        let Some(row) = rows.iter().find(|r| r.first() == Some(&e.family) && r.get(1) == Some(&e.style)) else {
            missing.push(format!("{} {}", e.family, e.style));
            continue;
        };
        let (Some(file), Some(scripts), Some(sha)) = (row.get(2), row.get(3), row.get(6)) else {
            return Err(format!("malformed manifest row for {} {}", e.family, e.style));
        };
        let integrity = web_fonts::sri(sha).ok_or_else(|| format!("bad sha256 for {} {}", e.family, e.style))?;
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    WebFont {{ family: {:?}, style: {:?}, startup: {}, demand: {}, scripts: &[{}], url: {:?}, integrity: {:?} }},",
            e.family,
            e.style,
            e.startup,
            e.demand,
            scripts.join(", "),
            web_fonts::url(sha, file),
            integrity,
        );
    }
    Ok((out, missing))
}
