import type { GridOptions } from './viewport';

export type PieceKind = 'wall' | 'floor' | 'ramp' | 'cube' | 'cylinder' | 'cone' | 'sphere' | 'object';

export interface PlacementPolicy {
  /** Snap placed pieces and objects to the grid cell. */
  snapToGrid: boolean;
  /** Grid cell size in metres. */
  cell: number;
  /** Restrict placement orientation to discrete turn steps (e.g. 90°). */
  discreteTurns: boolean;
  /** Rotation increment in degrees for piece placement. */
  turnStepDeg: number;
  /** Attach newly placed pieces against the hit surface normal. */
  attachToSurface: boolean;
  /** Piece kinds offered in the workspace palette. */
  palette: PieceKind[];
}

export interface TransformPolicy {
  /** Snap drag translations to the grid cell. */
  snapMove: boolean;
  /** Snap rotate-ring drags to `rotateStepDeg` increments. */
  snapRotate: boolean;
  /** Rotation step in degrees for button/keyboard turns and snapped drags. */
  rotateStepDeg: number;
  /** Allow pitching/rolling objects off the upright axis. */
  allowFreeTilt: boolean;
  /** Allow uniform scaling of selected instances. */
  allowScale: boolean;
}

export interface SculptPolicy {
  /** Whether direct mesh sculpting & splat painting tools are active in this workspace. */
  enabled: boolean;
  /** Optional grid-quantized sculpting brush centers (composable policy example). */
  snapToGrid: boolean;
}

export interface ViewportPolicy {
  /** Show the architectural floor grid. */
  showGrid: boolean;
  /** Grid spacing in metres. */
  gridCell: number;
  /** Floor grid extent in metres. */
  gridExtent: number;
  /** Show 3-axis rotate-ring gizmos on selected objects. */
  showRotateGizmo: boolean;
}

export interface InteractionPolicy {
  placement: PlacementPolicy;
  transform: TransformPolicy;
  sculpt: SculptPolicy;
  viewport: ViewportPolicy;
}

export interface InteractionPolicyPatch {
  placement?: Partial<PlacementPolicy>;
  transform?: Partial<TransformPolicy>;
  sculpt?: Partial<SculptPolicy>;
  viewport?: Partial<ViewportPolicy>;
}

export type WorkspaceMode = 'build' | 'select' | 'sculpt' | 'orbit';

export interface WorkspaceProfile {
  id: string;
  label: string;
  description: string;
  defaultMode: WorkspaceMode;
  policy: InteractionPolicy;
}

/**
 * Simple block/architectural construction mental model:
 * grid-snapped placement, discrete 90° turns, surface attachment, and a compact piece palette.
 */
export const SIMPLE_BUILDER_PROFILE: WorkspaceProfile = {
  id: 'simple-builder',
  label: 'Simple Builder',
  description: 'Grid-snapped block construction with 90° turns, surface attachment, and a focused piece palette.',
  defaultMode: 'build',
  policy: {
    placement: {
      snapToGrid: true,
      cell: 0.5,
      discreteTurns: true,
      turnStepDeg: 90,
      attachToSurface: true,
      palette: ['wall', 'floor', 'cube', 'ramp'],
    },
    transform: {
      snapMove: true,
      snapRotate: true,
      rotateStepDeg: 90,
      allowFreeTilt: false,
      allowScale: false,
    },
    sculpt: {
      enabled: false,
      snapToGrid: false,
    },
    viewport: {
      showGrid: true,
      gridCell: 0.5,
      gridExtent: 20,
      showRotateGizmo: true,
    },
  },
};

/**
 * Freeform mesh & composition mental model:
 * unrestricted object transforms, full piece palette, sculpting, and direct mesh editing.
 */
export const FREEFORM_MESH_PROFILE: WorkspaceProfile = {
  id: 'freeform-mesh',
  label: 'Freeform Mesh',
  description: 'Unrestricted object transforms, sculpting, splat painting, and whole-mesh topology tools.',
  defaultMode: 'sculpt',
  policy: {
    placement: {
      snapToGrid: false,
      cell: 0.5,
      discreteTurns: false,
      turnStepDeg: 15,
      attachToSurface: true,
      palette: ['wall', 'floor', 'ramp', 'cube', 'cylinder', 'cone', 'sphere', 'object'],
    },
    transform: {
      snapMove: false,
      snapRotate: false,
      rotateStepDeg: 15,
      allowFreeTilt: true,
      allowScale: true,
    },
    sculpt: {
      enabled: true,
      snapToGrid: false,
    },
    viewport: {
      showGrid: false,
      gridCell: 0.5,
      gridExtent: 20,
      showRotateGizmo: true,
    },
  },
};

export const WORKSPACE_PROFILES: Record<string, WorkspaceProfile> = {
  [SIMPLE_BUILDER_PROFILE.id]: SIMPLE_BUILDER_PROFILE,
  [FREEFORM_MESH_PROFILE.id]: FREEFORM_MESH_PROFILE,
};

