//! Linux: Preferences › Performance › Linux Display Server › X11 opens the window through XWayland
//! on a Wayland session, where dropped files arrive (winit 0.30 has no Wayland drag and drop,
//! #1151). X11 (XWayland) When a Pen Is Attached does so only when a pen is attached as the app
//! starts: winit 0.30 doesn't bind the Wayland tablet protocol, and XWayland hands the pen to X11
//! windows as a pointer (#2028). Only when XWayland can run the window: an X display is set and
//! the X11 libraries winit loads at runtime are installed. Otherwise the window opens on Wayland,
//! so a missing library never keeps the app from starting.
//!
//! The library check comes from PhotoCraft's `linux_libs.rs`: no `dlopen` (that needs `unsafe`),
//! just the dynamic linker's cache (`ldconfig -p`) and the usual library directories. The pen
//! check comes from PhotoCraft's `tablet.rs`: the input devices the kernel lists in
//! `/proc/bus/input/devices`.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What winit 0.30 `dlopen`s for an X11 window: x11-dl opens Xlib, Xlib-xcb, Xcursor and XInput2,
/// x11rb opens libxcb, and xkbcommon-dl opens xkbcommon and xkbcommon-x11.
const X11_LIBS: &[&str] =
    &["libX11.so.6", "libX11-xcb.so.1", "libxcb.so.1", "libXcursor.so.1", "libXi.so.6", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"];

/// Whether the saved `linuxDisplayServer` and the environment ask for XWayland: `x11` chosen, or
/// `x11WithPen` chosen and `pen` finding a pen; a Wayland session (winit picks Wayland whenever
/// `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set); and an X display to open. `pen` runs only for
/// `x11WithPen`, after the session checks.
pub fn wanted(pref: Option<&str>, var: impl Fn(&str) -> Option<OsString>, pen: impl FnOnce() -> bool) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    let session = (set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET")) && set("DISPLAY");
    match pref {
        Some("x11") => session,
        Some("x11WithPen") => session && pen(),
        _ => false,
    }
}

/// `BTN_TOOL_PEN` (linux/input-event-codes.h): the key that a pen tablet's stylus, a pen display's
/// pen or a pen-enabled touchscreen's pen reports.
const BTN_TOOL_PEN: u32 = 0x140;
/// `ABS_X` and `ABS_Y` (linux/input-event-codes.h): the absolute axes every pen reports.
const ABS_X: u32 = 0x00;
const ABS_Y: u32 = 0x01;

/// Is a pen attached? Reads `/proc/bus/input/devices`; a file that can't be read has no pen.
pub fn pen_attached() -> bool {
    std::fs::read_to_string("/proc/bus/input/devices").is_ok_and(|devices| has_pen(&devices))
}

/// Is a pen among the input devices `/proc/bus/input/devices` lists? A pen is a device whose
/// `B: KEY=` bitmap has `BTN_TOOL_PEN` and whose `B: ABS=` bitmap has `ABS_X` and `ABS_Y`, as
/// libinput asks of a tablet tool. A virtual keyboard made with uinput can claim every key code,
/// `BTN_TOOL_PEN` included, but has no absolute axes.
fn has_pen(devices: &str) -> bool {
    devices.split("\n\n").any(|device| {
        let bitmap = |name: &str| device.lines().find_map(|l| l.strip_prefix(name));
        bitmap("B: KEY=").is_some_and(|keys| bit_set(keys, BTN_TOOL_PEN))
            && bitmap("B: ABS=").is_some_and(|axes| bit_set(axes, ABS_X) && bit_set(axes, ABS_Y))
    })
}

/// Bit `bit` of a `/proc/bus/input/devices` bitmap: hex words of the kernel's `long`, most
/// significant first. Short or malformed bitmaps have no bits set.
fn bit_set(bitmap: &str, bit: u32) -> bool {
    let bits = usize::BITS;
    let word = usize::try_from(bit / bits).unwrap_or(usize::MAX);
    bitmap.split_whitespace().rev().nth(word).and_then(|w| u64::from_str_radix(w, 16).ok()).is_some_and(|w| (w >> (bit % bits)) & 1 == 1)
}

/// Are all the libraries an X11 window needs found on this system? Libraries the check can't find
/// count as missing.
pub fn x11_libraries_found() -> bool {
    let cache = read_ldconfig();
    let dirs = search_dirs(std::env::var("LD_LIBRARY_PATH").ok().as_deref());
    X11_LIBS
        .iter()
        .all(|lib| cache.as_deref().is_some_and(|out| ldconfig_has(out, lib, ldconfig_arch_tag())) || dirs.iter().any(|d| d.join(lib).exists()))
}

/// The `ldconfig -p` architecture tag for this build, when it's one we can tell apart from a
/// 32-bit library of the same name (`libc6,x86-64` vs plain `libc6` for i386).
fn ldconfig_arch_tag() -> Option<&'static str> {
    if cfg!(target_arch = "x86_64") {
        Some("x86-64")
    } else if cfg!(target_arch = "aarch64") {
        Some("AArch64")
    } else {
        None
    }
}

