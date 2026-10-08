use bevy::prelude::*;
use geo::contains::Contains;
use std::sync::Arc;

pub mod export;
mod systems;

// ---------------------------------------------------------------------------
// ECS Components – each former `Layer` field is now its own Component.
// ---------------------------------------------------------------------------

/// Marker component that tags an entity as a layer.
#[derive(Component, Debug)]
pub struct LayerMarker;

/// Human-readable name of the layer.
#[derive(Component, Debug, Clone)]
pub struct LayerName(pub String);

/// Whether the layer is currently visible.
#[derive(Component, Debug, Clone, Copy)]
pub struct LayerVisible(pub bool);

/// Fill and stroke colors.
#[derive(Component, Clone, Debug)]
pub struct LayerColor {
    pub fill: Option<Color>,
    pub stroke: Color,
}

/// The coordinate reference system the layer's source data is in.
#[derive(Component, Debug, Clone)]
pub struct LayerCrs(pub rgis_primitives::Crs);

/// Point rendering size.
#[derive(Component, Debug, Clone, Copy)]
pub struct LayerPointSize(pub f32);

/// The actual geospatial data (vector or raster).
#[derive(Component, Debug)]
pub enum LayerData {
    Vector {
        unprojected_feature_collection:
            Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
        projected_feature_collection:
            Option<geo_features::FeatureCollection<geo_projected::ProjectedScalar>>,
        geom_type: geo_geom_type::GeomType,
    },
    Raster {
        raster: geo_raster::Raster,
        projected_grid: Option<ProjectedRasterGrid>,
    },
}

/// Marker component placed on the currently selected layer.
#[derive(Component, Debug)]
pub struct SelectedLayer;

/// The feature currently selected on the map (e.g. by clicking it with the
/// query tool or picking it in the attribute table), if any.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SelectedFeature(pub Option<(rgis_primitives::LayerId, geo_features::FeatureId)>);

/// Z-order index – lower values are rendered below higher values.
/// Stored so that the renderer can calculate proper z positions.
#[derive(Component, Debug, Clone, Copy)]
pub struct LayerZIndex(pub usize);

// ---------------------------------------------------------------------------
// Supporting types (unchanged)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct LayerIndex(pub usize);

/// A raster's footprint in the target CRS, as a `(cols + 1) × (rows + 1)`
/// grid of vertices. Vertex `row * (cols + 1) + col` holds the projected
/// position of a source-CRS sample point and the texture coordinate of that
/// same point.
#[derive(Debug)]
pub struct ProjectedRasterGrid {
    pub cols: u32,
    pub rows: u32,
    pub positions: Vec<[f32; 2]>,
    /// Texture coordinates, `[0, 0]` = top-left of the image. These are not
    /// necessarily `[col / cols, row / rows]`: the sampled area may be only
    /// part of the raster (e.g. clamped to the target CRS's area of use).
    pub uvs: Vec<[f32; 2]>,
    pub valid: Vec<bool>,
    pub extent: geo::Rect<f64>,
}

impl ProjectedRasterGrid {
    /// Vertex-index triples for the triangles to render, two per grid cell
    /// whose four corners are all valid.
    pub fn triangles(&self) -> impl Iterator<Item = [usize; 3]> + '_ {
        let stride = self.cols as usize + 1;
        (0..self.rows as usize)
            .flat_map(move |row| (0..self.cols as usize).map(move |col| (row, col)))
            .filter_map(move |(row, col)| {
                let tl = row * stride + col;
                let tr = tl + 1;
                let bl = tl + stride;
                let br = bl + 1;
                [tl, tr, bl, br]
                    .iter()
                    .all(|&i| self.valid[i])
                    .then_some([[tl, bl, tr], [tr, bl, br]])
            })
            .flatten()
    }
}

// ---------------------------------------------------------------------------
// Bundle – for convenient spawning
// ---------------------------------------------------------------------------

