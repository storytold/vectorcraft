# PDF backend

PDF writing uses `craft-pdf` from `storytold/craft-libs`, pinned to
`a0916808f7d78bed8c7b90cdf937176015454a7f`. Its `write` feature supplies Krilla configuration,
metadata construction, fallible font preparation and serialization, and native unwind guards.
The dependency disables default features and enables `write` and `interpret`.
The production writer retains Krilla’s raster-image support without its optional text shaper.
PDF test fixtures enable `simple-text` through a dev dependency.
The PDF crate explicitly retains the `flate2` `zlib-rs` feature previously supplied
by the registry reader, preserving compressed export bytes and their derived document IDs.

VectorCraft owns document traversal, drawing, font selection and embedding policy, colour
conversion, presets, page boxes, layers, editing data, postprocessing and print composition.
Editable import keeps its VectorCraft adapter, layer routing, colour policy and text decisions.
Desktop printer submission remains in `craft-print`.

The shared writer vendors Krilla 0.8.2 with an unwind-safe cleanup and poisoning patch.
The root `[patch.crates-io]` selects that copy from the same immutable revision: Cargo does
not inherit a dependency workspace's patches. A failed page or surface cleanup poisons the
backend document, so serialization cannot return an incomplete PDF after unwinding.
The shared crate's compile-time backend marker and `cargo xtask pdf-deps` (included in CI)
check that the patched writer is used. Krilla retains its MIT OR Apache-2.0 licensing;
its upstream provenance and authored patch are recorded in the shared repository.

Export and print construction, drawing and postprocessing run inside the shared native
unwind guard. These guards do not catch WASM aborts or process-wide allocation failures.
VectorCraft retains its finite-coordinate checks and the existing behavior that omits art
outside the writer's supported coordinate range. Font preparation checks the shared
512 MiB per-font copy limit and variation coordinates before allocation; failure preserves
VectorCraft's reported outline fallback. The writer is not a peak-memory or runtime budget.

PDF reading uses the same shared revision's patched Hayro syntax 0.7.2, interpreter 0.7.0
and JBIG2 0.3.0. This snapshot also retains CCITT 0.3.0 and JPEG 2000 0.3.5.
Its established Lab conversion uses Bradford adaptation; calibrated gray/RGB and tint-function
conversion retain floating-point precision. This policy differs from the registry 0.8 reader's
Lab profile and byte-quantized conversion and is assessed against independent colour expectations.
The application explicitly preserves its existing accepted file sizes with
`OpenOptions { max_input_bytes: usize::MAX, ..Default::default() }`; this migration adds no
new file-size ceiling. Ordinary decoded streams use the shared backend's immutable
per-document limits. Those limits are separate from streaming page-content budgets and
do not bound aggregate document or process memory. Password classification and the
existing permissions-password fallback remain application policy.

The layer and overprint pre-scan consumes `ContentReader` windows through `TypedIter`,
retaining graphics and marked-content state between windows. It does not collect a whole
page before scanning. Reaching the scan's operator, nesting or instruction-window limit,
a reported failed/truncated stream, a cap-reaching filtered form, or an undecidable final
operand tail returns an import error and discards the partial document. Previously the
operator limit could return a partial layer routing with a warning. Interpreter completion
warnings also become errors before a document is returned. Whitespace and comments at
EOF remain valid. The typed iterator does not classify every malformed operator, so this
is known-incomplete rejection rather than a general PDF validity certificate.

The five patched Hayro crates and Krilla are selected in the root `[patch.crates-io]`
from the same immutable revision as `craft-pdf`. `cargo xtask pdf-deps` rejects missing,
duplicate, local-path, registry and mismatched revisions for those seven packages.
`craft-print`, `craft-ui` and the other craft-libs crates use the same revision.

Dependency asset hashes and sources are in `ASSETS.md`; redistributed license texts are
in `licenses/pdf-backend`. Package staging carries those texts together with `NOTICE`
and `ASSETS.md`. Fonts and profiles remain dependency data; this repository adds no font
or profile binaries. Native page interpretation has a shared unwind guard; WASM aborts
and process-wide allocation failures cannot be caught.

The shared parser retains current main's 64-level cap for direct arrays and dictionaries,
including decoded content and object streams. The application retains its two-pass traversal
of repaired object tables, 16-link function/colour-space cap, 64-level calculator-procedure cap,
256 MiB stream checks and predictor validation. Native interpretation uses the existing
16 MiB stack wrapper, and imported document depth remains bounded.

Known incomplete decoded content remains an import error, including deeply nested operands
that the bounded parser cannot finish. Lowered ordinary decode limits and dictionary delimiters
split across scan windows are covered by synthetic regressions; they require the typed
incomplete-content error and never return a partial document. The former 256-level lexical
preflight and its diagnostic are superseded by the parser cap and completion checks.
The scratch bound does not cover whole-filter fallback decoding or aggregate parser/process memory.
