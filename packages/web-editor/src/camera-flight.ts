/** Minimal camera target for local WASD / vertical flight. */
export interface CameraFlightTarget {
  panLocal(forward: number, right: number, up: number): void;
}

export interface CameraFlightOptions {
  /** Horizontal flight speed in metres per second (default: 4). */
  speed?: number;
  /** Vertical speed multiplier relative to horizontal speed (default: 0.75). */
  verticalScale?: number;
  /** Maximum frame delta in seconds to clamp lag spikes (default: 0.05). */
  maxDeltaSeconds?: number;
}

const FLIGHT_KEYS = new Set([
  'w',
  'a',
  's',
  'd',
  'q',
  'e',
  'arrowup',
  'arrowdown',
  'arrowleft',
  'arrowright',
  'shift',
  ' ',
]);

/** Returns true if a lowercased `KeyboardEvent.key` is a camera flight control key. */
export function isFlightKey(key: string): boolean {
  return FLIGHT_KEYS.has(key.toLowerCase());
}

/**
 * Smooth WASD / Arrow / Space-Shift / Q-E camera flight controller.
 *
 * Keeps a `requestAnimationFrame` loop running while active and translates held keys into
 * camera-relative `panLocal(forward, right, up)` increments.
 */
export class CameraFlightController {
  private readonly held = new Set<string>();
  private readonly speed: number;
  private readonly verticalScale: number;
  private readonly maxDelta: number;
  private target: CameraFlightTarget | null;
  private raf = 0;
  private lastTime = 0;

  constructor(target: CameraFlightTarget | null = null, options: CameraFlightOptions = {}) {
    this.target = target;
    this.speed = options.speed ?? 4;
    this.verticalScale = options.verticalScale ?? 0.75;
    this.maxDelta = options.maxDeltaSeconds ?? 0.05;
  }

  setTarget(target: CameraFlightTarget | null) {
    this.target = target;
  }

  /** Registers a key press. Returns true if the key is a flight key. */
  keyDown(key: string): boolean {
    const k = key.toLowerCase();
    if (!FLIGHT_KEYS.has(k)) return false;
    this.held.add(k);
    return true;
  }

  /** Releases a key. */
  keyUp(key: string) {
    this.held.delete(key.toLowerCase());
  }

  /** Releases all held keys (e.g. on window blur). */
  clear() {
    this.held.clear();
  }

  /** Number of currently held flight keys. */
  get activeKeyCount(): number {
    return this.held.size;
  }

  /** Advances flight by `dtSeconds` (clamped to `maxDeltaSeconds`). Returns true if camera moved. */
  step(dtSeconds: number): boolean {
    const target = this.target;
    if (!target || this.held.size === 0 || dtSeconds <= 0) return false;
    const dt = Math.min(this.maxDelta, dtSeconds);
    const k = this.held;
    const fwd = (k.has('w') || k.has('arrowup') ? 1 : 0) - (k.has('s') || k.has('arrowdown') ? 1 : 0);
    const right = (k.has('d') || k.has('arrowright') ? 1 : 0) - (k.has('a') || k.has('arrowleft') ? 1 : 0);
    const up = (k.has(' ') || k.has('e') ? 1 : 0) - (k.has('shift') || k.has('q') ? 1 : 0);
    if (!fwd && !right && !up) return false;
    const dist = this.speed * dt;
    target.panLocal(fwd * dist, right * dist, up * dist * this.verticalScale);
    return true;
  }

  /** Starts the `requestAnimationFrame` flight loop. Safe to call multiple times. */
  start() {
    if (this.raf) return;
    this.lastTime = typeof performance !== 'undefined' ? performance.now() : Date.now();
    const tick = (now: number) => {
      const dt = (now - this.lastTime) / 1000;
      this.lastTime = now;
      this.step(dt);
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  /** Stops the `requestAnimationFrame` flight loop and clears held keys. */
  stop() {
    if (this.raf) {
      cancelAnimationFrame(this.raf);
      this.raf = 0;
    }
    this.held.clear();
  }
}
