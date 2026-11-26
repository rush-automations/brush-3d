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
fn glb_has_a_valid_container_and_round_trips() {
    let m = tri();
    let g = write_glb(&m, "surface");
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

    let (back, report) = read(&g, Format::Glb).unwrap();
    assert_eq!(back.positions, m.positions);
    assert_eq!(back.colors, m.colors);
    assert_eq!(back.indices, m.indices);
    assert_eq!(report, ReadReport::default());

    // Truncated and invalid GLBs are rejected cleanly.
    assert!(read_glb(&g[..10]).is_err());
    assert!(read_glb(&g[..g.len() - 4]).is_err());
    assert!(read_glb(b"not a glb container").is_err());
}

#[test]
fn glb_applies_node_transforms_and_rejects_draco() {
    let positions: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
    let mut bin = positions.clone();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let idx_offset = bin.len();
    bin.extend_from_slice(&indices);
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    let make_glb = |doc: serde_json::Value| -> Vec<u8> {
        let mut json_bytes = serde_json::to_vec(&doc).unwrap();
        while json_bytes.len() % 4 != 0 {
            json_bytes.push(b' ');
        }
        let total = 28 + json_bytes.len() + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004e_4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    };

    let transformed = make_glb(serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"translation": [2.0, 3.0, 4.0], "scale": [2.0, 2.0, 2.0], "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.5, 0.0, 1.0]}}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"},
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": positions.len()},
            {"buffer": 0, "byteOffset": idx_offset, "byteLength": indices.len()},
        ],
        "buffers": [{"byteLength": bin.len()}],
    }));

    let (mesh, _) = read_glb(&transformed).unwrap();
    assert_eq!(mesh.vert(0), [2.0, 3.0, 4.0]);
    assert_eq!(mesh.vert(1), [4.0, 3.0, 4.0]);
    assert_eq!(mesh.vert(2), [2.0, 5.0, 4.0]);
    assert_eq!(mesh.color(0), [255, 128, 0]);

    let draco = make_glb(serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0},
            "extensions": {"KHR_draco_mesh_compression": {"bufferView": 0}}
        }]}],
    }));
    let err = read_glb(&draco).unwrap_err().to_string();
    assert!(err.contains("Draco"));
}

#[test]
fn project_container_round_trips_meshes_objects_history_and_design_intent() {
    use std::collections::BTreeMap;

    let mut surface = tri();
    surface
        .set_attribute("semantic", ScalarKind::U8, vec![1.0, 2.0, 3.0])
        .unwrap();
    let mut objects = BTreeMap::new();
    objects.insert("scan:chair".to_string(), tri());

    let proj = ProjectContainer {
        version: PROJECT_VERSION,
        ops_version: "0.5.0".to_string(),
        revision: 7,
        surface,
        original_surface: Some(Mesh::default()),
        scene_json: r#"{"instances":[{"id":"o1","object":"scan:chair","position":[1,0,2]}]}"#
            .to_string(),
        original_scene_json: Some(r#"{"instances":[]}"#.to_string()),
        objects,
        journal_jsonl: "{\"event\":\"applied\",\"seq\":1}\n".to_string(),
        metadata: serde_json::json!({"title": "Living Room"}),
        features: vec![serde_json::json!({"kind": "extrude", "height": 2.4})],
    };

    let bytes = write_project(&proj);
    assert!(is_project_container(&bytes));
    assert_eq!(Format::sniff(&bytes), None);

    let back = read_project(&bytes).unwrap();
    assert_eq!(back, proj);

    // Truncation and unsupported versions are rejected cleanly.
    assert!(read_project(&bytes[..10]).is_err());
    assert!(read_project(&bytes[..bytes.len() - 5]).is_err());
    let mut future = bytes.clone();
    future[8..12].copy_from_slice(&99u32.to_le_bytes());
    assert!(read_project(&future).is_err());
}
