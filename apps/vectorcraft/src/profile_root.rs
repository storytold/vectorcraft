//! App-owned nightly profile fences: a profile every one of VectorCraft's data paths is redirected
//! into ([`crate::prefs_dir::profile::install_data_root`]), and that never overlaps stable's folders, a portable
//! copy or an installation.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::prefs_dir::profile::PORTABLE_DATA_DIR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    Linux,
    Mac,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match std::env::consts::OS {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Mac),
            other => Err(format!("nightly-v1 is not defined for this operating system ({other})")),
        }
    }

    fn ignores_case(self) -> bool {
        self != Self::Linux
    }
}

fn absolute(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{label} must be absolute and contain no parent traversal"));
    }
    Ok(path)
}

fn required_base(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Result<PathBuf, String> {
    let value = env(key).filter(|v| !v.is_empty()).ok_or_else(|| format!("cannot establish the stable {key} boundary"))?;
    absolute(PathBuf::from(value), key)
}

fn xdg_base(env: &impl Fn(&str) -> Option<OsString>, key: &str, fallback: &str) -> Result<PathBuf, String> {
    match env(key).filter(|v| !v.is_empty()) {
        Some(value) => absolute(PathBuf::from(value), key),
        None => Ok(required_base(env, "HOME")?.join(fallback)),
    }
}

/// Resolve existing ancestors before creating anything. A dangling link or an inaccessible ancestor
/// is not a missing directory that may be replaced or followed later.
fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let mut parent = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(parent) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(parent.file_name().ok_or_else(|| "the profile root has no existing ancestor".to_string())?.to_os_string());
                parent = parent.parent().ok_or_else(|| "the profile root has no existing ancestor".to_string())?;
            }
            Err(error) => return Err(format!("cannot inspect the profile boundary: {error}")),
        }
    }
    let mut resolved = std::fs::canonicalize(parent).map_err(|e| format!("cannot resolve the profile boundary: {e}"))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

/// A stable entry point may be a dangling link once its target is uninstalled. Its own name is still
/// the fence, so only the folder it lives in is resolved: a link that cannot be resolved would refuse
/// every launch, a valid private profile included.
fn canonical_future_entry(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or_else(|| "an entry point has no folder".to_string())?;
    let name = path.file_name().ok_or_else(|| "an entry point has no name".to_string())?;
    Ok(canonical_future(parent)?.join(name))
}

