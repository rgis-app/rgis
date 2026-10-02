use arrow::array::Array;
use arrow::record_batch::RecordBatch;
use geo::{BoundingRect, Contains};
use std::{fmt, iter, num, sync};

#[derive(Default)]
pub struct FeatureBuilder<Scalar: geo::CoordNum> {
    geometry: Option<geo::Geometry<Scalar>>,
}

impl<Scalar: geo::CoordNum> FeatureBuilder<Scalar> {
    pub fn new() -> Self {
        FeatureBuilder { geometry: None }
    }

    pub fn with_geometry(self, geometry: geo::Geometry<Scalar>) -> Self {
        FeatureBuilder {
            geometry: Some(geometry),
        }
    }

    pub fn build(self) -> Feature<Scalar> {
        let bounding_rect = self
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.bounding_rect());
        Feature {
            id: FeatureId::new(),
            geometry: self.geometry,
            bounding_rect,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Feature<Scalar: geo::CoordNum> {
    pub id: FeatureId,
    pub geometry: Option<geo::Geometry<Scalar>>,
    pub bounding_rect: Option<geo::Rect<Scalar>>,
}

impl<Scalar: geo::CoordNum> geo::CoordsIter for Feature<Scalar> {
    type Scalar = Scalar;
    type Iter<'a>
        = Box<dyn Iterator<Item = geo::Coord<Self::Scalar>> + 'a>
    where
        Scalar: 'a;
    type ExteriorIter<'a>
        = Box<dyn Iterator<Item = geo::Coord<Self::Scalar>> + 'a>
    where
        Scalar: 'a;

    fn coords_count(&self) -> usize {
        self.geometry
            .as_ref()
            .map(|g| g.coords_count())
            .unwrap_or(0)
    }

    fn coords_iter(&self) -> Self::Iter<'_> {
        match self.geometry {
            Some(ref g) => Box::new(g.coords_iter()),
            None => Box::new(iter::empty()),
        }
    }

    fn exterior_coords_iter(&self) -> Self::ExteriorIter<'_> {
        match self.geometry {
            Some(ref g) => Box::new(g.exterior_coords_iter()),
            None => Box::new(iter::empty()),
        }
    }
}

impl<Scalar: geo::CoordNum> Feature<Scalar> {
    pub fn recalculate_bounding_rect(&mut self) {
        self.bounding_rect = self
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.bounding_rect());
    }
}

impl<Scalar, G> Contains<G> for Feature<Scalar>
where
    Scalar: geo::CoordNum,
    geo::Rect<Scalar>: Contains<G>,
    geo::Geometry<Scalar>: Contains<G>,
{
    fn contains(&self, coord: &G) -> bool {
        self.bounding_rect
            .as_ref()
            .map(|bounding_rect| bounding_rect.contains(coord))
            .unwrap_or(false)
            && self
                .geometry
                .as_ref()
                .map(|geometry| geometry.contains(coord))
                .unwrap_or(false)
    }
}

#[derive(Default, Clone, Debug)]
pub struct FeatureCollection<Scalar: geo::CoordNum> {
    pub features: Vec<Feature<Scalar>>,
    pub bounding_rect: Option<geo::Rect<Scalar>>,
    pub properties: Option<RecordBatch>,
}

impl<Scalar: geo::CoordNum + Default> FeatureCollection<Scalar> {
    pub fn new() -> Self {
        FeatureCollection::default()
    }
}

impl<Scalar: geo::CoordNum> geo::CoordsIter for FeatureCollection<Scalar> {
    type Scalar = Scalar;
    type Iter<'a>
        = Box<dyn Iterator<Item = geo::Coord<Self::Scalar>> + 'a>
    where
        Scalar: 'a;
    type ExteriorIter<'a>
        = Box<dyn Iterator<Item = geo::Coord<Self::Scalar>> + 'a>
    where
        Scalar: 'a;

    fn coords_count(&self) -> usize {
        self.features.iter().map(|f| f.coords_count()).sum()
    }

    fn coords_iter(&self) -> Self::Iter<'_> {
        Box::new(self.features.iter().flat_map(|feature| feature.coords_iter()))
    }

    fn exterior_coords_iter(&self) -> Self::ExteriorIter<'_> {
        Box::new(
            self.features
                .iter()
                .flat_map(|feature| feature.exterior_coords_iter()),
        )
    }
}

impl<Scalar> Contains<geo::Coord<Scalar>> for FeatureCollection<Scalar>
where
    Scalar: geo::CoordNum,
    geo::Geometry<Scalar>: Contains<geo::Coord<Scalar>>,
{
    fn contains(&self, coord: &geo::Coord<Scalar>) -> bool {
        self.bounding_rect
            .as_ref()
            .map(|bounding_rect| bounding_rect.contains(coord))
            .unwrap_or(false)
            && self.features.iter().any(|feature| {
                feature
                    .geometry
                    .as_ref()
                    .map(|geometry| geometry.contains(coord))
                    .unwrap_or(false)
            })
    }
}

#[derive(Debug)]
pub struct BoundingRectError;

impl fmt::Display for BoundingRectError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "could not generate bounding rect")
    }
}

impl std::error::Error for BoundingRectError {}

