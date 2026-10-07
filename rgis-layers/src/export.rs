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
                write_property(writer, i, field.name(), record_batch.column(i), idx)?;
            }
        }
        writer.properties_end()?;

        writer.feature_end(idx as u64)?;
    }

    writer.dataset_end()?;
    Ok(())
}

fn write_property<W: PropertyProcessor>(
    writer: &mut W,
    i: usize,
    name: &str,
    array: &dyn Array,
    row: usize,
) -> Result<(), GeozeroError> {
    if let Some(value) = property_value(array, row)? {
        writer.property(i, name, &value)?;
    } else {
        // Types without a native property representation are written as display strings.
        let value = arrow::util::display::array_value_to_string(array, row)
            .map_err(|e| GeozeroError::Property(e.to_string()))?;
        writer.property(i, name, &ColumnValue::String(&value))?;
    }
    Ok(())
}

fn property_value(array: &dyn Array, row: usize) -> Result<Option<ColumnValue<'_>>, GeozeroError> {
    const NULL: ColumnValue<'static> = ColumnValue::Json("null");

    if array.is_null(row) {
        return Ok(Some(NULL));
    }

    let invalid_array =
        || GeozeroError::Property(format!("Invalid Arrow array for {:?}", array.data_type()));
    Ok(Some(match array.data_type() {
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
        // JSON has no representation for NaN or infinity.
        DataType::Float64 => {
            let value = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(invalid_array)?
                .value(row);
            if value.is_finite() {
                ColumnValue::Double(value)
            } else {
                NULL
            }
        }
        DataType::Float32 => {
            let value = array
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(invalid_array)?
                .value(row);
            if value.is_finite() {
                ColumnValue::Float(value)
            } else {
                NULL
            }
        }
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
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{ArrayRef, Int8Array};
    use arrow::datatypes::{Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    #[test]
    fn geojson_export_preserves_property_types() {
        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("count", DataType::Int64, true),
            Field::new("score", DataType::Float64, true),
            Field::new("active", DataType::Boolean, true),
            Field::new("small", DataType::Int8, true),
        ]);
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(vec![Some("a"), None])),
            Arc::new(Int64Array::from(vec![Some(3), None])),
            Arc::new(Float64Array::from(vec![Some(1.5), Some(f64::NAN)])),
            Arc::new(BooleanArray::from(vec![Some(true), None])),
            Arc::new(Int8Array::from(vec![Some(7), None])),
        ];
        let mut fc = geo_features::FeatureCollection::new();
        fc.features = vec![geo_features::FeatureBuilder::new().build(); 2];
        fc.properties = RecordBatch::try_new(Arc::new(schema), columns).ok();

        let out = export_feature_collection(&fc, rgis_primitives::ExportFormat::GeoJson)
            .unwrap_or_default();

        assert!(out.contains(r#""name": "a""#), "{out}");
        assert!(out.contains(r#""count": 3"#), "{out}");
        assert!(out.contains(r#""score": 1.5"#), "{out}");
        assert!(out.contains(r#""active": true"#), "{out}");
        // Types without a native mapping fall back to strings.
        assert!(out.contains(r#""small": "7""#), "{out}");
        // Nulls and non-finite floats become JSON null.
        assert!(out.contains(r#""name": null"#), "{out}");
        assert!(out.contains(r#""score": null"#), "{out}");
    }
}
