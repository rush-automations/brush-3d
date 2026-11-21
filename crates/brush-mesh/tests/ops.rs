//! Behaviour of every op, driven through the JSON form an app or AI model sends.
#![allow(clippy::needless_range_loop)]

use brush_mesh::op::Context;
use brush_mesh::{Mesh, Op, ScalarKind};
use serde_json::json;

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).expect("valid op")
}
fn run(m: &mut Mesh, v: serde_json::Value) -> u64 {
    op(v)
        .apply(m, Context::default())
        .expect("op applies")
        .count
}

/// A flat n x n grid of unit quads in the xz plane at y = 0, grey 128.
fn grid(n: u32) -> Mesh {
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            positions.extend_from_slice(&[i as f32, 0.0, j as f32]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            let (b, c) = (a + 1, a + n + 1);
            indices.extend_from_slice(&[a, c, b, b, c, c + 1]);
        }
    }
    let v = positions.len() / 3;
    Mesh::new(positions, vec![128; v * 3], indices).unwrap()
}

/// A 4 x 4 floor of just two triangles.
fn floor4() -> Mesh {
    Mesh::new(
        vec![0., 0., 0., 4., 0., 0., 4., 0., 4., 0., 0., 4.],
        vec![120; 12],
        vec![0, 1, 2, 0, 2, 3],
    )
    .unwrap()
}

fn area(m: &Mesh) -> f64 {
    (0..m.face_count())
        .map(|t| {
            let [a, b, c] = m.face(t).map(|i| m.vert(i).map(f64::from));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() / 2.0
        })
        .sum()
}

#[test]
fn a_face_pointing_at_a_missing_vertex_is_refused() {
    assert!(Mesh::new(vec![0.0; 9], vec![0; 9], vec![0, 1, 99]).is_err());
}

#[test]
fn remove_box_deletes_faces_and_orphaned_vertices() {
    let mut m = grid(4);
    let removed = run(
        &mut m,
        json!({"op": "removeBox", "box": {"center": [2, 0, 2], "size": [2, 1, 2]}}),
    );
    assert!(removed > 0);
    assert_eq!(m.face_count() as u64, 32 - removed);
    assert!(m.vertex_count() < 25);
    let mut m = grid(4);
    let kept = run(
        &mut m,
        json!({"op": "removeBox", "box": {"center": [2, 0, 2], "size": [2, 1, 2]}, "invert": true}),
    );
    assert_eq!(m.face_count() as u64, 32 - kept);
}

#[test]
fn fill_holes_closes_the_hole_remove_made() {
    let mut m = grid(4);
    run(
        &mut m,
        json!({"op": "removeBox", "box": {"center": [1.5, 0, 1.5], "size": [0.7, 1, 0.7]}}),
    );
    let before = m.face_count() as u64;
    let added = run(&mut m, json!({"op": "fillHoles"}));
    assert!(added > 0);
    assert_eq!(m.face_count() as u64, before + added);
    assert_eq!(run(&mut m, json!({"op": "fillHoles"})), 0);
}

#[test]
fn smooth_flattens_a_spike_and_decimate_reduces_faces() {
    let mut m = grid(6);
    m.positions[(3 * 7 + 3) * 3 + 1] = 5.0;
    run(&mut m, json!({"op": "smooth", "iterations": 3}));
    assert!(m.positions[(3 * 7 + 3) * 3 + 1] < 5.0);
    let mut m = grid(8);
    let faces = run(&mut m, json!({"op": "decimate", "cell": 2}));
    assert!(faces > 0 && faces < 128);
}

#[test]
fn add_box_stands_on_the_bottom_of_its_box_and_paint_colours_it() {
    let mut m = Mesh::default();
    assert_eq!(
        run(
            &mut m,
            json!({"op": "addBox", "box": {"center": [0, 1, 0], "size": [1, 2, 1]}, "color": [200, 0, 0]})
        ),
        12
    );
    assert_eq!(m.vertex_count(), 8);
    assert_eq!(m.bounds(), Some(([-0.5, 0.0, -0.5], [0.5, 2.0, 0.5])));
    assert_eq!(
        run(
            &mut m,
            json!({"op": "paint", "box": {"center": [0, 1, 0], "size": [4, 4, 4]}, "color": [0, 0, 255]})
        ),
        8
    );
    assert_eq!(m.color(0), [0, 0, 255]);
}

