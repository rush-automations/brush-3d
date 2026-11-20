use brush_engine::*;
use serde_json::json;

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}
fn floor() -> Mesh {
    Mesh::new(
        vec![0., 0., 0., 4., 0., 0., 4., 0., 4., 0., 0., 4.],
        vec![120; 12],
        vec![0, 1, 2, 0, 2, 3],
    )
    .unwrap()
}
fn model() -> Actor {
    Actor {
        id: "model:test".into(),
        kind: Some(ActorKind::Model),
    }
}

#[test]
fn undo_redo_and_revert() {
    let mut d = Document::new(floor());
    let h0 = d.mesh().content_hash();
    let a = d
        .apply(
            op(json!({"op": "subdivide", "maxEdge": 1})),
            Actor::default(),
        )
        .unwrap();
    assert!(a.changed && d.is_dirty());
    let h1 = a.hash.clone();
    d.apply(op(json!({"op": "paint", "box": {"center": [1, 0, 1], "size": [2, 1, 2]}, "color": [255, 0, 0]})), model()).unwrap();
    let h2 = d.mesh().content_hash();
    assert!(matches!(d.last_op(), Some(Op::Paint { .. })));

    assert!(d.undo());
    assert_eq!(d.mesh().content_hash(), h1);
    assert!(d.redo());
    assert_eq!(d.mesh().content_hash(), h2);
    assert!(!d.redo());

    assert!(d.revert());
    assert_eq!(d.mesh().content_hash(), h0);
    assert!(d.undo()); // a slip is recoverable
    assert_eq!(d.mesh().content_hash(), h2);
}

#[test]
fn an_op_that_changes_nothing_is_not_recorded() {
    let mut d = Document::new(floor());
    let a = d
        .apply(
            op(json!({"op": "carveBox", "box": {"center": [50, 0, 50], "size": [1, 1, 1]}})),
            Actor::default(),
        )
        .unwrap();
    assert!(!a.changed);
    assert!(!d.can_undo());
    assert!(d.journal().entries.is_empty());
}

#[test]
fn a_batch_is_all_or_nothing() {
    let mut d = Document::new(floor());
    let h = d.mesh().content_hash();
    let bad = d.apply_all(
        vec![op(json!({"op": "subdivide", "maxEdge": 1})), op(json!({"op": "matchColor", "box": {"center": [0, 0, 0], "size": [1, 1, 1]}, "fromReference": true}))],
        Actor::default(),
    );
    // The reference is the original mesh, so this batch is fine; now make one that fails mid-way.
    assert!(bad.is_ok());
    d.undo();
    d.undo();
    assert_eq!(d.mesh().content_hash(), h);
    let journal_len = d.journal().entries.len();
    let err = d.apply_all(
        vec![
            op(json!({"op": "subdivide", "maxEdge": 1})),
            op(json!({"op": "addExtrude", "polygon": [[0, 0], [1, 0]], "y0": 0, "height": 1})),
        ],
        Actor::default(),
    );
    assert_eq!(err.unwrap_err().0, 1);
    assert_eq!(d.mesh().content_hash(), h);
    assert_eq!(d.journal().entries.len(), journal_len);
}

#[test]
fn a_journal_replays_to_the_same_mesh_and_detects_divergence() {
    let mut d = Document::new(floor());
    d.apply(op(json!({"op": "subdivide", "maxEdge": 0.5})), model())
        .unwrap();
    d.apply(
        op(json!({"op": "roughen", "amplitude": 0.05, "scale": 0.5, "seed": 9})),
        model(),
    )
    .unwrap();
    d.undo();
    d.apply(op(json!({"op": "tint", "a": [200, 10, 10], "b": [10, 10, 200], "scale": 1, "strength": 0.7})), Actor::default()).unwrap();
    d.apply(op(json!({"op": "carveBox", "box": {"center": [2, 0, 2], "size": [1, 1, 1], "yawDeg": 30}})), Actor::default()).unwrap();
    let text = d.journal().to_jsonl();
    assert_eq!(text.lines().count(), 5);
    let j = Journal::from_jsonl(&text).unwrap();
    let again = Document::replay(floor(), &j).unwrap();
    assert_eq!(again.mesh().content_hash(), d.mesh().content_hash());

    let mut other = floor();
    other.colors[0] = 1;
    assert!(matches!(
        Document::replay(other, &j),
        Err(ReplayError::Diverged { seq: 1, .. })
    ));
}

