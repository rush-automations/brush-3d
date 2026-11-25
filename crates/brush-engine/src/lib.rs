//! # brush-engine
//!
//! A [`Document`] is a mesh plus everything about how it got that way. Every change is a typed [`Op`] applied
//! through [`Document::apply`] or grouped as a single user action through [`Document::apply_transaction`], which:
//!
//! - refuses invalid ops without touching the mesh or scene;
//! - records the op (and `opsVersion`), who sent it (a person, an app, an AI model) and what it did in an append-only
//!   [`Journal`], which can be stored as JSON Lines or inside a versioned `.brush` project container and replayed;
//! - keeps an undo/redo history of snapshots and tracks a monotonic document [`Document::revision`];
//! - supports isolated preview sessions (`begin_preview` / `update_preview` / `commit_preview` / `cancel_preview`)
//!   that never pollute undo history or the journal.
//!
//! The original mesh is kept as the reference for `matchColor` with `fromReference` and for [`Document::revert`].
//!
//! A document also has a [`Scene`]: linked instances of other objects placed on its mesh (see [`scene`]). The host
//! supplies each object's mesh with [`Document::set_object`]; [`Document::flattened`] gives the mesh with them all in,
//! for exports. [`Document::place`] turns a pointer position into the op that builds a piece there (see [`place`]).

pub use brush_io as io;
pub use brush_mesh as mesh;
pub use brush_mesh::{
    BoxRegion, Island, Mesh, Op, OpError, OpOutcome, TangledCluster, TopologyKind, TopologyRemap,
    OPS_VERSION,
};
pub use brush_render as render;

pub mod edit;
pub mod place;
pub mod scene;
pub mod selection;

pub use edit::DetachedPart;
pub use place::{Piece, Placement};
pub use scene::{Instance, Pick, Scene};
pub use selection::{Axis, ClipItem, Clipboard};

use std::collections::BTreeMap;

use brush_mesh::op::Context;
use brush_mesh::sculpt::SculptTopology;
pub use brush_mesh::{Bvh, Dab, Hit};
use serde::{Deserialize, Serialize};

/// Undo keeps this many snapshots; older history stays in the journal.
pub const DEFAULT_UNDO_DEPTH: usize = 32;

/// Who made a change. Free-form so hosts can use their own ids (`"user:42"`, `"model:brush-clean-1"`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActorKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActorKind {
    Person,
    Model,
    App,
}

/// One line of the journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Entry {
    /// An op was applied and changed the document.
    Applied {
        seq: u64,
        op: Op,
        #[serde(default, skip_serializing_if = "is_default")]
        actor: Actor,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ops_version: Option<String>,
        outcome: OpOutcome,
        /// Content hash of the document after the op.
        hash: String,
    },
    /// Several ops committed together as one atomic user action (one undo step).
    Transaction {
        seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ops_version: String,
        ops: Vec<Op>,
        #[serde(default, skip_serializing_if = "is_default")]
        actor: Actor,
        outcome: OpOutcome,
        /// Content hash of the document after the transaction.
        hash: String,
    },
    Undo {
        seq: u64,
        hash: String,
    },
    Redo {
        seq: u64,
        hash: String,
    },
    Revert {
        seq: u64,
        hash: String,
    },
}

impl Entry {
    pub fn seq(&self) -> u64 {
        match self {
            Entry::Applied { seq, .. }
            | Entry::Transaction { seq, .. }
            | Entry::Undo { seq, .. }
            | Entry::Redo { seq, .. }
            | Entry::Revert { seq, .. } => *seq,
        }
    }
}

fn is_default(a: &Actor) -> bool {
    *a == Actor::default()
}

/// The append-only record of a document's changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Journal {
    pub entries: Vec<Entry>,
}

impl Journal {
    /// One JSON object per line.
    pub fn to_jsonl(&self) -> String {
        self.entries
            .iter()
            .map(|e| serde_json::to_string(e).expect("entry serialises") + "\n")
            .collect()
    }

    pub fn from_jsonl(text: &str) -> Result<Journal, serde_json::Error> {
        let entries = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        Ok(Journal { entries })
    }
}

/// The result of [`Document::apply`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub outcome: OpOutcome,
    /// False when the op ran but left the document exactly as it was; nothing is recorded then.
    pub changed: bool,
    pub hash: String,
}

#[derive(Clone)]
struct Step {
    /// Representative op (or last op of a transaction); `None` for a revert.
    op: Option<Op>,
    /// Optional human/tool transaction label (e.g. `"moveSelection"`, `"detachIsland"`).
    label: Option<String>,
    /// The mesh on the other side of this step: before it on the undo stack, after it on the redo stack. None when
    /// the step left the mesh alone (moving placed objects), so those steps cost no mesh copy.
    mesh: Option<Mesh>,
    /// The scene on the other side of this step.
    scene: Scene,
}

/// Derived data kept between edits. Connectivity survives ops that only move vertices or recolour them; the BVH is
/// refitted after those and rebuilt after anything else.
#[derive(Default)]
struct Caches {
    topology: Option<SculptTopology>,
    bvh: Option<Bvh>,
    bvh_stale: bool,
}

impl Caches {
    fn invalidate(&mut self, topology_changed: bool) {
        if topology_changed {
            *self = Caches::default();
        } else {
            self.bvh_stale = true;
        }
    }
}

/// A sculpt stroke still being drawn: later dabs extend it in place.
struct Stroke {
    touched: Vec<bool>,
    /// The journal entry holding the stroke, once it changed something.
    recorded: bool,
    /// The op so far (all dabs), and the actor, for recording it when it first changes the mesh.
    op: Op,
    actor: Actor,
    before: Option<Mesh>,
}

