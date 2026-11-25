// Standalone Simple Builder client built on brush-wasm + @brush/web-editor.
// Edits the exact same `.brush` project format as apps/demo and Brush Studio,
// and supports switching workspace policies live without converting the document.
import init, {
  Document,
  dragDelta,
  rotateGroupAbout,
  snapTurn,
  translateGroup,
  version,
  type Op,
  type Piece,
} from 'brush-wasm';
import {
  CameraFlightController,
  FREEFORM_MESH_PROFILE,
  SIMPLE_BUILDER_PROFILE,
  SelectController,
  Viewport,
  WORKSPACE_PROFILES,
  WorkspaceSession,
  rayPlaneY,
  syncDocumentToViewport,
  type PieceKind,
  type Vec3,
  type WorkspaceMode,
} from '@brush/web-editor';
import './style.css';

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

await init();
const v = version();
$('version').textContent = `engine ${v.engine}, ops ${v.ops}`;

const viewport = new Viewport($('view'));
viewport.setLeftButtonFree(true);

const flight = new CameraFlightController(viewport);
flight.start();

const actor = { id: 'simple-builder', kind: 'person' as const };
const session = new WorkspaceSession(new Document(), SIMPLE_BUILDER_PROFILE, viewport);

let activePiece: Exclude<PieceKind, 'object'> = 'wall';
let turns = 0;
let selectedIds: string[] = [];
const syncedObjects = new Set<string>();
let sceneSig = '';

