/*
 * The Brush engine's C ABI (bindings/swift/src/lib.rs). The same engine the web app runs as WebAssembly.
 *
 * Strings are UTF-8, NUL-terminated. Strings and bytes returned by the engine belong to the caller: free them with
 * brush_string_free and brush_bytes_free. A function that can fail takes `char **error`; on failure it returns NULL
 * (or -1, or false) and, when error is not NULL, stores a message there to free with brush_string_free.
 *
 * Buffers returned as `const T *` are borrowed from the engine and stay valid until the next call that changes the
 * document (an op, a stroke, undo, redo, revert, set_object for object buffers) or brush_document_free.
 */
#ifndef BRUSH_ENGINE_H
#define BRUSH_ENGINE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct BrushDocument BrushDocument;

/* A point on a surface under a ray. */
typedef struct BrushHit {
    uint32_t face;
    float t;
    float point[3];
    float normal[3];     /* turned to face the ray */
    float barycentric[3];
} BrushHit;

/* Library */
char *brush_version(void);      /* JSON: {"engine": "...", "ops": "..."} */
char *brush_ops_schema(void);   /* JSON Schema of ops */
void brush_string_free(char *s);
void brush_bytes_free(uint8_t *bytes, size_t len);

/* Documents */
BrushDocument *brush_document_open(const uint8_t *bytes, size_t len, bool read_only, size_t undo_budget, char **error);
BrushDocument *brush_document_new(void);
void brush_document_free(BrushDocument *doc);
bool brush_document_read_only(const BrushDocument *doc);
char *brush_document_read_report(const BrushDocument *doc);

/* Edits. Ops are JSON as in spec/ops.schema.json; actor_json may be NULL. Results are JSON. */
char *brush_document_apply(BrushDocument *doc, const char *op_json, const char *actor_json, char **error);
char *brush_document_apply_all(BrushDocument *doc, const char *ops_json, const char *actor_json, char **error);
/* centers: 3 doubles per dab; pressures: 1 per dab or NULL. Returns 1 changed, 0 unchanged, -1 error. */
int32_t brush_document_extend_stroke(BrushDocument *doc, const double *centers, const double *pressures, size_t count, char **error);
void brush_document_end_stroke(BrushDocument *doc);
bool brush_document_undo(BrushDocument *doc);
bool brush_document_redo(BrushDocument *doc);
bool brush_document_revert(BrushDocument *doc);
void brush_document_mark_saved(BrushDocument *doc);

/* State */
char *brush_document_state(const BrushDocument *doc);  /* JSON */
bool brush_document_can_undo(const BrushDocument *doc);
bool brush_document_can_redo(const BrushDocument *doc);
bool brush_document_dirty(const BrushDocument *doc);
size_t brush_document_vertex_count(const BrushDocument *doc);
size_t brush_document_face_count(const BrushDocument *doc);
char *brush_document_journal(const BrushDocument *doc);  /* JSON Lines */

/* The document's own mesh, borrowed. len receives element counts. */
const float *brush_document_positions(const BrushDocument *doc, size_t *len);
const uint8_t *brush_document_colors(const BrushDocument *doc, size_t *len);
const uint32_t *brush_document_indices(const BrushDocument *doc, size_t *len);
const float *brush_document_normals(BrushDocument *doc, size_t *len);

/* Picking. raycast sees the document's own mesh; pick also sees placed instances (instance gets the id, or NULL). */
bool brush_document_raycast(BrushDocument *doc, const float origin[3], const float dir[3], BrushHit *hit);
bool brush_document_pick(BrushDocument *doc, const float origin[3], const float dir[3], BrushHit *hit, char **instance);

/* Files: "ply", "obj" or "glb". flattened copies placed objects in. */
uint8_t *brush_document_export(const BrushDocument *doc, const char *format, bool flattened, size_t *len, char **error);

/* Scene: linked instances of other objects ("scan:<id>"). */
char *brush_document_scene(const BrushDocument *doc);  /* JSON {"instances": [...]} */
bool brush_document_load_scene(BrushDocument *doc, const char *scene_json, char **error);
bool brush_document_set_object(BrushDocument *doc, const char *object, const uint8_t *bytes, size_t len, char **error);
char *brush_document_missing_objects(const BrushDocument *doc);  /* JSON array */
const float *brush_document_object_positions(const BrushDocument *doc, const char *object, size_t *len);
const uint8_t *brush_document_object_colors(const BrushDocument *doc, const char *object, size_t *len);
const uint32_t *brush_document_object_indices(const BrushDocument *doc, const char *object, size_t *len);
const float *brush_document_object_normals(BrushDocument *doc, const char *object, size_t *len);
/* Column-major 4x4 object-to-world transforms. */
bool brush_document_instance_transform(const BrushDocument *doc, const char *id, float out[16]);
bool brush_document_object_transform(const BrushDocument *doc, const char *object, const double position[3], double yaw_deg, double scale, float out[16]);

/* Building: the op (JSON) that builds a piece where the pointer is. See Placement in crates/brush-engine/src/place.rs. */
char *brush_document_place(const BrushDocument *doc, const char *placement_json, char **error);

#ifdef __cplusplus
}
#endif

#endif /* BRUSH_ENGINE_H */
