use bevy::prelude::*;
use geo::contains::Contains;
use std::collections::BTreeMap;
use std::sync;
use std::sync::Arc;

mod systems;

#[derive(Clone, Debug, Default)]
pub struct ProjectedFeatureCollectionWithLOD {
    pub projected: BTreeMap<u8, geo_features::FeatureCollection<geo_projected::ProjectedScalar>>,
}

#[derive(Copy, Clone, Debug)]
pub struct LayerIndex(pub usize);

#[derive(Debug, Copy, Clone)]
pub struct LayerWithIndex<'a>(pub &'a Layer, pub LayerIndex);

#[derive(Debug)]
pub struct ProjectedRasterGrid {
    pub cols: u32,
    pub rows: u32,
    pub positions: Vec<[f32; 2]>,
    pub valid: Vec<bool>,
    pub extent: geo::Rect<f64>,
}

#[derive(Debug, Resource)]
pub struct Layers {
    // Ordered from bottom to top
    data: Vec<Layer>,
    // ID of the currently selected Layer
    pub selected_layer_id: Option<rgis_primitives::LayerId>,
    // Counter for cycling through layer colors
    color_counter: usize,
    // Counter for generating unique LayerIds
    next_layer_id_value: u16,
}

impl Default for Layers {
    fn default() -> Self {
        Self::new()
    }
}

impl Layers {
    pub fn new() -> Layers {
        Layers {
            data: vec![],
            selected_layer_id: None,
            color_counter: 0,
            // Starting at 1 so we can use NonZeroU16
            next_layer_id_value: 1,
        }
    }

    #[inline]
    pub fn iter_bottom_to_top(&self) -> impl Iterator<Item = &Layer> {
        self.data.iter()
    }

    #[inline]
    pub fn iter_top_to_bottom(&self) -> impl Iterator<Item = &Layer> {
        self.data.iter().rev()
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.data.len()
    }

    pub fn containing_coord(
        &self,
        coord: geo_projected::ProjectedCoord,
    ) -> impl Iterator<Item = &Layer> {
        self.iter_top_to_bottom().filter(move |layer| {
            match &layer.data {
                LayerData::Vector { .. } => {
                    layer
                        .get_current_lod_feature_collection()
                        .map_or(false, |p| p.contains(&coord))
                }
                _ => false,
            }
        })
    }

