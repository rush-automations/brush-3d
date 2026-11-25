import type { SceneInstanceLike } from './sync';
import type { Vec3 } from './viewport';

export type Axis = 'x' | 'y' | 'z';

export interface MarqueeRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface TurnReadout {
  axis: Axis;
  deg: number;
}

export interface PointerEventLike {
  pointerId: number;
  clientX: number;
  clientY: number;
  buttons?: number;
  shiftKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
}

export interface MoveObjectOpLike {
  op: 'moveObject';
  id: string;
  position: Vec3;
  yawDeg: number;
  pitchDeg: number;
  rollDeg: number;
  scale: number;
}

/** Structural subset of `brush-wasm`'s `Document` required by `SelectController`. */
export interface SelectableDocument {
  pick(origin: Float32Array, dir: Float32Array): { instance?: string; point: Vec3 } | null;
  scene(): { instances: SceneInstanceLike[] };
  state(): { inPreview?: boolean };
  revision(): number;
  beginPreview(): number;
  updatePreview(ops: MoveObjectOpLike[], expectedRevision?: number): unknown;
  commitPreview(label?: string, actor?: unknown): unknown;
  cancelPreview(): boolean;
  applyTransaction(ops: MoveObjectOpLike[], label?: string, actor?: unknown): unknown;
}

/** Structural subset of `Viewport` required by `SelectController`. */
export interface SelectableViewport {
  ray(clientX: number, clientY: number): { origin: Vec3; dir: Vec3 };
  pickGizmo(clientX: number, clientY: number): Axis | null;
  highlightGizmo(axis: Axis | null): void;
  selectionCenter(): Vec3 | null;
  instancesInRect(x0: number, y0: number, x1: number, y1: number): string[];
}

/** Transform math functions (normally supplied by `@brush/engine` / `brush-wasm`). */
export interface SelectTransformMath {
  translateGroup(instances: SceneInstanceLike[], delta: Vec3): SceneInstanceLike[];
  rotateGroupAbout(instances: SceneInstanceLike[], axis: Axis, deg: number, center: Vec3): SceneInstanceLike[];
  snapTurn(rawDeg: number, axis: Axis, start: SceneInstanceLike[], snap: boolean): number;
  dragDelta(grab: Vec3, current: Vec3, cell: number, snap: boolean): Vec3;
}

export interface SelectControllerHooks {
  getDocument(): SelectableDocument | null;
  getViewport(): SelectableViewport | null;
  getSelectedIds(): string[];
  getSnap(): boolean;
  getCell(): number;
  getActor(): unknown;
  math: SelectTransformMath;
  dragThresholdPx?: number;
  /** Called when selection changes. */
  onSelectionChange(ids: string[]): void;
  /** Called when the box-select marquee starts, updates, or clears. */
  onMarqueeChange?(marquee: MarqueeRect | null): void;
  /** Called when a rotate-ring turn readout updates or clears. */
  onTurnReadoutChange?(readout: TurnReadout | null): void;
  /** Called after preview updates, commits, or cancels so the host can sync & refresh the viewport. */
  onSync?(committed: boolean): void;
  /** Optional hook when clicking the main mesh (non-instance): can detach an island and return its new instance id. */
  onPickMesh?(clientX: number, clientY: number): string | null;
}

type ActiveGesture =
  | {
      kind: 'drag';
      pointerId: number;
      startX: number;
      startY: number;
      grab: Vec3;
      planeY: number;
      start: SceneInstanceLike[];
      baseRevision: number | null;
      moved: boolean;
      delta: Vec3;
      narrowTo: string | null;
    }
  | {
      kind: 'rotate';
      pointerId: number;
      startX: number;
      startY: number;
      axis: Axis;
      center: Vec3;
      lastAngle: number | null;
      lastX: number;
      raw: number;
      turn: number;
      start: SceneInstanceLike[];
      baseRevision: number | null;
      moved: boolean;
    }
  | {
      kind: 'box';
      pointerId: number;
      startX: number;
      startY: number;
      additive: boolean;
      base: string[];
      moved: boolean;
    };

const AXIS_INDEX: Record<Axis, 0 | 1 | 2> = { x: 0, y: 1, z: 2 };

/** Intersects a ray with the horizontal plane `y = planeY`. Returns null if parallel or behind the origin. */
export function rayPlaneY(origin: Vec3, dir: Vec3, planeY: number): Vec3 | null {
  if (Math.abs(dir[1]) < 1e-6) return null;
  const t = (planeY - origin[1]) / dir[1];
  if (t <= 0) return null;
  return [origin[0] + dir[0] * t, planeY, origin[2] + dir[2] * t];
}

/**
 * Angle in degrees of an offset vector `p` in the plane perpendicular to `axis`, increasing in the
 * right-hand rotation direction around `+axis`.
 */
export function planeAngle(axis: Axis, p: Vec3): number {
  const rad =
    axis === 'y'
      ? Math.atan2(p[0], p[2])
      : axis === 'x'
        ? Math.atan2(p[2], p[1])
        : Math.atan2(p[1], p[0]);
  return (rad * 180) / Math.PI;
}

