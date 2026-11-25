//! Versioned `.brush` project container: packages unmodified binary PLY meshes (surface, optional baseline, and
//! linked object assets) alongside scene placement metadata, the append-only edit journal, document revision, and
//! optional design-intent features.
//!
//! Wire layout (little-endian):
//! - `0..8`:   `PROJECT_MAGIC` (`b"BRSH\x00\x01\r\n"`)
//! - `8..12`:  `u32` container format version (`PROJECT_VERSION = 1`)
//! - `12..16`: `u32` byte length of the UTF-8 JSON manifest (`manifest_len`)
//! - `16 .. 16 + manifest_len`: JSON manifest indexing each binary PLY blob by `(offset, length)`
//! - `16 + manifest_len .. EOF`: concatenated binary PLY blobs

use std::collections::BTreeMap;

use brush_mesh::{Mesh, OPS_VERSION};
use serde::{Deserialize, Serialize};

use crate::ply::{read_ply, write_ply, PlyOptions};
use crate::{bad, IoError};

/// 8-byte file signature (catches text-mode CRLF mangling just like PNG's header).
pub const PROJECT_MAGIC: &[u8; 8] = b"BRSH\x00\x01\r\n";

/// Current container format version.
pub const PROJECT_VERSION: u32 = 1;

const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

/// Everything needed to save and reopen an editing session without flattening or losing history.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectContainer {
    pub version: u32,
    pub ops_version: String,
    pub revision: u64,
    pub surface: Mesh,
    pub original_surface: Option<Mesh>,
    pub scene_json: String,
    pub original_scene_json: Option<String>,
    pub objects: BTreeMap<String, Mesh>,
    pub journal_jsonl: String,
    pub metadata: serde_json::Value,
    pub features: Vec<serde_json::Value>,
}

impl Default for ProjectContainer {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            ops_version: OPS_VERSION.to_string(),
            revision: 0,
            surface: Mesh::default(),
            original_surface: None,
            scene_json: "{\"instances\":[]}".to_string(),
            original_scene_json: None,
            objects: BTreeMap::new(),
            journal_jsonl: String::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            features: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u32,
    ops_version: String,
    revision: u64,
    scene_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_scene_json: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    journal_jsonl: String,
    #[serde(default = "empty_object")]
    metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    features: Vec<serde_json::Value>,
    surface: BlobSlice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_surface: Option<BlobSlice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    objects: Vec<NamedBlobSlice>,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct BlobSlice {
    offset: u64,
    length: u64,
}

#[derive(Serialize, Deserialize)]
struct NamedBlobSlice {
    key: String,
    offset: u64,
    length: u64,
}

/// True when `bytes` begins with the `.brush` project container signature.
pub fn is_project_container(bytes: &[u8]) -> bool {
    bytes.starts_with(PROJECT_MAGIC)
}

fn push_ply(payload: &mut Vec<u8>, mesh: &Mesh) -> BlobSlice {
    let offset = payload.len() as u64;
    let ply = write_ply(mesh, PlyOptions::default());
    let length = ply.len() as u64;
    payload.extend_from_slice(&ply);
    BlobSlice { offset, length }
}

fn slice_ply(payload: &[u8], slice: BlobSlice) -> Result<Mesh, IoError> {
    let start = usize::try_from(slice.offset).map_err(|_| IoError::Truncated)?;
    let len = usize::try_from(slice.length).map_err(|_| IoError::Truncated)?;
    let end = start.checked_add(len).ok_or(IoError::Truncated)?;
    if end > payload.len() {
        return Err(IoError::Truncated);
    }
    let (mesh, _) = read_ply(&payload[start..end])?;
    Ok(mesh)
}

/// Serialises a [`ProjectContainer`] into deterministic `.brush` container bytes.
pub fn write_project(project: &ProjectContainer) -> Vec<u8> {
    let mut payload = Vec::new();
    let surface = push_ply(&mut payload, &project.surface);
    let original_surface = project
        .original_surface
        .as_ref()
        .map(|m| push_ply(&mut payload, m));
    let mut objects = Vec::with_capacity(project.objects.len());
    for (key, mesh) in &project.objects {
        let s = push_ply(&mut payload, mesh);
        objects.push(NamedBlobSlice {
            key: key.clone(),
            offset: s.offset,
            length: s.length,
        });
    }

    let manifest = Manifest {
        format: "brush-project".to_string(),
        version: PROJECT_VERSION,
        ops_version: project.ops_version.clone(),
        revision: project.revision,
        scene_json: project.scene_json.clone(),
        original_scene_json: project.original_scene_json.clone(),
        journal_jsonl: project.journal_jsonl.clone(),
        metadata: project.metadata.clone(),
        features: project.features.clone(),
        surface,
        original_surface,
        objects,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).expect("project manifest serialises");
    let manifest_len = u32::try_from(manifest_bytes.len()).expect("manifest fits u32");

    let mut out = Vec::with_capacity(16 + manifest_bytes.len() + payload.len());
    out.extend_from_slice(PROJECT_MAGIC);
    out.extend_from_slice(&PROJECT_VERSION.to_le_bytes());
    out.extend_from_slice(&manifest_len.to_le_bytes());
    out.extend_from_slice(&manifest_bytes);
    out.extend_from_slice(&payload);
    out
}

/// Reads a `.brush` project container from bytes.
pub fn read_project(bytes: &[u8]) -> Result<ProjectContainer, IoError> {
    if bytes.len() < 16 {
        if bytes.starts_with(b"BRSH") {
            return Err(IoError::Truncated);
        }
        return Err(IoError::NotFormat("brush project"));
    }
    if &bytes[0..8] != PROJECT_MAGIC {
        return Err(IoError::NotFormat("brush project"));
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version == 0 || version > PROJECT_VERSION {
        return Err(bad(format!(
            "unsupported brush project container version {version} (expected 1..={PROJECT_VERSION})"
        )));
    }
    let manifest_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if manifest_len > MAX_MANIFEST_BYTES {
        return Err(bad("brush project manifest exceeds size limit"));
    }
    let payload_start = 16usize
        .checked_add(manifest_len)
        .ok_or(IoError::Truncated)?;
    if payload_start > bytes.len() {
        return Err(IoError::Truncated);
    }
    let manifest: Manifest = serde_json::from_slice(&bytes[16..payload_start])
        .map_err(|e| bad(format!("invalid brush project manifest: {e}")))?;
    if manifest.format != "brush-project" {
        return Err(bad(format!(
            "unexpected project manifest format {:?}",
            manifest.format
        )));
    }

    let payload = &bytes[payload_start..];
    let surface = slice_ply(payload, manifest.surface)?;
    let original_surface = match manifest.original_surface {
        Some(s) => Some(slice_ply(payload, s)?),
        None => None,
    };
    let mut objects = BTreeMap::new();
    for entry in manifest.objects {
        let mesh = slice_ply(
            payload,
            BlobSlice {
                offset: entry.offset,
                length: entry.length,
            },
        )?;
        objects.insert(entry.key, mesh);
    }

    Ok(ProjectContainer {
        version: manifest.version,
        ops_version: manifest.ops_version,
        revision: manifest.revision,
        surface,
        original_surface,
        scene_json: manifest.scene_json,
        original_scene_json: manifest.original_scene_json,
        objects,
        journal_jsonl: manifest.journal_jsonl,
        metadata: manifest.metadata,
        features: manifest.features,
    })
}
