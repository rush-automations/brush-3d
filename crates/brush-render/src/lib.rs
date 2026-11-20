//! # brush-render
//!
//! The camera model shared by every view of a mesh, and a software rasteriser: flat shading from a headlight plus a
//! fixed key light, z-buffered, vertex colours, near-plane clipping so huge triangles behind the eye are cut rather
//! than dropped. It is deterministic, so a server rendering a view for an AI model sees exactly what a client does.

use brush_mesh::math::{cross, dot, norm, sub, tan, Vec3};
use brush_mesh::Mesh;
use serde::{Deserialize, Serialize};

/// A pinhole camera looking from `eye` to `target` with +y up (no roll) and a vertical field of view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    #[serde(default = "sixty")]
    pub fov_deg: f64,
}

fn sixty() -> f64 {
    60.0
}

pub const BACKGROUND: [u8; 3] = [240, 240, 242];
const NEAR: f32 = 0.05;

#[derive(Clone, Copy)]
struct CamV {
    p: Vec3,
    c: [f32; 3],
}

/// Renders `mesh` into a `width` x `height` RGBA buffer, row by row from the top.
pub fn render(mesh: &Mesh, width: u32, height: u32, cam: &Camera) -> Vec<u8> {
    let (wi, hi) = (width as usize, height as usize);
    let mut frame = Vec::with_capacity(wi * hi * 4);
    for _ in 0..wi * hi {
        frame.extend_from_slice(&[BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]);
    }
    if wi == 0 || hi == 0 {
        return frame;
    }
    let mut depth = vec![f32::INFINITY; wi * hi];
    let eye = cam.eye.map(|v| v as f32);
    let target = cam.target.map(|v| v as f32);
    let fwd = norm(sub(target, eye));
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    let f = 1.0 / tan(cam.fov_deg as f32 * std::f32::consts::PI / 360.0);
    let aspect = width as f32 / height as f32;
    let key = norm([0.4, 1.0, 0.3]);
    for t in 0..mesh.face_count() {
        let ids = mesh.face(t);
        let cam_v = ids.map(|vi| {
            let d = sub(mesh.vert(vi), eye);
            let c = mesh.color(vi);
            CamV {
                p: [dot(d, right), dot(d, up), dot(d, fwd)],
                c: [c[0] as f32, c[1] as f32, c[2] as f32],
            }
        });
        let n = norm(cross(
            sub(mesh.vert(ids[1]), mesh.vert(ids[0])),
            sub(mesh.vert(ids[2]), mesh.vert(ids[0])),
        ));
        let face_n = if dot(n, fwd) > 0.0 {
            [-n[0], -n[1], -n[2]]
        } else {
            n
        };
        let light = 0.35 + 0.45 * (-dot(face_n, fwd)).max(0.0) + 0.2 * dot(face_n, key).max(0.0);
        // Clip against the near plane.
        let mut poly = [CamV {
            p: [0.0; 3],
            c: [0.0; 3],
        }; 4];
        let mut np = 0;
        for j in 0..3 {
            let (a, b) = (cam_v[j], cam_v[(j + 1) % 3]);
            let (ain, bin) = (a.p[2] >= NEAR, b.p[2] >= NEAR);
            if ain {
                poly[np] = a;
                np += 1;
            }
            if ain != bin {
                let u = (NEAR - a.p[2]) / (b.p[2] - a.p[2]);
                poly[np] = CamV {
                    p: [
                        a.p[0] + (b.p[0] - a.p[0]) * u,
                        a.p[1] + (b.p[1] - a.p[1]) * u,
                        NEAR,
                    ],
                    c: [
                        a.c[0] + (b.c[0] - a.c[0]) * u,
                        a.c[1] + (b.c[1] - a.c[1]) * u,
                        a.c[2] + (b.c[2] - a.c[2]) * u,
                    ],
                };
                np += 1;
            }
        }
        if np < 3 {
            continue;
        }
        for i in 1..np - 1 {
            raster_tri(
                wi,
                hi,
                &mut depth,
                &mut frame,
                [poly[0], poly[i], poly[i + 1]],
                light,
                f,
                aspect,
            );
        }
    }
    frame
}

