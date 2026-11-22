//! The C ABI, exercised the way Swift calls it.

use super::*;
use std::ffi::CString;

fn cube_ply(size: f32, rgb: [u8; 3]) -> Vec<u8> {
    let mut m = Mesh::default();
    m.add_box([0.0, 0.0, 0.0], size, size, size, 0.0, rgb);
    io::write(&m, Format::Ply)
}

unsafe fn take(s: *mut c_char) -> String {
    assert!(!s.is_null(), "expected a string");
    let out = CStr::from_ptr(s).to_str().unwrap().to_string();
    brush_string_free(s);
    out
}

unsafe fn open(bytes: &[u8], read_only: bool) -> *mut BrushDocument {
    let mut err = ptr::null_mut();
    let d = brush_document_open(bytes.as_ptr(), bytes.len(), read_only, 0, &mut err);
    assert!(err.is_null(), "{}", take(err));
    d
}

unsafe fn apply(d: *mut BrushDocument, op: &str) -> Result<serde_json::Value, String> {
    let op = CString::new(op).unwrap();
    let mut err = ptr::null_mut();
    let out = brush_document_apply(d, op.as_ptr(), ptr::null(), &mut err);
    if out.is_null() {
        Err(take(err))
    } else {
        Ok(serde_json::from_str(&take(out)).unwrap())
    }
}

unsafe fn state(d: *mut BrushDocument) -> serde_json::Value {
    serde_json::from_str(&take(brush_document_state(d))).unwrap()
}

