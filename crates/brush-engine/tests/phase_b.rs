//! Phase B integration suite: document revisions, explicit transactions, preview sessions,
//! shared selection/group transforms, detach/join workflows, and `.brush` project persistence.

use brush_engine::selection::*;
use brush_engine::*;
use serde_json::json;

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}

fn cube_mesh() -> Mesh {
    let mut m = Mesh::default();
    op(json!({"op":"addBox","box":{"center":[0,0.5,0],"size":[1,1,1]},"color":[200,100,50]}))
        .apply(&mut m, mesh::op::Context::default())
        .unwrap();
    m
}

fn inst(id: &str, x: f64, y: f64, z: f64, yaw_deg: f64, scale: f64) -> Instance {
    Instance {
        id: id.into(),
        object: format!("obj:{id}"),
        position: [x, y, z],
        yaw_deg,
        pitch_deg: 0.0,
        roll_deg: 0.0,
        scale,
    }
}

#[test]
fn revisions_transactions_and_streaming_dyntopo_are_single_undo_steps_and_replay() {
    let mut doc = Document::new(Mesh::default());
    assert_eq!(doc.revision(), 0);

    // Atomic batch transaction: 3 boxes built in one user action = 1 undo step, 1 revision bump.
    doc.apply_transaction(
        vec![
            op(json!({"op":"addBox","box":{"center":[0,0.5,0],"size":[1,1,1]}})),
            op(json!({"op":"addBox","box":{"center":[2,0.5,0],"size":[1,1,1]}})),
            op(json!({"op":"addBox","box":{"center":[4,0.5,0],"size":[1,1,1]}})),
        ],
        Some("buildRow".into()),
        Actor::default(),
    )
    .unwrap();
    assert_eq!(doc.revision(), 1);
    assert_eq!(doc.mesh().face_count(), 36);
    assert_eq!(doc.last_transaction_label(), Some("buildRow"));
    assert_eq!(doc.journal().entries.len(), 1);

    // One undo takes back all 3 boxes at once.
    assert!(doc.undo());
    assert_eq!(doc.revision(), 2);
    assert_eq!(doc.mesh().face_count(), 0);
    assert!(!doc.can_undo());
    assert!(doc.redo());
    assert_eq!(doc.revision(), 3);
    assert_eq!(doc.mesh().face_count(), 36);

    // Streaming transaction: subdivide + live multi-dab sculpt stroke = 1 undo step!
    let rev_before = doc.revision();
    let hash_before = doc.hash();
    doc.begin_transaction(Some("autoDetailSculpt".into()), Actor::default())
        .unwrap();
    doc.apply(
        op(json!({"op":"subdivide","maxEdge":0.25})),
        Actor::default(),
    )
    .unwrap();
    doc.apply(
        op(json!({"op":"sculpt","brush":"draw","dabs":[{"center":[0,1,0]}],"radius":0.6,"view":[0,-1,0]})),
        Actor::default(),
    )
    .unwrap();
    doc.extend_stroke(vec![
        serde_json::from_value(json!({"center":[0.1,1,0]})).unwrap()
    ])
    .unwrap();
    let committed = doc.commit_transaction().unwrap();
    assert!(committed.changed);
    assert_eq!(doc.revision(), rev_before + 1);
    let hash_after = doc.hash();

    // Replay reproduces the exact mesh and scene hash, including opsVersion metadata.
    let journal_text = doc.journal().to_jsonl();
    assert!(journal_text.contains(&format!("\"opsVersion\":\"{OPS_VERSION}\"")));
    let parsed = Journal::from_jsonl(&journal_text).unwrap();
    let replayed = Document::replay(Mesh::default(), &parsed).unwrap();
    assert_eq!(replayed.hash(), hash_after);

    // Undoing the streaming transaction restores the exact pre-subdivide mesh in one step.
    assert!(doc.undo());
    assert_eq!(doc.hash(), hash_before);

    // Cancelling a streaming transaction leaves history and revision untouched.
    let r = doc.revision();
    let j = doc.journal().entries.len();
    doc.begin_transaction(Some("aborted".into()), Actor::default())
        .unwrap();
    doc.apply(
        op(json!({"op":"addBox","box":{"center":[10,0.5,0],"size":[1,1,1]}})),
        Actor::default(),
    )
    .unwrap();
    assert!(doc.cancel_transaction());
    assert_eq!(doc.revision(), r);
    assert_eq!(doc.journal().entries.len(), j);
    assert_eq!(doc.hash(), hash_before);
}

