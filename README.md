# Brush 3D

[![engine](https://github.com/rush-automations/brush-3d/actions/workflows/ci.yml/badge.svg)](https://github.com/rush-automations/brush-3d/actions/workflows/ci.yml)
[![demo](https://github.com/rush-automations/brush-3d/actions/workflows/pages.yml/badge.svg)](https://github.com/rush-automations/brush-3d/actions/workflows/pages.yml)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Deterministic, cross-platform editing of PLY meshes through **typed operations**.

A mesh is edited by sending it ops: plain JSON such as `{"op": "carveBox", "box": {...}}` or a sculpt stroke. Every op means exactly the same thing on every platform, so:

- a browser, a phone, Blender, a server and an AI model can all edit the same mesh and get **bit-identical results**;
- every edit is logged in a **journal** that can be replayed, audited, undone or learned from;
- the op set is published as a **JSON Schema** (`spec/ops.schema.json`), so any program, including an LLM with tool calling, can produce valid edits.

Licensed MIT OR Apache-2.0. Dependencies are permissive only, and `cargo deny` checks this.

## Layout

| Path | What |
| --- | --- |
| `crates/brush-mesh` | Geometry kernel: the mesh (vertex colours, named per-vertex attributes), the splat set, and the 29 ops. No I/O, no GPU. |
| `crates/brush-io` | PLY (ascii and binary, any vertex properties kept with their types; splat paint round-trips byte-identically), OBJ in and out, GLB out. |
| `crates/brush-render` | The shared camera model and a software rasteriser for headless views, including the splat compositor. |
| `crates/brush-engine` | `Document`: validation, all-or-nothing batches, live sculpt strokes, undo, redo, revert, picking, and the journal. |
| `crates/brush-cli` | The `brush` command: `info`, `convert`, `apply`, `replay`, `render`, `schema`. |
| `bindings/wasm` | The engine for browsers, Node and Bun (wasm-bindgen; npm package built by wasm-pack). |
| `bindings/swift` | The engine for iOS: a C ABI (`include/brush_engine.h`), a Swift wrapper (`Sources/BrushEngine.swift`) and `build-xcframework.sh`, which builds `BrushEngine.xcframework` into the iOS app. |
| `apps/demo` | A small browser editor (Vite + TypeScript + three.js) that shows the engine off, splat spray included. |
| `apps/studio` | Brush Studio: the desktop and web editor (egui). Primitives, splat painting and baking, undo/redo, journal, measure panel, PLY open/save. |
| `examples/` | Replayable example workflows — product design, architecture, civil engineering and game art — each with ops, a journal, a preview and a CI-checked hash. |
| `spec/` | The op and journal schemas and the format conventions. |
| `conformance/` | Fixtures, op lists and expected hashes that every binding must reproduce. |

Python (Blender, servers, agents) and Kotlin (Android) bindings come next.

## Algorithms

These are the standard techniques, chosen for correctness on scanned meshes rather than for speed:

- **`simplify`:** quadric error edge collapse (Garland and Heckbert). It refuses collapses that would make the surface non-manifold (the link condition) or flip a face, and adds boundary constraint planes so outlines keep their shape.
- **`smooth`:** Taubin λ|μ smoothing, which removes noise without shrinking. Open boundaries stay pinned. Plain Laplacian is available as an option.
- **`fillHoles`:**
  - traces loops edge by edge, splitting holes that touch themselves at a vertex;
  - drops slivers attached only at their corners;
  - triangulates each loop by ear clipping in its best-fit plane, with no new vertices.
- **`sculpt`:** draw, inflate, smooth, flatten, pinch and paint brushes with a smooth falloff. Dabs apply in order, so a live stroke streamed in pieces equals the whole stroke. It's recorded as one op and one undo step.
- **Splat painting:** surface colour as an oriented point set, not vertex colours, so paint doesn't depend on mesh resolution and survives `simplify` and `subdivide`. `splatPaint` stamps a jittered grid in the dab's tangent plane (deterministic from the seed), snaps it to the surface with the BVH, accumulates wet-on-wet opacity instead of layering geometry, and lets gravity drip paint along the surface. Splats render as screen-space Gaussians (EWA-style) on every target; `splatBake` splats them down to vertex colours when a pipeline wants a plain mesh.
- **`transform`:** translate, rotate and scale a region about its centre; splats and their normals transform with the mesh.
- **Scenes and building:**
  - A document also holds placed objects: linked instances that point at other models (`"scan:<id>"`).
  - The host supplies each object's mesh. Refreshing it updates every instance at once, so an edit to an object reaches every scene it is placed in.
  - `placeObject`, `moveObject`, `removeObject` and `bakeObject` are journaled and undoable. `flattened()` copies the objects in for exports.
  - `Document::place` turns "a piece, where the pointer is, which way the camera looks and a grid" into the op that builds it: walls, floors, ramps, cubes, cylinders, cones, spheres, or an object. The snapping, facing and stacking rules live in the engine, so every app builds the same thing in the same place.
- **Picking:** a BVH with a watertight ray–triangle test (Woop, Benthin and Wald). The tree is refitted after sculpting and rebuilt after topology changes.

## Quick start

### Command line

```bash
cargo build --release -p brush-cli
B=target/release/brush

$B info scan.ply
echo '[{"op":"fillHoles"},{"op":"simplify","targetRatio":0.25}]' \
  | $B apply scan.ply --ops - -o clean.ply --journal edits.jsonl --actor model:cleanup-1 --model
$B replay scan.ply edits.jsonl          # same mesh, every step's hash checked
$B render clean.ply -o view.png
$B convert clean.ply clean.glb          # for Blender, three.js and other viewers
```

### JavaScript

```ts
import init, { Document } from 'brush-wasm';

await init();
const doc = Document.open(new Uint8Array(await file.arrayBuffer()));
doc.apply({ op: 'carveBox', box: { center: [0, 1, 0], size: [1, 1, 1] } }, undefined);
const positions = doc.positions(); // Float32Array, plus colors(), indices(), normals()
```

### Demo

```bash
cd bindings/wasm && bun install && bun run build   # needs wasm-pack
cd ../../apps/demo && bun install && bun run dev
```

### Brush Studio

```bash
cd apps/studio && cargo run --release              # desktop (Linux, macOS, Windows)
cd apps/studio && trunk build --release            # the same editor for the web
```

### Example workflows

```bash
brush replay examples/fixtures/empty.ply examples/game-art-hero-rock/journal.jsonl -o rock.ply
```

Four full sessions — [product design](examples/product-design-desk-organiser/), [architecture](examples/architecture-pavilion/), [civil engineering](examples/civil-retaining-wall/) and [game art](examples/game-art-hero-rock/) — each with readable ops and a preview, replayable on any target.

## Guarantees and how they're checked

- **Determinism.**
  - Transcendental maths goes through `libm`, not the platform's maths library.
  - Nothing in the output depends on hash-map iteration order.
  - The checks: `conformance/` is run by the Rust tests on Linux, macOS and Windows, and by `bindings/wasm/test` against the wasm build.
- **Validation before mutation.** An op with bad values is refused and leaves the mesh untouched. A batch is all or nothing.
- **Attributes survive edits.** Float attributes are interpolated. Integer attributes (labels, ids) are copied from a source vertex, never averaged.
- **The contract can't drift.**
  - `spec/*.schema.json` is generated from the Rust types, and a test fails when it's stale.
  - The TypeScript op types are generated from that schema, and CI fails when they're stale.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
BRUSH_UPDATE_SPEC=1 cargo test --test spec                 # after changing an op
cd bindings/wasm && bun run types && bun run build && bun test
```

On Windows without Visual Studio Build Tools, use the GNU toolchain: `rustup override set stable-x86_64-pc-windows-gnu`.