#[derive(Bundle)]
pub struct LayerBundle {
    pub marker: LayerMarker,
    pub id: rgis_primitives::LayerId,
    pub name: LayerName,
    pub visible: LayerVisible,
    pub color: LayerColor,
    pub crs: LayerCrs,
    pub point_size: LayerPointSize,
    pub data: LayerData,
    pub z_index: LayerZIndex,
}

// ---------------------------------------------------------------------------
// The `Layers` resource is kept as a thin shim for the z-ordering / count
// information that doesn't map neatly to individual entities.
// Over time even this can be removed.
// ---------------------------------------------------------------------------

#[derive(Debug, Resource, Default)]
pub struct LayerOrder {
    /// Ordered list of layer entity IDs from bottom to top.
    pub order: Vec<Entity>,
}

impl LayerOrder {
    pub fn count(&self) -> usize {
        self.order.len()
    }

    pub fn index_of(&self, entity: Entity) -> Option<usize> {
        self.order.iter().position(|e| *e == entity)
    }

    pub fn push(&mut self, entity: Entity) {
        self.order.push(entity);
    }

    pub fn remove(&mut self, entity: Entity) {
        if let Some(idx) = self.index_of(entity) {
            self.order.remove(idx);
        }
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        self.order.swap(a, b);
    }

    pub fn get(&self, index: usize) -> Option<Entity> {
        self.order.get(index).copied()
    }

    pub fn iter_bottom_to_top(&self) -> impl Iterator<Item = Entity> + '_ {
        self.order.iter().copied()
    }

    pub fn iter_top_to_bottom(&self) -> impl Iterator<Item = Entity> + '_ {
        self.order.iter().rev().copied()
    }
}

// ---------------------------------------------------------------------------
// Helper resource to map LayerId -> Entity (for message-based lookups).
// ---------------------------------------------------------------------------

#[derive(Debug, Resource, Default)]
pub struct LayerIdToEntity {
    map: std::collections::HashMap<rgis_primitives::LayerId, Entity>,
}

impl LayerIdToEntity {
    pub fn insert(&mut self, id: rgis_primitives::LayerId, entity: Entity) {
        self.map.insert(id, entity);
    }

    pub fn get(&self, id: rgis_primitives::LayerId) -> Option<Entity> {
        self.map.get(&id).copied()
    }

    pub fn remove(&mut self, id: rgis_primitives::LayerId) {
        self.map.remove(&id);
    }
}

// ---------------------------------------------------------------------------
// LayerData helper methods
// ---------------------------------------------------------------------------

impl LayerData {
    pub fn is_vector(&self) -> bool {
        matches!(self, LayerData::Vector { .. })
    }

    pub fn is_raster(&self) -> bool {
        matches!(self, LayerData::Raster { .. })
    }

    pub fn geom_type(&self) -> Option<geo_geom_type::GeomType> {
        match self {
            LayerData::Vector { geom_type, .. } => Some(*geom_type),
            LayerData::Raster { .. } => None,
        }
    }

    pub fn unprojected_feature_collection(
        &self,
    ) -> Option<&Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>> {
        match self {
            LayerData::Vector {
                unprojected_feature_collection,
                ..
            } => Some(unprojected_feature_collection),
            LayerData::Raster { .. } => None,
        }
    }

    pub fn projected_feature_collection(
        &self,
    ) -> Option<&geo_features::FeatureCollection<geo_projected::ProjectedScalar>> {
        match self {
            LayerData::Vector {
                projected_feature_collection,
                ..
            } => projected_feature_collection.as_ref(),
            LayerData::Raster { .. } => None,
        }
    }

    pub fn is_active(&self) -> bool {
        match self {
            LayerData::Vector {
                projected_feature_collection,
                ..
            } => projected_feature_collection.is_some(),
            LayerData::Raster { projected_grid, .. } => projected_grid.is_some(),
        }
    }

    pub fn raster(&self) -> Option<&geo_raster::Raster> {
        match self {
            LayerData::Raster { raster, .. } => Some(raster),
            _ => None,
        }
    }

