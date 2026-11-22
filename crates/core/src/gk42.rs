//! GK-42 map calibration: two reference markers on the page map the
//! page millimetres to true GK-42 metres (Gauss-Kruger, the Pulkovo
//! 1942 grid), with an independent scale along each axis, because maps
//! redrawn or rescanned are often stretched differently in X and Y.
//!
//! The page's X axis (right) maps to the GK-42 Y axis (easting) and the
//! page's Y axis (up) to the GK-42 X axis (northing). Two markers fix
//! four unknowns: one scale and one offset per axis. A skewed map would
//! need a third marker, which the module does not model yet.
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

/// The two reference markers and their true GK-42 coordinates. Marker 1
/// is the bottom-left one, marker 2 the top-right one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GkCalibration {
    /// The layer the markers live on (the "GK-42 markers" layer).
    pub layer: LayerId,
    /// The bottom-left reference marker.
    pub marker1: ShapeId,
    /// The top-right reference marker.
    pub marker2: ShapeId,
    /// True GK-42 metres of marker 1.
    pub gk1: GkPoint,
    /// True GK-42 metres of marker 2.
    pub gk2: GkPoint,
}

/// The resolved page-to-GK-42 transform: an independent scale and
/// offset along each axis, no rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GkTransform {
    /// GK-42 metres per page millimetre along the page's X axis
    /// (easting). Always positive: a mirrored map is not a calibration.
    pub east: f64,
    /// Easting offset (metres) at page x = 0.
    pub east_off: f64,
    /// GK-42 metres per page millimetre along the page's Y axis
    /// (northing). Always positive.
    pub north: f64,
    /// Northing offset (metres) at page y = 0.
    pub north_off: f64,
}

impl GkCalibration {
    /// The zone number in the leading digits of the easting (both
    /// markers must agree): `None` when they disagree or a value is out
    /// of range.
    pub fn zone(&self) -> Option<u32> {
        let z1 = zone_of(self.gk1.y)?;
        let z2 = zone_of(self.gk2.y)?;
        (z1 == z2).then_some(z1)
    }

    /// The transform from the markers' page positions (millimetres,
    /// y-up). `None` when the markers coincide, sit on one vertical or
    /// horizontal line, or the implied scales are not positive and
    /// finite (a mirrored or degenerate map).
    pub fn transform_at(&self, p1: Point, p2: Point) -> Option<GkTransform> {
        let dx = p2.x - p1.x;
        let dy = p2.y - p1.y;
        if !dx.is_finite() || !dy.is_finite() || dx == 0.0 || dy == 0.0 {
            return None;
        }
        let east = (self.gk2.y - self.gk1.y) / dx;
        let north = (self.gk2.x - self.gk1.x) / dy;
        if !east.is_finite() || !north.is_finite() || east <= 0.0 || north <= 0.0 {
            return None;
        }
        Some(GkTransform {
            east,
            east_off: self.gk1.y - east * p1.x,
            north,
            north_off: self.gk1.x - north * p1.y,
        })
    }
}

impl GkTransform {
    /// The GK-42 metres of a page point (millimetres, y-up).
    pub fn to_gk(&self, p: Point) -> GkPoint {
        GkPoint {
            x: self.north * p.y + self.north_off,
            y: self.east * p.x + self.east_off,
        }
    }

    /// The page point of GK-42 metres, the inverse of [`to_gk`].
    pub fn from_gk(&self, g: GkPoint) -> Option<Point> {
        if self.east == 0.0 || self.north == 0.0 {
            return None;
        }
        Some(Point::new(
            (g.y - self.east_off) / self.east,
            (g.x - self.north_off) / self.north,
        ))
    }

    /// The map scale along the page's X axis as 1:N (N metres of map
    /// per paper millimetre, times 1000).
    pub fn scale_x(&self) -> f64 {
        self.east * 1000.0
    }

    /// The map scale along the page's Y axis as 1:N.
    pub fn scale_y(&self) -> f64 {
        self.north * 1000.0
    }

    /// How much the two axis scales differ, in percent of the X one:
    /// the map's deformation.
    pub fn deformation(&self) -> f64 {
        if self.east == 0.0 {
            return 0.0;
        }
        ((self.north - self.east) / self.east).abs() * 100.0
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

    /// A 500 x 350 mm map: marker 1 at (10, 20), marker 2 at (510, 370).
    /// True coordinates chosen so the scales are exactly 1:10 000 and
    /// 1:12 000 (deformation 20 %), zone 42.
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
        }
    }

    fn markers() -> (Point, Point) {
        (Point::new(10.0, 20.0), Point::new(510.0, 370.0))
    }

    #[test]
    fn transform_scales_and_offsets() {
        let t = calibration().transform_at(markers().0, markers().1).unwrap();
        assert_eq!(t.east, 10.0);
        assert_eq!(t.north, 12.0);
        assert_eq!(t.east_off, 42_531_000.0 - 10.0 * 10.0);
        assert_eq!(t.north_off, 6_519_000.0 - 12.0 * 20.0);
    }

    #[test]
    fn to_gk_at_the_markers_is_exact() {
        let t = calibration().transform_at(markers().0, markers().1).unwrap();
        let (p1, p2) = markers();
        assert_eq!(t.to_gk(p1), calibration().gk1);
        assert_eq!(t.to_gk(p2), calibration().gk2);
    }

    #[test]
    fn to_gk_between_the_markers() {
        let t = calibration().transform_at(markers().0, markers().1).unwrap();
        let g = t.to_gk(Point::new(260.0, 195.0));
        assert_eq!(g.x, 12.0 * 195.0 + 6_519_000.0 - 12.0 * 20.0);
        assert_eq!(g.y, 10.0 * 260.0 + 42_531_000.0 - 10.0 * 10.0);
    }

    #[test]
    fn from_gk_round_trip() {
        let t = calibration().transform_at(markers().0, markers().1).unwrap();
        for p in [
            Point::new(10.0, 20.0),
            Point::new(510.0, 370.0),
            Point::new(260.0, 195.0),
            Point::new(0.0, 0.0),
            Point::new(-30.0, 400.0),
        ] {
            let back = t.from_gk(t.to_gk(p)).unwrap();
            assert!((back.x - p.x).abs() < 1e-9);
            assert!((back.y - p.y).abs() < 1e-9);
        }
    }

    #[test]
    fn scales_and_deformation() {
        let t = calibration().transform_at(markers().0, markers().1).unwrap();
        assert_eq!(t.scale_x(), 10_000.0);
        assert_eq!(t.scale_y(), 12_000.0);
        assert_eq!(t.deformation(), 20.0);
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
    fn degenerate_marker_positions_are_none() {
        let cal = calibration();
        assert_eq!(cal.transform_at(Point::new(10.0, 20.0), Point::new(10.0, 20.0)), None);
        assert_eq!(cal.transform_at(Point::new(10.0, 20.0), Point::new(10.0, 370.0)), None);
        assert_eq!(cal.transform_at(Point::new(10.0, 20.0), Point::new(510.0, 20.0)), None);
        assert_eq!(
            cal.transform_at(Point::new(f64::NAN, 20.0), Point::new(510.0, 370.0)),
            None
        );
    }

    #[test]
    fn mirrored_map_is_none() {
        let mut cal = calibration();
        std::mem::swap(&mut cal.gk1.y, &mut cal.gk2.y);
        assert_eq!(cal.transform_at(markers().0, markers().1), None);
    }

    #[test]
    fn calibration_serializes_round_trip() {
        let cal = calibration();
        let json = serde_json::to_string(&cal).unwrap();
        let back: GkCalibration = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cal);
    }
}
