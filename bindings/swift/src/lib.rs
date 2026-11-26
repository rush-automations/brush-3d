//! The engine behind a C ABI, for Swift on iOS (`include/brush_engine.h`, wrapped by `Sources/BrushEngine.swift`) and
//! any other native host. It is the same `brush_engine::Document` the browser runs as WebAssembly, so a phone and the web
//! app produce bit-identical meshes and journals from the same ops.
//!
//! Built for phones:
//! - mesh buffers are borrowed straight from the engine, with no copies, until the next call that changes the mesh;
//! - a live sculpt stroke is extended with plain arrays of dab centres (and Apple Pencil pressures), not JSON;
//! - undo history is capped by memory as well as by count, since every undo step holds a whole mesh.
//!
//! Conventions: strings are UTF-8 and NUL-terminated. Strings and byte buffers the engine returns are owned by the
//! caller and freed with `brush_string_free` and `brush_bytes_free`. A function that can fail takes `char **error`;
//! on failure it returns NULL (or -1) and, when `error` is not NULL, stores a message there. Panics never cross the
//! boundary: they come back as errors.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use brush_engine::io::{self, Format};
use std::collections::BTreeMap;

use brush_engine::{
    capabilities, precision, Actor, AlignAxis, AlignMode, BoxRegion, ConstraintRecord, Dab,
    Document, FeatureKind, FeatureSpec, Mesh, Op, Placement, Scene, Workplane, DEFAULT_UNDO_DEPTH,
};

/// An open mesh: the engine document, plus the vertex normals and flat splat buffers last computed for display.
pub struct BrushDocument {
    doc: Document,
    read_only: bool,
    report: io::ReadReport,
    normals: Vec<f32>,
    normals_fresh: bool,
    /// The mesh's splats flattened for display: 8 floats and 4 bytes per splat, rebuilt after a change.
    splats: Vec<f32>,
    splat_colors: Vec<u8>,
    splats_fresh: bool,
    /// Normals of each object's mesh, computed on first use.
    object_normals: BTreeMap<String, Vec<f32>>,
}

impl BrushDocument {
    fn new(doc: Document, read_only: bool, report: io::ReadReport) -> BrushDocument {
        BrushDocument {
            doc,
            read_only,
            report,
            normals: Vec::new(),
            normals_fresh: false,
            splats: Vec::new(),
            splat_colors: Vec::new(),
            splats_fresh: false,
            object_normals: BTreeMap::new(),
        }
    }

    fn changed(&mut self) {
        self.normals_fresh = false;
        self.splats_fresh = false;
    }

    /// Rebuilds the flat splat buffers when the mesh has changed since they were last built.
    fn refresh_splats(&mut self) {
        if self.splats_fresh {
            return;
        }
        let s = &self.doc.mesh().splats;
        self.splats.clear();
        self.splat_colors.clear();
        self.splats.reserve(s.len() * 8);
        self.splat_colors.reserve(s.len() * 4);
        for i in 0..s.len() {
            self.splats
                .extend_from_slice(&s.positions[i * 3..i * 3 + 3]);
            self.splats.push(s.radii[i]);
            self.splats.extend_from_slice(&s.normals[i * 3..i * 3 + 3]);
            self.splats.push(s.drips[i]);
            self.splat_colors
                .extend_from_slice(&s.colors[i * 3..i * 3 + 3]);
            self.splat_colors.push(s.opacities[i]);
        }
        self.splats_fresh = true;
    }

    fn writable(&self) -> Result<(), String> {
        if self.read_only {
            Err("this document is read-only".into())
        } else {
            Ok(())
        }
    }
}

/// A point on the surface under a ray.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrushHit {
    pub face: u32,
    /// Distance along the ray, in units of the ray direction's length.
    pub t: f32,
    pub point: [f32; 3],
    /// The face normal, turned to face the ray.
    pub normal: [f32; 3],
    pub barycentric: [f32; 3],
}

// ---- plumbing ----

fn c_string(s: impl Into<Vec<u8>>) -> *mut c_char {
    // Engine output is JSON or messages, which never contain NUL; replace any just in case rather than fail.
    let bytes: Vec<u8> = s
        .into()
        .into_iter()
        .map(|b| if b == 0 { b' ' } else { b })
        .collect();
    CString::new(bytes).map_or(ptr::null_mut(), CString::into_raw)
}

unsafe fn set_error(error: *mut *mut c_char, message: &str) {
    if !error.is_null() {
        *error = c_string(message);
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    let what = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".into());
    format!("the engine failed unexpectedly: {what}")
}

/// Runs `f`, turning an `Err` or a panic into a message in `error` and `fallback` as the result.
unsafe fn guard<T>(
    error: *mut *mut c_char,
    fallback: T,
    f: impl FnOnce() -> Result<T, String>,
) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(message)) => {
            set_error(error, &message);
            fallback
        }
        Err(payload) => {
            set_error(error, &panic_message(payload));
            fallback
        }
    }
}

unsafe fn str_arg<'a>(s: *const c_char, what: &str) -> Result<&'a str, String> {
    if s.is_null() {
        return Err(format!("{what} is missing"));
    }
    CStr::from_ptr(s)
        .to_str()
        .map_err(|_| format!("{what} is not UTF-8"))
}

