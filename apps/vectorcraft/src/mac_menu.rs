//! The macOS menu bar, through muda (Tauri's menu crate: AppKit menus behind a safe API, no GTK
//! with default features off). The menus, their Mac layout and how their keys reach VectorCraft
//! are in `vectorcraft_ui_egui::native_menu`; this file only builds and updates the `NSMenu`s.
//!
//! - Item ids are the items' places (`vc12`, depth first): one command can run from several items
//!   (each language of VectorCraft › Language is `app.language` with its own code).
//! - A structure change (a recent file added, the language or a shortcut changed) rebuilds the
//!   menu; labels, enabled and checked are updated in place.
//! - A chosen item is a click or a key equivalent: AppKit's current event says which. A key
//!   equivalent goes back to egui as the key press that was made (the event's modifiers, not the
//!   item's: AppKit may match ⌘R to a ⇧⌘R item), so VectorCraft's key rules still apply.
//! - Hide and Hide Others are items of ours calling `NSApplication`: AppKit's own always take ⌘H
//!   (View › Hide Edges) and ⌥⌘H.

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use block2::RcBlock;

use egui::{Key, KeyboardShortcut};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType, NSMenuDidBeginTrackingNotification, NSMenuDidEndTrackingNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::native_menu::{self, Backend, Event, MenuBar, MenuRole, Node, Standard};

// AppKit can deliver window input while its system menu is still tracking (especially with
// asynchronous menu presentation). Keep the native tracking state separate from egui popups.
static TRACKING: AtomicBool = AtomicBool::new(false);
/// A tracking session began since the last [`Backend::drain`], which reports it as
/// [`Event::Opened`].
static OPENED: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_tracking() -> bool {
    TRACKING.load(Ordering::Relaxed)
}

fn observe_tracking(ctx: &egui::Context) {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // SAFETY: AppKit initializes these immutable notification-name constants.
        #[allow(unsafe_code)]
        let notifications = unsafe { [(NSMenuDidBeginTrackingNotification, true), (NSMenuDidEndTrackingNotification, false)] };
        for (name, tracking) in notifications {
            let repaint = ctx.clone();
            let block = RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
                TRACKING.store(tracking, Ordering::Relaxed);
                if tracking {
                    OPENED.store(true, Ordering::Relaxed);
                    super::mac_cursor::release();
                }
                // Report the opened menu and apply any deferred menu updates as soon as the
                // tracking session ends.
                if !tracking {
                    repaint.request_repaint();
                }
            });
            // SAFETY: these are AppKit's notification names. The notification center retains
            // the observer and block for the application's lifetime; callbacks update tracking, restore
            // owned cursor rectangles and request repaint.
            #[allow(unsafe_code)]
            unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
            }
        }
    });
}

fn cancel_tracking() {
    if let Some(mtm) = MainThreadMarker::new()
        && let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu()
    {
        menu.cancelTracking();
    }
    TRACKING.store(false, Ordering::Relaxed);
}

/// A dismissing click belongs to the menu, so the canvas and panel controls must not see it.
fn dismiss_input(raw: &mut egui::RawInput, tracking: bool) -> bool {
    if !raw
        .events
        .iter()
        .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }))
    {
        return false;
    }
    if tracking {
        raw.events.retain(|e| !matches!(e, egui::Event::PointerButton { .. } | egui::Event::Key { key: egui::Key::Escape, .. }));
    }
    true
}

/// Inspect real window input before native key equivalents and control events are added.
pub fn raw_input_hook(raw: &mut egui::RawInput) {
    // Asynchronous menu presentation may leave a displayed menu after its tracking notification.
    // Still cancel it, but consume the dismissing input only during a reported tracking session.
    if dismiss_input(raw, TRACKING.load(Ordering::Relaxed)) {
        cancel_tracking();
    }
}

