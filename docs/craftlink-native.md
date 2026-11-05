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

Optional `expected` is supported for synchronous `object.setProps` and the
read-only, no-path `file.saveCopy` snapshot. Byte snapshots use the existing
engine serializer, return native data without a Save dialog and keep the native
revision/path/dirty state. Other conditional commands are unsupported. Wrong or
stale UID/revision, a different explicit `document`, a closed source or a pending
interaction fails before normal app dispatch. Unsupported conditional commands
and malformed tokens fail explicitly. Existing unguarded command callers retain
their behavior. Address an open document with top-level `document: uid` (or the
UID in `expected`); `ui.inspect.documents` exposes UIDs and paths. Inspection,
guarded name/opacity writes with explicit nonempty `ids`, and guarded native
byte snapshots can target background tabs. Routing preserves the foreground
tab, tool and paint defaults; it never calls the tab-switch/deactivation path.
Writes fail closed during target-document interactions or pending undo groups;
an unrelated foreground interaction stays intact and does not block routing.
Other background properties/commands fail with a capability error. Target/guard
disagreement, unknown object IDs and closed UIDs cannot fall back to a selection
or a different tab. Capabilities advertise `documentRouting: guarded-properties`
and `backgroundProperties: [name, opacity]`.

`events.poll {after, timeoutMs}` runs on the existing control worker, not the UI
thread. Timeout defaults to 20 seconds and is capped at 30. A successful outer
command that changes committed document contents wakes waiters with small
`document.changed` stamps (`seq`, `data.documentId`, `data.revision`). Undo/redo
and close are included; close reports native revision zero. Queries, equal-content
no-ops, failed/rolled-back batches and interaction previews emit no invalidation.
Outer commands that finish interactions compare against their starting document.
Direct interaction commits also emit once after keeping their native undo step;
drag previews coalesce until release, and cancellation emits nothing. Arbitrary
engine mutations outside commands/interactions remain uncovered. Gesture event
coverage does not advertise transform interoperability: the consumer's semantic
contract still supports only rectangle name/opacity.

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

Verified on Windows: four engine invalidation tests, four UI guard/snapshot
tests and 26 native auth/control socket tests passed on the isolated auth
composition. The exact publication branch also compiles its native bin/tests.
The engine without default features compiled for wasm32-unknown-unknown.

The actual VectorCraft/FigCraft candidate consumer passed ten checks: unsaved
name/opacity in both directions with unchanged native files, duplicate delivery,
native and managed undo/redo, stale native write refusal, missing/wrong auth,
broker interruption recovery, same-port app reopen and standalone save. A single
sample measured 78.56 ms VectorCraft -> FigCraft and 61.63 ms reverse. These are
small-document observations, not percentiles. The native save/reopen test uses
existing `app.save {wait:true}` because background-save acknowledgement alone
does not mean the file has reached disk.

Strict library Clippy stopped at unchanged upstream `drop_non_drop` findings in
`cmd/addons/cleanup/text.rs:958` and `cmd/fonts.rs:626`. The same focused check
with only that lint allowed passed; no upstream file or lint policy was changed.
Full workspace CI and non-Windows runtime were not executed. Reproducer and
attributed auth test patch belong to the source-only CraftLink consumer. No
binary redistribution is included.
