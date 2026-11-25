//! Connected surface islands, tangled island clusters, face-subset extraction, and topology reference tracking.
//!
//! Hard-edged primitives (boxes, walls, extruded polygons) split vertices along creases so each face keeps flat
//! shading normals. Island detection therefore clusters vertices by 0.1 mm quantized world position rather than raw
//! vertex index, matching how a person perceives one solid piece.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::kernel::MAX_FACES;
use crate::math::Vec3;
use crate::mesh::{Attribute, Mesh};
use crate::op::OpError;

/// How an operation affects vertex and face index references on a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TopologyKind {
    /// Vertex and face indices `0..vertices` and `0..faces` still refer to the same topological elements.
    Preserved,
    /// Existing indices `< before` remain untouched; new geometry was appended at the end.
    Appended,
    /// Surviving vertices and faces kept their relative order and have exact `old -> new` index maps.
    Remapped,
    /// Remeshing (`simplify`, `decimate`, `carveBox`, `subdivide`) replaced local connectivity.
    Invalidated,
}

/// Exact reference survival and index remapping produced by a single operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyRemap {
    pub kind: TopologyKind,
    pub vertices_before: u32,
    pub faces_before: u32,
    pub vertex_map: Vec<Option<u32>>,
    pub face_map: Vec<Option<u32>>,
}

impl TopologyRemap {
    pub fn preserved(vertices_before: usize, faces_before: usize) -> Self {
        Self {
            kind: TopologyKind::Preserved,
            vertices_before: vertices_before as u32,
            faces_before: faces_before as u32,
            vertex_map: Vec::new(),
            face_map: Vec::new(),
        }
    }

    pub fn appended(vertices_before: usize, faces_before: usize) -> Self {
        Self {
            kind: TopologyKind::Appended,
            vertices_before: vertices_before as u32,
            faces_before: faces_before as u32,
            vertex_map: Vec::new(),
            face_map: Vec::new(),
        }
    }

    pub fn invalidated(vertices_before: usize, faces_before: usize) -> Self {
        Self {
            kind: TopologyKind::Invalidated,
            vertices_before: vertices_before as u32,
            faces_before: faces_before as u32,
            vertex_map: Vec::new(),
            face_map: Vec::new(),
        }
    }

    /// Maps an old vertex index through this operation, returning `None` if removed or invalidated.
    pub fn map_vertex(&self, index: u32) -> Option<u32> {
        if index >= self.vertices_before {
            return None;
        }
        match self.kind {
            TopologyKind::Preserved | TopologyKind::Appended => Some(index),
            TopologyKind::Remapped => self.vertex_map.get(index as usize).copied().flatten(),
            TopologyKind::Invalidated => None,
        }
    }

    /// Maps an old face index through this operation, returning `None` if removed or invalidated.
    pub fn map_face(&self, index: u32) -> Option<u32> {
        if index >= self.faces_before {
            return None;
        }
        match self.kind {
            TopologyKind::Preserved | TopologyKind::Appended => Some(index),
            TopologyKind::Remapped => self.face_map.get(index as usize).copied().flatten(),
            TopologyKind::Invalidated => None,
        }
    }
}

/// One position-connected triangle island on a mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Island {
    /// Sorted face indices (`0..face_count`) belonging to this island.
    pub faces: Vec<u32>,
    /// True when the island contains every face in the mesh.
    pub is_whole_mesh: bool,
    /// Tight axis-aligned bounding box `(min, max)` of the island, meters.
    pub bounds: (Vec3, Vec3),
    /// True if any face outside this island has a vertex inside this island's bounding box (within 0.1 mm).
    pub overlaps_other_faces: bool,
}

/// A starting island plus any other islands transitively overlapping its bounding box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TangledCluster {
    /// Sorted face indices (`0..face_count`) belonging to the cluster.
    pub faces: Vec<u32>,
    /// True when the cluster contains every face in the mesh.
    pub is_whole_mesh: bool,
    /// Tight axis-aligned bounding box `(min, max)` of the cluster, meters.
    pub bounds: (Vec3, Vec3),
    /// Number of position-connected islands merged into this cluster.
    pub islands: usize,
}

const QUANT_SCALE: f64 = 1e4;
const OVERLAP_EPS: f32 = 1e-4;

