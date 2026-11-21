//! Splats: paint that lives in object space, not in vertices. A splat is a small oriented Gaussian — a centre on
//! the surface, the surface normal where it landed, a colour, an opacity, a radius and how far it has dripped.
//! Because splats are anchored to space rather than to mesh resolution, a coarse scan and a subdivided master take
//! the same stroke the same way, and `simplify` never thins the paint out.
//!
//! Strokes are stamped deterministically: a jittered grid in the brush's tangent plane, each stamp raycast onto the
//! surface. Stamps that land on an earlier splat pour their opacity into it (the `flow` of the brush), so painting
//! over the same spot builds coverage exactly as a physical spray can does — and replaying a journal reproduces the
//! result bit for bit on every platform.

use crate::math::*;
use crate::mesh::Mesh;
use crate::region::Area;
use crate::Bvh;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The maximum splats one `splatPaint` op may add: strokes are sampled, not enumerated, so this only guards
/// against absurd parameters (a huge radius with a tiny spacing).
pub const MAX_STAMPS: usize = 1_000_000;

/// Paint as a set of 3D Gaussian splats, struct-of-arrays so it hashes and clones cheaply.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SplatSet {
    /// 3 per splat: the surface point it covers, meters.
    pub positions: Vec<f32>,
    /// 3 per splat: the surface normal where it landed (unit length).
    pub normals: Vec<f32>,
    /// 3 per splat.
    pub colors: Vec<u8>,
    /// 1 per splat: 0 is invisible, 255 fully covered.
    pub opacities: Vec<u8>,
    /// 1 per splat: Gaussian sigma in meters (the visible radius is about 2.5× this).
    pub radii: Vec<f32>,
    /// 1 per splat: 0 is a round dab; toward 1 it runs downward along the surface, like wet paint.
    pub drips: Vec<f32>,
}

impl SplatSet {
    #[inline]
    pub fn len(&self) -> usize {
        self.radii.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.radii.is_empty()
    }

    pub(crate) fn validate(&self, what: &mut Result<(), crate::mesh::MeshError>) {
        let n = self.len();
        let ok = self.positions.len() == n * 3
            && self.normals.len() == n * 3
            && self.colors.len() == n * 3
            && self.opacities.len() == n
            && self.drips.len() == n;
        if !ok {
            *what = Err(crate::mesh::MeshError::Splats);
        }
    }

    #[inline]
    pub fn center(&self, i: usize) -> Vec3 {
        [
            self.positions[i * 3],
            self.positions[i * 3 + 1],
            self.positions[i * 3 + 2],
        ]
    }
    #[inline]
    pub fn normal(&self, i: usize) -> Vec3 {
        [
            self.normals[i * 3],
            self.normals[i * 3 + 1],
            self.normals[i * 3 + 2],
        ]
    }
    #[inline]
    pub fn color(&self, i: usize) -> [u8; 3] {
        [
            self.colors[i * 3],
            self.colors[i * 3 + 1],
            self.colors[i * 3 + 2],
        ]
    }

    /// Appends one splat.
    pub fn push(
        &mut self,
        center: Vec3,
        normal: Vec3,
        color: [u8; 3],
        opacity: u8,
        radius: f32,
        drip: f32,
    ) {
        self.positions.extend_from_slice(&center);
        self.normals.extend_from_slice(&normal);
        self.colors.extend_from_slice(&color);
        self.opacities.push(opacity);
        self.radii.push(radius);
        self.drips.push(drip);
    }

    /// Keeps only the splats `keep` accepts, preserving order. The closure gets each splat's index and centre.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(usize, Vec3) -> bool) {
        let n = self.len();
        let mut w = 0;
        for r in 0..n {
            if !keep(r, self.center(r)) {
                continue;
            }
            if w != r {
                self.positions.copy_within(r * 3..r * 3 + 3, w * 3);
                self.normals.copy_within(r * 3..r * 3 + 3, w * 3);
                self.colors.copy_within(r * 3..r * 3 + 3, w * 3);
                self.opacities[w] = self.opacities[r];
                self.radii[w] = self.radii[r];
                self.drips[w] = self.drips[r];
            }
            w += 1;
        }
        self.positions.truncate(w * 3);
        self.normals.truncate(w * 3);
        self.colors.truncate(w * 3);
        self.opacities.truncate(w);
        self.radii.truncate(w);
        self.drips.truncate(w);
    }

    /// Content-hash continuation: called only when the set is not empty, so meshes without splats hash exactly as
    /// they always have.
    pub(crate) fn hash_into(&self, h: &mut crate::mesh::Fnv) {
        h.write(&(self.len() as u64).to_le_bytes());
        for v in self
            .positions
            .iter()
            .chain(&self.normals)
            .chain(&self.radii)
            .chain(&self.drips)
        {
            h.write(&v.to_bits().to_le_bytes());
        }
        h.write(&self.colors);
        h.write(&self.opacities);
    }
}

