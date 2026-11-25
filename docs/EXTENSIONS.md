# Extending Brush 3D

`brush-3d` and `@brush/web-editor` expose four compiled extension interfaces plus versioned capability discovery so external applications and developers can add tools, interaction policies, viewport overlays, and asset catalogs without modifying the core engine.

## Capability Discovery

Query `capabilities()` (available in Rust as `brush_engine::capabilities()` and in JS/WASM as `capabilities()` from `brush-wasm`) before registering extensions:

```ts
import { capabilities } from 'brush-wasm';

const caps = capabilities();
// {
//   engineVersion: "0.1.0",
//   opsVersion: "0.5.0",
//   projectFormatVersion: 1,
//   formats: { read: ["brush", "ply", "obj"], write: ["brush", "ply", "obj", "glb"] },
//   features: {
//     transactions: true, previews: true, revisionGuards: true,
//     workplanes: true, editableFeatures: true, persistentConstraints: true,
//     splats: true, linkedInstances: true
//   },
//   featureKinds: ["box", "cylinder", "extrusion", "linearPattern"],
//   constraintKinds: ["equalDimension", "coincidentOffset", "fixedDimension"]
// }
```

## Extension Interfaces (`@brush/web-editor`)

Register compiled extensions with `ExtensionRegistry`:

1. **`ToolExtension`** — Pointer-driven or programmatic editing tools. Tools express editing intent and produce validated changes through `ctx.commit(ops, label)` (atomic undoable transactions) or `ctx.document.beginPreview()` / `updatePreview()` / `commitPreview()` / `cancelPreview()`. Declare `requiredCapabilities` so `ExtensionRegistry` validates engine support upfront.
2. **`PolicyExtension`** — Composable transforms over `InteractionPolicy` (e.g. custom grid snapping, discrete rotations, palette filters) using `composePolicy(base, patch)`.
3. **`RendererExtension`** — Custom visual overlays/guides mounted onto `Viewport` via `mount(viewport)`, `unmount(viewport)`, and `onSync(document, viewport)`.
4. **`AssetProviderExtension`** — Pluggable 3D asset catalogs implementing `search(query, signal?)` and `loadBytes(key, signal?)` returning PLY, OBJ, GLB, or `.brush` bytes.

## Minimal Example

```ts
import { ExtensionRegistry, type ToolExtension } from '@brush/web-editor';
import { capabilities, type Document, type Op } from 'brush-wasm';

const registry = new ExtensionRegistry<Document>(capabilities());

const staircaseTool: ToolExtension<Document> = {
  id: 'ext:staircase',
  label: 'Staircase',
  requiredCapabilities: ['transactions'],
  execute(ctx, params) {
    const steps = Number(params?.steps ?? 4);
    const ops: Op[] = Array.from({ length: steps }, (_, i) => ({
      op: 'addBox',
      box: { center: [0, 0.2 * (i + 0.5), 0.3 * i], size: [1.2, 0.2 * (i + 1), 0.3], yawDeg: 0 },
      color: [180, 140, 100],
    }));
    ctx.commit(ops, 'ext:staircase');
  },
};

registry.registerTool(staircaseTool);
```

See [packages/web-editor/test/extension-example.test.ts](../packages/web-editor/test/extension-example.test.ts) for a complete runnable example covering `ToolExtension`, `PolicyExtension`, and `AssetProviderExtension` with undo/redo and `.brush` persistence.
