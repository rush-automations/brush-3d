//! Selection, group transforms, 90° world-axis snapping, and clipboard math.
//!
//! All trigonometry goes through `libm` in `f64` so browser gestures, numeric panels, native apps, and agents
//! resolve identical transforms on every platform.

use brush_mesh::Op;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scene::Instance;

/// A world axis to rotate about: `Y` is vertical (yaw), `X` and `Z` tilt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

/// How close (degrees) a free turn must come to a 90° stop before snapping to it.
pub const SNAP_DEG: f64 = 8.0;
pub const MIN_SCALE: f64 = 0.05;
pub const MAX_SCALE: f64 = 20.0;

const DEG: f64 = core::f64::consts::PI / 180.0;

/// One copied placement, stored relative to the copied group's pivot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipItem {
    pub object: String,
    pub offset: [f64; 3],
    #[serde(default)]
    pub yaw_deg: f64,
    #[serde(default)]
    pub pitch_deg: f64,
    #[serde(default)]
    pub roll_deg: f64,
    #[serde(default = "one")]
    pub scale: f64,
}

fn one() -> f64 {
    1.0
}

/// A copied group of placements ready to paste.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Clipboard {
    pub items: Vec<ClipItem>,
    /// Where the group's pivot was when copied.
    pub pivot: [f64; 3],
    /// How far a paste without a pointer steps aside along +x, meters.
    pub step: f64,
}