/// Does `ldconfig -p` output list `soname` for our architecture? Lines look like
/// `\tlibxkbcommon-x11.so.0 (libc6,x86-64) => /lib/x86_64-linux-gnu/libxkbcommon-x11.so.0`.
fn ldconfig_has(output: &str, soname: &str, arch_tag: Option<&str>) -> bool {
    output.lines().any(|line| {
        let Some((name, rest)) = line.trim_start().split_once(' ') else { return false };
        if name != soname {
            return false;
        }
        match arch_tag {
            None => true,
            Some(tag) => {
                let flags = rest.split_once('(').and_then(|(_, r)| r.split_once(')')).map_or("", |(f, _)| f);
                flags.split(',').any(|f| f.trim() == tag)
            }
        }
    })
}

/// `ldconfig -p`, from the usual locations (`/sbin` isn't on a normal user's PATH on Debian). A
/// real listing starts with "N libs found in cache".
fn read_ldconfig() -> Option<String> {
    ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"].into_iter().find_map(|cmd| {
        let out = std::process::Command::new(cmd).arg("-p").output().ok().filter(|out| out.status.success())?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().is_some_and(|l| l.contains("libs found in cache")).then_some(text)
    })
}

/// Library directories to search: `LD_LIBRARY_PATH`, then the usual system and Flatpak paths.
fn search_dirs(ld_library_path: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ld_library_path.unwrap_or("").split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect();
    let triplet = if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else if cfg!(target_arch = "riscv64") {
        "riscv64-linux-gnu"
    } else {
        "x86_64-linux-gnu"
    };
    dirs.extend(["/app/lib", "/usr/local/lib", "/usr/lib64", "/lib64", "/usr/lib", "/lib"].map(PathBuf::from));
    dirs.push(Path::new("/usr/lib").join(triplet));
    dirs.push(Path::new("/lib").join(triplet));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| pairs.iter().find(|(name, _)| *name == k).map(|(_, v)| OsString::from(v))
    }

    fn no_probe() -> bool {
        panic!("the pen check ran")
    }

    #[test]
    fn xwayland_only_when_chosen_on_wayland_with_an_x_display() {
        let wayland = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        assert!(wanted(Some("x11"), &wayland, no_probe));
        assert!(wanted(Some("x11"), env(&[("WAYLAND_SOCKET", "3"), ("DISPLAY", ":0")]), no_probe));
        assert!(!wanted(Some("automatic"), &wayland, no_probe));
        assert!(!wanted(None, &wayland, no_probe));
        assert!(!wanted(Some("wayland"), &wayland, no_probe));
        // An X11 session already opens on X11; without an X display there is no XWayland to use.
        assert!(!wanted(Some("x11"), env(&[("DISPLAY", ":0")]), no_probe));
        assert!(!wanted(Some("x11"), env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", "")]), no_probe));
        assert!(!wanted(Some("x11"), env(&[("WAYLAND_DISPLAY", "wayland-0")]), no_probe));
    }

    /// X11 (XWayland) When a Pen Is Attached (#2028): XWayland on a Wayland session with an X
    /// display when a pen is found, the session's own display server otherwise. The pen check
    /// runs only on such a session.
    #[test]
    fn the_pen_choice_opens_xwayland_only_with_a_pen() {
        let wayland = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        assert!(wanted(Some("x11WithPen"), &wayland, || true));
        assert!(wanted(Some("x11WithPen"), env(&[("WAYLAND_SOCKET", "3"), ("DISPLAY", ":0")]), || true));
        assert!(!wanted(Some("x11WithPen"), &wayland, || false));
        assert!(!wanted(Some("x11WithPen"), env(&[("DISPLAY", ":0")]), no_probe));
        assert!(!wanted(Some("x11WithPen"), env(&[("WAYLAND_DISPLAY", "wayland-0")]), no_probe));
        assert!(!wanted(Some("x11WithPen"), env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", "")]), no_probe));
    }

    /// `/proc/bus/input/devices` of a laptop with a Wacom Intuos Pro attached (64-bit longs: the
    /// pen's `KEY=1c03 0 0 0 0 0` has bit 0x140, `BTN_TOOL_PEN`), from PhotoCraft's tests.
    const WITH_PEN: &str = "I: Bus=0011 Vendor=0001 Product=0001 Version=ab83\n\
        N: Name=\"AT Translated Set 2 keyboard\"\n\
        H: Handlers=sysrq kbd event3 leds\n\
        B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe\n\
        \n\
        I: Bus=0018 Vendor=06cb Product=ce26 Version=0100\n\
        N: Name=\"SYNA2B33:00 06CB:CE26 Touchpad\"\n\
        H: Handlers=mouse1 event7\n\
        B: KEY=e520 10000 0 0 0 0\n\
        B: ABS=2e0800000000003\n\
        \n\
        I: Bus=0003 Vendor=056a Product=0357 Version=0110\n\
        N: Name=\"Wacom Intuos Pro M Pen\"\n\
        H: Handlers=mouse2 event10\n\
        B: KEY=1c03 0 0 0 0 0\n\
        B: ABS=1000f000003\n";

    /// A uinput virtual keyboard that claims every key code, `BTN_TOOL_PEN` included, and has no
    /// absolute axes (`EV=20013`, no `B: ABS=` line), from PhotoCraft's tests.
    const VIRTUAL_KEYBOARD: &str = "I: Bus=0003 Vendor=1234 Product=5678 Version=0111\n\
        N: Name=\"Virtual Keyboard\"\n\
        H: Handlers=sysrq rfkill kbd event256 leds \n\
        B: EV=20013\n\
        B: KEY=ffffffffff 0 7ffffff000301ff 1000f7fffffff 7fe001fffff000f 7ffffffffffffff ffffffff0003fdff 7fff8fff00ff03ff 1ffffffffffff07 ffffffffffffffff ffffffffffefffff fffffffffffffffe\n";

    #[test]
    fn a_pen_is_a_device_with_btn_tool_pen_and_absolute_axes() {
        assert!(has_pen(WITH_PEN));
        let without = WITH_PEN.split("\n\n").take(2).collect::<Vec<_>>().join("\n\n");
        assert!(!has_pen(&without), "a touchpad's BTN_TOOL_FINGER (0x145) is not a pen");
        // A pen display reports the pen with its own axes.
        assert!(has_pen("N: Name=\"Wacom One Pen\"\nB: KEY=1c03 0 0 0 0 0\nB: ABS=3\n"));
        // Axes without ABS_X or ABS_Y, or no axes at all, are no pen.
        assert!(!has_pen("B: KEY=1c03 0 0 0 0 0\nB: ABS=1000f000002\n"));
        assert!(!has_pen("B: KEY=1c03 0 0 0 0 0\nB: ABS=1000f000001\n"));
        let pen_without_axes = WITH_PEN.lines().filter(|l| !l.starts_with("B: ABS=1000f")).collect::<Vec<_>>().join("\n");
        assert!(!has_pen(&pen_without_axes));
        // The virtual keyboard claims the key but is no pen, and a real pen beside it is still found.
        assert!(VIRTUAL_KEYBOARD.lines().find_map(|l| l.strip_prefix("B: KEY=")).is_some_and(|k| bit_set(k, BTN_TOOL_PEN)));
        assert!(!has_pen(VIRTUAL_KEYBOARD));
        assert!(has_pen(&format!("{VIRTUAL_KEYBOARD}\n{WITH_PEN}")));
        // Short or malformed bitmaps are no pen, never a panic.
        for keys in ["", "B: KEY=", "B: KEY=1", "B: KEY=zz 0 0 0 0 0", "B: KEY=ffffffffffffffffff 0 0 0 0 0", "B: KEY=0 0 0 0 0 0 0 0"] {
            assert!(!has_pen(&format!("{keys}\nB: ABS=3")), "{keys:?}");
        }
        for abs in ["B: ABS=", "B: ABS=zz", "B: ABS=ffffffffffffffffff", "B: ABS=0 0"] {
            assert!(!has_pen(&format!("B: KEY=1c03 0 0 0 0 0\n{abs}")), "{abs:?}");
        }
    }

    #[test]
    fn ldconfig_parsing_matches_name_and_architecture() {
        let cache = "12 libs found in cache `/etc/ld.so.cache'\n\
            \tlibX11.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libX11.so.6\n\
            \tlibxkbcommon-x11.so.0 (libc6) => /lib/i386-linux-gnu/libxkbcommon-x11.so.0\n";
        assert!(ldconfig_has(cache, "libX11.so.6", Some("x86-64")));
        assert!(!ldconfig_has(cache, "libX11.so.6", Some("AArch64")));
        // Only the 32-bit build is installed: doesn't count for a 64-bit app.
        assert!(!ldconfig_has(cache, "libxkbcommon-x11.so.0", Some("x86-64")));
        assert!(ldconfig_has(cache, "libxkbcommon-x11.so.0", None));
        assert!(!ldconfig_has(cache, "libX11.so", None));
    }
}
