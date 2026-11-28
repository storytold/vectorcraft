//! The optional craft-fonts build input (https://github.com/storytold/craft-fonts,
//! `docs/integration.md`): with `CRAFT_FONTS_DIR=<checkout>` set, every font in its
//! `fonts/manifest.txt` is embedded as `CRAFT_FONTS` (the framework's `craft_fonts_build::Embed`);
//! unset, `CRAFT_FONTS` is empty and the app uses its own and the installed fonts. Desktop builds
//! also embed the Noto Sans Condensed SemiBold row of the checkout's `fonts/latin-manifest.txt`
//! (`Embed::tab_face`), which document tab titles draw in; a checkout without it gives a warning.
//! `CRAFT_FONTS_REQUIRED=1` turns a bad checkout into a build error (release builds). Nothing is
//! fetched: the build reads only the local checkout.
//!
//! Web (wasm32) builds embed only the UI font, BIZ UDPGothic Regular (~4.7 MB): the browser has no
//! system fonts, and all four fonts (~24 MB) would push the `.wasm` past static hosts' per-file
//! limits (Cloudflare Pages: 25 MiB).
//!
//! It also writes `WEB_FONTS`: the faces listed in `web-fonts.txt`, which web builds fetch from
//! `fonts/<sha16>/` beside the wasm instead of embedding (`cargo xtask web-fonts` copies them
//! there; `apps/vectorcraft-web/src/fonts.rs` fetches them). A list line the manifest lacks is left
//! out with a warning, never an error, and never costs the desktop its fonts. Web builds also get
//! `SERVED_FONTS`: the listed `startup` and `demand` faces as the interface font stack fetches them
//! (`Embed::serve`, under `fonts/ui/`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use craft_fonts_build::Embed;

#[path = "build/web_fonts.rs"]
mod web_fonts;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    println!("cargo::rerun-if-changed=web-fonts.txt");
    println!("cargo::rerun-if-changed=build/web_fonts.rs");
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let dir = std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty()).map(|d| from_workspace(PathBuf::from(d)));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let wasm = craft_fonts_build::target_is_wasm();
    let fonts = || {
        let embed = Embed::new().required(required);
        match &dir {
            Some(dir) => embed.dir(dir),
            None => embed,
        }
    };
    // Desktop builds also embed the checkout's condensed face for document tab titles.
    let embedded = fonts().filter(move |f| !wasm || (f.family == WEB_FONT.0 && f.style == WEB_FONT.1));
    let embedded = if wasm { embedded } else { embedded.tab_face() };
    if let Err(e) = embedded.write("craft_fonts.rs") {
        println!("cargo::error={e}");
    }
    let mut web = String::new();
    if let Some(dir) = &dir {
        // Separately: a bad web list must not cost the desktop build its craft fonts. A listed face
        // the checkout lacks (an older craft-fonts pin) is left out with a warning, never an error,
        // so the apps don't depend on which craft-fonts commit CI pins: that build just fetches
        // fewer fonts.
        match web_font_entries(dir) {
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
    }
    write_if_changed(&out.join("web_fonts.rs"), &format!("&[\n{web}]\n"));
    if wasm {
        let served = match interface_faces() {
            Ok(faces) => fonts()
                .filter(move |f| faces.iter().any(|(family, style)| f.family == *family && f.style == *style))
                .serve(out.join("served"), "fonts/ui/")
                .write("craft_fonts_served.rs")
                .map(drop)
                .map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        if let Err(e) = served {
            println!("cargo::error={e}");
        }
    }
}

/// Write `src` to `path` unless it already holds it. Unchanged output keeps its mtime: rewritten
/// just before this crate compiles, it can read as newer than that compile where cargo's recorded
/// times lose their sub-second part (ecryptfs home directories), and the next build would compile
/// this crate and everything above it again.
fn write_if_changed(path: &Path, src: &str) {
    if std::fs::read_to_string(path).is_ok_and(|old| old == src) {
        return;
    }
    if let Err(e) = std::fs::write(path, src) {
        println!("cargo::error=writing {}: {e}", path.display());
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

/// The `web-fonts.txt` faces the interface's text needs (`startup` and `demand`), as (family, style).
fn interface_faces() -> Result<Vec<(String, String)>, String> {
    let list = read_list()?;
    Ok(web_fonts::parse_list(&list)?.into_iter().filter(|e| e.startup || e.demand).map(|e| (e.family.to_owned(), e.style.to_owned())).collect())
}

/// `web-fonts.txt`'s text.
fn read_list() -> Result<String, String> {
    let path = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("web-fonts.txt");
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// One `WebFont { .. }` initialiser per `web-fonts.txt` line, from its craft-fonts manifest row, and
/// the listed faces the manifest lacks (an older craft-fonts checkout), which are left out.
fn web_font_entries(dir: &Path) -> Result<(String, Vec<String>), String> {
    let list = read_list()?;
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
