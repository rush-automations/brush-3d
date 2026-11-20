# Formats and conventions

## Space

- Units are meters. +y is up; the ground is the xz plane.
- Files are not converted on read or write. A Blender export must use **+Y Up**.
- A region is an oriented box: `center` [x, y, z], full `size` [x, y, z] (each ≥ 0) and `yawDeg`, a rotation in degrees about +y. A point is inside when its offset from the centre, turned by −yaw, is within half the size on every axis (boundary included).
- Ops that take an optional `box` apply to the whole mesh without one.

## Numbers

- JSON numbers are read as f64 and rounded to f32 once, at the kernel boundary. Half sizes and radians are computed in f64 before that rounding, so a box means the same thing whether it came from JSON, JavaScript or Python.
- Colours are `[r, g, b]`, integers 0–255.
- Seeds are u32. The same seed on the same mesh gives the same result everywhere.

## Mesh

- An indexed triangle list. Each vertex has a position (3 × f32) and an RGB colour (3 × u8). Each face is 3 × u32 vertex indices.
- Winding isn't relied on. Scans have none they can be trusted for, so shading and occlusion are two-sided.
- **Attributes:** any number of named per-vertex scalars, each with its file type (`i8 u8 i16 u16 i32 u32 f32 f64`). When an op creates a vertex:
  - a vertex split from an edge gets the midpoint of the two ends' values for float attributes, and the first end's value for integer attributes;
  - a hole-fill centre gets the mean of the loop's values for float attributes, and the first vertex's value for integer attributes;
  - a cloned or cut vertex copies its source;
  - a new primitive gets 0.

## PLY

### Reading

- `ascii`, `binary_little_endian` and `binary_big_endian` are all accepted.
- Vertex `x y z` can be any scalar type.
- Colour is read from `red green blue` (or `diffuse_*`). Integer colours are 0–255; float colours are 0–1. Vertices without colour read as 200 grey.
- Every other scalar vertex property becomes an attribute.
- Faces come from `vertex_indices` (or `vertex_index`), with any count and index types:
  - polygons are fanned into triangles;
  - faces with fewer than 3 corners or an out-of-range index are dropped and reported.
- Other elements and other face properties are dropped and reported.
- A file without faces reads as a mesh with no faces (a point cloud).

### Writing

The writer always produces binary little-endian, in this layout:

```
property float x / y / z
property uchar red / green / blue
property <type> <name>        (one per attribute, in order)
element face N
property list uchar int vertex_indices
```

Without attributes, this is byte-for-byte the format the Brush worker and server store.

## OBJ

- **Reading:**
  - `v x y z [r g b]` lines; colours are 0–1, or 0–255 if any value is above 1;
  - `f` lines with `v`, `v/vt`, `v//vn` or `v/vt/vn` corners, including negative indices;
  - polygons are fanned into triangles, and everything else is ignored.
- **Writing:** `v x y z r g b` with colours 0–1, and 1-based `f a b c`.

## GLB

Written only: one mesh with `POSITION`, `COLOR_0` (normalised u8 RGBA), u32 indices and a double-sided matte material.

## Content hash

`Mesh::content_hash` is FNV-1a 64 over the little-endian bytes of, in order:
- the vertex count and face count (as u64);
- the positions (f32 bits);
- the colours;
- the indices (u32);
- for each attribute, its name, a 0 byte, then its values as f64 bits.

It's printed as 16 lowercase hex digits. Equal meshes hash equal on every platform. It's an identity check, not a cryptographic one.

## Journal

JSON Lines, one entry per line, tagged by `event`. The full schema is in [`journal.schema.json`](journal.schema.json).

- `applied`: carries `seq`, `op`, an optional `actor` (`id`, plus `kind` = `person`, `model` or `app`), `outcome` (`count`, `vertices`, `faces`) and `hash`, the mesh's hash after the op.
- `undo`, `redo` and `revert`: carry `seq` and the resulting `hash`.

Replaying a journal re-applies each op and checks each `hash`. The first mismatch is reported as a divergence.
