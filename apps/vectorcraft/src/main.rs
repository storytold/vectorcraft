//! VectorCraft desktop app.
//!
//! Usage: `vectorcraft [--control <port> [--automation-read-root <dir>]... [--automation-write-root <dir>]...]
//! [--in-window-menus] [--new-instance] [--safe-mode] [--present-mode <mode>] [--frame-latency <frames>] [--] [files…]`,
//! `vectorcraft --version`, `vectorcraft --help` (or `-h`)
//!
//! Any other argument starting with '-' (not `-` alone) is an error; after `--`, every argument is
//! a file.
//!
//! Files given while VectorCraft already runs open in that window, unless `--new-instance` is given
//! (Windows and Linux; `single_instance`).
//!
//! `--in-window-menus` (or `VECTORCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar.
//!
//! `--control <port>` (or `VECTORCRAFT_CONTROL_PORT`; a port from 0 to 65535, 0 picking a free
//! one, or the app doesn't start) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"ui.inspect","params":{}}` → `{"id":1,"ok":true,"result":…}`.
//! `--control-token <token>` (or `VECTORCRAFT_CONTROL_TOKEN`) or `--control-token-file <file>`
//! (or `VECTORCRAFT_CONTROL_TOKEN_FILE`) sets the token each connection sends first.
//! See `vectorcraft_ui_egui::control` for the methods. `--automation-read-root` and
//! `--automation-write-root` (or `VECTORCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the
//! files its requests read and write (`vectorcraft_engine::file_access`); the person at the
//! keyboard isn't confined.
//!
//! `--present-mode` and `--frame-latency` (or `VECTORCRAFT_PRESENT_MODE` / `_FRAME_LATENCY`) change
//! how the window's frames reach the display (`presentation`).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod automation_args;
#[cfg(test)]
mod build_channel;
mod build_flavor;
mod clipboard;
#[cfg(feature = "wgpu")]
mod compositor_gpu;
mod cursor_theme;
mod display_profile;
#[cfg(feature = "wgpu")]
mod gpu;
mod hang_watch;
mod launch_profile;
#[cfg(target_os = "linux")]
mod linux_stylus;
mod logging;
#[cfg(all(target_os = "macos", feature = "wgpu"))]
mod mac_color_space;
#[cfg(target_os = "macos")]
mod mac_fonts;
mod mac_tablet;
#[cfg(target_os = "macos")]
mod mac_window;
mod nightly_session;
#[cfg(target_os = "macos")]
mod open_documents;
mod prefs_dir;
mod presentation;
mod printing;
mod process_identity;
mod profile_root;
#[cfg(not(any(target_os = "macos", windows)))]
mod quit_windows;
#[cfg(not(target_os = "macos"))]
mod single_instance;
#[cfg(windows)]
mod system_fonts;
mod tablet;
mod window_frame;
mod windows_alt_codes;
#[cfg(windows)]
mod windows_cores;
mod windows_stylus;
mod windows_symbol_chars;
mod xwayland;

use std::sync::{Arc, Mutex};

use craft::automation::args::{ControlOptions, EnvNames};
use craft::automation::server::{self, Listening};
use craft::dialogs::service::{FilePanels, FileService};
use craft::host::target::{Backend, ShellInfo};
use craft::host::{AppInfo, Wake};
use craft::settings::SharedSettings;
use craft::ui::InstallSpec;
use craft::ui::shell::embed;
use craft::window::Prefix;
use craft::window::policy::{TitleBarPref, decide_for};
use craft::window::restore::Store;
use craft::window::system::SystemWindows;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_ui_egui::dialogs::safe_mode;
use vectorcraft_ui_egui::graphics::GraphicsLoss;
use vectorcraft_ui_egui::{AppSettings, Services, VectorcraftApp};

mod app;
mod host_services;
mod prefs;

#[cfg(all(test, feature = "wgpu"))]
use app::bring_window_forward;
use app::open_or_hold;
use host_services::{file_service, services, start_pen};
use prefs::{load_prefs, log_dir, open_store, prefs_enabled};

/// The window's size when no saved placement overrides it, and the smallest it may be, in points.
const DEFAULT_SIZE: [f32; 2] = [1440.0, 900.0];
const MIN_SIZE: [f32; 2] = [800.0, 500.0];

/// The window, Dock, taskbar and app-switcher icon (`assets/app-icon/`, see its README). macOS gets
/// the version with Apple's transparent margin; elsewhere the full-bleed tile. The app ID matches
/// `packaging/linux/ai.storyteller.vectorcraft.desktop` so Wayland docks find the launcher icon.
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/vectorcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Windows shows a window's big icon (`ICON_BIG`) in the taskbar and Alt+Tab; eframe sets only the
/// small one (the title bar's), so the taskbar showed a generic icon. Set the big one too.
#[cfg(windows)]
fn set_taskbar_icon(w: &winit::window::Window) {
    use winit::platform::windows::WindowExtWindows as _;
    let icon = app_icon();
    // An icon that can't be made leaves the generic one: nothing else depends on it.
    if let Ok(i) = winit::window::Icon::from_rgba(icon.rgba, icon.width, icon.height) {
        w.set_taskbar_icon(Some(i));
    }
}