#[test]
fn journal_lines_are_plain_json() {
    let mut d = Document::new(floor());
    d.apply(op(json!({"op": "fillHoles"})), model())
        .unwrap_or_else(|_| unreachable!());
    d.apply(op(json!({"op": "decimate", "cell": 10})), model())
        .unwrap();
    let line: serde_json::Value =
        serde_json::from_str(d.journal().to_jsonl().lines().last().unwrap()).unwrap();
    assert_eq!(line["event"], "applied");
    assert_eq!(line["op"]["op"], "decimate");
    assert_eq!(line["actor"], json!({"id": "model:test", "kind": "model"}));
    assert!(line["hash"].as_str().unwrap().len() == 16);
}

fn dense_floor() -> Document {
    let mut d = Document::new(floor());
    d.apply(
        op(json!({"op": "subdivide", "maxEdge": 0.1})),
        Actor::default(),
    )
    .unwrap();
    d.mark_saved();
    d
}

fn dab(x: f64) -> Dab {
    serde_json::from_value(json!({"center": [x, 0, 2]})).unwrap()
}

#[test]
fn a_live_stroke_is_one_journal_entry_and_one_undo_step() {
    let mut d = dense_floor();
    let base = d.mesh().content_hash();
    let journal_before = d.journal().entries.len();
    d.apply(op(json!({"op": "sculpt", "brush": "draw", "dabs": [{"center": [1, 0, 2]}], "radius": 0.4, "view": [0, -1, 0]})), model()).unwrap();
    for k in 1..15 {
        d.extend_stroke(vec![dab(1.0 + k as f64 * 0.1)]).unwrap();
    }
    d.end_stroke();
    assert_eq!(d.journal().entries.len(), journal_before + 1);
    let Some(Entry::Applied {
        op: Op::Sculpt { dabs, .. },
        ..
    }) = d.journal().entries.last()
    else {
        panic!("not a sculpt entry")
    };
    assert_eq!(dabs.len(), 15);
    let live = d.mesh().content_hash();

    // Replaying the one recorded op gives exactly the live result.
    let mut again = dense_floor();
    let Some(Entry::Applied { op, .. }) = d.journal().entries.last().cloned() else {
        unreachable!()
    };
    again.apply(op, Actor::default()).unwrap();
    assert_eq!(again.mesh().content_hash(), live);

    assert!(d.undo());
    assert_eq!(
        d.mesh().content_hash(),
        base,
        "one undo takes back the whole stroke"
    );
    assert!(
        d.extend_stroke(vec![dab(2.0)]).is_err(),
        "undo closed the stroke"
    );
}

#[test]
fn a_stroke_that_starts_off_the_surface_is_recorded_once_it_touches() {
    let mut d = dense_floor();
    let n = d.journal().entries.len();
    let miss = d.apply(op(json!({"op": "sculpt", "brush": "draw", "dabs": [{"center": [50, 0, 50]}], "radius": 0.3, "view": [0, -1, 0]})), Actor::default()).unwrap();
    assert!(!miss.changed);
    assert_eq!(d.journal().entries.len(), n);
    assert!(d.extend_stroke(vec![dab(2.0)]).unwrap().changed);
    assert_eq!(d.journal().entries.len(), n + 1);
}

#[test]
fn picking_follows_sculpting_and_topology_changes() {
    let mut d = dense_floor();
    let down = [0.0, -1.0, 0.0];
    let h = d.raycast([2.0, 5.0, 2.0], down).unwrap();
    assert!(h.point[1].abs() < 1e-5);
    d.apply(op(json!({"op": "sculpt", "brush": "draw", "dabs": [{"center": [2, 0, 2]}], "radius": 0.5, "strength": 1, "view": [0, -1, 0]})), Actor::default()).unwrap();
    for _ in 0..5 {
        d.extend_stroke(vec![dab(2.0)]).unwrap();
    }
    let raised = d.raycast([2.0, 5.0, 2.0], down).unwrap();
    assert!(
        raised.point[1] > 0.05,
        "the refitted tree sees the bump: {raised:?}"
    );
    d.apply(
        op(json!({"op": "carveBox", "box": {"center": [2, 0, 2], "size": [0.3, 2, 0.3]}})),
        Actor::default(),
    )
    .unwrap();
    assert!(
        d.raycast([2.0, 5.0, 2.0], down).is_none(),
        "the rebuilt tree sees the hole"
    );
    d.undo();
    assert!(d.raycast([2.0, 5.0, 2.0], down).is_some());
    assert_eq!(d.vertex_normals().len(), d.mesh().vertex_count() * 3);
}
