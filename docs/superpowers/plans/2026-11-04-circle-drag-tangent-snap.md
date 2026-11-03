# Circle Drag Tangent Snap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a dragged circle snap, without resizing, into exact external tangency with two nearby editable circles.

**Architecture:** Add a focused circle geometry module under `crates/tools`, gather candidate circles once when a Selection-tool move starts, and ask that module for the nearest valid two-circle tangency center during each move frame. Smart Guides draw the accepted constraint, while grid, pixel and Shift-constrained moves retain precedence.

**Tech Stack:** Rust, kurbo geometry, VectorCraft tool events and engine commands, Cargo tests.

**Spec:** `docs/superpowers/specs/2026-11-04-circle-drag-tangent-snap-design.md`

## Global Constraints

- The moved circle keeps its radius.
- Only external tangency counts.
- Hidden, locked, guide, clipping, template and selected art does not participate.
- Snap to Grid, Snap to Pixel and Shift-constrained moves take precedence.
- No new `.wasm` plug-in: the existing ABI cannot intercept pointer movement.
- Product code must return errors/options rather than panic.
- Preserve the feature through source updates with a dedicated Git branch, commit and patch; never claim an official self-update preserves local source changes.

## Review Focus

- A transformed or plain-path circle is near-circular but extraction is wrong: it must be rejected or use its true center and radius.
- Two reference circles are too far apart, coincident or just touching: the solver must return no snap or one exact candidate without NaN.
- A hidden, locked, guide, clipping or selected circle lies nearby: it must never become a reference.
- A large document has many paths: gathering must be bounded and must not turn drag start into an expensive full geometric scan.
- An official in-app update replaces the binary: the custom source branch/patch must remain the documented update path until the feature is merged upstream.

---

### Task 1: Circle extraction and exact tangency geometry

**Files:**
- Create: `crates/tools/src/tangent.rs`
- Modify: `crates/tools/src/lib.rs`
- Test: inline `#[cfg(test)]` module in `crates/tools/src/tangent.rs`

**Interfaces:**
- Produces: `pub(crate) struct Circle { pub center: Point, pub radius: f64 }`
- Produces: `pub(crate) fn circle_of(node: &Node) -> Option<Circle>`
- Produces: `pub(crate) fn tangent_centers(moved_radius: f64, a: Circle, b: Circle) -> Vec<Point>`

- [ ] **Step 1: Write failing tests**

Add tests asserting:

```rust
assert_eq!(circle_of(&live_circle(30.0, Point::new(50.0, 20.0))), Some(Circle { center: Point::new(50.0, 20.0), radius: 30.0 }));
assert_eq!(circle_of(&ellipse(60.0, 40.0)), None);
assert_eq!(circle_of(&plain_circle_path(25.0, Point::new(80.0, 90.0))), Some(Circle { center: Point::new(80.0, 90.0), radius: 25.0 }));
assert!(circle_of(&non_finite_circle()).is_none());
let centers = tangent_centers(10.0, Circle { center: Point::new(0.0, 0.0), radius: 20.0 }, Circle { center: Point::new(40.0, 0.0), radius: 20.0 });
assert_eq!(centers.len(), 2);
for c in centers { assert!((c.distance(Point::ZERO) - 30.0).abs() < 1e-9); assert!((c.distance(Point::new(40.0, 0.0)) - 30.0).abs() < 1e-9); }
assert!(tangent_centers(2.0, Circle { center: Point::ZERO, radius: 1.0 }, Circle { center: Point::new(100.0, 0.0), radius: 1.0 }).is_empty());
assert!(tangent_centers(1.0, Circle { center: Point::ZERO, radius: 2.0 }, Circle { center: Point::ZERO, radius: 2.0 }).is_empty());
assert_eq!(tangent_centers(1.0, Circle { center: Point::ZERO, radius: 2.0 }, Circle { center: Point::new(6.0, 0.0), radius: 2.0 }), vec![Point::new(3.0, 0.0)]);
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p vectorcraft-tools tangent::tests --lib`
Expected: compilation fails because the module does not exist.