/// An open multi-step transaction (`begin_transaction` .. `commit_transaction`), collapsing several live mutations into
/// one undo step and one journal entry.
struct OpenTransaction {
    label: Option<String>,
    actor: Actor,
    mesh_before: Mesh,
    scene_before: Scene,
    ops: Vec<Op>,
    last_outcome: Option<OpOutcome>,
}

/// An active preview session (`begin_preview` .. `commit_preview` / `cancel_preview`).
struct PreviewSession {
    base_revision: u64,
    mesh_before: Mesh,
    scene_before: Scene,
    ops: Vec<Op>,
}

pub struct Document {
    mesh: Mesh,
    original: Mesh,
    scene: Scene,
    original_scene: Scene,
    /// Meshes behind the scene's pointers, as the host last supplied them, and their BVHs for picking.
    objects: BTreeMap<String, Mesh>,
    object_bvhs: BTreeMap<String, Bvh>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    journal: Journal,
    seq: u64,
    revision: u64,
    undo_depth: usize,
    caches: Caches,
    stroke: Option<Stroke>,
    open_tx: Option<OpenTransaction>,
    preview: Option<PreviewSession>,
    /// Optional host metadata preserved across `.brush` project save/open.
    metadata: serde_json::Value,
    /// Optional construction/parametric design-intent records preserved across `.brush` save/open.
    features: Vec<serde_json::Value>,
}

impl Document {
    pub fn new(mesh: Mesh) -> Document {
        Document {
            original: mesh.clone(),
            mesh,
            scene: Scene::default(),
            original_scene: Scene::default(),
            objects: BTreeMap::new(),
            object_bvhs: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Journal::default(),
            seq: 0,
            revision: 0,
            undo_depth: DEFAULT_UNDO_DEPTH,
            caches: Caches::default(),
            stroke: None,
            open_tx: None,
            preview: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            features: Vec::new(),
        }
    }

    /// Monotonic document revision, incremented on every committed change (apply, transaction commit, undo, redo,
    /// revert, or scene load). Unchanged during preview sessions.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Verifies that `expected` matches the current [`Document::revision`].
    pub fn check_revision(&self, expected: u64) -> Result<(), OpError> {
        if self.revision == expected {
            Ok(())
        } else {
            Err(OpError::Invalid(format!(
                "stale document revision: expected {expected}, current is {}",
                self.revision
            )))
        }
    }

    pub fn metadata(&self) -> &serde_json::Value {
        &self.metadata
    }

    pub fn set_metadata(&mut self, metadata: serde_json::Value) {
        self.metadata = metadata;
    }

    pub fn features(&self) -> &[serde_json::Value] {
        &self.features
    }

    pub fn set_features(&mut self, features: Vec<serde_json::Value>) {
        self.features = features;
    }

    fn topology(&mut self) -> &SculptTopology {
        if self.caches.topology.is_none() {
            self.caches.topology = Some(SculptTopology::of(&self.mesh));
        }
        self.caches.topology.as_ref().unwrap()
    }

