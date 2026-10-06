use arrow::array::{ArrayRef, BooleanBuilder, Float64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use geo_features::FeatureCollection;
use geo_projected::WrapTo;
use std::collections::HashMap;
use std::sync::Arc;

pub struct LoadFileJob {
    pub file_format: geo_file_loader::FileFormat,
    pub bytes: bytes::Bytes,
    pub name: String,
    pub source_crs: rgis_primitives::Crs,
}

pub enum LoadFileJobOutcome {
    Vector {
        feature_collection: FeatureCollection<geo_projected::UnprojectedScalar>,
        name: String,
        source_crs: rgis_primitives::Crs,
    },
    Raster {
        raster: geo_raster::Raster,
        name: String,
        source_crs: rgis_primitives::Crs,
    },
}

impl bevy_jobs::Job for LoadFileJob {
    type Outcome = Result<LoadFileJobOutcome, geo_file_loader::Error>;
    const JOB_TYPE: bevy_jobs::JobType = bevy_jobs::JobType::Io;

    fn name(&self) -> String {
        format!("Loading {} file", self.file_format.display_name())
    }

    async fn perform(self, _: bevy_jobs::Context) -> Self::Outcome {
        if self.file_format.is_raster() {
            let raster = geo_file_loader::load_raster_file(self.bytes).await?;
            Ok(LoadFileJobOutcome::Raster {
                raster,
                name: self.name,
                source_crs: self.source_crs,
            })
        } else {
            let is_geographic = self.source_crs.is_geographic();
            let loaded_features = geo_file_loader::load_file(self.file_format, self.bytes)?;

            let mut features = Vec::with_capacity(loaded_features.len());
            let mut property_maps: Vec<HashMap<String, geo_file_loader::OwnedColumnValue>> =
                Vec::with_capacity(loaded_features.len());

            for f in loaded_features {
                // Fix the geometry before building the feature so its bounding
                // rect describes the fixed geometry, not the original.
                let geometry = if is_geographic {
                    geo_fix_antimeridian::fix_geometry(&f.geometry)
                } else {
                    f.geometry
                };
                let feature = geo_features::FeatureBuilder::new()
                    .with_geometry(geometry)
                    .build();
                features.push(feature);
                property_maps.push(f.properties);
            }

            let properties = build_record_batch(&property_maps);
            let mut fc = FeatureCollection::from_features(features);
            fc.properties = properties;

            Ok(LoadFileJobOutcome::Vector {
                feature_collection: fc.wrap(),
                name: self.name,
                source_crs: self.source_crs,
            })
        }
    }
}

/// Determine the Arrow DataType for a column by examining all values.
fn infer_column_type(
    property_maps: &[HashMap<String, geo_file_loader::OwnedColumnValue>],
    key: &str,
) -> DataType {
    for map in property_maps {
        if let Some(val) = map.get(key) {
            return match val {
                geo_file_loader::OwnedColumnValue::String(_)
                | geo_file_loader::OwnedColumnValue::Binary(_)
                | geo_file_loader::OwnedColumnValue::Json(_)
                | geo_file_loader::OwnedColumnValue::DateTime(_) => DataType::Utf8,
                geo_file_loader::OwnedColumnValue::Bool(_) => DataType::Boolean,
                geo_file_loader::OwnedColumnValue::Int(_)
                | geo_file_loader::OwnedColumnValue::UInt(_)
                | geo_file_loader::OwnedColumnValue::Long(_)
                | geo_file_loader::OwnedColumnValue::ULong(_)
                | geo_file_loader::OwnedColumnValue::Float(_)
                | geo_file_loader::OwnedColumnValue::Double(_)
                | geo_file_loader::OwnedColumnValue::Byte(_)
                | geo_file_loader::OwnedColumnValue::UByte(_)
                | geo_file_loader::OwnedColumnValue::Short(_)
                | geo_file_loader::OwnedColumnValue::UShort(_) => DataType::Float64,
            };
        }
    }
    DataType::Utf8
}

fn build_record_batch(
    property_maps: &[HashMap<String, geo_file_loader::OwnedColumnValue>],
) -> Option<RecordBatch> {
    // Collect all unique keys in deterministic order.
    let mut keys: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for map in property_maps {
        for key in map.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys.sort();

    if keys.is_empty() {
        return None;
    }

    let num_rows = property_maps.len();
    let mut fields = Vec::with_capacity(keys.len());
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(keys.len());

    for key in &keys {
        let dtype = infer_column_type(property_maps, key);
        fields.push(Field::new(key, dtype.clone(), true));

        match dtype {
            DataType::Utf8 => {
                let mut builder = StringBuilder::with_capacity(num_rows, 0);
                for map in property_maps {
                    match map.get(key) {
                        Some(val) => builder.append_value(column_value_to_string(val)),
                        None => builder.append_null(),
                    }
                }
                columns.push(Arc::new(builder.finish()));
            }
            DataType::Float64 => {
                let mut builder = Float64Builder::with_capacity(num_rows);
                for map in property_maps {
                    match map.get(key) {
                        Some(val) => match column_value_to_f64(val) {
                            Some(n) => builder.append_value(n),
                            None => builder.append_null(),
                        },
                        None => builder.append_null(),
                    }
                }
                columns.push(Arc::new(builder.finish()));
            }
            DataType::Boolean => {
                let mut builder = BooleanBuilder::with_capacity(num_rows);
                for map in property_maps {
                    match map.get(key) {
                        Some(geo_file_loader::OwnedColumnValue::Bool(b)) => {
                            builder.append_value(*b)
                        }
                        Some(_) => builder.append_null(),
                        None => builder.append_null(),
                    }
                }
                columns.push(Arc::new(builder.finish()));
            }
            _ => unreachable!(),
        }
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, columns).ok()
}

fn column_value_to_string(val: &geo_file_loader::OwnedColumnValue) -> String {
    match val {
        geo_file_loader::OwnedColumnValue::String(s) => s.clone(),
        geo_file_loader::OwnedColumnValue::DateTime(s) => s.clone(),
        geo_file_loader::OwnedColumnValue::Binary(b) => format!("{b:?}"),
        geo_file_loader::OwnedColumnValue::Json(j) => format!("{j:?}"),
        other => match column_value_to_f64(other) {
            Some(n) => n.to_string(),
            None => "null".to_string(),
        },
    }
}

fn column_value_to_f64(val: &geo_file_loader::OwnedColumnValue) -> Option<f64> {
    match val {
        geo_file_loader::OwnedColumnValue::Int(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::UInt(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::Long(n) => Some(*n as f64),
        geo_file_loader::OwnedColumnValue::ULong(n) => Some(*n as f64),
        geo_file_loader::OwnedColumnValue::Float(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::Double(n) => Some(*n),
        geo_file_loader::OwnedColumnValue::Byte(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::UByte(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::Short(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::UShort(n) => Some(f64::from(*n)),
        geo_file_loader::OwnedColumnValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}
