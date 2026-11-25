use brush_engine::precision::{measure_distance, AlignAxis, AlignMode};
use brush_engine::{
    Actor, ConstraintKind, ConstraintRecord, Document, FeatureKind, FeatureSpec, FeatureStatus,
    Mesh, Op, Workplane,
};

#[test]
fn workplane_projection_unprojection_and_snapping() {
    let wp = Workplane::xz(2.5);
    let p = wp.unproject(1.25, -3.5);
    assert_eq!(p, [1.25, 2.5, -3.5]);
    assert_eq!(wp.project(p), [1.25, -3.5]);
    assert_eq!(wp.snap_2d(1.24, -3.62, 0.5), [1.0, 2.5, -3.5]);

    let d = measure_distance([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]);
    assert!((d.distance - 5.0).abs() < 1e-5);
    assert_eq!(d.delta, [3.0, 4.0, 0.0]);

    let mut doc = Document::new(Mesh::default());
    doc.set_workplane(Workplane::xy(1.5));
    let bytes = doc.save_project();
    let reopened = Document::open_project(&bytes).unwrap();
    assert_eq!(reopened.workplane().origin, [0.0, 0.0, 1.5]);
}

#[test]
fn align_and_distribute_instances_are_atomic_and_undoable() {
    let mut doc = Document::new(Mesh::default());
    let mut box_doc = Document::new(Mesh::default());
    box_doc
        .apply(
            Op::AddBox {
                region: brush_engine::BoxRegion {
                    center: [0.0, 0.5, 0.0],
                    size: [1.0, 1.0, 1.0],
                    yaw_deg: 0.0,
                },
                color: [204, 204, 204],
            },
            Actor::default(),
        )
        .unwrap();
    doc.set_object("cube", box_doc.mesh().clone());
    for (id, x, z) in [("a", 0.0, 1.0), ("b", 1.0, 5.0), ("c", 6.0, 3.0)] {
        doc.apply(
            Op::PlaceObject {
                id: id.into(),
                object: "cube".into(),
                position: [x, 0.0, z],
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                scale: 1.0,
            },
            Actor::default(),
        )
        .unwrap();
    }

    let ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    assert_eq!(
        doc.align_selection(&ids, AlignAxis::Z, AlignMode::Min, Actor::default())
            .unwrap(),
        3
    );
    for id in &ids {
        assert!((doc.scene().get(id).unwrap().position[2] - 1.0).abs() < 1e-6);
    }

    assert_eq!(
        doc.distribute_selection(&ids, AlignAxis::X, Actor::default())
            .unwrap(),
        3
    );
    assert!((doc.scene().get("a").unwrap().position[0] - 0.0).abs() < 1e-6);
    assert!((doc.scene().get("b").unwrap().position[0] - 3.0).abs() < 1e-6);
    assert!((doc.scene().get("c").unwrap().position[0] - 6.0).abs() < 1e-6);

    // One undo reverts the whole distribute transaction.
    assert!(doc.undo());
    assert!((doc.scene().get("b").unwrap().position[0] - 1.0).abs() < 1e-6);
}

