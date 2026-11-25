//! Precision direct editing, editable construction features with explicit dependencies,
//! and persistent constraints with conflict reporting (Phase E).
//!
//! Evaluated geometry always remains ordinary mesh geometry (stored as embedded objects +
//! placed instances in the [`Document`]), while optional parametric intent lives in the
//! document's persisted `features` metadata inside `.brush` containers.

use std::collections::{BTreeMap, BTreeSet};

use brush_mesh::{BoxRegion, Mesh, Op, OpError};
use serde::{Deserialize, Serialize};

use crate::{Actor, Document, Instance};

/// Coordinate axis for alignment, distribution, and constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignAxis {
    X,
    Y,
    Z,
}

impl AlignAxis {
    fn index(self) -> usize {
        match self {
            AlignAxis::X => 0,
            AlignAxis::Y => 1,
            AlignAxis::Z => 2,
        }
    }
}

/// Target alignment edge/center along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignMode {
    Min,
    Center,
    Max,
}

/// Euclidean distance and per-axis delta between two 3D points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistanceMeasurement {
    pub distance: f32,
    pub delta: [f32; 3],
}

/// Measures the exact 3D distance and axis deltas from `a` to `b`.
pub fn measure_distance(a: [f32; 3], b: [f32; 3]) -> DistanceMeasurement {
    let delta = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let distance = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    DistanceMeasurement { distance, delta }
}

/// A 2D construction plane in 3D world space for constrained numeric editing and sketching.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workplane {
    pub id: String,
    pub name: String,
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub x_axis: [f32; 3],
    pub y_axis: [f32; 3],
}

impl Default for Workplane {
    fn default() -> Self {
        Self::xz(0.0)
    }
}

