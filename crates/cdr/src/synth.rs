//! Synthetic drawings for tests and fuzz seeds, built from the layouts in `PROVENANCE.md`. Not a
//! `.cdr` writer: nothing checks that other programs open these files.

use std::io::Write as _;

/// A chunk or a list, before it is laid out.
#[derive(Clone, Debug)]
pub enum Node {
    Chunk([u8; 4], Vec<u8>),
    List([u8; 4], Vec<Node>),
}

pub fn chunk(id: &[u8; 4], data: Vec<u8>) -> Node {
    Node::Chunk(*id, data)
}

pub fn list(form: &[u8; 4], children: Vec<Node>) -> Node {
    Node::List(*form, children)
}

fn put(out: &mut Vec<u8>, id: &[u8; 4], size: u32, body: &[u8], len: usize) {
    out.extend(id);
    out.extend(size.to_le_bytes());
    out.extend(body);
    if len % 2 == 1 {
        out.push(0);
    }
}

/// `n` laid out with real sizes.
pub fn plain(n: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    match n {
        Node::Chunk(id, data) => put(&mut out, id, data.len() as u32, data, data.len()),
        Node::List(form, kids) => {
            let mut body = form.to_vec();
            body.extend(kids.iter().flat_map(plain));
            put(&mut out, b"LIST", body.len() as u32, &body, body.len());
        }
    }
    out
}

