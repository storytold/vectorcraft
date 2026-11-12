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
- **KR**: the README and the `bin/cdr-unpk` script of the Kaitai Struct specification (MIT), at
  the same commit as K,
  https://github.com/kaitai-io/coreldraw_cdr.ksy/blob/9d8e2704b946961f5114115f1a488750a3c86ed0/README.md
  and https://github.com/kaitai-io/coreldraw_cdr.ksy/blob/9d8e2704b946961f5114115f1a488750a3c86ed0/bin/cdr-unpk
- **Z**: the PKWARE ZIP specification, APPNOTE.TXT version 6.3.10,
  https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT, and RFC 1951 (DEFLATE),
  https://www.rfc-editor.org/rfc/rfc1951
- **TS**: the source of the reader that T's notes describe (MIT OR Apache-2.0), at the same commit
  as T, read for where a field lies when the notes leave it open; no code is copied,
  https://github.com/tracedraw/tracedraw/blob/2ac6d02e97773e040a51be6cfb27cc424e3deeaa/crates/cdr/src/wl.rs
  and https://github.com/tracedraw/tracedraw/blob/2ac6d02e97773e040a51be6cfb27cc424e3deeaa/crates/cdr/src/parse.rs
- **F**: the magic database of the `file` utility (BSD-2-Clause),
  https://github.com/file/file/blob/b778217dbd35/magic/Magdir/images and
  https://github.com/file/file/blob/b778217dbd35/magic/Magdir/riff
- **PR**: the PRONOM registry's signatures for versions 1.0, 2.0 and 4.0,
  https://pronom.nationalarchives.gov.uk/fmt/467.xml,
  https://pronom.nationalarchives.gov.uk/fmt/466.xml and
  https://pronom.nationalarchives.gov.uk/fmt/465.xml

Status: `public-spec` (K, KR, C, Z, F or PR), `public-notes` (a confirmed row of T, a row of T's
table of the layouts its writer takes from sample files, TS, or P), `unconfirmed` (a row T marks as
assumed, or a point no source states; listed so it can be checked against a file).

No public specification covers versions 1 to 4, and the files of those versions that this reader
has read are the synthetic ones `src/synth.rs` builds from the rows below. T's authors took their
rows for versions 3 and 4 from four version 3 drawings, six version 4 drawings and a set of
version 3 clip art, and their rows for versions 1 and 2 from 18 files; none of those files is
public.

T and P are themselves notes taken from files, not specifications. Drawings of versions 7 to 18,
20, 22, 24, 26 and 27 written by the converter that T's authors publish (release 0.44.0) read back
with the pages, layers, groups, shapes, colors, outline widths, caps, joins and dashes it wrote.
That converter also reads the synthetic packages of versions 14 to 18, 20, 22, 24, 26 and 27 that
`src/synth.rs` writes, with the page, its layer and the shapes on it. Those files are not
committed.

