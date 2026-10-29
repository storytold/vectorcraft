# Quick Look extensions

Finder's thumbnails and the Space-bar preview of VectorCraft documents (`.vectorcraft`, and
`.drawcraft` from before the rename), issue #663. Two app extensions inside
`VectorCraft.app/Contents/PlugIns`, written in Rust with objc2 (no Swift or Objective-C):

| Bundle | Extension point | Principal class | What it shows |
|---|---|---|---|
| `VectorCraftThumbnail.appex` | `com.apple.quicklook.thumbnail` | `VCThumbnailProvider` (`QLThumbnailProvider`) | the PNG a save embeds, scaled to the size Finder asks for |
| `VectorCraftPreview.appex` | `com.apple.quicklook.preview` (data-based, macOS 12+) | `VCPreviewProvider` (`QLPreviewProvider`) | the first artboard rendered at most 2048 px a side within 5 s, else the embedded PNG |

Both serve the type `ai.storyteller.vectorcraft.document` that `../Info.plist.in` exports.

## Files

- `crates/quicklook`: the read path (`src/lib.rs`, platform-neutral, tested and fuzzed on Linux) and
  the Objective-C side (`src/mac/`, macOS only). Binaries `vectorcraft-ql-thumbnail` and
  `vectorcraft-ql-preview`; on other systems they are stubs.
- `Thumbnail-Info.plist.in`, `Preview-Info.plist.in`: the bundles' `Info.plist`, filled in (`@VERSION@`,
  `@SHORT_VERSION@`) by `../package.sh` and `cargo xtask bundle`.
- `entitlements.plist`: app extensions must be sandboxed; they read only the file Quick Look hands
  them.
- `make-test-files.sh`, `check-thumbnails.sh`: what `.github/workflows/quicklook-macos.yml` runs on a
  macOS runner after installing the ad-hoc-signed app, with `crates/quicklook/examples/ql_probe.rs`.

## Thumbnails need the embedded preview

The thumbnail is the PNG saved in the file (File Handling → Embed Preview, on by default since
#663; `document.save {preview}`). Files saved with it off, or by a VectorCraft from before, show
the document icon until they are saved again. The thumbnail extension never reads the document,
so a thumbnail costs as little as reading the file.

## Failure behaviour

A file the extensions can't show is an error for Quick Look (an `NSError` in the domain
`ai.storyteller.vectorcraft.quicklook`), and Finder shows the document icon:

| Code | Why |
|---:|---|
| 1 | the file can't be read |
| 2 | the file is larger than 512 MB |
| 3 | a newer version than this VectorCraft reads |
| 4 | gzip that unpacks past what the extensions read (below) |
| 5 | not a VectorCraft document (damaged, cut short, another format) |
| 6 | no embedded preview (thumbnails only) |
| 7 | the embedded preview isn't a PNG the extension reads (at most 4096 px a side) |
| 8 | nothing to show |
| 9 | rendering ran out of time (and there is no embedded preview) |
| 10 | rendering failed (an internal error included: a panic is caught, never unwinds into Quick Look) |
| 11 | a preview that ran out of time is still rendering (and there is no embedded preview) |

A file is read within caps tighter than VectorCraft's own when it opens a document
(`vectorcraft_format::MAX_JSON`, 1 GiB, and `MAX_BLOBS`, 4 GiB), since Finder asks for thumbnails
of every file it shows: 256 MB of JSON and 256 MB of data after it for the embedded preview
(`THUMBNAIL_CAPS`), and 64 MB of JSON and 256 MB of images to render the Space-bar preview
(`Limits::caps`; past them, the embedded preview shows).

A preview render that runs out of time can't be stopped: it finishes on its own thread, unseen.
While one is still running, the next preview shows the embedded PNG instead of starting another
render (`Limits::max_late`), so paging through large documents doesn't pile renders up.

## Signing

`../package.sh` signs each extension (executable, then bundle) with the hardened runtime and these
entitlements before it signs the app, so `codesign --verify --strict --deep` covers them. With a
real identity, notarisation is unchanged: the app's zip and the DMG are submitted as before, and
the extensions inside are notarised with the app. Ad-hoc signing (no `MACOS_SIGN_IDENTITY`) is
enough to load them on the Mac that built them, which is what the workflow checks.

## Trying it on a Mac

```sh
cargo xtask bundle                                    # dist/VectorCraft.app, signed ad hoc
ditto dist/VectorCraft.app /Applications/VectorCraft.app
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f -R -trusted /Applications/VectorCraft.app
qlmanage -r && qlmanage -r cache
pluginkit -m -v -i ai.storyteller.vectorcraft.thumbnail   # registered (-p com.apple.quicklook.thumbnail lists nothing on macOS 15)
cargo run --release -p vectorcraft-quicklook --example ql_probe -- some.vectorcraft /tmp/some.png
log show --predicate 'subsystem contains "quicklook"' --last 10m
```

`ql_probe` asks QuickLookThumbnailing for the thumbnail, as Finder does. `qlmanage` doesn't reach
these extensions on macOS 15: `qlmanage -t` gets no answer for the document type (while it does
for a PNG, and the extension answers the probe in a tenth of a second), and `qlmanage -p` can't
host a data-based preview extension (it stops on a fault in ExtensionFoundation, "no extension
context host class"). Check the Space-bar preview in Finder. The workflow records both.