const hexToRgb = (hex: string): Vec3 =>
  [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Vec3;

function status(text: string) {
  $('status').textContent = text;
}

function refresh(frame = false) {
  const doc = session.getDocument();
  const synced = syncDocumentToViewport(doc, viewport, {
    previousSignature: sceneSig,
    syncObjects: true,
    syncedObjectKeys: syncedObjects,
  });
  sceneSig = synced.signature;
  viewport.showSelection(selectedIds);
  if (frame) viewport.frame();

  const s = doc.state();
  const profile = session.getProfile();
  const stats: [string, string][] = [
    ['Workspace', profile.label],
    ['Revision', String(s.revision)],
    ['Vertices', s.vertices.toLocaleString()],
    ['Faces', s.faces.toLocaleString()],
    ['Splats', s.splats.toLocaleString()],
    ['Hash', s.hash.slice(0, 12)],
  ];
  $('stats').innerHTML = stats.map(([k, val]) => `<dt>${k}</dt><dd>${val}</dd>`).join('');
  $<HTMLButtonElement>('undo').disabled = !s.canUndo;
  $<HTMLButtonElement>('redo').disabled = !s.canRedo;
}

function renderPolicyControls() {
  const profile = session.getProfile();
  const policy = session.getPolicy();
  const mode = session.getMode();

  for (const b of document.querySelectorAll<HTMLButtonElement>('#profiles button')) {
    b.setAttribute('aria-pressed', String(b.dataset.profile === profile.id));
  }
  for (const b of document.querySelectorAll<HTMLButtonElement>('#modes button')) {
    const m = b.dataset.mode as WorkspaceMode;
    b.setAttribute('aria-pressed', String(m === mode));
    if (m === 'sculpt') b.disabled = !policy.sculpt.enabled;
  }

  $<HTMLInputElement>('snapToggle').checked = policy.placement.snapToGrid;
  $<HTMLInputElement>('gridToggle').checked = policy.viewport.showGrid;
  $<HTMLSelectElement>('cellSize').value = String(policy.placement.cell);
  $('buildSection').hidden = mode !== 'build';
  viewport.setLeftButtonFree(mode !== 'orbit');
  if (mode !== 'build') viewport.showGhost(null);
  if (mode !== 'select') {
    selectController.reset();
    selectedIds = [];
    viewport.showSelection([]);
  }

  // Rebuild piece palette from the active policy.
  const palette = policy.placement.palette.filter((p) => p !== 'object');
  if (!palette.includes(activePiece)) activePiece = palette[0] ?? 'cube';
  const paletteEl = $('palette');
  paletteEl.innerHTML = '';
  for (const kind of palette) {
    const btn = document.createElement('button');
    btn.textContent = kind[0].toUpperCase() + kind.slice(1);
    btn.setAttribute('aria-pressed', String(kind === activePiece));
    btn.addEventListener('click', () => {
      activePiece = kind;
      renderPolicyControls();
    });
    paletteEl.append(btn);
  }
  $('turnReadout').textContent = `${(turns * policy.placement.turnStepDeg) % 360}°`;
}

session.subscribe(() => {
  renderPolicyControls();
  refresh();
});

// ---- SelectController wired to policy ----
const selectController = new SelectController({
  getDocument: () => session.getDocument(),
  getViewport: () => viewport,
  getSelectedIds: () => selectedIds,
  getSnap: () => session.getPolicy().transform.snapMove,
  getCell: () => session.getPolicy().placement.cell,
  getActor: () => actor,
  math: {
    translateGroup: (list, delta) => translateGroup(list, delta),
    rotateGroupAbout: (list, axis, deg, center) => rotateGroupAbout(list, axis, deg, center),
    snapTurn: (raw, axis, start, snapOn) => snapTurn(raw, axis, start, snapOn),
    dragDelta,
  },
  onSelectionChange: (ids) => {
    selectedIds = ids;
    viewport.showSelection(ids);
  },
  onSync: () => {
    sceneSig = '';
    refresh();
  },
});

// ---- Build placement helpers ----
function computePlacementOp(clientX: number, clientY: number): Op | null {
  const doc = session.getDocument();
  const policy = session.getPolicy().placement;
  const { origin, dir } = viewport.ray(clientX, clientY);
  const hit = policy.attachToSurface ? doc.raycast(new Float32Array(origin), new Float32Array(dir)) : null;
  const point = hit?.point ?? rayPlaneY(origin, dir, 0);
  if (!point) return null;
  const normal: Vec3 = hit ? hit.normal : [0, 1, 0];
  try {
    return doc.place({
      piece: activePiece as Piece,
      point,
      normal,
      view: dir,
      cell: policy.cell,
      snap: policy.snapToGrid,
      ground: 0,
      turns,
      span: [1, 1, 1],
      color: hexToRgb($<HTMLInputElement>('color').value),
    });
  } catch {
    return null;
  }
}

function showGhostAt(clientX: number, clientY: number) {
  const op = computePlacementOp(clientX, clientY);
  if (!op) {
    viewport.showGhost(null);
    return;
  }
  const previewDoc = new Document();
  try {
    previewDoc.apply(op, undefined);
    viewport.showGhost({
      buffers: {
        positions: previewDoc.positions(),
        indices: previewDoc.indices(),
      },
    });
  } finally {
    previewDoc.free();
  }
}

// ---- Pointer events ----
const canvas = viewport.canvas;
canvas.addEventListener('contextmenu', (e) => e.preventDefault());

canvas.addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || !e.isPrimary) return;
  const mode = session.getMode();
  const doc = session.getDocument();

  if (mode === 'select') {
    if (selectController.pointerDown(e)) canvas.setPointerCapture(e.pointerId);
    return;
  }

  if (mode === 'build') {
    const op = computePlacementOp(e.clientX, e.clientY);
    if (!op) return;
    doc.apply(op, actor);
    refresh();
    status(`Placed ${activePiece}`);
    return;
  }

  if (mode === 'sculpt' && session.getPolicy().sculpt.enabled) {
    const { origin, dir } = viewport.ray(e.clientX, e.clientY);
    const hit = doc.raycast(new Float32Array(origin), new Float32Array(dir));
    if (!hit) return;
    doc.apply(
      {
        op: 'sculpt',
        brush: 'draw',
        dabs: [{ center: hit.point }],
        radius: 0.25,
        strength: 0.5,
        color: hexToRgb($<HTMLInputElement>('color').value),
        view: dir,
      },
      actor,
    );
    refresh();
    status('Sculpted surface');
  }
});

canvas.addEventListener('pointermove', (e) => {
  const mode = session.getMode();
  if (mode === 'select') {
    selectController.pointerMove(e);
    return;
  }
  if (mode === 'build' && e.buttons === 0) {
    showGhostAt(e.clientX, e.clientY);
  }
});

const finishPointer = (e: PointerEvent) => {
  selectController.finish(e.type === 'pointercancel', e.pointerId);
};
canvas.addEventListener('pointerup', finishPointer);
canvas.addEventListener('pointercancel', finishPointer);
canvas.addEventListener('pointerleave', () => viewport.showGhost(null));

// ---- UI controls ----
for (const b of document.querySelectorAll<HTMLButtonElement>('#profiles button')) {
  b.addEventListener('click', () => {
    const next = WORKSPACE_PROFILES[b.dataset.profile!];
    if (next) {
      session.setProfile(next);
      status(`Switched to ${next.label} workspace (document preserved)`);
    }
  });
}