/// How one stamp of a `splatPaint` stroke is chosen and where it lands.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stamp {
    pub center: Vec3,
    pub normal: Vec3,
    pub radius: f32,
    pub opacity: f32,
    pub drip: f32,
}

/// A deterministic unit value in 0..1 from an integer lattice coordinate and a seed (same family as the noise in
/// `detail`, so all of Brush shares one hash).
fn jitter(ix: i32, iy: i32, salt: u32, seed: u32) -> f32 {
    let mut h = (ix as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((iy as u32).wrapping_mul(668_265_263))
        .wrapping_add(salt.wrapping_mul(2_147_483_629))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 16_777_216.0
}

/// Any unit vector orthogonal to `n`, deterministically.
fn tangent(n: Vec3) -> Vec3 {
    let axis = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    norm(cross(n, axis))
}

/// The stamps one dab deposits. The grid is square in the dab's tangent plane with spacing `spacing`; each cell's
/// stamp is jittered inside its cell, so dense strokes look organic rather than gridded, and the same dab always
/// produces the same stamps.
#[allow(clippy::too_many_arguments)]
fn dab_stamps(
    mesh: &Mesh,
    bvh: Option<&Bvh>,
    center: Vec3,
    pressure: f32,
    brush_radius: f32,
    spacing: f32,
    size: f32,
    opacity: f32,
    flow: f32,
    variation: f32,
    drip: f32,
    seed: u32,
    view: Option<Vec3>,
    align: bool,
) -> Vec<Stamp> {
    // The dab's own frame: spray direction and the normal at its centre.
    let spray = view.unwrap_or([0.0, -1.0, 0.0]);
    let (anchor, anchor_n) = match bvh {
        Some(b) => {
            let back = [
                center[0] - spray[0] * brush_radius,
                center[1] - spray[1] * brush_radius,
                center[2] - spray[2] * brush_radius,
            ];
            match b.raycast(mesh, back, spray) {
                Some(hit) => (hit.point, hit.normal),
                None => (center, norm([-spray[0], -spray[1], -spray[2]])),
            }
        }
        None => (center, norm([-spray[0], -spray[1], -spray[2]])),
    };
    let t1 = tangent(anchor_n);
    let t2 = norm(cross(anchor_n, t1));
    let cells = ceil(brush_radius / spacing) as i32;
    let mut out = Vec::new();
    for gy in -cells..=cells {
        for gx in -cells..=cells {
            // One stamp per grid cell, jittered within the cell.
            let jx = jitter(gx, gy, 1, seed) - 0.5;
            let jy = jitter(gx, gy, 2, seed) - 0.5;
            let ox = (gx as f32 + jx) * spacing;
            let oy = (gy as f32 + jy) * spacing;
            let d = sqrt(ox * ox + oy * oy);
            if d > brush_radius {
                continue;
            }
            let fall = smoothstep(1.0 - d / brush_radius);
            let stamp_origin = [
                anchor[0] + t1[0] * ox + t2[0] * oy,
                anchor[1] + t1[1] * ox + t2[1] * oy,
                anchor[2] + t1[2] * ox + t2[2] * oy,
            ];
            // Snap the stamp to the surface along the spray direction (paint wraps over bumps), unless the stroke
            // is meant to stay flat.
            let (p, n) = if align {
                match bvh {
                    Some(b) => {
                        let back = [
                            stamp_origin[0] - spray[0] * brush_radius,
                            stamp_origin[1] - spray[1] * brush_radius,
                            stamp_origin[2] - spray[2] * brush_radius,
                        ];
                        match b.raycast(mesh, back, spray) {
                            Some(hit) => (hit.point, hit.normal),
                            None => continue, // the brush is over empty space here
                        }
                    }
                    None => (stamp_origin, anchor_n),
                }
            } else {
                (stamp_origin, anchor_n)
            };
            let vsz = 1.0 - variation + 2.0 * variation * jitter(gx, gy, 3, seed);
            let vop = 1.0 - variation + 2.0 * variation * jitter(gx, gy, 4, seed);
            out.push(Stamp {
                center: p,
                normal: n,
                radius: (size * vsz).max(1e-6),
                opacity: opacity * flow * fall * pressure.clamp(0.0, 1.0) * vop,
                drip: if drip > 0.0 {
                    drip * (0.5 + jitter(gx, gy, 5, seed))
                } else {
                    0.0
                },
            });
            if out.len() >= MAX_STAMPS {
                return out;
            }
        }
    }
    out
}

/// A spatial hash over splat centres, for finding the splat a new stamp pours into. Cell size is the merge
/// distance; lookups scan the 27 neighbouring cells in a fixed order.
struct MergeGrid {
    cell: f32,
    map: rustc_hash::FxHashMap<(i32, i32, i32), Vec<u32>>,
}

impl MergeGrid {
    fn of(set: &SplatSet, cell_size: f32) -> MergeGrid {
        let mut map = rustc_hash::FxHashMap::default();
        for i in 0..set.len() {
            let c = set.center(i);
            map.entry((
                cell(c[0], cell_size),
                cell(c[1], cell_size),
                cell(c[2], cell_size),
            ))
            .or_insert_with(Vec::new)
            .push(i as u32);
        }
        MergeGrid {
            cell: cell_size,
            map,
        }
    }
    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        (
            cell(p[0], self.cell),
            cell(p[1], self.cell),
            cell(p[2], self.cell),
        )
    }

    /// The nearest splat within `dist` of `p`, or none. Ties go to the lower index, so the result does not depend
    /// on hash-map order.
    fn nearest(&self, set: &SplatSet, p: Vec3, dist: f32) -> Option<u32> {
        let (cx, cy, cz) = self.key(p);
        let d2 = dist * dist;
        let mut best: Option<(f32, u32)> = None;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(bucket) = self.map.get(&(cx + dx, cy + dy, cz + dz)) else {
                        continue;
                    };
                    for &i in bucket {
                        let q = set.center(i as usize);
                        let dd = dot(sub(p, q), sub(p, q));
                        if dd <= d2 && best.is_none_or(|(bd, bi)| dd < bd || (dd == bd && i < bi)) {
                            best = Some((dd, i));
                        }
                    }
                }
            }
        }
        best.map(|(_, i)| i)
    }

    fn insert(&mut self, i: u32, p: Vec3) {
        self.map.entry(self.key(p)).or_default().push(i);
    }
}

