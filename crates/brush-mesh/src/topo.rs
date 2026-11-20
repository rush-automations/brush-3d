//! Connectivity built on demand from the face list. Everything here is computed in a fixed order (sorts, not hash-map
//! iteration), so results are identical on every platform.

use crate::math::{cross, norm, sub, Vec3};
use crate::mesh::Mesh;

/// Compressed rows: `items[offsets[i]..offsets[i + 1]]` belong to vertex `i`.
#[derive(Clone, Debug, Default)]
pub struct Csr {
    pub offsets: Vec<u32>,
    pub items: Vec<u32>,
}

impl Csr {
    #[inline]
    pub fn row(&self, i: usize) -> &[u32] {
        &self.items[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    fn from_pairs(n: usize, pairs: &[(u32, u32)]) -> Csr {
        let mut offsets = vec![0u32; n + 1];
        for &(a, _) in pairs {
            offsets[a as usize + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let mut fill = offsets.clone();
        let mut items = vec![0u32; pairs.len()];
        for &(a, b) in pairs {
            items[fill[a as usize] as usize] = b;
            fill[a as usize] += 1;
        }
        Csr { offsets, items }
    }
}

/// The faces around each vertex, in face order.
pub fn vertex_faces(mesh: &Mesh) -> Csr {
    let mut pairs = Vec::with_capacity(mesh.indices.len());
    for t in 0..mesh.face_count() {
        for &v in &mesh.face(t) {
            pairs.push((v, t as u32));
        }
    }
    Csr::from_pairs(mesh.vertex_count(), &pairs)
}

/// The distinct vertices joined to each vertex by an edge, sorted.
pub fn vertex_neighbors(mesh: &Mesh) -> Csr {
    let mut pairs = Vec::with_capacity(mesh.indices.len() * 2);
    for t in 0..mesh.face_count() {
        let f = mesh.face(t);
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            if a != b {
                pairs.push((a, b));
                pairs.push((b, a));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    Csr::from_pairs(mesh.vertex_count(), &pairs)
}

/// Undirected edges as (low, high, faces using it), sorted.
pub fn edges(mesh: &Mesh) -> Vec<(u32, u32, u32)> {
    let mut e: Vec<(u32, u32)> = Vec::with_capacity(mesh.indices.len());
    for t in 0..mesh.face_count() {
        let f = mesh.face(t);
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            e.push((a.min(b), a.max(b)));
        }
    }
    e.sort_unstable();
    let mut out: Vec<(u32, u32, u32)> = Vec::with_capacity(e.len() / 2 + 1);
    for (a, b) in e {
        match out.last_mut() {
            Some(l) if l.0 == a && l.1 == b => l.2 += 1,
            _ => out.push((a, b, 1)),
        }
    }
    out
}

/// Vertices on an open edge (an edge used by exactly one face).
pub fn boundary_vertices(mesh: &Mesh) -> Vec<bool> {
    let mut b = vec![false; mesh.vertex_count()];
    for (p, q, n) in edges(mesh) {
        if n == 1 {
            b[p as usize] = true;
            b[q as usize] = true;
        }
    }
    b
}

/// The (unnormalised) face normal: its length is twice the face area.
#[inline]
pub fn face_cross(mesh: &Mesh, t: usize) -> Vec3 {
    let [a, b, c] = mesh.face(t);
    let (pa, pb, pc) = (mesh.vert(a), mesh.vert(b), mesh.vert(c));
    cross(sub(pb, pa), sub(pc, pa))
}

/// The area-weighted normal of one vertex, from its faces in `vf` order.
#[inline]
pub fn vertex_normal(mesh: &Mesh, vf: &Csr, i: usize) -> Vec3 {
    let mut s = [0f32; 3];
    for &t in vf.row(i) {
        let n = face_cross(mesh, t as usize);
        s = [s[0] + n[0], s[1] + n[1], s[2] + n[2]];
    }
    norm(s)
}

/// Area-weighted vertex normals. Vertices without faces get +y.
pub fn vertex_normals(mesh: &Mesh) -> Vec<Vec3> {
    let vf = vertex_faces(mesh);
    (0..mesh.vertex_count())
        .map(|i| vertex_normal(mesh, &vf, i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_has_one_interior_edge_and_four_boundary_vertices() {
        let m = Mesh::new(
            vec![0., 0., 0., 1., 0., 0., 1., 0., 1., 0., 0., 1.],
            vec![0; 12],
            vec![0, 1, 2, 0, 2, 3],
        )
        .unwrap();
        let e = edges(&m);
        assert_eq!(e.len(), 5);
        assert_eq!(e.iter().filter(|x| x.2 == 2).count(), 1);
        assert!(boundary_vertices(&m).iter().all(|&b| b));
        assert_eq!(vertex_neighbors(&m).row(0), &[1, 2, 3]);
        assert_eq!(vertex_faces(&m).row(2), &[0, 1]);
        let n = vertex_normals(&m);
        assert!((n[0][1].abs() - 1.0).abs() < 1e-6);
    }
}
