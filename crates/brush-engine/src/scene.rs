//! A document's scene: linked instances of other objects placed on its mesh.
//!
//! An instance is a pointer (`"scan:<id>"`) plus where it stands. The meshes behind pointers are supplied by the host
//! with [`crate::Document::set_object`] and are not part of the journal: they are whatever the object is now, so an
//! edit to an object reaches every instance of it in every document. Placing, moving, removing and baking instances
//! are ops, journaled and undoable like any other.

use brush_mesh::{Bvh, Hit, Mesh};
use serde::{Deserialize, Serialize};

/// One placed object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Instance {
    pub id: String,
    /// The object it points at, such as `"scan:<id>"`.
    pub object: String,
    /// Where the object's bounding-box bottom centre stands, meters.
    pub position: [f64; 3],
    #[serde(default)]
    pub yaw_deg: f64,
    #[serde(default = "one")]
    pub scale: f64,
}

fn one() -> f64 {
    1.0
}

/// Every placed object, in the order placed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene {
    pub instances: Vec<Instance>,
}

impl Scene {
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.id == id)
    }

    pub(crate) fn index(&self, id: &str) -> Option<usize> {
        self.instances.iter().position(|i| i.id == id)
    }

    /// The objects the scene points at, each once, in first-placed order.
    pub fn objects(&self) -> Vec<String> {
        let mut seen = Vec::<String>::new();
        for i in &self.instances {
            if !seen.contains(&i.object) {
                seen.push(i.object.clone());
            }
        }
        seen
    }

    /// An id not yet used: `o1`, `o2`, ...
    pub fn free_id(&self) -> String {
        (1..)
            .map(|n| format!("o{n}"))
            .find(|id| self.get(id).is_none())
            .expect("ids run out after usize::MAX")
    }
}

