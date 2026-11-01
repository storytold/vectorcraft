//! Linux Wayland: tell winit where the cursor theme is (#2450).
//!
//! Without the cursor-shape protocol (GNOME 46 and others) winit loads the pointer from an Xcursor
//! theme itself. When that lookup finds nothing, no pointer is ever set and the compositor keeps
//! showing its default one, until the canvas hides the system pointer to paint a tool glyph
//! (`CursorIcon::None`). Then the arrow never comes back over the panels and menus. The lookup
//! fails in a Flatpak sandbox (`XCURSOR_PATH` unset, no cursor theme in the runtime) and where the
//! `default` theme is a dangling symlink; the desktop's own setting names a theme that loads.
//!
//! Where no cursor theme can be found at all (KDE or sway without gsettings), the env fix does
//! nothing: [`theme_missing`] tells the canvas to keep the system pointer visible beside the tool glyph.
//!
//! This reads the desktop's cursor theme and size and sets what the user left unset, once, before
//! any thread starts and before the window opens.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where a Flatpak sandbox mounts the host's cursor themes (user and system).
const FLATPAK_HOST_ICONS: [&str; 2] = ["/run/host/user-share/icons", "/run/host/share/icons"];

/// The desktop's cursor settings (GNOME's `org.gnome.desktop.interface`).
#[derive(Debug, Default, PartialEq)]
pub struct DesktopCursor {
    pub theme: Option<String>,
    pub size: Option<u32>,
}

/// The variables to set, given the environment, the host icon folders that exist and the
/// desktop's cursor settings. Empty unless this is a Wayland session; variables the user set
/// stay as they are.
pub fn plan(var: impl Fn(&str) -> Option<OsString>, host_icons: &[&str], desktop: DesktopCursor) -> Vec<(&'static str, OsString)> {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    if !(set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET")) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if !set("XCURSOR_PATH") && !host_icons.is_empty() {
        out.push(("XCURSOR_PATH", OsString::from(host_icons.join(":"))));
    }
    if !set("XCURSOR_THEME")
        && let Some(theme) = desktop.theme.filter(|t| !t.is_empty())
    {
        out.push(("XCURSOR_THEME", OsString::from(theme)));
    }
    if !set("XCURSOR_SIZE")
        && let Some(size) = desktop.size.filter(|s| (1..=512).contains(s))
    {
        out.push(("XCURSOR_SIZE", OsString::from(size.to_string())));
    }
    out
}

/// Xcursor's default search path when `XCURSOR_PATH` is unset (relative entries are in `$HOME`).
const DEFAULT_ICON_DIRS: [&str; 5] = ["~/.local/share/icons", "~/.icons", "/usr/share/icons", "/usr/share/pixmaps", "/usr/X11R6/lib/X11/icons"];