unsafe fn doc_mut<'a>(doc: *mut BrushDocument) -> Result<&'a mut BrushDocument, String> {
    doc.as_mut().ok_or_else(|| "no document".to_string())
}

fn json<T: serde::Serialize>(v: &T) -> Result<*mut c_char, String> {
    serde_json::to_string(v)
        .map(c_string)
        .map_err(|e| e.to_string())
}

/// Roughly what one undo step costs: a whole copy of the mesh.
fn mesh_bytes(mesh: &Mesh) -> usize {
    // Each splat holds 8 floats (position, normal, radius, drip) and 4 bytes (colour, opacity).
    let base = mesh.positions.len() * 4
        + mesh.colors.len()
        + mesh.indices.len() * 4
        + mesh.splats.len() * 36;
    // Named attributes ride along in every copy too; they are rarely large, so a margin covers them.
    base + base / 4
}

/// How many undo steps fit in `budget` bytes: at least 1, at most the engine's default. 0 means no budget.
fn undo_depth_for(mesh: &Mesh, budget: usize) -> usize {
    if budget == 0 {
        return DEFAULT_UNDO_DEPTH;
    }
    (budget / mesh_bytes(mesh).max(1)).clamp(1, DEFAULT_UNDO_DEPTH)
}

// ---- library ----

/// Engine and op-set versions, as JSON: `{"engine": "0.1.0", "ops": "..."}`.
#[no_mangle]
pub extern "C" fn brush_version() -> *mut c_char {
    let v = serde_json::json!({ "engine": env!("CARGO_PKG_VERSION"), "ops": brush_engine::OPS_VERSION });
    c_string(v.to_string())
}

/// The JSON Schema of ops.
#[no_mangle]
pub extern "C" fn brush_ops_schema() -> *mut c_char {
    c_string(brush_engine::mesh::ops_schema().to_string())
}

/// Versioned capability manifest describing supported ops, formats, and precision features, as JSON.
#[no_mangle]
pub extern "C" fn brush_capabilities() -> *mut c_char {
    json(&capabilities()).unwrap_or(ptr::null_mut())
}

/// Measures the exact 3D distance and per-axis delta from `a` to `b`, as JSON: `{"distance", "delta"}`.
#[no_mangle]
pub unsafe extern "C" fn brush_measure_distance(
    a: *const f32,
    b: *const f32,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        if a.is_null() || b.is_null() {
            return Err("point is missing".into());
        }
        let pa = *(a as *const [f32; 3]);
        let pb = *(b as *const [f32; 3]);
        json(&precision::measure_distance(pa, pb))
    })
}

/// Frees a string returned by the engine. NULL is fine.
#[no_mangle]
pub unsafe extern "C" fn brush_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Frees bytes returned by the engine (`brush_document_export`). NULL is fine.
#[no_mangle]
pub unsafe extern "C" fn brush_bytes_free(bytes: *mut u8, len: usize) {
    if !bytes.is_null() {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(bytes, len)));
    }
}

// ---- documents ----

/// Opens PLY (ascii or binary, any vertex properties) or OBJ bytes. A PLY without faces opens as a point cloud.
///
/// A read-only document renders and picks but refuses every edit, so nothing reaches its journal. `undo_budget` caps
/// the memory undo history may hold, in bytes (0 for the engine's default depth alone); each step holds one mesh.
#[no_mangle]
pub unsafe extern "C" fn brush_document_open(
    bytes: *const u8,
    len: usize,
    read_only: bool,
    undo_budget: usize,
    error: *mut *mut c_char,
) -> *mut BrushDocument {
    guard(error, ptr::null_mut(), || {
        if bytes.is_null() && len > 0 {
            return Err("no bytes".into());
        }
        let data = if len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(bytes, len)
        };
        let format = Format::sniff(data).ok_or("not a PLY or OBJ file")?;
        let (mesh, report) = io::read(data, format).map_err(|e| e.to_string())?;
        let depth = undo_depth_for(&mesh, undo_budget);
        Ok(Box::into_raw(Box::new(BrushDocument::new(
            Document::new(mesh).with_undo_depth(depth),
            read_only,
            report,
        ))))
    })
}

/// An empty, editable document: a scratch space, such as for drawing a piece's ghost before it is built.
#[no_mangle]
pub extern "C" fn brush_document_new() -> *mut BrushDocument {
    let report = io::ReadReport::default();
    Box::into_raw(Box::new(BrushDocument::new(
        Document::new(Mesh::default()),
        false,
        report,
    )))
}

/// Closes a document. NULL is fine.
#[no_mangle]
pub unsafe extern "C" fn brush_document_free(doc: *mut BrushDocument) {
    if !doc.is_null() {
        drop(Box::from_raw(doc));
    }
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_read_only(doc: *const BrushDocument) -> bool {
    doc.as_ref().is_none_or(|d| d.read_only)
}

/// What the reader dropped or changed when the file was opened, as JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_read_report(doc: *const BrushDocument) -> *mut c_char {
    let Some(d) = doc.as_ref() else {
        return ptr::null_mut();
    };
    let r = &d.report;
    c_string(
        serde_json::json!({
            "polygonsTriangulated": r.polygons_triangulated,
            "facesDropped": r.faces_dropped,
            "elementsDropped": r.elements_dropped,
            "facePropertiesDropped": r.face_properties_dropped,
        })
        .to_string(),
    )
}