/**
 * Pointer's angle around a rotate ring where its ray meets the ring's plane. Returns null when the ring
 * is viewed nearly edge-on.
 */
export function ringAngle(
  ray: { origin: Vec3; dir: Vec3 },
  axis: Axis,
  center: Vec3,
): number | null {
  const k = AXIS_INDEX[axis];
  if (Math.abs(ray.dir[k]) < 0.06) return null;
  const t = (center[k] - ray.origin[k]) / ray.dir[k];
  if (t <= 0) return null;
  const p: Vec3 = [
    ray.origin[0] + ray.dir[0] * t - center[0],
    ray.origin[1] + ray.dir[1] * t - center[1],
    ray.origin[2] + ray.dir[2] * t - center[2],
  ];
  return planeAngle(axis, p);
}

function toMoveOp(inst: SceneInstanceLike): MoveObjectOpLike {
  return {
    op: 'moveObject',
    id: inst.id,
    position: inst.position,
    yawDeg: inst.yawDeg ?? 0,
    pitchDeg: inst.pitchDeg ?? 0,
    rollDeg: inst.rollDeg ?? 0,
    scale: inst.scale ?? 1,
  };
}

/**
 * Reusable pointer controller for Select mode: single/additive selection, box-select marquee,
 * horizontal plane drag, and 3-axis rotate-ring gizmo drag backed by engine preview sessions.
 */
export class SelectController {
  private gesture: ActiveGesture | null = null;
  private readonly hooks: SelectControllerHooks;
  private readonly dragThresholdPx: number;

  constructor(hooks: SelectControllerHooks) {
    this.hooks = hooks;
    this.dragThresholdPx = hooks.dragThresholdPx ?? 5;
  }

  get active(): boolean {
    return this.gesture !== null;
  }

  get activePointerId(): number | null {
    return this.gesture?.pointerId ?? null;
  }

  private updatePreview(ops: MoveObjectOpLike[], baseRevision: number | null): number | null {
    const doc = this.hooks.getDocument();
    if (!doc) return baseRevision;
    let rev = baseRevision;
    if (!doc.state().inPreview) {
      rev = doc.beginPreview();
    }
    doc.updatePreview(ops, rev ?? doc.revision());
    this.hooks.onSync?.(false);
    return rev;
  }

  /**
   * Handles a primary pointer-down in Select mode.
   * Returns `true` if a gesture started and the caller should capture the pointer.
   */
  pointerDown(e: PointerEventLike): boolean {
    const doc = this.hooks.getDocument();
    const view = this.hooks.getViewport();
    if (!doc || !view) return false;

    const selectedIds = this.hooks.getSelectedIds();

    // 1. Rotate ring hit takes priority when something is selected.
    const ring = selectedIds.length ? view.pickGizmo(e.clientX, e.clientY) : null;
    const center = view.selectionCenter();
    if (ring && center) {
      const live = doc.scene().instances;
      const start = selectedIds.flatMap((id) => live.find((i) => i.id === id) ?? []);
      this.gesture = {
        kind: 'rotate',
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
        axis: ring,
        center,
        lastAngle: ringAngle(view.ray(e.clientX, e.clientY), ring, center),
        lastX: e.clientX,
        raw: 0,
        turn: 0,
        start,
        baseRevision: null,
        moved: false,
      };
      view.highlightGizmo(ring);
      this.hooks.onTurnReadoutChange?.({ axis: ring, deg: 0 });
      return true;
    }

    const { origin, dir } = view.ray(e.clientX, e.clientY);
    const picked = doc.pick(new Float32Array(origin), new Float32Array(dir));
    const additive = Boolean(e.shiftKey || e.ctrlKey || e.metaKey);

    // 2. Empty space: start box-select drag (plain click clears on pointerUp).
    if (!picked) {
      this.gesture = {
        kind: 'box',
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
        additive,
        base: additive ? selectedIds : [],
        moved: false,
      };
      return true;
    }

    // 3. Instance or mesh island hit.
    let id = picked.instance ?? null;
    let wasSelected = Boolean(id && selectedIds.includes(id));
    if (id && additive && wasSelected) {
      this.hooks.onSelectionChange(selectedIds.filter((x) => x !== id));
      return false;
    }
    if (!id) {
      id = this.hooks.onPickMesh?.(e.clientX, e.clientY) ?? null;
      if (!id) return false;
      wasSelected = false;
    }

    const nextIds = wasSelected ? selectedIds : additive ? [...selectedIds, id] : [id];
    this.hooks.onSelectionChange(nextIds);

    const planeY = picked.point[1];
    const grab = rayPlaneY(origin, dir, planeY) ?? picked.point;
    const live = doc.scene().instances;
    this.gesture = {
      kind: 'drag',
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      grab,
      planeY,
      start: nextIds.flatMap((x) => live.find((i) => i.id === x) ?? []),
      baseRevision: null,
      moved: false,
      delta: [0, 0, 0],
      narrowTo: wasSelected && !additive && selectedIds.length > 1 ? id : null,
    };
    return true;
  }