/// The themes `index.theme` lists after `Inherits=`.
fn inherits(index: &str) -> Vec<String> {
    index
        .lines()
        .find_map(|l| l.trim().strip_prefix("Inherits="))
        .map(|v| v.split([',', ';']).map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default()
}

/// Can Xcursor load a pointer for the environment's theme (`XCURSOR_THEME`, else `default`)? It does
/// when the theme, or one it inherits from, has a `default` or `left_ptr` cursor file in a folder of
/// `XCURSOR_PATH` (else Xcursor's default folders). `read` returns a file's text, `exists` tells
/// whether a file is there.
pub fn theme_usable(
    var: impl Fn(&str) -> Option<OsString>,
    home: Option<&Path>,
    read: impl Fn(&Path) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> bool {
    let dirs: Vec<PathBuf> = match var("XCURSOR_PATH").filter(|v| !v.is_empty()) {
        Some(v) => std::env::split_paths(&v).collect(),
        None => DEFAULT_ICON_DIRS
            .iter()
            .filter_map(|d| match d.strip_prefix("~/") {
                Some(rest) => home.map(|h| h.join(rest)),
                None => Some(PathBuf::from(d)),
            })
            .collect(),
    };
    let first = var("XCURSOR_THEME").and_then(|v| v.into_string().ok()).filter(|t| !t.is_empty()).unwrap_or_else(|| "default".into());
    let mut todo = vec![first];
    let mut seen: Vec<String> = Vec::new();
    while let Some(theme) = todo.pop() {
        // Bounded: themes inherit in short chains, and a cycle must end.
        if seen.contains(&theme) || seen.len() >= 16 {
            continue;
        }
        for dir in &dirs {
            let base = dir.join(&theme);
            if ["default", "left_ptr"].iter().any(|c| exists(&base.join("cursors").join(c))) {
                return true;
            }
            if let Some(index) = read(&base.join("index.theme")) {
                todo.extend(inherits(&index));
            }
        }
        seen.push(theme);
    }
    false
}

/// On this machine, after [`apply`]: is there no cursor theme Xcursor can load? Then winit never
/// sets a pointer, and the canvas must not hide the system one (#2450).
pub fn theme_missing() -> bool {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    !theme_usable(|n| std::env::var_os(n), home.as_deref(), |p| std::fs::read_to_string(p).ok(), |p| p.exists())
}

/// The value of a `gsettings get` line: `'Adwaita'` becomes `Adwaita`, `24` stays `24`.
fn gsettings_value(output: &str) -> &str {
    output.trim().trim_matches('\'')
}

#[cfg(target_os = "linux")]
fn desktop_cursor() -> DesktopCursor {
    let get = |key: &str| {
        let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    DesktopCursor {
        theme: get("cursor-theme").map(|v| gsettings_value(&v).to_owned()),
        size: get("cursor-size").and_then(|v| gsettings_value(&v).parse().ok()),
    }
}

/// Sets `XCURSOR_PATH`, `XCURSOR_THEME` and `XCURSOR_SIZE` where a Wayland session leaves them
/// unset. Call first thing in `main`, while the process is single-threaded.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn apply() {
    let host: Vec<&str> = FLATPAK_HOST_ICONS.iter().copied().filter(|p| std::path::Path::new(p).is_dir()).collect();
    for (name, value) in plan(|n| std::env::var_os(n), &host, desktop_cursor()) {
        log::debug!("cursor theme: {name}={value:?}");
        // SAFETY: called at the top of `main`, before the app starts any thread.
        unsafe { std::env::set_var(name, value) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
    }

    fn adwaita() -> DesktopCursor {
        DesktopCursor { theme: Some("Adwaita".into()), size: Some(24) }
    }

    #[test]
    fn a_wayland_session_gets_the_desktop_theme_and_the_host_icons() {
        let got = plan(env(&[("WAYLAND_DISPLAY", "wayland-0")]), &["/run/host/share/icons"], adwaita());
        assert_eq!(
            got,
            vec![
                ("XCURSOR_PATH", OsString::from("/run/host/share/icons")),
                ("XCURSOR_THEME", OsString::from("Adwaita")),
                ("XCURSOR_SIZE", OsString::from("24")),
            ]
        );
    }

    #[test]
    fn settings_the_user_made_and_other_sessions_stay_untouched() {
        let set = [("WAYLAND_DISPLAY", "wayland-0"), ("XCURSOR_PATH", "/x"), ("XCURSOR_THEME", "Bibata"), ("XCURSOR_SIZE", "32")];
        assert!(plan(env(&set), &["/run/host/share/icons"], adwaita()).is_empty());
        assert!(plan(env(&[("DISPLAY", ":0")]), &["/run/host/share/icons"], adwaita()).is_empty());
        // A theme-less desktop and an absurd size set nothing.
        let odd = DesktopCursor { theme: Some(String::new()), size: Some(0) };
        assert!(plan(env(&[("WAYLAND_DISPLAY", "wayland-0")]), &[], odd).is_empty());
    }

    #[test]
    fn a_theme_is_usable_directly_or_through_inherits_and_missing_otherwise() {
        let files = ["/i/Bibata/cursors/left_ptr", "/i/Breeze/index.theme", "/i/Loop/index.theme"];
        // Compared as paths, so the joins match on Windows too, where they use `\`.
        let exists = |p: &Path| files.iter().any(|f| Path::new(f) == p);
        let read = |p: &Path| {
            if p == Path::new("/i/Breeze/index.theme") {
                Some("[Icon Theme]\nInherits=Other, Bibata\n".to_owned())
            } else if p == Path::new("/i/Loop/index.theme") {
                Some("Inherits=Loop\n".to_owned())
            } else {
                None
            }
        };
        let with = |theme: &'static str| {
            move |n: &str| (n == "XCURSOR_PATH").then(|| OsString::from("/i")).or((n == "XCURSOR_THEME").then(|| OsString::from(theme)))
        };
        assert!(theme_usable(with("Bibata"), None, read, exists));
        assert!(theme_usable(with("Breeze"), None, read, exists));
        // No such theme, a theme without cursors, an inheritance loop: nothing to load.
        assert!(!theme_usable(with("Nope"), None, read, exists));
        assert!(!theme_usable(with("Loop"), None, read, exists));
        // Unset: Xcursor's default folders and the `default` theme.
        assert!(!theme_usable(|_| None, Some(Path::new("/home/u")), read, exists));
    }

    #[test]
    fn gsettings_output_is_unquoted() {
        assert_eq!(gsettings_value("'Adwaita'\n"), "Adwaita");
        assert_eq!(gsettings_value("24\n"), "24");
    }
}
