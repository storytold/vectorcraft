# Hosting VectorCraft for the web

`vectorcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `vectorcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `vectorcraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `vectorcraft-web-<hash>_bg.wasm` | The app, about 13 MB, or 5 MB with compression |
| `vectorcraft-plugin-worker_loader.js`, `vectorcraft-plugin-worker.js`, `vectorcraft-plugin-worker_bg.wasm` | The plug-in worker, which checks plug-ins and runs filters off the page. These names carry no hash. |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/vectorcraft-web/Trunk.toml`),
so the site works at a domain root (`https://example.com/`), under a prefix
(`https://example.com/tools/vectorcraft/`) and from a CDN bucket. The app's asset names carry a
content hash, so they can be cached forever. `index.html` and the three `vectorcraft-plugin-worker`
files need revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  download from about 13 MB to about 5 MB. You can also precompress (`brotli -k *.wasm`) and let
  the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed
  `vectorcraft-web-<hash>` `.wasm` and `.js` files, and `no-cache` on `index.html` and the
  `vectorcraft-plugin-worker` files. The page refuses a cached worker whose messages differ from
  its own, and plug-ins then fail to install and run until the browser fetches the new worker.
- **Content Security Policy:** a site that sends one must allow three things
  ([CSP Level 3](https://www.w3.org/TR/CSP3/)):
  - The inline `<script type="module">` that Trunk writes into `index.html` to start the app.
    `script-src 'self'` alone blocks it, and the app does not start. A hash of that script in
    `script-src` allows it, or in `script-src-elem` when the policy sets that directive. The
    script changes with each build, so the hash must be taken from the build you deploy.
  - The page and plug-in worker policies can allow WebAssembly compilation with
    `'wasm-unsafe-eval'` in `script-src`, or in `default-src` when `script-src` is absent.
    A policy that blocks WebAssembly prevents the app from starting and plug-ins from installing
    and running.
  - The plug-in worker: `worker-src 'self'`. Without `worker-src`, the browser uses
    `child-src`, then `script-src`, then `default-src`, so a `script-src` that allows the
    site's own scripts is enough only when the policy sets neither `worker-src` nor
    `child-src`. When the policy blocks the worker, the app runs, and plug-ins fail to install
    and run.
- **HTTPS:** WebGPU (and the clipboard) only work in a secure context, which means `https://`
  or `http://localhost`. Over plain HTTP elsewhere, the app falls back to WebGL2.
- **No special isolation headers:** VectorCraft doesn't use `SharedArrayBuffer`, so it doesn't
  need `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy`. If your site already sends
  COEP `require-corp`, also send `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin`
  when the files live on a CDN) on the app's files.

nginx example:

```nginx
location /vectorcraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; }
    gzip on;
    gzip_types application/wasm text/javascript text/html;
    location ~* /vectorcraft-web-[^/]*\.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* (index\.html|/vectorcraft-plugin-worker[^/]*)$ { add_header Cache-Control "no-cache"; }
}
```

Local test: `python3 -m http.server 8765` inside the folder, then open http://localhost:8765/.

## Browser requirements

The wasm is built with WebAssembly SIMD (`-C target-feature=+simd128`, `.cargo/config.toml`,
#2786), so it needs a browser that has it:

| Browser | First version |
|---|---|
| Chrome, Edge | 91 |
| Firefox | 89 |
| Safari (macOS and iOS/iPadOS) | 16.4 |

Source: caniuse `wasm-simd` and the WebAssembly feature table (webassembly.org/features). An older
browser can't compile the module. There is no fallback build and no message: the page stays on
"Loading VectorCraft..." and the console shows a WebAssembly compile error. The app also needs
WebGPU or WebGL2 (see "Renderer selection" below). If you must serve older browsers, put a
version check in front of the app on your own page.

## Fonts folder

Builds made with craft-fonts (`CRAFT_FONTS_DIR`, all releases) have a `fonts/` folder beside the
wasm: the Arabic fonts the app fetches instead of embedding (about 8 MB, each next to its OFL
licence). Serve it with the rest of the site. Noto Sans Arabic is fetched before the app starts;
the other families load in the background and appear in the font menus as they arrive.

- Serve `.ttf` as `font/ttf` (the browser checks each file's SHA-256, so a wrong type or a
  rewritten file makes the app skip that font and log `web font …` in the console).
- The folder names are the files' SHA-256 prefixes, so `fonts/*` can be cached forever
  (`Cache-Control: public, max-age=31536000, immutable`); `_headers` and `.htaccess` do this.
- Without the folder the app still runs, but Arabic text shows as boxes.

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/vectorcraft/"
  title="VectorCraft vector illustration app"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen; clipboard-read; clipboard-write"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. Preferences are kept in the iframe's `localStorage`. Browsers
  that partition or block third-party storage may forget them between visits, and the app
  then starts with defaults.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without
  `allow-same-origin` there's no storage. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

## Renderer selection and fallback flags

VectorCraft renders with wgpu. It uses **WebGPU** when the browser has it and falls back to
**WebGL2** on its own. URL query flags override this, and they work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |
| `?cpu` | Force the CPU canvas path (slowest, most compatible) |

For example: `<iframe src="https://example.com/vectorcraft/?webgl" ...>`.

A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
