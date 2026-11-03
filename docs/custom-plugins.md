# Keeping the Tangent Circle tool across updates

Tangent Circle is an installable WebAssembly tool plug-in. The unmodified VectorCraft host
recognizes filters, live effects and exporters; this custom branch adds the small `tool` plug-in
kind needed to place the plug-in beside Ellipse.

## Current install

- Custom host: `dist/VectorCraft.app`
- Installable plug-in: `../../outputs/TangentCircle.wasm`
- Feature branch: `feature/tangent-circle-snap`

The plug-in is kept in VectorCraft's normal Plug-ins folder after installation, so restarting the
custom build preserves it.

## Updating

Run:

```sh
scripts/update-custom-vectorcraft.sh
```

The script refuses dirty worktrees, fetches `origin/main`, rebases this feature branch, reruns the
host and plug-in tests, rebuilds the `.wasm`, and produces a new `dist/VectorCraft.app`. If the
rebase conflicts, it stops with Git's normal conflict state; resolve the conflict and continue the
rebase. It never resets or force-updates.

## Why the official updater is not used

The official self-updater replaces the application binary with the official build. That build does
not contain the tool plug-in host extension, so the installed `.wasm` would remain on disk but
could not appear in the Ellipse tool group. Use the custom update script until the host extension
is merged upstream, then ordinary official updates can preserve the plug-in support.

The portable Git patch beside the `.wasm` file records every host and plug-in change for review,
backup, or upstream submission.
