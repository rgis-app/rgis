use std::num;
use std::sync::atomic::{AtomicU16, Ordering};

/// Named system sets for cross-crate ordering of Bevy systems.
///
/// The intended ordering is:
///   `FileLoading` → `LayerProcessing` → `Transform` → `Rendering` → `Camera`
///
/// This prevents race conditions such as newly-projected geometry meshes being
/// deleted by a stale despawn that runs out of order.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum RgisSet {
    /// Loading files and creating layer entities.
    FileLoading,
    /// Processing layer data (creation, deletion, visibility, CRS updates).
    LayerProcessing,
    /// Reprojecting geometry / raster data into the target CRS.
    Transform,
    /// Building and spawning render meshes / sprites.
    Rendering,
    /// Updating the camera (centering, panning, zooming).
    Camera,
}

impl RgisSet {
    /// Chain the sets in `Update` in the order documented above.
    pub fn configure(app: &mut bevy::app::App) {
        use bevy::ecs::schedule::IntoScheduleConfigs;
        app.configure_sets(
            bevy::app::Update,
            (
                RgisSet::FileLoading,
                RgisSet::LayerProcessing,
                RgisSet::Transform,
                RgisSet::Rendering,
                RgisSet::Camera,
            )
                .chain(),
        );
    }
}

#[derive(
    Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, bevy::ecs::component::Component,
)]
pub struct LayerId(num::NonZeroU16);

static NEXT_LAYER_ID: AtomicU16 = AtomicU16::new(1);

impl LayerId {
    /// Creates a new unique `LayerId`.
    pub fn new() -> Self {
        let value = NEXT_LAYER_ID.fetch_add(1, Ordering::Relaxed);
        LayerId(
            num::NonZeroU16::new(value).expect("LayerId overflow"),
        )
    }

    /// Creates a `LayerId` from a `u16` value.
    ///
    /// # Panics
    ///
    /// Panics if `value` is 0.
    pub fn from_u16(value: u16) -> Self {
        LayerId(
            num::NonZeroU16::new(value).expect("LayerId value must be non-zero"),
        )
    }

    /// Returns the raw numeric value of this `LayerId`.
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    GeoJson,
    Wkt,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::GeoJson => "geojson",
            ExportFormat::Wkt => "wkt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::GeoJson => "GeoJSON",
            ExportFormat::Wkt => "WKT",
        }
    }
}

/// Geographic bounding box defining the valid area of use for a CRS.
///
/// Coordinates are in degrees (WGS 84).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaOfUse {
    pub lat_south: f64,
    pub lon_west: f64,
    pub lat_north: f64,
    pub lon_east: f64,
}

/// Parse `BBOX[lat_south,lon_west,lat_north,lon_east]` from a WKT string.
fn parse_bbox_from_wkt(wkt: &str) -> Option<AreaOfUse> {
    let start = wkt.find("BBOX[")?;
    let after = &wkt[start + 5..];
    let end = after.find(']')?;
    let inner = &after[..end];
    let mut parts = inner.split(',');
    let lat_south = parts.next()?.parse().ok()?;
    let lon_west = parts.next()?.parse().ok()?;
    let lat_north = parts.next()?.parse().ok()?;
    let lon_east = parts.next()?.parse().ok()?;
    Some(AreaOfUse { lat_south, lon_west, lat_north, lon_east })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crs {
    pub epsg_code: Option<u16>,
    pub proj_string: Option<String>,
    pub op_handle: geodesy::ctx::OpHandle,
}

impl Crs {
    pub fn is_geographic(&self) -> bool {
        if let Some(code) = self.epsg_code {
            crs_definitions::from_code(code)
                .map(|def| def.proj4.contains("+proj=longlat"))
                .unwrap_or(true)
        } else if let Some(ref proj) = self.proj_string {
            proj.contains("+proj=longlat")
        } else {
            true
        }
    }

    /// Returns the geographic area of use for this CRS, if known.
    ///
    /// The bounds are parsed from the `BBOX` element in the WKT definition
    /// provided by the EPSG registry via the `crs-definitions` crate.
    pub fn area_of_use(&self) -> Option<AreaOfUse> {
        let code = self.epsg_code?;
        let def = crs_definitions::from_code(code)?;
        parse_bbox_from_wkt(def.wkt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bbox_web_mercator() {
        let wkt = r#"PROJCRS["WGS 84 / Pseudo-Mercator",USAGE[BBOX[-85.06,-180,85.06,180]],ID["EPSG",3857]]"#;
        let area = parse_bbox_from_wkt(wkt).unwrap();
        assert_eq!(area, AreaOfUse {
            lat_south: -85.06,
            lon_west: -180.0,
            lat_north: 85.06,
            lon_east: 180.0,
        });
    }

    #[test]
    fn parse_bbox_regional() {
        let wkt = r#"PROJCRS["Anguilla",USAGE[BBOX[18.11,-63.22,18.33,-62.92]],ID["EPSG",2000]]"#;
        let area = parse_bbox_from_wkt(wkt).unwrap();
        assert_eq!(area, AreaOfUse {
            lat_south: 18.11,
            lon_west: -63.22,
            lat_north: 18.33,
            lon_east: -62.92,
        });
    }

    #[test]
    fn parse_bbox_missing() {
        assert!(parse_bbox_from_wkt(r#"PROJCRS["no bbox here"]"#).is_none());
    }

    #[test]
    fn parse_bbox_malformed() {
        assert!(parse_bbox_from_wkt("BBOX[1,2,three,4]").is_none());
        assert!(parse_bbox_from_wkt("BBOX[1,2]").is_none());
    }

    #[test]
    fn area_of_use_epsg_3857() {
        let crs = Crs {
            epsg_code: Some(3857),
            proj_string: None,
            op_handle: geodesy::ctx::OpHandle::default(),
        };
        let area = crs.area_of_use().unwrap();
        assert_eq!(area.lat_south, -85.06);
        assert_eq!(area.lat_north, 85.06);
        assert_eq!(area.lon_west, -180.0);
        assert_eq!(area.lon_east, 180.0);
    }

    #[test]
    fn area_of_use_epsg_4326() {
        let crs = Crs {
            epsg_code: Some(4326),
            proj_string: None,
            op_handle: geodesy::ctx::OpHandle::default(),
        };
        let area = crs.area_of_use().unwrap();
        assert_eq!(area.lat_south, -90.0);
        assert_eq!(area.lat_north, 90.0);
    }

    #[test]
    fn area_of_use_no_epsg_code() {
        let crs = Crs {
            epsg_code: None,
            proj_string: Some("+proj=merc".into()),
            op_handle: geodesy::ctx::OpHandle::default(),
        };
        assert!(crs.area_of_use().is_none());
    }
}
