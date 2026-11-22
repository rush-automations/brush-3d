//! Brush sculpting. A stroke is a list of dabs (sphere centres on the surface); each dab moves or recolours the
//! vertices within `radius`, weighted by a smooth falloff. Dabs are applied one after another, each reading the
//! surface the previous one left, and normals are recomputed locally from the faces around each vertex. So a stroke
//! applied in several pieces gives exactly the same mesh as the whole stroke at once, which lets an app stream a
//! stroke live and record it as one op.

use crate::math::{cross, dot, norm, round, smoothstep, sqrt, sub, Vec3};
use crate::mesh::Mesh;
use crate::topo::{vertex_faces, vertex_neighbors, vertex_normal, Csr};
use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn weld_key(p: Vec3) -> [i32; 3] {
    [
        round(p[0] * 2000.0) as i32,
        round(p[1] * 2000.0) as i32,
        round(p[2] * 2000.0) as i32,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Brush {
    /// Raise the surface along its average normal (lower it with `invert`).
    Draw,
    /// Push each vertex out along its own normal: swells shapes (shrinks with `invert`).
    Inflate,
    /// Relax toward the neighbours' average: removes bumps and noise.
    Smooth,
    /// Pull vertices onto the plane that best fits the area under the brush.
    Flatten,
    /// Pull vertices toward the brush centre along the surface: sharpens creases (spreads with `invert`).
    Pinch,
    /// Blend the vertex colours toward `color`.
    Paint,
}

/// One brush touch. `pressure` (0..1) scales strength, as from a pen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dab {
    pub center: [f64; 3],
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub pressure: f64,
}

fn full() -> f64 {
    1.0
}
fn is_full(v: &f64) -> bool {
    *v == 1.0
}

/// Topology a sculpt stroke reuses between pieces. Valid while faces do not change.
pub struct SculptTopology {
    pub vf: Csr,
    pub nb: Csr,
}

impl SculptTopology {
    pub fn of(mesh: &Mesh) -> SculptTopology {
        SculptTopology {
            vf: vertex_faces(mesh),
            nb: vertex_neighbors(mesh),
        }
    }
}

impl Mesh {
    /// Applies the dabs in order. `view` is the direction the brush is pointed in (camera toward surface): normals are
    /// turned to face it, so "out" means toward whoever is sculpting even where a scan's winding is unreliable.
    /// Returns the number of distinct vertices touched.
    #[allow(clippy::too_many_arguments)]
    pub fn sculpt(
        &mut self,
        brush: Brush,
        dabs: &[(Vec3, f32)],
        radius: f32,
        strength: f32,
        invert: bool,
        color: [u8; 3],
        view: Option<Vec3>,
        topo: Option<&SculptTopology>,
    ) -> usize {
        let mut touched = vec![false; self.vertex_count()];
        self.sculpt_into(
            brush,
            dabs,
            radius,
            strength,
            invert,
            color,
            view,
            topo,
            &mut touched,
        );
        touched.iter().filter(|&&t| t).count()
    }

    /// [`Mesh::sculpt`], marking touched vertices in `touched` (one flag per vertex, kept across the pieces of a
    /// stroke). Returns the vertices this call touched, counting repeats.
    #[allow(clippy::too_many_arguments)]
    pub fn sculpt_into(
        &mut self,
        brush: Brush,
        dabs: &[(Vec3, f32)],
        radius: f32,
        strength: f32,
        invert: bool,
        color: [u8; 3],
        view: Option<Vec3>,
        topo: Option<&SculptTopology>,
        touched: &mut [bool],
    ) -> usize {
        if radius <= 0.0 || dabs.is_empty() || self.vertex_count() == 0 {
            return 0;
        }
        let mut hits_total = 0;
        let owned;
        let topo = match topo {
            Some(t) => t,
            None => {
                owned = SculptTopology::of(self);
                &owned
            }
        };
        let sign = if invert { -1.0 } else { 1.0 };
        let r2 = radius * radius;
        let mut hit: Vec<(u32, f32)> = Vec::new();
        let mut moves: Vec<[f32; 3]> = Vec::new();
        for &(c, pressure) in dabs {
            hit.clear();
            for (i, p) in self.positions.chunks_exact(3).enumerate() {
                let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                let dd = dot(d, d);
                if dd < r2 {
                    let w =
                        smoothstep(1.0 - sqrt(dd) / radius) * strength * pressure.clamp(0.0, 1.0);
                    if w > 0.0 {
                        hit.push((i as u32, w));
                    }
                }
            }
            if hit.is_empty() {
                continue;
            }
            for &(i, _) in &hit {
                touched[i as usize] = true;
                hits_total += 1;
            }
            if brush == Brush::Paint {
                let fc = color.map(|v| v as f32);
                for &(i, w) in &hit {
                    let k = w.min(1.0);
                    for j in 0..3 {
                        let o = &mut self.colors[i as usize * 3 + j];
                        *o = crate::math::q8(*o as f32 + (fc[j] - *o as f32) * k);
                    }
                }
                continue;
            }
            // The area under the brush: weighted centre and normal.
            // For Draw and Inflate, weld normals across coincident positions so hard edges/seams
            // move together instead of tearing open into sharp jagged lips.
            let mut weld_counts: FxHashMap<[i32; 3], u32> = FxHashMap::default();
            let normals: Vec<Vec3> = if matches!(brush, Brush::Smooth) {
                Vec::new()
            } else if matches!(brush, Brush::Draw | Brush::Inflate) {
                let mut welded: FxHashMap<[i32; 3], Vec3> = FxHashMap::default();
                for &(i, _) in &hit {
                    let key = weld_key(self.vert(i));
                    *weld_counts.entry(key).or_insert(0) += 1;
                    let entry = welded.entry(key).or_insert([0.0; 3]);
                    for &f in topo.vf.row(i as usize) {
                        let [a, b, c] = self.face(f as usize);
                        let n = cross(sub(self.vert(b), self.vert(a)), sub(self.vert(c), self.vert(a)));
                        entry[0] += n[0];
                        entry[1] += n[1];
                        entry[2] += n[2];
                    }
                }
                hit.iter()
                    .map(|&(i, _)| {
                        let raw = welded
                            .get(&weld_key(self.vert(i)))
                            .copied()
                            .unwrap_or([0.0, 1.0, 0.0]);
                        let n = norm(raw);
                        let n = if dot(n, n) > 0.0 {
                            n
                        } else {
                            vertex_normal(self, &topo.vf, i as usize)
                        };
                        match view {
                            Some(v) if dot(n, v) > 0.0 => n.map(|x| -x),
                            _ => n,
                        }
                    })
                    .collect()
            } else {
                hit.iter()
                    .map(|&(i, _)| {
                        let n = vertex_normal(self, &topo.vf, i as usize);
                        match view {
                            Some(v) if dot(n, v) > 0.0 => n.map(|x| -x),
                            _ => n,
                        }
                    })
                    .collect()
            };
            let (mut an, mut ac, mut wsum) = ([0f32; 3], [0f32; 3], 0f32);
            for (k, &(i, w)) in hit.iter().enumerate() {
                let p = self.vert(i);
                if let Some(n) = normals.get(k) {
                    an = [an[0] + n[0] * w, an[1] + n[1] * w, an[2] + n[2] * w];
                }
                ac = [ac[0] + p[0] * w, ac[1] + p[1] * w, ac[2] + p[2] * w];
                wsum += w;
            }
            let an = norm(an);
            let ac = ac.map(|v| v / wsum);
            let step = match brush {
                Brush::Inflate => radius * 0.16,
                _ => radius * 0.25,
            };
            moves.clear();
            for (k, &(i, w)) in hit.iter().enumerate() {
                let p = self.vert(i);
                let m: Vec3 = match brush {
                    Brush::Draw => an.map(|v| v * sign * step * w),
                    Brush::Inflate => normals[k].map(|v| v * sign * step * w),
                    Brush::Flatten => {
                        let dist = dot(sub(p, ac), an);
                        an.map(|v| -v * dist * w.min(1.0))
                    }
                    Brush::Pinch => {
                        let to = sub(c, p);
                        let along = dot(to, an);
                        let tangent = [
                            to[0] - an[0] * along,
                            to[1] - an[1] * along,
                            to[2] - an[2] * along,
                        ];
                        tangent.map(|v| v * sign * 0.5 * w.min(1.0))
                    }
                    Brush::Smooth => {
                        let row = topo.nb.row(i as usize);
                        if row.is_empty() {
                            [0.0; 3]
                        } else {
                            let mut s = [0f32; 3];
                            for &j in row {
                                let q = self.vert(j);
                                s = [s[0] + q[0], s[1] + q[1], s[2] + q[2]];
                            }
                            let n = row.len() as f32;
                            [0, 1, 2].map(|j| (s[j] / n - p[j]) * w.min(1.0))
                        }
                    }
                    Brush::Paint => unreachable!(),
                };
                moves.push(m);
            }
            for (k, &(i, _)) in hit.iter().enumerate() {
                let o = i as usize * 3;
                for j in 0..3 {
                    self.positions[o + j] += moves[k][j];
                }
            }
            // Gentle tangential relaxation after Draw/Inflate prevents vertices from bunching into
            // sharp jagged spikes while preserving the raised/inflated volume (skipping unwelded seams).
            if matches!(brush, Brush::Draw | Brush::Inflate) {
                moves.clear();
                for (k, &(i, w)) in hit.iter().enumerate() {
                    let row = topo.nb.row(i as usize);
                    let p = self.vert(i);
                    let is_seam = weld_counts.get(&weld_key(p)).copied().unwrap_or(1) > 1;
                    if row.is_empty() || is_seam {
                        moves.push([0.0; 3]);
                        continue;
                    }
                    let mut s = [0f32; 3];
                    for &j in row {
                        let q = self.vert(j);
                        s = [s[0] + q[0], s[1] + q[1], s[2] + q[2]];
                    }
                    let n = row.len() as f32;
                    let delta = [s[0] / n - p[0], s[1] / n - p[1], s[2] / n - p[2]];
                    let vn = normals[k];
                    let along = dot(delta, vn);
                    let relax = 0.22 * w.min(1.0);
                    moves.push([
                        (delta[0] - vn[0] * along) * relax,
                        (delta[1] - vn[1] * along) * relax,
                        (delta[2] - vn[2] * along) * relax,
                    ]);
                }
                for (k, &(i, _)) in hit.iter().enumerate() {
                    let o = i as usize * 3;
                    for j in 0..3 {
                        self.positions[o + j] += moves[k][j];
                    }
                }
            }
        }
        hits_total
    }
}
