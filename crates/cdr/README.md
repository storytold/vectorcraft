# vectorcraft-cdr

CorelDRAW (`.cdr`) compatibility for VectorCraft.

- **Read**: versions 3 to 26 (RIFF up to 13; ZIP with `content/riffData.cdr` in 14 and 15; ZIP with
  `content/root.dat` from 16).
- **Write**: versions 7 to 22. From 16 (CorelDRAW X6) on, files the writer produces were opened in
  CorelDRAW by the upstream project's tests; older versions are marked experimental.
- **Validation**: every export reads the written file back and checks its version, pages, page
  sizes and object count; a file that fails is not written.

The binary format is handled by the `tracedraw-cdr` and `tracedraw-core` crates of
[TraceDraw](https://github.com/igtoth/tracedraw) (MIT OR Apache-2.0, clean-room, pinned to a
commit). This crate does the conversion between the two document models, the limits for untrusted
files, the containment of third-party failures and the diagnostics. The compatibility matrix:
[`docs/cdr-compatibility.md`](../../docs/cdr-compatibility.md).

| Entry point | Engine command |
|---|---|
| `detect_version(&bytes) -> Result<CdrInfo>` | `document.cdrInfo` |
| `validate(&bytes) -> Validation` | `document.cdrInfo` (`readable`, `objects`…) |
| `import(&bytes, title, &ImportOptions) -> Result<Imported>` | `document.open`, `file.place` (`cdr: {…}`) |
| `export(&doc, &ExportOptions) -> Result<Exported>` | `document.export {format: "cdr", version…}` |
| `matrix::FEATURES`, `matrix::to_json()`, `export_versions()` | `document.cdrCapabilities` |
