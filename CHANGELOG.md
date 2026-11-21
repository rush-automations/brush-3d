# Changelog

The engine follows semver. The op set has its own version (`OPS_VERSION`, `x-ops-version` in the schema):
- adding an op or an optional field is a minor change;
- changing what an existing op produces is a major change, and must come with updated conformance expectations.

Before 1.0, a change in behaviour bumps the minor version.

## 0.4.0 (ops 0.4.0): unreleased

### New

- **Splat painting** — resolution-independent surface paint, stored as an oriented point set (position, normal,
  radius, colour, opacity, drip) instead of vertex colours:
  - `splatPaint`: physics-based stamping. A jittered grid of stamps in the dab's tangent plane snaps onto the
    surface through the BVH; pressure scales coverage, a smoothstep falloff softens edges, wet-on-wet strokes
    accumulate opacity instead of duplicating geometry, `drip` pulls paint along the gravity-projected tangent
    and `colorNoise` adds fbm mottling. Deterministic from `seed`: the same stroke sprayed in pieces equals the
    whole stroke, and the same stroke on a coarse or fine mesh lands identically.
  - `splatErase`, `splatTint` (region recolour) and `splatBake` (splat down to vertex colours, optionally
    clearing the set). `simplify` and `subdivide` leave splats untouched until you bake.
  - PLY round-trip: meshes with splats write an `element splat` block (13 properties) that readers without
    splat support skip; Brush reads it back byte-identically. Hashes of splat-free meshes are unchanged.
- `transform`: translate, rotate (yaw/pitch/roll) and scale a region (or the whole mesh) about its centre;
  splats and their normals transform with the mesh.
- `measure`: volume (signed tetrahedra, closed meshes only), surface area, area-weighted centroid and
  closedness, exposed on `brush info`, the wasm `Document.measure()` and the studio's measure panel.
- `apps/studio`: Brush Studio, the desktop and web editor (egui): orbit viewport on the deterministic software
  rasteriser, primitive creation, the splat brush with live spray and erase, baking, undo/redo, the op journal,
  the measure panel and PLY open/save. Ships native (Linux/macOS/Windows) and as a trunk web bundle.
- `examples/`: four replayable workflows — desk organiser (product design), pavilion (architecture), retaining
  wall (civil) and hero rock (game art) — each with readable ops, a recorded journal, a rendered preview and a
  manifest hash that CI replays.
- `bindings/wasm`: `splatCount`, `splats`, `splatColors` and `measure`; `state()` reports the splat count.
- `.github/workflows/release.yml`: tag builds for every target — CLI binaries (Linux/macOS/Windows), the wasm
  npm package, the web demo, studio desktop bundles, the studio web bundle and the iOS XCFramework — attached
  to the GitHub release. `.github/workflows/pages.yml` deploys the demo to GitHub Pages from main.

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
