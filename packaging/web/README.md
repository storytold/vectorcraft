# Hosting VectorCraft for the web

`vectorcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `vectorcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `vectorcraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `vectorcraft-web-<hash>_bg.wasm` | The app, about 13 MB, or 5 MB with compression |
| `_headers`, `.htaccess`, `nginx.conf` | Sample server configs for Netlify/Cloudflare Pages, Apache, and Nginx |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Docker

> The Docker recipe (`Dockerfile`, `.dockerignore`, `packaging/web/nginx.conf`) is
> **community-maintained**: it is not built in CI or used for releases, so it can lag behind the
> official web build above. Fixes are welcome.

From the repository root (Docker is the only build prerequisite):

```sh
docker build --load -t vectorcraft-web:local .
docker run -d --name vectorcraft-web --restart unless-stopped \
  -p 8080:8080 vectorcraft-web:local
```

Open **http://localhost:8080/**. The multi-stage `Dockerfile` builds the existing Rust/Wasm
app with Trunk, then copies only the static site into NGINX.

### Docker Compose

If you prefer Compose, run these commands from the repository root. Docker Compose builds the
image from the local source tree, publishes port `8080`, and restarts the container unless it
has been explicitly stopped:

```sh
docker compose up --build -d
```

Open **http://localhost:8080/** to use VectorCraft. To check the service status and health check:

```sh
docker compose ps
docker compose logs -f vectorcraft-web
```

Stop and remove the container with:

```sh
docker compose down
```

The included `compose.yaml` (and `docker-compose.yml`) uses the local image name `vectorcraft-web:local` and maps host port
`8080` to container port `8080`. If port 8080 is already in use, set `PORT=8081` (e.g. `PORT=8081 docker compose up -d`) or change the host side of the
mapping in `compose.yaml`, for example `"8081:8080"`, and then
open **http://localhost:8081/**. The first build downloads the Rust and WebAssembly toolchain
and dependencies, so it can take several minutes. The
runtime runs as the `nginx` user on port **8080**, with a `/healthz` endpoint and Docker
health check. It serves precompressed gzip assets, the Wasm MIME type, immutable hashed
assets, and an HTML page that revalidates after deployments. No Rust installation, Node.js,
database, GPU passthrough, or document volume is needed on the host.

The first build downloads Rust dependencies and Trunk's Wasm tools and can take several
minutes. Subsequent builds reuse Cargo caches. The builder supports Linux amd64 and arm64;
the default Rust version is the stable release CI used when the `Dockerfile` was last updated.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/vectorcraft-web/Trunk.toml`),
so the site works at a domain root (`https://example.com/`), under a prefix
(`https://example.com/tools/vectorcraft/`) and from a CDN bucket. The asset names carry a content
hash, so they can be cached forever. Only `index.html` needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  download from about 13 MB to about 5 MB. You can also precompress (`brotli -k *.wasm`) and let
  the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
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
    location ~* \.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* index\.html$ { add_header Cache-Control "no-cache"; }
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
