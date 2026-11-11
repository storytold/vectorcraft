# Where each layout comes from

The reader relies on the record layouts below. Code cites a row by its id (`R1` ...), and
`tests/provenance.rs` fails when a cited id has no row here.

Sources:

- **K**: the Kaitai Struct specification `coreldraw_cdr.ksy` (MIT, notice in `LICENSE-kaitai.txt`),
  https://github.com/kaitai-io/coreldraw_cdr.ksy/blob/9d8e2704b946961f5114115f1a488750a3c86ed0/coreldraw_cdr.ksy
- **T**: an open-source converter's CDR format notes (MIT OR Apache-2.0), rows its authors mark
  confirmed,
  https://github.com/tracedraw/tracedraw/blob/2ac6d02e97773e040a51be6cfb27cc424e3deeaa/docs/cdr-format.md
- **C**: the published object model reference for the format's drawing program,
  https://community.coreldraw.com/sdk/api/draw/17
- **P**: a web editor's published CDR notes,
  https://github.com/photopea/CDR-specification/blob/f91b3c7e8c812b28473f6399e95a624a72a3d6b8/README.md

Status: `public-spec` (K or C), `public-notes` (a confirmed row of T, a row of T's table of the
layouts its writer takes from sample files, or P), `unconfirmed` (a row T marks as assumed; listed
so it can be checked against a file).

T and P are themselves notes taken from files, not specifications. Drawings of versions 7 to 13
that the converter T's authors publish (release 0.44.0) wrote read back with the pages, layers,
groups, shapes, colors, outline widths, caps, joins and dashes it wrote; those files are not
committed.