/// The point of an object that lands on an instance's position: its bounding-box bottom centre.
pub fn anchor(mesh: &Mesh) -> [f32; 3] {
    match mesh.bounds() {
        Some((lo, hi)) => [(lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5],
        None => [0.0; 3],
    }
}

/// An object's full size, meters (zero when it has no vertices).
pub fn size(mesh: &Mesh) -> [f32; 3] {
    match mesh.bounds() {
        Some((lo, hi)) => [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        None => [0.0; 3],
    }
}

/// The instance's transform from object to world space, as a column-major 4x4 matrix (Metal, WebGL and three.js
/// order): translate to `position`, turn about +y, scale, after moving the object's anchor to the origin. Matches what
/// baking and flattening produce.
pub fn transform(anchor: [f32; 3], position: [f64; 3], yaw_deg: f64, scale: f64) -> [f32; 16] {
    let yaw = brush_mesh::region::rad(yaw_deg);
    let (c, s, k) = (libm::cosf(yaw), libm::sinf(yaw), scale as f32);
    let p = position.map(|v| v as f32);
    // world = p + R * (k * (x - a)), with R the box yaw: x' = c x + s z, z' = -s x + c z.
    let t = [
        p[0] - k * (c * anchor[0] + s * anchor[2]),
        p[1] - k * anchor[1],
        p[2] - k * (-s * anchor[0] + c * anchor[2]),
    ];
    [
        k * c,
        0.0,
        -k * s,
        0.0, //
        0.0,
        k,
        0.0,
        0.0, //
        k * s,
        0.0,
        k * c,
        0.0, //
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

/// A ray into an instance's object space. Affine maps keep the ray parameter `t`, so hits compare across instances.
fn ray_to_object(
    inst: &Instance,
    anchor: [f32; 3],
    origin: [f32; 3],
    dir: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let yaw = brush_mesh::region::rad(inst.yaw_deg);
    let (c, s, k) = (libm::cosf(yaw), libm::sinf(yaw), inst.scale as f32);
    let p = inst.position.map(|v| v as f32);
    // Inverse of the box yaw: x = c x' - s z', z = s x' + c z'.
    let unrotate = |v: [f32; 3]| [c * v[0] - s * v[2], v[1], s * v[0] + c * v[2]];
    let o = unrotate([origin[0] - p[0], origin[1] - p[1], origin[2] - p[2]]);
    let d = unrotate(dir);
    (
        [
            o[0] / k + anchor[0],
            o[1] / k + anchor[1],
            o[2] / k + anchor[2],
        ],
        [d[0] / k, d[1] / k, d[2] / k],
    )
}

/// What a ray met: the document's own mesh, or a placed instance (by id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    #[serde(flatten)]
    pub hit: Hit,
    /// The instance hit, or none for the document's own mesh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

/// The nearest hit on any instance, given each object's mesh and BVH.
pub(crate) fn pick_instances<'a>(
    scene: &Scene,
    objects: impl Fn(&str) -> Option<(&'a Mesh, &'a Bvh)>,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<Pick> {
    let mut best: Option<Pick> = None;
    for inst in &scene.instances {
        let Some((mesh, bvh)) = objects(&inst.object) else {
            continue;
        };
        let a = anchor(mesh);
        let (o, d) = ray_to_object(inst, a, origin, dir);
        let Some(h) = bvh.raycast(mesh, o, d) else {
            continue;
        };
        if best.as_ref().is_some_and(|b| b.hit.t <= h.t) {
            continue;
        }
        let m = transform(a, inst.position, inst.yaw_deg, inst.scale);
        let n = h.normal;
        // Uniform scale and a turn: the normal turns with the object and stays unit length.
        let normal = [
            m[0] * n[0] + m[8] * n[2],
            m[5] * n[1],
            m[2] * n[0] + m[10] * n[2],
        ]
        .map(|v| v / inst.scale as f32);
        best = Some(Pick {
            hit: Hit {
                face: h.face,
                t: h.t,
                point: [
                    origin[0] + h.t * dir[0],
                    origin[1] + h.t * dir[1],
                    origin[2] + h.t * dir[2],
                ],
                normal,
                barycentric: h.barycentric,
            },
            instance: Some(inst.id.clone()),
        });
    }
    best
}

/// `mesh` with every instance whose object is known copied in: what exports and other tools see.
pub(crate) fn flatten<'a>(
    mesh: &Mesh,
    scene: &Scene,
    objects: impl Fn(&str) -> Option<&'a Mesh>,
) -> Mesh {
    let mut out = mesh.clone();
    for inst in &scene.instances {
        if let Some(obj) = objects(&inst.object) {
            bake(&mut out, obj, inst);
        }
    }
    out
}

/// Copies `obj` into `mesh` where `inst` stands.
pub(crate) fn bake(mesh: &mut Mesh, obj: &Mesh, inst: &Instance) {
    mesh.add_mesh(
        obj,
        inst.position.map(|v| v as f32),
        brush_mesh::region::rad(inst.yaw_deg),
        inst.scale as f32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
        [
            m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
            m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
            m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        ]
    }

    #[test]
    fn transform_matches_baking() {
        let mut obj = Mesh::default();
        obj.add_box([0.3, 0.1, -0.2], 0.5, 0.8, 0.4, 0.0, [10, 20, 30]);
        let inst = Instance {
            id: "a".into(),
            object: "scan:x".into(),
            position: [1.0, 2.0, 3.0],
            yaw_deg: 37.0,
            scale: 1.5,
        };
        let mut baked = Mesh::default();
        bake(&mut baked, &obj, &inst);
        let m = transform(anchor(&obj), inst.position, inst.yaw_deg, inst.scale);
        for (p, q) in obj
            .positions
            .chunks_exact(3)
            .zip(baked.positions.chunks_exact(3))
        {
            let w = apply_point(&m, [p[0], p[1], p[2]]);
            for k in 0..3 {
                assert!((w[k] - q[k]).abs() < 1e-5, "{w:?} vs {q:?}");
            }
        }
    }

    #[test]
    fn rays_round_trip_into_object_space() {
        let mut obj = Mesh::default();
        obj.add_box([0.0, 0.0, 0.0], 1.0, 1.0, 1.0, 0.0, [1, 2, 3]);
        let bvh = Bvh::build(&obj);
        let scene = Scene {
            instances: vec![Instance {
                id: "a".into(),
                object: "o".into(),
                position: [5.0, 0.0, 0.0],
                yaw_deg: 90.0,
                scale: 2.0,
            }],
        };
        // From above, straight down onto the instance's top (2 m tall after scaling).
        let pick = pick_instances(
            &scene,
            |_| Some((&obj, &bvh)),
            [5.2, 10.0, 0.3],
            [0.0, -1.0, 0.0],
        )
        .unwrap();
        assert_eq!(pick.instance.as_deref(), Some("a"));
        assert!(
            (pick.hit.point[1] - 2.0).abs() < 1e-4,
            "{:?}",
            pick.hit.point
        );
        assert!((pick.hit.t - 8.0).abs() < 1e-4);
        assert!(pick.hit.normal[1] > 0.99);
        assert!(pick_instances(
            &scene,
            |_| Some((&obj, &bvh)),
            [0.0, 10.0, 0.0],
            [0.0, -1.0, 0.0]
        )
        .is_none());
    }
}