/// The direction drips run on a surface with normal `n`: gravity projected onto the tangent plane. Zero on a
/// ceiling- or floor-facing surface (paint there pools instead of running).
pub fn drip_dir(n: Vec3) -> Vec3 {
    let g = [0.0, -1.0, 0.0];
    let along = dot(g, n);
    let d = [
        g[0] - n[0] * along,
        g[1] - n[1] * along,
        g[2] - n[2] * along,
    ];
    if dot(d, d) > 1e-12 {
        norm(d)
    } else {
        [0.0; 3]
    }
}

impl Mesh {
    /// Stamps a stroke of Gaussian splats onto the surface. Returns the number of stamps deposited (new splats plus
    /// pours into existing ones).
    #[allow(clippy::too_many_arguments)]
    pub fn splat_paint(
        &mut self,
        dabs: &[(Vec3, f32)],
        brush_radius: f32,
        spacing: f32,
        size: f32,
        color: [u8; 3],
        opacity: f32,
        flow: f32,
        variation: f32,
        color_noise: f32,
        drip: f32,
        seed: u32,
        view: Option<Vec3>,
        align: bool,
    ) -> usize {
        if dabs.is_empty() || brush_radius <= 0.0 || size <= 0.0 || opacity <= 0.0 || flow <= 0.0 {
            return 0;
        }
        let bvh;
        let bvh = if self.face_count() > 0 {
            bvh = Bvh::build(self);
            Some(&bvh)
        } else {
            None
        };
        let merge_dist = size * 0.75;
        let mut grid = MergeGrid::of(&self.splats, merge_dist.max(1e-6));
        let mut deposited = 0usize;
        for (di, &(c, pressure)) in dabs.iter().enumerate() {
            let seed = seed.wrapping_add(di as u32);
            let stamps = dab_stamps(
                self,
                bvh,
                c,
                pressure,
                brush_radius,
                spacing,
                size,
                opacity,
                flow,
                variation,
                drip,
                seed,
                view,
                align,
            );
            for st in stamps {
                if st.opacity <= 0.0 {
                    continue;
                }
                deposited += 1;
                // Colour variation: value noise lightens or darkens the stamp.
                let rgb = if color_noise > 0.0 {
                    let v = crate::detail::fbm(
                        [
                            st.center[0] * 37.0,
                            st.center[1] * 37.0,
                            st.center[2] * 37.0,
                        ],
                        2,
                        seed,
                    );
                    let f = 1.0 + color_noise * (2.0 * v - 1.0) * 0.5;
                    color.map(|c| q8(c as f32 * f))
                } else {
                    color
                };
                match grid.nearest(&self.splats, st.center, merge_dist) {
                    // Wet paint on wet paint: pour the opacity in and pull the colour across.
                    Some(i) => {
                        let i = i as usize;
                        let old = self.splats.opacities[i] as f32 / 255.0;
                        let add = st.opacity.clamp(0.0, 1.0);
                        let new = (old + add * (1.0 - old)).min(1.0);
                        self.splats.opacities[i] = q8(new * 255.0);
                        let pull = add / new.max(1e-6);
                        for k in 0..3 {
                            let c0 = self.splats.colors[i * 3 + k] as f32;
                            self.splats.colors[i * 3 + k] = q8(c0 + (rgb[k] as f32 - c0) * pull);
                        }
                        self.splats.radii[i] = self.splats.radii[i].max(st.radius);
                        self.splats.drips[i] = self.splats.drips[i].max(st.drip);
                    }
                    None => {
                        let i = self.splats.len() as u32;
                        self.splats.push(
                            st.center,
                            st.normal,
                            rgb,
                            q8(st.opacity.clamp(0.0, 1.0) * 255.0),
                            st.radius,
                            st.drip,
                        );
                        grid.insert(i, st.center);
                    }
                }
            }
        }
        deposited
    }

