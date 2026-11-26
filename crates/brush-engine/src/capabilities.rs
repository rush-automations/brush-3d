//! Versioned capability discovery for applications and extensions (Phase F).

use brush_io::project::PROJECT_VERSION;
use brush_mesh::OPS_VERSION;
use serde::{Deserialize, Serialize};

/// Supported read/write file formats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCapabilities {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

/// Platform-level feature flags applications and extensions can query at runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub transactions: bool,
    pub previews: bool,
    pub revision_guards: bool,
    pub workplanes: bool,
    pub editable_features: bool,
    pub persistent_constraints: bool,
    pub splats: bool,
    pub linked_instances: bool,
}

/// Machine-readable capability manifest describing what this engine build supports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub engine_version: String,
    pub ops_version: String,
    pub project_format_version: u32,
    pub formats: FormatCapabilities,
    pub features: FeatureFlags,
    pub feature_kinds: Vec<String>,
    pub constraint_kinds: Vec<String>,
}

/// Returns the versioned capability manifest for this `brush-engine` build.
pub fn capabilities() -> Capabilities {
    Capabilities {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        ops_version: OPS_VERSION.to_string(),
        project_format_version: PROJECT_VERSION,
        formats: FormatCapabilities {
            read: vec!["brush".into(), "ply".into(), "obj".into(), "glb".into()],
            write: vec!["brush".into(), "ply".into(), "obj".into(), "glb".into()],
        },
        features: FeatureFlags {
            transactions: true,
            previews: true,
            revision_guards: true,
            workplanes: true,
            editable_features: true,
            persistent_constraints: true,
            splats: true,
            linked_instances: true,
        },
        feature_kinds: vec![
            "box".into(),
            "cylinder".into(),
            "extrusion".into(),
            "linearPattern".into(),
        ],
        constraint_kinds: vec![
            "equalDimension".into(),
            "coincidentOffset".into(),
            "fixedDimension".into(),
        ],
    }
}
