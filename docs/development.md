# Development

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
- **Drag-and-drop:** `WebShell` takes the frame's `dropped_files` before the app sees them, reads each with `DroppedFile::bytes_async` and pushes the bytes into the inbox (files dropped on a canvas go to `Services::place_inbox` with their `place::DropAt`: the document they were dropped on, by uid, and the point; `VectorcraftApp::drop_target` decides from the position the page's drag events give). Desktop drags carry no position (winit 0.30 reports none, and the pointer position egui last had is stale), so the desktop app decides by kind: documents open, pictures and text are placed in the middle of the view. A file that arrives after another document became active still lands in its own document, which becomes active again; if that document closed meanwhile, the file is not placed and the status bar says so. (The app's synchronous drop path is compiled out on wasm32.)
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

- **Order:** adapters that report they can't present to the window are never tried; then the one `WGPU_ADAPTER_NAME` names; hardware before software (llvmpipe, WARP); the native backends (Vulkan, Metal, DX12) before OpenGL; with Automatic, the GPU that drives a display, and among those the one that drives the primary display; then the power preference; then a GPU's DX12 adapter before its Vulkan one (Intel's Vulkan driver made the whole window flicker black, #545), keeping the system's order among equals.
- **Windows backends:** Direct3D 12, falling back to OpenGL. Vulkan isn't started at all unless `WGPU_BACKEND` asks for it (`WGPU_BACKEND=vulkan`): creating a Vulkan instance loads every installed Vulkan driver into the process, and a faulty one (Intel's `igvk64.dll`, #806) crashed the app before its window appeared.
- **The GPU that drives the display.** A GPU without a monitor is the wrong one even when it says it can show the window: on a desktop with a Ryzen's integrated GPU (no monitor) and an NVIDIA card driving both monitors, power saving picked the integrated GPU, Windows had to present every frame across adapters, and the integrated GPU's driver reset under that: the graphics device was lost, or the desktop's compositor went down and every monitor stayed black for minutes (pdfcraft#378). So Automatic asks the system which GPU drives which display and puts that GPU first; the power preference only orders the rest, and decides alone where the system can't say.
  - **Windows:** `EnumDisplayDevices` (the display devices attached to the desktop, and the primary one), through the `winsafe` crate; the PCI ids in each device id are matched against wgpu's adapters. The registry doesn't tell a user which adapter drives a display, and WMI reports a resolution for a GPU without a monitor, so neither is used.
  - **Linux:** the connected connectors in `/sys/class/drm` and their cards' PCI ids; a built-in panel (`eDP`, `LVDS`, `DSI`) counts as the primary display.
  - **macOS**, OpenGL adapters (no device id) and Metal (no ids at all): no match, so the power preference decides as before.
  - The log's first lines list the display GPUs (`display GPUs (PCI vendor:device): 10de:2204 (primary)`), every adapter with whether it drives a display, and the one drawn on and why (`drawing on NVIDIA GeForce RTX 3090 (Dx12, DiscreteGpu): it drives the primary display`). That is the first thing to look for in a black-window report.
- **Preferences › Performance › Graphics Processor** (`gpuPreference`): `automatic` (the default), `lowPower` (Power Saving, the integrated GPU) or `highPerformance` (the discrete GPU). The adapter is chosen when the window opens, so a change applies after a restart. Power Saving and High Performance are the user's own choice and order by kind alone, without looking at the displays; so does `WGPU_POWER_PREF`.
  - **Automatic on Windows and macOS** is the GPU that drives the primary display, then power saving: the system shows frames from any GPU, and presenting frames rendered on a discrete GPU through the integrated one made the window flicker on some hybrid laptops (#306). On a hybrid laptop the integrated GPU drives the panel, so both rules agree.
  - **Automatic on Linux and the BSDs** is the GPU that drives the panel or a monitor, then the system's order: Mesa's Vulkan device-select layer puts the GPU the desktop runs on first (the integrated one on hybrid laptops; `DRI_PRIME` and `MESA_VK_DEVICE_SELECT` steer it). A Wayland compositor may not accept frames from another GPU: on a desktop whose compositor ran on an NVIDIA GPU, rendering on the Ryzen's integrated GPU made KWin end the window's connection ("importing the supplied dmabufs failed") and 0.5.0 crashed at startup (#502).
  - 0.5.0 saved `powerSaving`, its default, for everyone; it reads as `automatic`.
- **If the window fails on its adapter while starting up** (an error from wgpu, or a panic inside wgpu or egui-wgpu during the first frames), the app starts again without that adapter and tries the next one, telling which in the status bar; the run that failed keeps its log as `vectorcraft.1.log`. On Unix the new app replaces the process (an AppImage stays mounted); on Windows it starts beside it. Adapters are told apart by backend and PCI ids, or by name where the backend reports no ids (Metal lists every GPU of a dual-GPU Mac as `0000:0000`, #651). The adapters left out travel in `VECTORCRAFT_GPU_SKIP` (`backend:vendor:device`, such as `Vulkan:1002:164e`), one more on each restart, so the restarts end. When no adapter is left, the app logs why and exits with an error instead of panicking.
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
| `VECTORCRAFT_WAYLAND` | Any value: keep native Wayland backend on Linux/Unix instead of running through XWayland for file drag-and-drop |
| `VECTORCRAFT_CONTROL_PORT` | Same as `--control <port>` |
| `VECTORCRAFT_IN_WINDOW_MENUS` | Any value but empty or `0`: same as `--in-window-menus` (see [macOS: the menu bar](#macos-the-menu-bar)) |
| `VECTORCRAFT_NO_PREFS` | No preferences read or written, no default Data Recovery folder and no log file (agents' test runs) |
| `WGPU_POWER_PREF` | Graphics adapter: `low`, `high` or `none` (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `WGPU_ADAPTER_NAME` | Graphics adapter by (part of) its name, any case (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `WGPU_BACKEND` | Graphics backends: `vulkan`, `dx12`, `metal`, `gl` |
| `VECTORCRAFT_GPU_SKIP` | Set by the app when it starts again without a graphics adapter that failed (see [Desktop graphics processor](#desktop-graphics-processor)) |
| `RUST_LOG` | Log levels for standard error and the log file (see [Logs](#logs)) |

## macOS: the menu bar

On macOS the menus live in the system menu bar, not in the window's app bar (which keeps the brand mark, Home, the search box and the workspace switcher next to the traffic lights). `--in-window-menus` or `VECTORCRAFT_IN_WINDOW_MENUS=1` keeps them in the window, as on Windows, Linux and the web; so does a menu bar AppKit refuses to build (logged). `ui.inspect` says which with `nativeMenuBar`.

- **One menu model.** `crates/ui-egui/src/native_menu.rs` (platform-free, tested on every platform) builds the menu bar from the in-window menus (`menus::menu_tree`, each item through `menus::entry`, which the in-window menus draw too) and lays it out the Mac way: the VectorCraft menu holds About, Settings ▸ (one item per Preferences page, General… ⌘K), Language ▸, Appearance ▸ (UI brightness), Services, Hide VectorCraft ⌃⌘H (⌘H stays View › Hide Edges), Hide Others ⌥⌘H, Show All and Quit ⌘Q (the app's own `app.quit`, so unsaved documents are asked about); those leave Edit and Help. Window starts with Minimize ⌃⌘M and Zoom and ends with Bring All to Front, and Help is the system's Help menu (its search field). A system key a command (or a user's shortcut) already has stays the command's, and the log says so.
- **The native side** is `apps/vectorcraft/src/mac_menu.rs`: muda (`=0.21.1`, default features off, macOS only, no `unsafe`) builds the `NSMenu`s. Labels, enabled and checked are updated in place when anything they show may have changed (a command ran, the document, its history or selection changed, a click or a key press); a change of structure (a recent file, the language, a shortcut) rebuilds the menus.
- **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key, so the item's key goes back to egui as the key press it was (⌘X/⌘C/⌘V as egui's Cut/Copy/Paste events), through `raw_input_hook`: `shortcuts::handle` runs it once, user shortcuts apply, and a focused text field keeps ⌘A, ⌘C, ⌘V and ⌘Z. A click runs the item like an in-window click. Bare keys and ⌥/⇧ chords are never key equivalents (they type).
- Built and clippy-checked for `aarch64-apple-darwin` and `x86_64-apple-darwin` from any host: `cargo clippy -p vectorcraft --target aarch64-apple-darwin --all-targets -- -D warnings`.

## Linux: Wayland and X11

The window runs natively on Wayland (eframe's `wayland` feature) and on X11. On machines with two GPUs it renders on the one the desktop runs on unless Preferences say otherwise; see [Desktop graphics processor](#desktop-graphics-processor) (#502). The system clipboard (`apps/vectorcraft/src/clipboard.rs`) is arboard with its `wayland-data-control` feature:

- **Clipboard:** under Wayland arboard talks to the compositor through the data-control protocol (wlroots compositors such as Sway and Hyprland, KDE Plasma), so bitmaps (`image/png`), SVG markup and text copied in any app paste (#398). Where the compositor lacks the protocol (GNOME's Mutter, [arboard#223](https://github.com/1Password/arboard/issues/223)), arboard falls back to the X11 clipboard through XWayland, which only holds what X11 apps copied. egui's own text paste into fields goes through smithay-clipboard and works either way.
- **Clipboard owners that never answer (#510):** reading the clipboard waits on the app that owns it: arboard gives up on an X11 owner after 4 s, and a Wayland read waits for the owner to send its data. So on Linux the check that enables the Paste menu items runs on a background thread with a second clipboard handle (`crates/ui-egui/src/clipboard_probe.rs`, installed as `Services::clipboard_probe`), only while nothing is copied in the app and a document is open, and the window keeps drawing. A paste itself still reads on the UI thread, so pasting from such an owner waits for it once. Windows and macOS keep the check on the UI thread: Windows only asks which formats the clipboard holds, macOS asks the pasteboard server.
- **Copied files:** files copied in a file manager (`text/uri-list`; `CF_HDROP` on Windows, file URLs on macOS) paste as the first one that is art (SVG, PDF, EMF/WMF or a bitmap), before the clipboard's other formats (which include the files' paths as text).
- **Dropping files on the window (Wayland desktops):** winit 0.30, which eframe 0.36 runs on, has no Wayland drag and drop ([winit#1881](https://github.com/rust-windowing/winit/issues/1881), added in winit 0.31; [egui#1563](https://github.com/emilk/egui/issues/1563)), so dropping files on a native Wayland window showed a "no" cursor. On Linux/Unix desktops where an X server is reachable (`DISPLAY` is set), VectorCraft opens through XWayland by default so file drag-and-drop works; setting `VECTORCRAFT_WAYLAND=1` keeps the native Wayland backend. Until eframe moves to winit 0.31, files can also be copied in the file manager and pasted into the canvas.
- **Pens and pen displays under Wayland (#491):** winit 0.30 doesn't bind the Wayland tablet protocol (`zwp_tablet_v2`; tablet input arrives in winit 0.31, [winit#4318](https://github.com/rust-windowing/winit/pull/4318), [egui#7731](https://github.com/emilk/egui/pull/7731)). Since Plasma 6.3, KWin no longer turns tablet input into pointer input for such apps ([kwin!6336](https://invent.kde.org/plasma/kwin/-/merge_requests/6336)): the pen moves the compositor's cursor over the window, but the app gets no motion or presses and shows none of its own cursors. Until eframe moves to winit 0.31, either run the app under XWayland, which supports the tablet protocol and hands the pen to X11 apps as a pointer (`WAYLAND_DISPLAY= vectorcraft`, or `env -u WAYLAND_DISPLAY vectorcraft`; for a desktop launcher, put `env WAYLAND_DISPLAY=` before the command in its `Exec=` line), or turn KWin's deprecated emulation back on for the whole session by setting `KWIN_WAYLAND_EMULATE_TABLET=1` in KWin's environment (for example in a file under `~/.config/environment.d/` or in `/etc/environment`) and logging in again. The pen then works as a mouse, without pressure: winit 0.30 reads pen pressure on Windows only (Windows Ink, which reaches the Liquify tools' Use Pressure Pen). Other compositors that don't emulate a pointer for tablets behave the same way.

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
- UI: `theme::install_fonts` adds them at the end of every egui family (BIZ UDPGothic first), after
  the UI fonts and before the installed fonts `ui_fonts.rs` discovers for other scripts.
- Tests that need Japanese glyphs skip with a message when built without craft-fonts. The FreeBSD
  CI job and every release job (`release.yml`) check craft-fonts out at a pinned commit; release
  packages carry each embedded font's `OFL-<family>.txt`.

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
so the two scripts never mix), Japanese (`ja`, complete), Spanish (`es`, complete, in neutral
international Spanish; every `es-*` locale such as `es-ES`, `es-MX`, `es-AR` or `es-419` resolves to it), French (`fr`,
complete; `fr-FR`, `fr-BE`, `fr-CA`, `fr-CH` and every other `fr-*` locale resolve to it), Italian (`it`,
complete; `it-IT`, `it-CH` and every other `it-*` locale resolve to it), Russian (`ru`, complete; every `ru-*`
locale such as `ru-RU`, `ru-BY` or `ru-KZ` resolves to it), Ukrainian (`uk`, complete; `uk-UA`,
`uk_UA.UTF-8` and every other `uk-*` locale resolve to it; integer plurals use one/few/many), Czech (`cs`, every menu label) and Brazilian Portuguese
(`pt-br`, every menu label and every `tl!` literal). Untranslated text falls back to English until its rows are added.

- `tl!("…")` translates a literal into the language the UI is drawn in; `i18n::t(s)` is the same for a
  `&str`. `tr(lang, s)` takes the language; `tr_ctx` when one English word needs different translations;
  `tr_id(lang, command_id, label)` for menu items (keyed by command id, English label as the fallback);
  `tn` / `trn(lang, n, one, other)` for plurals; `fmt` fills `{name}` placeholders, which a translation may
  reorder.
- The shared widgets (`widgets::check`, `dropdown`, `menu_item`, the buttons, `label_row`, tooltips of
  `icon_button`…), the menus, the dock, the toolbar, the dialog frame and `panels::empty_state` translate
  the text they are given, so a panel mostly needs its literals wrapped in `tl!` to be covered by the tests.
- The language is VectorCraft › Language (the `app.language` UI command, `{lang: auto|<code>}`) or Edit ›
  Preferences › User Interface › Language; both set the `interfaceLanguage` preference (`auto` or a language
  code; `auto` follows the system locale: `VECTORCRAFT_LOCALE`, then `LC_ALL`/`LC_MESSAGES`/`LANG`/`LANGUAGE`,
  the macOS preferred languages, the Windows user locale; on the web, `?lang=<code>` in the address, then
  the browser's `navigator.languages`). The Preferences dialog previews the chosen language before OK.
- To add a language: add `<code>.tsv` and one row in `i18n::LANGUAGES` (code, native name, catalog, plural
  rule). The Language menu, the dropdown, locale matching and the catalog tests (well-formed, no duplicates,
  placeholders and ellipses agree, command ids exist, every row of a partial catalog is a string the UI
  shows, the UI fonts have every glyph, no Simplified characters in `zh-hant`) pick it up. Set `complete_menus`
  once every menu string and `tl!` literal is translated; `i18n::tests` then enforce it
  (`VECTORCRAFT_I18N_DUMP=strings.txt cargo test -p vectorcraft-ui-egui dump_source_strings` lists them).
- Translations are clean-room: written from the meaning of the English text in ordinary vocabulary, never
  from another product's localisation resources. Product and technology names stay in Latin letters.
- Status-bar messages and errors stay English where they are made (`app.ui.status`, `EngineError`: the
  control channel, MCP and tests read them) and are translated only where the status bar draws them
  (`i18n::msg`). `@msg` catalog rows hold a whole message or a template such as
  `Couldn't open {name}: {e}`; `{_1}`, `{_2}` … stand for the format string's `{}`, and the values in the
  placeholders are translated in turn (the reason after `: {e}` is often a message too). Spanish, French, Italian, Japanese, Russian and Ukrainian cover every
  message literal the test scan finds (`complete_languages_translate_every_message`, languages listed in
  `COMPLETE_MESSAGES`): a new `Err("…")`, `Other(…)`, `#[error(…)]` or `status(…)` message needs an `es.tsv`,
  a `fr.tsv`, an `it.tsv`, a `ja.tsv`, a `ru.tsv` and a `uk.tsv` row (`VECTORCRAFT_I18N_DUMP_MESSAGES=messages.txt cargo test -p vectorcraft-ui-egui
  complete_languages_translate_every_message` lists them all). Other languages show messages in English
  until they add `@msg` rows.
- Not translated on purpose: names that are user data (layers, swatches, fonts, documents), the tab
  title's colour mode, command ids and parameter names inside messages. Not done yet: locale-aware number
  and date formats, right-to-left layout. Chinese and Japanese UI text
  is drawn with craft-fonts' BIZ UDPGothic when the build embeds it (see Fonts above), else with an installed
  system font; the glyph test checks Latin and Cyrillic catalogs always and the CJK ones only with craft-fonts.
  A Traditional Chinese UI font is still to be added to craft-fonts for the web build.

## Vendor names gate

`cargo xtask brands` (part of `cargo xtask ci`) fails when user-visible text names another vendor's products or company: string literals in Rust sources (command labels and params docs, menus, panels, MCP tool definitions), `Cargo.toml` descriptions and packaging files. Comments and test code are not checked. Say "the reference app" or name the feature itself. A line that must keep an old name, such as an alias that files or preferences from earlier versions still use, carries a `brand-ok` comment.

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
Integration-test files under `tests/` and `examples/`, and the `testkit` crate, opt out with
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
