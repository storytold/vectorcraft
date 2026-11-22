//! Linux: Sample Color from Screen (`vectorcraft_ui_egui::screen_color`) through the XDG desktop
//! portal's color sampler ([`portal`]). A thread of its own asks the portal and waits for the
//! person, so the UI keeps drawing; it sends the color to the UI and wakes the window. The dialog's
//! parent is the window, named on that thread as for the portal's save dialog (`portal_save`).

mod portal;

/// The portal's color sampler over the window `parent` holds; `ui` is woken when it answers.
pub fn sampler(ui: egui::Context, parent: super::host_services::Parent) -> vectorcraft_ui_egui::screen_color::StartSampler {
    Box::new(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ui, window) = (ui.clone(), parent.clone());
        std::thread::Builder::new()
            .name("screen-color".into())
            .spawn(move || {
                // The thread names the window and holds it until the export that names it is dropped.
                let (handle, export) = match window.as_deref() {
                    Some(window) => {
                        let (handle, export) = super::portal_save::exported_parent(window);
                        (handle, Some(export))
                    }
                    None => (String::new(), None),
                };
                let picked = portal::connect(zbus::blocking::connection::Builder::session()).and_then(|conn| portal::pick(&conn, &handle));
                drop(export);
                drop(window);
                let answer = picked.map_err(|failure| {
                    log::warn!("the portal's screen color sampler gave no color: {}", failure.detail());
                    failure.message().to_string()
                });
                // The UI may be gone (the app quitting): nothing waits for the color then.
                let _ = tx.send(answer);
                ui.request_repaint();
            })
            .map_err(|e| {
                log::warn!("could not start the screen color sampler's thread: {e}");
                "the screen color sampler failed".to_string()
            })?;
        Ok(rx)
    })
}
