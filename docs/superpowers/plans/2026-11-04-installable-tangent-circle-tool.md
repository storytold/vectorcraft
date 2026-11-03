# Installable Tangent Circle Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver an installable `.wasm` plug-in that appears beside Ellipse and snaps a dragged circle into external tangency with two neighbors.

**Architecture:** Extend plug-in ABI v1 with a sandboxed `tool` kind, add a host `PluginTool` that exchanges JSON pointer events and actions with it, expose installed tool plug-ins as dynamic tools in the Ellipse toolbar slot, and build the Tangent Circle plug-in against that contract.

**Tech Stack:** Rust, serde JSON, wasmi, WebAssembly, VectorCraft tools/engine/egui, Cargo tests.

**Spec:** `docs/superpowers/specs/2026-11-04-installable-tangent-circle-tool-design.md`

## Global Constraints

- Keep the plug-in ABI version at 1; add a manifest kind and optional tool metadata without changing existing exports.
- Tool plug-ins get no imports, filesystem, network, clock or randomness and use existing fuel/memory limits.
- The dragged circle keeps its radius; only external tangency counts.
- Hidden, locked, guide, clipping, template and moving circles do not participate.
- Installing adds the tool beside Ellipse; removing it hides the tool.
- No panics in shipped code.
- The unmodified official updater cannot preserve local host changes; custom updates use the feature branch script until the host extension is upstream.

## Review Focus

- A tool manifest omits or misstates `toolGroup`/`icon`: installation must fail or use the documented default without breaking existing tool catalogues.
- A plug-in returns malformed actions/guides, huge strings or non-finite coordinates: the host must reject safely.
- A plain path is only approximately circular: context collection must not invent an incorrect center/radius.
- Install/uninstall while the plug-in tool is active: the toolbar and tool state must converge without a dangling active tool.
- An official update replaces the binary: the patch and custom update path must remain explicit and reproducible.

---

### Task 1: Tool plug-in manifest and runtime API

**Files:**
- Modify: `crates/plugins/src/manifest.rs`
- Modify: `crates/plugins/src/lib.rs`
- Modify: `crates/plugins/src/runtime.rs`
- Modify: `docs/plugins.md`
- Test: `crates/plugins/tests/abi.rs`

**Interfaces:**
- Produces: `Kind::Tool`
- Produces manifest fields `Manifest::tool_group: Option<String>` and `Manifest::icon: Option<String>`
- Produces: `Plugin::run_tool(&self, input: &[u8]) -> Result<Vec<u8>>`

- [ ] **Step 1: Write failing manifest/ABI tests**

Add tests asserting a manifest with `"kind":"tool","toolGroup":"ellipse"` parses; an unknown group
or an overlong/non-icon-safe icon fails; a tool needs no `extensions`; existing filter/effect/
exporter manifests still parse and serialize unchanged.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p vectorcraft-plugins tool --test abi`
Expected: FAIL because `tool` is not a valid kind.

- [ ] **Step 3: Implement the manifest extension**

Add `Kind::Tool`, serialize `tool_group` as `toolGroup`, and default `icon` to `tool-ellipse`.
Validate `toolGroup` against `ellipse` for ABI v1 and require `plugin.<id>` to fit the 64-character
tool-id budget. Reject tool-group/icon fields on non-tool kinds.

- [ ] **Step 4: Implement `run_tool`**

Call the existing `vc_run` with event JSON and an empty JSON object as parameters. Keep all sandbox
limits unchanged.

- [ ] **Step 5: Document the ABI**

Add the tool manifest and event/output JSON contract to `docs/plugins.md`, including the action and
guide limits.

- [ ] **Step 6: Run tests**

Run: `cargo test -p vectorcraft-plugins --test abi`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/plugins docs/plugins.md
git commit -m "feat: add installable tool plugins"
```

### Task 2: Circle context and host plug-in tool

**Files:**
- Create: `crates/plugins/src/tool.rs`
- Create: `crates/tools/src/plugin_tool.rs`
- Modify: `crates/plugins/src/lib.rs`
- Modify: `crates/tools/src/lib.rs`
- Modify: `crates/tools/Cargo.toml`
- Test: inline tests in both new modules

**Interfaces:**
- Produces: `vectorcraft_plugins::tool::circles(doc: &Document, limit: usize) -> Vec<Value>`
- Produces: `PluginTool::new(id: &str) -> Option<Box<dyn Tool>>`
- Produces: `PluginTool` parsing `{state, actions, guides, error}` from `Plugin::run_tool`

