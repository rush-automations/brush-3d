# Garden pavilion — architecture

A small pavilion from plan to material pass.

1. **Slab** (`addExtrude`) — the 12 × 8 m floor plan extruded 0.2 m. Plans are just `[x, z]` point lists, so a
   surveyor's sketch becomes a solid in one op.
2. **Columns** (`addLathe` ×4) — a turned profile (foot, shaft, capital) revolved at each corner. Profiles are
   `[radius, height]` pairs, so entasis is editable text.
3. **Roof** (`addExtrude`) — the same plan with an overhang, lifted to 3.2 m, in terracotta.
4. **Weathering** (`splatPaint`) — two broad, low-flow sprays across the roof with gravity drip: streaked rain
   wear that would take hundreds of vertices to model, applied without touching the roof's tessellation.
5. **Material tint** (`splatTint`) — the slab taken down a few shades in one region op.

```sh
brush replay ../fixtures/empty.ply journal.jsonl -o out.ply
brush info out.ply        # slab + roof + columns volume, area, centroid
```