    #[inline]
    pub fn get_projected_feature_collection_or_log(
        &self,
        layer_id: rgis_primitives::LayerId,
    ) -> Option<&geo_features::FeatureCollection<geo_projected::ProjectedScalar>> {
        match self {
            LayerData::Vector {
                projected_feature_collection,
                ..
            } => match projected_feature_collection.as_ref() {
                Some(p) => Some(p),
                None => {
                    bevy::log::error!(
                        "Expected layer (id: {:?}) to have a projected feature collection",
                        layer_id
                    );
                    None
                }
            },
            LayerData::Raster { .. } => None,
        }
    }

    #[inline]
    pub fn get_projected_feature(
        &self,
        layer_id: rgis_primitives::LayerId,
        feature_id: geo_features::FeatureId,
    ) -> Option<&geo_features::Feature<geo_projected::ProjectedScalar>> {
        let feature_collection = self.get_projected_feature_collection_or_log(layer_id)?;
        feature_collection
            .features
            .iter()
            .find(|f| f.id == feature_id)
    }

    pub fn clear_projected(&mut self) {
        match self {
            LayerData::Vector {
                projected_feature_collection,
                ..
            } => {
                *projected_feature_collection = None;
            }
            LayerData::Raster { projected_grid, .. } => {
                *projected_grid = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions for feature-from-click (previously on Layers)
// ---------------------------------------------------------------------------

/// Given a projected coordinate, find the first feature that contains it.
/// The caller should pass an iterator of (LayerId, &LayerData) in
/// top-to-bottom order.
pub struct FeatureFromClickResult<'a> {
    pub layer_id: rgis_primitives::LayerId,
    pub feature: &'a geo_features::Feature<geo_projected::UnprojectedScalar>,
    pub properties: Option<Vec<(String, String)>>,
}

pub fn feature_from_click<'a>(
    coord: geo_projected::ProjectedCoord,
    layers: impl Iterator<Item = (rgis_primitives::LayerId, &'a LayerData)>,
) -> Option<FeatureFromClickResult<'a>> {
    for (layer_id, data) in layers {
        if let LayerData::Vector {
            unprojected_feature_collection,
            projected_feature_collection: Some(projected),
            ..
        } = data
        {
            for (feature_index, (unprojected, proj_feature)) in unprojected_feature_collection
                .features
                .iter()
                .zip(projected.features.iter())
                .enumerate()
            {
                if proj_feature.contains(&coord) {
                    let properties = unprojected_feature_collection
                        .properties
                        .as_ref()
                        .map(|rb| geo_features::properties_for_row(rb, feature_index));
                    return Some(FeatureFromClickResult {
                        layer_id,
                        feature: unprojected,
                        properties,
                    });
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Color helpers (unchanged)
// ---------------------------------------------------------------------------

fn colorous_color_to_bevy_color(colorous_color: colorous::Color) -> Color {
    Color::srgb_u8(colorous_color.r, colorous_color.g, colorous_color.b)
}

const COLORS: [colorous::Color; 10] = colorous::CATEGORY10;

pub fn next_colorous_color() -> colorous::Color {
    #[allow(clippy::indexing_slicing)]
    COLORS[next_color_index()]
}

fn next_color_index() -> usize {
    use std::sync;
    static COUNTER: sync::atomic::AtomicUsize = sync::atomic::AtomicUsize::new(0);
    COUNTER.fetch_add(1, sync::atomic::Ordering::Relaxed) % COLORS.len()
}

pub fn make_layer_color(geom_type: geo_geom_type::GeomType) -> LayerColor {
    if geom_type.has_fill() {
        LayerColor {
            fill: Some(colorous_color_to_bevy_color(next_colorous_color())),
            stroke: Color::BLACK,
        }
    } else {
        LayerColor {
            fill: None,
            stroke: colorous_color_to_bevy_color(next_colorous_color()),
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LayerOrder::default());
        app.insert_resource(LayerIdToEntity::default());
        app.insert_resource(SelectedFeature::default());
        systems::configure(app);
    }
}
