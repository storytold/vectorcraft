//! Linux pen tablets: winit reports barrel buttons as [`winit::event::MouseButton::Other`]
//! (for example `BTN_STYLUS`), and egui-winit drops those before they reach the UI (#3632).

use eframe::AppCreator;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use eframe::UserEvent;

/// `BTN_STYLUS` and neighbours (`linux/input-event-codes.h`): pen barrel buttons Wayland passes
/// through the pointer protocol as non-standard mouse buttons.
const BTN_STYLUS: u16 = 0x014b;
const BTN_STYLUS2: u16 = 0x014c;
const BTN_STYLUS3: u16 = 0x014d;

/// Like [`eframe::run_native`], but turns pen barrel buttons into mouse buttons egui understands.
pub fn run_native(app_name: &str, mut native_options: eframe::NativeOptions, app_creator: AppCreator<'_>) -> eframe::Result {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    if let Some(hook) = native_options.event_loop_builder.take() {
        hook(&mut builder);
    }
    let event_loop = builder.build().map_err(eframe::Error::WinitEventLoop)?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let inner = eframe::create_native(app_name, native_options, app_creator, &event_loop);
    let mut app = Forwarder { inner };
    event_loop.run_app(&mut app).map_err(eframe::Error::WinitEventLoop)?;
    Ok(())
}

struct Forwarder<'a> {
    inner: eframe::EframeWinitApplication<'a>,
}

impl ApplicationHandler<UserEvent> for Forwarder<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        self.inner.window_event(event_loop, window_id, map_window_event(event));
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.inner.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: DeviceId, event: DeviceEvent) {
        self.inner.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}

/// Map a tablet button code winit labels `MouseButton::Other` to a standard mouse button.
pub(crate) fn map_tablet_button(code: u16) -> Option<winit::event::MouseButton> {
    use winit::event::MouseButton;
    match code {
        BTN_STYLUS | BTN_STYLUS2 => Some(MouseButton::Right),
        BTN_STYLUS3 => Some(MouseButton::Middle),
        _ => None,
    }
}

fn map_window_event(event: WindowEvent) -> WindowEvent {
    if let WindowEvent::MouseInput { device_id, state, button: winit::event::MouseButton::Other(code), .. } = event
        && let Some(button) = map_tablet_button(code)
    {
        WindowEvent::MouseInput { device_id, state, button }
    } else {
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::{ElementState, MouseButton};

    #[test]
    fn stylus_barrel_buttons_become_secondary_clicks() {
        assert_eq!(map_tablet_button(BTN_STYLUS), Some(MouseButton::Right));
        assert_eq!(map_tablet_button(BTN_STYLUS2), Some(MouseButton::Right));
        assert_eq!(map_tablet_button(BTN_STYLUS3), Some(MouseButton::Middle));
        assert_eq!(map_tablet_button(0x110), None);
    }

    #[test]
    fn window_mouse_input_is_remapped_before_eframe() {
        let device_id = winit::event::DeviceId::dummy();
        let press = WindowEvent::MouseInput { device_id, state: ElementState::Pressed, button: MouseButton::Other(BTN_STYLUS) };
        let mapped = map_window_event(press);
        assert!(matches!(mapped, WindowEvent::MouseInput { button: MouseButton::Right, state: ElementState::Pressed, .. }));
        let release = map_window_event(WindowEvent::MouseInput { device_id, state: ElementState::Released, button: MouseButton::Other(BTN_STYLUS2) });
        assert!(matches!(release, WindowEvent::MouseInput { button: MouseButton::Right, state: ElementState::Released, .. }));
    }
}
