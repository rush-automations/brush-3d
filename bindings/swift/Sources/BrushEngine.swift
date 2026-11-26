//
//  BrushEngine.swift
//
//  The Brush engine for Swift: a thin, safe wrapper over its C ABI (BrushEngine.xcframework, module BrushEngineFFI).
//  It is the same engine the web app runs as WebAssembly, so a phone and a browser make bit-identical meshes and
//  journals from the same ops. Source of truth: engine/bindings/swift; copied into apps by build-xcframework.sh.
//
//  A document is not thread-safe. Use it from one thread at a time (the main actor, with heavy ops handed to a
//  background task while nothing else touches it).
//

import BrushEngineFFI
import Foundation
import simd

public struct BrushEngineError: LocalizedError, Sendable {
    public let message: String
    public var errorDescription: String? { message }
}

/// Runs an FFI call that reports failure through `char **error`.
private func attempt<T>(_ body: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> T?) throws -> T {
    var error: UnsafeMutablePointer<CChar>?
    if let value = body(&error) { return value }
    let message = error.map { String(cString: $0) } ?? "the engine failed"
    brush_string_free(error)
    throw BrushEngineError(message: message)
}

/// Takes ownership of a string the engine returned.
private func take(_ s: UnsafeMutablePointer<CChar>?) -> String? {
    guard let s else { return nil }
    defer { brush_string_free(s) }
    return String(cString: s)
}

private func encodeJSON(_ object: Any) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    return String(decoding: data, as: UTF8.self)
}

private func decodeJSON<T: Decodable>(_ type: T.Type, _ text: String?) -> T? {
    guard let text else { return nil }
    return try? JSONDecoder().decode(T.self, from: Data(text.utf8))
}

public enum BrushEngine {
    /// Engine and op-set versions.
    public static var version: (engine: String, ops: String) {
        struct V: Decodable { let engine: String; let ops: String }
        let v = decodeJSON(V.self, take(brush_version()))
        return (v?.engine ?? "?", v?.ops ?? "?")
    }

    /// Versioned capability manifest describing supported ops, formats, and precision features (Phase F).
    public static var capabilitiesJSON: String? {
        take(brush_capabilities())
    }

    /// Measures the exact 3D distance and per-axis delta from `a` to `b` (Phase E).
    public static func measureDistance(from a: SIMD3<Float>, to b: SIMD3<Float>) throws -> DistanceMeasurement {
        let pa = [a.x, a.y, a.z], pb = [b.x, b.y, b.z]
        let out = try attempt { err in brush_measure_distance(pa, pb, err) }
        guard let m = decodeJSON(DistanceMeasurement.self, take(out)) else {
            throw BrushEngineError(message: "could not read distance measurement")
        }
        return m
    }
}

/// Exact 3D distance and axis deltas between two points.
public struct DistanceMeasurement: Decodable, Equatable, Sendable {
    public let distance: Float
    public let delta: [Float]
}

/// Coordinate axis for alignment, distribution, and constraints (Phase E).
public enum AlignAxis: String, Codable, CaseIterable, Sendable {
    case x, y, z
}

/// Target alignment edge/center along an axis (Phase E).
public enum AlignMode: String, Codable, CaseIterable, Sendable {
    case min, center, max
}

/// A 2D construction plane in 3D world space for constrained numeric editing (Phase E).
public struct Workplane: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var origin: [Float]
    public var normal: [Float]
    public var xAxis: [Float]
    public var yAxis: [Float]

    public init(id: String, name: String, origin: [Float], normal: [Float], xAxis: [Float], yAxis: [Float]) {
        self.id = id
        self.name = name
        self.origin = origin
        self.normal = normal
        self.xAxis = xAxis
        self.yAxis = yAxis
    }

    public static func xz(y: Float = 0) -> Workplane {
        Workplane(id: "wp:xz", name: String(format: "Floor (Y = %.2fm)", y),
                  origin: [0, y, 0], normal: [0, 1, 0], xAxis: [1, 0, 0], yAxis: [0, 0, 1])
    }

    public static func xy(z: Float = 0) -> Workplane {
        Workplane(id: "wp:xy", name: String(format: "Front (Z = %.2fm)", z),
                  origin: [0, 0, z], normal: [0, 0, 1], xAxis: [1, 0, 0], yAxis: [0, 1, 0])
    }

    public static func yz(x: Float = 0) -> Workplane {
        Workplane(id: "wp:yz", name: String(format: "Side (X = %.2fm)", x),
                  origin: [x, 0, 0], normal: [1, 0, 0], xAxis: [0, 0, 1], yAxis: [0, 1, 0])
    }
}