#[test]
fn add_frustum_makes_a_closed_cylinder_and_cone() {
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addFrustum", "base": [1, 0, 1], "radiusBottom": 0.5, "radiusTop": 0.5, "height": 2, "segments": 8}),
    );
    assert_eq!(m.face_count(), 32);
    assert_eq!(m.vertex_count(), 18);
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addFrustum", "base": [0, 0, 0], "radiusBottom": 1, "radiusTop": 0, "height": 1, "segments": 2}),
    );
    assert_eq!(m.face_count(), 12); // too few segments is raised to 3
}

#[test]
fn carve_box_cuts_an_exact_hole_in_huge_triangles() {
    let mut m = Mesh::new(
        vec![0., 0., 0., 4., 0., 0., 4., 0., 4., 0., 0., 4.],
        vec![255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255],
        vec![0, 1, 2, 0, 2, 3],
    )
    .unwrap();
    let unit = json!({"center": [2, 0, 2], "size": [1, 1, 1]});
    assert_eq!(run(&mut m, json!({"op": "removeBox", "box": unit})), 0);
    assert_eq!(run(&mut m, json!({"op": "carveBox", "box": unit})), 2);
    assert!((area(&m) - 15.0).abs() < 1e-4);
    for p in m.positions.chunks_exact(3) {
        assert!(!((p[0] - 2.0).abs() < 0.4999 && (p[2] - 2.0).abs() < 0.4999));
    }
    assert_eq!(m.bounds(), Some(([0.0; 3], [4.0, 0.0, 4.0])));
    assert_eq!(
        run(
            &mut m,
            json!({"op": "carveBox", "box": {"center": [20, 0, 20], "size": [1, 1, 1]}})
        ),
        0
    );
    assert!(
        run(
            &mut m,
            json!({"op": "carveBox", "box": {"center": [1, 0, 1], "size": [0.5, 1, 0.5], "yawDeg": 45}})
        ) > 0
    );
}

#[test]
fn subdivide_splits_long_edges_without_changing_the_shape() {
    let mut m = floor4();
    let faces = run(&mut m, json!({"op": "subdivide", "maxEdge": 1}));
    assert!(faces > 32);
    assert!((area(&m) - 16.0).abs() < 1e-3);
    let longest = (0..m.face_count())
        .flat_map(|t| {
            let f = m.face(t);
            (0..3).map(move |k| (f[k], f[(k + 1) % 3]))
        })
        .map(|(a, b)| {
            let (p, q) = (m.vert(a), m.vert(b));
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
        })
        .fold(0f32, f32::max);
    assert!(longest <= 1.0001);
    assert_eq!(
        run(&mut m, json!({"op": "subdivide", "maxEdge": 10})),
        faces
    );
    let mut m = floor4();
    let capped = run(
        &mut m,
        json!({"op": "subdivide", "maxEdge": 0.01, "maxFaces": 3000, "passes": 20}),
    );
    assert!(capped <= 3000 && capped > 100);
}

#[test]
fn roughen_is_reproducible_and_moves_a_floor_only_vertically() {
    let fresh = || {
        let mut m = floor4();
        run(&mut m, json!({"op": "subdivide", "maxEdge": 0.25}));
        m
    };
    let before = fresh();
    let mut a = fresh();
    let moved = run(
        &mut a,
        json!({"op": "roughen", "amplitude": 0.05, "scale": 0.5, "seed": 3}),
    );
    assert_eq!(moved as usize, before.vertex_count());
    let (mut max_y, mut max_xz) = (0f32, 0f32);
    for (p, q) in a
        .positions
        .chunks_exact(3)
        .zip(before.positions.chunks_exact(3))
    {
        max_y = max_y.max(p[1].abs());
        max_xz = max_xz.max((p[0] - q[0]).abs()).max((p[2] - q[2]).abs());
    }
    assert!(max_y > 0.005 && max_y <= 0.0501);
    assert!(max_xz < 1e-5);
    let mut b = fresh();
    run(
        &mut b,
        json!({"op": "roughen", "amplitude": 0.05, "scale": 0.5, "seed": 3}),
    );
    assert_eq!(a.content_hash(), b.content_hash());
    let mut c = fresh();
    let some = run(
        &mut c,
        json!({"op": "roughen", "box": {"center": [1, 0, 1], "size": [2, 2, 2]}, "amplitude": 0.05, "scale": 0.5}),
    );
    assert!(some > 0 && (some as usize) < before.vertex_count());
}

