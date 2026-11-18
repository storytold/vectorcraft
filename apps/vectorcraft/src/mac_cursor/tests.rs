use super::*;
use egui::{CursorIcon, RawInput};
use std::cell::Cell;

#[derive(Default)]
struct Fake {
    window: Option<u8>,
    suspended: Vec<u8>,
    sets: usize,
    builds: usize,
    restores: Vec<u8>,
    fail: bool,
}
impl Backend for Fake {
    type Window = u8;
    type Cursor = Key;
    fn window(&self) -> Option<u8> {
        self.window
    }
    fn enabled(&self, w: &u8) -> bool {
        !self.suspended.contains(w)
    }
    fn suspend(&mut self, w: &u8) {
        self.suspended.push(*w);
    }
    fn restore(&mut self, w: &u8) {
        self.suspended.retain(|v| v != w);
        self.restores.push(*w);
    }
    fn build(&mut self, r: &Request) -> Option<Key> {
        self.builds += 1;
        (!self.fail).then_some(r.key)
    }
    fn set(&mut self, _: &Key) {
        self.sets += 1;
    }
}
fn fake_state() -> State<Fake> {
    State::new(Fake { window: Some(1), ..Default::default() })
}
fn raw(id: ViewportId, focused: bool) -> RawInput {
    let mut input = RawInput { viewport_id: id, focused, events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))], ..Default::default() };
    input.viewports.entry(id).or_default();
    input
}
fn publish(ctx: &Context, scale: f32) {
    let mut session = vectorcraft_engine::Session::new();
    let cursor = session.cursor(Default::default(), Default::default(), Default::default());
    let fallback = native_request::request(ctx, cursor, CursorIcon::Default, true, scale);
    ctx.set_cursor_icon(fallback);
}
fn request(scale: f32) -> Request {
    let ctx = Context::default();
    let _ = headless(&ctx, raw(ViewportId::ROOT, true), |ui| publish(ui.ctx(), scale));
    native_request::take(&ctx).unwrap()
}
fn deliver(state: &mut State<Fake>, request: Option<Request>) {
    state.begin();
    state.finish(request);
}

#[test]
fn mac_cursor_ownership_reasserts_every_pass_and_restores() {
    let req = request(1.0);
    let mut state = fake_state();
    deliver(&mut state, Some(req.clone()));
    assert_eq!(state.backend.suspended, [1]);
    assert_eq!((state.backend.builds, state.backend.sets), (1, 1));
    // The pointer moves and AppKit shows the system arrow, but the app's current cursor
    // still reads as ours. The next pass sets it again without rebuilding or re-suspending.
    deliver(&mut state, Some(req.clone()));
    assert_eq!((state.backend.builds, state.backend.sets, state.backend.suspended.len()), (1, 2, 1));
    deliver(&mut state, None);
    assert_eq!(state.backend.restores, [1]);
    deliver(&mut state, Some(req.clone()));
    state.backend.window = None;
    deliver(&mut state, Some(req.clone()));
    assert!(state.active.is_none());
    state.backend.window = Some(1);
    deliver(&mut state, Some(req.clone()));
    state.begin();
    state.backend.window = Some(2);
    state.finish(Some(req));
    assert!(state.active.is_none(), "never acquire a different key window mid-pass");
    assert_eq!(state.backend.restores, [1, 1, 1]);
}

#[test]
fn mac_cursor_respects_other_suspensions_failures_and_cache_eviction() {
    let mut state = fake_state();
    state.backend.suspended.push(1);
    deliver(&mut state, Some(request(1.0)));
    assert_eq!(state.backend.builds, 0);
    state.release();
    assert_eq!(state.backend.suspended, [1]);
    assert!(state.backend.restores.is_empty());
    state.backend.suspended.clear();
    state.backend.fail = true;
    for _ in 0..2 {
        deliver(&mut state, Some(request(1.0)));
    }
    assert_eq!(state.backend.builds, 1, "cache failed constructions");
    state.backend.fail = false;
    for i in 1..140 {
        deliver(&mut state, Some(request(1.0 + i as f32 / 1000.0)));
    }
    assert!(state.cache.len() <= CAPACITY);
    let active = state.active.unwrap();
    state.cache.clear();
    assert_eq!(state.active, Some(active));
    state.release();
    assert!(state.backend.suspended.is_empty());
}