#[test]
fn preview_sessions_never_pollute_history_check_revisions_and_commit_atomically() {
    let mut doc = Document::new(Mesh::default());
    doc.set_object("scan:chair", cube_mesh());
    doc.apply(
        op(json!({"op":"placeObject","id":"c1","object":"scan:chair","position":[0,0,0]})),
        Actor::default(),
    )
    .unwrap();
    doc.mark_saved();

    let base_rev = doc.revision();
    let base_hash = doc.hash();
    let journal_len = doc.journal().entries.len();

    assert_eq!(doc.begin_preview().unwrap(), base_rev);
    doc.update_preview(
        vec![op(json!({"op":"moveObject","id":"c1","position":[3,0,2]}))],
        Some(base_rev),
    )
    .unwrap();
    assert!(doc.in_preview());
    assert_eq!(doc.scene().get("c1").unwrap().position, [3.0, 0.0, 2.0]);
    assert_eq!(doc.revision(), base_rev, "preview must not bump revision");
    assert!(
        !doc.is_dirty(),
        "uncommitted preview does not mark saved state dirty"
    );
    assert!(!doc.can_undo(), "preview must not push undo steps");
    assert_eq!(doc.journal().entries.len(), journal_len);

    // Stale revision is rejected.
    assert!(doc
        .update_preview(
            vec![op(json!({"op":"moveObject","id":"c1","position":[5,0,0]}))],
            Some(base_rev + 99),
        )
        .is_err());

    // Cancel restores the exact starting state.
    assert!(doc.cancel_preview());
    assert!(!doc.in_preview());
    assert_eq!(doc.hash(), base_hash);
    assert_eq!(doc.scene().get("c1").unwrap().position, [0.0, 0.0, 0.0]);

    // Begin -> multiple updates -> commit records a single undo step.
    doc.begin_preview().unwrap();
    doc.update_preview(
        vec![op(json!({"op":"moveObject","id":"c1","position":[1,0,0]}))],
        Some(base_rev),
    )
    .unwrap();
    doc.update_preview(
        vec![op(json!({"op":"moveObject","id":"c1","position":[4,0,2]}))],
        Some(base_rev),
    )
    .unwrap();
    let committed = doc
        .commit_preview(Some("dragMove".into()), Actor::default())
        .unwrap()
        .unwrap();
    assert!(committed.changed);
    assert_eq!(doc.revision(), base_rev + 1);
    assert_eq!(doc.scene().get("c1").unwrap().position, [4.0, 0.0, 2.0]);
    assert_eq!(doc.journal().entries.len(), journal_len + 1);
    assert!(doc.undo());
    assert_eq!(doc.hash(), base_hash);
}

#[test]
fn selection_group_transforms_and_snapping_match_engine_conventions() {
    // 90° about +x tips upright +y onto +z -> yaw=0, pitch=90, roll=0.
    let turned = rotate_group_about(
        &[inst("a", 0.0, 0.0, 0.0, 0.0, 1.0)],
        Axis::X,
        90.0,
        [0.0; 3],
    );
    assert_eq!(
        (turned[0].yaw_deg, turned[0].pitch_deg, turned[0].roll_deg),
        (0.0, 90.0, 0.0)
    );

    // Four 90° tilts return to exact zero without floating-point drift.
    let mut loop_list = vec![inst("a", 1.0, 0.0, 0.0, 0.0, 1.0)];
    for _ in 0..4 {
        loop_list = rotate_group_about(&loop_list, Axis::X, 90.0, [0.0; 3]);
    }
    assert_eq!((loop_list[0].pitch_deg, loop_list[0].roll_deg), (0.0, 0.0));
    assert!((loop_list[0].position[0] - 1.0).abs() < 1e-9);

    // Snapping: single off-square object snaps back square to world axes; multi-object snaps to 90° stops.
    let tilted30 = [inst("a", 0.0, 0.0, 0.0, 30.0, 1.0)];
    assert_eq!(snap_turn(-27.0, Axis::Y, &tilted30, true), -30.0);
    assert_eq!(snap_turn(63.0, Axis::Y, &tilted30, true), 60.0);
    let group = [
        inst("a", 0.0, 0.0, 0.0, 30.0, 1.0),
        inst("b", 1.0, 0.0, 0.0, 30.0, 1.0),
    ];
    assert_eq!(snap_turn(-27.0, Axis::Y, &group, true), -27.0);
    assert_eq!(snap_turn(93.0, Axis::Y, &group, true), 90.0);

    // Group scale clamps uniformly so proportions hold.
    let scaled = scale_group(
        &[
            inst("a", 0.0, 0.0, 0.0, 0.0, 1.0),
            inst("b", 1.0, 0.0, 0.0, 0.0, 10.0),
        ],
        4.0,
        None,
    );
    assert!((scaled[1].scale - 20.0).abs() < 1e-9);
    assert!((scaled[0].scale - 2.0).abs() < 1e-9);

    // Drag delta and copy/paste round-trip.
    assert_eq!(
        drag_delta([0.0, 0.0, 0.0], [0.74, 3.0, -0.26], 0.5, true),
        [0.5, 0.0, -0.5]
    );
    let clip = copy_group(
        &[
            inst("a", 0.0, 0.0, 0.0, 90.0, 2.0),
            inst("b", 2.0, 1.0, 0.0, 0.0, 1.0),
        ],
        3.0,
    );
    let ids = free_ids(&[inst("o1", 0.0, 0.0, 0.0, 0.0, 1.0)], 2);
    assert_eq!(ids, vec!["o2", "o3"]);
    let pasted = paste_group(&clip, [10.0, 0.0, 5.0], &ids);
    assert_eq!(pasted[0].position, [9.0, 0.0, 5.0]);
    assert_eq!(pasted[1].position, [11.0, 1.0, 5.0]);
}

