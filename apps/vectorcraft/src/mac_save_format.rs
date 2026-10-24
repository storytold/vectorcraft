//! macOS: the Format menu on save sheets (#866). A save panel with two or more file types (Save As,
//! Save a Copy, Save as Template and the first Save of a new document) is an `NSSavePanel` made here
//! instead of by rfd, whose Cocoa backend lists the extensions of every filter but not their names.
//! The panel gets a Format menu as its accessory view
//! (<https://developer.apple.com/documentation/appkit/nssavepanel/accessoryview>) before
//! `beginSheetModalForWindow:completionHandler:`
//! (<https://developer.apple.com/documentation/appkit/nssavepanel/beginsheetmodal(for:completionhandler:)>)
//! shows it: an accessory set after the sheet began made recent macOS versions dismiss the sheet at
//! once (#1800, #1831, #1518). Choosing a format changes the extension in the name field and puts
//! the format's extension first in the allowed file types, which AppKit adds to a name typed without
//! one. The save command takes the format from the extension, so an extension typed by hand still
//! decides it.
//!
//! The panel retains the accessory, and the completion handler holds the panel until the sheet
//! closes; the popup's target is the accessory itself. The accessory holds the panel weakly, so
//! closing the sheet releases both. Reading the window from its handle and subclassing `NSView` for
//! the menu's action have no safe API; this module allows `unsafe_code`.
#![allow(unsafe_code)]

use std::sync::mpsc::{Receiver, channel};

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSPopUpButton, NSSavePanel, NSTextField, NSView};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_ui_egui::{FileFilter as Filter, FilePick, i18n};

/// The accessory's height and the room around and between its label and menu, in points.
const HEIGHT: f64 = 38.0;
const MARGIN: f64 = 12.0;
const GAP: f64 = 8.0;
/// The Format menu's size, in points.
const MENU: NSSize = NSSize::new(260.0, 26.0);

struct Ivars {
    panel: Weak<NSSavePanel>,
    filters: Vec<Filter>,
}

define_class!(
    // SAFETY: NSView has no extra subclassing requirements; the subclass implements no Drop.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "VectorcraftSaveFormatAccessory"]
    struct FormatAccessory;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for FormatAccessory {}

    impl FormatAccessory {
        // SAFETY: AppKit's target/action sends the popup as the single object argument.
        #[unsafe(method(chooseFormat:))]
        fn choose_format(&self, sender: &NSPopUpButton) {
            let Some(panel) = self.ivars().panel.load() else { return };
            let Some(index) = usize::try_from(sender.indexOfSelectedItem()).ok() else { return };
            let Some(extension) = self.ivars().filters.get(index).and_then(|(_, extensions)| extensions.first()) else { return };
            set_allowed_types(&panel, &self.ivars().filters, index);
            let name = panel.nameFieldStringValue().to_string();
            panel.setNameFieldStringValue(&NSString::from_str(&filename(&name, extension, &self.ivars().filters)));
        }
    }
);

/// The file types a Format menu lists: `filters` without "every file" ones. Empty when fewer than
/// two remain, and the panel is then left to rfd.
fn menu_formats(filters: &[Filter]) -> Vec<Filter> {
    let named: Vec<Filter> =
        filters.iter().filter(|(_, extensions)| !extensions.is_empty() && !extensions.iter().any(|x| x == "*")).cloned().collect();
    if named.len() < 2 { Vec::new() } else { named }
}

/// The menu entry of the suggested name's type, or the first (the request's default).
fn initial_format(name: &str, filters: &[Filter]) -> usize {
    let extension = fileio::extension(name);
    filters.iter().position(|(_, extensions)| extensions.iter().any(|x| extension.eq_ignore_ascii_case(x))).unwrap_or(0)
}

/// `name` with `extension`: a format's extension (one in the menu, or any other the app reads or
/// writes) is replaced, and other dots stay part of the name (a version number).
fn filename(name: &str, extension: &str, filters: &[Filter]) -> String {
    if name.is_empty() {
        return String::new();
    }
    let has = fileio::extension(name);
    let known =
        filters.iter().any(|(_, extensions)| extensions.iter().any(|x| has.eq_ignore_ascii_case(x))) || fileio::format_for_name(name).is_some();
    if known { std::path::Path::new(name).with_extension(extension).to_string_lossy().into_owned() } else { format!("{name}.{extension}") }
}