#[test]
fn tint_and_shade_change_colour_only() {
    let mut m = floor4();
    run(&mut m, json!({"op": "subdivide", "maxEdge": 0.25}));
    let g0 = m.positions.clone();
    let mut t = m.clone();
    run(
        &mut t,
        json!({"op": "tint", "a": [200, 0, 0], "b": [0, 0, 200], "scale": 1, "strength": 1}),
    );
    let reds: std::collections::BTreeSet<u8> = t.colors.chunks_exact(3).map(|c| c[0]).collect();
    assert!(reds.len() > 5);
    assert_eq!(t.positions, g0);
    run(
        &mut m,
        json!({"op": "tint", "scale": 1, "strength": 0.5, "mode": "shade"}),
    );
    for c in m.colors.chunks_exact(3) {
        assert_eq!(c[0], c[1]);
        assert!(c[0] > 55 && c[0] < 185);
    }
}

#[test]
fn bake_shadows_darkens_a_corner_but_not_an_open_floor() {
    let mut m = floor4();
    run(
        &mut m,
        json!({"op": "addBox", "box": {"center": [2, 1, 4.1], "size": [4, 2, 0.2]}, "color": [120, 120, 120]}),
    );
    run(&mut m, json!({"op": "subdivide", "maxEdge": 0.5}));
    run(
        &mut m,
        json!({"op": "bakeShadows", "strength": 0.8, "radius": 1.5, "cell": 0.1, "samples": 32}),
    );
    let (mut corner, mut open) = (255u8, 0u8);
    for i in 0..m.vertex_count() {
        let [x, y, z] = m.vert(i as u32);
        if y == 0.0 && z > 3.7 && z < 4.0 && x > 1.0 && x < 3.0 {
            corner = corner.min(m.colors[i * 3]);
        }
        if y == 0.0 && z < 1.0 && x > 1.0 && x < 3.0 {
            open = open.max(m.colors[i * 3]);
        }
    }
    assert!(open > corner + 15, "open {open}, corner {corner}");
}

#[test]
fn match_color_from_surroundings_and_from_a_reference() {
    let red_box = json!({"op": "addBox", "box": {"center": [2, 1, 2], "size": [1, 2, 1]}, "color": [255, 0, 0]});
    let region = json!({"center": [2, 1, 2], "size": [1.4, 2.4, 1.4]});
    let mut m = floor4();
    run(&mut m, json!({"op": "subdivide", "maxEdge": 1}));
    run(&mut m, red_box.clone());
    run(
        &mut m,
        json!({"op": "matchColor", "box": region, "maxDist": 3}),
    );
    assert!(m
        .colors
        .chunks_exact(3)
        .all(|c| !(c[0] == 255 && c[1] == 0)));

    let mut m = floor4();
    run(&mut m, red_box);
    let reference = Mesh::new(vec![2.0, 1.0, 2.0], vec![10, 200, 30], vec![]).unwrap();
    let o = op(json!({"op": "matchColor", "box": region, "maxDist": 3, "fromReference": true}));
    assert_eq!(
        o.apply(&mut m.clone(), Context::default()),
        Err(brush_mesh::OpError::NoReference)
    );
    assert!(
        o.apply(
            &mut m,
            Context {
                reference: Some(&reference),
                ..Default::default()
            }
        )
        .unwrap()
        .count
            > 0
    );
    assert_eq!(m.color(4), [10, 200, 30]);
}

