# Example workflows

Four end-to-end modeling sessions, one per audience. Each directory holds:

- `ops.json` — the session as a readable list of engine ops (the same wire format the wasm, Swift and studio
  targets speak).
- `journal.jsonl` — the recorded journal, with the content hash after every op.
- `manifest.json` — the expected final content hash. CI replays every journal and fails if a hash drifts.

The results are one command away, so the repository stays text-only:

```sh
# rebuild a workflow's model and preview:
brush apply examples/fixtures/empty.ply --ops examples/product-design-desk-organiser/ops.json \
  -o result.ply --journal journal.jsonl --actor brush-examples
brush render result.ply -o preview.png

# or replay the recorded journal, checking every step's hash:
brush replay examples/fixtures/empty.ply examples/product-design-desk-organiser/journal.jsonl -o out.ply
```

Every op is validated before it runs, so a replay is all-or-nothing; the per-step hashes in the journal prove the
edit history bit-for-bit on any platform.

| Workflow | Audience | Shows off |
| --- | --- | --- |
| [product-design-desk-organiser](product-design-desk-organiser/) | product / industrial designer | primitives, branding paint strokes that ignore mesh resolution |
| [architecture-pavilion](architecture-pavilion/) | architect | floor-plan extrusion, lathe columns, weathering splats, material tinting |
| [civil-retaining-wall](civil-retaining-wall/) | civil engineer | structural primitives, backfill ramp, carved drainage, inspection paint, measure |
| [game-art-hero-rock](game-art-hero-rock/) | game / environment artist | sculpting, stylised simplification, moss + lava splat painting, baking to vertices |
