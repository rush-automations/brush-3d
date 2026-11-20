//! Building: turning "put a wall here" into an op.
//!
//! An editor sends where the pointer meets the surface (or the ground), the surface normal there, the way the camera
//! looks and a grid; [`crate::Document::place`] answers with the op that builds the piece. The rules live here, in the
//! engine, so a tap on a phone and a click in a browser build the same thing in the same place:
//!
//! - Pieces face the camera, turned in quarter turns (plus any extra `turns`), so walls stand across the view and
//!   ramps climb away from it.
//! - Aimed at a floor or the top of a piece, a piece stands on it; aimed at a ceiling it hangs below; aimed at a side
//!   it sits against it. That is what makes stacking and building outward work.
//! - With `snap`, footprints line up with the grid (a piece an odd number of cells wide sits mid-cell, an even one on
//!   a grid line), walls go on the near edge of their cell, and heights close to a grid level land exactly on it.
//!
//! Built-in pieces become geometry (`addBox`, `addRamp`, `addFrustum`, `addSphere`); objects become linked instances
//! (`placeObject`), so later edits to the object reach every copy.

use brush_mesh::{BoxRegion, Op, OpError};
use serde::{Deserialize, Serialize};

/// What to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Piece {
    /// A thin panel standing across the view.
    Wall,
    /// A thin panel lying flat.
    Floor,
    /// A wedge climbing away from the view.
    Ramp,
    Cube,
    Cylinder,
    Cone,
    Sphere,
    /// Another object, placed as a linked instance (`object` names it).
    Object,
}

/// A request to build one piece.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Placement {
    pub piece: Piece,
    /// For `object`: the object to place, such as `"scan:<id>"`. It must be set in the document already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// For `object`: the instance's id. A free one (`o1`, `o2`, ...) when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Where the pointer meets the surface or the ground, meters.
    pub point: [f64; 3],
    /// The surface normal there. Up when absent (the ground).
    #[serde(default = "up")]
    pub normal: [f64; 3],
    /// The camera's viewing direction.
    pub view: [f64; 3],
    /// Grid cell, meters. Pieces are a cell (times `span`) in size.
    pub cell: f64,
    #[serde(default = "yes")]
    pub snap: bool,
    /// Height of the grid's ground level.
    #[serde(default)]
    pub ground: f64,
    /// Extra quarter turns about +y.
    #[serde(default)]
    pub turns: u32,
    /// Cells across, up and deep (in the piece's own frame). Ignored for objects.
    #[serde(default = "ones")]
    pub span: [u32; 3],
    /// For objects: size multiplier (1 is the object's real size).
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default = "light_grey")]
    pub color: [u8; 3],
}

fn up() -> [f64; 3] {
    [0.0, 1.0, 0.0]
}
fn yes() -> bool {
    true
}
fn ones() -> [u32; 3] {
    [1; 3]
}
fn one() -> f64 {
    1.0
}
fn light_grey() -> [u8; 3] {
    [180; 3]
}

/// Panels are this fraction of a cell thick.
const PANEL: f64 = 0.1;

fn invalid(msg: &str) -> OpError {
    OpError::Invalid(msg.into())
}