    /// Removes splats within `radius` of any dab (the eraser end of the brush). Pressure scales the radius.
    /// Returns the splats removed.
    pub fn splat_erase(&mut self, dabs: &[(Vec3, f32)], radius: f32) -> usize {
        if dabs.is_empty() || radius <= 0.0 {
            return 0;
        }
        let radii: Vec<f32> = dabs
            .iter()
            .map(|&(_, p)| radius * p.clamp(0.0, 1.0))
            .collect();
        let before = self.splats.len();
        self.splats.retain(|_, p| {
            !dabs.iter().zip(&radii).any(|(&(c, _), &r)| {
                let d = sub(p, c);
                dot(d, d) < r * r
            })
        });
        before - self.splats.len()
    }

    /// Blends splats inside `area` toward `color`. Returns the splats recoloured.
    pub fn splat_tint(&mut self, a: Area, color: [u8; 3], strength: f32) -> usize {
        let s = strength.clamp(0.0, 1.0);
        if s <= 0.0 {
            return 0;
        }
        let mut n = 0;
        for i in 0..self.splats.len() {
            if !a.contains(self.splats.center(i)) {
                continue;
            }
            n += 1;
            for k in 0..3 {
                let c0 = self.splats.colors[i * 3 + k] as f32;
                self.splats.colors[i * 3 + k] = q8(c0 + (color[k] as f32 - c0) * s);
            }
        }
        n
    }

