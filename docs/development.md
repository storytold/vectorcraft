# Development

> **Last updated:** 2026-10-29 · **Change:** Linux clipboard: the Flatpak on GNOME copies and pastes text through the core Wayland protocol.
> **Last updated:** 2026-10-29 · **Change:** Linux clipboard: a clipboard that can't be opened is retried with a growing wait and logged once.
> **Last updated:** 2026-10-25 · **Change:** Linux Wayland drag-and-drop note names eframe 0.37 (still on winit 0.30).
> **Last updated:** 2026-10-23 · **Change:** Tests: cargo xtask test, one integration-test binary per crate, the dev profile's debug info.
> **Last updated:** 2026-10-21 · **Change:** the interface faces as vectorcraft-ui-egui's `interface-fonts` feature, and `theme::install_fonts` for an embedder that draws the panels itself.
> **Last updated:** 2026-10-21 · **Change:** leaving out the bundled fonts with the egui panels (vectorcraft-ui-egui's and vectorcraft-mcp's `bundled-fonts` features).
> **Last updated:** 2026-10-14 · **Change:** leaving out the bundled fonts (`bundled-fonts` feature).
> **Last updated:** 2026-10-14 · **Change:** document-font registration for all explicitly supplied faces.

## Web build

`apps/vectorcraft-web` runs the same `VectorcraftApp` in the browser through eframe's web runner. The renderer is wgpu: WebGPU where the browser has it, WebGL2 otherwise. It is Rust only; the only JavaScript is the glue wasm-bindgen generates.

```sh
brew install trunk                 # or: cargo install trunk --locked
rustup target add wasm32-unknown-unknown
cd apps/vectorcraft-web
trunk build --release              # writes ../../dist/web (index.html, .js glue, .wasm)
trunk serve --release              # dev server on http://127.0.0.1:8766
```

What the web build needs: `trunk`, the `wasm32-unknown-unknown` target and `cargo`, nothing else (no
POSIX shell, so it works the same on Windows). Optionally, `CRAFT_FONTS_DIR` set to an absolute
craft-fonts checkout adds the fonts the site serves beside the wasm: Trunk's `post_build` hook runs
`cargo xtask web-fonts`, which copies them into `dist/web/fonts/` (it does nothing without
`CRAFT_FONTS_DIR`, and skips with a warning any listed face an older checkout lacks). See
[Arabic fonts and the web build](#arabic-fonts-and-the-web-build).

Any static file server works for `dist/web`, for example `python3 -m http.server 8766` inside that directory. The release `.wasm` is about 17.5 MB, or 7.1 MB gzipped, so serve it with compression.

URL flag: `?webgl` forces the WebGL2 backend.

How the web shell (`apps/vectorcraft-web/src/web.rs`) differs from desktop:

- **Open** sets `Services::open_async`, which shows `rfd::AsyncFileDialog`. The bytes arrive in `Services::inbox`, which the app drains every frame.
- **Save / Save As / Export** go through `Services::download`: a Blob, an object URL and a temporary `<a download>`, all created from Rust. There is no save dialog, so the suggested name becomes the download name.
- **Drag-and-drop:** `WebShell` takes the frame's `dropped_files` before the app sees them, reads each with `DroppedFile::bytes_async` and pushes the bytes into the inbox (files to be placed on a canvas go to `Services::place_inbox` with their `place::DropAt`: the document they were dropped on, by uid, and the point; `VectorcraftApp::drop_target` decides from the position the page's drag events give). Desktop drags carry no position (winit 0.30 reports none, and the pointer position egui last had is stale), so the desktop app decides by kind: documents open, pictures and text are placed in the middle of the view. On the web and the desktop, swatch and graphic style libraries, Libraries panel files, presets, plug-ins and shortcut sets open wherever they are dropped, as File › Open opens them (`io::opens_as`). A file that arrives after another document became active still lands in its own document, which becomes active again; if that document closed meanwhile, the file is not placed and the status bar says so. (The app's synchronous drop path is compiled out on wasm32.)
- **Pen and touch input** (`apps/vectorcraft-web/src/pencil.rs`): the shell reads pen and finger Pointer Events itself, in the capture phase on the window, before eframe's touch and pointer handlers (which can't tell a pen from a finger and see each touch twice, as a pointer and as a touch event). Mouse and trackpad events stay eframe's, as do finger events until the first pen contact, preserving eframe's touch-only keyboard path. Pen buttons and event modifiers follow eframe's mappings. A pen drives egui's pointer and is a touch contact whose force is the pen's pressure; its coalesced events (an Apple Pencil reports up to 240 a second) reach the tool one by one with their pressure (`canvas::frame_moves`); holding it still opens the context menu (egui's long touch) and drops the tool's press. Fingers are touch contacts (pinch, two-finger pan, the undo and redo taps); the first one also drives the pointer, so it taps panels and menus. Once a pen has been seen in this page session (reset by reloading; no persistent storage), a finger never presses a tool on the canvas (`touch::finger_navigates_only`), and a pen landing cancels the fingers already down (palm rejection). The page itself doesn't scroll, zoom or select under touch (`touch-action: none`, `overscroll-behavior`), and the canvas stays inside the safe areas.
- **Shortcuts the browser also has** (`apps/vectorcraft-web/src/keys.rs`): many of the app's Cmd and Ctrl chords are the browser's too (`Cmd+=`, `Cmd+-` and `Cmd+0` zoom the page, `Cmd+Z` undoes in the browser, in Safari Undo Close Tab, `Cmd+[` goes back, `Cmd+F` finds), and eframe lets most of them through. A capture listener on the window cancels the browser's action for a press the keymap in effect binds, the user's own shortcuts included (`shortcuts::runs_a_shortcut`, matched as the dispatcher matches it); egui still gets the key and runs the command. It acts only while the canvas or eframe's text input has the keyboard, and never on a plain or Shift press, an AltGr press (Windows reports it as Ctrl and Alt) or an IME's keys, which type; Cut, Copy and Paste stay the browser's, whose clipboard events carry the text, and so does Reload. Browsers keep a few chords from every page, which never reach it (Safari's `Cmd+L`, so New Layer's shortcut does nothing there; Chrome's `Cmd+T`, `Cmd+W`, `Cmd+N`): those still go to the browser. While the app has the keyboard, the page-zoom keys zoom the canvas instead; Preferences › User Interface › UI Scaling and the browser's View menu still scale the interface.
- **Data Recovery** keeps its copies in `localStorage` (`vectorcraft-recovery/<area>/<name>`, shared by the site's tabs), so tabs can't lock their areas the way desktop apps lock a folder (`RecoveryStore::lock`). Each tab holds its area with a heartbeat it refreshes every minute and, where the browser has Web Locks (secure pages: `https://` or localhost), with a Web Lock named after the area (`apps/vectorcraft-web/src/locks.rs`, through `js_sys::Reflect` since web-sys has them only as an unstable API). The browser releases the lock only when the tab is gone, not while its timers are paused in the background, which a heartbeat can't tell apart (#367). `navigator.locks.query()` is asynchronous, so the shell keeps a snapshot refreshed every 10 seconds (taken once before the first frame); a snapshot older than a minute counts every area as held. An area is offered as a crash's leftovers only when no tab holds its lock and its heartbeat is older than three intervals. Without Web Locks a paused tab can still be taken for gone; it repairs that when it resumes: its next heartbeat (or recovery save) writes again the copies missing from its area, and a heartbeat newer than the one the other tab wrote taking the area over makes the area running again for that tab.
- **Losing the graphics (#369):** the browser can drop the page's WebGPU device or WebGL context (a GPU reset, a driver update, too many tabs on the GPU; or a script calling `device.destroy()` / `WEBGL_lose_context.loseContext()`). The app goes on without it, so the shell watches for it: wgpu's device-lost callback for WebGPU, the canvas's `webglcontextlost` event for WebGL2. Both report to a `graphics::GraphicsLoss`, and the next frame of `WebShell::logic` handles it:
  1. `VectorcraftApp::graphics_lost` writes Data Recovery copies of the modified documents at once (to browser storage, also when Data Recovery is turned off).
  2. The shell puts a fresh `<canvas>` in place of the dead one (a canvas whose WebGL context is lost can't get another) and starts the same `WebRunner` again on it, with new graphics and a new egui context, handing over the same `VectorcraftApp`: documents, undo history, selection and panels stay as they were. Its `Services` are made again for the new context, and the status bar says the graphics were restarted.
  3. Textures belong to the egui context they were uploaded to. The app notices the new context (`VectorcraftApp::adopt_context`) and uploads everything again: fonts and theme, the canvas, the place cursor's thumbnails, and every per-thread texture cache (thumbnails, previews, font samples), which are `graphics::TexCache`s for that reason. A new texture cache must be a `TexCache` (or live in the context's `ctx.data`), or it shows stale or wrong pictures after a restart.
  4. If no new graphics can be had (no adapter, WebGL blocked after repeated losses) or the graphics were lost more than three times, the shell gives up: the dead canvas is removed, the app lets go of its recovery area but not its copies (`recovery::leave`: its heartbeat and Web Lock go, so the reloaded page offers the copies at once). The Web Locks are started once, before the first frame, and kept across graphics restarts, so a restarted tab stays the owner of its area and the page says to reload.

  To try it, in the browser console: for WebGL2 (`?webgl`), `document.querySelector('canvas').getContext('webgl2').getExtension('WEBGL_lose_context').loseContext()`. For WebGPU, keep the device the page creates (wrap `GPUAdapter.prototype.requestDevice` before the page loads) and call its `destroy()`.

  The desktop app can't make a new device for its window (eframe can't), so on a lost device (a driver reset) it logs the reason, writes the recovery copies, and says in the status bar to save and restart. Once the copies are written, closing the window skips the Save Changes question, which it can't show: the next launch offers the changes back.
- **No control server:** browsers can't listen on TCP. To automate the web build, drive headless Chrome with `--remote-debugging-port`.
- Quick smoke test: `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --enable-unsafe-webgpu --screenshot=web.png --window-size=1440,900 --virtual-time-budget=15000 http://127.0.0.1:8766/` (headless Chrome on macOS gets a real WebGPU adapter).

## Desktop graphics processor

The canvas is rasterized on the CPU (`vectorcraft-render`, vello_cpu); the GPU (wgpu, through eframe) only composites the canvas texture and draws the UI, so any GPU that can show the window will do. The desktop app picks the window's adapter itself (`apps/vectorcraft/src/gpu.rs`, eframe's `native_adapter_selector`) and logs every adapter it found and the one it uses.

