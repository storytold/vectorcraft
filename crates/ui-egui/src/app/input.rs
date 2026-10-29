use super::{Arc, Ordering, VectorcraftApp, graphics, native_menu, shortcuts, take_frame_once};
#[cfg(not(target_arch = "wasm32"))]
use super::{fileio, place};

impl VectorcraftApp {
    /// The first UI pass of a host iteration takes the host's input, once per context, viewport and
    /// frame: clipboard formats, native-menu clicks, shortcuts and dropped files. A later pass
    /// (`request_discard` rerun) or an interleaved second viewport pass with the same key draws
    /// again but does not replay them; a logic-only iteration never reaches this.
    pub(super) fn handle_input_once(&mut self, ctx: &egui::Context) {
        let frame = (graphics::context_tag(ctx), ctx.viewport_id(), ctx.cumulative_frame_nr_for(ctx.viewport_id()));
        if !take_frame_once(&mut self.input_frames, frame) {
            return;
        }
        self.write_formats_before_text_input(ctx);
        native_menu::run(self, ctx);
        if self.fonts_ready {
            shortcuts::handle(self, ctx);
        }
        // A Copy taken in this UI phase (an egui copy event, or a command it runs) is published in
        // this same pass: waiting for the next `tick` can leave the command unissued when an idle
        // web host has no further frame to run it in. Native rich-format Copy does not use this
        // path; it writes through `SystemClipboard` and leaves `clipboard_out` empty.
        self.flush_clipboard_out(ctx);
        // Native only: the web host reads dropped files asynchronously and feeds the inboxes.
        #[cfg(not(target_arch = "wasm32"))]
        self.take_dropped_files(ctx);
    }

