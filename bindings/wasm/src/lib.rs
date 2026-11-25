//! The engine for JavaScript, via wasm-bindgen. Build the npm package with `wasm-pack build --target web`.
//!
//! Values cross the boundary as plain JavaScript objects (ops, results, hits) through serde-wasm-bindgen, and mesh
//! buffers as typed-array copies. The TypeScript types of ops are generated from `spec/ops.schema.json` into
//! `types/ops.d.ts` and included in the generated `.d.ts`.

use brush_engine::io::{self, Format};
use brush_engine::selection;
use brush_engine::{
    Actor, Axis, BoxRegion, Clipboard, Dab, Document as EngineDocument, Instance, Mesh, Op,
    Placement, Scene,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = include_str!("../types/ops.d.ts");

#[wasm_bindgen(typescript_custom_section)]
const TS_EXTRA: &str = r#"
export type Op = BrushEngineOperation;
export interface Actor { id?: string; kind?: "person" | "model" | "app" }
export interface Applied { outcome: { count: number; vertices: number; faces: number }; changed: boolean; hash: string }
export interface Hit { face: number; t: number; point: [number, number, number]; normal: [number, number, number]; barycentric: [number, number, number] }
export interface DocState {
  vertices: number; faces: number; hash: string; documentHash: string;
  revision: number; opsVersion: string;
  bounds: { min: [number, number, number]; max: [number, number, number] } | null;
  canUndo: boolean; canRedo: boolean; dirty: boolean;
  inPreview: boolean; inTransaction: boolean;
  lastOp: string | null; lastTransactionLabel: string | null;
  journalLength: number;
  attributes: { name: string; kind: string }[];
  splats: number;
}
export interface Instance { id: string; object: string; position: [number, number, number]; yawDeg?: number; pitchDeg?: number; rollDeg?: number; scale?: number }
export interface Scene { instances: Instance[] }
export interface Pick extends Hit { instance?: string }
export type Piece = "wall" | "floor" | "ramp" | "cube" | "cylinder" | "cone" | "sphere" | "object";
export interface Placement {
  piece: Piece; object?: string; id?: string;
  point: [number, number, number]; normal?: [number, number, number]; view: [number, number, number];
  cell: number; snap?: boolean; ground?: number; turns?: number; span?: [number, number, number]; scale?: number;
  color?: [number, number, number];
}
export interface ReadReport { polygonsTriangulated: number; facesDropped: number; elementsDropped: string[]; facePropertiesDropped: string[] }
export interface Measure { volume: number; surfaceArea: number; centroid: [number, number, number]; closed: boolean; splats: number }
export type Axis = "x" | "y" | "z";
export interface ClipItem { object: string; offset: [number, number, number]; yawDeg?: number; pitchDeg?: number; rollDeg?: number; scale?: number }
export interface Clipboard { items: ClipItem[]; pivot: [number, number, number]; step: number }
export interface Island { faces: number[]; isWholeMesh: boolean; bounds: [[number, number, number], [number, number, number]]; overlapsOtherFaces: boolean }
export interface TangledCluster { faces: number[]; isWholeMesh: boolean; bounds: [[number, number, number], [number, number, number]]; islands: number }
export interface DetachedPart { instanceId: string; object: string; anchor: [number, number, number]; faces: number; islands: number; isWholeMesh: boolean }
"#;

fn to_js<T: Serialize>(v: &T) -> Result<JsValue, JsError> {
    v.serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsError::new(&e.to_string()))
}

fn from_js<T: serde::de::DeserializeOwned>(v: JsValue, what: &str) -> Result<T, JsError> {
    serde_wasm_bindgen::from_value(v).map_err(|e| JsError::new(&format!("bad {what}: {e}")))
}

fn actor(v: JsValue) -> Result<Actor, JsError> {
    if v.is_undefined() || v.is_null() {
        Ok(Actor::default())
    } else {
        from_js(v, "actor")
    }
}

fn vec3_f64(slice: &[f64], what: &str) -> Result<[f64; 3], JsError> {
    slice
        .try_into()
        .map_err(|_| JsError::new(&format!("{what} needs 3 numbers")))
}

/// Engine and op-set versions.
#[wasm_bindgen(unchecked_return_type = "{ engine: string; ops: string }")]
pub fn version() -> Result<JsValue, JsError> {
    to_js(
        &serde_json::json!({ "engine": env!("CARGO_PKG_VERSION"), "ops": brush_engine::OPS_VERSION }),
    )
}

