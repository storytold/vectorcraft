//! File → Open Remote…, Place from Remote… and Save As Remote…: pick a file, or a folder and a
//! name, on the server's mounts ([`crate::remote`]). Folders are listed by the host
//! ([`crate::remote::RemoteStorage::list`]); a listing that is still loading is asked for again on
//! the next frames.
//!
//! Fields: `mode` (`open`, `place` or `save`), `dir` (the folder shown; empty shows the mounts) and
//! `name` (the file chosen, or the name to save under).

use std::time::Duration;

use serde_json::json;

use super::{DialogResult, DialogSpec};
use crate::VectorcraftApp;
use crate::remote::{self, Mode, RemoteEntry, RemoteMount};
use crate::state::Dialog;
use crate::theme::Tokens;

/// The dialog kind of the remote file dialog.
pub const KIND: &str = "remote";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| match Mode::of(&d.str("mode")) {
        Mode::Open => tl!("Open Remote").into(),
        Mode::Place => tl!("Place from Remote").into(),
        Mode::Save => tl!("Save As Remote").into(),
    },
    body,
    confirm,
    ok: Some("Open"),
    ok_label: Some(|app| match app.ui.dialog.as_ref().map(|d| Mode::of(&d.str("mode"))) {
        Some(Mode::Place) => "Place",
        Some(Mode::Save) => "Save",
        _ => "Open",
    }),
    ok_enabled: Some(|_, d| !d.str("dir").is_empty() && remote::valid_name(&d.str("name"))),
    min_width: 460.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

/// Show the dialog. With one mount it starts in that mount's folder.
pub(crate) fn open(app: &mut VectorcraftApp, mode: Mode) {
    let dir = match app.services.remote.as_ref().map(|r| r.mounts.as_slice()) {
        Some([only]) => only.path.clone(),
        _ => String::new(),
    };
    let name = if mode == Mode::Save { remote::suggested_name(app) } else { String::new() };
    app.ui.dialog = Some(Dialog::new(KIND, json!({"mode": mode.id(), "dir": dir, "name": name})));
}

/// The mount that holds `dir`.
fn mount_of<'a>(mounts: &'a [RemoteMount], dir: &str) -> Option<&'a RemoteMount> {
    mounts.iter().find(|m| dir == m.path || dir.strip_prefix(m.path.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

/// The breadcrumb trail of `dir`: (label, path) from its mount down.
fn crumbs(mounts: &[RemoteMount], dir: &str) -> Vec<(String, String)> {
    let Some(m) = mount_of(mounts, dir) else { return Vec::new() };
    let mut out = vec![(m.name.clone(), m.path.clone())];
    let mut at = m.path.clone();
    for seg in dir.strip_prefix(m.path.as_str()).unwrap_or("").split('/').filter(|s| !s.is_empty()) {
        at = format!("{at}/{seg}");
        out.push((seg.to_string(), at.clone()));
    }
    out
}

/// Draw the dialog's body: the breadcrumbs, the folder listing and the name field.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let mode = Mode::of(&d.str("mode"));
    let dir = d.str("dir");
    let mounts = app.services.remote.as_ref().map(|r| r.mounts.clone()).unwrap_or_default();
    let listing = if dir.is_empty() { None } else { app.services.remote.as_mut().map(|r| (r.list)(&dir)) };
    let mut go: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        if ui.link(tl!("Remote")).clicked() {
            go = Some(String::new());
        }
        for (label, path) in crumbs(&mounts, &dir) {
            ui.label("›");
            if ui.link(&label).clicked() {
                go = Some(path);
            }
        }
    });
    ui.add_space(6.0);
    let mut picked: Option<String> = None;
    crate::widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &listing {
                None => {
                    for m in &mounts {
                        let label = if m.writable { m.name.clone() } else { format!("{} ({})", m.name, tl!("read-only")) };
                        if ui.selectable_label(false, label).clicked() {
                            go = Some(m.path.clone());
                        }
                    }
                    if mounts.is_empty() {
                        ui.label(egui::RichText::new(tl!("No remote storage")).color(t.text_dim));
                    }
                }
                Some(Ok(entries)) => {
                    let mut rows: Vec<&RemoteEntry> = entries.iter().collect();
                    rows.sort_by_key(|e| (!e.is_dir, e.name.to_lowercase()));
                    for e in &rows {
                        let label = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
                        if ui.selectable_label(!e.is_dir && e.name == d.str("name"), label).clicked() {
                            if e.is_dir {
                                go = Some(remote::join(&dir, &e.name));
                            } else {
                                picked = Some(e.name.clone());
                            }
                        }
                    }
                    if rows.is_empty() {
                        ui.label(egui::RichText::new(tl!("Nothing here")).color(t.text_dim));
                    }
                }
                Some(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    ui.label(egui::RichText::new(tl!("Loading…")).color(t.text_dim));
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                Some(Err(e)) => {
                    ui.label(egui::RichText::new(crate::i18n::message(crate::i18n::current(), &e.to_string())).color(t.text_dim));
                }
            }
        });
    });
    if let Some(name) = picked {
        d.fields.insert("name".into(), json!(name));
    }
    if let Some(next) = go {
        d.fields.insert("dir".into(), json!(next));
        if mode != Mode::Save {
            d.fields.insert("name".into(), json!(""));
        }
    }
    ui.add_space(8.0);
    if mode == Mode::Save {
        let mut name = d.str("name");
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).changed() {
            d.fields.insert("name".into(), json!(name));
        }
    } else {
        let chosen = d.str("name");
        let shown = if chosen.is_empty() { tl!("Choose a file").to_string() } else { chosen };
        ui.label(egui::RichText::new(shown).color(t.text_dim));
    }
    false
}

/// OK: fetch the chosen file, or save under the typed name. A save is refused here, with the
/// dialog still open, on a read-only mount and over another file that is there.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let mode = Mode::of(&d.str("mode"));
    let (dir, name) = (d.str("dir"), d.str("name"));
    if dir.is_empty() {
        return Err("Choose a folder first".into());
    }
    if !remote::valid_name(&name) {
        return Err("Enter a valid file name".into());
    }
    let mut path = remote::join(&dir, &name);
    if mode == Mode::Save {
        path = remote::save_path(&path)?;
        let name = vectorcraft_engine::cmd::fileio::file_name(&path);
        let remote = app.services.remote.as_mut().ok_or("remote storage is not configured")?;
        if let Some(m) = mount_of(&remote.mounts, &dir).filter(|m| !m.writable) {
            return Err(format!("{} is read-only: choose another place", m.name));
        }
        let own = app.session.active().and_then(|st| st.path.as_deref()) == Some(path.as_str());
        let there = (remote.list)(&dir).is_ok_and(|rows| rows.iter().any(|e| !e.is_dir && e.name == name));
        if there && !own {
            return Err(format!("{name} is already there: choose another name"));
        }
    }
    app.ui.dialog = None;
    remote::accept(app, mode, &path)
}