thread_local! {
    static FAKE: RefCell<State<Fake>> = RefCell::new(fake_state());
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}
fn install_fake(ctx: &Context) {
    ctx.add_plugin(Delivery {
        begin: || FAKE.with(|s| s.borrow_mut().begin()),
        finish: |r| FAKE.with(|s| s.borrow_mut().finish(r)),
        release: || FAKE.with(|s| s.borrow_mut().release()),
        tracking: || TRACKING.with(Cell::get),
        ended: Vec::new(),
    });
}
#[test]
fn mac_cursor_plugin_consumes_passes_and_releases_for_menu_focus_and_overrides() {
    FAKE.with(|s| *s.borrow_mut() = fake_state());
    TRACKING.with(|t| t.set(false));
    let ctx = Context::default();
    install_fake(&ctx);
    install_fake(&ctx);
    let root = ViewportId::ROOT;
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    FAKE.with(|s| assert_eq!(s.borrow().backend.sets, 1, "one installed plugin consumes the request"));
    let _ = headless(&ctx, raw(ViewportId::from_hash_of("floating-panel"), true), |_| {});
    FAKE.with(|s| assert!(s.borrow().active.is_some()));
    let _ = headless(&ctx, raw(root, true), |_| {});
    FAKE.with(|s| assert!(s.borrow().active.is_none(), "empty pass hands back even an unchanged Default icon"));
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    let _ = headless(&ctx, raw(root, true), |ui| {
        publish(ui.ctx(), 1.0);
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    });
    FAKE.with(|s| assert!(s.borrow().active.is_none(), "changed-icon handback"));
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    // The notification calls release without waiting for another UI pass.
    TRACKING.with(|t| t.set(true));
    FAKE.with(|s| s.borrow_mut().release());
    FAKE.with(|s| assert!(s.borrow().active.is_none()));
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    FAKE.with(|s| assert!(s.borrow().active.is_none()));
    TRACKING.with(|t| t.set(false));
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    let _ = headless(&ctx, raw(root, false), |_| {});
    FAKE.with(|s| assert!(s.borrow().active.is_none()));
    let _ = headless(&ctx, raw(root, true), |ui| publish(ui.ctx(), 1.0));
    FAKE.with(|s| assert!(s.borrow().active.is_some()));
}

#[test]
fn mac_cursor_discarded_pass_does_not_commit_and_installation_does_not_repaint() {
    FAKE.with(|s| *s.borrow_mut() = fake_state());
    TRACKING.with(|t| t.set(false));
    let ctx = Context::default();
    install_fake(&ctx);
    let mut passes = 0;
    let _ = headless(&ctx, raw(ViewportId::ROOT, true), |ui| {
        passes += 1;
        if passes == 1 {
            publish(ui.ctx(), 1.0);
            ui.ctx().request_discard("cursor regression");
        }
    });
    assert_eq!(passes, 2);
    FAKE.with(|s| assert_eq!(s.borrow().backend.sets, 0));
    let idle = || {
        let mut input = raw(ViewportId::ROOT, true);
        input.events.clear();
        input.time = Some(ctx.input(|i| i.time) + 0.1);
        input
    };
    for _ in 0..3 {
        let _ = headless(&ctx, idle(), |ui| publish(ui.ctx(), 1.0));
    }
    let out = headless(&ctx, idle(), |ui| publish(ui.ctx(), 1.0));
    assert!(out.viewport_output[&ViewportId::ROOT].repaint_delay > std::time::Duration::ZERO);
}

#[test]
fn mac_cursor_appkit_representations_have_pixel_sizes_and_point_hotspots() {
    // Only construct and inspect image/cursor objects: no NSApplication, window or cursor.set().
    for scale in [0.75, 1.0, 1.25, 1.5, 2.0] {
        let request = request(scale);
        let cursor = build(&request.bitmap).unwrap();
        let image = cursor.image();
        let one = &request.bitmap.representations[0];
        let size = NSSize::new(f64::from(one.size[0]), f64::from(one.size[1]));
        assert_eq!(image.size(), size);
        assert_eq!(cursor.hotSpot(), NSPoint::new(f64::from(one.hotspot[0]), f64::from(one.hotspot[1])));
        let reps = image.representations();
        assert_eq!(reps.len(), 2);
        let mut dimensions: Vec<_> = reps
            .iter()
            .map(|rep| {
                assert_eq!(rep.size(), size);
                (rep.pixelsWide(), rep.pixelsHigh())
            })
            .collect();
        dimensions.sort_unstable();
        assert_eq!(
            dimensions,
            [
                (isize::try_from(one.size[0]).unwrap(), isize::try_from(one.size[1]).unwrap()),
                (isize::try_from(one.size[0]).unwrap() * 2, isize::try_from(one.size[1]).unwrap() * 2)
            ]
        );
    }
}

fn headless(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, draw);
    output.textures_delta.clear();
    output
}
