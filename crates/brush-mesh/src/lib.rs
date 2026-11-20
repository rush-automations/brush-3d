//! # brush-mesh
//!
//! The geometry kernel of the Brush engine: an indexed triangle [`Mesh`] with vertex colours and named per-vertex
//! attributes, and the typed [`Op`]s that edit it. Pure Rust, no I/O and no GPU, so it builds for every target
//! (native, wasm32, iOS, Android) and gives bit-identical results on all of them.
//!
//! Coordinates are meters, +y up. Regions are oriented boxes: centre, full size and yaw in degrees about +y.
//!
//! ```
//! use brush_mesh::{Mesh, Op, op::Context};
//! let mut mesh = Mesh::default();
//! let op: Op = serde_json::from_str(r#"{"op":"addSphere","center":[0,1,0],"radii":[1,1,1]}"#).unwrap();
//! let out = op.apply(&mut mesh, Context::default()).unwrap();
//! assert_eq!(out.faces, mesh.face_count() as u64);
//! ```

// The kernels index several parallel arrays at once; index loops read closer to the maths than zipped iterators.
#![allow(clippy::needless_range_loop)]

pub mod bvh;
pub mod detail;
pub mod kernel;
pub mod math;
pub mod mesh;
pub mod op;
pub mod region;
pub mod sculpt;
pub mod simplify;
pub mod topo;

pub use bvh::{Bvh, Hit};
pub use mesh::{Attribute, Mesh, MeshError, ScalarKind};
pub use op::{Op, OpError, OpOutcome, OPS_VERSION};
pub use region::BoxRegion;
pub use sculpt::{Brush, Dab};

/// The JSON Schema of [`Op`]: the public contract for anything that produces edits, AI models included.
pub fn ops_schema() -> serde_json::Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(Op)).expect("schema serialises");
    schema["$id"] = serde_json::Value::String(format!(
        "https://brush-engine.dev/schema/ops/{OPS_VERSION}.json"
    ));
    schema["title"] = serde_json::Value::String("Brush engine operation".into());
    schema["x-ops-version"] = serde_json::Value::String(OPS_VERSION.into());
    schema
}
