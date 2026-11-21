// The wasm package against the shared conformance suite: the same fixtures and ops must give the same hashes here as
// in native Rust. Build first: `npm run build` (wasm-pack writes pkg/).
import { expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Document, initSync, opsSchema, version, type Op } from '../pkg/brush_wasm.js';

initSync({ module: readFileSync(join(import.meta.dir, '../pkg/brush_wasm_bg.wasm')) });

const root = join(import.meta.dir, '../../..');
const cases = readdirSync(join(root, 'conformance/cases')).filter((f) => f.endsWith('.json')).sort();

for (const name of cases) {
  test(`conformance: ${name}`, () => {
    const c = JSON.parse(readFileSync(join(root, 'conformance/cases', name), 'utf8'));
    const doc = Document.open(readFileSync(join(root, 'conformance/fixtures', c.fixture)));
    doc.applyAll(c.ops, undefined);
    const s = doc.state();
    expect({ vertices: s.vertices, faces: s.faces, hash: s.hash }).toEqual(c.expect);
    doc.free();
  });
}

test('buffers, picking, export and the journal', () => {
  const doc = new Document();
  doc.apply({ op: 'addSphere', center: [0, 1, 0], radii: [1, 1, 1], segments: 32, rings: 16 }, undefined);
  const positions = doc.positions();
  expect(positions.length).toBe(doc.state().vertices * 3);
  expect(doc.colors().length).toBe(positions.length);
  expect(doc.normals().length).toBe(positions.length);
  expect(Math.max(...doc.indices())).toBe(doc.state().vertices - 1);
  expect(doc.raycast(new Float32Array([0, 1, 5]), new Float32Array([0, 0, -1]))!.point[2]).toBeCloseTo(1, 1);
  expect(doc.raycast(new Float32Array([5, 5, 5]), new Float32Array([0, 1, 0]))).toBeNull();
  const again = Document.open(doc.export('ply'));
  expect(again.state().hash).toBe(doc.state().hash);
  expect(doc.journal().trim().split('\n')).toHaveLength(1);
});

test('a live stroke is one undo step and matches the whole stroke', () => {
  const make = () => {
    const d = new Document();
    d.applyAll([{ op: 'addBox', box: { center: [0, 0.5, 0], size: [2, 1, 2] } }, { op: 'subdivide', maxEdge: 0.1 }], undefined);
    return d;
  };
  const dabs = Array.from({ length: 12 }, (_, k) => ({ center: [-0.6 + k * 0.1, 1, 0] as [number, number, number] }));
  const live = make();
  const base = live.state().hash;
  live.apply({ op: 'sculpt', brush: 'draw', dabs: dabs.slice(0, 1), radius: 0.3, view: [0, -1, 0] }, undefined);
  for (const d of dabs.slice(1)) live.extendStroke([d]);
  live.endStroke();
  const whole = make();
  whole.apply({ op: 'sculpt', brush: 'draw', dabs, radius: 0.3, view: [0, -1, 0] }, undefined);
  expect(live.state().hash).toBe(whole.state().hash);
  expect(live.undo()).toBe(true);
  expect(live.state().hash).toBe(base);
});

test('errors are thrown and leave the mesh alone', () => {
  const doc = new Document();
  doc.apply({ op: 'addBox', box: { center: [0, 0, 0], size: [1, 1, 1] } }, undefined);
  const h = doc.state().hash;
  expect(() => doc.apply({ op: 'addLathe', base: [0, 0, 0], profile: [[1, 0]] }, undefined)).toThrow(/lathe/);
  expect(() => doc.apply({ op: 'nope' } as unknown as Op, undefined)).toThrow(/bad op/);
  expect(() => doc.applyAll([{ op: 'smooth' }, { op: 'decimate', cell: Number.NaN }], undefined)).toThrow(/op 1/);
  expect(doc.state().hash).toBe(h);
  expect(() => doc.extendStroke([{ center: [0, 0, 0] }])).toThrow(/no stroke/);
  expect(() => Document.open(new TextEncoder().encode('hello'))).toThrow(/not a PLY/);
});

test('the schema and versions are exposed', () => {
  expect(version().ops).toBe('0.4.0');
  expect((opsSchema() as { oneOf: unknown[] }).oneOf).toHaveLength(29);
});