/// The JSON Schema of ops: the contract for anything that produces edits, such as an LLM with tool calling.
#[wasm_bindgen(js_name = opsSchema)]
pub fn ops_schema() -> Result<JsValue, JsError> {
    to_js(&brush_engine::mesh::ops_schema())
}

/// True when `bytes` begins with the `.brush` versioned project container signature.
#[wasm_bindgen(js_name = isProjectContainer)]
pub fn is_project_container(bytes: &[u8]) -> bool {
    io::is_project_container(bytes)
}

// ---- standalone selection, group transform, snapping & clipboard helpers ----

#[wasm_bindgen(js_name = groupPivot, unchecked_return_type = "[number, number, number]")]
pub fn group_pivot(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    to_js(&selection::group_pivot(&list))
}

#[wasm_bindgen(js_name = planeAngle)]
pub fn plane_angle(
    #[wasm_bindgen(unchecked_param_type = "Axis")] axis: JsValue,
    v: &[f64],
) -> Result<f64, JsError> {
    let axis: Axis = from_js(axis, "axis")?;
    Ok(selection::plane_angle(axis, vec3_f64(v, "v")?))
}

#[wasm_bindgen(js_name = snapTurn)]
pub fn snap_turn(
    raw: f64,
    #[wasm_bindgen(unchecked_param_type = "Axis")] axis: JsValue,
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] start: JsValue,
    snap: bool,
) -> Result<f64, JsError> {
    let axis: Axis = from_js(axis, "axis")?;
    let start: Vec<Instance> = from_js(start, "instances")?;
    Ok(selection::snap_turn(raw, axis, &start, snap))
}

#[wasm_bindgen(js_name = translateGroup, unchecked_return_type = "Instance[]")]
pub fn translate_group(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
    delta: &[f64],
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    to_js(&selection::translate_group(
        &list,
        vec3_f64(delta, "delta")?,
    ))
}

#[wasm_bindgen(js_name = rotateGroup, unchecked_return_type = "Instance[]")]
pub fn rotate_group(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
    deg: f64,
    pivot: Option<Vec<f64>>,
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    let p = match pivot {
        Some(v) => Some(vec3_f64(&v, "pivot")?),
        None => None,
    };
    to_js(&selection::rotate_group(&list, deg, p))
}

#[wasm_bindgen(js_name = rotateGroupAbout, unchecked_return_type = "Instance[]")]
pub fn rotate_group_about(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
    #[wasm_bindgen(unchecked_param_type = "Axis")] axis: JsValue,
    deg: f64,
    pivot: &[f64],
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    let axis: Axis = from_js(axis, "axis")?;
    to_js(&selection::rotate_group_about(
        &list,
        axis,
        deg,
        vec3_f64(pivot, "pivot")?,
    ))
}

#[wasm_bindgen(js_name = scaleGroup, unchecked_return_type = "Instance[]")]
pub fn scale_group(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
    factor: f64,
    pivot: Option<Vec<f64>>,
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    let p = match pivot {
        Some(v) => Some(vec3_f64(&v, "pivot")?),
        None => None,
    };
    to_js(&selection::scale_group(&list, factor, p))
}

#[wasm_bindgen(js_name = dragDelta, unchecked_return_type = "[number, number, number]")]
pub fn drag_delta(from: &[f64], to: &[f64], cell: f64, snap: bool) -> Result<JsValue, JsError> {
    to_js(&selection::drag_delta(
        vec3_f64(from, "from")?,
        vec3_f64(to, "to")?,
        cell,
        snap,
    ))
}

#[wasm_bindgen(js_name = copyGroup, unchecked_return_type = "Clipboard")]
pub fn copy_group(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] list: JsValue,
    width: f64,
) -> Result<JsValue, JsError> {
    let list: Vec<Instance> = from_js(list, "instances")?;
    to_js(&selection::copy_group(&list, width))
}

#[wasm_bindgen(js_name = pasteGroup, unchecked_return_type = "Instance[]")]
pub fn paste_group(
    #[wasm_bindgen(unchecked_param_type = "Clipboard")] clip: JsValue,
    at: &[f64],
    #[wasm_bindgen(unchecked_param_type = "readonly string[]")] ids: JsValue,
) -> Result<JsValue, JsError> {
    let clip: Clipboard = from_js(clip, "clipboard")?;
    let ids: Vec<String> = from_js(ids, "ids")?;
    to_js(&selection::paste_group(&clip, vec3_f64(at, "at")?, &ids))
}

