//! Fix geometries that cross the antimeridian (±180° longitude).
//!
//! Geometries that cross the antimeridian cause problems for many spatial
//! operations (bounding rect, convex hull, rendering) because planar algorithms
//! don't understand coordinate wrapping. This crate splits such geometries at
//! the antimeridian, producing valid geometries that stay within [-180, 180].
//!
//! The approach is edge-based: each edge of a polygon or line string is checked
//! for antimeridian crossings (longitude jumps > 180°), and crossing points are
//! interpolated and used to split the geometry.
//!
//! # Example
//!
//! ```
//! use geo::polygon;
//! use geo_fix_antimeridian::fix_geometry;
//!
//! // A polygon crossing the antimeridian
//! let polygon = geo::polygon![
//!     (x: 170.0, y: 50.0),
//!     (x: -170.0, y: 50.0),
//!     (x: -170.0, y: 40.0),
//!     (x: 170.0, y: 40.0),
//! ];
//! let fixed = fix_geometry(&polygon.into());
//! // Result is a MultiPolygon with two parts, one on each side of ±180°
//! ```

use geo::{Coord, CoordFloat, LineString, MultiLineString, MultiPolygon, Polygon};

/// Normalize a longitude value to the range [-180, 180].
fn normalize_lon<T: CoordFloat>(lon: T) -> T {
    let full = T::from(360.0).unwrap();
    let half = T::from(180.0).unwrap();
    if lon >= -half && lon <= half {
        return lon;
    }

    let result = (lon + half).rem_euclid(full) - half;
    if result == -half && lon > T::zero() {
        half
    } else {
        result
    }
}

/// Normalize all coordinates in a list to have longitudes in [-180, 180].
fn normalize_coords<T: CoordFloat>(coords: &[(T, T)]) -> Vec<(T, T)> {
    coords
        .iter()
        .map(|&(x, y)| (normalize_lon(x), y))
        .collect()
}

/// Remove consecutive duplicate points.
fn dedup_consecutive<T: CoordFloat>(coords: &[(T, T)]) -> Vec<(T, T)> {
    if coords.is_empty() {
        return Vec::new();
    }
    let eps = T::from(1e-10).unwrap();
    let mut result = vec![coords[0]];
    for &c in &coords[1..] {
        if let Some(&last) = result.last() {
            if (last.0 - c.0).abs() > eps || (last.1 - c.1).abs() > eps {
                result.push(c);
            }
        }
    }
    result
}

/// Compute the latitude at which an edge crosses the antimeridian
/// using great circle interpolation on the sphere.
///
/// `start` is the point on the positive-longitude side (near +180),
/// `end` is the point on the negative-longitude side (near -180).
fn crossing_latitude_great_circle<T: CoordFloat>(start: (T, T), end: (T, T)) -> T {
    let half = T::from(180.0).unwrap();
    let eps = T::from(1e-10).unwrap();

    if (start.0 - half).abs() < eps {
        return start.1;
    }
    if (end.0 - (-half)).abs() < eps {
        return end.1;
    }

    // Convert to radians
    let to_rad = T::from(std::f64::consts::PI / 180.0).unwrap();
    let lon1 = start.0 * to_rad;
    let lat1 = start.1 * to_rad;
    let lon2 = (end.0 + T::from(360.0).unwrap()) * to_rad;
    let lat2 = end.1 * to_rad;
    let target_lon = half * to_rad;

    // Spherical linear interpolation along a great circle
    // Using the formula for latitude at a given longitude on a great circle
    let dlon = lon2 - lon1;
    if dlon.abs() < eps {
        return (start.1 + end.1) / T::from(2.0).unwrap();
    }

    let t = (target_lon - lon1) / dlon;
    // For small arcs, linear interpolation of latitude is a good approximation
    // For better accuracy, interpolate using spherical geometry:
    let sin_d = (lat2 - lat1).sin();
    let cos_d = (lat2 - lat1).cos();
    let _ = (sin_d, cos_d); // suppress unused warnings

    // Use the tangent-based formula for latitude on a great circle
    // tan(lat) = (tan(lat1) * sin(lon2 - lon) + tan(lat2) * sin(lon - lon1)) / sin(lon2 - lon1)
    let sin_dlon = dlon.sin();
    if sin_dlon.abs() < eps {
        return start.1 + t * (end.1 - start.1);
    }
    let lat = ((lat1.tan() * (lon2 - target_lon).sin()
        + lat2.tan() * (target_lon - lon1).sin())
        / sin_dlon)
        .atan();

    let to_deg = T::from(180.0 / std::f64::consts::PI).unwrap();
    lat * to_deg
}

