# KDE Plasma / Linux

Updated 2026-10-22. Linux exports the existing local command tree through
`vendor/craft-desktop`, while preserving the local menu. The adapter rechecks the active
document, dynamic targets, modal state and enabled state before calling the normal
interactive dispatcher, including its focused-field routing. No second command registry exists.
Native action dispatch and snapshot generation share the UI's last-resort panic recovery
behavior: report an internal error and keep the application open. Automatic brightness
checks the effective palette in Appearance, and portal-only changes invalidate its menu cache.

Native Wayland uses KDE's appmenu protocol on the existing window surface; the vendored
winit extension returns a safe cloned proxy and leaves socket/surface ownership and event
dispatch with winit. X11/XWayland uses window properties and `com.canonical.AppMenu.Registrar`.
Keep both vendor directories and the root Cargo winit patch in source distributions;
see [the patch record](../vendor/winit/CRAFT_PATCH.md). No Qt conversion or GTK module is needed.

## Build and launch

```sh
cargo build -p vectorcraft
vector_kde_config=$(mktemp -d)
# In a Plasma Wayland session, prevent the tablet policy from selecting XWayland:
env -u DISPLAY XDG_CONFIG_HOME="$vector_kde_config" cargo run -p vectorcraft
# In a session with an X display, test X11/XWayland separately:
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET XDG_CONFIG_HOME="$vector_kde_config" cargo run -p vectorcraft
```

Run these separately, closing the first test window before the second. Use disposable
documents and keep the temporary configuration for inspection. Existing tablet/display-server
policy is unchanged. The updated pinned winit fork implements native Wayland file-drop delivery;
that inherited support has not been verified in a live session for this reconciliation.
XWayland may blur fractional scaling.

Add Plasma's **Global Menu** widget manually if needed. Without its widget, registrar,
session bus or compositor protocol, the local menu remains available.
`VECTORCRAFT_NO_GLOBAL_MENU=1` or `KDE_NO_GLOBAL_MENU=1` disables export. The application
does not change desktop configuration. Existing window-decoration choices are retained.

## Automatic appearance through automation

Automatic is a persisted preference, not a fifth concrete palette. Through an explicitly
enabled control server, select it using the normal preference command:

```json
{"id":1,"method":"engine.execute","params":{"command":"prefs.set","params":{"key":"uiBrightness","value":"system"}}}
```

The effective UI brightness still reports a concrete palette (Light or Medium Dark).
The existing `window.brightness` command and `ui.set` brightness field select only the four
fixed palettes; `ui.set {"brightness":"system"}` is therefore not the Automatic API.
Changing a fixed palette remains an explicit opt-out from following system appearance.

## Reproducible validation

```sh
cargo fmt --all -- --check
cargo check -p vectorcraft
cargo clippy -p vectorcraft -p vectorcraft-ui-egui --all-targets -- -D warnings
cargo test -p vectorcraft-ui-egui tests_nativemenu
cargo test -p vectorcraft-ui-egui desktop_menu_
cargo test -p vectorcraft-ui-egui automatic_brightness_
cargo test -p vectorcraft-ui-egui fractional_monitor_changes_
cargo test -p vectorcraft-ui-egui fixed_palette_
cargo test -p vectorcraft-engine system_brightness_is_opt_in_and_round_trips
cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal
dbus-run-session -- cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal appmenu_dbus -- --ignored
```

Check native Wayland and X11 separately:

1. Focus the editor and another application; the panel must follow focus with textual
   menus. Open New/Open/Save As/Preferences through the panel and cancel each dialog.
2. Toggle Rulers, make an edit and Undo/Redo; compare checked and enabled state with
   the local menu. Check selection-dependent commands and text-field edit routing.
3. Switch between two documents, close one, and reject delayed old-document/dynamic-target
   clicks. Close a dirty document and cancel its normal save prompt. Document commands
   must be unavailable during a modal dialog.
4. Disable export and test without a widget; keep local menus usable. Test menu-service
   restart/re-registration manually, without scripting a restart of the user's shell.
5. Select Preferences › User Interface › Brightness › Automatic and change desktop
   light/dark manually. Then select a fixed palette and verify it stays fixed; custom
   accents and schemes must remain unchanged. System accent adoption is not implemented.
6. With UI Scaling at 1×, resize at 100/125/150/175/200% and move between different-DPI
   monitors. Inspect text, pointer alignment, long menus, dialogs and native titlebar controls.
   A manual UI Scaling value remains a multiplier, not an absolute desktop scale.
7. Open/save/cancel through the KDE portal picker. Repeat in packaged Flatpak; registrar
   permission alone does not establish sandbox/runtime acceptance.

