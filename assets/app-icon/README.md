# Vector W3K2 app icon

A bold cyan **V** drawn as a vector path, with its three anchor points as white squares, on a black
rounded tile: the Print That 204 colours (black and Print That cyan) with solid fills only, no gradients
or effects, per the Print That 204 branding guidelines.

| Colour | Hex | Used for |
|---|---|---|
| Black | `#0b0b0c` | the tile, anchor outlines |
| Print That cyan | `#00a0e3` | the V |
| White | `#ffffff` | the anchors |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`. `vectorcraft.svg` is the master; the
other files are rendered from it with `vectorcraft-cli convert vectorcraft.svg out.png --scale N`. The
file names keep the upstream VectorCraft names so the build and packaging scripts find them. Licence: see
`LICENSE.txt`.
