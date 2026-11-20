//! Wavefront OBJ with the common `v x y z r g b` vertex-colour extension (Blender 3.x+, MeshLab).

use crate::{bad, IoError, ReadReport};
use brush_mesh::Mesh;

pub fn read_obj(bytes: &[u8]) -> Result<(Mesh, ReadReport), IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotFormat("OBJ"))?;
    let mut mesh = Mesh::default();
    let mut report = ReadReport::default();
    let mut corner: Vec<i64> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut w = line.split_ascii_whitespace();
        match w.next() {
            Some("v") => {
                let nums: Vec<f64> = w
                    .map(|t| {
                        t.parse::<f64>()
                            .map_err(|_| bad(format!("line {}: bad number {t}", n + 1)))
                    })
                    .collect::<Result<_, _>>()?;
                if nums.len() < 3 {
                    return Err(bad(format!("line {}: a vertex needs x, y and z", n + 1)));
                }
                mesh.positions.extend(nums[..3].iter().map(|&v| v as f32));
                if nums.len() >= 6 {
                    // Colours are 0..1 by convention; some exporters write 0..255.
                    let scale = if nums[3..6].iter().any(|&c| c > 1.0) {
                        1.0
                    } else {
                        255.0
                    };
                    mesh.colors.extend(
                        nums[3..6]
                            .iter()
                            .map(|&c| (c * scale).round().clamp(0.0, 255.0) as u8),
                    );
                } else {
                    mesh.colors.extend_from_slice(&[200, 200, 200]);
                }
            }
            Some("f") => {
                let v = mesh.vertex_count() as i64;
                corner.clear();
                for t in w {
                    let i: i64 = t
                        .split('/')
                        .next()
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| bad(format!("line {}: bad face index {t}", n + 1)))?;
                    corner.push(if i < 0 { v + i } else { i - 1 });
                }
                if corner.len() < 3 || corner.iter().any(|&i| i < 0 || i >= v) {
                    report.faces_dropped += 1;
                    continue;
                }
                if corner.len() > 3 {
                    report.polygons_triangulated += 1;
                }
                for j in 1..corner.len() - 1 {
                    mesh.indices.extend_from_slice(&[
                        corner[0] as u32,
                        corner[j] as u32,
                        corner[j + 1] as u32,
                    ]);
                }
            }
            _ => {}
        }
    }
    if mesh.vertex_count() == 0 {
        return Err(IoError::NotFormat("OBJ"));
    }
    mesh.validate()?;
    Ok((mesh, report))
}

pub fn write_obj(mesh: &Mesh) -> String {
    let mut s = String::with_capacity(mesh.vertex_count() * 48 + mesh.face_count() * 24);
    s.push_str("# brush surface mesh, meters, +y up\n");
    for i in 0..mesh.vertex_count() {
        let p = mesh.vert(i as u32);
        let c = mesh.color(i as u32);
        s.push_str(&format!(
            "v {} {} {} {:.4} {:.4} {:.4}\n",
            p[0],
            p[1],
            p[2],
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0
        ));
    }
    for t in 0..mesh.face_count() {
        let [a, b, c] = mesh.face(t);
        s.push_str(&format!("f {} {} {}\n", a + 1, b + 1, c + 1));
    }
    s
}