/// "name (backend, kind)" of the adapter the window renders with, for Help › About and bug reports.
#[cfg(feature = "wgpu")]
fn adapter_summary(info: &eframe::wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type)
}

/// The window's options: the app draws the title bar when `custom_titlebar`, else the desktop does.
fn native_options(custom_titlebar: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(vectorcraft_ui_egui::os_window::name())
            .with_inner_size(DEFAULT_SIZE)
            .with_min_inner_size(MIN_SIZE)
            .with_drag_and_drop(true)
            .with_decorations(!custom_titlebar)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(app_icon())
            .with_app_id(vectorcraft_ui_egui::os_window::app_id()),
        // A nightly keeps its own window state in its private profile; a stable build lets eframe
        // keep it where it always did.
        persistence_path: nightly_window_state(),
        ..Default::default()
    }
}

/// Where eframe keeps the window's size and position: a nightly's private profile ([`nightly_root`]),
/// never the folder the stable app's own window state sits in.
fn nightly_window_state() -> Option<std::path::PathBuf> {
    nightly_root().map(|root| root.join(craft::window::restore::FILE_NAME))
}

/// The saved window placement (the framework's `window.json`): beside the preferences, as it always
/// was, or inside a nightly's private profile so the two channels never move each other's windows.
fn placement_store() -> Store {
    match nightly_root() {
        Some(root) => Store::at(&root.join(craft::window::restore::FILE_NAME)),
        None => Store::open("vectorcraft"),
    }
}

/// This launch's private Nightly profile, when it is one.
fn nightly_root() -> Option<&'static std::path::Path> {
    launch_profile::current().root()
}

struct App {
    app: VectorcraftApp,
    /// The preferences, written through [`prefs::save_tick`] and on exit.
    settings: SharedSettings<AppSettings>,
    /// The file service the host's `Files` is, kept for the folders its picks chose.
    files: Arc<FileService>,
    /// Shows the native dialogs the file service was asked for.
    file_panels: FilePanels,
    /// The interface language the Replace question of [`Self::file_panels`] is worded in.
    replace_lang: Option<vectorcraft_ui_egui::i18n::Lang>,
    /// Keeps the control server running while the app does.
    _control_server: Option<Listening>,
    /// Reported by wgpu when the window's graphics device is lost (a driver reset).
    graphics_loss: GraphicsLoss,
    /// The graphics device was lost and the unsaved changes are kept for Data Recovery.
    graphics_lost: bool,
    /// Frames the UI has run (see [`end_before_teardown`]).
    frames: u64,
    /// Keeps the custom title bar's window borderless (Windows, #1758).
    window_frame: Option<window_frame::WindowFrameFn>,
    /// The main window; used to read and set maximized state the UI layer does not see on Linux (#3635),
    /// and to take the window off screen on quit before the clipboard handles let go (`quit_windows`).
    winit_window: Option<std::sync::Arc<winit::window::Window>>,
    /// The command line's files, held back until the choice about a stopped plug-in is made.
    held_files: Vec<String>,
    /// Reads the profile of the display that shows the main window, for the canvas.
    display_profile: display_profile::Reader,
    /// When this frame's input hook started, for the hang report's frame times (`hang_watch`).
    frame_start: Option<std::time::Instant>,
    /// The shortest time between the starts of two frames, while the window presents without vsync
    /// ([`presentation::min_frame_interval`]).
    min_frame: Option<std::time::Duration>,
    /// When the last frame started, after its wait for [`Self::min_frame`].
    last_frame_began: Option<std::time::Instant>,
}

/// `vectorcraft --help`. Headless work (convert, run, MCP) is `vectorcraft-cli`'s.
const USAGE: &str = "\
Usage: vectorcraft [options] [files…]

Opens the VectorCraft desktop app with the given files.

Options:
  --control <port>               Start the localhost JSON-lines control server on <port>
                                 (or VECTORCRAFT_CONTROL_PORT)
  --control-token <token>        The token control connections send first
                                 (or VECTORCRAFT_CONTROL_TOKEN)
  --control-token-file <file>    Read that token from <file>
                                 (or VECTORCRAFT_CONTROL_TOKEN_FILE)
  --automation-read-root <dir>   Confine the files control requests read to <dir>
                                 (may be given more than once)
  --automation-write-root <dir>  Confine the files control requests write to <dir>
                                 (may be given more than once)
  --in-window-menus              Keep the menus inside the window on macOS
                                 (or VECTORCRAFT_IN_WINDOW_MENUS=1)
  --new-instance                 Open the files in a new window even when VectorCraft
                                 already runs (Windows and Linux)
  --safe-mode                    Start without plug-ins: none loads or runs until
                                 VectorCraft starts normally (Help › Safe Mode); the
                                 files open in a new window, as with --new-instance
  --present-mode <mode>          How frames reach the display: fifo (vsync),
                                 autovsync (the default; relaxed vsync where the
                                 driver offers it) or novsync
                                 (or VECTORCRAFT_PRESENT_MODE)
  --frame-latency <frames>       Frames the display may queue, 1 to 3 (the default
                                 is 1) (or VECTORCRAFT_FRAME_LATENCY); with either
                                 one set, the files open in a new window, as with
                                 --new-instance
  --version                     Print the version and exit
  -h, --help                     Print this help and exit
  --                             Treat every argument after it as a file

