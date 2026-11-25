//! The mesh: indexed triangles with an RGB colour per vertex, plus any number of named per-vertex scalar attributes
//! (normals, confidence, semantic labels, an AI model's mask...) that every edit carries along.

use crate::math::Vec3;
use crate::splat::SplatSet;
use serde::{Deserialize, Serialize};

/// The storage type an attribute had in its file, kept so it is written back the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScalarKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl ScalarKind {
    /// Integer attributes (labels, ids) are never averaged: a new vertex takes the value of its first source.
    pub fn is_integer(self) -> bool {
        !matches!(self, ScalarKind::F32 | ScalarKind::F64)
    }
}

/// One value per vertex. Values are held as f64, which represents every supported kind exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub kind: ScalarKind,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// 3 per vertex: meters, +y up.
    pub positions: Vec<f32>,
    /// 3 per vertex.
    pub colors: Vec<u8>,
    /// 3 per face.
    pub indices: Vec<u32>,
    pub attributes: Vec<Attribute>,
    /// Paint that lives in object space rather than in vertices (see [`crate::splat`]). Geometry ops leave it
    /// alone except where noted (`removeBox`, `carveBox`, `cloneBox`, `transform`, and baking objects all carry it).
    pub splats: SplatSet,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MeshError {
    #[error("positions must have 3 values per vertex")]
    Positions,
    #[error("colors must have 3 bytes per vertex")]
    Colors,
    #[error("indices must have 3 values per face")]
    Indices,
    #[error("a face refers to vertex {0}, which does not exist")]
    MissingVertex(u32),
    #[error("attribute {0} must have one value per vertex")]
    Attribute(String),
    #[error("splats must have matching positions, normals, colors, opacities, radii and drips")]
    Splats,
}

/// Where a new vertex takes its attribute values from.
#[derive(Clone, Copy)]
pub(crate) enum Src<'a> {
    /// Zero (a primitive added from nothing).
    Zero,
    Copy(u32),
    Mid(u32, u32),
    Mean(&'a [u32]),
}

impl Mesh {
    pub fn new(positions: Vec<f32>, colors: Vec<u8>, indices: Vec<u32>) -> Result<Mesh, MeshError> {
        let m = Mesh {
            positions,
            colors,
            indices,
            attributes: Vec::new(),
            splats: SplatSet::default(),
        };
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), MeshError> {
        if self.positions.len() % 3 != 0 {
            return Err(MeshError::Positions);
        }
        let v = self.vertex_count();
        if self.colors.len() != v * 3 {
            return Err(MeshError::Colors);
        }
        if self.indices.len() % 3 != 0 {
            return Err(MeshError::Indices);
        }
        if let Some(&bad) = self.indices.iter().find(|&&i| i as usize >= v) {
            return Err(MeshError::MissingVertex(bad));
        }
        if let Some(a) = self.attributes.iter().find(|a| a.values.len() != v) {
            return Err(MeshError::Attribute(a.name.clone()));
        }
        let mut splat_error = Ok(());
        self.splats.validate(&mut splat_error);
        splat_error
    }

    #[inline]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }
    #[inline]
    pub fn face_count(&self) -> usize {
        self.indices.len() / 3
    }

    #[inline]
    pub fn vert(&self, i: u32) -> Vec3 {
        let o = i as usize * 3;
        [
            self.positions[o],
            self.positions[o + 1],
            self.positions[o + 2],
        ]
    }
    #[inline]
    pub fn color(&self, i: u32) -> [u8; 3] {
        let o = i as usize * 3;
        [self.colors[o], self.colors[o + 1], self.colors[o + 2]]
    }
    #[inline]
    pub fn face(&self, t: usize) -> [u32; 3] {
        [
            self.indices[t * 3],
            self.indices[t * 3 + 1],
            self.indices[t * 3 + 2],
        ]
    }

    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// Adds or replaces an attribute. `values` must have one entry per vertex.
    pub fn set_attribute(
        &mut self,
        name: &str,
        kind: ScalarKind,
        values: Vec<f64>,
    ) -> Result<(), MeshError> {
        if values.len() != self.vertex_count() {
            return Err(MeshError::Attribute(name.to_owned()));
        }
        match self.attributes.iter_mut().find(|a| a.name == name) {
            Some(a) => {
                a.kind = kind;
                a.values = values;
            }
            None => self.attributes.push(Attribute {
                name: name.to_owned(),
                kind,
                values,
            }),
        }
        Ok(())
    }

