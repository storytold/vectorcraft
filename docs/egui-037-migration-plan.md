# VectorCraft — egui 0.37 migration plan

**Status (2026-10-25, shared-docking reconciliation):** the published qualification remains checkpoint `83d6dd2d6603845978f949523e6da2b4945dcffb`, tested with post-PR-4113 `main` `6e7854967d6b8a094bfed809643bd957aa3ba66f` and the two `Id::unique` edits at `crates/ui-egui/src/dialogs/recolor.rs:384,725`; managed job `egui037-vector-postmerge-ci-20261025-1381` passed all 12 mandatory CI stages for that working tree, and publication commit `1072d0e263623882da68d8115934630abbef33ae` records it in PR #4173. The reviewed recolor code is already in `main`: `HEAD` and `origin/main` resolve the same blob `6050af15a3bf7b9a056df6dff347bda7690ad251` for that file.

This checkout, at documentation-only follow-up `12fee85c9cbf9f7f1609a53bc67327b12e4f4dd1` (`M14: distinguish qualification from publication`), is locally reconciling upstream `main` `714434527e1a3e03c4b49096e30dd92b258aab50` (PR #4135: panels on craft-ui's shared docking, floating panels in OS windows). The four conflicted files were resolved by taking upstream's `dock.rs`, `panel_docking.rs` and `panel_docking/tests.rs` and deleting `floating.rs`; `git diff` against `714434527…` is empty for those three files, and no `crate::floating` or `mod floating` path remains. Managed CI job 1422 passed all 12 mandatory stages on the pre-merge tree (`12fee85c9…`); managed job `egui037-vector-combined-ci1443` subsequently passed all 12 mandatory CI stages on the combined working tree, including WASM. Independent integration reviews accepted the conflict resolutions and shared-library pins. This qualification covers the executable source and lockfile; the qualification prose was updated afterward to record that result. Native/platform GUI and IME workflows, final packages and paired app performance remain open.

**Historical status (superseded, not current acceptance):** the earlier reconciliation against `main` `8bf090b25389b48aa6f9d7415de57c9ce7ccf8cb` and managed job `egui037-vectorcraft-artboard-green-ci-20261025-0589` recorded 10,197 passing test executions, zero failures and 14 ignored, including the artboard-name geometry regression, input/IME fixtures, no-bundled-font checks and the web app build. The 0.36.2 lock table and the plan-time local craft-libs/CL-2F statements below are historical planning state. Current dependency pins (read from this checkout's `Cargo.toml` and `Cargo.lock` on 2026-10-25): `egui`, `eframe`, `egui-winit`, `egui-wgpu`, `egui_glow`, `egui_extras`, `egui_kittest`, `epaint`, `emath`, `ecolor`, `epaint_default_fonts` are `0.37.0` from `storytold/egui` rev `6d518fc799681571963842f95b55a6b68431b15a`; `egui_system_font_provider` has an unused patch at that same revision and is not in the resolved dependency graph; `winit` is `0.30.13` from `storytold/winit` rev `a517b3846ee8457d84ee4975110bfda6797c44dc`; `wgpu` is `30.0.1`; `kittest` is `0.5.0`; `accesskit`/`accesskit_winit` are `0.25.1`/`0.34.1`; `craft-ui`, `craft-window`, `craft-keymap`, `craft-print`, `craft-pdf` and `craft-docpath` use published `storytold/craft-libs` rev `a0916808f7d78bed8c7b90cdf937176015454a7f`, as do the `hayro-*` and `krilla` patches.

**Earlier status (historical):** implementation revision 4, 2026-10-24. The VectorCraft migration is implemented in the
`vectorcraft-implementation` worktree; this revision incorporates the final upstream-review repairs. At revision 4
the exact tree still needed a fresh managed build/test/lint run before publication; earlier managed evidence
(`tests-12`, `lints-13`) is not claimed for those later edits. Every claim below remains source-grounded in the
checkouts named here; historical compiler diagnostics and reviewer findings are clues, not acceptance evidence.

**Assigned source:** `/home/codex/work/CraftApps/egui037-migration-20261022/vectorcraft-implementation` — the
implementation worktree. The earlier pinned review checkout was `vectorcraft` at
`8906073438a2f7f0d3634d460b900956134f65e6` (2026-10-23 11:50:04 +0700).
**Worktree:** implementation worktree in place; earlier plan worktree `codex/egui-037-plan-20261022`.
**Pinned review baselines:** revision 2 was
`d8c6a7594015919751d2b83bbe0e44ee9fdb4696b4e5c90fb1f8ecbdf7335f83` (107,991 bytes), reviewed in
`plan-reviews/vectorcraft-v2.md`; revision 1 was
`70938f132709da9073910631e90364738e8df3dfc0d7b354920ba7910fc01fbe`, reviewed in
`plan-reviews/vectorcraft-v1.md`. Both reviews are under
`/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/`.
**Remote drift snapshot:** `storytold/vectorcraft` main advanced from `8906073…` to
`4a7cba0dd32e4ac249e4dc50af486b89531481aa` in two commits touching only
`crates/cad/src/import/curve.rs` (DXF spline); see
`/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/remote-plan-bases-20261023-0632.json`.
**Clean-room note:** this plan uses only public specifications, public documentation and source code. It does not use, inspect or compare against any Adobe application, asset or output.

### Shared-docking integration (current, 2026-10-25)

This subsection records the current code after PR #4135; §§2–5 below remain the original planning record and are marked historical where the merged code replaced them.

- **Desktop versus web/Wayland:** on the desktop, floating panels open in OS windows of their own through craft-ui's target-gated `native-windows` feature (`crates/ui-egui/Cargo.toml`, `crates/ui-egui/src/panel_docking.rs` module docs, `apps/vectorcraft/src/main.rs::native_input_hook`). On the web and on Wayland they float inside the app window (`panel_docking.rs`; `docs/control-protocol.md`).
- **Removed app-side floating:** `crates/ui-egui/src/floating.rs` is deleted. `dock.rs` is now 137 lines and holds `panel_body` plus two scroll-bar tests; `panel_docking.rs` draws the canvas and panels through `craft_ui::docking::DockArea`, with the session-only `Runtime`, `inspect`, `hit_rects`, `known` and `valid`. The old `apply_pending_dock_width`, `cancel_legacy_drag`, `seed_dock_panel_width`, `note_dock_width`, `legacy_dock_default_width`, `shared_dock_default_width` and `dock_width_pending` symbols are gone. `tools_float.rs` keeps the Tools panel's own move state.
- **Dock-width persistence:** `UiState::dock_width` is the app-owned preference. `panel_docking::main_width` clamps it into the default layout, `default_flags`/`Transaction::commit` writes a resized width back, `workspaces::Workspace` captures and applies it (test `saved_workspaces_restore_dock_width`), and `layout_migration` carries it across older preference files. No `PanelState` seeding is used for dock width.
- **Tests:** `panel_docking/tests.rs` covers pointer/keyboard docking; `panel_docking/native_tests.rs` covers desktop OS windows (keyboard, drops, modal focus and `ui.docking` hit-rect filtering); `layout_migration/tests.rs` covers conversion of layouts saved before the canvas joined the shared dock; `dock.rs` retains two scroll-bar accessibility tests; `workspaces.rs` covers saved-width restore. These test surfaces are included in the combined-tree CI qualification recorded above; native GUI workflows remain a separate open gate.

This plan is deliberately app-specific. The suite-wide dependency order and shared acceptance contract are in the
local planning draft `craft-libs/docs/egui-037-suite-rollout.md` (not yet published at the time of writing); this
document only records what VectorCraft must do, what it must prove and what remains unresolved. The shared
lifecycle, capture-slot, performance and font-fixture contracts in that rollout and in
`egui/docs/egui-037-migration-plan.md` apply here; this document does not weaken them.

---

## 1. Executive decision

Migrate VectorCraft from its pre-migration coherent egui **0.36.2** family (then resolved from crates.io, with
`craft-ui` and `craft-window` pinned to 0.36-era craft-libs revisions) to the selected **egui 0.37.0** family and the suite's
`winit 0.30.13` language fork, in a sequence that never leaves `main` with a mixed or unbuildable dependency graph.

The migration is not a version-string bump:

1. The app already has the **`eframe::App::logic` / `App::ui` split and a root `Ui`** on 0.36.2. That split is not a
   0.37 feature and must not be rewritten wholesale. The real work is to make the split honest: background
   `tick` work must not read stale UI input or stale `i.time` while the window is hidden, input must be consumed
   only in a known UI phase, and the UI pass must not duplicate side effects when egui runs a second layout pass.
   `raw_input_hook` is called by both native and web hosts before they decide whether to run a full update or
   logic-only; it marks a host iteration, not a UI phase.
2. 0.37's `WidgetType` → `accesskit::Role` deprecation/alias and variant renames, `Id`/`IdSalt` split, `Ui::scope_id`, screenshot callback, `FontsView` privacy,
   `ScrollArea::State::offset` privacy, `WidgetInfo.role`, `MouseWheelSource` and `accesskit::Role` changes are
   concrete compile/API changes. The historical diagnostics in
   `/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/{fork,stock}/vectorcraft.errors` already list many
   of them (35 entries; the fork and stock lists are identical). They are line-drifted clues, not a current
   acceptance baseline.
3. The app has three behaviours that need explicit redesign, not mechanical renames:
   - screenshot completion (`ui.screenshot` reads a removed `Event::Screenshot` and `egui::UserData`); the final
     design is a bounded per-request completion slot whose callback holds only a weak slot reference;
   - native/web clipboard input (0.37 makes eframe's `clipboard` feature opt-in, and adds `PasteImage` /
     `MiddleClickPaste`), including the winit-owned W15P PRIMARY path routed through E4;
   - IME replacement and outgoing surrounding text (`ImeEvent::DeleteSurrounding` upstream;
     `DeleteSurroundingBytes` in the selected fork; E21b `IMEOutput` payload for W27) for the canvas's custom text
     tool.
4. Accessibility is now a build gate: `egui_kittest` 0.37 fails an input widget with no accessible name, and the
   kittest harness sets `MissingGlyphPolicy::Panic`; production layout uses the default `Tofu` policy. VectorCraft
   has many custom painted fields and custom combo boxes; every visible input needs a real name and role, not just
   a painted label.
5. `egui` 0.37 no longer enables `default_fonts` by default. VectorCraft's `theme::install_fonts` starts from
   `FontDefinitions::default()`; without the explicit feature its default families are empty, so text silently
   lays out as synthetic tofu in production and the `install_fonts` non-empty-family assertion fails. Enable
   `egui/default_fonts` explicitly: kittest's own `[dev-dependencies]` fonts do not reach VectorCraft's tests.

**Selected dependency set (primary):** approved, reachable `storytold/winit` revision containing W1–W15 (minus
W5, plus replacement W20) and W17–W27 plus W15P and the bounded transfer API (D1a), and the complete `storytold/egui` P8 revision
containing P2b, P7, P7b/E21b and P7c (D1b). Final SHAs are execution prerequisites created by that fork work; none
is invented or asserted in this plan, and no intermediate fork stage becomes a production pin. The aiongg
`craft/0.37` (`a4f9a17…`) and `winit craft/0.30.13-lang` (`8b35388b…`) trees remain research inputs, not production
pins. **Fallback:** none for production. Stock 0.37.0 is the measurement/attribution arm only; it lacks E21b, W15P
and P7c, so it cannot substitute for D1b. If the fork work cannot land, stop and report the open gates rather than
publishing a mixed or reduced graph.

---

## 2. Baseline and inventory

### 2.1 Exact revisions and identities

| Item | Exact reference | Use in this plan |
|---|---|---|
| VectorCraft pinned review baseline | `8906073438a2f7f0d3634d460b900956134f65e6` (2026-10-23 11:50:04 +0700) | Source for every anchor and the pre-change executable/lock; the assigned checkout stays here during plan repair |
| VectorCraft remote main at drift snapshot | `4a7cba0dd32e4ac249e4dc50af486b89531481aa` | Two commits after the pinned baseline, both only `crates/cad/src/import/curve.rs` (DXF spline). Integration gate: rebase onto this (or later) main, preserve the DXF fix, and isolate it from migration performance (§6.2) |
| Upstream egui 0.37.0 | `841a328ba2dd8f6996beb7189ebb7fc00e668a2a` (tag `0.37.0`) | Behaviour/API reference; release notes and migration decisions |
| Upstream egui 0.36.2 | `49682f8baa058bf49e011035cfbd6e825f88a5ef` | Stock comparison and old coherent set |
| aiongg egui candidate | `a4f9a17f64a6bae61350fb85804c9df0d9865916` (`aiongg/craft/0.37`) | **Research input only**, not a production pin; source comparison anchor |
| storytold egui 0.36.2 | `767b1cad294120689a514bb4bd68c077e46679b9` (`craft/0.36.2`) | Existing published fork for comparison |
| aiongg winit candidate | `8b35388bd719e8cc86a95971ba0bc026c9a8f058` (`aiongg/winit craft/0.30.13-lang`) | **Research input only**, not a production pin; missing W15P and W27 lifecycle revisions |
| Published storytold winit | `ffae5198770d7730e718043f035a772b7dbc3b1b` (`craft/0.30.13`) | Old coherent comparator |
| craft-libs planning base (historical) | `3832c47ef68a0e6214f2c1e85f3526b6adcb686e` | Local shared-library tree used to prepare the port; it carries PR 199's egui 0.37 update. It is not the current pin. The current workspace uses published craft-libs `a0916808f7d78bed8c7b90cdf937176015454a7f`, which includes the CL-2F capture helper |
| craft-libs PR 199 head | `b68de8c5f4f1c060904d211a7b1320aa0f7bce78` | **Stale tree; do not merge.** Reuse selected port hunks only (§3.7) |
| craft-libs PR 199 API commit | `8614e4a04fae516fba16c22b31ff1008cb6be10b` | Historical source of the egui API-port hunks already contained in the `3832c47…` base |
| craft-libs PR 199 README commit | `1764bca` | README/`scope_id` example hunk to review and apply separately |
| craft-libs audit merge | `audit/main-plus-199` (`ec8840af…` locally) | **Evidence of conflict resolution risk, not a source.** It drops security checks and lacks `craft-platform`; see §3.7 |
| Adversarial plan review, revision 1 | `plan-reviews/vectorcraft-v1.md` | Reviewed document `70938f132709da9073910631e90364738e8df3dfc0d7b354920ba7910fc01fbe`; findings resolved in §8.1 |
| Adversarial plan review, revision 2 | `plan-reviews/vectorcraft-v2.md` | Reviewed document `d8c6a7594015919751d2b83bbe0e44ee9fdb4696b4e5c90fb1f8ecbdf7335f83`; findings resolved in §8.2 |
| Remote bases snapshot | `remote-plan-bases-20261023-0632.json` | Records all suite heads and the two-commit VectorCraft drift above |

Historical pre-migration VectorCraft lock identities (from `Cargo.lock` at the pinned `890607…` baseline; the 0.36 comparison arm, not the current lock):

| Package | Version | Source |
|---|---|---|
| `egui` | `0.36.2` | crates.io |
| `eframe` | `0.36.2` | crates.io |
| `egui-winit` | `0.36.2` | crates.io |
| `egui-wgpu` | `0.36.2` | crates.io |
| `egui_glow` | `0.36.2` | crates.io |
| `egui_extras` | `0.36.2` | crates.io |
| `egui_kittest` | `0.36.2` | crates.io |
| `epaint` | `0.36.2` | crates.io |
| `emath` | `0.36.2` | crates.io |
| `ecolor` | `0.36.2` | crates.io |
| `epaint_default_fonts` | `0.36.2` | crates.io |
| `kittest` | `0.4.0` | crates.io (external test crate, not an egui workspace member) |
| `accesskit` | `0.24.1` | crates.io (external accessibility crate) |
| `accesskit_winit` | `0.32.2` | crates.io (external accessibility crate) |
| `winit` | `0.30.13` | crates.io |
| `wgpu` | `30.0.1` | crates.io |
| `craft-ui` | `0.1.0` | `storytold/craft-libs` rev `5c874d51eeb3de70e735c8ce87a59a7dedd47675` |
| `craft-window` | `0.1.0` | `storytold/craft-libs` rev `1f805378d454dac48b5c214029d254055fdf961b` |
| `craft-pdf` | `0.1.0` | `storytold/craft-libs` rev `ca50d63b773c99f96e75c77ce5a1d21e7c9cda0d` |
| `craft-print` | `0.1.0` | `storytold/craft-libs` rev `d991bf6792f8ade99c51358bf3c1d902c8cd96b5` |
| `craft-docpath` | `0.1.0` | `storytold/craft-libs` rev `82a24b006eefa036b43c09a67fb9a3fbd17ffe63` |

`rust-version = "1.95"` and `edition = "2024"` are already the egui 0.37 MSRV/edition, so no toolchain change is
implied. Do not treat a compile time as UI performance evidence.

No final `storytold/egui`, `storytold/winit` or final craft-libs SHA is asserted here. Those revisions are created
by the fork and library stages after their own reviews, with exact full SHAs, tree hashes and clean-fetch evidence
recorded at execution time. The candidate aiongg SHAs above are comparison inputs; they lack W15P and E21b/W27
lifecycle work and must not become interim production pins. The two-commit main drift from `8906073…` to `4a7cba0…`
is unrelated DXF work; it is a rebase gate, not a reason to change the pinned review checkout now.

### 2.2 Workspace, manifests, patches and vendors

Root workspace (`Cargo.toml`):

- `resolver = "3"`, members `crates/*`, `apps/*`, `xtask`; default members include `apps/vectorcraft`,
  `apps/vectorcraft-cli` and `xtask`.
- Current workspace UI dependencies: `egui = "0.37"`, `egui_extras = { version = "0.37",
  default-features = false, features = ["svg"] }`, `eframe = { version = "0.37", default-features = false }`,
  `egui_kittest = "0.37"`.
- Current `craft-ui`, `craft-window` and `craft-keymap` pins are published craft-libs rev `a0916808…`
  (`craft-ui` with `serde` and target-gated `native-windows`; `craft-window` with `egui`); `craft-print`,
  `craft-pdf` and `craft-docpath` use the same revision.
- Root `[patch.crates-io]` maps the egui family (including `eframe` and `egui_kittest`) to `storytold/egui` rev
  `6d518fc7…`, `winit` to `storytold/winit` rev `a517b384…`, and `hayro-*`/`krilla` to craft-libs rev
  `a0916808…`. The hayro/krilla patches are unrelated to egui and must remain unless a separate comparison proves
  replacement.
- Nested workspaces: `crates/affinity/fuzz` (libfuzzer only; no egui) and `crates/plugins/example` (wasm plug-in
  example; no egui). `crates/plugins/example/Cargo.lock` is separate and must not acquire egui.
- `crates/affinity/fuzz` has no lockfile in the checkout. Do not create one merely for this migration.

Manifests that participate in the egui graph (the table records the plan-time declarations and required changes; current declarations are listed above):

| Manifest | Plan-time UI declaration | Required change |
|---|---|---|
| `crates/ui-egui/Cargo.toml` | `egui` workspace + `bytemuck`; `egui_extras`; dev `egui_kittest` `wgpu`,`snapshot` | `egui` must also enable `default_fonts` for library tests/embedders and for downstream kittest tests, which do not inherit kittest's own dev-dependency fonts; `egui_extras`/`egui_kittest` to 0.37; no feature rename assumed |
| `apps/vectorcraft/Cargo.toml` | `eframe` features `default_fonts`, `wayland`, `x11`; default features `wgpu`, `accessibility`; direct `winit 0.30` | add **`eframe/clipboard`**; keep `links` off (existing native `open_url` service); keep `system_fonts` off; retain winit feature set |
| `apps/vectorcraft-web/Cargo.toml` | wasm `eframe` features `default_fonts`, `wgpu`, `accesskit` | keep; add `clipboard` only if the chosen eframe 0.37 web path needs the feature (the web clipboard code is not gated by it, but test it); web `open_url` is handled by the web runner |
| `craft-ui` / `craft-window` pins | 0.36-era craft-libs revisions | final L2 craft-libs revision on `3832c47…` with workspace `egui = "0.37"` and the CL-2F capture helper |
| `Cargo.lock` | one coherent 0.36 graph | one coherent 0.37 graph; no second `egui`, `eframe`, `egui-winit`, `egui-wgpu`, `epaint`/`emath`/`ecolor`, or `winit`; external `kittest`/`accesskit*` are separate audited registry identities |

There is no `[patch]` in `crates/ui-egui`, apps, `xtask`, the fuzz workspace or the plug-in example. A dependency's
`[patch]` section does not propagate into the root; the root `[patch.crates-io]` must carry every fork-source
family crate and winit. The fork's own `Cargo.toml` patch of winit is **not** inherited by VectorCraft. The final
app lock is created only after D1a (complete winit), D1b (complete egui P8) and L2 (final craft-libs/CL-2F),
never against a candidate, an intermediate fork stage or a provisional craft-libs tree.

### 2.3 Targets, binaries and entry points

| Target | Path | Current role |
|---|---|---|
| `vectorcraft` binary | `apps/vectorcraft/src/main.rs` | Native eframe/wgpu app, control server, macOS menu, Linux stylus/XWayland, Windows stylus, printing |
| `vectorcraft-web` binary | `apps/vectorcraft-web/src/main.rs`, `web.rs` | wasm eframe web runner, WebGPU/WebGL2, DOM paste/drop, `window.vectorcraft` control API |
| `vectorcraft-cli` binary | `apps/vectorcraft-cli/src/main.rs`, `perf.rs` | Headless run/convert/bench/perf and MCP stdio |
| `vectorcraft-ui-egui` library | `crates/ui-egui/src/lib.rs` | All frontend state, panels, dialogs, canvas, control methods, i18n, clipboard, fonts, workers |
| `vectorcraft-mcp` library | `crates/mcp` | MCP protocol over the engine/control semantics; no direct egui dependency |
| `vectorcraft-testkit` | `crates/testkit` | Test fixtures; dev-only |
| `xtask` | `xtask/src/*` | `ci`, `wasm`, `layers`, `assets`, `brands`, `cursors`, `pdf-deps`, corpus and packaging checks |

Entry-point details that must remain true:

- `apps/vectorcraft/src/main.rs::App` implements `eframe::App::logic`, `ui`, `raw_input_hook`, `on_exit`; `logic_frame`
  calls `VectorcraftApp::logic`, tracks window geometry/maximize, closes the window, and handles graphics loss.
- `apps/vectorcraft-web/src/web.rs::WebShell` implements the same trait; `logic` also handles pasted/dropped files and
  `api::poll_replies`; `ui` calls `app.ui` and polls replies.
- `crates/ui-egui/src/lib.rs::VectorcraftApp::logic` currently does **both** background polling and input-dependent
  work (`shortcuts::handle` at the end, `take_dropped_files`, native-menu key processing). This is the central
  lifecycle defect to fix.
- `crates/ui-egui/src/lib.rs::VectorcraftApp::ui` guards a panic and calls `ui_frame`, which draws app bar, control
  bar, status/hint bars, toolbar, dock, `CentralPanel` + canvas, floating panels, dialogs, progress, palette,
  titlebar resize zones, fonts, scrub end, native-menu sync and clipboard format cleanup.
- `crates/ui-egui/src/control.rs` owns the JSON control methods; `apps/vectorcraft/src/control_server.rs` owns the
  loopback JSON-lines transport; `apps/vectorcraft-web/src/api.rs` exposes the same methods as page promises.
- The canvas is CPU-rasterized by `vectorcraft_render` and composited by wgpu; egui only uploads the raster texture.
  UI-frame work and document rendering are separate measurements.

### 2.4 Historical 0.36.2 lifecycle (planning snapshot, not current acceptance)

The 0.36.2 baseline already had:

- `App::logic(&mut self, ctx, frame)` and `App::ui(&mut self, ui, frame)` in both hosts.
- `VectorcraftApp::logic(ctx)` called from the host logic, guarded by `vectorcraft_engine::guard::catch_panic`.
- `VectorcraftApp::ui(ui)` called from the host UI, also guarded.
- A root `Ui` in `App::ui`; panels are already shown with `egui::Panel::…::show(ui, …)`.
- Custom `Area`/`Window` dialogs and floating panels in the 0.36.2 baseline; at planning time the app had no `show_viewport_deferred`/child native viewports. The shared-docking integration now opens desktop floating panels as craft-ui OS windows, while the web and Wayland keep them in-page (see “Shared-docking integration (current)” above).
- `ctx.run_ui` in many tests; `ctx.run_ui` in test helpers draws either the whole app or one panel/dialog.

What 0.37 makes explicit:

- `eframe` calls `App::logic` **inside** `egui_ctx.run_ui`, then `App::ui`, so both are called once per layout pass,
  not once per outer frame. A `Grid` created for the first time calls `Context::request_discard`, so the same outer
  frame can run `App::logic` and `App::ui` more than once. The host calls `App::raw_input_hook` once before that
  closure, not once per pass.
- When a window is hidden/minimized/occluded, native eframe runs `Context::run_logic` instead of `run_ui`, and web
  runs `run_logic` after seeing that the tab is hidden. That leaves the last pass's `InputState` (events and time)
  in place; only viewport state is updated. Reading `ctx.input` events or `i.time` in logic-only iterations replays
  or mis-times work. Stale events must therefore be consumed only in a known UI phase.
- **`raw_input_hook` is not a UI-phase marker.** Native `EpiIntegration::update_logic_only` calls
  `prepare_raw_input()`, which calls `app.raw_input_hook()`; web `AppRunner::logic` calls
  `self.app.raw_input_hook()` before it decides visibility and calls `run_ui` or `run_logic`. Both hosts call the
  hook for full and logic-only iterations. It is useful as a host-iteration boundary, not as evidence that a UI
  pass will follow.
- `Context::run_ui` passes the existing root `Ui`; `Context::root_ui` panics if called while that `Ui` is borrowed.
  VectorCraft must not call `root_ui` from inside its `App::ui` or tests.
- `ViewportCommand::RequestPaintWhileHidden` exists for a hidden window that must draw once (a dialog, a control
  request, a screenshot). A pending `Screenshot` command already asks for paint while hidden. A forced hidden UI
  pass is still a UI phase and must consume the input it was forced to present.
- `Context::cumulative_frame_nr()` advances once when a `run_ui` completes (not per pass and not in `run_logic`);
  `Context::cumulative_pass_nr()` advances per layout pass, including a discarded rerun. Keep these distinct from
  the app's host-iteration counter in §4.2.

---

## 3. Actual 0.36 → 0.37 incompatibilities in this source

The following is a source audit of the assigned checkout, plus the historical diagnostics as a checklist. Line
numbers will drift; symbols are authoritative.

### 3.1 Compile errors

| Class | Historical diagnostic examples | Source anchors in VectorCraft | Required migration |
|---|---|---|---|
| `Event::MouseWheel` gained `source: MouseWheelSource` | `canvas.rs:6226`, `chrome.rs:1691`, `stroke.rs:634`, tests | 21 `Event::MouseWheel` literals; production matches use `..` | Add `source: MouseWheelSource::Unknown` in tests/synthetic input; production events come from egui-winit. Preserve `unit`, `phase`, `delta`, `modifiers`. |
| `WidgetType` is now a deprecated alias for `accesskit::Role`, and variants were renamed | Historical anchors `chrome.rs:640`, `dock.rs:122`, `edit_toolbar.rs:533`, `panel_docking.rs:1241`, `widgets.rs:192/1535/1536/2487`; symbols are authoritative, not these line positions | 54 `WidgetType` call sites at planning time; the current tree has none | Map per §3.2 to the renamed `Role` variants. Do not hide all roles behind `Role::Unknown`. |
| `Id` is not an `IdSalt` | `dialogs/layer_options.rs:129/130`, `menus.rs:3482/3484` | `Atom::custom(chip, …)`; `Panel::right(Id::new(...))`; `ComboBox::from_id_salt(id)` | `IdSalt::new(...)` only for a parameter typed exactly `IdSalt` (`Atom::custom`); pass the original source to `impl AsIdSalt` APIs (`Panel`, `ComboBox`, `ScrollArea`, `make_persistent_id`). `Id::unique(...)` for root ids. Never pass an `Id` as a salt. |
| `WidgetInfo.typ` removed | `dialogs/tests_warp.rs:509` | one test | Use `info.role`. |
| `scroll_area::State.offset` private | `font_menu.rs:507`, `panels/layers.rs:650`, `tests/numfields.rs:614` | 3 sites | Use the public `offset()` method. |
| `FontsView.fonts` private | `i18n/tests.rs:1606/1628` | 2 test sites | Use `definitions()`, `families()`, `characters(&family)` public API. |
| `egui::UserData` removed | `lib.rs:1032` | screenshot queue | Replace with per-request `ScreenshotCallback`. |
| `Event::Screenshot` removed | `lib.rs:1052` | screenshot collector | Replace with callback completion channels. |

### 3.2 Role mapping for VectorCraft

Map each existing semantic `WidgetType` to the equivalent AccessKit role. The mapping used by the suite's PR 199
(`Checkbox` → `CheckBox`, `SelectableLabel` → `Button`, `Other` → `Unknown`) is the starting point; the additional
VectorCraft-specific rows below are chosen from egui 0.37's own widgets and AccessKit names.

| `WidgetType` | `Role` | Notes |
|---|---|---|
| `Button` | `Role::Button` | Includes icon buttons, paint chips, toolbar tools |
| `SelectableLabel` | `Role::Button` | Preserve selected state; tabs already set `Role::Tab` manually |
| `Checkbox` | `Role::CheckBox` | Preserve mixed state (`selected: None`) |
| `RadioButton` | `Role::RadioButton` | |
| `ComboBox` | `Role::ComboBox` | Must also gain an accessible name; current `current_text_value` is a value, not a name |
| `Slider` | `Role::Slider` | |
| `CollapsingHeader` | `Role::DisclosureTriangle` | `CollapsingHeader` 0.37 uses this role; the custom VectorCraft header should match |
| `Image` | `Role::Image` | |
| `Other` | `Role::Unknown` | Only where no semantic role exists; not for an input |

The tests that assert `Role::CheckBox`, `Role::RadioButton`, `Role::Slider`, `Role::ComboBox` and `Role::Button`
already encode the expected accessibility semantics; keep them and add the missing `DisclosureTriangle` and input
names.

### 3.3 Id migration

- 294 `Id::new(...)` occurrences across `crates/ui-egui`, `apps/vectorcraft` and `apps/vectorcraft-web`
  (`rg -o 'Id::new' … | wc -l`; 283 source lines contain one or more). `Id::new` is deprecated in 0.37;
  `Id::unique` hashes the same source with the same seeds, so persisted temp-memory keys keep their identity.
  Replace mechanically, then prove UI state/panel/selection tests.
- 100 `ui.id()` calls: rename to `Ui::scope_id()`. `UiBuilder` has no `.id(...)` sites in this checkout, but if one
  appears, it is `scope_id`.
- Distinguish the two APIs:
  - `impl AsIdSalt` parameters (`Panel::left/right/top/bottom`, `ComboBox::from_id_salt`, `ScrollArea::id_salt`,
    `CollapsingHeader::id_salt`, `Ui::make_persistent_id`, `Id::with`) hash their argument themselves. Pass the
    original source (string/tuple), never an `Id` and never a pre-built `IdSalt`.
  - Exact `IdSalt` parameters (`Atom::custom`, `Id::with_salt`) take `IdSalt::new(source)`.
- `Id` still satisfies the blanket `AsIdSalt` impl (`Hash + Debug`), so some wrong calls compile while hashing an
  `Id` as a salt. That is the wrong identity even when it compiles; `Atom::custom` rejects `Id` outright.
- Historical planning (pre-#4135): dynamic dock columns in the then-existing app `floating.rs` used `Panel::right(Id::new(("dock-column", first)))` and were to become `Panel::right(("dock-column", first))`. The app `floating` module is now deleted; craft-ui's shared docking owns dock columns and their layout in `panel_docking.rs`.
- Do not mechanically wrap an existing `Id` in `IdSalt::new(...)` just to satisfy a signature. Audit the few
  `make_persistent_id`/`with`/`Atom::custom` sites individually and preserve the intended key with a focus/state
  test. `ComboBox::from_id_salt` in `widgets::combo`, `Panel::right` in the deleted `floating.rs` (historical), and `Atom::custom` in `dialogs/layer_options`/`menus` were the concrete sites to review.
- `Id::NULL` and `Id::with(...)` remain valid.
- `Id::unique` also registers debug id sources; do not let debug-only memory growth become a concern.

### 3.4 Panel identity/persistence (historical planning; superseded by shared docking)

> **Historical planning (pre-#4135; not current code).** The text below describes the old app-owned `PanelState` width path and is retained as planning evidence. The current tree has none of `seed_dock_panel_width`, `note_dock_width`, `apply_pending_dock_width`, `legacy_dock_default_width`, `shared_dock_default_width` or `dock_width_pending`. Width now persists through `UiState::dock_width`: `panel_docking::main_width` clamps it into the default layout, `default_flags`/`Transaction::commit` writes the resized width back, `workspaces::Workspace` captures and applies it (`saved_workspaces_restore_dock_width`), and `layout_migration` carries it across older files. The layout itself is craft-ui `DockArea`/`craft_ui::docking::Layout`.

`crates/ui-egui/src/dock.rs::seed_dock_panel_width` and `note_dock_width` read/write
`egui::containers::panel::PanelState` under `Id::new(panel)`. In 0.37 `Panel` internally derives its bar id from
the parent `Ui`'s `scope_id` plus an `IdSalt`; the old global `Id::new(panel)` is no longer that key. The dock width
would silently stop restoring.

Do not copy the old global key. Refactor the app-owned width:

- have `apply_pending_dock_width` set `UiState::dock_width` (clamped) before the panel is shown;
- use `Panel::default_size(legacy_dock_default_width(&app.ui))` / `shared_dock_default_width` for restore;
- after `.show(...)`, take `InnerResponse.response.rect.width()` and store it in `UiState::dock_width`;
- keep `UiState` serde and the existing workspace/preference format unchanged;
- if an in-session preference change must resize an already-laid-out panel, seed the public `PanelState`
  under the panel's public parent-scope id, `ui.scope_id().with(panel)` (the same rule `Panel` uses for its bar
  id in 0.37: `scope_id.with_salt(IdSalt::new(panel))`), rather than the old `Id::new(panel)`. Encapsulate that in
  one helper with a regression test; do not reach into a private field or hard-code a hash;
- test save → new app/context → reopen with the same `UiState` and assert the dock width and layout; also test an
  in-session width preference change.

This removes the old global-id dependency while preserving the current immediate-resize behavior through public
egui types.

### 3.5 Screenshot path

Current:

- `crates/ui-egui/src/lib.rs::drain_control` turns `control::Outcome::Screenshot` into a queued token and a
  `PendingScreenshot` with path/data/reply/deadline.
- `issue_screenshots` sends `ViewportCommand::Screenshot(UserData::new(token))`.
- `collect_screenshots` scans `ctx.input(|i| i.raw.events)` for `Event::Screenshot { user_data, image, .. }`, maps
  the token, and writes the PNG through `control::save_screenshot`.
- `PendingScreenshot` times out after settle + 8 s with a user-facing "no frame presented" error.

0.37:

- `ViewportCommand::Screenshot(ScreenshotCallback)`; `Context::request_screenshot` is the convenience API.
- `ScreenshotCallback` stores a one-shot `FnOnce(Arc<ColorImage>) + Send + 'static`; clones share it; completion is
  at most once; it does **not** request a repaint.
- `Event::Screenshot` and `UserData` are gone.
- A pending screenshot asks the integration to paint even while hidden (the command's documentation says so).

Required design (shared weak completion slot, per the suite capture contract):

- `PendingScreenshot` owns `Arc<CompletionSlot>`, where the slot holds the optional `Arc<ColorImage>`, a strong `Context`/owning-viewport wake handle
  for the owning-viewport wakeup, and a live/canceled state. The request record owns the strong slot; the app keeps
  request id, path, data flag, reply sender, settle policy and deadline. Correlation comes from ownership of that
  slot, not a second token map or callback order.
- `issue_screenshots` creates `ScreenshotCallback::new` with a closure that captures **only a `Weak<CompletionSlot>`**
  and the callback receives pixels only. The closure stores the image in the live slot, releases the slot lock, then
  wakes the owning viewport through the strong `Context`/owning-viewport wake handle the live slot owns; the detached callback never strongly captures `Context`. That
  slot-owned wake is the only scheduling: the callback itself has no implicit wake and requests no repaint. No PNG
  encoding, file write, command, `Services` access or long lock on the callback thread. A callback that fires after
  timeout/close finds the slot canceled and drops the image.
- Keep `collect_screenshots` on the UI thread: take the image once, verify the request is still live and its
  geometry epoch is unchanged, then write through `control::save_screenshot` under `file_access::confine`. A late
  or stale result writes nothing and sends no second reply.
- Serialize at most one outstanding capture at a time (the current code can issue several due commands in one
  frame; a one-shot callback makes correlation easy, but serializing avoids integration differences and bounds
  readback memory). Keep the settle-frame policy and the ~8 s deadline.
- **Admission is a pre-issue reservation, not just a post-hoc cap.** Use the shared defaults: **4 pending
  requests, 128 MiB per image and 256 MiB aggregate** reserved in-flight/completed bytes, calculated with checked
  `width × height × 4` arithmetic, never evicting an earlier request. A stricter explicit budget is allowed only
  when a documented intended surface fits it (CADCraft's smaller budget is the example); 4/128/256 MiB admits a
  3840×2160 logical viewport at 2× (≈126.6 MiB) and must not be raised. Reject excess admission with a control
  error before issuing, release the reservation on drain/timeout/cancel, and account for renderer/readback transient
  allocation separately in peak RSS.
- **Bind a geometry epoch at issuance.** Record the target UI's known logical content and screen bounds, viewport
  id, `pixels_per_point` and a surface/geometry epoch. Before saving, reject or retry on **any** resize, DPI, zoom or
  surface-epoch change since issuance, including a change back to the original values. Do not assume Wayland
  `inner_rect` exists or that callback arrival supplies capture metadata; with an unchanged epoch derive the pixel
  scale from the returned image dimensions and verify aspect and rounding.
- **Hidden issuance needs a paint request.** The `Screenshot` command itself forces a paint while hidden, but a
  request queued in app logic and not yet issued — including every remaining settling frame — must send
  `ViewportCommand::RequestPaintWhileHidden` from logic so a UI pass reaches issuance. A bare `request_repaint()`
  only runs hidden logic, and callback arrival does not wake by itself.
- This app is one of the eleven existing craft-ui consumers, so it adopts the already-issued shared `craft-ui`
  completion-slot helper from L2 (CL-1 + CL-2F). VectorCraft keeps no app-local or provisional capture slot: the
  API must be frozen before the atomic compatibility PR lands.
- Keep the existing response schema: `{ok, result: {path?, width, height, pngBase64?}}` and the explanatory timeout
  error. The inline `pngBase64` copy is capped at 64 MiB; when the request named a `path`, write that file and omit
  only the oversized inline copy (a path-less `data: true` request fails before encoding). `docs/control-protocol.md`
  documents this downgrade. It says a minimized/covered window fails after ~8 s; 0.37 may now paint a hidden window,
  so re-test and update the doc only with evidence.

### 3.6 Clipboard, PRIMARY and paste images

0.37 adds:

- `egui::Event::PasteImage(Arc<ColorImage>)` (clipboard image when there is no usable text);
- `egui::Event::MiddleClickPaste { pos, text }` for X11/Wayland PRIMARY;
- eframe's `clipboard` feature is **opt-in**; the app currently sets `eframe` `default-features = false` and does
  not enable it. `egui-winit` then falls back to an in-app string clipboard for egui's own copy/paste path (the
  `Event::Copy/Cut/Paste` events still occur, but `Event::Paste` text and text-field paste read the fallback, not
  the OS clipboard). VectorCraft's arboard `SystemClipboard` is a separate, independent rich-format owner and is not
  broken by the feature; the egui-winit text path is what needs the feature;
- `egui-winit` uses `ScreenshotCallback`-independent clipboard code and can fall back from smithay to arboard;
- the fork's E4 uses winit's Wayland clipboard instead of smithay, while VectorCraft also uses `arboard` with
  `wayland-data-control` and `clipboard-win`/NSPasteboard on other platforms.

Required design:

- Add `eframe/clipboard` to the native eframe features (unconditionally). Keep the app's `SystemClipboard` as the multi-format owner for Copy/Cut (SVG/PDF/PNG/text)
  and for image/PDF/EPS/SVG paste.
- Consume `Event::PasteImage` in `shortcuts::handle` rather than letting it reach a `TextEdit` or the Type tool:
  - with a `system_clipboard`, treat it as the textless paste signal and let `adopt_system_clipboard` read the
    formats (do not paste twice);
  - without one (web/simple host), convert the image to a PNG `Flavour` and invoke the paste command;
  - test both a text+image clipboard and an image-only clipboard.
- Consume/handle `Event::MiddleClickPaste`: focused `TextEdit`s get it from egui; for the Type tool, either insert
  at the pointer as a normal paste or explicitly ignore it. Do not let it reach `route_type_input` as text, and do
  not treat it as a shortcut. Preserve app-internal clipboard precedence.
- **Wayland PRIMARY is the selected winit-owned W15P path, not a choose-one experiment.** W15P adds per-seat SCTK
  `PrimarySelectionManagerState`/device/source/offer support on the existing Wayland connection and event queue;
  E4/P2b routes egui's get/set-primary through it. Do not bind a second regular `wl_data_device`. Keep the app's
  `arboard` rich-format path for regular clipboard Copy/Cut where the app owns it, preserve password selection
  exclusion and no-unchanged-selection reclaim, and do not silently disable the app's multi-format clipboard.
- W15P keeps distinct per-seat keyboard-focus and last keyboard/pointer triggering serials with origin and epoch.
  A selection claim validates the fresh serial and focused seat, reserves source bytes, creates the source and
  calls `set_selection` **synchronously in that same input turn** — keyboard or pointer trigger, never an arbitrary
  first-seat fallback and never queued behind I/O. Reads and serving writes are bounded asynchronous work: 2 s total
  deadline from admission, 16 MiB byte cap, 4 in-flight reads, 2 writer threads, queue depth 8, cancellation on
  owner/seat/focus/request-generation change, and an owner-specific completion wake released before waking.
  Egui E4 consumes a completion only for the still-live paste request/owner/input generation; focus changes cancel
  it.
- Keep `write_formats_before_text_input` and `drop_formats_after_egui_copy`: they encode the app rule that a text
  field/Type tool paste must not overwrite the app's pending rich formats.
- Test: Cmd+C on art writes text/SVG/PDF/PNG (per preferences); Cmd+V pastes the rich formats; Cmd+C in a text
  field replaces the clipboard with text and cancels pending app formats; image-only clipboard paste; PRIMARY
  middle-click in a field and over the canvas. Under wlroots/KWin/Mutter, test keyboard and pointer focus with
  fresh triggering serials per seat/origin, owner/seat changes, empty/absent offers, self-paste, slow/canceled
  transfers and responsiveness during a trickle.

### 3.7 craft-libs PR 199 and #198

PR 199 is a source of selected port hunks, not a mergeable branch or tree. Its head `b68de8c` is stale relative
to the `3832c47…` base: it lacks current `craft-platform` and reverts security, settings, docking and printer fixes. The
historical audit tree `ec8840a` is likewise unsafe. Never merge either tree:

- Its API commit `8614e4a` changes workspace `egui` to 0.37, updates `craft-ui` dev dependencies to
  `egui_extras`/`egui_kittest` 0.37, maps `WidgetType` to `Role`, replaces `UiBuilder::id`/`Ui::id` with
  `scope_id`, and replaces `Id::new` with `Id::unique` in tests. That is the reusable shape.
- The README/`scope_id` documentation change is `1764bca`; apply it separately after review.
- The local `audit/main-plus-199` merge is based on an older tree and, as resolved there, **drops current security
  checks**:
  - `crates/ui/src/docking/model.rs`: `Action::MoveFloating` no longer checks that every member of the floating
    group is movable;
  - `crates/ui/src/docking/surface.rs`: resizing a floating group no longer requires all members movable.
  - These checks are present in current craft-libs security work (for example #236, docking group move permission). Do not merge that tree.
- Keep the egui-related `8614e4a`/`1764bca` shape already in the `3832c47…` base and the CL-2F capture helper
  (later published in `25da131…`), preserving the permission checks, `craft-platform`, settings, docking, printing
  and tests. Record the new commit, its parent and the diff. Publish a new immutable final craft-libs revision only
  after the shared tests and an adversarial review pass.
  That final revision comes **after** the final reviewed storytold egui/winit revisions (which include E21b and
  W15P), so no provisional craft-libs commit pretends to be final.
- The final craft-libs revision also carries the CL-2F frozen capture-slot helper; VectorCraft's final app lock
  depends on that revision, not on a provisional prototype.
- `craft-window` has no source change for 0.37, but it must be consumed from a craft-libs revision whose workspace
  `egui` is 0.37; otherwise its `egui = "0.36"` requirement cannot be satisfied by a 0.37 patch.
- `craft-print`, `craft-pdf` and `craft-docpath` do not depend on egui; keep their current revisions unless a
  separate comparison justifies a bump. Do not churn unrelated pins.
- #198 is a language-support proposal, not a merged dependency. VectorCraft has its own i18n catalog and no RTL UI
  language (the catalog list has no Arabic/Hebrew/Persian). The fork's bidi/RTL UI patches still affect text
  layout, so they need visual tests, but VectorCraft should not start calling fork-only `set_ui_language`/
  `LayoutJob::direction` APIs in this migration. `ImeEvent::DeleteSurroundingBytes` is the one fork-specific event
  the canvas must handle; document that dependency and keep a stock-compatible alternative in the plan.

### 3.8 eframe features and fonts

- `egui` 0.37's `default = []`; `default_fonts` is opt-in. `FontDefinitions::default()` is empty without it.
  `crates/ui-egui/src/theme.rs::install_fonts` must keep working for library-only builds and for
  `--no-default-features`; enable `egui/default_fonts` on the `ui-egui` egui dependency (or explicitly bind every
  fallback family, which is more fragile).
- **Corrected font mechanism:** production layout uses the default `MissingGlyphPolicy::Tofu`
  (`epaint/src/text/fonts.rs`), so an unbound family draws a synthetic box, not a panic. The kittest harness alone
  defaults to `Panic` (`egui_kittest/src/builder.rs`). `install_fonts.rs` is a plain `run_ui` test, so mutation 8 fails
  its non-empty-family assertion rather than panicking. `theme.rs`'s "egui panics…" doc comment is scheduled for
  correction with that change.
- `egui_kittest`'s own `[dev-dependencies]` enable `default_fonts` and `monochrome_emoji_fonts`, but a dependency's
  dev-dependencies do not propagate: VectorCraft's kittest tests get bundled fonts only from its own dependency
  features. Inspect the actual graph per test root instead of assuming inherited fonts.
- `eframe` 0.37 `default_fonts` is still a feature; the app/web already request it.
- eframe 0.37 `system_fonts` (fontique/egui_system_font_provider) is a new default in eframe's default feature
  set. VectorCraft disables eframe defaults, so it stays off. Keep it off initially: the app owns font discovery
  (`vectorcraft_text::FontDb`, `apps/vectorcraft/src/system_fonts.rs`, `mac_fonts.rs`, web `fonts.rs`) and must
  build/test with `CRAFT_FONTS` empty. Evaluate #8512 startup cost only if a measured case justifies enabling it.
- `eframe` 0.37 `links` is a feature; VectorCraft's native app installs its own `open_url` service
  (`webbrowser`), and the web runner handles `OutputCommand::OpenUrl`. Keep `links` off as before; add it only if a
  native path actually needs it.
- `eframe` 0.37 `clipboard` is new and required (above). `eframe` 0.37 `wayland`/`x11` remain required on Linux/CI.
- `egui_extras` 0.37 still exposes `install_image_loaders` at the crate root; `craft-ui` SVG icons use
  `ImageSource::Bytes` and the SVG loader. Test the SVG loader with the app's icon set.
- **Font fixtures follow the shared pin.** Use the exact `craft-fonts` revision
  `2481a8a324b986a45954e90045ed1f8f2efaaf3c` and the identities in `egui` plan §10.6, with recorded licences:
  Arabic `fonts/noto-sans-arabic/NotoSansArabic.ttf` SHA256
  `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`; Thai
  `fonts/noto-sans-thai/NotoSansThai.ttf` `5a1c559bb539583c8a1fd99d1c5b9491e5e14478c9cd2bd0970d5c3096cc9ef8`;
  CJK SC `fonts/noto-sans-cjk-sc/NotoSansCJKsc-Regular.otf`
  `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`; Japanese
  `fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf`
  `258d7156c165f2ff774b6efee637c22c3b950de0d8a10e501137061bc8085d01`; Noto Sans Hebrew variable
  `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88` (fix axes to weight 400 / width 100);
  Noto Color Emoji v2.047 `39ee3c587e10e89669b9ff32703261d10d5f9c4dd5ad147b6b5a1c5200591817` (CBDT, not a
  COLR/SBIX proof). Candidate `epaint_default_fonts` comparison bytes are Ubuntu-Light
  `80307b8da7649aa4ee4d484b232140e3ce1ec0ca093073d3c53c8f5a5ced7a70` and NotoEmoji-Regular
  `415dc6290378574135b64c808dc640c1df7531973290c4970c51fdeb849cb0c5`; use the same bytes for paired
  measurements and preserve the adjacent licence files. Fetch these as test resources into the task evidence
  directory; do not copy font binaries into this repository, and do not replace them with system fonts. Keep
  VectorCraft's own central-font policy (`vectorcraft_text::FontDb`, `CRAFT_FONTS_DIR`, interface-fonts /
  bundled-fonts) and the `CRAFT_FONTS` empty build/test requirement. A missing glyph fails the strict test unless a
  deliberate `allow_missing_glyphs` case is documented.

### 3.9 IME, text and RTL

- Upstream 0.37 adds `ImeEvent::DeleteSurrounding { before_chars, after_chars }`; the selected fork adds
  `ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }` (E21) and the winit fork supplies it for
  macOS press-and-hold/dictation and Wayland text-input-v3.
- `crates/ui-egui/src/shortcuts.rs::type_text` matches `Preedit`/`Commit` and deliberately leaves everything else
  in the event list. `crates/ui-egui/src/canvas.rs` and `vectorcraft_engine`'s `tool_text`/`tool_preedit`/`tool_key`
  have no “delete surrounding characters/bytes” operation. A system IME that replaces text around the cursor will
  therefore not edit the document as intended.
- Design a single canvas insertion/deletion API in the engine/tool layer: delete `before_chars`/`after_chars`
  Unicode scalar values (upstream) or the exact UTF-8 bytes supplied by the fork, in the same undo step as the
  commit, then insert the committed text. Never split a UTF-8 sequence; never use byte indices derived from
  character counts. The app's `ime_marked`/`ime_discard`/`keep_marked_text` state must remain coherent, and a
  `Commit` after a replacement is one undo step.
- **E21b/P7b outgoing surrounding text is a required prerequisite, not a nice-to-have.** The candidate
  `egui-winit` never calls W27 `set_ime_surrounding_text`, and candidate `IMEOutput` has no logical text. The final
  fork must add a bounded transient owner/context-tagged payload to `IMEOutput` (`Available` including an empty
  document, distinct from `Unavailable`), a built-in `TextEdit` producer, and an `egui-winit` Wayland adapter.
  The payload carries logical committed text and UTF-8 byte cursor/anchor indices, excludes preedit, remaps indices
  when preedit is removed, caps the excerpt at 4,000 bytes without a full-document clone, and returns explicit
  unavailable context when the selection cannot fit. Password fields emit no text. The latest completed pass,
  including `IMEOutput = None`, is authoritative; do not retain a previous payload with `ime.or(self.ime)`.
- VectorCraft's custom canvas producer (the `IMEOutput` construction in `crates/ui-egui/src/canvas.rs`, around the
  Type-tool text edit) must be a real model/preedit/byte-index adapter: it supplies the document's logical text,
  not display-bidi text, converts its scalar/grapheme cursor and anchor to UTF-8 byte offsets, excludes the active
  preedit, and clears on focus/password/unavailable transitions. Setting the new field to `None` everywhere is not
  acceptance, and the adapter belongs to the atomic E21b consumer work.
- W27 must gain the explicit owner/context transition and clear operation: cache and protocol state are cleared at
  owner/unavailable changes using the committed disable/enable boundary; old preedit/commit/delete batches are
  canceled; ordinary same-owner updates preserve composition. A current-epoch `done` with a mismatched serial
  applies incoming edits once while outgoing state waits for a matching serial; an **old-epoch `done` is rejected,
  invalidates that epoch and can never edit the new owner**. `Available` with empty text is distinct from
  `Unavailable`, and the latest completed pass, including `None`, is authoritative. Tests cover outgoing text after
  delete+commit and deletion-only batches, UTF-8 boundaries, preedit exclusion, long selections, focus/late serials,
  old-epoch rejection, two windows, password clearing and multi-pass final output. The complete contract is egui
  plan §E21b/P7b; VectorCraft consumes it rather than restating a weaker variant.
- `Shortcuts::type_text` must consume the new events (so `route_type_input` does not see them), and tests must cover
  preedit → replacement → commit, preedit → cancel, deletion-only batches, and focus/serial changes.
- `ImeEvent::Enabled`/`Disabled` are deprecated; do not add matches that depend on them.
- `Event::PasteImage`/`MiddleClickPaste` are not IME; keep them out of the IME sequence.
- VectorCraft's UI languages are LTR; document text can be RTL and uses `vectorcraft_text`'s own bidi/shaping, not
  egui's `LayoutJob`. UI text containing RTL layer names must still lay out and hit-test correctly through the
  fork's E5/E12 changes. Add RTL-name UI tests; do not claim a full RTL UI.

### 3.10 Accessibility and test harness

- `egui_kittest` 0.37 fails an input widget with no accessible name; `kittest` 0.5 is a new version.
- `MissingGlyphPolicy::Panic` is the **kittest harness** default; `HarnessBuilder::allow_missing_glyphs()` opts out.
  Production layout keeps the default `Tofu` policy and draws a synthetic box, so a missing glyph is a visible tofu,
  not a crash: tests must assert glyph presence, not rely on a production panic.
- Current VectorCraft custom fields paint labels beside `TextEdit`s (`widgets::recessed_text`, `dialogs/form.rs`,
  panel renames, search fields, hex fields, workspace names, canvas name edit). They do not name the input.
- Current custom combo boxes (`widgets::searchable_combo`, `combo`) set `current_text_value`/`Role::ComboBox` but no
  label.
- Current custom check/radio/slider widgets already label most controls; audit the rest.
- `Role::SelectableLabel` becomes `Role::Button`; existing tests already expect `Button` for selected tabs, so the
  semantic intent survives. `CollapsingHeader` becomes `DisclosureTriangle` and needs a test.

Required design:

- Add a small app-local naming helper around `Ui::name_inputs` and `Ui::label_inputs_by`; do not build a generic
  compatibility layer.
- `dialogs/form.rs::row_label` already calls `ui.label(egui::RichText::new(tl!(label)).color(t.text_dim))`; it is a
  real `Label` that currently returns `()`. Do **not** add a second label or repaint the text. Change `row_label` to
  return that existing `Response` (or its `Id`), then wrap `field_w`/`length_field`/form-grid value widgets in
  `ui.label_inputs_by(label.id, |ui| …)`. Keep the token colour and translated text unchanged.
- `widgets.rs::label_row` is the custom-painted case: it allocates a hover rect and paints text. Give it a real
  labelled response and apply `label_inputs_by` there separately; do not confuse it with `row_label`.
- In `widgets.rs`, give `recessed_text`/`text_field`/`exact_text_field`/`num_field`/`size_field` an accessible name
  from the row (or a required `name` parameter at call sites that are not rows).
- Name every standalone `TextEdit`: layer/artboard/asset rename, pattern name, color hex, graph data cells,
  workspace name, preference fields, shortcut search, print/PDF ranges, password fields, canvas text/name edit.
  Use translated UI strings (`tl!`) where the user sees them; do not use internal ids as names.
- Name combo boxes with the field they choose (font family, layer colour, etc.), not the selected value.
- Keep `WidgetInfo::selected` for check/radio/tab selected state and `WidgetInfo::slider` for sliders.
- Run all kittest tests with the strict name check on. `allow_missing_glyphs` only in a test that deliberately
  renders a glyph outside the fixture fonts; document it.
- Add semantic tests for hidden controls disappearing (0.37 hides invisible widgets), disabled state, labels,
  values, selected/mixed state, and keyboard activation of checkbox/radio/tab/menu items.

### 3.11 Other 0.37 behaviour changes to watch

Source-grounded from the 0.37 changelog and code, with tests rather than assumptions:

- `Grid` calls `request_discard` on first show; the host `App::logic`/`App::ui` closures run twice on that frame.
  Prove no command/undo duplicate (the second pass receives `new_input.take()` with no events, but test it); the
  app's `tick` remains once per host iteration.
- popup/menu/modal/panel accessibility trees changed; hidden widgets are removed from the tree.
- `TextEdit` layout/margin and drag-to-select changes; click-away selection (#8763/#8764) can affect
  `select_all_on_focus`, field commit, and rename tests. **P7c owns this upstream defect and its fix**: it reproduces
  the selection-action loss and adds pointer and keyboard regressions. #8764 is an unmerged proposal, not release
  behavior. VectorCraft must not silently accept changed click-away/field-commit behavior as 0.37 semantics; any
  app workaround needs equivalent tests and a removal condition, and an unresolved failure blocks release.
- `ScrollArea` clip/scrollbar/rubber-banding changes; panel scroll geometry and wheel tests need review, not
  tolerance widening.
- `ComboBox` is built from atoms and takes `IntoAtoms`; custom `searchable_combo` is separate but built-in combos
  may shift.
- `Ui::id`/`UiBuilder::id` rename; `scope_id` is stable.
- `Panel`/`Scene` clip rect respects parent (#8562), panel separator width changes; dock/canvas geometry tests.
- `Context::set_fonts`/`add_font` apply immediately when no text has been laid out; `ui_fonts` tests may observe
  fonts one frame earlier.
- default style tweaks (roomier buttons, larger corners); app sets most tokens but screenshots will move.
- text tessellation (#8731) and DX12 FXC shader compile (#8736) are upstream reports, not reproduced here. Measure
  our own fixtures before changing anything; preserve Repeat/MirroredRepeat semantics and color emoji.
- #8748 (hidden-parent deferred children) concerns child/deferred viewports. P7c owns the hidden-parent/child fix
  and its regression. VectorCraft has no child viewport, so it verifies only the root window's
  occluded/minimized logic-only path and that any `Area`/`Window` owned by the root does not assume a parent
  viewport that is still painted; it does not add a deferral workaround.
- `eframe` web file handle is opt-in for `DroppedFile::web_file`; eframe web enables it, but the app uses
  `bytes_async`, so no source change unless we choose the handle.
- `wgpu` remains 30.0.x; VectorCraft's lock has 30.0.1 while upstream 0.37 pins 30.0.0 for wasm-bindgen reasons.
  Check the resolved graph and wasm build rather than assuming a version match.

---

## 4. Migration design

### 4.1 Dependency graph and patch policy

The dependency direction is fixed:

```
D1a  approved storytold/winit: W1–W15 minus W5, plus W17–W27 including replacement W20, W15P, bounded transfer
D1b  approved storytold/egui P8: P2b + P7 + P7b/E21b + P7c, complete
        -> L1  craft-libs CL-1 API port on 3832c47 (PR199 already applied) + CL-2F helper
        -> L2  craft-libs CL-2F capture-helper freeze
        -> V2  VectorCraft atomic app lock
```

No stage depends on itself and no app lock precedes L2. No intermediate fork stage, aiongg candidate or provisional
craft-libs tree becomes a production pin. Candidate aiongg revisions are research inputs only; final fork SHAs and
tree hashes are recorded when the approved predecessors exist, never invented here.

One coherent 0.37 family:

```
eframe 0.37 ─┬─ egui 0.37 ── epaint/emath/ecolor/epaint_default_fonts 0.37
             ├─ egui-winit 0.37 ── winit 0.30.13 (approved storytold fork)
             └─ egui-wgpu 0.37 ── wgpu 30.0.x
egui_extras 0.37
egui_kittest 0.37 ── kittest 0.5.0 (registry)
craft-ui (final craft-libs rev) ── egui 0.37
craft-window (final craft-libs rev) ── egui 0.37
```

- **Required-used fork-source IDs** (each must appear once from the D1b revision): `egui`, `eframe`, `egui-winit`,
  `egui-wgpu`, `egui_extras`, `epaint`, `emath`, `ecolor`,
  `epaint_default_fonts`, `egui_kittest`.
- **Optional or forbidden-if-present IDs:** `egui_demo_lib`, `egui_inspection`, `egui_system_font_provider` (only
  with the disabled `system_fonts` feature) and `egui_demo_app` (workspace-only binary). Absence is a pass; if one
  appears it must resolve from the same D1b revision. `egui_plot`/`egui_dock` are not workspace crates and are
  pinned separately only if a consumer actually uses them; do not assume a 0.37 version number.
- **External registry identities are audited separately, not required to come from egui git:** `kittest`, `wgpu`,
  `accesskit`, `accesskit_winit`, `accesskit_consumer` and the transitive `accesskit_ios`, `accesskit_macos`,
  `accesskit_windows`, `accesskit_unix`, `accesskit_atspi_common`. The 0.37.0 lock resolves `kittest 0.5.0`,
  `accesskit 0.25.1`, `accesskit_consumer 0.39.1`, `accesskit_winit 0.34.1`, `accesskit_atspi_common 0.21.0`,
  `accesskit_unix 0.24.0`, `accesskit_macos 0.27.1`, `accesskit_windows 0.35.1` and `accesskit_ios 0.2.1`; the
  workspace manifest's requirement strings are lower (`accesskit 0.25.0`, `accesskit_consumer 0.39.0`,
  `accesskit_winit 0.34.0`). At the pre-migration baseline VectorCraft locked `kittest 0.4.0`, `accesskit 0.24.1`,
  `accesskit_winit 0.32.2`; the current pins are in the status block above. The gate checks resolved registry source,
  compatible version and a single identity; it
  must **not** reject them for not being egui-git packages.
- Keep version requirements at `0.37` in `[workspace.dependencies]`, and add root `[patch.crates-io]` entries for
  every fork-source crate used **and** `winit` to the final immutable revisions. This makes transitive
  dependencies from `craft-ui`/`craft-window` resolve to the same source. A dependency's `[patch]` section is
  ignored by the consumer.
- Do not use a branch or a machine-local path in publication. The winit fork's own patch is not inherited.
- Keep the existing hayro/krilla root patches and PDF fixes. These are unrelated to the egui graph and must not be removed, folded into the fork pin, or
  blanket-deleted with PdfCraft's vendors. Do not add `vendor` copies of egui/winit for VectorCraft.
- Add `cargo xtask ui-deps` (modelled on `xtask/src/pdf_deps.rs`) that runs `cargo metadata --locked` and checks a
  canonical ledger of every package ID and source: required-used fork IDs, optional IDs with an `absent` or
  `same-revision-if-present` rule, the external registry identities above, and the final `winit` rev. It rejects a
  registry fork-family copy, a branch/source mismatch, a **missing required** package, an optional package from an
  unexpected source, a duplicate `egui`/`eframe`/`egui-winit`/`egui-wgpu`/`winit`/`epaint`/`emath`/`ecolor`, an old
  `craft-ui`/`craft-window`, an unexpected version, or a `path`/`file` source. A version string alone must not pass.
  Add negative fixtures for a registry `epaint`, a registry `egui_extras` beside the git copy, a mixed family graph,
  wrong external `kittest`/`accesskit*` versions, a missing family patch and an unapproved moving branch. Run it in
  `cargo xtask ci`.
- `cargo tree -d`/`cargo tree -i egui` must show one egui, one epaint family and one winit, plus one compatible
  external test/accessibility identity. Duplicate `skrifa`/`harfrust`/`glifo` versions are expected from epaint
  0.37 vs `vectorcraft-text`; record them and measure binary size. Do not force a text-engine downgrade without
  evidence.
- VectorCraft keeps ownership of its document model, engine commands, undo stack, units, themes, font database,
  workspace format and persistence. `craft-ui`/`craft-window` remain shared widget/window helpers; this migration
  does not move app semantics into them.

**Publication reachability:** a clean checkout with no cache must `cargo fetch --locked` the chosen revisions;
`--locked` builds must not update the lock. Recheck the remote commit/tree before merge. If a fork revision or
craft-libs revision is unavailable, stop and report; do not substitute a branch.

### 4.2 Lifecycle: host iteration, input and UI

Split the current `logic` into an explicit, testable boundary. Proposed surface in
`crates/ui-egui/src/lib.rs`:

```text
VectorcraftApp::begin_host_iteration()  // called from both hosts' raw_input_hook; root viewport only
VectorcraftApp::tick(ctx)               // once per host iteration, full or logic-only; no raw input
VectorcraftApp::ui(ui)                  // first UI pass in an iteration consumes input; later passes draw only
```

Hosts:

- `apps/vectorcraft/src/main.rs::App::raw_input_hook` calls `self.app.begin_host_iteration()` before injecting host
  input and synthetic events; `App::logic` calls `self.app.tick(ctx)`.
- `apps/vectorcraft/src/main.rs::App::ui` calls `self.app.ui(ui)`.
- `apps/vectorcraft-web/src/web.rs::WebShell::raw_input_hook` calls `app.begin_host_iteration()` before taking its
  pointer events; `WebShell::logic` calls `app.tick(ctx)` and keeps graphics-loss/restart and `api::poll_replies`.
- `WebShell::ui` keeps its DOM paste/drop handling and calls `app.ui`.
- **This matches the actual host order:** eframe calls `raw_input_hook` from `prepare_raw_input` in both
  `EpiIntegration::update` and `EpiIntegration::update_logic_only`; web `AppRunner::logic` calls it before checking
  visibility and before `run_ui`/`run_logic`. It is a host-iteration boundary, not a UI-phase marker. It is also
  called for a child-viewport update; VectorCraft has only the root viewport, so `begin_host_iteration` must ignore
  a non-root update if a child is ever added. It must never be used to infer that a UI pass will follow.
- Test helpers must preserve that order: `raw_input_hook`/`begin_host_iteration` once, then `run_ui` with
  `tick` + `ui` inside the closure; logic-only runs `run_logic` with `tick` only. Do not call `tick` outside the
  host-ordered closure in realistic tests.

Both `tick` and `ui` keep the existing `vectorcraft_engine::guard::catch_panic` entry guard: a bug costs one
frame, rolls the document back and shows the existing status error; it must not close the app with unsaved work.

**Counters (keep them distinct):**

- `host_iteration`: increments once in `begin_host_iteration`, for every full or logic-only host update, including
  repeated hidden iterations and forced hidden paint. It is not a frame or pass count.
- `accepted_ui_frames`: derived from `Context::cumulative_frame_nr()` for the root viewport, which advances once
  when a `run_ui` completes and never in `run_logic`. Compare/store the previous value at `tick`; count only real
  completed UI frames. Do not use `cumulative_pass_nr()` for this.
- `ui_pass_nr`: `Context::cumulative_pass_nr()` for diagnostics only; a discarded pass increments it, so it is not
  an accepted-frame counter.
- Replace `app.frame` uses that mean "outer iteration": `native_menu::state_hash` uses `host_iteration`, and
  artboard key-repeat deadlines use monotonic `now_secs` rather than a frame delta. If a compatibility `frame`
  field remains, define it as `host_iteration` and document that it also advances on logic-only iterations.

`tick` rules:

- Runs when `host_iteration != tick_iteration`, which is exactly once per host iteration: once for a multi-pass
  `run_ui` (the hook ran once before all passes), and once for each logic-only `run_logic` (the hook ran before the
  visibility decision). Do **not** gate it on `ctx.cumulative_frame_nr()` or on `current_pass_index()`: after a
  completed multi-pass frame, `current_pass_index()` is zero, and a hidden logic-only call does not restore a
  nonzero pass index.
- Runs background polls (`picks`, `background`, `opening`, `plugin_runs`, `placing`, `screen_color`, `recovery`,
  link watch, font check, control request drain, screenshot completion).
- Uses **monotonic host time** (`now_secs`, see below), never `ctx.input(|i| i.time)`.
- Reads only viewport state (`close_requested`, `focused`, `visible`) from `ctx.input`; never raw events, dropped
  files or text input. Stale events from a previous UI pass must never be consumed here.
- If tick opens a dialog, a control request injects synthetic input or otherwise changes UI, or a screenshot
  request is queued but not yet issued (including settling frames) while the viewport is not visible, send
  `ViewportCommand::RequestPaintWhileHidden`; a plain repaint runs only logic for a hidden window, not the UI that
  applies synthetic input, shows the dialog or issues the capture. An already-issued `Screenshot` command asks for
  paint itself; every other queued `ui.*` request needs the explicit command.
- Keeps `ctx.request_repaint_after` scheduling for background jobs, autosave, recovery heartbeat and pending
  screenshots. A logic-only iteration must not stop these. VectorCraft has no audio/playback; the background work
  that must survive logic-only is file open, background save/export, plug-in runs, Place reads, link checks, font
  checks, Data Recovery and control/screenshot completion.

`ui` rules:

- On the first UI pass of a host iteration (`input_iteration != host_iteration`), call `handle_input_once(ctx)`
  before drawing, inside the same `file_access::confine` guard as `ui_frame` when `synthetic_frame` is set
  (control-injected input must not escape the automation roots), then set `input_iteration = host_iteration`:
  - `write_formats_before_text_input`;
  - `native_menu::run`;
  - `shortcuts::handle`, still gated by `fonts_ready` as today (do not run shortcuts before the UI fonts are
    bound);
  - native `take_dropped_files`;
  - synthetic input is already in `raw_input_hook`; do not drain it again.
- A later pass of the same host iteration (a `request_discard` rerun) draws again but does **not** re-handle input
  or re-run commands. This is the only place stale/current input is consumed; logic-only has no `ui`, so no input
  is replayed. A forced hidden UI paint is still a UI phase and must consume the input it was forced to present.
- Draws the existing panel order. Multi-pass drawing is allowed; side effects in the drawing code must be
  event-gated or deferred, not merely assumed idempotent.
- Measure `perf.frame_ms` on the first pass but finalize frame statistics only when `cumulative_frame_nr()` shows
  the frame was accepted; do not count a discarded pass as another frame.

Scrub undo scopes need an accepted-frame boundary: begin on the first pass that sees a drag, record the release
(and cancel flag) on that accepted pass, and commit the scope once in `tick` at the next host iteration boundary in
the document where it began. Do not commit from a pass egui may discard and rerun. Test a scrub release under a
forced second pass; the result must be exactly one undo step and no empty second scope.

**Multi-pass regression:** add a test that forces `ctx.request_discard("test")` around a frame containing a click
on a toolbar/panel command and a new `Grid`, then asserts exactly one command/undo step and one `tick` for that
host iteration. If a second pass does fire a duplicate command, introduce a narrowly scoped deferred-action queue
for the affected UI action; do not add a global compatibility layer and do not set `max_passes = 1` (that would
accept a mislaid-out Grid).

**Logic-only regression:** run a full frame with a pending shortcut/click (hook once, then `run_ui` with
`tick`/`ui`), then call `run_logic(&raw, |ctx| app.tick(ctx))` several times with a fresh
`begin_host_iteration` for each. Assert no command/undo/paste is repeated, background save/recovery/opening
continues, `tick` advances once per call rather than being blocked by a stale pass index, and timers use monotonic
time. Also assert no `ctx.input` event/time read occurs in `tick` (source/static check or a test clock).

**Version-neutral test driver:** the 0.36.2 baseline has no `tick` API. Its driver must use the existing
`app.logic(ctx)` + `app.ui(ui)` inside `run_ui` and `app.logic(ctx)` inside `run_logic`, preserving the same hook
order (call the 0.36.2 host `raw_input_hook` where the trait provides it; otherwise call its iteration-boundary
helper before the pass). The candidate driver uses `tick` + `ui` inside `run_ui` and `tick` inside `run_logic`.
Record the exact lifecycle and pass count for both in the performance evidence; do not pretend the baseline had a
`tick` surface.

**Monotonic clock:** add `now_secs()` beside `now_ms` using a process-start `Instant` (native) / `web_time::Instant`
(web). Use it for `system_paste_at`, `font_install_check_at`, `tick_link_updates`, `recovery::frame`, screenshot
deadlines and background scheduling. Keep egui's `i.time` for UI animation/gesture timing only. `recovery::tick`/
`recovery::frame` already accept an explicit `now`; tests keep using that.

### 4.3 Root Ui, panels, docking, viewports

- Pass the existing `&mut Ui` from `App::ui` to panels; never call `Context::root_ui` inside `App::ui`, tests that
  use `run_ui`, or any code that already holds the root. `root_ui` is only for a `Context`-only integration, which
  VectorCraft does not have.
- Dialogs keep `egui::Area`/`Window::show(ctx)`: they create their own `Ui` and do not borrow the root. Floating panels are drawn by `craft_ui::docking::DockArea`; on the desktop craft-ui's `native-windows` feature gives them OS windows of their own, and on the web and Wayland they float inside the app window.
- Panel order remains app bar / control bar / status+hint / toolbar / dock / central canvas / floating / dialogs.
  The new `Panel` id-salt behavior must not change the order or clip; assert `response.rect`, `ui.clip_rect()` and
  hit positions after resize/DPI changes.
- VectorCraft does not call `show_viewport_deferred` itself. The only child native viewports are the desktop floating-panel windows that craft-ui's `native-windows` feature creates; the web and Wayland stay in one window. Floating windows' controls are excluded from `ui.docking`'s `hitRects` and from control-channel pointer input (`panel_docking/native_tests.rs`, `docs/control-protocol.md`). Retest screenshot/root behavior for the main viewport with the native child windows present; still test two queued screenshot requests (issued one at a time) and hidden-window paint because the callback contract is new.
- Docking/floating use `craft_ui::docking`; the app `floating` module is deleted. `panel_docking::Runtime` keeps only session-only space/raise/focus state, and `panel_docking::valid`/`known` plus `Permissions` preserve the layout model and the `MoveFloating` group permission checks.
- Test: panels open/close/reorder, dock width restore, floating panel drag/dock, popup/flyout focus, window resize
  and 1x/1.5x/2x scale; screenshots and geometry agree.

### 4.4 Screenshot completion

Implement §3.5. Details:

- `PendingScreenshot` owns `Arc<CompletionSlot>`; the slot holds `Option<Arc<ColorImage>>`, a strong `Context`/owning-viewport wake handle for the
  owning-viewport wakeup, and live/canceled state. The record keeps `path`, `data`, `reply`, `deadline`, settle
  policy and a diagnostic id. No second token map.
- `queued_screenshots` becomes a FIFO of admitted requests; issue at most one `Screenshot` command at a time.
- `issue_screenshots` creates the callback in the UI thread. The closure captures **only `Weak<CompletionSlot>`** and
  receives pixels only. It writes the image into a live slot, releases the slot lock, then wakes the owning viewport
  through the strong `Context`/owning-viewport wake handle held by the live slot. It must not capture `self`, strongly capture `Context`, wake
  implicitly, encode PNG, touch `Services`, write files or hold a lock the UI thread needs.
- `collect_screenshots` is called from `tick`; it takes each completed image once, validates it against the request's
  geometry epoch (logical/screen bounds, DPI, zoom, viewport and surface epoch; any change since issuance, including
  a change back, rejects or retries), verifies pixel scale/aspect/rounding from the image dimensions, then writes on
  the UI thread through `control::save_screenshot`, under `file_access::confine`.
- On timeout/close, remove the pending record and cancel the slot; a late callback fails the weak upgrade and the
  image drops. Do not reply twice; do not write a file after timeout.
- Reserve the checked bytes **before** issuing with the shared defaults (4 pending requests, 128 MiB per image,
  256 MiB aggregate, no eviction); a stricter limit only when the documented intended surface fits. Release the
  reservation when the record is drained, timed out or canceled, and reject a completion whose image exceeds it.
- Send `RequestPaintWhileHidden` from logic when a screenshot is queued but not yet issued, including settling
  frames; the `Screenshot` command itself covers only the issued request, and callback arrival does not wake.
- **Use the shared craft-ui completion-slot helper from L2.** VectorCraft is one of the eleven existing craft-ui
  consumers, so it adopts the CL-1 + CL-2F API before V2 and keeps only app-owned request records, paths/schemas,
  confinement and settle policy around it. There is no app-local or provisional capture slot and no generic
  compatibility map.
- Test with a fake completion channel and with a real kittest/eframe screenshot where available. Test hidden/minimized
  behavior natively; on web, do not assume a background tab paints. Include a test that dropping the request record
  or closing the window makes a queued/late callback a no-op, and that retained image bytes return to zero after
  drain/shutdown.

### 4.5 Clipboard and PRIMARY

Implement §3.6. Keep the existing `Services::system_clipboard` abstraction and `sysclip` rules. Add:

- eframe `clipboard` feature;
- `PasteImage`/`MiddleClickPaste` consumption;
- **W15P/E4 PRIMARY integration**: the winit fork's per-seat PRIMARY API on the existing connection/event queue,
  routed through E4/P2b; no second regular `wl_data_device`, no arboard PRIMARY default, no first-seat fallback.
  Password exclusion and no-unchanged-selection reclaim remain.
- Synchronous ownership claims: validate the fresh keyboard **or** pointer triggering serial and focused seat,
  reserve source bytes, create the source and call `set_selection` in the same input turn.
- Bounded asynchronous reads/serving: 2 s total deadline from admission, 16 MiB cap, 4 in-flight reads, 2 writer
  threads, queue depth 8, cancellation on owner/seat/focus/generation change and an owner-specific wake released
  before waking. A completion is delivered only to the still-live paste request with unchanged input
  owner/generation.
- tests for text+image, image-only, rich app copy, text-field copy canceling rich formats, PRIMARY and Wayland
  ownership on wlroots/KWin/Mutter (per-seat keyboard and pointer focus/serials with origin and freshness,
  owner/seat change, empty/absent offer, self-paste, slow/canceled/oversized transfer, one data device per seat).
- Do not enable eframe `system_fonts` for clipboard or font work.

### 4.6 IME, text and RTL

Implement §3.9:

- Add the engine/tool deletion operation with a clear char/byte contract; use the fork's byte variant for E21.
- Consume the event in `shortcuts::type_text`; keep one undo step; preserve `ime_marked`/`ime_discard`.
- Implement the VectorCraft canvas `IMEOutput` producer for E21b: logical model text, preedit excluded, scalar/
  grapheme cursor and anchor converted to UTF-8 byte indices, 4,000-byte bounded excerpt, owner/context generation,
  `Available` (including empty) vs `Unavailable`, and password/no-context clearing. It must not pass display-bidi
  text and must not set the payload to `None` as a placeholder. The built-in `TextEdit` producer is inherited; the
  canvas adapter is app-owned.
- Rely on the complete D1b W27 `egui-winit` adapter to send the outgoing payload and apply the egui P7b lifecycle
  rules: owner identity with `Unavailable` still carrying identity; `Available` with empty text distinct from
  `Unavailable`; the committed clear boundary on owner/unavailable/password changes; the latest completed pass,
  including `None`, authoritative; a current-epoch mismatched serial applies incoming edits once and defers outgoing
  state; an old-epoch batch is invalidated and cannot edit the new owner. Test outgoing state immediately after
  delete+commit and deletion-only batches, focus/password clearing, long selections, UTF-8 boundaries, old-epoch
  rejection, two windows and multi-pass final output.
- Test UTF-8 boundaries, multi-byte scripts, grapheme clusters, delete-only batches, commit after replacement,
  cancel, focus changes, and two windows (if native). Use the locked `unicode-bidi 0.3.18` (Unicode 16) and
  `unicode-segmentation 1.13.3` (Unicode 17) data, not “latest”.
- Test UI `TextEdit` IME separately from the canvas type tool; they use different code paths.
- Do not claim full RTL UI; test RTL names in LTR UI and document text through the existing engine tests.

### 4.7 Fonts and glyphs

- Enable `egui/default_fonts` explicitly in `crates/ui-egui`, and do not rely on kittest's own dev-dependency
  features for downstream tests.
- Keep `theme::install_fonts` as the single binding point for `FONT_UI`, `FONT_UI_SEMIBOLD`, `FONT_MONO`; keep
  `add_craft_fonts` and `UiFonts` fallback.
- Keep eframe `system_fonts` off; the app's `FontDb` and platform font modules remain responsible.
- Update `i18n/tests.rs` to the public `FontsView` API (`definitions`, `families`, `characters`).
- Update `font_menu` sample/offset code to `state.offset()`.
- Add a glyph-coverage test for the UI's own symbols (`→ ▾ ● ✓ ★ ☆ …`) and for CJK/RTL names; `interface-fonts`
  and `bundled-fonts` on/off matrix. Do not add font files; fonts stay in `craft-fonts`. No new image/icon/font
  asset is needed; `ASSETS.md` and `NOTICE` stay unchanged unless a reviewed, licensed asset is separately proposed.
- Use the shared fixture contract from `egui` plan §10.6: craft-fonts `2481a8a…` for Arabic, Thai, CJK SC and
  Japanese, plus pinned Hebrew and Noto Color Emoji v2.047, each with recorded SHA256 and licence text. Fetch them
  into the task fixture directory and enter them in the evidence manifest. Keep the app's own central-font policy:
  `CRAFT_FONTS_DIR` may add fonts, but all builds/tests must still pass with `CRAFT_FONTS` empty and no fixture
  copied into the repository. A system font-provider test is a separate nondeterministic row, not a pixel oracle.
- `egui_kittest`'s missing-glyph default is `Panic`; fix fixture fonts rather than globally allowing tofu. Use
  `allow_missing_glyphs` only in a documented deliberate case. Production keeps `Tofu`, so glyph-coverage
  assertions (not an expected panic) guard the shipped layout.
- Test color emoji/combining marks if the fixture uses them; preserve texture wrap/filter semantics (the
  transparency grid uses `Repeat`).

### 4.8 IDs and accessibility

Implement §3.3, §3.10 and §3.4:

- Mechanical `Id::new` → `Id::unique`; `ui.id()` → `ui.scope_id()`; salt-taking APIs get `IdSalt`.
- Keep stable keys for persisted temp state; `Id::unique` uses the same hash. Add a test that UI state/preferences
  serialize identically before/after the rename and that a saved workspace restores.
- Replace `WidgetType` per §3.2; update tests to `WidgetInfo.role`.
- Add names to every visible input. Use `label_inputs_by` for rows and `name_inputs` for standalone fields.
- `row_label` returns its existing `ui.label` response; use `ui.label_inputs_by(label.id, |ui| …)` around
  `field_w`/`length_field`/form-grid values. Do not add a duplicate label. Treat the custom-painted
  `widgets::label_row` as a separate response/capture job.
- Keep role/name/value/selected/disabled/focus tests. Add hidden-control disappearance.
- `craft-ui` role changes are in the shared PR; review its `Other → Unknown` and `SelectableLabel → Button`
  decisions semantically, and retain its permission tests.

### 4.9 GPU and surface behavior

- Keep `gpu::surface_limit` and `LOW_LATENCY`; the fork's E2/E3 also fits the surface and returns
  `WgpuError::ConfigureSurface`/`CreateRenderer`. `gpu::finish` already treats any `eframe::Error::Wgpu` as a
  graphics failure and restarts without the adapter; add tests for the two new variants.
- Keep `Dx12Compiler::Fxc` and the indirect-validation flag; 0.37's #8736 is an upstream report, not a license to
  change renderer semantics.
- Keep `GraphicsLoss` recovery copies and restart path; test a device-lost callback still writes recovery copies.
- Test texture uploads/frees across DPI/resize, the canvas texture, transparency grid `Repeat`, and screenshot
  readback. Measure renderer construction separately from first frame on Windows (FXC).

---

## 5. Tests and regression assertions

### 5.1 Gate commands

Planning itself runs no build/test. Implementation must use the server's managed workflow
(`/home/codex/work/maintenance/task.py run/status/finish`) with its own task, source and evidence. Proposed checks:

```text
cargo xtask ci
cargo xtask wasm
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p vectorcraft-ui-egui --no-default-features --test install_fonts
cargo clippy -p vectorcraft-ui-egui --no-default-features --all-targets -- -D warnings
cargo test -p vectorcraft-text --no-default-features --test without_bundled_fonts
cargo xtask ui-deps
cargo xtask pdf-deps
CRAFT_FONTS_DIR=/absolute/craft-fonts cargo xtask ci
```

`cargo xtask ci` currently runs fmt, clippy, test, no-bundled-fonts, pdf-deps, assets, brands, cursors, layers and
wasm. Add `ui-deps` to that list. On Windows use the clone's toolchain and `--locked`; on Linux use separate
`CARGO_TARGET_DIR` allocations. Do not run a full workspace build inside another build.

### 5.2 Headless and UI assertions

Start with the tests that already encode behaviour:

| Area | Existing tests/files | What to add |
|---|---|---|
| Whole-window frame | `crates/ui-egui/src/graphics.rs` `frame`, texture tests | tick/ui split, multi-pass, hidden paint, texture counts |
| Panels/dock | `panel_docking/tests.rs` (pointer/keyboard docking), `panel_docking/native_tests.rs` (desktop OS windows), `layout_migration/tests.rs` (older layouts), `dock.rs` (2 scroll-bar tests), `workspaces.rs` (saved dock width) | dock width restore, panel order/clip, 1x/1.5x/2x, floating, native-window focus/keyboard/drops, layout conversion |
| Tabs/menus | `panels/tabs.rs` tests, `menus.rs` | keyboard/accessibility tree, menu geometry |
| Fields | `tests/numfields.rs`, `widgets.rs` tests | wheel, focus, names, roles, click-away selection |
| Canvas | `canvas.rs` tests, `tests/rotateview.rs`, `tests/scrub.rs` | wheel source, zoom/pan, multi-pass, IME |
| Dialogs | `dialogs/tests*.rs`, `dialogs/modal.rs` | sizing pass, names, focus, screen height |
| Clipboard | `tests/sysclip*.rs`, `tests/clipboard.rs`, `tests/pastechords.rs` | eframe clipboard feature, PasteImage, PRIMARY with per-seat keyboard and pointer serials |
| Saving/recovery | `tests/save.rs`, `tests/svgsave.rs`, `tests/recovery.rs` | save/reopen after migration, autosave under logic-only |
| Native menu | `tests/nativemenu.rs` | host-iteration count once, not per pass, menu sync |
| Automation | `tests/automation.rs`, `control_server.rs` tests | screenshot callback routing, timeout, hidden paint |

Add a `tests/lifecycle.rs` (new) covering:

- The test driver calls `raw_input_hook`/`begin_host_iteration` once, then `run_ui(|ui| { tick(ui.ctx()); ui(ui) })`
  for a full frame, and `run_logic(|ctx| tick(ctx))` for a logic-only frame — the host's actual order.
- `tick` runs once per host iteration (including multi-pass and logic-only), never once per layout pass; logic-only
  ticks but reads no raw input.
- A forced second pass does not duplicate a command/undo step.
- A shortcut delivered in a full pass is not replayed in subsequent `run_logic` calls.
- A full frame whose input is consumed, followed by hidden logic-only calls, leaves the consumed events consumed.
- Closing/unsaved prompt from a hidden window sends `RequestPaintWhileHidden`; a forced hidden UI pass consumes its
  current input.
- Background save/open/recovery continues while hidden.
- `host_iteration`, `accepted_ui_frames` and `ui_pass_nr` are distinct; menu hashing and key repeat use the first
  and monotonic time, not a layout-pass index.
- Baseline driver uses 0.36.2 `logic`/`ui` (no `tick`) and produces the same lifecycle/pass count record.

Add a `tests/screenshots.rs` (new) covering:

- one request → exactly one reply and one file;
- two requests → distinct images/paths, exactly two replies;
- timeout/close → late callback writes nothing and sends no second reply;
- dropping/canceling the request record makes a queued callback a no-op; retained image bytes return to zero;
- admission reserves 4/128/256 MiB checked bytes before issue; a fifth request or an oversized image is rejected
  without evicting an admitted one, and its reservation is released;
- a resize, DPI, zoom, surface or viewport change before save rejects/retries, including a change back to the
  original value; unchanged geometry derives scale from the returned dimensions and checks aspect/rounding;
- hidden/minimized behavior (native only, or a fake integration), including a queued-but-unissued request that
  needs `RequestPaintWhileHidden` to reach issuance and a callback that does not wake by itself;
- the callback holds no strong `Context`/app: a drop test retains only the weak slot, and no PNG encoding, file
  write or command runs on the callback thread; the live slot owns the strong `Context`/owning-viewport wake handle waker;
- the shared craft-ui helper (from L2, no app-local slot) passes the same correlation/cancel/close/epoch tests.

Add accessibility tests in the existing `role_and_state` style and via kittest:

- every visible input has a non-empty accessible name and a sensible role;
- checkbox mixed/checked/unchecked; radio selected; tab selected; slider value; combo value;
- disabled/hidden controls absent or marked disabled;
- keyboard Space/Enter/arrows activate the same command as a click.

### 5.3 Visual/geometry assertions

- No new committed raster goldens for the whole app. Cross-renderer pixel goldens are noisy and would require
  asset/provenance review. Use geometry, clip rectangles, AccessKit trees and texture counts as committed
  assertions; keep paired screenshots as migration evidence under
  `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/vectorcraft/`.
- The shared component suite is separate: run `cargo test -p craft-ui --test visual_regression` on the L1 and L2
  revisions and require its 414 exact positive pairs (icon buttons, toolbar fitting, widget palette, SVG
  preparation) plus 36 negative controls, comparing pixels and meshes with no tolerance or automatic baseline
  update. App screenshot tolerances never apply to craft-ui's exact-pixel contract.
- For each reviewed visual change, capture unchanged 0.36, **stock 0.37.0 (`841a328b`)** and the candidate at the
  same viewport, DPI, theme, fonts and renderer (the stock arm uses the same operation in a pinned minimal adapter
  when fork-only code cannot compile there); compare with a justified tolerance (for example ≤0.5% differing pixels
  on the same host/renderer), inspect the diff image, and record the reason. The stock arm attributes a difference
  to upstream versus the fork. Never bulk-update or auto-accept a golden.
- Check: app bar/titlebar, toolbar, dock tabs, Properties panels, layers rows, color picker, a modal dialog, menu,
  canvas rulers, transparency grid, selection handles, floating panel. Compare geometry/hit positions alongside
  the image.

### 5.4 Commands, undo, save and reopen

- Preserve command ids, labels, menu paths, shortcuts, params and `enabled`; `UI_COMMANDS` unchanged.
- Add an end-to-end test: open `examples/feature-sheet.vectorcraft` (synthetic, committed), edit via command,
  undo/redo, save as a temp `.vectorcraft`, close, reopen, assert document meaning, selection/view expectations,
  undo semantics and no format change. Also do this through the control channel.
- Test the Type tool's IME replacement as one undo step and a save/reopen of the result.
- Test workspace/preferences persistence after `Id`/panel changes; old `UiState` JSON must load.
- Do not change the document format for this migration.

### 5.5 Native and web workflows

- **Linux X11:** run under a managed Xvfb/real X11 session at 1440×900, 1x; control channel; open/edit/save/reopen;
  screenshot; Wayland separately under a compositor (Weston or the host session) at 1.5x/2x; test clipboard/PRIMARY,
  IME if available, pen pressure if a device is available, window resize. Record compositor/version/driver.
- **Windows:** use a uniquely owned clone per the Craft Windows procedure; build the exact SHA; launch in the
  interactive desktop session; drive the control channel; test save/reopen, native pickers, DPI, clipboard,
  IME where installed, and a software-rendered path. Do not claim physical GPU evidence from the baseline.
- **macOS:** no macOS host or display is available here. Treat as a blocking acceptance gate for release, not a
  pass. The existing unsafe `mac_window::center_buttons` may be replaceable by 0.37's
  `Frame::set_traffic_lights_position`, but do not adopt that without macOS evidence; keep the existing code and
  test it on real hardware later.
- **Windows ARM64 and FreeBSD:** keep the existing `.github/workflows/windows-arm64.yml` and `freebsd.yml` checks:
  compile/package and the smoke they actually run. Do not treat a cross/QEMU build as GUI, IME, clipboard or GPU
  evidence; record those rows as unavailable unless a real machine is used.
- **Web:** `trunk build --release`, serve, use headless Chrome with `--remote-debugging-port`; exercise
  `window.vectorcraft.openFile/execute/screenshot`, file drop, DOM paste, WebGPU and `?webgl`, save/download, and
  screenshot. A compile-only wasm check is not a browser workflow.

### 5.6 Fonts, IDs, IME and accessibility matrices

- Fonts: `interface-fonts` on/off; `bundled-fonts` on/off; `CRAFT_FONTS_DIR` empty/set; Latin, CJK, Cyrillic,
  combining marks, RTL names, emoji. Compare glyph presence, not just layout success.
- IDs: persisted temp state, workspace, panel width, selection, focused field, scroll offset, open popup.
- IME: preedit, commit, cancel, replacement bytes/chars, focus/serial changes, two windows, UTF-8 boundaries,
  graphemes, RTL text; outgoing surrounding text after delete+commit and deletion-only batches, preedit exclusion,
  long-selection/unavailable transitions, password clearing, current-epoch serial mismatch (incoming once, outgoing
  deferred) and old-epoch rejection; final multi-pass output.
- Accessibility: name/role/value/selected/disabled/focus; hidden controls; keyboard and pointer paths.

### 5.7 Mutation checks (prove the tests can fail)

Make each deliberate defect in a local branch, run the relevant test, record the failure, then revert:

1. Remove the new accessible name from a numeric field → kittest/role test fails.
2. Change a role from `CheckBox` to `Button` → role test fails.
3. Use one screenshot slot/channel for two requests → correlation test fails or one reply is lost.
4. Do not wake the owning viewport after a screenshot completion → pending test times out; capture a strong
   `Context` in the callback → weak-slot/drop test detects the retained cycle; skip the geometry epoch → a
   resize/DPI change back is accepted and the stale-image test fails.
5. Remove the panel-width update → dock restore test fails.
6. Force a duplicate command on a second pass → undo-count test fails.
7. Wrong clip/offset in a panel → geometry/hit test fails.
8. Remove `default_fonts` → `install_fonts` test fails its non-empty-family assertion (production would draw
   tofu; it does not panic).
9. Read `ctx.input` events or `i.time` in `tick` → logic-only stale-input/timer test fails.
10. Insert a paste twice (PasteImage + system clipboard) → clipboard test fails.
11. Reuse a layout-pass index as the host-iteration gate → hidden multi-pass test shows a skipped tick.
12. Add a second `row_label` instead of returning the existing Label response → accessibility tree shows a
    duplicate/unlinked label.
13. Set the E21b canvas payload to `None` or pass display-bidi text → outgoing surrounding-text test fails.
14. Accept an old-epoch `done` against the new owner → two-window/old-epoch test fails; treat `Available` empty as
    `Unavailable` → empty-editor protocol test fails.
15. Fall back to the first seat or reuse a stale serial for a W15P claim → per-seat serial/focus test fails.

Record the exact test name, command, expected/actual and the reverted diff in evidence.

---

## 6. Performance

### 6.1 Baseline before production edits

Add a UI-frame harness before touching production code. Proposed:

- `apps/vectorcraft-cli` gains an optional `ui-perf` feature/dependency on `vectorcraft-ui-egui` and
  `apps/vectorcraft-cli/src/perf_ui.rs`, invoked as `vectorcraft-cli perf --ui --frames 1200 --json PATH`.
- It builds a deterministic `Session` with `perf::synthetic(50_000)`, installs `theme::install_fonts` and the same
  `craft-fonts`/interface fonts as production, and runs a **version-neutral** frame driver on a fixed
  `egui::Context`: baseline 0.36.2 calls `VectorcraftApp::logic` then `ui` inside `run_ui` and `logic` inside
  `run_logic`; the candidate calls `tick` then `ui` inside `run_ui` and `tick` inside `run_logic`. Both invoke the
  host `raw_input_hook`/iteration boundary once per frame, in the real host order. The baseline must not use a
  nonexistent `tick` API or a shim that changes its lifecycle.
- Count one sample per accepted `run_ui` frame (use the driver's frame completion / `cumulative_frame_nr()` delta),
  not per layout pass. Logic-only iterations are reported separately. Record the exact lifecycle, hook calls and
  pass count for both sides.
- Fixed environment: 1440×900, 1x, dark theme, dock open, no hidden panels; `pixels_per_point` set explicitly;
  `CRAFT_FONTS_DIR` fixed; same profile/features/compiler.
- Workloads: cold first frame/font load; 1,200 idle frames; 1,000 pan frames; 1,000 zoom frames; selection drag;
  panel scroll; dialog open/close; text typing; save/reopen; screenshot request (where renderer available).
- Metrics: full `Context::run_ui` wall time, app `perf.frame_ms`, shape count, tessellation/vertex counts,
  `textures_delta` bytes and texture count, peak RSS, idle repaint delay/count, logic-only tick time.
- Report p50/p95/p99 over ≥1,000 accepted frames; 10 warmups + 30 timed iterations for discrete operations; ≥20
  fresh processes for startup. Do not call a p99 from 20/30 samples a tail estimate.

Also run the existing `vectorcraft-cli perf --paths 50000 --deep` (engine/render budgets) on the same fixture and
`vectorcraft-cli bench FILE` for a representative example. These measure different things; do not compare compile
time or CLI throughput to UI latency.

### 6.2 Paired comparison

- **Performance control:** the pre-migration control is the then-current main `4a7cba0…` (the two-commit DXF delta) with its
  coherent 0.36 lock, not `8906073…` alone. The candidate is the migration branch rebased onto `4a7cba0…` (or a
  later main after the same gate). This keeps `crates/cad/src/import/curve.rs` identical on both sides and isolates
  the egui migration. `8906073…` remains the historical planning baseline and rollback identity, not the paired
  control. If the DXF delta cannot be included in the control, measure it separately and show it does not touch the
  measured UI paths.
- Same compiler (1.95), profile, features (`wgpu`/`accessibility`, `interface-fonts`, `bundled-fonts`), fonts,
  fixture, OS, renderer/driver, DPI and viewport. Record all three source SHAs, lock identities and the exact diff
  between control and candidate (and the stock adapter, if any).
- **Three arms in the first baseline report and in every investigated regression:** unchanged 0.36, stock
  0.37.0 (`841a328b`) and the selected fork/candidate, with the same environment. The stock arm uses the same
  operation in a pinned minimal adapter/harness when fork-only app code cannot compile unchanged; record the
  adapter diff and any remaining semantic difference. App acceptance may still use the paired old/candidate arms,
  but no upstream attribution is claimed without the stock arm. Keep stock results even when they are not the
  acceptance pair.
- Use the shared performance protocol's floors: at least 10 warmups + 30 timed iterations for steady operations,
  20 fresh processes for startup, and at least 1,000 *accepted* frames for any frame distribution. Do not call a
  p99 from 20/30 samples a tail estimate. Software and real-GPU results are separate.
- Separate software renderer (Windows WARP / lavapipe) and real GPU (i915 renderD128 on Linux if the chosen
  Vulkan/GL backend works) results. No display session is inherited; qualify the backend through managed jobs.
- Alternate the control/candidate order, include the stock arm in the same environment, and run sequentially with
  no concurrent builds.
- Predefine gates: investigate a repeatable >5% median or >10% p95 regression, or >5% peak-RSS increase above the
  measured noise floor; a new idle busy-loop or repaint storm; or any existing absolute budget failure. For
  sub-millisecond operations require >0.05 ms or corroborating allocation/instruction counts. Re-run noisy
  comparisons and reject inconclusive ones; never widen a threshold or an expanding noise allowance after seeing a
  failure. Existing `perf` budgets are absolute and remain in force.
- Measure renderer construction (DX12 FXC) separately from first frame; measure font discovery separately if
  `system_fonts` is ever considered.
- Record raw samples, median/p95/p99, RSS, allocation/texture counters and load average for each arm. Retain
  baseline, stock and candidate executables and logs outside disposable targets.

### 6.3 Regression response

If #8731-like text tessellation or #8736-like shader cost appears on our fixtures:

- reproduce on unchanged 0.36, stock 0.37 and selected fork;
- isolate plain vs color-glyph rows, Repeat/MirroredRepeat semantics, first-frame vs steady state;
- only then consider a row-level fast path or explicit override constants;
- clamp-only sampling is not an acceptable fix; preserve wrap/filter semantics.

---

## 7. Reviewable PR stages, dependency DAG and rollback

### 7.1 Stages

Every branch/tag push and every new PR in this table needs fresh, explicit human approval after the exact
publication and validation are shown. Planning, implementation and review do not grant publication approval.
Public text carries no agent/model/tool attribution.

| Stage | Repository | Contents | Depends on | Must pass |
|---|---|---|---|---|
| V0 | VectorCraft | This plan only | — | Adversarial review; no code |
| V1 | VectorCraft | Version-neutral UI perf harness, lifecycle/screenshot/accessibility test scaffolding on **0.36.2**; baseline, `4a7cba0…` control and stock-0.37.0 captures | V0 | `cargo xtask ci` on baseline; baseline/control/stock perf report; no production behavior change |
| D1a | storytold/winit | Complete reviewed immutable winit: W1–W15 minus W5, plus W17–W27 including replacement W20, W15P, bounded transfer; provenance/PATCHES and clean-fetch proof | Independent winit adversarial review; upstream prerequisites | winit test suites, W15P/transfer contract tests, clean `cargo fetch --locked` |
| D1b | storytold/egui | Complete reviewed immutable egui **P8** (`craft/0.37.0-release`): P2/P3/P4/P5/P6, P2b (E4 regular + PRIMARY), P7 (E7/E19/E21), P7b (E21b), P7c (#8763/#8748); provenance/PATCHES and clean-fetch proof | D1a; independent egui adversarial review; P8 prerequisites | egui test suites, E21b/W27 and P7c regression tests, clean `cargo fetch --locked` |
| L1 | craft-libs | CL-1 coherent 0.37 API port on `3832c47…` (which already carries `8614e4a`/`1764bca`): `craft-platform` and security/settings/docking/printing fixes preserved, egui 0.37 pins to D1b; includes the CL-2 capture-slot prototype for the DesignCraft/GridCraft pilots (VectorCraft does not consume it) | D1b | craft-libs fmt/clippy/tests, `cargo test -p craft-ui --test visual_regression` (414 exact positives, 36 negative controls, no autoupdate), no security regression; packaged-manifest check; no stale PR199 tree |
| L2 | craft-libs | CL-2F final capture-helper API freeze after the DesignCraft/GridCraft adapters are reviewed; final immutable craft-libs revision on the L1 base | L1; adapter review | Shared capture contract tests; visual regression; final package/lock check; adversarial review |
| V2 | VectorCraft | Atomic compatibility PR: manifests, lock, root patches, `Id`/`Role`/`MouseWheel`/shared screenshot-helper adoption/fonts/scroll/panel-width changes, required E21 byte deletion and E21b canvas producer/owner lifecycle, **and the accessible names/glyph-fixture fixes required to keep kittest 0.37's strict tests green**; no blanket `with_accessibility_check(false)` | V1, D1a, D1b, L2 | `cargo xtask ci`, `ui-deps`, wasm, native build; exact test counts; rebase/DXF gate |
| V3 | VectorCraft | Lifecycle: host iteration/tick/input/ui, multi-pass/logic-only, hidden paint, monotonic clock, separate counters | V2 | Lifecycle tests; no command/undo duplicates; hidden background jobs |
| V4 | VectorCraft | Extended Clipboard/PRIMARY/PasteImage and native E21/E21b/text/RTL qualification; required canvas adapter already in V2 | V2/V3 | Clipboard/IME tests; native Wayland/X11/macOS-manual gate |
| V5 | VectorCraft | Broader accessibility audit/coverage (all names/roles, hidden/disabled, keyboard), panel width persistence, full font/glyph matrix | V2/V3 | `cargo test --workspace` with kittest strict; no-default-fonts gates |
| V6 | VectorCraft | Full native/web/Windows/Linux validation, performance, screenshots, docs/parity evidence; adversarial implementation review | V2–V5 | Full acceptance matrix; performance gates; review findings closed |
| V7 | VectorCraft | Remove superseded test helpers and local shims (never a capture slot); no behavior change | V6 | Same gates; diff review |

V2 is deliberately atomic: a manifest/lock bump alone would leave `main` unbuildable. It also cannot postpone the
accessible-name fixes that `egui_kittest` 0.37 now enforces: the workspace tests would fail immediately. Do not keep
a blanket `with_accessibility_check(false)` as a staging shortcut. V3–V5 may land in parallel only if each branch
includes V2 and they do not touch the same lifecycle files; prefer sequential for review. V2 waits for the complete
D1a winit and D1b egui P8 revisions and for L2; no intermediate fork or craft-libs stage is a production pin.

**Rebase gate before V2:** fetch current `storytold/vectorcraft` main, record its full SHA and diff against the
pinned review baseline. The recorded drift is exactly two commits from `8906073…` to `4a7cba0…`, both changing only
`crates/cad/src/import/curve.rs` (DXF spline). Rebase the migration branch onto that (or a later) main, preserve the
DXF fix, run the DXF import tests, and use the same `4a7cba0…` source as the performance control in §6.2. If more
files or dependency changes appear, stop and re-audit before implementation; do not drop or rewrite the unrelated
fix.

### 7.2 Rollback

- Before V2, preserve the exact baseline `Cargo.toml`, `Cargo.lock`, baseline/control/stock executables, D1a/D1b
  fork revisions and L1/L2 craft-libs revisions outside disposable build dirs. Do not rely on `cargo update` to
  reconstruct an old graph.
- Rollback is a revert of the complete V2 (or V2–V6) commits, not a version-string edit. The old 0.36.2 lock and
  craft-libs revisions must remain reachable. Keep the old dependency set in a local branch/commit and in
  evidence; do not delete it during build-cache cleanup. The `4a7cba0…` DXF fix is upstream work, not part of the
  rollback.
- Per stage: **D1a** and **D1b** roll back by a new immutable revision on the prior base; never force-push or move
  a published tag, and never delete a published revision. **L1** and **L2** roll back by a new craft-libs revision
  on the previous base (L2 → L1 → `3832c47…`), preserving the prior capture-helper and PR199 port. **V2–V6** revert
  the app commits to the coherent 0.36.2 lock and its craft-libs revisions. Record each revert revision, parent and
  validation, and keep the superseded immutable revisions reachable.
- If a fork/craft-libs revision becomes unavailable, stop and restore the last coherent graph; do not substitute
  a branch, path patch or a second egui version.
- No persistence format change is planned. If one becomes necessary, specify backward compatibility and keep
  unreadable data, or abandon that part of the change.

### 7.3 Documentation and evidence

- `docs/architecture.md`: revise the UI/lifecycle section to describe host iteration, `tick`/input/`ui`, logic-only iterations,
  root `Ui`, screenshot callbacks and the dependency patch policy.
- `docs/development.md`: record the selected egui/winit/craft-libs revisions, `ui-deps`, the UI perf harness and
  any changed browser/Windows workflow. Keep the existing font rules (`craft-fonts`, `CRAFT_FONTS_DIR`).
- `docs/control-protocol.md`: update `ui.screenshot` only if native evidence changes the minimized/covered
  behavior; keep the timeout/error contract otherwise.
- `ROADMAP.md` progress log and the touched docs' status/revision history: record the dependency migration. Do not
  move `docs/target-app-parity.md` scores, `docs/gaps.md` entries or parity checklists unless a measured behavior
  actually changes; this migration is infrastructure, not a feature claim.
- Keep all evidence under `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/vectorcraft/`: exact
  source/lock identity, build/test logs with counts, screenshots, perf raw samples, dependency metadata, review
  findings and commands. Do not put source or deliverables in disposable build targets.

---

## 8. Blocking unknowns and acceptance criteria

These are not “future work”; they block calling the migration done:

1. **Complete final forks and reachability.** Approve and publish reachable immutable **D1a** `storytold/winit`
   (W1–W15 minus W5, plus W17–W27 including replacement W20, W15P, bounded transfer) and **D1b** `storytold/egui` P8 (P2b, P7, P7b/E21b,
   P7c, complete). Record exact full SHAs and tree hashes, preserve fork authorship/PATCHES, and prove a clean
   `cargo fetch --locked`. No intermediate fork stage is a production pin; candidate aiongg revisions remain
   research inputs; no final SHA is invented in this plan.
2. **Final craft-libs revision (resolved by the published pin).** The CL-1/CL-2F delta is published in
   craft-libs `25da131…`; the current workspace pins `craft-ui`, `craft-window`, `craft-keymap` and
   `craft-print` to descendant `a0916808…`, retaining that delta and upstream shared docking. The plan-time requirement was to finish that delta on the `3832c47…` base (which already
   carries the selected `8614e4a`/`1764bca` changes), preserving `craft-platform`,
   security/settings/docking/printing fixes, pin D1b and pass `craft-ui` visual regression. Do not merge PR199
   head `b68de8c` or audit tree `ec8840a` as trees. L1 (CL-1) and L2 (CL-2F) preceded V2; publication waited for
   the shared gates and the capture-helper adapter review.
3. **`default_fonts`/font provider.** `ui-egui` library tests and no-default-feature builds must bind non-empty
   families; production `Tofu` would otherwise draw synthetic boxes, and downstream kittest tests do not inherit
   kittest's own dev-dependency fonts. `system_fonts` remains off unless measured and approved; the shared
   font-fixture identities and licences apply.
4. **kittest strict accessibility.** Every visible input in every test that renders it must have a name and a
   correct role. This may require broad, reviewable field/row changes; `row_label` must return its existing label
   response rather than add a duplicate.
5. **Capture callback semantics.** The app must prove exactly-one completion, hidden-window paint, timeout and
   late-callback safety with a weak completion slot (the live slot owns the strong `Context`/owning-viewport wake handle waker; callback pixels
   only; no strong `Context`), pre-issue checked-byte admission (4/128/256 MiB defaults), geometry-epoch binding
   and rejection of any resize/DPI/zoom/surface change before save, and `RequestPaintWhileHidden` for a queued but
   unissued request. Use the L2 shared craft-ui helper; no app-local slot.
6. **Clipboard/PRIMARY.** eframe `clipboard` must be enabled and must not duplicate the app's `SystemClipboard`
   paste, especially for `PasteImage`. W15P/E4 must provide the winit-owned per-seat PRIMARY path with fresh
   keyboard-or-pointer serials, synchronous claims, bounded 2 s/16 MiB/4-read/2-writer/queue-8 transfers and no
   first-seat fallback; arboard PRIMARY is not an unmeasured default.
7. **IME replacement and outgoing text.** The canvas needs an engine-level delete-surrounding operation with a
   precise char/byte contract and one undo step, plus the E21b model/preedit/UTF-8 payload producer and the
   complete egui P7b/W27 lifecycle (owner identity, `Available` empty vs `Unavailable`, committed clear boundary,
   latest-`None` authority, current-epoch mismatch and old-epoch rejection). UI `TextEdit` behavior is separate.
8. **Upstream rebase gate.** At plan time, main was `4a7cba0…`, two commits ahead of the pinned review baseline, touching
   only `crates/cad/src/import/curve.rs` (DXF spline). Rebase before V2, preserve that fix, rerun its import tests,
   and isolate it from performance comparisons (§6.2). Recheck if main advanced further.
9. **macOS evidence.** No macOS host/display is available here. Keyboard layouts, IME (including press-and-hold),
   Metal/GL, native pickers, termination and traffic-light positioning remain an explicit release gate.
10. **Performance.** Paired control at `4a7cba0…` and candidate with the same engine delta, plus a stock-0.37.0
    arm in the first report and every investigated regression, using the shared sample floors and the 5%
    median / 10% p95 / 5% RSS gates. Upstream #8731, #8736 and #8512 are reports, not results; do not claim a
    regression or a fix without ours.
11. **Dependency identities.** Enumerate every fork-family package at one storytold source; audit external
    `kittest`/`accesskit*` as compatible registry identities, not egui-git crates. Skrifa/harfrust/glifo duplicates
    are expected; measure binary size and confirm no public type crossing. A new `egui`/winit duplicate is a hard
    failure.
12. **Role semantics.** Review the `SelectableLabel → Button`, `Checkbox → CheckBox`, `CollapsingHeader →
    DisclosureTriangle` and `Other → Unknown` mappings against actual screen-reader semantics, not just compile.
13. **Wayland/X11/GPU backend.** The host has an i915 render device but no inherited display session; a managed
    Linux job must qualify the actual compositor/backend/DPI. Availability is not a qualified result.
14. **Web.** A wasm `check` is not a browser workflow; headless Chrome WebGPU/WebGL2, paste/drop, save/download and
    screenshots are required.

**Done means:** one coherent 0.37 graph; `cargo xtask ci` and `ui-deps` pass from a clean checkout; lifecycle
tests prove no duplicate/stopped work and no stale input replay; screenshots use the shared L2 weak slot and are
correlated, epoch-bound and admission-bounded; W15P/E21b/W27 and accessibility tests pass; native Linux/Windows and
browser workflows are recorded; performance is paired against the same-engine control with the stock-0.37
attribution arm and within gates; macOS is either evidenced or explicitly unresolved; and the final adversarial
review has no open correctness finding.

### 8.1 Dispositions of the adversarial review (`vectorcraft-v1`)

This table records how every finding was resolved in this revision. It is a static documentation reconciliation;
no reviewer finding is treated as an executed test result or an implementation claim.

| # | Finding | Disposition |
|---|---|---|
| 1 | E21b/P7b outgoing IME surrounding text missing | **Accepted and fixed.** Added E21b/W27 requirements, the canvas model/preedit/UTF-8 payload adapter, final-output/password/focus rules and tests in §3.9, §4.6, §5.2/§5.6, §8; D1b (P7b in P8) is a prerequisite. |
| 2 | Final storytold pins and craft-libs publication order not implementable | **Accepted and fixed.** D1a winit and D1b egui P8 precede the craft-libs CL-1 L1 and CL-2F L2; V2 depends on L2; no self-dependency; no invented SHAs; clean-fetch gate retained. §4.1, §7.1. |
| 3 | Required W15P/E4 PRIMARY integration absent | **Accepted and fixed.** W15P is a prerequisite, E4/P2b routes PRIMARY through it, second data device forbidden, bounded-transfer and compositor tests added. §3.6, §4.5, §8. |
| 4 | Screenshot callback violates weak-slot/cancel contract | **Accepted and fixed.** Replaced receiver/strong-`Context` design with a per-request weak completion slot, pre-issue pixel admission, aggregate limits and shared craft-ui helper after CL-2F. §3.5, §4.4, §5.2. |
| 5 | `raw_input_hook`/logic-only lifecycle premise is source-wrong | **Accepted and fixed.** Corrected to both hosts calling the hook before the logic-only decision; it is a host-iteration boundary, not a UI-phase marker. Separate host-iteration and accepted-UI-frame counters; stale input only in a known UI phase; test driver preserves host order; baseline uses `logic`/`ui`, not `tick`. §2.4, §4.2, §5.2, §6.1. |
| 6 | Whole-family pin gate not exhaustive | **Accepted and fixed.** Full fork-family allowlist plus separate external `kittest`/`accesskit*` registry audit; negative fixtures; version alone insufficient. §4.1. |
| 7 | The performance harness uses a nonexistent 0.36.2 `tick` API | **Accepted and fixed.** Version-neutral harness: baseline `logic`+`ui`, candidate `tick`+`ui`, same hook order, accepted-frame counting, same-engine control for isolation. §6.1/§6.2. |
| 8 | `row_label` is already a real `Label` | **Accepted and fixed.** Return the existing `ui.label` response and use `label_inputs_by`; do not duplicate the label; handle custom-painted `widgets::label_row` separately. §3.10, §4.8. |
| Minor | `Id::new` count stale | **Verified and corrected** to 294 occurrences across the three paths. §3.3. |
| Minor | Baseline date stale | **Verified and corrected** to 2026-10-23 11:50:04 +0700. Header/§2.1. |

### 8.2 Dispositions of the adversarial review (`vectorcraft-v2`)

Every finding in `plan-reviews/vectorcraft-v2.md` is closed below by source-grounded plan text, not a footnote.

| # | Finding | Disposition |
|---|---|---|
| 1 | No stock-0.37.0 control arm | **Accepted and fixed.** Stock 0.37.0 (`841a328b`) is a measured arm in the first baseline report and every investigated regression, with the same environment and a pinned minimal adapter where fork-only code cannot compile; retained even when app acceptance pairs 0.36 with the candidate. §5.3, §6.2, §7.1 V1, §8.10. |
| 2 | Capture admission and geometry under-specified; 64 MiB too small | **Accepted and fixed.** Adopted the shared 4 requests / 128 MiB per image / 256 MiB aggregate defaults (stricter only if the documented intended surface fits), checked bytes reserved before issue, no eviction; records logical/screen bounds, viewport, DPI/zoom and surface epoch at issuance and rejects/retries any change before save, including change-back, then verifies scale/aspect. §3.5, §4.4, §5.2, §8.5. |
| 3 | W27 old-epoch invalidation missing | **Accepted and fixed.** Old-epoch `done` is rejected and invalidates that epoch, never edits the new owner; current-epoch mismatch applies incoming once and defers outgoing. §3.9, §4.6, §5.6, §5.7. |
| 4 | P7c ownership of #8763/#8748 absent | **Accepted and fixed.** P7c owns both fixes and their pointer/keyboard and hidden-parent regressions; D1b includes P7c, V2 depends on it, and an app workaround needs equivalent tests plus a removal condition. §3.11, §7.1, §8.1. |
| 5 | Empty-family font claim source-wrong | **Accepted and fixed.** Production `MissingGlyphPolicy` defaults to `Tofu`; the kittest harness alone defaults to `Panic`. Mutation 8 now expects the non-empty-family assertion failure; `theme.rs`'s "egui panics…" comment is scheduled for correction. §1, §3.8, §3.10, §4.7, §5.7, §8.3. |
| 6 | craft-ui visual regression and negative controls missing from L1/V2 | **Accepted and fixed.** L1/L2 run `cargo test -p craft-ui --test visual_regression` with 414 exact positive pairs and 36 negative controls; no tolerance or autoupdate, and app screenshot tolerances stay separate. §5.3, §7.1. |
| 7 | `ui-deps` ledger conflates required and optional packages | **Accepted and fixed.** Required-used, optional/forbidden-if-present and external registry identities are separate ledger classes; the transitive `accesskit_*` platform crates are audited; a missing optional package passes. §4.1. |
| 8 | D1 not split into winit then egui | **Accepted and fixed.** D1a winit precedes D1b egui P8, each with its own review, SHA, tree hash and clean-fetch proof; V2 depends on D1a, D1b and L2. §4.1, §7.1, §8.1. |
| 9 | Rollback is app-only | **Accepted and fixed.** Per-stage rollback: new immutable revisions for D1a/D1b and L1/L2, never force-push or move published tags; V2–V6 revert to the coherent 0.36.2 lock. §7.2. |
| 10 | Dead provisional app-local capture slot | **Accepted and fixed.** Removed: VectorCraft adopts the L2 helper only and keeps no app-local or provisional slot. §3.5, §4.4, §7.1 V7. |
| 11 | Clipboard rationale overstates impact | **Accepted and fixed.** Reworded to egui-winit's text clipboard/`Event::Paste` path; the arboard `SystemClipboard` remains the independent rich-format owner and the textless-paste tests stay. §3.6. |
| 12 | W15P synchronous ownership details absent | **Accepted and fixed.** Fresh keyboard-or-pointer serial and focused seat, synchronous claim in the triggering turn, no first-seat fallback; bounded async reads 2 s/16 MiB/4 reads/2 writers/queue 8, cancellation with owner-specific wake. §3.6, §4.5, §5.7, §8.6. |

---

## 9. Source anchors and citations

Primary sources inspected (read-only):

- `/home/codex/work/CraftApps/egui037-migration-20261022/PLANNING-CONTRACT.md`
- `/home/codex/work/CraftApps/egui037-migration-20261022/vectorcraft` (assigned checkout)
- `/home/codex/work/CraftApps/egui037-migration-20261022/egui` at `0.37.0` and `aiongg/craft/0.37`
- `/home/codex/work/CraftApps/egui037-migration-20261022/winit` at `8b35388…`/`ffae519…`
- `/home/codex/work/CraftApps/egui037-migration-20261022/craft-libs-implementation` at `3832c47…` (the historical
  local planning base; the CL-2F helper later landed in published craft-libs `25da131…`) and the historical PR
  199/audit branches
- `craft-libs/docs/egui-037-suite-rollout.md` and `craft-libs/docs/egui-037-migration-plan.md`
- `egui/docs/egui-037-migration-plan.md` (E21b, W15P, family pins, §10.6 font fixtures)
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/` (upstream discussions, text-input research,
  lifecycle audit, IME-surrounding gap, remote bases)
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/lifecycle-source-audit.md`
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/ime-surrounding-gap.md`
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/remote-plan-bases-20261023-0632.json`
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/plan-reviews/vectorcraft-v1.md`
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/plan-reviews/vectorcraft-v2.md`
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/craft-fonts-manifest-2481a8a.txt`
- `/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/{fork,stock}/vectorcraft.errors`

Public upstream references (immutable where possible):

- egui 0.37.0 changelog: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/CHANGELOG.md`
- `App::logic`/`App::ui`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/eframe/src/epi.rs`
- `Context::run_ui`/`root_ui`/`run_logic`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/context.rs`
- screenshot callback: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/screenshot_callback.rs`
- event/`MouseWheelSource`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/input/event.rs`
- `Id`/`IdSalt`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/id.rs` and `id_salt.rs`
- panel ids: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/containers/panel.rs`
- `WidgetInfo`/roles: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/output.rs`
- `is_input`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/accessibility.rs`
- `FontsView`: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/epaint/src/text/fonts.rs`
- kittest strict names/glyphs: `https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui_kittest/src/builder.rs`
- upstream discussions collected in `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/upstream-discussions.json`
  (#8731, #8736, #8763/#8764, #8748, #8512, #8657, #8577)
- suite rollout: local `craft-libs/docs/egui-037-suite-rollout.md` planning draft (link only after it lands)
- craft-libs PR 199: `https://github.com/storytold/craft-libs/pull/199`; issue 198:
  `https://github.com/storytold/craft-libs/issues/198`

Fork provenance:

- egui fork `PATCHES.md` at `a4f9a17…`: E1 viewport hooks, E2+E3 surface fit/errors, E4 Wayland clipboard,
  E5/E12 bidi, E6 IME focus, E7/E19 Latin keys, E8 glyph clipping, E9 RTL UI, E10 web keys, E11 single-threaded
  vello, E13 grapheme editing, E14 fallback baseline, E15 line breaker, E16 cursor affinity, E17 language,
  E18 UI language, E20 widget language, E21 IME delete bytes. The final fork additionally owns **E21b** outgoing
  IME surrounding text, which is not present in the candidate.
- winit fork at `8b35388…` has W1–W27 as inspected but not the new **W15P** PRIMARY API or the W27 lifecycle/serial
  revisions. The final winit fork must add both; the candidate branch is not a production pin.
- winit fork `PATCHES.md` at `8b35388…`: W1, W2, W3–W10, W11, W12, W13, W14, W15, W17–W27. Relevant to VectorCraft:
  W1/W4 (Windows DPI/hidden), W2/W13/W15 (Wayland text/clipboard/drop/wheel), W5–W10/W18–W22 (IME), W11 (X11 DPI),
  W14 (macOS termination), W23 (⌘ keys), W24 (`reset_ime`), W25–W27 (Latin/IME surrounding text).
- PR 199 port commit `8614e4a` (author Davis Brief): `WidgetType` → `Role`, `UiBuilder::id`/`Ui::id` → `scope_id`,
  `Id::new` → `Id::unique`; test-only U+22EE → `…` because 0.37's bundled fonts dropped it. The U+22EE change is
  test-fixture only; VectorCraft production source has no U+22EE (checked).

---

## 10. Open decisions to confirm with the owner/reviewer

These are decisions this plan recommends but does not silently make:

- Use the full selected fork (including E5/E12/E21 and the new E21b) even though VectorCraft's UI is LTR.
  Recommended: yes, for one suite graph and correct IME; prove UI text layout with tests.
- Keep eframe `system_fonts` off. Recommended: yes, until measured and licensed/tested; the app already has a font
  database. The shared pinned fixtures, not system fonts, are the test oracle.
- Keep `links` off on native (existing behavior); add only if a native `ctx.open_url` path is shown to need it.
- Replace `mac_window::center_buttons` with `Frame::set_traffic_lights_position` only after macOS evidence.
  Recommended for this migration: no.
- Serialize screenshots (one outstanding) rather than supporting concurrent captures. Recommended: yes; simpler
  correlation and memory bound. Use the shared weak completion slot, never a strong `Context` in the callback.
- Add `PasteImage` handling rather than disabling eframe's image clipboard. Recommended: yes, with dedupe tests.
- Keep the app's `SystemClipboard` as the rich-format owner for regular clipboard on Wayland, while the selected
  W15P path owns PRIMARY. Recommended: yes; this is the selected design, not an unresolved ownership test.
- Adopt the shared craft-ui capture helper from L2 (CL-1 + CL-2F); no app-local capture slot.
  Recommended: yes, for all eleven craft-ui consumers.

---

## 11. Revision history

| Date | Change |
|---|---|
| 2026-10-23 | Initial source-grounded planning draft. No implementation, build, test or GUI run. |
| 2026-10-23 | Revision 2: resolved every `vectorcraft-v1` finding (§8.1); corrected the lifecycle model (`raw_input_hook` in both hosts, separate host/accepted-frame counters, host-ordered test driver); added E21b and W15P/E4 prerequisites; moved the weak capture slot to the shared craft-ui contract; fixed final-fork → final craft-libs/CL-2F → app ordering; enumerated family and external registry identities; corrected `row_label` and the `Id::new` inventory; recorded the pinned review baseline, `4a7cba0…` drift/rebase gate and same-engine performance control. Static documentation only; no build/test run. |
| 2026-10-23 | Revision 3: resolved every `vectorcraft-v2` finding (§8.2) by replacing contradictory prose: stock-0.37 third arm in visual/performance; shared 4/128/256 MiB capture admission with epoch-bound geometry and hidden paint request; W27 old-epoch invalidation and complete E21b contract; P7c owns #8763/#8748; corrected production `Tofu` vs kittest `Panic` and non-propagating kittest dev-dependency fonts; craft-ui exact-pixel visual regressions; split `ui-deps` ledger; split D1a winit / D1b egui P8; per-stage rollback; removed the dead provisional capture slot and app-local adapter; corrected clipboard scope; added W15P synchronous claims and bounded transfers; set 5% median/10% p95/5% RSS gates with no tolerance widening. Static documentation only; no build/test run. |
| 2026-10-24 | Revision 4: implementation status; repaired the final upstream-review defects (path+data capture downgrade, image-paste repaint and paste-time chord, monotonic hidden tick, IME context invalidation and unavailable revisions, per-viewport exact-once input, Artboard and Split Grid accessible names, fail-closed stale synthetic input); moved the shared-library base reference to the local `3832c47…` implementation base. At that revision the exact tree still needed fresh managed verification before publication. |
| 2026-10-25 | Documentation reconciliation after PR 4113 and managed job `egui037-vector-postmerge-ci-20261025-1381`: recorded the current merge tree and pass result, marked the `8bf090b…`/`0589` status and 0.36.2 lock table historical, listed current pins from `Cargo.toml`/`Cargo.lock`, and corrected the plan-time local CL-2F base description to the published craft-libs `25da131…`. Documentation only; no build/test run. |
| 2026-10-25 | Shared-docking reconciliation: recorded the local merge of upstream `main` `714434527…` (PR #4135), the upstream/delete resolution of the four conflicts, the reviewed recolor code already in `main`, and CI 1422's pre-merge pass; corrected current docking claims (desktop OS windows, web/Wayland in-page, deleted `floating.rs` and width helpers, `UiState::dock_width` persistence, new native and layout-migration tests) while retaining the earlier planning sections as historical. The combined executable source and lockfile were subsequently qualified by managed job `egui037-vector-combined-ci1443` (all 12 stages passed); the status text was updated after that run. |


## Local implementation update — 2026-10-23

The user clarified that reviewed plans remain local and implementation proceeds before publication. The earlier requirement to publish/land this plan first is superseded; pushes and PR publication still require explicit approval.

Canonical `storytold/vectorcraft` `main` now resolves to `125df6d8ecfae01d01ce369ac02e4aa03e0deed5`. The owned checkout was fast-forwarded to this revision. Earlier baseline SHAs and line anchors above are retained as historical review evidence; use this new revision and its exact lockfile for current-main integration and paired measurements. The original refs remain reachable.

Preserve supplied PDF passwords on confirmation at125df6d8, in addition to the previously recorded DXF fixes. Add dialog regression coverage without recording password contents.

The complete static delta review and exact old/new object identities are retained in the task evidence: `canonical-diffs-refresh-20261023.json` and `overnight-consumer-review-20261023.md`. This amendment claims no new build, native GUI or performance acceptance.

## Request-bound Type tool IME reset — local implementation, 2026-10-24

An interrupted incoming composition now retains the complete ID returned by
`Context::request_ime_reset`. Only its matching `Event::ImeResetCompleted`, in raw event order,
admits subsequent IME preedit/commit/surrounding-delete events. Old terminals, empty preedit,
idle frames and host cancellation return values do not acknowledge the request. A later caret,
target, committed-context or Context change supersedes the old ID. Pointer/navigation/command
notices keep the remaining raw suffix blocked until their actual handler establishes the final
destination. Ordinary uninterrupted composition keeps its existing key ownership.

Each UI pass observes its actual final custom descriptor, including discarded passes and exits
without geometry. Losing custom publication to absence, hidden/focus state or a stock field
retires a pending ID; returning to the same target/caret requests a new ID once. Hooks installed
before the host's first UI pass distinguish its preceding logic callback from standalone hidden
logic, which cannot claim fresh UI geometry. The callbacks capture no Context or app.

The regression source covers the previously failing click-A→B/fresh-が path with a real returned
fence, ordered fresh suffixes without an old terminal, stale/foreign/duplicate IDs, pressed
navigation and shortcuts, discarded/stock/focus reactivation, graphics replacement, external
edits at the same caret, and full-pass versus standalone logic. This local update is pending
dependency refresh, formatting, compilation, tests and independent implementation review. Native
and browser IME acceptance remains separate: unsupported integrations, reset failures or missing
acknowledgements keep interrupted IME mutations blocked; elapsed time or a nonempty preedit never
provides a fallback acknowledgement.