#[inline]
fn quantize_pos(p: Vec3) -> [i64; 3] {
    [
        libm::floor(f64::from(p[0]) * QUANT_SCALE + 0.5) as i64,
        libm::floor(f64::from(p[1]) * QUANT_SCALE + 0.5) as i64,
        libm::floor(f64::from(p[2]) * QUANT_SCALE + 0.5) as i64,
    ]
}

/// Maps each vertex to a deterministic 0.1 mm position cluster index (`0..cluster_count`).
fn cluster_vertices(mesh: &Mesh) -> (Vec<u32>, usize) {
    let v_count = mesh.vertex_count();
    let mut map: BTreeMap<[i64; 3], u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(v_count);
    for v in 0..v_count {
        let key = quantize_pos(mesh.vert(v as u32));
        let next = map.len() as u32;
        let id = *map.entry(key).or_insert(next);
        out.push(id);
    }
    let count = map.len();
    (out, count)
}

/// Labels every face with a deterministic island id (`0..island_count`).
fn label_islands(mesh: &Mesh) -> (Vec<u32>, usize) {
    let f_count = mesh.face_count();
    let (vert_cluster, cluster_count) = cluster_vertices(mesh);
    let mut parent: Vec<u32> = (0..cluster_count as u32).collect();

    fn find(parent: &mut [u32], mut a: u32) -> u32 {
        while parent[a as usize] != a {
            let p = parent[a as usize];
            parent[a as usize] = parent[p as usize];
            a = parent[a as usize];
        }
        a
    }

    for f in 0..f_count {
        let [i0, i1, i2] = mesh.face(f);
        let r0 = find(&mut parent, vert_cluster[i0 as usize]);
        let r1 = find(&mut parent, vert_cluster[i1 as usize]);
        let r2 = find(&mut parent, vert_cluster[i2 as usize]);
        parent[r1 as usize] = r0;
        let r2_now = find(&mut parent, r2);
        parent[r2_now as usize] = r0;
    }

    let mut root_to_island: BTreeMap<u32, u32> = BTreeMap::new();
    let mut island_of = Vec::with_capacity(f_count);
    for f in 0..f_count {
        let [i0, _, _] = mesh.face(f);
        let root = find(&mut parent, vert_cluster[i0 as usize]);
        let next = root_to_island.len() as u32;
        let id = *root_to_island.entry(root).or_insert(next);
        island_of.push(id);
    }
    let count = root_to_island.len();
    (island_of, count)
}

