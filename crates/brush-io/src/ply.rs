//! PLY: ascii, binary little- and big-endian; any scalar vertex properties; faces as a vertex index list of any
//! integer type, polygons fanned into triangles.

use crate::{bad, IoError, ReadReport};
use brush_mesh::{Attribute, Mesh, ScalarKind};

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Ascii,
    Le,
    Be,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "char" | "int8" => Ty::I8,
            "uchar" | "uint8" => Ty::U8,
            "short" | "int16" => Ty::I16,
            "ushort" | "uint16" => Ty::U16,
            "int" | "int32" => Ty::I32,
            "uint" | "uint32" => Ty::U32,
            "float" | "float32" => Ty::F32,
            "double" | "float64" => Ty::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }
    fn kind(self) -> ScalarKind {
        match self {
            Ty::I8 => ScalarKind::I8,
            Ty::U8 => ScalarKind::U8,
            Ty::I16 => ScalarKind::I16,
            Ty::U16 => ScalarKind::U16,
            Ty::I32 => ScalarKind::I32,
            Ty::U32 => ScalarKind::U32,
            Ty::F32 => ScalarKind::F32,
            Ty::F64 => ScalarKind::F64,
        }
    }
    fn is_float(self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
}

fn ply_name(k: ScalarKind) -> &'static str {
    match k {
        ScalarKind::I8 => "char",
        ScalarKind::U8 => "uchar",
        ScalarKind::I16 => "short",
        ScalarKind::U16 => "ushort",
        ScalarKind::I32 => "int",
        ScalarKind::U32 => "uint",
        ScalarKind::F32 => "float",
        ScalarKind::F64 => "double",
    }
}

enum Prop {
    Scalar { name: String, ty: Ty },
    List { name: String, count: Ty, item: Ty },
}

struct Element {
    name: String,
    count: usize,
    props: Vec<Prop>,
}

struct Reader<'a> {
    enc: Encoding,
    bytes: &'a [u8],
    pos: usize,
    tokens: std::iter::Peekable<std::str::SplitAsciiWhitespace<'a>>,
}

impl<'a> Reader<'a> {
    fn value(&mut self, ty: Ty) -> Result<f64, IoError> {
        if self.enc == Encoding::Ascii {
            let t = self.tokens.next().ok_or(IoError::Truncated)?;
            return t
                .parse::<f64>()
                .map_err(|_| bad(format!("not a number: {t}")));
        }
        let n = ty.size();
        let b = self
            .bytes
            .get(self.pos..self.pos + n)
            .ok_or(IoError::Truncated)?;
        self.pos += n;
        let mut a = [0u8; 8];
        a[..n].copy_from_slice(b);
        if self.enc == Encoding::Be {
            a[..n].reverse();
        }
        Ok(match ty {
            Ty::I8 => a[0] as i8 as f64,
            Ty::U8 => a[0] as f64,
            Ty::I16 => i16::from_le_bytes([a[0], a[1]]) as f64,
            Ty::U16 => u16::from_le_bytes([a[0], a[1]]) as f64,
            Ty::I32 => i32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::U32 => u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::F32 => f32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::F64 => f64::from_le_bytes(a),
        })
    }
}

const COLOR_NAMES: [[&str; 2]; 3] = [
    ["red", "diffuse_red"],
    ["green", "diffuse_green"],
    ["blue", "diffuse_blue"],
];