#[test]
fn clone_box_duplicates_geometry_turned_and_mirrored() {
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addBox", "box": {"center": [0, 1, 0], "size": [1, 2, 1]}}),
    );
    let faces = m.face_count() as u64;
    let region = json!({"center": [0.5, 1, 0.5], "size": [3, 3, 3]});
    assert_eq!(
        run(
            &mut m,
            json!({"op": "cloneBox", "box": region, "offset": [10, 0, 0]})
        ),
        faces
    );
    assert!((m.bounds().unwrap().1[0] - 10.5).abs() < 1e-4);
    let before = m.face_count() as u64;
    run(
        &mut m,
        json!({"op": "cloneBox", "box": region, "offset": [0, 0, 10], "rotateDeg": 90, "mirrorX": true}),
    );
    assert_eq!(m.face_count() as u64, before + faces);
    assert!(m.bounds().unwrap().1[2] > 9.9);
}

#[test]
fn sphere_lathe_and_extrude_have_the_right_size() {
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addSphere", "center": [0, 1, 0], "radii": [1, 1, 1]}),
    );
    let a = area(&m);
    assert!(a > 11.5 && a < 4.0 * std::f64::consts::PI + 0.01);
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addLathe", "base": [0, 0, 0], "profile": [[0.6, 0], [0.6, 0.3], [0.4, 0.4], [0.4, 3], [0.55, 3.2], [0.7, 3.4]], "segments": 24}),
    );
    let (_, hi) = m.bounds().unwrap();
    assert!((hi[1] - 3.4).abs() < 1e-4 && (hi[0] - 0.7).abs() < 0.05);
    let mut l = Mesh::default();
    run(
        &mut l,
        json!({"op": "addExtrude", "polygon": [[0, 0], [4, 0], [4, 1], [1, 1], [1, 3], [0, 3]], "y0": 0, "height": 2}),
    );
    assert_eq!(l.bounds(), Some(([0.0; 3], [4.0, 2.0, 3.0])));
    let mut top = l.clone();
    top.indices = (0..l.face_count())
        .filter(|&t| l.face(t).iter().all(|&i| l.vert(i)[1] == 2.0))
        .flat_map(|t| l.face(t))
        .collect();
    assert!((area(&top) - 6.0).abs() < 1e-4);
    let mut cw = Mesh::default();
    run(
        &mut cw,
        json!({"op": "addExtrude", "polygon": [[0, 0], [0, 3], [1, 3], [1, 1], [4, 1], [4, 0]], "y0": 0, "height": 2}),
    );
    assert_eq!(cw.face_count(), l.face_count());
}

#[test]
fn attributes_follow_every_edit() {
    let mut m = grid(4);
    let n = m.vertex_count();
    m.set_attribute(
        "confidence",
        ScalarKind::F32,
        (0..n).map(|i| i as f64).collect(),
    )
    .unwrap();
    m.set_attribute("label", ScalarKind::U8, vec![7.0; n])
        .unwrap();
    for o in [
        json!({"op": "carveBox", "box": {"center": [2, 0, 2], "size": [1.3, 1, 1.3]}}),
        json!({"op": "fillHoles"}),
        json!({"op": "subdivide", "maxEdge": 0.6}),
        json!({"op": "cloneBox", "box": {"center": [1, 0, 1], "size": [2, 1, 2]}, "offset": [0, 1, 0]}),
        json!({"op": "decimate", "cell": 0.5}),
        json!({"op": "addSphere", "center": [0, 3, 0], "radii": [0.5, 0.5, 0.5]}),
    ] {
        op(o).apply(&mut m, Context::default()).unwrap();
        m.validate().unwrap();
    }
    let label = m.attribute("label").unwrap();
    assert!(label.values.iter().all(|&v| v == 7.0 || v == 0.0)); // labels are copied, never averaged; new shapes get 0
    assert!(label.values.contains(&7.0));
}