#[test]
fn opens_edits_undoes_and_exports() {
    unsafe {
        let d = open(&cube_ply(1.0, [100, 100, 100]), false);
        assert_eq!(brush_document_vertex_count(d), 8);
        assert_eq!(brush_document_face_count(d), 12);
        let mut n = 0;
        let p = brush_document_positions(d, &mut n);
        assert!(!p.is_null());
        assert_eq!(n, 24);
        let normals = brush_document_normals(d, &mut n);
        assert!(!normals.is_null());
        assert_eq!(n, 24);

        let a = apply(
            d,
            r#"{"op":"addBox","box":{"center":[3,0.5,0],"size":[1,1,1]}}"#,
        )
        .unwrap();
        assert_eq!(a["changed"], true);
        assert_eq!(brush_document_face_count(d), 24);
        assert!(brush_document_dirty(d));
        assert!(brush_document_can_undo(d));
        assert!(brush_document_undo(d));
        assert_eq!(brush_document_face_count(d), 12);
        assert!(brush_document_redo(d));

        let err = apply(d, r#"{"op":"simplify","targetRatio":4}"#).unwrap_err();
        assert!(err.contains("targetRatio"), "{err}");
        let err = apply(d, "not json").unwrap_err();
        assert!(err.starts_with("bad op"), "{err}");

        let fmt = CString::new("obj").unwrap();
        let (mut len, mut e) = (0usize, ptr::null_mut());
        let bytes = brush_document_export(d, fmt.as_ptr(), false, &mut len, &mut e);
        assert!(!bytes.is_null() && len > 0);
        assert!(
            std::slice::from_raw_parts(bytes, len).starts_with(b"#")
                || std::slice::from_raw_parts(bytes, len).starts_with(b"v")
        );
        brush_bytes_free(bytes, len);

        let journal = take(brush_document_journal(d));
        assert_eq!(journal.lines().count(), 3, "{journal}");
        brush_document_free(d);
    }
}

#[test]
fn read_only_documents_refuse_edits() {
    unsafe {
        let d = open(&cube_ply(1.0, [1, 2, 3]), true);
        assert!(brush_document_read_only(d));
        let err = apply(d, r#"{"op":"fillHoles"}"#).unwrap_err();
        assert!(err.contains("read-only"));
        assert!(!brush_document_undo(d));
        let mut e = ptr::null_mut();
        assert_eq!(
            brush_document_extend_stroke(d, [0.0; 3].as_ptr(), ptr::null(), 1, &mut e),
            -1
        );
        take(e);
        brush_document_free(d);
    }
}

#[test]
fn strokes_take_plain_arrays_and_stay_one_step() {
    unsafe {
        let mut m = Mesh::default();
        m.add_box([0.0, 0.0, 0.0], 2.0, 0.2, 2.0, 0.0, [128; 3]);
        let d = open(&io::write(&m, Format::Ply), false);
        let sub = apply(d, r#"{"op":"subdivide","maxEdge":0.1}"#).unwrap();
        assert_eq!(sub["changed"], true);
        let before = state(d)["journalLength"].as_u64().unwrap();
        apply(d, r#"{"op":"sculpt","brush":"draw","dabs":[{"center":[0,0.2,0]}],"radius":0.4,"view":[0,-1,0]}"#).unwrap();
        let centers = [0.1, 0.2, 0.0, 0.2, 0.2, 0.0, 0.3, 0.2, 0.0];
        let pressures = [0.5, 0.75, 1.0];
        let mut e = ptr::null_mut();
        assert_eq!(
            brush_document_extend_stroke(d, centers.as_ptr(), pressures.as_ptr(), 3, &mut e),
            1
        );
        brush_document_end_stroke(d);
        assert_eq!(
            state(d)["journalLength"].as_u64().unwrap(),
            before + 1,
            "one stroke, one entry"
        );
        assert_eq!(
            brush_document_extend_stroke(d, centers.as_ptr(), ptr::null(), 1, &mut e),
            -1
        );
        assert!(take(e).contains("no stroke"));

        // The live stroke equals the whole stroke sent as one op.
        let whole = open(&io::write(&m, Format::Ply), false);
        apply(whole, r#"{"op":"subdivide","maxEdge":0.1}"#).unwrap();
        apply(
            whole,
            r#"{"op":"sculpt","brush":"draw","dabs":[{"center":[0,0.2,0]},{"center":[0.1,0.2,0],"pressure":0.5},{"center":[0.2,0.2,0],"pressure":0.75},{"center":[0.3,0.2,0]}],"radius":0.4,"view":[0,-1,0]}"#,
        )
        .unwrap();
        assert_eq!(state(d)["hash"], state(whole)["hash"]);
        brush_document_free(d);
        brush_document_free(whole);
    }
}

#[test]
fn raycasts_and_picks_instances() {
    unsafe {
        let mut floor = Mesh::default();
        floor.add_box([0.0, -0.1, 0.0], 10.0, 0.1, 10.0, 0.0, [90; 3]);
        let d = open(&io::write(&floor, Format::Ply), false);
        let chair = cube_ply(1.0, [200, 0, 0]);
        let key = CString::new("scan:chair").unwrap();
        let mut e = ptr::null_mut();
        assert!(brush_document_set_object(
            d,
            key.as_ptr(),
            chair.as_ptr(),
            chair.len(),
            &mut e
        ));

        // Build the placement through the engine, then apply it.
        let placement = CString::new(
            r#"{"piece":"object","object":"scan:chair","point":[2.2,0,0.1],"view":[0,-0.5,1],"cell":1}"#,
        )
        .unwrap();
        let op = take(brush_document_place(d, placement.as_ptr(), &mut e));
        assert!(
            op.contains(r#""op":"placeObject""#) && op.contains(r#""position":[2.5,0.0,0.5]"#),
            "{op}"
        );
        apply(d, &op).unwrap();
        assert_eq!(take(brush_document_missing_objects(d)), "[]");
        assert!(take(brush_document_scene(d)).contains(r#""id":"o1""#));

        let (mut hit, mut inst) = (BrushHit::default(), ptr::null_mut());
        assert!(brush_document_pick(
            d,
            [2.5, 5.0, 0.5].as_ptr(),
            [0.0, -1.0, 0.0].as_ptr(),
            &mut hit,
            &mut inst
        ));
        assert_eq!(take(inst), "o1");
        assert!((hit.point[1] - 1.0).abs() < 1e-4);
        assert!(brush_document_raycast(
            d,
            [2.5, 5.0, 0.5].as_ptr(),
            [0.0, -1.0, 0.0].as_ptr(),
            &mut hit
        ));
        assert!(
            hit.point[1].abs() < 1e-4,
            "raycast sees the document's own mesh only"
        );

        let mut m = [0f32; 16];
        let id = CString::new("o1").unwrap();
        assert!(brush_document_instance_transform(
            d,
            id.as_ptr(),
            m.as_mut_ptr()
        ));
        assert_eq!(
            [m[12], m[13], m[14]],
            [2.5, 0.0, 0.5],
            "the bottom centre stands on the floor"
        );
        let mut n = 0;
        assert!(!brush_document_object_normals(d, key.as_ptr(), &mut n).is_null());
        assert_eq!(n, 24);

        // Export: flattened carries the instance, the plain export is the document's own mesh.
        let fmt = CString::new("ply").unwrap();
        let mut len = 0;
        let own = brush_document_export(d, fmt.as_ptr(), false, &mut len, &mut e);
        let (own_mesh, _) = io::read(std::slice::from_raw_parts(own, len), Format::Ply).unwrap();
        brush_bytes_free(own, len);
        let flat = brush_document_export(d, fmt.as_ptr(), true, &mut len, &mut e);
        let (flat_mesh, _) = io::read(std::slice::from_raw_parts(flat, len), Format::Ply).unwrap();
        brush_bytes_free(flat, len);
        assert_eq!(own_mesh.face_count(), 12);
        assert_eq!(flat_mesh.face_count(), 24);
        brush_document_free(d);
    }
}

#[test]
fn stored_scenes_load_as_the_saved_state() {
    unsafe {
        let d = open(&cube_ply(1.0, [1, 1, 1]), false);
        let scene =
            CString::new(r#"{"instances":[{"id":"a","object":"scan:x","position":[1,0,0]}]}"#)
                .unwrap();
        let mut e = ptr::null_mut();
        assert!(brush_document_load_scene(d, scene.as_ptr(), &mut e));
        assert!(!brush_document_dirty(d));
        assert_eq!(take(brush_document_missing_objects(d)), r#"["scan:x"]"#);
        let bad = CString::new(r#"{"instances":[{"id":"a"}]}"#).unwrap();
        assert!(!brush_document_load_scene(d, bad.as_ptr(), &mut e));
        assert!(take(e).starts_with("bad scene"));
        brush_document_free(d);
    }
}

#[test]
fn undo_history_is_capped_by_memory() {
    let mut m = Mesh::default();
    m.add_box([0.0; 3], 1.0, 1.0, 1.0, 0.0, [1; 3]);
    assert_eq!(undo_depth_for(&m, 0), DEFAULT_UNDO_DEPTH);
    assert_eq!(undo_depth_for(&m, 1), 1);
    assert_eq!(undo_depth_for(&m, mesh_bytes(&m) * 5), 5);
    assert_eq!(undo_depth_for(&m, usize::MAX), DEFAULT_UNDO_DEPTH);
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    unsafe {
        let mut e = ptr::null_mut();
        let d = brush_document_open(b"nope".as_ptr(), 4, false, 0, &mut e);
        assert!(d.is_null());
        assert_eq!(take(e), "not a PLY or OBJ file");
        assert!(brush_document_apply(ptr::null_mut(), ptr::null(), ptr::null(), &mut e).is_null());
        take(e);
        brush_document_free(ptr::null_mut());
        brush_string_free(ptr::null_mut());
        let v = take(brush_version());
        assert!(v.contains(r#""ops":"0.4.0""#), "{v}");
        let scratch = brush_document_new();
        apply(
            scratch,
            r#"{"op":"addRamp","box":{"center":[0,0.5,0],"size":[1,1,1]}}"#,
        )
        .unwrap();
        assert_eq!(brush_document_face_count(scratch), 8);
        brush_document_free(scratch);
    }
}

#[test]
fn splats_and_measure() {
    unsafe {
        let d = open(&cube_ply(1.0, [100, 100, 100]), false);
        assert_eq!(brush_document_splat_count(d), 0);
        let mut n = 7;
        assert!(brush_document_splats(d, &mut n).is_null());
        assert_eq!(n, 0);
        assert!(brush_document_splat_colors(d, &mut n).is_null());
        assert_eq!(n, 0);

        let m: serde_json::Value = serde_json::from_str(&take(brush_document_measure(d))).unwrap();
        assert!((m["volume"].as_f64().unwrap() - 1.0).abs() < 1e-6, "{m}");
        assert!(
            (m["surfaceArea"].as_f64().unwrap() - 6.0).abs() < 1e-6,
            "{m}"
        );
        assert_eq!(m["closed"], true);
        assert_eq!(m["splats"], 0);
        let c = m["centroid"].as_array().unwrap();
        assert_eq!(c.len(), 3);
        assert!((c[1].as_f64().unwrap() - 0.5).abs() < 1e-6, "{m}");

        // Spray the cube's top face (y = 1) from above.
        apply(
            d,
            r#"{"op":"splatPaint","dabs":[{"center":[0,1,0]}],"radius":0.4,"size":0.1,"color":[200,30,40],"view":[0,-1,0],"seed":3}"#,
        )
        .unwrap();
        let count = brush_document_splat_count(d);
        assert!(count > 0);
        assert_eq!(state(d)["splats"], count);

        let geo = brush_document_splats(d, &mut n);
        assert_eq!(n, count * 8);
        let geo = std::slice::from_raw_parts(geo, n);
        let colors = brush_document_splat_colors(d, &mut n);
        assert_eq!(n, count * 4);
        let colors = std::slice::from_raw_parts(colors, n);
        let mesh = (*d).doc.mesh();
        for (i, s) in geo.chunks_exact(8).enumerate() {
            assert!(
                (s[1] - 1.0).abs() < 1e-3,
                "splat {i} sits on the top face: {s:?}"
            );
            assert!(s[3] > 0.0, "radius");
            assert!((s[5] - 1.0).abs() < 1e-3, "normal points up: {s:?}");
            assert_eq!(s[3], mesh.splats.radii[i]);
            assert_eq!(s[7], mesh.splats.drips[i]);
        }
        for (i, c) in colors.chunks_exact(4).enumerate() {
            assert_eq!(&c[..3], &mesh.splats.colors[i * 3..i * 3 + 3]);
            assert_eq!(c[3], mesh.splats.opacities[i]);
        }
        assert!(
            colors.chunks_exact(4).any(|c| c[3] > 0 && c[0] > c[1]),
            "some red paint shows"
        );
        let m: serde_json::Value = serde_json::from_str(&take(brush_document_measure(d))).unwrap();
        assert_eq!(m["splats"], count);

        // The cached buffers follow undo and redo.
        assert!(brush_document_undo(d));
        assert_eq!(brush_document_splat_count(d), 0);
        assert!(brush_document_splats(d, &mut n).is_null());
        assert_eq!(n, 0);
        assert!(brush_document_splat_colors(d, &mut n).is_null());
        assert_eq!(n, 0);
        assert!(brush_document_redo(d));
        brush_document_splats(d, &mut n);
        assert_eq!(n, count * 8);
        brush_document_splat_colors(d, &mut n);
        assert_eq!(n, count * 4);

        assert_eq!(brush_document_splat_count(ptr::null()), 0);
        assert!(brush_document_measure(ptr::null()).is_null());
        assert!(brush_document_splats(ptr::null_mut(), &mut n).is_null());
        assert_eq!(n, 0);
        brush_document_free(d);
    }
}
