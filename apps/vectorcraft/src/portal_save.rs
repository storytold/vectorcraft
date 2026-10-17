//! Linux and the BSDs: the save dialog of a save with two or more file types (Save, Save As, Save a
//! Copy, Save as Template; #2679), asked of the XDG desktop portal here
//! (<https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html>).
//! rfd 0.17 sends no `current_filter` with `SaveFile` and reads only `uris` from the answer
//! (PolyMeilex/rfd#341), and the GTK file chooser behind the portal keeps the typed name when the
//! user picks another file type, so the type picked was lost. This call sends the first file type as
//! `current_filter`, reads the one picked back, and answers its first extension after the path
//! ([`vectorcraft_ui_egui::picks::PickAnswer`]); the app gives the name that type's extension.
//! rfd shows the dialog when the portal can't (no session bus, no file chooser), with zenity as its
//! own fallback. PhotoCraft's `portal_save.rs` (storytold/photocraft#6151) makes the same call.
//!
//! The dialog's parent is the window: `x11:<window id>` on X11, and on Wayland (Linux) a handle
//! from the compositor's xdg-foreign protocol, as rfd names it (its `window_identifier`).

use std::collections::HashMap;

use vectorcraft_ui_egui::FilePick;
use zbus::zvariant::{OwnedValue, Value};

/// A file type in the dialog: its label and its extensions without the dot.
type Filter = (String, Vec<String>);

/// A save dialog to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SaveRequest {
    /// The portal's identifier of the parent window, or empty.
    parent: String,
    /// The suggested name; empty for none.
    name: String,
    folder: Option<String>,
    /// The file types, labeled in the interface's language; the dialog starts on the first.
    filters: Vec<Filter>,
}

impl SaveRequest {
    /// The dialog for `pick` over the window `parent` names.
    fn new(pick: &FilePick, parent: String) -> Self {
        let lang = vectorcraft_ui_egui::i18n::current();
        let filters = pick
            .filters
            .iter()
            .map(|(label, exts)| {
                (vectorcraft_ui_egui::i18n::tr_ctx(lang, "file-filter", label).to_string(), exts.iter().map(|e| e.to_string()).collect())
            })
            .collect();
        Self { parent, name: pick.name.clone(), folder: pick.folder.clone(), filters }
    }
}

/// What the dialog answered: the path, as typed, and the label of the file type picked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    path: String,
    filter: Option<String>,
}

/// `filter` as the portal takes it: its label and a `*.ext` pattern for each extension (`*` for
/// every file), as rfd sends them.
fn wire(filter: &Filter) -> (String, Vec<(u32, String)>) {
    let globs = filter.1.iter().map(|e| (0, if e.is_empty() || e == "*" { "*".to_string() } else { format!("*.{e}") })).collect();
    (filter.0.clone(), globs)
}

/// The `SaveFile` options for `request`, answering on `token`.
fn options<'a>(request: &'a SaveRequest, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut o = HashMap::from([("handle_token", Value::from(token))]);
    if !request.name.is_empty() {
        o.insert("current_name", Value::from(request.name.as_str()));
    }
    if let Some(folder) = &request.folder {
        // A byte string ending in NUL, as the portal specifies.
        let mut bytes = folder.as_bytes().to_vec();
        bytes.push(0);
        o.insert("current_folder", Value::from(bytes));
    }
    if let Some(first) = request.filters.first() {
        o.insert("filters", Value::from(request.filters.iter().map(wire).collect::<Vec<_>>()));
        o.insert("current_filter", Value::from(wire(first)));
    }
    o
}

/// The portal's `Response`: code 0 with the first of its `uris` and the label of its
/// `current_filter`. Any other code ends the dialog without a file (1 is Cancel, 2 a dialog closed
/// some other way), as rfd reads it.
fn answer(code: u32, results: &HashMap<String, OwnedValue>) -> Result<Option<Saved>, String> {
    if code != 0 {
        return Ok(None);
    }
    let uris: Vec<String> = results
        .get("uris")
        .ok_or("no file in the portal's answer")?
        .try_clone()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    let uri = uris.first().ok_or("no file in the portal's answer")?;
    let path = file_uri_path(uri).ok_or_else(|| format!("not a local file: {uri}"))?;
    let filter = results.get("current_filter").and_then(|v| {
        let (label, _globs): (String, Vec<(u32, String)>) = v.try_clone().ok()?.try_into().ok()?;
        Some(label)
    });
    Ok(Some(Saved { path, filter }))
}