| Id | What the code relies on | Source | Status |
|---|---|---|---|
| R1 | `RIFF`, the body length, then the form type `CDR` or `cdr` and a version character: `1` to `9`, `A` (10) to `H` (17), `J` (18) on, `I` unused; `cdr8` is 8.01, `CDR ` is 3, `CDRX` is another format | K L68-L133 | public-spec |
| R2 | `vrsn`: the version as a u16, in hundreds | K L276-L278 | public-spec |
| R3 | `LIST cmpr`: two (compressed, uncompressed) u32 pairs, then two blocks of `CPng`, `01 00 04 00` and a zlib stream; the first inflates to the chunk stream, the second to a pool of u32 sizes that each chunk's size field indexes | K L193-L220, L2626-L2682; P "LIST cmpr"; T L30 | public-spec |
| R4 | A chunk is an id, a u32 size and the body, padded to an even length; a `LIST` body starts with its form type; from version 7 a `LIST stlt` holds the style tables, not chunks | K L68-L78, L135-L155, L307-L334 | public-spec |
| R5 | Coordinates: i16 in 1/1000 inch before version 6, else i32 in 1/254000 inch; header fields, counts and offsets are u16 before version 6, else u32 | K L58-L66, L2688-L2700; T L145 | public-spec |
| R6 | `loda` and `lobj`: length, argument count, offsets table, types table, object type; count + 1 offsets bound the arguments; the types table runs in reverse; object types (1 rectangle, 2 ellipse, 3 curve, 0x14 polygon) and argument types (10 outline, 20 fill, 30 coordinates, 1000 name, 11000 polygon, 19130 page size) | K L335-L450, L846-L893; T L145-L157 | public-spec |
| R7 | Angles: i16 tenths of a degree before version 6, else i32 millionths of a degree | K L2835-L2855 | public-spec |
| R8 | `trfd`: length, count, offsets table; each entry has 8 bytes first from version 13, a u16 type, and type 8 is a matrix after 6 bytes (from version 6) of six f64 `a c tx b d ty`, `tx` and `ty` in coordinate units; the matrices of one record apply in file order | K L895-L953, L2808-L2834; T L201-L204 | public-spec |
| R9 | Name argument (1000): Windows-1252 up to a NUL before version 12, UTF-16LE from it | K L601-L630 | public-spec |
| R10 | `mcfg`: 12 bytes (13 on), 4 (9 to 12), 28 (6) or none, then the page width and height as coordinates | K L1670-L1765; P "mcfg" | public-spec |
| R11 | `fild` (`fill` before 7): id; from 13 a record tagged 1300 with its length, which records tagged 2400 or more may precede; the fill type u16 (0 none, 1 uniform, others not read); uniform: 2 bytes then a color before 13, from 13 a record tagged 1300 holding a property list (u8 type, u32 length, body; 0 ends it; 1 a color, 8 a palette id and a UTF-16 name, 3 a UTF-16 name) | K L1025-L1302; T L217-L220 | public-spec (2400 records: public-notes) |
| R12 | `outl`: id; from 13 records (u32 id, u32 length, body) up to id 1, whose body follows; line type, caps and join u16; 2 bytes (6 to 12); width; stretch u16; 2 bytes (6 on); nib angle; 52 bytes (6 to 12) or 46 (13 on); the color; 16 bytes (10 before 6); the dash count u16 and the dashes, in multiples of the width; the arrowhead ids 22 bytes after the first dash in 7 and 11 to 12, 20 bytes from 13. Caps 0 butt, 1 round, 2 square; joins 0 miter, 1 round, 2 bevel | K L954-L1024; T L242-L243; C `cdrOutlineLineCaps`, `cdrOutlineLineJoin` | public-spec (arrowhead places: public-notes) |
| R13 | Bit 0 of the outline's line type: no outline | T L246 | unconfirmed |
| R14 | A color record from version 5: model u16, palette u16, 4 bytes, four value bytes. Models: 2 CMYK in percent, 3 and 17 CMYK 0 to 255, 4 CMY 0 to 255, 5 blue, green, red, 9 gray 0 to 255, 20 registration, 21 blue, green, red and a tint in percent already applied; 1, 14 and 25 a library color's number (u16) and tint in percent (u16) | K L2856-L2965; T L256-L257 | public-spec (library values: public-notes) |
| R15 | Rectangle coordinates: width, height and one radius, three more radii from version 9, running counterclockwise from the corner at (width, 0) | K L674-L700; T L172-L173 | public-spec |
| R16 | Point list: u32 count in every version, the points, then one type byte each, the count held to what fits; type bits 7 and 6 move, line, curve end, control point; bit 3 closes the subpath | K L782-L787, L840-L845, L2701-L2807; T L170-L171, L180 | public-spec |
| R17 | Ellipse coordinates: width, height, start and end angles, pie flag (u16 before 6, else u32); the radii are half the width and height; the angles turn from the ellipse's own axes, which a negative width or height points the other way | K L754-L781; T L179 | public-spec (axis direction of a negative size: public-notes) |
| R18 | Polygon argument (11000): 4 bytes before 13, the side count u32, the next point u32 (the u32 after it when it is 0 or 1, else 4 bytes), 4 more from 13, rx and ry f64, the centre; the base curve is one side on a circle of 1000 mm from the top vertex to the vertex it runs to clockwise, copied once for each side | K L631-L655; T L182-L183 | public-spec (base curve: public-notes) |
| R19 | `flgs`: byte 3 the record type, byte 2 marks the master page; a layer's byte 0 is 0, 8 desktop, 0x0a guides, 0x1a grid; in versions 7 to 13 the master page flags its guides [2, 1, 1], desktop [0, 1, 1] and grid [0x12, 0, 1] | K L1602-L1639; T L132-L133 | public-spec (versions 7 to 13: public-notes) |
| R20 | Fill and outline arguments (20, 10): a u32 record id | K L452-L460, L548-L556 | public-spec |
| R21 | Page size argument (19130): width and height | K L666-L672 | public-spec |
| R22 | `LIST page` holds `LIST layr` lists, in versions 7 to 13 inside a `LIST gobj`; a layer holds `LIST obj ` and `LIST grp `; a page, layer or object keeps its own `loda` (and an object its `trfd`, in a `trfl` list) among its chunks or in a `LIST lgob` | K L1602-L1639 (record types); T L120, L129, L359, L362 | public-notes |
| R23 | Guideline (object type 0x16), argument 16000: 12 bytes, the angle in millionths of a degree, then points from the page centre; version 7 keeps them in a `guid` chunk: a u32 count, the count again, 40-byte records laid out the same way | T L159-L160 | public-notes |