for (const b of document.querySelectorAll<HTMLButtonElement>('#modes button')) {
  b.addEventListener('click', () => {
    session.setMode(b.dataset.mode as WorkspaceMode);
  });
}

$<HTMLInputElement>('snapToggle').addEventListener('change', (e) => {
  const on = (e.target as HTMLInputElement).checked;
  session.mergePolicy({ placement: { snapToGrid: on }, transform: { snapMove: on, snapRotate: on } });
});
$<HTMLInputElement>('gridToggle').addEventListener('change', (e) => {
  session.mergePolicy({ viewport: { showGrid: (e.target as HTMLInputElement).checked } });
});
$<HTMLSelectElement>('cellSize').addEventListener('change', (e) => {
  const cell = Number((e.target as HTMLSelectElement).value);
  session.mergePolicy({ placement: { cell }, viewport: { gridCell: cell } });
});

$('rotatePiece').addEventListener('click', () => {
  turns = (turns + 1) % 4;
  renderPolicyControls();
});

$('undo').addEventListener('click', () => session.getDocument().undo() && refresh());
$('redo').addEventListener('click', () => session.getDocument().redo() && refresh());

$('saveBrush').addEventListener('click', () => {
  const bytes = session.saveProject();
  const blob = new Blob([bytes as BlobPart], { type: 'application/octet-stream' });
  const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: 'project.brush' });
  a.click();
  URL.revokeObjectURL(a.href);
  status(`Saved project.brush (${bytes.byteLength.toLocaleString()} bytes)`);
});

$<HTMLInputElement>('file').addEventListener('change', async (ev) => {
  const file = (ev.target as HTMLInputElement).files?.[0];
  if (!file) return;
  try {
    const next = Document.open(new Uint8Array(await file.arrayBuffer()));
    session.getDocument().free();
    session.setDocument(next);
    syncedObjects.clear();
    sceneSig = '';
    selectedIds = [];
    refresh(true);
    status(`Opened ${file.name} in ${session.getProfile().label}`);
  } catch (err) {
    status(`Could not open ${file.name}: ${err instanceof Error ? err.message : err}`);
  }
});

function loadStarterCabin() {
  const next = new Document();
  const ops: Op[] = [
    { op: 'addBox', box: { center: [0, 0.1, 0], size: [3, 0.2, 3], yawDeg: 0 }, color: [156, 163, 175] },
    { op: 'addBox', box: { center: [0, 0.85, -1.35], size: [3, 1.3, 0.3], yawDeg: 0 }, color: [217, 119, 6] },
    { op: 'addBox', box: { center: [-1.35, 0.85, 0], size: [0.3, 1.3, 2.4], yawDeg: 0 }, color: [217, 119, 6] },
    { op: 'addBox', box: { center: [1.35, 0.85, 0], size: [0.3, 1.3, 2.4], yawDeg: 0 }, color: [217, 119, 6] },
  ];
  next.applyTransaction(ops, 'starterCabin', { id: 'builder', kind: 'app' });
  next.markSaved();
  session.getDocument().free();
  session.setDocument(next);
  syncedObjects.clear();
  sceneSig = '';
  selectedIds = [];
  refresh(true);
  status('Loaded starter cabin');
}

$('starter').addEventListener('click', loadStarterCabin);
$('blank').addEventListener('click', () => {
  session.getDocument().free();
  session.setDocument(new Document());
  syncedObjects.clear();
  sceneSig = '';
  selectedIds = [];
  refresh(true);
  status('Started blank project');
});

window.addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
  const k = e.key.toLowerCase();
  if ((e.ctrlKey || e.metaKey) && k === 'z') {
    e.preventDefault();
    if (e.shiftKey ? session.getDocument().redo() : session.getDocument().undo()) refresh();
    return;
  }
  if (!e.ctrlKey && !e.metaKey && !e.altKey) {
    if (k === 'r') {
      e.preventDefault();
      turns = (turns + 1) % 4;
      renderPolicyControls();
      return;
    }
    if (flight.keyDown(e.key)) {
      if (e.key === ' ') e.preventDefault();
    }
  }
});
window.addEventListener('keyup', (e) => flight.keyUp(e.key));
window.addEventListener('blur', () => flight.clear());

// Boot with starter cabin in Simple Builder profile.
void FREEFORM_MESH_PROFILE;
renderPolicyControls();
loadStarterCabin();