- [ ] **Step 1: Write failing circle-context tests**

Assert a live circle gives `{id,center,radius}`, an ellipse with unequal radii is omitted, a closed
plain circle path is included, hidden/locked/guide/clipping circles are omitted, and collection
stops at its limit.

- [ ] **Step 2: Write failing host-tool tests with WAT**

Build a WAT tool that returns `select.set` on down and a translation preview on drag. Assert
`PluginTool` emits the corresponding `Exec` and `Preview`, stores its state, and rejects an action
with a non-finite matrix or an unknown action type.

- [ ] **Step 3: Run and confirm failures**

Run: `cargo test -p vectorcraft-plugins tool:: --lib`
Run: `cargo test -p vectorcraft-tools plugin_tool:: --lib`
Expected: FAIL because both modules are absent.

- [ ] **Step 4: Implement circle context**

Recognize whole live ellipses under uniform similarity transforms; otherwise accept one closed
plain path whose bounds are square and whose anchors/sampled segments stay within `1e-3 * radius`.
Use document visibility/editing state and skip guides/clipping. Cap segments and output count.

- [ ] **Step 5: Implement `PluginTool`**

On each pointer event, serialize event, state, selection, circle context, zoom and tolerance; call
`run_tool`; validate output limits; convert only `exec`, `begin`, `preview`, `commit`, `cancel`,
`dialog` and `switchTool` actions; convert only finite `line` and `label` guides with bounded text.
Store returned state/guides. Use the manifest cursor (`move` default).

- [ ] **Step 6: Register plug-in tools**

`vectorcraft_tools::create("plugin.<id>")` returns `PluginTool` when the installed plug-in is a tool.

- [ ] **Step 7: Run tests**

Run: `cargo test -p vectorcraft-plugins tool:: --lib`
Run: `cargo test -p vectorcraft-tools plugin_tool:: --lib`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/src/tool.rs crates/plugins/src/lib.rs crates/tools/src/plugin_tool.rs crates/tools/src/lib.rs crates/tools/Cargo.toml
git commit -m "feat: host installed plugins as tools"
```

### Task 3: Dynamic catalog and Ellipse toolbar placement

**Files:**
- Modify: `crates/tools/src/catalog.rs`
- Modify: `crates/ui-egui/src/toolbar.rs`
- Modify: `crates/ui-egui/src/edit_toolbar.rs`
- Modify: `crates/engine/src/cmd/plugin.rs`
- Test: `crates/tools/src/catalog.rs`, `crates/ui-egui/src/toolbar` tests

**Interfaces:**
- Produces: `catalog::plugin_tools() -> Vec<&'static ToolInfo>`
- Produces: `catalog::plugin_group(id: &str) -> Option<&'static str>`
- `tool_info("plugin.<id>")` resolves installed tool metadata.

- [ ] **Step 1: Write failing catalog tests**

Install a WAT tool named `Tangent Circle`, assert `plugin_tools()` lists it, `tool_info` resolves
its label/icon and `plugin_group` returns `ellipse`; remove it and assert both disappear.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p vectorcraft-tools catalog::tests::tool_plugins_enter_and_leave_the_catalogue --lib`
Expected: FAIL because no plug-in tool lookup exists.

- [ ] **Step 3: Implement cached dynamic metadata**

Cache each tool's interned `ToolInfo` by plug-in serial. Rebuild on reinstall; removal must make
lookup return `None`. Keep built-in catalogue behavior unchanged.

- [ ] **Step 4: Place tools beside Ellipse**

In `toolbar::slots`, append each `toolGroup == "ellipse"` plug-in to the slot containing `ellipse`.
If that slot is unavailable, add a new slot immediately after the built-in Ellipse slot. Apply the
same rule to the Advanced layout and All Tools drawer.

- [ ] **Step 5: Handle uninstall while active**

After `plugin.remove`, if the active tool is that plug-in's id, switch to `selection`; a failure
looks like a status error but leaves no missing toolbar state.

- [ ] **Step 6: Run focused UI/engine tests**

Run: `cargo test -p vectorcraft-ui-egui tool_plugin_appears_beside_ellipse --lib`
Run: `cargo test -p vectorcraft-engine removing_an_active_tool_plugin_selects_the_selection_tool`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/tools/src/catalog.rs crates/ui-egui/src/toolbar.rs crates/ui-egui/src/edit_toolbar.rs crates/engine/src/cmd/plugin.rs
git commit -m "feat: show tool plugins beside ellipse"
```