/// Micrometre rounding, so journals read `0.5`, not `0.49999999999999994`.
fn tidy(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// Which way the camera looks, in quarter turns from +z (0: +z, 1: +x, 2: -z, 3: -x).
fn quarter(view: [f64; 3]) -> u32 {
    if view[0].abs() > view[2].abs() {
        if view[0] > 0.0 {
            1
        } else {
            3
        }
    } else if view[2] >= 0.0 {
        0
    } else {
        2
    }
}

/// Snaps a centre so a footprint `cells` wide covers whole cells.
fn snap_axis(v: f64, cell: f64, cells: f64) -> f64 {
    let n = cells.round().max(1.0);
    if n % 2.0 == 1.0 {
        ((v / cell).floor() + 0.5) * cell
    } else {
        (v / cell).round() * cell
    }
}

/// The op that builds `p`. `object_size` gives an object's full size (unscaled) when it is known.
pub fn place(
    p: &Placement,
    object_size: Option<[f32; 3]>,
    free_id: impl FnOnce() -> String,
) -> Result<Op, OpError> {
    let all = p
        .point
        .iter()
        .chain(&p.normal)
        .chain(&p.view)
        .chain([&p.cell, &p.ground, &p.scale]);
    if all.into_iter().any(|v| !v.is_finite()) {
        return Err(invalid("placement values must be finite numbers"));
    }
    if p.cell <= 0.0 {
        return Err(invalid("cell must be positive"));
    }
    if p.scale <= 0.0 {
        return Err(invalid("scale must be positive"));
    }
    if p.span.iter().any(|&s| s == 0 || s > 64) {
        return Err(invalid("span must be 1 to 64 cells"));
    }
    let g = p.cell;
    let [sx, sy, sz] = p.span.map(f64::from);
    let q = (quarter(p.view) + p.turns) % 4;
    let yaw_deg = 90.0 * f64::from(q);
    // The camera's way in world x and z: where local +z points after the turn.
    let fwd = [[0.0, 1.0], [1.0, 0.0], [0.0, -1.0], [-1.0, 0.0]][q as usize];

    // Size in the piece's own frame: across (x), up (y), deep (z).
    let (w, h, d) = match p.piece {
        Piece::Wall => (sx * g, sy * g, g * PANEL),
        Piece::Floor => (sx * g, g * PANEL, sz * g),
        Piece::Ramp | Piece::Cube | Piece::Sphere => (sx * g, sy * g, sz * g),
        Piece::Cylinder | Piece::Cone => (sx * g, sy * g, sx * g),
        Piece::Object => {
            let s = object_size.ok_or_else(|| {
                OpError::MissingObject(p.object.clone().unwrap_or_else(|| "(none)".into()))
            })?;
            (
                f64::from(s[0]) * p.scale,
                f64::from(s[1]) * p.scale,
                f64::from(s[2]) * p.scale,
            )
        }
    };
    // Extent along world x and z after the turn.
    let (ex, ez) = if q % 2 == 0 { (w, d) } else { (d, w) };

    let len =
        (p.normal[0] * p.normal[0] + p.normal[1] * p.normal[1] + p.normal[2] * p.normal[2]).sqrt();
    let n = if len > 1e-9 {
        p.normal.map(|v| v / len)
    } else {
        up()
    };
    let mut b = p.point; // where the piece's bottom centre goes
    let side = n[1].abs() < 0.5;
    if n[1] <= -0.5 {
        b[1] -= h;
    } else if side {
        let hl = (n[0] * n[0] + n[2] * n[2]).sqrt();
        let (nx, nz) = (n[0] / hl, n[2] / hl);
        let reach = nx.abs() * ex / 2.0 + nz.abs() * ez / 2.0;
        b[0] += nx * reach;
        b[2] += nz * reach;
        b[1] -= h / 2.0;
    }

    if p.snap {
        if p.piece == Piece::Wall {
            // A wall covers a row of cells one cell deep, on the edge nearest the camera.
            let (cx, cz) = if q % 2 == 0 { (sx, 1.0) } else { (1.0, sx) };
            b[0] = snap_axis(b[0], g, cx) - fwd[0] * g / 2.0;
            b[2] = snap_axis(b[2], g, cz) - fwd[1] * g / 2.0;
        } else {
            b[0] = snap_axis(b[0], g, ex / g);
            b[2] = snap_axis(b[2], g, ez / g);
        }
        let level = p.ground + ((b[1] - p.ground) / g).round() * g;
        if (b[1] - level).abs() <= 0.3 * g {
            b[1] = level;
        }
    }
    let b = b.map(tidy);
    let (w, h, d) = (tidy(w), tidy(h), tidy(d));
    let mid = [b[0], tidy(b[1] + h / 2.0), b[2]];
    let region = BoxRegion {
        center: mid,
        size: [w, h, d],
        yaw_deg,
    };
    let color = p.color;
    Ok(match p.piece {
        Piece::Wall | Piece::Floor | Piece::Cube => Op::AddBox { region, color },
        Piece::Ramp => Op::AddRamp { region, color },
        Piece::Cylinder | Piece::Cone => Op::AddFrustum {
            base: b,
            radius_bottom: tidy(w / 2.0),
            radius_top: if p.piece == Piece::Cone {
                0.0
            } else {
                tidy(w / 2.0)
            },
            height: h,
            segments: 24,
            color,
        },
        Piece::Sphere => Op::AddSphere {
            center: mid,
            radii: [tidy(w / 2.0), tidy(h / 2.0), tidy(d / 2.0)],
            segments: 24,
            rings: 12,
            color,
        },
        Piece::Object => Op::PlaceObject {
            id: p.id.clone().unwrap_or_else(free_id),
            object: p.object.clone().unwrap_or_default(),
            position: b,
            yaw_deg,
            scale: p.scale,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(piece: Piece) -> Placement {
        Placement {
            piece,
            object: None,
            id: None,
            point: [0.37, 0.02, 1.62],
            normal: [0.0, 1.0, 0.0],
            view: [0.2, -0.5, 0.9],
            cell: 1.0,
            snap: true,
            ground: 0.0,
            turns: 0,
            span: [1, 1, 1],
            scale: 1.0,
            color: [1, 2, 3],
        }
    }

    fn boxed(op: Op) -> BoxRegion {
        match op {
            Op::AddBox { region, .. } | Op::AddRamp { region, .. } => region,
            other => panic!("not a box: {other:?}"),
        }
    }

    #[test]
    fn a_cube_snaps_into_its_cell_on_the_ground() {
        let b = boxed(place(&req(Piece::Cube), None, || unreachable!()).unwrap());
        assert_eq!(b.center, [0.5, 0.5, 1.5]);
        assert_eq!(b.size, [1.0, 1.0, 1.0]);
        assert_eq!(b.yaw_deg, 0.0);
    }

    #[test]
    fn walls_stand_on_the_near_edge_across_the_view() {
        let b = boxed(place(&req(Piece::Wall), None, || unreachable!()).unwrap());
        // Looking along +z, the cell 0..1 x 1..2 has its near edge at z = 1.
        assert_eq!(b.center, [0.5, 0.5, 1.0]);
        assert_eq!(b.size, [1.0, 1.0, 0.1]);
        let mut r = req(Piece::Wall);
        r.view = [-1.0, 0.0, 0.1];
        let b = boxed(place(&r, None, || unreachable!()).unwrap());
        assert_eq!(b.yaw_deg, 270.0);
        assert_eq!(b.center, [1.0, 0.5, 1.5]); // near edge when looking along -x is x = 1
    }

    #[test]
    fn pieces_attach_to_sides_and_stack_on_tops() {
        // The +x side of a cube in cell 0..1: the new cube goes in the next cell.
        let mut r = req(Piece::Cube);
        r.point = [1.0, 0.4, 1.5];
        r.normal = [1.0, 0.0, 0.0];
        assert_eq!(
            boxed(place(&r, None, || unreachable!()).unwrap()).center,
            [1.5, 0.5, 1.5]
        );
        // Its top: the new cube stands on it, at grid level 1.
        r.point = [0.5, 1.0, 1.5];
        r.normal = [0.0, 1.0, 0.0];
        assert_eq!(
            boxed(place(&r, None, || unreachable!()).unwrap()).center,
            [0.5, 1.5, 1.5]
        );
    }

    #[test]
    fn even_spans_sit_on_grid_lines_and_turns_add_up() {
        let mut r = req(Piece::Floor);
        r.span = [2, 1, 2];
        r.turns = 1;
        let b = boxed(place(&r, None, || unreachable!()).unwrap());
        assert_eq!(b.center, [0.0, 0.05, 2.0]);
        assert_eq!(b.yaw_deg, 90.0);
    }

    #[test]
    fn without_snap_the_piece_sits_where_aimed() {
        let mut r = req(Piece::Cube);
        r.snap = false;
        assert_eq!(
            boxed(place(&r, None, || unreachable!()).unwrap()).center,
            [0.37, 0.52, 1.62]
        );
    }

    #[test]
    fn objects_become_linked_instances() {
        let mut r = req(Piece::Object);
        r.object = Some("scan:chair".into());
        r.snap = false;
        r.scale = 2.0;
        let op = place(&r, Some([0.5, 1.0, 0.5]), || "o7".into()).unwrap();
        assert_eq!(
            op,
            Op::PlaceObject {
                id: "o7".into(),
                object: "scan:chair".into(),
                position: [0.37, 0.02, 1.62],
                yaw_deg: 0.0,
                scale: 2.0
            }
        );
        assert!(matches!(
            place(&r, None, || "x".into()),
            Err(OpError::MissingObject(_))
        ));
    }
}
