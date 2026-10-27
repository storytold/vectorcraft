use super::*;

/// The window, Dock, taskbar and app-switcher icon (`assets/app-icon/`, see its README). macOS gets
/// the version with Apple's transparent margin; elsewhere the full-bleed tile. The app ID matches
/// `packaging/linux/ai.storyteller.vectorcraft.desktop` so Wayland docks find the launcher icon.
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/vectorcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Windows shows a window's big icon (`ICON_BIG`) in the taskbar and Alt+Tab; eframe sets only the
/// small one (the title bar's), so the taskbar showed a generic icon. Set the big one too.
#[cfg(windows)]
pub(crate) fn set_taskbar_icon(w: &winit::window::Window) {
    use winit::platform::windows::WindowExtWindows as _;
    let icon = app_icon();
    // An icon that can't be made leaves the generic one: nothing else depends on it.
    if let Ok(i) = winit::window::Icon::from_rgba(icon.rgba, icon.width, icon.height) {
        w.set_taskbar_icon(Some(i));
    }
}

/// "name (backend, kind)" of the adapter the window renders with, for Help › About and bug reports.
#[cfg(feature = "wgpu")]
pub(crate) fn adapter_summary(info: &eframe::wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type)
}

/// Windows and Linux: no OS title bar; the app bar is the title bar (`vectorcraft_ui_egui::titlebar`).
/// macOS keeps its traffic lights over the integrated title strip.
///
/// Used on macOS and Windows (where VectorCraft keeps today's frame regardless of the
/// `craft-window` probe) and by the tests; Linux and the BSDs take the decision from the probe.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");

/// Whether the user asked for the system title bar in Preferences › User Interface › System Title
/// Bar. Read from the saved preferences before the window opens; a missing or unreadable value is
/// `false` (the default). This preference forces the desktop/native frame back on.
pub(crate) fn opted_into_system_title_bar(saved: Option<&vectorcraft_ui_egui::UiState>) -> bool {
    saved.map(|ui| ui.engine_prefs.get("systemTitleBar").and_then(serde_json::Value::as_bool).unwrap_or(false)).unwrap_or(false)
}

/// Whether this start draws its own title bar, given the frame the shared `craft-window` crate
/// decided and the user's System Title Bar preference.
///
/// The suite-wide rule (`craft_window::Choice`) is: the desktop draws the frame
/// ([`craft_window::Frame::Desktop`]) on macOS, Windows, X11 and any Wayland compositor with
/// server-side decorations (KDE, Sway, Hyprland, COSMIC); the app draws it
/// ([`craft_window::Frame::App`]) only where the compositor leaves it to the app (GNOME's Mutter),
/// or when `VECTORCRAFT_TITLEBAR` overrides the probe. VectorCraft draws its own title bar exactly
/// when `craft-window` says the app owns the frame **and** the user has not opted into the system
/// title bar; the preference always forces the native frame back on.
///
/// macOS and Windows keep today's behavior and are decided by [`CUSTOM_TITLEBAR`] rather than the
/// probe: `craft-window` returns `Frame::Desktop` for both (the OS always draws a frame), but
/// VectorCraft insets its content under the macOS traffic lights and draws its own caption buttons
/// on Windows — an overdraw on an OS-decorated window that is VectorCraft's choice, not
/// `craft-window`'s. Only Linux and the BSDs take the decision from the probe.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn custom_titlebar_from(frame: &craft_window::Frame, opted_into_system: bool) -> bool {
    matches!(frame, craft_window::Frame::App(_)) && !opted_into_system
}

/// macOS and Windows: today's behavior, independent of the `craft-window` probe (see
/// [`custom_titlebar_from`]). macOS never draws its own frame (`CUSTOM_TITLEBAR` is `false`);
/// Windows draws its own caption unless the user opted into the system title bar.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn custom_titlebar_from(_frame: &craft_window::Frame, opted_into_system: bool) -> bool {
    CUSTOM_TITLEBAR && !opted_into_system
}

/// Whether this start draws its own title bar. Probes the shared `craft-window` crate once (its
/// override variable is `VECTORCRAFT_TITLEBAR`) and combines its decision with the saved System
/// Title Bar preference. Call once at startup, before [`native_options`].
///
/// On Linux, Preferences › Performance › Linux Display Server › X11 opens the window through
/// XWayland (below). A GNOME/Wayland probe would say `Frame::App` there, but the window really runs
/// on X11, whose window manager draws the frame — so when XWayland is being forced, keep the native
/// frame, the same as a real X11 session. An explicit `VECTORCRAFT_TITLEBAR` override still wins.
pub(crate) fn custom_titlebar(saved: Option<&vectorcraft_ui_egui::UiState>) -> bool {
    let choice = craft_window::Choice::probe(craft_window::Prefix::new("VECTORCRAFT"));
    let opted_into_system = opted_into_system_title_bar(saved);
    #[cfg(target_os = "linux")]
    {
        let pref = saved.and_then(|ui| ui.engine_prefs.get("linuxDisplayServer")).and_then(serde_json::Value::as_str);
        // `overridden() == None` means the probe decided (no VECTORCRAFT_TITLEBAR). When the window
        // will actually open on X11 (XWayland) — the user asked for it *and* the X11 libraries are
        // present, the same two conditions the launch path checks below — that run is effectively
        // X11, so keep the native frame. If the libraries are missing, startup stays on Wayland, so
        // the probe's own decision (app-drawn on GNOME) must stand.
        let forced_to_x11 = xwayland::wanted(pref, |name| std::env::var_os(name), xwayland::pen_attached) && xwayland::x11_libraries_found();
        if choice.overridden().is_none() && forced_to_x11 {
            return custom_titlebar_from(&craft_window::Frame::Desktop, opted_into_system);
        }
    }
    custom_titlebar_from(choice.frame(), opted_into_system)
}

pub(crate) fn native_options(custom_titlebar: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VectorCraft")
            .with_inner_size(window::DEFAULT_SIZE)
            .with_min_inner_size(window::MIN_SIZE)
            .with_drag_and_drop(true)
            .with_decorations(!custom_titlebar)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(app_icon())
            .with_app_id("ai.storyteller.vectorcraft"),
        #[cfg(feature = "windows7")]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}
