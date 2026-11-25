//! # brush-io
//!
//! Reading and writing meshes. PLY is the native format: every vertex property that is not position or colour is
//! kept as a named attribute and written back with its original type, so labels, confidences and normals from a
//! scanner or an AI model survive an edit. OBJ and GLB are for handing a mesh to other tools.
//!
//! Coordinates are not converted: files are expected to be meters, +y up.

mod glb;
mod obj;
mod ply;
pub mod project;

pub use glb::write_glb;
pub use obj::{read_obj, write_obj};
pub use ply::{read_ply, write_ply, PlyOptions};
pub use project::{
    is_project_container, read_project, write_project, ProjectContainer, PROJECT_MAGIC,
    PROJECT_VERSION,
};

use brush_mesh::Mesh;

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("not a {0} file")]
    NotFormat(&'static str),
    #[error("{0}")]
    Malformed(String),
    #[error("the file is truncated")]
    Truncated,
    #[error(transparent)]
    Mesh(#[from] brush_mesh::MeshError),
}

pub(crate) fn bad(msg: impl Into<String>) -> IoError {
    IoError::Malformed(msg.into())
}

/// Things a reader dropped or changed, for telling the user.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ReadReport {
    /// Polygons with more than three corners, split into triangle fans.
    pub polygons_triangulated: usize,
    /// Faces with fewer than three corners, or with a corner that is not a vertex.
    pub faces_dropped: usize,
    /// Elements other than vertex and face (edges, materials...), not kept.
    pub elements_dropped: Vec<String>,
    /// Face properties other than the vertex list, not kept.
    pub face_properties_dropped: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Ply,
    Obj,
    Glb,
}

impl Format {
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext.to_ascii_lowercase().as_str() {
            "ply" => Some(Format::Ply),
            "obj" => Some(Format::Obj),
            "glb" => Some(Format::Glb),
            _ => None,
        }
    }

    /// Sniffs the format from the first bytes.
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(b"ply") {
            Some(Format::Ply)
        } else if bytes.starts_with(b"glTF") {
            Some(Format::Glb)
        } else if std::str::from_utf8(&bytes[..bytes.len().min(4096)])
            .is_ok_and(|s| s.lines().any(|l| l.trim_start().starts_with("v ")))
        {
            Some(Format::Obj)
        } else {
            None
        }
    }
}

/// Reads PLY or OBJ.
pub fn read(bytes: &[u8], format: Format) -> Result<(Mesh, ReadReport), IoError> {
    match format {
        Format::Ply => read_ply(bytes),
        Format::Obj => read_obj(bytes),
        Format::Glb => Err(bad("reading GLB is not supported yet; export PLY or OBJ")),
    }
}

pub fn write(mesh: &Mesh, format: Format) -> Vec<u8> {
    match format {
        Format::Ply => write_ply(mesh, PlyOptions::default()),
        Format::Obj => write_obj(mesh).into_bytes(),
        Format::Glb => write_glb(mesh, "surface"),
    }
}
