//! Read the host's Caps Lock state, which gives the drawing tools the precise crosshair while it is on.

/// Only host transitions replace a value set by a script through `view.capsLock`.
#[derive(Default)]
pub struct HostCapsLock {
    previous: Option<bool>,
    #[cfg(target_os = "linux")]
    x11: Option<X11CapsLock>,
}

#[cfg(target_os = "linux")]
struct X11CapsLock {
    connection: x11rb::rust_connection::RustConnection,
    locked: bool,
    initial_sequence: u16,
}

impl HostCapsLock {
    pub fn new(window: Option<&winit::window::Window>) -> Self {
        #[cfg(target_os = "linux")]
        let x11 = window.and_then(x11_connection);
        #[cfg(not(target_os = "linux"))]
        let _ = window;
        Self {
            previous: None,
            #[cfg(target_os = "linux")]
            x11,
        }
    }

    pub fn read_transition(&mut self) -> Option<bool> {
        #[cfg(target_os = "linux")]
        let current = match self.x11.as_mut() {
            Some(state) => match state.poll() {
                Some(locked) => Some(locked),
                None => {
                    self.x11 = None;
                    None
                }
            },
            None => {
                // Native Wayland does not expose the seat's XKB lock bit through winit or egui.
                None
            }
        };
        #[cfg(any(target_os = "macos", windows))]
        let current = Some(read());
        // This platform has no lock-state reader.
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        let current = None;
        self.transition(current)
    }

    fn transition(&mut self, reading: Option<bool>) -> Option<bool> {
        reading.and_then(|current| self.observe(current))
    }

    fn observe(&mut self, current: bool) -> Option<bool> {
        if self.previous.replace(current) == Some(current) { None } else { Some(current) }
    }
}

#[cfg(target_os = "linux")]
fn x11_connection(window: &winit::window::Window) -> Option<X11CapsLock> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use x11rb::protocol::xkb;
    if !matches!(window.window_handle().ok()?.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) {
        return None;
    }
    let (connection, _) = x11rb::connect(None).ok()?;
    let supported = x11rb::protocol::xkb::use_extension(&connection, 1, 0).ok()?.reply().ok()?.supported;
    if !supported {
        return None;
    }
    // Subscribe before the initial read so changes during setup remain queued.
    xkb::select_events(
        &connection,
        xkb::ID::USE_CORE_KBD.into(),
        xkb::EventType::default(),
        xkb::EventType::default(),
        xkb::MapPart::default(),
        xkb::MapPart::default(),
        &xkb::SelectEventsAux {
            state_notify: Some(xkb::SelectEventsAuxStateNotify {
                affect_state: xkb::StatePart::MODIFIER_LOCK,
                state_details: xkb::StatePart::MODIFIER_LOCK,
            }),
            ..Default::default()
        },
    )
    .ok()?
    .check()
    .ok()?;
    let (locked, initial_sequence) = read_x11(&connection)?;
    Some(X11CapsLock { connection, locked, initial_sequence })
}

#[cfg(target_os = "linux")]
impl X11CapsLock {
    fn poll(&mut self) -> Option<bool> {
        use x11rb::connection::Connection as _;
        use x11rb::protocol::Event;
        while let Some(event) = self.connection.poll_for_event().ok()? {
            if let Event::XkbStateNotify(state) = event
                // Events queued before the initial GetState reply must not replace that newer snapshot.
                && state.sequence.wrapping_sub(self.initial_sequence) < 0x8000
            {
                self.locked = lock_modifier_is_set(state.locked_mods);
            }
        }
        Some(self.locked)
    }
}

#[cfg(target_os = "linux")]
fn read_x11(connection: &x11rb::rust_connection::RustConnection) -> Option<(bool, u16)> {
    use x11rb::protocol::xkb;
    let state = xkb::get_state(connection, xkb::ID::USE_CORE_KBD.into()).ok()?.reply().ok()?;
    Some((lock_modifier_is_set(state.locked_mods), state.sequence))
}

#[cfg(target_os = "linux")]
fn lock_modifier_is_set(modifiers: x11rb::protocol::xproto::ModMask) -> bool {
    use x11rb::protocol::xproto::ModMask;
    u16::from(modifiers) & u16::from(ModMask::LOCK) != 0
}

#[cfg(target_os = "macos")]
const MAC_CAPS_LOCK: usize = 1 << 16;

#[cfg(target_os = "macos")]
fn mac_flags_have_caps_lock(flags: usize) -> bool {
    flags & MAC_CAPS_LOCK != 0
}

#[cfg(target_os = "macos")]
pub fn read() -> bool {
    mac_flags_have_caps_lock(objc2_app_kit::NSEvent::modifierFlags_class().bits())
}

#[cfg(windows)]
fn windows_state_has_caps_lock(state: i16) -> bool {
    state & 1 != 0
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn read() -> bool {
    // GetKeyState reports the toggle bit for the calling UI thread's keyboard state.
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    // SAFETY: `GetKeyState` takes a virtual-key code by value and reads no caller memory.
    windows_state_has_caps_lock(unsafe { GetKeyState(i32::from(VK_CAPITAL.0)) })
}

// Querying XWayland for a native Wayland window would read a different keyboard.

#[cfg(test)]
mod tests {
    #[test]
    fn no_host_reader_preserves_scripted_value_and_next_reading_transitions() {
        let mut host = super::HostCapsLock::default();
        let mut session = vectorcraft_engine::Session::new();
        session.execute("view.capsLock", &serde_json::json!({"on": true})).unwrap();
        if let Some(on) = host.transition(None) {
            session.execute("view.capsLock", &serde_json::json!({"on": on})).unwrap();
        }
        assert!(session.caps_lock);
        assert_eq!(host.transition(Some(false)), Some(false));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_x11_reader_has_no_transition() {
        assert_eq!(super::HostCapsLock::default().read_transition(), None);
    }

    #[test]
    fn scripted_value_survives_unchanged_host_until_transition() {
        let mut host = super::HostCapsLock::default();
        let mut session = vectorcraft_engine::Session::new();
        if let Some(on) = host.observe(false) {
            session.execute("view.capsLock", &serde_json::json!({"on": on})).unwrap();
        }
        session.execute("view.capsLock", &serde_json::json!({"on": true})).unwrap();
        if let Some(on) = host.observe(false) {
            session.execute("view.capsLock", &serde_json::json!({"on": on})).unwrap();
        }
        assert!(session.caps_lock);
        session.execute("view.capsLock", &serde_json::json!({"on": false})).unwrap();
        if let Some(on) = host.observe(true) {
            session.execute("view.capsLock", &serde_json::json!({"on": on})).unwrap();
        }
        assert!(session.caps_lock);
        if let Some(on) = host.observe(false) {
            session.execute("view.capsLock", &serde_json::json!({"on": on})).unwrap();
        }
        assert!(!session.caps_lock);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_modifier_flags_map_caps_lock() {
        assert!(!super::mac_flags_have_caps_lock(0));
        assert!(!super::mac_flags_have_caps_lock(1 << 17));
        assert!(super::mac_flags_have_caps_lock(1 << 16));
        assert!(super::mac_flags_have_caps_lock((1 << 16) | (1 << 17)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_key_state_maps_toggle_bit() {
        assert!(!super::windows_state_has_caps_lock(0));
        assert!(!super::windows_state_has_caps_lock(i16::MIN));
        assert!(super::windows_state_has_caps_lock(1));
    }
}