/// Compute the latitude at which an edge crosses the antimeridian
/// using flat (linear) interpolation.
///
/// `start` is the point on the positive-longitude side (near +180),
/// `end` is the point on the negative-longitude side (near -180).
fn crossing_latitude_flat<T: CoordFloat>(start: (T, T), end: (T, T)) -> T {
    let half = T::from(180.0).unwrap();
    let full = T::from(360.0).unwrap();
    let eps = T::from(1e-10).unwrap();

    if (start.0 - half).abs() < eps {
        return start.1;
    }
    if (end.0 - (-half)).abs() < eps {
        return end.1;
    }

    // Linear interpolation: the edge goes from start to (end + 360°)
    // crossing at x = 180°.
    let lat_delta = end.1 - start.1;
    let adjusted_end_x = end.0 + full;
    let t = (half - start.0) / (adjusted_end_x - start.0);
    start.1 + t * lat_delta
}

/// Crossing latitude computation method.
#[derive(Debug, Clone, Copy, Default)]
pub enum CrossingMethod {
    /// Linear interpolation in lon/lat space. Fast but less accurate for long edges.
    Flat,
    /// Spherical great circle interpolation. More accurate for edges spanning
    /// large distances.
    #[default]
    GreatCircle,
}

fn crossing_latitude<T: CoordFloat>(
    start: (T, T),
    end: (T, T),
    method: CrossingMethod,
) -> T {
    match method {
        CrossingMethod::Flat => crossing_latitude_flat(start, end),
        CrossingMethod::GreatCircle => crossing_latitude_great_circle(start, end),
    }
}

/// Segment a coordinate ring at antimeridian crossings.
///
/// Returns a list of segments. Each segment is a list of (lon, lat) coordinates
/// that does not cross the antimeridian. Segments are pinned to ±180° at
/// crossing points.
///
/// Returns an empty vec if no crossings are detected.
fn segment<T: CoordFloat>(coords: &[(T, T)], method: CrossingMethod) -> Vec<Vec<(T, T)>> {
    let coords = normalize_coords(coords);
    let coords = dedup_consecutive(&coords);
    let half = T::from(180.0).unwrap();
    let full = T::from(360.0).unwrap();

    if coords.len() < 2 {
        return Vec::new();
    }

    let mut current_segment: Vec<(T, T)> = Vec::new();
    let mut segments: Vec<Vec<(T, T)>> = Vec::new();

    for i in 0..coords.len() - 1 {
        let start = coords[i];
        let end = coords[i + 1];
        current_segment.push(start);

        let dx = end.0 - start.0;

        if dx > half && dx != full {
            // Crossing from negative to positive side (left crossing)
            let lat = crossing_latitude(end, start, method);
            current_segment.push((-half, lat));
            segments.push(current_segment);
            current_segment = vec![(half, lat)];
        } else if dx < -half && dx != -full {
            // Crossing from positive to negative side (right crossing)
            let lat = crossing_latitude(start, end, method);
            current_segment.push((half, lat));
            segments.push(current_segment);
            current_segment = vec![(-half, lat)];
        }
    }

    if segments.is_empty() {
        return Vec::new();
    }

    let last = *coords.last().unwrap_or(&(T::zero(), T::zero()));
    let eps = T::from(1e-10).unwrap();
    let first_seg_start = segments[0][0];
    if (last.0 - first_seg_start.0).abs() < eps
        && (last.1 - first_seg_start.1).abs() < eps
    {
        let first = segments.remove(0);
        current_segment.extend(first);
        segments.push(current_segment);
    } else {
        current_segment.push(last);
        segments.push(current_segment);
    }

    segments
}