    /// Pours the splats into the vertex colours: each vertex within `radius` of a splat is blended toward it,
    /// weighted by the splat's Gaussian and opacity. This is the bridge back to resolution-dependent colour, for
    /// tools that only read vertex colours. With `clear` the splats are removed afterwards. Returns the vertices
    /// recoloured.
    pub fn splat_bake(&mut self, radius: f32, strength: f32, clear: bool) -> usize {
        if self.splats.is_empty() || self.vertex_count() == 0 || strength <= 0.0 {
            return 0;
        }
        // Auto radius: twice the mean splat radius, so coverage is seamless at any brush size.
        let radius = if radius <= 0.0 {
            let mean = self.splats.radii.iter().sum::<f32>() / self.splats.len() as f32;
            2.0 * mean
        } else {
            radius
        };
        let mut grid = MergeGrid::of(&self.splats, radius.max(1e-6));
        grid.cell = radius.max(1e-6);
        let mut touched = 0;
        for vi in 0..self.vertex_count() {
            let p = self.vert(vi as u32);
            // Gather the splats covering this vertex.
            let (cx, cy, cz) = grid.key(p);
            let (mut wsum, mut acc) = (0f32, [0f32; 3]);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(bucket) = grid.map.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &i in bucket {
                            let i = i as usize;
                            let q = self.splats.center(i);
                            let d = sub(p, q);
                            let dd = dot(d, d);
                            if dd >= radius * radius {
                                continue;
                            }
                            let sigma = (self.splats.radii[i] * 2.5).max(1e-6);
                            let w = (self.splats.opacities[i] as f32 / 255.0)
                                * exp(-0.5 * dd / (sigma * sigma))
                                * smoothstep(1.0 - sqrt(dd) / radius);
                            wsum += w;
                            for k in 0..3 {
                                acc[k] += w * self.splats.colors[i * 3 + k] as f32;
                            }
                        }
                    }
                }
            }
            if wsum <= 0.0 {
                continue;
            }
            touched += 1;
            let mix = (strength * wsum).min(1.0);
            for k in 0..3 {
                let paint = acc[k] / wsum;
                let c0 = self.colors[vi * 3 + k] as f32;
                self.colors[vi * 3 + k] = q8(c0 + (paint - c0) * mix);
            }
        }
        if clear {
            self.splats = SplatSet::default();
        }
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Context;
    use crate::Op;

    fn wall() -> Mesh {
        let mut m = Mesh::default();
        let op: Op = serde_json::from_str(
            r#"{"op":"addBox","box":{"center":[0,1,0],"size":[4,2,0.1]},"color":[200,200,200]}"#,
        )
        .unwrap();
        op.apply(&mut m, Context::default()).unwrap();
        m
    }

    #[test]
    fn paint_is_deterministic_and_resolution_independent() {
        let dabs = [([0.0, 1.0, 0.0], 1.0f32)];
        let mut a = wall();
        let mut b = wall();
        let na = a.splat_paint(
            &dabs,
            0.5,
            0.05,
            0.03,
            [255, 0, 0],
            1.0,
            0.5,
            0.2,
            0.0,
            0.0,
            42,
            Some([0.0, 0.0, -1.0]),
            true,
        );
        let nb = b.splat_paint(
            &dabs,
            0.5,
            0.05,
            0.03,
            [255, 0, 0],
            1.0,
            0.5,
            0.2,
            0.0,
            0.0,
            42,
            Some([0.0, 0.0, -1.0]),
            true,
        );
        assert_eq!(na, nb);
        assert!(na > 0);
        assert_eq!(a, b);
        // The mesh itself is untouched: same vertices, same faces, only splats added.
        assert_eq!(a.face_count(), wall().face_count());
        assert!(a.content_hash() != wall().content_hash());
    }

    #[test]
    fn overpainting_builds_opacity_not_duplicates() {
        let dabs = [([0.0, 1.0, 0.0], 1.0f32)];
        let mut m = wall();
        m.splat_paint(
            &dabs,
            0.3,
            0.05,
            0.03,
            [255, 0, 0],
            1.0,
            0.6,
            0.0,
            0.0,
            0.0,
            1,
            Some([0.0, 0.0, -1.0]),
            true,
        );
        let n1 = m.splats.len();
        m.splat_paint(
            &dabs,
            0.3,
            0.05,
            0.03,
            [255, 0, 0],
            1.0,
            0.6,
            0.0,
            0.0,
            0.0,
            1,
            Some([0.0, 0.0, -1.0]),
            true,
        );
        // Same stroke again: every stamp lands on an existing splat.
        assert_eq!(m.splats.len(), n1);
        let max_op = m.splats.opacities.iter().copied().max().unwrap();
        assert!(max_op > q8(255.0 * 0.6));
    }

    #[test]
    fn erase_and_bake() {
        let dabs = [([0.0, 1.0, 0.0], 1.0f32)];
        let mut m = wall();
        // Bake pours splats into nearby vertices, so the wall needs some resolution where it is painted.
        let sub: Op = serde_json::from_str(r#"{"op":"subdivide","maxEdge":0.05}"#).unwrap();
        sub.apply(&mut m, Context::default()).unwrap();
        m.splat_paint(
            &dabs,
            0.3,
            0.05,
            0.03,
            [10, 200, 30],
            1.0,
            1.0,
            0.0,
            0.0,
            0.0,
            7,
            Some([0.0, 0.0, -1.0]),
            true,
        );
        let n = m.splats.len();
        assert!(n > 0);
        let baked = m.splat_bake(0.0, 1.0, false);
        assert!(baked > 0);
        assert!(m.colors.iter().any(|&c| c != 200));
        let gone = m.splat_erase(&dabs, 1.0);
        assert_eq!(gone, n);
        assert!(m.splats.is_empty());
    }
}
