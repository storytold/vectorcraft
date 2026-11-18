//! Gradient meshes: a mesh as the `.ai` reader reads one ([`crate::import`]'s `.ai` reader,
//! `mesh.rs` and `read/art.rs`' `end_mesh`).
//!
//! `/Mesh X!`, then on `%_` lines `value /key X#` entries: `2 /Version` (corners with an opacity),
//! `/Cartesian /Type`, `[rows cols] /Size`, `/Data` (the patches follow), and for each patch, row by
//! row, its place `[row col] /P`, the color space (`/DeviceGray`,
//! `/DeviceRGB` or `/DeviceCMYK`) with `/CS` where it changes, its four corners with `/N` and `/E`;
//! then `/End X!`. A corner is `[color… opacity x y nx ny 1 px py 1]` in art space (in a symbol's
//! art, in the canvas's space turned y up about art space's origin there, as an image's matrix in
//! it is): its color (gray as lightness), its opacity, the point, the
//! handle towards the next corner and the handle towards the one before it, each with a weight of 1.
//! The corners go from the patch's top left mesh point to the one below it, then to the right, then
//! up: the first index of `/P` runs along the first edge. The mesh's transparency is set before it
//! (`0 XR`, `Xy`), its name and opacity mask follow `/End X!` in an `/ArtDictionary`, then `0 0 Xd`
//! and `6 () XW`.
//!
//! Each record is read back with the reader's mesh reading ([`crate::import::mesh_reads`], the
//! patches and how they join; not the rest of the `.ai` reader) before it is written: a mesh it
//! would read otherwise is written as its drawn look, as are folded meshes (patches whose corners
//! turn different ways, along the handles there, or a corner with no angle), meshes in pattern
//! tiles and in art cut to a clipping object, and every mesh on a large canvas. A reflected mesh,
//! whose patches all turn the negative way, is written with its columns in reverse order: it draws
//! the same patches, and reads back with each row's points in reverse order.

use std::fmt::Write;

use vectorcraft_color::Color;
use vectorcraft_doc::live::{GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::{Writer, Xy};
use crate::ai_ops::unit;
use crate::import::{MAX_MESH_SIDE, mesh_reads};
use crate::ps;

pub(in crate::ai_editing) const MESHES: &str = "gradient meshes with a point in a Lab color, with more than 128 rows or columns, folded meshes, meshes that would not open as they are drawn, meshes in pattern tiles and in art cut to a clipping object, and every gradient mesh on a large canvas are written as their drawn look in the editing data for other apps";

/// How far (in the document's units, at a scale of 1) a written point or handle's end may move:
/// coordinates are written to 4 decimals, which moves a point at most 0.00005 along each axis.
const SETTLE_POINT: f64 = 1e-4;
/// How far a written color channel or opacity may move: they are written to 4 decimals too.
const SETTLE_COLOR: f32 = 1e-4;

/// The color space a corner's color is written in.
#[derive(Clone, Copy, PartialEq)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Space::Gray => "/DeviceGray",
            Space::Rgb => "/DeviceRGB",
            Space::Cmyk => "/DeviceCMYK",
        }
    }
}