Any other argument starting with '-' is an error.

Without a window (converting, scripting, the MCP server), use vectorcraft-cli:
  vectorcraft-cli --help
";

/// The command line's options other than the automation roots and the control port.
struct CommandLine {
    files: Vec<String>,
    in_window_menus: bool,
    new_instance: bool,
    /// `--safe-mode`: no plug-in loads or runs in this launch.
    safe_mode: bool,
}

/// Reads the arguments after the program name. The automation roots, the control port and the
/// presentation overrides (with `VECTORCRAFT_PRESENT_MODE` and `VECTORCRAFT_FRAME_LATENCY` filling
/// what the flags leave unset) go into `roots_args`, `control` and `presentation`. `Err` is the exit code
/// when the process ends here: `--version` and `--help` print and succeed; a bad automation or
/// presentation option or an unknown option prints an error and fails.
fn read_command_line(
    mut args: impl Iterator<Item = String>,
    control: &mut automation_args::ControlArg,
    roots_args: &mut automation_args::AutomationArgs,
    presentation: &mut presentation::Overrides,
) -> Result<CommandLine, std::process::ExitCode> {
    let mut files = Vec::new();
    let mut in_window_menus = std::env::var_os("VECTORCRAFT_IN_WINDOW_MENUS").is_some_and(|v| !v.is_empty() && v != "0");
    let mut new_instance = false;
    let mut safe_mode = false;
    let mut unknown = None;
    while let Some(a) = args.next() {
        // After `--`, every argument is a file, even one whose name starts with '-'.
        if a == "--" {
            files.extend(args.by_ref());
            break;
        }
        match roots_args
            .consume(&a, &mut args)
            .and_then(|root| if root { Ok(true) } else { control.consume(&a, &mut args) })
            .and_then(|taken| if taken { Ok(true) } else { presentation.consume(&a, &mut args) })
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return Err(std::process::ExitCode::FAILURE);
            }
        }
        match a.as_str() {
            "--in-window-menus" => in_window_menus = true,
            "--new-instance" => new_instance = true,
            "--safe-mode" => safe_mode = true,
            "--version" => {
                // A nightly says what it is, like the early answer; a stable build's line is unchanged.
                let line = match build_flavor::COMPILED {
                    build_flavor::BuildFlavor::Stable => format!("vectorcraft {}\n", vectorcraft_ui_egui::os_window::version()),
                    build_flavor::BuildFlavor::NightlyV1 => {
                        format!("VectorCraft {}\n", build_flavor::version(build_flavor::COMPILED, vectorcraft_ui_egui::os_window::version()))
                    }
                };
                let _ = std::io::Write::write_all(&mut std::io::stdout(), line.as_bytes());
                return Err(std::process::ExitCode::SUCCESS);
            }
            // Before anything starts, like `--version`: from a terminal or an agent's shell the app
            // used to open (or hang without a display) instead (#2110).
            "--help" | "-h" => {
                // Not `print!`, which panics when standard output is closed (`| head -0`):
                // there is nothing left to tell then.
                let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
                return Err(std::process::ExitCode::SUCCESS);
            }
            // The process serial number older macOS versions pass to an app the Finder starts.
            #[cfg(target_os = "macos")]
            _ if a.starts_with("-psn_") => {}
            // An option this version doesn't know (a typo such as `--safe-mod`) is an error, not a
            // file to open. Reported after the loop, so a later `--help` or `--version` still wins.
            _ if a.starts_with('-') && a != "-" => {
                unknown.get_or_insert(a);
            }
            _ => files.push(a),
        }
    }
    if let Some(a) = unknown {
        eprintln!("vectorcraft: unknown option {a}; see vectorcraft --help, or put -- before a file whose name starts with '-'");
        return Err(std::process::ExitCode::FAILURE);
    }
    // After the arguments: the flags, `--version` and `--help` win over a bad environment value.
    match presentation.with_env(std::env::var_os("VECTORCRAFT_PRESENT_MODE"), std::env::var_os("VECTORCRAFT_FRAME_LATENCY")) {
        Ok(overrides) => *presentation = overrides,
        Err(e) => {
            eprintln!("vectorcraft: {e}");
            return Err(std::process::ExitCode::FAILURE);
        }
    }
    Ok(CommandLine { files, in_window_menus, new_instance, safe_mode })
}

/// Whether the window opens through XWayland, for Linux Display Server `pref` (`main`).
#[cfg(target_os = "linux")]
fn opens_through_xwayland(pref: Option<&str>) -> bool {
    if !xwayland::wanted(pref, |name| std::env::var_os(name), xwayland::pen_attached) {
        return false;
    }
    if !xwayland::x11_libraries_found() {
        log::warn!("Linux Display Server is X11, but the X11 libraries aren't all installed: opening the window on Wayland");
        return false;
    }
    if pref != Some("x11") {
        log::info!("pen found: opening the window through XWayland, where the pen works (VECTORCRAFT_NATIVE_WAYLAND=1 keeps Wayland)");
    } else {
        log::info!("opening the window through XWayland (Preferences › Performance › Linux Display Server)");
    }
    true
}

