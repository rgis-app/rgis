use std::ops::Range;

use async_tiff::decoder::DecoderRegistry;
use async_tiff::error::AsyncTiffResult;
use async_tiff::metadata::TiffMetadataReader;
use async_tiff::reader::AsyncFileReader;
use async_tiff::tags::PlanarConfiguration;
use async_tiff::TypedArray;

use crate::{Error, Raster, RasterFormat};

#[derive(Debug)]
struct BytesReader(bytes::Bytes);

#[async_trait::async_trait]
impl AsyncFileReader for BytesReader {
    async fn get_bytes(&self, range: Range<u64>) -> AsyncTiffResult<bytes::Bytes> {
        Ok(self.0.slice(range.start as usize..range.end as usize))
    }
}

pub struct GeoTiffSource {
    bytes: bytes::Bytes,
}

impl GeoTiffSource {
    pub fn from_bytes(bytes: bytes::Bytes) -> Self {
        GeoTiffSource { bytes }
    }

    pub async fn load(self) -> Result<Raster, Error> {
        let reader = BytesReader(self.bytes);

        let mut metadata_reader = TiffMetadataReader::try_open(&reader).await?;
        let tiff = metadata_reader.read(&reader).await?;

        let ifd = tiff.ifds().first().ok_or(Error::MissingGeoInfo)?;

        let width = ifd.image_width();
        let height = ifd.image_height();

        let raster_type = ifd.geo_key_directory().and_then(|gkd| gkd.raster_type);
        let extent = compute_extent(
            ifd.model_tiepoint(),
            ifd.model_pixel_scale(),
            ifd.model_transformation(),
            raster_type,
            width,
            height,
        )?;

        let epsg_code = ifd.geo_key_directory().and_then(|gkd| gkd.epsg_code());

        let tile_width = ifd.tile_width().ok_or(Error::UnsupportedColorType)?;
        let tile_height = ifd.tile_height().ok_or(Error::UnsupportedColorType)?;
        let (tiles_x, tiles_y) = ifd.tile_count().ok_or(Error::UnsupportedColorType)?;

        let samples_per_pixel = ifd.samples_per_pixel();
        let decoder_registry = DecoderRegistry::default();
        let is_planar = ifd.planar_configuration() == PlanarConfiguration::Planar;

        // Parse nodata value from GDAL metadata
        let nodata_f64: Option<f64> = ifd
            .gdal_nodata()
            .and_then(|s| s.trim().parse::<f64>().ok());

        // Fetch, decode, and normalize all tiles to chunky layout
        let mut decoded_tiles: Vec<(usize, usize, TypedArray, [usize; 3])> =
            Vec::with_capacity(tiles_x * tiles_y);
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let tile = ifd.fetch_tile(tx, ty, &reader).await?;
                let array = tile.decode(&decoder_registry)?;
                let (data, shape, _) = array.into_inner();

                let (data, shape) = if is_planar {
                    let [bands, h, w] = shape;
                    (planar_to_chunky(data, bands, h, w), [h, w, bands])
                } else {
                    (data, shape)
                };

                decoded_tiles.push((tx, ty, data, shape));
            }
        }

        // Determine pixel format from first tile and stitch
        let first_data = &decoded_tiles
            .first()
            .ok_or(Error::UnsupportedColorType)?
            .2;

        let (data, format) = match (samples_per_pixel, first_data) {
            (1, TypedArray::UInt8(_)) => {
                let mut buf = vec![0u8; (width * height) as usize];
                stitch_tiles_u8(&decoded_tiles, &mut buf, width, height, tile_width, tile_height, 1);
                let nodata_u8 = nodata_f64.map(|v| v as u8);
                let (min_val, max_val) = min_max_u8(&buf, nodata_u8);
                if nodata_u8.is_some() {
                    let nd = nodata_u8.unwrap();
                    let mut rgba = Vec::with_capacity(buf.len() * 4);
                    for &v in &buf {
                        if v == nd {
                            rgba.extend_from_slice(&[0, 0, 0, 0]);
                        } else {
                            let n = normalize_u8(v, min_val, max_val);
                            rgba.extend_from_slice(&[n, n, n, 255]);
                        }
                    }
                    (rgba, RasterFormat::Rgba8)
                } else {
                    let data: Vec<u8> = buf.iter().map(|&v| normalize_u8(v, min_val, max_val)).collect();
                    (data, RasterFormat::R8)
                }
            }
            (1, TypedArray::UInt16(_)) => {
                let mut buf16 = vec![0u16; (width * height) as usize];
                stitch_tiles_u16(
                    &decoded_tiles,
                    &mut buf16,
                    width,
                    height,
                    tile_width,
                    tile_height,
                    1,
                );
                let nodata_u16 = nodata_f64.map(|v| v as u16);
                let (min_val, max_val) = min_max_u16(&buf16, nodata_u16);
                if nodata_u16.is_some() {
                    let nd = nodata_u16.unwrap();
                    let mut rgba = Vec::with_capacity(buf16.len() * 4);
                    for &v in &buf16 {
                        if v == nd {
                            rgba.extend_from_slice(&[0, 0, 0, 0]);
                        } else {
                            let n = normalize_u16(v, min_val, max_val);
                            rgba.extend_from_slice(&[n, n, n, 255]);
                        }
                    }
                    (rgba, RasterFormat::Rgba8)
                } else {
                    let data: Vec<u8> = buf16.iter().map(|&v| normalize_u16(v, min_val, max_val)).collect();
                    (data, RasterFormat::R8)
                }
            }
            (3, TypedArray::UInt8(_)) => {
                let mut buf = vec![0u8; (width * height) as usize * 3];
                stitch_tiles_u8(&decoded_tiles, &mut buf, width, height, tile_width, tile_height, 3);
                let pixel_count = (width * height) as usize;
                let nodata_u8 = nodata_f64.map(|v| v as u8);
                let mut rgba = Vec::with_capacity(pixel_count * 4);
                for i in 0..pixel_count {
                    let base = i * 3;
                    if let (Some(&r), Some(&g), Some(&b)) =
                        (buf.get(base), buf.get(base + 1), buf.get(base + 2))
                    {
                        let alpha = if nodata_u8.is_some_and(|nd| r == nd && g == nd && b == nd) {
                            0
                        } else {
                            255
                        };
                        rgba.push(r);
                        rgba.push(g);
                        rgba.push(b);
                        rgba.push(alpha);
                    }
                }
                (rgba, RasterFormat::Rgba8)
            }
            (4, TypedArray::UInt8(_)) => {
                let mut buf = vec![0u8; (width * height) as usize * 4];
                stitch_tiles_u8(&decoded_tiles, &mut buf, width, height, tile_width, tile_height, 4);
                (buf, RasterFormat::Rgba8)
            }
            (3, TypedArray::UInt16(_)) => {
                let mut buf16 = vec![0u16; (width * height) as usize * 3];
                stitch_tiles_u16(
                    &decoded_tiles,
                    &mut buf16,
                    width,
                    height,
                    tile_width,
                    tile_height,
                    3,
                );
                let pixel_count = (width * height) as usize;
                let nodata_u16 = nodata_f64.map(|v| v as u16);
                let (min_val, max_val) = min_max_u16(&buf16, nodata_u16);
                let mut rgba = Vec::with_capacity(pixel_count * 4);
                for i in 0..pixel_count {
                    let base = i * 3;
                    let r = buf16[base];
                    let g = buf16[base + 1];
                    let b = buf16[base + 2];
                    let alpha = if nodata_u16.is_some_and(|nd| r == nd && g == nd && b == nd) {
                        0
                    } else {
                        255
                    };
                    rgba.push(normalize_u16(r, min_val, max_val));
                    rgba.push(normalize_u16(g, min_val, max_val));
                    rgba.push(normalize_u16(b, min_val, max_val));
                    rgba.push(alpha);
                }
                (rgba, RasterFormat::Rgba8)
            }
            (_, TypedArray::UInt8(_)) => {
                let spp = samples_per_pixel as usize;
                let mut buf = vec![0u8; (width * height) as usize * spp];
                stitch_tiles_u8(
                    &decoded_tiles,
                    &mut buf,
                    width,
                    height,
                    tile_width,
                    tile_height,
                    samples_per_pixel.into(),
                );
                let pixel_count = (width * height) as usize;
                let first_chan: Vec<u8> = (0..pixel_count).map(|i| buf[i * spp]).collect();
                let nodata_u8 = nodata_f64.map(|v| v as u8);
                let (min_val, max_val) = min_max_u8(&first_chan, nodata_u8);
                let data: Vec<u8> = first_chan.iter().map(|&v| normalize_u8(v, min_val, max_val)).collect();
                (data, RasterFormat::R8)
            }
            (_, TypedArray::UInt16(_)) => {
                let spp = samples_per_pixel as usize;
                let mut buf16 = vec![0u16; (width * height) as usize * spp];
                stitch_tiles_u16(
                    &decoded_tiles,
                    &mut buf16,
                    width,
                    height,
                    tile_width,
                    tile_height,
                    samples_per_pixel.into(),
                );
                let pixel_count = (width * height) as usize;
                let first_chan: Vec<u16> = (0..pixel_count).map(|i| buf16[i * spp]).collect();
                let nodata_u16 = nodata_f64.map(|v| v as u16);
                let (min_val, max_val) = min_max_u16(&first_chan, nodata_u16);
                let data: Vec<u8> = first_chan.iter().map(|&v| normalize_u16(v, min_val, max_val)).collect();
                (data, RasterFormat::R8)
            }
            _ => return Err(Error::UnsupportedColorType),
        };

        Ok(Raster {
            width,
            height,
            data,
            format,
            extent,
            epsg_code,
        })
    }
}