test('a read-only document views but never edits or journals', () => {
  const source = new Document();
  source.apply({ op: 'addBox', box: { center: [0, 0, 0], size: [1, 1, 1] } }, undefined);
  const doc = Document.openReadOnly(source.export('ply'));
  const before = doc.state();
  expect(doc.readOnly).toBe(true);
  expect(() => doc.apply({ op: 'smooth' }, undefined)).toThrow(/read-only/);
  expect(() => doc.applyAll([{ op: 'smooth' }], undefined)).toThrow(/read-only/);
  expect(() => doc.extendStroke([{ center: [0, 0, 0] }])).toThrow(/read-only/);
  expect(doc.undo() || doc.redo() || doc.revert()).toBe(false);
  expect(doc.state()).toEqual(before);
  expect(doc.journal()).toBe('');
  expect(doc.raycast(new Float32Array([0, 0, 5]), new Float32Array([0, 0, -1]))).not.toBeNull();
});

test('a PLY without faces opens as a point cloud', () => {
  const ply = 'ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n1 2 3\n';
  const doc = Document.openReadOnly(new TextEncoder().encode(ply));
  expect(doc.state().vertices).toBe(2);
  expect(doc.indices()).toHaveLength(0);
  expect(Array.from(doc.positions())).toEqual([0, 0, 0, 1, 2, 3]);
});

// ---- scenes: linked instances of other objects ----

type V3 = [number, number, number];

/** A 10 m square slab whose top is y = 0, opened like a scanned floor (clean, empty journal). */
const floor = () => {
  const d = new Document();
  d.apply({ op: 'addBox', box: { center: [0, -0.05, 0], size: [10, 0.1, 10] } }, undefined);
  return Document.open(d.export('ply'));
};

/** PLY bytes of a cube with its bottom centre at the origin: an object other documents point at. */
const cubePly = (size: number) => {
  const d = new Document();
  d.apply({ op: 'addBox', box: { center: [0, size / 2, 0], size: [size, size, size] } }, undefined);
  return d.export('ply');
};

test('a placed object is a pointer: refreshing its mesh moves every instance, without an edit', () => {
  const doc = floor();
  doc.loadScene({ instances: [{ id: 'a', object: 'scan:chair', position: [1, 0, 0] }, { id: 'b', object: 'scan:chair', position: [-1, 0, 0], yawDeg: 90, scale: 2 }] });
  expect(doc.state().dirty).toBe(false);
  expect(doc.missingObjects()).toEqual(['scan:chair']);
  expect(doc.hasObject('scan:chair')).toBe(false);
  const baseFaces = doc.state().faces;

  doc.setObject('scan:chair', cubePly(0.5));
  expect(doc.missingObjects()).toEqual([]);
  expect(doc.objectIndices('scan:chair').length).toBe(36);
  expect(doc.objectPositions('scan:chair').length).toBe(doc.objectNormals('scan:chair').length);
  expect(doc.objectColors('scan:chair').length).toBe(doc.objectPositions('scan:chair').length);
  const flat = Document.open(doc.export('ply', true));
  expect(flat.state().faces).toBe(baseFaces + 24);
  expect(Document.open(doc.export('ply')).state().faces).toBe(baseFaces);

  const journal = doc.journal();
  doc.setObject('scan:chair', cubePly(1));
  expect(doc.journal()).toBe(journal);
  expect(doc.state().dirty).toBe(false);
  const t = doc.instanceTransform('b')!;
  expect(t).toHaveLength(16);
  expect([t[12], t[13], t[14]]).toEqual([-1, 0, 0]);
  expect(doc.instanceTransform('nope')).toBeUndefined();
});

test('scene edits are journaled ops that undo, and dirty the document', () => {
  const doc = floor();
  doc.setObject('scan:chair', cubePly(0.5));
  doc.markSaved();
  doc.apply({ op: 'placeObject', id: 'a', object: 'scan:chair', position: [1, 0, 0] }, undefined);
  expect(doc.state().dirty).toBe(true);
  doc.apply({ op: 'moveObject', id: 'a', position: [2, 0, 0], yawDeg: 90, scale: 1 }, undefined);
  expect(doc.scene().instances[0].position).toEqual([2, 0, 0]);
  expect(doc.undo()).toBe(true);
  expect(doc.scene().instances[0].position).toEqual([1, 0, 0]);
  expect(() => doc.apply({ op: 'removeObject', id: 'nope' }, undefined)).toThrow();
  const faces = doc.state().faces;
  doc.apply({ op: 'bakeObject', id: 'a' }, undefined);
  expect(doc.scene().instances).toEqual([]);
  expect(doc.state().faces).toBe(faces + 12);
  doc.undo();
  doc.apply({ op: 'removeObject', id: 'a' }, undefined);
  expect(doc.scene().instances).toEqual([]);
  expect(journalOps(doc)).toEqual(['placeObject', 'moveObject', 'bakeObject', 'removeObject']);
});

