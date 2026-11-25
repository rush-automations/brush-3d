import type { InteractionPolicy } from './policies';
import type { PointerEventLike } from './select-controller';
import type { SyncableDocument, ViewportSyncTarget } from './sync';

/** Capability flag names exposed by `brush-wasm`'s `capabilities().features`. */
export type CapabilityFlag =
  | 'transactions'
  | 'previews'
  | 'revisionGuards'
  | 'workplanes'
  | 'editableFeatures'
  | 'persistentConstraints'
  | 'splats'
  | 'linkedInstances';

/** Structural subset of `brush-wasm`'s `capabilities()` manifest. */
export interface EngineCapabilities {
  engineVersion: string;
  opsVersion: string;
  projectFormatVersion: number;
  formats: { read: string[]; write: string[] };
  features: Record<CapabilityFlag, boolean>;
  featureKinds: string[];
  constraintKinds: string[];
}

/** Structural subset of `Document` available to tool extensions. */
export interface ExtensionDocument extends SyncableDocument {
  apply(op: unknown, actor?: unknown): unknown;
  applyTransaction(ops: unknown[], label?: string, actor?: unknown): unknown;
  beginPreview(): number;
  updatePreview(ops: unknown[], expectedRevision?: number): unknown;
  commitPreview(label?: string, actor?: unknown): unknown;
  cancelPreview(): boolean;
  revision(): number;
}

/** Context passed to `ToolExtension` handlers so all edits flow through the shared transaction system. */
export interface ToolContext<TDoc extends ExtensionDocument = ExtensionDocument, TView = unknown> {
  document: TDoc;
  viewport: TView;
  policy: InteractionPolicy;
  actor: unknown;
  /** Commits one or more validated engine ops as a single atomic undo step. */
  commit(ops: unknown[], label: string): void;
  /** Requests a viewport refresh after mutating or previewing the document. */
  requestSync(): void;
}

/**
 * Compiled tool extension interface.
 * Tools express editing intent and produce validated document changes through `ctx.commit` or
 * `ctx.document` preview sessions without modifying the core editor.
 */
export interface ToolExtension<TDoc extends ExtensionDocument = ExtensionDocument, TView = unknown> {
  readonly id: string;
  readonly label: string;
  readonly description?: string;
  readonly requiredCapabilities?: readonly CapabilityFlag[];
  activate?(ctx: ToolContext<TDoc, TView>): void;
  deactivate?(ctx: ToolContext<TDoc, TView>): void;
  pointerDown?(ctx: ToolContext<TDoc, TView>, event: PointerEventLike): boolean | void;
  pointerMove?(ctx: ToolContext<TDoc, TView>, event: PointerEventLike): void;
  pointerUp?(ctx: ToolContext<TDoc, TView>, event: PointerEventLike, canceled: boolean): void;
  /** Optional programmatic entry point (e.g. from a numeric panel or AI agent). */
  execute?(ctx: ToolContext<TDoc, TView>, params?: Record<string, unknown>): void;
}

/**
 * Composable policy extension interface.
 * Transforms an `InteractionPolicy` without mutating the base policy or converting the document.
 */
export interface PolicyExtension {
  readonly id: string;
  readonly label: string;
  readonly description?: string;
  apply(base: InteractionPolicy): InteractionPolicy;
}

/**
 * Custom viewport renderer / overlay extension interface.
 * Mounts additional visual feedback onto a viewport without modifying `Viewport` core code.
 */
export interface RendererExtension<TView = ViewportSyncTarget, TDoc = SyncableDocument> {
  readonly id: string;
  readonly label: string;
  mount(viewport: TView): void;
  unmount(viewport: TView): void;
  onSync?(document: TDoc, viewport: TView): void;
}

/** Metadata describing one loadable 3D asset from an `AssetProviderExtension`. */
export interface AssetDescriptor {
  key: string;
  name: string;
  format: 'ply' | 'obj' | 'glb' | 'brush';
  tags?: string[];
  description?: string;
}

/**
 * Pluggable asset/catalog provider interface (e.g. local demos, Brush Gallery, Meshy, CAD libraries).
 */
export interface AssetProviderExtension {
  readonly id: string;
  readonly label: string;
  search(query: string, signal?: AbortSignal): Promise<AssetDescriptor[]>;
  loadBytes(key: string, signal?: AbortSignal): Promise<Uint8Array>;
}

/**
 * Central registry for compiled Brush extensions with capability checking.
 */
export class ExtensionRegistry<TDoc extends ExtensionDocument = ExtensionDocument, TView = unknown> {
  private readonly tools = new Map<string, ToolExtension<TDoc, TView>>();
  private readonly policies = new Map<string, PolicyExtension>();
  private readonly renderers = new Map<string, RendererExtension<TView, TDoc>>();
  private readonly assetProviders = new Map<string, AssetProviderExtension>();
  private readonly capabilities: EngineCapabilities | null;

  constructor(capabilities: EngineCapabilities | null = null) {
    this.capabilities = capabilities;
  }

  registerTool(tool: ToolExtension<TDoc, TView>): this {
    if (this.tools.has(tool.id)) {
      throw new Error(`Tool extension '${tool.id}' is already registered`);
    }
    if (this.capabilities && tool.requiredCapabilities?.length) {
      const missing = tool.requiredCapabilities.filter((cap) => !this.capabilities!.features[cap]);
      if (missing.length) {
        throw new Error(
          `Tool extension '${tool.id}' requires unsupported engine capabilities: ${missing.join(', ')}`,
        );
      }
    }
    this.tools.set(tool.id, tool);
    return this;
  }

  registerPolicy(ext: PolicyExtension): this {
    if (this.policies.has(ext.id)) {
      throw new Error(`Policy extension '${ext.id}' is already registered`);
    }
    this.policies.set(ext.id, ext);
    return this;
  }

  registerRenderer(ext: RendererExtension<TView, TDoc>): this {
    if (this.renderers.has(ext.id)) {
      throw new Error(`Renderer extension '${ext.id}' is already registered`);
    }
    this.renderers.set(ext.id, ext);
    return this;
  }

  registerAssetProvider(ext: AssetProviderExtension): this {
    if (this.assetProviders.has(ext.id)) {
      throw new Error(`Asset provider extension '${ext.id}' is already registered`);
    }
    this.assetProviders.set(ext.id, ext);
    return this;
  }

  getTool(id: string): ToolExtension<TDoc, TView> | undefined {
    return this.tools.get(id);
  }

  listTools(): ToolExtension<TDoc, TView>[] {
    return [...this.tools.values()];
  }

  listPolicies(): PolicyExtension[] {
    return [...this.policies.values()];
  }

  listRenderers(): RendererExtension<TView, TDoc>[] {
    return [...this.renderers.values()];
  }

  listAssetProviders(): AssetProviderExtension[] {
    return [...this.assetProviders.values()];
  }

  /** Applies registered policy extensions (by id, or all in registration order) to a base policy. */
  applyPolicies(base: InteractionPolicy, ids?: readonly string[]): InteractionPolicy {
    const list = ids
      ? ids.flatMap((id) => {
          const p = this.policies.get(id);
          return p ? [p] : [];
        })
      : [...this.policies.values()];
    return list.reduce((acc, ext) => ext.apply(acc), base);
  }
}
