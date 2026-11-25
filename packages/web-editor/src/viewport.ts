// The 3D view. three.js only draws what the engine holds: a surface with vertex colours, or
// points when the file has no faces, plus placed objects, see-through build/carve ghosts,
// an optional architectural floor grid, wireframe overlay, and room-plan drawing preview.
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';

export type Vec3 = [number, number, number];

export interface ModelBuffers {
  positions: Float32Array;
  colors: Uint8Array;
  indices: Uint32Array;
  normals: Float32Array;
}

export interface CameraView { position: Vec3; target: Vec3 }

export type CameraPreset = 'iso' | 'top' | 'front' | 'side' | 'eye';

/** A placed instance: which object it shows, and its object-to-world matrix (column-major, from the engine). */
export interface InstanceView { id: string; object: string; matrix: ArrayLike<number> }

/** What build mode would place: built geometry in world space, or an object's geometry under a matrix. */
export type Ghost = { buffers: Pick<ModelBuffers, 'positions' | 'indices'>; matrix?: undefined } | { object: string; matrix: ArrayLike<number> };

export interface GridOptions {
  visible: boolean;
  y?: number;
  cell?: number;
  extent?: number;
}

export interface OverlayCloudData {
  positions: Float32Array;
  colors: Uint8Array;
}

export interface OverlayCloudStyle {
  visible?: boolean;
  opacity?: number;
  pointSize?: number;
}

export interface PhotoCameraSpec {
  id: string;
  width: number;
  height: number;
  fov_deg: number;
  eye: Vec3;
  forward: Vec3;
  up: Vec3;
}

/** A theme colour (an `H S% L%` CSS variable), so the view follows light and dark. */
function themeColor(name: string, fallback: number): THREE.Color {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const m = raw.match(/^([\d.]+)\s+([\d.]+)%\s+([\d.]+)%$/);
  return m ? new THREE.Color().setHSL(Number(m[1]) / 360, Number(m[2]) / 100, Number(m[3]) / 100, THREE.SRGBColorSpace) : new THREE.Color(fallback);
}

const buffersGeometry = (m: ModelBuffers) => {
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(m.positions, 3));
  g.setAttribute('color', new THREE.BufferAttribute(m.colors, 3, true));
  g.setAttribute('normal', new THREE.BufferAttribute(m.normals, 3));
  g.setIndex(new THREE.BufferAttribute(m.indices, 1));
  g.computeBoundingBox();
  g.computeBoundingSphere();
  return g;
};

/** Paint splats as instanced quads with a Gaussian falloff, matching the engine's own software renderer. */
export class SplatCloud {
  readonly object: THREE.Mesh;
  private readonly geometry = new THREE.InstancedBufferGeometry();
  private readonly material: THREE.ShaderMaterial;

  constructor() {
    const g = this.geometry;
    g.setAttribute('position', new THREE.BufferAttribute(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3));
    g.setIndex([0, 1, 2, 0, 2, 3]);
    g.instanceCount = 0;
    this.material = new THREE.ShaderMaterial({
      transparent: true,
      depthWrite: false,
      side: THREE.DoubleSide,
      vertexShader: /* glsl */ `
        attribute vec3 iPos;
        attribute float iRadius;
        attribute vec3 iNormal;
        attribute float iDrip;
        attribute vec4 iColor;
        varying vec2 vQ;
        varying vec4 vColor;
        varying vec3 vN;
        void main() {
          vec3 n = normalize(iNormal);
          vec3 axis = abs(n.x) < 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
          vec3 t1 = normalize(cross(n, axis));
          vec3 t2 = normalize(cross(n, t1));
          float r = iRadius * 2.5;
          vec2 q = position.xy;
          vec3 off = t1 * q.x * r + t2 * q.y * r;
          if (iDrip > 0.001) {
            vec3 g = vec3(0.0, -1.0, 0.0);
            vec3 gd = g - n * dot(g, n);
            if (dot(gd, gd) > 1e-6) {
              gd = normalize(gd);
              float t = dot(gd, t1) * q.x + dot(gd, t2) * q.y;
              off += gd * iDrip * r * max(t + 1.0, 0.0);
            }
          }
          vQ = q;
          vColor = iColor;
          vN = n;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(iPos + off, 1.0);
        }`,
      fragmentShader: /* glsl */ `
        varying vec2 vQ;
        varying vec4 vColor;
        varying vec3 vN;
        void main() {
          float m = dot(vQ, vQ) * 6.25;
          if (m > 6.25) discard;
          float a = vColor.a * exp(-0.5 * m);
          vec3 lit = vColor.rgb * (0.65 + 0.35 * max(vN.y, 0.0));
          gl_FragColor = vec4(lit, a);
        }`,
    });
    this.object = new THREE.Mesh(g, this.material);
    this.object.frustumCulled = false;
  }

  /** geo: 8 floats per splat (x, y, z, radius, nx, ny, nz, drip); colors: 4 bytes per splat (r, g, b, opacity). */
  setData(geo: Float32Array, colors: Uint8Array) {
    const n = geo.length / 8;
    const g = this.geometry;
    const pos = new Float32Array(n * 3);
    const radius = new Float32Array(n);
    const normal = new Float32Array(n * 3);
    const drip = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      pos.set(geo.subarray(i * 8, i * 8 + 3), i * 3);
      radius[i] = geo[i * 8 + 3];
      normal.set(geo.subarray(i * 8 + 4, i * 8 + 7), i * 3);
      drip[i] = geo[i * 8 + 7];
    }
    const rgba = new Float32Array(n * 4);
    for (let i = 0; i < n * 4; i++) rgba[i] = colors[i] / 255;
    g.setAttribute('iPos', new THREE.InstancedBufferAttribute(pos, 3));
    g.setAttribute('iRadius', new THREE.InstancedBufferAttribute(radius, 1));
    g.setAttribute('iNormal', new THREE.InstancedBufferAttribute(normal, 3));
    g.setAttribute('iDrip', new THREE.InstancedBufferAttribute(drip, 1));
    g.setAttribute('iColor', new THREE.InstancedBufferAttribute(rgba, 4));
    g.instanceCount = n;
  }

  dispose() {
    this.geometry.dispose();
    this.material.dispose();
  }
}

