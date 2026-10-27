//! `cargo xtask bundle`: build the release app and wrap it as `dist/VectorCraft.app` (macOS), with the
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
    let app = root.join("dist/VectorCraft.app/Contents");
    let _ = std::fs::remove_dir_all(root.join("dist/VectorCraft.app"));
    std::fs::create_dir_all(app.join("MacOS")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(app.join("Resources")).map_err(|e| e.to_string())?;
    std::fs::copy(target.join("release/vectorcraft"), app.join("MacOS/VectorCraft")).map_err(|e| format!("copy app: {e}"))?;
    std::fs::copy(target.join("release/vectorcraft-cli"), app.join("MacOS/vectorcraft-cli")).map_err(|e| format!("copy cli: {e}"))?;
    std::fs::copy(root.join("assets/app-icon/vectorcraft.icns"), app.join("Resources/VectorCraft.icns")).map_err(|e| format!("copy icon: {e}"))?;
    let sha = std::env::var("VECTORCRAFT_BUILD_SHA").unwrap_or_else(|_| "unknown".into());
    std::fs::write(app.join("Info.plist"), info_plist(env!("CARGO_PKG_VERSION"), &sha)).map_err(|e| e.to_string())?;
    println!("built {}", root.join("dist/VectorCraft.app").display());
    Ok(())
}

/// The release `Info.plist` template, which `packaging/macos/package.sh` fills in the same way: the
/// development bundle declares the same document types (Finder opens them with the app).
const INFO_PLIST: &str = include_str!("../../packaging/macos/Info.plist.in");

/// The bundle's `Info.plist`: [`INFO_PLIST`] for `version` and the build commit `sha`.
fn info_plist(version: &str, sha: &str) -> String {
    let short = version.split('-').next().unwrap_or(version);
    INFO_PLIST.replace("@VERSION@", version).replace("@SHORT_VERSION@", short).replace("@BUILD_SHA@", sha)
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
    fn packaging_file_associations_reference_installed_executable() {
        let wxs = std::fs::read_to_string(crate::root().join("packaging/windows/vectorcraft.wxs")).unwrap();
        for (extension, prog_id) in [("vectorcraft", "VectorCraft.vectorcraft")] {
            let prog_id_start = wxs.find(&format!(r#"<ProgId Id="{prog_id}""#)).unwrap();
            let prog_id_end = wxs[prog_id_start..].find("</ProgId>").unwrap() + prog_id_start;
            let prog_id = &wxs[prog_id_start..prog_id_end];

            let extension_start = prog_id.find(&format!(r#"<Extension Id="{extension}""#)).unwrap();
            let extension_end = prog_id[extension_start..].find("</Extension>").unwrap() + extension_start;
            let extension = &prog_id[extension_start..extension_end];

            assert!(
                extension.contains(r#"<Verb Id="open" Command="Open" TargetFile="[#VectorcraftExe]" Argument="&quot;%1&quot;" />"#),
                ".{extension} does not open with the installed executable",
            );
        }
        assert!(wxs.contains(r#"Directory="INSTALLFOLDER""#), "associations are not installed with the app");
    }

    #[test]
    fn dev_bundle_plist_is_the_release_one() {
        let p = info_plist("1.2.3-rc.1", "abc123");
        assert!(p.contains("<string>1.2.3</string>") && p.contains("<string>1.2.3-rc.1</string>"), "{p}");
        assert!(p.contains("<string>abc123</string>"));
        assert!(!p.contains('@'), "every placeholder filled");
        assert!(p.contains("<key>CFBundleDocumentTypes</key>") && p.contains("<string>pdf</string>"), "Finder opens files with it");
    }
}
