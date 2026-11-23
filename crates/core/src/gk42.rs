//! GK-42 map calibration: three reference markers on the page map the
//! page millimetres to true GK-42 metres (Gauss-Kruger, the Pulkovo
//! 1942 grid). Three markers fix an affine map: a turn, a scale along
//! each page axis and an offset, so a map that is turned or stretched
//! on the sheet still lines up. A calibration saved with two markers
//! keeps the older two-marker model, which has no turn.
//!
//! The page's X axis (right) maps to the GK-42 Y axis (easting) and the
//! page's Y axis (up) to the GK-42 X axis (northing).
//!
//! The markers are small shapes on a layer of their own; their page
//! positions are read from the document by the caller, so moving a
//! marker with any tool recalibrates at once and the calibration data
//! itself carries only the true coordinates.

use crate::geometry::Point;
use crate::id::{LayerId, ShapeId};
use serde::{Deserialize, Serialize};

/// A GK-42 coordinate in metres: `x` is the northing (the grid's X
/// axis, kilometres north of the equator) and `y` the easting (the
/// grid's Y axis, whose leading digits carry the zone number).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GkPoint {
    pub x: f64,
    pub y: f64,
}

/// The reference markers and their true GK-42 coordinates. Marker 1
/// is the bottom-left one, marker 2 the top-right one, marker 3 the
/// third corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GkCalibration {
    /// The layer the markers live on (the "GK-42 markers" layer).
    pub layer: LayerId,
    /// The first reference marker.
    pub marker1: ShapeId,
    /// The second reference marker.
    pub marker2: ShapeId,
    /// True GK-42 metres of marker 1.
    pub gk1: GkPoint,
    /// True GK-42 metres of marker 2.
    pub gk2: GkPoint,
    /// The third reference marker; absent in calibrations saved with
    /// two markers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker3: Option<ShapeId>,
    /// True GK-42 metres of marker 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gk3: Option<GkPoint>,
}

/// The resolved page-to-GK-42 transform, affine in the page's millimetres
/// (y up): `E = e[0] x + e[1] y + e[2]` and `N = n[0] x + n[1] y + n[2]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GkTransform {
    /// Easting in metres: the coefficients of x, of y and the offset.
    pub e: [f64; 3],
    /// Northing in metres: the coefficients of x, of y and the offset.
    pub n: [f64; 3],
}

impl GkCalibration {
    /// The zone number in the leading digits of the eastings (every
    /// marker must agree): `None` when they disagree or a value is out
    /// of range.
    pub fn zone(&self) -> Option<u32> {
        let mut zone = None;
        for g in [Some(self.gk1), Some(self.gk2), self.gk3].into_iter().flatten() {
            let z = zone_of(g.y)?;
            match zone {
                None => zone = Some(z),
                Some(prev) if prev != z => return None,
                Some(_) => {}
            }
        }
        zone
    }

    /// The transform from the markers' page positions (millimetres,
    /// y-up). With the third marker placed, `p3` is its position and the
    /// map is affine; without it, the two-marker model. `None` when the
    /// markers coincide or sit on one line, or the implied map is
    /// mirrored or degenerate.
    pub fn transform_at(&self, p1: Point, p2: Point, p3: Option<Point>) -> Option<GkTransform> {
        match (self.gk3, p3) {
            (Some(g3), Some(p3)) => GkTransform::fit(&[p1, p2, p3], &[self.gk1, self.gk2, g3]),
            (Some(_), None) => None,
            (None, _) => GkTransform::fit(&[p1, p2], &[self.gk1, self.gk2]),
        }
    }
}

impl GkTransform {
    /// The transform through the markers: three page points with their
    /// GK-42 metres give the affine map, two give the map without a turn.
    /// `None` when the points are degenerate or the map is mirrored.
    pub fn fit(pages: &[Point], gk: &[GkPoint]) -> Option<GkTransform> {
        match (pages, gk) {
            ([p1, p2], [g1, g2]) => axis_fit(*p1, *p2, *g1, *g2),
            ([p1, p2, p3], [g1, g2, g3]) => affine_fit([*p1, *p2, *p3], [*g1, *g2, *g3]),
            _ => None,
        }
    }