fn overlaps(a: &Path, b: &Path, platform: Platform) -> bool {
    let parts = |p: &Path| {
        p.components()
            .map(|c| {
                let value = c.as_os_str().to_string_lossy().into_owned();
                if platform.ignores_case() { value.to_lowercase() } else { value }
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(a), parts(b));
    a.starts_with(&b) || b.starts_with(&a)
}

fn installation_boundary(platform: Platform, exe: &Path) -> Result<PathBuf, String> {
    let parent = exe.parent().ok_or_else(|| "cannot establish the installation boundary".to_string())?;
    if platform == Platform::Mac {
        // A profile in Resources is still inside the versioned install, not beside MacOS.
        if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))) {
            return Ok(bundle.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

/// Every folder a stable VectorCraft keeps user state in, on `platform`: the settings folder each
/// platform convention and the portable layout use, the eframe window state, the logs, the recovery
/// copies, the plug-ins, the caches, the install folders and the entry points. A nightly profile that
/// overlapped any of them would read, rewrite or prune stable's own state.
fn forbidden_roots(platform: Platform, env: &impl Fn(&str) -> Option<OsString>, exe: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = match platform {
        Platform::Windows => {
            // The settings folder (`profile.rs`) and the eframe window state, which eframe puts under
            // `%APPDATA%\ai.storyteller.vectorcraft\data` (the window id).
            let mut roots = ["APPDATA", "LOCALAPPDATA"]
                .into_iter()
                .map(|key| required_base(env, key).map(|p| p.join("VectorCraft")))
                .collect::<Result<Vec<_>, _>>()?;
            roots.push(required_base(env, "APPDATA")?.join("ai.storyteller.vectorcraft"));
            // The MSI's default per-user and per-machine folders (install-policy.wxi). A custom
            // install folder is not known here; the executable's own folder is fenced below.
            roots.push(required_base(env, "LOCALAPPDATA")?.join("Programs").join("ArtCraft").join("VectorCraft"));
            for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(base) = env(key).filter(|v| !v.is_empty()) {
                    roots.push(absolute(PathBuf::from(base), key)?.join("ArtCraft").join("VectorCraft"));
                }
            }
            roots
        }
        Platform::Linux => {
            let mut roots =
                [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache"), ("XDG_STATE_HOME", ".local/state")]
                    .into_iter()
                    .map(|(key, fallback)| xdg_base(env, key, fallback).map(|p| p.join("vectorcraft")))
                    .collect::<Result<Vec<_>, _>>()?;
            // The eframe window state, under the lowercased window id.
            roots.push(xdg_base(env, "XDG_DATA_HOME", ".local/share")?.join("ai.storyteller.vectorcraft"));
            roots.push(required_base(env, "HOME")?.join(".config").join("VectorCraft"));
            // The same folders at their HOME defaults: an XDG override moves this launch's own paths,
            // not stable's, which stays where a launch without the override finds it.
            for (directory, app) in
                [(".config", "vectorcraft"), (".local/share", "vectorcraft"), (".cache", "vectorcraft"), (".local/state", "vectorcraft")]
            {
                roots.push(required_base(env, "HOME")?.join(directory).join(app));
            }
            // The stable Flatpak keeps its data in its own per-app folder.
            roots.push(required_base(env, "HOME")?.join(".var/app/ai.storyteller.vectorcraft"));
            roots
        }
        Platform::Mac => {
            let home = required_base(env, "HOME")?;
            let library = home.join("Library");
            vec![
                library.join("Application Support/VectorCraft"),
                library.join("Application Support/ai.storyteller.vectorcraft"),
                library.join("Caches/VectorCraft"),
                library.join("Caches/ai.storyteller.vectorcraft"),
                library.join("Preferences/VectorCraft"),
                library.join("Preferences/ai.storyteller.vectorcraft.plist"),
                library.join("Saved Application State/ai.storyteller.vectorcraft.savedState"),
                home.join("Applications/VectorCraft.app"),
                PathBuf::from("/Applications/VectorCraft.app"),
            ]
        }
    };
    roots.push(installation_boundary(platform, exe)?);
    let mut fences = roots.into_iter().map(|p| canonical_future(&p)).collect::<Result<Vec<_>, _>>()?;
    if platform == Platform::Linux {
        for entry in ["/usr/bin/vectorcraft", "/usr/local/bin/vectorcraft", "/opt/vectorcraft"] {
            fences.push(canonical_future_entry(Path::new(entry))?);
        }
    }
    Ok(fences)
}

fn private_subdirectory(root: &Path, name: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let chosen = canonical_future(&absolute(root.to_path_buf(), "the nightly profile root")?)?;
    let child = canonical_future(&chosen.join(name))?;
    if child == chosen || !child.starts_with(&chosen) {
        return Err(format!("the nightly {label} must stay inside its selected profile"));
    }
    Ok(child)
}

/// Every existing entry directly in the profile (settings, logs, the user's libraries and so on) must
/// resolve inside it, so a link there cannot point Nightly's writes at another app's data.
fn entries_stay_inside(root: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect the nightly profile: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect the nightly profile: {e}"))?;
        private_subdirectory(root, entry.file_name(), "profile entry")?;
    }
    Ok(())
}

/// Recognise the shared portable data namespace in both the supplied and the resolved path. Up to
/// the chosen folder itself, a marker only counts beside a VectorCraft executable, so an unrelated
/// `portable.txt` higher up (in a user folder or at a drive root) does not refuse the profile.
fn is_portable_layout(path: &Path, platform: Platform) -> bool {
    path.ancestors().enumerate().any(|(depth, ancestor)| {
        crate::prefs_dir::profile::PORTABLE_MARKERS.iter().any(|marker| ancestor.join(marker).is_file())
            && (depth == 0 || ["vectorcraft.exe", "vectorcraft"].iter().any(|exe| ancestor.join(exe).is_file()))
    }) || path.components().any(|component| match component {
        Component::Normal(name) if platform.ignores_case() => name.to_string_lossy().eq_ignore_ascii_case(PORTABLE_DATA_DIR),
        Component::Normal(name) => name == PORTABLE_DATA_DIR,
        _ => false,
    })
}

/// The writable hook is the app's own exclusive probe ([`crate::prefs_dir::profile::ensure_writable`]). Keeping
/// it injected lets the boundary tests exercise the real policy without starting the app.
pub fn validate(
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    validate_for(Platform::current()?, root, env, exe, writable)
}

fn validate_for(
    platform: Platform,
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    absolute(root.to_path_buf(), "the nightly profile root")?;
    if env("VECTORCRAFT_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
        return Err("nightly-v1 cannot be combined with VECTORCRAFT_CONFIG_DIR".to_string());
    }
    // The app turns preferences off whenever the variable is present, empty value included.
    if env("VECTORCRAFT_NO_PREFS").is_some() {
        // A run without preferences keeps nothing: its own profile would stay unused and the app
        // would log to standard error only, so it is never a nightly launch.
        return Err("nightly-v1 cannot inherit VECTORCRAFT_NO_PREFS".to_string());
    }
    let nightly_flatpak = vectorcraft_ui_egui::os_window::NIGHTLY_PACKAGE_ID;
    if platform != Platform::Windows
        && let Some(id) = env("FLATPAK_ID").filter(|value| !value.is_empty())
        && id.to_string_lossy().as_ref() != nightly_flatpak
    {
        // `packaging/linux/flatpak-bundle.sh` builds VectorCraft Nightly as its own Flatpak
        // (`ai.storyteller.vectorcraft.nightly`), whose sandbox is already a private home. Any other
        // instance, the stable app's included, would put this profile inside another app's tree.
        return Err(format!("nightly-v1 can only run inside its own Flatpak instance ({nightly_flatpak}), not {}", id.to_string_lossy()));
    }
    let chosen = canonical_future(root)?;
    if is_portable_layout(root, platform) || is_portable_layout(&chosen, platform) {
        return Err("the nightly root must not use a portable VectorCraft installation or its VectorCraftData settings folder".to_string());
    }
    let exe = absolute(exe.ok_or_else(|| "cannot establish the installation boundary".to_string())?.to_path_buf(), "the executable")?;
    let forbidden = forbidden_roots(platform, &env, &exe)?;
    if forbidden.iter().any(|stable| overlaps(&chosen, stable, platform)) {
        return Err("the nightly root must not overlap stable data or an installation".to_string());
    }
    // Data Recovery can read, adopt, overwrite and sweep entries: never let its alias leave the
    // private profile.
    private_subdirectory(&chosen, "Data Recovery", "recovery folder")?;
    entries_stay_inside(&chosen)?;
    let mut create = std::fs::DirBuilder::new();
    create.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        create.mode(0o700);
    }
    create.create(&chosen).map_err(|e| format!("cannot create the nightly profile: {e}"))?;
    let confirmed = std::fs::canonicalize(&chosen).map_err(|e| format!("cannot resolve the created nightly profile: {e}"))?;
    if confirmed != chosen || forbidden.iter().any(|stable| overlaps(&confirmed, stable, platform)) {
        return Err("the nightly profile changed while it was being validated".to_string());
    }
    private_subdirectory(&confirmed, "Data Recovery", "recovery folder")?;
    entries_stay_inside(&confirmed)?;
    writable(&confirmed).map_err(|e| format!("the nightly profile is not writable: {e}"))?;
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// An owned fixture tree: a home, a roaming/local app-data pair and an install folder, so the
    /// fence can be checked for every platform on any host.
    struct Fixture {
        root: PathBuf,
        env: BTreeMap<String, OsString>,
        exe: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = loop {
                let candidate =
                    std::env::temp_dir().join(format!("vectorcraft-profile-policy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => break std::fs::canonicalize(candidate).unwrap(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create owned fixture: {error}"),
                }
            };
            let home = root.join("home");
            let exe = root.join("install/vectorcraft");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"fixture executable, never run").unwrap();
            let env = [("HOME", home), ("APPDATA", root.join("roaming")), ("LOCALAPPDATA", root.join("local"))]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into_os_string()))
                .collect();
            Self { root, env, exe }
        }

        fn get(&self, key: &str) -> Option<OsString> {
            self.env.get(key).cloned()
        }

        fn home(&self) -> PathBuf {
            PathBuf::from(self.get("HOME").unwrap())
        }

        fn check(&self, platform: Platform, root: &Path) -> Result<PathBuf, String> {
            self.check_with(platform, root, |_| Ok(()))
        }

        fn check_with(&self, platform: Platform, root: &Path, writable: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<PathBuf, String> {
            validate_for(platform, root, |key| self.get(key), Some(&self.exe), writable)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only the directory this fixture created is owned for removal.
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    const PLATFORMS: [Platform; 3] = [Platform::Windows, Platform::Linux, Platform::Mac];

    #[test]
    fn a_private_profile_is_accepted_on_every_platform() {
        let fixture = Fixture::new();
        for platform in PLATFORMS {
            let root = fixture.root.join(format!("nightly profiles/{platform:?}/vectorcraft"));
            let resolved = fixture.check(platform, &root).unwrap();
            assert!(resolved.is_dir(), "{platform:?}");
            assert_eq!(resolved, std::fs::canonicalize(root).unwrap(), "{platform:?}");
            std::fs::remove_dir_all(fixture.root.join("nightly profiles")).unwrap();
        }
    }

    #[test]
    fn stable_settings_folders_are_refused_on_every_platform() {
        let fixture = Fixture::new();
        let stable = [
            (Platform::Windows, fixture.get("APPDATA").map(|v| PathBuf::from(v).join("VectorCraft"))),
            (Platform::Windows, fixture.get("APPDATA").map(|v| PathBuf::from(v).join("ai.storyteller.vectorcraft"))),
            (Platform::Windows, fixture.get("LOCALAPPDATA").map(|v| PathBuf::from(v).join("VectorCraft"))),
            (Platform::Windows, fixture.get("LOCALAPPDATA").map(|v| PathBuf::from(v).join("Programs/ArtCraft/VectorCraft"))),
            (Platform::Linux, Some(fixture.home().join(".config/vectorcraft"))),
            (Platform::Linux, Some(fixture.home().join(".local/share/vectorcraft"))),
            (Platform::Linux, Some(fixture.home().join(".local/share/ai.storyteller.vectorcraft"))),
            (Platform::Linux, Some(fixture.home().join(".cache/vectorcraft"))),
            (Platform::Linux, Some(fixture.home().join(".var/app/ai.storyteller.vectorcraft"))),
            (Platform::Mac, Some(fixture.home().join("Library/Application Support/VectorCraft"))),
            (Platform::Mac, Some(fixture.home().join("Library/Application Support/ai.storyteller.vectorcraft"))),
            (Platform::Mac, Some(fixture.home().join("Library/Preferences/ai.storyteller.vectorcraft.plist"))),
        ];
        for (platform, stable) in stable {
            let stable = stable.unwrap();
            for candidate in [stable.clone(), stable.join("nested nightly")] {
                let probed = Cell::new(false);
                let result = fixture.check_with(platform, &candidate, |_| {
                    probed.set(true);
                    Ok(())
                });
                assert!(result.is_err(), "{platform:?} accepted {candidate:?}");
                assert!(!probed.get(), "a refused root is rejected before the writable probe");
            }
        }
    }

    #[test]
    fn the_install_folder_and_the_entry_points_are_refused() {
        let fixture = Fixture::new();
        for platform in PLATFORMS {
            let install = fixture.exe.parent().unwrap();
            assert!(fixture.check(platform, install).is_err(), "{platform:?}: the install folder");
            assert!(fixture.check(platform, &install.join("NightlyData")).is_err(), "{platform:?}: inside the install");
        }
    }

    #[test]
    fn an_inherited_config_or_no_prefs_override_is_refused() {
        let fixture = Fixture::new();
        let root = fixture.root.join("nightly");
        for (key, value) in [
            ("VECTORCRAFT_CONFIG_DIR", "/elsewhere"),
            ("VECTORCRAFT_NO_PREFS", "1"),
            // The app turns preferences off on presence, so an empty value is a run too.
            ("VECTORCRAFT_NO_PREFS", ""),
            ("FLATPAK_ID", "ai.storyteller.vectorcraft"),
        ] {
            for platform in PLATFORMS {
                let result = validate_for(
                    platform,
                    &root,
                    |name| if name == key { Some(value.into()) } else { fixture.get(name) },
                    Some(&fixture.exe),
                    |_| Ok(()),
                );
                let refused = result.is_err();
                // A Flatpak instance only keeps the stable app's sockets on native Unix packages.
                assert_eq!(refused, !(key == "FLATPAK_ID" && platform == Platform::Windows), "{key} on {platform:?}");
                if refused {
                    assert!(!root.exists(), "a refused launch creates nothing ({key} on {platform:?})");
                } else if root.exists() {
                    // The accepted case validated the root, which creates it: start the next one clean.
                    std::fs::remove_dir_all(&root).expect("remove the accepted launch's root");
                }
            }
        }
    }

    #[test]
    fn only_the_nightly_s_own_flatpak_instance_may_carry_a_profile() {
        let fixture = Fixture::new();
        let root = fixture.root.join("flatpak profile");
        for (id, accepted) in
            [(vectorcraft_ui_egui::os_window::NIGHTLY_PACKAGE_ID, true), ("ai.storyteller.vectorcraft", false), ("io.other.App", false)]
        {
            let result = validate_for(
                Platform::Linux,
                &root,
                |name| if name == "FLATPAK_ID" { Some(id.into()) } else { fixture.get(name) },
                Some(&fixture.exe),
                |_| Ok(()),
            );
            assert_eq!(result.is_ok(), accepted, "{id}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_portable_copy_and_its_data_folder_are_refused() {
        let fixture = Fixture::new();
        let portable = fixture.root.join("portable copy");
        let data = portable.join(crate::prefs_dir::profile::PORTABLE_DATA_DIR);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(portable.join("portable.txt"), b"owned portable fixture").unwrap();
        std::fs::write(data.join("config.toml"), b"stable settings must not change").unwrap();
        for platform in PLATFORMS {
            for candidate in [&data, &data.join("nested nightly")] {
                let probed = Cell::new(false);
                let result = fixture.check_with(platform, candidate, |_| {
                    probed.set(true);
                    Ok(())
                });
                assert!(result.is_err(), "{platform:?} accepted {candidate:?}");
                assert!(!probed.get());
                assert!(!data.join("nested nightly").exists());
                assert_eq!(std::fs::read(data.join("config.toml")).unwrap(), b"stable settings must not change");
            }
        }
        // A merely similar name is not the portable settings folder.
        let similar = fixture.root.join("VectorCraftData-backup-name/nightly");
        assert!(fixture.check(Platform::Linux, &similar).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_profile_cannot_redirect_recovery_or_a_library() {
        let fixture = Fixture::new();
        let root = fixture.root.join("nightly");
        std::fs::create_dir_all(&root).unwrap();
        let external = fixture.root.join("stable-document");
        std::fs::write(&external, b"stable bytes").unwrap();
        for name in ["Data Recovery", "Swatches"] {
            let alias = root.join(name);
            std::os::unix::fs::symlink(&external, &alias).unwrap();
            assert!(fixture.check(Platform::Linux, &root).is_err(), "{name} alias");
            std::fs::remove_file(&alias).unwrap();
        }
        assert_eq!(std::fs::read(&external).unwrap(), b"stable bytes");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_or_a_linked_ancestor_is_refused() {
        let fixture = Fixture::new();
        let elsewhere = fixture.root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let linked = fixture.root.join("nightly");
        std::os::unix::fs::symlink(&elsewhere, &linked).unwrap();
        // Canonical resolution keeps a link's target: a profile reached through one is fine, but it
        // is judged (and named) as its target, so nothing is written under the link.
        let resolved = fixture.check(Platform::Linux, &linked).unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&elsewhere).unwrap());
        std::fs::remove_file(&linked).unwrap();
        let dangling = fixture.root.join("dangling/nightly");
        std::fs::create_dir_all(dangling.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(fixture.root.join("missing"), dangling.parent().unwrap().join("nightly")).unwrap();
        assert!(fixture.check(Platform::Linux, &dangling).is_err());
    }

    #[test]
    fn a_home_settings_folder_is_fenced_even_when_the_xdg_override_moves_elsewhere() {
        let fixture = Fixture::new();
        let xdg = fixture.root.join("override");
        std::fs::create_dir_all(&xdg).unwrap();
        let config = fixture.home().join(".config/vectorcraft");
        let data = fixture.home().join(".local/share/vectorcraft");
        for stable in [config.clone(), data.clone()] {
            for candidate in [stable.clone(), stable.join("nested nightly")] {
                let result = validate_for(
                    Platform::Linux,
                    &candidate,
                    |name| match name {
                        "XDG_CONFIG_HOME" | "XDG_DATA_HOME" => Some(xdg.clone().into_os_string()),
                        _ => fixture.get(name),
                    },
                    Some(&fixture.exe),
                    |_| Ok(()),
                );
                assert!(result.is_err(), "accepted {candidate:?} with the XDG folders moved to {xdg:?}");
            }
        }
        assert!(!config.exists() && !data.exists(), "a refused root creates nothing");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_entry_point_keeps_its_fence_instead_of_refusing_every_launch() {
        let fixture = Fixture::new();
        let bin = fixture.root.join("entry/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let entry = bin.join("vectorcraft");
        std::os::unix::fs::symlink(fixture.root.join("uninstalled target"), &entry).unwrap();
        // The shared resolver refuses the dangling link, as a profile root must be.
        assert!(canonical_future(&entry).is_err());
        assert_eq!(canonical_future_entry(&entry).unwrap(), std::fs::canonicalize(&bin).unwrap().join("vectorcraft"));
    }

    #[test]
    fn an_unwritable_profile_is_refused_after_the_probe_and_never_used() {
        let fixture = Fixture::new();
        let root = fixture.root.join("nightly");
        let result = fixture.check_with(Platform::Linux, &root, |_| Err(std::io::Error::other("read-only")));
        assert!(result.unwrap_err().contains("not writable"));
    }

    #[test]
    fn the_platform_case_rules_are_the_file_systems_ones() {
        let fixture = Fixture::new();
        let upper = fixture.root.join("NIGHTLY");
        assert!(fixture.check(Platform::Linux, &upper).is_ok());
        let stable_mac = fixture.home().join("library/application support/vectorcraft");
        assert!(fixture.check(Platform::Mac, &stable_mac).is_err(), "macOS compares folders case-insensitively");
    }
}
