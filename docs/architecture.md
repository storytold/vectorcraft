# Architecture

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (shared PDF reader and writer; app output policy and editable-import adapter retained; verified PngSuite downloads, #1729; synthetic control-input budgets, #1709; desktop printing uses craft-print) · **Target:** Adobe Illustrator 2026 (30.x)

How VectorCraft is built today. The design rationale lives in the local, gitignored `plan/architecture.md`;
this file describes what is committed. Day-to-day rules (clean-room, never crash, quality gates) are in
[`AGENTS.md`](../AGENTS.md) and [development.md](development.md).

## Shape

A Cargo workspace of 23 library crates under `crates/`, three apps under `apps/` and `xtask`, ~354k lines of Rust,
pure Rust (no handwritten JS/TS). The same engine runs the desktop app, the browser build (WASM), the headless
CLI and the MCP server.

## Layers

`cargo xtask layers` (part of `cargo xtask ci`) enforces that a crate depends only on lower layers, and that
nothing below L6 depends on a UI toolkit (egui, eframe, winit, rfd). Table: `xtask/src/layers.rs`.

| Layer | Crates | Role |
|---|---|---|
| L0 | `geom`, `color`; `affinity` (standalone, no workspace deps) | curves and transforms (on `kurbo`), colour models, ICC and conversions; the Affinity reader |
| L1 | `doc` | the document model: artboards, layers, objects, appearance, text, symbols; pure data + serde, structural sharing |
| L2 | `pathops`, `brush`, `trace`, `text`, `effects`, `plugins`, `three-d` | booleans/Shape Builder/offset/outline; brushes; Image Trace; fonts, shaping (HarfRust, skrifa), bidi and layout; live effects and the pixel-filter pipeline; sandboxed WebAssembly plug-ins (wasmi); 3D surfaces of revolution, projection and shading (`three-d`, on `geom` only) |
| L3 | `render`, `svg`, `pdf`, `format`, `eps`, `cad`, `metafile` | document to pixels (`vello_cpu`, multithreaded); SVG, PDF, native JSON, EPS/PostScript (with its own interpreter), DXF, EMF/WMF readers and writers |
| L4 | `tools` | tool state machines: pointer events in, commands and overlays out (Pen, Direct Selection, smart guides…) |
| L5 | `engine` | sessions, history (unlimited undo by structural sharing), selection, the command registry (~710 commands), file I/O, the panic guard |
| L6 | `ui-egui`, `mcp` | the egui front end (menus, panels, dialogs, canvas, i18n); the MCP server (JSON-RPC over stdio) |
| L7 | `apps/vectorcraft`, `apps/vectorcraft-cli`, `apps/vectorcraft-web`, `xtask` | desktop app (eframe, wgpu), CLI (`run`, `convert`, `info`, `bench`, `perf`, `mcp`), browser runner (WebGPU, WebGL2 fallback), build tooling |

`testkit` holds test helpers (fixtures, generators, render comparison) and is only a dev-dependency.

## Everything is a command

Every user-visible action is a `CommandSpec` in `crates/engine/src/cmd/*` (id, label, menu path, shortcut,
params doc, `enabled`, `run`, journal flag); UI-only commands are `UI_COMMANDS` in
`crates/ui-egui/src/menus.rs`. Tools emit commands (begin, preview, commit). Menus, the command palette, Actions,
the control channel, the CLI and MCP all reach the same registry, so anything a user can do an agent can do.
`command.batch` runs several as one undo step.

## Rendering

The canvas is rasterized on the CPU by `vello_cpu` on worker threads, off the UI thread, with caches per object
and per effect; the GPU (wgpu: Metal, DX12, Vulkan/GL; WebGPU/WebGL2 on the web) only composites the result. Raster
effects are rendered offscreen per object and cached while panning. See
[hardware-parity.md](hardware-parity.md).

## Files

The native `.vectorcraft` format is versioned, compressed JSON with embedded images, saved atomically, readable
down to v1 and from the old `.drawcraft` name. The `FORMATS` table in `crates/engine/src/cmd/fileio/formats.rs`
lists every format read or written; importers are fuzzed (`crates/engine/tests/import_fuzz.rs`). See
[file-format-parity.md](file-format-parity.md).

Desktop printer discovery and PDF submission use `craft-print`, pinned to an immutable
craft-libs revision. The engine still owns sheet composition, copies, separations and colour
management. Submission runs on the existing background worker; its receipt or refusal
reaches the status bar, and Quit waits for pending submissions.

PDF writing uses the optional `craft-pdf` writer at an immutable craft-libs revision. Shared
configuration, metadata, font preparation and guarded serialization leave drawing, colour
policy, layers, editing data and print composition in this app. `cargo xtask pdf-deps` verifies
the shared facade and patched Krilla and Hayro sources at the same revision.
The reader facade opens the PDF with per-document decode limits; the app scans layer and
overprint content in windows and rejects known incomplete content before returning a document.
See [pdf-backend.md](pdf-backend.md).

## Agent control

- **Control channel:** JSON lines on a local TCP port (`--control 7979`), with real egui pointer and keyboard
  injection and screenshots ([control-protocol.md](control-protocol.md)). Synthetic gestures are validated
  before queue mutation; the pending input queue is capped, and control requests yield after a bounded
  batch per frame (#1709).
- **MCP:** `vectorcraft-cli mcp`, attached to a running app or headless ([mcp.md](mcp.md)).
- **Sandboxing:** `--automation-read-root` / `--automation-write-root` confine what agents read and write.

## Robustness

No panics in shipped code (workspace clippy lints ban `unwrap`, `expect`, `panic!` and friends outside tests);
`vectorcraft_engine::guard` catches a panic at every entry point and rolls the document back; Data Recovery
saves copies in the background. See [development.md](development.md#robustness-vectorcraft-never-crashes).

## Corpus fetching

`cargo xtask corpus --download` retrieves only allowlisted regular PngSuite files over HTTPS at a pinned mirror commit. SHA-256 and exact size checks precede publication; the downloader never extracts an archive. Existing verified files are reused; failed downloads leave existing corpus files intact. See [development.md](development.md#verified-pngsuite-corpus).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Pinned PngSuite HTTPS files verified by SHA-256 before publication; bounded downloads replace archive extraction (#1729). Scores and hours unchanged. |
| 2026-10-13 | minor | Bounded synthetic control input (#1709): counts and queued events validated before mutation; control-channel bursts yield between frames. Scores and hours unchanged. |
| 2026-10-13 | minor | PDF.R1: editable PDF import uses the shared pinned reader; layer and overprint scans preserve state across content windows and reject known incomplete input. Synthetic streaming, decoder-limit and dependency-source regressions added; scores and hours unchanged. |
| 2026-10-13 | minor | PDF writer setup and guarded finalization use craft-pdf; drawing and output policy remain app-owned. |
| 2026-10-12 | minor | Desktop PDF spooler shared through craft-print; exact physical sizes, composed copies and Windows 7 fallback retained. Scores and remaining gaps unchanged. |
| 2026-10-10 | minor | Added the `three-d` crate |
| 2026-10-10 | major | First version, from the workspace, the layering table and the existing docs |