/// The nightly channel this build is, as a string for the log.
fn channel_name() -> &'static str {
    match build_flavor::COMPILED {
        build_flavor::BuildFlavor::Stable => "stable",
        build_flavor::BuildFlavor::NightlyV1 => launch_profile::VERSION,
    }
}

/// Says why a nightly launch was refused, then exits 2. A window could not have shown it yet: this
/// app has no message box of its own before eframe starts (its dialogs are egui windows), so the
/// text goes to standard error, which a Launcher launch records. The desktop-session test still
/// decides whether a daily user would have seen it, for the message's wording.
fn report_launch_profile_error(error: &str) {
    eprintln!("VectorCraft launch profile: {error}");
    let desktop_session = cfg!(windows)
        || cfg!(target_os = "macos")
        || (cfg!(target_os = "linux") && ["DISPLAY", "WAYLAND_DISPLAY"].into_iter().any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty())));
    if build_flavor::needs_native_error(build_flavor::COMPILED, desktop_session) {
        eprintln!("VectorCraft Nightly: open it from ArtCraft Launcher, which starts it in its own profile");
    }
}

/// Selects the immutable launch profile before any startup service or side effect, and returns the
/// arguments the rest of `main` reads. `None` means the process ends here (`--version` answered).
fn prepare_launch() -> Option<Vec<std::ffi::OsString>> {
    // The trusted Launcher protocol is decided before the logger, updates, fonts or any saved state.
    let argv: Vec<_> = std::env::args_os().skip(1).collect();
    // A nightly answers its version before root validation, process identity, IPC or any window:
    // plain `--version`/`-V`, or the Launcher's `--launch-profile nightly-v1 --profile-root <dir>
    // --version`. A stable build leaves the answer to `read_command_line`, exactly as before.
    if build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1 && (build_flavor::version_query(&argv) || launch_profile::version_query(&argv))
    {
        use std::io::Write as _;
        let line = format!("VectorCraft {}\n", build_flavor::version(build_flavor::COMPILED, env!("VECTORCRAFT_BUILT_VERSION")));
        let _ = std::io::stdout().write_all(line.as_bytes());
        return None;
    }
    let early = launch_profile::parse(argv)
        .and_then(|launch| {
            build_flavor::require_profile(build_flavor::COMPILED, &launch.profile)?;
            Ok(launch)
        })
        .and_then(|launch| {
            launch_profile::prepare(launch, |root| {
                launch_profile::validate_root(root, |key| std::env::var_os(key), std::env::current_exe().ok().as_deref())
            })
        })
        .unwrap_or_else(|error| {
            report_launch_profile_error(&error);
            std::process::exit(2)
        });
    let identity = if early.profile.is_nightly() {
        vectorcraft_ui_egui::os_window::LaunchIdentity::NightlyV1
    } else {
        vectorcraft_ui_egui::os_window::LaunchIdentity::Stable
    };
    let install = launch_profile::install(early.profile).and_then(|()| {
        // Every data path follows the selected profile from here on: settings, logs, recovery, the
        // user's libraries, plug-ins, caches and the single-instance record.
        if let Some(root) = launch_profile::current().root() {
            prefs_dir::profile::install_data_root(root.to_path_buf())?;
        }
        vectorcraft_ui_egui::os_window::select_identity(identity)
    });
    if let Err(error) = install {
        report_launch_profile_error(&error);
        std::process::exit(2);
    }
    if let Err(error) = vectorcraft_ui_egui::os_window::set_version(env!("VECTORCRAFT_BUILT_VERSION")) {
        report_launch_profile_error(&error);
        std::process::exit(2);
    }
    Some(early.args)
}