/// The extensions a panel with menu `filters` allows, those of entry `selected` first. AppKit adds
/// the first one to a name typed without an extension, and a name typed with another of them is
/// saved in that format.
fn allowed_types(filters: &[Filter], selected: usize) -> Vec<&str> {
    let first = filters.get(selected).map_or(&[][..], |(_, extensions)| extensions.as_slice());
    let others = filters.iter().enumerate().filter(|(i, _)| *i != selected).flat_map(|(_, (_, extensions))| extensions.iter());
    first.iter().chain(others).map(String::as_str).collect()
}

/// Set `panel`'s allowed file types ([`allowed_types`]).
fn set_allowed_types(panel: &NSSavePanel, filters: &[Filter], selected: usize) {
    let types: Vec<Retained<NSString>> = allowed_types(filters, selected).into_iter().map(NSString::from_str).collect();
    // rfd sets the same deprecated property (`add_filters` in its `panel_ffi.rs`).
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
}

/// Show the save panel for `pick` as a sheet on `window`, with a Format menu, and answer on the
/// receiver with the path picked (none when canceled). `None` when the request has fewer than two
/// named file types, or `window` has no AppKit window; nothing is shown then.
pub fn begin(window: &winit::window::Window, pick: &FilePick) -> Option<Receiver<Vec<String>>> {
    let mtm = MainThreadMarker::new()?;
    let filters = menu_formats(&pick.filters);
    if filters.is_empty() {
        return None;
    }
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: the handle's view is the window's content view, which AppKit keeps alive while the
    // window is open, and winit hands out the handle only while it is.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let parent = view.window()?;
    let panel = NSSavePanel::savePanel(mtm);
    let selected = initial_format(&pick.name, &filters);
    set_allowed_types(&panel, &filters, selected);
    if let Some(folder) = &pick.folder {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true)));
    }
    panel.setExtensionHidden(false);
    if !pick.name.is_empty() {
        panel.setNameFieldStringValue(&NSString::from_str(&pick.name));
    }
    let accessory = accessory(mtm, &panel, filters, selected);
    panel.setAccessoryView(Some(&accessory));
    let (tx, rx) = channel();
    let answered = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let picked = answered.URL().and_then(|url| url.path()).map(|p| p.to_string());
        let paths = if response == NSModalResponseOK { picked.into_iter().collect() } else { Vec::new() };
        // The app gone meanwhile has no use for the answer.
        let _ = tx.send(paths);
    });
    // The panel and its accessory are set up; AppKit keeps the handler (and the panel it holds)
    // until the sheet closes.
    panel.beginSheetModalForWindow_completionHandler(&parent, &handler);
    Some(rx)
}

