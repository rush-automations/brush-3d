//! High-level editing workflows built on top of [`crate::Document`] transactions:
//! detaching surface islands or box sections into movable objects, deleting islands cleanly,
//! and combining placed objects into a single independent asset.

use brush_mesh::op::Context;
use brush_mesh::{BoxRegion, Mesh, Op, OpError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{scene, Actor, Applied, Document};

/// Result of detaching geometry or combining placed instances into a standalone object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetachedPart {
    /// ID of the newly placed [`crate::Instance`] in `doc.scene()`.
    pub instance_id: String,
    /// Object key under which the new mesh was registered in `doc`.
    pub object: String,
    /// World-space bottom-centre anchor of the extracted mesh, meters.
    pub anchor: [f64; 3],
    /// Face count of the extracted object mesh.
    pub faces: u64,
    /// Number of connected islands grouped into this part (`1` unless `tangled` merged overlapping islands).
    pub islands: u64,
    /// True when the detached faces comprised the entire surface mesh.
    pub is_whole_mesh: bool,
}

fn anchor64(mesh: &Mesh) -> [f64; 3] {
    let a = scene::anchor(mesh);
    [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])]
}

impl Document {
    /// Deletes the connected mesh island containing `start_face` as a single undoable step, even when its bounding box
    /// interlocks with neighbouring islands.
    pub fn delete_island(&mut self, start_face: u32, actor: Actor) -> Result<Applied, OpError> {
        let comp = self
            .mesh()
            .connected_component(start_face)
            .ok_or_else(|| OpError::Invalid(format!("face {start_face} is out of range")))?;
        self.apply(
            Op::RemoveFaces {
                faces: comp.faces,
                invert: false,
            },
            actor,
        )
    }

    /// Lifts the connected island (or the entire tangled cluster when `tangled` is true) containing `start_face` out of
    /// the surface mesh, registers it under `object_key`, and places an instance at its exact world position in one
    /// atomic transaction (one undo step).
    pub fn detach_island(
        &mut self,
        start_face: u32,
        tangled: bool,
        object_key: String,
        instance_id: Option<String>,
        actor: Actor,
    ) -> Result<DetachedPart, OpError> {
        if object_key.is_empty() || object_key.len() > 256 {
            return Err(OpError::Invalid(
                "object key must be 1 to 256 characters".into(),
            ));
        }
        let (faces, islands, is_whole_mesh) =
            if tangled {
                let c = self.mesh().tangled_cluster(start_face).ok_or_else(|| {
                    OpError::Invalid(format!("face {start_face} is out of range"))
                })?;
                (c.faces, c.islands as u64, c.is_whole_mesh)
            } else {
                let c = self.mesh().connected_component(start_face).ok_or_else(|| {
                    OpError::Invalid(format!("face {start_face} is out of range"))
                })?;
                (c.faces, 1, c.is_whole_mesh)
            };

        let part_mesh = self.mesh().extract_faces(&faces)?;
        let part_faces = part_mesh.face_count() as u64;
        let anchor = anchor64(&part_mesh);
        let inst_id = instance_id.unwrap_or_else(|| self.scene().free_id());
        if self.scene().get(&inst_id).is_some() {
            return Err(OpError::DuplicateInstance(inst_id));
        }

        self.set_object(object_key.clone(), part_mesh);
        let ops = vec![
            Op::RemoveFaces {
                faces,
                invert: false,
            },
            Op::PlaceObject {
                id: inst_id.clone(),
                object: object_key.clone(),
                position: anchor,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                scale: 1.0,
            },
        ];
        if let Err((_, err)) = self.apply_transaction(ops, Some("detachIsland".into()), actor) {
            self.remove_object_asset(&object_key);
            return Err(err);
        }

        Ok(DetachedPart {
            instance_id: inst_id,
            object: object_key,
            anchor,
            faces: part_faces,
            islands,
            is_whole_mesh,
        })
    }