#[test]
fn bad_values_are_refused_before_anything_changes() {
    let mut m = grid(2);
    let h = m.content_hash();
    let e = op(json!({"op": "addLathe", "base": [0, 0, 0], "profile": [[1, 0]]}))
        .apply(&mut m, Context::default());
    assert!(e.is_err());
    let e = op(json!({"op": "paint", "box": {"center": [0, 0, 0], "size": [-1, 1, 1]}, "color": [1, 2, 3]})).apply(&mut m, Context::default());
    assert!(e.is_err());
    assert_eq!(m.content_hash(), h);
    assert!(serde_json::from_value::<Op>(json!({"op": "paint", "box": {"center": [0, 0, 0], "size": [1, 1, 1]}, "color": [1, 2, 3], "colour": 1})).is_err());
    assert!(serde_json::from_value::<Op>(json!({"op": "explode"})).is_err());
}

#[test]
fn ops_round_trip_through_json_with_defaults_filled_in() {
    let o = op(json!({"op": "smooth"}));
    let back = serde_json::to_value(&o).unwrap();
    assert_eq!(
        back,
        json!({"op": "smooth", "iterations": 1, "strength": 0.5, "method": "taubin", "keepBoundary": true})
    );
    assert_eq!(op(back), o);
}

// ---- the improved and new algorithms ----

fn sphere(seg: u32, rings: u32) -> Mesh {
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op": "addSphere", "center": [0, 0, 0], "radii": [1, 1, 1], "segments": seg, "rings": rings}),
    );
    m
}

fn mean_radius(m: &Mesh) -> f64 {
    m.positions
        .chunks_exact(3)
        .map(|p| ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]) as f64).sqrt())
        .sum::<f64>()
        / m.vertex_count() as f64
}

fn open_edges(m: &Mesh) -> usize {
    brush_mesh::topo::edges(m)
        .iter()
        .filter(|e| e.2 == 1)
        .count()
}

#[test]
fn taubin_smoothing_keeps_volume_that_laplacian_loses() {
    let mut rough = sphere(48, 24);
    run(
        &mut rough,
        json!({"op": "roughen", "amplitude": 0.05, "scale": 0.15, "seed": 2}),
    );
    let (mut t, mut l) = (rough.clone(), rough.clone());
    run(&mut t, json!({"op": "smooth", "iterations": 10}));
    run(
        &mut l,
        json!({"op": "smooth", "iterations": 10, "method": "laplacian"}),
    );
    let (rt, rl) = (mean_radius(&t), mean_radius(&l));
    assert!((rt - 1.0).abs() < 0.02, "taubin radius {rt}");
    assert!(
        rl < rt - 0.02,
        "laplacian {rl} should shrink more than taubin {rt}"
    );
}

#[test]
fn smoothing_keeps_boundary_vertices_in_place() {
    let mut m = grid(6);
    for p in m.positions.chunks_exact_mut(3) {
        p[1] = ((p[0] * 3.1).sin() * (p[2] * 2.3).cos()) * 0.2;
    }
    let b = brush_mesh::topo::boundary_vertices(&m);
    let before = m.clone();
    run(&mut m, json!({"op": "smooth", "iterations": 5}));
    for i in 0..m.vertex_count() {
        if b[i] {
            assert_eq!(m.vert(i as u32), before.vert(i as u32));
        }
    }
    assert_ne!(m.positions, before.positions);
}

#[test]
fn simplify_reduces_a_sphere_without_tearing_or_flipping_it() {
    let mut m = sphere(64, 32);
    let f0 = m.face_count() as u64;
    let left = run(&mut m, json!({"op": "simplify", "targetRatio": 0.1}));
    assert!(left <= f0 / 10 + 1 && left > f0 / 20, "{left} of {f0}");
    assert_eq!(open_edges(&m), 0, "the sphere stays closed");
    for t in 0..m.face_count() {
        let n = brush_mesh::topo::face_cross(&m, t);
        let [a, b, c] = m.face(t).map(|i| m.vert(i));
        let centre = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        assert!(brush_mesh::math::dot(n, centre) > 0.0, "face {t} flipped");
    }
    let r = mean_radius(&m);
    assert!((r - 1.0).abs() < 0.03, "shape kept: mean radius {r}");
}

