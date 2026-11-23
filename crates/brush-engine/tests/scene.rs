//! Placed objects: linked instances in a document's scene.

use brush_engine::{Actor, Document, Mesh, Op, Piece, Placement};

fn cube(size: f32, rgb: [u8; 3]) -> Mesh {
    let mut m = Mesh::default();
    m.add_box([0.0, 0.0, 0.0], size, size, size, 0.0, rgb);
    m
}

fn floor() -> Document {
    let mut m = Mesh::default();
    m.add_box([0.0, -0.1, 0.0], 10.0, 0.1, 10.0, 0.0, [90, 90, 90]);
    Document::new(m)
}

fn place(id: &str, x: f64) -> Op {
    Op::PlaceObject {
        id: id.into(),
        object: "scan:chair".into(),
        position: [x, 0.0, 0.0],
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        roll_deg: 0.0,
        scale: 1.0,
    }
}

#[test]
fn instances_are_journaled_undoable_and_leave_the_mesh_alone() {
    let mut doc = floor();
    let mesh_hash = doc.mesh().content_hash();
    assert_eq!(
        doc.hash(),
        mesh_hash,
        "no scene: the document hashes as its mesh"
    );
    let a = doc.apply(place("a", 1.0), Actor::default()).unwrap();
    assert!(a.changed);
    assert_ne!(
        a.hash, mesh_hash,
        "the scene is part of the document's hash"
    );
    assert_eq!(doc.mesh().content_hash(), mesh_hash);
    assert!(doc.is_dirty());

    doc.apply(
        Op::MoveObject {
            id: "a".into(),
            position: [2.0, 0.0, 0.0],
            yaw_deg: 90.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            scale: 2.0,
        },
        Actor::default(),
    )
    .unwrap();
    assert_eq!(doc.scene().instances[0].position, [2.0, 0.0, 0.0]);
    assert!(doc.undo());
    assert_eq!(doc.scene().instances[0].position, [1.0, 0.0, 0.0]);
    assert!(doc.redo());
    assert_eq!(doc.scene().instances[0].scale, 2.0);
    assert!(
        doc.apply(place("a", 0.0), Actor::default()).is_err(),
        "ids are unique"
    );
    assert!(doc
        .apply(Op::RemoveObject { id: "nope".into() }, Actor::default())
        .is_err());

    doc.mark_saved();
    assert!(!doc.is_dirty());
    doc.apply(Op::RemoveObject { id: "a".into() }, Actor::default())
        .unwrap();
    assert!(doc.revert());
    assert_eq!(doc.scene().instances.len(), 1);
}

#[test]
fn an_object_change_reaches_every_instance() {
    let mut doc = floor();
    doc.apply(place("a", 1.0), Actor::default()).unwrap();
    doc.apply(place("b", -1.0), Actor::default()).unwrap();
    assert_eq!(doc.missing_objects(), vec!["scan:chair".to_string()]);
    let base_faces = doc.mesh().face_count();

    doc.set_object("scan:chair", cube(0.5, [200, 0, 0]));
    let flat = doc.flattened();
    assert_eq!(flat.face_count(), base_faces + 2 * 12);
    assert!(doc.missing_objects().is_empty());

    // The chair is edited elsewhere (now with a second box): both instances follow, without a journal entry.
    let journal = doc.journal().entries.len();
    let mut chair = cube(0.5, [0, 0, 200]);
    chair.add_box([0.0, 0.5, 0.0], 0.1, 0.5, 0.1, 0.0, [0, 0, 200]);
    doc.set_object("scan:chair", chair);
    let flat = doc.flattened();
    assert_eq!(flat.face_count(), base_faces + 2 * 24);
    assert_eq!(doc.journal().entries.len(), journal);
    assert!(flat.colors.chunks_exact(3).any(|c| c == [0, 0, 200]));
}