    /// Extracts the geometry inside `region` into a standalone object registered under `object_key`, carves `region`
    /// out of the surface mesh, and places the new object at its exact world anchor in one transaction (one undo step).
    pub fn detach_box(
        &mut self,
        region: BoxRegion,
        object_key: String,
        instance_id: Option<String>,
        actor: Actor,
    ) -> Result<DetachedPart, OpError> {
        if object_key.is_empty() || object_key.len() > 256 {
            return Err(OpError::Invalid(
                "object key must be 1 to 256 characters".into(),
            ));
        }
        let mut scratch = self.mesh().clone();
        Op::RemoveBox {
            region,
            invert: true,
        }
        .apply(&mut scratch, Context::default())?;
        if scratch.face_count() == 0 {
            return Err(OpError::Invalid("no geometry inside box to detach".into()));
        }

        let part_faces = scratch.face_count() as u64;
        let is_whole_mesh = scratch.face_count() == self.mesh().face_count();
        let anchor = anchor64(&scratch);
        let inst_id = instance_id.unwrap_or_else(|| self.scene().free_id());
        if self.scene().get(&inst_id).is_some() {
            return Err(OpError::DuplicateInstance(inst_id));
        }

        self.set_object(object_key.clone(), scratch);
        let ops = vec![
            Op::CarveBox { region },
            Op::PlaceObject {
                id: inst_id.clone(),
                object: object_key.clone(),
                position: anchor,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                scale: 1.0,
            },
        ];
        if let Err((_, err)) = self.apply_transaction(ops, Some("detachBox".into()), actor) {
            self.remove_object_asset(&object_key);
            return Err(err);
        }

        Ok(DetachedPart {
            instance_id: inst_id,
            object: object_key,
            anchor,
            faces: part_faces,
            islands: 1,
            is_whole_mesh,
        })
    }

    /// Combines two or more placed instances into a single standalone object registered under `object_key`, replacing
    /// the original instances in one transaction (one undo step). Preserves tilt, scale, attributes, and splats.
    pub fn combine_instances(
        &mut self,
        ids: &[String],
        object_key: String,
        instance_id: Option<String>,
        actor: Actor,
    ) -> Result<DetachedPart, OpError> {
        if ids.len() < 2 {
            return Err(OpError::Invalid(
                "combineInstances requires at least 2 instance ids".into(),
            ));
        }
        if object_key.is_empty() || object_key.len() > 256 {
            return Err(OpError::Invalid(
                "object key must be 1 to 256 characters".into(),
            ));
        }

        let mut selected = Vec::with_capacity(ids.len());
        for id in ids {
            if selected.iter().any(|(seen, _): &(String, _)| seen == id) {
                return Err(OpError::Invalid(format!("duplicate instance id {id}")));
            }
            let inst = self
                .scene()
                .get(id)
                .ok_or_else(|| OpError::NoInstance(id.clone()))?
                .clone();
            if self.object(&inst.object).is_none() {
                return Err(OpError::MissingObject(inst.object));
            }
            selected.push((id.clone(), inst));
        }

        let mut combined = Mesh::default();
        for (_, inst) in &selected {
            let obj = self.object(&inst.object).unwrap();
            scene::bake(&mut combined, obj, inst);
        }
        if combined.face_count() == 0 && combined.vertex_count() == 0 {
            return Err(OpError::Invalid(
                "combined instances contain no geometry".into(),
            ));
        }

        let anchor = anchor64(&combined);
        let part_faces = combined.face_count() as u64;

        let mut ops: Vec<Op> = selected
            .iter()
            .map(|(id, _)| Op::RemoveObject { id: id.clone() })
            .collect();

        let new_id = match instance_id {
            Some(id) => {
                let being_removed = ids.iter().any(|old| old == &id);
                if !being_removed && self.scene().get(&id).is_some() {
                    return Err(OpError::DuplicateInstance(id));
                }
                id
            }
            None => {
                // Pick the lowest free `oN` after removing `ids`.
                (1..)
                    .map(|n| format!("o{n}"))
                    .find(|cand| {
                        self.scene().get(cand).is_none() || ids.iter().any(|old| old == cand)
                    })
                    .unwrap()
            }
        };

        ops.push(Op::PlaceObject {
            id: new_id.clone(),
            object: object_key.clone(),
            position: anchor,
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            scale: 1.0,
        });

        self.set_object(object_key.clone(), combined);
        if let Err((_, err)) = self.apply_transaction(ops, Some("combineInstances".into()), actor) {
            self.remove_object_asset(&object_key);
            return Err(err);
        }

        Ok(DetachedPart {
            instance_id: new_id,
            object: object_key,
            anchor,
            faces: part_faces,
            islands: selected.len() as u64,
            is_whole_mesh: false,
        })
    }
}