/// Applies one op (JSON, as in `spec/ops.schema.json`). `actor_json` may be NULL. Returns the `Applied` result as
/// JSON. A `sculpt` op opens a stroke that `brush_document_extend_stroke` adds to.
#[no_mangle]
pub unsafe extern "C" fn brush_document_apply(
    doc: *mut BrushDocument,
    op_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let op: Op =
            serde_json::from_str(str_arg(op_json, "op")?).map_err(|e| format!("bad op: {e}"))?;
        let actor = parse_actor(actor_json)?;
        let applied = d.doc.apply(op, actor).map_err(|e| e.to_string());
        d.changed();
        json(&applied?)
    })
}

/// Applies a JSON array of ops in order, all or nothing. A failure names the index of the op that failed.
#[no_mangle]
pub unsafe extern "C" fn brush_document_apply_all(
    doc: *mut BrushDocument,
    ops_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let ops: Vec<Op> =
            serde_json::from_str(str_arg(ops_json, "ops")?).map_err(|e| format!("bad ops: {e}"))?;
        let actor = parse_actor(actor_json)?;
        let applied = d
            .doc
            .apply_all(ops, actor)
            .map_err(|(i, e)| format!("op {i}: {e}"));
        d.changed();
        json(&applied?)
    })
}

unsafe fn parse_actor(actor_json: *const c_char) -> Result<Actor, String> {
    if actor_json.is_null() {
        return Ok(Actor::default());
    }
    serde_json::from_str(str_arg(actor_json, "actor")?).map_err(|e| format!("bad actor: {e}"))
}

/// Adds `count` dabs to the open stroke: `centers` holds 3 doubles per dab, `pressures` one per dab (or NULL for full
/// pressure). The same result as if they had been in the first op, and still one undo step. Returns 1 when the mesh
/// changed, 0 when it did not, and -1 on error (such as no open stroke).
#[no_mangle]
pub unsafe extern "C" fn brush_document_extend_stroke(
    doc: *mut BrushDocument,
    centers: *const f64,
    pressures: *const f64,
    count: usize,
    error: *mut *mut c_char,
) -> i32 {
    guard(error, -1, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        if count == 0 {
            return Ok(0);
        }
        if centers.is_null() {
            return Err("no dab centres".into());
        }
        let c = std::slice::from_raw_parts(centers, count * 3);
        let p = (!pressures.is_null()).then(|| std::slice::from_raw_parts(pressures, count));
        let dabs = (0..count)
            .map(|i| Dab {
                center: [c[i * 3], c[i * 3 + 1], c[i * 3 + 2]],
                pressure: p.map_or(1.0, |p| p[i].clamp(0.0, 1.0)),
            })
            .collect();
        let applied = d.doc.extend_stroke(dabs).map_err(|e| e.to_string());
        d.changed();
        Ok(i32::from(applied?.changed))
    })
}

