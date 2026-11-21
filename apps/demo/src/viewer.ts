// The 3D view: three.js draws whatever mesh the engine holds. All geometry and picking stay in the engine; this file
// only presents it.
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';

export type Vec3 = [number, number, number];

export interface MeshData {
  positions: Float32Array;
  colors: Uint8Array;
  indices: Uint32Array;
  normals: Float32Array;
}

/** Paint splats as instanced quads with a Gaussian falloff, matching the engine's own software renderer. */
class SplatCloud {
  readonly object: THREE.Mesh;
  private readonly geometry = new THREE.InstancedBufferGeometry();

  constructor() {
    const g = this.geometry;
    g.setAttribute('position', new THREE.BufferAttribute(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3));
    g.setIndex([0, 1, 2, 0, 2, 3]);
    g.instanceCount = 0;
    const material = new THREE.ShaderMaterial({
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
    this.object = new THREE.Mesh(g, material);
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
}

export class Viewer {
  readonly camera: THREE.PerspectiveCamera;
  readonly controls: OrbitControls;
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly material = new THREE.MeshStandardMaterial({ vertexColors: true, side: THREE.DoubleSide, roughness: 0.85 });
  private readonly mesh = new THREE.Mesh(new THREE.BufferGeometry(), this.material);
  private readonly splats = new SplatCloud();
  private readonly cursor: THREE.Mesh;

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.scene.background = new THREE.Color(0xeef0f3);
    this.camera = new THREE.PerspectiveCamera(50, 1, 0.01, 1000);
    this.camera.position.set(3, 3, 5);
    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    // Left drag is the brush; right drag orbits, middle drag pans, the wheel zooms.
    this.controls.mouseButtons = { LEFT: null, MIDDLE: THREE.MOUSE.PAN, RIGHT: THREE.MOUSE.ROTATE };

    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x8a8f99, 1.6));
    const sun = new THREE.DirectionalLight(0xffffff, 1.4);
    sun.position.set(4, 8, 3);
    this.scene.add(sun);
    this.scene.add(this.mesh);
    this.scene.add(this.splats.object);

    const grid = new THREE.GridHelper(20, 20, 0xc5c9d1, 0xdde0e5);
    grid.position.y = -0.001;
    this.scene.add(grid);

    this.cursor = new THREE.Mesh(
      new THREE.RingGeometry(0.92, 1, 48),
      new THREE.MeshBasicMaterial({ color: 0xff6a00, side: THREE.DoubleSide, depthTest: false, transparent: true, opacity: 0.9 }),
    );
    this.cursor.renderOrder = 1;
    this.cursor.visible = false;
    this.scene.add(this.cursor);

    new ResizeObserver(() => this.resize()).observe(canvas);
    this.renderer.setAnimationLoop(() => {
      this.controls.update();
      this.renderer.render(this.scene, this.camera);
    });
  }

  private resize() {
    const { clientWidth: w, clientHeight: h } = this.canvas;
    if (!w || !h) return;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  setMesh(m: MeshData) {
    const g = this.mesh.geometry;
    g.setAttribute('position', new THREE.BufferAttribute(m.positions, 3));
    g.setAttribute('color', new THREE.BufferAttribute(m.colors, 3, true));
    g.setAttribute('normal', new THREE.BufferAttribute(m.normals, 3));
    g.setIndex(new THREE.BufferAttribute(m.indices, 1));
    g.computeBoundingSphere();
  }

  /** The paint splats: 8 floats and 4 bytes each, straight from the engine. */
  setSplats(geo: Float32Array, colors: Uint8Array) {
    this.splats.setData(geo, colors);
  }

  setFlatShading(flat: boolean) {
    this.material.flatShading = flat;
    this.material.needsUpdate = true;
  }

  setWireframe(on: boolean) {
    this.material.wireframe = on;
  }

  /** Points the camera at a bounding box. */
  frame(bounds: { min: Vec3; max: Vec3 } | null) {
    if (!bounds) return;
    const min = new THREE.Vector3(...bounds.min);
    const max = new THREE.Vector3(...bounds.max);
    const centre = min.clone().add(max).multiplyScalar(0.5);
    const size = Math.max(max.distanceTo(min), 0.5);
    this.controls.target.copy(centre);
    this.camera.position.copy(centre).add(new THREE.Vector3(0.6, 0.5, 1).normalize().multiplyScalar(size * 1.3));
    this.camera.near = size / 1000;
    this.camera.far = size * 100;
    this.camera.updateProjectionMatrix();
  }

  /** The ray under a pointer position, in world space. */
  ray(clientX: number, clientY: number): { origin: Vec3; dir: Vec3 } {
    const r = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
    const caster = new THREE.Raycaster();
    caster.setFromCamera(ndc, this.camera);
    return { origin: caster.ray.origin.toArray() as Vec3, dir: caster.ray.direction.toArray() as Vec3 };
  }

  /** The brush outline at a surface point, or hidden. */
  showCursor(at: { point: Vec3; normal: Vec3 } | null, radius: number) {
    this.cursor.visible = !!at;
    if (!at) return;
    this.cursor.position.set(...at.point);
    this.cursor.lookAt(new THREE.Vector3(...at.point).add(new THREE.Vector3(...at.normal)));
    this.cursor.scale.setScalar(radius);
  }
}