    /// The GK-42 metres of a page point (millimetres, y-up).
    pub fn to_gk(&self, p: Point) -> GkPoint {
        GkPoint {
            x: self.n[0] * p.x + self.n[1] * p.y + self.n[2],
            y: self.e[0] * p.x + self.e[1] * p.y + self.e[2],
        }
    }

    /// The page point of GK-42 metres, the inverse of [`to_gk`].
    pub fn from_gk(&self, g: GkPoint) -> Option<Point> {
        let det = self.e[0] * self.n[1] - self.e[1] * self.n[0];
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let u = g.y - self.e[2];
        let v = g.x - self.n[2];
        Some(Point::new(
            (self.n[1] * u - self.e[1] * v) / det,
            (self.e[0] * v - self.n[0] * u) / det,
        ))
    }

    /// The map metres between two page points, measured with the scales
    /// of both axes and the turn of the map.
    pub fn distance_m(&self, a: Point, b: Point) -> f64 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let de = self.e[0] * dx + self.e[1] * dy;
        let dn = self.n[0] * dx + self.n[1] * dy;
        (de * de + dn * dn).sqrt()
    }

    /// The map scale along the page's X axis as 1:N (N metres of map
    /// per paper millimetre, times 1000).
    pub fn scale_x(&self) -> f64 {
        self.e[0].hypot(self.n[0]) * 1000.0
    }

    /// The map scale along the page's Y axis as 1:N.
    pub fn scale_y(&self) -> f64 {
        self.e[1].hypot(self.n[1]) * 1000.0
    }

    /// How much the two axis scales differ, in percent of the X one:
    /// the map's deformation.
    pub fn deformation(&self) -> f64 {
        let sx = self.e[0].hypot(self.n[0]);
        if sx == 0.0 {
            return 0.0;
        }
        let sy = self.e[1].hypot(self.n[1]);
        ((sy - sx) / sx).abs() * 100.0
    }
}

/// The two-marker model: an independent scale and offset per axis, no turn.
fn axis_fit(p1: Point, p2: Point, g1: GkPoint, g2: GkPoint) -> Option<GkTransform> {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    if !dx.is_finite() || !dy.is_finite() || dx == 0.0 || dy == 0.0 {
        return None;
    }
    let east = (g2.y - g1.y) / dx;
    let north = (g2.x - g1.x) / dy;
    if !east.is_finite() || !north.is_finite() || east <= 0.0 || north <= 0.0 {
        return None;
    }
    Some(GkTransform {
        e: [east, 0.0, g1.y - east * p1.x],
        n: [0.0, north, g1.x - north * p1.y],
    })
}

/// The three-marker model: the affine map through three points. A
/// mirrored map (negative determinant) is refused.
fn affine_fit(p: [Point; 3], g: [GkPoint; 3]) -> Option<GkTransform> {
    let a = [
        [p[0].x, p[0].y, 1.0],
        [p[1].x, p[1].y, 1.0],
        [p[2].x, p[2].y, 1.0],
    ];
    let e = solve3(a, [g[0].y, g[1].y, g[2].y])?;
    let n = solve3(a, [g[0].x, g[1].x, g[2].x])?;
    let det = e[0] * n[1] - e[1] * n[0];
    (det.is_finite() && det > 0.0).then_some(GkTransform { e, n })
}

/// Solves `a x = b` for a 3 x 3 system by Cramer's rule; `None` when the
/// matrix is singular (the three markers lie on one line).
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = det3(a);
    if !det.is_finite() || det.abs() < 1e-9 {
        return None;
    }
    let mut out = [0.0; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        let mut m = a;
        for (r, row) in m.iter_mut().enumerate() {
            row[c] = b[r];
        }
        *slot = det3(m) / det;
    }
    Some(out)
}

