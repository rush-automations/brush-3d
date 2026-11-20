//! Detail: the edits that make clean geometry read as real. Noise displacement, noise tints, baked ambient occlusion,
//! and colour taken from nearby geometry.

use crate::math::*;
use crate::mesh::Mesh;
use crate::region::Area;
use rustc_hash::FxHashMap;

fn hash3(ix: i32, iy: i32, iz: i32, seed: u32) -> f32 {
    let mut h = (ix as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((iy as u32).wrapping_mul(668_265_263))
        .wrapping_add((iz as u32).wrapping_mul(2_147_483_629))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 16_777_216.0
}

fn vnoise(p: Vec3, seed: u32) -> f32 {
    let (fx, fy, fz) = (floor(p[0]), floor(p[1]), floor(p[2]));
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (tx, ty, tz) = (
        smoothstep(p[0] - fx),
        smoothstep(p[1] - fy),
        smoothstep(p[2] - fz),
    );
    let c: [f32; 8] = std::array::from_fn(|k| {
        let (dx, dy, dz) = ((k & 1) as i32, ((k >> 1) & 1) as i32, ((k >> 2) & 1) as i32);
        hash3(
            ix.wrapping_add(dx),
            iy.wrapping_add(dy),
            iz.wrapping_add(dz),
            seed,
        )
    });
    let x00 = c[0] + (c[1] - c[0]) * tx;
    let x10 = c[2] + (c[3] - c[2]) * tx;
    let x01 = c[4] + (c[5] - c[4]) * tx;
    let x11 = c[6] + (c[7] - c[6]) * tx;
    let y0 = x00 + (x10 - x00) * ty;
    let y1 = x01 + (x11 - x01) * ty;
    y0 + (y1 - y0) * tz
}

/// Fractal value noise in 0..1.
pub fn fbm(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut total) = (0f32, 0.5f32, 0f32);
    let mut q = p;
    for o in 0..octaves.clamp(1, 8) {
        sum += amp * vnoise(q, seed.wrapping_add(o.wrapping_mul(7919)));
        total += amp;
        amp *= 0.5;
        q = [q[0] * 2.03, q[1] * 2.03, q[2] * 2.03];
    }
    sum / total
}

fn weld_key(p: Vec3) -> [i32; 3] {
    [
        round(p[0] * 2000.0) as i32,
        round(p[1] * 2000.0) as i32,
        round(p[2] * 2000.0) as i32,
    ]
}

impl Mesh {
    /// Area-weighted vertex normals, shared between vertices at the same position (cuts and shape seams), so
    /// displacing along them cannot tear a seam open.
    pub fn welded_normals(&self) -> Vec<Vec3> {
        let mut map: FxHashMap<[i32; 3], Vec3> = FxHashMap::default();
        for t in 0..self.face_count() {
            let [i, j, k] = self.face(t);
            let (a, b, c) = (self.vert(i), self.vert(j), self.vert(k));
            let n = cross(sub(b, a), sub(c, a));
            for p in [a, b, c] {
                let e = map.entry(weld_key(p)).or_insert([0.0; 3]);
                *e = [e[0] + n[0], e[1] + n[1], e[2] + n[2]];
            }
        }
        (0..self.vertex_count())
            .map(|i| {
                norm(
                    map.get(&weld_key(self.vert(i as u32)))
                        .copied()
                        .unwrap_or([0.0, 1.0, 0.0]),
                )
            })
            .collect()
    }