/// Ends the open stroke, if any.
#[no_mangle]
pub unsafe extern "C" fn brush_document_end_stroke(doc: *mut BrushDocument) {
    if let Some(d) = doc.as_mut() {
        d.doc.end_stroke();
    }
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_undo(doc: *mut BrushDocument) -> bool {
    let Some(d) = doc.as_mut() else { return false };
    let done = !d.read_only && d.doc.undo();
    if done {
        d.changed();
    }
    done
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_redo(doc: *mut BrushDocument) -> bool {
    let Some(d) = doc.as_mut() else { return false };
    let done = !d.read_only && d.doc.redo();
    if done {
        d.changed();
    }
    done
}

/// Back to the mesh as opened or last saved. Undoable.
#[no_mangle]
pub unsafe extern "C" fn brush_document_revert(doc: *mut BrushDocument) -> bool {
    let Some(d) = doc.as_mut() else { return false };
    let done = !d.read_only && d.doc.revert();
    if done {
        d.changed();
    }
    done
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_mark_saved(doc: *mut BrushDocument) {
    if let Some(d) = doc.as_mut() {
        d.doc.mark_saved();
    }
}

/// The document's state as JSON: counts (vertices, faces, splats), bounds, hash, undo and redo, dirty, the last op and the journal length.
#[no_mangle]
pub unsafe extern "C" fn brush_document_state(doc: *const BrushDocument) -> *mut c_char {
    let Some(d) = doc.as_ref() else {
        return ptr::null_mut();
    };
    let m = d.doc.mesh();
    c_string(
        serde_json::json!({
            "vertices": m.vertex_count(),
            "faces": m.face_count(),
            "bounds": m.bounds().map(|(lo, hi)| serde_json::json!({"min": lo, "max": hi})),
            "hash": m.content_hash(),
            "canUndo": d.doc.can_undo(),
            "canRedo": d.doc.can_redo(),
            "dirty": d.doc.is_dirty(),
            "lastOp": d.doc.last_op().map(|o| o.name()),
            "attributes": m.attributes.iter().map(|a| serde_json::json!({"name": a.name, "kind": a.kind})).collect::<Vec<_>>(),
            "splats": m.splats.len(),
            "journalLength": d.doc.journal().entries.len(),
        })
        .to_string(),
    )
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_can_undo(doc: *const BrushDocument) -> bool {
    doc.as_ref()
        .is_some_and(|d| !d.read_only && d.doc.can_undo())
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_can_redo(doc: *const BrushDocument) -> bool {
    doc.as_ref()
        .is_some_and(|d| !d.read_only && d.doc.can_redo())
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_dirty(doc: *const BrushDocument) -> bool {
    doc.as_ref().is_some_and(|d| d.doc.is_dirty())
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_vertex_count(doc: *const BrushDocument) -> usize {
    doc.as_ref().map_or(0, |d| d.doc.mesh().vertex_count())
}

#[no_mangle]
pub unsafe extern "C" fn brush_document_face_count(doc: *const BrushDocument) -> usize {
    doc.as_ref().map_or(0, |d| d.doc.mesh().face_count())
}

/// How many paint splats the mesh carries (see the `splatPaint` op).
#[no_mangle]
pub unsafe extern "C" fn brush_document_splat_count(doc: *const BrushDocument) -> usize {
    doc.as_ref().map_or(0, |d| d.doc.mesh().splats.len())
}

/// Volume, surface area, centroid and watertightness (exact for a closed surface), as JSON:
/// `{"volume", "surfaceArea", "centroid", "closed", "splats"}`.
#[no_mangle]
pub unsafe extern "C" fn brush_document_measure(doc: *const BrushDocument) -> *mut c_char {
    let Some(d) = doc.as_ref() else {
        return ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| d.doc.mesh().measure()))
        .ok()
        .and_then(|m| json(&m).ok())
        .unwrap_or(ptr::null_mut())
}

// Borrowed buffers. Each stays valid until the next call that changes the document (an op, a stroke, undo, redo,
// revert) or frees it. `len` receives the number of elements, not bytes.

unsafe fn borrow<T>(v: &[T], len: *mut usize) -> *const T {
    if !len.is_null() {
        *len = v.len();
    }
    if v.is_empty() {
        ptr::null()
    } else {
        v.as_ptr()
    }
}

/// Vertex positions, 3 floats per vertex (meters, +y up).
#[no_mangle]
pub unsafe extern "C" fn brush_document_positions(
    doc: *const BrushDocument,
    len: *mut usize,
) -> *const f32 {
    match doc.as_ref() {
        Some(d) => borrow(&d.doc.mesh().positions, len),
        None => borrow::<f32>(&[], len),
    }
}

/// Vertex colours, 3 bytes per vertex.
#[no_mangle]
pub unsafe extern "C" fn brush_document_colors(
    doc: *const BrushDocument,
    len: *mut usize,
) -> *const u8 {
    match doc.as_ref() {
        Some(d) => borrow(&d.doc.mesh().colors, len),
        None => borrow::<u8>(&[], len),
    }
}

/// Triangle corners, 3 per face. Empty for a point cloud.
#[no_mangle]
pub unsafe extern "C" fn brush_document_indices(
    doc: *const BrushDocument,
    len: *mut usize,
) -> *const u32 {
    match doc.as_ref() {
        Some(d) => borrow(&d.doc.mesh().indices, len),
        None => borrow::<u32>(&[], len),
    }
}

/// Area-weighted vertex normals, 3 floats per vertex, computed when the mesh has changed since the last call.
#[no_mangle]
pub unsafe extern "C" fn brush_document_normals(
    doc: *mut BrushDocument,
    len: *mut usize,
) -> *const f32 {
    let Some(d) = doc.as_mut() else {
        return borrow::<f32>(&[], len);
    };
    if !d.normals_fresh {
        // A panic here would leave stale normals; recompute next time rather than abort.
        match catch_unwind(AssertUnwindSafe(|| d.doc.vertex_normals())) {
            Ok(n) => {
                d.normals = n;
                d.normals_fresh = true;
            }
            Err(_) => d.normals.clear(),
        }
    }
    borrow(&d.normals, len)
}

/// The splats' geometry, 8 floats per splat: x, y, z, radius, nx, ny, nz, drip. Rebuilt when the mesh has changed
/// since the last call to this or `brush_document_splat_colors`.
#[no_mangle]
pub unsafe extern "C" fn brush_document_splats(
    doc: *mut BrushDocument,
    len: *mut usize,
) -> *const f32 {
    let Some(d) = doc.as_mut() else {
        return borrow::<f32>(&[], len);
    };
    d.refresh_splats();
    borrow(&d.splats, len)
}

/// The splats' colours, 4 bytes per splat: r, g, b, opacity. Borrowed like `brush_document_splats`.
#[no_mangle]
pub unsafe extern "C" fn brush_document_splat_colors(
    doc: *mut BrushDocument,
    len: *mut usize,
) -> *const u8 {
    let Some(d) = doc.as_mut() else {
        return borrow::<u8>(&[], len);
    };
    d.refresh_splats();
    borrow(&d.splat_colors, len)
}

/// The first point where `origin + t * dir` meets the surface. Faces are two-sided. Returns false on a miss.
#[no_mangle]
pub unsafe extern "C" fn brush_document_raycast(
    doc: *mut BrushDocument,
    origin: *const f32,
    dir: *const f32,
    hit: *mut BrushHit,
) -> bool {
    let Some(d) = doc.as_mut() else { return false };
    if origin.is_null() || dir.is_null() {
        return false;
    }
    let o = *(origin as *const [f32; 3]);
    let v = *(dir as *const [f32; 3]);
    let found = catch_unwind(AssertUnwindSafe(|| d.doc.raycast(o, v)))
        .ok()
        .flatten();
    match found {
        Some(h) => {
            if !hit.is_null() {
                *hit = BrushHit {
                    face: h.face,
                    t: h.t,
                    point: h.point,
                    normal: h.normal,
                    barycentric: h.barycentric,
                };
            }
            true
        }
        None => false,
    }
}

/// The mesh as a file: "ply", "obj" or "glb". With `flattened`, placed objects are copied in (for downloads and other
/// tools); without, it is the document's own mesh (what is saved, beside the scene). Free the bytes with
/// `brush_bytes_free(bytes, *len)`.
#[no_mangle]
pub unsafe extern "C" fn brush_document_export(
    doc: *const BrushDocument,
    format: *const c_char,
    flattened: bool,
    len: *mut usize,
    error: *mut *mut c_char,
) -> *mut u8 {
    guard(error, ptr::null_mut(), || {
        let d = doc.as_ref().ok_or("no document")?;
        let f = Format::from_extension(str_arg(format, "format")?)
            .ok_or("format must be ply, obj or glb")?;
        let mesh = if flattened {
            std::borrow::Cow::Owned(d.doc.flattened())
        } else {
            std::borrow::Cow::Borrowed(d.doc.mesh())
        };
        let bytes = io::write(&mesh, f).into_boxed_slice();
        if !len.is_null() {
            *len = bytes.len();
        }
        Ok(Box::into_raw(bytes) as *mut u8)
    })
}

/// Every change, one JSON object per line.
#[no_mangle]
pub unsafe extern "C" fn brush_document_journal(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => c_string(d.doc.journal().to_jsonl()),
        None => ptr::null_mut(),
    }
}

// ---- scene: linked instances of other objects ----

/// The placed objects as JSON: `{"instances": [{"id", "object", "position", "yawDeg", "pitchDeg"?, "rollDeg"?, "scale"}]}`.
#[no_mangle]
pub unsafe extern "C" fn brush_document_scene(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => json(d.doc.scene()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Loads a stored scene (JSON, as `brush_document_scene` writes it) as the saved state. Call right after opening.
#[no_mangle]
pub unsafe extern "C" fn brush_document_load_scene(
    doc: *mut BrushDocument,
    scene_json: *const c_char,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        let scene: Scene = serde_json::from_str(str_arg(scene_json, "scene")?)
            .map_err(|e| format!("bad scene: {e}"))?;
        d.doc.load_scene(scene);
        Ok(true)
    })
}

/// Supplies (or refreshes) the mesh behind an object pointer such as "scan:<id>", from PLY or OBJ bytes. Every
/// instance of it shows the new mesh at once. Works on read-only documents too: it is what they show, not an edit.
#[no_mangle]
pub unsafe extern "C" fn brush_document_set_object(
    doc: *mut BrushDocument,
    object: *const c_char,
    bytes: *const u8,
    len: usize,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        let key = str_arg(object, "object")?.to_string();
        if bytes.is_null() || len == 0 {
            return Err("no bytes".into());
        }
        let data = std::slice::from_raw_parts(bytes, len);
        let format = Format::sniff(data).ok_or("not a PLY or OBJ file")?;
        let (mesh, _) = io::read(data, format).map_err(|e| e.to_string())?;
        d.object_normals.remove(&key);
        d.doc.set_object(key, mesh);
        Ok(true)
    })
}

/// Objects the scene points at whose meshes have not been supplied, as a JSON array of strings.
#[no_mangle]
pub unsafe extern "C" fn brush_document_missing_objects(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => json(&d.doc.missing_objects()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

unsafe fn object_mesh<'a>(doc: *const BrushDocument, object: *const c_char) -> Option<&'a Mesh> {
    let d = doc.as_ref()?;
    d.doc.object(str_arg(object, "object").ok()?)
}

/// An object's vertex positions (3 floats per vertex), borrowed until that object is set again or the document freed.
#[no_mangle]
pub unsafe extern "C" fn brush_document_object_positions(
    doc: *const BrushDocument,
    object: *const c_char,
    len: *mut usize,
) -> *const f32 {
    borrow(
        object_mesh(doc, object).map_or(&[][..], |m| &m.positions),
        len,
    )
}

/// An object's vertex colours (3 bytes per vertex), borrowed like its positions.
#[no_mangle]
pub unsafe extern "C" fn brush_document_object_colors(
    doc: *const BrushDocument,
    object: *const c_char,
    len: *mut usize,
) -> *const u8 {
    borrow(object_mesh(doc, object).map_or(&[][..], |m| &m.colors), len)
}

/// An object's triangle corners (3 per face), borrowed like its positions.
#[no_mangle]
pub unsafe extern "C" fn brush_document_object_indices(
    doc: *const BrushDocument,
    object: *const c_char,
    len: *mut usize,
) -> *const u32 {
    borrow(
        object_mesh(doc, object).map_or(&[][..], |m| &m.indices),
        len,
    )
}

/// An object's vertex normals (3 floats per vertex), borrowed like its positions.
#[no_mangle]
pub unsafe extern "C" fn brush_document_object_normals(
    doc: *mut BrushDocument,
    object: *const c_char,
    len: *mut usize,
) -> *const f32 {
    let Some(d) = doc.as_mut() else {
        return borrow::<f32>(&[], len);
    };
    let Ok(key) = str_arg(object, "object") else {
        return borrow::<f32>(&[], len);
    };
    if !d.object_normals.contains_key(key) {
        let Some(n) = catch_unwind(AssertUnwindSafe(|| d.doc.object_normals(key)))
            .ok()
            .flatten()
        else {
            return borrow::<f32>(&[], len);
        };
        d.object_normals.insert(key.to_string(), n);
    }
    borrow(&d.object_normals[key], len)
}

/// A placed instance's object-to-world transform (column-major 4x4) into `out` (16 floats). False when the instance
/// or its object's mesh is unknown.
#[no_mangle]
pub unsafe extern "C" fn brush_document_instance_transform(
    doc: *const BrushDocument,
    id: *const c_char,
    out: *mut f32,
) -> bool {
    let (Some(d), Ok(id)) = (doc.as_ref(), str_arg(id, "id")) else {
        return false;
    };
    match d.doc.instance_transform(id) {
        Some(m) if !out.is_null() => {
            std::ptr::copy_nonoverlapping(m.as_ptr(), out, 16);
            true
        }
        _ => false,
    }
}

/// Where an object would be drawn for an upright placement (a ghost), as `brush_document_instance_transform` gives it.
#[no_mangle]
pub unsafe extern "C" fn brush_document_object_transform(
    doc: *const BrushDocument,
    object: *const c_char,
    position: *const f64,
    yaw_deg: f64,
    scale: f64,
    out: *mut f32,
) -> bool {
    let (Some(d), Ok(object)) = (doc.as_ref(), str_arg(object, "object")) else {
        return false;
    };
    if position.is_null() || out.is_null() {
        return false;
    }
    let p = *(position as *const [f64; 3]);
    match d.doc.object_transform(object, p, yaw_deg, scale, 0.0, 0.0) {
        Some(m) => {
            std::ptr::copy_nonoverlapping(m.as_ptr(), out, 16);
            true
        }
        None => false,
    }
}

/// The nearest surface along `origin + t * dir`, the document's own mesh or a placed instance. When an instance was
/// hit and `instance` is not NULL, its id is stored there (free with `brush_string_free`); otherwise NULL is stored.
#[no_mangle]
pub unsafe extern "C" fn brush_document_pick(
    doc: *mut BrushDocument,
    origin: *const f32,
    dir: *const f32,
    hit: *mut BrushHit,
    instance: *mut *mut c_char,
) -> bool {
    if !instance.is_null() {
        *instance = ptr::null_mut();
    }
    let Some(d) = doc.as_mut() else { return false };
    if origin.is_null() || dir.is_null() {
        return false;
    }
    let (o, v) = (*(origin as *const [f32; 3]), *(dir as *const [f32; 3]));
    let Some(p) = catch_unwind(AssertUnwindSafe(|| d.doc.pick(o, v)))
        .ok()
        .flatten()
    else {
        return false;
    };
    if !hit.is_null() {
        let h = p.hit;
        *hit = BrushHit {
            face: h.face,
            t: h.t,
            point: h.point,
            normal: h.normal,
            barycentric: h.barycentric,
        };
    }
    if let (Some(id), false) = (p.instance, instance.is_null()) {
        *instance = c_string(id);
    }
    true
}

/// The op (JSON) that builds a piece where the pointer is: `placement_json` is a Placement (piece, point, normal,
/// view, cell, snap, ground, turns, span, scale, color, and object for placing an object). Nothing changes until the
/// op is applied.
#[no_mangle]
pub unsafe extern "C" fn brush_document_place(
    doc: *const BrushDocument,
    placement_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc.as_ref().ok_or("no document")?;
        let p: Placement = serde_json::from_str(str_arg(placement_json, "placement")?)
            .map_err(|e| format!("bad placement: {e}"))?;
        let op: Op = d.doc.place(&p).map_err(|e| e.to_string())?;
        json(&op)
    })
}

// ---- transactions & revision ----

/// Monotonic document revision, incremented on every committed change.
#[no_mangle]
pub unsafe extern "C" fn brush_document_revision(doc: *const BrushDocument) -> u64 {
    doc.as_ref().map_or(0, |d| d.doc.revision())
}

/// Begins a transaction collapsing subsequent mutations into one undo step.
#[no_mangle]
pub unsafe extern "C" fn brush_document_begin_transaction(
    doc: *mut BrushDocument,
    label: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let lbl = if label.is_null() {
            None
        } else {
            Some(str_arg(label, "label")?.to_string())
        };
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        d.doc
            .begin_transaction(lbl, actor)
            .map_err(|e| e.to_string())?;
        Ok(true)
    })
}

/// Commits the open transaction and returns `Applied` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_commit_transaction(
    doc: *mut BrushDocument,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let applied = d.doc.commit_transaction().map_err(|e| e.to_string())?;
        if applied.changed {
            d.changed();
        }
        json(&applied)
    })
}