/// Result of solving persistent constraints or updating a feature (Phase E).
public struct SolveReport: Decodable, Sendable {
    public struct Conflict: Decodable, Identifiable, Sendable {
        public let constraintId: String
        public let reason: String
        public var id: String { constraintId }
    }
    public let moved: [String]
    public let conflicts: [Conflict]
}

/// Connected component (island) of faces in the document's own mesh.
public struct MeshIsland: Decodable, Sendable {
    public let faces: [UInt32]
    public let vertices: Int
}

/// Result of detaching an island/box or combining instances into a placed object.
public struct DetachedPart: Decodable, Sendable {
    public let instanceId: String
    public let objectKey: String
    public let vertices: Int
    public let faces: Int
}

/// The result of an op.
public struct Applied: Decodable, Sendable {
    public struct Outcome: Decodable, Sendable { public let count: Int; public let vertices: Int; public let faces: Int }
    public let outcome: Outcome
    /// False when the op ran but changed nothing; nothing is recorded then.
    public let changed: Bool
    public let hash: String
}

public struct DocState: Decodable, Sendable {
    public struct Bounds: Decodable, Sendable { public let min: SIMD3<Float>; public let max: SIMD3<Float> }
    public let vertices: Int
    public let faces: Int
    public let hash: String
    public let bounds: Bounds?
    public let canUndo: Bool
    public let canRedo: Bool
    public let dirty: Bool
    public let lastOp: String?
    public let journalLength: Int
    /// Paint splats on the mesh; nil from engines before 0.4.
    public let splats: Int?
}

/// Volume, surface area, centroid and watertightness of the document's own mesh.
public struct Measure: Decodable, Sendable {
    /// Cubic meters, exact when `closed`.
    public let volume: Double
    /// Square meters.
    public let surfaceArea: Double
    /// Where the surface area balances, meters.
    public let centroid: SIMD3<Double>
    /// No boundary edges: the surface is watertight and `volume` is exact.
    public let closed: Bool
    /// Paint splats on the surface.
    public let splats: Int
}

/// A placed, linked instance of another object.
public struct Instance: Codable, Hashable, Sendable {
    public var id: String
    public var object: String
    public var position: SIMD3<Double>
    public var yawDeg: Double
    public var pitchDeg: Double
    public var rollDeg: Double
    public var scale: Double

    public init(id: String, object: String, position: SIMD3<Double>, yawDeg: Double = 0, pitchDeg: Double = 0, rollDeg: Double = 0, scale: Double = 1) {
        self.id = id
        self.object = object
        self.position = position
        self.yawDeg = yawDeg
        self.pitchDeg = pitchDeg
        self.rollDeg = rollDeg
        self.scale = scale
    }

    private enum CodingKeys: String, CodingKey { case id, object, position, yawDeg, pitchDeg, rollDeg, scale }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        object = try c.decode(String.self, forKey: .object)
        let p = try c.decode([Double].self, forKey: .position)
        guard p.count == 3 else { throw DecodingError.dataCorruptedError(forKey: .position, in: c, debugDescription: "position needs 3 numbers") }
        position = SIMD3(p[0], p[1], p[2])
        yawDeg = try c.decodeIfPresent(Double.self, forKey: .yawDeg) ?? 0
        pitchDeg = try c.decodeIfPresent(Double.self, forKey: .pitchDeg) ?? 0
        rollDeg = try c.decodeIfPresent(Double.self, forKey: .rollDeg) ?? 0
        scale = try c.decodeIfPresent(Double.self, forKey: .scale) ?? 1
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(object, forKey: .object)
        try c.encode([position.x, position.y, position.z], forKey: .position)
        try c.encode(yawDeg, forKey: .yawDeg)
        if pitchDeg != 0 { try c.encode(pitchDeg, forKey: .pitchDeg) }
        if rollDeg != 0 { try c.encode(rollDeg, forKey: .rollDeg) }
        try c.encode(scale, forKey: .scale)
    }
}

