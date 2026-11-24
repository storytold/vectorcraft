//! Launcher-owned nightly-v1 bootstrap. Selected before logging, updates, fonts or saved state.
//! Stable launches retain their original arguments and directory policy.
//!
//! The protocol is `vectorcraft --launch-profile nightly-v1 --profile-root <abs> -- [files…]`: the
//! selector and the root are separate tokens, exactly once each, and everything after `--` is a file
//! even when it looks like one of them. The Launcher's version check
//! (`… --version`) is answered before the root is validated or touched.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const VERSION: &str = "nightly-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Profile {
    Stable,
    NightlyV1(PathBuf),
}

impl Profile {
    pub fn is_nightly(&self) -> bool {
        matches!(self, Self::NightlyV1(_))
    }

    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Stable => None,
            Self::NightlyV1(root) => Some(root),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub profile: Profile,
    pub args: Vec<OsString>,
}

static PROFILE: OnceLock<Profile> = OnceLock::new();

pub fn current() -> &'static Profile {
    PROFILE.get().unwrap_or(&Profile::Stable)
}

pub fn install(profile: Profile) -> Result<(), String> {
    PROFILE.set(profile).map_err(|_| "the launch profile was already selected".to_string())
}

/// Recognize only the agreed literal selector/root pair before `--`. No filesystem or startup hook
/// runs here.
pub fn parse(args: Vec<OsString>) -> Result<Launch, String> {
    let mut selector = None;
    let mut root = None;
    let mut other = Vec::new();
    let mut files = Vec::new();
    let mut iter = args.iter();
    let mut delimiter = false;
    while let Some(arg) = iter.next() {
        if arg == "--" {
            delimiter = true;
            files.extend(iter.cloned());
            break;
        }
        if arg == "--launch-profile" || arg == "--profile-root" {
            let slot = if arg == "--launch-profile" { &mut selector } else { &mut root };
            if slot.is_some() {
                return Err("the profile selector and root must each occur exactly once".to_string());
            }
            let value = iter.next().filter(|v| !v.is_empty()).ok_or_else(|| "the profile selector/root needs a value".to_string())?;
            if value.to_string_lossy().starts_with("--") {
                return Err("the profile selector/root needs a value".to_string());
            }
            *slot = Some(value.clone());
        } else if arg.to_string_lossy().starts_with("--launch-profile=") || arg.to_string_lossy().starts_with("--profile-root=") {
            return Err("the nightly protocol requires separate literal selector/root argument tokens".to_string());
        } else {
            other.push(arg.clone());
        }
    }
    match (selector, root) {
        (None, None) => Ok(Launch { profile: Profile::Stable, args }),
        (Some(version), Some(root)) if version == VERSION => {
            if !delimiter || !other.is_empty() {
                return Err("nightly-v1 accepts only its selector/root followed by -- and file names".to_string());
            }
            let root = PathBuf::from(root);
            if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err("the nightly profile root must be absolute and contain no parent traversal".to_string());
            }
            Ok(Launch { profile: Profile::NightlyV1(root), args: std::iter::once(OsString::from("--")).chain(files).collect() })
        }
        (Some(_), Some(_)) => Err("unsupported launch profile (expected nightly-v1)".to_string()),
        _ => Err("--launch-profile and --profile-root must be supplied together".to_string()),
    }
}

/// The Launcher's version check: the selector/root pair followed by exactly `--version` (or `-V`),
/// answered like a plain version query. Only [`parse`]'s lexical checks apply; the root is never
/// validated or touched, and `-- --version` stays a file name.
pub fn version_query(args: &[OsString]) -> bool {
    let Some((last, pair)) = args.split_last() else { return false };
    if last != "--version" && last != "-V" {
        return false;
    }
    let lexical = pair.iter().cloned().chain(std::iter::once(OsString::from("--"))).collect();
    matches!(parse(lexical), Ok(Launch { profile: Profile::NightlyV1(_), args }) if args.len() == 1)
}

/// Validation finishes before the caller's first startup effect. Stable never calls the nightly root
/// validator, so its portable and config resolution stays unchanged.
pub fn prepare(mut launch: Launch, validate: impl FnOnce(&Path) -> Result<PathBuf, String>) -> Result<Launch, String> {
    if let Profile::NightlyV1(root) = &launch.profile {
        launch.profile = Profile::NightlyV1(validate(root)?);
    }
    Ok(launch)
}

/// Resolve the selected private root before the caller's first startup effect.
pub fn validate_root(root: &Path, env: impl Fn(&str) -> Option<OsString>, exe: Option<&Path>) -> Result<PathBuf, String> {
    crate::profile_root::validate(root, env, exe, crate::prefs_dir::profile::ensure_writable)
}

/// The arguments a child of this process starts with: a nightly launch hands its profile on, so the
/// child opens the same private profile instead of being refused ("Open VectorCraft Nightly…"), and
/// `--` keeps a file whose name starts with '-' a file. Stable's child arguments are unchanged.
pub fn child_args(files: &[String]) -> Vec<OsString> {
    child_args_for(current(), files)
}

