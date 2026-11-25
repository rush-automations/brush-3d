# `@brush/web-editor`

Reusable browser viewport and editor interaction kit for applications built on `brush-3d`.

## What's included

- **`Viewport` & `SplatCloud`** (`src/viewport.ts`) — Three.js WebGL viewport with orbit controls, smooth camera presets and local flight (`panLocal`), instanced object rendering, selection outlines, 3-axis rotate-ring gizmos, placement/carve ghosts, floor grid, floorplan preview, LiDAR point-cloud overlays, posed photo camera frustums, flat/wireframe shading, and screenshot capture.
- **`syncDocumentToViewport`** (`src/sync.ts`) — Syncs a `brush-wasm` `Document` (surface mesh, splats, embedded object meshes, and scene instances) into a `Viewport` without hard-coupling to a specific WASM import path.
- **`CameraFlightController`** (`src/camera-flight.ts`) — Smooth WASD / Arrow / Space-Shift / Q-E camera flight controller.
- **`SelectController`** (`src/select-controller.ts`) — Framework-agnostic pointer controller for object selection, additive multi-select, box-select marquee, horizontal drag translation, and 3-axis rotate-ring gizmo drags backed by `Document` preview sessions (`beginPreview` / `updatePreview` / `commitPreview` / `cancelPreview`).
- **`WorkspaceSession` & Composable Policies** (`src/policies.ts`) — Composable interaction policies (`SIMPLE_BUILDER_PROFILE`, `FREEFORM_MESH_PROFILE`, `composePolicy`) so distinct mental models (grid-snapped block building vs freeform sculpting/mesh editing) operate on the exact same `Document` and `.brush` project without conversion or data loss.

