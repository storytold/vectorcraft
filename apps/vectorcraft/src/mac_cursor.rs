//! AppKit delivery of the UI's existing cursor artwork, independent of canvas presentation.
#![forbid(unsafe_code)]
// Interim #2292: remove when pinned winit correctly handles macOS cursor image and hotspot sizing.

use egui::{Context, FullOutput, Plugin, ViewportId};
use objc2::{AllocAnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSCursor, NSImage, NSWindow};
use objc2_foundation::{NSData, NSPoint, NSSize};
use std::cell::RefCell;
use vectorcraft_ui_egui::cursors::native_request::{self, Bitmap, Key, Request};

const CAPACITY: usize = 128;

fn build(bitmap: &Bitmap) -> Option<Retained<NSCursor>> {
    let one = &bitmap.representations[0];
    let size = NSSize::new(f64::from(one.size[0]), f64::from(one.size[1]));
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for bitmap in &bitmap.representations {
        let pixels = image::RgbaImage::from_raw(u32::from(bitmap.size[0]), u32::from(bitmap.size[1]), bitmap.rgba.to_vec())?;
        let mut png = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png).ok()?;
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(png.get_ref()))?;
        rep.setSize(size);
        image.addRepresentation(&rep);
    }
    Some(NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, NSPoint::new(f64::from(one.hotspot[0]), f64::from(one.hotspot[1]))))
}

/// The ownership protocol also runs against a headless backend in tests.
trait Backend {
    type Window: Clone + PartialEq;
    type Cursor: Clone + PartialEq;
    fn window(&self) -> Option<Self::Window>;
    fn enabled(&self, window: &Self::Window) -> bool;
    fn suspend(&mut self, window: &Self::Window);
    fn restore(&mut self, window: &Self::Window);
    fn build(&mut self, request: &Request) -> Option<Self::Cursor>;
    fn set(&mut self, cursor: &Self::Cursor);
}

struct State<B: Backend> {
    backend: B,
    candidate: Option<B::Window>,
    cache: Vec<(Key, Option<B::Cursor>)>,
    // Retain the active object independently of cache eviction.
    active: Option<(Key, B::Window, B::Cursor)>,
}

impl<B: Backend> State<B> {
    fn new(backend: B) -> Self {
        Self { backend, candidate: None, cache: Vec::new(), active: None }
    }

    fn begin(&mut self) {
        self.candidate = self.backend.window();
    }

    fn release(&mut self) {
        self.candidate = None;
        if let Some((_, window, _)) = self.active.take() {
            self.backend.restore(&window);
        }
    }

    fn finish(&mut self, request: Option<Request>) {
        let window = self.candidate.take().filter(|w| self.backend.window().as_ref() == Some(w));
        let (Some(request), Some(window)) = (request, window) else {
            self.release();
            return;
        };
        if let Some((key, owned, cursor)) = &self.active
            && *key == request.key
            && *owned == window
        {
            // AppKit can show the system arrow when the pointer moves while
            // `NSCursor.currentCursor` (the app's last set cursor) still reads as ours, so a
            // comparison would keep the arrow. Setting the cursor again costs nothing.
            self.backend.set(cursor);
            return;
        }
        self.release();
        // Never take another component's suspension (a sheet or native tracking session).
        if !self.backend.enabled(&window) {
            return;
        }
        let cursor = if let Some((_, cursor)) = self.cache.iter().find(|(key, _)| *key == request.key) {
            cursor.clone()
        } else {
            let cursor = self.backend.build(&request);
            if self.cache.len() >= CAPACITY {
                self.cache.clear();
            }
            self.cache.push((request.key, cursor.clone()));
            cursor
        };
        if let Some(cursor) = cursor {
            self.backend.suspend(&window);
            self.backend.set(&cursor);
            self.active = Some((request.key, window, cursor));
        }
    }
}