#[inline]
fn clean(deg: f64) -> f64 {
    let r = libm::floor(deg * 1e6 + 0.5) / 1e6;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

#[inline]
fn norm_deg(deg: f64) -> f64 {
    let mut r = libm::fmod(deg, 360.0);
    if r < 0.0 {
        r += 360.0;
    }
    if (r - 360.0).abs() < 1e-9 || r == 0.0 {
        0.0
    } else {
        clean(r)
    }
}

type Mat3 = [[f64; 3]; 3];

fn mul3(a: Mat3, b: Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn apply3(m: Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn axis_rotation(axis: Axis, deg: f64) -> Mat3 {
    let a = deg * DEG;
    let (c, s) = (libm::cos(a), libm::sin(a));
    match axis {
        Axis::Y => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        Axis::X => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        Axis::Z => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// `R = Ry(yaw) * Rx(pitch) * Rz(roll)` in `f64`, matching [`crate::scene::rotation`] and three.js `"YXZ"`.
pub fn rotation_f64(yaw_deg: f64, pitch_deg: f64, roll_deg: f64) -> Mat3 {
    mul3(
        mul3(
            axis_rotation(Axis::Y, yaw_deg),
            axis_rotation(Axis::X, pitch_deg),
        ),
        axis_rotation(Axis::Z, roll_deg),
    )
}

/// Decomposes a rotation matrix into `(yaw_deg, pitch_deg, roll_deg)` in `"YXZ"` order.
pub fn euler_yxz_deg(m: Mat3) -> (f64, f64, f64) {
    let m12 = m[1][2].clamp(-1.0, 1.0);
    let pitch = libm::asin(-m12) / DEG;
    let (yaw, roll) = if m12.abs() < 0.999_999_9 {
        (
            libm::atan2(m[0][2], m[2][2]) / DEG,
            libm::atan2(m[1][0], m[1][1]) / DEG,
        )
    } else {
        (libm::atan2(-m[2][0], m[0][0]) / DEG, 0.0)
    };
    (norm_deg(yaw), clean(pitch), clean(roll))
}

fn axis_unit(axis: Axis) -> [f64; 3] {
    match axis {
        Axis::X => [1.0, 0.0, 0.0],
        Axis::Y => [0.0, 1.0, 0.0],
        Axis::Z => [0.0, 0.0, 1.0],
    }
}

fn axis_plane(axis: Axis) -> ([f64; 3], [f64; 3]) {
    match axis {
        Axis::Y => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        Axis::X => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        Axis::Z => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    }
}

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Angle of `v` in the plane perpendicular to `axis`, in degrees.
pub fn plane_angle(axis: Axis, v: [f64; 3]) -> f64 {
    let (u, w) = axis_plane(axis);
    libm::atan2(dot(v, w), dot(v, u)) / DEG
}

/// The group's pivot: the x/z centre of its placements at the lowest placement's y.
pub fn group_pivot(list: &[Instance]) -> [f64; 3] {
    if list.is_empty() {
        return [0.0; 3];
    }
    let (mut x0, mut x1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut y = f64::INFINITY;
    for i in list {
        x0 = x0.min(i.position[0]);
        x1 = x1.max(i.position[0]);
        z0 = z0.min(i.position[2]);
        z1 = z1.max(i.position[2]);
        y = y.min(i.position[1]);
    }
    [(x0 + x1) * 0.5, y, (z0 + z1) * 0.5]
}

/// Shifts every placement in `list` by `delta`.
pub fn translate_group(list: &[Instance], delta: [f64; 3]) -> Vec<Instance> {
    list.iter()
        .map(|i| Instance {
            position: [
                i.position[0] + delta[0],
                i.position[1] + delta[1],
                i.position[2] + delta[2],
            ],
            ..i.clone()
        })
        .collect()
}

/// Turns `list` by `deg` about the vertical (+y) axis through `pivot`.
pub fn rotate_group(list: &[Instance], deg: f64, pivot: Option<[f64; 3]>) -> Vec<Instance> {
    let p = pivot.unwrap_or_else(|| group_pivot(list));
    let r = deg * DEG;
    let (c, s) = (libm::cos(r), libm::sin(r));
    list.iter()
        .map(|i| {
            let dx = i.position[0] - p[0];
            let dz = i.position[2] - p[2];
            Instance {
                position: [
                    p[0] + c * dx + s * dz,
                    i.position[1],
                    p[2] - s * dx + c * dz,
                ],
                yaw_deg: norm_deg(i.yaw_deg + deg),
                ..i.clone()
            }
        })
        .collect()
}

/// Turns `list` by `deg` about a world `axis` through `pivot`.
pub fn rotate_group_about(
    list: &[Instance],
    axis: Axis,
    deg: f64,
    pivot: [f64; 3],
) -> Vec<Instance> {
    if axis == Axis::Y {
        return rotate_group(list, deg, Some(pivot));
    }
    let rot = axis_rotation(axis, deg);
    list.iter()
        .map(|i| {
            let rel = [
                i.position[0] - pivot[0],
                i.position[1] - pivot[1],
                i.position[2] - pivot[2],
            ];
            let swung = apply3(rot, rel);
            let composed = mul3(rot, rotation_f64(i.yaw_deg, i.pitch_deg, i.roll_deg));
            let (yaw_deg, pitch_deg, roll_deg) = euler_yxz_deg(composed);
            Instance {
                position: [
                    pivot[0] + swung[0],
                    pivot[1] + swung[1],
                    pivot[2] + swung[2],
                ],
                yaw_deg,
                pitch_deg,
                roll_deg,
                ..i.clone()
            }
        })
        .collect()
}

/// Snaps a free turn of `raw` degrees about `axis` to 90° stops within [`SNAP_DEG`] (and for a single object,
/// also snaps back square to the world axes).
pub fn snap_turn(raw: f64, axis: Axis, start: &[Instance], snap: bool) -> f64 {
    if !snap {
        return raw;
    }
    let near = |base: f64| base + libm::floor((raw - base) / 90.0 + 0.5) * 90.0;
    let mut best = near(0.0);
    if start.len() == 1 {
        let s = &start[0];
        let m = rotation_f64(s.yaw_deg, s.pitch_deg, s.roll_deg);
        let ax = axis_unit(axis);
        let cols = [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ];
        for v in cols {
            let proj = dot(v, ax);
            let flat = [
                v[0] - ax[0] * proj,
                v[1] - ax[1] * proj,
                v[2] - ax[2] * proj,
            ];
            let len = libm::sqrt(dot(flat, flat));
            if len > 0.5 {
                let cand = near(-plane_angle(axis, flat));
                if (cand - raw).abs() < (best - raw).abs() {
                    best = cand;
                }
            }
        }
    }
    if (best - raw).abs() <= SNAP_DEG {
        clean(best)
    } else {
        raw
    }
}

/// Scales `list` (both positions and instance scales) by `factor` about `pivot`, clamped to `[MIN_SCALE, MAX_SCALE]`.
pub fn scale_group(list: &[Instance], factor: f64, pivot: Option<[f64; 3]>) -> Vec<Instance> {
    let p = pivot.unwrap_or_else(|| group_pivot(list));
    let mut f = factor;
    for i in list {
        let k = if i.scale > 0.0 { i.scale } else { 1.0 };
        f = f.min(MAX_SCALE / k).max(MIN_SCALE / k);
    }
    list.iter()
        .map(|i| {
            let k = if i.scale > 0.0 { i.scale } else { 1.0 };
            Instance {
                position: [
                    p[0] + (i.position[0] - p[0]) * f,
                    p[1] + (i.position[1] - p[1]) * f,
                    p[2] + (i.position[2] - p[2]) * f,
                ],
                scale: k * f,
                ..i.clone()
            }
        })
        .collect()
}

/// Horizontal drag offset from `from` to `to`, optionally snapped to `cell`.
pub fn drag_delta(from: [f64; 3], to: [f64; 3], cell: f64, snap: bool) -> [f64; 3] {
    let dx = to[0] - from[0];
    let dz = to[2] - from[2];
    if !snap || cell <= 0.0 {
        return [dx, 0.0, dz];
    }
    let round_js = |v: f64| {
        let r = libm::floor(v + 0.5);
        if r == 0.0 {
            0.0
        } else {
            r
        }
    };
    [round_js(dx / cell) * cell, 0.0, round_js(dz / cell) * cell]
}

/// Copies `list` relative to its group pivot so it can be pasted elsewhere.
pub fn copy_group(list: &[Instance], width: f64) -> Clipboard {
    let pivot = group_pivot(list);
    Clipboard {
        pivot,
        step: width.max(0.1),
        items: list
            .iter()
            .map(|i| ClipItem {
                object: i.object.clone(),
                offset: [
                    i.position[0] - pivot[0],
                    i.position[1] - pivot[1],
                    i.position[2] - pivot[2],
                ],
                yaw_deg: i.yaw_deg,
                pitch_deg: i.pitch_deg,
                roll_deg: i.roll_deg,
                scale: i.scale,
            })
            .collect(),
    }
}

/// Allocates `count` unused instance IDs (`o1`, `o2`, ...).
pub fn free_ids(existing: &[Instance], count: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(count);
    let mut n = 1usize;
    while out.len() < count {
        let cand = format!("o{n}");
        if !existing.iter().any(|i| i.id == cand) {
            out.push(cand);
        }
        n += 1;
    }
    out
}

/// Resolves pasted placements around `at` using `ids`.
pub fn paste_group(clip: &Clipboard, at: [f64; 3], ids: &[String]) -> Vec<Instance> {
    clip.items
        .iter()
        .zip(ids.iter())
        .map(|(c, id)| Instance {
            id: id.clone(),
            object: c.object.clone(),
            position: [
                at[0] + c.offset[0],
                at[1] + c.offset[1],
                at[2] + c.offset[2],
            ],
            yaw_deg: c.yaw_deg,
            pitch_deg: c.pitch_deg,
            roll_deg: c.roll_deg,
            scale: c.scale,
        })
        .collect()
}

/// Converts updated instances into `moveObject` operations.
pub fn move_ops(instances: &[Instance]) -> Vec<Op> {
    instances
        .iter()
        .map(|i| Op::MoveObject {
            id: i.id.clone(),
            position: i.position,
            yaw_deg: i.yaw_deg,
            pitch_deg: i.pitch_deg,
            roll_deg: i.roll_deg,
            scale: i.scale,
        })
        .collect()
}

/// Converts new instances into `placeObject` operations.
pub fn place_ops(instances: &[Instance]) -> Vec<Op> {
    instances
        .iter()
        .map(|i| Op::PlaceObject {
            id: i.id.clone(),
            object: i.object.clone(),
            position: i.position,
            yaw_deg: i.yaw_deg,
            pitch_deg: i.pitch_deg,
            roll_deg: i.roll_deg,
            scale: i.scale,
        })
        .collect()
}
