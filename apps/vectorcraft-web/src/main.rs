//! VectorCraft in the browser.
//!
//! Runs the same [`vectorcraft_ui_egui::VectorcraftApp`] as the desktop app through eframe's web
//! runner (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release`
//! from this directory; see `docs/development.md` ("Web build").
//!
//! Differences from the desktop app:
//! - no TCP control server (browsers can't listen on sockets);
//! - File → Open uses the browser file picker; bytes arrive asynchronously through
//!   `Services::inbox`;
//! - Save / Export trigger a browser download;
//! - dropped files are read asynchronously by `web::WebShell` and delivered through the inbox;
//! - pen and finger input is read from Pointer Events by `pencil` (Apple Pencil pressure and
//!   hover, palm rejection, fingers navigate once a pen has been seen, the pen's eraser end
//!   selects the Eraser tool);
//! - a Cmd or Ctrl chord the keymap binds runs its command, not the browser's own action as well
//!   (`Cmd+=` zooms the canvas, not the page; `Cmd+Z` undoes in the document) (`keys`).
//!
//! URL query flags: `?webgl` forces the WebGL2 backend instead of WebGPU; `?lang=es` (any
//! language code) stands in for the browser's languages when the interface language is Automatic.

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod fonts;
#[cfg(any(target_arch = "wasm32", test))]
mod keys;
#[cfg(target_arch = "wasm32")]
mod locks;
#[cfg(any(target_arch = "wasm32", test))]
mod pencil;
#[cfg(any(target_arch = "wasm32", test))]
mod saved_files;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("vectorcraft-web only runs in the browser: build it with `trunk build --release` in apps/vectorcraft-web");
}
