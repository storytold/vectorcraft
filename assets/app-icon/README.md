# Vector W3K2 app icon

"PT" for Print That: a bold black **P** and a Print That cyan **T** on a white rounded tile framed in
solid orange (a supporting colour; the letters keep the core black and cyan), following the Print That 204 logo treatment (Print in black, That in cyan; solid colours, no
gradients or effects).

| Colour | Hex | Used for |
|---|---|---|
| Black | `#0b0b0c` | P |
| Print That cyan | `#00a0e3` | T |
| White | `#ffffff` | the tile |
| Orange | `#f7941d` | the frame |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112` (orange) holding a white one with `rx=80`. `vectorcraft.svg` is the master, with the
letters as outlines (set in the bundled bold sans and outlined with `vectorcraft-cli convert --outline-text`);
the PNG, ICO and ICNS files are rendered from it with `vectorcraft-cli convert vectorcraft.svg out.png
--scale N`. The file names keep the upstream VectorCraft names so the build and packaging scripts find
them. Licence: see `LICENSE.txt`.