#[derive(Default)]
struct AppKit {
    root: std::sync::Weak<winit::window::Window>,
}
impl Backend for AppKit {
    type Window = Retained<NSWindow>;
    type Cursor = Retained<NSCursor>;
    fn window(&self) -> Option<Self::Window> {
        let mtm = MainThreadMarker::new()?;
        // Bind the candidate to the host's actual root window without converting raw pointers.
        // A floating native panel may be key while egui still has stale root focus input.
        let root = self.root.upgrade()?;
        if !root.has_focus() || root.is_visible() == Some(false) || root.is_minimized() == Some(true) {
            return None;
        }
        NSApplication::sharedApplication(mtm).keyWindow().filter(|w| w.isVisible() && w.isKeyWindow())
    }
    fn enabled(&self, window: &Self::Window) -> bool {
        window.areCursorRectsEnabled()
    }
    fn suspend(&mut self, window: &Self::Window) {
        window.disableCursorRects();
    }
    fn restore(&mut self, window: &Self::Window) {
        window.enableCursorRects();
        window.resetCursorRects();
        // egui-winit may skip an unchanged fallback icon. Rebuild its stored rectangle anyway.
        if let Some(view) = window.contentView() {
            window.invalidateCursorRectsForView(&view);
        }
    }
    fn build(&mut self, request: &Request) -> Option<Self::Cursor> {
        build(&request.bitmap)
    }
    fn set(&mut self, cursor: &Self::Cursor) {
        cursor.set();
    }
}

thread_local! { static STATE: RefCell<State<AppKit>> = RefCell::new(State::new(AppKit::default())); }

fn with_state(f: impl FnOnce(&mut State<AppKit>)) {
    // All production AppKit access, including restoration, stays on the main thread.
    if MainThreadMarker::new().is_some() {
        STATE.with(|state| {
            if let Ok(mut state) = state.try_borrow_mut() {
                f(&mut state);
            }
        });
    }
}

/// Also called synchronously when native menu tracking begins and before process teardown.
pub(crate) fn release() {
    with_state(State::release);
}

struct EndedPass {
    viewport: ViewportId,
    discarded: bool,
    request: Option<Request>,
}

struct Delivery {
    begin: fn(),
    finish: fn(Option<Request>),
    release: fn(),
    tracking: fn() -> bool,
    ended: Vec<EndedPass>,
}

impl Plugin for Delivery {
    fn debug_name(&self) -> &'static str {
        "macOS canvas cursor"
    }
    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        native_request::clear(ui.ctx());
        // Floating panels must not acquire or release the main canvas's window.
        if ui.ctx().viewport_id() == ViewportId::ROOT {
            (self.begin)();
        }
    }
    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        // egui restores the parent viewport before output_hook. Capture this pass here so
        // a floating panel's output cannot consume or release its parent's cursor.
        self.ended.push(EndedPass { viewport: ui.ctx().viewport_id(), discarded: ui.ctx().will_discard(), request: native_request::take(ui.ctx()) });
    }
    fn input_hook(&mut self, _: &Context, input: &mut egui::RawInput) {
        if input.viewport_id == ViewportId::ROOT
            && (!input.focused || input.viewports.get(&ViewportId::ROOT).is_some_and(|v| v.minimized == Some(true)))
        {
            (self.release)();
        }
    }
    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        let Some(ended) = self.ended.pop() else { return };
        if ended.viewport != ViewportId::ROOT {
            return;
        }
        if ended.discarded
            || (output.platform_output.requested_discard() && output.platform_output.num_completed_passes < ctx.options(|o| o.max_passes.get()))
        {
            return;
        }
        let request = ended.request.filter(|r| {
            r.viewport == ended.viewport
                && r.pass.checked_add(1) == Some(ctx.cumulative_pass_nr_for(ended.viewport))
                && ctx.input(|i| i.focused && i.pointer.hover_pos().is_some())
                && output.platform_output.cursor_icon == r.fallback
                && output.platform_output.cursor_image.is_none()
                && !(self.tracking)()
        });
        // This is BEFORE egui-winit's platform output. In pinned winit, fallback changes update
        // the view's stored cursor and invalidate rectangles. Our owned suspension is intended
        // to keep that update from replacing the native cursor; native timing needs hand testing.
        (self.finish)(request);
    }
    fn on_exit(&mut self, _: &Context) {
        (self.release)();
    }
}

pub(crate) fn install(ctx: &Context, window: Option<&std::sync::Arc<winit::window::Window>>) {
    with_state(|state| state.backend.root = window.map(std::sync::Arc::downgrade).unwrap_or_default());
    ctx.add_plugin(Delivery {
        begin: || with_state(State::begin),
        finish: |request| with_state(|state| state.finish(request)),
        release,
        tracking: super::mac_menu::is_tracking,
        ended: Vec::new(),
    });
}

#[cfg(test)]
mod tests;