/// Build polygons from segments produced by splitting a polygon ring.
fn build_polygons<T: CoordFloat>(mut segments: Vec<Vec<(T, T)>>) -> Vec<Polygon<T>> {
    if segments.is_empty() {
        return Vec::new();
    }

    let eps = T::from(1e-10).unwrap();
    let mut polygons = Vec::new();

    while !segments.is_empty() {
        let mut ring = segments.remove(0);

        let mut changed = true;
        while changed {
            changed = false;
            let ring_end = *ring.last().unwrap_or(&(T::zero(), T::zero()));

            let mut best_idx = None;
            let mut best_dist = T::infinity();

            for (i, seg) in segments.iter().enumerate() {
                let seg_start = seg[0];
                if (seg_start.0 - ring_end.0).abs() < eps {
                    let dist = (seg_start.1 - ring_end.1).abs();
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = Some(i);
                    }
                }
            }

            if let Some(idx) = best_idx {
                let next_seg = segments.remove(idx);
                let ring_end = *ring.last().unwrap_or(&(T::zero(), T::zero()));
                let next_start = next_seg[0];
                if (ring_end.1 - next_start.1).abs() > eps {
                    ring.push(next_start);
                }
                ring.extend(next_seg.into_iter().skip(1));
                changed = true;
            }
        }

        if let (Some(&first), Some(&last)) = (ring.first(), ring.last()) {
            if (first.0 - last.0).abs() > eps || (first.1 - last.1).abs() > eps {
                ring.push(first);
            }
        }

        if ring.len() >= 4 {
            let line_string = LineString::from(
                ring.into_iter()
                    .map(|(x, y)| Coord { x, y })
                    .collect::<Vec<_>>(),
            );
            polygons.push(Polygon::new(line_string, vec![]));
        }
    }

    polygons
}

/// Ensure a polygon ring has counter-clockwise winding order.
///
/// GeoJSON (RFC 7946) requires exterior rings to be counter-clockwise
/// and interior rings to be clockwise.
fn ensure_ccw<T: CoordFloat>(coords: &[(T, T)]) -> Vec<(T, T)> {
    if is_clockwise(coords) {
        let mut reversed: Vec<(T, T)> = coords.to_vec();
        reversed.reverse();
        reversed
    } else {
        coords.to_vec()
    }
}

/// Ensure a polygon ring has clockwise winding order (for interior rings).
fn ensure_cw<T: CoordFloat>(coords: &[(T, T)]) -> Vec<(T, T)> {
    if !is_clockwise(coords) {
        let mut reversed: Vec<(T, T)> = coords.to_vec();
        reversed.reverse();
        reversed
    } else {
        coords.to_vec()
    }
}

/// Check if a ring is wound clockwise using the shoelace formula.
fn is_clockwise<T: CoordFloat>(coords: &[(T, T)]) -> bool {
    let mut sum = T::zero();
    for i in 0..coords.len() {
        let (x1, y1) = coords[i];
        let (x2, y2) = coords[(i + 1) % coords.len()];
        sum = sum + (x2 - x1) * (y2 + y1);
    }
    sum > T::zero()
}