enum Handle {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

/// One native item, in the order of [`MenuBar::items`].
struct Entry {
    handle: Handle,
    item: native_menu::Item,
    /// Its shortcut, when it is a key equivalent.
    chord: Option<KeyboardShortcut>,
}

/// How an item was chosen, read from AppKit's current event while it calls the handler.
struct Chosen {
    id: String,
    /// A key press (not a mouse click), and the modifiers held: ⌘, ⌃, ⌥, ⇧.
    key: bool,
    mods: [bool; 4],
}

pub struct MacMenu {
    menu: Option<Menu>,
    entries: Vec<Entry>,
    rx: Receiver<Chosen>,
    structure: u64,
    /// Latest menu state to apply after AppKit finishes tracking; item ids stay stable meanwhile.
    pending: Option<MenuBar>,
}

impl MacMenu {
    fn new(ctx: &egui::Context) -> MacMenu {
        observe_tracking(ctx);
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            // AppKit calls this on the main thread, from the event that chose the item: a key
            // equivalent is a key-down, a click a mouse-up (or ↩ inside the open menu).
            let event = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent());
            let key = event.as_ref().is_some_and(|ev| ev.r#type() == NSEventType::KeyDown);
            let flags = event.as_ref().map_or(NSEventModifierFlags::empty(), |ev| ev.modifierFlags());
            let mods = [NSEventModifierFlags::Command, NSEventModifierFlags::Control, NSEventModifierFlags::Option, NSEventModifierFlags::Shift]
                .map(|m| flags.contains(m));
            // The receiver only goes away with the app.
            let _ = tx.send(Chosen { id: e.id.0, key, mods });
            // Wake egui, so the item runs now rather than on the next mouse move.
            repaint.request_repaint();
        }));
        MacMenu { menu: None, entries: Vec::new(), rx, structure: 0, pending: None }
    }

    /// Build the menus of `bar` and make them the app's, in place of the ones before.
    fn build(&mut self, bar: &MenuBar) -> muda::Result<()> {
        let mut entries = Vec::new();
        let menu = Menu::new();
        let mut special = Vec::new();
        for m in &bar.menus {
            let sub = Submenu::new(escape(&m.title), true);
            append(&sub, &m.children, &mut entries)?;
            menu.append(&sub)?;
            if matches!(m.role, MenuRole::Window | MenuRole::Help) {
                special.push((m.role, sub));
            }
        }
        if let Some(old) = self.menu.take() {
            old.remove_for_nsapp();
        }
        menu.init_for_nsapp();
        // Only once the menu is the app's: muda looks the submenus up in the installed main menu.
        for (role, sub) in special {
            match role {
                MenuRole::Window => sub.set_as_windows_menu_for_nsapp(),
                _ => sub.set_as_help_menu_for_nsapp(),
            }
        }
        self.menu = Some(menu);
        self.entries = entries;
        self.structure = native_menu::structure_key(bar);
        Ok(())
    }
}

fn append(sub: &Submenu, nodes: &[Node], entries: &mut Vec<Entry>) -> muda::Result<()> {
    for n in nodes {
        match n {
            Node::Separator => sub.append(&PredefinedMenuItem::separator())?,
            Node::Header(label) => sub.append(&MenuItem::new(escape(label), false, None))?,
            Node::Submenu { label, children } => {
                let s = Submenu::new(escape(label), !children.is_empty());
                append(&s, children, entries)?;
                sub.append(&s)?;
            }
            Node::Standard(item) => {
                let text = Some(vectorcraft_ui_egui::i18n::t(item.label()));
                let p = match item {
                    Standard::Services => PredefinedMenuItem::services(text),
                    Standard::ShowAll => PredefinedMenuItem::show_all(text),
                    Standard::Zoom => PredefinedMenuItem::maximize(text),
                    Standard::BringAllToFront => PredefinedMenuItem::bring_all_to_front(text),
                };
                sub.append(&p)?;
            }
            Node::Item(it) => {
                let id = format!("vc{}", entries.len());
                let chord = it.shortcut.and_then(vectorcraft_ui_egui::shortcuts::parse).filter(native_menu::native_ok);
                let accel = chord.as_ref().and_then(accelerator);
                let handle = match it.checked {
                    Some(c) => {
                        let h = CheckMenuItem::with_id(id, escape(&it.label), it.enabled, c, accel);
                        sub.append(&h)?;
                        Handle::Check(h)
                    }
                    None => {
                        let h = MenuItem::with_id(id, escape(&it.label), it.enabled, accel);
                        sub.append(&h)?;
                        Handle::Plain(h)
                    }
                };
                entries.push(Entry { handle, item: it.clone(), chord });
            }
        }
    }
    Ok(())
}

impl Backend for MacMenu {
    fn sync(&mut self, bar: &MenuBar) {
        // Replacing or mutating an NSMenu during tracking can strand its displayed menu. Keep
        // the latest state: the shared model considers this sync complete and may not resend it.
        if TRACKING.load(Ordering::Relaxed) {
            self.pending = Some(bar.clone());
            return;
        }
        self.pending = None;
        let items = bar.items();
        if self.menu.is_none() || native_menu::structure_key(bar) != self.structure || items.len() != self.entries.len() {
            if let Err(e) = self.build(bar) {
                // The menus as they were stay up (a failed build changed nothing).
                log::warn!("couldn't rebuild the macOS menu bar: {e}");
            }
            return;
        }
        for (e, it) in self.entries.iter_mut().zip(items) {
            if e.item.label != it.label {
                let text = escape(&it.label);
                match &e.handle {
                    Handle::Plain(h) => h.set_text(text),
                    Handle::Check(h) => h.set_text(text),
                }
            }
            if e.item.enabled != it.enabled {
                match &e.handle {
                    Handle::Plain(h) => h.set_enabled(it.enabled),
                    Handle::Check(h) => h.set_enabled(it.enabled),
                }
            }
            // Compare with AppKit's own state: muda toggles a check item when it's clicked.
            if let (Handle::Check(h), Some(c)) = (&e.handle, it.checked)
                && h.is_checked() != c
            {
                h.set_checked(c);
            }
            if e.item != *it {
                e.item = it.clone();
            }
        }
    }

    fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if OPENED.swap(false, Ordering::Relaxed) {
            out.push(Event::Opened);
        }
        while let Ok(chosen) = self.rx.try_recv() {
            cancel_tracking();
            // Ids that aren't ours are AppKit's own items (Services, Zoom…), already handled.
            let Some(e) = chosen.id.strip_prefix("vc").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.entries.get(i)) else { continue };
            if let Some(command @ (native_menu::HIDE | native_menu::HIDE_OTHERS)) = e.item.command {
                if let Some(mtm) = MainThreadMarker::new() {
                    let app = NSApplication::sharedApplication(mtm);
                    if command == native_menu::HIDE { app.hide(None) } else { app.hideOtherApplications(None) }
                }
                continue;
            }
            // A key-down chose it: its key equivalent, unless it was ↩ or Space inside the open
            // menu (no ⌘ or ⌃ while the item's shortcut has one). The key is the item's (AppKit
            // matched on it); the modifiers are the ones held.
            let [cmd, ctrl, alt, shift] = chosen.mods;
            let key = e
                .chord
                .filter(|c| {
                    chosen.key && e.item.command != Some(native_menu::MINIMIZE) && (cmd || ctrl || !(c.modifiers.command || c.modifiers.ctrl))
                })
                .map(|c| KeyboardShortcut::new(egui::Modifiers { alt, ctrl, shift, mac_cmd: cmd, command: cmd }, c.logical_key));
            out.push(match key {
                Some(k) => Event::Key(k),
                None => Event::Click(e.item.clone()),
            });
        }
        // Drain against the old item ids before a deferred structure change replaces them.
        if !TRACKING.load(Ordering::Relaxed)
            && let Some(bar) = self.pending.take()
        {
            self.sync(&bar);
        }
        out
    }
}

/// Install the menu bar (in eframe's creator closure, so winit's default menu doesn't stay up).
/// `None` keeps the menus in the window: AppKit refused them (logged), never a crash.
pub fn install(ctx: &egui::Context, app: &VectorcraftApp) -> Option<native_menu::NativeMenu> {
    vectorcraft_ui_egui::i18n::set_current(app.ui_language());
    let bar = native_menu::layout(app).bar;
    let built = vectorcraft_engine::guard::catch_panic(|| {
        let mut menu = MacMenu::new(ctx);
        menu.build(&bar).map(|()| menu)
    });
    match built {
        Ok(Ok(menu)) => Some(native_menu::NativeMenu::new(Box::new(menu))),
        Ok(Err(e)) => {
            log::warn!("couldn't install the macOS menu bar ({e}); the menus stay in the window");
            None
        }
        Err(e) => {
            log::warn!("couldn't install the macOS menu bar ({e}); the menus stay in the window");
            None
        }
    }
}

/// `&` marks a mnemonic in muda labels: "Fill & Stroke" keeps its ampersand.
fn escape(s: &str) -> String {
    s.replace('&', "&&")
}

/// A key equivalent for AppKit. `Cmd` is ⌘ (`META`) and `Ctrl` is ⌃: never folded together.
fn accelerator(c: &KeyboardShortcut) -> Option<Accelerator> {
    let m = c.modifiers;
    let mut mods = Modifiers::empty();
    for (on, flag) in [(m.command || m.mac_cmd, Modifiers::META), (m.ctrl, Modifiers::CONTROL), (m.alt, Modifiers::ALT), (m.shift, Modifiers::SHIFT)]
    {
        if on {
            mods |= flag;
        }
    }
    Some(Accelerator::new(mods, code(c.logical_key)?))
}

