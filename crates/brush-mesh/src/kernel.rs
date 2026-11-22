//! The edits themselves, in kernel form (f32 arguments, boxes as [`Area`]). Each returns a count describing what it
//! did. [`crate::op::Op`] is the typed, serialisable front door to these.

use crate::math::*;
use crate::mesh::{Mesh, Src};
use crate::region::Area;
use rustc_hash::FxHashMap;

/// Meshes never grow past this many faces through cloning.
pub const MAX_FACES: usize = 2_000_000;

// ---- removing ----

impl Mesh {
    /// Deletes faces whose centroid is inside the box (or outside, with `invert`). Returns faces removed.
    pub fn remove_box(&mut self, a: Area, invert: bool) -> usize {
        let f = self.face_count();
        let keep: Vec<bool> = (0..f)
            .map(|t| {
                let [i, j, k] = self.face(t);
                let (p, q, r) = (self.vert(i), self.vert(j), self.vert(k));
                let m = [
                    (p[0] + q[0] + r[0]) / 3.0,
                    (p[1] + q[1] + r[1]) / 3.0,
                    (p[2] + q[2] + r[2]) / 3.0,
                ];
                let l = to_local(m, a.c, a.yaw);
                let inside = l[0].abs() <= a.h[0] && l[1].abs() <= a.h[1] && l[2].abs() <= a.h[2];
                inside == invert
            })
            .collect();
        self.retain_faces(|t| keep[t]);
        let removed = f - self.face_count();
        self.compact();
        // Paint in the removed region goes with it.
        self.splats.retain(|_, p| a.contains(p) == invert);
        removed
    }

    /// Cuts the box out of the surface. Triangles crossing the box boundary are split along it and only the part
    /// inside is dropped, so the hole has the exact shape of the box. Returns the triangles cut or removed.
    pub fn carve_box(&mut self, a: Area) -> usize {
        let f0 = self.face_count();
        let mut cut = vec![false; f0];
        let mut count = 0;
        for t in 0..f0 {
            let ids = self.face(t);
            let corner: [Pt; 3] = ids.map(|vi| {
                let c = self.color(vi);
                Pt {
                    p: to_local(self.vert(vi), a.c, a.yaw),
                    c: [c[0] as f32, c[1] as f32, c[2] as f32],
                    src: vi,
                }
            });
            // All three corners beyond (or on) the same face of the box: nothing to cut.
            let apart = (0..3).any(|ax| {
                corner.iter().all(|k| k.p[ax] <= -a.h[ax])
                    || corner.iter().all(|k| k.p[ax] >= a.h[ax])
            });
            if apart {
                continue;
            }
            let mut inside = Poly::tri(corner);
            let mut kept: Vec<Poly> = Vec::with_capacity(6);
            for ax in 0..3 {
                kept.push(inside.clip(ax, -a.h[ax], false)); // beyond the low face: kept
                inside = inside.clip(ax, -a.h[ax], true);
                kept.push(inside.clip(ax, a.h[ax], true)); // beyond the high face: kept
                inside = inside.clip(ax, a.h[ax], false);
                if inside.n < 3 {
                    break;
                }
            }
            if inside.n < 3 {
                continue; // no part of this triangle is inside the box
            }
            cut[t] = true;
            count += 1;
            for piece in &kept {
                self.emit_local(piece, a);
            }
        }
        // Paint inside the carved box is cut away with the surface.
        self.splats.retain(|_, p| !a.contains(p));
        if count == 0 {
            return 0;
        }
        self.retain_faces(|t| t >= f0 || !cut[t]);
        self.compact();
        count
    }

