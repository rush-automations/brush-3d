// Proves Phase D: two distinct mental models (Simple Builder vs Freeform Mesh) edit the exact same
// `.brush` project and switch policies live on the same Document without conversion or data loss.
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  Document,
  initSync,
  isProjectContainer,
  rotateGroupAbout,
  translateGroup,
} from '../../../bindings/wasm/pkg/brush_wasm.js';
import {
  FREEFORM_MESH_PROFILE,
  SIMPLE_BUILDER_PROFILE,
  WorkspaceSession,
  composePolicy,
} from '../src/policies';

initSync({
  module: readFileSync(join(import.meta.dir, '../../../bindings/wasm/pkg/brush_wasm_bg.wasm')),
});

describe('Phase D: two mental models on one shared document', () => {
  test('composePolicy combines grid snapping with sculpting without mutating base profiles', () => {
    const hybrid = composePolicy(FREEFORM_MESH_PROFILE.policy, {
      placement: { snapToGrid: true, cell: 0.25 },
      sculpt: { snapToGrid: true },
      viewport: { showGrid: true, gridCell: 0.25 },
    });

    expect(hybrid.sculpt.enabled).toBe(true);
    expect(hybrid.sculpt.snapToGrid).toBe(true);
    expect(hybrid.placement.snapToGrid).toBe(true);
    expect(hybrid.placement.cell).toBe(0.25);
    expect(hybrid.viewport.showGrid).toBe(true);

    // Base profile remains untouched.
    expect(FREEFORM_MESH_PROFILE.policy.placement.snapToGrid).toBe(false);
    expect(FREEFORM_MESH_PROFILE.policy.sculpt.snapToGrid).toBe(false);
    expect(SIMPLE_BUILDER_PROFILE.policy.sculpt.enabled).toBe(false);
  });

  test('switching profiles live in WorkspaceSession preserves document state and updates viewport policy', () => {
    const doc = new Document();
    doc.apply(
      { op: 'addBox', box: { center: [0, 0.5, 0], size: [1, 1, 1], yawDeg: 0 }, color: [200, 120, 40] },
      { id: 'builder', kind: 'person' },
    );

    const gridCalls: boolean[] = [];
    const gizmoCalls: boolean[] = [];
    const viewport = {
      setGrid: (opts: { visible: boolean }) => gridCalls.push(opts.visible),
      setRotateGizmo: (visible: boolean) => gizmoCalls.push(visible),
    };

    const session = new WorkspaceSession(doc, SIMPLE_BUILDER_PROFILE, viewport);
    const beforeState = doc.state();
    expect(session.getPolicy().viewport.showGrid).toBe(true);
    expect(session.getPolicy().sculpt.enabled).toBe(false);

    // Switch to Freeform Mesh on the exact same Document.
    session.setProfile(FREEFORM_MESH_PROFILE);
    expect(session.getProfile().id).toBe('freeform-mesh');
    expect(session.getPolicy().sculpt.enabled).toBe(true);
    expect(session.getPolicy().viewport.showGrid).toBe(false);
    expect(gridCalls).toEqual([true, false]);

    // Document geometry, hash, and revision are untouched by the workspace switch.
    const afterState = doc.state();
    expect(afterState.hash).toBe(beforeState.hash);
    expect(afterState.revision).toBe(beforeState.revision);
    expect(afterState.faces).toBe(beforeState.faces);

    // Composable override: turn grid back on while staying in Freeform Mesh.
    session.mergePolicy({ viewport: { showGrid: true }, placement: { snapToGrid: true } });
    expect(session.getPolicy().sculpt.enabled).toBe(true);
    expect(session.getPolicy().viewport.showGrid).toBe(true);
    expect(gridCalls).toEqual([true, false, true]);

    doc.free();
  });

  test('Simple Builder -> .brush -> Freeform Mesh -> .brush -> Simple Builder round-trips with zero data loss', () => {
    // 1. Simple Builder: construct blocks on the grid and place an embedded prop instance.
    const builderDoc = new Document();
    const builderSession = new WorkspaceSession(builderDoc, SIMPLE_BUILDER_PROFILE);
    const placeOp = builderDoc.place({
      piece: 'wall',
      point: [0, 0, 0],
      normal: [0, 1, 0],
      view: [0, 0, -1],
      cell: builderSession.getPolicy().placement.cell,
      snap: builderSession.getPolicy().placement.snapToGrid,
      ground: 0,
      turns: 1,
      span: [2, 1, 1],
      color: [217, 119, 6],
    });
    builderDoc.apply(placeOp, { id: 'builder', kind: 'person' });

    // Create a reusable prop object and place an instance on the grid.
    const propDoc = new Document();
    propDoc.apply({ op: 'addBox', box: { center: [0, 0.25, 0], size: [0.5, 0.5, 0.5] }, color: [37, 99, 235] }, undefined);
    builderDoc.setObject('block:crate', propDoc.export('ply'));
    propDoc.free();
    builderDoc.apply(
      { op: 'placeObject', id: 'crate-1', object: 'block:crate', position: [1, 0, 0], yawDeg: 90, scale: 1 },
      { id: 'builder', kind: 'person' },
    );

    const builderBytes = builderSession.saveProject();
    expect(isProjectContainer(builderBytes)).toBe(true);
    const builderHash = builderDoc.state().documentHash;
    const builderRev = builderDoc.state().revision;
    builderDoc.free();

    // 2. Open in Freeform Mesh workspace: verify exact fidelity, then sculpt + splat paint + free-tilt the instance.
    const freeformDoc = Document.open(builderBytes);
    const freeformSession = new WorkspaceSession(freeformDoc, FREEFORM_MESH_PROFILE);
    expect(freeformDoc.state().documentHash).toBe(builderHash);
    expect(freeformDoc.state().revision).toBe(builderRev);
    expect((freeformDoc.metadata() as { lastWorkspaceProfile?: string }).lastWorkspaceProfile).toBe('simple-builder');
    expect(freeformDoc.loadedObjects()).toContain('block:crate');
    expect(freeformDoc.scene().instances).toHaveLength(1);

    // Subdivide + sculpt + paint splats + unrestricted tilt & translate on the placed crate.
    freeformDoc.apply({ op: 'subdivide', maxEdge: 0.2 }, { id: 'sculptor', kind: 'person' });
    const bounds = freeformDoc.state().bounds!;
    const center: [number, number, number] = [
      (bounds.min[0] + bounds.max[0]) / 2,
      (bounds.min[1] + bounds.max[1]) / 2,
      (bounds.min[2] + bounds.max[2]) / 2,
    ];
    freeformDoc.apply(
      { op: 'sculpt', brush: 'inflate', dabs: [{ center }], radius: 0.4, strength: 0.4, view: [0, 0, -1] },
      { id: 'sculptor', kind: 'person' },
    );
    freeformDoc.apply(
      { op: 'splatPaint', dabs: [{ center }], radius: 0.35, size: 0.05, color: [185, 28, 28], opacity: 0.9, flow: 0.6, seed: 42, view: [0, 0, -1] },
      { id: 'sculptor', kind: 'person' },
    );
    const moved = translateGroup(
      rotateGroupAbout(freeformDoc.scene().instances, 'x', 22.5, [1, 0, 0]),
      [0.13, 0, -0.27],
    );
    freeformDoc.apply(
      {
        op: 'moveObject',
        id: moved[0].id,
        position: moved[0].position,
        yawDeg: moved[0].yawDeg ?? 0,
        pitchDeg: moved[0].pitchDeg ?? 0,
        rollDeg: moved[0].rollDeg ?? 0,
        scale: moved[0].scale ?? 1,
      },
      { id: 'sculptor', kind: 'person' },
    );

    const freeformBytes = freeformSession.saveProject();
    const freeformState = freeformDoc.state();
    expect(freeformState.splats).toBeGreaterThan(0);
    freeformDoc.free();

    // 3. Re-open back in Simple Builder: sculpted mesh, splats, tilted instance, metadata, and journal all survive!
    const reopenedDoc = Document.open(freeformBytes);
    const reopenedSession = new WorkspaceSession(reopenedDoc, SIMPLE_BUILDER_PROFILE);
    expect(reopenedSession.getProfile().id).toBe('simple-builder');
    expect(reopenedDoc.state().documentHash).toBe(freeformState.documentHash);
    expect(reopenedDoc.state().vertices).toBe(freeformState.vertices);
    expect(reopenedDoc.state().faces).toBe(freeformState.faces);
    expect(reopenedDoc.state().splats).toBe(freeformState.splats);
    expect(reopenedDoc.state().journalLength).toBe(freeformState.journalLength);
    expect((reopenedDoc.metadata() as { lastWorkspaceProfile?: string }).lastWorkspaceProfile).toBe('freeform-mesh');

    const inst = reopenedDoc.scene().instances[0];
    expect(inst.id).toBe('crate-1');
    expect(inst.position[0]).toBeCloseTo(1.13, 4);
    expect(inst.pitchDeg ?? 0).toBeCloseTo(moved[0].pitchDeg ?? 0, 4);
    expect(inst.rollDeg ?? 0).toBeCloseTo(moved[0].rollDeg ?? 0, 4);
    expect(Math.hypot(inst.pitchDeg ?? 0, inst.rollDeg ?? 0)).toBeCloseTo(22.5, 4);

    // And Simple Builder can immediately keep building on top of the sculpted project!
    const extraOp = reopenedDoc.place({
      piece: 'cube',
      point: [2, 0, 0],
      normal: [0, 1, 0],
      view: [0, 0, -1],
      cell: 0.5,
      snap: true,
      ground: 0,
      turns: 0,
      span: [1, 1, 1],
      color: [16, 185, 129],
    });
    reopenedDoc.apply(extraOp, { id: 'builder', kind: 'person' });
    expect(reopenedDoc.state().faces).toBeGreaterThan(freeformState.faces);

    reopenedDoc.free();
  });
});