const journalOps = (doc: Document) =>
  doc.journal().trim().split('\n').map((l) => JSON.parse(l)).filter((e) => e.event === 'applied').map((e) => e.op.op);

test('pick sees instances as well as the mesh', () => {
  const doc = floor();
  doc.setObject('scan:chair', cubePly(1));
  doc.apply({ op: 'placeObject', id: 'a', object: 'scan:chair', position: [2, 0, 0] }, undefined);
  const down = new Float32Array([0, -1, 0]);
  const hit = doc.pick(new Float32Array([2, 5, 0]), down)!;
  expect(hit.instance).toBe('a');
  expect(hit.point[1]).toBeCloseTo(1, 4);
  expect(doc.pick(new Float32Array([-2, 5, 0]), down)!.instance).toBeUndefined();
  expect(doc.pick(new Float32Array([0, 5, 20]), down)).toBeNull();
});

test('place snaps built-in pieces to the grid and stacks objects on them', () => {
  const doc = floor();
  doc.setObject('scan:chair', cubePly(0.5));
  const base = { point: [0.1, 0, 0.1] as V3, normal: [0, 1, 0] as V3, view: [0, -0.5, 1] as V3, cell: 0.5, color: [7, 7, 7] as V3 };
  const cube = doc.place({ piece: 'cube', ...base });
  expect(cube).toMatchObject({ op: 'addBox', box: { center: [0.25, 0.25, 0.25], size: [0.5, 0.5, 0.5] } });
  for (const piece of ['wall', 'floor'] as const) expect(doc.place({ piece, ...base }).op).toBe('addBox');
  expect(doc.place({ piece: 'ramp', ...base }).op).toBe('addRamp');
  expect(doc.place({ piece: 'cone', ...base }).op).toBe('addFrustum');
  expect(doc.place({ piece: 'sphere', ...base }).op).toBe('addSphere');
  // Placing changes nothing until the op is applied.
  expect(doc.state().journalLength).toBe(0);
  doc.apply(cube, undefined);

  const top = doc.pick(new Float32Array([0.25, 5, 0.25]), new Float32Array([0, -1, 0]))!;
  const op = doc.place({ piece: 'object', object: 'scan:chair', ...base, point: top.point, normal: top.normal });
  expect(op).toMatchObject({ op: 'placeObject', id: 'o1', object: 'scan:chair', position: [0.25, 0.5, 0.25] });
  if (op.op !== 'placeObject') throw new Error(op.op);
  const ghost = doc.objectTransform('scan:chair', op.position, op.yawDeg ?? 0, op.scale ?? 1)!;
  doc.apply(op, undefined);
  expect(Array.from(doc.instanceTransform('o1')!)).toEqual(Array.from(ghost));
  // A given id re-places an existing instance (the frontend turns it into moveObject).
  expect(doc.place({ piece: 'object', object: 'scan:chair', id: 'o1', ...base }).op).toBe('placeObject');
  expect(doc.objectTransform('scan:unknown', [0, 0, 0], 0, 1)).toBeUndefined();
  expect(() => doc.place({ piece: 'object', object: 'scan:unknown', ...base })).toThrow();
  expect(() => doc.place({ piece: 'cube', ...base, cell: 0 })).toThrow(/cell/);
});

test('a read-only document still shows its scene', () => {
  const source = floor();
  const doc = Document.openReadOnly(source.export('ply'));
  doc.loadScene({ instances: [{ id: 'a', object: 'scan:chair', position: [0, 0, 0] }] });
  doc.setObject('scan:chair', cubePly(0.5));
  expect(doc.scene().instances).toHaveLength(1);
  expect(doc.instanceTransform('a')).toBeDefined();
  expect(() => doc.apply({ op: 'removeObject', id: 'a' }, undefined)).toThrow(/read-only/);
});