    /// Axis-aligned bounds, or None for a mesh without vertices.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        if self.vertex_count() == 0 {
            return None;
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in self.positions.chunks_exact(3) {
            for j in 0..3 {
                lo[j] = lo[j].min(p[j]);
                hi[j] = hi[j].max(p[j]);
            }
        }
        Some((lo, hi))
    }

    pub(crate) fn push_vertex(&mut self, p: Vec3, c: [u8; 3], from: Src) -> u32 {
        let i = self.vertex_count() as u32;
        self.positions.extend_from_slice(&p);
        self.colors.extend_from_slice(&c);
        for a in &mut self.attributes {
            let v = match from {
                Src::Zero => 0.0,
                Src::Copy(s) => a.values[s as usize],
                Src::Mid(x, y) => {
                    if a.kind.is_integer() {
                        a.values[x as usize]
                    } else {
                        (a.values[x as usize] + a.values[y as usize]) / 2.0
                    }
                }
                Src::Mean(src) => {
                    if a.kind.is_integer() || src.is_empty() {
                        src.first().map_or(0.0, |&s| a.values[s as usize])
                    } else {
                        src.iter().map(|&s| a.values[s as usize]).sum::<f64>() / src.len() as f64
                    }
                }
            };
            a.values.push(v);
        }
        i
    }

    /// Keeps only the vertices a face still uses, renumbering faces and returning `old -> Option<new>` vertex indices.
    pub(crate) fn compact_with_map(&mut self) -> Vec<Option<u32>> {
        let v = self.vertex_count();
        const NONE: u32 = u32::MAX;
        let mut map = vec![NONE; v];
        for &i in &self.indices {
            map[i as usize] = 0;
        }
        let mut next = 0usize;
        for i in 0..v {
            if map[i] == NONE {
                continue;
            }
            map[i] = next as u32;
            if next != i {
                self.positions.copy_within(i * 3..i * 3 + 3, next * 3);
                self.colors.copy_within(i * 3..i * 3 + 3, next * 3);
                for a in &mut self.attributes {
                    a.values[next] = a.values[i];
                }
            }
            next += 1;
        }
        for i in &mut self.indices {
            *i = map[*i as usize];
        }
        self.positions.truncate(next * 3);
        self.colors.truncate(next * 3);
        for a in &mut self.attributes {
            a.values.truncate(next);
        }
        map.into_iter().map(|m| (m != NONE).then_some(m)).collect()
    }

    /// Keeps only the vertices a face still uses, renumbering faces. Survivors keep their relative order.
    pub(crate) fn compact(&mut self) {
        let _ = self.compact_with_map();
    }

    /// Drops faces for which `keep` is false, keeping the order of the rest and returning `old -> Option<new>`.
    pub(crate) fn retain_faces_with_map(
        &mut self,
        mut keep: impl FnMut(usize) -> bool,
    ) -> Vec<Option<u32>> {
        let f = self.face_count();
        let mut map = vec![None; f];
        let mut w = 0;
        for t in 0..f {
            if !keep(t) {
                continue;
            }
            map[t] = Some(w as u32);
            if w != t {
                self.indices.copy_within(t * 3..t * 3 + 3, w * 3);
            }
            w += 1;
        }
        self.indices.truncate(w * 3);
        map
    }

    /// Drops faces for which `keep` is false, keeping the order of the rest.
    pub(crate) fn retain_faces(&mut self, keep: impl FnMut(usize) -> bool) {
        let _ = self.retain_faces_with_map(keep);
    }

    /// A content hash over positions, colours, faces, attributes and splats: equal meshes hash equal on every
    /// platform. FNV-1a 64 over the little-endian bytes, printed as 16 hex digits. A mesh without splats hashes
    /// exactly as it did before splats existed.
    pub fn content_hash(&self) -> String {
        let mut h = Fnv::new();
        h.write(&(self.vertex_count() as u64).to_le_bytes());
        h.write(&(self.face_count() as u64).to_le_bytes());
        for p in &self.positions {
            h.write(&p.to_bits().to_le_bytes());
        }
        h.write(&self.colors);
        for i in &self.indices {
            h.write(&i.to_le_bytes());
        }
        for a in &self.attributes {
            h.write(a.name.as_bytes());
            h.write(&[0]);
            for v in &a.values {
                h.write(&v.to_bits().to_le_bytes());
            }
        }
        if !self.splats.is_empty() {
            self.splats.hash_into(&mut h);
        }
        format!("{:016x}", h.0)
    }
}

pub(crate) struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
