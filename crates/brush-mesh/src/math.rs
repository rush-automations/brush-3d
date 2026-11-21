//! Small f32 vector helpers. Transcendentals go through `libm` so every target (native, wasm, mobile) computes
//! bit-identical results: an op applied anywhere gives the same mesh.

pub type Vec3 = [f32; 3];

#[inline]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn norm(a: Vec3) -> Vec3 {
    let l = sqrt(dot(a, a));
    if l > 1e-12 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}
#[inline]
pub fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}
#[inline]
pub fn floor(x: f32) -> f32 {
    libm::floorf(x)
}
#[inline]
pub fn ceil(x: f32) -> f32 {
    libm::ceilf(x)
}
/// Rounds half away from zero.
#[inline]
pub fn round(x: f32) -> f32 {
    libm::roundf(x)
}
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A colour channel from a float: rounded, then clamped to 0..255.
#[inline]
pub fn q8(v: f32) -> u8 {
    round(v).clamp(0.0, 255.0) as u8
}

/// Into a box's frame: translated to its centre, turned by -yaw about +y.
#[inline]
pub fn to_local(p: Vec3, c: Vec3, yaw: f32) -> Vec3 {
    let (co, si) = (cos(yaw), sin(yaw));
    let d = sub(p, c);
    [co * d[0] - si * d[2], d[1], si * d[0] + co * d[2]]
}

/// Out of a box's frame: the inverse of [`to_local`].
#[inline]
pub fn to_world(l: Vec3, c: Vec3, yaw: f32) -> Vec3 {
    let (co, si) = (cos(yaw), sin(yaw));
    [
        c[0] + co * l[0] + si * l[2],
        c[1] + l[1],
        c[2] - si * l[0] + co * l[2],
    ]
}

/// Integer grid cell of a coordinate. Saturates like a float-to-int cast.
#[inline]
pub fn cell(v: f32, size: f32) -> i32 {
    floor(v / size) as i32
}
