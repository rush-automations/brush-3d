//! Replays the example workflows in `examples/`: each journal must rebuild its result with the recorded final
//! hash, so the workflows designers read in the README are the same ones CI proves every day.

use brush_engine::{io, Document, Journal};
use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn every_workflow_journal_replays_to_its_manifest_hash() {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(examples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && p.join("journal.jsonl").exists())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 4, "example workflows are missing");
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let fixture = std::fs::read(examples().join("fixtures/empty.ply")).unwrap();
        let (mesh, _) = io::read_ply(&fixture).unwrap();
        let journal_text = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
        let journal = Journal::from_jsonl(&journal_text)
            .unwrap_or_else(|e| panic!("{name}: journal does not parse: {e}"));
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let doc = Document::replay(mesh, &journal)
            .unwrap_or_else(|e| panic!("{name}: replay failed at a step hash: {e}"));
        assert_eq!(
            doc.hash(),
            manifest["hash"].as_str().unwrap(),
            "{name}: replayed hash drifted from the manifest"
        );
    }
}