/// `GTRasterTypeGeoKey` value meaning the georeferenced point of a pixel is
/// its center rather than its top-left corner (`RasterPixelIsArea` = 1).
const RASTER_PIXEL_IS_POINT: u16 = 2;

/// Compute the extent covered by the raster's pixels (outer pixel edges).
///
/// Supports a single `ModelTiepointTag` + `ModelPixelScaleTag`, or a
/// north-up `ModelTransformationTag` (rotation/shear terms are ignored).
fn compute_extent(
    tiepoint: Option<&[f64]>,
    pixel_scale: Option<&[f64]>,
    transformation: Option<&[f64]>,
    raster_type: Option<u16>,
    width: u32,
    height: u32,
) -> Result<geo_types::Rect<f64>, Error> {
    // Map raster space (i, j) → model space (x, y) as
    // x = origin_x + i * scale_x, y = origin_y - j * scale_y.
    let (mut origin_x, mut origin_y, scale_x, scale_y) =
        if let (Some(tiepoint), Some(scale)) = (tiepoint, pixel_scale) {
            let [i, j, _k, x, y, ..] = tiepoint else {
                return Err(Error::MissingGeoInfo);
            };
            let [scale_x, scale_y, ..] = scale else {
                return Err(Error::MissingGeoInfo);
            };
            // The tiepoint ties raster point (i, j) to model point (x, y);
            // it's usually, but not necessarily, the (0, 0) corner.
            (x - i * scale_x, y + j * scale_y, *scale_x, *scale_y)
        } else if let Some(matrix) = transformation {
            let (Some(&scale_x), Some(&neg_scale_y), Some(&origin_x), Some(&origin_y)) =
                (matrix.first(), matrix.get(5), matrix.get(3), matrix.get(7))
            else {
                return Err(Error::MissingGeoInfo);
            };
            // Negate matrix[5] to match the positive convention used by ModelPixelScaleTag
            (origin_x, origin_y, scale_x, -neg_scale_y)
        } else {
            return Err(Error::MissingGeoInfo);
        };

    // With PixelIsPoint, raster point (0, 0) is the *center* of the top-left
    // pixel, so its outer corner is half a pixel up and to the left.
    if raster_type == Some(RASTER_PIXEL_IS_POINT) {
        origin_x -= scale_x / 2.0;
        origin_y += scale_y / 2.0;
    }

    Ok(geo_types::Rect::new(
        geo_types::coord! { x: origin_x, y: origin_y - scale_y * f64::from(height) },
        geo_types::coord! { x: origin_x + scale_x * f64::from(width), y: origin_y },
    ))
}

