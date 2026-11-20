# Changelog

The engine follows semver. The op set has its own version (`OPS_VERSION`, `x-ops-version` in the schema):
- adding an op or an optional field is a minor change;
- changing what an existing op produces is a major change, and must come with updated conformance expectations.

Before 1.0, a change in behaviour bumps the minor version.

## 0.3.0 (ops 0.3.0): unreleased

### New

- `addRamp`: a wedge filling a box, climbing toward the box's +z.
- Scenes of linked objects:
  - `placeObject`, `moveObject`, `removeObject` and `bakeObject`.
  - `Document::set_object`, `flattened`, `pick` (sees placed objects), `instance_transform`, `load_scene` and `replay_scene`.
  - A document's hash includes its scene when anything is placed; documents without placed objects hash exactly as before.
- `Document::place`: the build tool's rules (grid snapping, facing the camera, standing on tops and against sides) as one engine call.
- Undo steps that leave the mesh alone (moving placed objects) no longer hold a copy of the mesh.
- `bindings/swift`: a C ABI and Swift wrapper for iOS. Mesh buffers are borrowed without copies, live strokes take plain arrays with Apple Pencil pressure, and undo history can be capped by memory.
- `bindings/wasm`: the scene, object and placement API; `export(format, flattened)`.

## 0.2.0 (ops 0.2.0): unreleased

### Changed behaviour

These change conformance hashes: `fill`, `smooth` and `kitchen-sink` were re-baselined.

- `smooth` is now Taubin λ|μ by default (`method: "laplacian"` gives the old behaviour), with distinct-neighbour weights. Boundary vertices really are pinned now (`keepBoundary`, default true); the old Zig core claimed this but didn't do it.
- `fillHoles`:
  - triangulates by ear clipping in the loop's best-fit plane instead of a centroid fan, so concave holes fill correctly and no vertices are added;
  - traces loops edge by edge, so holes that touch themselves at a vertex are filled;
  - drops slivers attached only at their corners;
  - fixes an off-by-one that accepted loops of `maxEdges + 1` edges, which could cap an open mesh's outer rim.

### New

- `simplify`: quadric error decimation that keeps the mesh manifold, unflipped and its outline in place.
- `sculpt`: draw, inflate, smooth, flatten, pinch and paint brushes. The `view` field lets brushes work on scans with unreliable winding.
- `Document`:
  - live strokes (`extendStroke`, `endStroke`), each recorded as one journal entry and one undo step;
  - `raycast` (BVH with a watertight triangle test, refitted after sculpting);
  - `vertex_normals`.
- `bindings/wasm`: wasm-bindgen package with TypeScript op types generated from the schema.
- `apps/demo`: a browser editor built on the package.
- `Document.openReadOnly` in the wasm package: the same engine for viewing, which refuses edits and never adds to its journal.

## 0.1.0

- Port of Brush's Zig mesh core, verified against it: 14 of 16 scenarios were bit-identical, and the other 2 gave the same mesh with vertices in a different order.
- PLY, OBJ and GLB I/O; `Document` with journal, undo and redo; the `brush` CLI; op and journal schemas; 16 conformance cases.