    /// Files dropped on the window: documents, libraries, presets, plug-ins and shortcut sets
    /// opened, pictures and text placed on the canvas ([`Self::drop_target`]).
    #[cfg(not(target_arch = "wasm32"))]
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, shift) = ctx.input(|i| (i.raw.dropped_files.clone(), i.modifiers.shift));
        self.open_dropped_files(dropped, place::drag_pos(ctx), shift);
    }

    /// Files dropped on the main window or on a floating panel's own OS window, opened or placed
    /// by kind; `pos` is the drop point in the main window, where the backend reports one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_dropped_files(&mut self, dropped: Vec<egui::DroppedFileHandle>, pos: Option<egui::Pos2>, shift: bool) {
        if dropped.is_empty() {
            return;
        }
        let mut files = vec![];
        for f in dropped {
            let path = Some(f.path().to_string_lossy().to_string()).filter(|s| !s.is_empty());
            let name = path.as_deref().map_or_else(|| "dropped".into(), vectorcraft_engine::cmd::fileio::file_name);
            // Native file drops usually carry a path. Use the same file reader as File → Open
            // (and File → Place), even if the drop handle can't supply the bytes itself.
            let bytes = if path.is_some() && self.services.read.is_some() { Ok(vec![]) } else { f.bytes() };
            match bytes {
                Ok(bytes) => {
                    // A backend may give bytes without a path or extension. Sniff their format
                    // before deciding: an SVG opens as a document, a raster image is placed.
                    let target = if path.is_none() && fileio::detect(&name, &bytes).is_none_or(|f| !f.raster) {
                        place::DropTarget::Open
                    } else {
                        self.drop_target(&name, pos, shift)
                    };
                    files.push((target, (name, path, bytes)));
                }
                Err(e) => self.status(format!("Couldn't read {name}: {e}")),
            }
        }
        place::drop_files(self, files);
    }

    /// Install the end-of-pass observer that tells the next host iteration a settled UI pass took
    /// the injected batch (a font-gated or discarded pass does not). The shipped hosts have only
    /// the root viewport; a multi-viewport host would need pending batches and settlement keyed
    /// per viewport instead of this root-only acknowledgement.
    pub(super) fn install_injected_settle_hook(&mut self, ctx: &egui::Context) {
        if self.injected_settle_hook {
            return;
        }
        let settled = self.injected_settled.clone();
        ctx.on_end_pass(
            "vectorcraft injected input settlement",
            Arc::new(move |ui| {
                if ui.ctx().viewport_id() == egui::ViewportId::ROOT && !ui.ctx().will_discard() {
                    settled.store(true, Ordering::Relaxed);
                }
            }),
        );
        self.injected_settle_hook = true;
    }

    /// The settled pass took the injected batch: allow the next step at the next host iteration.
    fn acknowledge_injected_batch(&mut self) {
        self.synthetic_injected = false;
        self.injected_events.clear();
        self.synthetic_frame = false;
    }

    /// The document, dialog or focus changed before a UI pass took the batch: remove exactly the
    /// events we appended (they still live in eframe's pending raw input) so a forced hidden paint
    /// cannot replay stale clicks or text into the new state.
    fn discard_stale_injected_batch(&mut self, raw: &mut egui::RawInput) {
        if !self.synthetic_injected || self.input_generation() == self.injected_generation {
            return;
        }
        let events = std::mem::take(&mut self.injected_events);
        let mut removed = false;
        if !events.is_empty()
            && let Some(start) = raw.events.windows(events.len()).rposition(|window| window == events.as_slice())
        {
            raw.events.drain(start..start + events.len());
            removed = true;
        }
        if !events.is_empty() && !removed {
            // The host's raw input no longer contains the batch as we injected it (another
            // handler rewrote or interleaved events). We cannot tell stale synthetic input from
            // the person's, so fail closed: deliver nothing this pass rather than replay a stale
            // click or text into the state that replaced it, and cancel the rest of that
            // synthetic sequence instead of injecting its next step after this one.
            raw.events.clear();
            self.synthetic.clear();
        }
        // The discarded batch may have carried the modifiers it set; the host's modifiers are
        // what the next real event has, so do not leave a synthetic latch behind.
        self.synthetic_modifiers = false;
        self.synthetic_injected = false;
        self.synthetic_frame = false;
    }

    /// Release the pointer buttons egui still holds down when the system reports that no mouse
    /// button is down (`system_held` is false), at the pointer's last position. egui ends a drag
    /// only on a button release or Escape, so a release the window never received leaves a drag,
    /// such as a dock column's resize, following the pointer (#4326). Frames that carry a button
    /// event, and control-channel input that holds a button between steps, are left alone.
    pub fn release_lost_buttons(&self, ctx: &egui::Context, raw: &mut egui::RawInput, system_held: bool) {
        if system_held
            || self.synthetic_injected
            || !self.synthetic.is_empty()
            || raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { .. }))
        {
            return;
        }
        let (down, pos, modifiers) = ctx.input_for(raw.viewport_id, |i| {
            let p = &i.pointer;
            let down: Vec<egui::PointerButton> = [
                egui::PointerButton::Primary,
                egui::PointerButton::Secondary,
                egui::PointerButton::Middle,
                egui::PointerButton::Extra1,
                egui::PointerButton::Extra2,
            ]
            .into_iter()
            .filter(|b| p.button_down(*b))
            .collect();
            (down, p.latest_pos().or_else(|| p.interact_pos()).or_else(|| p.press_origin()), i.modifiers)
        });
        // The releases go after this frame's last pointer movement, at its position, so a pointer
        // that moved away from the press does not end it with a click at the press position.
        let moved = raw.events.iter().enumerate().rev().find_map(|(i, e)| match e {
            egui::Event::PointerMoved(p) => Some((i + 1, *p)),
            _ => None,
        });
        let (at, pos) = match moved {
            Some((at, p)) => (at, Some(p)),
            None => (0, pos),
        };
        let Some(pos) = pos else { return };
        let releases = down.into_iter().map(|button| egui::Event::PointerButton { pos, button, pressed: false, modifiers });
        raw.events.splice(at..at, releases);
    }

    /// Inject synthetic events (one press/release step, wheel turn or short key sequence per host
    /// iteration), carrying control provenance until a settled UI pass has taken the batch. The
    /// batch belongs to the one root viewport the shipped hosts draw (see
    /// `install_injected_settle_hook`); a multi-viewport host would key it by viewport. A
    /// hidden logic-only iteration may see this hook run without a UI pass: nothing more is
    /// injected until the batch was actually applied, so a burst of hidden ticks cannot queue
    /// several steps of a drag or replay them together later.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Both hosts call this hook once per host iteration, before deciding whether a UI pass
        // follows; it is the iteration boundary, not a UI-phase marker.
        self.begin_host_iteration();
        // The observer lives on the context so it also settles a harness that consumes raw input
        // without calling `VectorcraftApp::ui`.
        self.install_injected_settle_hook(ctx);
        // The native menu's key equivalents become the input their keys make, ahead of what came
        // after them (see `native_menu`).
        let keys = self.services.native_menu.as_mut().map(native_menu::NativeMenu::take_keys).unwrap_or_default();
        // The menu bar does nothing while a file dialog is open (`native_menu`): a key that raced it
        // going inert is dropped.
        if !keys.is_empty() && !self.file_dialog_open() {
            let events: Vec<egui::Event> = keys.into_iter().flat_map(|k| native_menu::key_events(k, || self.system_clipboard_text())).collect();
            raw.events.splice(0..0, events);
        }
        for e in &raw.events {
            match e {
                egui::Event::ModifiersChanged(m) => self.host_modifiers = *m,
                egui::Event::WindowFocused(false) => self.host_modifiers = egui::Modifiers::NONE,
                egui::Event::WindowFocused(true) => self.refresh_installed_fonts(true),
                _ => {}
            }
        }
        // Keys a floating panel's own OS window left over run here, as if typed in this window,
        // under the modifiers held there for this frame only.
        if std::mem::take(&mut self.forwarded_modifiers_held) {
            raw.events.insert(0, egui::Event::ModifiersChanged(self.host_modifiers));
        }
        self.forwarded_frame = !self.forwarded.is_empty();
        if !self.forwarded.is_empty() {
            if let Some(m) = self.forwarded_modifiers.take() {
                raw.events.push(egui::Event::ModifiersChanged(m));
                self.forwarded_modifiers_held = true;
            }
            raw.events.append(&mut self.forwarded);
        }
        // The previous pass settled: the batch it took is done, so the next step may go.
        if self.injected_settled.swap(false, Ordering::Relaxed) {
            self.acknowledge_injected_batch();
        }
        self.discard_stale_injected_batch(raw);
        // While the previous batch is unapplied (it lives in eframe's pending raw input), inject
        // nothing more: one step per settled UI pass, not per hidden logic tick.
        if self.synthetic_injected {
            return;
        }
        let Some(first) = self.synthetic.first() else {
            if std::mem::take(&mut self.synthetic_modifiers) {
                raw.events.push(egui::Event::ModifiersChanged(self.host_modifiers));
            }
            return;
        };
        // Pointer events go one per frame so egui sees presses, drags and releases as real input;
        // keyboard sequences go up to the key release.
        let n = match first {
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        let (now, later) = self.synthetic.split_at(n.min(self.synthetic.len()));
        let mut batch: Vec<egui::Event> = Vec::with_capacity(n + 2);
        if let egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } = first {
            batch.push(egui::Event::PointerMoved(*p));
        }
        // This frame's key or button, else the button a drag holds down (released later).
        let held = now
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => {
                    Some(*modifiers)
                }
                _ => None,
            })
            .or_else(|| match later.iter().find(|e| matches!(e, egui::Event::PointerButton { .. })) {
                Some(egui::Event::PointerButton { pressed: false, modifiers, .. }) => Some(*modifiers),
                _ => None,
            });
        match held {
            Some(m) => {
                batch.push(egui::Event::ModifiersChanged(m));
                self.synthetic_modifiers = true;
            }
            None if std::mem::take(&mut self.synthetic_modifiers) => batch.push(egui::Event::ModifiersChanged(self.host_modifiers)),
            None => {}
        }
        batch.extend(now.iter().cloned());
        // `batch` holds the n drained events plus the pointer/modifier companions that make the
        // step read like real input.
        self.synthetic.drain(..n);
        self.injected_generation = self.input_generation();
        self.injected_events = batch.clone();
        self.synthetic_injected = true;
        self.synthetic_frame = true;
        raw.events.extend(batch);
    }
}
