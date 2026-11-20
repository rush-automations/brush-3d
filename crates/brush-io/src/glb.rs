//! A single-mesh binary glTF with vertex colours, +y up, meters: what Blender, three.js and every viewer open.

use brush_mesh::Mesh;
use serde_json::json;

const fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

pub fn write_glb(mesh: &Mesh, name: &str) -> Vec<u8> {
    let v = mesh.vertex_count();
    let f = mesh.face_count();
    let (min, max) = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    let positions: Vec<u8> = mesh
        .positions
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .collect();
    let colors: Vec<u8> = mesh
        .colors
        .chunks_exact(3)
        .flat_map(|c| [c[0], c[1], c[2], 255])
        .collect();
    let indices: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let parts = [&positions, &colors, &indices];
    let mut offsets = Vec::new();
    let mut total = 0;
    for p in parts {
        offsets.push(total);
        total += pad4(p.len());
    }
    let doc = json!({
        "asset": {"version": "2.0", "generator": "brush-engine"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": [{"attributes": {"POSITION": 0, "COLOR_0": 1}, "indices": 2, "material": 0, "mode": 4}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1}, "doubleSided": true}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": v, "type": "VEC3", "min": min, "max": max},
            {"bufferView": 1, "componentType": 5121, "normalized": true, "count": v, "type": "VEC4"},
            {"bufferView": 2, "componentType": 5125, "count": f * 3, "type": "SCALAR"},
        ],
        "bufferViews": parts.iter().enumerate().map(|(i, p)| json!({
            "buffer": 0, "byteOffset": offsets[i], "byteLength": p.len(), "target": if i == 2 { 34963 } else { 34962 },
        })).collect::<Vec<_>>(),
        "buffers": [{"byteLength": total}],
    });
    let json_bytes = serde_json::to_vec(&doc).expect("json");
    let json_len = pad4(json_bytes.len());
    let mut out = Vec::with_capacity(28 + json_len + total);
    out.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((28 + json_len + total) as u32).to_le_bytes());
    out.extend_from_slice(&(json_len as u32).to_le_bytes());
    out.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.resize(20 + json_len, b' ');
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    for (i, p) in parts.iter().enumerate() {
        out.resize(28 + json_len + offsets[i], 0);
        out.extend_from_slice(p);
    }
    out.resize(28 + json_len + total, 0);
    out
}