impl Workplane {
    /// Horizontal floor workplane at elevation `y`.
    pub fn xz(y: f32) -> Self {
        Self {
            id: "wp:xz".to_string(),
            name: format!("Floor (Y = {y:.2}m)"),
            origin: [0.0, y, 0.0],
            normal: [0.0, 1.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
        }
    }

    /// Vertical front workplane at `z`.
    pub fn xy(z: f32) -> Self {
        Self {
            id: "wp:xy".to_string(),
            name: format!("Front (Z = {z:.2}m)"),
            origin: [0.0, 0.0, z],
            normal: [0.0, 0.0, 1.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
        }
    }

    /// Vertical side workplane at `x`.
    pub fn yz(x: f32) -> Self {
        Self {
            id: "wp:yz".to_string(),
            name: format!("Side (X = {x:.2}m)"),
            origin: [x, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            x_axis: [0.0, 0.0, 1.0],
            y_axis: [0.0, 1.0, 0.0],
        }
    }

    /// Projects a 3D world point onto this workplane's `(u, v)` coordinates.
    pub fn project(&self, point: [f32; 3]) -> [f32; 2] {
        let d = [
            point[0] - self.origin[0],
            point[1] - self.origin[1],
            point[2] - self.origin[2],
        ];
        let u = d[0] * self.x_axis[0] + d[1] * self.x_axis[1] + d[2] * self.x_axis[2];
        let v = d[0] * self.y_axis[0] + d[1] * self.y_axis[1] + d[2] * self.y_axis[2];
        [u, v]
    }

    /// Maps 2D workplane coordinates `(u, v)` back to 3D world space.
    pub fn unproject(&self, u: f32, v: f32) -> [f32; 3] {
        [
            self.origin[0] + self.x_axis[0] * u + self.y_axis[0] * v,
            self.origin[1] + self.x_axis[1] * u + self.y_axis[1] * v,
            self.origin[2] + self.x_axis[2] * u + self.y_axis[2] * v,
        ]
    }

    /// Snaps `(u, v)` to `step` and returns the snapped 3D world point.
    pub fn snap_2d(&self, u: f32, v: f32, step: f32) -> [f32; 3] {
        if step <= 0.0 {
            return self.unproject(u, v);
        }
        let su = (u / step).round() * step;
        let sv = (v / step).round() * step;
        self.unproject(su, sv)
    }
}

/// Computes `Op::MoveObject` operations aligning the selected instances along `axis`.
pub fn align_instances_ops(
    instances: &[Instance],
    ids: &[String],
    axis: AlignAxis,
    mode: AlignMode,
) -> Vec<Op> {
    let selected: Vec<&Instance> = ids
        .iter()
        .filter_map(|id| instances.iter().find(|i| &i.id == id))
        .collect();
    if selected.len() < 2 {
        return Vec::new();
    }
    let k = axis.index();
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for inst in &selected {
        lo = lo.min(inst.position[k]);
        hi = hi.max(inst.position[k]);
    }
    let target = match mode {
        AlignMode::Min => lo,
        AlignMode::Center => (lo + hi) * 0.5,
        AlignMode::Max => hi,
    };
    selected
        .into_iter()
        .map(|inst| {
            let mut pos = inst.position;
            pos[k] = target;
            Op::MoveObject {
                id: inst.id.clone(),
                position: pos,
                yaw_deg: inst.yaw_deg,
                pitch_deg: inst.pitch_deg,
                roll_deg: inst.roll_deg,
                scale: inst.scale,
            }
        })
        .collect()
}

/// Computes `Op::MoveObject` operations evenly distributing >= 3 instances along `axis`.
pub fn distribute_instances_ops(
    instances: &[Instance],
    ids: &[String],
    axis: AlignAxis,
) -> Vec<Op> {
    let mut selected: Vec<&Instance> = ids
        .iter()
        .filter_map(|id| instances.iter().find(|i| &i.id == id))
        .collect();
    if selected.len() < 3 {
        return Vec::new();
    }
    let k = axis.index();
    selected.sort_by(|a, b| {
        a.position[k]
            .partial_cmp(&b.position[k])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let lo = selected.first().unwrap().position[k];
    let hi = selected.last().unwrap().position[k];
    let step = (hi - lo) / ((selected.len() - 1) as f64);
    selected
        .into_iter()
        .enumerate()
        .map(|(idx, inst)| {
            let mut pos = inst.position;
            pos[k] = lo + step * (idx as f64);
            Op::MoveObject {
                id: inst.id.clone(),
                position: pos,
                yaw_deg: inst.yaw_deg,
                pitch_deg: inst.pitch_deg,
                roll_deg: inst.roll_deg,
                scale: inst.scale,
            }
        })
        .collect()
}

/// Parameterized construction feature definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FeatureKind {
    Box {
        center: [f32; 3],
        width: f32,
        height: f32,
        depth: f32,
        #[serde(default)]
        yaw_deg: f32,
        #[serde(default = "default_color")]
        color: [u8; 3],
    },
    Cylinder {
        center: [f32; 3],
        radius: f32,
        height: f32,
        #[serde(default = "default_segments")]
        segments: u32,
        #[serde(default = "default_color")]
        color: [u8; 3],
    },
    Extrusion {
        polygon: Vec<[f32; 2]>,
        y0: f32,
        height: f32,
        #[serde(default = "default_color")]
        color: [u8; 3],
    },
    LinearPattern {
        source_feature: String,
        count: u32,
        step: [f32; 3],
    },
}

fn default_color() -> [u8; 3] {
    [204, 204, 204]
}

fn default_segments() -> u32 {
    24
}

impl FeatureKind {
    pub fn origin(&self) -> Option<[f32; 3]> {
        match self {
            FeatureKind::Box { center, .. } | FeatureKind::Cylinder { center, .. } => Some(*center),
            FeatureKind::Extrusion { polygon, y0, .. } => {
                if polygon.is_empty() {
                    return None;
                }
                let n = polygon.len() as f32;
                let cx = polygon.iter().map(|p| p[0]).sum::<f32>() / n;
                let cz = polygon.iter().map(|p| p[1]).sum::<f32>() / n;
                Some([cx, *y0, cz])
            }
            FeatureKind::LinearPattern { .. } => None,
        }
    }

    pub fn set_origin(&mut self, next: [f32; 3]) -> bool {
        match self {
            FeatureKind::Box { center, .. } | FeatureKind::Cylinder { center, .. } => {
                *center = next;
                true
            }
            FeatureKind::Extrusion { polygon, y0, .. } => {
                if polygon.is_empty() {
                    return false;
                }
                let n = polygon.len() as f32;
                let cx = polygon.iter().map(|p| p[0]).sum::<f32>() / n;
                let cz = polygon.iter().map(|p| p[1]).sum::<f32>() / n;
                let dx = next[0] - cx;
                let dz = next[2] - cz;
                for p in polygon.iter_mut() {
                    p[0] += dx;
                    p[1] += dz;
                }
                *y0 = next[1];
                true
            }
            FeatureKind::LinearPattern { .. } => false,
        }
    }

    pub fn get_param(&self, name: &str) -> Option<f32> {
        match (self, name) {
            (FeatureKind::Box { width, .. }, "width") => Some(*width),
            (FeatureKind::Box { height, .. }, "height") => Some(*height),
            (FeatureKind::Box { depth, .. }, "depth") => Some(*depth),
            (FeatureKind::Box { yaw_deg, .. }, "yawDeg") => Some(*yaw_deg),
            (FeatureKind::Cylinder { radius, .. }, "radius") => Some(*radius),
            (FeatureKind::Cylinder { height, .. }, "height") => Some(*height),
            (FeatureKind::Extrusion { height, .. }, "height") => Some(*height),
            (FeatureKind::Extrusion { y0, .. }, "y0") => Some(*y0),
            (FeatureKind::LinearPattern { count, .. }, "count") => Some(*count as f32),
            _ => None,
        }
    }

    pub fn set_param(&mut self, name: &str, value: f32) -> Result<bool, OpError> {
        if !value.is_finite() {
            return Err(OpError::Invalid("parameter must be finite".into()));
        }
        let changed = match (self, name) {
            (FeatureKind::Box { width, .. }, "width") if value > 0.0 => {
                let c = (*width - value).abs() > 1e-6;
                *width = value;
                c
            }
            (FeatureKind::Box { height, .. }, "height") if value > 0.0 => {
                let c = (*height - value).abs() > 1e-6;
                *height = value;
                c
            }
            (FeatureKind::Box { depth, .. }, "depth") if value > 0.0 => {
                let c = (*depth - value).abs() > 1e-6;
                *depth = value;
                c
            }
            (FeatureKind::Box { yaw_deg, .. }, "yawDeg") => {
                let c = (*yaw_deg - value).abs() > 1e-6;
                *yaw_deg = value;
                c
            }
            (FeatureKind::Cylinder { radius, .. }, "radius") if value > 0.0 => {
                let c = (*radius - value).abs() > 1e-6;
                *radius = value;
                c
            }
            (FeatureKind::Cylinder { height, .. }, "height") if value > 0.0 => {
                let c = (*height - value).abs() > 1e-6;
                *height = value;
                c
            }
            (FeatureKind::Extrusion { height, .. }, "height") if value > 0.0 => {
                let c = (*height - value).abs() > 1e-6;
                *height = value;
                c
            }
            (FeatureKind::Extrusion { y0, .. }, "y0") => {
                let c = (*y0 - value).abs() > 1e-6;
                *y0 = value;
                c
            }
            (FeatureKind::LinearPattern { count, .. }, "count") if value >= 1.0 => {
                let next = value.round() as u32;
                let c = *count != next;
                *count = next;
                c
            }
            _ => {
                return Err(OpError::Invalid(format!(
                    "unsupported or out-of-range parameter {name:?} = {value}"
                )));
            }
        };
        Ok(changed)
    }

    pub fn dependencies(&self) -> Vec<String> {
        match self {
            FeatureKind::LinearPattern { source_feature, .. } => vec![source_feature.clone()],
            _ => Vec::new(),
        }
    }
}

/// Lifecycle state of a parametric construction feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum FeatureStatus {
    Valid,
    Invalidated { reason: String },
    Baked,
}

/// One persisted construction feature in the document's feature graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureRecord {
    pub id: String,
    pub name: String,
    pub kind: FeatureKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub object_key: String,
    #[serde(default)]
    pub instance_ids: Vec<String>,
    pub status: FeatureStatus,
    #[serde(default)]
    pub evaluated_hash: String,
}

/// Spec passed when creating a new construction feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSpec {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub kind: FeatureKind,
}

/// Persistent parametric constraint between features or on a feature parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConstraintKind {
    /// Forces `feature_b.param_b = feature_a.param_a`.
    EqualDimension {
        feature_a: String,
        param_a: String,
        feature_b: String,
        param_b: String,
    },
    /// Keeps `target_feature` positioned at `source_feature.origin + offset`.
    CoincidentOffset {
        source_feature: String,
        target_feature: String,
        offset: [f32; 3],
    },
    /// Locks `feature.param` to an exact numeric value (detects conflicts with other constraints).
    FixedDimension {
        feature: String,
        param: String,
        value: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintRecord {
    pub id: String,
    pub kind: ConstraintKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintConflict {
    pub constraint_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveReport {
    pub updated_features: Vec<String>,
    pub conflicts: Vec<ConstraintConflict>,
}

/// Complete precision state stored inside `Document.features` (persisted in `.brush`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecisionState {
    #[serde(default)]
    pub workplane: Workplane,
    #[serde(default)]
    pub features: Vec<FeatureRecord>,
    #[serde(default)]
    pub constraints: Vec<ConstraintRecord>,
}

const PRECISION_ENTRY_KEY: &str = "brushPrecision";

impl PrecisionState {
    pub(crate) fn from_entries(entries: &[serde_json::Value]) -> Self {
        for entry in entries {
            if let Some(inner) = entry.get(PRECISION_ENTRY_KEY) {
                if let Ok(parsed) = serde_json::from_value(inner.clone()) {
                    return parsed;
                }
            }
        }
        Self::default()
    }

    pub(crate) fn write_into(&self, entries: &mut Vec<serde_json::Value>) {
        let wrapped = serde_json::json!({ PRECISION_ENTRY_KEY: self });
        if let Some(slot) = entries
            .iter_mut()
            .find(|e| e.get(PRECISION_ENTRY_KEY).is_some())
        {
            *slot = wrapped;
        } else {
            entries.push(wrapped);
        }
    }
}

/// Evaluates a primitive/extrusion feature into its local-space mesh and world-space placement.
fn evaluate_primitive(kind: &FeatureKind) -> Result<(Mesh, [f64; 3], f64), OpError> {
    let mut temp = Document::new(Mesh::default());
    match kind {
        FeatureKind::Box {
            center,
            width,
            height,
            depth,
            yaw_deg,
            color,
        } => {
            if *width <= 0.0 || *height <= 0.0 || *depth <= 0.0 {
                return Err(OpError::Invalid("box dimensions must be positive".into()));
            }
            temp.apply(
                Op::AddBox {
                    region: BoxRegion {
                        center: [0.0, (*height as f64) * 0.5, 0.0],
                        size: [*width as f64, *height as f64, *depth as f64],
                        yaw_deg: 0.0,
                    },
                    color: *color,
                },
                Actor::default(),
            )?;
            Ok((
                temp.mesh().clone(),
                [center[0] as f64, center[1] as f64, center[2] as f64],
                *yaw_deg as f64,
            ))
        }
        FeatureKind::Cylinder {
            center,
            radius,
            height,
            segments,
            color,
        } => {
            if *radius <= 0.0 || *height <= 0.0 {
                return Err(OpError::Invalid(
                    "cylinder radius and height must be positive".into(),
                ));
            }
            temp.apply(
                Op::AddFrustum {
                    base: [0.0, 0.0, 0.0],
                    radius_bottom: *radius as f64,
                    radius_top: *radius as f64,
                    height: *height as f64,
                    segments: (*segments).max(6),
                    color: *color,
                },
                Actor::default(),
            )?;
            Ok((
                temp.mesh().clone(),
                [center[0] as f64, center[1] as f64, center[2] as f64],
                0.0,
            ))
        }
        FeatureKind::Extrusion {
            polygon,
            y0,
            height,
            color,
        } => {
            if polygon.len() < 3 || *height <= 0.0 {
                return Err(OpError::Invalid(
                    "extrusion needs >= 3 polygon points and positive height".into(),
                ));
            }
            let n = polygon.len() as f32;
            let cx = polygon.iter().map(|p| p[0]).sum::<f32>() / n;
            let cz = polygon.iter().map(|p| p[1]).sum::<f32>() / n;
            let local_poly: Vec<[f64; 2]> = polygon
                .iter()
                .map(|p| [(p[0] - cx) as f64, (p[1] - cz) as f64])
                .collect();
            temp.apply(
                Op::AddExtrude {
                    polygon: local_poly,
                    y0: 0.0,
                    height: *height as f64,
                    color: *color,
                },
                Actor::default(),
            )?;
            Ok((temp.mesh().clone(), [cx as f64, *y0 as f64, cz as f64], 0.0))
        }
        FeatureKind::LinearPattern { .. } => Err(OpError::Invalid(
            "linear pattern does not define standalone primitive geometry".into(),
        )),
    }
}

/// Computes topological evaluation order for a set of features (detecting cycles).
fn topo_order(features: &[FeatureRecord]) -> Result<Vec<usize>, String> {
    let idx_by_id: BTreeMap<&str, usize> = features
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.as_str(), i))
        .collect();
    let mut visited = vec![0u8; features.len()]; // 0 = unvisited, 1 = visiting, 2 = done
    let mut order = Vec::with_capacity(features.len());

    fn dfs(
        u: usize,
        features: &[FeatureRecord],
        idx_by_id: &BTreeMap<&str, usize>,
        visited: &mut [u8],
        order: &mut Vec<usize>,
    ) -> Result<(), String> {
        if visited[u] == 2 {
            return Ok(());
        }
        if visited[u] == 1 {
            return Err(format!(
                "dependency cycle involving feature '{}'",
                features[u].id
            ));
        }
        visited[u] = 1;
        for dep in &features[u].depends_on {
            if let Some(&v) = idx_by_id.get(dep.as_str()) {
                dfs(v, features, idx_by_id, visited, order)?;
            }
        }
        visited[u] = 2;
        order.push(u);
        Ok(())
    }

    for i in 0..features.len() {
        dfs(i, features, &idx_by_id, &mut visited, &mut order)?;
    }
    Ok(order)
}

impl Document {
    pub(crate) fn precision_state(&self) -> PrecisionState {
        PrecisionState::from_entries(&self.features)
    }

