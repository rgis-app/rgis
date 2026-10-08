mod geotiff;

pub use geotiff::GeoTiffSource;

#[derive(Clone, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: RasterFormat,
    pub extent: geo_types::Rect<f64>,
    pub epsg_code: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterFormat {
    R8,
    Rgba8,
}

#[derive(Debug)]
pub enum Error {
    AsyncTiff(async_tiff::error::AsyncTiffError),
    UnsupportedColorType,
    MissingGeoInfo,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AsyncTiff(err) => write!(f, "TIFF error: {err}"),
            Error::UnsupportedColorType => write!(f, "Unsupported color type"),
            Error::MissingGeoInfo => write!(f, "Missing geo-referencing information"),
        }
    }
}

impl From<async_tiff::error::AsyncTiffError> for Error {
    fn from(err: async_tiff::error::AsyncTiffError) -> Self {
        Error::AsyncTiff(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_uint8_rgb_deflate() {
        let bytes = bytes::Bytes::from(
            std::fs::read(
                "../geotiff-test-data/rasterio_generated/fixtures/uint8_rgb_deflate_block64_cog.tif",
            )
            .expect("read test file"),
        );
        let raster = GeoTiffSource::from_bytes(bytes)
            .load()
            .await
            .expect("load GeoTIFF");
        assert!(raster.width > 0);
        assert!(raster.height > 0);
        assert_eq!(raster.format, RasterFormat::Rgba8);
    }

    /// A PixelIsPoint raster's tiepoint is a pixel *center*; the extent must
    /// still cover the outer pixel edges (as GDAL reports it).
    #[tokio::test]
    async fn test_pixel_is_point_extent_matches_gdal() {
        let bytes = bytes::Bytes::from(
            std::fs::read("../geotiff-test-data/rasterio_generated/fixtures/pixel_as_point.tif")
                .expect("read test file"),
        );
        let raster = GeoTiffSource::from_bytes(bytes)
            .load()
            .await
            .expect("load GeoTIFF");
        // `gdalinfo`: Upper Left (-180, 24), Lower Right (-138, -18).
        let (min, max) = (raster.extent.min(), raster.extent.max());
        assert_eq!((min.x, min.y, max.x, max.y), (-180.0, -18.0, -138.0, 24.0));
    }
}
