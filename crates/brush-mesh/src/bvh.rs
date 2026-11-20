//! A bounding volume hierarchy over the faces, for picking (raycasts) and spatial queries. Build is a median split
//! on the longest axis (deterministic, O(n log n)). After a sculpt, which moves vertices but keeps faces, `refit`
//! updates the boxes in O(n) instead of rebuilding.

use crate::math::{cross, dot, sub, Vec3};
use crate::mesh::Mesh;
use serde::{Deserialize, Serialize};

const LEAF: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    /// Leaf: first index into `faces`. Inner: index of the left child (the right one follows its subtree).
    start: u32,
    /// Leaf: face count. Inner: 0, and `right` is the right child.
    count: u32,
    right: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    faces: Vec<u32>,
}

/// Where a ray met the surface.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub face: u32,
    /// Distance along the ray, in units of the ray direction's length.
    pub t: f32,
    pub point: Vec3,
    /// The face normal, turned to face the ray.
    pub normal: Vec3,
    /// Barycentric coordinates of the hit in the face.
    pub barycentric: Vec3,
}

fn face_bounds(m: &Mesh, t: u32) -> (Vec3, Vec3) {
    let f = m.face(t as usize);
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in f {
        let p = m.vert(v);
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

impl Bvh {
    pub fn build(mesh: &Mesh) -> Bvh {
        let n = mesh.face_count();
        let mut bvh = Bvh {
            nodes: Vec::with_capacity(2 * n / LEAF + 1),
            faces: (0..n as u32).collect(),
        };
        if n == 0 {
            return bvh;
        }
        let centroids: Vec<Vec3> = (0..n)
            .map(|t| {
                let (lo, hi) = face_bounds(mesh, t as u32);
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            })
            .collect();
        bvh.split(mesh, &centroids, 0, n);
        bvh
    }

    fn split(&mut self, mesh: &Mesh, cen: &[Vec3], start: usize, end: usize) -> u32 {
        let id = self.nodes.len() as u32;
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        let (mut clo, mut chi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for &t in &self.faces[start..end] {
            let (a, b) = face_bounds(mesh, t);
            let c = cen[t as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
                clo[k] = clo[k].min(c[k]);
                chi[k] = chi[k].max(c[k]);
            }
        }
        self.nodes.push(Node {
            lo,
            hi,
            start: start as u32,
            count: (end - start) as u32,
            right: 0,
        });
        let ext = [chi[0] - clo[0], chi[1] - clo[1], chi[2] - clo[2]];
        let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] {
            0
        } else if ext[1] >= ext[2] {
            1
        } else {
            2
        };
        if end - start <= LEAF || ext[axis] <= 0.0 {
            return id;
        }
        let mid = (start + end) / 2;
        self.faces[start..end].select_nth_unstable_by(mid - start, |&a, &b| {
            cen[a as usize][axis]
                .total_cmp(&cen[b as usize][axis])
                .then(a.cmp(&b))
        });
        let left = self.split(mesh, cen, start, mid);
        let right = self.split(mesh, cen, mid, end);
        let node = &mut self.nodes[id as usize];
        node.start = left;
        node.count = 0;
        node.right = right;
        id
    }

    /// Updates every box after vertices moved. The faces must be the same as at build.
    pub fn refit(&mut self, mesh: &Mesh) {
        // Children always come after their parent, so a reverse pass sees children first.
        for i in (0..self.nodes.len()).rev() {
            let n = self.nodes[i];
            let (lo, hi) = if n.count > 0 {
                let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for &t in &self.faces[n.start as usize..(n.start + n.count) as usize] {
                    let (a, b) = face_bounds(mesh, t);
                    for k in 0..3 {
                        lo[k] = lo[k].min(a[k]);
                        hi[k] = hi[k].max(b[k]);
                    }
                }
                (lo, hi)
            } else {
                let (l, r) = (self.nodes[n.start as usize], self.nodes[n.right as usize]);
                (
                    [0, 1, 2].map(|k| l.lo[k].min(r.lo[k])),
                    [0, 1, 2].map(|k| l.hi[k].max(r.hi[k])),
                )
            };
            self.nodes[i].lo = lo;
            self.nodes[i].hi = hi;
        }
    }

    /// The nearest face hit by the ray `origin + t * dir`, t > 0. Faces are two-sided.
    pub fn raycast(&self, mesh: &Mesh, origin: Vec3, dir: Vec3) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = dir.map(|d| 1.0 / d);
        let mut best: Option<Hit> = None;
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = self.nodes[i as usize];
            let limit = best.map_or(f32::INFINITY, |h| h.t);
            if slab(n.lo, n.hi, origin, inv).is_none_or(|t| t > limit) {
                continue;
            }
            if n.count > 0 {
                for &t in &self.faces[n.start as usize..(n.start + n.count) as usize] {
                    if let Some(h) = ray_triangle(mesh, t, origin, dir) {
                        if h.t < best.map_or(f32::INFINITY, |b| b.t)
                            || (h.t == best.unwrap().t && h.face < best.unwrap().face)
                        {
                            best = Some(h);
                        }
                    }
                }
            } else {
                stack.push(n.right);
                stack.push(n.start);
            }
        }
        best
    }
}