#[test]
fn baking_copies_an_instance_into_the_mesh_and_undo_takes_it_back() {
    let mut doc = floor();
    doc.apply(place("a", 1.0), Actor::default()).unwrap();
    assert!(
        doc.apply(Op::BakeObject { id: "a".into() }, Actor::default())
            .is_err(),
        "needs the object's mesh"
    );
    doc.set_object("scan:chair", cube(0.5, [1, 2, 3]));
    let faces = doc.mesh().face_count();
    let r = doc
        .apply(Op::BakeObject { id: "a".into() }, Actor::default())
        .unwrap();
    assert_eq!(r.outcome.count, 12);
    assert_eq!(doc.mesh().face_count(), faces + 12);
    assert!(doc.scene().is_empty());
    assert!(doc.undo());
    assert_eq!(doc.mesh().face_count(), faces);
    assert_eq!(doc.scene().instances.len(), 1);
}

#[test]
fn picking_finds_the_nearest_of_mesh_and_instances() {
    let mut doc = floor();
    doc.set_object("scan:chair", cube(1.0, [1, 2, 3]));
    doc.apply(place("a", 2.0), Actor::default()).unwrap();
    let down = [0.0, -1.0, 0.0];
    let on_chair = doc.pick([2.0, 5.0, 0.0], down).unwrap();
    assert_eq!(on_chair.instance.as_deref(), Some("a"));
    assert!((on_chair.hit.point[1] - 1.0).abs() < 1e-4);
    let on_floor = doc.pick([-2.0, 5.0, 0.0], down).unwrap();
    assert_eq!(on_floor.instance, None);
}

#[test]
fn place_builds_pieces_and_instances_that_stack() {
    let mut doc = floor();
    doc.set_object("scan:chair", cube(0.5, [1, 2, 3]));
    let req = |piece, point: [f64; 3]| Placement {
        piece,
        object: (piece == Piece::Object).then(|| "scan:chair".to_string()),
        id: None,
        point,
        normal: [0.0, 1.0, 0.0],
        view: [0.0, -0.5, 1.0],
        cell: 0.5,
        snap: true,
        ground: 0.0,
        turns: 0,
        span: [1, 1, 1],
        scale: 1.0,
        color: [7, 7, 7],
    };
    let op = doc.place(&req(Piece::Cube, [0.1, 0.0, 0.1])).unwrap();
    doc.apply(op, Actor::default()).unwrap();
    // Aim at the top of that cube and place the chair on it.
    let top = doc.pick([0.25, 5.0, 0.25], [0.0, -1.0, 0.0]).unwrap();
    let op = doc
        .place(&Placement {
            normal: top.hit.normal.map(f64::from),
            ..req(Piece::Object, top.hit.point.map(f64::from))
        })
        .unwrap();
    let Op::PlaceObject { id, position, .. } = &op else {
        panic!("{op:?}")
    };
    assert_eq!(id, "o1");
    assert_eq!(*position, [0.25, 0.5, 0.25]);
    doc.apply(op, Actor::default()).unwrap();
    let t = doc.instance_transform("o1").unwrap();
    assert_eq!(
        [t[12], t[13], t[14]],
        [0.25, 0.5, 0.25],
        "the chair's bottom centre stands on the cube"
    );
}

#[test]
fn journals_with_scenes_replay() {
    let mut doc = floor();
    let start = doc.mesh().clone();
    doc.set_object("scan:chair", cube(0.5, [1, 2, 3]));
    doc.apply(place("a", 1.0), Actor::default()).unwrap();
    doc.apply(place("b", 2.0), Actor::default()).unwrap();
    doc.apply(Op::BakeObject { id: "b".into() }, Actor::default())
        .unwrap();
    doc.undo();
    doc.apply(
        Op::MoveObject {
            id: "a".into(),
            position: [0.0, 1.0, 0.0],
            yaw_deg: 45.0,
            pitch_deg: 30.0,
            roll_deg: -15.0,
            scale: 1.0,
        },
        Actor::default(),
    )
    .unwrap();
    let objects = [("scan:chair".to_string(), cube(0.5, [1, 2, 3]))]
        .into_iter()
        .collect();
    let again = Document::replay_scene(start, Default::default(), objects, doc.journal()).unwrap();
    assert_eq!(again.hash(), doc.hash());
    assert_eq!(again.scene(), doc.scene());
}
