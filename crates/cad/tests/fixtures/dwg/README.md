# Original DWG test drawings

These small, synthetic DWG files are original VectorCraft work under MIT OR Apache-2.0
(LICENSE-MIT and LICENSE-APACHE in this directory). They were created by `generate.rs`
using the public Rust API of [acadrust 0.6.6](https://crates.io/crates/acadrust/0.6.6)
(MPL-2.0), without running a vendor CAD application or copying any sample drawing.
Acadrust is only a local fixture-generation tool, not a VectorCraft dependency.

Both versions contain the same millimetre drawing: a line from (10,20) to (110,70)
on layer Geometry, a radius-10 circle at (40,40), and the text VectorCraft CAD at (10,80).
The older file exercises the uncompressed object container; the newer file exercises
compressed sections. No images, fonts, proprietary assets or user files are included.

To regenerate, make a temporary standalone Cargo binary project, add
`acadrust = { version = "=0.6.6", default-features = false }`, and copy `generate.rs`
to its `src/main.rs`. Run it with this fixture directory as the working directory.
The writer may update metadata timestamps, so compare imported geometry rather than bytes.
