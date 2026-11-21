# Retaining wall — civil engineering

A 3 m cantilever retaining wall bay: footing, stem, compacted backfill, drainage, and the site-inspection mark-up.

1. **Footing + stem** (`addBox` ×2) — 0.3 m footing, 1.8 m stem, dimensioned in metres.
2. **Backfill** (`addRamp`) — the compacted slope behind the stem, high edge against the wall.
3. **Weep holes** (`carveBox` ×2) — drainage cored through the stem. Carving splits the triangles exactly along
   the box, so the hole is watertight geometry, not a texture trick.
4. **Inspection paint** (`splatPaint`) — red survey marks sprayed above each weep hole. Splats stick to the
   *surface*, so a later mesh refinement pass won't smear them; `drip` lets gravity pull the spray down the face.
5. **Grime pass** (`splatPaint`) — a wide, faint, drippy spray along the top edge for weathering context.
6. **Quantity take-off** — `brush info` reports volume, surface area and centroid from the exact solid:

```sh
brush replay ../fixtures/empty.ply journal.jsonl -o out.ply
brush info out.ply        # volume → concrete m³; area → formwork m²
```