/// Convert tile data from planar layout [bands, height, width] to chunky layout [height, width, bands].
fn planar_to_chunky(data: TypedArray, bands: usize, height: usize, width: usize) -> TypedArray {
    match data {
        TypedArray::UInt8(planar) => {
            let mut chunky = vec![0u8; planar.len()];
            for b in 0..bands {
                for h in 0..height {
                    for w in 0..width {
                        chunky[(h * width + w) * bands + b] =
                            planar[b * height * width + h * width + w];
                    }
                }
            }
            TypedArray::UInt8(chunky)
        }
        TypedArray::UInt16(planar) => {
            let mut chunky = vec![0u16; planar.len()];
            for b in 0..bands {
                for h in 0..height {
                    for w in 0..width {
                        chunky[(h * width + w) * bands + b] =
                            planar[b * height * width + h * width + w];
                    }
                }
            }
            TypedArray::UInt16(chunky)
        }
        other => other,
    }
}

fn normalize_u8(val: u8, min: u8, max: u8) -> u8 {
    if min == 0 && max == 255 {
        return val;
    }
    if min >= max {
        return val;
    }
    ((val.saturating_sub(min) as u16 * 255) / (max - min) as u16) as u8
}

fn normalize_u16(val: u16, min: u16, max: u16) -> u8 {
    if min >= max {
        return (val >> 8) as u8;
    }
    ((val.saturating_sub(min) as u32 * 255) / (max - min) as u32) as u8
}