/// Cancels the open transaction, restoring the pre-transaction state.
#[no_mangle]
pub unsafe extern "C" fn brush_document_cancel_transaction(
    doc: *mut BrushDocument,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let cancelled = d.doc.cancel_transaction().map_err(|e| e.to_string())?;
        if cancelled {
            d.changed();
        }
        Ok(cancelled)
    })
}

// ---- island inspection & detach / join workflows ----

/// Connected component (island) containing `start_face`, as JSON, or NULL if invalid.
#[no_mangle]
pub unsafe extern "C" fn brush_document_connected_component(
    doc: *const BrushDocument,
    start_face: u32,
) -> *mut c_char {
    let Some(d) = doc.as_ref() else {
        return ptr::null_mut();
    };
    match d.doc.mesh().connected_component(start_face) {
        Some(island) => json(&island).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Deletes the connected island containing `start_face`. Returns `Applied` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_delete_island(
    doc: *mut BrushDocument,
    start_face: u32,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let applied = d
            .doc
            .delete_island(start_face, actor)
            .map_err(|e| e.to_string())?;
        if applied.changed {
            d.changed();
        }
        json(&applied)
    })
}

/// Detaches the connected island (or tangled cluster) containing `start_face` into a placed object.
#[no_mangle]
pub unsafe extern "C" fn brush_document_detach_island(
    doc: *mut BrushDocument,
    start_face: u32,
    tangled: bool,
    object_key: *const c_char,
    instance_id: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let key = str_arg(object_key, "object_key")?.to_string();
        let inst = if instance_id.is_null() {
            None
        } else {
            Some(str_arg(instance_id, "instance_id")?.to_string())
        };
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let part = d
            .doc
            .detach_island(start_face, tangled, key.clone(), inst, actor)
            .map_err(|e| e.to_string())?;
        d.object_normals.remove(&key);
        d.changed();
        json(&part)
    })
}

