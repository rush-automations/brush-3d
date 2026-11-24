use std::{fs, path::Path, process::Command};

const TRIANGLE: &str = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n-1 0 0\n1 0 0\n0 2 0\n3 0 1 2\n";

#[test]
fn scene_operations_and_picking_use_supplied_objects() {
    let root = std::env::temp_dir().join(format!("brush-cli-scene-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mesh = root.join("mesh.ply");
    let object = root.join("object.ply");
    let scene = root.join("scene.json");
    let objects = root.join("objects.json");
    let ops = root.join("ops.json");
    let output = root.join("output.ply");
    fs::write(&mesh, TRIANGLE).unwrap();
    fs::write(&object, TRIANGLE).unwrap();
    fs::write(
        &scene,
        r#"{"instances":[{"id":"chair","object":"scan:chair","position":[10,0,0]}]}"#,
    )
    .unwrap();
    fs::write(
        &objects,
        serde_json::to_vec(&serde_json::json!({"scan:chair": object})).unwrap(),
    )
    .unwrap();
    fs::write(
        &ops,
        r#"[{"op":"moveObject","id":"chair","position":[20,0,0]}]"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_brush"))
        .args([
            "apply",
            path(&mesh),
            "--ops",
            path(&ops),
            "--output",
            path(&output),
            "--scene",
            path(&scene),
            "--objects",
            path(&objects),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        body["scene"]["instances"][0]["position"],
        serde_json::json!([20.0, 0.0, 0.0])
    );
    fs::write(&scene, serde_json::to_vec(&body["scene"]).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_brush"))
        .args([
            "pick",
            path(&output),
            "--origin",
            "20,1,3",
            "--direction",
            "0,0,-1",
            "--scene",
            path(&scene),
            "--objects",
            path(&objects),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(body["hit"]["instance"], "chair");
    let image = root.join("view.png");
    let result = Command::new(env!("CARGO_BIN_EXE_brush"))
        .args([
            "render",
            path(&output),
            "--output",
            path(&image),
            "--eye=-5,3,10",
            "--target=0,0,0",
            "--scene",
            path(&scene),
            "--objects",
            path(&objects),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG"));
    let missing = Command::new(env!("CARGO_BIN_EXE_brush"))
        .args([
            "render",
            path(&output),
            "--output",
            path(&image),
            "--scene",
            path(&scene),
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing object meshes"));
    fs::remove_dir_all(root).unwrap();
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}
