# Brush Studio

The desktop (and web) editor for the Brush engine. One window, four areas:

- **Viewport** — orbit camera over the deterministic software rasteriser, so what you see is bit-identical to what
  the CLI, the wasm demo and CI render. Left-drag sprays or erases splats, right-drag (or Shift-drag) orbits,
  middle-drag pans, scroll zooms.
- **Tools (left)** — create primitives (box, sphere, lathe), tune the splat brush (radius, splat size, flow,
  opacity, gravity drip, colour), bake splats down to vertex colours, undo/redo, and open/save binary PLY
  (splat data included, round-trips byte-for-byte).
- **Model + journal (right)** — vertex/face/splat counts, content hash, the measure panel (volume, surface area,
  centroid, closedness) and the running op journal. Every edit is an engine op, so the journal replays on any
  other target and the hash matches.

## Run on desktop

```sh
cargo run --release
```

## Build for web

The web build uses [trunk](https://trunkrs.dev):

```sh
cargo install trunk
trunk build --release   # serves dist/ — the full editor, no backend
```

Open and Save use the native file dialog on desktop and the browser's download on the web.

Splat painting is resolution-independent: the spray lands in object space, not on vertices, so it survives
`simplify` and `subdivide` untouched until you bake it.