interface CameraAnimation {
  startPos: THREE.Vector3;
  startTarget: THREE.Vector3;
  endPos: THREE.Vector3;
  endTarget: THREE.Vector3;
  startedAt: number;
  duration: number;
}

export class Viewport {
  readonly controls: OrbitControls;
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(48, 1, 0.02, 400);
  private readonly surface = new THREE.MeshStandardMaterial({ vertexColors: true, side: THREE.DoubleSide, roughness: 0.78, metalness: 0.04 });
  private readonly wireframeMaterial = new THREE.MeshBasicMaterial({ color: 0x0f172a, wireframe: true, transparent: true, opacity: 0.16, depthWrite: false });
  private readonly dots = new THREE.PointsMaterial({ vertexColors: true, sizeAttenuation: true });
  private readonly geometry = new THREE.BufferGeometry();
  private object: THREE.Mesh | THREE.Points | null = null;
  private readonly splatCloud = new SplatCloud();
  private wireframeMesh: THREE.Mesh | null = null;
  private wireframeEnabled = false;
  private readonly cursor: THREE.Mesh;
  private readonly resizer: ResizeObserver;
  private home: CameraView | null = null;
  private dotSize = 0.01;
  private hasGeometry = false;
  // Placed objects: geometry shared by every instance of an object.
  private readonly objects = new Map<string, THREE.BufferGeometry>();
  private readonly instances = new THREE.Group();
  private instanceList: InstanceView[] = [];
  private readonly ghostMaterial = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.42, depthWrite: false, side: THREE.DoubleSide });
  private readonly ghost = new THREE.Mesh(new THREE.BufferGeometry(), this.ghostMaterial);
  private ghostOwned: THREE.BufferGeometry | null = null; // built-piece geometry the ghost made, disposed on change
  private ghostTone: 'build' | 'carve' = 'build';
  // Outlines around selected instances: one box each, plus a dashed box around the whole group when several are selected.
  private readonly selection = new THREE.Group();
  private readonly selectionMaterial = new THREE.LineBasicMaterial({ depthTest: false, transparent: true });
  private readonly groupMaterial = new THREE.LineDashedMaterial({ depthTest: false, transparent: true, opacity: 0.7, dashSize: 0.08, gapSize: 0.06 });
  private selected: string[] = [];
  // Rotate rings around the selection: red turns about x, green about y (yaw), blue about z. Each ring has a thick
  // invisible twin that pointer picks hit, so the thin visible ring is easy to grab.
  private readonly gizmo = new THREE.Group();
  private readonly gizmoRings: Record<'x' | 'y' | 'z', { ring: THREE.Mesh; hit: THREE.Mesh; material: THREE.MeshBasicMaterial }>;
  private gizmoEnabled = false;
  private gizmoCenter: Vec3 | null = null;
  // Studio floor grid (off by default for scans, enabled on Canvas).
  private readonly gridGroup = new THREE.Group();
  private gridOptions: Required<GridOptions> = { visible: false, y: 0, cell: 0.5, extent: 20 };
  // Floorplan room-drawing overlay.
  private readonly planGroup = new THREE.Group();
  private readonly planLineMaterial = new THREE.LineBasicMaterial({ color: 0x007a52, depthTest: false });
  private readonly planDotMaterial = new THREE.MeshBasicMaterial({ color: 0x007a52, depthTest: false });
  private readonly planFillMaterial = new THREE.MeshBasicMaterial({ color: 0x007a52, transparent: true, opacity: 0.18, side: THREE.DoubleSide, depthWrite: false });
  private cameraAnim: CameraAnimation | null = null;
  private modelVisible = true;
  // Studio overlay layers: LiDAR point clouds and photogrammetry camera frustums.
  private readonly overlayCloudGroup = new THREE.Group();
  private readonly overlayClouds = new Map<string, { points: THREE.Points; geometry: THREE.BufferGeometry; material: THREE.PointsMaterial }>();
  private readonly photoCameraGroup = new THREE.Group();
  private activePhoto: PhotoCameraSpec | null = null;
  private savedFreeView: { view: CameraView; fov: number } | null = null;
  private onPhotoExit: (() => void) | null = null;
  private readonly defaultFov = 48;

  constructor(private readonly host: HTMLElement) {
    // Keep the normal viewport on the fast path. Agent screenshots use an on-demand render target below rather than
    // keeping the browser's drawing buffer alive on every animation frame.
    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance', preserveDrawingBuffer: false });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    const canvas = this.renderer.domElement;
    canvas.style.display = 'block';
    canvas.style.touchAction = 'none';
    canvas.style.width = '100%';
    canvas.style.height = '100%';
    host.appendChild(canvas);

    // Three-point studio lighting so interior surfaces and sculpted forms read clearly.
    this.scene.add(new THREE.HemisphereLight(0xfcfcff, 0x8b8e99, 1.35));
    const key = new THREE.DirectionalLight(0xfffaf2, 1.45);
    key.position.set(6, 11, 5);
    this.scene.add(key);
    const fill = new THREE.DirectionalLight(0xdbeafe, 0.55);
    fill.position.set(-7, 5, -4);
    this.scene.add(fill);
    const rim = new THREE.DirectionalLight(0xffffff, 0.35);
    rim.position.set(0, 4, -9);
    this.scene.add(rim);

    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.screenSpacePanning = true;
    this.controls.autoRotateSpeed = 0.9;
    this.controls.addEventListener('start', () => {
      this.cameraAnim = null;
      if (this.activePhoto) {
        const cb = this.onPhotoExit;
        this.activePhoto = null;
        this.onPhotoExit = null;
        this.camera.up.set(0, 1, 0);
        this.camera.fov = this.defaultFov;
        this.camera.updateProjectionMatrix();
        cb?.();
      }
    });

    this.cursor = new THREE.Mesh(
      new THREE.RingGeometry(0.88, 1, 48),
      new THREE.MeshBasicMaterial({ color: 0xff6a00, side: THREE.DoubleSide, depthTest: false, transparent: true, opacity: 0.92 }),
    );
    this.cursor.renderOrder = 4;
    this.cursor.visible = false;
    this.scene.add(this.cursor);
    this.scene.add(this.splatCloud.object);
    this.scene.add(this.gridGroup);
    this.gridGroup.visible = false;
    this.scene.add(this.planGroup);
    this.planGroup.renderOrder = 3;
    this.scene.add(this.instances);
    this.ghost.matrixAutoUpdate = false;
    this.ghost.visible = false;
    this.ghost.renderOrder = 2;
    this.scene.add(this.ghost);
    this.selection.renderOrder = 3;
    const ringGeometry = new THREE.TorusGeometry(1, 0.012, 8, 96);
    const hitGeometry = new THREE.TorusGeometry(1, 0.07, 6, 48);
    const hitMaterial = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, depthWrite: false, depthTest: false, colorWrite: false });
    const ring = (axis: 'x' | 'y' | 'z', color: number) => {
      const material = new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.85, depthTest: false, depthWrite: false });
      const visible = new THREE.Mesh(ringGeometry, material);
      const hit = new THREE.Mesh(hitGeometry, hitMaterial);
      hit.userData.axis = axis;
      // A torus lies in the xy plane (around z): turn it to lie square to its axis.
      for (const m of [visible, hit]) {
        if (axis === 'x') m.rotation.y = Math.PI / 2;
        if (axis === 'y') m.rotation.x = Math.PI / 2;
        m.renderOrder = 6;
        this.gizmo.add(m);
      }
      return { ring: visible, hit, material };
    };
    this.gizmoRings = { x: ring('x', 0xe5484d), y: ring('y', 0x30a46c), z: ring('z', 0x3e63dd) };
    this.gizmo.visible = false;
    this.scene.add(this.gizmo);
    this.scene.add(this.selection);
    this.scene.add(this.overlayCloudGroup);
    this.scene.add(this.photoCameraGroup);

    this.refreshTheme();

    this.resizer = new ResizeObserver(() => this.resize());
    this.resizer.observe(host);
    this.resize();
    this.renderer.setAnimationLoop(() => {
      this.stepCameraAnimation();
      this.controls.update();
      this.renderer.render(this.scene, this.camera);
    });
  }

  get canvas(): HTMLCanvasElement {
    return this.renderer.domElement;
  }

  /** Captures the current 3D viewport for the multimodal studio agent. */
  captureDataUrl(): string | null {
    const width = this.renderer.domElement.width;
    const height = this.renderer.domElement.height;
    if (!width || !height) return null;
    const target = new THREE.WebGLRenderTarget(width, height, { depthBuffer: true, stencilBuffer: false });
    const pixels = new Uint8Array(width * height * 4);
    const output = document.createElement('canvas');
    output.width = width;
    output.height = height;
    const context = output.getContext('2d');
    if (!context) { target.dispose(); return null; }
    try {
      this.renderer.setRenderTarget(target);
      this.renderer.render(this.scene, this.camera);
      this.renderer.readRenderTargetPixels(target, 0, 0, width, height, pixels);
      this.renderer.setRenderTarget(null);
      const image = context.createImageData(width, height);
      const rowBytes = width * 4;
      for (let y = 0; y < height; y++) {
        image.data.set(pixels.subarray((height - y - 1) * rowBytes, (height - y) * rowBytes), y * rowBytes);
      }
      context.putImageData(image, 0, 0);
      return output.toDataURL('image/jpeg', 0.82);
    } finally {
      this.renderer.setRenderTarget(null);
      target.dispose();
    }
  }

  /** Left drag orbits (viewing), or is left to a tool (editing, where the right button orbits). */
  setLeftButtonFree(free: boolean) {
    this.controls.mouseButtons = { LEFT: free ? null : THREE.MOUSE.ROTATE, MIDDLE: THREE.MOUSE.PAN, RIGHT: free ? THREE.MOUSE.ROTATE : THREE.MOUSE.PAN };
    this.controls.touches = { ONE: free ? null : THREE.TOUCH.ROTATE, TWO: THREE.TOUCH.DOLLY_PAN } as OrbitControls['touches'];
  }

  /** Shows the model. The first call frames it; later calls (edits) keep the camera. */
  setModel(m: ModelBuffers) {
    const g = this.geometry;
    g.setAttribute('position', new THREE.BufferAttribute(m.positions, 3));
    g.setAttribute('color', new THREE.BufferAttribute(m.colors, 3, true));
    const points = m.indices.length === 0;
    this.hasGeometry = m.positions.length > 0;
    if (points) {
      g.deleteAttribute('normal');
      g.setIndex(null);
    } else {
      g.setAttribute('normal', new THREE.BufferAttribute(m.normals, 3));
      g.setIndex(new THREE.BufferAttribute(m.indices, 1));
    }
    g.computeBoundingBox();
    g.computeBoundingSphere();
    if (!this.object || (this.object instanceof THREE.Points) !== points) {
      if (this.object) this.scene.remove(this.object);
      this.object = points ? new THREE.Points(g, this.dots) : new THREE.Mesh(g, this.surface);
      this.object.visible = this.modelVisible;
      this.scene.add(this.object);
    }
    if (!points && this.wireframeEnabled && !this.wireframeMesh) {
      this.wireframeMesh = new THREE.Mesh(g, this.wireframeMaterial);
      this.wireframeMesh.renderOrder = 1;
      this.scene.add(this.wireframeMesh);
    } else if (this.wireframeMesh) {
      this.wireframeMesh.visible = this.modelVisible && this.wireframeEnabled && !points && this.hasGeometry;
    }
    if (!this.home) this.frame();
  }

  /** Shows or hides the editable mesh layer (while keeping LiDAR / photo overlays visible). */
  setModelVisible(visible: boolean) {
    this.modelVisible = visible;
    if (this.object) this.object.visible = visible;
    this.splatCloud.object.visible = visible;
    if (this.wireframeMesh) this.wireframeMesh.visible = visible && this.wireframeEnabled && this.hasGeometry;
  }

  /** Adds, replaces, or removes a LiDAR point cloud overlay layer in the same 3D space as the mesh. */
  setOverlayCloud(id: string, cloud: OverlayCloudData | null, style: OverlayCloudStyle = {}) {
    const existing = this.overlayClouds.get(id);
    if (existing) {
      this.overlayCloudGroup.remove(existing.points);
      existing.geometry.dispose();
      existing.material.dispose();
      this.overlayClouds.delete(id);
    }
    if (!cloud || cloud.positions.length === 0) return;
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(cloud.positions, 3));
    g.setAttribute('color', new THREE.BufferAttribute(cloud.colors, 3, true));
    g.computeBoundingBox();
    g.computeBoundingSphere();
    const opacity = style.opacity ?? 0.9;
    const material = new THREE.PointsMaterial({
      vertexColors: true,
      sizeAttenuation: true,
      size: Math.max(this.dotSize, 0.012) * (style.pointSize ?? 1.2),
      transparent: opacity < 0.99,
      opacity,
      depthWrite: opacity >= 0.99,
    });
    const points = new THREE.Points(g, material);
    points.visible = style.visible ?? true;
    this.overlayCloudGroup.add(points);
    this.overlayClouds.set(id, { points, geometry: g, material });
    // If the editable mesh is empty, reframe onto the newly overlaid LiDAR cloud.
    if (!this.hasGeometry && points.visible) this.frame();
  }

  /** Updates visibility, opacity, or point size for an existing LiDAR overlay layer. */
  setOverlayCloudStyle(id: string, style: OverlayCloudStyle) {
    const entry = this.overlayClouds.get(id);
    if (!entry) return;
    if (style.visible !== undefined) entry.points.visible = style.visible;
    if (style.opacity !== undefined) {
      entry.material.opacity = style.opacity;
      entry.material.transparent = style.opacity < 0.99;
      entry.material.depthWrite = style.opacity >= 0.99;
      entry.material.needsUpdate = true;
    }
    if (style.pointSize !== undefined) {
      entry.material.size = Math.max(this.dotSize, 0.012) * style.pointSize;
    }
  }

  /** Draws 3D camera frustums for posed photogrammetry photos in Brush Y-up OpenGL space. */
  setPhotoCameras(photos: readonly PhotoCameraSpec[], visible: boolean, activeId: string | null) {
    this.clearGroup(this.photoCameraGroup);
    this.photoCameraGroup.visible = visible && photos.length > 0;
    if (!visible || photos.length === 0) return;
    const accent = themeColor('--primary', 0xd97706);
    const muted = themeColor('--muted-foreground', 0x64748b);
    const frustumDepth = 0.22;
    for (const photo of photos) {
      const eye = new THREE.Vector3(...photo.eye);
      const fwd = new THREE.Vector3(...photo.forward).normalize();
      const upRaw = new THREE.Vector3(...photo.up).normalize();
      const right = new THREE.Vector3().crossVectors(fwd, upRaw);
      if (right.lengthSq() < 1e-6) continue;
      right.normalize();
      const up = new THREE.Vector3().crossVectors(right, fwd).normalize();

      const halfH = Math.tan(THREE.MathUtils.degToRad(photo.fov_deg) / 2) * frustumDepth;
      const aspect = photo.height > 0 ? photo.width / photo.height : 4 / 3;
      const halfW = halfH * aspect;
      const center = eye.clone().addScaledVector(fwd, frustumDepth);
      const tl = center.clone().addScaledVector(up, halfH).addScaledVector(right, -halfW);
      const tr = center.clone().addScaledVector(up, halfH).addScaledVector(right, halfW);
      const br = center.clone().addScaledVector(up, -halfH).addScaledVector(right, halfW);
      const bl = center.clone().addScaledVector(up, -halfH).addScaledVector(right, -halfW);
      const upTip = center.clone().addScaledVector(up, halfH * 1.35);

      const verts = [
        eye, tl, eye, tr, eye, br, eye, bl,
        tl, tr, tr, br, br, bl, bl, tl,
        tl, upTip, tr, upTip,
      ];
      const geo = new THREE.BufferGeometry().setFromPoints(verts);
      const isActive = photo.id === activeId;
      const mat = new THREE.LineBasicMaterial({
        color: isActive ? accent : muted,
        transparent: true,
        opacity: isActive ? 1 : 0.65,
      });
      this.photoCameraGroup.add(new THREE.LineSegments(geo, mat));
    }
  }

  /** Computes the Three.js vertical FOV so an object-contain photo overlay aligns 1:1 with the 3D view. */
  private applyPhotoFov(photo: PhotoCameraSpec) {
    const photoAspect = photo.height > 0 ? photo.width / photo.height : 4 / 3;
    const canvasAspect = this.camera.aspect || 1;
    let vFovRad = THREE.MathUtils.degToRad(photo.fov_deg);
    if (canvasAspect < photoAspect) {
      // Narrower viewport: photo fits width-wise, so vertical span grows proportionally.
      vFovRad = 2 * Math.atan(Math.tan(vFovRad / 2) * (photoAspect / canvasAspect));
    }
    this.camera.fov = THREE.MathUtils.radToDeg(vFovRad);
    this.camera.updateProjectionMatrix();
  }

  /** Aligns the 3D camera to a posed capture photo (like Blender's "Look through"). */
  lookThroughPhoto(photo: PhotoCameraSpec, onExit?: () => void) {
    if (!this.activePhoto) {
      this.savedFreeView = { view: this.getView(), fov: this.defaultFov };
    }
    this.cameraAnim = null;
    this.activePhoto = photo;
    this.onPhotoExit = onExit ?? null;
    const eye = new THREE.Vector3(...photo.eye);
    const fwd = new THREE.Vector3(...photo.forward).normalize();
    const upRaw = new THREE.Vector3(...photo.up).normalize();
    const right = new THREE.Vector3().crossVectors(fwd, upRaw);
    const up = right.lengthSq() > 1e-6 ? new THREE.Vector3().crossVectors(right.normalize(), fwd).normalize() : new THREE.Vector3(0, 1, 0);
    this.camera.position.copy(eye);
    this.camera.up.copy(up);
    this.controls.target.copy(eye).addScaledVector(fwd, 1.5);
    this.applyPhotoFov(photo);
    this.controls.update();
  }

  /** Leaves photo camera view and restores the previous free-orbit camera. */
  exitPhotoView(restoreView = true) {
    this.activePhoto = null;
    this.onPhotoExit = null;
    this.camera.up.set(0, 1, 0);
    this.camera.fov = this.savedFreeView?.fov ?? this.defaultFov;
    this.camera.updateProjectionMatrix();
    if (restoreView && this.savedFreeView) {
      this.setView(this.savedFreeView.view);
    }
    this.savedFreeView = null;
  }

  /** The model's paint splats: 8 floats and 4 bytes each, straight from the engine. */
  setSplats(geo: Float32Array, colors: Uint8Array) {
    this.splatCloud.setData(geo, colors);
  }

  /** Points the camera at the whole model (or visible LiDAR overlays, or a cozy 6m room studio when empty) and makes that the home view. */
  frame() {
    this.cameraAnim = null;
    let sphere: THREE.Sphere | null = this.hasGeometry ? this.geometry.boundingSphere : null;
    if (!sphere || sphere.radius < 1e-4) {
      const box = new THREE.Box3();
      for (const entry of this.overlayClouds.values()) {
        if (entry.points.visible && entry.geometry.boundingBox) box.union(entry.geometry.boundingBox);
      }
      if (!box.isEmpty()) {
        sphere = new THREE.Sphere();
        box.getBoundingSphere(sphere);
      }
    }
    if (!sphere || sphere.radius < 1e-4) {
      const target: Vec3 = [0, 0.75, 0];
      const position: Vec3 = [5.8, 4.9, 6.8];
      this.home = { position, target };
      this.camera.near = 0.02;
      this.camera.far = 400;
      this.camera.updateProjectionMatrix();
      this.controls.minDistance = 0.25;
      this.controls.maxDistance = 85;
      this.dotSize = 0.015;
      this.setPointSize(1);
      this.setView(this.home);
      return;
    }
    const radius = Math.max(sphere.radius, 0.5);
    const distance = radius / Math.sin((this.camera.fov * Math.PI) / 360);
    const position = sphere.center.clone().add(new THREE.Vector3(0.58, 0.48, 0.68).normalize().multiplyScalar(distance * 0.95));
    this.home = { position: position.toArray() as Vec3, target: sphere.center.toArray() as Vec3 };
    this.camera.near = Math.max(0.005, radius / 1000);
    this.camera.far = Math.max(250, radius * 120);
    this.camera.updateProjectionMatrix();
    this.controls.minDistance = Math.max(0.05, radius / 200);
    this.controls.maxDistance = Math.max(50, radius * 14);
    this.dotSize = radius / 450;
    this.setPointSize(1);
    this.setView(this.home);
  }

  /** Smoothly animates the camera to a named architectural preset. */
  setCameraPreset(preset: CameraPreset, floorY = 0) {
    const sphere = this.hasGeometry ? this.geometry.boundingSphere : null;
    const center = sphere && sphere.radius > 1e-4 ? sphere.center.clone() : new THREE.Vector3(0, floorY + 0.8, 0);
    const radius = sphere && sphere.radius > 1e-4 ? Math.max(sphere.radius, 2.2) : 4.2;
    const dist = Math.max(radius * 2.1, 4.8);
    let endTarget = center.clone();
    let endPos = center.clone();

    if (preset === 'iso') {
      endPos = center.clone().add(new THREE.Vector3(0.62, 0.54, 0.68).normalize().multiplyScalar(dist));
    } else if (preset === 'top') {
      endTarget = new THREE.Vector3(center.x, floorY, center.z);
      // Slight +z offset prevents OrbitControls gimbal lock at straight vertical polar angle.
      endPos = new THREE.Vector3(center.x, floorY + dist * 1.15, center.z + 0.08);
    } else if (preset === 'front') {
      endPos = new THREE.Vector3(center.x, Math.max(center.y, floorY + 1.4), center.z + dist);
    } else if (preset === 'side') {
      endPos = new THREE.Vector3(center.x + dist, Math.max(center.y, floorY + 1.4), center.z);
    } else if (preset === 'eye') {
      const eyeY = floorY + 1.62;
      endTarget = new THREE.Vector3(center.x, eyeY - 0.08, center.z);
      endPos = new THREE.Vector3(center.x, eyeY, center.z + Math.min(Math.max(radius * 0.9, 2.6), 5.5));
    }

    this.cameraAnim = {
      startPos: this.camera.position.clone(),
      startTarget: this.controls.target.clone(),
      endPos,
      endTarget,
      startedAt: performance.now(),
      duration: 280,
    };
  }

  /** Moves both camera and target along the camera's ground-projected axes (Minecraft/Fortnite WASD glide). */
  panLocal(forward: number, right: number, up = 0) {
    this.cameraAnim = null;
    const fwd = new THREE.Vector3();
    this.camera.getWorldDirection(fwd);
    fwd.y = 0;
    if (fwd.lengthSq() < 1e-6) fwd.set(0, 0, -1);
    else fwd.normalize();
    const side = new THREE.Vector3().crossVectors(fwd, new THREE.Vector3(0, 1, 0)).normalize();
    const delta = new THREE.Vector3()
      .addScaledVector(fwd, forward)
      .addScaledVector(side, right)
      .add(new THREE.Vector3(0, up, 0));
    this.camera.position.add(delta);
    this.controls.target.add(delta);
    this.controls.update();
  }

  private stepCameraAnimation() {
    const anim = this.cameraAnim;
    if (!anim) return;
    const raw = Math.min(1, (performance.now() - anim.startedAt) / anim.duration);
    const t = 1 - Math.pow(1 - raw, 3);
    this.camera.position.lerpVectors(anim.startPos, anim.endPos, t);
    this.controls.target.lerpVectors(anim.startTarget, anim.endTarget, t);
    if (raw >= 1) this.cameraAnim = null;
  }

  resetCamera() {
    if (this.home) this.setView(this.home);
  }

  getView(): CameraView {
    return { position: this.camera.position.toArray() as Vec3, target: this.controls.target.toArray() as Vec3 };
  }

  setView(view: CameraView) {
    this.cameraAnim = null;
    this.camera.position.set(...view.position);
    this.controls.target.set(...view.target);
    this.controls.update();
  }

  /** Point size as a multiple of the default, for point clouds. */
  setPointSize(scale: number) {
    this.dots.size = this.dotSize * scale;
  }

  setAutoRotate(on: boolean) {
    this.controls.autoRotate = on;
  }

  /** Toggles flat-shaded vs smooth-shaded surface rendering. */
  setFlatShading(flat: boolean) {
    this.surface.flatShading = flat;
    this.surface.needsUpdate = true;
  }

  /** Toggles a subtle wireframe overlay on top of the shaded surface mesh. */
  setWireframe(on: boolean) {
    this.wireframeEnabled = on;
    if (on && !this.wireframeMesh) {
      this.wireframeMesh = new THREE.Mesh(this.geometry, this.wireframeMaterial);
      this.wireframeMesh.renderOrder = 1;
      this.scene.add(this.wireframeMesh);
    }
    if (this.wireframeMesh) {
      this.wireframeMesh.visible = on && this.hasGeometry && !(this.object instanceof THREE.Points);
    }
  }

  /** Shows or updates the architectural floor grid at the active story elevation. */
  setGrid(options: GridOptions) {
    this.gridOptions = {
      visible: options.visible,
      y: options.y ?? this.gridOptions.y,
      cell: options.cell ?? this.gridOptions.cell,
      extent: options.extent ?? this.gridOptions.extent,
    };
    this.gridGroup.visible = this.gridOptions.visible;
    this.rebuildGrid();
  }

  private rebuildGrid() {
    this.clearGroup(this.gridGroup);
    if (!this.gridOptions.visible) return;
    const { y, cell, extent } = this.gridOptions;
    const dark = document.documentElement.classList.contains('dark');
    const majorColor = dark ? 0x3b4258 : 0xb8c0d4;
    const minorColor = dark ? 0x222738 : 0xdbe0ec;
    const axisXColor = dark ? 0xef4444 : 0xdc2626;
    const axisZColor = dark ? 0x3b82f6 : 0x2563eb;

    const half = Math.max(4, Math.round(extent / 2));
    const step = Math.max(0.1, cell);
    const minorPts: number[] = [];
    const majorPts: number[] = [];
    const count = Math.round(half / step);
    const span = count * step;
    for (let i = -count; i <= count; i++) {
      const coord = Math.round(i * step * 1000) / 1000;
      if (Math.abs(coord) < 1e-4) continue;
      const isMajor = Math.abs(coord - Math.round(coord)) < 1e-3;
      const target = isMajor ? majorPts : minorPts;
      target.push(coord, 0, -span, coord, 0, span);
      target.push(-span, 0, coord, span, 0, coord);
    }

    if (minorPts.length) {
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(minorPts, 3));
      this.gridGroup.add(new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ color: minorColor, transparent: true, opacity: dark ? 0.55 : 0.7 })));
    }
    if (majorPts.length) {
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(majorPts, 3));
      this.gridGroup.add(new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ color: majorColor, transparent: true, opacity: dark ? 0.72 : 0.85 })));
    }
    // Subtle origin X and Z axis guides so orientation is always intuitive.
    const xGeo = new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(-span, 0.002, 0), new THREE.Vector3(span, 0.002, 0)]);
    const zGeo = new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(0, 0.002, -span), new THREE.Vector3(0, 0.002, span)]);
    this.gridGroup.add(new THREE.Line(xGeo, new THREE.LineBasicMaterial({ color: axisXColor, transparent: true, opacity: 0.4 })));
    this.gridGroup.add(new THREE.Line(zGeo, new THREE.LineBasicMaterial({ color: axisZColor, transparent: true, opacity: 0.4 })));
    this.gridGroup.position.set(0, y - 0.002, 0);
  }

  /** Draws a glowing room floorplan outline while the user clicks out polygon corners. */
  setPlanPreview(points: Vec3[], cursor: Vec3 | null) {
    this.clearGroup(this.planGroup);
    const list = cursor ? [...points, cursor] : [...points];
    if (!list.length) return;
    const lift = 0.025;
    const vecs = list.map((p) => new THREE.Vector3(p[0], p[1] + lift, p[2]));
    if (vecs.length >= 2) {
      const loop = points.length >= 3 ? [...vecs, vecs[0]] : vecs;
      const lineGeo = new THREE.BufferGeometry().setFromPoints(loop);
      const line = new THREE.Line(lineGeo, this.planLineMaterial);
      line.renderOrder = 4;
      this.planGroup.add(line);
    }
    // Corner dots so clicking the start corner to close feels tactile.
    for (let i = 0; i < points.length; i++) {
      const dot = new THREE.Mesh(new THREE.SphereGeometry(i === 0 ? 0.09 : 0.065, 16, 12), this.planDotMaterial);
      dot.position.set(points[i][0], points[i][1] + lift, points[i][2]);
      dot.renderOrder = 5;
      this.planGroup.add(dot);
    }
    if (list.length >= 3) {
      const shape = new THREE.Shape(list.map((p) => new THREE.Vector2(p[0], -p[2])));
      const geo = new THREE.ShapeGeometry(shape);
      geo.rotateX(-Math.PI / 2);
      const fill = new THREE.Mesh(geo, this.planFillMaterial);
      fill.position.y = list[0][1] + 0.012;
      fill.renderOrder = 2;
      this.planGroup.add(fill);
    }
  }

  private clearGroup(group: THREE.Group) {
    while (group.children.length) {
      const child = group.children[0];
      group.remove(child);
      if ('geometry' in child && child.geometry instanceof THREE.BufferGeometry) child.geometry.dispose();
      if ('material' in child && child.material instanceof THREE.Material && child.material !== this.planLineMaterial && child.material !== this.planDotMaterial && child.material !== this.planFillMaterial) {
        child.material.dispose();
      }
    }
  }

  /** Sets whether the placement preview represents adding geometry or carving a cutout. */
  setGhostTone(tone: 'build' | 'carve') {
    this.ghostTone = tone;
    const primary = themeColor('--primary', 0x007a52);
    const destructive = themeColor('--destructive', 0xdc2626);
    this.ghostMaterial.color.copy(tone === 'carve' ? destructive : primary);
    this.ghostMaterial.opacity = tone === 'carve' ? 0.45 : 0.38;
  }

  refreshTheme() {
    this.scene.background = themeColor('--background', 0xeef0f7);
    const primary = themeColor('--primary', 0x007a52);
    const fg = themeColor('--foreground', 0x111827);
    this.setGhostTone(this.ghostTone);
    this.selectionMaterial.color.copy(primary);
    this.groupMaterial.color.copy(primary);
    this.planLineMaterial.color.copy(primary);
    this.planDotMaterial.color.copy(primary);
    this.planFillMaterial.color.copy(primary);
    this.wireframeMaterial.color.copy(fg);
    this.rebuildGrid();
  }

  /** Renders synchronously and captures a crisp PNG snapshot of the current viewport. */
  captureScreenshot(): string {
    const prevGrid = this.gridGroup.visible;
    const prevGhost = this.ghost.visible;
    const prevCursor = this.cursor.visible;
    this.ghost.visible = false;
    this.cursor.visible = false;
    this.renderer.render(this.scene, this.camera);
    const dataUrl = this.renderer.domElement.toDataURL('image/png');
    this.gridGroup.visible = prevGrid;
    this.ghost.visible = prevGhost;
    this.cursor.visible = prevCursor;
    return dataUrl;
  }

  /** Supplies or refreshes an object's geometry: every instance of it shows the new one. */
  setObject(key: string, m: ModelBuffers) {
    const old = this.objects.get(key);
    this.objects.set(key, buffersGeometry(m));
    this.setInstances(this.instanceList);
    old?.dispose();
  }

  hasObject(key: string) {
    return this.objects.has(key);
  }

  /** Shows these placed instances (objects not supplied yet are skipped until they are). */
  setInstances(list: InstanceView[]) {
    this.instanceList = list;
    this.instances.clear();
    for (const i of list) {
      const g = this.objects.get(i.object);
      if (!g) continue;
      const mesh = new THREE.Mesh(g, this.surface);
      mesh.matrixAutoUpdate = false;
      mesh.matrix.fromArray(i.matrix);
      mesh.name = i.id;
      this.instances.add(mesh);
    }
    this.instances.updateMatrixWorld(true);
    this.showSelection(this.selected);
  }

  /** Outlines placed instances (one, several, or none). Several also get a dashed box around the whole group. */
  showSelection(ids: string | readonly string[] | null) {
    this.selected = ids == null ? [] : typeof ids === 'string' ? [ids] : [...ids];
    for (const child of [...this.selection.children]) {
      (child as THREE.LineSegments).geometry.dispose();
      this.selection.remove(child);
    }
    const all = new THREE.Box3();
    let count = 0;
    for (const id of this.selected) {
      const mesh = this.instances.getObjectByName(id);
      if (!mesh) continue;
      const helper = new THREE.BoxHelper(mesh);
      (helper.material as THREE.Material).dispose();
      helper.material = this.selectionMaterial;
      helper.renderOrder = 3;
      this.selection.add(helper);
      all.expandByObject(mesh);
      count++;
    }
    if (count > 0 && !all.isEmpty()) {
      const center = all.getCenter(new THREE.Vector3());
      this.gizmoCenter = [center.x, center.y, center.z];
      this.gizmo.position.copy(center);
      this.gizmo.scale.setScalar(Math.max(0.25, all.getSize(new THREE.Vector3()).length() * 0.6));
    } else {
      this.gizmoCenter = null;
    }
    this.gizmo.visible = this.gizmoEnabled && this.gizmoCenter !== null;
    if (count > 1 && !all.isEmpty()) {
      all.expandByScalar(all.getSize(new THREE.Vector3()).length() * 0.02);
      const box = new THREE.Box3Helper(all);
      (box.material as THREE.Material).dispose();
      box.material = this.groupMaterial;
      box.computeLineDistances();
      box.renderOrder = 3;
      this.selection.add(box);
    }
  }

  /** Shows (or hides) the rotate rings around the current selection. */
  setRotateGizmo(enabled: boolean) {
    this.gizmoEnabled = enabled;
    this.gizmo.visible = enabled && this.gizmoCenter !== null;
  }

  /** The centre of the selection, where the rotate rings sit, or null with nothing selected. */
  selectionCenter(): Vec3 | null {
    return this.gizmoCenter;
  }

  /** Which rotate ring is under a pointer position: 'y' turns about the vertical, 'x' and 'z' tilt. */
  pickGizmo(clientX: number, clientY: number): 'x' | 'y' | 'z' | null {
    if (!this.gizmo.visible) return null;
    const r = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
    const caster = new THREE.Raycaster();
    caster.setFromCamera(ndc, this.camera);
    this.gizmo.updateMatrixWorld(true);
    const hit = caster.intersectObjects(Object.values(this.gizmoRings).map((g) => g.hit), false)[0];
    return (hit?.object.userData.axis as 'x' | 'y' | 'z' | undefined) ?? null;
  }

  /** Brightens one ring (hovered or being dragged) and dims the others, or shows all evenly. */
  highlightGizmo(axis: 'x' | 'y' | 'z' | null) {
    for (const [a, g] of Object.entries(this.gizmoRings)) {
      g.material.opacity = axis === null ? 0.85 : a === axis ? 1 : 0.25;
    }
  }

  /**
   * Placed instances whose on-screen centre lies inside a rectangle given in client (page) pixels: what a box-select
   * drag picks up.
   */
  instancesInRect(x0: number, y0: number, x1: number, y1: number): string[] {
    const r = this.canvas.getBoundingClientRect();
    const [lx, hx] = [Math.min(x0, x1), Math.max(x0, x1)];
    const [ly, hy] = [Math.min(y0, y1), Math.max(y0, y1)];
    const out: string[] = [];
    const box = new THREE.Box3();
    const c = new THREE.Vector3();
    for (const mesh of this.instances.children) {
      box.setFromObject(mesh);
      if (box.isEmpty()) continue;
      box.getCenter(c).project(this.camera);
      if (c.z < -1 || c.z > 1) continue; // behind the camera or past the far plane
      const sx = r.left + ((c.x + 1) / 2) * r.width;
      const sy = r.top + ((1 - c.y) / 2) * r.height;
      if (sx >= lx && sx <= hx && sy >= ly && sy <= hy) out.push(mesh.name);
    }
    return out;
  }

  /** The see-through preview of what a click would place, or hidden. */
  showGhost(g: Ghost | null) {
    if (!g) { this.ghost.visible = false; return; }
    if (g.matrix === undefined) {
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.BufferAttribute(g.buffers.positions, 3));
      geo.setIndex(new THREE.BufferAttribute(g.buffers.indices, 1));
      this.setGhostGeometry(geo, geo);
      this.ghost.matrix.identity();
    } else {
      const geo = this.objects.get(g.object);
      if (!geo) { this.ghost.visible = false; return; }
      this.setGhostGeometry(geo, null);
      this.ghost.matrix.fromArray(g.matrix);
    }
    this.ghost.matrixWorld.copy(this.ghost.matrix);
    this.ghost.visible = true;
  }

  private setGhostGeometry(geo: THREE.BufferGeometry, owned: THREE.BufferGeometry | null) {
    if (this.ghost.geometry !== geo) {
      this.ghost.geometry = geo;
      this.ghostOwned?.dispose();
      this.ghostOwned = owned;
    }
  }

  /** The ray under a pointer position, in world space. */
  ray(clientX: number, clientY: number): { origin: Vec3; dir: Vec3 } {
    const r = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
    const caster = new THREE.Raycaster();
    caster.setFromCamera(ndc, this.camera);
    return { origin: caster.ray.origin.toArray() as Vec3, dir: caster.ray.direction.toArray() as Vec3 };
  }

  /** The brush outline lying on the surface, or hidden. */
  showCursor(at: { point: Vec3; normal: Vec3 } | null, radius = 0) {
    this.cursor.visible = !!at;
    if (!at) return;
    this.cursor.position.set(...at.point);
    this.cursor.lookAt(new THREE.Vector3(...at.point).add(new THREE.Vector3(...at.normal)));
    this.cursor.scale.setScalar(radius);
  }

  /** A length that looks the same on screen whatever the model's size: a fraction of its bounding radius. */
  modelScale(): number {
    return this.hasGeometry ? Math.max(this.geometry.boundingSphere?.radius ?? 1, 0.5) : 3;
  }

  private resize() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    if (this.activePhoto) this.applyPhotoFov(this.activePhoto);
    else this.camera.updateProjectionMatrix();
  }

  dispose() {
    this.resizer.disconnect();
    this.renderer.setAnimationLoop(null);
    this.controls.dispose();
    this.geometry.dispose();
    this.splatCloud.dispose();
    for (const entry of this.overlayClouds.values()) {
      entry.geometry.dispose();
      entry.material.dispose();
    }
    this.overlayClouds.clear();
    this.clearGroup(this.photoCameraGroup);
    this.clearGroup(this.gridGroup);
    this.clearGroup(this.planGroup);
    this.planLineMaterial.dispose();
    this.planDotMaterial.dispose();
    this.planFillMaterial.dispose();
    this.wireframeMaterial.dispose();
    for (const g of this.objects.values()) g.dispose();
    this.ghostOwned?.dispose();
    this.ghostMaterial.dispose();
    this.showSelection(null);
    this.selectionMaterial.dispose();
    this.groupMaterial.dispose();
    const disposed = new Set<{ dispose: () => void }>();
    for (const g of Object.values(this.gizmoRings)) {
      for (const d of [g.ring.geometry, g.hit.geometry, g.material, g.hit.material as THREE.Material]) disposed.add(d);
    }
    for (const d of disposed) d.dispose();
    this.surface.dispose();
    this.dots.dispose();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
    this.canvas.remove();
  }
}