- [ ] **Step 3: Implement circle extraction**

Prefer a whole live ellipse whose width and height are equal and whose affine transform is a uniform similarity. Compute `center = xf * Point::new(w / 2.0, h / 2.0)` and `radius = w / 2.0 * scale`. Reject non-finite or non-positive values.

When no live circle is available, accept one closed plain subpath if its bounds are square within `1e-9 * max(1, radius)`, sample each segment at `t = 0.25, 0.5, 0.75`, and require every sample and anchor to be within `1e-3 * radius` of the center. Cap sampling at 4,096 segments.

- [ ] **Step 4: Implement external tangent centers**

Intersect the two auxiliary circles centered at the reference centers with radii `moved_radius + a.radius` and `moved_radius + b.radius`. Return no points for coincident centers, unreachable circles, negative/non-finite inputs, or non-finite results. Clamp tiny negative `h²` caused by rounding to zero.

- [ ] **Step 5: Run the focused tests**

Run: `cargo test -p vectorcraft-tools tangent::tests --lib`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/tools/src/lib.rs crates/tools/src/tangent.rs
git commit -m "feat: add circle tangency geometry"
```

### Task 2: Gather tangent references and choose the nearest result

**Files:**
- Modify: `crates/tools/src/tangent.rs`
- Modify: `crates/tools/src/guides.rs` if the guide style needs to be exposed
- Test: inline tests in `crates/tools/src/tangent.rs`

**Interfaces:**
- Consumes: `Circle`, `circle_of`, and `tangent_centers`.
- Produces: `pub(crate) struct TangentSnap`
- Produces: `pub(crate) fn new(cx: &ToolContext, excluded: &[NodeId]) -> Option<TangentSnap>`
- Produces: `pub(crate) fn snap(&self, cx: &ToolContext, proposed_center: Point, tolerance: f64) -> Option<(Point, Vec<Overlay>)>`

- [ ] **Step 1: Write failing selection tests**

Create a moved circle of radius 10 and references `(0,0,r=20)` and `(40,0,r=20)`. Assert a proposed center near `(20,sqrt(500))` snaps to the upper intersection and returns two tangent lines plus a `tangent` label. Add cases where the nearest valid center is over tolerance, the references are unreachable, and the only nearby reference is hidden or locked.

- [ ] **Step 2: Run tests and confirm the snap API is missing**

Run: `cargo test -p vectorcraft-tools tangent::tests --lib`
Expected: FAIL because `TangentSnap` does not exist.

- [ ] **Step 3: Implement candidate gathering**

`TangentSnap::new` returns `None` unless Smart Guides is on and Snap to Grid/Pixel are both off. It stores the moved circle and up to 4,096 visible, editable circles from the document, excluding selected IDs. Skip `guide`, `clipping`, template and non-circle paths.

- [ ] **Step 4: Implement candidate selection**

During `snap`, keep only references whose raw center distance differs from `moved.radius + reference.radius` by at most `tolerance`, capped at 64 nearest references. Pair them, solve with `tangent_centers`, reject unreachable pairs, and choose the center nearest `proposed_center`. Return `None` when no center is within `tolerance`.

- [ ] **Step 5: Return guides**

Use `GuideStyle::of(cx)`. When lines are enabled, draw one line from the moved center to each contact point. When labels are enabled, place a `tangent` label at each contact point.

- [ ] **Step 6: Run tests**

Run: `cargo test -p vectorcraft-tools tangent::tests --lib`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/tools/src/tangent.rs crates/tools/src/guides.rs
git commit -m "feat: gather two-circle tangent snaps"
```

### Task 3: Integrate dragging and prove the committed transform

**Files:**
- Modify: `crates/tools/src/select.rs`
- Modify: `crates/tools/src/select/tests.rs`
- Test: `crates/engine/src/tests/xform.rs`

**Interfaces:**
- Consumes: `TangentSnap::new` and `TangentSnap::snap`.
- Produces: `MoveSnap` applies a tangent result after ordinary Smart Guides alignment and returns its overlays.

- [ ] **Step 1: Write the failing Selection tool test**