#[wasm_bindgen(js_name = freeIds, unchecked_return_type = "string[]")]
pub fn free_ids(
    #[wasm_bindgen(unchecked_param_type = "readonly Instance[]")] existing: JsValue,
    count: usize,
) -> Result<JsValue, JsError> {
    let existing: Vec<Instance> = from_js(existing, "instances")?;
    to_js(&selection::free_ids(&existing, count))
}

/// A mesh document being edited, with revisions, transactions, previews, undo/redo, and a journal of every change.
///
/// A read-only document (`Document.openReadOnly`) is the same engine used for viewing: it renders and picks, but
/// refuses every edit, so nothing is ever added to its journal.
#[wasm_bindgen]
pub struct Document {
    doc: EngineDocument,
    report: Option<io::ReadReport>,
    read_only: bool,
}

#[wasm_bindgen]
impl Document {
    /// An empty document.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Document {
        Document {
            doc: EngineDocument::new(Mesh::default()),
            report: None,
            read_only: false,
        }
    }

    /// Opens `.brush` project containers, PLY (ascii or binary, any vertex properties), or OBJ bytes.
    pub fn open(bytes: &[u8]) -> Result<Document, JsError> {
        if io::is_project_container(bytes) {
            let doc =
                EngineDocument::open_project(bytes).map_err(|e| JsError::new(&e.to_string()))?;
            return Ok(Document {
                doc,
                report: None,
                read_only: false,
            });
        }
        let format = Format::sniff(bytes)
            .ok_or_else(|| JsError::new("not a PLY, OBJ or Brush project file"))?;
        let (mesh, report) = io::read(bytes, format).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Document {
            doc: EngineDocument::new(mesh),
            report: Some(report),
            read_only: false,
        })
    }

    /// Opens a file for viewing only: edits throw, and undo, redo and revert do nothing.
    #[wasm_bindgen(js_name = openReadOnly)]
    pub fn open_read_only(bytes: &[u8]) -> Result<Document, JsError> {
        let mut doc = Document::open(bytes)?;
        doc.read_only = true;
        Ok(doc)
    }

    #[wasm_bindgen(getter, js_name = readOnly)]
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    fn writable(&self) -> Result<(), JsError> {
        if self.read_only {
            Err(JsError::new("this document is read-only"))
        } else {
            Ok(())
        }
    }

    /// Monotonic document revision, incremented on every committed change (unchanged during previews).
    pub fn revision(&self) -> f64 {
        self.doc.revision() as f64
    }

    #[wasm_bindgen(js_name = checkRevision)]
    pub fn check_revision(&self, expected: f64) -> Result<(), JsError> {
        self.doc
            .check_revision(expected as u64)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// What the reader dropped or changed when the file was opened.
    #[wasm_bindgen(js_name = readReport, unchecked_return_type = "ReadReport | null")]
    pub fn read_report(&self) -> Result<JsValue, JsError> {
        match &self.report {
            None => Ok(JsValue::NULL),
            Some(r) => to_js(&serde_json::json!({
                "polygonsTriangulated": r.polygons_triangulated,
                "facesDropped": r.faces_dropped,
                "elementsDropped": r.elements_dropped,
                "facePropertiesDropped": r.face_properties_dropped,
            })),
        }
    }

    /// Applies one op. A `sculpt` op opens a stroke that `extendStroke` adds to.
    #[wasm_bindgen(unchecked_return_type = "Applied")]
    pub fn apply(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Op")] op: JsValue,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let op: Op = from_js(op, "op")?;
        let applied = self
            .doc
            .apply(op, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&applied)
    }

    /// Applies ops in order, all or nothing (one undo step per op). Prefer `applyTransaction` for user actions.
    #[wasm_bindgen(js_name = applyAll, unchecked_return_type = "Applied[]")]
    pub fn apply_all(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Op[]")] ops: JsValue,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let ops: Vec<Op> = from_js(ops, "ops")?;
        let applied = self
            .doc
            .apply_all(ops, actor(actor_)?)
            .map_err(|(i, e)| JsError::new(&format!("op {i}: {e}")))?;
        to_js(&applied)
    }

    /// Applies ops atomically as a single user action (one undo step and one transaction journal entry).
    #[wasm_bindgen(js_name = applyTransaction, unchecked_return_type = "Applied[]")]
    pub fn apply_transaction(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Op[]")] ops: JsValue,
        label: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let ops: Vec<Op> = from_js(ops, "ops")?;
        let applied = self
            .doc
            .apply_transaction(ops, label, actor(actor_)?)
            .map_err(|(i, e)| JsError::new(&format!("op {i}: {e}")))?;
        to_js(&applied)
    }

    /// Opens a streaming transaction so subsequent `apply` / `extendStroke` calls collapse into one undo step.
    #[wasm_bindgen(js_name = beginTransaction)]
    pub fn begin_transaction(
        &mut self,
        label: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<f64, JsError> {
        self.writable()?;
        self.doc
            .begin_transaction(label, actor(actor_)?)
            .map(|r| r as f64)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = commitTransaction, unchecked_return_type = "Applied | null")]
    pub fn commit_transaction(&mut self) -> Result<JsValue, JsError> {
        self.writable()?;
        match self.doc.commit_transaction() {
            Some(a) => to_js(&a),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = cancelTransaction)]
    pub fn cancel_transaction(&mut self) -> bool {
        !self.read_only && self.doc.cancel_transaction()
    }

    // ---- preview sessions ----

    #[wasm_bindgen(js_name = beginPreview)]
    pub fn begin_preview(&mut self) -> Result<f64, JsError> {
        self.writable()?;
        self.doc
            .begin_preview()
            .map(|r| r as f64)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = updatePreview, unchecked_return_type = "Applied[]")]
    pub fn update_preview(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Op[]")] ops: JsValue,
        expected_revision: Option<f64>,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let ops: Vec<Op> = from_js(ops, "ops")?;
        let applied = self
            .doc
            .update_preview(ops, expected_revision.map(|r| r as u64))
            .map_err(|(i, e)| JsError::new(&format!("op {i}: {e}")))?;
        to_js(&applied)
    }

    #[wasm_bindgen(js_name = commitPreview, unchecked_return_type = "Applied | null")]
    pub fn commit_preview(
        &mut self,
        label: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        match self
            .doc
            .commit_preview(label, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?
        {
            Some(a) => to_js(&a),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = cancelPreview)]
    pub fn cancel_preview(&mut self) -> bool {
        !self.read_only && self.doc.cancel_preview()
    }

    /// Adds dabs to the open stroke: the same result as if they had been in the first op, and still one undo step.
    #[wasm_bindgen(js_name = extendStroke, unchecked_return_type = "Applied")]
    pub fn extend_stroke(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Dab[]")] dabs: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let dabs: Vec<Dab> = from_js(dabs, "dabs")?;
        let applied = self
            .doc
            .extend_stroke(dabs)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&applied)
    }

    #[wasm_bindgen(js_name = endStroke)]
    pub fn end_stroke(&mut self) {
        self.doc.end_stroke();
    }

    pub fn undo(&mut self) -> bool {
        !self.read_only && self.doc.undo()
    }

    pub fn redo(&mut self) -> bool {
        !self.read_only && self.doc.redo()
    }

    /// Back to the mesh as opened or last saved. Undoable.
    pub fn revert(&mut self) -> bool {
        !self.read_only && self.doc.revert()
    }

    #[wasm_bindgen(js_name = markSaved)]
    pub fn mark_saved(&mut self) {
        self.doc.mark_saved();
    }

    #[wasm_bindgen(unchecked_return_type = "DocState")]
    pub fn state(&self) -> Result<JsValue, JsError> {
        let m = self.doc.mesh();
        to_js(&serde_json::json!({
            "vertices": m.vertex_count(),
            "faces": m.face_count(),
            "bounds": m.bounds().map(|(lo, hi)| serde_json::json!({"min": lo, "max": hi})),
            "hash": m.content_hash(),
            "documentHash": self.doc.hash(),
            "revision": self.doc.revision(),
            "opsVersion": brush_engine::OPS_VERSION,
            "canUndo": self.doc.can_undo(),
            "canRedo": self.doc.can_redo(),
            "dirty": self.doc.is_dirty(),
            "inPreview": self.doc.in_preview(),
            "inTransaction": self.doc.in_transaction(),
            "lastOp": self.doc.last_op().map(|o| o.name()),
            "lastTransactionLabel": self.doc.last_transaction_label(),
            "attributes": m.attributes.iter().map(|a| serde_json::json!({"name": a.name, "kind": a.kind})).collect::<Vec<_>>(),
            "splats": m.splats.len(),
            "journalLength": self.doc.journal().entries.len(),
        }))
    }

    /// Vertex positions, 3 per vertex (meters, +y up).
    pub fn positions(&self) -> Vec<f32> {
        self.doc.mesh().positions.clone()
    }

    /// Vertex colours, 3 bytes per vertex.
    pub fn colors(&self) -> Vec<u8> {
        self.doc.mesh().colors.clone()
    }

    /// Triangle corners, 3 per face.
    pub fn indices(&self) -> Vec<u32> {
        self.doc.mesh().indices.clone()
    }

    /// How many paint splats the mesh carries (see the `splatPaint` op).
    #[wasm_bindgen(js_name = splatCount)]
    pub fn splat_count(&self) -> usize {
        self.doc.mesh().splats.len()
    }

    /// The splats' geometry, 8 floats per splat: x, y, z, radius, nx, ny, nz, drip.
    pub fn splats(&self) -> Vec<f32> {
        let s = &self.doc.mesh().splats;
        let mut out = Vec::with_capacity(s.len() * 8);
        for i in 0..s.len() {
            out.extend_from_slice(&s.positions[i * 3..i * 3 + 3]);
            out.push(s.radii[i]);
            out.extend_from_slice(&s.normals[i * 3..i * 3 + 3]);
            out.push(s.drips[i]);
        }
        out
    }

    /// The splats' colours, 4 bytes per splat: r, g, b, opacity.
    #[wasm_bindgen(js_name = splatColors)]
    pub fn splat_colors(&self) -> Vec<u8> {
        let s = &self.doc.mesh().splats;
        let mut out = Vec::with_capacity(s.len() * 4);
        for i in 0..s.len() {
            out.extend_from_slice(&s.colors[i * 3..i * 3 + 3]);
            out.push(s.opacities[i]);
        }
        out
    }

    /// Volume, surface area, centroid and watertightness (exact for a closed surface).
    #[wasm_bindgen(unchecked_return_type = "Measure")]
    pub fn measure(&self) -> Result<JsValue, JsError> {
        to_js(&self.doc.mesh().measure())
    }

    /// Area-weighted vertex normals, 3 per vertex.
    pub fn normals(&mut self) -> Vec<f32> {
        self.doc.vertex_normals()
    }

    /// The first point where `origin + t * dir` meets the surface, or null. Faces are two-sided.
    #[wasm_bindgen(unchecked_return_type = "Hit | null")]
    pub fn raycast(&mut self, origin: &[f32], dir: &[f32]) -> Result<JsValue, JsError> {
        let (o, d): ([f32; 3], [f32; 3]) = (
            origin
                .try_into()
                .map_err(|_| JsError::new("origin needs 3 numbers"))?,
            dir.try_into()
                .map_err(|_| JsError::new("dir needs 3 numbers"))?,
        );
        match self.doc.raycast(o, d) {
            Some(hit) => to_js(&hit),
            None => Ok(JsValue::NULL),
        }
    }

    /// Packages the entire project (surface, linked objects, scene, journal, revision, metadata, features) as `.brush` bytes.
    #[wasm_bindgen(js_name = saveProject)]
    pub fn save_project(&self) -> Vec<u8> {
        self.doc.save_project()
    }

    /// Exports `"brush"` (versioned project container) or a mesh interchange format (`"ply"`, `"obj"`, `"glb"`).
    /// With `flattened`, placed objects are copied into mesh exports.
    pub fn export(&self, format: &str, flattened: Option<bool>) -> Result<Vec<u8>, JsError> {
        if format.eq_ignore_ascii_case("brush") {
            return Ok(self.doc.save_project());
        }
        let f = Format::from_extension(format)
            .ok_or_else(|| JsError::new("format must be brush, ply, obj or glb"))?;
        Ok(if flattened.unwrap_or(false) {
            io::write(&self.doc.flattened(), f)
        } else {
            io::write(self.doc.mesh(), f)
        })
    }

    /// Exports one loaded object mesh (e.g. a detached or combined part) as `"ply"`, `"obj"` or `"glb"`.
    #[wasm_bindgen(js_name = exportObject)]
    pub fn export_object(&self, object: &str, format: Option<String>) -> Result<Vec<u8>, JsError> {
        let mesh = self
            .doc
            .object(object)
            .ok_or_else(|| JsError::new(&format!("object {object} is not loaded")))?;
        let fmt_str = format.as_deref().unwrap_or("ply");
        let f = Format::from_extension(fmt_str)
            .ok_or_else(|| JsError::new("format must be ply, obj or glb"))?;
        Ok(io::write(mesh, f))
    }

    pub fn metadata(&self) -> Result<JsValue, JsError> {
        to_js(self.doc.metadata())
    }

    #[wasm_bindgen(js_name = setMetadata)]
    pub fn set_metadata(&mut self, value: JsValue) -> Result<(), JsError> {
        self.writable()?;
        let v: serde_json::Value = from_js(value, "metadata")?;
        self.doc.set_metadata(v);
        Ok(())
    }

    pub fn features(&self) -> Result<JsValue, JsError> {
        to_js(&self.doc.features())
    }

    #[wasm_bindgen(js_name = setFeatures)]
    pub fn set_features(&mut self, list: JsValue) -> Result<(), JsError> {
        self.writable()?;
        let v: Vec<serde_json::Value> = from_js(list, "features")?;
        self.doc.set_features(v);
        Ok(())
    }

    // ---- island inspection & detach / join workflows ----

    #[wasm_bindgen(js_name = connectedComponent, unchecked_return_type = "Island | null")]
    pub fn connected_component(&self, start_face: u32) -> Result<JsValue, JsError> {
        match self.doc.mesh().connected_component(start_face) {
            Some(c) => to_js(&c),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = tangledCluster, unchecked_return_type = "TangledCluster | null")]
    pub fn tangled_cluster(&self, start_face: u32) -> Result<JsValue, JsError> {
        match self.doc.mesh().tangled_cluster(start_face) {
            Some(c) => to_js(&c),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = deleteIsland, unchecked_return_type = "Applied")]
    pub fn delete_island(
        &mut self,
        start_face: u32,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let applied = self
            .doc
            .delete_island(start_face, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&applied)
    }

    #[wasm_bindgen(js_name = detachIsland, unchecked_return_type = "DetachedPart")]
    pub fn detach_island(
        &mut self,
        start_face: u32,
        tangled: bool,
        object_key: String,
        instance_id: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let part = self
            .doc
            .detach_island(start_face, tangled, object_key, instance_id, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&part)
    }

    #[wasm_bindgen(js_name = detachBox, unchecked_return_type = "DetachedPart")]
    pub fn detach_box(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "BoxRegion")] region: JsValue,
        object_key: String,
        instance_id: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let region: BoxRegion = from_js(region, "box")?;
        let part = self
            .doc
            .detach_box(region, object_key, instance_id, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&part)
    }

    #[wasm_bindgen(js_name = combineInstances, unchecked_return_type = "DetachedPart")]
    pub fn combine_instances(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "readonly string[]")] ids: JsValue,
        object_key: String,
        instance_id: Option<String>,
        #[wasm_bindgen(unchecked_param_type = "Actor | undefined")] actor_: JsValue,
    ) -> Result<JsValue, JsError> {
        self.writable()?;
        let ids: Vec<String> = from_js(ids, "ids")?;
        let part = self
            .doc
            .combine_instances(&ids, object_key, instance_id, actor(actor_)?)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&part)
    }

    // ---- scene: linked instances of other objects ----

    /// The placed objects.
    #[wasm_bindgen(unchecked_return_type = "Scene")]
    pub fn scene(&self) -> Result<JsValue, JsError> {
        to_js(self.doc.scene())
    }

    /// Loads a stored scene as the saved state. Call right after opening.
    #[wasm_bindgen(js_name = loadScene)]
    pub fn load_scene(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Scene")] scene: JsValue,
    ) -> Result<(), JsError> {
        let scene: Scene = from_js(scene, "scene")?;
        self.doc.load_scene(scene);
        Ok(())
    }

    /// Supplies (or refreshes) the mesh behind an object pointer such as "scan:<id>", from PLY or OBJ bytes. Every
    /// instance of it shows the new mesh at once. Allowed on read-only documents: it is what they show, not an edit.
    #[wasm_bindgen(js_name = setObject)]
    pub fn set_object(&mut self, object: &str, bytes: &[u8]) -> Result<(), JsError> {
        let format = Format::sniff(bytes).ok_or_else(|| JsError::new("not a PLY or OBJ file"))?;
        let (mesh, _) = io::read(bytes, format).map_err(|e| JsError::new(&e.to_string()))?;
        self.doc.set_object(object, mesh);
        Ok(())
    }

    #[wasm_bindgen(js_name = hasObject)]
    pub fn has_object(&self, object: &str) -> bool {
        self.doc.object(object).is_some()
    }

    /// Keys of every object mesh currently loaded in this document.
    #[wasm_bindgen(js_name = loadedObjects)]
    pub fn loaded_objects(&self) -> Vec<String> {
        self.doc.loaded_objects()
    }

    /// Objects the scene points at whose meshes have not been supplied.
    #[wasm_bindgen(js_name = missingObjects)]
    pub fn missing_objects(&self) -> Vec<String> {
        self.doc.missing_objects()
    }

    #[wasm_bindgen(js_name = objectPositions)]
    pub fn object_positions(&self, object: &str) -> Vec<f32> {
        self.doc
            .object(object)
            .map(|m| m.positions.clone())
            .unwrap_or_default()
    }

    #[wasm_bindgen(js_name = objectColors)]
    pub fn object_colors(&self, object: &str) -> Vec<u8> {
        self.doc
            .object(object)
            .map(|m| m.colors.clone())
            .unwrap_or_default()
    }

    #[wasm_bindgen(js_name = objectIndices)]
    pub fn object_indices(&self, object: &str) -> Vec<u32> {
        self.doc
            .object(object)
            .map(|m| m.indices.clone())
            .unwrap_or_default()
    }

    #[wasm_bindgen(js_name = objectNormals)]
    pub fn object_normals(&self, object: &str) -> Vec<f32> {
        self.doc.object_normals(object).unwrap_or_default()
    }

    /// A placed instance's object-to-world transform, column-major 4x4 (three.js `Matrix4.fromArray`), or undefined.
    #[wasm_bindgen(js_name = instanceTransform)]
    pub fn instance_transform(&self, id: &str) -> Option<Vec<f32>> {
        self.doc.instance_transform(id).map(|m| m.to_vec())
    }

    /// Where an object would be drawn for a placement (a ghost), or undefined when its mesh is unknown. The optional
    /// `pitchDeg` and `rollDeg` tilt it as an instance's do.
    #[wasm_bindgen(js_name = objectTransform)]
    pub fn object_transform(
        &self,
        object: &str,
        position: &[f64],
        yaw_deg: f64,
        scale: f64,
        pitch_deg: Option<f64>,
        roll_deg: Option<f64>,
    ) -> Option<Vec<f32>> {
        let p: [f64; 3] = position.try_into().ok()?;
        self.doc
            .object_transform(
                object,
                p,
                yaw_deg,
                scale,
                pitch_deg.unwrap_or(0.0),
                roll_deg.unwrap_or(0.0),
            )
            .map(|m| m.to_vec())
    }

    /// The nearest surface: the document's own mesh, or a placed instance (`instance` is its id).
    #[wasm_bindgen(unchecked_return_type = "Pick | null")]
    pub fn pick(&mut self, origin: &[f32], dir: &[f32]) -> Result<JsValue, JsError> {
        let (o, d): ([f32; 3], [f32; 3]) = (
            origin
                .try_into()
                .map_err(|_| JsError::new("origin needs 3 numbers"))?,
            dir.try_into()
                .map_err(|_| JsError::new("dir needs 3 numbers"))?,
        );
        match self.doc.pick(o, d) {
            Some(p) => to_js(&p),
            None => Ok(JsValue::NULL),
        }
    }

    /// The op that builds a piece where the pointer is (grid snapping, facing the camera, stacking). Nothing changes
    /// until it is applied.
    #[wasm_bindgen(unchecked_return_type = "Op")]
    pub fn place(
        &self,
        #[wasm_bindgen(unchecked_param_type = "Placement")] placement: JsValue,
    ) -> Result<JsValue, JsError> {
        let p: Placement = from_js(placement, "placement")?;
        let op = self
            .doc
            .place(&p)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&op)
    }

    /// Every change, one JSON object per line.
    pub fn journal(&self) -> String {
        self.doc.journal().to_jsonl()
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}
