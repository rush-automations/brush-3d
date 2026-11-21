# Desk organiser — product design

A desk tidy sketched the way an industrial designer blocks out a concept: boxes for the tray, pen cup and phone
stand, a lathed cable grommet, then branding paint sprayed straight onto the surface.

1. **Block out** (`addBox` ×3, `addLathe`) — tray, cup and stand in one dark ABS colour; every dimension in metres.
2. **Brand stroke** (`splatPaint`) — a vertical stripe on the pen cup. The paint is *splats*, not vertex colours:
   it lands in object space with a deterministic spray pattern, so the coarse block-out mesh takes the same stroke
   as a production-density mesh. `drip` pulls the spray down slightly, like real rattle-can paint.
3. **Tray logo** (`splatPaint`) — three low-pressure dabs along the tray lip, wet-on-wet: where the dabs overlap,
   opacity builds instead of z-fighting geometry.
4. Ship it — the finished PLY keeps the splats live. Slice it, measure it, or bake later (`splatBake`) once the
   mesh is subdivided for tooling.

```sh
brush replay ../fixtures/empty.ply journal.jsonl -o out.ply
brush info out.ply        # counts, bounds, volume, hash
```