    pub(crate) fn set_precision_state(&mut self, state: &PrecisionState) {
        state.write_into(&mut self.features);
    }

    /// Active 2D construction workplane.
    pub fn workplane(&self) -> Workplane {
        self.precision_state().workplane
    }

    /// Sets the active 2D construction workplane (persisted in `.brush`).
    pub fn set_workplane(&mut self, workplane: Workplane) {
        let mut state = self.precision_state();
        state.workplane = workplane;
        self.set_precision_state(&state);
    }

    /// Aligns the selected instances along `axis` (`min`, `center`, or `max`) as one undoable transaction.
    pub fn align_selection(
        &mut self,
        ids: &[String],
        axis: AlignAxis,
        mode: AlignMode,
        actor: Actor,
    ) -> Result<usize, OpError> {
        let ops = align_instances_ops(&self.scene.instances, ids, axis, mode);
        if ops.is_empty() {
            return Ok(0);
        }
        let n = ops.len();
        self.apply_transaction(ops, Some("alignSelection".to_string()), actor)
            .map_err(|(_, e)| e)?;
        Ok(n)
    }

    /// Evenly distributes >= 3 selected instances along `axis` as one undoable transaction.
    pub fn distribute_selection(
        &mut self,
        ids: &[String],
        axis: AlignAxis,
        actor: Actor,
    ) -> Result<usize, OpError> {
        let ops = distribute_instances_ops(&self.scene.instances, ids, axis);
        if ops.is_empty() {
            return Ok(0);
        }
        let n = ops.len();
        self.apply_transaction(ops, Some("distributeSelection".to_string()), actor)
            .map_err(|(_, e)| e)?;
        Ok(n)
    }

