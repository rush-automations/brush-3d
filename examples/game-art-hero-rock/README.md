# Hero rock — game art

A stylised boulder for an environment kit, from sphere to engine-ready vertex colours.

1. **Base** (`addSphere`) — an ellipsoid slightly squashed on Y.
2. **Shape language** (`sculpt`) — `inflate` dabs push character bulges out along the normals, then `pinch`
   dabs sharpen the crown and one flank. Strokes stream: each dab is its own op, so a tablet session and a batch
   file produce identical geometry.
3. **Stylise** (`simplify`) — quadric-error collapse to 60% of the faces keeps the silhouette and drops the
   noise: instant low-poly read.
4. **Moss** (`splatPaint`) — broad green sprays from above with heavy colour noise and drip, so the moss reads
   organic instead of airbrushed. The splats are stored independently of the mesh, so the simplify in step 3 did
   not blur them — they were painted *after*, but they would have survived it either way.
5. **Lava accent** (`splatPaint`) — one hot orange dab low on the face.
6. **Bake** (`splatBake`) — splats are splatted into vertex colours and cleared: one draw call in the engine, no
   splat runtime needed.

```sh
brush replay ../fixtures/empty.ply journal.jsonl -o out.ply
brush convert out.ply out.glb   # straight into the engine of your choice
```
