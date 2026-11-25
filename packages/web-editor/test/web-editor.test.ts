import { describe, expect, test } from 'bun:test';
import {
  CameraFlightController,
  isFlightKey,
} from '../src/camera-flight';
import {
  SelectController,
  planeAngle,
  rayPlaneY,
  ringAngle,
  type MoveObjectOpLike,
  type SelectTransformMath,
  type SelectableDocument,
  type SelectableViewport,
} from '../src/select-controller';
import { instanceSignature, syncDocumentToViewport, type SceneInstanceLike } from '../src/sync';
import type { Vec3 } from '../src/viewport';

describe('CameraFlightController', () => {
  test('recognizes flight keys case-insensitively', () => {
    expect(isFlightKey('W')).toBe(true);
    expect(isFlightKey('ArrowUp')).toBe(true);
    expect(isFlightKey(' ')).toBe(true);
    expect(isFlightKey('Shift')).toBe(true);
    expect(isFlightKey('r')).toBe(false);
  });

  test('translates held keys into camera-relative panLocal deltas', () => {
    const calls: [number, number, number][] = [];
    const ctrl = new CameraFlightController(
      { panLocal: (f, r, u) => calls.push([f, r, u]) },
      { speed: 4, verticalScale: 0.75, maxDeltaSeconds: 0.05 },
    );

    expect(ctrl.keyDown('w')).toBe(true);
    expect(ctrl.keyDown('d')).toBe(true);
    expect(ctrl.keyDown(' ')).toBe(true);
    expect(ctrl.step(0.025)).toBe(true);
    expect(calls.length).toBe(1);
    expect(calls[0][0]).toBeCloseTo(0.1, 10);
    expect(calls[0][1]).toBeCloseTo(0.1, 10);
    expect(calls[0][2]).toBeCloseTo(0.075, 10);

    // Opposing keys cancel out.
    ctrl.keyDown('s');
    ctrl.keyDown('a');
    ctrl.keyDown('shift');
    expect(ctrl.step(0.025)).toBe(false);
    expect(calls.length).toBe(1);

    // Lag spikes are clamped to maxDeltaSeconds.
    ctrl.clear();
    ctrl.keyDown('w');
    expect(ctrl.step(1.0)).toBe(true);
    expect(calls[1][0]).toBeCloseTo(0.2, 10);
    expect(calls[1][1]).toBeCloseTo(0, 10);
    expect(calls[1][2]).toBeCloseTo(0, 10);
  });
});

describe('syncDocumentToViewport', () => {
  test('syncs mesh, splats, embedded objects, and skips unchanged instances', () => {
    const instances: SceneInstanceLike[] = [
      { id: 'i1', object: 'chair', position: [1, 0, 2], yawDeg: 90, pitchDeg: 0, rollDeg: 0, scale: 1 },
    ];
    const doc = {
      positions: () => new Float32Array([0, 0, 0]),
      colors: () => new Uint8Array([255, 255, 255]),
      indices: () => new Uint32Array([0, 0, 0]),
      normals: () => new Float32Array([0, 1, 0]),
      splats: () => new Float32Array(),
      splatColors: () => new Uint8Array(),
      scene: () => ({ instances }),
      instanceTransform: () => new Float32Array(16),
      loadedObjects: () => ['chair'],
      objectPositions: () => new Float32Array([0, 0, 0]),
      objectColors: () => new Uint8Array([255, 0, 0]),
      objectIndices: () => new Uint32Array([0, 0, 0]),
      objectNormals: () => new Float32Array([0, 1, 0]),
    };

    let modelCalls = 0;
    let splatCalls = 0;
    let objectCalls = 0;
    let instanceCalls = 0;
    const viewport = {
      setModel: () => { modelCalls++; },
      setSplats: () => { splatCalls++; },
      setObject: () => { objectCalls++; },
      setInstances: () => { instanceCalls++; },
    };

    const syncedKeys = new Set<string>();
    const first = syncDocumentToViewport(doc, viewport, { syncObjects: true, syncedObjectKeys: syncedKeys });
    expect(modelCalls).toBe(1);
    expect(splatCalls).toBe(1);
    expect(objectCalls).toBe(1);
    expect(instanceCalls).toBe(1);
    expect(first.instancesChanged).toBe(true);
    expect(first.signature).toBe(instanceSignature(instances));

    // Second sync with same signature skips setInstances and already-synced objects.
    const second = syncDocumentToViewport(doc, viewport, {
      previousSignature: first.signature,
      syncObjects: true,
      syncedObjectKeys: syncedKeys,
    });
    expect(objectCalls).toBe(1);
    expect(instanceCalls).toBe(1);
    expect(second.instancesChanged).toBe(false);
  });
});