    /// Pushes vertices along their surface normal by fractal noise: chips, pitting, uneven stone. `amp` is the largest
    /// displacement in meters, `scale` the feature size, `edge` how far in from the box boundary the effect eases
    /// off; `ridged` makes crack-like ridges instead of soft lumps. Returns vertices moved.
    #[allow(clippy::too_many_arguments)]
    pub fn roughen(
        &mut self,
        a: Area,
        amp: f32,
        scale: f32,
        octaves: u32,
        seed: u32,
        edge: f32,
        ridged: bool,
    ) -> usize {
        let v = self.vertex_count();
        if v == 0 || scale <= 0.0 {
            return 0;
        }
        let normals = self.welded_normals();
        let orig = self.positions.clone();
        let mut n = 0;
        for i in 0..v {
            let p = [orig[i * 3], orig[i * 3 + 1], orig[i * 3 + 2]];
            let w = a.weight(p, edge);
            if w <= 0.0 {
                continue;
            }
            let mut t = fbm([p[0] / scale, p[1] / scale, p[2] / scale], octaves, seed);
            if ridged {
                t = 1.0 - (2.0 * t - 1.0).abs();
            }
            let d = (t - 0.5) * 2.0 * amp * w;
            for k in 0..3 {
                self.positions[i * 3 + k] = p[k] + normals[i][k] * d;
            }
            n += 1;
        }
        n
    }

    /// Varies vertex colours with noise. `shade == false` blends from colour `ca` to `cb` across the noise and mixes
    /// that in by `strength`; `shade == true` only darkens and lightens what is there. The noise is stretched by
    /// `stretch` (a long stretch along x gives wood grain). Returns vertices changed.
    #[allow(clippy::too_many_arguments)]
    pub fn tint(
        &mut self,
        a: Area,
        ca: [u8; 3],
        cb: [u8; 3],
        scale: f32,
        stretch: Vec3,
        octaves: u32,
        seed: u32,
        strength: f32,
        shade: bool,
    ) -> usize {
        if scale <= 0.0 {
            return 0;
        }
        let fa = ca.map(|v| v as f32);
        let fb = cb.map(|v| v as f32);
        let mut n = 0;
        for i in 0..self.vertex_count() {
            let p = self.vert(i as u32);
            let w = a.weight(p, 0.0);
            if w <= 0.0 {
                continue;
            }
            let t = fbm(
                [
                    p[0] / (scale * stretch[0]),
                    p[1] / (scale * stretch[1]),
                    p[2] / (scale * stretch[2]),
                ],
                octaves,
                seed,
            );
            let s = strength.clamp(0.0, 1.0) * w;
            for j in 0..3 {
                let old = self.colors[i * 3 + j] as f32;
                let next = if shade {
                    old * (1.0 + (t - 0.5) * 2.0 * s)
                } else {
                    old + ((fa[j] + (fb[j] - fa[j]) * t) - old) * s
                };
                self.colors[i * 3 + j] = q8(next);
            }
            n += 1;
        }
        n
    }