- **Order:** adapters that report they can't present to the window are never tried; then the one `WGPU_ADAPTER_NAME` names; hardware before software (llvmpipe, WARP); the native backends (Vulkan, Metal, DX12) before OpenGL; with Automatic, the GPU the Wayland compositor draws with, then the GPU that drives a display, and among those the one that drives the primary display; then the power preference; then a GPU's DX12 adapter before its Vulkan one (Intel's Vulkan driver made the whole window flicker black, #545), keeping the system's order among equals.
- **Windows backends:** Direct3D 12. OpenGL is listed beside it only when a start leaves an adapter out (a crash or a failed restart), when `WGPU_ADAPTER_NAME` names an adapter, or when Direct3D 12 lists no hardware adapter (Windows 7, or a GPU without a Direct3D 12 driver, which then draws through OpenGL rather than on the CPU with WARP): wgpu's OpenGL backend keeps a hidden window whose thread pumps no messages, and the app hung on a switch to another input language (#4150, gfx-rs/wgpu#10504). Vulkan isn't started at all unless `WGPU_BACKEND` asks for it (`WGPU_BACKEND=vulkan`): creating a Vulkan instance loads every installed Vulkan driver into the process, and a faulty one (Intel's `igvk64.dll`, #806) crashed the app before its window appeared. After a start that crashed while it listed the Direct3D 12 adapters, later starts list OpenGL's alone (see the crash bullet below).
- **The GPU that drives the display.** A GPU without a monitor is the wrong one even when it says it can show the window: on a desktop with a Ryzen's integrated GPU (no monitor) and an NVIDIA card driving both monitors, power saving picked the integrated GPU, Windows had to present every frame across adapters, and the integrated GPU's driver reset under that: the graphics device was lost, or the desktop's compositor went down and every monitor stayed black for minutes (pdfcraft#378). So Automatic asks the system which GPU drives which display and puts that GPU first; the power preference only orders the rest, and decides alone where the system can't say.
  - **Windows:** `EnumDisplayDevices` (the display devices attached to the desktop, and the primary one), through the `winsafe` crate; the PCI ids in each device id are matched against wgpu's adapters. The registry doesn't tell a user which adapter drives a display, and WMI reports a resolution for a GPU without a monitor, so neither is used.
  - **Linux:** the connected connectors in `/sys/class/drm` and their cards' PCI ids; a built-in panel (`eDP`, `LVDS`, `DSI`) counts as the primary display.
  - **macOS**, OpenGL adapters (no device id) and Metal (no ids at all): no match, so the power preference decides as before.
  - The log's first lines list the display GPUs (`display GPUs (PCI vendor:device): 10de:2204 (primary)`), every adapter with whether it drives a display, and the one drawn on and why (`drawing on NVIDIA GeForce RTX 3090 (Dx12, DiscreteGpu): it drives the primary display`). That is the first thing to look for in a black-window report.
- **Preferences › Performance › Graphics Processor** (`gpuPreference`): `automatic` (the default), `lowPower` (Power Saving, the integrated GPU) or `highPerformance` (the discrete GPU). The adapter is chosen when the window opens, so a change applies after a restart. Power Saving and High Performance are the user's own choice and order by kind alone, without looking at the displays; so does `WGPU_POWER_PREF`.
  - **Automatic on Windows and macOS** is the GPU that drives the primary display, then power saving: the system shows frames from any GPU, and presenting frames rendered on a discrete GPU through the integrated one made the window flicker on some hybrid laptops (#306). On a hybrid laptop the integrated GPU drives the panel, so both rules agree.
  - **Automatic under Wayland** puts the GPU the compositor draws with first: the compositor names it (the `main_device` of its linux-dmabuf feedback, protocol version 4; `apps/vectorcraft/src/compositor_gpu.rs`), and the log says `the compositor draws with 10de:25ac`. Which GPU drives a display only stands in for that, and can be wrong: on a hybrid laptop whose Hyprland ran on the NVIDIA GPU (`AQ_DRM_DEVICES`), the panel rule picked the Intel GPU behind the built-in panel, and the compositor showed nothing it drew, with no error, so neither the restart nor the first-frame watch below noticed. A compositor that doesn't say (linux-dmabuf older than version 4), or names a GPU no adapter matches, leaves the rules below in charge.
  - **Automatic on Linux and the BSDs** is the GPU that drives the panel or a monitor, then the system's order: Mesa's Vulkan device-select layer puts the GPU the desktop runs on first (the integrated one on hybrid laptops; `DRI_PRIME` and `MESA_VK_DEVICE_SELECT` steer it). A Wayland compositor may not accept frames from another GPU: on a desktop whose compositor ran on an NVIDIA GPU, rendering on the Ryzen's integrated GPU made KWin end the window's connection ("importing the supplied dmabufs failed") and 0.5.0 crashed at startup (#502).
  - 0.5.0 saved `powerSaving`, its default, for everyone; it reads as `automatic`.
- **If the window fails on its adapter while starting up** (an error from wgpu, or a panic inside wgpu or egui-wgpu during the first frames), the app starts again without that adapter and tries the next one, telling which in the status bar; the run that failed keeps its log as `vectorcraft.1.log`. On Unix the new app replaces the process (an AppImage stays mounted); on Windows it starts beside it. Adapters are told apart by backend and PCI ids, or by name where the backend reports no ids (Metal lists every GPU of a dual-GPU Mac as `0000:0000`, #651). The adapters left out travel in `VECTORCRAFT_GPU_SKIP` (`backend:vendor:device`, such as `Vulkan:1002:164e`), one more on each restart, so the restarts end. When no adapter is left, the app logs why and exits with an error instead of panicking.
- **If the window's first frame never reaches the screen** within 20 seconds of the app being created (the files named at the start opened), the adapter counts as hanging and the app starts again without it in the same way (#964: High Performance on a hybrid laptop left the app running in the background with no window and nothing in the log). A window minimized at the start isn't counted. When the processor that hung was chosen in Preferences, the status bar points to Automatic.
- **If the app crashes while the window starts**, as when Intel's Direct3D 12 driver (`igd12umd64.dll`) ended 0.9.0 before its window appeared on an Intel UHD Graphics 605 (#2243), no code in the process runs to start it again, so the next start leaves the adapter out. Each start writes `graphics-start.json` in the log folder before the graphics start and adds the adapter once one is picked; when the first frame reaches the screen or the app ends without crashing, the start is cleared from the file. A start that finds an earlier start still recorded there logs the adapter, leaves it out and shows a note in the status bar. When the crash came while the adapters were being listed (wgpu's Direct3D 12 backend creates a device on each GPU to list it), the next start lists OpenGL's adapters alone; on macOS, where wgpu has no OpenGL adapters, it lists them all. Later starts keep leaving them out until the app version or Preferences › Performance › Graphics Processor changes, a start shows its first frame on the adapter that crashed, or no adapter is left that can show the window. `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` choose regardless of the file: a start with `WGPU_BACKEND=dx12` tries Direct3D 12 again, and when its first frame appears, the crash is cleared. A process ended from outside during the start (Task Manager, `kill`) counts as a crash. Runs without preferences (`VECTORCRAFT_NO_PREFS`) record nothing, and a start made while another copy of the app is starting neither reads nor writes the file.
- **To recover from a Graphics Processor choice that doesn't work** without editing files: start once with `WGPU_POWER_PREF=low` (or `WGPU_ADAPTER_NAME=…`), which overrides the preference for that start while settings still save, and set Graphics Processor back to Automatic. `VECTORCRAFT_NO_PREFS=1` also starts without it, but saves nothing.
- **Environment variables** for when the choice still goes wrong, all read at startup and stronger than the preference:
  - `WGPU_POWER_PREF`: `low`, `high` or `none` (the system's order).
  - `WGPU_ADAPTER_NAME`: part of an adapter's name, any case (`WGPU_ADAPTER_NAME=nvidia`, `=radv`, `=intel`); the names are in the log.
  - `WGPU_BACKEND`: the backends to use, such as `vulkan`, `dx12`, `metal` or `gl`.
  - `WGPU_VALIDATION_INDIRECT_CALL=1`: turns wgpu's check of indirect draw arguments back on. VectorCraft draws nothing indirectly and turns it off, since the compute shader it needs fails to compile on some drivers (#651).
  - On Linux with Mesa, `MESA_VK_DEVICE_SELECT=10de:2f04!` (vendor:device as `vulkaninfo --summary` or `lspci -nn` show it) leaves only that GPU to Vulkan, and `DRI_PRIME=1` picks the other GPU.
- Help › About and the control channel's `ui.inspect` (`graphicsAdapter`) show the adapter in use, and the app logs it at startup.

## Logs

The desktop app writes its `log` records to standard error and to `logs/vectorcraft.log` next to the preferences (Linux `$XDG_CONFIG_HOME/vectorcraft/logs/`, by default `~/.config/vectorcraft/logs/`; macOS `~/Library/Application Support/VectorCraft/logs/`; Windows `%APPDATA%\VectorCraft\logs\`). A start launched from a desktop menu or the Dock has no terminal, so this file is what to attach to a bug report: the graphics adapter in use, a lost graphics device, panics the engine's guard recovered from and clipboard formats that couldn't be made land there. Each launch moves the previous log to `vectorcraft.1.log` (and that one to `vectorcraft.2.log`), so the log of a run that crashed survives the next start. The file stops growing at 16 MiB. `--version` writes no file, and runs with `VECTORCRAFT_NO_PREFS` log to standard error only.

By default VectorCraft's own crates log at `info` and everything else at `warn`. `RUST_LOG` replaces that with env_logger-style directives, for example `RUST_LOG=debug`, `RUST_LOG=warn,vectorcraft_render=trace` or `RUST_LOG=info,wgpu_core=warn`; a directive ending in `*` covers every target starting with it (`vectorcraft*=debug`). The logger is `apps/vectorcraft/src/logging.rs`. `vectorcraft-cli mcp` has its own logger, which sends records to the MCP client (`docs/mcp.md`).

## Environment variables (desktop app)

| Variable | Effect |
|---|---|
| `VECTORCRAFT_CONTROL_PORT` | Same as `--control <port>`, which overrides it; a value that isn't a port from 1 to 65535 stops the app |
| `VECTORCRAFT_IN_WINDOW_MENUS` | Any value but empty or `0`: same as `--in-window-menus` (see [macOS: the menu bar](#macos-the-menu-bar)) |
| `VECTORCRAFT_NO_PREFS` | No preferences read or written, no default Data Recovery folder and no log file, and no saved PDF presets or loaded ICC profiles kept (agents' test runs; also for `vectorcraft-cli`) |
| `VECTORCRAFT_CONFIG_DIR` | The folder for the preferences and the folders next to them (`PDF Presets`, `Color Profiles`, …), for the app and `vectorcraft-cli` (see `docs/control-protocol.md`) |
| `WGPU_POWER_PREF` | Graphics adapter: `low`, `high` or `none` (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `WGPU_ADAPTER_NAME` | Graphics adapter by (part of) its name, any case (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `WGPU_BACKEND` | Graphics backends: `vulkan`, `dx12`, `metal`, `gl` |
| `VECTORCRAFT_GPU_SKIP` | Set by the app when it starts again without a graphics adapter that failed (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `RUST_LOG` | Log levels for standard error and the log file (see [Logs](#logs)) |

## macOS: the menu bar

On macOS the menus live in the system menu bar, not in the window's app bar (which keeps the brand mark, Home, the search icon and the workspace switcher next to the traffic lights, centred on the bar by `apps/vectorcraft/src/mac_window.rs`). `--in-window-menus` or `VECTORCRAFT_IN_WINDOW_MENUS=1` keeps them in the window, as on Windows, Linux and the web; so does a menu bar AppKit refuses to build (logged). `ui.inspect` says which with `nativeMenuBar`.

- **One menu model.** `crates/ui-egui/src/native_menu.rs` (platform-free, tested on every platform) builds the menu bar from the in-window menus (`menus::menu_tree`, each item through `menus::entry`, which the in-window menus draw too) and lays it out the Mac way: the VectorCraft menu holds About, Settings ▸ (one item per Preferences page, General… ⌘K), Language ▸, Appearance ▸ (UI brightness), Services, Hide VectorCraft ⌃⌘H (⌘H stays View › Hide Edges), Hide Others ⌥⌘H, Show All and Quit ⌘Q (the app's own `app.quit`, so unsaved documents are asked about); those leave Edit and Help. The in-window bar itself never shows the VectorCraft menu (it starts at File, as in Illustrator, on every platform): Language and Appearance have no in-window menu and are synthesized in `mac_layout`, and Quit is File › Exit in the window, relabelled to Quit in the App menu. Window starts with Minimize ⌃⌘M and Zoom and ends with Bring All to Front, and Help is the system's Help menu (its search field). A system key a command (or a user's shortcut) already has stays the command's, and the log says so.
- **The native side** is `apps/vectorcraft/src/mac_menu.rs`: muda (`=0.21.1`, default features off, macOS only, no `unsafe`) builds the `NSMenu`s. Labels, enabled and checked are updated in place when anything they show may have changed (a command ran, the document, its history or selection changed, a click or a key press); a change of structure (a recent file, the language, a shortcut) rebuilds the menus. While an interaction is open (for example a move, scale or shape drag, a dialog's preview or the font menu's preview), the app updates them for the document's changes only when the interaction begins and when it ends.
- **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key, so the item's key goes back to egui as the key press it was (⌘X/⌘C/⌘V as egui's Cut/Copy/Paste events), through `raw_input_hook`: `shortcuts::handle` runs it once, user shortcuts apply, and a focused text field keeps ⌘A, ⌘C, ⌘V and ⌘Z. A click runs the item like an in-window click. Bare keys and ⌥/⇧ chords are never key equivalents (they type).
- Built and clippy-checked for `aarch64-apple-darwin` and `x86_64-apple-darwin` from any host: `cargo clippy -p vectorcraft --target aarch64-apple-darwin --all-targets -- -D warnings`.

## Linux: Wayland and X11

On GNOME/Wayland, enable **Preferences › User Interface › System Title Bar** and restart to use winit's font-free Adwaita window controls (`wayland-csd-adwaita-notitle`); the existing custom title bar remains the Linux default.

The window runs natively on Wayland (eframe's `wayland` feature) and on X11. On machines with two GPUs it renders on the one the desktop runs on unless Preferences say otherwise; see [Desktop graphics processor](#desktop-graphics-processor) (#502). The system clipboard (`apps/vectorcraft/src/clipboard.rs`) is arboard with its `wayland-data-control` feature:

- **Clipboard:** under Wayland arboard talks to the compositor through the data-control protocol (wlroots compositors such as Sway and Hyprland, KDE Plasma), so bitmaps (`image/png`), SVG markup and text copied in any app paste (#398). Where the compositor lacks the protocol (GNOME's Mutter, [arboard#223](https://github.com/1Password/arboard/issues/223)), arboard falls back to the X11 clipboard through XWayland, which only holds what X11 apps copied. Where there is no X11 either (the Flatpak on a Wayland session: `--socket=fallback-x11` gives it none), Copy and Paste use the compositor's core data-device protocol on the window's own connection through smithay-clipboard: text only (a copy's text or its SVG markup), and only while the window has focus. smithay-clipboard waits for the owner's data with no limit, so its reads run on a worker thread and a Paste waits 2 s at most: an owner that never sends its data costs one Paste 2 s, and later reads return at once until it answers. A clipboard neither backend opens is tried again after 1 s, doubling to a minute (a Copy or Paste of the user's at most once a second), and logged once when it goes and once when it comes back: before, the background Paste check reconnected and logged a warning every 250 ms. egui's own text paste into fields goes through smithay-clipboard and works either way.
- **Clipboard owners that never answer (#510):** reading the clipboard waits on the app that owns it: arboard gives up on an X11 owner after 4 s, and a Wayland read waits for the owner to send its data. So on Linux the check that enables the Paste menu items runs on a background thread with a second clipboard handle (`crates/ui-egui/src/clipboard_probe.rs`, installed as `Services::clipboard_probe`), only while nothing is copied in the app and a document is open, and the window keeps drawing. A paste itself still reads on the UI thread, so pasting from such an owner waits for it once. Windows and macOS keep the check on the UI thread: Windows only asks which formats the clipboard holds, macOS asks the pasteboard server.
- **Copied files:** files copied in a file manager (`text/uri-list`; `CF_HDROP` on Windows, file URLs on macOS) paste as the first one that is art (SVG, PDF, EPS, EMF/WMF or a bitmap; a `.ai` or `.ait` file from the editing data it holds, else as the PDF or PostScript it holds), before the clipboard's other formats (which include the files' paths as text).
- **Dropping files on the window does not work under Wayland:** winit 0.30, which eframe 0.37 runs on, has no Wayland drag and drop ([winit#1881](https://github.com/rust-windowing/winit/issues/1881), added in winit 0.31; [egui#1563](https://github.com/emilk/egui/issues/1563)), so no `dropped_files` arrive. Until eframe moves to winit 0.31: copy the files in the file manager and paste them, use File › Place or Open, or open the window through XWayland, where drops work: set **Preferences › Performance › Linux Display Server** to **X11 (XWayland)** and restart (#1151), or start the app with `WAYLAND_DISPLAY= vectorcraft`. The preference (`linuxDisplayServer`, read at startup in `apps/vectorcraft/src/main.rs`, shown on Linux only) has winit open the window on X11 (`EventLoopBuilderExtX11::with_x11`) when the session is Wayland, `DISPLAY` is set and the X11 libraries winit loads are installed (`apps/vectorcraft/src/xwayland.rs` reads `ldconfig -p` and the usual library directories); otherwise the window opens on Wayland and the log says why. Unlike clearing `WAYLAND_DISPLAY`, it leaves the clipboard on the compositor's own.
- **Pens and pen displays under Wayland (#491, #2028):** winit 0.30 doesn't bind the Wayland tablet protocol (`zwp_tablet_v2`; tablet input arrives in winit 0.31, [winit#4318](https://github.com/rust-windowing/winit/pull/4318), [egui#7731](https://github.com/emilk/egui/pull/7731)). Since Plasma 6.3, KWin no longer turns tablet input into pointer input for such apps ([kwin!6336](https://invent.kde.org/plasma/kwin/-/merge_requests/6336), commit [25c65abd](https://invent.kde.org/plasma/kwin/-/commit/25c65abddd95eb87fb892a8594faccbf509ff62f)), and GNOME's Mutter does not turn it into pointer input either (that commit's message): on KWin the pen moves the compositor's cursor over the window, but the app gets no motion or presses and shows none of its own cursors. Until eframe moves to winit 0.31, either run the app under XWayland, which supports the tablet protocol and hands the pen to X11 apps as a pointer (Preferences › Performance › Linux Display Server › X11 (XWayland); X11 (XWayland) When a Pen Is Attached (`x11WithPen`), which opens the window through XWayland only when `/proc/bus/input/devices` lists a pen as the app starts, a device with the `BTN_TOOL_PEN` key and the `ABS_X` and `ABS_Y` axes, and logs `pen found: opening the window through XWayland`; `WAYLAND_DISPLAY= vectorcraft`, or `env -u WAYLAND_DISPLAY vectorcraft`; for a desktop launcher, put `env WAYLAND_DISPLAY=` before the command in its `Exec=` line), or turn KWin's deprecated emulation back on for the whole session by setting `KWIN_WAYLAND_EMULATE_TABLET=1` in KWin's environment (for example in a file under `~/.config/environment.d/` or in `/etc/environment`) and logging in again. The pen then works as a mouse. Under XWayland the app also reads the pen's pressure (next item); with KWin's emulation on native Wayland the pen has no pressure. Other compositors that don't emulate a pointer for tablets behave the same way.
- **Pen pressure under X11 and XWayland (#764):** winit 0.30 hands a pen to an X11 window as a mouse, without its pressure. When the window is on X11, XWayland included, the desktop app opens a second X connection (x11rb's `RustConnection`) and reads XInput2 raw events for every device (`apps/vectorcraft/src/tablet.rs`, from PhotoCraft's `crates/tablet`). A device with an `Abs Pressure` valuator is a pen, unless its XInput type is `TOUCHPAD` or `TOUCHSCREEN`. While a pen is the pointer that moved last, its pressure, scaled from the valuator's range to 0..1, reaches the tools that use pressure: the Paintbrush's pressure-sensitive brushes and the Liquify tools' Use Pressure Pen. A pen device whose name or XInput type contains `eraser` (in any case) is the pen's eraser end: while the pointer is over the window, the canvas selects the Eraser tool when the eraser end moves, and it selects the tool used before when the tip moves again (`crates/ui-egui/src/stylus.rs`). The X server needs XInput 2.1 or later; otherwise the log has a `tablet pressure:` warning with the reason, and the pen draws at full pressure.

## Fonts: craft-fonts (optional build input)

Font files are never committed to this repository. Fonts shared by the Crafting Apps live in
[storytold/craft-fonts](https://github.com/storytold/craft-fonts); the rules are in craftrules
[`standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md). To add
a font, add it there. (The Latin fonts in `assets/fonts/` predate the rule and stay.)

VectorCraft builds, tests and runs without craft-fonts. To embed its Japanese fonts (BIZ UDPGothic
for UI text, Shippori Mincho and BIZ UDMincho for document text), point the `CRAFT_FONTS_DIR` build
option at a checkout:

```sh
git clone https://github.com/storytold/craft-fonts ../craft-fonts
CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo run --release -p vectorcraft
CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo xtask ci   # also runs the Japanese-glyph tests
```

- `crates/text/build.rs` reads the checkout's `fonts/manifest.txt` and embeds every font it lists as
  `vectorcraft_text::CRAFT_FONTS` (empty without `CRAFT_FONTS_DIR`). Give it an absolute path:
  build scripts run in the crate's directory (a relative path is taken from the workspace root
  as a convenience, but CI and the docs always use absolute paths). Nothing is downloaded, and
  craft-fonts is never a `Cargo.toml` dependency. A bad path is a build warning, or an error with
  `CRAFT_FONTS_REQUIRED=1` (release builds).
- Web (wasm32) builds embed only BIZ UDPGothic Regular (+4.7 MB of `.wasm`), to stay well under
  static hosts' per-file limits (Cloudflare Pages: 25 MiB). Shippori Mincho, which this repo used to
  embed everywhere, was 8.7 MB, so the web build is still smaller than before.
- Document text: the Japanese faces join the font database after the bundled fonts (Mincho first),
  so they are the fallback for Japanese after the requested font; installed system fonts come after.
  Noto Sans Thai (when the checkout has it) is the fallback for Thai where no Thai font is
  installed, and the web build fetches it before the app starts (`crates/text/web-fonts.txt`).
  Noto Sans Georgian (when the checkout has it) is the same for Georgian.
  All other supplied faces follow those existing document fallbacks in manifest order, so they
  can also be selected by family name. This does not change which faces the web build embeds
  or fetches, and no craft-fonts faces are registered when the optional input is absent.
- UI: `theme::install_fonts` adds them at the end of every egui family (BIZ UDPGothic first, then
  Noto Sans Georgian for the Georgian interface), after the UI fonts and before the installed fonts
  `ui_fonts.rs` discovers for other scripts. On the web, Noto Sans Georgian is fetched at startup
  and reaches the UI through `ui_fonts.rs`.
- Tests that need Japanese or Georgian glyphs skip with a message when built without craft-fonts. The FreeBSD
  CI job and every release job (`release.yml`) check craft-fonts out at a pinned commit; release
  packages carry each embedded font's `OFL-<family>.txt`.

### Leaving out the bundled fonts (embedders)

The Latin fonts in `assets/fonts/` are compiled into `vectorcraft-text` by its `bundled-fonts`
feature (3.4 MB of the binary). It is on by default, and `vectorcraft-engine`'s default
`bundled-fonts` feature turns it on, so the apps and anything built on the engine with its default
features have them. An
embedder that brings every font itself at run time (`FontDb::add_font`, say a web app serving its
own font catalogue) depends on the engine with `default-features = false` and doesn't ship them
twice. Its font database starts without them: on the web and without craft-fonts it starts empty,
and text has no glyphs until a font is added.

An embedder that also hosts the egui panels depends on `vectorcraft-ui-egui` with
`default-features = false` as well: its own `bundled-fonts` feature, on by default, turns the
engine's on; `vectorcraft-mcp` has the same feature. The panels' interface faces (Source Sans 3
Regular and Semibold, Inter Regular and SemiBold, JetBrains Mono, about 2 MB: `theme.rs`) are
`vectorcraft-ui-egui`'s `interface-fonts` feature, also on by default; without it the panels draw in
egui's built-in fonts, which lack some symbols they show (→ ▾ ● ✓), so such an embedder adds a
font that has them to each of the families below (`ctx.add_font` with an `InsertFontFamily` per
family).

The panels and dialogs draw in their own egui families (`theme::FONT_UI`, `FONT_UI_SEMIBOLD`,
`FONT_MONO`), and egui panics drawing text in a family no font is bound to. `VectorcraftApp::logic`
binds them on its first frame; an embedder that draws the panels or dialogs without it calls
`theme::install_fonts` once on its context, with or without `interface-fonts`. It replaces the
context's font definitions, so the embedder's own fonts are added after it.

- The workspace declares `vectorcraft-text` and `vectorcraft-engine` without default features, so
  each crate that wants the fonts asks for them (the apps, and `vectorcraft-testkit` for tests);
  a crate whose tests lay type out asks for `bundled-fonts` in its dev-dependencies (or gets it
  through `vectorcraft-testkit`).
- `cargo xtask ci`'s `no-bundled-fonts` step also checks that `vectorcraft-ui-egui` and
  `vectorcraft-mcp` without their default features turn them on nowhere, for any target
  (`cargo tree -p vectorcraft-ui-egui --no-default-features --target all -e features,normal -i
  vectorcraft-text`, and the same for `vectorcraft-mcp`), and with them do; a test of either can't
  see that configuration, since their dev-dependencies turn the fonts on.
- `cargo test -p vectorcraft-text --no-default-features --test without_bundled_fonts` checks the
  configuration without them, and `cargo test -p vectorcraft-ui-egui --no-default-features --test
  install_fonts` that the panels without their interface faces still bind every family; the same
  step lints `vectorcraft-ui-egui` without its default features. Of the crate's other tests, those
  about the interface faces need them.

### Arabic fonts and the web build

Desktop builds load craft-fonts' Arabic families into the document font database (Noto Sans
Arabic first, the fallback for Arabic text; the others are picked by name).

The web build doesn't embed them (wasm size). Instead `crates/text/web-fonts.txt` lists the faces
it fetches from beside the wasm, `startup` (before the app starts) or `background` (after):

- `crates/text/build.rs` turns the list into `vectorcraft_text::WEB_FONTS` (URL
  `fonts/<sha16>/<file>` and the SRI hash, from the craft-fonts manifest).
- `cargo xtask web-fonts` (`xtask/src/web_fonts.rs`), the Trunk `post_build` hook, copies the files
  there, checking each one's SHA-256. A listed face the checkout lacks is skipped with a warning, as
  in `build.rs`, so an older craft-fonts just means fewer fonts.
- `apps/vectorcraft-web/src/fonts.rs` fetches and registers them, logging `web font …` for any
  that fail.

Try it: `cd apps/vectorcraft-web && CRAFT_FONTS_DIR=$PWD/../../../craft-fonts trunk serve`. To serve
another craft-fonts face on the web, add a line to `web-fonts.txt`.

## Localisation

Strings in code stay English and are the default lookup keys. `crates/ui-egui/src/i18n` maps them to display
text at render time from one catalog per language (`i18n/<code>.tsv`; the format is documented in the header of
`zh-hant.tsv`). Command ids, menu paths used for logic, `ui.menu.list`, the control channel, the CLI and MCP
always use the English ids and labels, so agents and scripts never see translated text.

Languages shipped: English (`en`, the source), Traditional Chinese (`zh-hant`, complete, in the vocabulary used
in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` locales all resolve to it), Simplified Chinese (`zh-hans`,
complete, in the vocabulary used in mainland China; `zh-CN`, `zh-SG`, `zh-Hans-*` and a bare `zh` resolve to it,
so the two scripts never mix), Japanese (`ja`, complete), Korean (`ko`, complete; every `ko-*` locale resolves to it; a count takes no plural, and Hangul is drawn with an installed system font), Vietnamese (`vi`, complete; every `vi-*` locale resolves to it; a count takes no plural, so plurals have one form), Finnish (`fi`, complete; every `fi-*` locale resolves to it; one/other plurals with the partitive after numbers other than 1), German (`de`, complete; `de-DE`, `de-AT`, `de-CH` and every
other `de-*` locale resolve to it), Spanish (`es`, complete, in neutral
international Spanish; every `es-*` locale such as `es-ES`, `es-MX`, `es-AR` or `es-419` resolves to it), French (`fr`,
complete; `fr-FR`, `fr-BE`, `fr-CA`, `fr-CH` and every other `fr-*` locale resolve to it), Italian (`it`,
complete; `it-IT`, `it-CH` and every other `it-*` locale resolve to it), Russian (`ru`, complete; every `ru-*`
locale such as `ru-RU`, `ru-BY` or `ru-KZ` resolves to it), Ukrainian (`uk`, complete; `uk-UA`,
`uk_UA.UTF-8` and every other `uk-*` locale resolves to it; integer plurals use one/few/many), Hungarian (`hu`, complete;
`hu-HU` and every other `hu-*` locale resolves to it; a count takes the singular, so plurals have one form), Polish (`pl`, complete; `pl-PL` and every other `pl-*`
locale resolve to it; plurals use one/few/many, with 21, 31 … taking many), Swedish (`sv`, complete; `sv-SE`, `sv-FI` and every
other `sv-*` locale resolve to it), Georgian (`ka`, complete; `ka-GE` and `ka_GE.UTF-8` resolve to it; one count-neutral plural form), Catalan (`ca`, complete; `ca-ES`, `ca-AD`, `ca-FR`, `ca-IT` and every other `ca-*` locale resolve to it; one/other plurals), Czech (`cs`, every menu label) and Brazilian Portuguese
(`pt-br`, complete; `pt-BR`, `pt_BR.UTF-8` and other tags that resolve to `pt-br` through `candidates`). Untranslated text falls back to English until its rows are added.

- `tl!("…")` translates a literal into the language the UI is drawn in; `i18n::t(s)` is the same for a
  `&str`. `tr(lang, s)` takes the language; `tr_ctx` when one English word needs different translations;
  `tr_id(lang, command_id, label)` for menu items (keyed by command id, English label as the fallback);
  `tn` / `trn(lang, n, one, other)` for plurals; `fmt` fills `{name}` placeholders, which a translation may
  reorder.
- The shared widgets (`widgets::check`, `dropdown`, `menu_item`, the buttons, `label_row`, tooltips of
  `icon_button`…), the menus, the dock, the toolbar, the dialog frame and `panels::empty_state` translate
  the text they are given, so a panel mostly needs its literals wrapped in `tl!` to be covered by the tests.
- The language is Edit › Preferences › User Interface › Language or, on macOS, VectorCraft › Language (the
  `app.language` UI command, `{lang: auto|<code>}`); both set the `interfaceLanguage` preference (`auto` or a language
  code; `auto` follows the system locale: `VECTORCRAFT_LOCALE`, then `LC_ALL`/`LC_MESSAGES`/`LANG`/`LANGUAGE`,
  the macOS preferred languages, the Windows user locale; on the web, `?lang=<code>` in the address, then
  the browser's `navigator.languages`). The Preferences dialog previews the chosen language before OK.
- To add a language: add `<code>.tsv` and one row in `i18n::LANGUAGES` (code, native name, catalog, plural
  rule). The Language menu, the dropdown, locale matching and the catalog tests (well-formed, no duplicates,
  placeholders and ellipses agree, command ids exist, every row of a partial catalog is a string the UI
  shows, the UI fonts have every glyph, no Simplified characters in `zh-hant`) pick it up. Set `complete_menus`
  once the menus and `tl!` literals are translated; the untranslated report then lists what the catalog lacks
  (`VECTORCRAFT_I18N_DUMP=strings.txt cargo test -p vectorcraft-ui-egui dump_source_strings` lists every
  English string).
- A missing translation never fails a test or a build: the interface shows the English text until a translator
  adds the row. `cargo xtask i18n-report` prints, per language, the strings its catalog lacks (menu strings,
  `tl!` literals, undo step names, menu labels, render-site labels, status and error messages, PostScript
  error meanings, default swatch names), and `cargo xtask ci` prints the same list in its `i18n-report` step.
  The report is the test `i18n::tests::untranslated_report`, run with `--nocapture`; it fails only when a
  scan finds too few strings to be working. Tests don't check one language's exact wording: a test that needs a
  translation compares with the catalog lookup (`tr(lang, "English")`).
- Translations are clean-room: written from the meaning of the English text in ordinary vocabulary, never
  from another product's localisation resources. Product and technology names stay in Latin letters.
- Status-bar messages and errors stay English where they are made (`app.ui.status`, `EngineError`: the
  control channel, MCP and tests read them) and are translated only where the status bar draws them
  (`i18n::msg`). `@msg` catalog rows hold a whole message or a template such as
  `Couldn't open {name}: {e}`; `{_1}`, `{_2}` … stand for the format string's `{}`, and the values in the
  placeholders are translated in turn (the reason after `: {e}` is often a message too). Czech, German, Greek, Spanish, French, Georgian, Hungarian, Italian, Japanese, Korean, Polish, Brazilian Portuguese, Russian, Simplified Chinese, Traditional Chinese, Swedish, Ukrainian, Vietnamese, Finnish, Turkish and Catalan
  translate status and error messages (`COMPLETE_MESSAGES` in `i18n::tests`); the untranslated report lists the
  message literals the scan finds that one of them has no row for. The scan reads `Err("…")`, `Other(…)`,
  `#[error(…)]` and `status(…)` messages, messages given to an error enum's variant (`PsError::Limit(…)`, the reason
  in `EngineError::Disabled(id, …)`), to the engine's `bad(cmd, …)`, `err(…)` or `error(…)`, and to the PDF writer's
  `bad(…)` or `failed(…)`, and skips calls inside comments
  (`VECTORCRAFT_I18N_DUMP_MESSAGES=messages.txt cargo test -p vectorcraft-ui-egui dump_source_strings` lists them
  all). A missing row shows the English message. Other languages show messages in English until they add `@msg`
  rows.
- Not translated on purpose: names that are user data (layers, swatches, fonts, documents), the tab
  title's colour mode, command ids and parameter names inside messages. Not done yet: locale-aware number
  and date formats, right-to-left layout. Chinese and Japanese UI text
  is drawn with craft-fonts' BIZ UDPGothic when the build embeds it (see Fonts above), else with an installed
  system font; the glyph test checks Latin and Cyrillic catalogs always and the CJK ones only with craft-fonts.
  A Traditional Chinese UI font is still to be added to craft-fonts for the web build.

## Cursors

`crates/tools/src/cursors.toml` lists every `Cursor` value: when it shows, in plain behaviour terms, and where its hotspot is, so whoever draws the cursors knows what each one is for without reading the tools (CLAUDE.md › Adding a cursor). `cargo xtask cursors` (part of `cargo xtask ci`) warns about a `Cursor` value with no entry, an entry with no `Cursor` value, a repeated id or a missing field; it never fails the build. A cursor not drawn yet borrows another's look (`Cursor::stand_in`).

## Vendor names gate

`cargo xtask brands` (part of `cargo xtask ci`) fails when user-visible text names another vendor's products or company: string literals in Rust sources (command labels and params docs, menus, panels, MCP tool definitions), `Cargo.toml` descriptions and packaging files. Comments, test files and `#[cfg(test)]` modules are not checked. Say "the reference app" or name the feature itself. A line that must keep an old name, such as an alias that files or preferences from earlier versions still use, or a name another project defines, such as a shared MIME type, carries a `brand-ok` comment. In the Linux desktop entry, whose comments take whole lines, the comment goes on the `#` line just above it; `MARK_ABOVE` in `xtask/src/brands.rs` lists the files that accept a mark there.

## Tests

- `cargo xtask test` runs the workspace's unit and integration tests with
  [cargo-nextest](https://nexte.st) when it is installed (`cargo install cargo-nextest --locked`),
  otherwise with `cargo test`, and then the doctests (`cargo test --workspace --doc`), which nextest
  doesn't run. `cargo xtask ci` runs the same two as its `test` and `doctests` steps.
- The test build leaves out the examples (`--lib --bins --tests`); clippy (`--all-targets`) still
  checks them.
- Each crate has one integration-test binary: its test files are modules of `tests/it/main.rs`. Run
  one file's tests with `cargo nextest run -p vectorcraft-svg --test it roundtrip::` (or `cargo test`
  with the same arguments). A new test file gets a `mod` line in `main.rs`.
- A test file stays a binary of its own (`tests/<name>.rs`) when it changes process-wide state that
  other tests would see, because `cargo test` runs a binary's tests as threads of one process (nextest
  runs each test in its own process). In `crates/engine/tests`: the Color Settings
  (`color_settings.rs`), the font database and Fonts folder (`removed_font.rs`, `font_folder.rs`,
  `text_import_fuzz.rs`), installed plug-ins (`plugin_export.rs`, `plugin_fuzz.rs`), the working
  directory (`open_relative_path.rs`) and every command run with junk parameters
  (`command_sweep.rs`); the `system_fonts` tests of `vectorcraft-text` and `vectorcraft-pdf` too. The
  fuzz files under Robustness › Fuzzing keep their own binaries, so a deeper search runs alone, and so
  do `affinity_artboards`, `affinity_text` and `affinity_corpus`, which the Affinity corpus workflow
  names.
- The dev profile builds the workspace crates with line tables only (`debug = "line-tables-only"`),
  so panics and backtraces still name file and line, and dependencies without debug info. For a
  debugger session, `CARGO_PROFILE_DEV_DEBUG=true cargo build` gives full debug info.

## Robustness: VectorCraft never crashes

A crash takes the user's unsaved work with it, and much of what the app reads is untrusted: SVG,
PDF, `.ai` and `.vectorcraft` files, pasted data, MCP and control-channel messages, command
parameters. Shipped code therefore never panics. Anything that can fail returns `Result` (or
`Option`), the caller handles or propagates it, and the user sees an error message.

### Enforced by lints

`Cargo.toml` denies these clippy lints for the whole workspace, and `cargo xtask ci` runs clippy with
`-D warnings`. The gate targets the current stable Rust (`rust-version` in `Cargo.toml` is only the
minimum that builds the app, and no toolchain is pinned): when a new stable adds lints, they are fixed
in the code:

| Banned in shipped code | Use instead |
|---|---|
| `x.unwrap()`, `x.expect("…")` | `x?`, `x.ok_or(err)?`, `let Some(v) = x else { return … }`, `if let`, `unwrap_or` / `unwrap_or_default` / `unwrap_or_else` |
| `v.last().unwrap()`, `v.pop().unwrap()` | `if let Some(l) = v.last_mut()`, `let Some((last, rest)) = v.split_last() else { … }` |
| `panic!`, `unreachable!()` in a match arm | return an error (`EngineError::Other`, `bad(C, "…")`) or a sensible default |
| `todo!`, `unimplemented!` | don't merge unfinished paths; return an error that says what isn't supported |

`clippy.toml` allows `unwrap`, `expect` and `panic` inside `#[test]` and `#[cfg(test)]` code.
Integration tests (`tests/it/main.rs` and the test files that are binaries of their own), examples
and the `testkit` crate opt out with
`#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. Tests use `panic!("…")`
instead of `unreachable!()`.

### Not caught by lints: avoid implicit panics too

- **Indexing and slicing:** `v[i]` and `&s[a..b]` panic when out of range. When the index comes from
  data (a file, a parameter, another document), use `v.get(i)` / `s.get(a..b)`. Slicing a `&str` also
  panics off a UTF-8 boundary.
- **Arithmetic:** integer division or `%` by zero, `usize` subtraction that can go below zero (write
  `i + 1 == n` instead of `i == n - 1`, or use `saturating_sub`), and `as` casts from non-finite floats.
- **Unbounded work:** cap counts, sizes and allocations read from input. Image dimensions, repeat
  counts, line breaks and recursion depth all need a limit.
- **Encoders and decoders:** propagate their errors. Writing an empty file is a silent failure, not a fix.

Don't silence the lint by discarding errors. `let _ = …` and `.ok()` are for failures that really
don't matter, with a comment saying why.

### The safety net

`vectorcraft_engine::guard::catch_panic` wraps every entry point:

- `Session::execute` restores the active document, selection and interaction as they were before
  the top-level command and returns `EngineError::Internal`.
- Tool events (`Session::pointer`, `tool_key`, `tool_text`) reset the tool and cancel its drag.
- The MCP server answers the request with a JSON-RPC internal error and keeps serving.
- `VectorcraftApp::logic` and `ui` lose one frame and show the error in the status bar; each
  control-channel request is guarded on its own.

The net exists for bugs, not as a substitute for `Result`. On wasm a panic aborts the app, so the
web build has no net at all.

### Fuzzing

Untrusted input has property tests that must never panic:

- `crates/engine/tests/import_fuzz.rs`: garbage, hostile and mutated SVG and PDF, and swatch
  (`.vcswatches`, `.gpl`, `.ase`), graphic style (`.vcstyles`) and flattener preset (`.vcflattener`)
  libraries, loaded and used, then rendered and exported.
- `crates/engine/tests/command_sweep.rs`: every command with junk parameters.
- `crates/format/tests/prop_format.rs`: garbage and mutated `.vectorcraft` files.
- `crates/mcp/tests/protocol_props.rs`: malformed MCP messages.

CI runs a few dozen cases each. Before touching an importer, run a deeper search, e.g.
`PROPTEST_CASES=20000 cargo test --release -p vectorcraft-engine --test import_fuzz`. When it finds a
panic, fix the code and add the input as a regular test.

## Bidirectional type

Point, area and path type lay out Hebrew and Arabic with the Unicode bidirectional algorithm
(`unicode-bidi`, in `crates/text/src/layout.rs`). Each paragraph's base direction is its
`ParaStyle::direction` (Paragraph panel › Left-to-Right / Right-to-Left Paragraph Direction, shown
with Preferences › Type › Show Indic Options; `text.setFormat {direction}`), else its first strong
character. Text is shaped in logical order, right-to-left runs right to left (`shape.rs` splits
segments by bidi level and script), lines are broken in logical order and then each line's whole
shaping clusters are put in visual order. Stored text, undo and style ranges stay logical; the
caret, selection, hit testing and arrow keys follow the visual order. A paragraph with nothing
right to left in it skips the algorithm, so plain text pays nothing for it.

Alignment buttons are physical (Align Left is left in either direction). New type
(`text.create`, the Type tools) is aligned `Justify::Auto`, the start of each paragraph's direction:
Hebrew grows to the left of a point type's anchor. Imported text keeps its own alignment, and SVG
import pins the direction SVG implies (left to right unless `direction: rtl`).

Vertical type keeps its logical order down the column. SVG export outlines text with right-to-left
lines (with a warning) until it is written with `direction`/`unicode-bidi`; PDF and raster exports
use the laid-out glyphs. No font is bundled for these scripts: the installed fonts' fallback
covers them (the web build needs a font with that coverage).

## Verified PngSuite corpus

`cargo xtask corpus --download` fetches the 175 original PngSuite PNGs and their redistribution
license/README from [the pinned lunapaint mirror](https://github.com/lunapaint/pngsuite/tree/8cd768dd0d0063195174d0d01cacbd5a7d1e5605).
Its README identifies `png/` as the original 2017-07-19 files; converted BMP/JSON files are excluded.
Each file's SHA-256 and exact byte length are committed in `xtask/pngsuite.sha256`.

Only HTTPS downloads and redirects are allowed. The manifest admits at most 256 flat filenames,
1 MiB per file and 16 MiB total; current PNG data totals 114,649 bytes. Curl limits each download
to its expected length, with a 10-second connect timeout and 60-second transfer timeout.
Network fetching requires curl 8.4 or newer: earlier versions do not bound unknown-length
responses ([curl documentation](https://curl.se/docs/manpage.html#--max-filesize)). Verified
caches remain usable without invoking curl.
Reads are independently bounded. Files are staged under temporary filenames, checked, then
renamed into place; failed or substituted downloads never replace an existing oracle. The fetch
stops on unexpected directories/symlinks or an existing staging file rather than overwriting them.
Verified cached files need no network access. The corpus remains gitignored and optional.

The original site's plain HTTP archive is no longer fetched or extracted. To refresh the suite,
review the mirror commit, original-file list, redistribution license, sizes and hashes together;
do not replace expected hashes automatically in response to a download mismatch.

## Highlighted-text Eyedropper (2026-10-21)

Switching from Type to Eyedropper preserves the highlighted character range and its overlay.
Sampling applies enabled font and paint attributes to that range; Shift samples color only.
Returning to Type retains the range; deselection and document changes clear stale ranges.
Character styles cannot represent object opacity or blend modes. With Transparency enabled,
sampling refuses a non-neutral source or target object before changing art, paint defaults,
selection or history. Disable Transparency in Eyedropper Options to sample supported attributes
while preserving the target object's transparency. Whole-object sampling remains unchanged.
Regression evidence: `tests/eyedropper.rs` covers Unicode, font/paint, Undo/Redo, tool switches,
and atomic rejection of source and target opacity/blend modes; `text_range_and_brush_sampling_options_coexist`
covers upstream brush-key sampling. Scores and estimates unchanged.

## Drag artwork between document tabs (2026-10-22)

Hold a Selection-tool move over another visible document tab for 400 ms to activate it,
then release over its canvas to copy the selected artwork. The original grab point is kept
across different zoom levels and view rotations. The source preview is rolled back before
the switch; source artwork, source history and the internal clipboard are preserved. Returning
to the source tab resumes a move (Alt copies). Escape or release outside the canvas cancels.
Changing the source revision or unexpectedly changing/closing the destination cancels a stale
transfer; focus loss, touch cancellation and multitouch cancel too. Moving tabs under a stationary
pointer does not activate further tabs until the pointer moves. The close icon does not activate a tab during a drag. Overflow-menu hover activation and
cross-window dragging remain outside this change.
The cited regressions assert the internal clipboard; they do not exercise the native
system-clipboard adapter. Tab activation resets the Selection tool through
`Session::set_active`; gesture regressions verify it is idle after switching, dropping
and cancelling a transfer.

`edit.dropSelection {sourceDocument, ids, grab, at}` uses document-scoped ids
(`sourceDocument` is `document.inspect`’s session-unique `uid`), validates 1–10000 ids
before editing, targets the current editable insertion context (Paste Remembers Layers does
not reroute a drag into a matching source layer), and uses the existing
lossless paste resource pipeline: groups, editable paths/type, images, symbols and patterns
travel together. Same-name conflicting swatches are added under unique names to preserve
source colours; pattern fills/strokes translate with the dropped artwork to preserve their alignment. The target receives one Undo/Redo step. Invalid or closed sources fail before
editing. Existing artwork-to-panel drags retain their original behavior.

Evidence: `tests_document_drag` drives actual tab-strip/canvas pointer frames; `tests_clipboard`
covers resource rendering, id collisions, swatch conflicts, grab-point placement, locked-target
refusal, excessive ids, malformed numeric parameters and atomic invalid-input refusal. A private Linux/Xvfb native build also placed an original logo group
onto a flyer through the command API; its screenshot was inspected, source preservation and
Undo/Redo checked. Physical mouse hover on the desktop is not covered by that native smoke test.
Public behavior reference: [Illustrator public duplication documentation](https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/duplicate-objects.html).
The tab-hover workflow follows the contributor’s requested behavior. All test artwork is original geometry/text generated locally.
The new command and validation messages are translated in all 24 language catalogs, including Turkish.
Scores and hour estimates are unchanged.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-29 | minor | Linux clipboard: core Wayland protocol text fallback where arboard has neither data-control nor X11 (the Flatpak on GNOME).; its reads wait 2 s at most. |
| 2026-10-29 | minor | Linux clipboard: a clipboard arboard can't open is retried after 1 s doubling to 60 s and logged once, instead of every 250 ms. |
| 2026-10-28 | minor | Localisation: Catalan (`ca`) catalog and registry entry (every `ca-*` locale); complete under the catalog tests. |
| 2026-10-25 | minor | Linux Wayland drag-and-drop note names eframe 0.37 (still on winit 0.30); behavior unchanged. |
| 2026-10-23 | minor | Tests: `cargo xtask test` (nextest, then doctests), one integration-test binary per crate and the files that keep their own, the dev profile's debug info. |
| 2026-10-21 | minor | Fonts › Leaving out the bundled fonts: vectorcraft-ui-egui's `interface-fonts` feature, and `theme::install_fonts` for an embedder that draws the panels without `VectorcraftApp::logic`. |
| 2026-10-21 | minor | M3: highlighted-text Eyedropper sampling preserves ranges and rejects unsupported source/target transparency atomically; regression-tested; scores and estimates unchanged. |
| 2026-10-21 | minor | Fonts › Leaving out the bundled fonts: vectorcraft-ui-egui's and vectorcraft-mcp's `bundled-fonts` features, and the engine declared without default features. |
| 2026-10-14 | minor | Document the `bundled-fonts` feature of vectorcraft-text and vectorcraft-engine (Fonts › Leaving out the bundled fonts). |
| 2026-10-14 | minor | Document registration of every explicitly supplied craft-fonts face after the existing script-specific prefix; empty input and web selection remain unchanged. |
| 2026-10-13 | minor | Document pinned PngSuite files, integrity checks and download budgets (#1729). |

> **Tab-drag status (2026-10-22):** updated from upstream, including the Turkish catalog and all three tab-drag translations requested in #3479; Selection-tool reset regression-checked and clipboard evidence scoped to internal state; drag lifecycle, insertion and regression contracts documented; behavior, scores and estimates unchanged.

| Date | Change | Tab-drag history |
|---|---|---|
| 2026-10-22 | minor | Verify Selection-tool reset after tab activation, drop and cancellation; narrow clipboard evidence to the tested internal clipboard and correct documentation grammar; behavior, scores and estimates unchanged. |
| 2026-10-22 | minor | Merge current main and complete the Turkish tab-drag labels and validation messages requested by the maintainer; all 24 catalogs covered; scores and estimates unchanged. |
| 2026-10-22 | minor | Tab-drag follow-up: reconcile upstream progress updates and document drag lifecycle, insertion behavior and regression contracts; behavior, scores and estimates unchanged. |
| 2026-10-22 | minor | Artwork dragged with Selection switches visible document tabs after a 400 ms hover, then copies editable artwork on canvas release; source and clipboard stay intact; scores and hours unchanged |