fn det3(m: [[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// True when two of the GK-42 eastings carry different zone numbers.
pub fn zones_disagree(gk: &[GkPoint]) -> bool {
    let mut zones = gk.iter().filter_map(|g| zone_of(g.y));
    match zones.next() {
        None => false,
        Some(first) => zones.any(|z| z != first),
    }
}

/// The zone number in the leading digits of a GK-42 easting (metres):
/// 42 531 250 is zone 42. `None` when the value cannot carry one.
pub fn zone_of(easting: f64) -> Option<u32> {
    if !(1_000_000.0..=60_999_999.0).contains(&easting) {
        return None;
    }
    let zone = (easting / 1_000_000.0).floor() as u32;
    (1..=60).contains(&zone).then_some(zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 500 x 350 mm map: markers at (10, 20), (510, 370) and (10, 370).
    /// True coordinates chosen so the scales are exactly 1:10 000 along
    /// X and 1:12 000 along Y (deformation 20 %), zone 42, no turn.
    fn calibration() -> GkCalibration {
        GkCalibration {
            layer: LayerId(1),
            marker1: ShapeId(2),
            marker2: ShapeId(3),
            gk1: GkPoint {
                x: 6_519_000.0,
                y: 42_531_000.0,
            },
            gk2: GkPoint {
                x: 6_523_200.0,
                y: 42_536_000.0,
            },
            marker3: Some(ShapeId(4)),
            gk3: Some(GkPoint {
                x: 6_523_200.0,
                y: 42_531_000.0,
            }),
        }
    }

    fn markers() -> (Point, Point, Point) {
        (
            Point::new(10.0, 20.0),
            Point::new(510.0, 370.0),
            Point::new(10.0, 370.0),
        )
    }

    fn applied() -> GkTransform {
        let (p1, p2, p3) = markers();
        calibration().transform_at(p1, p2, Some(p3)).unwrap()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn near_m(a: GkPoint, b: GkPoint) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn transform_scales_and_offsets() {
        let t = applied();
        assert!(near(t.e[0], 10.0) && near(t.e[1], 0.0));
        assert!(near(t.e[2], 42_531_000.0 - 10.0 * 10.0));
        assert!(near(t.n[0], 0.0) && near(t.n[1], 12.0));
        assert!(near(t.n[2], 6_519_000.0 - 12.0 * 20.0));
    }

    #[test]
    fn to_gk_at_the_markers_is_exact() {
        let t = applied();
        let (p1, p2, p3) = markers();
        assert!(near_m(t.to_gk(p1), calibration().gk1));
        assert!(near_m(t.to_gk(p2), calibration().gk2));
        assert!(near_m(t.to_gk(p3), calibration().gk3.unwrap()));
    }

    #[test]
    fn to_gk_between_the_markers() {
        let t = applied();
        let g = t.to_gk(Point::new(260.0, 195.0));
        assert!(near(g.x, 12.0 * 195.0 + 6_519_000.0 - 12.0 * 20.0));
        assert!(near(g.y, 10.0 * 260.0 + 42_531_000.0 - 10.0 * 10.0));
    }

    #[test]
    fn from_gk_round_trip() {
        let t = applied();
        for p in [
            Point::new(10.0, 20.0),
            Point::new(510.0, 370.0),
            Point::new(260.0, 195.0),
            Point::new(0.0, 0.0),
            Point::new(-30.0, 400.0),
        ] {
            let back = t.from_gk(t.to_gk(p)).unwrap();
            assert!((back.x - p.x).abs() < 1e-6);
            assert!((back.y - p.y).abs() < 1e-6);
        }
    }

    #[test]
    fn scales_and_deformation() {
        let t = applied();
        assert!(near(t.scale_x(), 10_000.0));
        assert!(near(t.scale_y(), 12_000.0));
        assert!(near(t.deformation(), 20.0));
    }

    #[test]
    fn distance_uses_both_scales() {
        let t = applied();
        assert!(near(t.distance_m(Point::new(0.0, 0.0), Point::new(3.0, 0.0)), 30.0));
        assert!(near(t.distance_m(Point::new(0.0, 0.0), Point::new(0.0, 3.0)), 36.0));
        let diag = t.distance_m(Point::new(0.0, 0.0), Point::new(3.0, 4.0));
        assert!(near(diag, (30.0f64 * 30.0 + 48.0 * 48.0).sqrt()));
    }

    #[test]
    fn a_turned_map_is_recovered_by_three_markers() {
        let truth = |p: Point| {
            let (s, c) = (0.5f64, 3.0f64.sqrt() / 2.0);
            GkPoint {
                x: 12.0 * (s * p.x + c * p.y) + 6_518_000.0,
                y: 10.0 * (c * p.x - s * p.y) + 42_530_000.0,
            }
        };
        let pages = [Point::new(10.0, 20.0), Point::new(510.0, 370.0), Point::new(200.0, 40.0)];
        let gk = [truth(pages[0]), truth(pages[1]), truth(pages[2])];
        let t = GkTransform::fit(&pages, &gk).expect("a turned map fits");
        for p in [Point::new(300.0, 100.0), Point::new(-50.0, 500.0)] {
            assert!(near_m(t.to_gk(p), truth(p)), "{:?}", t.to_gk(p));
        }
    }

    #[test]
    fn a_sheared_map_gets_its_own_coefficients() {
        let mut cal = calibration();
        cal.gk3 = Some(GkPoint {
            x: 6_523_200.0,
            y: 42_532_500.0,
        });
        let (p1, p2, p3) = markers();
        let t = cal.transform_at(p1, p2, Some(p3)).unwrap();
        assert!(near(t.e[1], 1500.0 / 350.0));
    }

    #[test]
    fn zone_of_both_markers_agrees() {
        assert_eq!(calibration().zone(), Some(42));
    }

    #[test]
    fn zone_of_disagreeing_markers_is_none() {
        let mut cal = calibration();
        cal.gk2.y = 43_536_000.0;
        assert_eq!(cal.zone(), None);
    }

    #[test]
    fn zone_of_out_of_range_is_none() {
        assert_eq!(zone_of(999_999.0), None);
        assert_eq!(zone_of(61_000_000.0), None);
        assert_eq!(zone_of(42_531_250.0), Some(42));
        assert_eq!(zone_of(1_000_000.0), Some(1));
    }

    #[test]
    fn zones_disagree_across_the_markers() {
        let same = [
            GkPoint { x: 1.0, y: 42_531_000.0 },
            GkPoint { x: 2.0, y: 42_536_000.0 },
        ];
        assert!(!zones_disagree(&same));
        let mixed = [same[0], GkPoint { x: 2.0, y: 43_536_000.0 }];
        assert!(zones_disagree(&mixed));
    }

    #[test]
    fn degenerate_marker_positions_are_none() {
        let cal = calibration();
        let (p1, p2, p3) = markers();
        assert_eq!(cal.transform_at(p1, p1, Some(p3)), None);
        assert_eq!(cal.transform_at(p1, p2, Some(Point::new(260.0, 195.0))), None);
        assert_eq!(cal.transform_at(p1, p2, None), None);
        assert_eq!(
            cal.transform_at(Point::new(f64::NAN, 20.0), p2, Some(p3)),
            None
        );
    }

    #[test]
    fn mirrored_map_is_none() {
        let mut cal = calibration();
        std::mem::swap(&mut cal.gk1.y, &mut cal.gk2.y);
        let (p1, p2, p3) = markers();
        assert_eq!(cal.transform_at(p1, p2, Some(p3)), None);
    }

    #[test]
    fn two_marker_calibration_keeps_the_axis_model() {
        let mut cal = calibration();
        cal.marker3 = None;
        cal.gk3 = None;
        let (p1, p2, _) = markers();
        let t = cal.transform_at(p1, p2, None).unwrap();
        assert!(near(t.e[0], 10.0) && near(t.e[1], 0.0));
        assert!(near(t.n[1], 12.0) && near(t.n[0], 0.0));
    }

    #[test]
    fn a_saved_two_marker_file_loads_without_the_third() {
        let mut cal = calibration();
        cal.marker3 = None;
        cal.gk3 = None;
        let json = serde_json::to_string(&cal).unwrap();
        assert!(!json.contains("marker3"));
        let back: GkCalibration = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cal);
    }

    #[test]
    fn calibration_serializes_round_trip() {
        let cal = calibration();
        let json = serde_json::to_string(&cal).unwrap();
        let back: GkCalibration = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cal);
    }
}