/// `n` laid out for a compressed stream: each size field is an index into `pool`.
fn pooled(n: &Node, pool: &mut Vec<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    let (id, body) = match n {
        Node::Chunk(id, data) => (*id, data.clone()),
        Node::List(form, kids) => {
            let mut body = form.to_vec();
            for k in kids {
                body.extend(pooled(k, pool));
            }
            (*b"LIST", body)
        }
    };
    let index = pool.len() as u32;
    pool.push(body.len() as u32);
    put(&mut out, &id, index, &body, body.len());
    out
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// A `LIST cmpr` holding `children` (R3).
pub fn cmpr(children: &[Node]) -> Node {
    let mut pool = Vec::new();
    let stream: Vec<u8> = children.iter().flat_map(|c| pooled(c, &mut pool)).collect();
    let sizes: Vec<u8> = pool.iter().flat_map(|s| s.to_le_bytes()).collect();
    let block = |raw: &[u8]| {
        let mut b = b"CPng".to_vec();
        b.extend([1, 0, 4, 0]);
        b.extend(zlib(raw));
        b
    };
    let (b1, b2) = (block(&stream), block(&sizes));
    let mut body = Vec::new();
    for (c, u) in [(b1.len(), stream.len()), (b2.len(), sizes.len())] {
        body.extend((c as u32).to_le_bytes());
        body.extend((u as u32).to_le_bytes());
    }
    body.extend(b1);
    body.extend(b2);
    Node::Chunk(*b"LIST", [b"cmpr".to_vec(), body].concat())
}

/// The form type letter of `version` (hundreds) (R1).
pub fn form(version: u16) -> [u8; 4] {
    let n = (version / 100) as u8;
    let c = match n {
        0..=9 => b'0' + n,
        10..=17 => b'A' + n - 10,
        _ => b'J' + n - 18,
    };
    [b'C', b'D', b'R', c]
}

/// A RIFF file of `version` holding `children`.
pub fn riff(version: u16, children: &[Node]) -> Vec<u8> {
    let mut body = form(version).to_vec();
    for c in children {
        body.extend(match c {
            // A prebuilt `LIST cmpr` carries its own form type.
            Node::Chunk(id, data) if id == b"LIST" => {
                let mut o = Vec::new();
                put(&mut o, id, data.len() as u32, data, data.len());
                o
            }
            _ => plain(c),
        });
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// Field writers for one version.
#[derive(Clone, Copy, Debug)]
pub struct V(pub u16);

impl V {
    pub fn short(self) -> bool {
        self.0 < 600
    }

    pub fn word(self, v: usize) -> Vec<u8> {
        if self.short() { (v as u16).to_le_bytes().to_vec() } else { (v as u32).to_le_bytes().to_vec() }
    }

    /// A length in points as a coordinate.
    pub fn coord(self, pt: f64) -> Vec<u8> {
        if self.short() {
            ((pt / 72.0 * 1000.0).round() as i16).to_le_bytes().to_vec()
        } else {
            ((pt / 72.0 * 254_000.0).round() as i32).to_le_bytes().to_vec()
        }
    }

    pub fn angle(self, deg: f64) -> Vec<u8> {
        if self.short() { ((deg * 10.0).round() as i16).to_le_bytes().to_vec() } else { ((deg * 1_000_000.0).round() as i32).to_le_bytes().to_vec() }
    }

    /// A `loda` record of object type `kind` (R6): header, arguments, offsets, then the types in
    /// reverse.
    pub fn loda(self, kind: u32, args: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let w = if self.short() { 2 } else { 4 };
        let head = 5 * w;
        let mut body: Vec<u8> = Vec::new();
        let mut offsets = Vec::new();
        for (_, a) in args {
            offsets.push(head + body.len());
            body.extend(a);
        }
        offsets.push(head + body.len());
        let offsets_at = head + body.len();
        let types_at = offsets_at + offsets.len() * w;
        let total = types_at + args.len() * w;
        let mut out = Vec::new();
        for f in [total, args.len(), offsets_at, types_at, kind as usize] {
            out.extend(self.word(f));
        }
        out.extend(body);
        for o in offsets {
            out.extend(self.word(o));
        }
        for (t, _) in args.iter().rev() {
            out.extend(self.word(*t as usize));
        }
        out
    }

    /// A `trfd` of `matrices` `[a, b, c, d, e, f]` with offsets in points (R8).
    pub fn trfd(self, matrices: &[[f64; 6]]) -> Vec<u8> {
        let w = if self.short() { 2 } else { 4 };
        let head = 3 * w;
        let table = head;
        let mut entries = Vec::new();
        let mut offsets = Vec::new();
        let first = table + matrices.len() * w;
        for m in matrices {
            offsets.push(first + entries.len());
            if self.0 >= 1300 {
                entries.extend([0; 8]);
            }
            entries.extend(8u16.to_le_bytes());
            if !self.short() {
                entries.extend([0; 6]);
            }
            let unit = if self.short() { 1000.0 / 72.0 } else { 254_000.0 / 72.0 };
            let [a, b, c, d, e, f] = *m;
            for v in [a, c, e * unit, b, d, f * unit] {
                entries.extend(v.to_le_bytes());
            }
        }
        let mut out = Vec::new();
        for f in [first + entries.len(), matrices.len(), table] {
            out.extend(self.word(f));
        }
        for o in offsets {
            out.extend(self.word(o));
        }
        out.extend(entries);
        out
    }

    /// A curve's point list (R16).
    pub fn points(self, pts: &[(f64, f64, u8)]) -> Vec<u8> {
        let mut out = (pts.len() as u32).to_le_bytes().to_vec();
        for (x, y, _) in pts {
            out.extend(self.coord(*x));
            out.extend(self.coord(*y));
        }
        out.extend(pts.iter().map(|p| p.2));
        out
    }

    /// A uniform fill record (R11) of 12-byte color `color`.
    pub fn fill(self, id: u32, color: &[u8; 12]) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        if self.0 >= 1300 {
            let mut props = vec![1];
            props.extend(12u32.to_le_bytes());
            props.extend(color);
            props.extend([0, 0, 0, 0, 0]);
            let mut solid = 1300u32.to_le_bytes().to_vec();
            solid.extend((props.len() as u32).to_le_bytes());
            solid.extend(props);
            let mut body = 1u16.to_le_bytes().to_vec();
            body.extend(solid);
            out.extend(1300u32.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        } else {
            out.extend(1u16.to_le_bytes());
            out.extend([0, 0]);
            out.extend(color);
        }
        out
    }

    /// An outline record (R12) of `pen`, `color` and `dashes`.
    pub fn outline(self, id: u32, pen: Pen, color: &[u8; 12], dashes: &[u16]) -> Vec<u8> {
        let Pen { line_type, width, cap, join } = pen;
        let mut out = id.to_le_bytes().to_vec();
        if self.0 >= 1300 {
            out.extend(1u32.to_le_bytes());
            out.extend(0u32.to_le_bytes());
        }
        for f in [line_type, cap, join] {
            out.extend(f.to_le_bytes());
        }
        let v = self.0;
        out.extend(vec![0; if (600..1300).contains(&v) { 2 } else { 0 }]);
        out.extend(self.coord(width));
        out.extend(100u16.to_le_bytes());
        out.extend(vec![0; if self.short() { 0 } else { 2 }]);
        out.extend(self.angle(0.0));
        out.extend(vec![
            0;
            if v >= 1300 {
                46
            } else if self.short() {
                0
            } else {
                52
            }
        ]);
        out.extend(color);
        out.extend(vec![0; if self.short() { 10 } else { 16 }]);
        out.extend((dashes.len() as u16).to_le_bytes());
        for d in dashes {
            out.extend(d.to_le_bytes());
        }
        out.extend([0; 32]);
        out
    }
}

/// An outline's line type (bit 0: none), width in points, cap and join.
#[derive(Clone, Copy, Debug)]
pub struct Pen {
    pub line_type: u16,
    pub width: f64,
    pub cap: u16,
    pub join: u16,
}

/// A 12-byte color record (R14) of `model` and its four value bytes.
pub fn color(model: u16, v: [u8; 4]) -> [u8; 12] {
    let mut c = [0; 12];
    c[..2].copy_from_slice(&model.to_le_bytes());
    c[8..].copy_from_slice(&v);
    c
}

/// An object list: flags, then a `lgob` list with the `loda` and an optional transform in a `trfl`
/// list (R22).
pub fn object(v: V, kind: u32, args: &[(u32, Vec<u8>)], matrix: Option<[f64; 6]>) -> Node {
    let mut own = vec![chunk(b"loda", v.loda(kind, args))];
    if let Some(m) = matrix {
        own.push(list(b"trfl", vec![chunk(b"trfd", v.trfd(&[m]))]));
    }
    list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 0x08]), list(b"lgob", own)])
}