### Task 4: Build Tangent Circle plug-in and prove exact tangency

**Files:**
- Create: `plugins/tangent-circle/Cargo.toml`
- Create: `plugins/tangent-circle/src/lib.rs`
- Create: `plugins/tangent-circle/build.sh`
- Test: `crates/engine/src/tests/plugins.rs`

**Interfaces:**
- Produces manifest id `org.vectorcraft.tangentCircle`, name `Tangent Circle`, kind `tool`, group
  `ellipse`.
- Produces `outputs/TangentCircle.wasm`.

- [ ] **Step 1: Write a failing engine gesture test**

Install the fixture module. Create one radius-10 circle and references `(0,0,r=20)` and
`(40,0,r=20)`, select the plug-in tool, press on the moved circle, and drag near
`(20,sqrt(500))`. Assert the final center is exactly the upper intersection, both center distances
are 30, radius stays 10, Undo/Redo work, and remove hides the tool.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p vectorcraft-engine tangent_circle_plugin_ --test it`
Expected: FAIL because the plug-in module and tool behavior are absent.

- [ ] **Step 3: Implement the plug-in state machine**

On down, hit-test the topmost circle and emit `select.set`. On drag, compute the moved center and,
within tolerance of two-circle solution centers, choose the nearest exact solution. Emit one
`Begin`, one `Preview` using `object.transform`, and two guide lines/labels. On up, emit `Commit`
only if a drag began.

- [ ] **Step 4: Implement the exact solver and output guards in the plug-in**

Check finite inputs, positive radii, reachable auxiliary circles, 4,096-circle and 64-neighbor
caps. Reject duplicate/crossing degeneracies without producing NaN.

- [ ] **Step 5: Build the WASM module**

Run: `plugins/tangent-circle/build.sh`
Copy the release module to `outputs/TangentCircle.wasm`.

- [ ] **Step 6: Run engine tests and inspect the UI**

Run: `cargo test -p vectorcraft-engine tangent_circle_plugin_ --test it`
Launch the custom build, install `outputs/TangentCircle.wasm`, verify it appears beside Ellipse,
drag the test circles, capture a screenshot, and inspect the tangent guides.

- [ ] **Step 7: Commit**

```bash
git add plugins/tangent-circle crates/engine/src/tests/plugins.rs
git commit -m "feat: add tangent circle plugin"
```

### Task 5: Update-safe custom build and deliverable

**Files:**
- Create: `scripts/update-custom-vectorcraft.sh`
- Create: `docs/custom-plugins.md`
- Modify: `ROADMAP.md`
- Modify: `docs/target-app-parity.md`
- Generated: `outputs/vectorcraft-tangent-circle-tool.patch`

**Interfaces:**
- Produces a long-lived `feature/tangent-circle-snap` branch.
- Produces an update script that refuses dirty worktrees, fetches upstream, rebases the feature,
  tests, bundles the host, and rebuilds the plug-in.

- [ ] **Step 1: Run focused and broad verification**

Run: `cargo fmt --all -- --check`
Run: `cargo test -p vectorcraft-plugins --test abi`
Run: `cargo test -p vectorcraft-tools plugin --lib`
Run: `cargo test -p vectorcraft-engine tangent_circle_plugin_ --test it`
Run: `cargo xtask bundle`
Expected: all pass.

- [ ] **Step 2: Add the update script and documentation**

The script must stop on conflicts and must never reset or force-update. Document that the custom
host is required for tool plug-ins and that official self-update replaces it.

- [ ] **Step 3: Update project records**

Add the feature to the roadmap progress log and parity evidence without changing unrelated scores.

- [ ] **Step 4: Create the portable patch**

Run: `git format-patch origin/main --stdout > /Users/anunnaki/Documents/Codex/2026-11-04/b-na/outputs/vectorcraft-tangent-circle-tool.patch`
Expected: patch includes host extension, plugin source, design/plan and update workflow.

- [ ] **Step 5: Commit the update workflow**

```bash
git add scripts/update-custom-vectorcraft.sh docs/custom-plugins.md ROADMAP.md docs/target-app-parity.md
git commit -m "docs: preserve the tangent tool across custom updates"
```