impl Writer<'_> {
    /// Mesh `m`'s record as the reader reads it, or `None` when it would read it otherwise (the
    /// mesh is then written as its drawn look).
    pub(super) fn mesh_record(&self, m: &GradientMesh) -> Option<String> {
        // Art cut to a clipping object is written cut, which a mesh can't be. Meshes in pattern
        // tiles and on a large canvas are written as their drawn look too. (A mesh holds no objects,
        // so it is written as a mesh however deep it is nested.) A grid the reader doesn't read as
        // one mesh is left before its record is built.
        if !self.meshes_live
            || self.clip.is_some()
            || !self.defining.is_empty()
            || !m.is_valid()
            || m.rows as usize > MAX_MESH_SIDE
            || m.cols as usize > MAX_MESH_SIDE
        {
            return None;
        }
        // In a symbol's art, corners are written in the space an image's matrix is written in there.
        let to_art = if self.in_symbol { self.instance_space } else { self.to_art };
        // Every patch turns the positive way where it is written: from its first corner, the second
        // (below it) then the fourth (beside it) go anticlockwise, y up. A reflected mesh, whose
        // patches all turn the other way, is written with its columns in reverse order, which turns
        // each of them the positive way and draws the same mesh; any other mesh with a patch that
        // doesn't turn the positive way is written as its drawn look.
        let reversed;
        let m = match turns(m, to_art)? {
            true => m,
            false => {
                reversed = columns_reversed(m);
                &reversed
            }
        };
        let (rows, cols) = (m.rows as usize, m.cols as usize);
        let at = |r: usize, c: usize| m.points.get(r * (cols + 1) + c);
        // Version 2: each corner has an opacity.
        let mut s = String::from("/Mesh X!\n%_2 /Version X#\n%_/Cartesian /Type X#\n");
        let _ = writeln!(s, "%_[{rows} {cols}] /Size X#\n%_/Data X#");
        let mut space = None;
        for r in 0..rows {
            for c in 0..cols {
                let _ = writeln!(s, "%_[{r} {c}] /P X#");
                // From the top left point down, right and up; each with its handles towards the
                // next corner and the one before it.
                for (p, next, prev) in
                    [(at(r, c)?, H_DOWN, H_RIGHT), (at(r + 1, c)?, H_RIGHT, H_UP), (at(r + 1, c + 1)?, H_UP, H_LEFT), (at(r, c + 1)?, H_LEFT, H_DOWN)]
                {
                    corner(&mut s, &mut space, to_art, p, next, prev)?;
                }
                s.push_str("%_/E X#\n");
            }
        }
        s.push_str("/End X!\n");
        let read = mesh_reads(s.as_bytes(), to_art.inverse())?;
        same_mesh(&read, m, SETTLE_POINT * to_art.inverse().determinant().abs().sqrt().max(1.0)).then_some(s)
    }

    /// Mesh `n` written as `record`: its transparency before it (`xy`), the record, and the block
    /// its name and opacity mask go before ([`Self::node`]). The mesh is one object
    /// ([`Self::take_objects`]); nothing is written when there is no room for it.
    pub(super) fn write_mesh(&mut self, record: &str, xy: Xy) {
        if !self.take_objects(1) {
            return;
        }
        self.set_object_xy(xy.object());
        self.body.push_str(record);
        self.block_start = self.body.len();
        self.line("0 0 Xd\n6 () XW");
        self.block_end = self.body.len();
    }
}

/// Which way every patch of `m` turns where `to_art` takes it, at each of its four corners, from
/// its top left point to the one below it, then right and up, y up: `Some(true)` when all turn
/// anticlockwise, `Some(false)` when all turn clockwise, and `None` when they differ (a patch
/// folded across itself, or patches folded over each other) or a corner has no angle. Each corner
/// is measured twice: along the straight lines to the next corners, and along its handles (towards
/// the next corners where a handle is zero), where the patch's edges leave it.
fn turns(m: &GradientMesh, to_art: Affine) -> Option<bool> {
    let (rows, cols) = (m.rows as usize, m.cols as usize);
    let at = |r: usize, c: usize| m.points.get(r * (cols + 1) + c);
    let mut way = None;
    for r in 0..rows {
        for c in 0..cols {
            // Each corner with the slots of its handles towards the next corner and the one before it.
            let corners =
                [(at(r, c)?, H_DOWN, H_RIGHT), (at(r + 1, c)?, H_RIGHT, H_UP), (at(r + 1, c + 1)?, H_UP, H_LEFT), (at(r, c + 1)?, H_LEFT, H_DOWN)];
            for i in 0..4 {
                let (before, (here, next, prev), after) = (corners[(i + 3) % 4].0, corners[i], corners[(i + 1) % 4].0);
                let line = |to: &MeshPoint| to_art * to.p - to_art * here.p;
                let along = |h: usize, to: &MeshPoint| -> Option<Vec2> {
                    let h = *here.handles.get(h)?;
                    Some(if h == Vec2::ZERO { line(to) } else { to_art * (here.p + h) - to_art * here.p })
                };
                for cross in [line(after).cross(line(before)), along(next, after)?.cross(along(prev, before)?)] {
                    let positive = match cross {
                        x if x > 0.0 => true,
                        x if x < 0.0 => false,
                        _ => return None,
                    };
                    if *way.get_or_insert(positive) != positive {
                        return None;
                    }
                }
            }
        }
    }
    way
}