fn main() -> std::process::ExitCode {
    // The launch profile first: a nightly build without one is refused here, before the update
    // helper, the logger, fonts or any saved state, and `--version` is answered here too.
    let Some(arguments) = prepare_launch() else { return std::process::ExitCode::SUCCESS };
    let nightly = launch_profile::current().is_nightly();
    // A nightly's taskbar identity is set before the first window: otherwise Windows groups it with
    // the stable app and the Launcher's own entry.
    if let Err(error) =
        process_identity::initialize(nightly, vectorcraft_ui_egui::os_window::id_for(vectorcraft_ui_egui::os_window::LaunchIdentity::NightlyV1))
    {
        report_launch_profile_error(&error.to_string());
        return std::process::ExitCode::FAILURE;
    }
    // Windows: the update helper is a copy of this program started with `--finish-update`. Nightly is
    // updated by the Launcher, so it never starts one (`build_flavor` keeps its updater off).
    #[cfg(windows)]
    if !nightly
        && let Some(result) = craft::update::self_update::finish(&vectorcraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>())
    {
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
    let mut presentation = presentation::Overrides::default();
    // A nightly's profile tokens were consumed by `prepare_launch`: what is left is `--` and the
    // files to open. A stable launch reads its arguments exactly as it always did.
    let given: Vec<String> =
        if nightly { arguments.iter().map(|a| a.to_string_lossy().into_owned()).collect() } else { std::env::args().skip(1).collect() };
    let CommandLine { files: command_line_files, in_window_menus, new_instance, safe_mode: safe_mode_flag } =
        match read_command_line(given.into_iter(), &mut control, &mut roots_args, &mut presentation) {
            Ok(command_line) => command_line,
            Err(code) => return code,
        };
    let automation_args::AutomationArgs { read_roots, write_roots, .. } = roots_args;
    // After the arguments: the flags and `--version` win over a bad environment value. A nightly is
    // started by the Launcher, never by an agent, so the control channel and its inherited
    // environment stay off: no token file is read and nothing listens.
    let (control_port, control_token) = if nightly {
        if control.options().port.is_some() || std::env::var_os("VECTORCRAFT_CONTROL_PORT").is_some() {
            eprintln!("vectorcraft: the control channel is off in a nightly build");
        }
        (None, None)
    } else {
        let control_options = match ControlOptions::resolve(control.options(), &EnvNames::for_app("VECTORCRAFT")) {
            Ok(options) => options,
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let control_port = control_options.port;
        // Read before the window opens: a token file that can't be read keeps the app from starting
        // rather than leaving the channel open without one.
        let control_token = match control_port.map(|_| control_options.server_token()).transpose() {
            Ok(token) => token.map(|token| (control_options.token_notice(&token), token)),
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        (control_port, control_token)
    };
    // Before desktop services, so every start-up warning is recorded (`logging`).
    let logger = logging::install();
    log::info!("VectorCraft {} ({channel_name()})", vectorcraft_ui_egui::os_window::version());
    if nightly {
        log::info!("nightly profile: {}", nightly_root().map(|root| root.display().to_string()).unwrap_or_default());
    }
    // Files opened from the file manager while VectorCraft runs open in its window (#4892). Before
    // the log file is attached: a launch that hands its files over leaves the running app's log be.
    #[cfg(not(target_os = "macos"))]
    // A launch that sets the presentation runs as its own app: a running app keeps its own surface.
    match single_instance::start(control_port.is_some(), new_instance || presentation.is_set(), safe_mode_flag, &command_line_files) {
        single_instance::Start::Forwarded => return std::process::ExitCode::SUCCESS,
        single_instance::Start::Running(primary) => single_instance::hold(primary),
    }
    #[cfg(target_os = "macos")]
    let _ = new_instance;
    // Before the first render: the cores the process may run on.
    #[cfg(windows)]
    windows_cores::install();
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
                Ok(path) => log::info!("VectorCraft {}, log file {}", vectorcraft_ui_egui::os_window::version(), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
        }
    }
    // A nightly's own session record: the previous session that did not finish normally is reported
    // once the window is up, and this one is cleared only on a controlled return (`on_exit`, or the
    // code after the event loop). Stable keeps no record.
    let previous_nightly_unclean = if nightly {
        match nightly_session::begin(nightly_root(), vectorcraft_ui_egui::os_window::version(), None) {
            Ok(unclean) => unclean,
            Err(e) => {
                log::warn!("Nightly session health is unavailable: {e}");
                false
            }
        }
    } else {
        false
    };
    // The report a session left when it was closed while it did not answer (`hang_watch`).
    let previous_hang = log_dir().filter(|_| prefs_enabled()).and_then(|dir| hang_watch::take_previous(&dir));
    if let Some(report) = &previous_hang {
        log::warn!("the last session was closed while it did not answer; its report:\n{}", report.trim_end());
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
    let (settings, settings_report) = open_store();
    if settings_report.has_problems() {
        log::warn!("the preferences were not read as saved: {settings_report:?}");
    }
    // An engine preference the window's start reads before the app exists.
    let engine_pref = |key: &str| settings.lock().get().ui.engine_prefs.get(key).cloned();
    #[cfg(feature = "wgpu")]
    let gpu_pref_value = engine_pref("gpuPreference");
    #[cfg(feature = "wgpu")]
    let gpu_pref = gpu_pref_value.as_ref().and_then(serde_json::Value::as_str);
    // Preferences › Performance › Low-Latency Drawing; anything but `true` in the file is off.
    #[cfg(feature = "wgpu")]
    let low_latency_drawing = engine_pref("lowLatencyDrawing").and_then(|v| v.as_bool()).unwrap_or(false);
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
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; Preferences › Performance › Linux
    // Display Server › X11 opens the window through XWayland instead, where file drops work (#1151),
    // and the other choices do so when a pen is attached, where the pen draws (#2028, #4967).
    #[cfg(target_os = "linux")]
    let forced_to_x11 = opens_through_xwayland(engine_pref("linuxDisplayServer").as_ref().and_then(serde_json::Value::as_str));
    #[cfg(not(target_os = "linux"))]
    let forced_to_x11 = false;
    // A window forced onto XWayland runs on X11, whose window manager draws the frame: the
    // framework decides the frame for that backend, from the platform, `VECTORCRAFT_TITLEBAR` and
    // Preferences › User Interface › System Title Bar.
    let shell_info = if forced_to_x11 { ShellInfo::current().backend(Backend::X11) } else { ShellInfo::current() };
    let titlebar_pref =
        if engine_pref("systemTitleBar").and_then(|v| v.as_bool()).unwrap_or(false) { TitleBarPref::System } else { TitleBarPref::Integrated };
    let decision = decide_for(Prefix::new("VECTORCRAFT"), shell_info.backend, titlebar_pref);
    let custom_titlebar = matches!(decision.frame, craft::window::policy::Frame::Integrated(_));
    let options = native_options(custom_titlebar);
    #[cfg(target_os = "linux")]
    let options = {
        let mut options = options;
        if forced_to_x11 {
            use winit::platform::x11::EventLoopBuilderExtX11 as _;
            options.event_loop_builder = Some(Box::new(|builder| {
                builder.with_x11();
            }));
        }
        options
    };
    // With Windows Ink, winit reports a pen as a touch and drops its barrel button and which end
    // is down; the message hook notes each pointer contact and whether it starts with the barrel
    // button held, and writes the pen's end into the pen feed (`windows_stylus`). It also notes the
    // characters Alt + keypad codes type (`windows_alt_codes`).
    #[cfg(windows)]
    let options = {
        use winit::platform::windows::EventLoopBuilderExtWindows as _;
        let mut options = options;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_msg_hook(|msg| {
                // Alt + keypad codes (#5108), which winit drops; neither hook consumes a message.
                windows_alt_codes::msg_hook(msg);
                windows_stylus::msg_hook(msg)
            });
        }));
        options
    };
    // Quitting takes the X11 windows of floating panels and documents off screen with the main
    // window, so eframe's windows are recorded as it makes and drops them (`quit_windows`).
    #[cfg(target_os = "linux")]
    let options = eframe::NativeOptions { viewport_hooks: Some(std::sync::Arc::new(quit_windows::Hooks)), ..options };
    #[cfg(feature = "wgpu")]
    let options = {
        let mut options = options;
        // One frame queued, not two: the canvas is rasterized on the CPU and the GPU only
        // composites it, so the window answers the pointer a frame sooner (#444).
        // `--present-mode` and `--frame-latency` change it for a launch, and Low-Latency Drawing
        // asks for no vsync when neither sets the mode (`presentation`).
        options.wgpu_options.surface = presentation.apply(eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY, low_latency_drawing);
        // What the app asks for: the driver picks novsync's mode and may clamp the latency.
        log::info!("requested window surface: {}", presentation::describe(&options.wgpu_options.surface, &presentation, low_latency_drawing));
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
            // The device asks for no more than the adapter offers, so a GPU short of one of
            // eframe's limits still gets a device (#2105). The window's surface may be as large as
            // the adapter allows, not eframe's fixed 8192 px: a window stretched over several
            // monitors, or grown by a display scale change, otherwise fails to configure its
            // surface and the app closes (#1459).
            let device = create.device_descriptor.clone();
            create.device_descriptor = std::sync::Arc::new(move |adapter: &eframe::wgpu::Adapter| {
                let mut descriptor = device(adapter);
                descriptor.required_limits = gpu::device_limits(descriptor.required_limits, &adapter.limits());
                descriptor
            });
        }
        options
    };
    #[cfg(not(feature = "wgpu"))]
    if presentation != presentation::Overrides::default() {
        log::warn!("--present-mode and --frame-latency set the wgpu renderer's window surface: ignored without it");
    }
    #[cfg(feature = "wgpu")]
    gpu::watch_panics();
    #[cfg(feature = "wgpu")]
    gpu::watch_first_frame(startup.clone());
    // Files opened from Finder and the Dock arrive as events, not arguments.
    #[cfg(target_os = "macos")]
    open_documents::install();
    #[cfg(feature = "wgpu")]
    let created = startup.clone();
    // For the hang report (`hang_watch`).
    #[cfg(feature = "wgpu")]
    let requested_surface = options.wgpu_options.surface;
    // Without vsync, frames start at most 240 times a second (`presentation`).
    #[cfg(feature = "wgpu")]
    let min_frame = presentation::min_frame_interval(&requested_surface);
    #[cfg(not(feature = "wgpu"))]
    let min_frame = None;
    // A panic that ends the window (egui-wgpu's own, #502) is caught here: never a crash.
    let run = |options: eframe::NativeOptions, app_creator: eframe::AppCreator<'_>| {
        #[cfg(target_os = "linux")]
        {
            linux_stylus::run_native(vectorcraft_ui_egui::os_window::name(), options, app_creator)
        }
        #[cfg(not(target_os = "linux"))]
        {
            eframe::run_native(vectorcraft_ui_egui::os_window::name(), options, app_creator)
        }
    };
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        run(
            options,
            Box::new(move |cc| {
                let ctx = &cc.egui_ctx;
                let winit_window = cc.winit_window().cloned();
                let spec = InstallSpec::new(shell_info, Arc::new(SystemWindows))
                    .app_id(vectorcraft_ui_egui::os_window::app_id())
                    .placement(Arc::new(Mutex::new(placement_store())))
                    .frame(decision);
                let spec = match start_pen(&winit_window) {
                    Some(pen) => spec.pen(pen),
                    None => spec,
                };
                embed::install(ctx, spec, &vectorcraft_ui_egui::ui_fonts()).map_err(|e| e.to_string())?;
                egui_extras::install_image_loaders(ctx);
                let folders = settings.lock().get().ui.last_folders.clone();
                let wake = {
                    let ctx = ctx.clone();
                    Wake::new(move || ctx.request_repaint())
                };
                let (files, file_panels) = file_service(wake, folders);
                let files = Arc::new(files);
                craft::host::Host::builder(AppInfo {
                    id: vectorcraft_ui_egui::os_window::app_id(),
                    name: vectorcraft_ui_egui::os_window::name(),
                    version: vectorcraft_ui_egui::os_window::version(),
                })
                .i18n(vectorcraft_ui_egui::i18n::shared())
                .settings(Arc::new(settings.clone()))
                .files(files.clone())
                .install(ctx);
                let mut session = Session::new();
                // The profiles loaded before come back before the preferences name any of them.
                prefs_dir::attach_profile_store(&mut session);
                // Then Edit › Color Settings, which may name one of those profiles.
                prefs_dir::attach_color_settings(&mut session);
                // Before any plug-in loads: this launch's marker in the Launch folder next to the
                // preferences (none for runs without preferences), and Safe Mode.
                let launch_dir = prefs_dir::prefs_dir().filter(|_| prefs_enabled()).map(|d| d.join("Launch"));
                let folder = |name: &str| prefs_dir::prefs_dir().map(|d| d.join(name).to_string_lossy().to_string());
                let stopped = safe_mode::begin_launch(launch_dir.as_deref(), safe_mode_flag);
                prefs_dir::attach_plugin_store(&mut session);
                // File › Scripts lists the scripts of the Scripts folder next to the preferences.
                prefs_dir::attach_scripts_dir(&mut session);
                // File Info's metadata templates live in their own folder next to the preferences.
                prefs_dir::attach_metadata_templates_dir(&mut session);
                prefs_dir::attach_new_doc_profiles(&mut session);
                prefs_dir::attach_proof_setups(&mut session);
                prefs_dir::attach_script_data(&mut session);
                let mut app = VectorcraftApp::new(session, services(winit_window.clone()));
                app.adopt_context(&cc.egui_ctx);
                // A nightly is updated by the Launcher: it never checks for a stable release, which
                // would install this program's own updater over the nightly build.
                app.set_startup_update_check(!nightly);
                load_prefs(&mut app, &settings);
                safe_mode::after_start(&mut app, stopped);
                // The saved PDF presets live in their own folder (also read by vectorcraft-cli),
                // which takes over from the copy the preferences file used to hold.
                prefs_dir::attach_pdf_preset_store(&mut app.session);
                // The custom dictionary's words, kept in their own file next to the preferences.
                prefs_dir::attach_custom_dictionary(&mut app.session);
                // The custom arrowheads, kept in their own file next to the preferences.
                prefs_dir::attach_arrowheads(&mut app.session);
                // The custom bevels, kept in their own file next to the preferences.
                prefs_dir::attach_bevels(&mut app.session);
                // The window's placement comes back through the framework's placement store.
                #[cfg(windows)]
                if let Some(w) = &winit_window {
                    set_taskbar_icon(w);
                }
                // User Defined swatch, graphic style, symbol and brush libraries live next to the preferences.
                app.session.swatch_libraries.set_user_dir(folder("Swatches"));
                app.session.style_libraries.set_user_dir(folder("Graphic Styles"));
                app.session.symbol_libraries.set_user_dir(folder("Symbols"));
                app.session.brush_libraries.set_user_dir(folder("Brushes"));
                // With the User Defined libraries listed, the persistent library panels reopen.
                vectorcraft_ui_egui::panels::library_panel::restore(&mut app);
                // So do the Libraries panel's libraries; runs without preferences (agents' test
                // runs) keep theirs for the session only, never touching the user's.
                if prefs_enabled() {
                    app.session.libraries.set_dir(folder("Libraries"));
                }
                // Data Recovery copies live next to the preferences too (none for runs without
                // preferences, such as agents' test runs, unless the recoveryFolder preference is set).
                if std::env::var_os("VECTORCRAFT_NO_PREFS").is_none() {
                    app.session.recovery.set_default_folder(folder("Data Recovery"));
                }
                let graphics_loss = GraphicsLoss::default();
                #[cfg(feature = "wgpu")]
                if let Some(rs) = &cc.wgpu_render_state {
                    let summary = adapter_summary(&rs.adapter.get_info());
                    log::info!("rendering with {summary} (power preference {power:?})");
                    created.created(&cc.egui_ctx);
                    if !gpu::skipped().is_empty() {
                        // A processor chosen in Settings can be the one that can't show the window.
                        if chosen_gpu {
                            app.status(format!(
                                "The graphics processor tried first couldn't show the window, so VectorCraft started again on {summary} (Settings › Performance › Graphics Processor › Automatic uses the one that drives your display)"
                            ));
                        } else {
                            app.status(format!(
                                "The graphics processor tried first couldn't show the window, so VectorCraft started again on {summary}"
                            ));
                        }
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
                // The hang report and Help › Copy Troubleshooting Info (`hang_watch`).
                // The surface settings the options asked for; the driver resolves the present mode.
                #[cfg(feature = "wgpu")]
                let surface = cc
                    .wgpu_render_state
                    .as_ref()
                    .map(|_| (format!("{:?}", requested_surface.present_mode), requested_surface.desired_maximum_frame_latency));
                #[cfg(not(feature = "wgpu"))]
                let surface = None;
                let hang = std::sync::Arc::new(hang_watch::Shared {
                    facts: hang_watch::Facts::new(
                        |name| std::env::var(name).ok(),
                        std::path::Path::new("/.flatpak-info").exists(),
                        hang_watch::window_system(winit_window.as_deref()),
                        app.graphics_adapter.clone(),
                        hang_watch::surface(surface),
                    ),
                    started: std::time::Instant::now(),
                    last: std::sync::Mutex::new(None),
                    previous: previous_hang.clone(),
                });
                vectorcraft_ui_egui::hang_beacon::watch_input(&vectorcraft_ui_egui::hang_beacon::BEACON, &cc.egui_ctx);
                hang_watch::start(log_dir().filter(|_| prefs_enabled()), hang.clone());
                app.services.troubleshooting = Some(Box::new(move || {
                    hang_watch::troubleshooting(&hang, &vectorcraft_ui_egui::hang_beacon::BEACON, std::time::SystemTime::now())
                }));
                // A message about the graphics processor, or a stopped plug-in, comes first.
                if previous_hang.is_some() && app.ui.status.is_empty() {
                    vectorcraft_ui_egui::troubleshooting::last_session_stopped(&mut app);
                }
                // The previous nightly session did not finish normally: say so once, in the window
                // that is now up, and only then let a clean exit clear the record.
                if previous_nightly_unclean {
                    app.status(
                        "The previous VectorCraft Nightly session did not finish normally. Nothing has been sent anywhere; Help › About has this build's version for a bug report",
                    );
                    nightly_session::mark_notice_shown();
                }
                let mut control_server = None;
                if let (Some(port), Some((notice, token))) = (control_port, control_token) {
                    match server::listen(port, token, craft::ui::runtime::windows::WindowHost::of(&cc.egui_ctx)) {
                        Ok((rx, listening)) => {
                            eprintln!("{notice}");
                            log::info!("control channel: {listening:?}");
                            control_server = Some(listening);
                            app = app.with_control(rx).with_automation_roots(roots.clone());
                            if let Some(roots) = &roots {
                                // Everywhere the app doesn't say whose work it is (worker threads), the
                                // roots hold: unconfined is only what the person at the keyboard does.
                                file_access::confine_process(roots.clone());
                                log::info!("control channel confined to the automation roots: {}", roots.describe());
                            }
                        }
                        Err(e) => {
                            log::error!("no control channel on port {port}: {e}");
                            eprintln!("vectorcraft: can't listen on port {port}: {e}");
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    open_documents::set_ui(&cc.egui_ctx);
                    // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                    if !in_window_menus {
                        app.services.native_menu = vectorcraft_ui_egui::menus::native::install(&cc.egui_ctx, &mut app);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = in_window_menus;
                    single_instance::set_ui(&cc.egui_ctx);
                }
                let held_files = file_access::unconfined(|| open_or_hold(&mut app, command_line_files));
                // The first frame is due from now (`gpu::watch_first_frame`).
                #[cfg(feature = "wgpu")]
                created.ready();
                // The content and the pointer stay aligned on Windows (#1758).
                let window_frame = window_frame::service(cc, custom_titlebar);
                Ok(Box::new(App {
                    app,
                    settings,
                    files,
                    file_panels,
                    replace_lang: None,
                    _control_server: control_server,
                    graphics_loss,
                    graphics_lost: false,
                    frames: 0,
                    window_frame,
                    winit_window,
                    held_files,
                    display_profile: Default::default(),
                    frame_start: None,
                    min_frame,
                    last_frame_began: None,
                }))
            }),
        )
    });
    // The window is gone: later launches start their own app, including the one a graphics
    // failure starts again below. A nightly's session record is finished here, by the controlled
    // return, before a successor can take the profile; on macOS the window's own `on_exit` already
    // did it (a Dock quit ends the process inside AppKit), so this is then a no-op.
    nightly_session::finish_once();
    #[cfg(not(target_os = "macos"))]
    single_instance::resign();
    // The process ends normally from here: this launch didn't stop while a plug-in was loading.
    safe_mode::end_launch();
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
/// program holds its write side; a test that depends on the start file's lock holds [`no_spawns`].
#[cfg(all(test, feature = "wgpu"))]
static CHILD_SPAWN: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Held by a test that releases a locked file and opens it again (see [`CHILD_SPAWN`]).
#[cfg(all(test, feature = "wgpu"))]
fn no_spawns() -> std::sync::RwLockReadGuard<'static, ()> {
    CHILD_SPAWN.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(all(test, feature = "wgpu"))]
mod tests;
