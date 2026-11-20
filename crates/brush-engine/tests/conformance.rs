//! Runs `conformance/cases`: every binding of the engine must turn each fixture into exactly the expected mesh.
//! `BRUSH_UPDATE_CONFORMANCE=1 cargo test --test conformance` rewrites the expectations (review the diff!).

use brush_engine::{io, Actor, Document, Op};
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

#[test]
fn every_case_produces_the_expected_mesh() {
    let update = std::env::var_os("BRUSH_UPDATE_CONFORMANCE").is_some();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(root().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 10, "conformance cases are missing");
    let mut failures = Vec::new();
    for path in &cases {
        let text = std::fs::read_to_string(path).unwrap();
        let mut case: Value = serde_json::from_str(&text).unwrap();
        let fixture = std::fs::read(
            root()
                .join("fixtures")
                .join(case["fixture"].as_str().unwrap()),
        )
        .unwrap();
        let (mesh, _) = io::read_ply(&fixture).unwrap();
        let ops: Vec<Op> = serde_json::from_value(case["ops"].clone()).unwrap();
        let mut doc = Document::new(mesh);
        doc.apply_all(ops, Actor::default()).unwrap();
        let m = doc.mesh();
        let got = serde_json::json!({"vertices": m.vertex_count(), "faces": m.face_count(), "hash": m.content_hash()});
        if update {
            case["expect"] = got;
            std::fs::write(path, serde_json::to_string_pretty(&case).unwrap() + "\n").unwrap();
        } else if case["expect"] != got {
            failures.push(format!(
                "{}: expected {}, got {got}",
                path.file_name().unwrap().to_string_lossy(),
                case["expect"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