public struct ModelScene: Codable, Hashable, Sendable {
    public var instances: [Instance]
    public init(instances: [Instance] = []) { self.instances = instances }
}

/// What a ray met.
public struct Pick: Sendable {
    public let point: SIMD3<Float>
    public let normal: SIMD3<Float>
    public let t: Float
    public let face: UInt32
    /// The placed instance hit, or nil for the document's own mesh.
    public let instance: String?
}

/// Built-in pieces, and placed objects.
public enum Piece: String, Codable, CaseIterable, Sendable {
    case wall, floor, ramp, cube, cylinder, cone, sphere, object
}

/// A request to build one piece where the pointer is. The engine applies the rules (snapping, facing the camera,
/// stacking), so phone and web build the same thing in the same place.
public struct Placement: Encodable, Sendable {
    public var piece: Piece
    public var object: String?
    public var id: String?
    public var point: SIMD3<Double>
    public var normal: SIMD3<Double>
    public var view: SIMD3<Double>
    public var cell: Double
    public var snap = true
    public var ground = 0.0
    public var turns: UInt32 = 0
    public var span: SIMD3<UInt32> = [1, 1, 1]
    public var scale = 1.0
    public var color: SIMD3<UInt8> = [180, 180, 180]

    public init(piece: Piece, object: String? = nil, point: SIMD3<Double>, normal: SIMD3<Double>, view: SIMD3<Double>, cell: Double) {
        self.piece = piece
        self.object = object
        self.point = point
        self.normal = normal
        self.view = view
        self.cell = cell
    }

    private enum CodingKeys: String, CodingKey { case piece, object, id, point, normal, view, cell, snap, ground, turns, span, scale, color }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(piece, forKey: .piece)
        try c.encodeIfPresent(object, forKey: .object)
        try c.encodeIfPresent(id, forKey: .id)
        try c.encode([point.x, point.y, point.z], forKey: .point)
        try c.encode([normal.x, normal.y, normal.z], forKey: .normal)
        try c.encode([view.x, view.y, view.z], forKey: .view)
        try c.encode(cell, forKey: .cell)
        try c.encode(snap, forKey: .snap)
        try c.encode(ground, forKey: .ground)
        try c.encode(turns, forKey: .turns)
        try c.encode([span.x, span.y, span.z], forKey: .span)
        try c.encode(scale, forKey: .scale)
        try c.encode([color.x, color.y, color.z], forKey: .color)
    }
}

/// An op as JSON, with the fields a ghost preview needs.
public struct Op: Sendable, Hashable {
    public let json: String
    public let name: String
    /// For `placeObject`: where the instance would stand.
    public let object: String?
    public let id: String?
    public let position: SIMD3<Double>?
    public let yawDeg: Double
    public let scale: Double

    public init(json: String) {
        self.json = json
        let o = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
        name = o["op"] as? String ?? ""
        object = o["object"] as? String
        id = o["id"] as? String
        position = (o["position"] as? [Double]).flatMap { $0.count == 3 ? SIMD3($0[0], $0[1], $0[2]) : nil }
        yawDeg = o["yawDeg"] as? Double ?? 0
        scale = o["scale"] as? Double ?? 1
    }

    public init(_ object: [String: Any]) throws {
        self.init(json: try encodeJSON(object))
    }
}

/// Borrowed mesh buffers: valid only inside the closure they are passed to.
public struct MeshBuffers {
    public let positions: UnsafeBufferPointer<Float>
    public let colors: UnsafeBufferPointer<UInt8>
    public let indices: UnsafeBufferPointer<UInt32>
    public let normals: UnsafeBufferPointer<Float>
    public var vertexCount: Int { positions.count / 3 }
}

/// Borrowed paint splats: valid only inside the closure they are passed to.
public struct SplatBuffers {
    /// 8 floats per splat: x, y, z, radius (Gaussian sigma, meters), nx, ny, nz, drip.
    public let geometry: UnsafeBufferPointer<Float>
    /// 4 bytes per splat: r, g, b, opacity.
    public let colors: UnsafeBufferPointer<UInt8>
    public var count: Int { colors.count / 4 }
}

