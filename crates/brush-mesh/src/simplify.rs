//! Quadric error simplification (Garland and Heckbert 1997): repeatedly collapse the edge whose removal moves the
//! surface least, placing the merged vertex where it best fits the planes of the faces around both ends.
//!
//! Safeguards that keep a scan intact:
//! - the link condition is checked, so a collapse never makes the surface non-manifold;
//! - a collapse that would flip or flatten a nearby face is refused;
//! - open edges carry extra constraint planes, so outlines and hole rims keep their shape.
//!
//! Colours and attributes are interpolated along the collapsed edge. Everything runs in f64 with ties broken by
//! vertex id, so the result is the same on every platform.

use crate::mesh::Mesh;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A symmetric 4x4 quadric, upper triangle: a2 ab ac ad b2 bc bd c2 cd d2.
#[derive(Clone, Copy, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn plane(n: [f64; 3], d: f64, w: f64) -> Quadric {
        let [a, b, c] = n;
        Quadric([
            a * a * w,
            a * b * w,
            a * c * w,
            a * d * w,
            b * b * w,
            b * c * w,
            b * d * w,
            c * c * w,
            c * d * w,
            d * d * w,
        ])
    }
    fn add(&mut self, o: &Quadric) {
        for i in 0..10 {
            self.0[i] += o.0[i];
        }
    }
    fn error(&self, p: [f64; 3]) -> f64 {
        let q = &self.0;
        let [x, y, z] = p;
        q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9]
    }
    /// The point minimising the error, if the system is well conditioned.
    fn optimum(&self) -> Option<[f64; 3]> {
        let q = &self.0;
        let (a, b, c, d, e, f) = (q[0], q[1], q[2], q[4], q[5], q[7]);
        let det = a * (d * f - e * e) - b * (b * f - e * c) + c * (b * e - d * c);
        let scale = a.abs().max(d.abs()).max(f.abs());
        if scale == 0.0 || det.abs() < 1e-9 * scale * scale * scale {
            return None;
        }
        let r = [-q[3], -q[6], -q[8]];
        // Cramer's rule on the symmetric 3x3.
        let x =
            (r[0] * (d * f - e * e) - b * (r[1] * f - e * r[2]) + c * (r[1] * e - d * r[2])) / det;
        let y =
            (a * (r[1] * f - e * r[2]) - r[0] * (b * f - e * c) + c * (b * r[2] - r[1] * c)) / det;
        let z =
            (a * (d * r[2] - r[1] * e) - b * (b * r[2] - r[1] * c) + r[0] * (b * e - d * c)) / det;
        Some([x, y, z])
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[derive(PartialEq)]
struct Candidate {
    cost: f64,
    a: u32,
    b: u32,
    stamp: (u32, u32),
    target: [f64; 3],
}
impl Eq for Candidate {}
impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        // Cheapest first; ties by vertex ids so the order is total and the same everywhere.
        o.cost
            .total_cmp(&self.cost)
            .then(o.a.cmp(&self.a))
            .then(o.b.cmp(&self.b))
    }
}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Weight of an open edge's constraint plane, relative to the faces' own.
const BOUNDARY_WEIGHT: f64 = 1000.0;

struct State {
    pos: Vec<[f64; 3]>,
    q: Vec<Quadric>,
    faces: Vec<[u32; 3]>,
    face_alive: Vec<bool>,
    vf: Vec<Vec<u32>>,
    alive: Vec<bool>,
    boundary: Vec<bool>,
    stamp: Vec<u32>,
}

impl State {
    fn neighbors(&self, v: u32) -> Vec<u32> {
        let mut n: Vec<u32> = self.vf[v as usize]
            .iter()
            .flat_map(|&t| self.faces[t as usize])
            .filter(|&x| x != v)
            .collect();
        n.sort_unstable();
        n.dedup();
        n
    }