fn min_max_u8(data: &[u8], nodata: Option<u8>) -> (u8, u8) {
    let mut min = u8::MAX;
    let mut max = u8::MIN;
    for &v in data {
        if nodata.is_some_and(|nd| v == nd) {
            continue;
        }
        min = min.min(v);
        max = max.max(v);
    }
    if min > max {
        (0, 255)
    } else {
        (min, max)
    }
}

fn min_max_u16(data: &[u16], nodata: Option<u16>) -> (u16, u16) {
    let mut min = u16::MAX;
    let mut max = u16::MIN;
    for &v in data {
        if nodata.is_some_and(|nd| v == nd) {
            continue;
        }
        min = min.min(v);
        max = max.max(v);
    }
    if min > max {
        (0, 65535)
    } else {
        (min, max)
    }
}

fn stitch_tiles_u8(
    tiles: &[(usize, usize, TypedArray, [usize; 3])],
    buf: &mut [u8],
    image_width: u32,
    image_height: u32,
    tile_width: u32,
    tile_height: u32,
    channels: u32,
) {
    for (tx, ty, data, shape) in tiles {
        if let TypedArray::UInt8(tile_data) = data {
            let src_w = shape[1] as u32;

            let tile_x_offset = (*tx as u32) * tile_width;
            let tile_y_offset = (*ty as u32) * tile_height;
            let copy_w = src_w.min(image_width.saturating_sub(tile_x_offset));
            let copy_h = (shape[0] as u32).min(image_height.saturating_sub(tile_y_offset));

            for row in 0..copy_h {
                let src_start = (row * src_w * channels) as usize;
                let dst_start =
                    ((tile_y_offset + row) * image_width + tile_x_offset) as usize * channels as usize;
                let copy_len = (copy_w * channels) as usize;
                if let (Some(src), Some(dst)) = (
                    tile_data.get(src_start..src_start + copy_len),
                    buf.get_mut(dst_start..dst_start + copy_len),
                ) {
                    dst.copy_from_slice(src);
                }
            }
        }
    }
}

