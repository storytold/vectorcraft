# Releasing VectorCraft

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
installers for macOS, Windows, Linux (including Flatpak bundles) and FreeBSD, plus the web build, and creates or updates a
**draft** GitHub Release named `VectorCraft v<version>`. Nobody sees a draft until a maintainer
publishes it.

User-facing names say **VectorCraft**. Files, binaries and ids stay lowercase
(`vectorcraft-<version>-<platform>-<arch>.<ext>`, `ai.storyteller.vectorcraft`).

## Cutting a release

1. **Bump the version** on `main`. It lives in exactly one place, `[workspace.package] version`
   in the root `Cargo.toml`; every crate inherits it and the packaging scripts read it from there:

   ```sh
   cargo xtask version                 # prints the current version, e.g. 0.3.1
   cargo xtask version set 0.4.0       # or 0.4.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit the change (`Cargo.toml` + `Cargo.lock`) through the normal review flow, as a
   `Release: VectorCraft v0.4.0` commit whose message says what changed for users.
2. **Merge `main` into `release`** (or fast-forward it) and push. The workflow starts by itself.
3. **Wait for the draft.** When every job is done (macOS notarization is the slow part), the
   Releases page has a draft `VectorCraft v0.4.0`, targeting the pushed commit, with every
   artifact and `SHA256SUMS.txt`. The notes are generated from the merged pull requests.
4. **Check it.** Download an installer or two and read the job summaries. A `::warning::` there
   means a signing secret was missing and that artifact is unsigned.
5. **Publish** the draft in the GitHub UI. Publishing creates the `v0.4.0` tag. A version with a
   pre-release suffix (`-rc.1`) is marked as a pre-release.

Pushing to `release` again before you publish rebuilds the same draft and replaces its assets.
Once the draft is published, the workflow refuses to touch that version again: bump it first.

**Test runs:** *Actions › Release › Run workflow* runs the whole pipeline by hand. The optional
`version` input (such as `0.4.0-rc.1`) overrides `Cargo.toml` for that run only; each build job
applies it with `cargo xtask version set` before building. The signing jobs (macOS, Windows) and
the draft-release job run in the `release` environment, which only the `release` branch can use, so
pick that branch for a full run. Run on any other branch, it is a dry run: the Linux, Flatpak,
FreeBSD and web jobs build and upload their artifacts, the environment refuses the signing jobs,
and no draft release is created.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| macOS 11+ (universal: Apple silicon and Intel) | `vectorcraft-<v>-macos-universal.dmg`, `vectorcraft-cli-<v>-macos-universal.zip` | `macos-15` |
| Windows x64 | `vectorcraft-<v>-windows-x64.msi`, `vectorcraft-<v>-windows-x64-portable.zip` | `windows-latest` |
| Windows x86 (32-bit) | `vectorcraft-<v>-windows-x86.msi`, `vectorcraft-<v>-windows-x86-portable.zip` | `windows-latest` |
| Windows on ARM64 | `vectorcraft-<v>-windows-arm64.msi`, `vectorcraft-<v>-windows-arm64-portable.zip` | `windows-latest` (cross-compiled) |
| Linux x86_64 | `vectorcraft-<v>-linux-x86_64.{AppImage,deb,rpm,tar.gz}` | `ubuntu-22.04` |
| Linux aarch64 | `vectorcraft-<v>-linux-aarch64.{AppImage,deb,rpm,tar.gz}` | `ubuntu-22.04-arm` |
| Linux riscv64 | `vectorcraft-<v>-linux-riscv64.tar.gz` | cross-compiled on `ubuntu-26.04` |
| Linux AppImage updates | `vectorcraft-<v>-linux-{x86_64,aarch64}.AppImage.zsync` | with the AppImages |
| Flatpak x86_64, aarch64 | `vectorcraft-<v>-linux-{x86_64,aarch64}.flatpak` | `ubuntu-24.04`, `ubuntu-24.04-arm` (repackages the Linux tarball) |
| FreeBSD 14 x86_64 | `vectorcraft-<v>-freebsd-x86_64.tar.gz` | a FreeBSD 14.3 VM on `ubuntu-latest` |
| Web | `vectorcraft-web-<v>.zip`, a static site (see [`packaging/web/README.md`](../packaging/web/README.md)) | `ubuntu-latest` |

`vectorcraft --version` and `vectorcraft-cli --version` print the version from `Cargo.toml`.

**Fonts.** Every build job checks out [craft-fonts](https://github.com/storytold/craft-fonts) at a
pinned commit (the `ref:` of its craft-fonts checkout step) and builds with `CRAFT_FONTS_DIR` and
`CRAFT_FONTS_REQUIRED=1`, so releases embed its Japanese fonts and fail rather than ship without
them (the web build embeds only the UI font, BIZ UDPGothic Regular, to stay small). The packages carry each embedded font's licence as `OFL-<family>.txt`
(`copy_font_licences` in `packaging/env.sh`). Bump the pins deliberately, in every job at once.
The rules are in craftrules `standards/fonts.md`, the build option in
[`docs/development.md`](development.md) › Fonts.

### macOS

`packaging/macos/package.sh` builds `aarch64-apple-darwin` and `x86_64-apple-darwin` with
`MACOSX_DEPLOYMENT_TARGET=11.0`, joins them with `lipo` and assembles `VectorCraft.app`:

- `Info.plist` is generated from `Info.plist.in` (bundle id `ai.storyteller.vectorcraft`,
  `LSMinimumSystemVersion` 11.0, the version and the build commit). The icon is
  `assets/app-icon/vectorcraft.icns`. `cargo xtask bundle` fills in the same template for its
  development app.
- **Document types:** `Info.plist.in` declares every format File › Open reads, so Finder opens
  them with VectorCraft (double-click, Open With, a drop on the Dock icon). VectorCraft owns its own
  formats and is an alternate for the others. A test keeps the list in step with the engine's
  `OPEN_EXTS`. AppKit hands the files to the app as open-document events, which
  `apps/vectorcraft/src/open_documents.rs` opens like files named on the command line.
- **Signing** goes inside-out with the hardened runtime and a secure timestamp: the executable
  first, then the bundle, with no `--deep` on the final signature. The entitlements
  (`entitlements.plist`) are deliberately empty.
- **Notarization:** the app is zipped and sent with `xcrun notarytool submit --wait`, then the
  ticket is stapled and checked with `stapler validate` and `spctl`. The app goes on a DMG
  (`hdiutil makehybrid` and `convert`, with an `Applications` link to drag onto), which is signed,
  notarized and stapled too. Its Finder window (background, icon size and positions) comes from
  [`packaging/macos/dmg/`](../packaging/macos/dmg/README.md), and its volume is named `VectorCraft`
  without the version, which the window's background needs; the DMG file name keeps the version.
- **CLI:** the universal `vectorcraft-cli` is signed the same way, zipped, and the zip is
  notarized. A bare executable can't hold a stapled ticket, so Gatekeeper looks the CLI's ticket up
  online the first time a downloaded copy runs.

Locally, without certificates, the script signs ad hoc (`codesign -s -`) and skips notarization,
which is enough to check the bundle and the DMG on your own Mac:

```sh
packaging/macos/package.sh --arch universal   # needs both rustup targets
packaging/macos/package.sh --arch aarch64     # quicker, host only
```

### Windows

`packaging/windows/package.ps1 -Arch x64|x86|arm64` builds with `-C target-feature=+crt-static`,
so neither the MSI nor the portable zip needs the Visual C++ redistributable. The flag is scoped
to the target triple, so host build scripts aren't affected.

- Before packaging, the script checks each binary's PE machine type against `-Arch` (an x64 build
  can never ship labelled arm64), that `vectorcraft.exe` is a GUI program (no console window) and
  that `vectorcraft-cli.exe` is a console program.
- `vectorcraft.wxs` (WiX) is a per-machine install into Program Files with a Start Menu shortcut
  and an App Paths entry (Win+R `vectorcraft`). Same-version upgrades are allowed, so release
  candidates replace each other.
- Double-clicking the MSI opens a setup wizard (WixUI_InstallDir without the licence page):
  Welcome, install folder (remembered for upgrades in `HKLM\Software\VectorCraft\InstallDir`),
  Ready, a progress page, and a Finish page that says where to find the app, with a ticked "Launch
  VectorCraft" box. The pages name the full version (`vectorcraft.en-us.wxl`). `package.ps1` draws
  the wizard's banner and side bitmaps from the app icon, so no WiX stock art ships. `msiexec /qn`
  still installs silently. Building needs the WiX UI and Util extensions:
  `wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`.
- The ARM64 build is cross-compiled on the x64 runner. `.github/workflows/windows-arm64.yml`
  installs that MSI on a Windows 11 ARM64 runner, checks that both installed programs are ARM64,
  runs `vectorcraft-cli --version` natively and uninstalls.
- **Signing:** `packaging/windows/sign.ps1` signs both `.exe` files and then the `.msi` with
  `signtool` (SHA-256, RFC 3161 timestamp), using whichever material is present:
  1. a `.pfx` certificate (`WINDOWS_CERTIFICATE`, base64, and `WINDOWS_CERTIFICATE_PASSWORD`), or
  2. Azure Trusted Signing (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
     `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`).

  With neither, it warns and leaves the files unsigned, so test builds still produce installers.
  It is the one place to adapt when the signing setup changes; `WINDOWS_TIMESTAMP_URL` and
  `SIGNTOOL` override the timestamp server and the `signtool.exe` path.

### Linux

`packaging/linux/package.sh` builds the release binaries and packages them as an AppImage, a
`.deb` and an `.rpm` (with [nfpm](https://nfpm.goreleaser.com), from `nfpm.yaml`) and a plain
`.tar.gz` tree (`bin/`, `share/`). The packages install both programs, the desktop entry, the
AppStream metainfo, the MIME type for VectorCraft documents, the icons and the licence files.

The jobs run on `ubuntu-22.04`, the oldest GitHub-hosted image, so the binaries only need
glibc 2.35 or newer: Ubuntu 22.04+, Debian 12+, Fedora 36+ and RHEL 10. Moving the job to a newer
image raises that floor, so do it deliberately. After packaging, the job prints the `.deb`'s
metadata and contents, runs `ldd` on the binary and runs each AppImage with `--version`.

**AppImage updates.** Each AppImage embeds the update information
`gh-releases-zsync|storytold|vectorcraft|latest|vectorcraft-*-linux-<arch>.AppImage.zsync`
(appimagetool `-u`), and `package.sh` writes the matching `.zsync` beside it when `zsyncmake` (the
`zsync` package) is installed. [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate)
and AppImageLauncher then download only the changed blocks from the newest published,
non-pre-release release. The job checks both are there.

**Flatpak.** The `flatpak` jobs (x86_64 on `ubuntu-24.04`, aarch64 on `ubuntu-24.04-arm`) take the
Linux job's tarball and repackage it with `packaging/linux/flatpak-bundle.sh` and
`packaging/linux/flatpak/ai.storyteller.vectorcraft.bundle.yml` into a single-file bundle
(`flatpak install --user vectorcraft-<v>-linux-<arch>.flatpak`), then install it and run
`vectorcraft-cli --version` in the sandbox. No Rust build happens there, so the bundle carries the
same binaries as the AppImage. `packaging/linux/flatpak/ai.storyteller.vectorcraft.yml` is the
from-source manifest for a Flathub submission (its header says how to build it); packaging-lint
keeps the runtime and sandbox permissions of the two manifests identical.

### FreeBSD

GitHub has no FreeBSD runners, so the job builds in a FreeBSD 14.3 VM on `ubuntu-latest` (the same
image as `.github/workflows/freebsd.yml`). `packaging/freebsd/package.sh` writes a
`/usr/local`-style tree; users install it with
`tar -xzf vectorcraft-*-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local`.

### Web

The web job builds `apps/vectorcraft-web` with trunk and zips the static site
(`packaging/web/package.sh`). [`packaging/web/README.md`](../packaging/web/README.md) covers
hosting: any path works, `.wasm` must be served as `application/wasm`, and the sample header
files for common hosts.

## The draft release

The last job waits for every build, downloads their artifacts, writes `SHA256SUMS.txt` and creates
the draft `VectorCraft v<version>` with notes generated from the merged pull requests. If the draft
already exists, it replaces its assets and keeps it a draft. If that version is already published,
the job fails and asks for a version bump (`cargo xtask version set`).

## Secrets

The jobs that sign (macOS, Windows) and the draft-release job run in the `release` environment,
which only the `release` branch can use and which holds the signing secrets. The other jobs get no
secrets. Every secret is optional: a missing one produces unsigned artifacts and a
warning, never a failed build.

| Secret | Used for |
|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` | the Developer ID certificate (base64 `.p12`), imported into a temporary keychain by `packaging/macos/import-cert.sh` |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarization (`APPLE_PASSWORD` is an app-specific password) |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Windows signing with a `.pfx` |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Windows signing with Azure Trusted Signing |