fn face_list_bounds(mesh: &Mesh, faces: &[u32]) -> (Vec3, Vec3) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for &f in faces {
        for vi in mesh.face(f as usize) {
            let p = mesh.vert(vi);
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    (lo, hi)
}

fn retain_splats_near_faces(mesh: &mut Mesh) {
    if mesh.splats.is_empty() {
        return;
    }
    if mesh.face_count() == 0 {
        mesh.splats = Default::default();
        return;
    }
    // Precompute per-face AABBs + overall mesh AABB so splats far from surviving geometry are dropped cleanly.
    let Some((mesh_lo, mesh_hi)) = mesh.bounds() else {
        mesh.splats = Default::default();
        return;
    };
    let f_count = mesh.face_count();
    let mut face_boxes: Vec<(Vec3, Vec3)> = Vec::with_capacity(f_count);
    for t in 0..f_count {
        let [i0, i1, i2] = mesh.face(t);
        let (p0, p1, p2) = (mesh.vert(i0), mesh.vert(i1), mesh.vert(i2));
        let lo = [
            p0[0].min(p1[0]).min(p2[0]),
            p0[1].min(p1[1]).min(p2[1]),
            p0[2].min(p1[2]).min(p2[2]),
        ];
        let hi = [
            p0[0].max(p1[0]).max(p2[0]),
            p0[1].max(p1[1]).max(p2[1]),
            p0[2].max(p1[2]).max(p2[2]),
        ];
        face_boxes.push((lo, hi));
    }
    let radii = mesh.splats.radii.clone();
    mesh.splats.retain(|idx, p| {
        let pad = (radii[idx] * 2.5).max(1e-3);
        for a in 0..3 {
            if p[a] < mesh_lo[a] - pad || p[a] > mesh_hi[a] + pad {
                return false;
            }
        }
        face_boxes
            .iter()
            .any(|(lo, hi)| (0..3).all(|a| p[a] >= lo[a] - pad && p[a] <= hi[a] + pad))
    });
}

impl Mesh {
    /// Finds the position-connected triangle island containing `start_face`.
    pub fn connected_component(&self, start_face: u32) -> Option<Island> {
        let f_count = self.face_count();
        if f_count == 0 || start_face as usize >= f_count {
            return None;
        }
        let (vert_cluster, cluster_count) = cluster_vertices(self);
        let mut cluster_faces: Vec<Vec<u32>> = vec![Vec::new(); cluster_count];
        for f in 0..f_count {
            let [i0, i1, i2] = self.face(f);
            let (c0, c1, c2) = (
                vert_cluster[i0 as usize] as usize,
                vert_cluster[i1 as usize] as usize,
                vert_cluster[i2 as usize] as usize,
            );
            cluster_faces[c0].push(f as u32);
            if c1 != c0 {
                cluster_faces[c1].push(f as u32);
            }
            if c2 != c0 && c2 != c1 {
                cluster_faces[c2].push(f as u32);
            }
        }

        let mut visited = vec![false; f_count];
        let mut queue = vec![start_face];
        visited[start_face as usize] = true;
        let mut head = 0usize;
        while head < queue.len() {
            let f = queue[head] as usize;
            head += 1;
            for vi in self.face(f) {
                let cid = vert_cluster[vi as usize] as usize;
                for &nf in &cluster_faces[cid] {
                    if !visited[nf as usize] {
                        visited[nf as usize] = true;
                        queue.push(nf);
                    }
                }
            }
        }

        queue.sort_unstable();
        let is_whole_mesh = queue.len() == f_count;
        let (lo, hi) = face_list_bounds(self, &queue);
        let mut overlaps_other_faces = false;
        if !is_whole_mesh {
            'outer: for f in 0..f_count {
                if visited[f] {
                    continue;
                }
                for vi in self.face(f) {
                    let p = self.vert(vi);
                    if (0..3).all(|a| p[a] >= lo[a] - OVERLAP_EPS && p[a] <= hi[a] + OVERLAP_EPS) {
                        overlaps_other_faces = true;
                        break 'outer;
                    }
                }
            }
        }

        Some(Island {
            faces: queue,
            is_whole_mesh,
            bounds: (lo, hi),
            overlaps_other_faces,
        })
    }

    /// Finds the island at `start_face` plus every island transitively overlapping its bounding box.
    pub fn tangled_cluster(&self, start_face: u32) -> Option<TangledCluster> {
        let f_count = self.face_count();
        if f_count == 0 || start_face as usize >= f_count {
            return None;
        }
        let (island_of, island_count) = label_islands(self);
        let mut faces_by_island: Vec<Vec<u32>> = vec![Vec::new(); island_count];
        let mut lo = vec![[f32::INFINITY; 3]; island_count];
        let mut hi = vec![[f32::NEG_INFINITY; 3]; island_count];

        for f in 0..f_count {
            let id = island_of[f] as usize;
            faces_by_island[id].push(f as u32);
            for vi in self.face(f) {
                let p = self.vert(vi);
                for a in 0..3 {
                    lo[id][a] = lo[id][a].min(p[a]);
                    hi[id][a] = hi[id][a].max(p[a]);
                }
            }
        }

        let start_id = island_of[start_face as usize] as usize;
        let mut in_group = vec![false; island_count];
        in_group[start_id] = true;
        let mut min = lo[start_id];
        let mut max = hi[start_id];
        let mut islands = 1usize;

        loop {
            let mut grew = false;
            for id in 0..island_count {
                if in_group[id] {
                    continue;
                }
                let apart = (0..3)
                    .any(|a| hi[id][a] < min[a] - OVERLAP_EPS || lo[id][a] > max[a] + OVERLAP_EPS);
                if apart {
                    continue;
                }
                let reaches = faces_by_island[id].iter().any(|&f| {
                    self.face(f as usize).into_iter().any(|vi| {
                        let p = self.vert(vi);
                        (0..3).all(|a| p[a] >= min[a] - OVERLAP_EPS && p[a] <= max[a] + OVERLAP_EPS)
                    })
                });
                if !reaches {
                    continue;
                }
                in_group[id] = true;
                islands += 1;
                grew = true;
                for a in 0..3 {
                    min[a] = min[a].min(lo[id][a]);
                    max[a] = max[a].max(hi[id][a]);
                }
            }
            if !grew {
                break;
            }
        }

        let mut out_faces = Vec::new();
        for id in 0..island_count {
            if in_group[id] {
                out_faces.extend_from_slice(&faces_by_island[id]);
            }
        }
        out_faces.sort_unstable();
        let is_whole_mesh = out_faces.len() == f_count;
        Some(TangledCluster {
            faces: out_faces,
            is_whole_mesh,
            bounds: (min, max),
            islands,
        })
    }

    /// Extracts the specified `faces` into a new standalone [`Mesh`], preserving vertex colours, named attributes,
    /// and nearby object-space splats.
    pub fn extract_faces(&self, faces: &[u32]) -> Result<Mesh, OpError> {
        if faces.len() > MAX_FACES {
            return Err(OpError::Invalid("too many faces in selection".into()));
        }
        let f_count = self.face_count();
        let mut keep_face = vec![false; f_count];
        for &f in faces {
            if f as usize >= f_count {
                return Err(OpError::Invalid(format!(
                    "face {f} is out of range (mesh has {f_count} faces)"
                )));
            }
            keep_face[f as usize] = true;
        }

        let v_count = self.vertex_count();
        const NONE: u32 = u32::MAX;
        let mut v_map = vec![NONE; v_count];
        for (f, &kept) in keep_face.iter().enumerate() {
            if kept {
                for vi in self.face(f) {
                    v_map[vi as usize] = 0;
                }
            }
        }

        let mut positions = Vec::new();
        let mut colors = Vec::new();
        let mut attrs: Vec<Attribute> = self
            .attributes
            .iter()
            .map(|a| Attribute {
                name: a.name.clone(),
                kind: a.kind,
                values: Vec::new(),
            })
            .collect();

        let mut next_v = 0u32;
        for (v, slot) in v_map.iter_mut().enumerate() {
            if *slot == NONE {
                continue;
            }
            *slot = next_v;
            next_v += 1;
            positions.extend_from_slice(&self.positions[v * 3..v * 3 + 3]);
            colors.extend_from_slice(&self.colors[v * 3..v * 3 + 3]);
            for (ai, a) in self.attributes.iter().enumerate() {
                attrs[ai].values.push(a.values[v]);
            }
        }

        let mut indices = Vec::new();
        for (f, &kept) in keep_face.iter().enumerate() {
            if kept {
                let [i0, i1, i2] = self.face(f);
                indices.extend_from_slice(&[
                    v_map[i0 as usize],
                    v_map[i1 as usize],
                    v_map[i2 as usize],
                ]);
            }
        }

        let mut sub = Mesh {
            positions,
            colors,
            indices,
            attributes: attrs,
            splats: self.splats.clone(),
        };
        retain_splats_near_faces(&mut sub);
        Ok(sub)
    }

    /// Removes the listed faces (or keeps only the listed faces when `invert` is true), returning `(removed_count, remap)`.
    pub fn remove_faces_with_remap(
        &mut self,
        faces: &[u32],
        invert: bool,
    ) -> (usize, TopologyRemap) {
        let v0 = self.vertex_count();
        let f0 = self.face_count();
        let mut listed = vec![false; f0];
        for &f in faces {
            if (f as usize) < f0 {
                listed[f as usize] = true;
            }
        }
        let face_map = self.retain_faces_with_map(|t| listed[t] == invert);
        let removed = f0 - self.face_count();
        let vertex_map = self.compact_with_map();
        if removed > 0 {
            retain_splats_near_faces(self);
        }
        let remap = if removed == 0 {
            TopologyRemap::preserved(v0, f0)
        } else {
            TopologyRemap {
                kind: TopologyKind::Remapped,
                vertices_before: v0 as u32,
                faces_before: f0 as u32,
                vertex_map,
                face_map,
            }
        };
        (removed, remap)
    }

    /// Removes the listed faces (or keeps only the listed faces when `invert` is true), returning faces removed.
    pub fn remove_faces(&mut self, faces: &[u32], invert: bool) -> usize {
        self.remove_faces_with_remap(faces, invert).0
    }
}
