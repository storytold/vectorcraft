//! VectorCraft desktop app.
//!
//! Usage: `vectorcraft [--control <port> [--automation-read-root <dir>]... [--automation-write-root <dir>]...]
//! [--in-window-menus] [files…]`, `vectorcraft --version`, `vectorcraft --help` (or `-h`)
//!
//! `--in-window-menus` (or `VECTORCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`mac_menu`).
//!
//! `--control <port>` (or `VECTORCRAFT_CONTROL_PORT`; a port from 1 to 65535, or the app doesn't
//! start) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"ui.inspect","params":{}}` → `{"id":1,"ok":true,"result":…}`.
//! See `vectorcraft_ui_egui::control` for the methods. `--automation-read-root` and
//! `--automation-write-root` (or `VECTORCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the
//! files its requests read and write (`vectorcraft_engine::file_access`); the person at the
//! keyboard isn't confined.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod automation_args;
mod clipboard;
#[cfg(feature = "wgpu")]
mod compositor_gpu;
mod control_server;
mod cursor_theme;
#[cfg(feature = "wgpu")]
mod gpu;
#[cfg(target_os = "linux")]
mod linux_stylus;
mod logging;
#[cfg(all(target_os = "macos", feature = "wgpu"))]
mod mac_color_space;
#[cfg(target_os = "macos")]
mod mac_fonts;
#[cfg(target_os = "macos")]
mod mac_menu;
#[cfg(target_os = "macos")]
mod mac_save_format;
#[cfg(target_os = "macos")]
mod mac_screen_color;
mod mac_tablet;
#[cfg(target_os = "macos")]
mod mac_window;
#[cfg(target_os = "macos")]
mod open_documents;
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd", test))]
mod portal_save;
mod prefs_dir;
mod printing;
#[cfg(windows)]
mod system_fonts;
mod tablet;
mod window;
mod window_frame;
mod windows_stylus;
mod xwayland;

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_ui_egui::graphics::GraphicsLoss;
use vectorcraft_ui_egui::picks::PickRequest;
use vectorcraft_ui_egui::{ClipboardProbeFactory, FilePick, Services, VectorcraftApp};

mod app;
mod host_services;
mod prefs;
mod titlebar;

#[cfg(all(test, feature = "wgpu"))]
use app::bring_window_forward;
use app::native_input_hook;
use app::open_files;
use host_services::services;
#[cfg(all(test, feature = "wgpu"))]
use prefs::{KEEP_UI_PREFS, decode_saved_prefs, keep_prefs_copy, prefs_bytes};
use prefs::{load_prefs, log_dir, prefs_enabled, prefs_path, read_prefs};
#[cfg(all(test, feature = "wgpu", any(target_os = "macos", target_os = "windows")))]
use titlebar::CUSTOM_TITLEBAR;
#[cfg(feature = "wgpu")]
use titlebar::adapter_summary;
#[cfg(all(test, feature = "wgpu"))]
use titlebar::custom_titlebar_from;
#[cfg(all(test, feature = "wgpu"))]
use titlebar::opted_into_system_title_bar;
#[cfg(windows)]
use titlebar::set_taskbar_icon;
use titlebar::{custom_titlebar, native_options};

struct App {
    app: VectorcraftApp,
    /// Reported by wgpu when the window's graphics device is lost (a driver reset).
    graphics_loss: GraphicsLoss,
    /// The graphics device was lost and the unsaved changes are kept for Data Recovery.
    graphics_lost: bool,
    /// Frames the UI has run (see [`end_before_teardown`]).
    frames: u64,
    /// Keeps the custom title bar's window borderless (Windows, #1758).
    window_frame: Option<window_frame::WindowFrameFn>,
    /// The main window; used to read and set maximized state the UI layer does not see on Linux (#3635).
    winit_window: Option<std::sync::Arc<winit::window::Window>>,
    /// A saved or fitted geometry asked for maximize once the window is mapped (Linux).
    pending_maximize: bool,
    /// Frames left to retry [`window::apply_pending_maximize`].
    maximize_tries: u8,
}

/// `vectorcraft --help`. Headless work (convert, run, MCP) is `vectorcraft-cli`'s.
const USAGE: &str = "\
Usage: vectorcraft [options] [files…]

Opens the VectorCraft desktop app with the given files.

Options:
  --control <port>               Start the localhost JSON-lines control server on <port>
                                 (or VECTORCRAFT_CONTROL_PORT)
  --automation-read-root <dir>   Confine the files control requests read to <dir>
                                 (may be given more than once)
  --automation-write-root <dir>  Confine the files control requests write to <dir>
                                 (may be given more than once)
  --in-window-menus              Keep the menus inside the window on macOS
                                 (or VECTORCRAFT_IN_WINDOW_MENUS=1)
  --version                      Print the version and exit
  -h, --help                     Print this help and exit