#[test]
fn detach_interlocking_island_detach_box_combine_and_project_container_round_trip() {
    let mut doc = Document::new(Mesh::default());
    // Two overlapping boxes + one separate box.
    doc.apply_transaction(
        vec![
            op(json!({"op":"addBox","box":{"center":[0,0.5,0],"size":[1,1,1]},"color":[220,40,40]})),
            op(json!({"op":"addBox","box":{"center":[0.3,0.5,0.3],"size":[1,1,1]},"color":[40,220,40]})),
            op(json!({"op":"addBox","box":{"center":[5,0.5,0],"size":[1,1,1]},"color":[40,40,220]})),
        ],
        Some("seed".into()),
        Actor::default(),
    )
    .unwrap();
    let seed_hash = doc.hash();

    // Detaching ONLY the first interlocking island keeps the overlapping second box intact in the surface mesh
    // AND preserves full undo history (1 undo step!).
    let part1 = doc
        .detach_island(0, false, "local:part-1".into(), None, Actor::default())
        .unwrap();
    assert_eq!(part1.instance_id, "o1");
    assert_eq!(part1.faces, 12);
    assert_eq!(part1.islands, 1);
    assert_eq!(doc.mesh().face_count(), 24);
    assert_eq!(doc.scene().instances.len(), 1);
    assert!(doc.undo());
    assert_eq!(doc.hash(), seed_hash);
    assert!(doc.redo());

    // Detaching a box section from the distant third box creates o2 in one undo step.
    let part2 = doc
        .detach_box(
            BoxRegion {
                center: [5.0, 0.5, 0.0],
                size: [2.0, 2.0, 2.0],
                yaw_deg: 0.0,
            },
            "local:part-2".into(),
            None,
            Actor::default(),
        )
        .unwrap();
    assert_eq!(part2.instance_id, "o2");
    assert_eq!(doc.scene().instances.len(), 2);

    // Combine o1 and o2 into a single object in one undo step.
    let combined = doc
        .combine_instances(
            &["o1".into(), "o2".into()],
            "local:combined".into(),
            None,
            Actor::default(),
        )
        .unwrap();
    assert_eq!(combined.instance_id, "o1");
    assert_eq!(combined.faces, 24);
    assert_eq!(doc.scene().instances.len(), 1);

    // Save and reopen as a `.brush` project container: meshes, linked objects, scene, journal, revision, and
    // design-intent features all survive intact!
    doc.set_metadata(json!({"title": "Block + Freeform Test"}));
    doc.set_features(vec![json!({"id": "f1", "kind": "wall", "length": 4.0})]);
    let expected_hash = doc.hash();
    let expected_rev = doc.revision();
    let expected_journal = doc.journal().entries.len();

    let project_bytes = doc.save_project();
    let reopened = Document::open_project(&project_bytes).unwrap();
    assert_eq!(reopened.hash(), expected_hash);
    assert_eq!(reopened.revision(), expected_rev);
    assert_eq!(reopened.journal().entries.len(), expected_journal);
    assert!(reopened.missing_objects().is_empty());
    assert_eq!(reopened.metadata()["title"], "Block + Freeform Test");
    assert_eq!(reopened.features().len(), 1);
    assert!(reopened.is_dirty(), "dirty baseline survives save/open");
}
