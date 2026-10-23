# Windows 7 x64 compatibility build (experimental, unsupported)

Windows 7 is not a supported platform. This build is experimental and community-maintained,
and the VectorCraft maintainers have not verified it on Windows 7.

This separate build targets **64-bit Windows 7 SP1**. It uses OpenGL instead of wgpu,
and omits the native AccessKit accessibility adapter. Printer discovery supports the
PowerShell version included with Windows 7, and printer setup opens Control Panel.
Printing retains the PDF viewer's Print/PrintTo verb on this build: Windows 7 has no
Windows.Data.Pdf renderer. The win7 target excludes `craft-print` from its dependency
tree; normal desktop builds use the shared spooler. For the same reason this build has no
Print › Network Printer… destination (WS-Print and IPP, which come through `craft-print`). The `windows7` feature and dedicated win7 target must be selected together; the
existing compile-time guards reject mixing this feature with the ordinary Windows target.
The normal desktop and web builds retain their existing defaults. A vendor graphics driver providing **OpenGL 3.3 or newer**
is required; the Windows built-in generic display driver is insufficient. GPU selection
in Preferences and `WGPU_POWER_PREF` do not control this OpenGL build.

## Build or download

In your GitHub fork, open **Actions → Windows 7 x64 compatibility → Run workflow** (it
also runs weekly and on pull requests changing the compatibility path).
When it succeeds, download the `vectorcraft-windows7-x64-portable` artifact, extract the
contained portable ZIP, then run `vectorcraft.exe` on Windows 7. The CLI is included.
No MSI is provided: the existing installer remains for modern Windows.

To build locally, use a **Windows 10/11 build computer**, with Rust/rustup and Visual Studio
2022 C++ build tools (x64 compiler and Windows SDK). Open an **x64 Native Tools Command
Prompt for VS 2022**, change to your clone's repository root (the directory containing
`Cargo.toml`), and run:

```powershell
powershell -ExecutionPolicy Bypass -File packaging/windows/windows7.ps1
```

Add `-Test` to also run the app and CLI tests on the build computer.

Output: `dist/release/vectorcraft-windows7-x64-portable.zip`. The script installs the pinned
nightly compiler and `rust-src`, rebuilds the standard library for
`x86_64-win7-windows-msvc`, and links the C runtime statically. It never installs Rust on
the Windows 7 machine. The optional `CRAFT_FONTS_DIR` setting works as in the normal build.
Without it, install fonts for the languages you need on the destination machine.

The Windows 7 target matters: modern Rust's ordinary `x86_64-pc-windows-msvc` target
requires Windows 10, including standard-library and random-number APIs. Selecting OpenGL
alone or changing an installer version check cannot fix that.

References: [Rust's Windows 7 target](https://doc.rust-lang.org/rustc/platform-support/win7-windows-msvc.html)
and [Windows baseline change](https://blog.rust-lang.org/2024/02/26/Windows-7/).

The packaging script checks x64 PE headers, the subsystem version, and known incompatible
imports (including `GetSystemTimePreciseAsFileTime`) before creating the ZIP. This is a
regression check, not an exhaustive audit of every Windows API.

The small vendored `windows-link` patch routes `CoTaskMemFree` to its documented
`ole32.dll` export on the Windows 7 target; the newer binding otherwise imports
`combase.dll`, which is absent on Windows 7. Only this script uses it: it passes
`--config "patch.crates-io.windows-link.path='vendor/windows-link'"` to Cargo, so every other
build, including the normal Windows one, keeps the checksum-verified crates.io crate. Cargo
records the patch in `Cargo.lock`, so the script resolves once, fails if anything other than
windows-link's source changed, builds with `--locked`, and then restores `Cargo.lock`.

## Runtime validation

The contributor reports that a user ran the portable x64 compatibility build on Windows 7 SP1
with an NVIDIA graphics card. Their original build failed at startup with
`GetSystemTimePreciseAsFileTime` missing from `kernel32.dll`; the compatibility
build resolved that failure. The tested build was based on upstream commit
`1f7873273b67007b0aefc63313e0d34a39f45453`. The exact GPU/driver version and completion
of every checklist item below were not recorded, and the maintainers have not reproduced it,
so the build remains experimental and unsupported.

## Runtime acceptance checklist

A successful build on the Windows 2022 CI runner does **not** prove Windows 7 runtime
compatibility. Before declaring support, test on a Windows 7 SP1 x64 machine and record
its GPU, driver version, Windows updates and any missing-DLL/entry-point error:

- Start `vectorcraft.exe`; verify a visible canvas, resizing, minimize/restore and exit.
- Draw a shape, save a `.vectorcraft` document, close and reopen it.
- Open `examples/dusk-poster.vectorcraft`; export PNG and PDF and inspect both.
- Exercise native Open/Save dialogs, clipboard text/images and drag-and-drop.
- Change a preference, exit and verify it survives restarting.
- Run `vectorcraft-cli.exe --version` and a headless export from a Command Prompt in the
  extracted application folder.

This build is experimental until that checklist passes. New dependency versions can add
newer Windows imports, so retain Cargo.lock and repeat runtime QA after dependency updates.
