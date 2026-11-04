# Native command invalidations and conditional properties

This additive seam preserves the existing engine registry, document format and
standalone workflow. It reuses the bounded `craft-command::ChangeBus` proposed in
[craft-libs #431](https://github.com/storytold/craft-libs/pull/431), pinned at
`61db4a13ddd42a3727c8802168d90a0a2dafb9de`. That proposal is not merged.

`document.inspect` reports native `uid`, `revision` and capabilities. These tokens
are scoped to this open document in this authenticated app launch; they are not
durable CraftLink object IDs. Revision also advances for selection/view changes,
so a guard may reject conservatively.

```json
{"method":"engine.execute","params":{"command":"object.setProps","params":{"ids":[4],"name":"Linked","opacity":60},"expected":{"document":1,"revision":7}}}
```

Optional `expected` is supported only for synchronous `object.setProps`. Wrong or
stale UID/revision, a different explicit `document`, a closed source or a pending
interaction fails before normal app dispatch. Unsupported conditional commands
and malformed tokens fail explicitly. Existing unguarded command callers retain
their behavior. Routing is **active document only**: select/reconnect the bound
source explicitly; this does not switch tabs or overwrite another open document.

`events.poll {after, timeoutMs}` runs on the existing control worker, not the UI
thread. Timeout defaults to 20 seconds and is capped at 30. A successful outer
command that changes committed document contents wakes waiters with small
`document.changed` stamps (`seq`, `data.documentId`, `data.revision`). Undo/redo
and close are included; close reports native revision zero. Queries, equal-content
no-ops, failed/rolled-back batches and interaction previews emit no invalidation.
Outer commands that finish interactions compare against their starting document.
Direct interaction commits, gesture callbacks and engine data mutations outside
command dispatch are not covered.

The response has `events`, `cursor`, `reset`, `dropped`. Each waiting subscriber
uses the shared 256-entry bus and is released after its poll. History between
polls is deliberately coalesced: a gap or cursor ahead of this session returns
`reset:true`. Reinspect current native state on reset/overflow/reconnect; never
replay native editing commands from this feed. An idle wait reads no document or
filesystem. Snapshot/asset bytes are not transported in events.

Authentication remains owned by existing [VectorCraft #4623](https://github.com/storytold/vectorcraft/pull/4623).
Use an isolated composition with it before enabling CraftLink writes. This seam
does not replace its token protocol or claim the stock control port is secure.
Shared local sockets/discovery/push events remain owned by
[craft-ipc #433](https://github.com/storytold/craft-libs/pull/433); its complete host
adoption is separate. Plugin ABI [#432](https://github.com/storytold/craft-libs/pull/432)
and SDK [#455](https://github.com/storytold/craft-libs/pull/455) retain guest manifests
and `link.call`/`link.exec` permission gates. Native preconditions grant no plugin
permissions. No new broker, installer, plugin ABI or renderer is introduced here.

Focused native checks and the real two-candidate consumer reproduction are
recorded with the feature PR. Full workspace CI and non-Windows runtime must not
be inferred from those results. No binary redistribution is included.