    fn candidate(&self, a: u32, b: u32) -> Candidate {
        let (a, b) = (a.min(b), a.max(b));
        let mut q = self.q[a as usize];
        q.add(&self.q[b as usize]);
        let (pa, pb) = (self.pos[a as usize], self.pos[b as usize]);
        let mid = [
            (pa[0] + pb[0]) / 2.0,
            (pa[1] + pb[1]) / 2.0,
            (pa[2] + pb[2]) / 2.0,
        ];
        let mut best = (q.error(mid), mid);
        for p in [pa, pb].into_iter().chain(q.optimum()) {
            let e = q.error(p);
            if e < best.0 {
                best = (e, p);
            }
        }
        // An optimum far from the edge is a numerical accident; such a collapse would be refused anyway.
        Candidate {
            cost: best.0.max(0.0),
            a,
            b,
            stamp: (self.stamp[a as usize], self.stamp[b as usize]),
            target: best.1,
        }
    }

    /// Faces around `v` other than those on edge (a, b) keep their orientation when `v` moves to `p`.
    fn keeps_orientation(&self, v: u32, other: u32, p: [f64; 3]) -> bool {
        for &t in &self.vf[v as usize] {
            let f = self.faces[t as usize];
            if f.contains(&other) {
                continue; // removed by the collapse
            }
            let pts = f.map(|i| self.pos[i as usize]);
            let moved = f.map(|i| if i == v { p } else { self.pos[i as usize] });
            let n0 = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
            let n1 = cross(sub(moved[1], moved[0]), sub(moved[2], moved[0]));
            let (l0, l1) = (len(n0), len(n1));
            if l1 <= 1e-12 * (1.0 + l0) || dot(n0, n1) < 0.2 * l0 * l1 {
                return false;
            }
        }
        true
    }
}

