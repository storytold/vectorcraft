//! `cargo xtask bundle`: build the release app and wrap it as `dist/VectorCraft.app` (macOS), with the
//! committed app icon `assets/app-icon/vectorcraft.icns` (regenerate it with `packaging/icons.sh`) and
//! the Quick Look extensions in `Contents/PlugIns` (packaging/macos/quicklook), signed ad hoc on a
//! Mac (an app extension must be signed, sandboxed, to load).

use std::path::Path;
use std::process::Command;

pub fn run(root: &Path) -> Result<(), String> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["build", "--release", "-p", "vectorcraft", "-p", "vectorcraft-cli", "-p", "vectorcraft-quicklook"])
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
    let mut appexes = vec![];
    for x in EXTENSIONS {
        let appex = app.join(format!("PlugIns/{}.appex/Contents", x.name));
        std::fs::create_dir_all(appex.join("MacOS")).map_err(|e| e.to_string())?;
        std::fs::copy(target.join("release").join(x.bin), appex.join("MacOS").join(x.name)).map_err(|e| format!("copy {}: {e}", x.bin))?;
        std::fs::write(appex.join("Info.plist"), extension_plist(x.plist, env!("CARGO_PKG_VERSION"))).map_err(|e| e.to_string())?;
        appexes.push(appex.parent().map(Path::to_path_buf).unwrap_or(appex));
    }
    let bundle = root.join("dist/VectorCraft.app");
    if cfg!(target_os = "macos") {
        // Inside-out, as packaging/macos/package.sh signs a release (there with a real identity).
        let entitlements = root.join("packaging/macos/quicklook/entitlements.plist");
        for appex in &appexes {
            codesign(&[std::ffi::OsStr::new("--entitlements"), entitlements.as_os_str(), appex.as_os_str()])?;
        }
        codesign(&[app.join("MacOS/vectorcraft-cli").as_os_str()])?;
        codesign(&[bundle.as_os_str()])?;
    } else {
        println!("note: not on macOS, so the Quick Look extensions are unsigned (they load only once signed)");
    }
    println!("built {}", bundle.display());
    Ok(())
}

/// An app extension in `Contents/PlugIns`.
struct Extension {
    /// The bundle's name and its executable's.
    name: &'static str,
    /// The cargo binary it is built from.
    bin: &'static str,
    /// Its `Info.plist` template.
    plist: &'static str,
}

/// The Quick Look extensions (crates/quicklook), as packaging/macos/package.sh adds them.
const EXTENSIONS: [Extension; 2] = [
    Extension {
        name: "VectorCraftThumbnail",
        bin: "vectorcraft-ql-thumbnail",
        plist: include_str!("../../packaging/macos/quicklook/Thumbnail-Info.plist.in"),
    },
    Extension {
        name: "VectorCraftPreview",
        bin: "vectorcraft-ql-preview",
        plist: include_str!("../../packaging/macos/quicklook/Preview-Info.plist.in"),
    },
];

/// An extension's `Info.plist`: its template for `version`.
fn extension_plist(template: &str, version: &str) -> String {
    let short = version.split('-').next().unwrap_or(version);
    template.replace("@VERSION@", version).replace("@SHORT_VERSION@", short)
}

/// `codesign` ad hoc (no identity: this Mac only) with `args` before the path to sign.
fn codesign(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let status = Command::new("codesign")
        .args(["--force", "--sign", "-", "--options", "runtime", "--timestamp=none"])
        .args(args)
        .status()
        .map_err(|e| format!("codesign: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("codesign {args:?} failed")) }
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
    fn extensions_declare_the_native_document_type() {
        for x in EXTENSIONS {
            let p = extension_plist(x.plist, "1.2.3-rc.1");
            assert!(!p.contains('@'), "{}: every placeholder filled", x.name);
            assert!(p.contains(&format!("<string>{}</string>", x.name)), "{}: executable named", x.name);
            assert!(p.contains("<array><string>ai.storyteller.vectorcraft.document</string></array>"), "{}", x.name);
            assert!(p.contains("<string>XPC!</string>") && p.contains("<string>ai.storyteller.vectorcraft."), "{}", x.name);
        }
        let point = |x: &Extension| ["thumbnail", "preview"].into_iter().find(|k| x.plist.contains(&format!("com.apple.quicklook.{k}</string>")));
        assert_eq!(EXTENSIONS.iter().map(point).collect::<Vec<_>>(), [Some("thumbnail"), Some("preview")]);
        // The release script adds the same ones.
        let script = std::fs::read_to_string(crate::root().join("packaging/macos/package.sh")).unwrap();
        for x in EXTENSIONS {
            assert!(script.contains(&format!("add_appex {} {}", x.name, x.bin)), "{}", x.name);
        }
        let entitlements = std::fs::read_to_string(crate::root().join("packaging/macos/quicklook/entitlements.plist")).unwrap();
        assert!(entitlements.contains("<key>com.apple.security.app-sandbox</key>\n  <true/>"), "app extensions are sandboxed");
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
