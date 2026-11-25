// Wires the engine, the shared @brush/web-editor viewport, and the panels together. Every change to the mesh goes
// through an engine op, so everything on screen is also in the journal.
import init, { Document, opsSchema, version, type Applied, type Brush, type Op } from 'brush-wasm';
import { CameraFlightController, Viewport, syncDocumentToViewport, type Vec3 } from '@brush/web-editor';
import { SAMPLE_SCENE } from './sample';
import './style.css';

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

await init();
const v = version();
$('version').textContent = `engine ${v.engine}, ops ${v.ops}`;

const viewer = new Viewport($('view'));
viewer.setLeftButtonFree(true);
const flight = new CameraFlightController(viewer);
flight.start();
const person = { id: 'demo-user', kind: 'person' as const };
let doc = new Document();
const syncedObjects = new Set<string>();
let sceneSig = '';

// ---- state shown on screen ----

let brush: Brush = 'draw';

function refresh(frame = false) {
  const synced = syncDocumentToViewport(doc, viewer, {
    previousSignature: sceneSig,
    syncObjects: true,
    syncedObjectKeys: syncedObjects,
  });
  sceneSig = synced.signature;
  const s = doc.state();
  if (frame) viewer.frame();
  const m = doc.measure();
  const stats: [string, string][] = [
    ['Vertices', s.vertices.toLocaleString()],
    ['Faces', s.faces.toLocaleString()],
    ['Splats', s.splats.toLocaleString()],
    ['Area', `${m.surfaceArea.toFixed(2)} m²`],
    ['Volume', m.closed ? `${m.volume.toFixed(3)} m³` : 'open mesh'],
    ['Hash', s.hash],
    ['Edits', String(s.journalLength)],
    ['Attributes', s.attributes.map((a) => a.name).join(', ') || 'none'],
  ];
  $('stats').innerHTML = stats.map(([k, val]) => `<dt>${k}</dt><dd>${val}</dd>`).join('');
  $<HTMLButtonElement>('undo').disabled = !s.canUndo;
  $<HTMLButtonElement>('redo').disabled = !s.canRedo;
  $<HTMLButtonElement>('revert').disabled = !s.dirty;
  const lines = doc.journal().trim().split('\n').filter(Boolean).map((l) => JSON.parse(l));
  $('journal').innerHTML = lines
    .map((e) => {
      const what = e.event === 'applied' ? `${e.op.op} → ${e.outcome.count.toLocaleString()}` : e.event;
      const who = e.actor?.id ? ` · ${e.actor.id}` : '';
      return `<li title="${escapeHtml(JSON.stringify(e))}">${what}${who}</li>`;
    })
    .join('');
  $('journal').scrollTop = 1e9;
}

function escapeHtml(s: string) {
  return s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);
}

function status(text: string) {
  $('status').textContent = text;
}

/** Applies ops and reports what happened; errors are shown, never thrown at the user. */
function run(ops: Op | Op[], actor: { id: string; kind: 'person' | 'app' | 'model' } = person): boolean {
  const started = performance.now();
  try {
    const results: Applied[] = Array.isArray(ops) ? doc.applyAll(ops, actor) : [doc.apply(ops, actor)];
    const last = results.at(-1);
    const name = Array.isArray(ops) ? `${ops.length} ops` : ops.op;
    status(`${name}: ${last ? last.outcome.count.toLocaleString() : 0} in ${(performance.now() - started).toFixed(0)} ms${results.some((r) => r.changed) ? '' : ' (no change)'}`);
    refresh();
    return true;
  } catch (e) {
    status(String(e instanceof Error ? e.message : e));
    return false;
  }
}

function load(next: Document, label: string) {
  doc.free();
  doc = next;
  syncedObjects.clear();
  sceneSig = '';
  refresh(true);
  status(label);
}

// ---- file ----

$<HTMLInputElement>('file').addEventListener('change', async (ev) => {
  const file = (ev.target as HTMLInputElement).files?.[0];
  if (!file) return;
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const next = Document.open(bytes);
    load(next, `Opened ${file.name}`);
  } catch (e) {
    status(`Could not open ${file.name}: ${e instanceof Error ? e.message : e}`);
  }
});

$('sample').addEventListener('click', () => {
  const next = new Document();
  next.applyAll(SAMPLE_SCENE, { id: 'sample', kind: 'app' });
  next.markSaved();
  load(next, 'Sample scene');
});