    /// The first face hit by the ray `origin + t * dir`, for picking. Faces are two-sided.
    pub fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3]) -> Option<Hit> {
        match &mut self.caches.bvh {
            None => self.caches.bvh = Some(Bvh::build(&self.mesh)),
            Some(b) if self.caches.bvh_stale => b.refit(&self.mesh),
            _ => {}
        }
        self.caches.bvh_stale = false;
        self.caches
            .bvh
            .as_ref()
            .unwrap()
            .raycast(&self.mesh, origin, dir)
    }

    /// Area-weighted vertex normals, 3 per vertex, for display.
    pub fn vertex_normals(&mut self) -> Vec<f32> {
        self.topology();
        let vf = &self.caches.topology.as_ref().unwrap().vf;
        (0..self.mesh.vertex_count())
            .flat_map(|i| brush_mesh::topo::vertex_normal(&self.mesh, vf, i))
            .collect()
    }

    /// Replaces the mesh after undo, redo, revert, or preview restore, keeping caches that still apply.
    fn swap_mesh(&mut self, next: Mesh) -> Mesh {
        let topology_changed = next.indices != self.mesh.indices;
        self.caches.invalidate(topology_changed);
        std::mem::replace(&mut self.mesh, next)
    }

    /// Swaps in a step's side: its mesh, when it has one, and its scene.
    fn swap_state(&mut self, mesh: Option<Mesh>, scene: Scene) -> (Option<Mesh>, Scene) {
        let mesh = mesh.map(|m| self.swap_mesh(m));
        (mesh, std::mem::replace(&mut self.scene, scene))
    }

    fn hash_pair(mesh: &Mesh, scene: &Scene) -> String {
        let m = mesh.content_hash();
        if scene.is_empty() {
            return m;
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let s = serde_json::to_string(scene).expect("scenes serialise");
        for b in m.bytes().chain(s.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    /// The document's hash: the mesh's content hash, extended with the scene when anything is placed. Documents without
    /// placed objects hash exactly as their mesh does.
    pub fn hash(&self) -> String {
        Self::hash_pair(&self.mesh, &self.scene)
    }

    /// The placed objects.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Supplies (or refreshes) the mesh behind an object pointer. Every instance of it shows the new mesh at once; the
    /// journal is untouched, since the object's own history records how it changed.
    pub fn set_object(&mut self, object: impl Into<String>, mesh: Mesh) {
        let object = object.into();
        self.object_bvhs.remove(&object);
        self.objects.insert(object, mesh);
    }

    pub(crate) fn remove_object_asset(&mut self, object: &str) {
        self.object_bvhs.remove(object);
        self.objects.remove(object);
    }

    pub fn object(&self, object: &str) -> Option<&Mesh> {
        self.objects.get(object)
    }

    /// Keys of every object mesh currently loaded in this document, in deterministic sorted order.
    pub fn loaded_objects(&self) -> Vec<String> {
        self.objects.keys().cloned().collect()
    }

    /// Area-weighted vertex normals of an object's mesh, 3 per vertex, for drawing its instances.
    pub fn object_normals(&self, object: &str) -> Option<Vec<f32>> {
        let m = self.objects.get(object)?;
        let topo = SculptTopology::of(m);
        Some(
            (0..m.vertex_count())
                .flat_map(|i| brush_mesh::topo::vertex_normal(m, &topo.vf, i))
                .collect(),
        )
    }

    /// Replaces the placed objects with a scene as stored (when opening a model), as the saved state. Not an edit:
    /// the journal is untouched and undo history is cleared.
    pub fn load_scene(&mut self, scene: Scene) {
        self.cancel_preview();
        let _ = self.cancel_transaction();
        self.end_stroke();
        if self.scene != scene {
            self.revision += 1;
        }
        self.original_scene = scene.clone();
        self.scene = scene;
        self.undo.clear();
        self.redo.clear();
    }

    /// Objects the scene points at that have not been supplied yet.
    pub fn missing_objects(&self) -> Vec<String> {
        self.scene
            .objects()
            .into_iter()
            .filter(|o| !self.objects.contains_key(o))
            .collect()
    }

    /// The mesh with every placed object copied in, as exports and other tools should see it.
    pub fn flattened(&self) -> Mesh {
        scene::flatten(&self.mesh, &self.scene, |o| self.objects.get(o))
    }

    /// A placed instance's object-to-world transform, column-major 4x4, for drawing its object's mesh. None when the
    /// instance or its object's mesh is unknown.
    pub fn instance_transform(&self, id: &str) -> Option<[f32; 16]> {
        let inst = self.scene.get(id)?;
        let obj = self.objects.get(&inst.object)?;
        Some(scene::transform(
            scene::anchor(obj),
            inst.position,
            inst.rotation(),
            inst.scale,
        ))
    }

    /// Where an object would be drawn for a given placement (a ghost before placing it). `pitch_deg` and `roll_deg`
    /// tilt it as an instance's do (see [`scene::rotation`]).
    pub fn object_transform(
        &self,
        object: &str,
        position: [f64; 3],
        yaw_deg: f64,
        scale: f64,
        pitch_deg: f64,
        roll_deg: f64,
    ) -> Option<[f32; 16]> {
        let obj = self.objects.get(object)?;
        Some(scene::transform(
            scene::anchor(obj),
            position,
            scene::rotation(yaw_deg, pitch_deg, roll_deg),
            scale,
        ))
    }

    /// The nearest surface along `origin + t * dir`: the document's own mesh or a placed instance.
    pub fn pick(&mut self, origin: [f32; 3], dir: [f32; 3]) -> Option<Pick> {
        let own = self.raycast(origin, dir).map(|hit| Pick {
            hit,
            instance: None,
        });
        for o in self.scene.objects() {
            if let (Some(m), false) = (self.objects.get(&o), self.object_bvhs.contains_key(&o)) {
                self.object_bvhs.insert(o.clone(), Bvh::build(m));
            }
        }
        let placed = scene::pick_instances(
            &self.scene,
            |o| Some((self.objects.get(o)?, self.object_bvhs.get(o)?)),
            origin,
            dir,
        );
        match (own, placed) {
            (Some(a), Some(b)) => Some(if b.hit.t < a.hit.t { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// The op that builds a piece where an editor's pointer is (see [`place`]). Nothing changes until it is applied.
    pub fn place(&self, p: &Placement) -> Result<Op, OpError> {
        let size = p
            .object
            .as_deref()
            .and_then(|o| self.objects.get(o))
            .map(scene::size);
        place::place(p, size, || self.scene.free_id())
    }

    /// Applies a scene op. Returns the outcome, and whether the mesh changed (baking).
    fn apply_scene(&mut self, op: &Op) -> Result<(OpOutcome, bool), OpError> {
        let outcome = |count: u64, m: &Mesh| OpOutcome {
            count,
            vertices: m.vertex_count() as u64,
            faces: m.face_count() as u64,
        };
        let find = |s: &Scene, id: &str| {
            s.index(id)
                .ok_or_else(|| OpError::NoInstance(id.to_string()))
        };
        match op {
            Op::PlaceObject {
                id,
                object,
                position,
                yaw_deg,
                pitch_deg,
                roll_deg,
                scale,
            } => {
                if self.scene.get(id).is_some() {
                    return Err(OpError::DuplicateInstance(id.clone()));
                }
                self.scene.instances.push(Instance {
                    id: id.clone(),
                    object: object.clone(),
                    position: *position,
                    yaw_deg: *yaw_deg,
                    pitch_deg: *pitch_deg,
                    roll_deg: *roll_deg,
                    scale: *scale,
                });
                Ok((outcome(1, &self.mesh), false))
            }
            Op::MoveObject {
                id,
                position,
                yaw_deg,
                pitch_deg,
                roll_deg,
                scale,
            } => {
                let i = find(&self.scene, id)?;
                let inst = &mut self.scene.instances[i];
                (inst.position, inst.yaw_deg, inst.scale) = (*position, *yaw_deg, *scale);
                (inst.pitch_deg, inst.roll_deg) = (*pitch_deg, *roll_deg);
                Ok((outcome(1, &self.mesh), false))
            }
            Op::RemoveObject { id } => {
                let i = find(&self.scene, id)?;
                self.scene.instances.remove(i);
                Ok((outcome(1, &self.mesh), false))
            }
            Op::BakeObject { id } => {
                let i = find(&self.scene, id)?;
                let inst = self.scene.instances[i].clone();
                let obj = self
                    .objects
                    .get(&inst.object)
                    .ok_or_else(|| OpError::MissingObject(inst.object.clone()))?;
                let f0 = self.mesh.face_count();
                scene::bake(&mut self.mesh, obj, &inst);
                self.scene.instances.remove(i);
                let added = (self.mesh.face_count() - f0) as u64;
                Ok((outcome(added, &self.mesh), true))
            }
            _ => unreachable!("not a scene op"),
        }
    }

    /// Starts the document with these placed objects (as loaded from where the scene is stored).
    pub fn with_scene(mut self, scene: Scene) -> Document {
        self.original_scene = scene.clone();
        self.scene = scene;
        self
    }

    pub fn with_undo_depth(mut self, depth: usize) -> Document {
        self.undo_depth = depth.max(1);
        self
    }

    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
    pub fn original(&self) -> &Mesh {
        &self.original
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// The op the next undo would take back.
    pub fn last_op(&self) -> Option<&Op> {
        self.undo.last().and_then(|s| s.op.as_ref())
    }
    /// Label of the transaction the next undo would take back, when it was recorded with one.
    pub fn last_transaction_label(&self) -> Option<&str> {
        self.undo.last().and_then(|s| s.label.as_deref())
    }
    /// Changes since load or the last [`Document::mark_saved`], evaluated against the committed state (ignoring an
    /// uncommitted preview).
    pub fn is_dirty(&self) -> bool {
        if let Some(p) = &self.preview {
            return p.mesh_before != self.original || p.scene_before != self.original_scene;
        }
        self.mesh != self.original || self.scene != self.original_scene
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn push_undo_step(&mut self, step: Step) {
        self.undo.push(step);
        if self.undo.len() > self.undo_depth {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Executes a single validated op directly onto `self.mesh` / `self.scene` and updates caches, without recording
    /// undo or journal entries. Used internally by transactions and previews.
    fn execute_unrecorded(&mut self, op: &Op) -> Result<(OpOutcome, bool), OpError> {
        op.validate()?;
        if op.is_scene_op() {
            let scene_before = self.scene.clone();
            let (outcome, mesh_changed) = self.apply_scene(op)?;
            if mesh_changed {
                self.caches.invalidate(true);
            }
            let changed = self.scene != scene_before || mesh_changed;
            return Ok((outcome, changed));
        }
        let before = self.mesh.clone();
        let outcome = op.apply(
            &mut self.mesh,
            Context {
                reference: Some(&self.original),
                topology: None,
            },
        )?;
        let changed = self.mesh != before;
        if changed {
            self.caches.invalidate(op.changes_topology());
        }
        Ok((outcome, changed))
    }

    /// Applies one op. An op that changes nothing is not recorded and cannot be undone.
    ///
    /// A `sculpt` op also opens a stroke: [`Document::extend_stroke`] then adds dabs to it in place, so a stroke drawn
    /// live is still one op in the journal and one step of undo. Any other call closes the stroke.
    pub fn apply(&mut self, op: Op, actor: Actor) -> Result<Applied, OpError> {
        self.cancel_preview();
        self.end_stroke();
        op.validate()?;

        if self.open_tx.is_some() {
            if let Op::Sculpt { .. } = op {
                self.stroke = Some(Stroke {
                    touched: vec![false; self.mesh.vertex_count()],
                    recorded: false,
                    op: op.clone(),
                    actor,
                    before: None,
                });
                return self.sculpt_piece(&op);
            }
            let (outcome, changed) = self.execute_unrecorded(&op)?;
            let hash = self.hash();
            if changed {
                let tx = self.open_tx.as_mut().unwrap();
                tx.ops.push(op);
                tx.last_outcome = Some(outcome);
            }
            return Ok(Applied {
                outcome,
                changed,
                hash,
            });
        }

        if let Op::Sculpt { .. } = op {
            self.stroke = Some(Stroke {
                touched: vec![false; self.mesh.vertex_count()],
                recorded: false,
                op: op.clone(),
                actor,
                before: Some(self.mesh.clone()),
            });
            return self.sculpt_piece(&op);
        }
        if op.is_scene_op() {
            let scene_before = self.scene.clone();
            // Only baking touches the mesh, so only baking pays for a copy of it.
            let mesh_before = matches!(op, Op::BakeObject { .. }).then(|| self.mesh.clone());
            let (outcome, mesh_changed) = self.apply_scene(&op)?;
            let changed = self.scene != scene_before || mesh_changed;
            let hash = self.hash();
            if changed {
                if mesh_changed {
                    self.caches.invalidate(true);
                }
                self.record(op, actor, outcome, hash.clone(), mesh_before, scene_before);
            }
            return Ok(Applied {
                outcome,
                changed,
                hash,
            });
        }
        let before = self.mesh.clone();
        let outcome = op.apply(
            &mut self.mesh,
            Context {
                reference: Some(&self.original),
                topology: None,
            },
        )?;
        let changed = self.mesh != before;
        let hash = self.hash();
        if changed {
            self.caches.invalidate(op.changes_topology());
            let scene = self.scene.clone();
            self.record(op, actor, outcome, hash.clone(), Some(before), scene);
        }
        Ok(Applied {
            outcome,
            changed,
            hash,
        })
    }

    fn record(
        &mut self,
        op: Op,
        actor: Actor,
        outcome: OpOutcome,
        hash: String,
        before: Option<Mesh>,
        scene_before: Scene,
    ) {
        let seq = self.next_seq();
        self.revision += 1;
        self.journal.entries.push(Entry::Applied {
            seq,
            op: op.clone(),
            actor,
            ops_version: Some(OPS_VERSION.to_string()),
            outcome,
            hash,
        });
        self.push_undo_step(Step {
            op: Some(op),
            label: None,
            mesh: before,
            scene: scene_before,
        });
    }

    /// Adds dabs to the open sculpt stroke. The result is exactly what the whole stroke would give as one op.
    pub fn extend_stroke(&mut self, dabs: Vec<Dab>) -> Result<Applied, OpError> {
        let Some(stroke) = &mut self.stroke else {
            return Err(OpError::Invalid(
                "no stroke is open: start one by applying a sculpt op".into(),
            ));
        };
        let Op::Sculpt {
            brush,
            radius,
            strength,
            invert,
            color,
            view,
            dabs: all,
        } = &mut stroke.op
        else {
            unreachable!()
        };
        all.extend_from_slice(&dabs);
        let piece = Op::Sculpt {
            brush: *brush,
            dabs,
            radius: *radius,
            strength: *strength,
            invert: *invert,
            color: *color,
            view: *view,
        };
        piece.validate()?;
        self.sculpt_piece(&piece)
    }

    /// Closes the open sculpt stroke, if any.
    pub fn end_stroke(&mut self) {
        self.stroke = None;
    }

    fn sculpt_piece(&mut self, piece: &Op) -> Result<Applied, OpError> {
        let Op::Sculpt {
            brush,
            dabs,
            radius,
            strength,
            invert,
            color,
            view,
        } = piece
        else {
            unreachable!()
        };
        let d: Vec<([f32; 3], f32)> = dabs
            .iter()
            .map(|d| (d.center.map(|v| v as f32), d.pressure as f32))
            .collect();
        let view = view.map(|v| v.map(|x| x as f32));
        self.topology();
        let mut touched = std::mem::take(&mut self.stroke.as_mut().unwrap().touched);
        let topo = self.caches.topology.as_ref();
        let before_hash_needed = !self.stroke.as_ref().unwrap().recorded;
        let moved = self.mesh.sculpt_into(
            *brush,
            &d,
            *radius as f32,
            *strength as f32,
            *invert,
            *color,
            view,
            topo,
            &mut touched,
        );
        let count = touched.iter().filter(|&&t| t).count() as u64;
        let stroke = self.stroke.as_mut().unwrap();
        stroke.touched = touched;
        let outcome = OpOutcome {
            count,
            vertices: self.mesh.vertex_count() as u64,
            faces: self.mesh.face_count() as u64,
        };
        if moved == 0 && before_hash_needed {
            return Ok(Applied {
                outcome,
                changed: false,
                hash: self.hash(),
            });
        }
        self.caches.invalidate(false);
        let hash = self.hash();
        let stroke = self.stroke.as_mut().unwrap();
        if let Some(tx) = &mut self.open_tx {
            let op = stroke.op.clone();
            if !stroke.recorded {
                stroke.recorded = true;
                tx.ops.push(op);
            } else if let Some(last) = tx.ops.last_mut() {
                *last = op;
            }
            tx.last_outcome = Some(outcome);
            return Ok(Applied {
                outcome,
                changed: moved > 0,
                hash,
            });
        }
        if !stroke.recorded {
            stroke.recorded = true;
            let (op, actor, before) = (
                stroke.op.clone(),
                stroke.actor.clone(),
                stroke.before.take().unwrap(),
            );
            let scene = self.scene.clone();
            self.record(op, actor, outcome, hash.clone(), Some(before), scene);
        } else {
            let op = stroke.op.clone();
            if let Some(Entry::Applied {
                op: o,
                outcome: out,
                hash: h,
                ..
            }) = self.journal.entries.last_mut()
            {
                *o = op.clone();
                *out = outcome;
                *h = hash.clone();
            }
            if let Some(step) = self.undo.last_mut() {
                step.op = Some(op);
            }
        }
        Ok(Applied {
            outcome,
            changed: moved > 0,
            hash,
        })
    }

    // ---- explicit transactions (one user action = one undo step) ----

    /// Whether an open streaming transaction (`begin_transaction`) is currently active.
    pub fn in_transaction(&self) -> bool {
        self.open_tx.is_some()
    }

    /// Opens a multi-step transaction so subsequent `apply` / `extend_stroke` calls collapse into one undo step when
    /// [`Document::commit_transaction`] is called (or roll back completely on [`Document::cancel_transaction`]).
    pub fn begin_transaction(
        &mut self,
        label: Option<String>,
        actor: Actor,
    ) -> Result<u64, OpError> {
        if self.open_tx.is_some() {
            return Err(OpError::Invalid("a transaction is already open".into()));
        }
        self.cancel_preview();
        self.end_stroke();
        self.open_tx = Some(OpenTransaction {
            label,
            actor,
            mesh_before: self.mesh.clone(),
            scene_before: self.scene.clone(),
            ops: Vec::new(),
            last_outcome: None,
        });
        Ok(self.revision)
    }

    /// Commits the open streaming transaction as a single undo step and journal entry. Returns `None` if nothing
    /// changed since `begin_transaction`.
    pub fn commit_transaction(&mut self) -> Option<Applied> {
        self.end_stroke();
        let tx = self.open_tx.take()?;
        let mesh_changed = self.mesh != tx.mesh_before;
        let scene_changed = self.scene != tx.scene_before;
        if (!mesh_changed && !scene_changed) || tx.ops.is_empty() {
            return None;
        }
        let outcome = tx.last_outcome.unwrap_or(OpOutcome {
            count: tx.ops.len() as u64,
            vertices: self.mesh.vertex_count() as u64,
            faces: self.mesh.face_count() as u64,
        });
        let hash = self.hash();
        let seq = self.next_seq();
        self.revision += 1;
        let last_op = tx.ops.last().cloned();
        self.journal.entries.push(Entry::Transaction {
            seq,
            label: tx.label.clone(),
            ops_version: OPS_VERSION.to_string(),
            ops: tx.ops,
            actor: tx.actor,
            outcome,
            hash: hash.clone(),
        });
        self.push_undo_step(Step {
            op: last_op,
            label: tx.label,
            mesh: mesh_changed.then_some(tx.mesh_before),
            scene: tx.scene_before,
        });
        Some(Applied {
            outcome,
            changed: true,
            hash,
        })
    }

    /// Rolls back the open streaming transaction, restoring the exact mesh and scene from `begin_transaction` without
    /// touching undo history, the journal, or the document revision.
    pub fn cancel_transaction(&mut self) -> bool {
        self.end_stroke();
        let Some(tx) = self.open_tx.take() else {
            return false;
        };
        self.swap_mesh(tx.mesh_before);
        self.scene = tx.scene_before;
        true
    }

    /// Applies `ops` atomically and records the entire batch as **one** undo step and **one** [`Entry::Transaction`]
    /// in the journal. If any op fails, the document is left untouched and `(index, error)` identifies which failed.
    pub fn apply_transaction(
        &mut self,
        ops: Vec<Op>,
        label: Option<String>,
        actor: Actor,
    ) -> Result<Vec<Applied>, (usize, OpError)> {
        self.cancel_preview();
        self.end_stroke();
        for (i, op) in ops.iter().enumerate() {
            op.validate().map_err(|e| (i, e))?;
        }
        if ops.is_empty() {
            return Ok(Vec::new());
        }

        // If nested inside an already-open streaming transaction, execute atomically and append to the outer tx.
        if self.open_tx.is_some() {
            let mesh_saved = self.mesh.clone();
            let scene_saved = self.scene.clone();
            let mut results = Vec::with_capacity(ops.len());
            let mut changed_ops = Vec::new();
            let mut last_outcome = None;
            for (i, op) in ops.into_iter().enumerate() {
                match self.execute_unrecorded(&op) {
                    Ok((outcome, changed)) => {
                        let hash = self.hash();
                        if changed {
                            changed_ops.push(op);
                            last_outcome = Some(outcome);
                        }
                        results.push(Applied {
                            outcome,
                            changed,
                            hash,
                        });
                    }
                    Err(e) => {
                        self.swap_mesh(mesh_saved);
                        self.scene = scene_saved;
                        return Err((i, e));
                    }
                }
            }
            if !changed_ops.is_empty() {
                let tx = self.open_tx.as_mut().unwrap();
                tx.ops.extend(changed_ops);
                tx.last_outcome = last_outcome;
            }
            return Ok(results);
        }

        let mesh_before = self.mesh.clone();
        let scene_before = self.scene.clone();
        let mut results = Vec::with_capacity(ops.len());
        let mut applied_ops = Vec::with_capacity(ops.len());
        let mut last_outcome = None;

        for (i, op) in ops.into_iter().enumerate() {
            match self.execute_unrecorded(&op) {
                Ok((outcome, changed)) => {
                    let hash = self.hash();
                    if changed {
                        applied_ops.push(op);
                        last_outcome = Some(outcome);
                    }
                    results.push(Applied {
                        outcome,
                        changed,
                        hash,
                    });
                }
                Err(e) => {
                    self.swap_mesh(mesh_before);
                    self.scene = scene_before;
                    return Err((i, e));
                }
            }
        }

        let mesh_changed = self.mesh != mesh_before;
        let scene_changed = self.scene != scene_before;
        if (mesh_changed || scene_changed) && !applied_ops.is_empty() {
            let outcome = last_outcome.unwrap_or(OpOutcome {
                count: applied_ops.len() as u64,
                vertices: self.mesh.vertex_count() as u64,
                faces: self.mesh.face_count() as u64,
            });
            let hash = self.hash();
            let seq = self.next_seq();
            self.revision += 1;
            let last_op = applied_ops.last().cloned();
            self.journal.entries.push(Entry::Transaction {
                seq,
                label: label.clone(),
                ops_version: OPS_VERSION.to_string(),
                ops: applied_ops,
                actor,
                outcome,
                hash,
            });
            self.push_undo_step(Step {
                op: last_op,
                label,
                mesh: mesh_changed.then_some(mesh_before),
                scene: scene_before,
            });
        }
        Ok(results)
    }

    /// Applies ops in order, all or nothing (recording one undo step per op for backward-compatible callers).
    /// Prefer [`Document::apply_transaction`] when one user action should produce a single undo step.
    pub fn apply_all(
        &mut self,
        ops: Vec<Op>,
        actor: Actor,
    ) -> Result<Vec<Applied>, (usize, OpError)> {
        self.cancel_preview();
        for (i, op) in ops.iter().enumerate() {
            op.validate().map_err(|e| (i, e))?;
        }
        let saved_scene = self.scene.clone();
        let saved = (
            self.mesh.clone(),
            self.undo.len(),
            self.redo.clone(),
            self.journal.entries.len(),
            self.seq,
            self.revision,
        );
        let mut out = Vec::with_capacity(ops.len());
        for (i, op) in ops.into_iter().enumerate() {
            match self.apply(op, actor.clone()) {
                Ok(a) => out.push(a),
                Err(e) => {
                    self.end_stroke();
                    self.swap_mesh(saved.0);
                    self.scene = saved_scene;
                    self.undo.truncate(saved.1);
                    self.redo = saved.2;
                    self.journal.entries.truncate(saved.3);
                    self.seq = saved.4;
                    self.revision = saved.5;
                    return Err((i, e));
                }
            }
        }
        self.end_stroke();
        Ok(out)
    }

    // ---- preview sessions (begin -> update -> commit / cancel) ----

    /// True while an uncommitted preview session is active.
    pub fn in_preview(&self) -> bool {
        self.preview.is_some()
    }

    /// Starts (or resets) a preview session anchored at the current [`Document::revision`]. Previews mutate the live
    /// mesh/scene for display without touching undo history, redo history, the journal, or the document revision.
    pub fn begin_preview(&mut self) -> Result<u64, OpError> {
        if self.open_tx.is_some() {
            return Err(OpError::Invalid(
                "cannot start a preview while a transaction is open".into(),
            ));
        }
        self.end_stroke();
        if let Some(p) = &mut self.preview {
            let m = p.mesh_before.clone();
            let s = p.scene_before.clone();
            p.ops.clear();
            let base = p.base_revision;
            self.swap_mesh(m);
            self.scene = s;
            return Ok(base);
        }
        let base = self.revision;
        self.preview = Some(PreviewSession {
            base_revision: base,
            mesh_before: self.mesh.clone(),
            scene_before: self.scene.clone(),
            ops: Vec::new(),
        });
        Ok(base)
    }

    /// Replaces the current preview with `ops` evaluated from the preview's starting state. When `expected_revision`
    /// is given, rejects stale updates computed against a different document revision.
    pub fn update_preview(
        &mut self,
        ops: Vec<Op>,
        expected_revision: Option<u64>,
    ) -> Result<Vec<Applied>, (usize, OpError)> {
        if self.preview.is_none() {
            self.begin_preview().map_err(|e| (0, e))?;
        }
        let base_rev = self.preview.as_ref().unwrap().base_revision;
        if let Some(expected) = expected_revision {
            if expected != base_rev {
                return Err((
                    0,
                    OpError::Invalid(format!(
                        "stale preview revision: expected {expected}, base is {base_rev}"
                    )),
                ));
            }
        }
        for (i, op) in ops.iter().enumerate() {
            op.validate().map_err(|e| (i, e))?;
        }
        let (m0, s0) = {
            let p = self.preview.as_ref().unwrap();
            (p.mesh_before.clone(), p.scene_before.clone())
        };
        self.swap_mesh(m0.clone());
        self.scene = s0.clone();

        let mut results = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            match self.execute_unrecorded(op) {
                Ok((outcome, changed)) => {
                    results.push(Applied {
                        outcome,
                        changed,
                        hash: self.hash(),
                    });
                }
                Err(e) => {
                    self.swap_mesh(m0);
                    self.scene = s0;
                    self.preview.as_mut().unwrap().ops.clear();
                    return Err((i, e));
                }
            }
        }
        self.preview.as_mut().unwrap().ops = ops;
        Ok(results)
    }

    /// Cancels the active preview session and restores the exact starting mesh and scene.
    pub fn cancel_preview(&mut self) -> bool {
        let Some(p) = self.preview.take() else {
            return false;
        };
        self.swap_mesh(p.mesh_before);
        self.scene = p.scene_before;
        true
    }

    /// Commits the active preview session as one atomic transaction (one undo step). Returns `Ok(None)` if no preview
    /// was active or the previewed operations made no net change.
    pub fn commit_preview(
        &mut self,
        label: Option<String>,
        actor: Actor,
    ) -> Result<Option<Applied>, OpError> {
        let Some(p) = self.preview.take() else {
            return Ok(None);
        };
        self.swap_mesh(p.mesh_before);
        self.scene = p.scene_before;
        if p.ops.is_empty() {
            return Ok(None);
        }
        let results = self
            .apply_transaction(p.ops, label, actor)
            .map_err(|(_, e)| e)?;
        let any_changed = results.iter().any(|r| r.changed);
        if !any_changed {
            return Ok(None);
        }
        Ok(results.into_iter().last())
    }

    // ---- history ----

    /// Takes back the last change. False when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        self.cancel_preview();
        let _ = self.cancel_transaction();
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.end_stroke();
        let (mesh, scene) = self.swap_state(step.mesh, step.scene);
        self.redo.push(Step {
            op: step.op,
            label: step.label,
            mesh,
            scene,
        });
        self.revision += 1;
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Undo { seq, hash });
        true
    }

    /// Puts back the last undone change. False when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        self.cancel_preview();
        let _ = self.cancel_transaction();
        let Some(step) = self.redo.pop() else {
            return false;
        };
        self.end_stroke();
        let (mesh, scene) = self.swap_state(step.mesh, step.scene);
        self.undo.push(Step {
            op: step.op,
            label: step.label,
            mesh,
            scene,
        });
        self.revision += 1;
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Redo { seq, hash });
        true
    }

    /// Back to the mesh and scene as loaded (or last saved). Kept as one undo step.
    pub fn revert(&mut self) -> bool {
        self.cancel_preview();
        let _ = self.cancel_transaction();
        if !self.is_dirty() {
            return false;
        }
        self.end_stroke();
        let mesh = (self.mesh != self.original).then(|| self.original.clone());
        let (mesh, scene) = self.swap_state(mesh, self.original_scene.clone());
        self.undo.push(Step {
            op: None,
            label: Some("revert".into()),
            mesh,
            scene,
        });
        self.redo.clear();
        self.revision += 1;
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Revert { seq, hash });
        true
    }

    /// The current mesh and scene become the new original; undo/redo stacks are cleared (the journal is kept).
    pub fn mark_saved(&mut self) {
        self.cancel_preview();
        let _ = self.cancel_transaction();
        self.end_stroke();
        self.original = self.mesh.clone();
        self.original_scene = self.scene.clone();
        self.undo.clear();
        self.redo.clear();
    }

    // ---- versioned project container persistence (.brush) ----

    /// Packages the entire editing state (surface mesh, baseline if dirty, linked object meshes, scene placements,
    /// edit journal, revision, and optional metadata/features) into a versioned `.brush` container. Uncommitted
    /// previews are excluded automatically.
    pub fn save_project(&self) -> Vec<u8> {
        let (surface, scene) = match &self.preview {
            Some(p) => (&p.mesh_before, &p.scene_before),
            None => (&self.mesh, &self.scene),
        };
        let original_surface = (surface != &self.original).then(|| self.original.clone());
        let scene_json = serde_json::to_string(scene).expect("scene serialises");
        let original_scene_json = (scene != &self.original_scene)
            .then(|| serde_json::to_string(&self.original_scene).expect("scene serialises"));

        io::write_project(&io::ProjectContainer {
            version: io::PROJECT_VERSION,
            ops_version: OPS_VERSION.to_string(),
            revision: self.revision,
            surface: surface.clone(),
            original_surface,
            scene_json,
            original_scene_json,
            objects: self.objects.clone(),
            journal_jsonl: self.journal.to_jsonl(),
            metadata: self.metadata.clone(),
            features: self.features.clone(),
        })
    }

    /// Opens a versioned `.brush` project container saved by [`Document::save_project`].
    pub fn open_project(bytes: &[u8]) -> Result<Document, io::IoError> {
        let proj = io::read_project(bytes)?;
        let scene: Scene = serde_json::from_str(&proj.scene_json)
            .map_err(|e| io::IoError::Malformed(format!("invalid project scene: {e}")))?;
        let original_scene = match proj.original_scene_json {
            Some(s) => serde_json::from_str(&s).map_err(|e| {
                io::IoError::Malformed(format!("invalid project baseline scene: {e}"))
            })?,
            None => scene.clone(),
        };
        let journal = Journal::from_jsonl(&proj.journal_jsonl)
            .map_err(|e| io::IoError::Malformed(format!("invalid project journal: {e}")))?;
        let seq = journal.entries.iter().map(Entry::seq).max().unwrap_or(0);
        let original = proj
            .original_surface
            .unwrap_or_else(|| proj.surface.clone());

        Ok(Document {
            mesh: proj.surface,
            original,
            scene,
            original_scene,
            objects: proj.objects,
            object_bvhs: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            journal,
            seq,
            revision: proj.revision,
            undo_depth: DEFAULT_UNDO_DEPTH,
            caches: Caches::default(),
            stroke: None,
            open_tx: None,
            preview: None,
            metadata: proj.metadata,
            features: proj.features,
        })
    }

    // ---- journal replay ----

    /// Replays a journal's applied ops onto `mesh`, checking each resulting hash. Returns the first entry whose hash
    /// did not match, which means the engines disagree.
    pub fn replay(mesh: Mesh, journal: &Journal) -> Result<Document, ReplayError> {
        Document::replay_scene(mesh, Scene::default(), BTreeMap::new(), journal)
    }

    /// [`Document::replay`] for a document that starts with placed objects, given the object meshes `bakeObject`
    /// needs.
    pub fn replay_scene(
        mesh: Mesh,
        scene: Scene,
        objects: BTreeMap<String, Mesh>,
        journal: &Journal,
    ) -> Result<Document, ReplayError> {
        let mut doc = Document::new(mesh).with_scene(scene);
        doc.objects = objects;
        for e in &journal.entries {
            match e {
                Entry::Applied {
                    seq,
                    op,
                    actor,
                    hash,
                    ..
                } => {
                    let got = doc
                        .apply(op.clone(), actor.clone())
                        .map_err(|err| ReplayError::Op(*seq, err))?;
                    if &got.hash != hash {
                        return Err(ReplayError::Diverged {
                            seq: *seq,
                            expected: hash.clone(),
                            got: got.hash,
                        });
                    }
                }
                Entry::Transaction {
                    seq,
                    label,
                    ops,
                    actor,
                    hash,
                    ..
                } => {
                    doc.apply_transaction(ops.clone(), label.clone(), actor.clone())
                        .map_err(|(_, err)| ReplayError::Op(*seq, err))?;
                    let got_hash = doc.hash();
                    if &got_hash != hash {
                        return Err(ReplayError::Diverged {
                            seq: *seq,
                            expected: hash.clone(),
                            got: got_hash,
                        });
                    }
                }
                Entry::Undo { .. } => {
                    doc.undo();
                }
                Entry::Redo { .. } => {
                    doc.redo();
                }
                Entry::Revert { .. } => {
                    doc.revert();
                }
            }
        }
        Ok(doc)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ReplayError {
    #[error("entry {0}: {1}")]
    Op(u64, OpError),
    #[error("entry {seq}: expected mesh {expected}, got {got}")]
    Diverged {
        seq: u64,
        expected: String,
        got: String,
    },
}

/// JSON Schema of a journal entry.
pub fn journal_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Entry)).expect("schema serialises")
}