    /// Bakes ambient occlusion into the vertex colours: vertices in corners, under ledges and between columns get
    /// darker. Occlusion is measured against a voxel grid of the surface (`cell` meters), out to `radius` meters in
    /// `samples` directions. Returns vertices shaded.
    pub fn bake_shadows(
        &mut self,
        a: Area,
        strength: f32,
        radius: f32,
        cell_in: f32,
        samples_in: u32,
    ) -> usize {
        let v = self.vertex_count();
        if v == 0 || self.face_count() == 0 {
            return 0;
        }
        let (lo, hi) = self.bounds().unwrap();
        let mut cs = cell_in.max(0.15); // finer cells would measure a scan's own bumps, not real corners
        let mut dims = [0usize; 3];
        loop {
            let mut total = 1usize;
            for k in 0..3 {
                dims[k] = ((hi[k] - lo[k]) / cs) as usize + 3;
                total = total.saturating_mul(dims[k]);
            }
            if total <= 12_000_000 {
                break;
            }
            cs *= 1.5;
        }
        let mut grid = vec![0u8; dims[0] * dims[1] * dims[2]];
        let at = |p: Vec3| -> Option<usize> {
            let mut k = [0usize; 3];
            for ax in 0..3 {
                let f = (p[ax] - lo[ax]) / cs + 1.0;
                if f < 0.0 {
                    return None;
                }
                let u = f as usize;
                if u >= dims[ax] {
                    return None;
                }
                k[ax] = u;
            }
            Some((k[2] * dims[1] + k[1]) * dims[0] + k[0])
        };
        // Mark the voxels the surface passes through, sampling each triangle finely enough to leave no gaps.
        for t in 0..self.face_count() {
            let [i, j, k] = self.face(t);
            let (pa, pb, pd) = (self.vert(i), self.vert(j), self.vert(k));
            let longest = sqrt(
                dot(sub(pb, pa), sub(pb, pa))
                    .max(dot(sub(pd, pb), sub(pd, pb)).max(dot(sub(pa, pd), sub(pa, pd)))),
            );
            let steps = (ceil(longest / (cs * 0.6)) as usize + 1).min(400);
            for i in 0..=steps {
                for j in 0..=(steps - i) {
                    let u = i as f32 / steps as f32;
                    let w = j as f32 / steps as f32;
                    let p = [
                        pa[0] + (pb[0] - pa[0]) * u + (pd[0] - pa[0]) * w,
                        pa[1] + (pb[1] - pa[1]) * u + (pd[1] - pa[1]) * w,
                        pa[2] + (pb[2] - pa[2]) * u + (pd[2] - pa[2]) * w,
                    ];
                    if let Some(o) = at(p) {
                        grid[o] = 1;
                    }
                }
            }
        }
        // Flood the open air in from outside. Space sealed inside a hollow shell is never reached, so surfaces are
        // only shaded by the air they actually face.
        {
            let mut queue: Vec<u32> = vec![0];
            grid[0] = 2;
            let mut head = 0;
            while head < queue.len() {
                let o = queue[head] as usize;
                head += 1;
                let (x, y, z) = (
                    o % dims[0],
                    (o / dims[0]) % dims[1],
                    o / (dims[0] * dims[1]),
                );
                for d in [
                    [-1isize, 0, 0],
                    [1, 0, 0],
                    [0, -1, 0],
                    [0, 1, 0],
                    [0, 0, -1],
                    [0, 0, 1],
                ] {
                    let (nx, ny, nz) = (x as isize + d[0], y as isize + d[1], z as isize + d[2]);
                    if nx < 0
                        || ny < 0
                        || nz < 0
                        || nx >= dims[0] as isize
                        || ny >= dims[1] as isize
                        || nz >= dims[2] as isize
                    {
                        continue;
                    }
                    let no = (nz as usize * dims[1] + ny as usize) * dims[0] + nx as usize;
                    if grid[no] != 0 {
                        continue;
                    }
                    grid[no] = 2;
                    queue.push(no as u32);
                }
            }
        }
        let normals = self.welded_normals();
        let samples = samples_in.clamp(4, 64);
        // A Fibonacci sphere: evenly spread directions.
        let dirs: Vec<Vec3> = (0..samples)
            .map(|k| {
                let fk = k as f32;
                let y = 1.0 - 2.0 * (fk + 0.5) / samples as f32;
                let r = sqrt((1.0 - y * y).max(0.0));
                let phi = fk * 2.399_963_2;
                [r * cos(phi), y, r * sin(phi)]
            })
            .collect();
        let reach = (cs * 2.0).max(radius);
        let mut shaded = 0;
        for i in 0..v {
            let p = self.vert(i as u32);
            let w = a.weight(p, 0.0);
            if w <= 0.0 {
                continue;
            }
            let n = normals[i];
            // Scans have no reliable winding: measure the air on both sides and take the more occluded one.
            let mut blocked = [0u32; 2];
            let mut valid = [0u32; 2];
            for &dir in &dirs {
                let along = dot(dir, n);
                if along.abs() < 0.45 {
                    continue; // near-tangent rays only graze the surface's own bumps
                }
                let side = if along > 0.0 { 0 } else { 1 };
                let mut s = (cs * 2.5).max(0.15);
                let mut started = false;
                let mut hit = false;
                while s <= reach {
                    let q = [p[0] + dir[0] * s, p[1] + dir[1] * s, p[2] + dir[2] * s];
                    let Some(o) = at(q) else { break };
                    if !started {
                        if grid[o] == 0 {
                            break; // sealed space: skip this ray
                        }
                        started = true;
                    }
                    if grid[o] == 1 {
                        hit = true;
                        break;
                    }
                    s += cs * 0.9;
                }
                if !started {
                    continue;
                }
                valid[side] += 1;
                if hit {
                    blocked[side] += 1;
                }
            }
            let frac = |s: usize| {
                if valid[s] > 0 {
                    blocked[s] as f32 / valid[s] as f32
                } else {
                    0.0
                }
            };
            let occ = frac(0).max(frac(1));
            let k = 1.0 - strength.clamp(0.0, 1.0) * occ * w;
            for j in 0..3 {
                self.colors[i * 3 + j] = q8(self.colors[i * 3 + j] as f32 * k);
            }
            shaded += 1;
        }
        shaded
    }