/// Simple point-in-polygon test using ray casting.
fn point_in_polygon<T: CoordFloat>(px: T, py: T, polygon: &Polygon<T>) -> bool {
    let exterior = polygon.exterior();
    let coords: Vec<_> = exterior.coords().collect();
    let mut inside = false;

    let mut j = coords.len() - 1;
    for i in 0..coords.len() {
        let xi = coords[i].x;
        let yi = coords[i].y;
        let xj = coords[j].x;
        let yj = coords[j].y;

        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Fix a polygon that crosses the antimeridian.
///
/// If the polygon does not cross the antimeridian, it is returned unchanged
/// (with coordinates normalized to [-180, 180]).
///
/// If it does cross, it is split into a `MultiPolygon` with one polygon on
/// each side.
pub fn fix_polygon(polygon: &Polygon<f64>) -> geo::Geometry<f64> {
    fix_polygon_with_method(polygon, CrossingMethod::default())
}

/// Fix a polygon that crosses the antimeridian using the specified crossing method.
pub fn fix_polygon_with_method(
    polygon: &Polygon<f64>,
    method: CrossingMethod,
) -> geo::Geometry<f64> {
    let exterior_coords: Vec<(f64, f64)> = polygon
        .exterior()
        .coords()
        .map(|c| (c.x, c.y))
        .collect();

    let exterior_coords = ensure_ccw(&exterior_coords);
    let segments = segment(&exterior_coords, method);

    if segments.is_empty() {
        let normalized_exterior = LineString::from(
            normalize_coords(&exterior_coords)
                .into_iter()
                .map(|(x, y)| Coord { x, y })
                .collect::<Vec<_>>(),
        );
        let normalized_interiors: Vec<LineString<f64>> = polygon
            .interiors()
            .iter()
            .map(|interior| {
                let coords: Vec<(f64, f64)> =
                    interior.coords().map(|c| (c.x, c.y)).collect();
                let coords = ensure_cw(&coords);
                LineString::from(
                    normalize_coords(&coords)
                        .into_iter()
                        .map(|(x, y)| Coord { x, y })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        return Polygon::new(normalized_exterior, normalized_interiors).into();
    }

    let mut polygons = build_polygons(segments);

    // Handle interior rings
    for interior in polygon.interiors() {
        let interior_coords: Vec<(f64, f64)> =
            interior.coords().map(|c| (c.x, c.y)).collect();
        let interior_coords = ensure_cw(&interior_coords);
        let interior_segments = segment(&interior_coords, method);

        if interior_segments.is_empty() {
            let normalized = LineString::from(
                normalize_coords(&interior_coords)
                    .into_iter()
                    .map(|(x, y)| Coord { x, y })
                    .collect::<Vec<_>>(),
            );
            let centroid_x = normalized.coords().map(|c| c.x).sum::<f64>()
                / normalized.coords().count() as f64;
            let centroid_y = normalized.coords().map(|c| c.y).sum::<f64>()
                / normalized.coords().count() as f64;

            for poly in &mut polygons {
                if point_in_polygon(centroid_x, centroid_y, poly) {
                    let mut interiors = poly.interiors().to_vec();
                    interiors.push(normalized);
                    *poly = Polygon::new(poly.exterior().clone(), interiors);
                    break;
                }
            }
        } else {
            let interior_polys = build_polygons(interior_segments);
            for int_poly in interior_polys {
                let int_ring = int_poly.into_inner().0;
                let centroid_x = int_ring.coords().map(|c| c.x).sum::<f64>()
                    / int_ring.coords().count() as f64;
                let centroid_y = int_ring.coords().map(|c| c.y).sum::<f64>()
                    / int_ring.coords().count() as f64;
                for poly in &mut polygons {
                    if point_in_polygon(centroid_x, centroid_y, poly) {
                        let mut interiors = poly.interiors().to_vec();
                        interiors.push(int_ring);
                        *poly = Polygon::new(poly.exterior().clone(), interiors);
                        break;
                    }
                }
            }
        }
    }

    if polygons.len() == 1 {
        polygons.into_iter().next().unwrap().into()
    } else {
        MultiPolygon(polygons).into()
    }
}

/// Fix a line string that crosses the antimeridian.
pub fn fix_line_string(line_string: &LineString<f64>) -> geo::Geometry<f64> {
    fix_line_string_with_method(line_string, CrossingMethod::default())
}

/// Fix a line string that crosses the antimeridian using the specified crossing method.
pub fn fix_line_string_with_method(
    line_string: &LineString<f64>,
    method: CrossingMethod,
) -> geo::Geometry<f64> {
    let coords: Vec<(f64, f64)> = line_string.coords().map(|c| (c.x, c.y)).collect();
    let coords = normalize_coords(&coords);
    let coords = dedup_consecutive(&coords);
    let half = 180.0_f64;
    let full = 360.0_f64;

    if coords.len() < 2 {
        return line_string.clone().into();
    }

    let mut current_segment: Vec<(f64, f64)> = Vec::new();
    let mut segments: Vec<Vec<(f64, f64)>> = Vec::new();

    for i in 0..coords.len() - 1 {
        let start = coords[i];
        let end = coords[i + 1];
        current_segment.push(start);

        let dx = end.0 - start.0;

        if dx > half && dx != full {
            let lat = crossing_latitude(end, start, method);
            current_segment.push((-half, lat));
            segments.push(current_segment);
            current_segment = vec![(half, lat)];
        } else if dx < -half && dx != -full {
            let lat = crossing_latitude(start, end, method);
            current_segment.push((half, lat));
            segments.push(current_segment);
            current_segment = vec![(-half, lat)];
        }
    }

    if let Some(&last) = coords.last() {
        current_segment.push(last);
    }

    if segments.is_empty() {
        let normalized = LineString::from(
            coords
                .into_iter()
                .map(|(x, y)| Coord { x, y })
                .collect::<Vec<_>>(),
        );
        return normalized.into();
    }

    segments.push(current_segment);

    let line_strings: Vec<LineString<f64>> = segments
        .into_iter()
        .filter(|s| s.len() >= 2)
        .map(|s| {
            LineString::from(
                s.into_iter()
                    .map(|(x, y)| Coord { x, y })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    if line_strings.len() == 1 {
        line_strings.into_iter().next().unwrap().into()
    } else {
        MultiLineString(line_strings).into()
    }
}

/// Fix any geometry that may cross the antimeridian.
///
/// Recursively processes all geometry types. Polygons and line strings that
/// cross the antimeridian are split. Points are normalized. Collections are
/// processed element by element.
pub fn fix_geometry(geometry: &geo::Geometry<f64>) -> geo::Geometry<f64> {
    fix_geometry_with_method(geometry, CrossingMethod::default())
}

/// Fix any geometry using the specified crossing method.
pub fn fix_geometry_with_method(
    geometry: &geo::Geometry<f64>,
    method: CrossingMethod,
) -> geo::Geometry<f64> {
    match geometry {
        geo::Geometry::Point(p) => {
            geo::Point::new(normalize_lon(p.x()), p.y()).into()
        }
        geo::Geometry::Line(l) => {
            let ls = LineString::from(vec![l.start, l.end]);
            fix_line_string_with_method(&ls, method)
        }
        geo::Geometry::LineString(ls) => fix_line_string_with_method(ls, method),
        geo::Geometry::Polygon(p) => fix_polygon_with_method(p, method),
        geo::Geometry::MultiPoint(mp) => {
            let points: Vec<geo::Point<f64>> = mp
                .iter()
                .map(|p| geo::Point::new(normalize_lon(p.x()), p.y()))
                .collect();
            geo::MultiPoint::new(points).into()
        }
        geo::Geometry::MultiLineString(mls) => {
            let mut all_lines = Vec::new();
            for ls in mls.iter() {
                match fix_line_string_with_method(ls, method) {
                    geo::Geometry::LineString(l) => all_lines.push(l),
                    geo::Geometry::MultiLineString(ml) => all_lines.extend(ml.0),
                    _ => {}
                }
            }
            if all_lines.len() == 1 {
                all_lines.into_iter().next().unwrap().into()
            } else {
                MultiLineString(all_lines).into()
            }
        }
        geo::Geometry::MultiPolygon(mp) => {
            let mut all_polys = Vec::new();
            for p in mp.iter() {
                match fix_polygon_with_method(p, method) {
                    geo::Geometry::Polygon(p) => all_polys.push(p),
                    geo::Geometry::MultiPolygon(mp) => all_polys.extend(mp.0),
                    _ => {}
                }
            }
            if all_polys.len() == 1 {
                all_polys.into_iter().next().unwrap().into()
            } else {
                MultiPolygon(all_polys).into()
            }
        }
        geo::Geometry::GeometryCollection(gc) => {
            let fixed: Vec<geo::Geometry<f64>> = gc
                .iter()
                .map(|g| fix_geometry_with_method(g, method))
                .collect();
            geo::Geometry::GeometryCollection(geo::GeometryCollection::new_from(fixed))
        }
        geo::Geometry::Rect(r) => fix_polygon_with_method(&r.to_polygon(), method),
        geo::Geometry::Triangle(t) => fix_polygon_with_method(&t.to_polygon(), method),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{line_string, polygon};

    #[test]
    fn no_crossing() {
        let polygon = polygon![
            (x: 0.0, y: 0.0),
            (x: 10.0, y: 0.0),
            (x: 10.0, y: 10.0),
            (x: 0.0, y: 10.0),
        ];
        let result = fix_polygon(&polygon);
        assert!(matches!(result, geo::Geometry::Polygon(_)));
    }

    #[test]
    fn simple_crossing() {
        let polygon = polygon![
            (x: 170.0, y: 50.0),
            (x: -170.0, y: 50.0),
            (x: -170.0, y: 40.0),
            (x: 170.0, y: 40.0),
        ];
        let result = fix_polygon(&polygon);
        assert!(
            matches!(result, geo::Geometry::MultiPolygon(ref mp) if mp.0.len() == 2),
            "Expected MultiPolygon with 2 parts, got: {:?}",
            result
        );

        if let geo::Geometry::MultiPolygon(mp) = &result {
            for poly in &mp.0 {
                for coord in poly.exterior().coords() {
                    assert!(
                        coord.x >= -180.0 && coord.x <= 180.0,
                        "Coordinate out of range: {}",
                        coord.x
                    );
                }
            }
        }
    }

    #[test]
    fn extended_coordinates() {
        let polygon = polygon![
            (x: -170.0, y: 55.0),
            (x: -188.0, y: 55.0),
            (x: -188.0, y: 50.0),
            (x: -170.0, y: 50.0),
        ];
        let result = fix_polygon(&polygon);
        assert!(
            matches!(result, geo::Geometry::MultiPolygon(ref mp) if mp.0.len() == 2),
            "Expected MultiPolygon with 2 parts, got: {:?}",
            result
        );
    }

    #[test]
    fn line_string_crossing() {
        let ls = line_string![
            (x: 170.0, y: 45.0),
            (x: -170.0, y: 45.0),
        ];
        let result = fix_line_string(&ls);
        assert!(
            matches!(result, geo::Geometry::MultiLineString(ref mls) if mls.0.len() == 2),
            "Expected MultiLineString with 2 parts, got: {:?}",
            result
        );
    }

    #[test]
    fn line_string_no_crossing() {
        let ls = line_string![
            (x: 10.0, y: 45.0),
            (x: 20.0, y: 45.0),
        ];
        let result = fix_line_string(&ls);
        assert!(matches!(result, geo::Geometry::LineString(_)));
    }

    #[test]
    fn normalize_extended_point() {
        let geom: geo::Geometry<f64> = geo::Point::new(-200.0, 45.0).into();
        let result = fix_geometry(&geom);
        if let geo::Geometry::Point(p) = result {
            assert!((p.x() - 160.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected Point");
        }
    }

    #[test]
    fn crossing_latitude_flat_interpolation() {
        let lat = crossing_latitude_flat((170.0, 40.0), (-170.0, 50.0));
        assert!(
            (lat - 45.0_f64).abs() < 0.01,
            "Expected ~45.0, got {}",
            lat
        );
    }

    #[test]
    fn crossing_latitude_great_circle_interpolation() {
        let lat = crossing_latitude_great_circle((170.0, 40.0), (-170.0, 50.0));
        // Great circle result should be close to 45° for this symmetric case
        assert!(
            (lat - 45.0_f64).abs() < 1.0,
            "Expected ~45.0, got {}",
            lat
        );
    }

    #[test]
    fn geometry_collection() {
        let gc = geo::GeometryCollection::new_from(vec![
            polygon![
                (x: 170.0, y: 50.0),
                (x: -170.0, y: 50.0),
                (x: -170.0, y: 40.0),
                (x: 170.0, y: 40.0),
            ]
            .into(),
            geo::Point::new(10.0, 20.0).into(),
        ]);
        let result = fix_geometry(&geo::Geometry::GeometryCollection(gc));
        assert!(matches!(result, geo::Geometry::GeometryCollection(_)));
    }

    #[test]
    fn simple_crossing_coordinates_are_correct() {
        let polygon = polygon![
            (x: 170.0, y: 50.0),
            (x: -170.0, y: 50.0),
            (x: -170.0, y: 40.0),
            (x: 170.0, y: 40.0),
        ];
        let result = fix_polygon(&polygon);
        let geo::Geometry::MultiPolygon(mp) = result else {
            panic!("Expected MultiPolygon");
        };
        assert_eq!(mp.0.len(), 2);

        let mut has_east = false;
        let mut has_west = false;
        for poly in &mp.0 {
            let xs: Vec<f64> = poly.exterior().coords().map(|c| c.x).collect();
            let min_x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_x = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if max_x <= 180.0 + f64::EPSILON && min_x >= 170.0 - f64::EPSILON {
                has_east = true;
            }
            if min_x >= -180.0 - f64::EPSILON && max_x <= -170.0 + f64::EPSILON {
                has_west = true;
            }
        }
        assert!(has_east, "Expected polygon on east side [170, 180]");
        assert!(has_west, "Expected polygon on west side [-180, -170]");
    }

    #[test]
    fn multi_polygon_crossing() {
        let mp = geo::MultiPolygon(vec![
            polygon![
                (x: 170.0, y: 50.0),
                (x: -170.0, y: 50.0),
                (x: -170.0, y: 40.0),
                (x: 170.0, y: 40.0),
            ],
            polygon![
                (x: 0.0, y: 10.0),
                (x: 10.0, y: 10.0),
                (x: 10.0, y: 0.0),
                (x: 0.0, y: 0.0),
            ],
        ]);
        let result = fix_geometry(&mp.into());
        if let geo::Geometry::MultiPolygon(mp) = result {
            assert_eq!(mp.0.len(), 3);
        } else {
            panic!("Expected MultiPolygon");
        }
    }

    #[test]
    fn multiple_crossings_line_string() {
        let ls = line_string![
            (x: 170.0, y: 45.0),
            (x: -170.0, y: 45.0),
            (x: -160.0, y: 50.0),
            (x: 170.0, y: 55.0),
        ];
        let result = fix_line_string(&ls);
        assert!(
            matches!(result, geo::Geometry::MultiLineString(ref mls) if mls.0.len() == 3),
            "Expected MultiLineString with 3 parts, got: {:?}",
            result
        );
    }

    #[test]
    fn polygon_entirely_beyond_180() {
        let polygon = polygon![
            (x: 190.0, y: 10.0),
            (x: 200.0, y: 10.0),
            (x: 200.0, y: 0.0),
            (x: 190.0, y: 0.0),
        ];
        let result = fix_polygon(&polygon);
        assert!(
            matches!(result, geo::Geometry::Polygon(_)),
            "Expected Polygon, got: {:?}",
            result
        );
        if let geo::Geometry::Polygon(p) = result {
            for coord in p.exterior().coords() {
                assert!(
                    coord.x >= -180.0 && coord.x <= 180.0,
                    "Coordinate out of range: {}",
                    coord.x
                );
            }
        }
    }

    #[test]
    fn winding_order_is_corrected() {
        // Clockwise exterior (wrong for GeoJSON) — should be corrected
        let polygon = polygon![
            (x: 170.0, y: 40.0),
            (x: 170.0, y: 50.0),
            (x: -170.0, y: 50.0),
            (x: -170.0, y: 40.0),
        ];
        // Should still produce valid split result
        let result = fix_polygon(&polygon);
        assert!(
            matches!(result, geo::Geometry::MultiPolygon(ref mp) if mp.0.len() == 2),
            "Expected MultiPolygon with 2 parts, got: {:?}",
            result
        );
    }

    #[test]
    fn flat_vs_great_circle_methods() {
        let polygon = polygon![
            (x: 170.0, y: 50.0),
            (x: -170.0, y: 50.0),
            (x: -170.0, y: 40.0),
            (x: 170.0, y: 40.0),
        ];
        let flat = fix_polygon_with_method(&polygon, CrossingMethod::Flat);
        let gc = fix_polygon_with_method(&polygon, CrossingMethod::GreatCircle);
        // Both should produce MultiPolygon with 2 parts
        assert!(matches!(flat, geo::Geometry::MultiPolygon(ref mp) if mp.0.len() == 2));
        assert!(matches!(gc, geo::Geometry::MultiPolygon(ref mp) if mp.0.len() == 2));
    }
}