    /// Appends a convex polygon given in box-local space as a triangle fan, in world space.
    fn emit_local(&mut self, poly: &Poly, a: Area) {
        if poly.n < 3 {
            return;
        }
        let mut first = 0;
        for i in 0..poly.n {
            let v = &poly.v[i];
            let w = to_world(v.p, a.c, a.yaw);
            let k = self.push_vertex(w, [q8(v.c[0]), q8(v.c[1]), q8(v.c[2])], Src::Copy(v.src));
            if i == 0 {
                first = k;
            }
        }
        for j in 1..(poly.n as u32 - 1) {
            self.indices
                .extend_from_slice(&[first, first + j, first + j + 1]);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Pt {
    p: Vec3,
    c: [f32; 3],
    /// The original vertex nearest in the clip chain, whose attributes a new vertex takes.
    src: u32,
}

#[derive(Clone, Copy)]
struct Poly {
    n: usize,
    v: [Pt; 12],
}

impl Poly {
    fn tri(c: [Pt; 3]) -> Poly {
        let mut p = Poly {
            n: 3,
            v: [Pt::default(); 12],
        };
        p.v[..3].copy_from_slice(&c);
        p
    }

    /// The part of a convex polygon on one side of the plane `p[axis] == value`.
    fn clip(&self, axis: usize, value: f32, keep_ge: bool) -> Poly {
        let mut out = Poly {
            n: 0,
            v: [Pt::default(); 12],
        };
        for i in 0..self.n {
            let a = self.v[i];
            let b = self.v[(i + 1) % self.n];
            let da = if keep_ge {
                a.p[axis] - value
            } else {
                value - a.p[axis]
            };
            let db = if keep_ge {
                b.p[axis] - value
            } else {
                value - b.p[axis]
            };
            if da >= 0.0 {
                out.v[out.n] = a;
                out.n += 1;
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                out.v[out.n] = Pt {
                    p: [
                        a.p[0] + (b.p[0] - a.p[0]) * t,
                        a.p[1] + (b.p[1] - a.p[1]) * t,
                        a.p[2] + (b.p[2] - a.p[2]) * t,
                    ],
                    c: [
                        a.c[0] + (b.c[0] - a.c[0]) * t,
                        a.c[1] + (b.c[1] - a.c[1]) * t,
                        a.c[2] + (b.c[2] - a.c[2]) * t,
                    ],
                    src: if t < 0.5 { a.src } else { b.src },
                };
                out.n += 1;
            }
        }
        out
    }
}

// ---- reshaping ----

impl Mesh {
    /// Smooths the vertices in the area (chosen once, before any moves), `iters` passes moving each vertex
    /// `lambda` of the way to the average of its neighbours. With `taubin`, every pass is followed by a slightly
    /// stronger step back (Taubin's λ|μ filter, pass-band 0.1), which removes noise without shrinking the surface.
    /// With `keep_boundary`, vertices on open edges stay put so holes and outlines keep their shape. Returns the
    /// vertices that move.
    pub fn smooth(
        &mut self,
        a: Area,
        iters: u32,
        lambda: f32,
        taubin: bool,
        keep_boundary: bool,
    ) -> usize {
        let nb = crate::topo::vertex_neighbors(self);
        let boundary = if keep_boundary {
            crate::topo::boundary_vertices(self)
        } else {
            vec![false; self.vertex_count()]
        };
        let movable: Vec<u32> = (0..self.vertex_count())
            .filter(|&i| !nb.row(i).is_empty() && !boundary[i] && a.contains(self.vert(i as u32)))
            .map(|i| i as u32)
            .collect();
        if movable.is_empty() || lambda == 0.0 {
            return 0;
        }
        // μ from 1/λ + 1/μ = k_pb, the standard non-shrinking choice.
        let mu = if taubin && lambda > 0.0 {
            1.0 / (0.1 - 1.0 / lambda)
        } else {
            0.0
        };
        let mut next = vec![[0f32; 3]; movable.len()];
        let mut step = |m: &mut Mesh, factor: f32| {
            for (k, &i) in movable.iter().enumerate() {
                let row = nb.row(i as usize);
                let mut s = [0f32; 3];
                for &j in row {
                    let q = m.vert(j);
                    s = [s[0] + q[0], s[1] + q[1], s[2] + q[2]];
                }
                let n = row.len() as f32;
                let p = m.vert(i);
                next[k] = [0, 1, 2].map(|j| p[j] + factor * (s[j] / n - p[j]));
            }
            for (k, &i) in movable.iter().enumerate() {
                m.positions[i as usize * 3..i as usize * 3 + 3].copy_from_slice(&next[k]);
            }
        };
        for _ in 0..iters {
            step(self, lambda);
            if mu != 0.0 {
                step(self, mu);
            }
        }
        movable.len()
    }

    /// Vertex-clustering decimation on a grid of `cell` meters. Returns the new face count.
    pub fn decimate(&mut self, cell_size: f32) -> usize {
        if cell_size <= 0.0 {
            return self.face_count();
        }
        let v = self.vertex_count();
        let mut map: FxHashMap<[i32; 3], u32> = FxHashMap::default();
        let mut remap = vec![0u32; v];
        let mut acc: Vec<[f32; 7]> = Vec::new(); // x, y, z, r, g, b, count
        let mut members: Vec<Vec<u32>> = Vec::new();
        for i in 0..v {
            let key = [
                cell(self.positions[i * 3], cell_size),
                cell(self.positions[i * 3 + 1], cell_size),
                cell(self.positions[i * 3 + 2], cell_size),
            ];
            let g = *map.entry(key).or_insert_with(|| {
                acc.push([0.0; 7]);
                members.push(Vec::new());
                (acc.len() - 1) as u32
            });
            let o = &mut acc[g as usize];
            for j in 0..3 {
                o[j] += self.positions[i * 3 + j];
                o[3 + j] += self.colors[i * 3 + j] as f32;
            }
            o[6] += 1.0;
            remap[i] = g;
            if !self.attributes.is_empty() {
                members[g as usize].push(i as u32);
            }
        }
        let mut faces = Vec::with_capacity(self.indices.len());
        for t in 0..self.face_count() {
            let [p, q, r] = self.face(t).map(|i| remap[i as usize]);
            if p == q || q == r || p == r {
                continue;
            }
            faces.extend_from_slice(&[p, q, r]);
        }
        let mut out = Mesh {
            attributes: self
                .attributes
                .iter()
                .map(|a| crate::mesh::Attribute {
                    values: Vec::with_capacity(acc.len()),
                    ..a.clone()
                })
                .collect(),
            ..Mesh::default()
        };
        let old = std::mem::take(&mut self.attributes);
        for (g, o) in acc.iter().enumerate() {
            let n = o[6];
            out.positions
                .extend_from_slice(&[o[0] / n, o[1] / n, o[2] / n]);
            out.colors
                .extend((0..3).map(|j| (o[3 + j] / n).min(255.0) as u8));
            for (dst, src) in out.attributes.iter_mut().zip(&old) {
                let m = &members[g];
                let val = if src.kind.is_integer() {
                    src.values[m[0] as usize]
                } else {
                    m.iter().map(|&i| src.values[i as usize]).sum::<f64>() / m.len() as f64
                };
                dst.values.push(val);
            }
        }
        out.indices = faces;
        *self = out;
        self.compact();
        self.face_count()
    }

    /// Splits every edge longer than `max_edge` whose midpoint is in the area (the same decision from both sides of
    /// an edge, so no cracks open), up to `passes` rounds, stopping short of `max_faces`. Returns the face count.
    pub fn subdivide(&mut self, max_edge: f32, passes: u32, max_faces: u32, a: Area) -> usize {
        if max_edge <= 0.0 {
            return self.face_count();
        }
        let max_sq = max_edge * max_edge;
        let split = |m: &Mesh, p: u32, q: u32| -> bool {
            let (pa, pb) = (m.vert(p), m.vert(q));
            let d = sub(pa, pb);
            if dot(d, d) <= max_sq {
                return false;
            }
            a.intersects_segment(pa, pb)
        };
        let mut pass = 0;
        while pass < passes && self.face_count() < max_faces as usize {
            pass += 1;
            let f0 = self.face_count();
            // Count what this pass would add first, and stop short of the face limit rather than overshoot it.
            let mut extra = 0;
            for t in 0..f0 {
                let v = self.face(t);
                for e in 0..3 {
                    if split(self, v[e], v[(e + 1) % 3]) {
                        extra += 1;
                    }
                }
            }
            if extra == 0 || f0 + extra > max_faces as usize {
                break;
            }
            let mut mids: FxHashMap<(u32, u32), u32> = FxHashMap::default();
            let mut out: Vec<u32> = Vec::with_capacity(self.indices.len() * 2);
            for t in 0..f0 {
                let v = self.face(t);
                let mut mask = 0u32;
                for e in 0..3 {
                    if split(self, v[e], v[(e + 1) % 3]) {
                        mask |= 1 << e;
                    }
                }
                if mask == 0 {
                    out.extend_from_slice(&v);
                    continue;
                }
                let mut m = [0u32; 3];
                for e in 0..3 {
                    if mask & (1 << e) == 0 {
                        continue;
                    }
                    let (p, q) = (v[e], v[(e + 1) % 3]);
                    let key = (p.min(q), p.max(q));
                    m[e] = match mids.get(&key) {
                        Some(&k) => k,
                        None => {
                            let (pa, pb) = (self.vert(p), self.vert(q));
                            let (ca, cb) = (self.color(p), self.color(q));
                            let c = [0, 1, 2].map(|j| ((ca[j] as u32 + cb[j] as u32) / 2) as u8);
                            let mid = [
                                (pa[0] + pb[0]) / 2.0,
                                (pa[1] + pb[1]) / 2.0,
                                (pa[2] + pb[2]) / 2.0,
                            ];
                            let k = self.push_vertex(mid, c, Src::Mid(p, q));
                            mids.insert(key, k);
                            k
                        }
                    };
                }
                match mask.count_ones() {
                    3 => out.extend_from_slice(&[
                        v[0], m[0], m[2], m[0], v[1], m[1], m[2], m[1], v[2], m[0], m[1], m[2],
                    ]),
                    2 => {
                        let k = (0..3).rev().find(|&e| mask & (1 << e) == 0).unwrap(); // the edge that stays whole
                        let (pa, pb, pc) = (v[(k + 1) % 3], v[(k + 2) % 3], v[k]);
                        let (mab, mbc) = (m[(k + 1) % 3], m[(k + 2) % 3]);
                        out.extend_from_slice(&[mab, pb, mbc, pa, mab, mbc, pa, mbc, pc]);
                    }
                    _ => {
                        let k = (0..3).rev().find(|&e| mask & (1 << e) != 0).unwrap(); // the edge that is split
                        let (pa, pb, pc) = (v[k], v[(k + 1) % 3], v[(k + 2) % 3]);
                        out.extend_from_slice(&[pa, m[k], pc, m[k], pb, pc]);
                    }
                }
            }
            self.indices = out;
        }
        self.face_count()
    }

    /// Closes every hole (boundary loop) of at most `max_edges` edges. Returns faces added.
    ///
    /// Loops are traced edge by edge, not vertex by vertex. Where a hole touches itself at a single vertex (a pinch),
    /// the walk rotates around that vertex's fan of faces to find the boundary edge that continues the same hole. Loops
    /// are visited in the order their first boundary edge appears in the face list, so the result is the same
    /// everywhere.
    pub fn fill_holes(&mut self, max_edges: u32) -> usize {
        self.drop_pinned_slivers();
        let f = self.face_count();
        let undirected = crate::topo::edges(self);
        let is_open = |p: u32, q: u32| {
            undirected
                .binary_search_by(|e| (e.0, e.1).cmp(&(p.min(q), p.max(q))))
                .is_ok_and(|i| undirected[i].2 == 1)
        };
        // Directed edges (from, to, face), sorted, to find the face across an edge.
        let mut directed: Vec<(u32, u32, u32)> = Vec::with_capacity(f * 3);
        for t in 0..f {
            let v = self.face(t);
            for k in 0..3 {
                directed.push((v[k], v[(k + 1) % 3], t as u32));
            }
        }
        directed.sort_unstable();
        let face_with = |from: u32, to: u32| -> Option<u32> {
            let i = directed.partition_point(|e| (e.0, e.1) < (from, to));
            directed
                .get(i)
                .filter(|e| e.0 == from && e.1 == to)
                .map(|e| e.2)
        };
        // A boundary edge is (face, corner k): the edge from corner k to corner k + 1.
        let mut used = vec![false; f * 3];
        let mut added = 0;
        for t0 in 0..f {
            for k0 in 0..3 {
                if used[t0 * 3 + k0] {
                    continue;
                }
                let v = self.face(t0);
                if !is_open(v[k0], v[(k0 + 1) % 3]) {
                    continue;
                }
                // Walk the boundary in face-edge direction, collecting each edge's start vertex.
                let mut ring: Vec<u32> = Vec::new();
                let (mut t, mut k) = (t0, k0);
                let mut ok = true;
                loop {
                    used[t * 3 + k] = true;
                    let fv = self.face(t);
                    ring.push(fv[k]);
                    let q = fv[(k + 1) % 3];
                    // Rotate around q from face t until the next open edge leaving q.
                    let (mut g, mut steps) = (t, 0);
                    let next = loop {
                        let gv = self.face(g);
                        let j = gv.iter().position(|&x| x == q).unwrap();
                        let s = gv[(j + 1) % 3];
                        if is_open(q, s) {
                            break Some((g, j));
                        }
                        steps += 1;
                        match face_with(s, q) {
                            Some(h) if steps < 64 => g = h as usize,
                            _ => break None, // inconsistent winding or a non-manifold fan
                        }
                    };
                    let Some((nt, nk)) = next else {
                        ok = false;
                        break;
                    };
                    if (nt, nk) == (t0, k0) {
                        break;
                    }
                    if used[nt * 3 + nk] || ring.len() >= max_edges as usize {
                        ok = false;
                        break;
                    }
                    (t, k) = (nt, nk);
                }
                if !ok || ring.len() < 3 {
                    continue;
                }
                // The new faces run against the boundary edges, matching the winding of the faces around them.
                ring.reverse();
                for part in split_pinches(&ring) {
                    added += self.close_loop(&part);
                }
            }
        }
        added
    }

    /// Removes faces whose three edges are all open but which touch other faces at a corner: slivers left inside a
    /// hole by a cut. They would make the hole's outline non-simple (and their own outline would read as a hole).
    /// Free-floating triangles, touching nothing, are left alone. Returns faces removed.
    fn drop_pinned_slivers(&mut self) -> usize {
        let edges = crate::topo::edges(self);
        let open = |p: u32, q: u32| {
            edges
                .binary_search_by(|e| (e.0, e.1).cmp(&(p.min(q), p.max(q))))
                .is_ok_and(|i| edges[i].2 == 1)
        };
        let mut uses = vec![0u32; self.vertex_count()];
        for &i in &self.indices {
            uses[i as usize] += 1;
        }
        let f = self.face_count();
        let drop: Vec<bool> = (0..f)
            .map(|t| {
                let v = self.face(t);
                (0..3).all(|k| open(v[k], v[(k + 1) % 3]))
                    && v.iter().any(|&i| uses[i as usize] > 1)
            })
            .collect();
        let n = drop.iter().filter(|&&d| d).count();
        if n > 0 {
            self.retain_faces(|t| !drop[t]);
            self.compact();
        }
        n
    }

    /// Triangulates one hole. The loop is projected onto its best-fit plane (Newell normal) and ears are clipped,
    /// always taking the best-shaped ear, so concave holes fill correctly without new vertices. If the projection
    /// folds over itself (a saddle-shaped hole), what remains is closed with a fan around its centroid.
    fn close_loop(&mut self, ring: &[u32]) -> usize {
        let pts: Vec<Vec3> = ring.iter().map(|&i| self.vert(i)).collect();
        let n = pts.len();
        let mut c = [0f32; 3];
        for p in &pts {
            c = [c[0] + p[0], c[1] + p[1], c[2] + p[2]];
        }
        c = c.map(|v| v / n as f32);
        let mut nrm = [0f32; 3];
        for k in 0..n {
            let x = cross(sub(pts[k], c), sub(pts[(k + 1) % n], c));
            nrm = [nrm[0] + x[0], nrm[1] + x[1], nrm[2] + x[2]];
        }
        let nrm = norm(nrm);
        let helper = if nrm[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = norm(cross(helper, nrm));
        let v = cross(nrm, u);
        let flat: Vec<[f32; 2]> = pts
            .iter()
            .map(|p| {
                let d = sub(*p, c);
                [dot(d, u), dot(d, v)]
            })
            .collect();
        // Tolerances relative to the loop's size, so collinear runs (common along straight cuts) are recognised.
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for p in &flat {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let eps = 1e-6 * ((hi[0] - lo[0]).max(hi[1] - lo[1])).powi(2);
        let orient = |a: [f32; 2], b: [f32; 2], p: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        // An ear is blocked by a vertex strictly inside it, or on the diagonal it would add (a T-junction).
        let blocks = |p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            let (d1, d2, d3) = (orient(a, b, p), orient(b, c, p), orient(c, a, p));
            if d1 > eps && d2 > eps && d3 > eps {
                return true;
            }
            let ac = [c[0] - a[0], c[1] - a[1]];
            let t = (p[0] - a[0]) * ac[0] + (p[1] - a[1]) * ac[1];
            d3.abs() <= eps && t > 0.0 && t < ac[0] * ac[0] + ac[1] * ac[1]
        };
        let mut order: Vec<usize> = (0..n).collect();
        let mut added = 0;
        while order.len() > 3 {
            let m = order.len();
            let mut best: Option<(f32, usize)> = None;
            for k in 0..m {
                let (ia, ib, ic) = (order[(k + m - 1) % m], order[k], order[(k + 1) % m]);
                let (a, b, cc) = (flat[ia], flat[ib], flat[ic]);
                let area2 = orient(a, b, cc);
                if area2 <= eps {
                    continue; // reflex or collinear corner
                }
                if order
                    .iter()
                    .any(|&iq| iq != ia && iq != ib && iq != ic && blocks(flat[iq], a, b, cc))
                {
                    continue;
                }
                let l2 = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
                let quality = area2 / (l2(a, b) + l2(b, cc) + l2(cc, a));
                if best.is_none_or(|(q, _)| quality > q) {
                    best = Some((quality, k));
                }
            }
            let Some((_, k)) = best else { break };
            let (ia, ib, ic) = (order[(k + m - 1) % m], order[k], order[(k + 1) % m]);
            self.indices
                .extend_from_slice(&[ring[ia], ring[ib], ring[ic]]);
            added += 1;
            order.remove(k);
        }
        if order.len() == 3 {
            self.indices
                .extend_from_slice(&[ring[order[0]], ring[order[1]], ring[order[2]]]);
            return added + 1;
        }
        // Fallback: a fan around the centroid of what is left.
        let rest: Vec<u32> = order.iter().map(|&k| ring[k]).collect();
        let mut s = [0f32; 3];
        let mut col = [0u32; 3];
        for &vi in &rest {
            let (p, cl) = (self.vert(vi), self.color(vi));
            for j in 0..3 {
                s[j] += p[j];
                col[j] += cl[j] as u32;
            }
        }
        let (nf, nl) = (rest.len() as f32, rest.len() as u32);
        let cv = self.push_vertex(
            [s[0] / nf, s[1] / nf, s[2] / nf],
            col.map(|x| (x / nl) as u8),
            Src::Mean(&rest),
        );
        for k in 0..rest.len() {
            self.indices
                .extend_from_slice(&[rest[k], rest[(k + 1) % rest.len()], cv]);
            added += 1;
        }
        added
    }
}

// ---- colour ----

impl Mesh {
    /// Paints the vertices inside the box. Returns vertices painted.
    pub fn paint_box(&mut self, a: Area, rgb: [u8; 3]) -> usize {
        let mut n = 0;
        for i in 0..self.vertex_count() {
            if !a.contains(self.vert(i as u32)) {
                continue;
            }
            self.colors[i * 3..i * 3 + 3].copy_from_slice(&rgb);
            n += 1;
        }
        n
    }
}

// ---- primitives ----

impl Mesh {
    fn add_vert(&mut self, p: Vec3, c: [u8; 3]) -> u32 {
        self.push_vertex(p, c, Src::Zero)
    }

    /// An oriented box (12 triangles, outward winding) whose bottom-face centre is `c`. Returns the first new face.
    pub fn add_box(&mut self, c: Vec3, w: f32, h: f32, d: f32, yaw: f32, rgb: [u8; 3]) -> usize {
        let first = self.face_count();
        let (co, si) = (cos(yaw), sin(yaw));
        let mut corner = [0u32; 8];
        for (k, slot) in corner.iter_mut().enumerate() {
            let sx = if k & 1 != 0 { 0.5 } else { -0.5 };
            let sy = if k & 2 != 0 { 1.0 } else { 0.0 };
            let sz = if k & 4 != 0 { 0.5 } else { -0.5 };
            let (lx, lz) = (sx * w, sz * d);
            *slot = self.add_vert(
                [
                    c[0] + co * lx + si * lz,
                    c[1] + sy * h,
                    c[2] - si * lx + co * lz,
                ],
                rgb,
            );
        }
        for q in [
            [0, 4, 6, 2],
            [1, 3, 7, 5],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 2, 3, 1],
            [4, 5, 7, 6],
        ] {
            self.indices.extend_from_slice(&[
                corner[q[0]],
                corner[q[1]],
                corner[q[2]],
                corner[q[0]],
                corner[q[2]],
                corner[q[3]],
            ]);
        }
        first
    }

    /// A ramp: a wedge on the footprint of a `w` x `d` box whose bottom-face centre is `c`, rising from the box's -z
    /// edge (in its own frame, turned by `yaw`) to height `h` at its +z face. 8 triangles, outward winding. Returns the
    /// first new face.
    pub fn add_ramp(&mut self, c: Vec3, w: f32, h: f32, d: f32, yaw: f32, rgb: [u8; 3]) -> usize {
        let first = self.face_count();
        let (co, si) = (cos(yaw), sin(yaw));
        // The box's corner numbering (bit 0: +x, bit 1: top, bit 2: +z), without the two top corners at -z.
        let mut corner = [0u32; 8];
        for k in [0usize, 1, 4, 5, 6, 7] {
            let sx = if k & 1 != 0 { 0.5 } else { -0.5 };
            let sy = if k & 2 != 0 { 1.0 } else { 0.0 };
            let sz = if k & 4 != 0 { 0.5 } else { -0.5 };
            let (lx, lz) = (sx * w, sz * d);
            corner[k] = self.add_vert(
                [
                    c[0] + co * lx + si * lz,
                    c[1] + sy * h,
                    c[2] - si * lx + co * lz,
                ],
                rgb,
            );
        }
        for t in [[0, 4, 6], [1, 7, 5]] {
            self.indices.extend(t.map(|k| corner[k]));
        }
        // Bottom, slope, back.
        for q in [[0, 1, 5, 4], [0, 6, 7, 1], [4, 5, 7, 6]] {
            self.indices.extend_from_slice(&[
                corner[q[0]],
                corner[q[1]],
                corner[q[2]],
                corner[q[0]],
                corner[q[2]],
                corner[q[3]],
            ]);
        }
        first
    }

    /// A copy of `src`: its bounding-box bottom centre moved to `at`, scaled by `scale` and turned by `yaw` about +y
    /// (the same turn as a box's yaw). Colours come along; named attributes start at zero. Returns the first new face.
    pub fn add_mesh(&mut self, src: &Mesh, at: Vec3, yaw: f32, scale: f32) -> usize {
        let first = self.face_count();
        let anchor = match src.bounds() {
            Some((lo, hi)) => [(lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5],
            // No vertices: the anchor is the origin; splats (if any) still come along.
            None => [0.0; 3],
        };
        let (co, si) = (cos(yaw), sin(yaw));
        let base = self.vertex_count() as u32;
        for (i, p) in src.positions.chunks_exact(3).enumerate() {
            let l = [
                (p[0] - anchor[0]) * scale,
                (p[1] - anchor[1]) * scale,
                (p[2] - anchor[2]) * scale,
            ];
            let c = [
                src.colors[i * 3],
                src.colors[i * 3 + 1],
                src.colors[i * 3 + 2],
            ];
            self.add_vert(
                [
                    at[0] + co * l[0] + si * l[2],
                    at[1] + l[1],
                    at[2] - si * l[0] + co * l[2],
                ],
                c,
            );
        }
        self.indices.extend(src.indices.iter().map(|&i| base + i));
        // The source's splats come along, standing where the copy stands and scaled with it.
        for i in 0..src.splats.len() {
            let p = src.splats.center(i);
            let l = [
                (p[0] - anchor[0]) * scale,
                (p[1] - anchor[1]) * scale,
                (p[2] - anchor[2]) * scale,
            ];
            let n = src.splats.normal(i);
            self.splats.push(
                [
                    at[0] + co * l[0] + si * l[2],
                    at[1] + l[1],
                    at[2] - si * l[0] + co * l[2],
                ],
                [co * n[0] + si * n[2], n[1], -si * n[0] + co * n[2]],
                src.splats.color(i),
                src.splats.opacities[i],
                src.splats.radii[i] * scale,
                src.splats.drips[i],
            );
        }
        first
    }

    /// An upright frustum (cylinder, cone, taper) whose base centre is `c`. Returns the first new face.
    pub fn add_frustum(
        &mut self,
        c: Vec3,
        r_bottom: f32,
        r_top: f32,
        h: f32,
        segments: u32,
        rgb: [u8; 3],
    ) -> usize {
        let first = self.face_count();
        let n = segments.clamp(3, 64);
        let bottom = self.add_vert(c, rgb);
        let top = self.add_vert([c[0], c[1] + h, c[2]], rgb);
        for k in 0..n {
            let a = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
            self.add_vert(
                [c[0] + r_bottom * cos(a), c[1], c[2] + r_bottom * sin(a)],
                rgb,
            );
            self.add_vert(
                [c[0] + r_top * cos(a), c[1] + h, c[2] + r_top * sin(a)],
                rgb,
            );
        }
        let base = top + 1;
        for j in 0..n {
            let nx = (j + 1) % n;
            let (b0, b1) = (base + 2 * j, base + 2 * nx);
            let (t0, t1) = (b0 + 1, b1 + 1);
            self.indices
                .extend_from_slice(&[b0, t1, t0, b0, b1, t1, bottom, b1, b0, top, t0, t1]);
        }
        first
    }

    /// An ellipsoid centred at `c` with radii `r`. Returns the first new face.
    pub fn add_sphere(
        &mut self,
        c: Vec3,
        r: Vec3,
        segments: u32,
        rings: u32,
        rgb: [u8; 3],
    ) -> usize {
        let first = self.face_count();
        let sg = segments.clamp(3, 64);
        let rg = rings.clamp(2, 48);
        let top = self.add_vert([c[0], c[1] + r[1], c[2]], rgb);
        let bottom = self.add_vert([c[0], c[1] - r[1], c[2]], rgb);
        let ring0 = self.vertex_count() as u32;
        for i in 1..rg {
            let theta = std::f32::consts::PI * i as f32 / rg as f32;
            for k in 0..sg {
                let phi = 2.0 * std::f32::consts::PI * k as f32 / sg as f32;
                self.add_vert(
                    [
                        c[0] + r[0] * sin(theta) * cos(phi),
                        c[1] + r[1] * cos(theta),
                        c[2] + r[2] * sin(theta) * sin(phi),
                    ],
                    rgb,
                );
            }
        }
        let last = ring0 + (rg - 2) * sg;
        for k0 in 0..sg {
            let k1 = (k0 + 1) % sg;
            self.indices.extend_from_slice(&[
                top,
                ring0 + k1,
                ring0 + k0,
                bottom,
                last + k0,
                last + k1,
            ]);
        }
        for i in 0..rg - 2 {
            let row = ring0 + i * sg;
            for k0 in 0..sg {
                let k1 = (k0 + 1) % sg;
                self.indices.extend_from_slice(&[
                    row + k0,
                    row + k1,
                    row + sg + k1,
                    row + k0,
                    row + sg + k1,
                    row + sg + k0,
                ]);
            }
        }
        first
    }

    /// Revolves a profile of (radius, height) pairs about the vertical axis through `c`: columns, vases, domes.
    /// With `capped`, the first and last rings are closed with discs. Returns the first new face.
    pub fn add_lathe(
        &mut self,
        profile: &[[f32; 2]],
        segments: u32,
        c: Vec3,
        rgb: [u8; 3],
        capped: bool,
    ) -> usize {
        let first = self.face_count();
        let prof = &profile[..profile.len().min(256)];
        if prof.len() < 2 {
            return first;
        }
        let sg = segments.clamp(3, 96);
        let mut ring_first = Vec::with_capacity(prof.len());
        for p in prof {
            ring_first.push(self.vertex_count() as u32);
            for k in 0..sg {
                let a = 2.0 * std::f32::consts::PI * k as f32 / sg as f32;
                self.add_vert(
                    [c[0] + p[0] * cos(a), c[1] + p[1], c[2] + p[0] * sin(a)],
                    rgb,
                );
            }
        }
        for i in 0..prof.len() - 1 {
            for k0 in 0..sg {
                let k1 = (k0 + 1) % sg;
                let (a0, a1) = (ring_first[i] + k0, ring_first[i] + k1);
                let (b0, b1) = (ring_first[i + 1] + k0, ring_first[i + 1] + k1);
                if prof[i][0] > 1e-6 {
                    self.indices.extend_from_slice(&[a0, a1, b1]);
                }
                if prof[i + 1][0] > 1e-6 {
                    self.indices.extend_from_slice(&[a0, b1, b0]);
                }
            }
        }
        if capped {
            for ring in [0, prof.len() - 1] {
                if prof[ring][0] <= 1e-6 {
                    continue;
                }
                let centre = self.add_vert([c[0], c[1] + prof[ring][1], c[2]], rgb);
                for k0 in 0..sg {
                    let k1 = (k0 + 1) % sg;
                    self.indices.extend_from_slice(&[
                        centre,
                        ring_first[ring] + k1,
                        ring_first[ring] + k0,
                    ]);
                }
            }
        }
        first
    }

    /// Extrudes a floor plan of (x, z) points straight up from `y0` by `h`, with walls and ear-clipped caps.
    /// Returns the first new face.
    pub fn add_extrude(&mut self, polygon: &[[f32; 2]], y0: f32, h: f32, rgb: [u8; 3]) -> usize {
        let first = self.face_count();
        let pts = &polygon[..polygon.len().min(512)];
        let np = pts.len();
        if np < 3 {
            return first;
        }
        let ccw = poly_area2(pts) > 0.0;
        let base: Vec<u32> = pts
            .iter()
            .map(|p| self.add_vert([p[0], y0, p[1]], rgb))
            .collect();
        let topv: Vec<u32> = pts
            .iter()
            .map(|p| self.add_vert([p[0], y0 + h, p[1]], rgb))
            .collect();
        for i in 0..np {
            let j = (i + 1) % np;
            self.indices
                .extend_from_slice(&[base[i], base[j], topv[j], base[i], topv[j], topv[i]]);
        }
        let mut order: Vec<usize> = (0..np).map(|i| if ccw { i } else { np - 1 - i }).collect();
        let mut guard = 0;
        while order.len() > 3 && guard < np * np {
            guard += 1;
            let m = order.len();
            let mut clipped = false;
            for k in 0..m {
                let (ia, ib, ic) = (order[(k + m - 1) % m], order[k], order[(k + 1) % m]);
                let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
                if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) <= 0.0 {
                    continue; // a reflex corner
                }
                let empty = order
                    .iter()
                    .all(|&iq| iq == ia || iq == ib || iq == ic || !in_tri(pts[iq], a, b, c));
                if !empty {
                    continue;
                }
                self.indices.extend_from_slice(&[
                    topv[ia], topv[ib], topv[ic], base[ia], base[ic], base[ib],
                ]);
                order.remove(k);
                clipped = true;
                break;
            }
            if !clipped {
                break;
            }
        }
        if order.len() == 3 {
            let o = &order;
            self.indices.extend_from_slice(&[
                topv[o[0]], topv[o[1]], topv[o[2]], base[o[0]], base[o[2]], base[o[1]],
            ]);
        } else {
            // A self-intersecting outline: fall back to a fan so something is drawn.
            for i in 1..order.len() - 1 {
                let o = &order;
                self.indices.extend_from_slice(&[
                    topv[o[0]],
                    topv[o[i]],
                    topv[o[i + 1]],
                    base[o[0]],
                    base[o[i + 1]],
                    base[o[i]],
                ]);
            }
        }
        first
    }
}

/// Splits a boundary loop that passes through a vertex more than once (a hole that touches itself) into simple
/// loops, each keeping the original direction. Loops of fewer than three vertices (a dangling edge) are dropped.
fn split_pinches(ring: &[u32]) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    let mut stack: Vec<u32> = Vec::with_capacity(ring.len());
    for &v in ring {
        if let Some(at) = stack.iter().position(|&x| x == v) {
            let part: Vec<u32> = stack.drain(at..).collect();
            if part.len() >= 3 {
                out.push(part);
            }
        }
        stack.push(v);
    }
    if stack.len() >= 3 {
        out.push(stack);
    }
    out
}

fn poly_area2(pts: &[[f32; 2]]) -> f32 {
    let mut a = 0.0;
    for i in 0..pts.len() {
        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a
}

fn in_tri(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d1 = (p[0] - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (p[1] - b[1]);
    let d2 = (p[0] - c[0]) * (b[1] - c[1]) - (b[0] - c[0]) * (p[1] - c[1]);
    let d3 = (p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1]);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

// ---- cloning ----

impl Mesh {
    /// Copies the triangles whose centre is in the box: moved by `offset`, turned about the box's vertical axis by
    /// `rot_yaw`, optionally mirrored across the box's x and/or z axis. Returns the faces added.
    pub fn clone_box(
        &mut self,
        a: Area,
        offset: Vec3,
        rot_yaw: f32,
        mirror_x: bool,
        mirror_z: bool,
    ) -> usize {
        let f0 = self.face_count();
        const NONE: u32 = u32::MAX;
        let mut map = vec![NONE; self.vertex_count()];
        let (co, si) = (cos(rot_yaw), sin(rot_yaw));
        let flip = mirror_x != mirror_z;
        let mut added = 0;
        for t in 0..f0 {
            if self.face_count() > MAX_FACES {
                break;
            }
            let ids = self.face(t);
            let (p, q, r) = (self.vert(ids[0]), self.vert(ids[1]), self.vert(ids[2]));
            let m = [
                (p[0] + q[0] + r[0]) / 3.0,
                (p[1] + q[1] + r[1]) / 3.0,
                (p[2] + q[2] + r[2]) / 3.0,
            ];
            if a.weight(m, 0.0) <= 0.0 {
                continue;
            }
            let mut out = [0u32; 3];
            for k in 0..3 {
                let vi = ids[k];
                if map[vi as usize] == NONE {
                    let mut l = to_local(self.vert(vi), a.c, a.yaw);
                    if mirror_x {
                        l[0] = -l[0];
                    }
                    if mirror_z {
                        l[2] = -l[2];
                    }
                    let rl = [co * l[0] + si * l[2], l[1], -si * l[0] + co * l[2]];
                    let w = to_world(rl, a.c, a.yaw);
                    let c = self.color(vi);
                    map[vi as usize] = self.push_vertex(
                        [w[0] + offset[0], w[1] + offset[1], w[2] + offset[2]],
                        c,
                        Src::Copy(vi),
                    );
                }
                out[k] = map[vi as usize];
            }
            if flip {
                self.indices.extend_from_slice(&[out[0], out[2], out[1]]);
            } else {
                self.indices.extend_from_slice(&out);
            }
            added += 1;
        }
        // Splats in the box are cloned by the same transform (mirror, turn, offset), normals included.
        let splats: Vec<usize> = (0..self.splats.len())
            .filter(|&i| a.weight(self.splats.center(i), 0.0) > 0.0)
            .collect();
        for i in splats {
            let (p, n) = (self.splats.center(i), self.splats.normal(i));
            let mut lp = to_local(p, a.c, a.yaw);
            // A normal is a direction: it turns with the box frame but does not translate.
            let tail = to_local([p[0] + n[0], p[1] + n[1], p[2] + n[2]], a.c, a.yaw);
            let mut ln = [tail[0] - lp[0], tail[1] - lp[1], tail[2] - lp[2]];
            if mirror_x {
                lp[0] = -lp[0];
                ln[0] = -ln[0];
            }
            if mirror_z {
                lp[2] = -lp[2];
                ln[2] = -ln[2];
            }
            let rp = [co * lp[0] + si * lp[2], lp[1], -si * lp[0] + co * lp[2]];
            let rn = [co * ln[0] + si * ln[2], ln[1], -si * ln[0] + co * ln[2]];
            let w = to_world(rp, a.c, a.yaw);
            self.splats.push(
                [w[0] + offset[0], w[1] + offset[1], w[2] + offset[2]],
                norm(rn),
                self.splats.color(i),
                self.splats.opacities[i],
                self.splats.radii[i],
                self.splats.drips[i],
            );
        }
        added
    }
}

// ---- rigid placement ----

impl Mesh {
    /// Moves, turns and scales the vertices in `area` about `pivot`, and the splats there with them. Rotation is
    /// roll (about +z), then pitch (about +x), then yaw (about +y), all in radians. Returns the vertices moved.
    #[allow(clippy::too_many_arguments)]
    pub fn transform(
        &mut self,
        a: Area,
        translate: Vec3,
        roll: f32,
        pitch: f32,
        yaw: f32,
        scale: f32,
        pivot: Vec3,
    ) -> usize {
        let (cr, sr) = (cos(roll), sin(roll));
        let (cp, sp) = (cos(pitch), sin(pitch));
        let (cy, sy) = (cos(yaw), sin(yaw));
        // R = Ry * Rx * Rz, row-major.
        let r = [
            cy * cr + sy * sp * sr,
            -cy * sr + sy * sp * cr,
            sy * cp,
            cp * sr,
            cp * cr,
            -sp,
            -sy * cr + cy * sp * sr,
            sy * sr + cy * sp * cr,
            cy * cp,
        ];
        let apply = |p: Vec3| -> Vec3 {
            let l = [
                (p[0] - pivot[0]) * scale,
                (p[1] - pivot[1]) * scale,
                (p[2] - pivot[2]) * scale,
            ];
            [
                pivot[0] + r[0] * l[0] + r[1] * l[1] + r[2] * l[2] + translate[0],
                pivot[1] + r[3] * l[0] + r[4] * l[1] + r[5] * l[2] + translate[1],
                pivot[2] + r[6] * l[0] + r[7] * l[1] + r[8] * l[2] + translate[2],
            ]
        };
        let turn = |n: Vec3| -> Vec3 {
            norm([
                r[0] * n[0] + r[1] * n[1] + r[2] * n[2],
                r[3] * n[0] + r[4] * n[1] + r[5] * n[2],
                r[6] * n[0] + r[7] * n[1] + r[8] * n[2],
            ])
        };
        let mut moved = 0;
        for i in 0..self.vertex_count() {
            let p = self.vert(i as u32);
            if !a.contains(p) {
                continue;
            }
            let q = apply(p);
            self.positions[i * 3] = q[0];
            self.positions[i * 3 + 1] = q[1];
            self.positions[i * 3 + 2] = q[2];
            moved += 1;
        }
        for i in 0..self.splats.len() {
            let p = self.splats.center(i);
            if !a.contains(p) {
                continue;
            }
            let q = apply(p);
            self.splats.positions[i * 3] = q[0];
            self.splats.positions[i * 3 + 1] = q[1];
            self.splats.positions[i * 3 + 2] = q[2];
            let n = turn(self.splats.normal(i));
            self.splats.normals[i * 3] = n[0];
            self.splats.normals[i * 3 + 1] = n[1];
            self.splats.normals[i * 3 + 2] = n[2];
            self.splats.radii[i] *= scale;
        }
        moved
    }

    /// Volume (signed-sum magnitude, exact for a watertight mesh), surface area, area-weighted centroid and whether
    /// the surface is closed. Sums run in face order, so the result is the same on every platform.
    pub fn measure(&self) -> crate::op::Measure {
        let (mut six_v, mut twice_a) = (0f64, 0f64);
        let mut c = [0f64; 3];
        for t in 0..self.face_count() {
            let [i, j, k] = self.face(t);
            let (p, q, r) = (
                self.vert(i).map(f64::from),
                self.vert(j).map(f64::from),
                self.vert(k).map(f64::from),
            );
            let cross = [
                q[1] * r[2] - q[2] * r[1],
                q[2] * r[0] - q[0] * r[2],
                q[0] * r[1] - q[1] * r[0],
            ];
            six_v += p[0] * cross[0] + p[1] * cross[1] + p[2] * cross[2];
            let ab = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
            let ac = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
            let n = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let a2 = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            twice_a += a2;
            for k in 0..3 {
                c[k] += a2 * (p[k] + q[k] + r[k]) / 3.0;
            }
        }
        let closed = if self.indices.is_empty() {
            true
        } else {
            !crate::topo::boundary_vertices(self).iter().any(|&b| b)
        };
        crate::op::Measure {
            volume: if closed { six_v.abs() / 6.0 } else { 0.0 },
            surface_area: twice_a / 2.0,
            centroid: if twice_a > 0.0 {
                c.map(|v| v / twice_a)
            } else {
                [0.0; 3]
            },
            closed,
            splats: self.splats.len() as u64,
        }
    }
}
