//! The macOS menu bar on craft-menu's native menu: installed once, polled and run before the UI
//! each frame, and synced after it.

use craft::menu::{Gate, LayoutOptions, NativeMenu, mac, mac_layout};
use craft::ui::runtime::commands::{FirePhase, FireSource, Fired, holder};

use crate::VectorcraftApp;
use crate::menus::model::{APP_NAME, pressed_pass, recording_shortcut, refresh, state_key, translate_label};

/// The layout options every Mac layout of VectorCraft's menus uses: the app's name and its
/// translation of the labels the layout writes itself.
pub(crate) fn options() -> LayoutOptions<'static> {
    LayoutOptions::new(APP_NAME).label(&translate_label)
}

/// Install the native menu bar from the app's menus; `None` off macOS or when AppKit refuses it.
/// The model is built first so the bar exists before the first frame.
pub fn install(ctx: &egui::Context, app: &mut VectorcraftApp) -> Option<NativeMenu> {
    refresh(app, ctx);
    mac::install(ctx, &mac_layout(&app.menus.bar, &options()).bar)
}

/// Once per frame, before the UI: take what the native menu reported and run the clicks.
pub(crate) fn run(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let gate =
        Gate::default().modal(app.file_dialog_open()).recording_shortcut(recording_shortcut(app)).text_editing(ctx.egui_wants_keyboard_input());
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    menu.poll(ctx);
    menu.set_gate(gate);
    let clicks = menu.take_clicks(ctx);
    // A menu opening reads the Scripts folder again.
    if menu.take_opened() {
        crate::scripts_menu::refresh(app, ctx);
    }
    let time = ctx.input(|i| i.time);
    for command in clicks {
        crate::shortcuts::run_fired(app, ctx, &Fired::new(command, FireSource::Menu, FirePhase::Press, time));
    }
}

/// Once per frame, after the UI: bring the native menu up to date. Frames with input read the
/// menus again, as input may change state the key doesn't list.
pub(crate) fn sync(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.services.native_menu.is_none() {
        return;
    }
    let state = state_key(app, pressed_pass(ctx));
    let keys = |chord: &_| holder(ctx, chord);
    let options = options().keys(&keys);
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    let bar = &app.menus.bar;
    for c in menu.sync_with(&state, || mac_layout(bar, &options)) {
        log::info!("menu: {} is {} on macOS and {} in VectorCraft, which keeps it", c.shortcut.portable(), c.system, c.command.id);
    }
}
