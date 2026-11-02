use super::{Arc, ClipboardProbeFactory, Mutex, SystemClipboard, background, native_menu, picks, place, print, screen_color, stylus};

/// A file type a dialog filters by: its label, which shows translated, and its extensions without
/// the dot.
pub type FileFilter = (String, Vec<String>);

/// What a file dialog shows: a suggested file name (save dialogs), the folder to start in and the
/// file-type filters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilePick {
    pub name: String,
    pub folder: Option<String>,
    /// The default type first; empty = every file.
    pub filters: Vec<FileFilter>,
}

impl FilePick {
    /// A save dialog suggesting `name`.
    pub fn named(name: &str) -> Self {
        Self { name: name.to_string(), ..Default::default() }
    }

    /// The file type `label` with `extensions`.
    pub fn filter(label: &str, extensions: &[&str]) -> FileFilter {
        (label.to_owned(), extensions.iter().map(|e| (*e).to_owned()).collect())
    }

    /// The file types of `list`, as the engine lists them: `(label, extensions)`.
    pub fn filters<'a>(list: impl IntoIterator<Item = (&'a str, &'a [&'a str])>) -> Vec<FileFilter> {
        list.into_iter().map(|(label, extensions)| Self::filter(label, extensions)).collect()
    }
}

pub type PickOpen = Box<dyn FnMut(&FilePick) -> Option<String>>;

pub type PickSave = Box<dyn FnMut(&FilePick) -> Option<String>>;

pub type ReadFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;

pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

pub type ExistsFn = Box<dyn Fn(&str) -> bool>;

pub type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Starts a browser download of the bytes under a file name; an error when it couldn't be started
/// (the save or export then fails as a refused write does: #1693).
pub type DownloadFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// Opens a URL in the system browser.
pub type OpenUrlFn = Box<dyn FnMut(&str)>;

/// Whether a URL from a document may open in the browser: an `http:`, `https:` or `mailto:` URL
/// without control characters. The system hands other schemes (`file:`, application schemes) to
/// other programs.
pub(crate) fn browsable_url(url: &str) -> Result<(), String> {
    let scheme = url.split_once(':').map_or("", |(s, _)| s);
    if ["http", "https", "mailto"].iter().any(|k| scheme.eq_ignore_ascii_case(k)) && !url.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(format!("only http, https and mailto URLs open in the browser, not “{url}”"))
    }
}

/// Shows a file in the system file manager, or opens it in its app.
pub type RevealFn = Box<dyn FnMut(&str) -> Result<(), String>>;