fn stitch_tiles_u16(
    tiles: &[(usize, usize, TypedArray, [usize; 3])],
    buf: &mut [u16],
    image_width: u32,
    image_height: u32,
    tile_width: u32,
    tile_height: u32,
    channels: u32,
) {
    for (tx, ty, data, shape) in tiles {
        if let TypedArray::UInt16(tile_data) = data {
            let src_w = shape[1] as u32;

            let tile_x_offset = (*tx as u32) * tile_width;
            let tile_y_offset = (*ty as u32) * tile_height;
            let copy_w = src_w.min(image_width.saturating_sub(tile_x_offset));
            let copy_h = (shape[0] as u32).min(image_height.saturating_sub(tile_y_offset));

            for row in 0..copy_h {
                let src_start = (row * src_w * channels) as usize;
                let dst_start =
                    ((tile_y_offset + row) * image_width + tile_x_offset) as usize * channels as usize;
                let copy_len = (copy_w * channels) as usize;
                if let (Some(src), Some(dst)) = (
                    tile_data.get(src_start..src_start + copy_len),
                    buf.get_mut(dst_start..dst_start + copy_len),
                ) {
                    dst.copy_from_slice(src);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(r: geo_types::Rect<f64>) -> [f64; 4] {
        [r.min().x, r.min().y, r.max().x, r.max().y]
    }

    #[test]
    fn extent_from_tiepoint_at_origin() {
        let e = compute_extent(
            Some(&[0.0, 0.0, 0.0, -180.0, 90.0, 0.0]),
            Some(&[0.1, 0.1, 0.0]),
            None,
            Some(1),
            3600,
            1800,
        )
        .unwrap();
        let [a, b, c, d] = bounds(e);
        assert!((a + 180.0).abs() < 1e-9 && (b + 90.0).abs() < 1e-9);
        assert!((c - 180.0).abs() < 1e-9 && (d - 90.0).abs() < 1e-9);
    }

    #[test]
    fn extent_from_tiepoint_not_at_origin() {
        // Raster point (10, 20) is tied to (-179, 88), so (0, 0) is at
        // (-180, 90).
        let e = compute_extent(
            Some(&[10.0, 20.0, 0.0, -179.0, 88.0, 0.0]),
            Some(&[0.1, 0.1, 0.0]),
            None,
            None,
            3600,
            1800,
        )
        .unwrap();
        let [a, b, c, d] = bounds(e);
        assert!((a + 180.0).abs() < 1e-9 && (b + 90.0).abs() < 1e-9, "{e:?}");
        assert!((c - 180.0).abs() < 1e-9 && (d - 90.0).abs() < 1e-9, "{e:?}");
    }

    #[test]
    fn extent_pixel_is_point_shifts_half_pixel() {
        // Pixel centers span -179.95..179.95, 89.95..-89.95.
        let e = compute_extent(
            Some(&[0.0, 0.0, 0.0, -179.95, 89.95, 0.0]),
            Some(&[0.1, 0.1, 0.0]),
            None,
            Some(RASTER_PIXEL_IS_POINT),
            3600,
            1800,
        )
        .unwrap();
        let [a, b, c, d] = bounds(e);
        assert!((a + 180.0).abs() < 1e-9 && (b + 90.0).abs() < 1e-9, "{e:?}");
        assert!((c - 180.0).abs() < 1e-9 && (d - 90.0).abs() < 1e-9, "{e:?}");
    }

    #[test]
    fn extent_from_model_transformation() {
        #[rustfmt::skip]
        let matrix = [
            0.1, 0.0, 0.0, -180.0,
            0.0, -0.1, 0.0, 90.0,
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let e = compute_extent(None, None, Some(&matrix), None, 3600, 1800).unwrap();
        let [a, b, c, d] = bounds(e);
        assert!((a + 180.0).abs() < 1e-9 && (b + 90.0).abs() < 1e-9, "{e:?}");
        assert!((c - 180.0).abs() < 1e-9 && (d - 90.0).abs() < 1e-9, "{e:?}");
    }

    #[test]
    fn extent_requires_georeferencing() {
        assert!(compute_extent(None, None, None, None, 10, 10).is_err());
    }
}