/// A mesh being edited (or viewed), with undo, redo, a journal of every change, and a scene of linked instances.
public final class EngineDocument: @unchecked Sendable {
    private let handle: OpaquePointer

    /// Bumped whenever the document's own mesh changes. Renderers compare it to know when to re-upload.
    public private(set) var meshRevision = 0
    /// Bumped when the mesh's faces may have changed (not during a sculpt stroke), so index buffers re-upload.
    public private(set) var topologyRevision = 0
    /// Bumped when the scene or an object's mesh changes.
    public private(set) var sceneRevision = 0

    /// Opens PLY or OBJ bytes. A read-only document shows and picks but refuses edits. `undoBudget` caps the memory
    /// undo may hold, in bytes (each step is a whole mesh); 0 keeps the engine's default depth.
    public init(bytes: Data, readOnly: Bool, undoBudget: Int = 0) throws {
        handle = try bytes.withUnsafeBytes { raw in
            try attempt { err in
                brush_document_open(raw.bindMemory(to: UInt8.self).baseAddress, raw.count, readOnly, undoBudget, err)
            }
        }
    }

    /// An empty, editable document: a scratch space, such as for a ghost preview.
    public init() {
        handle = brush_document_new()
    }

    deinit { brush_document_free(handle) }

    public var isReadOnly: Bool { brush_document_read_only(handle) }
    public var canUndo: Bool { brush_document_can_undo(handle) }
    public var canRedo: Bool { brush_document_can_redo(handle) }
    public var isDirty: Bool { brush_document_dirty(handle) }
    public var vertexCount: Int { brush_document_vertex_count(handle) }
    public var faceCount: Int { brush_document_face_count(handle) }
    /// Paint splats on the mesh (see the `splatPaint` op).
    public var splatCount: Int { brush_document_splat_count(handle) }
    public var state: DocState? { decodeJSON(DocState.self, take(brush_document_state(handle))) }
    /// Every change, one JSON object per line.
    public var journal: String { take(brush_document_journal(handle)) ?? "" }

    private func meshChanged(topology: Bool = true) {
        meshRevision += 1
        if topology { topologyRevision += 1 }
    }

    // MARK: Edits