/// `file:///home/me/A%20b.svg` → `/home/me/A b.svg`; `None` for a file elsewhere (`file://host/`)
/// or another scheme.
fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let mut out = Vec::with_capacity(rest.len());
    let mut rest = rest.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail.get(..2).filter(|h| b == b'%' && h.iter().all(u8::is_ascii_hexdigit));
        match hex.and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(decoded) => {
                out.push(decoded);
                rest = tail.get(2..).unwrap_or_default();
            }
            None => {
                out.push(b);
                rest = tail;
            }
        }
    }
    // `show_dialog` makes rfd's path a string with `to_string_lossy` too.
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// `saved` as the answer of the save dialog for `request`: the path, then the first extension of
/// the file type picked, found by its label. Empty when the dialog was canceled.
fn pick_answer(saved: Option<Saved>, request: &SaveRequest) -> Vec<String> {
    let Some(saved) = saved else { return Vec::new() };
    let picked = saved.filter.and_then(|label| request.filters.iter().find(|(l, _)| *l == label)).and_then(|(_, exts)| exts.first().cloned());
    std::iter::once(saved.path).chain(picked).collect()
}

/// The portal's identifier of an X11 window.
fn x11_parent(window: impl std::fmt::LowerHex) -> String {
    format!("x11:{window:x}")
}

/// Show the portal's save dialog for `pick` over `window` and wait for it → the answer (see the
/// module's comment). `None` when `pick` offers fewer than two file types, or the portal couldn't
/// show the dialog; rfd shows it then.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
pub fn pick(pick: &FilePick, window: Option<&winit::window::Window>) -> Option<Vec<String>> {
    if pick.filters.len() < 2 {
        return None;
    }
    // The export stays until the dialog has answered: the handle is valid while it lives.
    let (parent, _export) = window.map(parent).unwrap_or_default();
    let request = SaveRequest::new(pick, parent);
    let saved = zbus::blocking::Connection::session().map_err(|e| e.to_string()).and_then(|conn| save_file(&conn, &request));
    match saved {
        Ok(saved) => Some(pick_answer(saved, &request)),
        Err(e) => {
            log::info!("the portal's save dialog failed ({e}): rfd shows it");
            None
        }
    }
}

/// Ask the portal on `conn` for the save dialog of `request` and wait for its answer.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
fn save_file(conn: &zbus::blocking::Connection, request: &SaveRequest) -> Result<Option<Saved>, String> {
    use std::sync::atomic::{AtomicU32, Ordering};

    use zbus::blocking::Proxy;
    use zbus::zvariant::OwnedObjectPath;

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let sender = conn.unique_name().ok_or("no name on the session bus")?.as_str().trim_start_matches(':').replace('.', "_");
    let token = format!("vectorcraft_save_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // Listen before asking, so that an answer that comes at once is not missed.
    let mut replies = Proxy::new(conn, DESKTOP, path.as_str(), REQUEST).and_then(|p| p.receive_signal("Response")).map_err(|e| e.to_string())?;
    let chooser = Proxy::new(conn, DESKTOP, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").map_err(|e| e.to_string())?;
    // The title is empty, as rfd sends it for a save.
    let handle: OwnedObjectPath = chooser.call("SaveFile", &(request.parent.as_str(), "", options(request, &token))).map_err(|e| e.to_string())?;
    // A portal older than version 0.9 ignores `handle_token` and answers on the request path it
    // returns.
    let reply = if handle.as_str() == path {
        replies.next()
    } else {
        Proxy::new(conn, DESKTOP, handle.as_str(), REQUEST).and_then(|p| p.receive_signal("Response")).map_err(|e| e.to_string())?.next()
    }
    .ok_or("the save dialog closed without an answer")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
    answer(code, &results)
}

/// The portal's identifier of `window` when it is an X11 window.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
fn x11_window(window: &winit::window::Window) -> Option<String> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(x11_parent(h.window)),
        RawWindowHandle::Xcb(h) => Some(x11_parent(h.window.get())),
        _ => None,
    }
}

/// The portal's identifier of `window`, and on Wayland the export that keeps it valid; empty when
/// the window has neither an X11 id nor a Wayland handle.
#[cfg(target_os = "linux")]
fn parent(window: &winit::window::Window) -> (String, Option<wayland::Export>) {
    if let Some(id) = x11_window(window) {
        return (id, None);
    }
    match wayland::export(window) {
        Some(export) => (format!("wayland:{}", export.handle), Some(export)),
        None => (String::new(), None),
    }
}

