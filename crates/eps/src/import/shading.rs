//! Shadings that aren't gradients, as gradient meshes: function-based ones (type 1) sampled on a
//! grid, free-form and lattice triangle meshes (types 4 and 5) and Coons and tensor patch meshes
//! (types 6 and 7), their vertices read from an array or packed in a string or file; and sampled
//! functions (type 0) that shadings and tint transforms use.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::live::GradientMesh;
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point};

use super::graphics::Space;
use super::interp::{Interp, matrix_of};
use super::obj::{DictRef, Key, Obj, PsError, Res, clamp_between, ps_err};

/// Most patches (or triangles) one shading makes: past it, it is left out with a warning.
const MAX_PATCHES: usize = 4096;
/// Rows and columns of the mesh a function-based shading is sampled on.
const FUNCTION_GRID: u32 = 16;
/// Most samples a sampled function has.
const MAX_SAMPLES: usize = 1 << 24;
/// Most outputs (colour components) a sampled function has.
const MAX_OUTPUTS: usize = 32;
/// Work one patch costs against the operation budget.
const PATCH_OPS: u64 = 64;

const TOO_MANY_PATCHES: &str = "shadings of more than 4096 patches or triangles were left out";

/// The numbers of a shading's vertices: from an array, or packed in bits ([`Packed`]).
enum Vertices {
    Nums(Vec<f64>, usize),
    Packed(Packed),
}

/// Vertex data packed in bits: flags, coordinates and colour components, each its own width and
/// mapped through `Decode`.
struct Packed {
    bytes: Vec<u8>,
    /// The next bit.
    at: usize,
    flag: u32,
    coord: u32,
    comp: u32,
    decode: Vec<f64>,
}

impl Packed {
    fn bits(&mut self, n: u32) -> Option<u64> {
        if n == 0 || n > 32 || self.at.checked_add(n as usize)? > self.bytes.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.bytes.get(self.at / 8)?;
            v = v << 1 | u64::from(byte >> (7 - self.at % 8) & 1);
            self.at += 1;
        }
        Some(v)
    }

    /// A value of `n` bits through decode range `i` (`[min max]` pairs).
    fn value(&mut self, n: u32, i: usize) -> Option<f64> {
        let raw = self.bits(n)? as f64;
        let max = ((1u64 << n) - 1) as f64;
        let (lo, hi) = (self.decode.get(2 * i).copied().unwrap_or(0.0), self.decode.get(2 * i + 1).copied().unwrap_or(1.0));
        Some(lo + raw * (hi - lo) / max)
    }
}

impl Vertices {
    fn flag(&mut self) -> Option<u32> {
        match self {
            Self::Nums(v, at) => {
                let f = *v.get(*at)?;
                *at += 1;
                Some(f.max(0.0) as u32)
            }
            Self::Packed(p) => p.bits(p.flag).map(|f| f as u32),
        }
    }

    fn num(&mut self, comp: bool, i: usize) -> Option<f64> {
        match self {
            Self::Nums(v, at) => {
                let f = *v.get(*at)?;
                *at += 1;
                f.is_finite().then_some(f)
            }
            Self::Packed(p) => p.value(if comp { p.comp } else { p.coord }, i),
        }
    }

    fn point(&mut self) -> Option<Point> {
        Some(Point::new(self.num(false, 0)?, self.num(false, 1)?))
    }

    /// A vertex colour's `n` values (components, or the function's input).
    fn color(&mut self, n: usize) -> Option<Vec<f64>> {
        (0..n).map(|i| self.num(true, 2 + i)).collect()
    }

    /// Packed vertices start at a byte (triangle meshes).
    fn align(&mut self) {
        if let Self::Packed(p) = self {
            p.at = p.at.div_ceil(8) * 8;
        }
    }
}

