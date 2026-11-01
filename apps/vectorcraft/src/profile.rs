//! Native profile paths. Isolated profiles never migrate preferences from a user's old profile.
//!
//! In order:
//! 1. `VECTORCRAFT_CONFIG_DIR`, when set (tests, agents, custom setups).
//! 2. **Portable mode** (#1366), as in PhotoCraft: a marker file ([`PORTABLE_MARKERS`]) in the
//!    install folder puts the settings in `VectorCraftData` beside it, so a portable copy leaves
//!    nothing in `%APPDATA%`. The install folder is the executable's; on macOS the one holding
//!    `VectorCraft.app` (nothing is written inside a signed bundle), for an AppImage the one holding
//!    the `.AppImage` file (`$APPIMAGE`; the executable runs from a read-only mount). The Windows
//!    portable zip ships with `portable.txt`; the MSI doesn't. The first time `VectorCraftData` is
//!    created, the settings in the user's profile are copied into it, so a portable zip that gains
//!    the marker keeps its users' settings. If the folder can't be written (read-only medium,
//!    Program Files), a warning is logged and the platform folder is used. A portable copy never
//!    reads the old DrawCraft profile, and a run without preferences (`VECTORCRAFT_NO_PREFS`) never
//!    looks for the marker.
//! 3. The platform convention.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Files in the install folder that switch on portable mode (either one; contents are ignored).
pub const PORTABLE_MARKERS: [&str; 2] = ["portable.txt", "VectorCraft.portable"];
/// The settings folder created in the install folder in portable mode.
pub const PORTABLE_DATA_DIR: &str = "VectorCraftData";

pub fn prefs_path(legacy: bool) -> Option<PathBuf> {
    prefs_path_with(std::env::consts::OS, legacy, |key| std::env::var_os(key), portable_dir)
}

/// `portable` is the portable settings folder, if any: only asked for when no override is set and
/// preferences are in use.
fn prefs_path_with(os: &str, legacy: bool, env: impl Fn(&str) -> Option<OsString>, portable: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    if let Some(root) = env("VECTORCRAFT_CONFIG_DIR") {
        // Never fall back to DrawCraft's real profile when the isolated file does not exist.
        return (!legacy && !root.is_empty()).then(|| PathBuf::from(root).join("ui.json"));
    }
    // A run without preferences reads and writes nothing, so it must not create the portable folder.
    if env("VECTORCRAFT_NO_PREFS").is_none()
        && let Some(dir) = portable()
    {
        // A portable copy is self-contained: DrawCraft's old profile is not read into it.
        return (!legacy).then(|| dir.join("ui.json"));
    }
    platform_dir(os, legacy, &env).map(|p| p.join("ui.json"))
}

/// The platform's settings folder of VectorCraft, or of DrawCraft (`legacy`).
fn platform_dir(os: &str, legacy: bool, env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let (name, lower) = if legacy { ("DrawCraft", "drawcraft") } else { ("VectorCraft", "vectorcraft") };
    match os {
        "macos" => env("HOME").map(|p| PathBuf::from(p).join("Library/Application Support").join(name)),
        "windows" => env("APPDATA").map(|p| PathBuf::from(p).join(name)),
        _ => env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| env("HOME").map(|p| PathBuf::from(p).join(".config"))).map(|p| p.join(lower)),
    }
}

/// The portable settings folder of this copy, looked for once per process (it probes the disk, and
/// the settings path is asked for many times).
fn portable_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let os = std::env::consts::OS;
        let env = |key: &str| std::env::var_os(key);
        let install = install_dir(os, &std::env::current_exe().ok()?, &env)?;
        portable_dir_in(&install, platform_dir(os, false, &env).as_deref()).unwrap_or_else(|warning| {
            // Only the log: `eprintln!` panics when standard error is a closed pipe, and the logger
            // keeps early records for the log file.
            log::warn!("{warning}");
            None
        })
    })
    .clone()
}

/// The folder the user sees the app in, where the portable marker is looked for: the executable's;
/// on macOS the one holding `VectorCraft.app` when the executable sits in its `Contents/MacOS` (a
/// signed bundle is never written into); on Linux the one holding the `.AppImage` file when
/// `$APPIMAGE` names it (the executable itself runs from a read-only mount).
fn install_dir(os: &str, exe: &Path, env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;
    let dir = match os {
        "macos" => app_bundle_parent(exe_dir).unwrap_or(exe_dir).to_path_buf(),
        "windows" => exe_dir.to_path_buf(),
        _ => {
            let appimage = env("APPIMAGE").map(PathBuf::from);
            let beside = appimage.as_deref().and_then(Path::parent).filter(|p| !p.as_os_str().is_empty());
            beside.unwrap_or(exe_dir).to_path_buf()
        }
    };
    Some(dir)
}