#[test]
fn simplify_keeps_the_outline_of_an_open_surface() {
    let mut m = grid(16);
    for p in m.positions.chunks_exact_mut(3) {
        p[1] = (p[0] * 0.4).sin() * 0.5;
    }
    let bounds = m.bounds().unwrap();
    run(&mut m, json!({"op": "simplify", "targetFaces": 120}));
    assert!(m.face_count() <= 120, "{} faces", m.face_count());
    let after = m.bounds().unwrap();
    for k in [0, 2] {
        assert!((after.0[k] - bounds.0[k]).abs() < 1e-3 && (after.1[k] - bounds.1[k]).abs() < 1e-3);
    }
    let b = brush_mesh::topo::boundary_vertices(&m);
    for i in (0..m.vertex_count()).filter(|&i| b[i]) {
        let [x, _, z] = m.vert(i as u32);
        let on = [x, z]
            .iter()
            .any(|&v| v.abs() < 1e-3 || (v - 16.0).abs() < 1e-3);
        assert!(on, "outline vertex {x},{z} moved inward");
    }
}

#[test]
fn simplify_carries_colours_and_attributes() {
    let mut m = sphere(32, 16);
    for i in 0..m.vertex_count() {
        let y = m.vert(i as u32)[1];
        m.colors[i * 3] = if y > 0.0 { 250 } else { 10 };
    }
    let n = m.vertex_count();
    let labels: Vec<f64> = (0..n)
        .map(|i| if m.vert(i as u32)[1] > 0.0 { 1.0 } else { 2.0 })
        .collect();
    m.set_attribute("label", ScalarKind::U8, labels).unwrap();
    run(&mut m, json!({"op": "simplify", "targetRatio": 0.3}));
    m.validate().unwrap();
    let labels = &m.attribute("label").unwrap().values;
    assert!(labels.iter().all(|&v| v == 1.0 || v == 2.0));
    let top = (0..m.vertex_count())
        .filter(|&i| m.vert(i as u32)[1] > 0.5)
        .all(|i| m.colors[i * 3] > 200);
    assert!(top, "colours of the top half survive");
}

#[test]
fn fill_holes_closes_a_concave_hole_flat_and_without_new_vertices() {
    let mut m = grid(8);
    // An L-shaped hole from two overlapping boxes.
    run(
        &mut m,
        json!({"op": "removeBox", "box": {"center": [3, 0, 3], "size": [3.2, 1, 1.2]}}),
    );
    run(
        &mut m,
        json!({"op": "removeBox", "box": {"center": [2, 0, 4], "size": [1.2, 1, 3.2]}}),
    );
    let v = m.vertex_count();
    let outer = 4 * 8;
    assert!(open_edges(&m) > outer);
    run(&mut m, json!({"op": "fillHoles", "maxEdges": 31}));
    assert_eq!(m.vertex_count(), v, "no centroid vertex");
    assert_eq!(
        open_edges(&m),
        outer,
        "only the grid's own outline is left open"
    );
    assert!(
        (area(&m) - 64.0).abs() < 1e-3,
        "the patch is flat and exactly covers the hole: {}",
        area(&m)
    );
}

#[test]
fn sculpt_raises_lowers_paints_and_flattens() {
    let mut base = floor4();
    run(&mut base, json!({"op": "subdivide", "maxEdge": 0.1}));
    let dab = json!([{"center": [2, 0, 2]}]);

    let mut up = base.clone();
    let touched = run(
        &mut up,
        json!({"op": "sculpt", "brush": "draw", "dabs": dab, "radius": 0.5, "view": [0, -1, 0]}),
    );
    assert!(touched > 10);
    let lifted = up
        .positions
        .chunks_exact(3)
        .map(|p| p[1])
        .fold(0f32, f32::max);
    assert!(lifted > 0.01, "draw lifts the middle by {lifted}");
    let far = up
        .positions
        .chunks_exact(3)
        .filter(|p| (p[0] - 2.0).abs() > 0.6 || (p[2] - 2.0).abs() > 0.6)
        .all(|p| p[1] == 0.0);
    assert!(far, "nothing outside the radius moves");

    let mut down = base.clone();
    run(
        &mut down,
        json!({"op": "sculpt", "brush": "draw", "dabs": dab, "radius": 0.5, "invert": true, "view": [0, -1, 0]}),
    );
    assert!(
        down.positions
            .chunks_exact(3)
            .map(|p| p[1])
            .fold(0f32, f32::min)
            < -0.01
    );

    let mut paint = base.clone();
    run(
        &mut paint,
        json!({"op": "sculpt", "brush": "paint", "dabs": dab, "radius": 0.5, "strength": 1, "color": [255, 0, 0]}),
    );
    assert!(paint.colors.chunks_exact(3).any(|c| c == [255, 0, 0]));
    assert_eq!(paint.positions, base.positions);

    let mut flat = up.clone();
    for _ in 0..5 {
        run(
            &mut flat,
            json!({"op": "sculpt", "brush": "flatten", "dabs": dab, "radius": 0.8, "strength": 1}),
        );
    }
    let spread = |m: &Mesh| {
        let ys = m.positions.chunks_exact(3).map(|p| p[1]);
        ys.clone().fold(0f32, f32::max) - ys.fold(0f32, f32::min)
    };
    assert!(spread(&flat) < spread(&up) * 0.5);
}