/// Entry distance into a box, if the ray meets it in front of the origin.
fn slab(lo: Vec3, hi: Vec3, o: Vec3, inv: Vec3) -> Option<f32> {
    let (mut t0, mut t1) = (0f32, f32::INFINITY);
    for k in 0..3 {
        if inv[k].is_infinite() {
            // The ray runs parallel to this pair of planes: it is inside the slab everywhere or nowhere.
            if o[k] < lo[k] || o[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[k] - o[k]) * inv[k], (hi[k] - o[k]) * inv[k]);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// Watertight ray–triangle intersection (Woop, Benthin and Wald 2013), two-sided, in f64. Neighbouring faces compute
/// their shared edge's function with the same operands, so a ray through an edge or vertex always hits one of them.
fn ray_triangle(m: &Mesh, t: u32, o: Vec3, d: Vec3) -> Option<Hit> {
    let [ia, ib, ic] = m.face(t as usize);
    let d64 = d.map(f64::from);
    let kz = if d64[0].abs() >= d64[1].abs() && d64[0].abs() >= d64[2].abs() {
        0
    } else if d64[1].abs() >= d64[2].abs() {
        1
    } else {
        2
    };
    if d64[kz] == 0.0 {
        return None;
    }
    let (mut kx, mut ky) = ((kz + 1) % 3, (kz + 2) % 3);
    if d64[kz] < 0.0 {
        std::mem::swap(&mut kx, &mut ky);
    }
    let (sx, sy, sz) = (d64[kx] / d64[kz], d64[ky] / d64[kz], 1.0 / d64[kz]);
    let rel = |i: u32| {
        let p = m.vert(i);
        [
            p[0] as f64 - o[0] as f64,
            p[1] as f64 - o[1] as f64,
            p[2] as f64 - o[2] as f64,
        ]
    };
    let (a, b, c) = (rel(ia), rel(ib), rel(ic));
    let (ax, ay) = (a[kx] - sx * a[kz], a[ky] - sy * a[kz]);
    let (bx, by) = (b[kx] - sx * b[kz], b[ky] - sy * b[kz]);
    let (cx, cy) = (c[kx] - sx * c[kz], c[ky] - sy * c[kz]);
    let u = cx * by - cy * bx;
    let v = ax * cy - ay * cx;
    let w = bx * ay - by * ax;
    if (u < 0.0 || v < 0.0 || w < 0.0) && (u > 0.0 || v > 0.0 || w > 0.0) {
        return None;
    }
    let det = u + v + w;
    if det == 0.0 {
        return None;
    }
    let dist = (u * sz * a[kz] + v * sz * b[kz] + w * sz * c[kz]) / det;
    if dist <= 1e-7 {
        return None;
    }
    let (p0, p1, p2) = (m.vert(ia), m.vert(ib), m.vert(ic));
    let n = crate::math::norm(cross(sub(p1, p0), sub(p2, p0)));
    let n = if dot(n, d) > 0.0 { n.map(|x| -x) } else { n };
    let dist = dist as f32;
    Some(Hit {
        face: t,
        t: dist,
        point: [o[0] + d[0] * dist, o[1] + d[1] * dist, o[2] + d[2] * dist],
        normal: n,
        barycentric: [(u / det) as f32, (v / det) as f32, (w / det) as f32],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Context;
    use crate::Op;

    #[test]
    fn hits_the_nearest_face_and_refits_after_moves() {
        let mut m = Mesh::default();
        let o: Op = serde_json::from_str(
            r#"{"op":"addSphere","center":[0,0,0],"radii":[1,1,1],"segments":32,"rings":16}"#,
        )
        .unwrap();
        o.apply(&mut m, Context::default()).unwrap();
        let mut bvh = Bvh::build(&m);
        let h = bvh.raycast(&m, [0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((h.point[2] - 1.0).abs() < 0.02, "{h:?}");
        assert!(h.normal[2] > 0.9);
        assert!(bvh.raycast(&m, [0.0, 3.0, 5.0], [0.0, 0.0, -1.0]).is_none());
        // Move everything +2 in x: a refit tree finds it there.
        for p in m.positions.chunks_exact_mut(3) {
            p[0] += 2.0;
        }
        bvh.refit(&m);
        assert!(bvh.raycast(&m, [0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).is_none());
        let h = bvh.raycast(&m, [2.0, 0.0, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((h.point[2] - 1.0).abs() < 0.02);
        // Brute force agrees on many random-ish rays.
        for k in 0..200 {
            let a = k as f32 * 0.37;
            let origin = [2.0 + 3.0 * a.cos(), 0.7 * (a * 1.3).sin(), 3.0 * a.sin()];
            let dir = [2.0 - origin[0], -origin[1] * 0.5, -origin[2]];
            let fast = bvh.raycast(&m, origin, dir).map(|h| h.face);
            let slow = (0..m.face_count() as u32)
                .filter_map(|t| ray_triangle(&m, t, origin, dir))
                .min_by(|x, y| x.t.total_cmp(&y.t).then(x.face.cmp(&y.face)))
                .map(|h| h.face);
            assert_eq!(fast, slow);
        }
    }
}
