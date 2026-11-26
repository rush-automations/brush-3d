//! Binary glTF 2.0 (`.glb`) reader and writer: +y up, meters, vertex colours.

use crate::{bad, IoError, ReadReport};
use brush_mesh::Mesh;
use serde_json::{json, Value};

const GLB_MAGIC: u32 = 0x4654_6c67; // "glTF"
const CHUNK_JSON: u32 = 0x4e4f_534a; // "JSON"
const CHUNK_BIN: u32 = 0x004e_4942; // "BIN\0"

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
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((28 + json_len + total) as u32).to_le_bytes());
    out.extend_from_slice(&(json_len as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.resize(20 + json_len, b' ');
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    for (i, p) in parts.iter().enumerate() {
        out.resize(28 + json_len + offsets[i], 0);
        out.extend_from_slice(p);
    }
    out.resize(28 + json_len + total, 0);
    out
}

/// Reads a binary glTF 2.0 (`.glb`) container into a [`Mesh`], baking node transforms and vertex/material colours.
pub fn read_glb(bytes: &[u8]) -> Result<(Mesh, ReadReport), IoError> {
    if bytes.len() < 12 {
        return Err(IoError::Truncated);
    }
    let magic = read_u32(bytes, 0)?;
    if magic != GLB_MAGIC {
        return Err(IoError::NotFormat("GLB"));
    }
    let version = read_u32(bytes, 4)?;
    if version != 2 {
        return Err(bad(format!("unsupported GLB version {version}")));
    }
    let total_len = read_u32(bytes, 8)? as usize;
    if total_len < 12 || total_len > bytes.len() {
        return Err(IoError::Truncated);
    }

    let mut json_slice: Option<&[u8]> = None;
    let mut bin_slice: &[u8] = &[];
    let mut offset = 12usize;
    while offset + 8 <= total_len {
        let chunk_len = read_u32(bytes, offset)? as usize;
        let chunk_type = read_u32(bytes, offset + 4)?;
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(chunk_len)
            .ok_or(IoError::Truncated)?;
        if data_end > total_len {
            return Err(IoError::Truncated);
        }
        match chunk_type {
            CHUNK_JSON if json_slice.is_none() => json_slice = Some(&bytes[data_start..data_end]),
            CHUNK_BIN if bin_slice.is_empty() => bin_slice = &bytes[data_start..data_end],
            _ => {}
        }
        offset = data_end;
    }

    let json_bytes = json_slice.ok_or_else(|| bad("GLB is missing its JSON chunk"))?;
    let root: Value =
        serde_json::from_slice(json_bytes).map_err(|e| bad(format!("invalid GLB JSON: {e}")))?;

    let mut positions = Vec::<f32>::new();
    let mut colors = Vec::<u8>::new();
    let mut indices = Vec::<u32>::new();
    let mut report = ReadReport::default();

    let ctx = GlbContext {
        root: &root,
        bin: bin_slice,
    };

    let nodes = root.get("nodes").and_then(Value::as_array);
    let scenes = root.get("scenes").and_then(Value::as_array);
    let scene_idx = root.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;

    let mut visited_any_node = false;
    if let (Some(scenes_arr), Some(nodes_arr)) = (scenes, nodes) {
        if let Some(scene_nodes) = scenes_arr
            .get(scene_idx)
            .and_then(|s| s.get("nodes"))
            .and_then(Value::as_array)
        {
            for n in scene_nodes {
                if let Some(idx) = n.as_u64().map(|v| v as usize) {
                    visited_any_node = true;
                    ctx.walk_node(
                        nodes_arr,
                        idx,
                        IDENTITY_MAT4,
                        0,
                        &mut positions,
                        &mut colors,
                        &mut indices,
                        &mut report,
                    )?;
                }
            }
        }
    }

    if !visited_any_node {
        if let Some(meshes) = root.get("meshes").and_then(Value::as_array) {
            for mesh_idx in 0..meshes.len() {
                ctx.append_mesh(
                    mesh_idx,
                    IDENTITY_MAT4,
                    &mut positions,
                    &mut colors,
                    &mut indices,
                    &mut report,
                )?;
            }
        }
    }

    if positions.is_empty() {
        return Err(bad("GLB file contains no triangle mesh geometry"));
    }

    let mesh = Mesh::new(positions, colors, indices)?;
    Ok((mesh, report))
}

struct GlbContext<'a> {
    root: &'a Value,
    bin: &'a [u8],
}

