//! The identity every VectorCraft OS window carries: the main window (`main.rs`), the runtime's
//! windows (`InstallSpec::app_id`) and the extra windows built with [`builder`], so docks show the
//! VectorCraft icon on them and Wayland compositors' window rules can match them. Without one the
//! app id is empty.
//!
//! The identity and the version are selected once, by the desktop app, before any window or UI
//! service exists ([`select_identity`], [`set_version`]). A library user (the web build, a test)
//! gets the stable identity and this crate's own version.

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
pub const APP_ID: &str = "ai.storyteller.vectorcraft";

/// The package, macOS bundle and Linux archive identity of a nightly-v1 build; the window and the
/// Windows AUMID end in `.nightly-v1` instead (see [`NIGHTLY_APP_ID`]), so the two stay apart.
pub const NIGHTLY_PACKAGE_ID: &str = "ai.storyteller.vectorcraft.nightly";

/// The window, Dock and Wayland id of a nightly-v1 build.
pub const NIGHTLY_APP_ID: &str = "ai.storyteller.vectorcraft.nightly-v1";

/// Stable's name; `nightly`'s differs so the two are told apart in the Dock, the taskbar and menus.
pub const APP_NAME: &str = "VectorCraft";
pub const NIGHTLY_APP_NAME: &str = "VectorCraft Nightly";

/// Immutable startup identity. The desktop selects it before any UI or runtime service is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchIdentity {
    Stable,
    NightlyV1,
}

/// 0: not selected yet (stable), 1: stable, 2: nightly.
static IDENTITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Selects the identity once; a second selection is an error rather than a silent change.
pub fn select_identity(identity: LaunchIdentity) -> Result<(), String> {
    let value = match identity {
        LaunchIdentity::Stable => 1,
        LaunchIdentity::NightlyV1 => 2,
    };
    IDENTITY
        .compare_exchange(0, value, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "the window launch identity was already selected".to_string())
}

pub fn identity() -> LaunchIdentity {
    if IDENTITY.load(std::sync::atomic::Ordering::Acquire) == 2 { LaunchIdentity::NightlyV1 } else { LaunchIdentity::Stable }
}

pub const fn id_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => APP_ID,
        LaunchIdentity::NightlyV1 => NIGHTLY_APP_ID,
    }
}

pub fn app_id() -> &'static str {
    id_for(identity())
}

pub const fn name_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => APP_NAME,
        LaunchIdentity::NightlyV1 => NIGHTLY_APP_NAME,
    }
}

pub fn name() -> &'static str {
    name_for(identity())
}

/// Whether the stable services (the updater, the control channel, the shared suite state) may run.
pub const fn stable_services_for(identity: LaunchIdentity) -> bool {
    matches!(identity, LaunchIdentity::Stable)
}

pub fn stable_services_allowed() -> bool {
    stable_services_for(identity())
}

/// The version the build was made with: the exact feed identity on a nightly, the package version
/// on a stable build (the host app hands it over from its build script).
static VERSION: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Sets the version once; a second call is an error rather than a silent change.
pub fn set_version(version: &'static str) -> Result<(), String> {
    if version.is_empty() {
        return Err("the built version is empty".to_string());
    }
    VERSION.set(version).map_err(|_| "the built version was already selected".to_string())
}

pub fn version() -> &'static str {
    VERSION.get().copied().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// A builder for an extra OS window, carrying [`app_id`].
pub fn builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_app_id(app_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nightly_identity_is_distinct_and_never_enables_stable_services() {
        assert_eq!(id_for(LaunchIdentity::Stable), APP_ID);
        assert_ne!(id_for(LaunchIdentity::NightlyV1), APP_ID);
        assert_ne!(name_for(LaunchIdentity::Stable), name_for(LaunchIdentity::NightlyV1));
        assert!(stable_services_for(LaunchIdentity::Stable));
        assert!(!stable_services_for(LaunchIdentity::NightlyV1));
    }

    #[test]
    fn the_two_nightly_ids_are_the_contract_ones() {
        // The package and bundle identity ends in `.nightly`; the window and AUMID in `.nightly-v1`.
        assert_eq!(NIGHTLY_PACKAGE_ID, "ai.storyteller.vectorcraft.nightly");
        assert_eq!(NIGHTLY_APP_ID, "ai.storyteller.vectorcraft.nightly-v1");
        assert_ne!(NIGHTLY_PACKAGE_ID, NIGHTLY_APP_ID);
    }

    #[test]
    fn unselected_windows_carry_the_stable_app_id() {
        assert_eq!(builder().app_id.as_deref(), Some(APP_ID));
    }

    #[test]
    fn only_a_non_empty_version_is_kept() {
        assert!(set_version("").is_err());
    }
}
