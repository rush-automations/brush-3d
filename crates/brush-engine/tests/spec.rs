//! `spec/*.schema.json` are the published contract. They are generated from the code; this test fails when they are
//! stale. `BRUSH_UPDATE_SPEC=1 cargo test --test spec` regenerates them.

use std::path::PathBuf;

fn check(file: &str, schema: serde_json::Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec")
        .join(file);
    let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    if std::env::var_os("BRUSH_UPDATE_SPEC").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == text,
        "{file} is stale: run BRUSH_UPDATE_SPEC=1 cargo test --test spec"
    );
}

#[test]
fn ops_schema_is_current() {
    check("ops.schema.json", brush_engine::mesh::ops_schema());
}

#[test]
fn journal_schema_is_current() {
    check("journal.schema.json", brush_engine::journal_schema());
}

#[test]
fn every_op_in_the_schema_has_a_description() {
    let schema = brush_engine::mesh::ops_schema();
    let variants = schema["oneOf"].as_array().expect("ops are a oneOf");
    assert_eq!(variants.len(), 29, "op count changed: update this test");
    for v in variants {
        let name = &v["properties"]["op"];
        assert!(
            v["description"].as_str().is_some_and(|d| d.len() > 20),
            "{name} has no description"
        );
        assert_eq!(
            v["additionalProperties"], false,
            "{name} accepts unknown fields"
        );
    }
}