pub fn read_ply(bytes: &[u8]) -> Result<(Mesh, ReadReport), IoError> {
    if !bytes.starts_with(b"ply") {
        return Err(IoError::NotFormat("PLY"));
    }
    let end = find(bytes, b"end_header").ok_or_else(|| bad("PLY header has no end_header"))?;
    let mut body = end + b"end_header".len();
    if bytes.get(body) == Some(&b'\r') {
        body += 1;
    }
    if bytes.get(body) == Some(&b'\n') {
        body += 1;
    }
    let header = std::str::from_utf8(&bytes[..end]).map_err(|_| bad("PLY header is not text"))?;
    let mut enc = None;
    let mut elements: Vec<Element> = Vec::new();
    for line in header.lines().map(str::trim) {
        let w: Vec<&str> = line.split_ascii_whitespace().collect();
        match w.as_slice() {
            ["format", f, _] => {
                enc = Some(match *f {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Le,
                    "binary_big_endian" => Encoding::Be,
                    _ => return Err(bad(format!("unknown PLY format {f}"))),
                })
            }
            ["element", name, count] => elements.push(Element {
                name: (*name).to_owned(),
                count: count.parse().map_err(|_| bad("bad element count"))?,
                props: Vec::new(),
            }),
            ["property", "list", c, i, name] => {
                let el = elements
                    .last_mut()
                    .ok_or_else(|| bad("property before element"))?;
                let count = Ty::parse(c).ok_or_else(|| bad(format!("unknown type {c}")))?;
                let item = Ty::parse(i).ok_or_else(|| bad(format!("unknown type {i}")))?;
                el.props.push(Prop::List {
                    name: (*name).to_owned(),
                    count,
                    item,
                });
            }
            ["property", t, name] => {
                let el = elements
                    .last_mut()
                    .ok_or_else(|| bad("property before element"))?;
                let ty = Ty::parse(t).ok_or_else(|| bad(format!("unknown type {t}")))?;
                el.props.push(Prop::Scalar {
                    name: (*name).to_owned(),
                    ty,
                });
            }
            _ => {}
        }
    }
    let enc = enc.ok_or_else(|| bad("PLY header has no format line"))?;
    let text = if enc == Encoding::Ascii {
        std::str::from_utf8(&bytes[body..]).map_err(|_| bad("ascii PLY body is not text"))?
    } else {
        ""
    };
    let mut r = Reader {
        enc,
        bytes,
        pos: body,
        tokens: text.split_ascii_whitespace().peekable(),
    };
    let mut report = ReadReport::default();
    let mut mesh = Mesh::default();
    let mut have_vertex = false;
    for el in &elements {
        match el.name.as_str() {
            "vertex" if !have_vertex => {
                have_vertex = true;
                read_vertices(&mut r, el, &mut mesh)?;
            }
            "face" => read_faces(&mut r, el, &mut mesh, &mut report)?,
            "splat" => read_splats(&mut r, el, &mut mesh)?,
            _ => {
                report.elements_dropped.push(el.name.clone());
                skip(&mut r, el)?;
            }
        }
    }
    mesh.validate()?;
    Ok((mesh, report))
}

