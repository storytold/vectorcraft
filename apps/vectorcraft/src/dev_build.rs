//! Is this a development build? One running from a cargo target directory (`…/release/`,
//! `…/debug/` or another profile's) of a checkout that is still there, as `cargo run` makes.
//! Installed apps never run from there (`/usr/bin`, an AppImage, `VectorCraft.app`, Program Files).
//! `VECTORCRAFT_DEV=1` / `=0` forces it either way. A development build shows a DEV tag in the app
//! bar ([`vectorcraft_ui_egui::dev_build`]) and "VectorCraft (dev)" in its window title.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use vectorcraft_ui_egui::dev_build::DevBuild;

/// How long startup waits for `git status` before showing the tag without the git details.
const GIT_WAIT: Duration = Duration::from_millis(500);

/// The checkout this binary was built from (the workspace root, two levels above this crate).
fn checkout() -> Option<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()
}

/// What this build knows about itself, when it is a development build.
pub fn detect() -> Option<DevBuild> {
    let forced = std::env::var("VECTORCRAFT_DEV").ok();
    let exe = std::env::current_exe().ok();
    let root = checkout()?;
    let dev = match forced.as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => exe.as_deref().is_some_and(|e| in_profile_dir(e) && root.join("Cargo.toml").is_file()),
    };
    if !dev {
        return None;
    }
    let (branch, commit, changes) = git_status(root).map(|s| parse_git_status(&s)).unwrap_or_default();
    let age_at_start = exe
        .and_then(|e| std::fs::metadata(e).ok()?.modified().ok())
        .and_then(|built| SystemTime::now().duration_since(built).ok())
        .map(|d| d.as_secs_f64());
    Some(DevBuild { checkout: root.display().to_string(), branch, commit, changes, age_at_start })
}

/// The binary sits in a cargo profile directory: `target/release` or `target/debug` (also under a
/// target triple or another `CARGO_TARGET_DIR`), or any profile's, which cargo marks with a
/// `.fingerprint` directory.
fn in_profile_dir(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    dir.file_name().is_some_and(|d| d == "release" || d == "debug") || dir.join(".fingerprint").is_dir()
}

/// `git status --porcelain=v2 --branch` in the checkout, or `None` without git or when it takes
/// longer than [`GIT_WAIT`] (a slow disk or a huge checkout mustn't hold up the window).
fn git_status(root: &Path) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("dev-build-git".into())
        .spawn(move || {
            // The receiver is gone when startup stopped waiting: nothing to report then.
            let _ = tx.send(run_git_status(&root));
        })
        .ok()?;
    rx.recv_timeout(GIT_WAIT).ok().flatten()
}

fn run_git_status(root: &Path) -> Option<String> {
    let mut cmd = Command::new("git");
    // Read-only: no index refresh or lock (a concurrent git command may hold it) and no fsmonitor hook.
    cmd.arg("-C").arg(root).args(["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain=v2", "--branch"]);
    #[cfg(windows)]
    {
        // A GUI app would otherwise flash a console window for git.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// (branch, abbreviated commit, uncommitted changes) from `git status --porcelain=v2 --branch`.
fn parse_git_status(status: &str) -> (Option<String>, Option<String>, bool) {
    let (mut branch, mut commit, mut changes) = (None, None, false);
    for line in status.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(oid) = line.strip_prefix("# branch.oid ") {
            commit = (oid != "(initial)").then(|| oid.chars().take(7).collect());
        } else if !line.starts_with('#') && !line.is_empty() {
            changes = true;
        }
    }
    (branch, commit, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_profile_dirs_are_dev_and_installs_are_not() {
        for dev in ["/src/vc/target/release/vectorcraft", "/src/vc/target/debug/vectorcraft", "/tmp/t/x86_64-unknown-linux-gnu/release/vectorcraft"] {
            assert!(in_profile_dir(Path::new(dev)), "{dev}");
        }
        for installed in [
            "/usr/bin/vectorcraft",
            "/tmp/.mount_vectorXYZ/usr/bin/vectorcraft",
            "/Applications/VectorCraft.app/Contents/MacOS/vectorcraft",
            "C:/Program Files/VectorCraft/vectorcraft.exe",
            "vectorcraft",
        ] {
            assert!(!in_profile_dir(Path::new(installed)), "{installed}");
        }
    }

    #[test]
    fn any_cargo_profile_dir_is_dev() {
        let dir = std::env::temp_dir().join(format!("vc-dev-build-{}", std::process::id()));
        let profile = dir.join("dist");
        std::fs::create_dir_all(profile.join(".fingerprint")).unwrap();
        assert!(in_profile_dir(&profile.join("vectorcraft")));
        assert!(!in_profile_dir(&dir.join("vectorcraft")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_this_checkout_with_git() {
        // Without git on the machine there is nothing to read; with it, this checkout has a commit.
        if Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success()) {
            let status = run_git_status(checkout().unwrap()).expect("git status");
            assert!(parse_git_status(&status).1.is_some(), "{status}");
        }
    }

    #[test]
    fn git_status_gives_branch_commit_and_changes() {
        let clean = "# branch.oid 44db1c9e0f1a2b3c\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n";
        assert_eq!(parse_git_status(clean), (Some("main".into()), Some("44db1c9".into()), false));
        let dirty = format!("{clean}1 .M N... 100644 100644 100644 aaa bbb README.md\n? scratch.txt\n");
        assert_eq!(parse_git_status(&dirty), (Some("main".into()), Some("44db1c9".into()), true));
        let detached = "# branch.oid 44db1c9e0f1a2b3c\n# branch.head (detached)\n";
        assert_eq!(parse_git_status(detached), (None, Some("44db1c9".into()), false));
        assert_eq!(parse_git_status("# branch.oid (initial)\n# branch.head main\n"), (Some("main".into()), None, false));
        assert_eq!(parse_git_status(""), (None, None, false));
    }

    #[test]
    fn the_checkout_is_the_workspace_root() {
        let root = checkout().expect("workspace root");
        assert!(root.join("Cargo.toml").is_file() && root.join("apps/vectorcraft").is_dir(), "{}", root.display());
    }
}
