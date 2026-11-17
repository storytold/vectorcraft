# Proposal: plug-in panels with flexible layouts and drawing surfaces

**Status:** design proposal; not implemented or approved. **Updated:** 2026-11-17.
This document changes no ABI, permission, command, feature score or delivery estimate.

Related: [#2185](https://github.com/storytold/vectorcraft/issues/2185),
[#2179](https://github.com/storytold/vectorcraft/issues/2179), and the existing panel implementation
in [#5287](https://github.com/storytold/vectorcraft/pull/5287). This proposal complements that work;
it does not replace it or ask to merge a second implementation of the same panel host.

## Problem and current boundary

A scientific figure toolbox needs a panel that stays open beside the canvas: page navigation,
a swatch gallery, compact alignment buttons, fields and an interactive preview in the same view.
Other plug-ins need maps or geometry previews. These workflows need the plug-in to choose the
arrangement of its content and respond to the panel's size, rather than receive one field per
parameter in a generated dialog.

In the reviewed main revision, `9a01d266b0438f96608c9f75b51437f912587d22`:

- [`Kind`](../crates/plugins/src/manifest.rs) has filters, effects, exporters and importers.
- [`permissions::RESERVED`](../crates/plugins/src/permissions.rs) contains `ui.panel` and `ui.tool`;
  requesting either is refused at install.
- [`dialogs/plugin.rs`](../crates/ui-egui/src/dialogs/plugin.rs) draws parameter forms, with a
  420-point maximum width for the filter dialog. Changing that width would not provide a panel.
- [`panel_docking.rs`](../crates/ui-egui/src/panel_docking.rs) already supplies shared docking;
  [`library_panel.rs`](../crates/ui-egui/src/panels/library_panel.rs) demonstrates dynamic panel ids.

PR #5287 provides a dockable panel with host-rendered controls. Its description leaves a drawing
surface outside its scope. The [maintainer's review](https://github.com/storytold/vectorcraft/pull/5287#issuecomment-6599514802)
asks for a decision on its relationship to PhotoCraft's design, exact command permissions and
installation approval before proceeding. The author has agreed to wait for that decision.

## Proposed division of responsibility

| Host owns | Plug-in chooses |
|---|---|
| Panel identity, title, opening, closing, docking, floating and workspace placement | Pages, navigation, control grouping, rows, columns, grids and split proportions |
| Native controls, theme tokens, focus, keyboard navigation and accessibility | Labels, values, enabled/visible state and the order and sizing of controls |
| Validated drawing primitives, clipping, text layout and approved resource handles | Preview geometry, colors, transforms, hit regions and responses to pointer events |
| Permissions, document snapshots, command execution and undo | Requested document actions and temporary UI state within granted capabilities |

The host should register each installed panel through the existing docking system and list it in
Window > Plug-ins. It should not require a toolbox-specific panel implementation in the app.
The initial proposal is one panel per module; supporting several later is an open question below.

### Layout and drawing

Provide composable containers for rows, columns, wrapping grids, tabs, scroll areas and resizable
splits, plus native fields, buttons, lists and swatch items. The plug-in supplies stable item ids,
padding, spacing and minimum/preferred/flexible sizing. The host supplies the available content
rectangle and scale. A panel may be wider than a parameter dialog and must work when resized.
This is a layout tree, not a form schema whose parameters fix the visual order.

A drawing-surface item is the escape hatch for arrangements or previews that the native containers
cannot express. It accepts a bounded display list of paths, rectangles, text and raster resource
references, with transforms and clips. The plug-in defines their positions inside the surface.
The host clips everything to that surface; drawing cannot cover app chrome or another panel.
Colors may use theme tokens or explicit content colors, so a scientific swatch retains its color.
Text uses host fonts and host text measurement; a plug-in does not bundle a font or touch egui.

Surface items declare accessible names, focusable hit regions and keyboard equivalents. Pointer
press/move/release, capture cancellation, wheel and key events use logical coordinates local to
the surface; the host manages capture and focus. Decorative preview drawing does not silently
become the only way to invoke a document action. Text entry remains a native editable control.

Illustrative view data, **not an accepted manifest or an implemented wire format**:

```json
{
  "type": "split",
  "id": "toolbox",
  "axis": "horizontal",
  "children": [
    {
      "type": "tabs",
      "id": "pages",
      "children": [
        {
          "type": "tab",
          "id": "palette",
          "label": "Palette",
          "children": [
            {
              "type": "swatchGrid",
              "id": "colors",
              "wrap": true,
              "colors": ["#0072B2", "#E69F00", "#009E73"]
            }
          ]
        }
      ]
    },
    {
      "type": "surface",
      "id": "preview",
      "accessibleName": "Arrangement preview",
      "preferredSize": [320, 240]
    }
  ]
}
```

The display list and input-event schema need their own reviewed specification. The example
shows the required composition and sizing, not a promise that those keys already work.

## Permissions, events and document actions

Keep the existing WASM sandbox and capability model. A panel must explicitly request `ui.panel`
at a host API level that implements it; current hosts continue refusing it. Showing a panel alone
does not grant document reads, writes, storage, file or network access. The existing filters and
their generated dialogs keep working. This document does not select a new API number or ABI export.

Installation must retain the current person's approval of the module hash and requested
permissions, including an exact `commandAllowlist` when commands are requested. Panel code must
use the same reviewed command policy and transaction path as
[`plugin_call`](../crates/engine/src/cmd/plugin_call.rs), extended deliberately for the chosen
event protocol. Prefix matches must not make future commands available automatically. The panel
feature adds no direct toolkit, filesystem, clipboard, network or process access.

Proposed event behavior:

1. Opening sends the content size, scale and theme with a document snapshot only if authorized.
   Resizing, selection/document changes and Undo/Redo can request a fresh view. A hidden panel
   does no continuous rendering work and refreshes when shown again.
2. Events and responses carry a panel-instance generation, module identity and, for document
   actions, document identity and revision. Closing, replacement, uninstall or document changes
   invalidate pending work. A stale response is discarded, not redirected to the active document.
3. Rendering and refresh events only update the view. They cannot commit document commands,
   change selection or write persistent settings. Coalesce repeated resize/pointer-move and
   document-refresh events; do not invoke the module once per UI frame.
4. A user action explicitly requests document operations. The host checks permissions and
   arguments, executes through the existing confined command path and returns success or error
   with a correlation id. A transaction is one undo step; failure rolls all its document edits back.
   Drawing the panel again must never repeat the action, including extra egui layout passes.
5. A preview belongs to an explicit interaction with Begin/Preview/Commit/Cancel semantics.
   Closing the panel, losing its document or replacing its module cancels it. Merely drawing a
   surface cannot start an edit. Undo/Redo refreshes the view from the resulting document.

Keep document state separate from UI state: selected tabs and scroll positions are not document
history. Document-associated plug-in data must use the existing transactional `state` capability;
per-plug-in preferences use `settings` only when granted. A transient UI-state value must not be
treated as proof that a previous document action still exists after Undo.

Results should be able to arrive as later events, as the PhotoCraft proposals describe, without
making the UI depend on blocking work. Whether this uses a new event export or an adapter around
the existing entry point is a maintainer decision. The implementation must define what persists
across fresh WASM instances and report action results back to the panel.

## Bounds, portability and inspection

Layout freedom still needs bounded untrusted input. Specify and enforce byte, item-count, nesting,
display-list, resource-memory and event-queue limits before rendering. Validate finite coordinates,
sizes, transforms, clips, ids and resource handles. Reuse existing run budgets and cancellation;
cap work on the UI thread. A malformed response keeps the previous valid view with an error and
commits no edits. Initial ceilings and native/web performance measurements belong in the
implementation PR; none is measured by this proposal.

Raster bytes stay behind host-managed, quota-accounted resource handles. A surface may reference
only resources made available to that plug-in by an approved mechanism. Define ownership and
release on replacement, close and uninstall. Adding a display surface grants no new file picker
or download capability. Bundled example assets must follow the repository's attribution rules.

Use Rust host code and the existing renderer on native and web, with no webview or handwritten
JavaScript. Floating panels follow the existing platform policy; the plug-in's content contract
is the same. No native-only drawing API should be necessary to use the surface.

Opening and interacting with a panel must remain command-driven and discoverable through MCP
and the control channel. Inspection should expose item ids, accessible labels, values and enabled
state; an action uses the same event path as a real click. Surface hit regions must be inspectable
too. This gives automated clients usable controls without guessing pixel coordinates or gaining
an alternative route to approve installation.

## Implementation acceptance criteria

- A standalone example module opens, docks, floats, tabs, closes and restores placement through
  the existing docking system. Uninstall/replacement removes or invalidates its content cleanly.
- The example combines tabs, a wrapping swatch grid, native fields and a resizable interactive
  surface. Native and web tests cover resize, display scaling, focus, keyboard access and clipping.
- The same action from UI, MCP or the control channel produces the same document edit and one
  undo step. Failed batches roll back; redraws do not repeat edits; stale responses change nothing.
- Undo/Redo, selection changes, document switches, hiding/showing, close during preview and module
  replacement update or invalidate state correctly without modifying an unrelated document.
- Permission tests refuse an ungranted panel, a command outside its exact allowlist and a newly
  added command that has not been reviewed. Installation continues requiring a person's approval.
- Hostile/fuzz tests cover deep and oversized trees, invalid numbers, bad resources, duplicate ids,
  event floods and exhausted budgets, with a bounded error rather than a panic or UI hang.
- Existing plug-in ABI tests keep passing, along with `cargo xtask ci`; the implementation supplies
  screenshots of its own example and reports which native/web interactions were actually tested.

## Decisions requested before implementation

1. Adopt this layout/surface boundary and develop it with #5287, or scope an initial surface-free
   implementation explicitly and track surfaces separately?
2. Which event ABI and capability version should align VectorCraft with
   [PhotoCraft #662](https://github.com/storytold/photocraft/issues/662) and
   [#4301](https://github.com/storytold/photocraft/pull/4301)?
3. Start with one panel per module, or include multiple independently dockable panels?
4. Which native layout/control primitives, surface operations, resource limits and measurements
   are required in the first implementation?

## Revision history

| Date | Change |
|---|---|
| 2026-11-17 | Initial proposal, grounded in the current host and the open panel review; no shipped behavior changed. |