/// Detaches geometry inside a `BoxRegion` (JSON) into a placed object.
#[no_mangle]
pub unsafe extern "C" fn brush_document_detach_box(
    doc: *mut BrushDocument,
    region_json: *const c_char,
    object_key: *const c_char,
    instance_id: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let region: BoxRegion = serde_json::from_str(str_arg(region_json, "region")?)
            .map_err(|e| format!("bad region: {e}"))?;
        let key = str_arg(object_key, "object_key")?.to_string();
        let inst = if instance_id.is_null() {
            None
        } else {
            Some(str_arg(instance_id, "instance_id")?.to_string())
        };
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let part = d
            .doc
            .detach_box(region, key.clone(), inst, actor)
            .map_err(|e| e.to_string())?;
        d.object_normals.remove(&key);
        d.changed();
        json(&part)
    })
}

/// Combines multiple placed instances into one new embedded object + instance.
#[no_mangle]
pub unsafe extern "C" fn brush_document_combine_instances(
    doc: *mut BrushDocument,
    ids_json: *const c_char,
    object_key: *const c_char,
    instance_id: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let ids: Vec<String> = serde_json::from_str(str_arg(ids_json, "ids")?)
            .map_err(|e| format!("bad ids: {e}"))?;
        let key = str_arg(object_key, "object_key")?.to_string();
        let inst = if instance_id.is_null() {
            None
        } else {
            Some(str_arg(instance_id, "instance_id")?.to_string())
        };
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let part = d
            .doc
            .combine_instances(&ids, key.clone(), inst, actor)
            .map_err(|e| e.to_string())?;
        d.object_normals.remove(&key);
        d.changed();
        json(&part)
    })
}

