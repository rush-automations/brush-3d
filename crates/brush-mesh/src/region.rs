//! Where an edit applies. The public form is a box (centre, full size, yaw in degrees about +y); internally a box is
//! its centre, half sizes and yaw in radians, and a negative half size means the whole mesh.

use crate::math::{smoothstep, to_local, Vec3};
use serde::{Deserialize, Serialize};

/// An oriented box: `center` and full `size` in meters, turned by `yawDeg` degrees about +y.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxRegion {
    pub center: [f64; 3],
    pub size: [f64; 3],
    #[serde(default, skip_serializing_if = "is_zero")]
    pub yaw_deg: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A box in kernel form. Conversions from [`BoxRegion`] are done in f64 and rounded once to f32, so a box means
/// exactly the same thing whether it came from JSON, JavaScript or Python.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub c: Vec3,
    pub h: Vec3,
    pub yaw: f32,
}

impl Area {
    pub const ALL: Area = Area {
        c: [0.0; 3],
        h: [-1.0, 0.0, 0.0],
        yaw: 0.0,
    };

    pub fn is_all(&self) -> bool {
        self.h[0] < 0.0
    }

    /// Strictly-inclusive containment test used by remove, paint and smooth.
    pub fn contains(&self, p: Vec3) -> bool {
        if self.is_all() {
            return true;
        }
        let l = to_local(p, self.c, self.yaw);
        l[0].abs() <= self.h[0] && l[1].abs() <= self.h[1] && l[2].abs() <= self.h[2]
    }

    /// True when any point of the segment `a..b` lies inside the oriented box (symmetric in `a` and `b`).
    pub fn intersects_segment(&self, a: Vec3, b: Vec3) -> bool {
        if self.is_all() {
            return true;
        }
        let la = to_local(a, self.c, self.yaw);
        let lb = to_local(b, self.c, self.yaw);
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        for axis in 0..3 {
            let p0 = la[axis];
            let d = lb[axis] - p0;
            let h = self.h[axis];
            if d.abs() < 1e-8 {
                if p0.abs() > h {
                    return false;
                }
            } else {
                let inv = 1.0 / d;
                let mut near = (-h - p0) * inv;
                let mut far = (h - p0) * inv;
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                }
                t0 = t0.max(near);
                t1 = t1.min(far);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }

    /// 1 inside, 0 outside, easing over `edge` meters at the boundary when edge > 0. The whole mesh weighs 1.
    pub fn weight(&self, p: Vec3, edge: f32) -> f32 {
        if self.is_all() {
            return 1.0;
        }
        let l = to_local(p, self.c, self.yaw);
        let mut m: f32 = 1.0;
        for a in 0..3 {
            let inside = self.h[a] - l[a].abs();
            if inside < 0.0 {
                return 0.0;
            }
            if edge > 0.0 {
                m = m.min(smoothstep((inside / edge).min(1.0)));
            }
        }
        m
    }
}

pub fn rad(deg: f64) -> f32 {
    (deg * std::f64::consts::PI / 180.0) as f32
}

impl std::convert::From<&BoxRegion> for Area {
    fn from(b: &BoxRegion) -> Area {
        Area {
            c: [b.center[0] as f32, b.center[1] as f32, b.center[2] as f32],
            h: [
                (b.size[0] / 2.0) as f32,
                (b.size[1] / 2.0) as f32,
                (b.size[2] / 2.0) as f32,
            ],
            yaw: rad(b.yaw_deg),
        }
    }
}

/// The whole mesh when no box is given.
pub fn area(b: Option<&BoxRegion>) -> Area {
    b.map_or(Area::ALL, Area::from)
}
