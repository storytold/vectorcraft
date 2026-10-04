# Development

## Never crash

Non-test code never panics: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` or `unsafe`; errors go through `Result` and `?`, and every crash fix comes with a regression test. See the **Never crash** section of [`AGENTS.md`](../AGENTS.md) for the rules.

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

## Vendor names gate

`cargo xtask brands` (part of `cargo xtask ci`) fails when user-visible text names another vendor's products or company: string literals in Rust sources (command labels and params docs, menus, panels, MCP tool definitions), `Cargo.toml` descriptions and packaging files. Comments and test code are not checked. Say "the reference app" or name the feature itself. A line that must keep an old name, such as an alias that files or preferences from earlier versions still use, carries a `brand-ok` comment.
