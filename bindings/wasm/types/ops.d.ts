/* Generated from spec/ops.schema.json by json-schema-to-typescript. Do not edit; run bun run types. */

/**
 * One edit.
 */
export type BrushEngineOperation =
  | {
      box: BoxRegion;
      invert?: boolean;
      op: "removeBox";
    }
  | {
      box: BoxRegion;
      op: "carveBox";
    }
  | {
      box?: BoxRegion | null;
      iterations?: number;
      /**
       * Keep vertices on open edges (hole rims, outlines) where they are.
       */
      keepBoundary?: boolean;
      method?: "taubin" | "laplacian";
      op: "smooth";
      /**
       * 0..1, how far each pass moves a vertex toward the average of its neighbours.
       */
      strength?: number;
    }
  | {
      cell: number;
      op: "decimate";
    }
  | {
      /**
       * Keep open edges (hole rims, outlines) in shape.
       */
      keepBoundary?: boolean;
      op: "simplify";
      targetFaces?: number | null;
      targetRatio?: number | null;
    }
  | {
      brush: Brush;
      /**
       * The paint brush's colour.
       *
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      dabs: Dab[];
      /**
       * Lower instead of raise (draw, inflate) or spread instead of gather (pinch).
       */
      invert?: boolean;
      op: "sculpt";
      /**
       * Meters.
       */
      radius: number;
      /**
       * 0..1 per dab.
       */
      strength?: number;
      /**
       * Direction the brush points (camera toward surface). Normals are turned to face it, so `draw` raises the
       * surface toward the person sculpting even where a scan's winding is unreliable.
       *
       * @minItems 3
       * @maxItems 3
       */
      view?: [number, number, number] | null;
    }
  | {
      maxEdges?: number;
      op: "fillHoles";
    }
  | {
      box?: BoxRegion | null;
      maxEdge: number;
      maxFaces?: number;
      op: "subdivide";
      passes?: number;
    }
  | {
      /**
       * Largest displacement, meters.
       */
      amplitude: number;
      box?: BoxRegion | null;
      /**
       * Meters over which the effect eases off toward the box boundary.
       */
      edge?: number;
      octaves?: number;
      op: "roughen";
      /**
       * Crack-like ridges instead of soft lumps.
       */
      ridged?: boolean;
      /**
       * Feature size, meters.
       */
      scale: number;
      seed?: number;
    }
  | {
      /**
       * @minItems 3
       * @maxItems 3
       */
      a?: [number, number, number];
      /**
       * @minItems 3
       * @maxItems 3
       */
      b?: [number, number, number];
      box?: BoxRegion | null;
      mode?: "mix" | "shade";
      octaves?: number;
      op: "tint";
      scale: number;
      seed?: number;
      /**
       * 0..1.
       */
      strength: number;
      /**
       * Stretches the noise per axis (a long x stretch gives wood grain).
       *
       * @minItems 3
       * @maxItems 3
       */
      stretch?: [number, number, number];
    }
  | {
      box?: BoxRegion | null;
      cell?: number;
      op: "bakeShadows";
      radius?: number;
      samples?: number;
      strength?: number;
    }
  | {
      box: BoxRegion;
      fromReference?: boolean;
      maxDist?: number;
      op: "matchColor";
      strength?: number;
    }
  | {
      box: BoxRegion;
      mirrorX?: boolean;
      mirrorZ?: boolean;
      /**
       * @minItems 3
       * @maxItems 3
       */
      offset: [number, number, number];
      op: "cloneBox";
      rotateDeg?: number;
    }
  | {
      box: BoxRegion;
      /**
       * @minItems 3
       * @maxItems 3
       */
      color: [number, number, number];
      op: "paint";
    }
  | {
      box: BoxRegion;
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      op: "addBox";
    }
  | {
      box: BoxRegion;
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      op: "addRamp";
    }
  | {
      /**
       * Names the instance within this document; later ops refer to it.
       */
      id: string;
      object: string;
      op: "placeObject";
      /**
       * @minItems 3
       * @maxItems 3
       */
      position: [number, number, number];
      scale?: number;
      yawDeg?: number;
    }
  | {
      id: string;
      op: "moveObject";
      /**
       * @minItems 3
       * @maxItems 3
       */
      position: [number, number, number];
      scale?: number;
      yawDeg?: number;
    }
  | {
      id: string;
      op: "removeObject";
    }
  | {
      id: string;
      op: "bakeObject";
    }
  | {
      /**
       * @minItems 3
       * @maxItems 3
       */
      base: [number, number, number];
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      height: number;
      op: "addFrustum";
      radiusBottom: number;
      radiusTop: number;
      segments?: number;
    }
  | {
      /**
       * @minItems 3
       * @maxItems 3
       */
      center: [number, number, number];
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      op: "addSphere";
      /**
       * @minItems 3
       * @maxItems 3
       */
      radii: [number, number, number];
      rings?: number;
      segments?: number;
    }
  | {
      /**
       * @minItems 3
       * @maxItems 3
       */
      base: [number, number, number];
      capped?: boolean;
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      op: "addLathe";
      profile: [number, number][];
      segments?: number;
    }
  | {
      /**
       * @minItems 3
       * @maxItems 3
       */
      color?: [number, number, number];
      height: number;
      op: "addExtrude";
      polygon: [number, number][];
      y0: number;
    };
export type Brush = "draw" | "inflate" | "smooth" | "flatten" | "pinch" | "paint";

/**
 * An oriented box: `center` and full `size` in meters, turned by `yawDeg` degrees about +y.
 */
export interface BoxRegion {
  /**
   * @minItems 3
   * @maxItems 3
   */
  center: [number, number, number];
  /**
   * @minItems 3
   * @maxItems 3
   */
  size: [number, number, number];
  yawDeg?: number;
}
/**
 * One brush touch. `pressure` (0..1) scales strength, as from a pen.
 */
export interface Dab {
  /**
   * @minItems 3
   * @maxItems 3
   */
  center: [number, number, number];
  pressure?: number;
}
