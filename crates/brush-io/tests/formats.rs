use brush_io::*;
use brush_mesh::{Mesh, ScalarKind};

fn tri() -> Mesh {
    Mesh::new(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255],
        vec![0, 1, 2],
    )
    .unwrap()
}

#[test]
fn binary_ply_round_trips_and_matches_the_worker_format() {
    let m = tri();
    let bytes = write_ply(&m, PlyOptions::default());
    let head = "ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";
    assert!(bytes.starts_with(head.as_bytes()));
    assert_eq!(bytes.len(), head.len() + 3 * 15 + 13);
    let (back, report) = read_ply(&bytes).unwrap();
    assert_eq!(back, m);
    assert_eq!(report, ReadReport::default());
}

#[test]
fn extra_vertex_properties_survive_with_their_types() {
    let mut m = tri();
    m.set_attribute("label", ScalarKind::U16, vec![1.0, 2.0, 65535.0])
        .unwrap();
    m.set_attribute("confidence", ScalarKind::F32, vec![0.25, 0.5, 1.0])
        .unwrap();
    let (back, _) = read_ply(&write_ply(&m, PlyOptions::default())).unwrap();
    assert_eq!(back, m);
    let (plain, _) = read_ply(&write_ply(
        &m,
        PlyOptions {
            geometry_only: true,
        },
    ))
    .unwrap();
    assert!(plain.attributes.is_empty());
}

#[test]
fn ascii_and_big_endian_ply_with_quads_and_float_colours() {
    let ascii = b"ply\nformat ascii 1.0\ncomment from somewhere\nelement vertex 4\nproperty double x\nproperty double y\nproperty double z\nproperty float red\nproperty float green\nproperty float blue\nproperty float nx\nelement face 1\nproperty uchar flags\nproperty list uchar uint vertex_indices\nelement edge 0\nproperty int vertex1\nend_header\n0 0 0 1 0 0 0.5\n1 0 0 0 1 0 0.5\n1 0 1 0 0 1 0.5\n0 0 1 1 1 1 0.5\n7 4 0 1 2 3\n";
    let (m, report) = read_ply(ascii).unwrap();
    assert_eq!((m.vertex_count(), m.face_count()), (4, 2));
    assert_eq!(m.color(3), [255, 255, 255]);
    assert_eq!(m.attribute("nx").unwrap().values, vec![0.5; 4]);
    assert_eq!(report.polygons_triangulated, 1);
    assert_eq!(report.face_properties_dropped, vec!["flags".to_string()]);
    assert_eq!(report.elements_dropped, vec!["edge".to_string()]);

    let mut be = b"ply\r\nformat binary_big_endian 1.0\r\nelement vertex 3\r\nproperty float x\r\nproperty float y\r\nproperty float z\r\nelement face 1\r\nproperty list uchar int vertex_indices\r\nend_header\r\n".to_vec();
    for v in [0f32, 0., 0., 2., 0., 0., 0., 0., 2.] {
        be.extend_from_slice(&v.to_be_bytes());
    }
    be.push(3);
    for i in [0i32, 1, 2] {
        be.extend_from_slice(&i.to_be_bytes());
    }
    let (m, _) = read_ply(&be).unwrap();
    assert_eq!(m.vert(1), [2.0, 0.0, 0.0]);
    assert_eq!(m.indices, vec![0, 1, 2]);
}

#[test]
fn a_point_cloud_reads_as_a_mesh_without_faces() {
    let p = b"ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n1 1 1\n";
    let (m, _) = read_ply(p).unwrap();
    assert_eq!((m.vertex_count(), m.face_count()), (2, 0));
}

#[test]
fn broken_files_are_refused() {
    assert!(read_ply(b"not a ply").is_err());
    let mut bytes = write_ply(&tri(), PlyOptions::default());
    bytes.truncate(bytes.len() - 4);
    assert!(read_ply(&bytes).is_err());
    let bad_index = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n3 0 0 5\n";
    let (m, report) = read_ply(bad_index).unwrap();
    assert_eq!((m.face_count(), report.faces_dropped), (0, 1));
}

#[test]
fn obj_round_trips_colours_and_handles_polygons_and_negative_indices() {
    let m = tri();
    let (back, _) = read_obj(write_obj(&m).as_bytes()).unwrap();
    assert_eq!(back.positions, m.positions);
    assert_eq!(back.colors, m.colors);
    assert_eq!(back.indices, m.indices);
    let quad = b"v 0 0 0\nv 1 0 0\nv 1 0 1\nv 0 0 1\nvt 0 0\nf 1/1 2/1 -2/1 -1/1\n";
    let (q, report) = read_obj(quad).unwrap();
    assert_eq!(q.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(report.polygons_triangulated, 1);
}

#[test]
fn glb_has_a_valid_container() {
    let g = write_glb(&tri(), "surface");
    assert_eq!(&g[0..4], b"glTF");
    assert_eq!(
        u32::from_le_bytes(g[8..12].try_into().unwrap()) as usize,
        g.len()
    );
    assert_eq!(g.len() % 4, 0);
    let json_len = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&g[20..20 + json_len]).unwrap();
    assert_eq!(doc["accessors"][2]["count"], 3);
    assert_eq!(Format::sniff(&g), Some(Format::Glb));
}