#[test]
fn a_stroke_sent_in_pieces_equals_the_whole_stroke() {
    let mut base = floor4();
    run(&mut base, json!({"op": "subdivide", "maxEdge": 0.1}));
    run(
        &mut base,
        json!({"op": "roughen", "amplitude": 0.03, "scale": 0.2, "seed": 4}),
    );
    let dabs: Vec<serde_json::Value> = (0..20).map(|k| json!({"center": [1.0 + k as f64 * 0.1, 0, 2.0], "pressure": 0.5 + k as f64 * 0.02})).collect();
    for brush in ["draw", "inflate", "smooth", "flatten", "pinch", "paint"] {
        let mut whole = base.clone();
        run(
            &mut whole,
            json!({"op": "sculpt", "brush": brush, "dabs": dabs, "radius": 0.4, "color": [0, 90, 200]}),
        );
        let mut pieces = base.clone();
        for chunk in dabs.chunks(3) {
            run(
                &mut pieces,
                json!({"op": "sculpt", "brush": brush, "dabs": chunk, "radius": 0.4, "color": [0, 90, 200]}),
            );
        }
        assert_eq!(whole.content_hash(), pieces.content_hash(), "{brush}");
        assert_ne!(
            whole.content_hash(),
            base.content_hash(),
            "{brush} did something"
        );
    }
}

#[test]
fn ops_know_whether_they_change_topology() {
    assert!(
        !op(json!({"op": "sculpt", "brush": "draw", "dabs": [], "radius": 1})).changes_topology()
    );
    assert!(!op(json!({"op": "smooth"})).changes_topology());
    assert!(op(json!({"op": "simplify"})).changes_topology());
    assert!(
        op(json!({"op": "carveBox", "box": {"center": [0, 0, 0], "size": [1, 1, 1]}}))
            .changes_topology()
    );
}