/**
 * Composes an `InteractionPolicy` from a base policy and partial overrides without mutating the base.
 * Enables mixing policies across mental models (e.g. grid snapping + sculpting).
 */
export function composePolicy(
  base: InteractionPolicy,
  patch: InteractionPolicyPatch = {},
): InteractionPolicy {
  return {
    placement: {
      ...base.placement,
      ...patch.placement,
      palette: patch.placement?.palette ? [...patch.placement.palette] : [...base.placement.palette],
    },
    transform: {
      ...base.transform,
      ...patch.transform,
    },
    sculpt: {
      ...base.sculpt,
      ...patch.sculpt,
    },
    viewport: {
      ...base.viewport,
      ...patch.viewport,
    },
  };
}

/** Minimal structural subset of `Viewport` configured by a `WorkspaceSession`. */
export interface PolicyViewportTarget {
  setGrid(options: GridOptions): void;
  setRotateGizmo(visible: boolean): void;
}

/** Minimal structural subset of `Document` persisted by a `WorkspaceSession`. */
export interface WorkspaceDocumentLike {
  saveProject(): Uint8Array;
  metadata?(): unknown;
  setMetadata?(meta: unknown): void;
}

export type WorkspaceChangeListener = (state: {
  profile: WorkspaceProfile;
  policy: InteractionPolicy;
  mode: WorkspaceMode;
}) => void;

/**
 * Coordinates an active `WorkspaceProfile` and composed `InteractionPolicy` over a single shared
 * `Document`. Switching profiles or composing policy overrides never converts or resets the document.
 */
export class WorkspaceSession<TDoc extends WorkspaceDocumentLike = WorkspaceDocumentLike> {
  private doc: TDoc;
  private profile: WorkspaceProfile;
  private policy: InteractionPolicy;
  private mode: WorkspaceMode;
  private viewport: PolicyViewportTarget | null;
  private readonly listeners = new Set<WorkspaceChangeListener>();

  constructor(
    doc: TDoc,
    profile: WorkspaceProfile = SIMPLE_BUILDER_PROFILE,
    viewport: PolicyViewportTarget | null = null,
    initialPatch?: InteractionPolicyPatch,
  ) {
    this.doc = doc;
    this.profile = profile;
    this.policy = composePolicy(profile.policy, initialPatch);
    this.mode = profile.defaultMode;
    this.viewport = viewport;
    this.applyViewportPolicy();
  }

  getDocument(): TDoc {
    return this.doc;
  }

  getProfile(): WorkspaceProfile {
    return this.profile;
  }

  getPolicy(): InteractionPolicy {
    return this.policy;
  }

  getMode(): WorkspaceMode {
    return this.mode;
  }

  setViewport(viewport: PolicyViewportTarget | null) {
    this.viewport = viewport;
    this.applyViewportPolicy();
  }

  /** Replaces the active document (e.g. after opening a `.brush` project) while keeping the current workspace. */
  setDocument(doc: TDoc) {
    this.doc = doc;
  }

  /** Switches to another workspace mental model on the exact same document without data loss. */
  setProfile(profile: WorkspaceProfile, patch?: InteractionPolicyPatch) {
    this.profile = profile;
    this.policy = composePolicy(profile.policy, patch);
    this.mode = profile.defaultMode;
    this.applyViewportPolicy();
    this.emit();
  }

  /** Applies composable policy overrides on top of the current policy (e.g. turning on grid snapping while sculpting). */
  mergePolicy(patch: InteractionPolicyPatch) {
    this.policy = composePolicy(this.policy, patch);
    this.applyViewportPolicy();
    this.emit();
  }

  setMode(mode: WorkspaceMode) {
    if (this.mode === mode) return;
    this.mode = mode;
    this.applyViewportPolicy();
    this.emit();
  }

  /** Saves the shared document as a `.brush` container, recording the last-used workspace profile in metadata. */
  saveProject(): Uint8Array {
    if (this.doc.setMetadata) {
      const existing =
        this.doc.metadata && typeof this.doc.metadata() === 'object' && this.doc.metadata() !== null
          ? (this.doc.metadata() as Record<string, unknown>)
          : {};
      this.doc.setMetadata({
        ...existing,
        lastWorkspaceProfile: this.profile.id,
      });
    }
    return this.doc.saveProject();
  }

  subscribe(listener: WorkspaceChangeListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private applyViewportPolicy() {
    if (!this.viewport) return;
    const vp = this.policy.viewport;
    this.viewport.setGrid({
      visible: vp.showGrid,
      y: 0,
      cell: vp.gridCell,
      extent: vp.gridExtent,
    });
    this.viewport.setRotateGizmo(vp.showRotateGizmo && this.mode === 'select');
  }

  private emit() {
    const snapshot = { profile: this.profile, policy: this.policy, mode: this.mode };
    for (const listener of this.listeners) listener(snapshot);
  }
}
