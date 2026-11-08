<!-- craft-house-rules:start core source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=540e6cea75d12d0710c3dca89658328e81e508ecef102446ca5bb95b7c0b92bb -->
## House rules

These apply in every Craft repository. A later rule may be stricter, not weaker. The source is `standards/house-rules-core.md` in `storytold/craft-contrib`. Propose changes there, not in a copy.

1. **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
2. **Check, then claim.** Before writing code, read the issue comments, search pull requests for `#N` and the keywords, and check recent commits on `main`. Comment on the issue first. Release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it. Confirm the cause in the code, even when someone names a location.
4. **Keep fixes narrow and tested.** Fix what the issue describes, where the bug is. Add a test that fails without the fix.
5. **Review the diff before opening a pull request.** Identifiers exist as used, the test fails without the fix, nothing unrelated changed, and the description is true.
6. **Look for a better pull request.** If another fixes the same thing, close yours when it is better or equal, and say why.
7. **Report only what ran.** Say what you tested and what you could not run. Without a local build or test, say so and rely on CI before marking ready.
8. **`main` changes through a pull request.** Do not push to it. Branch protection enforces this. There is no exception.
9. **Stop and ask only about** licensing outside the allowlist, publishing, releases, a new remote, secrets, spending money, destructive git, and anything else hard to undo. Otherwise keep working. `plan/` is a gitignored notebook, not rules. Without it, do not ask for it and do not stop.
10. **Keep secrets and personal details out** of the repository, logs and requests.
11. **CI stays shared.** Workflows call `storytold/craft-common-actions`. Do not change CI in a feature or fix pull request. If CI cannot cover a change, open an issue there.
12. **Guards say why.** A check that looks removable says what it prevents. Read that before changing it.
13. **Instructions must be readable by everyone.** No notes about one machine, and no links to files a reader cannot open. `craftrules` is retired. Do not link to it.
14. **Keep pull requests moving.** Concurrent pull requests are fine; unanswered actionable feedback is not. Respond in 7 days, a reminder at 14, closure eligible at 30, from when an actionable request waits on the author. Suggestions and unconfirmed bot findings do not start the clock. A fix, explanation or agreed plan resets it; unrelated activity does not. Waiting on a reviewer or maintainer pauses it and does not count against the author. A maintainer decides closure; it is never automatic. Reuse search results, do not poll, and stop at a rate limit.
15. **No agent attribution.** A pull request does not say an agent wrote it. A repository rule may not require that.
<!-- craft-house-rules:end -->
<!-- craft-house-rules:start rust source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=387e0ccb4b49e567f60951d7d6fff7305054c89f43ad812b9993dc7c38528013 -->
## Rust Craft repositories

These apply to native Craft apps and to shared Rust libraries. Select them by repository type. A repository that merely contains some Rust does not take this block. Rules later in a repository's file may be stricter, but not weaker. The source is `standards/house-rules-rust.md` in [storytold/craft-contrib](https://github.com/storytold/craft-contrib).