/// The portal's identifier of `window`: its X11 id, else empty (no Wayland export is made off
/// Linux).
#[cfg(any(target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
fn parent(window: &winit::window::Window) -> (String, Option<()>) {
    (x11_window(window).unwrap_or_default(), None)
}

/// The Wayland window's handle for the portal: an xdg-foreign v2 export of its surface
/// (<https://wayland.app/protocols/xdg-foreign-unstable-v2>), made as rfd's `window_identifier`
/// makes it.
#[cfg(target_os = "linux")]
mod wayland {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::foreign::zv2::client::zxdg_exported_v2::{self, ZxdgExportedV2};
    use wayland_protocols::xdg::foreign::zv2::client::zxdg_exporter_v2::{self, ZxdgExporterV2};

    /// A window exported for the portal: `handle` names it until this is dropped.
    pub struct Export {
        pub handle: String,
        exported: ZxdgExportedV2,
        conn: Connection,
    }

    impl Drop for Export {
        fn drop(&mut self) {
            self.exported.destroy();
            let _ = self.conn.flush();
        }
    }

    /// The events the export waits for.
    #[derive(Default)]
    struct State {
        handle: Option<String>,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<ZxdgExporterV2, ()> for State {
        fn event(_: &mut Self, _: &ZxdgExporterV2, _: zxdg_exporter_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<ZxdgExportedV2, ()> for State {
        fn event(state: &mut Self, _: &ZxdgExportedV2, event: zxdg_exported_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let zxdg_exported_v2::Event::Handle { handle } = event {
                state.handle = Some(handle);
            }
        }
    }

    /// Export `window`'s surface on the window's own Wayland connection, with an event queue of its
    /// own → `None` when the compositor has no xdg-foreign v2 or answers no handle.
    pub fn export(window: &winit::window::Window) -> Option<Export> {
        let (conn, surface) = surface(window)?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let exporter: ZxdgExporterV2 = globals.bind(&qh, 1..=1, ()).ok()?;
        let exported = exporter.export_toplevel(&surface, &qh, ());
        let mut state = State::default();
        let answered = queue.roundtrip(&mut state);
        exporter.destroy();
        match (answered, state.handle) {
            (Ok(_), Some(handle)) => Some(Export { handle, exported, conn }),
            _ => {
                exported.destroy();
                let _ = conn.flush();
                log::info!("the compositor gave no xdg-foreign handle: the save dialog has no parent window");
                None
            }
        }
    }

    /// `window`'s Wayland connection and surface, from its raw handles.
    #[allow(unsafe_code)]
    fn surface(window: &winit::window::Window) -> Option<(Connection, WlSurface)> {
        use wayland_backend::sys::client::{Backend, ObjectId};
        use winit::raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle};
        let (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) =
            (window.window_handle().ok()?.as_raw(), window.display_handle().ok()?.as_raw())
        else {
            return None;
        };
        // SAFETY: the display is winit's open `wl_display`, which outlives `window` (the caller
        // holds it until the export is dropped); a backend made from a foreign display does not
        // close it when dropped.
        let backend = unsafe { Backend::from_foreign_display(d.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        // SAFETY: the pointer is the window's live `wl_surface` proxy on that display.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), w.surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, id).ok()?;
        Some((conn, surface))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SaveRequest {
        let pick = FilePick {
            name: "Untitled-1.vectorcraft".into(),
            folder: Some("/home/me/Art".into()),
            filters: vec![("VectorCraft", &["vectorcraft"]), ("SVG", &["svg"]), ("PDF-compatible .ai", &["ai"])],
        };
        SaveRequest::new(&pick, "x11:3a0000f".into())
    }

    /// The dialog starts on the first file type (the document's format) and shows the suggested
    /// name in its folder.
    #[test]
    fn the_request_names_the_file_type_the_dialog_starts_on() {
        let request = request();
        let o = options(&request, "t1");
        assert_eq!(o.get("current_filter"), Some(&Value::from(("VectorCraft".to_string(), vec![(0u32, "*.vectorcraft".to_string())]))));
        let filters = vec![
            ("VectorCraft".to_string(), vec![(0u32, "*.vectorcraft".to_string())]),
            ("SVG".to_string(), vec![(0u32, "*.svg".to_string())]),
            ("PDF-compatible .ai".to_string(), vec![(0u32, "*.ai".to_string())]),
        ];
        assert_eq!(o.get("filters"), Some(&Value::from(filters)));
        assert_eq!(o.get("current_name"), Some(&Value::from("Untitled-1.vectorcraft")));
        assert_eq!(o.get("current_folder"), Some(&Value::from(b"/home/me/Art\0".to_vec())));
        assert_eq!(o.get("handle_token"), Some(&Value::from("t1")));
        let bare = SaveRequest { name: String::new(), folder: None, filters: Vec::new(), ..request };
        let o = options(&bare, "t2");
        assert_eq!(o.keys().copied().collect::<Vec<_>>(), ["handle_token"]);
    }

    /// The portal's answer as a D-Bus dictionary: `uris` and `current_filter`.
    fn results(uri: &str, filter: Option<&str>) -> HashMap<String, OwnedValue> {
        let mut r = HashMap::new();
        r.insert("uris".to_string(), OwnedValue::try_from(Value::from(vec![uri.to_string()])).unwrap());
        if let Some(label) = filter {
            let picked = Value::from((label.to_string(), vec![(0u32, "*.ai".to_string())]));
            r.insert("current_filter".to_string(), OwnedValue::try_from(picked).unwrap());
        }
        r
    }

    /// The file type picked comes back with the path as typed, and its first extension follows
    /// the path in the answer.
    #[test]
    fn the_answer_carries_the_file_type_picked() {
        let request = request();
        let saved = answer(0, &results("file:///home/me/Art/poster", Some("PDF-compatible .ai"))).unwrap();
        assert_eq!(saved, Some(Saved { path: "/home/me/Art/poster".into(), filter: Some("PDF-compatible .ai".into()) }));
        assert_eq!(pick_answer(saved, &request), ["/home/me/Art/poster", "ai"]);
        // A portal that names no file type, or one the dialog did not offer: the path alone.
        let saved = answer(0, &results("file:///home/me/Art/poster.svg", None)).unwrap();
        assert_eq!(pick_answer(saved, &request), ["/home/me/Art/poster.svg"]);
        let saved = answer(0, &results("file:///home/me/Art/poster", Some("PNG"))).unwrap();
        assert_eq!(pick_answer(saved, &request), ["/home/me/Art/poster"]);
    }

    /// Cancel (1) and a dialog closed another way (2) answer no file; an answer without a local file
    /// is an error, and rfd shows the dialog instead.
    #[test]
    fn a_canceled_dialog_answers_no_file() {
        let request = request();
        for code in [1, 2] {
            assert_eq!(answer(code, &HashMap::new()), Ok(None));
            assert!(pick_answer(None, &request).is_empty());
        }
        assert!(answer(0, &HashMap::new()).is_err());
        assert!(answer(0, &results("sftp://host/poster.svg", Some("SVG"))).is_err());
    }

    #[test]
    fn file_uris_become_local_paths() {
        assert_eq!(file_uri_path("file:///home/me/Untitled-1.vectorcraft").as_deref(), Some("/home/me/Untitled-1.vectorcraft"));
        assert_eq!(file_uri_path("file:///home/me/A%20b%C3%A9.svg").as_deref(), Some("/home/me/A bé.svg"));
        assert_eq!(file_uri_path("file://localhost/tmp/x.pdf").as_deref(), Some("/tmp/x.pdf"));
        // A `%` without two hex digits after it stays as it is.
        assert_eq!(file_uri_path("file:///tmp/100%+1%2").as_deref(), Some("/tmp/100%+1%2"));
        assert_eq!(file_uri_path("file://host/x.svg"), None);
        assert_eq!(file_uri_path("sftp://host/x.svg"), None);
    }

    #[test]
    fn x11_windows_are_named_by_their_id() {
        assert_eq!(x11_parent(0x3a0000f), "x11:3a0000f");
    }

    /// What the mock portal answers to each `SaveFile` call: the response code, and the file and
    /// the label of the file type "picked".
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    type Script = Vec<(u32, Option<(&'static str, &'static str)>)>;

    /// What each `SaveFile` call carried: `current_name` and the label of `current_filter`.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(Option<String>, Option<String>)>>>;

    /// A session bus of its own (`dbus-daemon`) with a mock `org.freedesktop.portal.FileChooser`
    /// that answers `SaveFile` from `script`, as PhotoCraft's portal tests run it. With `own_paths`
    /// it is a portal older than 0.9, which answers on a request path of its own. `None` when there
    /// is no `dbus-daemon` to run.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    fn mock_portal(script: Script, own_paths: bool) -> Option<(zbus::blocking::Connection, Seen, std::process::Child)> {
        use std::io::BufRead as _;

        use zbus::zvariant::OwnedObjectPath;
        let mut daemon = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .ok()?;
        let mut address = String::new();
        std::io::BufReader::new(daemon.stdout.take()?).read_line(&mut address).ok()?;
        let address = address.trim().to_string();
        let server =
            zbus::blocking::connection::Builder::address(address.as_str()).ok()?.name("org.freedesktop.portal.Desktop").ok()?.build().ok()?;
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            let mut script = script.into_iter();
            for msg in zbus::blocking::MessageIterator::from(&server) {
                let Ok(msg) = msg else { break };
                let header = msg.header();
                if header.message_type() != zbus::message::Type::MethodCall || header.member().map(|m| m.as_str()) != Some("SaveFile") {
                    continue;
                }
                let Ok((_parent, _title, options)) = msg.body().deserialize::<(String, String, HashMap<String, OwnedValue>)>() else { break };
                let text = |key: &str| options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
                let filter = options.get("current_filter").and_then(|v| {
                    let (label, _): (String, Vec<(u32, String)>) = v.try_clone().ok()?.try_into().ok()?;
                    Some(label)
                });
                let calls = log.lock().map(|mut l| {
                    l.push((text("current_name"), filter));
                    l.len()
                });
                let Some(sender) = header.sender().map(|s| s.as_str().trim_start_matches(':').replace('.', "_")) else { break };
                let token = if own_paths { format!("old{}", calls.unwrap_or_default()) } else { text("handle_token").unwrap_or_default() };
                let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                let Ok(handle) = OwnedObjectPath::try_from(path.as_str()) else { break };
                if server.reply(&header, &handle).is_err() {
                    break;
                }
                if own_paths {
                    // As a person answers the dialog: after the caller listened on the path returned.
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                let (code, pick) = script.next().unwrap_or((2, None));
                let mut results: HashMap<&str, Value<'_>> = HashMap::new();
                if let Some((uri, label)) = pick {
                    results.insert("uris", Value::from(vec![uri.to_string()]));
                    results.insert("current_filter", Value::from((label.to_string(), vec![(0u32, "*.x".to_string())])));
                }
                let to = header.sender().cloned().map(zbus::names::BusName::from);
                if server.emit_signal(to, path.as_str(), "org.freedesktop.portal.Request", "Response", &(code, results)).is_err() {
                    break;
                }
            }
        });
        let client = zbus::blocking::connection::Builder::address(address.as_str()).ok()?.build().ok()?;
        Some((client, seen, daemon))
    }

    /// Through a real D-Bus: the call starts the dialog on the first file type with the suggested
    /// name, and the answer names the file and the type picked; Cancel answers no file.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    #[test]
    fn the_portal_is_asked_on_the_first_file_type_and_answers_the_one_picked() {
        let script = vec![(0, Some(("file:///home/me/Art/poster", "PDF-compatible .ai"))), (1, None)];
        let Some((conn, seen, mut daemon)) = mock_portal(script, false) else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let request = request();
        let saved = save_file(&conn, &request).unwrap();
        assert_eq!(pick_answer(saved, &request), ["/home/me/Art/poster", "ai"]);
        assert_eq!(save_file(&conn, &request), Ok(None));
        let seen = seen.lock().map(|s| s.clone()).unwrap_or_default();
        assert_eq!(seen.first(), Some(&(Some("Untitled-1.vectorcraft".to_string()), Some(request.filters[0].0.clone()))));
        let _ = daemon.kill();
    }

    /// A portal older than 0.9 answers on the request path it returns, and the answer is awaited
    /// there.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    #[test]
    fn the_answer_is_awaited_on_the_path_the_portal_returns() {
        let Some((conn, _seen, mut daemon)) = mock_portal(vec![(0, Some(("file:///tmp/card", "SVG")))], true) else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let request = request();
        assert_eq!(pick_answer(save_file(&conn, &request).unwrap(), &request), ["/tmp/card", "svg"]);
        let _ = daemon.kill();
    }

    /// The labels go to the dialog in the interface's language, which is how the answer is matched.
    #[test]
    fn the_file_types_are_labeled_as_the_other_dialogs_label_them() {
        let request = request();
        let labels: Vec<&str> = request.filters.iter().map(|(l, _)| l.as_str()).collect();
        let lang = vectorcraft_ui_egui::i18n::current();
        let expected: Vec<&str> =
            ["VectorCraft", "SVG", "PDF-compatible .ai"].iter().map(|l| vectorcraft_ui_egui::i18n::tr_ctx(lang, "file-filter", l)).collect();
        assert_eq!(labels, expected);
        assert_eq!(request.parent, "x11:3a0000f");
    }
}
