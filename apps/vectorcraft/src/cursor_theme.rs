//! Linux Wayland: tell winit where the cursor theme is (#2450).
//!
//! Without the cursor-shape protocol (GNOME 46 and others) winit loads the pointer from an Xcursor
//! theme itself. When that lookup finds nothing, no pointer is ever set and the compositor keeps
//! showing its default one, until the canvas hides the system pointer to paint a tool glyph
//! (`CursorIcon::None`). Then the arrow never comes back over the panels and menus. The lookup
//! fails in a Flatpak sandbox (`XCURSOR_PATH` unset, no cursor theme in the runtime) and where the
//! `default` theme is a dangling symlink; the desktop's own setting names a theme that loads.
//!
//! This reads the desktop's cursor theme and size and sets what the user left unset, once, before
//! any thread starts and before the window opens.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;

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
    fn gsettings_output_is_unquoted() {
        assert_eq!(gsettings_value("'Adwaita'\n"), "Adwaita");
        assert_eq!(gsettings_value("24\n"), "24");
    }
}