describe('SelectController math & preview lifecycle', () => {
  test('rayPlaneY and planeAngle / ringAngle compute expected intersections', () => {
    expect(rayPlaneY([0, 2, 0], [0, -1, 0], 0)).toEqual([0, 0, 0]);
    expect(rayPlaneY([0, 2, 0], [1, 0, 0], 0)).toBeNull();
    expect(planeAngle('y', [1, 0, 0])).toBeCloseTo(90, 5);
    expect(planeAngle('y', [0, 0, 1])).toBeCloseTo(0, 5);
    expect(ringAngle({ origin: [1, 2, 0], dir: [0, -1, 0] }, 'y', [0, 0, 0])).toBeCloseTo(90, 5);
  });

  const math: SelectTransformMath = {
    translateGroup: (items, delta) =>
      items.map((i) => ({
        ...i,
        position: [i.position[0] + delta[0], i.position[1] + delta[1], i.position[2] + delta[2]] as Vec3,
      })),
    rotateGroupAbout: (items, axis, deg) =>
      items.map((i) => ({
        ...i,
        yawDeg: axis === 'y' ? i.yawDeg + deg : i.yawDeg,
        pitchDeg: axis === 'x' ? (i.pitchDeg ?? 0) + deg : (i.pitchDeg ?? 0),
        rollDeg: axis === 'z' ? (i.rollDeg ?? 0) + deg : (i.rollDeg ?? 0),
      })),
    snapTurn: (raw, _axis, _start, snap) => (snap ? Math.round(raw / 90) * 90 : raw),
    dragDelta: (grab, cur, cell, snap) => {
      const dx = cur[0] - grab[0];
      const dz = cur[2] - grab[2];
      return snap
        ? [Math.round(dx / cell) * cell, 0, Math.round(dz / cell) * cell]
        : [dx, 0, dz];
    },
  };

  function makeHarness() {
    let selected: string[] = [];
    let inPreview = false;
    let rev = 3;
    const events: string[] = [];
    let lastPreviewOps: MoveObjectOpLike[] = [];
    let instances: SceneInstanceLike[] = [
      { id: 'a', object: 'cube', position: [0, 0, 0], yawDeg: 0, pitchDeg: 0, rollDeg: 0, scale: 1 },
      { id: 'b', object: 'cube', position: [2, 0, 0], yawDeg: 0, pitchDeg: 0, rollDeg: 0, scale: 1 },
    ];

    const doc: SelectableDocument = {
      pick: (origin) => {
        if (origin[0] < 1) return { instance: 'a', point: [0, 0, 0] };
        if (origin[0] < 3) return { instance: 'b', point: [2, 0, 0] };
        return null;
      },
      scene: () => ({ instances }),
      state: () => ({ inPreview }),
      revision: () => rev,
      beginPreview: () => {
        inPreview = true;
        events.push(`begin:${rev}`);
        return rev;
      },
      updatePreview: (ops, expectedRev) => {
        lastPreviewOps = ops;
        events.push(`update:${expectedRev}:${ops.length}`);
      },
      commitPreview: (label) => {
        inPreview = false;
        rev += 1;
        events.push(`commit:${label}`);
        for (const op of lastPreviewOps) {
          instances = instances.map((i) =>
            i.id === op.id ? { ...i, position: op.position, yawDeg: op.yawDeg } : i,
          );
        }
      },
      cancelPreview: () => {
        const was = inPreview;
        inPreview = false;
        if (was) events.push('cancel');
        return was;
      },
      applyTransaction: (_ops, label) => {
        events.push(`tx:${label}`);
      },
    };

    const viewport: SelectableViewport = {
      ray: (clientX, clientY) => ({ origin: [clientX / 100, 5, clientY / 100], dir: [0, -1, 0] }),
      pickGizmo: () => null,
      highlightGizmo: () => {},
      selectionCenter: () => (selected.length ? [0, 0, 0] : null),
      instancesInRect: () => ['a', 'b'],
    };

    const controller = new SelectController({
      getDocument: () => doc,
      getViewport: () => viewport,
      getSelectedIds: () => selected,
      getSnap: () => true,
      getCell: () => 0.5,
      getActor: () => ({ kind: 'person' }),
      math,
      onSelectionChange: (ids) => { selected = ids; },
      onSync: (committed) => { events.push(`sync:${committed}`); },
    });

    return { controller, getSelected: () => selected, events, getInstances: () => instances };
  }

  test('drags an object through preview session and commits one transaction', () => {
    const h = makeHarness();
    expect(h.controller.pointerDown({ pointerId: 1, clientX: 0, clientY: 0 })).toBe(true);
    expect(h.getSelected()).toEqual(['a']);

    // Drag 1.0m in X (100px / 100 = 1.0)
    h.controller.pointerMove({ pointerId: 1, clientX: 100, clientY: 0, buttons: 1 });
    expect(h.events).toContain('begin:3');
    expect(h.events).toContain('update:3:1');

    h.controller.finish(false, 1);
    expect(h.events).toContain('commit:moveSelection');
    expect(h.getInstances()[0].position).toEqual([1, 0, 0]);
  });

  test('canceling a drag discards the preview cleanly', () => {
    const h = makeHarness();
    h.controller.pointerDown({ pointerId: 2, clientX: 0, clientY: 0 });
    h.controller.pointerMove({ pointerId: 2, clientX: 100, clientY: 0, buttons: 1 });
    h.controller.finish(true, 2);

    expect(h.events).toContain('cancel');
    expect(h.getInstances()[0].position).toEqual([0, 0, 0]);
  });

  test('box-select drags across empty space and selects instances in rect', () => {
    const h = makeHarness();
    expect(h.controller.pointerDown({ pointerId: 3, clientX: 500, clientY: 0 })).toBe(true);
    h.controller.pointerMove({ pointerId: 3, clientX: 600, clientY: 100, buttons: 1 });
    expect(h.getSelected()).toEqual(['a', 'b']);
    h.controller.finish(false, 3);
    expect(h.getSelected()).toEqual(['a', 'b']);
  });
});