for (const b of document.querySelectorAll<HTMLButtonElement>('[data-export]')) {
  b.addEventListener('click', () => {
    const format = b.dataset.export!;
    const bytes = format === 'brush' ? doc.saveProject() : doc.export(format);
    const blob = new Blob([bytes as BlobPart], { type: 'application/octet-stream' });
    const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: `mesh.${format}` });
    a.click();
    URL.revokeObjectURL(a.href);
  });
}

// ---- sculpting ----

const BRUSHES: Brush[] = ['draw', 'inflate', 'smooth', 'flatten', 'pinch', 'paint'];
for (const name of BRUSHES) {
  const b = Object.assign(document.createElement('button'), { textContent: name[0].toUpperCase() + name.slice(1) });
  b.setAttribute('aria-pressed', String(name === brush));
  b.addEventListener('click', () => {
    brush = name;
    for (const other of $('brushes').children) other.setAttribute('aria-pressed', String(other === b));
  });
  $('brushes').append(b);
}

const num = (id: string) => Number($<HTMLInputElement>(id).value);
const hexToRgb = (hex: string): Vec3 => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Vec3;

function brushOp(center: Vec3, view: Vec3): Op {
  return {
    op: 'sculpt',
    brush,
    dabs: [{ center }],
    radius: num('radius'),
    strength: num('strength'),
    invert: $<HTMLInputElement>('invert').checked,
    color: hexToRgb($<HTMLInputElement>('color').value),
    view,
  };
}

/** The splat op for one dab. `seed` counts the stroke's dabs, so a stroke sprayed in pieces equals the whole. */
function sprayOp(center: Vec3, view: Vec3, seed: number): Op {
  return {
    op: 'splatPaint',
    dabs: [{ center }],
    radius: num('sprayRadius'),
    size: num('spraySize'),
    color: hexToRgb($<HTMLInputElement>('sprayColor').value),
    opacity: num('sprayOpacity'),
    flow: num('sprayFlow'),
    variation: num('sprayVariation'),
    colorNoise: num('sprayNoise'),
    drip: num('sprayDrip'),
    seed,
    view,
  };
}

function eraseOp(center: Vec3): Op {
  return { op: 'splatErase', dabs: [{ center }], radius: num('eraseRadius') };
}

// ---- brush mode ----

type Mode = 'sculpt' | 'spray' | 'erase';
let mode: Mode = 'sculpt';
for (const b of document.querySelectorAll<HTMLButtonElement>('#mode button')) {
  b.addEventListener('click', () => {
    mode = b.dataset.mode as Mode;
    for (const other of document.querySelectorAll('#mode button')) {
      other.setAttribute('aria-pressed', String(other === b));
    }
    $('sculptPanel').hidden = mode !== 'sculpt';
    $('sprayPanel').hidden = mode !== 'spray';
    $('erasePanel').hidden = mode !== 'erase';
  });
}

$('bakeSplats').addEventListener('click', () => run({ op: 'splatBake', strength: 1, clear: true }));
$('clearSplats').addEventListener('click', () => {
  // Erase everywhere: a huge dab over the bounds centre clears all splats through the engine, so it is journaled.
  const b = doc.state().bounds;
  if (!b) return;
  const c: Vec3 = [(b.min[0] + b.max[0]) / 2, (b.min[1] + b.max[1]) / 2, (b.min[2] + b.max[2]) / 2];
  const span = Math.max(b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]);
  run({ op: 'splatErase', dabs: [{ center: c }], radius: span * 2 || 1e6 });
});

const canvas = viewer.canvas;
canvas.addEventListener('contextmenu', (e) => e.preventDefault());

let stroke: { last: Vec3; seed: number; dabs: number } | null = null;

function pick(e: PointerEvent) {
  const { origin, dir } = viewer.ray(e.clientX, e.clientY);
  const hit = doc.raycast(new Float32Array(origin), new Float32Array(dir));
  return hit ? { hit, dir } : null;
}

const brushRadius = () => (mode === 'sculpt' ? num('radius') : mode === 'spray' ? num('sprayRadius') : num('eraseRadius'));