impl<Scalar: geo::CoordNum> FeatureCollection<Scalar> {
    pub fn from_geometry(geometry: geo::Geometry<Scalar>) -> Self {
        let feature = FeatureBuilder::new().with_geometry(geometry).build();
        Self::from_feature(feature)
    }

    pub fn from_feature(feature: Feature<Scalar>) -> Self {
        FeatureCollection {
            bounding_rect: feature.bounding_rect,
            features: vec![feature],
            properties: None,
        }
    }

    pub fn from_features(features: Vec<Feature<Scalar>>) -> Self {
        FeatureCollection {
            bounding_rect: bounding_rect_from_features(&features),
            features,
            properties: None,
        }
    }

    pub fn from_features_with_properties(
        features: Vec<Feature<Scalar>>,
        properties: RecordBatch,
    ) -> Self {
        FeatureCollection {
            bounding_rect: bounding_rect_from_features(&features),
            features,
            properties: Some(properties),
        }
    }

    pub fn geometry_iter(&self) -> impl Iterator<Item = &geo::Geometry<Scalar>> {
        self.features.iter().filter_map(|f| f.geometry.as_ref())
    }

    pub fn to_geometry_collection(&self) -> geo::GeometryCollection<Scalar> {
        geo::GeometryCollection(self.geometry_iter().cloned().collect())
    }

    pub fn bounding_rect(&self) -> Result<geo::Rect<Scalar>, BoundingRectError> {
        rect_merge_many(
            self.geometry_iter()
                .filter_map(|geometry| geometry.bounding_rect()),
        )
    }

    pub fn recalculate_bounding_rect(&mut self) {
        self.bounding_rect = bounding_rect_from_features(&self.features);
    }
}

/// Extract properties for a single feature (row) as key-value string pairs.
pub fn properties_for_row(record_batch: &RecordBatch, row: usize) -> Vec<(String, String)> {
    let schema = record_batch.schema();
    schema
        .fields()
        .iter()
        .enumerate()
        .map(|(col_idx, field)| {
            let name = field.name().clone();
            let col = record_batch.column(col_idx);
            let value = array_value_to_string(col, row);
            (name, value)
        })
        .collect()
}

fn array_value_to_string(array: &dyn Array, row: usize) -> String {
    use arrow::array::{self as aa};
    use arrow::datatypes::DataType;

    if array.is_null(row) {
        return "null".to_string();
    }

    match array.data_type() {
        DataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<aa::StringArray>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<aa::LargeStringArray>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<aa::Float64Array>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<aa::Float32Array>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<aa::Int64Array>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<aa::Int32Array>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<aa::BooleanArray>();
            match arr {
                Some(a) => a.value(row).to_string(),
                None => "<error>".to_string(),
            }
        }
        _ => format!("<unsupported type: {:?}>", array.data_type()),
    }
}

fn bounding_rect_from_features<Scalar: geo::CoordNum>(
    features: &[Feature<Scalar>],
) -> Option<geo::Rect<Scalar>> {
    features
        .iter()
        .map(|feature| feature.bounding_rect)
        .fold(None, option_rect_merge)
}

// TODO: this assumes the iterator has one item. is that okay?
fn rect_merge_many<Scalar: geo::CoordNum>(
    mut iter: impl Iterator<Item = geo::Rect<Scalar>>,
) -> Result<geo::Rect<Scalar>, BoundingRectError> {
    let first = iter.next().ok_or(BoundingRectError)?;
    Ok(iter.fold(first, rect_merge))
}

fn option_rect_merge<Scalar: geo::CoordNum>(
    a: Option<geo::Rect<Scalar>>,
    b: Option<geo::Rect<Scalar>>,
) -> Option<geo::Rect<Scalar>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(rect_merge(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn rect_merge<Scalar: geo::CoordNum>(
    a: geo::Rect<Scalar>,
    b: geo::Rect<Scalar>,
) -> geo::Rect<Scalar> {
    geo::Rect::new(
        geo::Coord {
            x: min(a.min().x, b.min().x),
            y: min(a.min().y, b.min().y),
        },
        geo::Coord {
            x: max(a.max().x, b.max().x),
            y: max(a.max().y, b.max().y),
        },
    )
}

// Hack because `min` and `max` are not implemented for `PartialOrd`
fn min<Scalar: geo::CoordNum>(a: Scalar, b: Scalar) -> Scalar {
    if a < b {
        a
    } else {
        b
    }
}

// Hack because `min` and `max` are not implemented for `PartialOrd`
fn max<Scalar: geo::CoordNum>(a: Scalar, b: Scalar) -> Scalar {
    if a > b {
        a
    } else {
        b
    }
}

// The starting value is `1` so we can utilize `NonZeroU64`.
static NEXT_ID: sync::atomic::AtomicU64 = sync::atomic::AtomicU64::new(1);

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct FeatureId(num::NonZeroU64);

impl Default for FeatureId {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureId {
    pub fn new() -> Self {
        FeatureId(new_id())
    }
}

fn new_id() -> num::NonZeroU64 {
    #[allow(clippy::expect_used)]
    num::NonZeroU64::new(NEXT_ID.fetch_add(1, sync::atomic::Ordering::SeqCst))
        .expect("Encountered more than 2^64 features")
}