    /// Lists all persisted construction features in the document.
    pub fn list_features(&self) -> Vec<FeatureRecord> {
        self.precision_state().features
    }

    /// Lists all persisted parametric constraints in the document.
    pub fn list_constraints(&self) -> Vec<ConstraintRecord> {
        self.precision_state().constraints
    }

    /// Adds a new parameterized construction feature, evaluates its mesh, and places its instance(s)
    /// in one atomic undoable transaction.
    pub fn add_feature(
        &mut self,
        spec: FeatureSpec,
        actor: Actor,
    ) -> Result<FeatureRecord, OpError> {
        let mut state = self.precision_state();
        let id = spec
            .id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("feat:{}", state.features.len() + 1));
        if state.features.iter().any(|f| f.id == id) {
            return Err(OpError::Invalid(format!("feature '{id}' already exists")));
        }
        let depends_on = spec.kind.dependencies();
        for dep in &depends_on {
            if !state.features.iter().any(|f| &f.id == dep) {
                return Err(OpError::Invalid(format!(
                    "feature '{id}' depends on unknown feature '{dep}'"
                )));
            }
        }
        let object_key = match &spec.kind {
            FeatureKind::LinearPattern { source_feature, .. } => state
                .features
                .iter()
                .find(|f| &f.id == source_feature)
                .map(|f| f.object_key.clone())
                .ok_or_else(|| {
                    OpError::Invalid(format!("missing source feature '{source_feature}'"))
                })?,
            _ => format!("feature:{id}"),
        };

