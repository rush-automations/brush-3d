# Conformance

Each `cases/*.json` names a fixture in `fixtures/`, a list of ops, and what the mesh must be after them:

```json
{ "fixture": "paint.ply", "ops": [ ... ], "expect": { "vertices": 49, "faces": 72, "hash": "9417b6b5145d0e98" } }
```

Every implementation and binding of the engine must load the fixture, apply the ops in order and produce exactly that vertex count, face count and content hash (see `spec/formats.md`). The runners are:

- Rust: `crates/brush-engine/tests/conformance.rs`;
- wasm: `bindings/wasm/test/engine.test.ts`.

**History:**
- The first 16 cases were cross-checked against Brush's earlier Zig mesh core when it was ported.
- Ops 0.2.0 deliberately changed `smooth` and `fillHoles`, and the cases using them were re-baselined (see `CHANGELOG.md`).

To change an expectation on purpose, run `BRUSH_UPDATE_CONFORMANCE=1 cargo test --test conformance`. Explain the change in `CHANGELOG.md`, because it changes what an op means.