/// The key code AppKit matches for an egui key.
fn code(key: Key) -> Option<Code> {
    Some(match key {
        Key::A => Code::KeyA,
        Key::B => Code::KeyB,
        Key::C => Code::KeyC,
        Key::D => Code::KeyD,
        Key::E => Code::KeyE,
        Key::F => Code::KeyF,
        Key::G => Code::KeyG,
        Key::H => Code::KeyH,
        Key::I => Code::KeyI,
        Key::J => Code::KeyJ,
        Key::K => Code::KeyK,
        Key::L => Code::KeyL,
        Key::M => Code::KeyM,
        Key::N => Code::KeyN,
        Key::O => Code::KeyO,
        Key::P => Code::KeyP,
        Key::Q => Code::KeyQ,
        Key::R => Code::KeyR,
        Key::S => Code::KeyS,
        Key::T => Code::KeyT,
        Key::U => Code::KeyU,
        Key::V => Code::KeyV,
        Key::W => Code::KeyW,
        Key::X => Code::KeyX,
        Key::Y => Code::KeyY,
        Key::Z => Code::KeyZ,
        Key::Num0 => Code::Digit0,
        Key::Num1 => Code::Digit1,
        Key::Num2 => Code::Digit2,
        Key::Num3 => Code::Digit3,
        Key::Num4 => Code::Digit4,
        Key::Num5 => Code::Digit5,
        Key::Num6 => Code::Digit6,
        Key::Num7 => Code::Digit7,
        Key::Num8 => Code::Digit8,
        Key::Num9 => Code::Digit9,
        Key::F1 => Code::F1,
        Key::F2 => Code::F2,
        Key::F3 => Code::F3,
        Key::F4 => Code::F4,
        Key::F5 => Code::F5,
        Key::F6 => Code::F6,
        Key::F7 => Code::F7,
        Key::F8 => Code::F8,
        Key::F9 => Code::F9,
        Key::F10 => Code::F10,
        Key::F11 => Code::F11,
        Key::F12 => Code::F12,
        Key::F13 => Code::F13,
        Key::F14 => Code::F14,
        Key::F15 => Code::F15,
        Key::F16 => Code::F16,
        Key::F17 => Code::F17,
        Key::F18 => Code::F18,
        Key::F19 => Code::F19,
        Key::F20 => Code::F20,
        Key::F21 => Code::F21,
        Key::F22 => Code::F22,
        Key::F23 => Code::F23,
        Key::F24 => Code::F24,
        Key::OpenBracket => Code::BracketLeft,
        Key::CloseBracket => Code::BracketRight,
        Key::Backslash => Code::Backslash,
        Key::Slash => Code::Slash,
        // `+` is Shift+`=` on the keyboards AppKit matches by key: Zoom In shows ⌘=.
        Key::Equals | Key::Plus => Code::Equal,
        Key::Minus => Code::Minus,
        Key::Quote => Code::Quote,
        Key::Semicolon => Code::Semicolon,
        Key::Comma => Code::Comma,
        Key::Period => Code::Period,
        Key::Backtick => Code::Backquote,
        Key::Backspace => Code::Backspace,
        Key::Delete => Code::Delete,
        Key::Enter => Code::Enter,
        Key::Tab => Code::Tab,
        Key::Space => Code::Space,
        Key::Escape => Code::Escape,
        Key::ArrowLeft => Code::ArrowLeft,
        Key::ArrowRight => Code::ArrowRight,
        Key::ArrowUp => Code::ArrowUp,
        Key::ArrowDown => Code::ArrowDown,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shortcut the Mac menu shows as a key equivalent has one AppKit can match.
    #[test]
    fn every_menu_shortcut_maps_to_a_key_equivalent() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let bar = native_menu::layout(&app).bar;
        let mut native = 0;
        for it in bar.items() {
            let Some(c) = it.shortcut.and_then(vectorcraft_ui_egui::shortcuts::parse).filter(native_menu::native_ok) else { continue };
            assert!(accelerator(&c).is_some(), "{:?} ({:?}) has no key equivalent", it.shortcut, it.command);
            native += 1;
        }
        assert!(native > 70, "{native} key equivalents");
    }

    #[test]
    fn control_and_command_stay_distinct() {
        let parse = |s| vectorcraft_ui_egui::shortcuts::parse(s).and_then(|c| accelerator(&c));
        assert_ne!(parse("Cmd+M"), parse("Ctrl+Cmd+M"));
        assert!(parse("Cmd+M").is_some());
    }

    fn press() -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(300.0, 200.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn outside_press_dismisses_without_starting_a_canvas_gesture() {
        let moved = egui::Event::PointerMoved(egui::pos2(300.0, 200.0));
        let mut raw = egui::RawInput { events: vec![moved.clone(), press()], ..Default::default() };
        assert!(dismiss_input(&mut raw, true));
        assert_eq!(raw.events, [moved]);
        assert!(!dismiss_input(&mut raw, true));
        raw.events.push(press());
        assert!(dismiss_input(&mut raw, false));
        assert_eq!(raw.events.len(), 2, "ordinary canvas presses still reach egui");
    }

    #[test]
    fn escape_dismisses_without_reaching_the_active_tool() {
        let escape = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        let mut raw = egui::RawInput { events: vec![escape(true), escape(false)], ..Default::default() };
        assert!(dismiss_input(&mut raw, true));
        assert!(raw.events.is_empty());
    }

    #[test]
    fn moving_the_pointer_does_not_dismiss_a_tracking_menu() {
        let mut raw = egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0))], ..Default::default() };
        assert!(!dismiss_input(&mut raw, true));
        assert_eq!(raw.events.len(), 1);
    }
}