    /// Gives the vertices inside the box the colour of the nearest source vertex within `max_dist`, mixed in by
    /// `strength`. Sources are `reference` (positions and colours of another mesh, normally the untouched scan) or,
    /// without one, every vertex of this mesh outside the box. Returns vertices recoloured.
    pub fn match_color(
        &mut self,
        a: Area,
        max_dist: f32,
        strength: f32,
        reference: Option<&Mesh>,
    ) -> usize {
        let v = self.vertex_count();
        if v == 0 || max_dist <= 0.0 {
            return 0;
        }
        let inside: Vec<bool> = (0..v)
            .map(|i| a.weight(self.vert(i as u32), 0.0) > 0.0)
            .collect();
        let src: &Mesh = reference.unwrap_or(self);
        let use_ref = reference.is_some();
        let src_n = src.vertex_count();
        const NONE: u32 = u32::MAX;
        let mut head: FxHashMap<[i32; 3], u32> = FxHashMap::default();
        let mut next = vec![NONE; src_n];
        for i in 0..src_n {
            if !use_ref && inside[i] {
                continue;
            }
            let p = src.vert(i as u32);
            let key = [
                cell(p[0], max_dist),
                cell(p[1], max_dist),
                cell(p[2], max_dist),
            ];
            let e = head.entry(key).or_insert(NONE);
            next[i] = *e;
            *e = i as u32;
        }
        // Sources keep their own colours while destinations change.
        let snapshot = src.colors.clone();
        let src_pos = src.positions.clone();
        let s = strength.clamp(0.0, 1.0);
        let mut changed = 0;
        for i in 0..v {
            if !inside[i] {
                continue;
            }
            let p = self.vert(i as u32);
            let (kx, ky, kz) = (
                cell(p[0], max_dist),
                cell(p[1], max_dist),
                cell(p[2], max_dist),
            );
            let mut best = max_dist * max_dist;
            let mut found = NONE;
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let mut it = head
                            .get(&[kx + dx, ky + dy, kz + dz])
                            .copied()
                            .unwrap_or(NONE);
                        while it != NONE {
                            let o = it as usize * 3;
                            let d = sub([src_pos[o], src_pos[o + 1], src_pos[o + 2]], p);
                            let dd = dot(d, d);
                            if dd < best {
                                best = dd;
                                found = it;
                            }
                            it = next[it as usize];
                        }
                    }
                }
            }
            if found == NONE {
                continue;
            }
            for j in 0..3 {
                let old = self.colors[i * 3 + j] as f32;
                let new = snapshot[found as usize * 3 + j] as f32;
                self.colors[i * 3 + j] = q8(old + (new - old) * s);
            }
            changed += 1;
        }
        changed
    }
}