#[test]
fn transform_moves_turns_and_scales_about_the_pivot() {
    let mut m = floor4();
    run(
        &mut m,
        json!({"op":"transform","translate":[1,2,3],"yawDeg":90,"scale":2}),
    );
    // yaw 90 about the origin, then translate: (x,z) -> (z,-x), scaled by 2.
    let p = m.vert(0);
    assert!((p[0] - 1.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5 && (p[2] - 3.0).abs() < 1e-5);
    let q = m.vert(1); // was (4,0,0)
    assert!((q[0] - 1.0).abs() < 1e-5 && (q[1] - 2.0).abs() < 1e-5 && (q[2] - (-5.0)).abs() < 1e-5);
    // Region transforms pivot on the box centre and leave the rest alone.
    let mut n = floor4();
    let moved = run(
        &mut n,
        json!({"op":"transform","box":{"center":[0,0,0],"size":[1,1,1]},"translate":[0,1,0]}),
    );
    assert_eq!(moved, 1);
    assert_eq!(n.vert(3), [0.0, 0.0, 4.0]);
    // Bad values are refused without touching the mesh.
    let before = n.clone();
    assert!(op(json!({"op":"transform","scale":0}))
        .apply(&mut n, Context::default())
        .is_err());
    assert_eq!(n, before);
}

#[test]
fn transform_carries_splats() {
    let mut m = floor4();
    run(
        &mut m,
        json!({"op":"splatPaint","dabs":[{"center":[2,0,2]}],"radius":0.5,"size":0.1,"view":[0,-1,0],"seed":3}),
    );
    let n0 = m.splats.len();
    assert!(n0 > 0);
    let c0 = m.splats.center(0);
    run(&mut m, json!({"op":"transform","translate":[10,0,0]}));
    let c1 = m.splats.center(0);
    assert!((c1[0] - (c0[0] + 10.0)).abs() < 1e-4);
    assert_eq!(m.splats.len(), n0);
    // And removeBox takes the paint away with the surface.
    run(
        &mut m,
        json!({"op":"removeBox","box":{"center":[12,0,2],"size":[20,20,20]}}),
    );
    assert_eq!(m.splats.len(), 0);
}

#[test]
fn splat_paint_is_resolution_independent() {
    // The same stroke on a coarse and a fine mesh stamps the same splats where the surfaces agree.
    let mut coarse = grid(2);
    let mut fine = grid(8);
    let stroke = json!({"op":"splatPaint","dabs":[{"center":[1,0,1]}],"radius":0.8,"size":0.1,"seed":9,"view":[0,-1,0]});
    run(&mut coarse, stroke.clone());
    run(&mut fine, stroke);
    assert_eq!(coarse.splats, fine.splats);
    assert_eq!(coarse.face_count(), 8);
    assert_eq!(fine.face_count(), 128);
}

#[test]
fn splat_strokes_stream_like_sculpt_strokes() {
    // A stroke sent in two pieces gives the same splats as the whole stroke at once.
    let whole = json!({"op":"splatPaint","dabs":[{"center":[1,0,1]},{"center":[1.4,0,1.1]}],"radius":0.6,"size":0.1,"seed":4,"view":[0,-1,0]});
    let first = json!({"op":"splatPaint","dabs":[{"center":[1,0,1]}],"radius":0.6,"size":0.1,"seed":4,"view":[0,-1,0]});
    let second = json!({"op":"splatPaint","dabs":[{"center":[1.4,0,1.1]}],"radius":0.6,"size":0.1,"seed":5,"view":[0,-1,0]});
    let mut a = grid(4);
    let mut b = grid(4);
    run(&mut a, whole);
    run(&mut b, first);
    run(&mut b, second);
    assert_eq!(a.splats, b.splats);
}

#[test]
fn splat_ops_validate_and_erase() {
    let mut m = grid(4);
    assert!(
        op(json!({"op":"splatPaint","dabs":[],"radius":0,"size":0.1}))
            .apply(&mut m, Context::default())
            .is_err()
    );
    assert!(op(json!({"op":"splatErase","dabs":[],"radius":-1}))
        .apply(&mut m, Context::default())
        .is_err());
    run(
        &mut m,
        json!({"op":"splatPaint","dabs":[{"center":[1,0,1]}],"radius":0.5,"size":0.1,"seed":2,"view":[0,-1,0]}),
    );
    let n = m.splats.len();
    let gone = run(
        &mut m,
        json!({"op":"splatErase","dabs":[{"center":[1,0,1]}],"radius":2}),
    );
    assert_eq!(gone as usize, n);
    assert!(m.splats.is_empty());
}

#[test]
fn measure_reports_volume_area_and_closure() {
    let mut m = Mesh::default();
    run(
        &mut m,
        json!({"op":"addBox","box":{"center":[0,1,0],"size":[2,2,2]}}),
    );
    let mm = m.measure();
    assert!(mm.closed);
    assert!((mm.volume - 8.0).abs() < 1e-4, "{}", mm.volume);
    assert!((mm.surface_area - 24.0).abs() < 1e-4, "{}", mm.surface_area);
    assert!((mm.centroid[1] - 1.0).abs() < 1e-4);
    assert_eq!(mm.splats, 0);
    // An open sheet has no volume.
    let sheet = grid(2);
    let ms = sheet.measure();
    assert!(!ms.closed);
    assert_eq!(ms.volume, 0.0);
    assert!((ms.surface_area - 4.0).abs() < 1e-4);
}