/// The folder holding `X.app` when `exe_dir` is `X.app/Contents/MacOS`.
fn app_bundle_parent(exe_dir: &Path) -> Option<&Path> {
    let contents = exe_dir.parent().filter(|_| exe_dir.file_name() == Some("MacOS".as_ref()))?;
    let bundle = contents.parent().filter(|_| contents.file_name() == Some("Contents".as_ref()))?;
    bundle.extension().filter(|ext| *ext == "app").and(bundle.parent())
}

/// `<install_dir>/VectorCraftData` when a portable marker sits in `install_dir`, created if missing;
/// `None` without a marker, and the warning to log when there is one but that folder can't be
/// written. The first time the folder is created, the settings in `previous` (the user's profile
/// folder, when it exists) are copied into it, so a portable zip that gains the marker keeps its
/// users' settings. `previous` is never changed; a copy that fails halfway is logged and left as it
/// is, and a later start finds the folder and copies nothing.
fn portable_dir_in(install_dir: &Path, previous: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let Some(marker) = PORTABLE_MARKERS.iter().map(|m| install_dir.join(m)).find(|p| p.is_file()) else { return Ok(None) };
    let data = install_dir.join(PORTABLE_DATA_DIR);
    let first_start = !data.exists();
    if let Err(e) = ensure_writable(&data) {
        return Err(format!(
            "portable mode ({} found) but {} isn't writable ({e}); settings are kept in the user's settings folder instead",
            marker.display(),
            data.display()
        ));
    }
    if first_start && let Some(previous) = previous.filter(|p| p.is_dir()) {
        match copy_dir(previous, &data) {
            Ok(()) => log::info!("portable mode: copied the settings from {} to {}", previous.display(), data.display()),
            Err(e) => log::warn!("portable mode: the settings in {} were only partly copied to {}: {e}", previous.display(), data.display()),
        }
    }
    Ok(Some(data))
}

/// Create `dir` and prove a file can be written in it (a folder can exist yet be read-only).
fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    // A leftover probe is harmless; don't give up portable mode over it.
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Copy the contents of `from` into `to` (which exists), going on past entries that can't be
/// copied; the error says how many were skipped and why the first one was.
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    let mut skipped = 0usize;
    let mut first: Option<String> = None;
    copy_dir_into(from, to, &mut |why| {
        skipped += 1;
        first.get_or_insert(why);
    });
    match first {
        None => Ok(()),
        Some(why) => Err(format!("{skipped} skipped, the first because {why}")),
    }
}