// ---- precision: workplanes, alignment, editable features, persistent constraints (Phase E) ----

/// Current active workplane as JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_workplane(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => json(&d.doc.workplane()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Sets the active workplane from JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_set_workplane(
    doc: *mut BrushDocument,
    workplane_json: *const c_char,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let wp: Workplane = serde_json::from_str(str_arg(workplane_json, "workplane")?)
            .map_err(|e| format!("bad workplane: {e}"))?;
        d.doc.set_workplane(wp);
        Ok(true)
    })
}

/// Aligns selected instance IDs (JSON array) along `axis` ("x"|"y"|"z") and `mode` ("min"|"center"|"max").
/// Returns the number of instances moved, or -1 on error.
#[no_mangle]
pub unsafe extern "C" fn brush_document_align_selection(
    doc: *mut BrushDocument,
    ids_json: *const c_char,
    axis_json: *const c_char,
    mode_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> i32 {
    guard(error, -1, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let ids: Vec<String> = serde_json::from_str(str_arg(ids_json, "ids")?)
            .map_err(|e| format!("bad ids: {e}"))?;
        let axis: AlignAxis = serde_json::from_str(str_arg(axis_json, "axis")?)
            .map_err(|e| format!("bad axis: {e}"))?;
        let mode: AlignMode = serde_json::from_str(str_arg(mode_json, "mode")?)
            .map_err(|e| format!("bad mode: {e}"))?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let n = d
            .doc
            .align_selection(&ids, axis, mode, actor)
            .map_err(|e| e.to_string())?;
        Ok(n as i32)
    })
}