canvas.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  const p = pick(e);
  if (!p) return;
  canvas.setPointerCapture(e.pointerId);
  try {
    const seed = Math.floor(Math.random() * 0xffffffff);
    const op = mode === 'sculpt' ? brushOp(p.hit.point, p.dir) : mode === 'spray' ? sprayOp(p.hit.point, p.dir, seed) : eraseOp(p.hit.point);
    doc.apply(op, person);
    stroke = { last: p.hit.point, seed, dabs: 1 };
    refresh();
  } catch (err) {
    status(String(err));
  }
});

canvas.addEventListener('pointermove', (e) => {
  const p = pick(e);
  viewer.showCursor(p?.hit ?? null, brushRadius());
  if (!stroke || !p) return;
  // Space dabs a fifth of the radius apart, filling the gap since the last one.
  const spacing = brushRadius() * 0.2;
  const [a, b] = [stroke.last, p.hit.point];
  const dist = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
  if (dist < spacing) return;
  const steps = Math.ceil(dist / spacing);
  const dabs = Array.from({ length: steps }, (_, i) => {
    const t = (i + 1) / steps;
    return { center: [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] as Vec3 };
  });
  if (mode === 'sculpt') {
    doc.extendStroke(dabs);
  } else {
    for (const dab of dabs) {
      const op = mode === 'spray' ? sprayOp(dab.center, p.dir, stroke.seed + stroke.dabs) : eraseOp(dab.center);
      doc.apply(op, person);
      stroke.dabs += 1;
    }
  }
  stroke.last = b;
  refresh();
});

const endStroke = () => {
  if (!stroke) return;
  doc.endStroke();
  stroke = null;
  refresh();
};
canvas.addEventListener('pointerup', endStroke);
canvas.addEventListener('pointercancel', endStroke);
canvas.addEventListener('pointerleave', () => viewer.showCursor(null, 0));

// ---- whole-mesh operations ----

const MESH_OPS: [string, () => Op][] = [
  ['Fill holes', () => ({ op: 'fillHoles', maxEdges: 500 })],
  ['Simplify ½', () => ({ op: 'simplify', targetRatio: 0.5 })],
  ['Smooth', () => ({ op: 'smooth', iterations: 3 })],
  ['Bake shadows', () => ({ op: 'bakeShadows', strength: 0.5 })],
];
for (const [label, make] of MESH_OPS) {
  const b = Object.assign(document.createElement('button'), { textContent: label });
  b.addEventListener('click', () => run(make()));
  $('meshOps').append(b);
}

// ---- view, history, console ----

$<HTMLInputElement>('flat').addEventListener('change', (e) => viewer.setFlatShading((e.target as HTMLInputElement).checked));
$<HTMLInputElement>('wire').addEventListener('change', (e) => viewer.setWireframe((e.target as HTMLInputElement).checked));
$<HTMLInputElement>('grid').addEventListener('change', (e) =>
  viewer.setGrid({ visible: (e.target as HTMLInputElement).checked, y: 0, cell: 0.5, extent: 16 }),
);

$('undo').addEventListener('click', () => doc.undo() && refresh());
$('redo').addEventListener('click', () => doc.redo() && refresh());
$('revert').addEventListener('click', () => doc.revert() && refresh());
window.addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
    e.preventDefault();
    if (e.shiftKey ? doc.redo() : doc.undo()) refresh();
    return;
  }
  if (!e.ctrlKey && !e.metaKey && !e.altKey && flight.keyDown(e.key)) {
    if (e.key === ' ') e.preventDefault();
  }
});
window.addEventListener('keyup', (e) => flight.keyUp(e.key));
window.addEventListener('blur', () => flight.clear());

const consoleBox = $<HTMLTextAreaElement>('console');
consoleBox.value = JSON.stringify({ op: 'carveBox', box: { center: [0, 1.2, 0], size: [1, 0.4, 1], yawDeg: 30 } }, null, 2);
$('run').addEventListener('click', () => {
  $('consoleError').textContent = '';
  let ops: Op | Op[];
  try {
    ops = JSON.parse(consoleBox.value);
  } catch (e) {
    $('consoleError').textContent = `Not JSON: ${e instanceof Error ? e.message : e}`;
    return;
  }
  if (!run(ops, { id: 'console', kind: 'app' })) $('consoleError').textContent = $('status').textContent ?? '';
});
$('schema').addEventListener('click', async () => {
  await navigator.clipboard.writeText(JSON.stringify(opsSchema(), null, 2));
  status('Op schema copied: paste it into an LLM as its tool definition.');
});

// Start with the sample scene.
$('sample').click();
