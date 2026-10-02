use arrow::array::{
    Array, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, LargeStringArray,
    StringArray,
};
use arrow::datatypes::DataType;
use geo::MapCoords;
use geozero::error::GeozeroError;
use geozero::{ColumnValue, FeatureProcessor, GeozeroGeometry, PropertyProcessor};

pub fn export_feature_collection(
    fc: &geo_features::FeatureCollection<geo_projected::UnprojectedScalar>,
    format: rgis_primitives::ExportFormat,
) -> Result<String, geozero::error::GeozeroError> {
    let mut out: Vec<u8> = Vec::new();

    match format {
        rgis_primitives::ExportFormat::GeoJson => {
            let mut writer = geozero::geojson::GeoJsonWriter::new(&mut out);
            write_features(fc, &mut writer)?;
        }
        rgis_primitives::ExportFormat::Wkt => {
            let mut writer = geozero::wkt::WktWriter::new(&mut out);
            write_features(fc, &mut writer)?;
        }
    }

    String::from_utf8(out)
        .map_err(|_| geozero::error::GeozeroError::Geometry("Invalid UTF-8".to_string()))
}

fn write_features<W: FeatureProcessor + geozero::GeomProcessor + PropertyProcessor>(
    fc: &geo_features::FeatureCollection<geo_projected::UnprojectedScalar>,
    writer: &mut W,
) -> Result<(), geozero::error::GeozeroError> {
    writer.dataset_begin(None)?;

    for (idx, feature) in fc.features.iter().enumerate() {
        writer.feature_begin(idx as u64)?;

        if let Some(ref geom) = feature.geometry {
            writer.geometry_begin()?;
            let geom_f64: geo::Geometry<f64> =
                geom.map_coords(|coord| geo::Coord { x: coord.x.0, y: coord.y.0 });
            geom_f64.process_geom(writer)?;
            writer.geometry_end()?;
        }

        writer.properties_begin()?;
        if let Some(ref record_batch) = fc.properties {
            for (i, field) in record_batch.schema().fields().iter().enumerate() {
                let value = property_value(record_batch.column(i), idx)?;
                writer.property(i, field.name(), &value)?;
            }
        }
        writer.properties_end()?;

        writer.feature_end(idx as u64)?;
    }

    writer.dataset_end()?;
    Ok(())
}

fn property_value(array: &dyn Array, row: usize) -> Result<ColumnValue<'_>, GeozeroError> {
    if array.is_null(row) {
        return Ok(ColumnValue::Json("null"));
    }

    let invalid_array = || {
        GeozeroError::Geometry(format!("Invalid Arrow array for {:?}", array.data_type()))
    };
    Ok(match array.data_type() {
        DataType::Utf8 => ColumnValue::String(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::LargeUtf8 => ColumnValue::String(
            array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::Float64 => ColumnValue::Double(
            array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::Float32 => ColumnValue::Float(
            array
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::Int64 => ColumnValue::Long(
            array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::Int32 => ColumnValue::Int(
            array
                .as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        DataType::Boolean => ColumnValue::Bool(
            array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(invalid_array)?
                .value(row),
        ),
        other => {
            return Err(GeozeroError::Geometry(format!(
                "Cannot export Arrow property type {other:?}"
            )));
        }
    })
}