/// The "Format:" label and menu of `filters`, `selected` chosen, that rename `panel`'s file.
fn accessory(mtm: MainThreadMarker, panel: &NSSavePanel, filters: Vec<Filter>, selected: usize) -> Retained<FormatAccessory> {
    let label = NSTextField::labelWithString(&NSString::from_str(i18n::t("Format:")), mtm);
    label.sizeToFit();
    let size = label.frame().size;
    label.setFrameOrigin(NSPoint::new(MARGIN, (HEIGHT - size.height) / 2.0));
    let x = MARGIN + size.width + GAP;
    let popup =
        NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::new(NSPoint::new(x, (HEIGHT - MENU.height) / 2.0), MENU), false);
    for (name, _) in &filters {
        popup.addItemWithTitle(&NSString::from_str(i18n::tr_ctx(i18n::current(), "file-filter", name)));
    }
    // The filter list comes from the built-in format registry, small enough for NSInteger.
    if let Ok(index) = isize::try_from(selected) {
        popup.selectItemAtIndex(index);
    }
    let this = FormatAccessory::alloc(mtm).set_ivars(Ivars { panel: Weak::new(panel), filters });
    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(x + MENU.width + MARGIN, HEIGHT));
    // SAFETY: NSView's designated initializer is called once on the allocated subclass.
    let accessory: Retained<FormatAccessory> = unsafe { msg_send![super(this), initWithFrame: frame] };
    // SAFETY: the accessory holds the popup as a subview and AppKit keeps the accessory while the
    // sheet is open. The selector has the target/action signature NSControl calls it with.
    unsafe {
        popup.setTarget(Some(&accessory));
        popup.setAction(Some(sel!(chooseFormat:)));
    }
    accessory.addSubview(&label);
    accessory.addSubview(&popup);
    accessory
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters() -> Vec<Filter> {
        FilePick::filters([("VectorCraft", &["vectorcraft"][..]), ("SVG", &["svg"]), ("PDF", &["pdf"])])
    }

    /// Save, Save As and Save a Copy list every save format with the default first; a panel with one
    /// named type, or one for every file, gets no menu, and rfd shows it.
    #[test]
    fn save_panels_with_several_formats_get_the_menu() {
        let save = menu_formats(&FilePick::filters(fileio::save_filters("vectorcraft")));
        assert!(save.len() >= 2, "{save:?}");
        assert_eq!(save.first().map(|(_, e)| e.as_slice()), Some(&["vectorcraft".to_owned()][..]));
        assert!(menu_formats(&FilePick::filters(fileio::save_filters("svg"))).first().is_some_and(|(_, e)| e.iter().any(|e| e == "svg")));
        assert!(menu_formats(&FilePick::filters(fileio::template_filters("template"))).len() >= 2);
        assert!(menu_formats(&FilePick::filters([("PDF", &["pdf"][..])])).is_empty());
        assert!(menu_formats(&FilePick::filters([("PDF", &["pdf"][..]), ("All files", &["*"]), ("Any", &[])])).is_empty());
        assert_eq!(menu_formats(&FilePick::filters([("PDF", &["pdf"][..]), ("All files", &["*"]), ("SVG", &["svg"])])).len(), 2);
    }

    #[test]
    fn the_suggested_name_selects_its_format_and_an_unknown_one_the_default() {
        for (name, index) in [("Untitled-1", 0), ("a b.SVG", 1), ("art.pdf", 2), ("version.2", 0), ("", 0)] {
            assert_eq!(initial_format(name, &filters()), index, "{name}");
        }
    }

    #[test]
    fn choosing_a_format_changes_the_extension_and_keeps_the_name() {
        for (name, expected) in [
            ("図面 1.vectorcraft", "図面 1.svg"),
            ("a.PDF", "a.svg"),
            ("art", "art.svg"),
            ("art.v2", "art.v2.svg"),
            ("art.svg", "art.svg"),
            ("old.drawcraft", "old.svg"),
            ("photo.png", "photo.svg"),
            ("", ""),
        ] {
            assert_eq!(filename(name, "svg", &filters()), expected, "{name}");
        }
    }

    /// A name typed without an extension gets the chosen format's: AppKit adds the first allowed
    /// type. The others stay allowed.
    #[test]
    fn the_chosen_format_is_the_first_allowed_type() {
        assert_eq!(allowed_types(&filters(), 0), ["vectorcraft", "svg", "pdf"]);
        assert_eq!(allowed_types(&filters(), 1), ["svg", "vectorcraft", "pdf"]);
        assert_eq!(allowed_types(&filters(), 2), ["pdf", "vectorcraft", "svg"]);
        assert_eq!(allowed_types(&filters(), 9), ["vectorcraft", "svg", "pdf"]);
    }

    /// Each entry of the Save As menu gives a name the save command reads as that entry's format.
    #[test]
    fn each_menu_entry_names_a_file_the_save_command_writes_in_that_format() {
        let filters = menu_formats(&FilePick::filters(fileio::save_filters("vectorcraft")));
        for (label, extensions) in &filters {
            let name = filename("Untitled-1.vectorcraft", &extensions[0], &filters);
            assert_eq!(fileio::format_for_name(&name).map(|f| f.label), Some(label.as_str()), "{name}");
        }
    }
}