impl Mesh {
    /// Collapses edges until at most `target_faces` remain (or no collapse is safe). With `keep_boundary`, open edges
    /// are weighted heavily and never collapsed toward the interior. Returns the face count.
    pub fn simplify(&mut self, target_faces: usize, keep_boundary: bool) -> usize {
        let fcount = self.face_count();
        if fcount <= target_faces || fcount == 0 {
            return fcount;
        }
        let nv = self.vertex_count();
        let edges = crate::topo::edges(self);
        let mut s = State {
            pos: (0..nv)
                .map(|i| self.vert(i as u32).map(f64::from))
                .collect(),
            q: vec![Quadric::default(); nv],
            faces: (0..fcount).map(|t| self.face(t)).collect(),
            face_alive: vec![true; fcount],
            vf: vec![Vec::new(); nv],
            alive: vec![true; nv],
            boundary: vec![false; nv],
            stamp: vec![0; nv],
        };
        for (t, f) in s.faces.iter().enumerate() {
            for &v in f {
                s.vf[v as usize].push(t as u32);
            }
        }
        // Degenerate input faces (a repeated corner) are dropped up front.
        let mut alive_faces = fcount;
        for t in 0..fcount {
            let f = s.faces[t];
            if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
                s.face_alive[t] = false;
                alive_faces -= 1;
                for &v in &f {
                    s.vf[v as usize].retain(|&x| x != t as u32);
                }
            }
        }
        let face_normal = |s: &State, t: usize| {
            let p = s.faces[t].map(|i| s.pos[i as usize]);
            cross(sub(p[1], p[0]), sub(p[2], p[0]))
        };
        for t in 0..fcount {
            if !s.face_alive[t] {
                continue;
            }
            let n = face_normal(&s, t);
            let l = len(n);
            if l == 0.0 {
                continue;
            }
            let un = [n[0] / l, n[1] / l, n[2] / l];
            let d = -dot(un, s.pos[s.faces[t][0] as usize]);
            let pq = Quadric::plane(un, d, l / 2.0);
            for &v in &s.faces[t] {
                s.q[v as usize].add(&pq);
            }
        }
        // Open edges: a plane through the edge, perpendicular to its face.
        let open: std::collections::BTreeSet<(u32, u32)> = edges
            .iter()
            .filter(|e| e.2 == 1)
            .map(|e| (e.0, e.1))
            .collect();
        for t in 0..fcount {
            if !s.face_alive[t] {
                continue;
            }
            let f = s.faces[t];
            let n = face_normal(&s, t);
            for k in 0..3 {
                let (a, b) = (f[k], f[(k + 1) % 3]);
                if !open.contains(&(a.min(b), a.max(b))) {
                    continue;
                }
                s.boundary[a as usize] = true;
                s.boundary[b as usize] = true;
                let e = sub(s.pos[b as usize], s.pos[a as usize]);
                let c = cross(e, n);
                let l = len(c);
                if l == 0.0 {
                    continue;
                }
                let un = [c[0] / l, c[1] / l, c[2] / l];
                let d = -dot(un, s.pos[a as usize]);
                let pq = Quadric::plane(un, d, BOUNDARY_WEIGHT * dot(e, e));
                s.q[a as usize].add(&pq);
                s.q[b as usize].add(&pq);
            }
        }
        let mut heap: BinaryHeap<Candidate> = edges.iter().map(|e| s.candidate(e.0, e.1)).collect();
        let mut parent: Vec<(u32, u32, f32)> = Vec::new(); // (kept, removed, t along kept->removed), for attributes
        while alive_faces > target_faces {
            let Some(c) = heap.pop() else { break };
            let (a, b) = (c.a, c.b);
            if !s.alive[a as usize]
                || !s.alive[b as usize]
                || c.stamp != (s.stamp[a as usize], s.stamp[b as usize])
            {
                continue;
            }
            let shared: Vec<u32> = s.vf[a as usize]
                .iter()
                .copied()
                .filter(|&t| s.faces[t as usize].contains(&b))
                .collect();
            if shared.is_empty() {
                continue; // no longer an edge
            }
            let edge_open = shared.len() == 1;
            if keep_boundary && (s.boundary[a as usize] || s.boundary[b as usize]) && !edge_open {
                continue; // would pull a rim inward
            }
            let (na, nb) = (s.neighbors(a), s.neighbors(b));
            let common = na.iter().filter(|x| nb.binary_search(x).is_ok()).count();
            if common != shared.len() {
                continue; // link condition: the collapse would pinch the surface
            }
            let p = c.target;
            if !s.keeps_orientation(a, b, p) || !s.keeps_orientation(b, a, p) {
                continue;
            }
            // Collapse b into a.
            let (pa, pb) = (s.pos[a as usize], s.pos[b as usize]);
            let e = sub(pb, pa);
            let t = if dot(e, e) > 0.0 {
                (dot(sub(p, pa), e) / dot(e, e)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            for &f in &shared {
                s.face_alive[f as usize] = false;
                alive_faces -= 1;
                for &v in &s.faces[f as usize] {
                    if v != a && v != b {
                        s.vf[v as usize].retain(|&x| x != f);
                    }
                }
            }
            let moved: Vec<u32> = std::mem::take(&mut s.vf[b as usize])
                .into_iter()
                .filter(|&f| s.face_alive[f as usize])
                .collect();
            s.vf[a as usize].retain(|&f| s.face_alive[f as usize]);
            for &f in &moved {
                for x in s.faces[f as usize].iter_mut() {
                    if *x == b {
                        *x = a;
                    }
                }
                s.vf[a as usize].push(f);
            }
            s.vf[a as usize].sort_unstable();
            s.pos[a as usize] = p;
            let qb = s.q[b as usize];
            s.q[a as usize].add(&qb);
            s.boundary[a as usize] |= s.boundary[b as usize];
            s.alive[b as usize] = false;
            s.stamp[a as usize] += 1;
            parent.push((a, b, t as f32));
            for n in s.neighbors(a) {
                heap.push(s.candidate(a, n));
            }
        }
        // Write back: interpolate colours and attributes along each collapse, in collapse order.
        for &(a, b, t) in &parent {
            let (ai, bi) = (a as usize, b as usize);
            for j in 0..3 {
                let (ca, cb) = (
                    self.colors[ai * 3 + j] as f32,
                    self.colors[bi * 3 + j] as f32,
                );
                self.colors[ai * 3 + j] = crate::math::q8(ca + (cb - ca) * t);
            }
            for at in &mut self.attributes {
                at.values[ai] = if at.kind.is_integer() {
                    if t < 0.5 {
                        at.values[ai]
                    } else {
                        at.values[bi]
                    }
                } else {
                    at.values[ai] + (at.values[bi] - at.values[ai]) * t as f64
                };
            }
        }
        for (i, p) in s.pos.iter().enumerate() {
            if s.alive[i] {
                self.positions[i * 3..i * 3 + 3].copy_from_slice(&p.map(|v| v as f32));
            }
        }
        self.indices = (0..fcount)
            .filter(|&t| s.face_alive[t])
            .flat_map(|t| s.faces[t])
            .collect();
        self.compact();
        self.face_count()
    }
}
