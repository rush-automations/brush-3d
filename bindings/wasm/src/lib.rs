//! The engine for JavaScript, via wasm-bindgen. Build the npm package with `wasm-pack build --target web`.
//!
//! Values cross the boundary as plain JavaScript objects (ops, results, hits) through serde-wasm-bindgen, and mesh
//! buffers as typed-array copies. The TypeScript types of ops are generated from `spec/ops.schema.json` into
//! `types/ops.d.ts` and included in the generated `.d.ts`.

use brush_engine::io::{self, Format};
use brush_engine::{Actor, Dab, Document as EngineDocument, Mesh, Op, Placement, Scene};
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
  vertices: number; faces: number; hash: string;
  bounds: { min: [number, number, number]; max: [number, number, number] } | null;
  canUndo: boolean; canRedo: boolean; dirty: boolean; lastOp: string | null; journalLength: number;
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

/// A mesh being edited, with undo, redo and a journal of every change.
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

    /// Opens PLY (ascii or binary, any vertex properties) or OBJ bytes. A PLY without faces opens as a point cloud.
    pub fn open(bytes: &[u8]) -> Result<Document, JsError> {
        let format = Format::sniff(bytes).ok_or_else(|| JsError::new("not a PLY or OBJ file"))?;
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

    /// Applies ops in order, all or nothing. A failure names the index of the op that failed.
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
            "canUndo": self.doc.can_undo(),
            "canRedo": self.doc.can_redo(),
            "dirty": self.doc.is_dirty(),
            "lastOp": self.doc.last_op().map(|o| o.name()),
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

    /// The mesh as a file: "ply", "obj" or "glb". With `flattened`, placed objects are copied in (downloads, other
    /// tools); without, it is the document's own mesh, which is what is saved beside the scene.
    pub fn export(&self, format: &str, flattened: Option<bool>) -> Result<Vec<u8>, JsError> {
        let f = Format::from_extension(format)
            .ok_or_else(|| JsError::new("format must be ply, obj or glb"))?;
        Ok(if flattened.unwrap_or(false) {
            io::write(&self.doc.flattened(), f)
        } else {
            io::write(self.doc.mesh(), f)
        })
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