1. **Never crash.** Non-test code does not panic and has no `unsafe`, except where the repository file names an exception. Return errors, treat input as hostile, and bound recursion and allocations. New production crate roots copy the `deny` attribute from an existing crate: `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::unimplemented`, `clippy::todo`, `clippy::unreachable`. Every crash fix comes with a test that panicked before the fix.
2. **Clean room.** Learn from observed behaviour and public specifications. Do not copy another product's code, assets, strings or artwork. Check a dependency's licence before adding it.
3. **Assets and fonts.** Every asset is original or suitably licensed, and is recorded in `ATTRIBUTION.md` in the same change. Font files live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), not in the app repository.
4. **Rust only for native Craft apps.** No handwritten JavaScript or TypeScript, and no Tauri, Electron or webview shell, in those apps. Shared tooling follows its documented technical scope and maintainer-approved exceptions. Craft PR Review's existing Tauri and TypeScript UI is one such exception. It does not relax this rule for native apps. Preserve any existing web build.
5. **Time goes through `chrono`.** Do not hand-roll calendar conversion. For dates beyond what `chrono` can represent, follow [standards/time.md](https://github.com/storytold/craft-contrib/blob/main/standards/time.md).
6. **Contributor credits.** Add or edit only your own person's entry in `contributors/people.toml`, and only after asking them. Never guess a name or hand-edit `contributors.json`.

## Licence allowlist

Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in this list. A repository may name a shorter list. It may not add an identifier here by editing its own copy of these rules.

- `ISC`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `Apache-2.0`
- `OFL-1.1`
- `Ubuntu-font-1.0`
- `Bitstream-Vera`
- `CC0-1.0`
- `CC-BY-4.0`
- `CC-BY-SA-4.0`
- `LicenseRef-Public-Domain`

`LicenseRef-Storyteller-Trademark` is not in this list. It covers Storyteller brand marks only, where a repository already says so.
<!-- craft-house-rules:end -->

# VectorCraft — instructions for agents

VectorCraft is a clean-room, open-source, Rust-native vector illustration app targeting Adobe Illustrator parity (and beyond). It runs natively on macOS, Windows and Linux, and on the web via WASM. Siblings: `../photocraft` (Photoshop-class) and `../pdfcraft` (Acrobat-class); same conventions.

Formerly **DrawCraft** (renamed 2026-10-01): old `.drawcraft` files and `"format": "drawcraft"` headers still open (`vectorcraft_format::LEGACY_EXTENSION`), and preferences migrate from the old config folder. Keep those paths working; use the new name everywhere else.

## Start every session here
1. Read [`ROADMAP.md`](ROADMAP.md) (stage, numbers by dimension, what's next) and the ranked work list in [`docs/gaps.md`](docs/gaps.md). Unless the user gives you a task, pick work from that list.
2. `plan/` is an optional gitignored notebook. Behaviour notes may be in `plan/illustrator/*.md`. Do not ask for it and do not stop when it is absent. Orient, implement, test, verify, then record in the public docs.

## Non-negotiables
- **Clean-room: never use Adobe software (absolute rule, from the project owner).** Adobe's Terms of Use forbid reverse engineering, including observing a program's inputs and outputs to recreate it. So:
  - Never launch, run, script, automate (COM, AppleScript, ExtendScript, UI automation), screenshot or measure Illustrator or any other Adobe application, and never compare our output against one. Don't make test files with them, not even "synthetic" ones.
  - Never read, disassemble or copy anything inside an Adobe install (binaries, plug-ins, presets, procsets, resources, help files, fonts, ICC profiles).
  - Never copy Adobe-authored code or text into this repo, tests included: no PostScript procsets or prologs (AGM, CoolType, the Illustrator prologs), no sample files. Write test inputs from scratch from the public specifications (PostScript Language Reference, PDF / ISO 32000, TIFF, EPSF/DCS).
  - Behaviour comes only from public documentation (helpx, published specs, public forum posts), cited by URL, and from files users send us that they own (used locally, never committed). Never copy Adobe icons, artwork, presets or wording beyond feature names.
  - If a task seems to need any of this, stop and say so instead. `cargo xtask cleanroom` (run by `cargo xtask ci`) fails on Adobe copyright notices and on fixtures made with Adobe applications.
  - Never copy GPL/AGPL code (Inkscape, lib2geom…).
- **Assets: no Adobe iconography or images — ever (absolute rule, from the project owner).**
  - Never add, copy, trace, redraw-from, embed or ship any icon, image, artwork, cursor, preset, swatch/brush/symbol/pattern/style library, ICC profile or screenshot from Adobe products or from any other source whose licence doesn't allow it.
  - Every image, icon, font or other asset must be one of:
    - original work created for VectorCraft by a contributor (who licenses it MIT OR Apache-2.0);
    - OSI open source;
    - public domain / CC0;
    - Creative Commons with redistribution allowed.
  - **Every asset file must have a row in [`ASSETS.md`](ASSETS.md)** (path, author, source URL, licence, notes). Put licence texts next to the assets (e.g. `assets/fonts/OFL-*.txt`) and summarize them in `NOTICE`.
  - `cargo xtask assets` (run by `cargo xtask ci`) fails on any unattributed asset.
  - **Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this repo**. Never commit a font file here; add new fonts to craft-fonts. The app uses it only through the optional `CRAFT_FONTS_DIR` build option (`CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo xtask ci`; use an absolute path), read by `crates/text/build.rs` into `vectorcraft_text::CRAFT_FONTS`; never add it to a `Cargo.toml`. Everything must build, test and run with `CRAFT_FONTS` empty, and tests of its glyphs skip when it is. Details: `docs/development.md` › Fonts.
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
- **Adding a cursor.** The app decides which cursors exist; whoever draws them works from the list. For a new tool state with a cursor of its own: add the `Cursor` variant (`crates/tools/src/lib.rs`, with a doc comment), return it from the tool's `cursor()` with a tool test, cover it in `cursor_icon` (`crates/ui-egui/src/canvas.rs`), draw it in `crates/ui-egui/src/cursors.rs` or give it a stand-in in `Cursor::stand_in()` until it's drawn, and add its `[[cursor]]` entry to `crates/tools/src/cursors.toml` (when it shows, in plain behaviour terms, and its hotspot; never another app's look). A missing entry only warns (`cargo xtask cursors`): the cursor still works.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui/eframe/winit/rfd.
- **The UI is thin**: panels read engine state and act through `app.run(id, params)`. Colours come from `theme::Tokens`.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).
- **Shared test corpora.** Real-file test oracles (Photoshop-authored PSDs, etc.) live in [`storytold/photocraft-corpus`](https://github.com/storytold/photocraft-corpus). Never commit large binary fixtures to this repo; fetch them pinned by commit and sha256-verified, as PhotoCraft does with `cargo xtask corpus`.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D warnings including the no-panic lints, tests, layers, wasm). One task id per commit (`M2.1: pen tool`).

## Running and looking at the app
- `cargo run --release -p vectorcraft -- --control 7979 [file.svg|file.vectorcraft]` (sibling apps' agents use the same default port: if the log says it failed to bind, pick another port — otherwise your requests reach a different app).
- Drive it: JSON lines on `127.0.0.1:7979`, e.g. `{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":10,"y":10,"width":100,"height":50}}}` then `{"id":2,"method":"ui.screenshot","params":{"path":"/tmp/shot.png"}}`. Methods: `crates/ui-egui/src/control.rs`.
- **For UI work, look at the result** (take `ui.screenshot`, read the PNG) and compare with the public documentation (`plan/illustrator/02-ui-ux.md` notes). `ui.screenshot` needs a presented frame: if it errors (screen locked), check the art with `ui.render` / `vectorcraft-cli run … --export x.png`, and cover panels with a headless egui frame test (see `panels/transparency.rs` tests).
- **Performance:** `vectorcraft-cli perf` checks the budgets (render/pan/zoom, hit test, save/open, SVG, Pathfinder) on a synthetic 50k-path document; `vectorcraft-cli bench FILE` times one file. Run them before and after renderer, format or geometry changes, on an idle machine (the report warns when the load average makes timings noise).
- MCP: `vectorcraft-cli mcp` (see `docs/mcp.md`).
- Shell gotcha: `mv`/`cp` are aliased interactive here — use `/bin/mv -f` / `/bin/cp -f`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; write manifests atomically. Each target dir grows to ~30 GB: delete yours when you finish (a full disk fails links with `errno=28`).

## Roadmap and progress docs
The progress docs:
- [`ROADMAP.md`](ROADMAP.md): stage, headline numbers, dimensions, features, languages, upcoming, progress log.
- [`docs/target-app-parity.md`](docs/target-app-parity.md): the authoritative parity assessment (area table, methodology, calibration, evidence of what ships).
- [`docs/gaps.md`](docs/gaps.md): every known shortfall, ranked; the work list. [`docs/roadmap.md`](docs/roadmap.md): milestones and current focus.
- Checklists: [`ui-parity.md`](docs/ui-parity.md), [`file-format-parity.md`](docs/file-format-parity.md), [`hardware-parity.md`](docs/hardware-parity.md), [`localization-parity.md`](docs/localization-parity.md), [`effects-parity.md`](docs/effects-parity.md), [`type-parity.md`](docs/type-parity.md); [`architecture.md`](docs/architecture.md).

When a task lands, update in the same PR: the area row in `docs/target-app-parity.md` (score, missing items, hours) and its evidence list, the gap entry in `docs/gaps.md` if it closed or shrank, the checklist it belongs to, the milestone row in `docs/roadmap.md`, and the ROADMAP's tables and progress log when a headline number moves. Update each touched doc's status line and revision history.
- Grade by behaviour against the public documentation (`plan/illustrator/` notes), not by whether a menu item exists. Scores are self-assessed, so err low; mark each number measured or estimated.
- Keep the README's Status section in step with the ROADMAP headline.

## Contributor credits (About window)

- About ▸ Contributors/Models are compiled into the binary from `contributors/contributors.json`
  (commit stats; never hand-edit) and `contributors/people.toml` (names people chose for
  themselves). See `docs/contributors.md`.
- **Agents working for a contributor:** when you prepare a PR, check whether your human's GitHub
  username has a `[people.<username>]` entry in `contributors/people.toml`. If not, ask them once
  whether they want to be credited by more than their username: a real name, a display name, and/or
  their public GitHub profile name (`sync_github_name = true`). If yes, add **only their own** entry
  (copy the template at the top of the file)
  and include it in their PR, committed as them. If no, change nothing: they are credited as
  `@username` anyway.
- Never add, edit, guess or copy anyone else's entry or name (not from git config, commit authors or
  GitHub profiles). Never hand-edit `contributors.json`.

## CI workflows
- Workflows in `.github/workflows/` call the shared workflows in [`storytold/craft-common-actions`](https://github.com/storytold/craft-common-actions) at `@main`, with their inputs and secrets. No other jobs: if no shared workflow does what's needed, it doesn't go here.
- Don't add or change CI in a feature or fix PR. If the existing workflows can't build or test a change, link an existing craft-common-actions issue or PR, or open a new issue, or a PR only for a small fix that is safe for every app and does not increase CI time.
- Some workflow files here still run their own jobs. Don't add to them. Moving a job into craft-common-actions is its own task, never part of another PR, and the job stays here until a shared workflow runs it.

New workspace crates include `[lints] workspace = true` in `Cargo.toml`. Production crate roots declare the crate-level lint attributes before modules; crate-level `//!` documentation may precede them.
