use geo::{Coord, CoordFloat, MapCoords};
use geodesy::ctx::Context;
use rgis_primitives::Crs;

/// Transforms coordinates between two CRSs, taking each CRS's native units
/// into account.
///
/// Geographic CRSs (`+proj=longlat`) take and return degrees. Projected CRSs
/// take and return their linear unit (e.g. metres). geodesy itself works in
/// radians for angular values, so degrees are converted on the way in only
/// when the *source* is geographic, and on the way out only when the *target*
/// is geographic.
///
/// `geo_geodesy::Transformer` always treats its input as degrees, which
/// silently scales projected input by π/180. Use this type instead.
pub struct CrsTransformer<'a, C: Context> {
    ctx: &'a C,
    source: geodesy::ctx::OpHandle,
    target: geodesy::ctx::OpHandle,
    source_is_geographic: bool,
    target_is_geographic: bool,
}

impl<'a, C: Context> CrsTransformer<'a, C> {
    pub fn new(ctx: &'a C, source: &Crs, target: &Crs) -> Self {
        Self::from_parts(
            ctx,
            (source.op_handle, source.is_geographic()),
            (target.op_handle, target.is_geographic()),
        )
    }

    /// Build a transformer from `(op_handle, is_geographic)` pairs.
    pub fn from_parts(
        ctx: &'a C,
        (source, source_is_geographic): (geodesy::ctx::OpHandle, bool),
        (target, target_is_geographic): (geodesy::ctx::OpHandle, bool),
    ) -> Self {
        CrsTransformer {
            ctx,
            source,
            target,
            source_is_geographic,
            target_is_geographic,
        }
    }

    pub fn transform_coord(&self, coord: Coord<f64>) -> Result<Coord<f64>, geodesy::Error> {
        let mut buf = if self.source_is_geographic {
            [geodesy::coord::Coor2D::gis(coord.x, coord.y)]
        } else {
            [geodesy::coord::Coor2D::raw(coord.x, coord.y)]
        };
        self.ctx
            .apply(self.source, geodesy::Direction::Inv, &mut buf)?;
        self.ctx
            .apply(self.target, geodesy::Direction::Fwd, &mut buf)?;
        let [x, y] = buf[0].0;
        Ok(if self.target_is_geographic {
            geo::coord! { x: x.to_degrees(), y: y.to_degrees() }
        } else {
            geo::coord! { x: x, y: y }
        })
    }

    pub fn transform_geometry<Scalar: CoordFloat>(
        &self,
        geometry: &mut geo::Geometry<Scalar>,
    ) -> Result<(), TransformError> {
        let transformed = geometry.try_map_coords(|coord| {
            let coord = geo::coord! {
                x: coord.x.to_f64().ok_or(TransformError::NumConversion)?,
                y: coord.y.to_f64().ok_or(TransformError::NumConversion)?,
            };
            let out = self.transform_coord(coord)?;
            Ok::<_, TransformError>(geo::coord! {
                x: Scalar::from(out.x).ok_or(TransformError::NumConversion)?,
                y: Scalar::from(out.y).ok_or(TransformError::NumConversion)?,
            })
        })?;
        *geometry = transformed;
        Ok(())
    }
}

/// Project a WGS 84-ish longitude/latitude (degrees) into `crs`.
pub fn lon_lat_to_crs<C: Context>(
    ctx: &C,
    crs: &Crs,
    lon_lat: Coord<f64>,
) -> Result<Coord<f64>, geodesy::Error> {
    let mut buf = [geodesy::coord::Coor2D::gis(lon_lat.x, lon_lat.y)];
    ctx.apply(crs.op_handle, geodesy::Direction::Fwd, &mut buf)?;
    let [x, y] = buf[0].0;
    Ok(if crs.is_geographic() {
        geo::coord! { x: x.to_degrees(), y: y.to_degrees() }
    } else {
        geo::coord! { x: x, y: y }
    })
}

#[derive(Debug)]
pub enum TransformError {
    Geodesy(geodesy::Error),
    NumConversion,
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TransformError::Geodesy(err) => write!(f, "Geodesy error: {err}"),
            TransformError::NumConversion => write!(f, "Could not convert coordinate to/from f64"),
        }
    }
}

impl std::error::Error for TransformError {}

impl From<geodesy::Error> for TransformError {
    fn from(err: geodesy::Error) -> Self {
        TransformError::Geodesy(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crs(ctx: &mut geodesy::ctx::Minimal, code: u16) -> Crs {
        Crs {
            epsg_code: Some(code),
            proj_string: None,
            op_handle: crate::epsg_code_to_geodesy_op_handle(ctx, code).unwrap(),
        }
    }

    /// Projected → geographic must not treat metres as degrees.
    #[test]
    fn projected_source_round_trips() {
        let mut ctx = geodesy::ctx::Minimal::new();
        let wgs84 = crs(&mut ctx, 4326);
        let utm18n = crs(&mut ctx, 32618);
        let fwd = CrsTransformer::new(&ctx, &wgs84, &utm18n);
        let inv = CrsTransformer::new(&ctx, &utm18n, &wgs84);

        let lake_erie = geo::coord! { x: -79.5, y: 42.5 };
        let projected = fwd.transform_coord(lake_erie).unwrap();
        // UTM northing ~4.7 million metres at 42.5°N.
        assert!((4_600_000.0..4_800_000.0).contains(&projected.y), "{projected:?}");
        let back = inv.transform_coord(projected).unwrap();
        assert!((back.x - lake_erie.x).abs() < 1e-7, "{back:?}");
        assert!((back.y - lake_erie.y).abs() < 1e-7, "{back:?}");
    }

    /// Projected → projected, e.g. a UTM layer displayed in Web Mercator.
    #[test]
    fn projected_to_projected_matches_via_geographic() {
        let mut ctx = geodesy::ctx::Minimal::new();
        let wgs84 = crs(&mut ctx, 4326);
        let utm18n = crs(&mut ctx, 32618);
        let merc = crs(&mut ctx, 3857);

        let p = geo::coord! { x: -76.0, y: 43.5 };
        let utm = CrsTransformer::new(&ctx, &wgs84, &utm18n).transform_coord(p).unwrap();
        let direct = CrsTransformer::new(&ctx, &wgs84, &merc).transform_coord(p).unwrap();
        let via_utm = CrsTransformer::new(&ctx, &utm18n, &merc).transform_coord(utm).unwrap();
        assert!((direct.x - via_utm.x).abs() < 1e-3, "{direct:?} vs {via_utm:?}");
        assert!((direct.y - via_utm.y).abs() < 1e-3, "{direct:?} vs {via_utm:?}");
    }
}
