// A starter scene built entirely from engine ops: a weathered stone plinth with a column and a dome.
import type { Op } from 'brush-wasm';

export const SAMPLE_SCENE: Op[] = [
  { op: 'addBox', box: { center: [0, 0.15, 0], size: [3, 0.3, 3] }, color: [168, 160, 148] },
  { op: 'addLathe', base: [0, 0.3, 0], segments: 40, color: [190, 182, 168], profile: [[0.5, 0], [0.5, 0.08], [0.38, 0.14], [0.32, 0.2], [0.3, 1.9], [0.38, 1.98], [0.5, 2.05], [0.5, 2.15]] },
  { op: 'addSphere', center: [0, 2.45, 0], radii: [0.5, 0.3, 0.5], segments: 40, rings: 20, color: [150, 120, 96] },
  { op: 'subdivide', maxEdge: 0.06, maxFaces: 400000 },
  { op: 'roughen', amplitude: 0.012, scale: 0.12, seed: 7 },
  { op: 'tint', a: [150, 140, 125], b: [205, 198, 182], scale: 0.4, strength: 0.6, seed: 3 },
  { op: 'bakeShadows', strength: 0.45, radius: 1 },
];
