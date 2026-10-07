//! `cargo xtask bundle`: build the release app and wrap it as `dist/Vector W3K2.app` (macOS), with the
//! committed app icon `assets/app-icon/vectorcraft.icns` (regenerate it with `packaging/icons.sh`).

use std::path::Path;
use std::process::Command;

pub fn run(root: &Path) -> Result<(), String> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["build", "--release", "-p", "vectorcraft", "-p", "vectorcraft-cli"])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("release build failed".into());
    }
    let target = std::env::var("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("target"));
    let app = root.join("dist/Vector W3K2.app/Contents");
    let _ = std::fs::remove_dir_all(root.join("dist/Vector W3K2.app"));
    std::fs::create_dir_all(app.join("MacOS")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(app.join("Resources")).map_err(|e| e.to_string())?;
    std::fs::copy(target.join("release/vectorcraft"), app.join("MacOS/VectorW3K2")).map_err(|e| format!("copy app: {e}"))?;
    std::fs::copy(target.join("release/vectorcraft-cli"), app.join("MacOS/vectorcraft-cli")).map_err(|e| format!("copy cli: {e}"))?;
    std::fs::copy(root.join("assets/app-icon/vectorcraft.icns"), app.join("Resources/VectorW3K2.icns")).map_err(|e| format!("copy icon: {e}"))?;
    std::fs::write(app.join("Info.plist"), info_plist(env!("CARGO_PKG_VERSION"))).map_err(|e| e.to_string())?;
    println!("built {}", root.join("dist/Vector W3K2.app").display());
    Ok(())
}

/// The development bundle's `Info.plist`.
fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Vector W3K2</string>
<key>CFBundleDisplayName</key><string>Vector W3K2</string>
<key>CFBundleIdentifier</key><string>ca.printthat.vectorw3k2</string>
<key>CFBundleExecutable</key><string>VectorW3K2</string>
<key>CFBundleIconFile</key><string>VectorW3K2</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>CFBundleDocumentTypes</key><array>
 <dict><key>CFBundleTypeName</key><string>Vector W3K2 Document</string><key>CFBundleTypeExtensions</key><array><string>vectorcraft</string></array><key>CFBundleTypeRole</key><string>Editor</string></dict>
 <dict><key>CFBundleTypeName</key><string>SVG</string><key>CFBundleTypeExtensions</key><array><string>svg</string></array><key>CFBundleTypeRole</key><string>Editor</string></dict>
 <dict><key>CFBundleTypeName</key><string>PDF Document</string><key>CFBundleTypeExtensions</key><array><string>pdf</string><string>ai</string></array><key>CFBundleTypeRole</key><string>Viewer</string></dict>
</array>
</dict></plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File types and wording a raster image editor's packaging carries (lowercase).
    const RASTER_EDITOR: &[&str] = &["pcraft", "psd", "psb", "qoi", "photo", "rastergraphics", "image editor"];

    fn text_files(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                text_files(&p, out);
            } else if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p.display().to_string(), t.to_ascii_lowercase()));
            }
        }
    }

    #[test]
    fn packaging_describes_a_vector_app() {
        let mut files = vec![];
        text_files(&crate::root().join("packaging"), &mut files);
        assert!(files.len() > 5, "packaging files found");
        for (path, text) in &files {
            for w in RASTER_EDITOR {
                assert!(!text.contains(w), "{path} mentions `{w}`");
            }
        }
        let read = |p: &str| std::fs::read_to_string(crate::root().join(p)).unwrap();
        let plist = read("packaging/macos/Info.plist.in");
        assert!(plist.contains("<array><string>vectorcraft</string><string>drawcraft</string></array>"), "native format extensions");
        assert!(read("packaging/linux/ai.storyteller.vectorcraft.desktop").contains("Categories=Graphics;2DGraphics;VectorGraphics;"));
        assert!(read("packaging/linux/ai.storyteller.vectorcraft.metainfo.xml.in").contains("<category>VectorGraphics</category>"));
    }

    #[test]
    fn dev_bundle_plist_names_types_neutrally() {
        let p = info_plist("1.2.3");
        assert!(p.contains("<key>CFBundleShortVersionString</key><string>1.2.3</string>"));
        assert!(p.contains("<string>Vector W3K2 Document</string>") && p.contains("<string>PDF Document</string>"));
    }
}
