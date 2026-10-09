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

Any static file server works for `dist/web`, for example `python3 -m http.server 8766` inside that directory. The release `.wasm` is about 17.5 MB, or 7.1 MB gzipped, so serve it with compression.

URL flag: `?webgl` forces the WebGL2 backend.

How the web shell (`apps/vectorcraft-web/src/web.rs`) differs from desktop:

- **Open** sets `Services::open_async`, which shows `rfd::AsyncFileDialog`. The bytes arrive in `Services::inbox`, which the app drains every frame.
- **Save / Save As / Export** go through `Services::download`: a Blob, an object URL and a temporary `<a download>`, all created from Rust. There is no save dialog, so the suggested name becomes the download name.
- **Drag-and-drop:** `WebShell` takes the frame's `dropped_files` before the app sees them, reads each with `DroppedFile::bytes_async` and pushes the bytes into the inbox. (The app's synchronous drop path is compiled out on wasm32.)
- **No control server:** browsers can't listen on TCP. To automate the web build, drive headless Chrome with `--remote-debugging-port`.
- Quick smoke test: `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --enable-unsafe-webgpu --screenshot=web.png --window-size=1440,900 --virtual-time-budget=15000 http://127.0.0.1:8766/` (headless Chrome on macOS gets a real WebGPU adapter).

## Fonts: craft-fonts (optional build input)

Font files are never committed to this repository. Fonts shared by the Crafting Apps live in
[storytold/craft-fonts](https://github.com/storytold/craft-fonts); the rules are in craftrules
[`standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md). To add
a font, add it there. (The Latin fonts in `assets/fonts/` predate the rule and stay.)

Vector W3K2 builds, tests and runs without craft-fonts. To embed its Japanese fonts (BIZ UDPGothic
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

## Localisation

Strings in code stay English and are the default lookup keys. `crates/ui-egui/src/i18n` maps them to display
text at render time from one catalog per language (`i18n/<code>.tsv`; the format is documented in the header of
`zh-hant.tsv`). Command ids, menu paths used for logic, `ui.menu.list`, the control channel, the CLI and MCP
always use the English ids and labels, so agents and scripts never see translated text.

Languages shipped: English (`en`, the source), Traditional Chinese (`zh-hant`, complete, in the vocabulary used
in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` locales all resolve to it), Czech (`cs`, every menu
label) and Japanese (`ja`, the main menus so far). Untranslated text falls back to English until its rows are
added. Simplified Chinese locales
(`zh-CN`, `zh-SG`, `zh-Hans`) fall back to English until a `zh-hans` catalog is registered: the resolver already
tells the two scripts apart, so the Traditional catalog is never shown to a Simplified locale.

- `tl!("…")` translates a literal into the language the UI is drawn in; `i18n::t(s)` is the same for a
  `&str`. `tr(lang, s)` takes the language; `tr_ctx` when one English word needs different translations;
  `tr_id(lang, command_id, label)` for menu items (keyed by command id, English label as the fallback);
  `tn` / `trn(lang, n, one, other)` for plurals; `fmt` fills `{name}` placeholders, which a translation may
  reorder.
- The shared widgets (`widgets::check`, `dropdown`, `menu_item`, the buttons, `label_row`, tooltips of
  `icon_button`…), the menus, the dock, the toolbar, the dialog frame and `panels::empty_state` translate
  the text they are given, so a panel mostly needs its literals wrapped in `tl!` to be covered by the tests.
- The language is Vector W3K2 › Language (the `app.language` UI command, `{lang: auto|<code>}`) or Edit ›
  Preferences › User Interface › Language; both set the `interfaceLanguage` preference (`auto` or a language
  code; `auto` follows the system locale: `VECTORCRAFT_LOCALE`, then `LC_ALL`/`LC_MESSAGES`/`LANG`/`LANGUAGE`,
  the macOS preferred languages, the Windows user locale). The web build has no locale detection yet and
  starts in English. The Preferences dialog previews the chosen language before OK.
- To add a language: add `<code>.tsv` and one row in `i18n::LANGUAGES` (code, native name, catalog, plural
  rule). The Language menu, the dropdown, locale matching and the catalog tests (well-formed, no duplicates,
  placeholders and ellipses agree, command ids exist, every row of a partial catalog is a string the UI
  shows, the UI fonts have every glyph, no Simplified characters in `zh-hant`) pick it up. Set `complete_menus`
  once every menu string and `tl!` literal is translated; `i18n::tests` then enforce it
  (`VECTORCRAFT_I18N_DUMP=strings.txt cargo test -p vectorcraft-ui-egui dump_source_strings` lists them).
- Translations are clean-room: written from the meaning of the English text in ordinary vocabulary, never
  from another product's localisation resources. Product and technology names stay in Latin letters.
- Not translated on purpose: status-bar messages and errors (agents and tests read them), names that are
  user data (layers, swatches, fonts, documents), the tab title's colour mode. Not done yet: locale-aware
  number and date formats, right-to-left layout, locale detection on the web. Chinese and Japanese UI text
  is drawn with craft-fonts' BIZ UDPGothic when the build embeds it (see Fonts above), else with an installed
  system font; the glyph test checks Latin-script catalogs always and the CJK ones only with craft-fonts.
  A Traditional Chinese UI font is still to be added to craft-fonts for the web build.

## Vendor names gate

`cargo xtask brands` (part of `cargo xtask ci`) fails when user-visible text names another vendor's products or company: string literals in Rust sources (command labels and params docs, menus, panels, MCP tool definitions), `Cargo.toml` descriptions and packaging files. Comments and test code are not checked. Say "the reference app" or name the feature itself. A line that must keep an old name, such as an alias that files or preferences from earlier versions still use, carries a `brand-ok` comment.

## Robustness: Vector W3K2 never crashes

A crash takes the user's unsaved work with it, and much of what the app reads is untrusted: SVG,
PDF, `.ai` and `.vectorcraft` files, pasted data, MCP and control-channel messages, command
parameters. Shipped code therefore never panics. Anything that can fail returns `Result` (or
`Option`), the caller handles or propagates it, and the user sees an error message.

### Enforced by lints

`Cargo.toml` denies these clippy lints for the whole workspace, and `cargo xtask ci` runs clippy with
`-D warnings`:

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
  (`.vcswatches`, `.gpl`), graphic style (`.vcstyles`) and flattener preset (`.vcflattener`)
  libraries, loaded and used, then rendered and exported.
- `crates/engine/tests/command_sweep.rs`: every command with junk parameters.
- `crates/format/tests/prop_format.rs`: garbage and mutated `.vectorcraft` files.
- `crates/mcp/tests/protocol_props.rs`: malformed MCP messages.

CI runs a few dozen cases each. Before touching an importer, run a deeper search, e.g.
`PROPTEST_CASES=20000 cargo test --release -p vectorcraft-engine --test import_fuzz`. When it finds a
panic, fix the code and add the input as a regular test.