Without a window (converting, scripting, the MCP server), use vectorcraft-cli:
  vectorcraft-cli --help
";

fn main() -> std::process::ExitCode {
    // Windows: the update helper is a copy of this program started with `--finish-update`.
    #[cfg(windows)]
    if let Some(result) = craft_update::self_update::finish(&vectorcraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        std::process::exit(i32::from(result.is_err()));
    }
    // Before any thread starts: the pointer must find a cursor theme on Wayland (#2450).
    #[cfg(target_os = "linux")]
    cursor_theme::apply();
    // Validate roots before logging, font scans, preferences or the window: a missing value
    // must not clear an environment root and start the control channel without confinement.
    let mut control = automation_args::ControlArg::default();
    let root_env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let mut roots_args =
        automation_args::AutomationArgs::new(root_env("VECTORCRAFT_AUTOMATION_READ_ROOT"), root_env("VECTORCRAFT_AUTOMATION_WRITE_ROOT"));
    let mut files = Vec::new();
    let mut in_window_menus = std::env::var_os("VECTORCRAFT_IN_WINDOW_MENUS").is_some_and(|v| !v.is_empty() && v != "0");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match roots_args.consume(&a, &mut args).and_then(|root| if root { Ok(true) } else { control.consume(&a, &mut args) }) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        match a.as_str() {
            "--in-window-menus" => in_window_menus = true,
            "--version" => {
                println!("vectorcraft {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            // Before anything starts, like `--version`: from a terminal or an agent's shell the app
            // used to open (or hang without a display) instead (#2110).
            "--help" | "-h" => {
                // Not `print!`, which panics when standard output is closed (`| head -0`):
                // there is nothing left to tell then.
                let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
                return std::process::ExitCode::SUCCESS;
            }
            _ => files.push(a),
        }
    }
    let automation_args::AutomationArgs { read_roots, write_roots, .. } = roots_args;
    // After the arguments: `--control` and `--version` win over a bad environment value.
    let control_port = match control.port {
        Some(port) => Some(port),
        None => match automation_args::env_control_port(std::env::var_os("VECTORCRAFT_CONTROL_PORT")) {
            Ok(port) => port,
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
    };
    // Before desktop services, so every start-up warning is recorded (`logging`).
    let logger = logging::install();
    vectorcraft_ui_egui::i18n::detect_system_lang_in_background();
    // Before the first font scan (the app's start): the fonts font services load (#579).
    #[cfg(windows)]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    // VectorCraft's own Fonts folder, which text.addFontFiles copies fonts into.
    prefs_dir::install_fonts();
    // The log file lives in the settings directory, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences (agents' test runs) log to standard error only, so they
    // don't rotate away the user's own logs.
    if let Some(logger) = logger {
        match log_dir().filter(|_| prefs_enabled()) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => log::info!("VectorCraft {}, log file {}", env!("CARGO_PKG_VERSION"), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
        }
    }
    // The roots confine the control channel: a bad one keeps the app from starting rather than
    // leaving agents unconfined. Without a control port there is nothing to confine.
    let roots = if control_port.is_none() {
        if !read_roots.is_empty() || !write_roots.is_empty() {
            log::warn!("--automation-read-root and --automation-write-root confine the control channel: ignored without --control");
        }
        None
    } else {
        let read: Vec<&std::path::Path> = read_roots.iter().map(std::path::PathBuf::as_path).collect();
        let write: Vec<&std::path::Path> = write_roots.iter().map(std::path::PathBuf::as_path).collect();
        match AutomationRoots::new(&read, &write) {
            Ok(roots) => roots,
            Err(e) => {
                log::error!("{e}");
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    };
    let saved = read_prefs();
    let saved_window = saved.as_ref().and_then(|ui| ui.window);
    #[cfg(feature = "wgpu")]
    let gpu_pref = saved.as_ref().and_then(|ui| ui.engine_prefs.get("gpuPreference")).and_then(serde_json::Value::as_str);
    #[cfg(feature = "wgpu")]
    let power_env = eframe::wgpu::PowerPreference::from_env();
    #[cfg(feature = "wgpu")]
    let power = gpu::power_preference(gpu_pref, power_env);
    // Automatic draws on the GPU that drives the (primary) display: a GPU without a monitor reset
    // its driver and took every monitor down (pdfcraft#378). Logged first: the first question in
    // every black-window report.
    #[cfg(feature = "wgpu")]
    let displays = gpu::preferred_displays(gpu_pref, power_env);
    // Under Wayland, the GPU the compositor draws with comes even before those (src/compositor_gpu.rs).
    #[cfg(feature = "wgpu")]
    let compositor = gpu::preferred_compositor(gpu_pref, power_env);
    #[cfg(feature = "wgpu")]
    let chosen_gpu = !gpu::automatic(gpu_pref, power_env);
    #[cfg(feature = "wgpu")]
    if !chosen_gpu {
        let listed: Vec<String> = displays.iter().map(ToString::to_string).collect();
        log::info!("display GPUs (PCI vendor:device): {}", if listed.is_empty() { "unknown".to_string() } else { listed.join(", ") });
    } else {
        log::info!("graphics processor chosen by the user ({power:?}): which GPU drives the display isn't considered");
    }
    #[cfg(feature = "wgpu")]
    let startup = std::sync::Arc::new(gpu::Startup::default());
    // A driver that crashes the process while the window starts leaves this start recorded in the
    // log folder; the next start leaves that adapter out (#2243). Runs without preferences, which
    // write no log file, don't record their starts.
    #[cfg(feature = "wgpu")]
    let left_out = match log_dir().filter(|_| prefs_enabled()) {
        Some(dir) => startup.track(&dir, gpu_pref, gpu::explicit_choice()),
        None => gpu::LeftOut::default(),
    };
    #[cfg(feature = "wgpu")]
    let crash = left_out.crash.clone();
    let custom_titlebar = custom_titlebar(saved.as_ref());
    let options = native_options(custom_titlebar);
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; Preferences › Performance › Linux
    // Display Server › X11 opens the window through XWayland instead, where file drops work (#1151),
    // and X11 (XWayland) When a Pen Is Attached does so when a pen is attached, where the pen draws (#2028).
    #[cfg(target_os = "linux")]
    let options = {
        let mut options = options;
        let pref = saved.as_ref().and_then(|ui| ui.engine_prefs.get("linuxDisplayServer")).and_then(serde_json::Value::as_str);
        if xwayland::wanted(pref, |name| std::env::var_os(name), xwayland::pen_attached) {
            if xwayland::x11_libraries_found() {
                if pref == Some("x11WithPen") {
                    log::info!("pen found: opening the window through XWayland (Preferences › Performance › Linux Display Server)");
                } else {
                    log::info!("opening the window through XWayland (Preferences › Performance › Linux Display Server)");
                }
                use winit::platform::x11::EventLoopBuilderExtX11 as _;
                options.event_loop_builder = Some(Box::new(|builder| {
                    builder.with_x11();
                }));
            } else {
                log::warn!("Linux Display Server is X11, but the X11 libraries aren't all installed: opening the window on Wayland");
            }
        }
        options
    };
    // With Windows Ink, winit reports a pen as a touch and drops its barrel button and which end
    // is down; the message hook notes each pointer contact and whether it starts with the barrel
    // button held, and writes the pen's end into the pen feed (`windows_stylus`).
    #[cfg(windows)]
    let options = {
        use winit::platform::windows::EventLoopBuilderExtWindows as _;
        let mut options = options;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_msg_hook(windows_stylus::msg_hook);
        }));
        options
    };
    #[cfg(feature = "wgpu")]
    let options = {
        let mut options = options;
        // One frame queued, not two: the canvas is rasterized on the CPU and the GPU only
        // composites it, so the window answers the pointer a frame sooner (#444).
        options.wgpu_options.surface = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
        // Only adapters that can show the window, in the order `gpu` gives (#306, #502).
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            create.native_adapter_selector = Some(gpu::selector(power, displays, compositor, left_out.adapters.clone(), startup.clone()));
            // Nothing draws or dispatches indirectly, so wgpu's check of indirect arguments only
            // costs a compute shader at start-up, one some drivers can't compile (OCLP-patched
            // Metal on an Iris Pro, #651). `WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on.
            create.instance_descriptor.flags =
                (eframe::wgpu::InstanceFlags::from_build_config() - eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
            let env_set = std::env::var_os("WGPU_BACKEND").is_some();
            let opengl = cfg!(windows) && !env_set && gpu::windows_opengl(&left_out);
            create.instance_descriptor.backends = left_out.backends(gpu::backends(create.instance_descriptor.backends, env_set, opengl));
            // DX12 shaders compile with FXC, part of Windows: wgpu's default loads whichever
            // dxcompiler.dll the search path finds, and one without its dxil.dll fails every shader,
            // losing the device before the first frame (#1194). `WGPU_DX12_COMPILER` picks another.
            create.instance_descriptor.backend_options.dx12.shader_compiler =
                eframe::wgpu::Dx12Compiler::from_env().unwrap_or(eframe::wgpu::Dx12Compiler::Fxc);
            // The window's surface may be as large as the adapter allows, not eframe's fixed
            // 8192 px: a window stretched over several monitors, or grown by a display scale
            // change, otherwise fails to configure its surface and the app closes (#1459).
            let device = create.device_descriptor.clone();
            create.device_descriptor = std::sync::Arc::new(move |adapter: &eframe::wgpu::Adapter| {
                let mut descriptor = device(adapter);
                let limit = &mut descriptor.required_limits.max_texture_dimension_2d;
                *limit = gpu::surface_limit(*limit, adapter.limits().max_texture_dimension_2d);
                descriptor
            });
        }
        options
    };
    #[cfg(feature = "wgpu")]
    gpu::watch_panics();
    #[cfg(feature = "wgpu")]
    gpu::watch_first_frame(startup.clone());
    // Files opened from Finder and the Dock arrive as events, not arguments.
    #[cfg(target_os = "macos")]
    open_documents::install();
    #[cfg(feature = "wgpu")]
    let created = startup.clone();
    // A panic that ends the window (egui-wgpu's own, #502) is caught here: never a crash.
    let run = |options: eframe::NativeOptions, app_creator: eframe::AppCreator<'_>| {
        #[cfg(target_os = "linux")]
        {
            linux_stylus::run_native("VectorCraft", options, app_creator)
        }
        #[cfg(not(target_os = "linux"))]
        {
            eframe::run_native("VectorCraft", options, app_creator)
        }
    };
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        run(
            options,
            Box::new(move |cc| {
                VectorcraftApp::install_ime_reset_hooks(&cc.egui_ctx);
                let mut session = Session::new();
                // The profiles loaded before come back before the preferences name any of them.
                prefs_dir::attach_profile_store(&mut session);
                prefs_dir::attach_plugin_store(&mut session);
                let mut app = VectorcraftApp::new(session, services(cc.winit_window().cloned(), cc.egui_ctx.clone()));
                app.set_startup_update_check(true);
                load_prefs(&mut app, saved);
                // The saved PDF presets live in their own folder (also read by vectorcraft-cli),
                // which takes over from the copy the preferences file used to hold.
                prefs_dir::attach_pdf_preset_store(&mut app.session);
                // Fit the window to its monitor, or put it back where it was (still hidden).
                let winit_window = cc.winit_window().cloned();
                if let Some(w) = &winit_window {
                    let g = window::restore(w, saved_window);
                    app.ui.window = Some(g);
                    #[cfg(windows)]
                    set_taskbar_icon(w);
                }
                // Linux may not map the window before the first frames, so a saved maximize is retried then (#3635).
                let pending_maximize = cfg!(target_os = "linux") && app.ui.window.as_ref().is_some_and(|g| g.maximized);
                // User Defined swatch, graphic style, symbol and brush libraries live next to the preferences.
                let swatches = prefs_path().and_then(|p| Some(p.parent()?.join("Swatches").to_string_lossy().to_string()));
                app.session.swatch_libraries.set_user_dir(swatches);
                let styles = prefs_path().and_then(|p| Some(p.parent()?.join("Graphic Styles").to_string_lossy().to_string()));
                app.session.style_libraries.set_user_dir(styles);
                let symbols = prefs_path().and_then(|p| Some(p.parent()?.join("Symbols").to_string_lossy().to_string()));
                app.session.symbol_libraries.set_user_dir(symbols);
                let brushes = prefs_path().and_then(|p| Some(p.parent()?.join("Brushes").to_string_lossy().to_string()));
                app.session.brush_libraries.set_user_dir(brushes);
                // With the User Defined libraries listed, the persistent library panels reopen.
                vectorcraft_ui_egui::panels::library_panel::restore(&mut app);
                // So do the Libraries panel's libraries; runs without preferences (agents' test
                // runs) keep theirs for the session only, never touching the user's.
                if prefs_enabled() {
                    let libraries = prefs_path().and_then(|p| Some(p.parent()?.join("Libraries").to_string_lossy().to_string()));
                    app.session.libraries.set_dir(libraries);
                }
                // Data Recovery copies live next to the preferences too (none for runs without
                // preferences, such as agents' test runs, unless the recoveryFolder preference is set).
                if std::env::var_os("VECTORCRAFT_NO_PREFS").is_none() {
                    let recovery = prefs_path().and_then(|p| Some(p.parent()?.join("Data Recovery").to_string_lossy().to_string()));
                    app.session.recovery.set_default_folder(recovery);
                }
                let graphics_loss = GraphicsLoss::default();
                #[cfg(feature = "wgpu")]
                if let Some(rs) = &cc.wgpu_render_state {
                    let summary = adapter_summary(&rs.adapter.get_info());
                    log::info!("rendering with {summary} (power preference {power:?})");
                    created.created(&cc.egui_ctx);
                    if !gpu::skipped().is_empty() {
                        // A processor chosen in Settings can be the one that can't show the window.
                        let hint = if chosen_gpu {
                            " (Settings › Performance › Graphics Processor › Automatic uses the one that drives your display)"
                        } else {
                            ""
                        };
                        app.status(format!(
                            "The graphics processor tried first couldn't show the window, so VectorCraft started again on {summary}{hint}"
                        ));
                    } else if let Some(crash) = &crash {
                        app.status(match crash {
                            gpu::Crash::Adapter { name, .. } => format!(
                                "The last start of VectorCraft ended on {name} before its window appeared, so it started on {summary}. To try that graphics processor again, change Preferences › Performance › Graphics Processor or start with WGPU_BACKEND set"
                            ),
                            gpu::Crash::Listing => format!(
                                "The last start of VectorCraft ended while it listed the graphics processors, so it started on {summary} with OpenGL alone. To try the other graphics backends again, change Preferences › Performance › Graphics Processor or start with WGPU_BACKEND set"
                            ),
                        });
                    }
                    app.graphics_adapter = Some(summary);
                    let (loss, ctx) = (graphics_loss.clone(), cc.egui_ctx.clone());
                    rs.device.set_device_lost_callback(move |reason, msg| loss.report(&ctx, format!("{reason:?}: {msg}")));
                }
                app.custom_titlebar = custom_titlebar;
                if let Some(port) = control_port {
                    let rx = control_server::start(port, cc.egui_ctx.clone());
                    app = app.with_control(rx).with_automation_roots(roots.clone());
                    if let Some(roots) = &roots {
                        // Everywhere the app doesn't say whose work it is (worker threads), the
                        // roots hold: unconfined is only what the person at the keyboard does.
                        file_access::confine_process(roots.clone());
                        log::info!("control channel confined to the automation roots: {}", roots.describe());
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    open_documents::set_ui(&cc.egui_ctx);
                    // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                    if !in_window_menus {
                        app.services.native_menu = mac_menu::install(&cc.egui_ctx, &app);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = in_window_menus;
                app.services.native_input_hook = native_input_hook(&app);
                file_access::unconfined(|| open_files(&mut app, files));
                // The first frame is due from now (`gpu::watch_first_frame`).
                #[cfg(feature = "wgpu")]
                created.ready();
                // The content and the pointer stay aligned on Windows (#1758).
                let window_frame = window_frame::service(cc, custom_titlebar);
                Ok(Box::new(App {
                    app,
                    graphics_loss,
                    graphics_lost: false,
                    frames: 0,
                    window_frame,
                    winit_window,
                    pending_maximize,
                    maximize_tries: 8,
                }))
            }),
        )
    });
    #[cfg(feature = "wgpu")]
    let outcome = gpu::finish(outcome, &startup);
    #[cfg(not(feature = "wgpu"))]
    let outcome = outcome.and_then(|r| r.map_err(|e| e.to_string()));
    match outcome {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("VectorCraft stopped: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Tests only: while a test starts another program, the child briefly holds a copy of every file
/// this process has open, until it runs that program. A file lock held through such a copy outlives
/// its own handle, so a GPU start test that released the start file and opened it again found it
/// still locked, as if another copy of the app were running, and failed. A test that starts a
/// program holds [`spawning`]; a test that depends on the start file's lock holds [`no_spawns`].
#[cfg(all(
    test,
    any(feature = "wgpu", any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))
))]
static CHILD_SPAWN: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Held while a test starts another program (see [`CHILD_SPAWN`]).
#[cfg(all(test, any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
fn spawning() -> std::sync::RwLockWriteGuard<'static, ()> {
    CHILD_SPAWN.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Held by a test that releases a locked file and opens it again (see [`CHILD_SPAWN`]).
#[cfg(all(test, feature = "wgpu"))]
fn no_spawns() -> std::sync::RwLockReadGuard<'static, ()> {
    CHILD_SPAWN.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(all(test, feature = "wgpu"))]
mod tests;