See [appearance and scaling](development.md#kde-appearance-and-scaling) for implementation
details and fallback behavior. Compilation and protocol/model tests do not certify visible
Plasma rendering, physical monitor transitions or portal interaction. The
[Recorded verification](#recorded-verification-2026-10-21) section lists checks actually run;
the checklist is not a claim of comprehensive GUI acceptance.

## Review follow-up (2026-10-22)

App/UI all-targets Clippy with `-D warnings` passed on the review fixes. Asset checks
cover 332 remaining files; dependency layering passed for 28 crates. The shared adapter
passed 17 Linux/portal tests, two separately run private-bus tests and two model-only tests.
The fresh UI test binary passed six desktop-menu tests, the automatic-brightness regression
and twenty existing native-menu tests (27 distinct tests; the full UI suite was not run).
The four unused share-alike keyboard diagrams were removed, with provenance retained in
the vendored winit notices and `CRAFT_PATCH.md`.

A fresh native Wayland run used a temporary configuration with preference persistence
disabled. Automatic resolved to the current dark theme; the window screenshot was inspected,
the exported menu answered `GetLayout`, Preferences opened/closed, and Quit exited normally.
This was not a physical desktop-theme transition or a complete panel interaction test.
Full CI remains incomplete: the fresh 120-second run stopped during workspace Clippy,
and WASM remains blocked by the missing target standard library. Current-main conflicts
were not resolved as part of this follow-up.

## Migration verification (2026-10-30)

The isolated reconciliation with upstream baseline `e256c4c43` preserves the split host modules,
egui/winit fork versions and desktop command panic boundary. App/UI all-target checking
and Clippy with warnings denied passed. The 29 focused regressions passed (six desktop,
twenty native-menu and three appearance/scaling tests), as did layers, assets, brands,
PDF dependency checks and the exact WASM stage (23 crates).

The exact `cargo xtask ci` completed successfully: all 12 stages passed, including workspace
Clippy, 10,966 workspace tests (14 ignored), doctests, the no-bundled-fonts checks, PDF
dependencies, 334 attributed assets, brands, cursors, i18n reporting, layers and WASM.
The UI suite passed all 2,377 tests. Required fonts used the workflow-pinned craft-fonts
revision `b076417b5880bd4154bd22f00695b1d28c6bea83`; the no-bundled-fonts stage also passed
its one text and two panel integration tests. Existing optional web-font warnings and
translation-report gaps are not claims of complete font or language coverage.

The rebuilt native application passed separate Plasma Wayland and XWayland smoke tests.
In each, a real D-Bus `window.control` activation changed the control-bar state from false
to true and updated its exported checkmark; Plasma requested the menu layout, screenshots
were inspected, and normal shutdown removed the menu service without forced termination.
These checks used isolated configuration and disabled preference persistence.

This verifies the stated baseline, not later upstream commits. Physical file dragging,
multi-monitor scale transitions, theme transitions and packaged Flatpak acceptance were
not established. The observations below retain historical results from the older build.

## Recorded verification (2026-10-21)

The Linux production build, `cargo check`, app/UI `cargo clippy --all-targets -- -D warnings`
and combined app/UI/engine test typecheck passed. Targeted runtime tests also passed:
four `desktop_menu_` adapter regressions, twenty existing `tests_nativemenu` regressions,
three appearance/scaling regressions and one engine System-brightness preference roundtrip
(28 distinct tests, zero failures or ignored tests). The app, UI and engine test packages
were built together with production feature unification; remaining filters reused the
already-built executables. The full 1831-test UI and 2717-test engine suites were not run.
The host's empty filtered run is not counted as host-test coverage.

In an isolated configuration on the host's real Plasma Wayland session:

- Plasma was observed consuming the exported menu through `GetLayout` and `AboutToShow`;
  D-Bus integration checks exercised its `Event` path. Association used the existing native
  surface, without an X11 window ID.
- The ten text sections were VectorCraft, File, Edit, Object, Type, Select, Effect, View,
  Window and Help. Activating the exported Show Rulers action changed its checkmark from
  0 to 1. New opened `newDocument` and disabled Close until the dialog was cancelled.
- After a synthetic rectangle edit, the exported Close action opened `saveChanges`;
  Cancel retained the document. Quit also opened the save guard; discarding only the
  synthetic document allowed exit and removed the exported service.
- The preference command above accepted `uiBrightness: system`; its effective Medium Dark
  palette matched the host portal's dark preference. Both Automatic and explicit Light
  native screenshots (2880 × 1537) were visually inspected. Both test processes quit
  and their exported services disappeared.

Runtime logs/screenshots were local, temporary test artifacts, not committed fixtures.
The observations above summarize the native-session checks. This checks
the current portal preference, not a physical desktop-theme transition or every menu item.
Physical multi-monitor transitions, X11 behavior and packaged Flatpak acceptance remain
separate checks. Targeted unit-test results are distinct from typechecking and native-session
observations; none establishes that every manual checklist item has passed.
