import type { InstanceView, ModelBuffers, Vec3 } from './viewport';

/** Structural representation of a placed scene instance from `Document.scene().instances`. */
export interface SceneInstanceLike {
  id: string;
  object: string;
  position: Vec3;
  yawDeg?: number;
  pitchDeg?: number;
  rollDeg?: number;
  scale?: number;
}

/** Minimal structural subset of `brush-wasm`'s `Document` needed for viewport sync. */
export interface SyncableDocument {
  positions(): Float32Array;
  colors(): Uint8Array;
  indices(): Uint32Array;
  normals(): Float32Array;
  splats(): Float32Array;
  splatColors(): Uint8Array;
  scene(): { instances: SceneInstanceLike[] };
  instanceTransform?(id: string): Float32Array | number[] | null | undefined;
  loadedObjects?(): string[];
  objectPositions?(key: string): Float32Array;
  objectColors?(key: string): Uint8Array;
  objectIndices?(key: string): Uint32Array;
  objectNormals?(key: string): Float32Array;
}

/** Minimal structural subset of `Viewport` needed for document sync. */
export interface ViewportSyncTarget {
  setModel(buffers: ModelBuffers): void;
  setSplats(positions: Float32Array, colors: Uint8Array): void;
  setObject?(key: string, buffers: ModelBuffers): void;
  setInstances?(instances: InstanceView[]): void;
}

export interface SyncDocumentOptions {
  /** Previous instance signature returned by `syncDocumentToViewport`; skips `setInstances` when unchanged. */
  previousSignature?: string;
  /** When true, also pushes any loaded object meshes from `doc` into `viewport.setObject`. */
  syncObjects?: boolean;
  /** Optional set of object keys already pushed to the viewport (mutated when new keys are synced). */
  syncedObjectKeys?: Set<string>;
}

export interface SyncDocumentResult {
  instances: SceneInstanceLike[];
  signature: string;
  instancesChanged: boolean;
}

/** Computes a cheap string signature for scene instances so redundant matrix updates can be skipped. */
export function instanceSignature(instances: readonly SceneInstanceLike[]): string {
  return instances
    .map(
      (i) =>
        `${i.id}:${i.object}:${i.position.join(',')}:${i.yawDeg ?? 0}:${i.pitchDeg ?? 0}:${i.rollDeg ?? 0}:${i.scale ?? 1}`,
    )
    .join('|');
}

/**
 * Syncs a `Document`'s surface mesh, Gaussian splats, optional embedded object geometries, and scene instances
 * into a `Viewport`.
 */
export function syncDocumentToViewport(
  doc: SyncableDocument,
  viewport: ViewportSyncTarget,
  options: SyncDocumentOptions = {},
): SyncDocumentResult {
  viewport.setModel({
    positions: doc.positions(),
    colors: doc.colors(),
    indices: doc.indices(),
    normals: doc.normals(),
  });
  viewport.setSplats(doc.splats(), doc.splatColors());

  if (
    options.syncObjects &&
    viewport.setObject &&
    doc.loadedObjects &&
    doc.objectPositions &&
    doc.objectColors &&
    doc.objectIndices &&
    doc.objectNormals
  ) {
    for (const key of doc.loadedObjects()) {
      if (options.syncedObjectKeys?.has(key)) continue;
      viewport.setObject(key, {
        positions: doc.objectPositions(key),
        colors: doc.objectColors(key),
        indices: doc.objectIndices(key),
        normals: doc.objectNormals(key),
      });
      options.syncedObjectKeys?.add(key);
    }
  }

  const instances = doc.scene().instances;
  const signature = instanceSignature(instances);
  const instancesChanged = signature !== options.previousSignature;
  if (instancesChanged && viewport.setInstances && doc.instanceTransform) {
    viewport.setInstances(
      instances.flatMap((i) => {
        const matrix = doc.instanceTransform!(i.id);
        return matrix ? [{ id: i.id, object: i.object, matrix }] : [];
      }),
    );
  }
  return { instances, signature, instancesChanged };
}