        let record = FeatureRecord {
            id: id.clone(),
            name: spec.name,
            kind: spec.kind,
            depends_on,
            object_key,
            instance_ids: Vec::new(),
            status: FeatureStatus::Valid,
            evaluated_hash: String::new(),
        };
        state.features.push(record);
        self.set_precision_state(&state);
        self.recompute_features(std::slice::from_ref(&id), actor, "addFeature")?;
        Ok(self
            .precision_state()
            .features
            .into_iter()
            .find(|f| f.id == id)
            .unwrap())
    }

    /// Updates a feature's parameters, solves enabled constraints, and recomputes the feature
    /// and all downstream dependents as one atomic undoable transaction.
    pub fn update_feature(
        &mut self,
        id: &str,
        kind: FeatureKind,
        actor: Actor,
    ) -> Result<SolveReport, OpError> {
        let mut state = self.precision_state();
        let pos = state
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| OpError::Invalid(format!("unknown feature '{id}'")))?;
        if matches!(state.features[pos].status, FeatureStatus::Baked) {
            return Err(OpError::Invalid(format!(
                "feature '{id}' was baked into ordinary mesh geometry and can no longer be edited parametrically"
            )));
        }
        state.features[pos].depends_on = kind.dependencies();
        state.features[pos].kind = kind;
        state.features[pos].status = FeatureStatus::Valid;
        self.set_precision_state(&state);
        let mut report = self.solve_constraints_internal(actor.clone(), Some(id))?;
        if !report.updated_features.iter().any(|x| x == id) {
            report.updated_features.insert(0, id.to_string());
        }
        let dirty = report.updated_features.clone();
        self.recompute_features(&dirty, actor, "updateFeature")?;
        Ok(report)
    }

    /// Validates all non-baked features against the live document objects and scene instances.
    /// Features whose object mesh or instances were modified/deleted by direct edits are marked
    /// `Invalidated { reason }` (propagating to dependents) so callers can offer repair or bake.
    pub fn validate_features(&mut self) -> Vec<FeatureRecord> {
        let mut state = self.precision_state();
        let order =
            topo_order(&state.features).unwrap_or_else(|_| (0..state.features.len()).collect());
        let mut invalid_ids = BTreeSet::new();

        for idx in order {
            let feat = &mut state.features[idx];
            if matches!(feat.status, FeatureStatus::Baked) {
                invalid_ids.insert(feat.id.clone());
                continue;
            }
            let mut reason: Option<String> = None;
            for dep in &feat.depends_on {
                if invalid_ids.contains(dep) {
                    reason = Some(format!("upstream feature '{dep}' is invalidated or baked"));
                    break;
                }
            }
            if reason.is_none() {
                match self.objects.get(&feat.object_key) {
                    None => {
                        reason = Some(format!("evaluated mesh '{}' was removed", feat.object_key));
                    }
                    Some(mesh)
                        if !feat.evaluated_hash.is_empty()
                            && mesh.content_hash() != feat.evaluated_hash =>
                    {
                        reason = Some("evaluated mesh was modified by a direct edit".to_string());
                    }
                    _ => {}
                }
            }
            if reason.is_none() {
                for inst_id in &feat.instance_ids {
                    if self.scene.get(inst_id).is_none() {
                        reason = Some(format!(
                            "instance '{inst_id}' was deleted or baked by a direct edit"
                        ));
                        break;
                    }
                }
            }
            if let Some(r) = reason {
                feat.status = FeatureStatus::Invalidated { reason: r };
                invalid_ids.insert(feat.id.clone());
            } else {
                feat.status = FeatureStatus::Valid;
            }
        }

        self.set_precision_state(&state);
        state.features
    }

    /// Repairs an invalidated feature (and its dependents) by re-evaluating its parameters and
    /// restoring its mesh and scene instances.
    pub fn repair_feature(&mut self, id: &str, actor: Actor) -> Result<FeatureRecord, OpError> {
        let mut state = self.precision_state();
        let pos = state
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| OpError::Invalid(format!("unknown feature '{id}'")))?;
        if matches!(state.features[pos].status, FeatureStatus::Baked) {
            return Err(OpError::Invalid(format!(
                "feature '{id}' is already baked into ordinary mesh geometry"
            )));
        }
        state.features[pos].status = FeatureStatus::Valid;
        self.set_precision_state(&state);
        self.recompute_features(&[id.to_string()], actor, "repairFeature")?;
        Ok(self
            .precision_state()
            .features
            .into_iter()
            .find(|f| f.id == id)
            .unwrap())
    }

    /// Bakes a feature's evaluated instances into ordinary main mesh geometry (for sculpting or
    /// arbitrary topology edits) and marks the feature `Baked`.
    pub fn bake_feature(&mut self, id: &str, actor: Actor) -> Result<FeatureRecord, OpError> {
        let mut state = self.precision_state();
        let pos = state
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| OpError::Invalid(format!("unknown feature '{id}'")))?;
        let ops: Vec<Op> = state.features[pos]
            .instance_ids
            .iter()
            .filter(|inst_id| self.scene.get(inst_id).is_some())
            .map(|inst_id| Op::BakeObject {
                id: inst_id.clone(),
            })
            .collect();
        if !ops.is_empty() {
            self.apply_transaction(ops, Some(format!("bakeFeature:{id}")), actor)
                .map_err(|(_, e)| e)?;
        }
        state.features[pos].status = FeatureStatus::Baked;
        state.features[pos].instance_ids.clear();
        self.set_precision_state(&state);
        self.validate_features();
        Ok(self
            .precision_state()
            .features
            .into_iter()
            .find(|f| f.id == id)
            .unwrap())
    }

    /// Adds a persistent constraint and solves the constraint graph immediately.
    pub fn add_constraint(
        &mut self,
        constraint: ConstraintRecord,
        actor: Actor,
    ) -> Result<SolveReport, OpError> {
        let mut state = self.precision_state();
        if state.constraints.iter().any(|c| c.id == constraint.id) {
            return Err(OpError::Invalid(format!(
                "constraint '{}' already exists",
                constraint.id
            )));
        }
        state.constraints.push(constraint);
        self.set_precision_state(&state);
        self.solve_constraints(actor)
    }

    /// Removes a constraint by id. Returns true if it existed.
    pub fn remove_constraint(&mut self, id: &str) -> bool {
        let mut state = self.precision_state();
        let before = state.constraints.len();
        state.constraints.retain(|c| c.id != id);
        let removed = state.constraints.len() != before;
        if removed {
            self.set_precision_state(&state);
        }
        removed
    }

    /// Solves all enabled persistent constraints, recomputes affected features, and reports any conflicts.
    pub fn solve_constraints(&mut self, actor: Actor) -> Result<SolveReport, OpError> {
        let report = self.solve_constraints_internal(actor.clone(), None)?;
        if !report.updated_features.is_empty() {
            self.recompute_features(&report.updated_features, actor, "solveConstraints")?;
        }
        Ok(report)
    }

    fn solve_constraints_internal(
        &mut self,
        _actor: Actor,
        _changed_feature: Option<&str>,
    ) -> Result<SolveReport, OpError> {
        let mut state = self.precision_state();
        let mut updated = BTreeSet::new();
        let mut conflicts = Vec::new();
        // Track parameters locked by FixedDimension so contradictory constraints are reported clearly.
        let mut fixed_locks: BTreeMap<(String, String), (f32, String)> = BTreeMap::new();

        for c in state.constraints.clone().iter().filter(|c| c.enabled) {
            if let ConstraintKind::FixedDimension {
                feature,
                param,
                value,
            } = &c.kind
            {
                let key = (feature.clone(), param.clone());
                if let Some((prev_val, prev_id)) = fixed_locks.get(&key) {
                    if (*prev_val - *value).abs() > 1e-4 {
                        conflicts.push(ConstraintConflict {
                            constraint_id: c.id.clone(),
                            reason: format!(
                                "conflicts with '{prev_id}': {feature}.{param} cannot be both {prev_val} and {value}"
                            ),
                        });
                        continue;
                    }
                }
                fixed_locks.insert(key, (*value, c.id.clone()));
                if let Some(f) = state.features.iter_mut().find(|f| &f.id == feature) {
                    match f.kind.set_param(param, *value) {
                        Ok(true) => {
                            updated.insert(feature.clone());
                        }
                        Ok(false) => {}
                        Err(e) => conflicts.push(ConstraintConflict {
                            constraint_id: c.id.clone(),
                            reason: e.to_string(),
                        }),
                    }
                } else {
                    conflicts.push(ConstraintConflict {
                        constraint_id: c.id.clone(),
                        reason: format!("feature '{feature}' not found"),
                    });
                }
            }
        }

        // Solve relational constraints (up to 4 relaxation passes to handle chains).
        for _pass in 0..4 {
            let mut any_changed = false;
            for c in state.constraints.clone().iter().filter(|c| c.enabled) {
                match &c.kind {
                    ConstraintKind::FixedDimension { .. } => {}
                    ConstraintKind::EqualDimension {
                        feature_a,
                        param_a,
                        feature_b,
                        param_b,
                    } => {
                        let val_a = state
                            .features
                            .iter()
                            .find(|f| &f.id == feature_a)
                            .and_then(|f| f.kind.get_param(param_a));
                        let Some(val_a) = val_a else {
                            if _pass == 0 {
                                conflicts.push(ConstraintConflict {
                                    constraint_id: c.id.clone(),
                                    reason: format!(
                                        "missing source parameter {feature_a}.{param_a}"
                                    ),
                                });
                            }
                            continue;
                        };
                        if let Some((locked, lock_id)) =
                            fixed_locks.get(&(feature_b.clone(), param_b.clone()))
                        {
                            if (*locked - val_a).abs() > 1e-4 {
                                if _pass == 0 {
                                    conflicts.push(ConstraintConflict {
                                        constraint_id: c.id.clone(),
                                        reason: format!(
                                            "conflicts with '{lock_id}': {feature_b}.{param_b} is locked to {locked}, cannot match {feature_a}.{param_a} ({val_a})"
                                        ),
                                    });
                                }
                                continue;
                            }
                        }
                        if let Some(fb) = state.features.iter_mut().find(|f| &f.id == feature_b) {
                            match fb.kind.set_param(param_b, val_a) {
                                Ok(true) => {
                                    updated.insert(feature_b.clone());
                                    any_changed = true;
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    if _pass == 0 {
                                        conflicts.push(ConstraintConflict {
                                            constraint_id: c.id.clone(),
                                            reason: e.to_string(),
                                        });
                                    }
                                }
                            }
                        } else if _pass == 0 {
                            conflicts.push(ConstraintConflict {
                                constraint_id: c.id.clone(),
                                reason: format!("feature '{feature_b}' not found"),
                            });
                        }
                    }
                    ConstraintKind::CoincidentOffset {
                        source_feature,
                        target_feature,
                        offset,
                    } => {
                        let src_origin = state
                            .features
                            .iter()
                            .find(|f| &f.id == source_feature)
                            .and_then(|f| f.kind.origin());
                        let Some(src) = src_origin else {
                            if _pass == 0 {
                                conflicts.push(ConstraintConflict {
                                    constraint_id: c.id.clone(),
                                    reason: format!(
                                        "source feature '{source_feature}' has no origin"
                                    ),
                                });
                            }
                            continue;
                        };
                        let desired = [src[0] + offset[0], src[1] + offset[1], src[2] + offset[2]];
                        if let Some(ft) =
                            state.features.iter_mut().find(|f| &f.id == target_feature)
                        {
                            if let Some(cur) = ft.kind.origin() {
                                if (cur[0] - desired[0]).abs() > 1e-5
                                    || (cur[1] - desired[1]).abs() > 1e-5
                                    || (cur[2] - desired[2]).abs() > 1e-5
                                {
                                    ft.kind.set_origin(desired);
                                    updated.insert(target_feature.clone());
                                    any_changed = true;
                                }
                            }
                        } else if _pass == 0 {
                            conflicts.push(ConstraintConflict {
                                constraint_id: c.id.clone(),
                                reason: format!("target feature '{target_feature}' not found"),
                            });
                        }
                    }
                }
            }
            if !any_changed {
                break;
            }
        }

        self.set_precision_state(&state);
        Ok(SolveReport {
            updated_features: updated.into_iter().collect(),
            conflicts,
        })
    }

    fn recompute_features(
        &mut self,
        roots: &[String],
        actor: Actor,
        label: &str,
    ) -> Result<(), OpError> {
        let mut state = self.precision_state();
        let order = topo_order(&state.features).map_err(OpError::Invalid)?;
        let mut dirty: BTreeSet<String> = roots.iter().cloned().collect();
        let mut ops: Vec<Op> = Vec::new();

        for idx in order {
            let feat_id = state.features[idx].id.clone();
            let depends_dirty = state.features[idx]
                .depends_on
                .iter()
                .any(|d| dirty.contains(d));
            if !dirty.contains(&feat_id) && !depends_dirty {
                continue;
            }
            if matches!(state.features[idx].status, FeatureStatus::Baked) {
                continue;
            }
            dirty.insert(feat_id.clone());

            let kind = state.features[idx].kind.clone();
            let old_instances = state.features[idx].instance_ids.clone();
            for old_id in &old_instances {
                if self.scene.get(old_id).is_some() {
                    ops.push(Op::RemoveObject { id: old_id.clone() });
                }
            }

            match kind {
                FeatureKind::LinearPattern {
                    ref source_feature,
                    count,
                    step,
                } => {
                    let src = state
                        .features
                        .iter()
                        .find(|f| &f.id == source_feature)
                        .cloned()
                        .ok_or_else(|| {
                            OpError::Invalid(format!("missing source feature '{source_feature}'"))
                        })?;
                    let src_origin = src.kind.origin().ok_or_else(|| {
                        OpError::Invalid(format!("source feature '{source_feature}' has no origin"))
                    })?;
                    let src_yaw = src.kind.get_param("yawDeg").unwrap_or(0.0) as f64;
                    let mut new_ids = Vec::with_capacity(count as usize);
                    for k in 1..=count {
                        let inst_id = format!("{feat_id}:inst:{k}");
                        let kf = k as f64;
                        ops.push(Op::PlaceObject {
                            id: inst_id.clone(),
                            object: src.object_key.clone(),
                            position: [
                                src_origin[0] as f64 + (step[0] as f64) * kf,
                                src_origin[1] as f64 + (step[1] as f64) * kf,
                                src_origin[2] as f64 + (step[2] as f64) * kf,
                            ],
                            yaw_deg: src_yaw,
                            pitch_deg: 0.0,
                            roll_deg: 0.0,
                            scale: 1.0,
                        });
                        new_ids.push(inst_id);
                    }
                    state.features[idx].object_key = src.object_key;
                    state.features[idx].evaluated_hash = src.evaluated_hash;
                    state.features[idx].instance_ids = new_ids;
                    state.features[idx].status = FeatureStatus::Valid;
                }
                _ => {
                    let (mesh, position, yaw_deg) = evaluate_primitive(&kind)?;
                    let hash = mesh.content_hash();
                    let obj_key = state.features[idx].object_key.clone();
                    self.set_object(obj_key.clone(), mesh);
                    let inst_id = format!("{feat_id}:inst:0");
                    ops.push(Op::PlaceObject {
                        id: inst_id.clone(),
                        object: obj_key,
                        position,
                        yaw_deg,
                        pitch_deg: 0.0,
                        roll_deg: 0.0,
                        scale: 1.0,
                    });
                    state.features[idx].instance_ids = vec![inst_id];
                    state.features[idx].evaluated_hash = hash;
                    state.features[idx].status = FeatureStatus::Valid;
                }
            }
        }

        if !ops.is_empty() {
            self.apply_transaction(ops, Some(label.to_string()), actor)
                .map_err(|(_, e)| e)?;
        }
        self.set_precision_state(&state);
        Ok(())
    }
}