/// A layer list of `kind` flags named `name` holding `objects`, its `loda` in a `lgob` list (R22).
pub fn layer(v: V, flags: [u8; 4], name: &str, objects: Vec<Node>) -> Node {
    let name: Vec<u8> =
        if v.0 >= 1200 { name.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect() } else { name.bytes().chain([0]).collect() };
    let mut kids = vec![chunk(b"flgs", flags.to_vec()), list(b"lgob", vec![chunk(b"loda", v.loda(0, &[(1000, name)]))])];
    kids.extend(objects);
    list(b"layr", kids)
}

/// A page list: flags (byte 2 set for the master page), then its layers in a `gobj` list (R22).
pub fn page(master: bool, layers: Vec<Node>) -> Node {
    list(b"page", vec![chunk(b"flgs", vec![0, 0, u8::from(master), 0x90]), list(b"gobj", layers)])
}

/// The `mcfg` chunk of `version` with a page of `w` × `h` points (R10).
pub fn mcfg(v: V, w: f64, h: f64) -> Node {
    let skip = if v.0 >= 1300 {
        12
    } else if v.0 >= 900 {
        4
    } else if (600..700).contains(&v.0) {
        28
    } else {
        0
    };
    let mut b = vec![0; skip];
    b.extend(v.coord(w));
    b.extend(v.coord(h));
    b.extend([0; 16]);
    chunk(b"mcfg", b)
}