fn read_vertices(r: &mut Reader, el: &Element, mesh: &mut Mesh) -> Result<(), IoError> {
    let n = el.count;
    // Where each property lands: 0..3 position, 3..6 colour, otherwise an attribute.
    enum Slot {
        Pos(usize),
        Col(usize, bool),
        Attr(usize),
        Skip,
    }
    let mut slots = Vec::new();
    let mut attrs: Vec<Attribute> = Vec::new();
    let mut has = [false; 6];
    for p in &el.props {
        match p {
            Prop::Scalar { name, ty } => {
                let pos = ["x", "y", "z"].iter().position(|a| a == name);
                let col = COLOR_NAMES.iter().position(|c| c.contains(&name.as_str()));
                if let Some(k) = pos.filter(|&k| !has[k]) {
                    has[k] = true;
                    slots.push(Slot::Pos(k));
                } else if let Some(k) = col.filter(|&k| !has[3 + k]) {
                    has[3 + k] = true;
                    slots.push(Slot::Col(k, ty.is_float()));
                } else {
                    slots.push(Slot::Attr(attrs.len()));
                    attrs.push(Attribute {
                        name: name.clone(),
                        kind: ty.kind(),
                        values: Vec::with_capacity(n),
                    });
                }
            }
            Prop::List { .. } => slots.push(Slot::Skip),
        }
    }
    if !(has[0] && has[1] && has[2]) {
        return Err(bad("PLY vertices need x, y and z"));
    }
    let colored = has[3] && has[4] && has[5];
    mesh.positions = vec![0.0; n * 3];
    mesh.colors = vec![if colored { 0 } else { 200 }; n * 3];
    for i in 0..n {
        for (p, s) in el.props.iter().zip(&slots) {
            match (p, s) {
                (Prop::Scalar { ty, .. }, Slot::Pos(k)) => {
                    mesh.positions[i * 3 + k] = r.value(*ty)? as f32
                }
                (Prop::Scalar { ty, .. }, Slot::Col(k, float)) => {
                    let v = r.value(*ty)?;
                    mesh.colors[i * 3 + k] = if *float {
                        (v * 255.0).round().clamp(0.0, 255.0) as u8
                    } else {
                        v.clamp(0.0, 255.0) as u8
                    };
                }
                (Prop::Scalar { ty, .. }, Slot::Attr(a)) => {
                    let v = r.value(*ty)?;
                    attrs[*a].values.push(v);
                }
                (Prop::List { count, item, .. }, _) => {
                    let c = r.value(*count)? as usize;
                    for _ in 0..c {
                        r.value(*item)?;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
    mesh.attributes = attrs;
    Ok(())
}

fn read_faces(
    r: &mut Reader,
    el: &Element,
    mesh: &mut Mesh,
    report: &mut ReadReport,
) -> Result<(), IoError> {
    let list = el.props.iter().position(|p| matches!(p, Prop::List { name, .. } if name == "vertex_indices" || name == "vertex_index"));
    let list = list.ok_or_else(|| bad("PLY faces need a vertex_indices list"))?;
    for (k, p) in el.props.iter().enumerate() {
        if k != list {
            let name = match p {
                Prop::Scalar { name, .. } | Prop::List { name, .. } => name.clone(),
            };
            if !report.face_properties_dropped.contains(&name) {
                report.face_properties_dropped.push(name);
            }
        }
    }
    let v = mesh.vertex_count() as f64;
    mesh.indices.reserve(el.count * 3);
    let mut corner = Vec::with_capacity(8);
    for _ in 0..el.count {
        for (k, p) in el.props.iter().enumerate() {
            match p {
                Prop::Scalar { ty, .. } => {
                    r.value(*ty)?;
                }
                Prop::List { count, item, .. } => {
                    let c = r.value(*count)? as usize;
                    corner.clear();
                    for _ in 0..c {
                        corner.push(r.value(*item)?);
                    }
                    if k != list {
                        continue;
                    }
                    if c < 3 || corner.iter().any(|&i| i < 0.0 || i >= v) {
                        report.faces_dropped += 1;
                        continue;
                    }
                    if c > 3 {
                        report.polygons_triangulated += 1;
                    }
                    for j in 1..c - 1 {
                        mesh.indices.extend_from_slice(&[
                            corner[0] as u32,
                            corner[j] as u32,
                            corner[j + 1] as u32,
                        ]);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The `splat` element: Brush's resolution-independent paint (see `spec/formats.md`). Properties are matched by
/// name; anything missing takes its default, anything extra is skipped.
fn read_splats(r: &mut Reader, el: &Element, mesh: &mut Mesh) -> Result<(), IoError> {
    // Slots: 0..3 position, 3..6 colour, 6 opacity, 7 radius, 8..11 normal, 12 drip.
    let slots: Vec<Option<usize>> = el
        .props
        .iter()
        .map(|p| {
            let Prop::Scalar { name, .. } = p else {
                return None;
            };
            Some(match name.as_str() {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                "red" | "diffuse_red" => 3,
                "green" | "diffuse_green" => 4,
                "blue" | "diffuse_blue" => 5,
                "opacity" | "alpha" => 6,
                "radius" => 7,
                "nx" => 8,
                "ny" => 9,
                "nz" => 10,
                "drip" => 11,
                _ => return None,
            })
        })
        .collect();
    for _ in 0..el.count {
        let mut v = [0.0f64; 3];
        let mut c = [255.0f64; 3];
        let (mut opacity, mut radius, mut n, mut drip) = (255.0, 0.01, [0.0, 1.0, 0.0], 0.0);
        for (p, slot) in el.props.iter().zip(&slots) {
            match p {
                Prop::Scalar { ty, .. } => {
                    let value = r.value(*ty)?;
                    match slot {
                        Some(k @ 0..=2) => v[*k] = value,
                        Some(k @ 3..=5) => c[*k - 3] = value,
                        Some(6) => opacity = value,
                        Some(7) => radius = value,
                        Some(k @ 8..=10) => n[*k - 8] = value,
                        Some(11) => drip = value,
                        _ => {}
                    }
                }
                Prop::List { count, item, .. } => {
                    let c = r.value(*count)? as usize;
                    for _ in 0..c {
                        r.value(*item)?;
                    }
                }
            }
        }
        let n = [n[0] as f32, n[1] as f32, n[2] as f32];
        // Brush writes unit normals; keep those bit-exact, and only fix up normals from foreign files.
        let n = {
            let l2 = brush_mesh::math::dot(n, n);
            if (l2 - 1.0).abs() > 1e-4 {
                brush_mesh::math::norm(n)
            } else {
                n
            }
        };
        mesh.splats.push(
            [v[0] as f32, v[1] as f32, v[2] as f32],
            n,
            [
                c[0].clamp(0.0, 255.0) as u8,
                c[1].clamp(0.0, 255.0) as u8,
                c[2].clamp(0.0, 255.0) as u8,
            ],
            opacity.clamp(0.0, 255.0) as u8,
            (radius as f32).max(1e-6),
            (drip as f32).clamp(0.0, 1.0),
        );
    }
    Ok(())
}

fn skip(r: &mut Reader, el: &Element) -> Result<(), IoError> {
    for _ in 0..el.count {
        for p in &el.props {
            match p {
                Prop::Scalar { ty, .. } => {
                    r.value(*ty)?;
                }
                Prop::List { count, item, .. } => {
                    let c = r.value(*count)? as usize;
                    for _ in 0..c {
                        r.value(*item)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlyOptions {
    /// Leave out the named attributes (write only position, colour and faces).
    pub geometry_only: bool,
}

/// Binary little-endian PLY: `float x y z`, `uchar red green blue`, then each attribute in its own type, and faces as
/// `list uchar int vertex_indices`. Without attributes this is byte-for-byte the format the Brush worker writes.
/// A mesh carrying splats gets a `splat` element after the faces (Brush's resolution-independent paint; readers that
/// do not know it skip it).
pub fn write_ply(mesh: &Mesh, opts: PlyOptions) -> Vec<u8> {
    let v = mesh.vertex_count();
    let f = mesh.face_count();
    let attrs: &[Attribute] = if opts.geometry_only {
        &[]
    } else {
        &mesh.attributes
    };
    let mut header = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex {v}\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\n"
    );
    for a in attrs {
        header.push_str(&format!("property {} {}\n", ply_name(a.kind), a.name));
    }
    header.push_str(&format!(
        "element face {f}\nproperty list uchar int vertex_indices\n"
    ));
    let s = mesh.splats.len();
    if s > 0 {
        header.push_str(&format!(
            "element splat {s}\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nproperty uchar opacity\nproperty float radius\nproperty float nx\nproperty float ny\nproperty float nz\nproperty float drip\n"
        ));
    }
    header.push_str("end_header\n");
    let stride = 15 + attrs.iter().map(|a| kind_size(a.kind)).sum::<usize>();
    let mut out = Vec::with_capacity(header.len() + v * stride + f * 13 + s * 32);
    out.extend_from_slice(header.as_bytes());
    for i in 0..v {
        for j in 0..3 {
            out.extend_from_slice(&mesh.positions[i * 3 + j].to_le_bytes());
        }
        out.extend_from_slice(&mesh.colors[i * 3..i * 3 + 3]);
        for a in attrs {
            push_scalar(&mut out, a.kind, a.values[i]);
        }
    }
    for t in 0..f {
        out.push(3);
        for j in 0..3 {
            out.extend_from_slice(&(mesh.indices[t * 3 + j] as i32).to_le_bytes());
        }
    }
    for i in 0..s {
        for j in 0..3 {
            out.extend_from_slice(&mesh.splats.positions[i * 3 + j].to_le_bytes());
        }
        out.extend_from_slice(&mesh.splats.colors[i * 3..i * 3 + 3]);
        out.push(mesh.splats.opacities[i]);
        out.extend_from_slice(&mesh.splats.radii[i].to_le_bytes());
        for j in 0..3 {
            out.extend_from_slice(&mesh.splats.normals[i * 3 + j].to_le_bytes());
        }
        out.extend_from_slice(&mesh.splats.drips[i].to_le_bytes());
    }
    out
}

fn kind_size(k: ScalarKind) -> usize {
    match k {
        ScalarKind::I8 | ScalarKind::U8 => 1,
        ScalarKind::I16 | ScalarKind::U16 => 2,
        ScalarKind::I32 | ScalarKind::U32 | ScalarKind::F32 => 4,
        ScalarKind::F64 => 8,
    }
}

fn push_scalar(out: &mut Vec<u8>, k: ScalarKind, v: f64) {
    let r = v.round();
    match k {
        ScalarKind::I8 => out.push(r.clamp(i8::MIN as f64, i8::MAX as f64) as i8 as u8),
        ScalarKind::U8 => out.push(r.clamp(0.0, 255.0) as u8),
        ScalarKind::I16 => {
            out.extend_from_slice(&(r.clamp(i16::MIN as f64, i16::MAX as f64) as i16).to_le_bytes())
        }
        ScalarKind::U16 => {
            out.extend_from_slice(&(r.clamp(0.0, u16::MAX as f64) as u16).to_le_bytes())
        }
        ScalarKind::I32 => {
            out.extend_from_slice(&(r.clamp(i32::MIN as f64, i32::MAX as f64) as i32).to_le_bytes())
        }
        ScalarKind::U32 => {
            out.extend_from_slice(&(r.clamp(0.0, u32::MAX as f64) as u32).to_le_bytes())
        }
        ScalarKind::F32 => out.extend_from_slice(&(v as f32).to_le_bytes()),
        ScalarKind::F64 => out.extend_from_slice(&v.to_le_bytes()),
    }
}
