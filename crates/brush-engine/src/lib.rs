//! # brush-engine
//!
//! A [`Document`] is a mesh plus everything about how it got that way. Every change is a typed [`Op`] applied
//! through [`Document::apply`], which:
//!
//! - refuses invalid ops without touching the mesh;
//! - records the op, who sent it (a person, an app, an AI model) and what it did in an append-only [`Journal`],
//!   which can be stored as JSON Lines and replayed;
//! - keeps an undo/redo history of snapshots, so undo and redo are instant whatever the op cost.
//!
//! The original mesh is kept as the reference for `matchColor` with `fromReference` and for [`Document::revert`].
//!
//! A document also has a [`Scene`]: linked instances of other objects placed on its mesh (see [`scene`]). The host
//! supplies each object's mesh with [`Document::set_object`]; [`Document::flattened`] gives the mesh with them all in,
//! for exports. [`Document::place`] turns a pointer position into the op that builds a piece there (see [`place`]).

pub use brush_io as io;
pub use brush_mesh as mesh;
pub use brush_mesh::{Mesh, Op, OpError, OpOutcome, OPS_VERSION};
pub use brush_render as render;

pub mod place;
pub mod scene;

pub use place::{Piece, Placement};
pub use scene::{Instance, Pick, Scene};

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
    /// An op was applied and changed the mesh.
    Applied {
        seq: u64,
        op: Op,
        #[serde(default, skip_serializing_if = "is_default")]
        actor: Actor,
        outcome: OpOutcome,
        /// Content hash of the mesh after the op.
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
    /// False when the op ran but left the mesh exactly as it was; nothing is recorded then.
    pub changed: bool,
    pub hash: String,
}

#[derive(Clone)]
struct Step {
    /// None for a revert.
    op: Option<Op>,
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
    undo_depth: usize,
    caches: Caches,
    stroke: Option<Stroke>,
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
            undo_depth: DEFAULT_UNDO_DEPTH,
            caches: Caches::default(),
            stroke: None,
        }
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

    /// Replaces the mesh after undo, redo or revert, keeping caches that still apply.
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

    /// The document's hash: the mesh's content hash, extended with the scene when anything is placed. Documents without
    /// placed objects hash exactly as their mesh does.
    pub fn hash(&self) -> String {
        let mesh = self.mesh.content_hash();
        if self.scene.is_empty() {
            return mesh;
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let scene = serde_json::to_string(&self.scene).expect("scenes serialise");
        for b in mesh.bytes().chain(scene.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{h:016x}")
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

    pub fn object(&self, object: &str) -> Option<&Mesh> {
        self.objects.get(object)
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
        self.end_stroke();
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
            inst.yaw_deg,
            inst.scale,
        ))
    }

    /// Where an object would be drawn for a given placement (a ghost before placing it).
    pub fn object_transform(
        &self,
        object: &str,
        position: [f64; 3],
        yaw_deg: f64,
        scale: f64,
    ) -> Option<[f32; 16]> {
        let obj = self.objects.get(object)?;
        Some(scene::transform(
            scene::anchor(obj),
            position,
            yaw_deg,
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
                    scale: *scale,
                });
                Ok((outcome(1, &self.mesh), false))
            }
            Op::MoveObject {
                id,
                position,
                yaw_deg,
                scale,
            } => {
                let i = find(&self.scene, id)?;
                let inst = &mut self.scene.instances[i];
                (inst.position, inst.yaw_deg, inst.scale) = (*position, *yaw_deg, *scale);
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
    /// Changes since load or the last [`Document::mark_saved`].
    pub fn is_dirty(&self) -> bool {
        self.mesh != self.original || self.scene != self.original_scene
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Applies one op. An op that changes nothing is not recorded and cannot be undone.
    ///
    /// A `sculpt` op also opens a stroke: [`Document::extend_stroke`] then adds dabs to it in place, so a stroke drawn
    /// live is still one op in the journal and one step of undo. Any other call closes the stroke.
    pub fn apply(&mut self, op: Op, actor: Actor) -> Result<Applied, OpError> {
        self.end_stroke();
        op.validate()?;
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
        self.journal.entries.push(Entry::Applied {
            seq,
            op: op.clone(),
            actor,
            outcome,
            hash,
        });
        self.undo.push(Step {
            op: Some(op),
            mesh: before,
            scene: scene_before,
        });
        if self.undo.len() > self.undo_depth {
            self.undo.remove(0);
        }
        self.redo.clear();
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

    /// Applies ops in order, all or nothing: if one fails, the document is left as it was and the error says which.
    pub fn apply_all(
        &mut self,
        ops: Vec<Op>,
        actor: Actor,
    ) -> Result<Vec<Applied>, (usize, OpError)> {
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
                    return Err((i, e));
                }
            }
        }
        self.end_stroke();
        Ok(out)
    }

    /// Takes back the last change. False when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.end_stroke();
        let (mesh, scene) = self.swap_state(step.mesh, step.scene);
        self.redo.push(Step {
            op: step.op,
            mesh,
            scene,
        });
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Undo { seq, hash });
        true
    }

    /// Puts back the last undone change. False when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        self.end_stroke();
        let (mesh, scene) = self.swap_state(step.mesh, step.scene);
        self.undo.push(Step {
            op: step.op,
            mesh,
            scene,
        });
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Redo { seq, hash });
        true
    }

    /// Back to the mesh as loaded (or last saved). Kept as one undo step.
    pub fn revert(&mut self) -> bool {
        if !self.is_dirty() {
            return false;
        }
        self.end_stroke();
        let mesh = (self.mesh != self.original).then(|| self.original.clone());
        let (mesh, scene) = self.swap_state(mesh, self.original_scene.clone());
        self.undo.push(Step {
            op: None,
            mesh,
            scene,
        });
        self.redo.clear();
        let (seq, hash) = (self.next_seq(), self.hash());
        self.journal.entries.push(Entry::Revert { seq, hash });
        true
    }

    /// The current mesh becomes the new original; history is cleared (the journal is kept).
    pub fn mark_saved(&mut self) {
        self.end_stroke();
        self.original = self.mesh.clone();
        self.original_scene = self.scene.clone();
        self.undo.clear();
        self.redo.clear();
    }

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
