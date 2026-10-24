# VectorCraft on Android

VectorCraft on Android: a small `apps/` crate that hosts the existing `vectorcraft_ui_egui` interface in an
`android-activity` (GameActivity) window, on top of `vectorcraft_engine`. The desktop entry crate is not
compiled, so `rfd`, `arboard` and the macOS/Windows/Linux platform code never enter the dependency
graph.

## What the shell does

- **Entry point** — builds as `libmain.so` (`[lib] name = "main"`, `crate-type = ["cdylib"]`), loaded
  with `System.loadLibrary`; `android_main` is the usual `android-activity` entry.
- **Storage** — `AndroidApp::internal_data_path()` is the data directory, and VectorCraft's `ui.json`/`prefs.json` pair is loaded before the first frame and written back in `on_exit`.
- **File pickers** — VectorCraft's `Services` hooks are filled from the Storage Access Framework. The picker
  cannot answer inside the frame that asks, so `pick_open` starts a worker, returns
  `None`, and the picked file is copied into `<filesDir>/import/` and opened over the
  control channel. `pick_save` answers with a scratch path in `export/` and publishes the
  finished file to the `ACTION_CREATE_DOCUMENT` destination. `read`/`write` are plain
  `std::fs`.
- **Fonts** — Android does not expose the Unix font directories, so the CJK face from `/system/fonts`
  is appended to every family *after* VectorCraft installs its theme (any earlier, and the theme's named
  families would be dropped).
- **Touch** — hit targets are raised to 40 pt, sliders widened and hover tooltips suppressed, applied
  after the theme's style exists. The layout is not zoomed: it is designed for a desktop window and a
  phone gives ~411 pt, so the next step is a touch rework, not a scale factor.

## Known gaps

- Printing has no backend, so the Print dialog is unreachable rather than broken.
- Documents open from the app's own import copy; "save back to the original" is not wired.
- Pen pressure is not wired (`crates/tablet` has macOS/X11 back ends only).

- The system clipboard is not wired (`arboard` has no Android backend); VectorCraft's own clipboard is
  still used.
- The launcher icons are the Android Studio template's placeholder art.

## Build

```bash
cargo ndk -t arm64-v8a -o apps/vectorcraft-android/jniLibs build --release -p vectorcraft-android
cd apps/vectorcraft-android/android && ./gradlew assembleRelease
```

`jniLibs/` and the Gradle output are gitignored; the APK is not part of this change.
