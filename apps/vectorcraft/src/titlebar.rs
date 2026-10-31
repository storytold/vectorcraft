use super::window;

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

/// Windows: no OS title bar; the app bar is the title bar (`vectorcraft_ui_egui::titlebar`).
/// macOS keeps its traffic lights over the integrated title strip.
///
/// Used on macOS and Windows; Linux and the BSDs decide in [`custom_titlebar_from`], where
/// VectorCraft also draws its own title bar by default.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");

/// Whether the user asked for the system title bar in Preferences › User Interface › System Title
/// Bar. Read from the saved preferences before the window opens; a missing or unreadable value is
/// `false` (the default). This preference forces the desktop/native frame back on.
pub(crate) fn opted_into_system_title_bar(saved: Option<&vectorcraft_ui_egui::UiState>) -> bool {
    saved.map(|ui| ui.engine_prefs.get("systemTitleBar").and_then(serde_json::Value::as_bool).unwrap_or(false)).unwrap_or(false)
}

/// Whether this start draws its own title bar, given `VECTORCRAFT_TITLEBAR` (`overridden`, as
/// [`craft_window::Choice::overridden`] reads it) and the user's System Title Bar preference.
///
/// Linux and the BSDs: VectorCraft draws its own title bar on every desktop (GNOME, KDE, Sway,
/// Hyprland, COSMIC, X11 window managers) unless System Title Bar is on, as PhotoCraft does. The
/// variable decides for one run when it is set: `app` draws VectorCraft's bar and `system` or
/// `desktop` keeps the desktop's frame, whatever the preference says. The desktop the
/// `craft-window` probe finds does not change the result.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn custom_titlebar_from(overridden: Option<bool>, opted_into_system: bool) -> bool {
    overridden.unwrap_or(!opted_into_system)
}

/// macOS and Windows: decided by [`CUSTOM_TITLEBAR`] and the preference; `VECTORCRAFT_TITLEBAR`
/// is not read. macOS never draws its own frame (`CUSTOM_TITLEBAR` is `false`); Windows draws its
/// own caption unless the user opted into the system title bar.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn custom_titlebar_from(_overridden: Option<bool>, opted_into_system: bool) -> bool {
    CUSTOM_TITLEBAR && !opted_into_system
}

/// What `VECTORCRAFT_TITLEBAR` asks for on Linux and the BSDs: `Some(true)` VectorCraft's own title
/// bar, `Some(false)` the desktop's frame, `None` when it is unset or holds another value. Always
/// `None` on macOS and Windows. The `craft-window` probe runs once per process.
pub(crate) fn titlebar_override() -> Option<bool> {
    static OVERRIDE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return None;
    }
    *OVERRIDE.get_or_init(|| craft_window::Choice::probe(craft_window::Prefix::new("VECTORCRAFT")).overridden())
}

/// Whether this start draws its own title bar: [`custom_titlebar_from`] with `VECTORCRAFT_TITLEBAR`
/// and the saved System Title Bar preference. Call once at startup, before [`native_options`].
pub(crate) fn custom_titlebar(saved: Option<&vectorcraft_ui_egui::UiState>) -> bool {
    custom_titlebar_from(titlebar_override(), opted_into_system_title_bar(saved))
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
        ..Default::default()
    }
}