/// A drawing of `version` with one page of each kind of object this reader reads, a text object
/// it doesn't, a master page with a guideline and a desktop object; compressed (version 7 on)
/// when `compress`.
pub fn sample(version: u16, compress: bool) -> Vec<u8> {
    let v = V(version);
    let red = color(5, [0, 0, 255, 0]);
    let black = color(2, [0, 0, 0, 100]);
    let doc = list(
        b"doc ",
        vec![
            mcfg(v, 612.0, 792.0),
            list(b"filt", vec![chunk(if version >= 700 { b"fild" } else { b"fill" }, v.fill(1, &red))]),
            list(
                b"otlt",
                vec![
                    chunk(b"outl", v.outline(2, Pen { line_type: 0, width: 2.0, cap: 1, join: 1 }, &black, &[])),
                    chunk(b"outl", v.outline(3, Pen { line_type: 1, width: 1.0, cap: 0, join: 0 }, &black, &[])),
                ],
            ),
        ],
    );
    let style = |fill: u32, outline: u32| vec![(20, fill.to_le_bytes().to_vec()), (10, outline.to_le_bytes().to_vec())];
    let mut rect_args = style(1, 2);
    let mut rc = [v.coord(100.0), v.coord(50.0), v.coord(5.0)].concat();
    if version >= 900 {
        rc.extend([v.coord(5.0), v.coord(5.0), v.coord(5.0)].concat());
    }
    rect_args.push((30, rc));
    let mut ellipse_args = style(1, 3);
    ellipse_args.push((30, [v.coord(40.0), v.coord(-40.0), v.angle(0.0), v.angle(0.0), v.word(0)].concat()));
    let mut curve_args = style(0, 2);
    curve_args.push((30, v.points(&[(0.0, 0.0, 0x08), (50.0, 0.0, 0x40), (50.0, 20.0, 0xC0), (20.0, 50.0, 0xC0), (0.0, 50.0, 0x88)])));
    let mut arc_args = style(0, 2);
    arc_args.push((30, [v.coord(30.0), v.coord(30.0), v.angle(0.0), v.angle(90.0), v.word(1)].concat()));
    let side = POLYGON_SIDE;
    let mut poly_args = style(1, 2);
    poly_args.push((30, v.points(&[(0.0, side, 0x0C), (side * 0.587, side * 0.809, 0x44), (side * 0.951, side * 0.309, 0x44)])));
    let mut poly = if version >= 1300 { vec![] } else { vec![0; 4] };
    poly.extend(5u32.to_le_bytes());
    poly.extend([0; 8]);
    if version >= 1300 {
        poly.extend([0; 4]);
    }
    poly.extend(0.05f64.to_le_bytes());
    poly.extend(0.05f64.to_le_bytes());
    poly.extend([v.coord(0.0), v.coord(0.0)].concat());
    poly_args.push((11000, poly));
    let art = layer(
        v,
        [0, 0, 0, 0x98],
        "Art",
        vec![
            object(v, 1, &rect_args, Some([1.0, 0.0, 0.0, 1.0, -100.0, 100.0])),
            object(v, 2, &ellipse_args, Some([1.0, 0.0, 0.0, 1.0, 50.0, -50.0])),
            object(v, 3, &curve_args, None),
            list(b"grp ", vec![chunk(b"flgs", vec![0, 0, 0, 0x10]), object(v, 0x14, &poly_args, None), object(v, 2, &arc_args, None)]),
            object(v, 4, &[(30, [v.coord(0.0), v.coord(0.0)].concat())], None),
        ],
    );
    let mut guide = vec![0; 12];
    guide.extend((90_000_000i32).to_le_bytes());
    guide.extend([v.coord(36.0), v.coord(0.0), v.coord(36.0), v.coord(100.0), v.coord(36.0), v.coord(0.0)].concat());
    let guides = layer(v, [0x0a, 0, 1, 0x98], "Guides", vec![object(v, 0x16, &[(16000, guide)], None)]);
    let desktop = layer(
        v,
        [0x08, 0, 1, 0x98],
        "Desktop",
        vec![object(
            v,
            1,
            &[(30, [v.coord(10.0), v.coord(10.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())],
            Some([1.0, 0.0, 0.0, 1.0, 400.0, 0.0]),
        )],
    );
    let pages = vec![page(true, vec![guides, desktop]), page(false, vec![art])];
    let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
    let content: Vec<Node> = std::iter::once(doc).chain(pages).collect();
    if compress {
        body.push(cmpr(&content));
    } else {
        body.extend(content);
    }
    riff(version, &body)
}

/// The radius the base curve of a polygon lies on, 1000 mm, in points (R18).
pub const POLYGON_SIDE: f64 = 1000.0 / 25.4 * 72.0;