  /** Handles pointer-move in Select mode (both hover gizmo highlight and active gestures). */
  pointerMove(e: PointerEventLike) {
    const view = this.hooks.getViewport();
    if (!view) return;
    const g = this.gesture;
    if (!g) {
      if ((e.buttons ?? 0) === 0) view.highlightGizmo(view.pickGizmo(e.clientX, e.clientY));
      return;
    }
    if (g.pointerId !== e.pointerId) return;

    if (g.kind === 'rotate') {
      const angle = ringAngle(view.ray(e.clientX, e.clientY), g.axis, g.center);
      if (angle !== null && g.lastAngle !== null) {
        g.raw += ((angle - g.lastAngle + 540) % 360) - 180;
      } else if (angle === null) {
        g.raw += (e.clientX - g.lastX) * 0.5;
      }
      g.lastAngle = angle;
      g.lastX = e.clientX;
      g.moved = true;
      const turn = this.hooks.math.snapTurn(g.raw, g.axis, g.start, this.hooks.getSnap());
      if (turn === g.turn) return;
      g.turn = turn;
      const ops = this.hooks.math.rotateGroupAbout(g.start, g.axis, turn, g.center).map(toMoveOp);
      g.baseRevision = this.updatePreview(ops, g.baseRevision);
      this.hooks.onTurnReadoutChange?.({ axis: g.axis, deg: turn });
      return;
    }

    const far = Math.hypot(e.clientX - g.startX, e.clientY - g.startY) >= this.dragThresholdPx;
    if (!g.moved && !far) return;
    g.moved = true;

    if (g.kind === 'drag') {
      const { origin, dir } = view.ray(e.clientX, e.clientY);
      const pt = rayPlaneY(origin, dir, g.planeY);
      if (!pt) return;
      const delta = this.hooks.math.dragDelta(g.grab, pt, this.hooks.getCell(), this.hooks.getSnap());
      if (delta[0] === g.delta[0] && delta[2] === g.delta[2]) return;
      g.delta = delta;
      const ops = this.hooks.math.translateGroup(g.start, delta).map(toMoveOp);
      g.baseRevision = this.updatePreview(ops, g.baseRevision);
    } else {
      this.hooks.onMarqueeChange?.({ x0: g.startX, y0: g.startY, x1: e.clientX, y1: e.clientY });
      const inside = view.instancesInRect(g.startX, g.startY, e.clientX, e.clientY);
      this.hooks.onSelectionChange([...new Set([...g.base, ...inside])]);
    }
  }

  /** Ends an active gesture for `pointerId` (or any active gesture if omitted). */
  finish(cancel = false, pointerId?: number) {
    const g = this.gesture;
    if (!g) return;
    if (pointerId !== undefined && g.pointerId !== pointerId) return;
    this.gesture = null;

    if (g.kind === 'box') {
      if (!g.moved && !g.additive) this.hooks.onSelectionChange([]);
      this.hooks.onMarqueeChange?.(null);
      return;
    }

    const doc = this.hooks.getDocument();
    const view = this.hooks.getViewport();
    const actor = this.hooks.getActor();

    if (g.kind === 'rotate') {
      view?.highlightGizmo(null);
      this.hooks.onTurnReadoutChange?.(null);
      if (!doc || !view) return;
      let committed = false;
      if (g.turn !== 0 && !cancel) {
        try {
          if (doc.state().inPreview) doc.commitPreview('rotateSelection', actor);
          else {
            const ops = this.hooks.math.rotateGroupAbout(g.start, g.axis, g.turn, g.center).map(toMoveOp);
            doc.applyTransaction(ops, 'rotateSelection', actor);
          }
          committed = true;
        } catch {
          doc.cancelPreview();
        }
      } else {
        doc.cancelPreview();
      }
      this.hooks.onSync?.(committed);
      return;
    }

    const moved = g.moved && (g.delta[0] !== 0 || g.delta[2] !== 0);
    if (!doc || !view) return;
    if (moved && !cancel) {
      let committed = false;
      try {
        if (doc.state().inPreview) doc.commitPreview('moveSelection', actor);
        else {
          const ops = this.hooks.math.translateGroup(g.start, g.delta).map(toMoveOp);
          doc.applyTransaction(ops, 'moveSelection', actor);
        }
        committed = true;
      } catch {
        doc.cancelPreview();
      }
      this.hooks.onSync?.(committed);
    } else if (g.moved || doc.state().inPreview) {
      doc.cancelPreview();
      this.hooks.onSync?.(false);
    } else if (g.narrowTo) {
      this.hooks.onSelectionChange([g.narrowTo]);
    }
  }

  /** Cancels any active gesture and discards any in-flight preview. */
  reset() {
    this.finish(true);
    const doc = this.hooks.getDocument();
    if (doc?.cancelPreview()) {
      this.hooks.onSync?.(false);
    }
    this.hooks.onMarqueeChange?.(null);
    this.hooks.onTurnReadoutChange?.(null);
  }
}