| Id | What the code relies on | Source | Status |
|---|---|---|---|
| R1 | `RIFF`, the body length, then the form type `CDR` or `cdr` and a version character: `1` to `9`, `A` (10) to `H` (17), `J` (18) on, `I` unused; `cdr8` is 8.01, `CDR ` is 3, `CDRX` is another format. Files of versions 23 to 27 carry `O` to `U`, which count differently from the version 24 update on (`CDRQ` with a `vrsn` of 2430, `CDRU` with 2700), so their `vrsn` chunk decides (R2) | K L68-L133; T L22 | public-spec (letters from version 23: public-notes) |
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
| R16 | Point list: u32 count (before version 6 as R39 gives it), the points, then one type byte each, the count held to what fits; type bits 7 and 6 move, line, curve end, control point; bit 3 closes the subpath | K L782-L787, L840-L845, L2701-L2807; T L170-L171, L180 | public-spec |
| R17 | Ellipse coordinates: width, height, start and end angles, pie flag (u16 before 6, else u32); the radii are half the width and height; the angles turn from the ellipse's own axes, which a negative width or height points the other way | K L754-L781; T L179 | public-spec (axis direction of a negative size: public-notes) |
| R18 | Polygon argument (11000): 4 bytes before 13, the side count u32, the next point u32 (the u32 after it when it is 0 or 1, else 4 bytes), 4 more from 13, rx and ry f64, the centre; the base curve is one side on a circle of 1000 mm from the top vertex to the vertex it runs to clockwise, copied once for each side | K L631-L655; T L182-L183 | public-spec (base curve: public-notes) |
| R19 | `flgs`: byte 3 the record type, byte 2 marks the master page; a layer's byte 0 is 0, 8 desktop, 0x0a guides, 0x1a grid; in versions 7 to 13 the master page flags its guides [2, 1, 1], desktop [0, 1, 1] and grid [0x12, 0, 1]; from 14 it flags them [0x0a, 0, 1], [8, 0, 1] and [0x1a or 0x5a, 1, 1], by byte 0 as other layers | K L1602-L1639; T L132-L133 | public-spec (master page flags: public-notes for versions 7 to 21, unconfirmed for 22 to 27) |
| R20 | Fill and outline arguments (20, 10): a u32 record id | K L452-L460, L548-L556 | public-spec |
| R21 | Page size argument (19130): width and height | K L666-L672 | public-spec |
| R22 | `LIST page` holds `LIST layr` lists, in versions 7 to 13 inside a `LIST gobj`; a layer holds `LIST obj ` and `LIST grp `; a page, layer or object keeps its own `loda` (and an object its `trfd`, in a `trfl` list) among its chunks or in a `LIST lgob` | K L1602-L1639 (record types); T L120, L129, L359, L362 | public-notes |
| R23 | Guideline (object type 0x16), argument 16000: 12 bytes, the angle in millionths of a degree, then points from the page centre; version 7 keeps them in a `guid` chunk: a u32 count, the count again, 40-byte records laid out the same way | T L159-L160 | public-notes |
| R24 | ZIP: the end of central directory record (signature `PK\5\6`, 22 bytes and a comment of up to 65,535 bytes) gives the member count and where the central directory lies; each central directory entry (46 bytes, then name, extra field and comment) gives the flags, method, CRC-32, sizes and its local header; a local header is 30 bytes, then its name and extra field, then the member's bytes. Flag bit 0 marks an encrypted member; method 0 is stored and 8 deflated; 0xFFFF and 0xFFFFFFFF mark ZIP64 fields, and a ZIP64 locator (`PK\6\7`, 20 bytes) precedes the end record of a ZIP64 archive; disk numbers other than 0 mark an archive split across files | Z 4.3.7, 4.3.12, 4.3.15, 4.3.16, 4.4.1.4, 4.4.4, 4.4.5, 4.4.7, 4.4.8, 4.4.13 | public-spec |
| R25 | Versions 14 and 15 save a ZIP package whose member `content/riffData.cdr` is the RIFF file | KR README L33, `cdr-unpk` L30; T L29, L355 | public-spec |
| R26 | From version 16 the package's member `content/root.dat` is the RIFF file; `content/dataFileList.dat` names the data streams, one per line, members of `content/data/`; a redirect's stream number counts lines from 0 | KR README L34, L40, `cdr-unpk` L34-L66; K L173-L180; T L37, L355 | public-spec |
| R27 | In a package of version 16 on, a chunk outside a compressed list whose size is 16 (in `root.dat` or in a redirected list's body) is a redirect record: stream u32, length u32, offset u32 and a reserved u32; its body is `length` bytes at `offset` of the stream; stream `0xFFFFFFFF` holds the body (at most 8 bytes) in the record after the length; a redirected list's body starts with its form type | K L144-L193; T L38-L40 | public-spec |
| R28 | Templates have the form type `CDT` and a letter of their own, `CDTQ` in version 21 with a `vrsn` of 2100; the `vrsn` chunk names the version. The reader takes any letter `A` to `Z` there | T L26 | public-notes (letters other than `Q`: unconfirmed) |
| R29 | Rectangle coordinates from version 15: width and height as f64 coordinate units, scale x and y (f64, repeating the transform's scale), the scale-with flag (u8) and 7 bytes, then the four radii as f64, 16 bytes apart (a corner type byte and 15 bytes after the first), in the order of R15. With the flag clear each radius is a share of half the shorter side; with it set the radii are multiplied by 254000 coordinate units, which makes them inches | K L674-L752; T L173-L177 | public-spec (radius share with the flag clear, scale not applied: public-notes) |
| R30 | Object type 0x25 (version 16 on) is a path: 4 bytes, two u16 counts that add up to the point count, 16 bytes, then the points and their type bytes as in R16 | K L788-L799, L892; T L157, L181 | public-spec (from version 16: public-notes) |
| R31 | Version 3 is a RIFF file with the form type `CDR ` (a space) and a `vrsn` of 300; version 4 has the form type `CDR4` and a `vrsn` chunk. Both use the 16-bit fields of R5 and R7. Their pages, layers, groups and objects are read by the rules of R19 and R22 | K L117-L134; T L44; PR fmt/465; F `riff` L786 | public-spec (pages and layers as R19 and R22: unconfirmed) |
| R32 | Version 3 counts its object types from 0: rectangle 0, ellipse 1, curve 2, text 3, bitmap 4; later versions use these plus one | T L53; TS `parse.rs` | public-notes |
| R33 | Version 3 keeps an object's transforms in argument 100 of its `lobj`: a u16, the entry count (u16), then 10-byte entries of a type (u16) and, at byte 6, where the entry's data starts, counted from the start of the argument. Type 0x8c is a matrix as R37, and the matrices apply in file order. Type 0x14 is an envelope, which this reader does not apply: the shape keeps its outline, placed by the matrices alone, and the drawing's warnings name the envelopes. A group's own `lobj` holds an argument 100 that places all of the group's objects after their own transforms | T L54-L56, L60; K L851; TS `parse.rs` | public-notes |
| R34 | Version 3 fill, argument 20: a type byte (0 none, 1 uniform, 2 and 4 fountain, 6 texture, 7 and 10 pattern); a uniform fill's color follows as R36 | K L452-L547; T L62 | public-spec |
| R35 | Version 3 outline, argument 10: a flags byte with bit 0 set when the object is outlined, the width (coordinate), the stretch (u16), the nib angle, the color as R36, 7 bytes, the dash count (u8) and 10 bytes that hold the dashes as one byte each, the join (u16 at byte 30) and the caps (u16 at byte 32) numbered as R12, then the two arrowhead ids (u32). Dashes are read in multiples of the width as R12 | K L548-L592; T L64 | public-spec (flag bit: public-notes; dash unit, and the join, which T puts in the byte at 33: unconfirmed) |
| R36 | A color before version 5: in versions 1 to 3 a model byte and four value bytes, in version 4 a model u16, four u16 values and 2 bytes. Model 1 is cyan, magenta, yellow and black in percent. Model 0 is an ink number (u16) and a tint in percent (u16): ink 0 is black at the tint, ink 1 is white, and other numbers are inks of a numbered library that the file does not store. Other models are read as R14 | K L2882-L2908; T L65-L66, L89; TS `parse.rs`, `wl.rs` | public-notes (models other than 0 and 1: unconfirmed, K and T differ on model 1) |
| R37 | A matrix of versions 3 and 4: six i32 `a c tx b d ty`, the four factors in 16.16 fixed point and `tx` and `ty` in coordinate units. A version 4 `trfd` entry of type 8 holds one where R8 has six f64 | T L55, L61; TS `parse.rs` | public-notes |
| R38 | `mcfg` of version 3: 2 bytes, then the page's two corners as four coordinates; the page size is the distance between them | K L1676-L1678, L1745-L1756; T L71 | public-spec |
| R39 | A point list before version 6 starts with a u16 count and another u16 (1 in some curves of versions 3 and 4), where K reads the two as one u32 | T L67 | public-notes |
| R40 | Versions 1 and 2 are not RIFF files. A file starts with `WL`, a marker byte, a zero byte and five u32: the start of the settings (28), the start and the end of the object records, the start of the trailer and that value minus two. F and PR give the marker `e` for version 1.0 and `m` for 2.0; T's authors also saw `d`. The reader takes these three markers and reads the three alike | T L82; F `images` L4159-L4172; PR fmt/467, fmt/466 | public-notes (which version `d` marks: unconfirmed) |
| R41 | The page's box in a file of version 1 or 2: four i16 at byte 30 (left, top, right, bottom) in 1/1000 inch around the page's centre, y pointing up | T L75-L78, L83; TS `wl.rs` | public-notes |
| R42 | The records of a file of version 1 or 2 run from the front of the stacking order to the back. A group or layer record is 28 bytes, and the low byte of its flags at byte 26 is not 0x40. A drawn object has 0x40 there and a u16 body size at byte 28; it is the body size plus 32 bytes long, and at least 118 | T L85, L86, L93; TS `wl.rs` | public-notes |
| R43 | A drawn object of version 1 or 2: a point (two i16) at byte 32; six i32 in 16.16 fixed point at 36, `a b tx c d ty`, with x' = a x + b y + tx + the point's x and y' = c x + d y + ty + the point's y; the object type at 60 (0 rectangle, 1 ellipse, 2 curve, 3 text); a byte at 67 that is not 0 when the object is outlined, the outline's width (u16) at 68 and its color at 74; the fill type at 94 (0 none, 1 uniform) and its color at 95; the shape from byte 114. Colors are as R36 | T L87-L89; TS `wl.rs` | public-notes |
| R44 | Shapes of version 1 or 2, from byte 114 of the object: a rectangle's width and height (i16), then after 2 bytes a corner radius; an ellipse's width and height, then two angles; a curve's point list as R39 and R16 | T L90, L91; TS `wl.rs` | public-notes (radius: unconfirmed; how the angles make an arc: unconfirmed, so an ellipse whose angles differ is left out) |