    fn feature_collections_iter(&'_ self) -> impl Iterator<Item = FeatureCollectionsIterItem<'_>> {
        self.iter_top_to_bottom().filter_map(|layer| {
            match &layer.data {
                LayerData::Vector {
                    unprojected_feature_collection,
                    ..
                } => {
                    layer
                        .get_current_lod_feature_collection()
                        .map(|projected| FeatureCollectionsIterItem {
                            layer,
                            unprojected: unprojected_feature_collection,
                            projected,
                        })
                }
                _ => None,
            }
        })
    }

    fn features_iter(&'_ self) -> impl Iterator<Item = FeaturesIterItem<'_>> {
        self.feature_collections_iter().flat_map(
            |FeatureCollectionsIterItem {
                 layer,
                 projected,
                 unprojected,
             }| {
                unprojected
                    .features
                    .iter()
                    .zip(projected.features.iter())
                    .map(move |(unprojected, projected)| FeaturesIterItem {
                        layer,
                        projected,
                        unprojected,
                    })
            },
        )
    }

    pub fn feature_from_click(
        &self,
        coord: geo_projected::ProjectedCoord,
    ) -> Option<(
        &Layer,
        &geo_features::Feature<geo_projected::UnprojectedScalar>,
    )> {
        self.features_iter()
            .find(|item| item.projected.contains(&coord))
            .map(|item| (item.layer, item.unprojected))
    }

    fn get_index(&self, layer_id: rgis_primitives::LayerId) -> Option<usize> {
        self.data.iter().position(|entry| entry.id == layer_id)
    }

    #[inline]
    pub fn get(&self, layer_id: rgis_primitives::LayerId) -> Option<&Layer> {
        let index = self.get_index(layer_id)?;
        self.data.get(index)
    }

    #[inline]
    pub fn get_with_index(
        &'_ self,
        layer_id: rgis_primitives::LayerId,
    ) -> Option<LayerWithIndex<'_>> {
        let index = self.get_index(layer_id)?;
        self.data
            .get(index)
            .map(|layer| LayerWithIndex(layer, LayerIndex(index)))
    }

    #[inline]
    pub fn get_mut(&mut self, layer_id: rgis_primitives::LayerId) -> Option<&mut Layer> {
        let index = self.get_index(layer_id)?;
        self.data.get_mut(index)
    }

    #[inline]
    pub fn remove(&mut self, layer_id: rgis_primitives::LayerId) {
        if let Some(index) = self.get_index(layer_id) {
            self.data.remove(index);
        }
    }

    #[allow(unused)]
    pub fn selected_layer(&self) -> Option<&Layer> {
        self.selected_layer_id
            .and_then(|layer_id| self.get(layer_id))
    }

    fn next_layer_id(&mut self) -> rgis_primitives::LayerId {
        let value = self.next_layer_id_value;
        self.next_layer_id_value += 1;
        rgis_primitives::LayerId::from_u16(value)
    }

    fn add(
        &mut self,
        unprojected: Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
        name: String,
        crs: rgis_primitives::Crs,
    ) -> rgis_primitives::LayerId {
        let layer_id = self.next_layer_id();
        let geom_type = geo_geom_type::determine(unprojected.geometry_iter());
        let color = next_colorous_color(&mut self.color_counter);
        let layer = Layer {
            data: LayerData::Vector {
                unprojected_feature_collection: unprojected,
                projected_feature_collection: None,
                geom_type,
            },
            current_lod: None,
            color: if geom_type.has_fill() {
                LayerColor {
                    fill: Some(colorous_color_to_bevy_color(color)),
                    stroke: Color::BLACK,
                }
            } else {
                LayerColor {
                    fill: None,
                    stroke: colorous_color_to_bevy_color(color),
                }
            },
            name,
            visible: true,
            id: layer_id,
            crs,
            point_size: 5.0,
        };
        self.data.push(layer);
        layer_id
    }

    pub fn add_raster(
        &mut self,
        raster: geo_raster::Raster,
        name: String,
        crs: rgis_primitives::Crs,
    ) -> rgis_primitives::LayerId {
        let layer_id = self.next_layer_id();
        let layer = Layer {
            data: LayerData::Raster { raster, projected_grid: None },
            current_lod: None,
            color: LayerColor {
                fill: None,
                stroke: Color::WHITE,
            },
            name,
            visible: true,
            id: layer_id,
            crs,
            point_size: 5.0,
        };
        self.data.push(layer);
        layer_id
    }

    pub fn clear_projected(&mut self) {
        for layer in self.data.iter_mut() {
            match &mut layer.data {
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
            layer.current_lod = None;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.data.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Layer> {
        self.data.iter_mut()
    }
}

#[derive(Clone, Debug)]
pub struct LayerColor {
    pub fill: Option<Color>,
    pub stroke: Color,
}

#[derive(Debug)]
pub enum LayerData {
    Vector {
        unprojected_feature_collection:
            Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
        projected_feature_collection: Option<ProjectedFeatureCollectionWithLOD>,
        geom_type: geo_geom_type::GeomType,
    },
    Raster {
        raster: geo_raster::Raster,
        projected_grid: Option<ProjectedRasterGrid>,
    },
}

#[derive(Debug)]
pub struct Layer {
    pub data: LayerData,
    pub current_lod: Option<u8>,
    pub color: LayerColor,
    pub id: rgis_primitives::LayerId,
    pub name: String,
    pub visible: bool,
    pub crs: rgis_primitives::Crs,
    pub point_size: f32,
}

impl Layer {
    pub fn get_current_lod_feature_collection(
        &self,
    ) -> Option<&geo_features::FeatureCollection<geo_projected::ProjectedScalar>> {
        let lod = self.current_lod?;
        match &self.data {
            LayerData::Vector {
                projected_feature_collection,
                ..
            } => {
                projected_feature_collection
                    .as_ref()?
                    .projected
                    .get(&lod)
            }
            _ => None,
        }
    }

    pub fn is_vector(&self) -> bool {
        matches!(self.data, LayerData::Vector { .. })
    }

    pub fn is_raster(&self) -> bool {
        matches!(&self.data, LayerData::Raster { .. })
    }

    pub fn geom_type(&self) -> Option<geo_geom_type::GeomType> {
        match &self.data {
            LayerData::Vector { geom_type, .. } => Some(*geom_type),
            LayerData::Raster { .. } => None,
        }
    }

    pub fn unprojected_feature_collection(
        &self,
    ) -> Option<&Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>> {
        match &self.data {
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
        self.get_current_lod_feature_collection()
    }

    pub fn is_active(&self) -> bool {
        match &self.data {
            LayerData::Vector { .. } => self.get_current_lod_feature_collection().is_some(),
            LayerData::Raster { projected_grid, .. } => projected_grid.is_some(),
        }
    }

    pub fn raster(&self) -> Option<&geo_raster::Raster> {
        match &self.data {
            LayerData::Raster { raster, .. } => Some(raster),
            _ => None,
        }
    }

    #[inline]
    pub fn get_projected_feature_collection_or_log(
        &self,
    ) -> Option<&geo_features::FeatureCollection<geo_projected::ProjectedScalar>> {
        match self.get_current_lod_feature_collection() {
            Some(p) => Some(p),
            None => {
                error!(
                    "Expected layer (id: {:?}) to have a projected feature collection for LOD {:?}",
                    self.id, self.current_lod
                );
                None
            }
        }
    }

    #[inline]
    pub fn get_projected_feature(
        &self,
        feature_id: geo_features::FeatureId,
    ) -> Option<&geo_features::Feature<geo_projected::ProjectedScalar>> {
        let feature_collection = self.get_projected_feature_collection_or_log()?;
        feature_collection
            .features
            .iter()
            .find(|f| f.id == feature_id)
    }
}

fn colorous_color_to_bevy_color(colorous_color: colorous::Color) -> Color {
    Color::srgb_u8(colorous_color.r, colorous_color.g, colorous_color.b)
}

const COLORS: [colorous::Color; 10] = colorous::CATEGORY10;

fn next_colorous_color(counter: &mut usize) -> colorous::Color {
    #[allow(clippy::indexing_slicing)]
    let color = COLORS[*counter % COLORS.len()];
    *counter += 1;
    color
}

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Layers::new());
        systems::configure(app);
    }
}

struct FeatureCollectionsIterItem<'a> {
    layer: &'a Layer,
    projected: &'a geo_features::FeatureCollection<geo_projected::ProjectedScalar>,
    unprojected: &'a geo_features::FeatureCollection<geo_projected::UnprojectedScalar>,
}

struct FeaturesIterItem<'a> {
    layer: &'a Layer,
    projected: &'a geo_features::Feature<geo_projected::ProjectedScalar>,
    unprojected: &'a geo_features::Feature<geo_projected::UnprojectedScalar>,
}