/// `m` with its columns in reverse order: each row's points from the last to the first, each
/// point's handles to the right and left swapped. It draws the same mesh.
fn columns_reversed(m: &GradientMesh) -> GradientMesh {
    let width = m.cols as usize + 1;
    let points = m
        .points
        .chunks(width)
        .flat_map(|row| {
            row.iter().rev().map(|p| {
                let mut p = p.clone();
                p.handles.swap(H_RIGHT, H_LEFT);
                p
            })
        })
        .collect();
    GradientMesh { rows: m.rows, cols: m.cols, points }
}

/// A corner `p` of a patch: its color space when it changes (in `space`), then `[color… opacity x y
/// nx ny 1 px py 1] /N X#` with its handles `next` and `prev`, `to_art` taking the document to the
/// space the corners are written in.
fn corner(s: &mut String, space: &mut Option<Space>, to_art: Affine, p: &MeshPoint, next: usize, prev: usize) -> Option<()> {
    let (sp, values): (Space, Vec<f32>) = match p.color {
        // The format's gray is lightness.
        Color::Gray { k } => (Space::Gray, vec![1.0 - unit(k)]),
        Color::Rgb { r, g, b } => (Space::Rgb, vec![unit(r), unit(g), unit(b)]),
        Color::Cmyk { c, m, y, k } => (Space::Cmyk, vec![unit(c), unit(m), unit(y), unit(k)]),
        // The reader reads no Lab mesh colors.
        Color::Lab { .. } => return None,
    };
    if *space != Some(sp) {
        let _ = writeln!(s, "%_{} /CS X#", sp.name());
        *space = Some(sp);
    }
    let art = |q: Point| to_art * q;
    let (at, n, b) = (art(p.p), art(p.p + *p.handles.get(next)?), art(p.p + *p.handles.get(prev)?));
    s.push_str("%_[");
    for v in values {
        ps::push_num(s, f64::from(v));
        s.push(' ');
    }
    ps::push_num(s, f64::from(unit(p.opacity)));
    for v in [at.x, at.y, n.x, n.y] {
        s.push(' ');
        ps::push_num(s, v);
    }
    s.push_str(" 1");
    for v in [b.x, b.y] {
        s.push(' ');
        ps::push_num(s, v);
    }
    s.push_str(" 1] /N X#\n");
    Some(())
}

/// Whether `read` is `m` as written: the same grid, each point within `tol` of its place, each
/// handle that points into the grid within `tol` (the reader gives the others none, and they draw
/// nothing), each color in the same model and within [`SETTLE_COLOR`] a channel, and each opacity
/// within it.
fn same_mesh(read: &GradientMesh, m: &GradientMesh, tol: f64) -> bool {
    if (read.rows, read.cols) != (m.rows, m.cols) || read.points.len() != m.points.len() {
        return false;
    }
    let (rows, cols) = (m.rows as usize, m.cols as usize);
    let near = |a: Point, b: Point| (a - b).hypot() <= tol * (1.0 + 1e-9 * a.to_vec2().hypot());
    read.points.iter().zip(&m.points).enumerate().all(|(i, (a, b))| {
        let (r, c) = (i / (cols + 1), i % (cols + 1));
        let inside = [c < cols, c > 0, r < rows, r > 0];
        let handles = [H_RIGHT, H_LEFT, H_DOWN, H_UP].iter().zip(inside).all(|(&h, inside)| {
            !inside || near(a.p + a.handles.get(h).copied().unwrap_or(Vec2::ZERO), b.p + b.handles.get(h).copied().unwrap_or(Vec2::ZERO))
        });
        near(a.p, b.p) && handles && same_color(&a.color, &b.color) && (a.opacity - unit(b.opacity)).abs() <= SETTLE_COLOR
    })
}

/// Whether `a` is `b` as written: the same model, each channel within [`SETTLE_COLOR`] of `b`'s
/// clamped to 0 to 1.
fn same_color(a: &Color, b: &Color) -> bool {
    let close = |x: &[f32], y: &[f32]| x.iter().zip(y).all(|(x, y)| (x - unit(*y)).abs() <= SETTLE_COLOR);
    match (a, b) {
        (Color::Gray { k }, Color::Gray { k: k2 }) => close(&[*k], &[*k2]),
        (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) => close(&[*r, *g, *b], &[*r2, *g2, *b2]),
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => close(&[*c, *m, *y, *k], &[*c2, *m2, *y2, *k2]),
        _ => false,
    }
}
