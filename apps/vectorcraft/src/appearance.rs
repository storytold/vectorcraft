//! The shared portal watcher feeds only Automatic brightness; custom accents remain app-owned.

pub fn service() -> Option<vectorcraft_ui_egui::SystemThemeFn> {
    #[cfg(target_os = "linux")]
    {
        use craft_desktop::appearance::Watcher;
        // Start lazily: fixed custom themes do not need a portal worker. The callback belongs
        // to the UI thread; no D-Bus work or wait runs inside it.
        let state = std::cell::RefCell::new((None::<Watcher>, None::<egui::Theme>));
        Some(Box::new(move |ctx| {
            let mut state = state.try_borrow_mut().ok()?;
            let (watcher, current) = &mut *state;
            let watcher = watcher.get_or_insert_with(|| {
                let repaint = ctx.clone();
                Watcher::start(move || repaint.request_repaint())
            });
            if let Some(appearance) = watcher.take_change() {
                *current = appearance.dark.map(|dark| if dark { egui::Theme::Dark } else { egui::Theme::Light });
            }
            *current
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None // winit/egui supplies the system appearance on other platforms.
    }
}
