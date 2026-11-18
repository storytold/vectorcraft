# Production Preflight

Open **Window > Production Preflight** before delivering artwork. Choose **PDF**, **Print** or
**Screen**, set the minimum image resolution, and optionally check only the active artboard.
The default minimum is 300 ppi for PDF/Print and 96 ppi for Screen. These are editable workflow
defaults, not a guarantee of suitability for a particular printer or display.

The panel checks:

- **Fonts:** unavailable font families and styles that will use a fallback or a substitute.
- **Text Overflow:** area type or type on a path whose text extends beyond its frame.
- **Linked Files:** missing files and files that changed since they were read.
- **Image Resolution:** effective resolution along both pixel axes at the image's placed size.
  Print includes the job's page transform, so enlarging the print can lower effective resolution.

Click a finding to select and zoom to the affected artwork. A font finding can select several
objects. A finding inside a symbol, pattern, brush or opacity mask locates its first visible
canvas owner. If some artwork cannot be selected, the status bar reports it; any selectable
objects are still located without unlocking artwork or artboards. **Find Font…**, **Relink…** and
**Update Link** open the existing repair workflows;
they do not apply an automatic font substitution or change artwork just by checking it.

Results refresh after document edits, Undo/Redo, document switches and changes to the panel's
options. During an interaction the panel waits for the edit to finish. External files are
checked again every two seconds while the panel is visible; **Recheck** runs immediately.

PDF and Screen inspect visible artwork intersecting the chosen artboards. Print reuses the saved
Print settings with the chosen artboards, including which layers print and what fits on the
sheet; it also shows the existing print-preview geometry/settings warnings. Locked artwork is
still checked because locking it does not prevent output. Hidden artwork is checked only if the
Print settings include hidden layers.

## Coverage limits

This is a check of the document and selected output context, not validation or certification of
an exported file. PDF export presets, colour conversion, ink coverage, bleed adequacy and
accessibility compliance are not checked by this first version.

Resolution inside symbols, patterns, brushes, opacity masks and placed documents is reported as
**Not checked**, because the resource's local transform alone does not establish its final output
resolution. A resolution check with both findings and unchecked art shows **Needs attention**
and also states that some art was not checked. A vector placed document has no single bitmap PPI.

The font and text checks share the existing output-aware walk, including its bounds/clipping
approximations for generated art and container effects. Resource findings identify a representative
visible owner, not an exhaustive list of every instance. A passed check means only that no problem
of that kind was found within the stated coverage.

The same read-only report is available through `document.preflight`; see [MCP](mcp.md#production-preflight).

## Validation (2026-11-18)

Checked on Windows with Rust 1.99.1, based on `main` at `36b5a5920c`:

- Engine integration tests: 9 passed (`cargo test -p vectorcraft-engine --test preflight`).
- Panel tests: 5 passed (`cargo test -p vectorcraft-ui-egui --lib preflight`), including pointer
  selection/zoom, edits and Undo, document switching, external-file polling and explicit Recheck.
- Both test commands used `--offline --target-dir target --config profile.test.opt-level=0`.
- Existing Print Preview and Links panel engine regressions: 16 + 16 passed, run from the native
  engine test binary with filters `tests::printpreview` and `tests::linkspanel`.
- The ignored `preflight_panel_preview` test was run explicitly, rendered through the offscreen GPU
  harness and visually inspected. Its output remains under the ignored `target/` directory.
- Native Clippy for the engine and UI, all targets, passed with `-D warnings` (explicit
  `--target x86_64-pc-windows-msvc`, using `target/wasm-check` for the validation artifacts).
- Engine/UI WebAssembly typecheck passed. Formatting, asset attribution, branding, dependency
  layering and PDF dependency checks passed.
- Windows desktop build passed (`cargo build -p vectorcraft --offline --target-dir target
  --config profile.dev.opt-level=0`). The running panel was visually inspected with a sample
  exercising all four findings, and the contributor tried it in the desktop app.

`cargo xtask ci` was attempted with `CARGO_BUILD_TARGET=x86_64-pc-windows-msvc`: formatting and
workspace Clippy (all targets, `-D warnings`) passed. It was stopped during workspace-test
compilation after the separate `ui-commands --check` confirmed pre-existing drift between the
unchanged `menus/ui_commands.rs` and generated `mcp/src/ui_catalog.rs`. The complete workspace
test suite and remaining CI stages were not completed. An additional
WebAssembly Clippy run also encounters existing target-specific lints in unchanged `cmd/plugin.rs`
(`drop_non_drop`) and `font_menu.rs` (`unnecessary_lazy_evaluations`); the WebAssembly typecheck and
the native Clippy run above pass without source changes for those findings.

### Review follow-up on `main` at `ee60c51fd`

The exact-threshold and locked-artboard regressions were first run against the original
implementation and failed. After the fixes and merge-conflict resolution:

- All 10 engine preflight tests and 7 panel tests passed. The optional GPU preview also passed
  and was visually inspected.
- The boundary regression checks several exact PPI thresholds and a real shortfall on one axis.
- Navigation regressions cover a locked artboard, retry after unlocking, and partial selection
  that still zooms to the selectable artwork. Finding navigation never unlocks artwork.
- Formatting, native workspace Clippy with warnings denied, engine/UI WebAssembly typecheck,
  layering and PDF dependency checks passed. The complete workspace CI suite remains incomplete; the unchanged UI command catalog
  still fails its separate drift check.

## Revision history

| Date | Change |
|---|---|
| 2026-11-18 | Review follow-up: tolerate PPI rounding at the minimum, report incomplete finding selection, and retain main's symbol lookup improvements when resolving merge conflicts. |
| 2026-11-18 | Initial Production Preflight panel and command. |