impl<'a> GlbContext<'a> {
    #[allow(clippy::too_many_arguments)]
    fn walk_node(
        &self,
        nodes: &[Value],
        node_idx: usize,
        parent: [f64; 16],
        depth: usize,
        positions: &mut Vec<f32>,
        colors: &mut Vec<u8>,
        indices: &mut Vec<u32>,
        report: &mut ReadReport,
    ) -> Result<(), IoError> {
        if depth > 64 {
            return Err(bad("GLB node hierarchy exceeds maximum nesting depth"));
        }
        let Some(node) = nodes.get(node_idx) else {
            return Ok(());
        };
        let world = mat4_mul(parent, node_local_matrix(node));
        if let Some(mesh_idx) = node.get("mesh").and_then(Value::as_u64) {
            self.append_mesh(mesh_idx as usize, world, positions, colors, indices, report)?;
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                if let Some(child_idx) = child.as_u64() {
                    self.walk_node(
                        nodes,
                        child_idx as usize,
                        world,
                        depth + 1,
                        positions,
                        colors,
                        indices,
                        report,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn append_mesh(
        &self,
        mesh_idx: usize,
        world: [f64; 16],
        positions: &mut Vec<f32>,
        colors: &mut Vec<u8>,
        indices: &mut Vec<u32>,
        report: &mut ReadReport,
    ) -> Result<(), IoError> {
        let Some(mesh) = self
            .root
            .get("meshes")
            .and_then(Value::as_array)
            .and_then(|arr| arr.get(mesh_idx))
        else {
            return Ok(());
        };
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            return Ok(());
        };
        let flip_winding = mat4_det3(world) < 0.0;

        for prim in primitives {
            if prim
                .get("extensions")
                .and_then(|e| e.get("KHR_draco_mesh_compression"))
                .is_some()
            {
                return Err(bad(
                    "Draco-compressed GLB meshes are not supported yet; export an uncompressed GLB, PLY, or OBJ",
                ));
            }
            let mode = prim.get("mode").and_then(Value::as_u64).unwrap_or(4);
            if mode != 4 && mode != 5 && mode != 6 {
                report
                    .elements_dropped
                    .push(format!("primitive_mode_{mode}"));
                continue;
            }
            let Some(attrs) = prim.get("attributes") else {
                continue;
            };
            let Some(pos_acc_idx) = attrs.get("POSITION").and_then(Value::as_u64) else {
                continue;
            };

            let raw_positions = self.read_vec3_f32(pos_acc_idx as usize)?;
            let local_v = raw_positions.len();
            if local_v == 0 {
                continue;
            }

            let base_vertex = u32::try_from(positions.len() / 3)
                .map_err(|_| bad("GLB mesh exceeds maximum vertex count"))?;

            let mut valid_vertex = vec![true; local_v];
            positions.reserve(local_v * 3);
            for (i, p) in raw_positions.iter().enumerate() {
                let tp = transform_point(&world, *p);
                if tp[0].is_finite() && tp[1].is_finite() && tp[2].is_finite() {
                    positions.extend_from_slice(&tp);
                } else {
                    valid_vertex[i] = false;
                    positions.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }

            let mat_factor = self.material_base_color_factor(
                prim.get("material")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize),
            );
            let vertex_colors = attrs
                .get("COLOR_0")
                .and_then(Value::as_u64)
                .map(|idx| self.read_colors(idx as usize))
                .transpose()?;

            colors.reserve(local_v * 3);
            match vertex_colors {
                Some(vc) if vc.len() >= local_v => {
                    let factor_is_unit = mat_factor
                        .map(|f| {
                            (f[0] - 1.0).abs() < 1e-6
                                && (f[1] - 1.0).abs() < 1e-6
                                && (f[2] - 1.0).abs() < 1e-6
                        })
                        .unwrap_or(true);
                    if factor_is_unit {
                        for rgb in vc.iter().take(local_v) {
                            colors.extend_from_slice(rgb);
                        }
                    } else {
                        let f = mat_factor.unwrap_or([1.0, 1.0, 1.0]);
                        for rgb in vc.iter().take(local_v) {
                            colors.push(scale_u8(rgb[0], f[0]));
                            colors.push(scale_u8(rgb[1], f[1]));
                            colors.push(scale_u8(rgb[2], f[2]));
                        }
                    }
                }
                _ => {
                    let rgb = match mat_factor {
                        Some(f) => [unit_to_u8(f[0]), unit_to_u8(f[1]), unit_to_u8(f[2])],
                        None => [215, 215, 215],
                    };
                    for _ in 0..local_v {
                        colors.extend_from_slice(&rgb);
                    }
                }
            }

            let raw_indices: Vec<u32> = match prim.get("indices").and_then(Value::as_u64) {
                Some(idx) => self.read_indices(idx as usize)?,
                None => (0..local_v as u32).collect(),
            };

            let mut emit_tri = |a: u32, b: u32, c: u32| {
                let (ai, bi, ci) = (a as usize, b as usize, c as usize);
                if ai >= local_v
                    || bi >= local_v
                    || ci >= local_v
                    || !valid_vertex[ai]
                    || !valid_vertex[bi]
                    || !valid_vertex[ci]
                {
                    report.faces_dropped += 1;
                    return;
                }
                if flip_winding {
                    indices.push(base_vertex + a);
                    indices.push(base_vertex + c);
                    indices.push(base_vertex + b);
                } else {
                    indices.push(base_vertex + a);
                    indices.push(base_vertex + b);
                    indices.push(base_vertex + c);
                }
            };

            match mode {
                4 => {
                    for tri in raw_indices.chunks_exact(3) {
                        emit_tri(tri[0], tri[1], tri[2]);
                    }
                }
                5 => {
                    for i in 0..raw_indices.len().saturating_sub(2) {
                        if i % 2 == 0 {
                            emit_tri(raw_indices[i], raw_indices[i + 1], raw_indices[i + 2]);
                        } else {
                            emit_tri(raw_indices[i + 1], raw_indices[i], raw_indices[i + 2]);
                        }
                    }
                }
                6 => {
                    for i in 1..raw_indices.len().saturating_sub(1) {
                        emit_tri(raw_indices[0], raw_indices[i], raw_indices[i + 1]);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn material_base_color_factor(&self, material_idx: Option<usize>) -> Option<[f32; 3]> {
        let mat = self
            .root
            .get("materials")
            .and_then(Value::as_array)?
            .get(material_idx?)?;
        let pbr = mat.get("pbrMetallicRoughness")?;
        let arr = pbr.get("baseColorFactor").and_then(Value::as_array)?;
        if arr.len() < 3 {
            return None;
        }
        let r = arr[0].as_f64()? as f32;
        let g = arr[1].as_f64()? as f32;
        let b = arr[2].as_f64()? as f32;
        if r.is_finite() && g.is_finite() && b.is_finite() {
            Some([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])
        } else {
            None
        }
    }

    fn accessor_layout(&self, accessor_idx: usize) -> Result<AccessorLayout, IoError> {
        let acc = self
            .root
            .get("accessors")
            .and_then(Value::as_array)
            .and_then(|a| a.get(accessor_idx))
            .ok_or_else(|| bad(format!("GLB accessor {accessor_idx} is missing")))?;
        let bv_idx = acc
            .get("bufferView")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad(format!("GLB accessor {accessor_idx} has no bufferView")))?
            as usize;
        let bv = self
            .root
            .get("bufferViews")
            .and_then(Value::as_array)
            .and_then(|a| a.get(bv_idx))
            .ok_or_else(|| bad(format!("GLB bufferView {bv_idx} is missing")))?;

        let component_type =
            acc.get("componentType")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("GLB accessor missing componentType"))? as u32;
        let count = acc
            .get("count")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad("GLB accessor missing count"))? as usize;
        let ty = acc
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("GLB accessor missing type"))?;
        let components = match ty {
            "SCALAR" => 1usize,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            other => return Err(bad(format!("unsupported GLB accessor type {other}"))),
        };
        let comp_bytes = match component_type {
            5120 | 5121 => 1usize,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            other => return Err(bad(format!("unsupported GLB componentType {other}"))),
        };
        let elem_bytes = components * comp_bytes;
        let bv_offset = bv.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let bv_length =
            bv.get("byteLength")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("GLB bufferView missing byteLength"))? as usize;
        let bv_end = bv_offset.checked_add(bv_length).ok_or(IoError::Truncated)?;
        if bv_end > self.bin.len() {
            return Err(IoError::Truncated);
        }
        let acc_offset = acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let start = bv_offset
            .checked_add(acc_offset)
            .ok_or(IoError::Truncated)?;
        let stride = bv
            .get("byteStride")
            .and_then(Value::as_u64)
            .map(|s| s as usize)
            .unwrap_or(elem_bytes);
        if stride < elem_bytes {
            return Err(bad(
                "GLB bufferView byteStride is smaller than element size",
            ));
        }
        if count > 0 {
            let last_end = start
                .checked_add((count - 1).checked_mul(stride).ok_or(IoError::Truncated)?)
                .and_then(|v| v.checked_add(elem_bytes))
                .ok_or(IoError::Truncated)?;
            if last_end > bv_end {
                return Err(IoError::Truncated);
            }
        }

        Ok(AccessorLayout {
            start,
            stride,
            count,
            component_type,
            components,
        })
    }

    fn read_vec3_f32(&self, accessor_idx: usize) -> Result<Vec<[f32; 3]>, IoError> {
        let lay = self.accessor_layout(accessor_idx)?;
        if lay.component_type != 5126 || lay.components != 3 {
            return Err(bad("GLB POSITION accessor must be FLOAT VEC3"));
        }
        let mut out = Vec::with_capacity(lay.count);
        for i in 0..lay.count {
            let o = lay.start + i * lay.stride;
            out.push([
                read_f32(self.bin, o)?,
                read_f32(self.bin, o + 4)?,
                read_f32(self.bin, o + 8)?,
            ]);
        }
        Ok(out)
    }

    fn read_indices(&self, accessor_idx: usize) -> Result<Vec<u32>, IoError> {
        let lay = self.accessor_layout(accessor_idx)?;
        if lay.components != 1 {
            return Err(bad("GLB indices accessor must be SCALAR"));
        }
        let mut out = Vec::with_capacity(lay.count);
        for i in 0..lay.count {
            let o = lay.start + i * lay.stride;
            let val = match lay.component_type {
                5121 => u32::from(self.bin[o]),
                5123 => u32::from(read_u16(self.bin, o)?),
                5125 => read_u32(self.bin, o)?,
                other => return Err(bad(format!("unsupported GLB index componentType {other}"))),
            };
            out.push(val);
        }
        Ok(out)
    }

    fn read_colors(&self, accessor_idx: usize) -> Result<Vec<[u8; 3]>, IoError> {
        let lay = self.accessor_layout(accessor_idx)?;
        if lay.components < 3 {
            return Err(bad("GLB COLOR_0 accessor must be VEC3 or VEC4"));
        }
        let mut out = Vec::with_capacity(lay.count);
        for i in 0..lay.count {
            let o = lay.start + i * lay.stride;
            let rgb = match lay.component_type {
                5121 => [self.bin[o], self.bin[o + 1], self.bin[o + 2]],
                5123 => [
                    (read_u16(self.bin, o)? >> 8) as u8,
                    (read_u16(self.bin, o + 2)? >> 8) as u8,
                    (read_u16(self.bin, o + 4)? >> 8) as u8,
                ],
                5126 => [
                    unit_to_u8(read_f32(self.bin, o)?),
                    unit_to_u8(read_f32(self.bin, o + 4)?),
                    unit_to_u8(read_f32(self.bin, o + 8)?),
                ],
                _ => [215, 215, 215],
            };
            out.push(rgb);
        }
        Ok(out)
    }
}

struct AccessorLayout {
    start: usize,
    stride: usize,
    count: usize,
    component_type: u32,
    components: usize,
}

const IDENTITY_MAT4: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

fn node_local_matrix(node: &Value) -> [f64; 16] {
    if let Some(m) = node.get("matrix").and_then(Value::as_array) {
        if m.len() == 16 {
            let mut out = IDENTITY_MAT4;
            let mut all_finite = true;
            for (i, v) in m.iter().enumerate() {
                if let Some(f) = v.as_f64() {
                    if f.is_finite() {
                        out[i] = f;
                    } else {
                        all_finite = false;
                        break;
                    }
                } else {
                    all_finite = false;
                    break;
                }
            }
            if all_finite {
                return out;
            }
        }
    }

    let t = read_json_vec3(node.get("translation"), [0.0, 0.0, 0.0]);
    let s = read_json_vec3(node.get("scale"), [1.0, 1.0, 1.0]);
    let q = read_json_vec4(node.get("rotation"), [0.0, 0.0, 0.0, 1.0]);

    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;

    // Column-major T * R * S
    [
        (1.0 - (yy + zz)) * s[0],
        (xy + wz) * s[0],
        (xz - wy) * s[0],
        0.0,
        (xy - wz) * s[1],
        (1.0 - (xx + zz)) * s[1],
        (yz + wx) * s[1],
        0.0,
        (xz + wy) * s[2],
        (yz - wx) * s[2],
        (1.0 - (xx + yy)) * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

fn read_json_vec3(val: Option<&Value>, default: [f64; 3]) -> [f64; 3] {
    let Some(arr) = val.and_then(Value::as_array) else {
        return default;
    };
    if arr.len() < 3 {
        return default;
    }
    let a = arr[0].as_f64().unwrap_or(default[0]);
    let b = arr[1].as_f64().unwrap_or(default[1]);
    let c = arr[2].as_f64().unwrap_or(default[2]);
    if a.is_finite() && b.is_finite() && c.is_finite() {
        [a, b, c]
    } else {
        default
    }
}

fn read_json_vec4(val: Option<&Value>, default: [f64; 4]) -> [f64; 4] {
    let Some(arr) = val.and_then(Value::as_array) else {
        return default;
    };
    if arr.len() < 4 {
        return default;
    }
    let a = arr[0].as_f64().unwrap_or(default[0]);
    let b = arr[1].as_f64().unwrap_or(default[1]);
    let c = arr[2].as_f64().unwrap_or(default[2]);
    let d = arr[3].as_f64().unwrap_or(default[3]);
    if a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite() {
        [a, b, c, d]
    } else {
        default
    }
}

fn mat4_mul(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    out
}

fn mat4_det3(m: [f64; 16]) -> f64 {
    m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2])
        + m[8] * (m[1] * m[6] - m[5] * m[2])
}

fn transform_point(m: &[f64; 16], p: [f32; 3]) -> [f32; 3] {
    let x = f64::from(p[0]);
    let y = f64::from(p[1]);
    let z = f64::from(p[2]);
    [
        (m[0] * x + m[4] * y + m[8] * z + m[12]) as f32,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) as f32,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) as f32,
    ]
}

fn unit_to_u8(v: f32) -> u8 {
    if !v.is_finite() {
        return 215;
    }
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn scale_u8(c: u8, factor: f32) -> u8 {
    if !factor.is_finite() {
        return c;
    }
    (f32::from(c) * factor.clamp(0.0, 1.0))
        .round()
        .clamp(0.0, 255.0) as u8
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, IoError> {
    let slice = bytes.get(offset..offset + 2).ok_or(IoError::Truncated)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, IoError> {
    let slice = bytes.get(offset..offset + 4).ok_or(IoError::Truncated)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32, IoError> {
    let slice = bytes.get(offset..offset + 4).ok_or(IoError::Truncated)?;
    Ok(f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