fn copy_dir_into(from: &Path, to: &Path, skip: &mut impl FnMut(String)) {
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) => return skip(format!("{}: {e}", from.display())),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                skip(format!("{}: {e}", from.display()));
                continue;
            }
        };
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        // A symbolic link is copied as what it points to, or skipped when that is a folder.
        let copied = match entry.file_type() {
            Ok(kind) if kind.is_dir() => std::fs::create_dir_all(&dst).map(|()| copy_dir_into(&src, &dst, skip)),
            Ok(_) => std::fs::copy(&src, &dst).map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = copied {
            skip(format!("{}: {e}", src.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn environment(key: &str) -> Option<OsString> {
        match key {
            "HOME" => Some("/user".into()),
            "APPDATA" => Some("/roaming".into()),
            "XDG_CONFIG_HOME" => Some("/xdg-config".into()),
            _ => None,
        }
    }

    /// No portable marker in the install folder.
    fn installed() -> Option<PathBuf> {
        None
    }

    #[test]
    fn isolated_profile_has_no_legacy_fallback_on_any_platform() {
        for os in ["macos", "windows", "linux"] {
            let env = |key: &str| {
                if key == "VECTORCRAFT_CONFIG_DIR" { Some("/isolated".into()) } else { environment(key) }
            };
            assert_eq!(prefs_path_with(os, false, env, installed), Some("/isolated/ui.json".into()));
            assert_eq!(prefs_path_with(os, true, env, installed), None);
        }
    }

    #[test]
    fn empty_override_does_not_fall_back_to_user_files() {
        let env = |key: &str| if key == "VECTORCRAFT_CONFIG_DIR" { Some(OsString::new()) } else { environment(key) };
        assert_eq!(prefs_path_with("macos", false, env, installed), None);
        assert_eq!(prefs_path_with("macos", true, env, installed), None);
    }

    #[test]
    fn unset_override_keeps_platform_paths_and_legacy_migration() {
        assert_eq!(prefs_path_with("macos", false, environment, installed), Some("/user/Library/Application Support/VectorCraft/ui.json".into()));
        assert_eq!(prefs_path_with("macos", true, environment, installed), Some("/user/Library/Application Support/DrawCraft/ui.json".into()));
        assert_eq!(prefs_path_with("windows", false, environment, installed), Some("/roaming/VectorCraft/ui.json".into()));
        assert_eq!(prefs_path_with("linux", false, environment, installed), Some("/xdg-config/vectorcraft/ui.json".into()));
        assert_eq!(
            prefs_path_with("linux", true, |key| if key == "HOME" { environment(key) } else { None }, installed),
            Some("/user/.config/drawcraft/ui.json".into())
        );
    }

    #[test]
    fn portable_mode_keeps_the_settings_beside_the_executable_and_skips_the_drawcraft_profile() {
        let portable = || Some(PathBuf::from("/usb/VectorCraft/VectorCraftData"));
        for os in ["macos", "windows", "linux"] {
            assert_eq!(prefs_path_with(os, false, environment, portable), Some("/usb/VectorCraft/VectorCraftData/ui.json".into()), "{os}");
            // A portable copy is self-contained: the one-time read of DrawCraft's old settings is skipped.
            assert_eq!(prefs_path_with(os, true, environment, portable), None, "{os}");
        }
    }

    #[test]
    fn the_config_dir_override_wins_over_portable_mode_without_probing_it() {
        let env = |key: &str| if key == "VECTORCRAFT_CONFIG_DIR" { Some("/isolated".into()) } else { environment(key) };
        let portable = || -> Option<PathBuf> { panic!("the portable folder is not looked for under VECTORCRAFT_CONFIG_DIR") };
        assert_eq!(prefs_path_with("windows", false, env, portable), Some("/isolated/ui.json".into()));
        assert_eq!(prefs_path_with("windows", true, env, portable), None);
    }

    #[test]
    fn a_run_without_preferences_never_looks_for_the_portable_folder() {
        let env = |key: &str| if key == "VECTORCRAFT_NO_PREFS" { Some("1".into()) } else { environment(key) };
        let portable = || -> Option<PathBuf> { panic!("the portable folder is not looked for under VECTORCRAFT_NO_PREFS") };
        assert_eq!(prefs_path_with("windows", false, env, portable), Some("/roaming/VectorCraft/ui.json".into()));
        assert_eq!(prefs_path_with("windows", true, env, portable), Some("/roaming/DrawCraft/ui.json".into()));
    }

    #[test]
    fn on_macos_the_marker_is_looked_for_beside_the_app_bundle() {
        let bundled = Path::new("/Apps/VectorCraft.app/Contents/MacOS/vectorcraft");
        assert_eq!(install_dir("macos", bundled, &environment), Some("/Apps".into()));
        // A bare executable (the CLI zip) and other platforms: the executable's own folder.
        assert_eq!(install_dir("macos", Path::new("/Apps/vectorcraft-cli"), &environment), Some("/Apps".into()));
        assert_eq!(install_dir("windows", bundled, &environment), Some("/Apps/VectorCraft.app/Contents/MacOS".into()));
        assert_eq!(install_dir("linux", bundled, &environment), Some("/Apps/VectorCraft.app/Contents/MacOS".into()));
    }

    #[test]
    fn an_appimage_looks_for_the_marker_beside_the_appimage_file() {
        let mounted = Path::new("/tmp/.mount_vc/usr/bin/vectorcraft");
        let appimage = |key: &str| if key == "APPIMAGE" { Some("/home/user/Apps/VectorCraft.AppImage".into()) } else { environment(key) };
        assert_eq!(install_dir("linux", mounted, &appimage), Some("/home/user/Apps".into()));
        assert_eq!(install_dir("linux", mounted, &environment), Some("/tmp/.mount_vc/usr/bin".into()));
        let empty = |key: &str| if key == "APPIMAGE" { Some(OsString::new()) } else { environment(key) };
        assert_eq!(install_dir("linux", mounted, &empty), Some("/tmp/.mount_vc/usr/bin".into()));
        // The variable means nothing on other platforms.
        assert_eq!(install_dir("windows", mounted, &appimage), Some("/tmp/.mount_vc/usr/bin".into()));
        assert_eq!(install_dir("macos", mounted, &appimage), Some("/tmp/.mount_vc/usr/bin".into()));
    }

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("vectorcraft-profile-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn either_marker_beside_the_executable_makes_a_writable_data_folder() {
        for marker in PORTABLE_MARKERS {
            let exe = temp("portable");
            std::fs::write(exe.join(marker), b"").unwrap();
            let dir = portable_dir_in(&exe, None).unwrap().unwrap();
            assert_eq!(dir, exe.join(PORTABLE_DATA_DIR), "{marker}");
            assert!(dir.is_dir(), "the data folder is created");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the write probe is removed");
            let _ = std::fs::remove_dir_all(&exe);
        }
    }

    #[test]
    fn a_fake_app_bundle_keeps_the_marker_and_the_data_outside_the_bundle() {
        let folder = temp("bundle");
        let exe = folder.join("VectorCraft.app/Contents/MacOS/vectorcraft");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(folder.join("portable.txt"), b"").unwrap();
        let install = install_dir("macos", &exe, &environment).unwrap();
        assert_eq!(portable_dir_in(&install, None), Ok(Some(folder.join(PORTABLE_DATA_DIR))));
        assert!(!exe.parent().unwrap().join(PORTABLE_DATA_DIR).exists(), "nothing is written inside the bundle");
        // A marker dropped inside the bundle is not looked at.
        std::fs::remove_file(folder.join("portable.txt")).unwrap();
        std::fs::write(exe.parent().unwrap().join("portable.txt"), b"").unwrap();
        let _ = std::fs::remove_dir_all(folder.join(PORTABLE_DATA_DIR));
        assert_eq!(portable_dir_in(&install, None), Ok(None));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn without_a_marker_file_there_is_no_portable_folder() {
        let exe = temp("installed");
        assert_eq!(portable_dir_in(&exe, None), Ok(None));
        // A folder named like the marker is not a marker.
        std::fs::create_dir_all(exe.join("portable.txt")).unwrap();
        assert_eq!(portable_dir_in(&exe, None), Ok(None));
        assert!(!exe.join(PORTABLE_DATA_DIR).exists(), "no data folder without a marker");
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn an_unwritable_portable_folder_is_reported_instead_of_used() {
        let exe = temp("unwritable");
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        // A file where the data folder should go: it can't be created, on every platform.
        std::fs::write(exe.join(PORTABLE_DATA_DIR), b"in the way").unwrap();
        let warning = portable_dir_in(&exe, None).unwrap_err();
        assert!(warning.contains("portable") && warning.contains(PORTABLE_DATA_DIR), "{warning}");
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn the_first_start_copies_the_user_profile_into_the_data_folder_once() {
        let exe = temp("upgrade");
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        let previous = temp("previous-profile");
        std::fs::write(previous.join("ui.json"), b"{\"theme\":\"dark\"}").unwrap();
        std::fs::create_dir_all(previous.join("Swatches/Mine")).unwrap();
        std::fs::write(previous.join("Swatches/Mine/warm.ase"), b"ase").unwrap();

        let data = portable_dir_in(&exe, Some(&previous)).unwrap().unwrap();
        assert_eq!(data, exe.join(PORTABLE_DATA_DIR));
        assert_eq!(std::fs::read(data.join("ui.json")).unwrap(), b"{\"theme\":\"dark\"}", "the settings file is copied");
        assert_eq!(std::fs::read(data.join("Swatches/Mine/warm.ase")).unwrap(), b"ase", "nested folders are copied");
        assert_eq!(std::fs::read(previous.join("ui.json")).unwrap(), b"{\"theme\":\"dark\"}", "the source is left as it is");
        assert!(previous.join("Swatches/Mine/warm.ase").is_file(), "the source is left as it is");

        // A second start finds the folder and copies nothing, whatever changed on either side.
        std::fs::write(data.join("ui.json"), b"{\"theme\":\"light\"}").unwrap();
        std::fs::write(previous.join("ui.json"), b"{\"theme\":\"newer\"}").unwrap();
        std::fs::write(previous.join("later.txt"), b"").unwrap();
        assert_eq!(portable_dir_in(&exe, Some(&previous)), Ok(Some(data.clone())));
        assert_eq!(std::fs::read(data.join("ui.json")).unwrap(), b"{\"theme\":\"light\"}", "the portable settings are not overwritten");
        assert!(!data.join("later.txt").exists(), "nothing is copied after the first start");

        let _ = std::fs::remove_dir_all(&exe);
        let _ = std::fs::remove_dir_all(&previous);
    }

    #[test]
    fn without_a_user_profile_the_data_folder_starts_empty() {
        let exe = temp("fresh");
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        let missing = exe.join("no-such-profile");
        let data = portable_dir_in(&exe, Some(&missing)).unwrap().unwrap();
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn a_copy_that_fails_halfway_reports_what_it_skipped_and_keeps_the_rest() {
        let from = temp("copy-from");
        let to = temp("copy-to");
        std::fs::write(from.join("kept.txt"), b"kept").unwrap();
        std::fs::create_dir_all(from.join("blocked")).unwrap();
        std::fs::write(from.join("blocked/inner.txt"), b"").unwrap();
        // A file where the sub-folder should go: that entry can't be copied, the others can.
        std::fs::write(to.join("blocked"), b"in the way").unwrap();
        let error = copy_dir(&from, &to).unwrap_err();
        assert!(error.starts_with("1 skipped"), "{error}");
        assert!(error.contains("blocked"), "{error}");
        assert_eq!(std::fs::read(to.join("kept.txt")).unwrap(), b"kept");
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }
}
