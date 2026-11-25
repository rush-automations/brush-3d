// Phase F proof: an external developer adds a custom tool ("Staircase Generator"), a custom policy
// ("Fine Metric Grid"), and a custom asset provider ("Procedural Furniture") using only the public
// @brush/web-editor + brush-wasm interfaces — zero core edits.
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Document, capabilities, initSync, type Op } from '../../../bindings/wasm/pkg/brush_wasm.js';
import {
  ExtensionRegistry,
  type AssetProviderExtension,
  type PolicyExtension,
  type ToolContext,
  type ToolExtension,
} from '../src/extensions';
import { SIMPLE_BUILDER_PROFILE, composePolicy } from '../src/policies';

initSync({
  module: readFileSync(join(import.meta.dir, '../../../bindings/wasm/pkg/brush_wasm_bg.wasm')),
});

describe('Phase F: open extension points and capability discovery', () => {
  test('capabilities() reports versioned engine, ops, formats, and precision features', () => {
    const caps = capabilities();
    expect(caps.opsVersion).toBe('0.5.0');
    expect(caps.projectFormatVersion).toBe(1);
    expect(caps.formats.read).toContain('brush');
    expect(caps.formats.write).toContain('glb');
    expect(caps.features.transactions).toBe(true);
    expect(caps.features.previews).toBe(true);
    expect(caps.features.editableFeatures).toBe(true);
    expect(caps.features.persistentConstraints).toBe(true);
    expect(caps.featureKinds).toContain('linearPattern');
    expect(caps.constraintKinds).toContain('equalDimension');
  });

  test('external developer adds a ToolExtension, PolicyExtension, and AssetProviderExtension without editing core', async () => {
    const caps = capabilities();
    const registry = new ExtensionRegistry<Document, null>(caps);

    // 1. Custom ToolExtension: Staircase Generator producing one atomic undoable transaction.
    const staircaseTool: ToolExtension<Document, null> = {
      id: 'ext:staircase',
      label: 'Staircase',
      description: 'Generates a stepped staircase as one atomic transaction.',
      requiredCapabilities: ['transactions'],
      execute(ctx, params) {
        const steps = Number(params?.steps ?? 4);
        const width = Number(params?.width ?? 1.2);
        const rise = Number(params?.rise ?? 0.2);
        const run = Number(params?.run ?? 0.3);
        const ops: Op[] = Array.from({ length: steps }, (_, i) => ({
          op: 'addBox',
          box: {
            center: [0, rise * (i + 0.5), run * i],
            size: [width, rise * (i + 1), run],
            yawDeg: 0,
          },
          color: [180, 140, 100],
        }));
        ctx.commit(ops, 'ext:staircase');
      },
    };

    // 2. Custom PolicyExtension: Fine 0.1m grid snapping.
    const fineGridPolicy: PolicyExtension = {
      id: 'ext:fine-grid',
      label: 'Fine 0.1m Grid',
      apply: (base) =>
        composePolicy(base, {
          placement: { snapToGrid: true, cell: 0.1 },
          viewport: { showGrid: true, gridCell: 0.1 },
        }),
    };

    // 3. Custom AssetProviderExtension: Procedural furniture catalog.
    const furnitureProvider: AssetProviderExtension = {
      id: 'ext:procedural-furniture',
      label: 'Procedural Furniture',
      async search(query) {
        const catalog = [
          { key: 'furn:table', name: 'Dining Table', format: 'ply' as const, tags: ['table'] },
        ];
        return catalog.filter((item) => item.name.toLowerCase().includes(query.toLowerCase()));
      },
      async loadBytes(key) {
        if (key !== 'furn:table') throw new Error(`Unknown asset ${key}`);
        const d = new Document();
        try {
          d.apply({ op: 'addBox', box: { center: [0, 0.75, 0], size: [1.6, 0.08, 0.9] }, color: [160, 110, 70] }, undefined);
          return d.export('ply');
        } finally {
          d.free();
        }
      },
    };

    registry.registerTool(staircaseTool).registerPolicy(fineGridPolicy).registerAssetProvider(furnitureProvider);

    // Apply policy extension cleanly.
    const policy = registry.applyPolicies(SIMPLE_BUILDER_PROFILE.policy);
    expect(policy.placement.cell).toBe(0.1);
    expect(policy.viewport.gridCell).toBe(0.1);

    // Execute custom tool through the shared transaction system on a real WASM Document.
    const doc = new Document();
    let syncCount = 0;
    const ctx: ToolContext<Document, null> = {
      document: doc,
      viewport: null,
      policy,
      actor: { id: 'ext-dev', kind: 'app' },
      commit(ops, label) {
        doc.applyTransaction(ops as Op[], label, ctx.actor);
        ctx.requestSync();
      },
      requestSync() {
        syncCount += 1;
      },
    };

    registry.getTool('ext:staircase')!.execute!(ctx, { steps: 5 });
    expect(syncCount).toBe(1);
    expect(doc.state().faces).toBe(5 * 12);
    expect(doc.state().lastTransactionLabel).toBe('ext:staircase');

    // Load an asset from the custom provider and place it in the same Document.
    const results = await registry.listAssetProviders()[0].search('table');
    expect(results).toHaveLength(1);
    const tableBytes = await registry.listAssetProviders()[0].loadBytes(results[0].key);
    doc.setObject(results[0].key, tableBytes);
    doc.apply(
      { op: 'placeObject', id: 'table-1', object: results[0].key, position: [2, 0, 0], yawDeg: 0, scale: 1 },
      ctx.actor,
    );

    // Save to `.brush` and reopen: both custom tool geometry and custom provider object survive!
    const projectBytes = doc.saveProject();
    const reopened = Document.open(projectBytes);
    expect(reopened.state().faces).toBe(60);
    expect(reopened.loadedObjects()).toContain('furn:table');

    // One undo reverts the placed table; second undo reverts the entire 5-step staircase atomically!
    expect(doc.undo()).toBe(true);
    expect(doc.scene().instances).toHaveLength(0);
    expect(doc.undo()).toBe(true);
    expect(doc.state().faces).toBe(0);

    doc.free();
    reopened.free();
  });

  test('ExtensionRegistry rejects tools whose required capabilities are unsupported', () => {
    const restrictedCaps = {
      ...capabilities(),
      features: { ...capabilities().features, persistentConstraints: false },
    };
    const registry = new ExtensionRegistry(restrictedCaps);
    expect(() =>
      registry.registerTool({
        id: 'ext:needs-constraints',
        label: 'Constraint Tool',
        requiredCapabilities: ['persistentConstraints'],
      }),
    ).toThrow(/unsupported engine capabilities: persistentConstraints/);
  });
});