/// Fills one camera-space triangle, interpolating depth and colour perspective-correctly.
#[allow(clippy::too_many_arguments)]
fn raster_tri(
    wi: usize,
    hi: usize,
    depth: &mut [f32],
    frame: &mut [u8],
    v: [CamV; 3],
    light: f32,
    f: f32,
    aspect: f32,
) {
    let (wf, hf) = (wi as f32, hi as f32);
    let mut sx = [0f32; 3];
    let mut sy = [0f32; 3];
    let mut iz = [0f32; 3];
    for j in 0..3 {
        let z = v[j].p[2];
        sx[j] = (v[j].p[0] * f / (z * aspect) * 0.5 + 0.5) * wf;
        sy[j] = (0.5 - v[j].p[1] * f / z * 0.5) * hf;
        iz[j] = 1.0 / z;
    }
    let area = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (sy[1] - sy[0]);
    if area.abs() < 1e-6 {
        return;
    }
    let fl = |x: f32| brush_mesh::math::floor(x) as i32;
    let ce = |x: f32| brush_mesh::math::ceil(x) as i32;
    let minx = fl(sx[0].min(sx[1].min(sx[2]))).max(0);
    let maxx = ce(sx[0].max(sx[1].max(sx[2]))).min(wi as i32 - 1);
    let miny = fl(sy[0].min(sy[1].min(sy[2]))).max(0);
    let maxy = ce(sy[0].max(sy[1].max(sy[2]))).min(hi as i32 - 1);
    for py in miny..=maxy {
        for px in minx..=maxx {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let w0 = ((sx[1] - fx) * (sy[2] - fy) - (sx[2] - fx) * (sy[1] - fy)) / area;
            let w1 = ((sx[2] - fx) * (sy[0] - fy) - (sx[0] - fx) * (sy[2] - fy)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let inv = w0 * iz[0] + w1 * iz[1] + w2 * iz[2];
            let z = 1.0 / inv;
            let o = py as usize * wi + px as usize;
            if z >= depth[o] {
                continue;
            }
            depth[o] = z;
            for j in 0..3 {
                let c = (w0 * iz[0] * v[0].c[j] + w1 * iz[1] * v[1].c[j] + w2 * iz[2] * v[2].c[j])
                    / inv;
                frame[o * 4 + j] = (c * light).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brush_mesh::op::Context;
    use brush_mesh::Op;

    #[test]
    fn draws_the_mesh_in_front_of_the_background() {
        let mut m = Mesh::default();
        let o: Op = serde_json::from_str(
            r#"{"op":"addBox","box":{"center":[0,1,0],"size":[2,2,2]},"color":[255,0,0]}"#,
        )
        .unwrap();
        o.apply(&mut m, Context::default()).unwrap();
        let px = render(
            &m,
            64,
            64,
            &Camera {
                eye: [0.0, 1.0, 6.0],
                target: [0.0, 1.0, 0.0],
                fov_deg: 60.0,
            },
        );
        let mid = (32 * 64 + 32) * 4;
        assert!(px[mid] > px[mid + 1]);
        assert_eq!(px[3], 255);
    }

    #[test]
    fn keeps_a_huge_floor_visible_with_corners_behind_the_camera() {
        let m = Mesh::new(
            vec![
                -100., 0., -100., 100., 0., -100., 100., 0., 100., -100., 0., 100.,
            ],
            vec![120; 12],
            vec![0, 1, 2, 0, 2, 3],
        )
        .unwrap();
        let px = render(
            &m,
            64,
            64,
            &Camera {
                eye: [0.0, 1.6, 0.0],
                target: [0.0, 1.0, 5.0],
                fov_deg: 60.0,
            },
        );
        assert!(px[(60 * 64 + 32) * 4] < 200);
        assert_eq!(px[(2 * 64 + 32) * 4], 240);
        let down = render(
            &m,
            64,
            64,
            &Camera {
                eye: [0.0, 3.0, 0.0],
                target: [0.0, 0.0, 0.01],
                fov_deg: 60.0,
            },
        );
        assert!(down[(32 * 64 + 32) * 4] < 200);
    }
}