/// Evenly distributes selected instance IDs (JSON array) along `axis` ("x"|"y"|"z").
/// Returns the number of instances moved, or -1 on error.
#[no_mangle]
pub unsafe extern "C" fn brush_document_distribute_selection(
    doc: *mut BrushDocument,
    ids_json: *const c_char,
    axis_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> i32 {
    guard(error, -1, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let ids: Vec<String> = serde_json::from_str(str_arg(ids_json, "ids")?)
            .map_err(|e| format!("bad ids: {e}"))?;
        let axis: AlignAxis = serde_json::from_str(str_arg(axis_json, "axis")?)
            .map_err(|e| format!("bad axis: {e}"))?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let n = d
            .doc
            .distribute_selection(&ids, axis, actor)
            .map_err(|e| e.to_string())?;
        Ok(n as i32)
    })
}

/// Lists all parametric construction features as JSON array.
#[no_mangle]
pub unsafe extern "C" fn brush_document_list_features(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => json(&d.doc.list_features()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Adds a new editable feature from `FeatureSpec` JSON. Returns `FeatureRecord` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_add_feature(
    doc: *mut BrushDocument,
    spec_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let spec: FeatureSpec = serde_json::from_str(str_arg(spec_json, "spec")?)
            .map_err(|e| format!("bad feature spec: {e}"))?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let rec = d.doc.add_feature(spec, actor).map_err(|e| e.to_string())?;
        d.object_normals.remove(&rec.object_key);
        d.changed();
        json(&rec)
    })
}

/// Updates an existing feature's `FeatureKind` JSON. Returns `SolveReport` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_update_feature(
    doc: *mut BrushDocument,
    id: *const c_char,
    kind_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let fid = str_arg(id, "id")?;
        let kind: FeatureKind = serde_json::from_str(str_arg(kind_json, "kind")?)
            .map_err(|e| format!("bad feature kind: {e}"))?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let report = d
            .doc
            .update_feature(fid, kind, actor)
            .map_err(|e| e.to_string())?;
        d.object_normals.clear();
        d.changed();
        json(&report)
    })
}

/// Validates all features and returns updated `FeatureRecord[]` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_validate_features(doc: *mut BrushDocument) -> *mut c_char {
    match doc.as_mut() {
        Some(d) => json(&d.doc.validate_features()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Repairs a broken feature by ID. Returns `FeatureRecord` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_repair_feature(
    doc: *mut BrushDocument,
    id: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let fid = str_arg(id, "id")?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let rec = d.doc.repair_feature(fid, actor).map_err(|e| e.to_string())?;
        d.object_normals.remove(&rec.object_key);
        d.changed();
        json(&rec)
    })
}

/// Bakes a feature into the base mesh. Returns `FeatureRecord` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_bake_feature(
    doc: *mut BrushDocument,
    id: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let fid = str_arg(id, "id")?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let rec = d.doc.bake_feature(fid, actor).map_err(|e| e.to_string())?;
        d.changed();
        json(&rec)
    })
}

/// Lists all persistent constraints as JSON array.
#[no_mangle]
pub unsafe extern "C" fn brush_document_list_constraints(doc: *const BrushDocument) -> *mut c_char {
    match doc.as_ref() {
        Some(d) => json(&d.doc.list_constraints()).unwrap_or(ptr::null_mut()),
        None => ptr::null_mut(),
    }
}

/// Adds a persistent constraint (`ConstraintRecord` JSON) and solves. Returns `SolveReport` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_add_constraint(
    doc: *mut BrushDocument,
    constraint_json: *const c_char,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let c: ConstraintRecord = serde_json::from_str(str_arg(constraint_json, "constraint")?)
            .map_err(|e| format!("bad constraint: {e}"))?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let report = d.doc.add_constraint(c, actor).map_err(|e| e.to_string())?;
        json(&report)
    })
}

/// Removes a constraint by ID.
#[no_mangle]
pub unsafe extern "C" fn brush_document_remove_constraint(
    doc: *mut BrushDocument,
    id: *const c_char,
    error: *mut *mut c_char,
) -> bool {
    guard(error, false, || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let cid = str_arg(id, "id")?;
        Ok(d.doc.remove_constraint(cid))
    })
}

/// Solves all persistent constraints. Returns `SolveReport` JSON.
#[no_mangle]
pub unsafe extern "C" fn brush_document_solve_constraints(
    doc: *mut BrushDocument,
    actor_json: *const c_char,
    error: *mut *mut c_char,
) -> *mut c_char {
    guard(error, ptr::null_mut(), || {
        let d = doc_mut(doc)?;
        d.writable()?;
        let actor: Actor = if actor_json.is_null() {
            Actor::default()
        } else {
            serde_json::from_str(str_arg(actor_json, "actor")?)
                .map_err(|e| format!("bad actor: {e}"))?
        };
        let report = d.doc.solve_constraints(actor).map_err(|e| e.to_string())?;
        json(&report)
    })
}

#[cfg(test)]
mod tests;