    /// Applies one op. A `sculpt` op opens a stroke that `extendStroke` adds to.
    @discardableResult
    public func apply(_ op: Op, actor: [String: String]? = nil) throws -> Applied {
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            op.json.withCString { o in
                if let actorJSON {
                    return actorJSON.withCString { a in brush_document_apply(handle, o, a, err) }
                }
                return brush_document_apply(handle, o, nil, err)
            }
        }
        let applied = decodeJSON(Applied.self, take(out))
        if applied?.changed ?? true {
            // Scene ops leave the mesh alone, except baking; refreshing both is cheap next to what changed.
            meshChanged(topology: op.name != "sculpt")
            sceneRevision += 1
        }
        guard let applied else { throw BrushEngineError(message: "the engine answered with something unreadable") }
        return applied
    }

    @discardableResult
    public func apply(_ op: [String: Any], actor: [String: String]? = nil) throws -> Applied {
        try apply(Op(op), actor: actor)
    }

    /// Adds dabs to the open stroke: the same result as one op holding them all, and still one undo step.
    /// Returns whether the mesh changed.
    @discardableResult
    public func extendStroke(centers: [SIMD3<Double>], pressures: [Double]? = nil) throws -> Bool {
        guard !centers.isEmpty else { return false }
        let flat = centers.flatMap { [$0.x, $0.y, $0.z] }
        let result: Int32 = try attempt { err in
            let r = flat.withUnsafeBufferPointer { c in
                if let pressures {
                    return pressures.withUnsafeBufferPointer { p in
                        brush_document_extend_stroke(handle, c.baseAddress, p.baseAddress, centers.count, err)
                    }
                }
                return brush_document_extend_stroke(handle, c.baseAddress, nil, centers.count, err)
            }
            return r < 0 ? nil : r
        }
        if result == 1 { meshChanged(topology: false) }
        return result == 1
    }

    public func endStroke() { brush_document_end_stroke(handle) }

    @discardableResult public func undo() -> Bool { changed(brush_document_undo(handle)) }
    @discardableResult public func redo() -> Bool { changed(brush_document_redo(handle)) }
    /// Back to the mesh and scene as opened or last saved. Undoable.
    @discardableResult public func revert() -> Bool { changed(brush_document_revert(handle)) }
    public func markSaved() { brush_document_mark_saved(handle) }

    private func changed(_ did: Bool) -> Bool {
        if did {
            meshChanged()
            sceneRevision += 1
        }
        return did
    }

    // MARK: Mesh

    /// The document's own mesh, borrowed for the duration of `body`.
    public func withMesh<R>(_ body: (MeshBuffers) throws -> R) rethrows -> R {
        var np = 0, nc = 0, ni = 0, nn = 0
        let n = brush_document_normals(handle, &nn) // computed first: it may update the engine's cached normals
        let p = brush_document_positions(handle, &np)
        let c = brush_document_colors(handle, &nc)
        let i = brush_document_indices(handle, &ni)
        return try body(MeshBuffers(
            positions: UnsafeBufferPointer(start: p, count: np),
            colors: UnsafeBufferPointer(start: c, count: nc),
            indices: UnsafeBufferPointer(start: i, count: ni),
            normals: UnsafeBufferPointer(start: n, count: nn)
        ))
    }

    /// The document's paint splats, borrowed for the duration of `body`. Empty buffers when there are none.
    public func withSplats<R>(_ body: (SplatBuffers) throws -> R) rethrows -> R {
        var ng = 0, nc = 0
        let g = brush_document_splats(handle, &ng)
        let c = brush_document_splat_colors(handle, &nc)
        return try body(SplatBuffers(
            geometry: UnsafeBufferPointer(start: g, count: ng),
            colors: UnsafeBufferPointer(start: c, count: nc)
        ))
    }

    /// Volume, surface area, centroid and closedness of the document's own mesh.
    public func measure() -> Measure? { decodeJSON(Measure.self, take(brush_document_measure(handle))) }

    /// An object's mesh, borrowed for the duration of `body`; nil when it has not been supplied.
    public func withObject<R>(_ object: String, _ body: (MeshBuffers) throws -> R) rethrows -> R? {
        try object.withCString { key in
            var np = 0, nc = 0, ni = 0, nn = 0
            let n = brush_document_object_normals(handle, key, &nn)
            let p = brush_document_object_positions(handle, key, &np)
            guard np > 0 else { return nil }
            let c = brush_document_object_colors(handle, key, &nc)
            let i = brush_document_object_indices(handle, key, &ni)
            return try body(MeshBuffers(
                positions: UnsafeBufferPointer(start: p, count: np),
                colors: UnsafeBufferPointer(start: c, count: nc),
                indices: UnsafeBufferPointer(start: i, count: ni),
                normals: UnsafeBufferPointer(start: n, count: nn)
            ))
        }
    }

    /// The first point on the document's own mesh along the ray.
    public func raycast(origin: SIMD3<Float>, direction: SIMD3<Float>) -> Pick? {
        var hit = BrushHit()
        let o = [origin.x, origin.y, origin.z], d = [direction.x, direction.y, direction.z]
        guard brush_document_raycast(handle, o, d, &hit) else { return nil }
        return Pick(point: SIMD3(hit.point.0, hit.point.1, hit.point.2), normal: SIMD3(hit.normal.0, hit.normal.1, hit.normal.2), t: hit.t, face: hit.face, instance: nil)
    }

    /// The nearest surface along the ray: the document's own mesh or a placed instance.
    public func pick(origin: SIMD3<Float>, direction: SIMD3<Float>) -> Pick? {
        var hit = BrushHit()
        var instance: UnsafeMutablePointer<CChar>?
        let o = [origin.x, origin.y, origin.z], d = [direction.x, direction.y, direction.z]
        guard brush_document_pick(handle, o, d, &hit, &instance) else { return nil }
        return Pick(point: SIMD3(hit.point.0, hit.point.1, hit.point.2), normal: SIMD3(hit.normal.0, hit.normal.1, hit.normal.2), t: hit.t, face: hit.face, instance: take(instance))
    }

    /// The mesh as a file: "ply", "obj" or "glb". `flattened` copies placed objects in (for downloads); without it
    /// this is the document's own mesh, which is what is saved beside the scene.
    public func export(_ format: String, flattened: Bool = false) throws -> Data {
        var len = 0
        let bytes: UnsafeMutablePointer<UInt8> = try attempt { err in
            format.withCString { brush_document_export(handle, $0, flattened, &len, err) }
        }
        defer { brush_bytes_free(bytes, len) }
        return Data(bytes: bytes, count: len)
    }

    // MARK: Scene

    public var scene: ModelScene { decodeJSON(ModelScene.self, take(brush_document_scene(handle))) ?? ModelScene() }

    /// Loads a stored scene as the saved state. Call right after opening.
    public func loadScene(_ scene: ModelScene) throws {
        let text = String(decoding: try JSONEncoder().encode(scene), as: UTF8.self)
        _ = try attempt { err in text.withCString { brush_document_load_scene(handle, $0, err) ? true : nil } }
        sceneRevision += 1
    }

    /// Supplies or refreshes the mesh behind an object pointer ("scan:<id>"): every instance of it updates at once.
    public func setObject(_ object: String, bytes: Data) throws {
        _ = try attempt { err in
            object.withCString { key in
                bytes.withUnsafeBytes { raw in
                    brush_document_set_object(handle, key, raw.bindMemory(to: UInt8.self).baseAddress, raw.count, err) ? true : nil
                }
            }
        }
        sceneRevision += 1
    }

    /// Whether the mesh behind an object pointer has been supplied.
    public func hasObject(_ object: String) -> Bool {
        var n = 0
        _ = object.withCString { brush_document_object_positions(handle, $0, &n) }
        return n > 0
    }

    /// Objects the scene points at whose meshes have not been supplied yet.
    public var missingObjects: [String] { decodeJSON([String].self, take(brush_document_missing_objects(handle))) ?? [] }

    /// A placed instance's object-to-world transform.
    public func instanceTransform(_ id: String) -> simd_float4x4? {
        var m = [Float](repeating: 0, count: 16)
        guard id.withCString({ brush_document_instance_transform(handle, $0, &m) }) else { return nil }
        return matrix(m)
    }

    /// Where an object would be drawn for a placement (a ghost).
    public func objectTransform(_ object: String, position: SIMD3<Double>, yawDeg: Double, scale: Double) -> simd_float4x4? {
        var m = [Float](repeating: 0, count: 16)
        let p = [position.x, position.y, position.z]
        guard object.withCString({ brush_document_object_transform(handle, $0, p, yawDeg, scale, &m) }) else { return nil }
        return matrix(m)
    }

    private func matrix(_ m: [Float]) -> simd_float4x4 {
        simd_float4x4(columns: (SIMD4(m[0], m[1], m[2], m[3]), SIMD4(m[4], m[5], m[6], m[7]),
                                SIMD4(m[8], m[9], m[10], m[11]), SIMD4(m[12], m[13], m[14], m[15])))
    }

    /// The op that builds a piece where the pointer is. Nothing changes until it is applied.
    public func place(_ placement: Placement) throws -> Op {
        let text = String(decoding: try JSONEncoder().encode(placement), as: UTF8.self)
        let out = try attempt { err in text.withCString { brush_document_place(handle, $0, err) } }
        return Op(json: take(out) ?? "{}")
    }

    // MARK: Transactions & revision (Phase B)

    public var revision: UInt64 { brush_document_revision(handle) }

    public func beginTransaction(label: String? = nil, actor: [String: String]? = nil) throws {
        let actorJSON = try actor.map { try encodeJSON($0) }
        _ = try attempt { err in
            let run: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Bool? = { l, a in
                brush_document_begin_transaction(self.handle, l, a, err) ? true : nil
            }
            if let label, let actorJSON {
                return label.withCString { l in actorJSON.withCString { a in run(l, a) } }
            } else if let label {
                return label.withCString { l in run(l, nil) }
            } else if let actorJSON {
                return actorJSON.withCString { a in run(nil, a) }
            }
            return run(nil, nil)
        }
    }

    @discardableResult
    public func commitTransaction() throws -> Applied {
        let out = try attempt { err in brush_document_commit_transaction(handle, err) }
        let applied = decodeJSON(Applied.self, take(out))
        if applied?.changed ?? true {
            meshChanged()
            sceneRevision += 1
        }
        guard let applied else { throw BrushEngineError(message: "the engine answered with something unreadable") }
        return applied
    }

    @discardableResult
    public func cancelTransaction() throws -> Bool {
        let did = try attempt { err in brush_document_cancel_transaction(handle, err) ? true : nil }
        if did {
            meshChanged()
            sceneRevision += 1
        }
        return did
    }

    // MARK: Islands & detach/combine (Phase B/E)

    public func connectedComponent(startFace: UInt32) -> MeshIsland? {
        decodeJSON(MeshIsland.self, take(brush_document_connected_component(handle, startFace)))
    }

    @discardableResult
    public func deleteIsland(startFace: UInt32, actor: [String: String]? = nil) throws -> Applied {
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            if let actorJSON {
                return actorJSON.withCString { brush_document_delete_island(handle, startFace, $0, err) }
            }
            return brush_document_delete_island(handle, startFace, nil, err)
        }
        let applied = decodeJSON(Applied.self, take(out))
        if applied?.changed ?? true { meshChanged() }
        guard let applied else { throw BrushEngineError(message: "the engine answered with something unreadable") }
        return applied
    }

    @discardableResult
    public func detachIsland(startFace: UInt32, tangled: Bool = false, objectKey: String, instanceID: String? = nil, actor: [String: String]? = nil) throws -> DetachedPart {
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            objectKey.withCString { key in
                let call: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? = { inst, act in
                    brush_document_detach_island(self.handle, startFace, tangled, key, inst, act, err)
                }
                if let instanceID, let actorJSON {
                    return instanceID.withCString { i in actorJSON.withCString { a in call(i, a) } }
                } else if let instanceID {
                    return instanceID.withCString { i in call(i, nil) }
                } else if let actorJSON {
                    return actorJSON.withCString { a in call(nil, a) }
                }
                return call(nil, nil)
            }
        }
        meshChanged()
        sceneRevision += 1
        guard let part = decodeJSON(DetachedPart.self, take(out)) else {
            throw BrushEngineError(message: "could not decode detached part")
        }
        return part
    }

    @discardableResult
    public func combineInstances(_ ids: [String], objectKey: String, instanceID: String? = nil, actor: [String: String]? = nil) throws -> DetachedPart {
        let idsJSON = String(decoding: try JSONEncoder().encode(ids), as: UTF8.self)
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            idsJSON.withCString { idsPtr in
                objectKey.withCString { key in
                    let call: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? = { inst, act in
                        brush_document_combine_instances(self.handle, idsPtr, key, inst, act, err)
                    }
                    if let instanceID, let actorJSON {
                        return instanceID.withCString { i in actorJSON.withCString { a in call(i, a) } }
                    } else if let instanceID {
                        return instanceID.withCString { i in call(i, nil) }
                    } else if let actorJSON {
                        return actorJSON.withCString { a in call(nil, a) }
                    }
                    return call(nil, nil)
                }
            }
        }
        meshChanged()
        sceneRevision += 1
        guard let part = decodeJSON(DetachedPart.self, take(out)) else {
            throw BrushEngineError(message: "could not decode combined part")
        }
        return part
    }

    // MARK: Precision: workplanes, alignment, editable features, constraints (Phase E)

    public var workplane: Workplane {
        decodeJSON(Workplane.self, take(brush_document_workplane(handle))) ?? .xz()
    }

    public func setWorkplane(_ workplane: Workplane) throws {
        let text = String(decoding: try JSONEncoder().encode(workplane), as: UTF8.self)
        _ = try attempt { err in text.withCString { brush_document_set_workplane(handle, $0, err) ? true : nil } }
    }

    @discardableResult
    public func alignSelection(_ ids: [String], axis: AlignAxis, mode: AlignMode, actor: [String: String]? = nil) throws -> Int {
        let idsJSON = String(decoding: try JSONEncoder().encode(ids), as: UTF8.self)
        let axisJSON = String(decoding: try JSONEncoder().encode(axis), as: UTF8.self)
        let modeJSON = String(decoding: try JSONEncoder().encode(mode), as: UTF8.self)
        let actorJSON = try actor.map { try encodeJSON($0) }
        let moved: Int32 = try attempt { err in
            let r = idsJSON.withCString { i in
                axisJSON.withCString { ax in
                    modeJSON.withCString { m in
                        if let actorJSON {
                            return actorJSON.withCString { a in brush_document_align_selection(handle, i, ax, m, a, err) }
                        }
                        return brush_document_align_selection(handle, i, ax, m, nil, err)
                    }
                }
            }
            return r < 0 ? nil : r
        }
        if moved > 0 { sceneRevision += 1 }
        return Int(moved)
    }

    @discardableResult
    public func distributeSelection(_ ids: [String], axis: AlignAxis, actor: [String: String]? = nil) throws -> Int {
        let idsJSON = String(decoding: try JSONEncoder().encode(ids), as: UTF8.self)
        let axisJSON = String(decoding: try JSONEncoder().encode(axis), as: UTF8.self)
        let actorJSON = try actor.map { try encodeJSON($0) }
        let moved: Int32 = try attempt { err in
            let r = idsJSON.withCString { i in
                axisJSON.withCString { ax in
                    if let actorJSON {
                        return actorJSON.withCString { a in brush_document_distribute_selection(handle, i, ax, a, err) }
                    }
                    return brush_document_distribute_selection(handle, i, ax, nil, err)
                }
            }
            return r < 0 ? nil : r
        }
        if moved > 0 { sceneRevision += 1 }
        return Int(moved)
    }

    public func listFeaturesJSON() -> String {
        take(brush_document_list_features(handle)) ?? "[]"
    }

    @discardableResult
    public func addFeature(_ spec: [String: Any], actor: [String: String]? = nil) throws -> String {
        let specJSON = try encodeJSON(spec)
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            specJSON.withCString { s in
                if let actorJSON {
                    return actorJSON.withCString { a in brush_document_add_feature(handle, s, a, err) }
                }
                return brush_document_add_feature(handle, s, nil, err)
            }
        }
        meshChanged()
        sceneRevision += 1
        return take(out) ?? "{}"
    }

    @discardableResult
    public func bakeFeature(id: String, actor: [String: String]? = nil) throws -> String {
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            id.withCString { i in
                if let actorJSON {
                    return actorJSON.withCString { a in brush_document_bake_feature(handle, i, a, err) }
                }
                return brush_document_bake_feature(handle, i, nil, err)
            }
        }
        meshChanged()
        sceneRevision += 1
        return take(out) ?? "{}"
    }

    public func listConstraintsJSON() -> String {
        take(brush_document_list_constraints(handle)) ?? "[]"
    }

    @discardableResult
    public func addConstraint(_ constraint: [String: Any], actor: [String: String]? = nil) throws -> SolveReport {
        let cJSON = try encodeJSON(constraint)
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            cJSON.withCString { c in
                if let actorJSON {
                    return actorJSON.withCString { a in brush_document_add_constraint(handle, c, a, err) }
                }
                return brush_document_add_constraint(handle, c, nil, err)
            }
        }
        sceneRevision += 1
        guard let report = decodeJSON(SolveReport.self, take(out)) else {
            throw BrushEngineError(message: "could not decode solve report")
        }
        return report
    }

    @discardableResult
    public func removeConstraint(id: String) throws -> Bool {
        try attempt { err in id.withCString { brush_document_remove_constraint(handle, $0, err) ? true : nil } }
    }

    @discardableResult
    public func solveConstraints(actor: [String: String]? = nil) throws -> SolveReport {
        let actorJSON = try actor.map { try encodeJSON($0) }
        let out = try attempt { err in
            if let actorJSON {
                return actorJSON.withCString { a in brush_document_solve_constraints(handle, a, err) }
            }
            return brush_document_solve_constraints(handle, nil, err)
        }
        sceneRevision += 1
        guard let report = decodeJSON(SolveReport.self, take(out)) else {
            throw BrushEngineError(message: "could not decode solve report")
        }
        return report
    }
}