/// Platform services injected by the host app (desktop or web).
#[derive(Default)]
pub struct Services {
    /// Show an open dialog; returns a path.
    pub pick_open: Option<PickOpen>,
    /// Show a save dialog (suggested name, folder, file types); returns a path.
    pub pick_save: Option<PickSave>,
    pub read: Option<ReadFn>,
    pub write: Option<WriteFn>,
    /// Is there a file or folder at a path (desktop)? The app asks before replacing a file that a
    /// save panel's name names once the app gives it an extension ([`picks::ask_to_replace`]).
    /// Without it, no file counts as there.
    pub exists: Option<ExistsFn>,
    /// Files that arrived asynchronously (web open / drops): (name, bytes).
    pub inbox: Option<Inbox>,
    /// Web: trigger a browser download instead of writing a path.
    pub download: Option<DownloadFn>,
    /// Web: start an async open (bytes arrive via `inbox`).
    pub open_async: Option<Box<dyn FnMut()>>,
    /// Read the system clipboard's text (desktop). Without it, pasted text only arrives with
    /// egui's Paste event (web, and keyboard paste everywhere).
    pub clipboard_read: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Open a URL in the system browser (desktop). Without it, egui opens it (a new tab on the web).
    pub open_url: Option<OpenUrlFn>,
    /// Show an open dialog for several files (File → Place…); returns their paths (none: cancelled).
    pub pick_open_multi: Option<Box<dyn FnMut() -> Vec<String>>>,
    /// Web: start an async pick of files to place (they arrive via `place_inbox`).
    pub place_async: Option<Box<dyn FnMut()>>,
    /// Files that arrived asynchronously to be placed (web: picked for Place, or dropped on the
    /// canvas).
    pub place_inbox: Option<place::PlaceInbox>,
    /// The system clipboard with all its formats (desktop): Copy offers text, SVG, PDF and PNG,
    /// Paste takes SVG, PDF, text and bitmaps from other apps. Without it, SVG text only (through
    /// egui and `clipboard_read`).
    pub system_clipboard: Option<Box<dyn SystemClipboard>>,
    /// A second system-clipboard handle, for checking whether Paste has something to take on a
    /// background thread (Linux: an X11 clipboard owner that never answers would freeze the UI).
    /// Taken on the first frame. Without it the check runs on the UI thread.
    pub clipboard_probe: Option<ClipboardProbeFactory>,
    /// File → Show in Folder: select a file in the system file manager (desktop).
    pub reveal: Option<RevealFn>,
    /// Write a file from any thread (desktop): lets Background Save and Export write off the UI
    /// thread ([`background`]). Without it they run at once.
    pub write_shared: Option<background::SharedWriteFn>,
    /// Write a file from any thread with what a filler writes into it (desktop): native saves
    /// stream into the file as they are encoded instead of being held whole in memory first.
    /// Without it they are encoded, then written with `write` / `write_shared`.
    pub stream_shared: Option<background::SharedStreamFn>,
    /// Open a file (or a folder) in the system's default app for it (desktop: Edit Original, Show
    /// Package). Without it (the web) those answer with an error.
    pub open_file: Option<RevealFn>,
    /// Show a folder picker; returns its path (desktop: Relink to Folder, Package).
    pub pick_folder: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Show the system's color sampler, which picks a color anywhere on the screen (macOS, and
    /// browsers with an EyeDropper): Sample Color from Screen ([`screen_color`]). Without it that
    /// command answers with an error and the Color Picker's eyedropper button is gray.
    pub screen_color: Option<screen_color::StartSampler>,
    /// Where Data Recovery keeps its copies when not in a folder (the web's browser storage;
    /// tests): installed into the session ([`vectorcraft_engine::cmd::recovery`]).
    pub recovery_store: Option<std::sync::Arc<dyn vectorcraft_engine::cmd::recovery::RecoveryStore>>,
    /// File → Print: the system's printers and print queue (desktop), the browser's print dialog
    /// (web). Without it Print saves the job as a PDF.
    pub print: Option<Box<dyn print::PrintService>>,
    /// The macOS menu bar, when the desktop app installed one: the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
    /// Show file dialogs off the UI thread (desktop Linux, where a dialog in line holds the window
    /// and the compositor finds it not answering, #592) or as sheets on the window (macOS, where a
    /// dialog in line aborts the app, #867): what asked runs again with the answer ([`picks`]).
    /// Without it they are shown in line.
    pub start_pick: Option<picks::StartPick>,
    /// [`Self::start_pick`]'s dialogs are sheets on the window (macOS): while one is open the app
    /// neither quits nor closes documents, as under the modal dialog before.
    pub dialogs_are_sheets: bool,
    /// The window is on Wayland: the tool cursors are painted into it instead of handed to the
    /// compositor as bitmaps ([`cursors::os_cursors`]).
    pub wayland: bool,
    /// On Wayland, no cursor theme can be loaded, so winit sets no pointer and the compositor's
    /// default one would never come back once the canvas hid it: the canvas keeps the system
    /// pointer visible next to the tool glyph ([`cursors::hide_system_pointer`], #2450).
    pub no_cursor_theme: bool,
    /// The pen pressure and pen end the host reads from the system itself (desktop Linux under
    /// X11 and XWayland, macOS and Windows). The canvas uses the pressure when a frame's events carry no touch force,
    /// and the end to switch to the Eraser tool and back. Without it, pen pressure comes from
    /// touch events only ([`stylus`]).
    pub stylus: Option<stylus::StylusFeed>,
    /// The host's rewrites of a window's raw input (pen barrel button, closing the macOS menu
    /// bar's open menu) for floating panels in OS windows of their own: eframe runs the host's
    /// `raw_input_hook` for the main window only.
    pub native_input_hook: Option<NativeInputHook>,
}

/// Rewrites one window's raw input before egui reads it ([`Services::native_input_hook`]).
pub type NativeInputHook = std::sync::Arc<dyn Fn(&mut egui::RawInput) + Send + Sync>;