Place one radius-10 circle and two references at `(0,0,r=20)` and `(40,0,r=20)`. Press on the moved circle and drag so the unsnapped center is within Smart Guides tolerance of `(20,sqrt(500))`. Assert the `object.transform` preview translation puts the moved center at `(20,sqrt(500))` and that its guides include two `Line`s and a `tangent` label. Assert the diameter in the preview is unchanged.

- [ ] **Step 2: Run and confirm the ordinary move does not produce the snap**

Run: `cargo test -p vectorcraft-tools move_a_circle_snaps_to_two_external_tangencies --lib`
Expected: FAIL because the translation is unsnapped.

- [ ] **Step 3: Integrate `TangentSnap` into `MoveSnap`**

Add a `tangent: Option<TangentSnap>` field. Build it in `MoveSnap::new` from the pre-move document snapshot and `cx.selection.objects`. Apply it in `MoveSnap::snap` after bounds, anchor and ruler alignment, translating the proposed circle center and replacing the returned guides on a successful snap. Leave `snap_along` and grid-only moves unchanged.

- [ ] **Step 4: Add engine coverage**

Execute the resulting translation through `object.transform` on a circle, assert both center distances equal `r + reference_radius` within `1e-9`, assert the live ellipse width and height are unchanged, then Undo and Redo and assert the same geometry.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p vectorcraft-tools move_a_circle_snaps_to_two_external_tangencies --lib`
Run: `cargo test -p vectorcraft-engine dragged_circle_keeps_exact_external_tangencies_through_undo_redo`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/tools/src/select.rs crates/tools/src/select/tests.rs crates/engine/src/tests/xform.rs
git commit -m "feat: snap dragged circles to two neighbors"
```

### Task 4: Update-safe branch, package and runtime verification

**Files:**
- Create: `scripts/update-custom-vectorcraft.sh`
- Create: `docs/custom-tangent-build.md`
- Modify: `ROADMAP.md`
- Modify: `docs/target-app-parity.md`
- Generated: `outputs/vectorcraft-tangent-circle-snap.patch`

**Interfaces:**
- Produces: a long-lived `feature/tangent-circle-snap` branch whose feature commit can be rebased onto later `origin/main` revisions.
- Produces: an update script that refuses dirty trees, fetches upstream, rebases the feature branch, runs the focused tests, and builds `dist/VectorCraft.app`.

- [ ] **Step 1: Run the feature tests and formatting before packaging**

Run: `cargo fmt --all -- --check`
Run: `cargo test -p vectorcraft-tools tangent --lib`
Run: `cargo test -p vectorcraft-tools move_a_circle_snaps_to_two_external_tangencies --lib`
Expected: all pass.

- [ ] **Step 2: Build the desktop app**

Run: `cargo xtask bundle`
Expected: `dist/VectorCraft.app` is produced.

- [ ] **Step 3: Launch the custom build and visually test**

Start it with a free control port, create three live circles through the control channel, drag the moved circle near both tangencies, capture `ui.screenshot`, and inspect that the two guide lines and tangent label are visible.

- [ ] **Step 4: Add the update script and documentation**

The script must:

1. Require branch `feature/tangent-circle-snap` and a clean worktree.
2. Run `git fetch origin`.
3. Run `git rebase origin/main`; on conflict, stop and instruct the user rather than resetting.
4. Rerun the focused tests and `cargo xtask bundle`.

Document that VectorCraft's official self-updater replaces the executable with an official binary. Until the feature is merged upstream, updates must use the feature branch/script; never run the official update installer over the custom build.

- [ ] **Step 5: Produce the portable patch**

Run: `git format-patch origin/main --stdout > /Users/anunnaki/Documents/Codex/2026-11-04/b-na/outputs/vectorcraft-tangent-circle-snap.patch`
Expected: the patch contains both specification/plan documents and all feature commits.

- [ ] **Step 6: Commit the update workflow**

```bash
git add scripts/update-custom-vectorcraft.sh docs/custom-tangent-build.md ROADMAP.md docs/target-app-parity.md
git commit -m "docs: keep the tangent snap in custom updates"
```
