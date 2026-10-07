# VectorCraft — instructions for agents

VectorCraft is a clean-room, open-source, Rust-native vector illustration app targeting Adobe Illustrator parity (and beyond). It runs natively on macOS, Windows and Linux, and on the web via WASM. Siblings: `../photocraft` (Photoshop-class) and `../pdfcraft` (Acrobat-class); same conventions.

Formerly **DrawCraft** (renamed 2026-10-01): old `.drawcraft` files and `"format": "drawcraft"` headers still open (`vectorcraft_format::LEGACY_EXTENSION`), and preferences migrate from the old config folder. Keep those paths working; use the new name everywhere else.

## Start every session here
1. Read the [honest assessment in `ROADMAP.md`](ROADMAP.md#honest-assessment-2026-10-05): where we stand by dimension, **where we're lacking** (the prioritized gap list) and **where we're going**. Unless the user gives you a task, pick work from that list.
2. Read `plan/STATUS.md` (local session notes, may lag the ROADMAP), then the task in `plan/execution-plan.md` §3 and the relevant `plan/architecture.md` section. Behaviour reference: `plan/illustrator/*.md`.
3. Follow the autonomous operation protocol (`plan/execution-plan.md` §7): orient → plan → implement + test → verify → record → commit. Don't stop to ask unless §7 lists the decision as the user's.

`plan/` is gitignored (local only).

## Non-negotiables
- **Clean-room.** Never read, disassemble or copy anything inside the Illustrator bundle (names/listings only). Never copy Adobe icons, artwork, presets or wording beyond feature names. Behaviour comes from public docs and black-box observation of the running app with synthetic documents only (screenshots by window id, stored under `plan/illustrator/screenshots/`, never committed). Never copy GPL/AGPL code (Inkscape, lib2geom…).
- **Assets: no Adobe iconography or images — ever (absolute rule, from the project owner).**
  - Never add, copy, trace, redraw-from, embed or ship any icon, image, artwork, cursor, preset, swatch/brush/symbol/pattern/style library, ICC profile or screenshot from Adobe products or from any other source whose licence doesn't allow it.
  - Every image, icon, font or other asset must be one of:
    - original work created for VectorCraft by a contributor (who licenses it MIT OR Apache-2.0);
    - OSI open source;
    - public domain / CC0;
    - Creative Commons with redistribution allowed.
  - **Every asset file must have a row in [`ASSETS.md`](ASSETS.md)** (path, author, source URL, licence, notes). Put licence texts next to the assets (e.g. `assets/fonts/OFL-*.txt`) and summarize them in `NOTICE`.
  - `cargo xtask assets` (run by `cargo xtask ci`) fails on any unattributed asset.
  - **Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this repo** (rules: craftrules [`standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md)). Never commit a font file here; add new fonts to craft-fonts. The app uses it only through the optional `CRAFT_FONTS_DIR` build option (`CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo xtask ci`; use an absolute path), read by `crates/text/build.rs` into `vectorcraft_text::CRAFT_FONTS`; never add it to a `Cargo.toml`. Everything must build, test and run with `CRAFT_FONTS` empty, and tests of its glyphs skip when it is. Details: `docs/development.md` › Fonts.
  - Code and original assets are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The ArtCraft logos in `docs/brand/` are trademarks, not open source, under `docs/brand/LICENSE-brand.txt`: never modify them or use them outside VectorCraft.
  - Prefer art generated in code for defaults (swatches, brushes, symbols, patterns, cursors). If the provenance of an asset is unclear, don't add it.
  - Reference screenshots of Illustrator stay local under the gitignored `plan/` and are never committed, published or used as assets.
- **Never crash: no panics in shipped code.** A crash loses the user's unsaved work. Files, pasted data, MCP and control messages, and command params are all untrusted input.
  - Fallible work returns `Result` (or `Option`); callers handle it or propagate it with `?`. Failures reach the user as an error message (`EngineError` for commands, the crate's own error type or `String` below the engine), never as an abort.
  - Workspace clippy lints ban these outside tests, so `cargo xtask ci` fails on them: `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`. Use instead: `?`, `.ok_or(…)?`, `let … else { return … }`, `if let`, `unwrap_or` / `unwrap_or_default` / `unwrap_or_else`, `.first()` / `.last()` / `.split_last()`, and a real error for a match arm you believe is impossible.
  - Don't panic implicitly either: use `.get(i)` instead of `v[i]` or `&s[a..b]` when the index comes from data. Don't divide integers by something that can be zero, cap sizes and counts read from input (allocations, images, loops), and check that floats are finite before casting them or looping on them.
  - Don't silence errors just to satisfy the lint: use `let _ =` or `.ok()` only where ignoring the failure is the right behaviour, with a comment saying why.
  - Tests may unwrap and panic (`#[test]`, `#[cfg(test)]`, `tests/`, `testkit`). Integration-test files start with an `#![allow(…)]` for that.
  - `vectorcraft_engine::guard` is the last line of defence. Every entry point (commands, tool events, MCP requests, the UI frame, control requests) catches a panic, rolls the document back and reports an internal error. It exists for bugs and doesn't replace `Result`: wasm aborts on panic, so the web app has no safety net.
  - Untrusted input is fuzzed (`engine/tests/import_fuzz.rs`, `engine/tests/command_sweep.rs`, `format/tests/prop_format.rs`, `mcp/tests/protocol_props.rs`). Extend these when you add an importer, a parser or a protocol method.
- **Everything is a command.** User-visible behaviour = a command in `crates/engine/src/cmd/*` (id, label, menu path, shortcut, params doc, `enabled`, `run`) + tests. Tools emit commands (Begin/Preview/Commit). UI-only commands live in `crates/ui-egui/src/menus.rs` (`UI_COMMANDS`). The control channel and MCP reach all of them.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui/eframe/winit/rfd.
- **The UI is thin**: panels read engine state and act through `app.run(id, params)`. Colours come from `theme::Tokens`.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).
- **Shared test corpora.** Real-file test oracles (Photoshop-authored PSDs, etc.) live in [`storytold/photocraft-corpus`](https://github.com/storytold/photocraft-corpus), explained in [craftrules `standards/test-corpora.md`](https://github.com/storytold/craftrules/blob/main/standards/test-corpora.md). Never commit large binary fixtures to this repo; fetch them pinned by commit and sha256-verified, as PhotoCraft does with `cargo xtask corpus`.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D warnings including the no-panic lints, tests, layers, wasm). One task id per commit (`M2.1: pen tool`).
- **Contributor credits are compiled in.** Help ▸ About ▸ Contributors/Models come from `contributors/contributors.json`, baked into the binary by `crates/ui-egui/build.rs` (never read at run time). Regenerate it with `python3 ../craftrules/scripts/contributors.py .` and commit it; never hand-edit it. GitHub usernames only; display and real names only with consent recorded in craftrules `contributors/people.toml`. See `docs/contributors.md`.

## Running and looking at the app
- `cargo run --release -p vectorcraft -- --control 7979 [file.svg|file.vectorcraft]` (sibling apps' agents use the same default port: if the log says it failed to bind, pick another port — otherwise your requests reach a different app).
- Drive it: JSON lines on `127.0.0.1:7979`, e.g. `{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":10,"y":10,"width":100,"height":50}}}` then `{"id":2,"method":"ui.screenshot","params":{"path":"/tmp/shot.png"}}`. Methods: `crates/ui-egui/src/control.rs`.
- **For UI work, look at the result** (take `ui.screenshot`, read the PNG) and compare with `plan/illustrator/02-ui-ux.md` / `10-observed-ui.md`. `ui.screenshot` needs a presented frame: if it errors (screen locked), check the art with `ui.render` / `vectorcraft-cli run … --export x.png`, and cover panels with a headless egui frame test (see `panels/transparency.rs` tests).
- **Performance:** `vectorcraft-cli perf` checks the budgets (render/pan/zoom, hit test, save/open, SVG, Pathfinder) on a synthetic 50k-path document; `vectorcraft-cli bench FILE` times one file. Run them before and after renderer, format or geometry changes, on an idle machine (the report warns when the load average makes timings noise).
- MCP: `vectorcraft-cli mcp` (see `docs/mcp.md`).
- Shell gotcha: `mv`/`cp` are aliased interactive here — use `/bin/mv -f` / `/bin/cp -f`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; write manifests atomically. Each target dir grows to ~30 GB: delete yours when you finish (a full disk fails links with `errno=28`).

## Roadmap
`ROADMAP.md` (committed) is the shared picture of where VectorCraft stands. It holds status, the honest assessment (by dimension, the gap list, the direction), milestones, the parity table and time-to-parity estimates.
- When a task lands, update it in the same PR: the milestone row, the parity-table row (score, missing items, hours), "Shipped so far", and the gap list if the gap closed or shrank.
- Grade by behaviour against `plan/illustrator/`, not by whether a menu item exists. Scores are self-assessed, so err low.
- Keep the README's Status section in step with the ROADMAP headline.