fn child_args_for(profile: &Profile, files: &[String]) -> Vec<OsString> {
    let mut args: Vec<OsString> = match profile {
        Profile::Stable => Vec::new(),
        Profile::NightlyV1(root) => {
            vec![OsString::from("--launch-profile"), OsString::from(VERSION), OsString::from("--profile-root"), root.clone().into_os_string()]
        }
    };
    args.push(OsString::from("--"));
    args.extend(files.iter().map(OsString::from));
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn absolute() -> &'static str {
        if cfg!(windows) { r"C:\NightlyData\profiles\vectorcraft" } else { "/nightly-data/profiles/vectorcraft" }
    }

    fn nightly() -> Vec<OsString> {
        args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--"])
    }

    #[test]
    fn stable_arguments_and_directory_policy_are_unchanged() {
        for argv in [args(&[]), args(&["--new-instance", "a.svg"]), args(&["--", "--launch-profile", "file.svg"])] {
            let launch = prepare(parse(argv.clone()).unwrap(), |_| panic!("stable must never validate a nightly root")).unwrap();
            assert_eq!(launch, Launch { profile: Profile::Stable, args: argv });
        }
    }

    #[test]
    fn exact_version_and_root_are_parsed_without_reinterpreting_file_names() {
        let mut argv = nightly();
        argv.extend(args(&["--profile-root", "art with spaces.svg"]));
        let launch = parse(argv).unwrap();
        assert_eq!(launch.profile, Profile::NightlyV1(PathBuf::from(absolute())));
        assert_eq!(launch.args, args(&["--", "--profile-root", "art with spaces.svg"]));
    }

    #[test]
    fn missing_duplicate_unknown_and_conflicting_profile_arguments_are_refused() {
        for argv in [
            args(&["--launch-profile", VERSION]),
            args(&["--profile-root", absolute()]),
            args(&["--launch-profile", "future-v2", "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--launch-profile", VERSION, "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", "relative", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", "../escape", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", "", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--new-instance", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute()]),
        ] {
            assert!(parse(argv.clone()).is_err(), "{argv:?}");
        }
    }

    #[test]
    fn the_launcher_version_check_is_a_version_query_only_in_its_exact_form() {
        let pair = &["--launch-profile", VERSION, "--profile-root", absolute()];
        let reversed = &["--profile-root", absolute(), "--launch-profile", VERSION];
        for flag in ["--version", "-V"] {
            for form in [pair, reversed] {
                let mut argv = args(form);
                argv.push(flag.into());
                assert!(version_query(&argv), "{argv:?}");
            }
        }
        for argv in [
            args(&["--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--", "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--new-instance", "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version", "--version"]),
            args(&["--version", "--launch-profile", VERSION, "--profile-root", absolute()]),
            args(&["--launch-profile", "future-v2", "--profile-root", absolute(), "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", "relative", "--version"]),
            args(&["--launch-profile", VERSION, "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--help"]),
        ] {
            assert!(!version_query(&argv), "{argv:?}");
        }
        // Outside the query the strict protocol still refuses the version flag.
        assert!(parse(args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version"])).is_err());
    }

    #[test]
    fn failed_root_validation_never_reaches_startup_or_falls_back_to_stable() {
        let reached = Cell::new(false);
        let result = prepare(parse(nightly()).unwrap(), |_| Err("read-only or unsafe profile".to_string())).map(|_| reached.set(true));
        assert!(result.is_err());
        assert!(!reached.get());
    }

    #[test]
    fn successful_root_validation_keeps_the_persistent_root_not_the_binary_tag() {
        let checked = PathBuf::from(absolute()).join("checked");
        let launch = prepare(parse(nightly()).unwrap(), |_| Ok(checked.clone())).unwrap();
        assert_eq!(launch.profile.root(), Some(checked.as_path()));
        assert!(launch.profile.is_nightly());
    }

    #[test]
    fn a_joined_flag_form_is_refused() {
        for joined in [format!("--launch-profile={VERSION}"), format!("--profile-root={absolute()}")] {
            let argv = vec![OsString::from(joined), OsString::from("--")];
            assert!(parse(argv).is_err());
        }
    }

    #[test]
    fn a_stable_child_gets_the_file_delimiter_only() {
        let child = child_args_for(&Profile::Stable, &["a file.svg".to_string(), "--looks-like-a-flag".to_string()]);
        assert_eq!(child, args(&["--", "a file.svg", "--looks-like-a-flag"]));
    }

    #[test]
    fn a_nightly_child_inherits_the_selected_profile() {
        let root = PathBuf::from(absolute());
        let child = child_args_for(&Profile::NightlyV1(root.clone()), &["art.svg".to_string()]);
        let expected = vec![
            OsString::from("--launch-profile"),
            OsString::from(VERSION),
            OsString::from("--profile-root"),
            root.into_os_string(),
            OsString::from("--"),
            OsString::from("art.svg"),
        ];
        assert_eq!(child, expected);
        // The child's own parse finds the same profile and the file list.
        let launch = parse(child).unwrap();
        assert!(launch.profile.is_nightly());
        assert_eq!(launch.args, args(&["--", "art.svg"]));
    }
}
