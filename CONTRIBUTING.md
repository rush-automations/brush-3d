# Contributing

## Adding or changing an op

1. Put the geometry in `brush-mesh` (`kernel.rs` or `detail.rs`) as a method on `Mesh` with f32 arguments.
2. Add a variant to `Op` in `op.rs`:
   - a doc comment, which becomes the schema description, including what `count` means;
   - camelCase fields with defaults;
   - validation in `Op::validate`.
3. Keep it deterministic:
   - transcendentals come from `brush_mesh::math` (`libm`);
   - nothing in the output may depend on hash-map iteration order;
   - new vertices go through `push_vertex` with the right `Src`, so attributes follow.
4. Add behaviour tests to `crates/brush-mesh/tests/ops.rs` and a conformance case in `conformance/`.
5. Regenerate the schema with `BRUSH_UPDATE_SPEC=1 cargo test --test spec`, then the TypeScript types with `cd bindings/wasm && bun run types`.
6. Bump `OPS_VERSION`: minor for a new op or optional field, major for changed behaviour. Note it in `CHANGELOG.md`.

## Rules

- Licence: dependencies must be MIT, Apache-2.0, BSD, ISC or Zlib (`cargo deny check licenses`). No GPL code, including ports of GPL code.
- The engine is standalone. It must not reference any product, server or repository outside this folder.
- `cargo fmt`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` must pass.