impl Interp<'_> {
    /// Paint shading `sh` (types 1, 4–7) as gradient meshes, `m` mapping shading space onto the
    /// document, under the current clip.
    pub fn mesh_fill(&mut self, sh: &DictRef, m: Affine) -> Res {
        let get = |k: &str| sh.borrow().get(&Key::name(k)).cloned();
        let kind = get("ShadingType").and_then(|o| o.as_num()).unwrap_or(0.0);
        let space = self.space_of(&get("ColorSpace").ok_or(PsError::Ps("undefined", "ColorSpace".into()))?, 0)?;
        let function = get("Function");
        let meshes = if kind == 1.0 { self.function_mesh(sh, &space, m)? } else { self.patch_mesh(sh, kind, &space, function.as_ref(), m)? };
        let Some(meshes) = meshes else {
            self.out.warn(TOO_MANY_PATCHES);
            return Ok(());
        };
        let mut nodes: Vec<Node> = meshes.into_iter().map(|g| Node::new(NodeId(0), NodeKind::Mesh(g))).collect();
        let node = match nodes.len() {
            0 => return Ok(()),
            1 => nodes.swap_remove(0),
            _ => {
                let children = nodes
                    .into_iter()
                    .map(|mut n| {
                        n.id = self.out.doc.alloc_id();
                        Arc::new(n)
                    })
                    .collect();
                Node::group(NodeId(0), children)
            }
        };
        let clips = self.g.clips.clone();
        self.out.push(node, &clips);
        Ok(())
    }

    /// The colour of a vertex's values `v`: through the shading's function when it has one.
    fn vertex_color(&mut self, space: &Space, function: Option<&Obj>, v: &[f64]) -> Res<Color> {
        let comps = match function {
            Some(f) => self.eval(f, v, 0)?,
            None => v.to_vec(),
        };
        Ok(match self.paint_of(space, &comps)? {
            Paint::Solid { color, .. } => color,
            _ => Color::BLACK,
        })
    }

    /// A function-based shading (type 1) sampled on a grid over its domain.
    fn function_mesh(&mut self, sh: &DictRef, space: &Space, m: Affine) -> Res<Option<Vec<GradientMesh>>> {
        let get = |k: &str| sh.borrow().get(&Key::name(k)).cloned();
        let domain: Vec<f64> = get("Domain").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default();
        let [x0, x1, y0, y1] = <[f64; 4]>::try_from(domain).unwrap_or([0.0, 1.0, 0.0, 1.0]);
        let f = get("Function").ok_or(PsError::Ps("undefined", "Function".into()))?;
        let matrix = get("Matrix").and_then(|o| o.items().and_then(|i| matrix_of(&i.borrow()))).unwrap_or(Affine::IDENTITY);
        let n = FUNCTION_GRID as usize;
        let at = |i: usize, lo: f64, hi: f64| lo + (hi - lo) * i as f64 / n as f64;
        let mut colors = Vec::with_capacity((n + 1) * (n + 1));
        for r in 0..=n {
            for c in 0..=n {
                self.spend(PATCH_OPS)?;
                colors.push(self.vertex_color(space, Some(&f), &[at(c, x0, x1), at(r, y0, y1)])?);
            }
        }
        let xf = m * matrix;
        let surface = |u: f64, v: f64| xf * Point::new(x0 + (x1 - x0) * u, y0 + (y1 - y0) * v);
        let color = |u: f64, v: f64| {
            let (c, r) = ((u * n as f64).round() as usize, (v * n as f64).round() as usize);
            colors.get(r * (n + 1) + c).copied().unwrap_or(Color::BLACK)
        };
        Ok(Some(vec![GradientMesh::from_surface(FUNCTION_GRID, FUNCTION_GRID, &surface, &color)]))
    }

    /// The vertices of a mesh shading's `DataSource`.
    fn vertices(&mut self, sh: &DictRef, flags: bool) -> Res<Vertices> {
        let get = |k: &str| sh.borrow().get(&Key::name(k)).cloned();
        let bytes = match get("DataSource").ok_or(PsError::Ps("undefined", "DataSource".into()))? {
            Obj::Array { items, .. } => {
                let v: Option<Vec<f64>> = items.borrow().iter().map(Obj::as_num).collect();
                return Ok(Vertices::Nums(v.ok_or(PsError::Ps("typecheck", "DataSource".into()))?, 0));
            }
            Obj::Str(s) => s.to_vec(),
            Obj::File { stream, .. } => self.peek_all(&stream)?,
            _ => return ps_err("typecheck", "DataSource"),
        };
        let bits = |k: &str| get(k).and_then(|o| o.as_num()).filter(|v| (1.0..=32.0).contains(v)).map(|v| v as u32);
        let (Some(coord), Some(comp)) = (bits("BitsPerCoordinate"), bits("BitsPerComponent")) else {
            return ps_err("rangecheck", "BitsPerCoordinate");
        };
        let flag = if flags { bits("BitsPerFlag").ok_or(PsError::Ps("rangecheck", "BitsPerFlag".into()))? } else { 0 };
        let decode: Vec<f64> = get("Decode").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default();
        Ok(Vertices::Packed(Packed { bytes, at: 0, flag, coord, comp, decode }))
    }

    /// A triangle (types 4, 5) or patch (6, 7) mesh in the document (`m`); `None` past
    /// [`MAX_PATCHES`].
    fn patch_mesh(&mut self, sh: &DictRef, kind: f64, space: &Space, function: Option<&Obj>, m: Affine) -> Res<Option<Vec<GradientMesh>>> {
        let n = if function.is_some() { 1 } else { space.n() };
        let mut v = self.vertices(sh, kind != 5.0)?;
        let mut corner = |it: &mut Self, v: &mut Vertices| -> Res<Option<Color>> {
            match v.color(n) {
                Some(c) => it.vertex_color(space, function, &c).map(Some),
                None => Ok(None),
            }
        };
        if kind == 6.0 || kind == 7.0 {
            return self.patches(&mut v, kind == 7.0, m, &mut corner);
        }
        let per_row = sh.borrow().get(&Key::name("VerticesPerRow")).and_then(Obj::as_num).filter(|r| (2.0..=1e6).contains(r)).map(|r| r as usize);
        let lattice = kind == 5.0;
        let mut pts: Vec<(Point, Color)> = vec![];
        let mut tris: Vec<[usize; 3]> = vec![];
        // Free-form: vertices still to come of the triangle being made.
        let mut left = 0;
        loop {
            let flag = if lattice { 0 } else { v.flag().unwrap_or(u32::MAX) };
            let Some(p) = v.point().filter(|_| flag != u32::MAX) else { break };
            let Some(color) = corner(self, &mut v)? else { break };
            if !lattice {
                v.align();
            }
            self.spend(PATCH_OPS)?;
            pts.push((m * p, color));
            let i = pts.len() - 1;
            if lattice {
                // Each cell between two rows is two triangles.
                if let Some(w) = per_row
                    && i >= w
                    && !i.is_multiple_of(w)
                {
                    tris.push([i - w - 1, i - w, i - 1]);
                    tris.push([i - w, i - 1, i]);
                }
            } else if left > 0 {
                // A vertex ending a triangle begun with flag 0.
                left -= 1;
                if left == 0 && i >= 2 {
                    tris.push([i - 2, i - 1, i]);
                }
            } else {
                // A triangle on the last one's edge (flag 1: its second and third vertices, 2: its
                // first and third), or the first vertex of a new one.
                match (flag, tris.last().copied()) {
                    (1, Some([_, b, c])) => tris.push([b, c, i]),
                    (2, Some([a, _, c])) => tris.push([a, c, i]),
                    _ => left = 2,
                }
            }
            if tris.len() > MAX_PATCHES {
                return Ok(None);
            }
        }
        let at = |i: usize| pts.get(i).copied().unwrap_or((Point::ZERO, Color::BLACK));
        Ok(Some(tris.iter().filter_map(|t| GradientMesh::triangle(t.map(at))).collect()))
    }

    /// Coons (`tensor`: tensor-product) patches: twelve boundary points (a tensor patch's four
    /// inner ones are read and left out), four corner colours; a patch on the last one's edge
    /// (flag 1–3) takes that edge's four points and two colours from it.
    fn patches(
        &mut self,
        v: &mut Vertices,
        tensor: bool,
        m: Affine,
        corner: &mut impl FnMut(&mut Self, &mut Vertices) -> Res<Option<Color>>,
    ) -> Res<Option<Vec<GradientMesh>>> {
        let mut out = vec![];
        let mut prev: Option<([Point; 12], [Color; 4])> = None;
        while let Some(flag) = v.flag() {
            let shared = match (flag, &prev) {
                (0, _) => None,
                (1, Some((p, c))) => Some(([p[3], p[4], p[5], p[6]], [c[1], c[2]])),
                (2, Some((p, c))) => Some(([p[6], p[7], p[8], p[9]], [c[2], c[3]])),
                (3, Some((p, c))) => Some(([p[9], p[10], p[11], p[0]], [c[3], c[0]])),
                _ => return ps_err("rangecheck", "a patch flag"),
            };
            let mut cp = [Point::ZERO; 12];
            let mut colors = [Color::BLACK; 4];
            let (first_point, first_color) = match shared {
                Some((pts, cs)) => {
                    cp[..4].copy_from_slice(&pts);
                    colors[..2].copy_from_slice(&cs);
                    (4, 2)
                }
                None => (0, 0),
            };
            for slot in cp.iter_mut().skip(first_point) {
                let Some(p) = v.point() else { return Ok(Some(out)) };
                *slot = m * p;
            }
            if tensor && (0..4).any(|_| v.point().is_none()) {
                return Ok(Some(out));
            }
            for slot in colors.iter_mut().skip(first_color) {
                let Some(c) = corner(self, v)? else { return Ok(Some(out)) };
                *slot = c;
            }
            self.spend(PATCH_OPS)?;
            if out.len() >= MAX_PATCHES {
                return Ok(None);
            }
            out.extend(GradientMesh::coons(&cp, colors));
            prev = Some((cp, colors));
        }
        Ok(Some(out))
    }

    /// A sampled function (type 0) at `x`: the samples around it, interpolated linearly for
    /// one input, the nearest one for more.
    pub(super) fn sampled(&mut self, f: &DictRef, x: &[f64]) -> Res<Vec<f64>> {
        let get = |k: &str| f.borrow().get(&Key::name(k)).cloned();
        let nums =
            |k: &str| -> Vec<f64> { get(k).and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default() };
        let (domain, range, size) = (nums("Domain"), nums("Range"), nums("Size"));
        let m = domain.len() / 2;
        let outs = range.len() / 2;
        if m == 0 || outs == 0 || outs > MAX_OUTPUTS || size.len() < m || size.iter().any(|s| !(1.0..=MAX_SAMPLES as f64).contains(s)) {
            return ps_err("rangecheck", "a sampled function");
        }
        let bps =
            get("BitsPerSample").and_then(|o| o.as_num()).filter(|b| [1.0, 2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0].contains(b)).map(|b| b as usize);
        let bps = bps.ok_or(PsError::Ps("rangecheck", "BitsPerSample".into()))?;
        if size.iter().take(m).try_fold(1usize, |acc, s| acc.checked_mul(*s as usize)).is_none_or(|t| t > MAX_SAMPLES) {
            return ps_err("limitcheck", "Size");
        }
        let bytes = match get("DataSource") {
            Some(Obj::Str(s)) => s.to_vec(),
            Some(Obj::File { stream, .. }) => self.peek_all(&stream)?,
            _ => return ps_err("typecheck", "DataSource"),
        };
        let (encode, decode) = (nums("Encode"), nums("Decode"));
        let max = ((1u64 << bps) - 1) as f64;
        let sample = |index: usize, j: usize| -> f64 {
            let bit = (index as u64 * outs as u64 + j as u64) * bps as u64;
            let raw = (0..bps as u64).fold(0u64, |v, k| {
                let b = usize::try_from((bit + k) / 8).ok().and_then(|i| bytes.get(i)).copied().unwrap_or(0);
                v << 1 | u64::from(b >> (7 - (bit + k) % 8) & 1)
            }) as f64;
            let (d0, d1) = (
                decode.get(2 * j).or(range.get(2 * j)).copied().unwrap_or(0.0),
                decode.get(2 * j + 1).or(range.get(2 * j + 1)).copied().unwrap_or(1.0),
            );
            let (r0, r1) = (range.get(2 * j).copied().unwrap_or(0.0), range.get(2 * j + 1).copied().unwrap_or(1.0));
            clamp_between(d0 + raw * (d1 - d0) / max, r0, r1)
        };
        // Each input into sample space.
        let mut pos = Vec::with_capacity(m);
        for i in 0..m {
            let (a, b) = (domain.get(2 * i).copied().unwrap_or(0.0), domain.get(2 * i + 1).copied().unwrap_or(1.0));
            let s = size.get(i).copied().unwrap_or(1.0);
            let (e0, e1) = (encode.get(2 * i).copied().unwrap_or(0.0), encode.get(2 * i + 1).copied().unwrap_or(s - 1.0));
            let xi = clamp_between(x.get(i).copied().unwrap_or(a), a, b);
            let e = if b != a { e0 + (xi - a) * (e1 - e0) / (b - a) } else { e0 };
            pos.push(clamp_between(e, 0.0, s - 1.0));
        }
        if let ([p], Some(s)) = (pos.as_slice(), size.first()) {
            let (lo, hi) = (p.floor() as usize, (p.ceil() as usize).min(*s as usize - 1));
            let t = p - p.floor();
            return Ok((0..outs).map(|j| sample(lo, j) * (1.0 - t) + sample(hi, j) * t).collect());
        }
        let mut index = 0;
        let mut stride = 1;
        for (i, p) in pos.iter().enumerate() {
            index += p.round() as usize * stride;
            stride *= size.get(i).copied().unwrap_or(1.0) as usize;
        }
        Ok((0..outs).map(|j| sample(index, j)).collect())
    }
}