#[test]
fn dimensioned_feature_with_pattern_edits_reopens_and_combines_with_direct_edits() {
    let mut doc = Document::new(Mesh::default());
    let pillar = doc
        .add_feature(
            FeatureSpec {
                id: Some("pillar".into()),
                name: "Pillar".into(),
                kind: FeatureKind::Box {
                    center: [0.0, 0.0, 0.0],
                    width: 0.5,
                    height: 2.0,
                    depth: 0.5,
                    yaw_deg: 0.0,
                    color: [200, 180, 150],
                },
            },
            Actor::default(),
        )
        .unwrap();
    assert_eq!(pillar.status, FeatureStatus::Valid);
    assert_eq!(pillar.instance_ids.len(), 1);

    // Add a LinearPattern that depends explicitly on `pillar`.
    let row = doc
        .add_feature(
            FeatureSpec {
                id: Some("colonnade".into()),
                name: "Colonnade".into(),
                kind: FeatureKind::LinearPattern {
                    source_feature: "pillar".into(),
                    count: 3,
                    step: [2.0, 0.0, 0.0],
                },
            },
            Actor::default(),
        )
        .unwrap();
    assert_eq!(row.instance_ids.len(), 3);
    assert_eq!(doc.scene().instances.len(), 4);

    // Edit pillar height from 2.0 -> 3.5: both pillar and colonnade update cleanly.
    let report = doc
        .update_feature(
            "pillar",
            FeatureKind::Box {
                center: [0.0, 0.0, 0.0],
                width: 0.5,
                height: 3.5,
                depth: 0.5,
                yaw_deg: 0.0,
                color: [200, 180, 150],
            },
            Actor::default(),
        )
        .unwrap();
    assert!(report.conflicts.is_empty());
    let (min_b, max_b) = doc.object(&pillar.object_key).unwrap().bounds().unwrap();
    assert!((max_b[1] - min_b[1] - 3.5).abs() < 1e-4);

    // Save to `.brush`, reopen, and verify full feature metadata + evaluated geometry survive.
    let bytes = doc.save_project();
    let mut reopened = Document::open_project(&bytes).unwrap();
    assert_eq!(reopened.list_features().len(), 2);
    assert_eq!(reopened.scene().instances.len(), 4);

    // Direct edit: delete the pillar instance manually -> validate_features reports Invalidation cleanly!
    reopened
        .apply(
            Op::RemoveObject {
                id: "pillar:inst:0".into(),
            },
            Actor::default(),
        )
        .unwrap();
    let validated = reopened.validate_features();
    assert!(matches!(
        validated[0].status,
        FeatureStatus::Invalidated { .. }
    ));
    assert!(matches!(
        validated[1].status,
        FeatureStatus::Invalidated { .. }
    ));

    // Repair restores pillar and its dependent colonnade!
    let repaired = reopened.repair_feature("pillar", Actor::default()).unwrap();
    assert_eq!(repaired.status, FeatureStatus::Valid);
    assert!(reopened.scene().get("pillar:inst:0").is_some());

    // Bake converts pillar into ordinary mesh geometry ready for sculpting!
    let baked = reopened.bake_feature("pillar", Actor::default()).unwrap();
    assert_eq!(baked.status, FeatureStatus::Baked);
    assert!(reopened.mesh().face_count() > 0);
}

#[test]
fn persistent_constraints_solve_and_report_conflicts() {
    let mut doc = Document::new(Mesh::default());
    doc.add_feature(
        FeatureSpec {
            id: Some("a".into()),
            name: "Block A".into(),
            kind: FeatureKind::Box {
                center: [0.0, 0.0, 0.0],
                width: 2.0,
                height: 1.0,
                depth: 1.0,
                yaw_deg: 0.0,
                color: [180, 180, 180],
            },
        },
        Actor::default(),
    )
    .unwrap();
    doc.add_feature(
        FeatureSpec {
            id: Some("b".into()),
            name: "Block B".into(),
            kind: FeatureKind::Box {
                center: [5.0, 0.0, 0.0],
                width: 1.0,
                height: 1.0,
                depth: 1.0,
                yaw_deg: 0.0,
                color: [180, 180, 180],
            },
        },
        Actor::default(),
    )
    .unwrap();

    // Constrain b.width == a.width and b.origin == a.origin + [3, 0, 0].
    let r1 = doc
        .add_constraint(
            ConstraintRecord {
                id: "c:eq-width".into(),
                kind: ConstraintKind::EqualDimension {
                    feature_a: "a".into(),
                    param_a: "width".into(),
                    feature_b: "b".into(),
                    param_b: "width".into(),
                },
                enabled: true,
            },
            Actor::default(),
        )
        .unwrap();
    assert!(r1.conflicts.is_empty());
    assert!(r1.updated_features.contains(&"b".to_string()));

    let r2 = doc
        .add_constraint(
            ConstraintRecord {
                id: "c:offset".into(),
                kind: ConstraintKind::CoincidentOffset {
                    source_feature: "a".into(),
                    target_feature: "b".into(),
                    offset: [3.0, 0.0, 0.0],
                },
                enabled: true,
            },
            Actor::default(),
        )
        .unwrap();
    assert!(r2.conflicts.is_empty());
    assert!((doc.scene().get("b:inst:0").unwrap().position[0] - 3.0).abs() < 1e-4);

    // Lock b.width to 4.0 -> conflicts with c:eq-width (which wants b.width == a.width == 2.0)!
    let r3 = doc
        .add_constraint(
            ConstraintRecord {
                id: "c:lock-b".into(),
                kind: ConstraintKind::FixedDimension {
                    feature: "b".into(),
                    param: "width".into(),
                    value: 4.0,
                },
                enabled: true,
            },
            Actor::default(),
        )
        .unwrap();
    assert_eq!(r3.conflicts.len(), 1);
    assert_eq!(r3.conflicts[0].constraint_id, "c:eq-width");
    assert!(r3.conflicts[0].reason.contains("conflicts with 'c:lock-b'"));
}
