// This module is responsible for generating Levels of Detail (LODs) for geographic features.
// It defines a series of "passes" that are applied to a feature collection to simplify,
// cull, and validate geometries for different zoom levels.

use bevy::log::{debug, error};
use bevy_jobs::{AsyncReturn, Context, Job};
use geo::{algorithm::simplify::Simplify, CoordsIter, MapCoords};
use geo_features::FeatureCollection;
use geo_projected::ProjectedScalar;
use rgis_layers::ProjectedFeatureCollectionWithLOD;
use std::collections::BTreeMap;

/// A background job that takes a projected feature collection and generates
/// multiple levels of detail (LODs) for it.
pub struct LODBuilderJob {
    pub projected: FeatureCollection<ProjectedScalar>,
    pub layer_id: rgis_layer_id::LayerId,
}

impl Job for LODBuilderJob {
    type Outcome = Result<LODBuilderJobOutcome, ()>;
    const JOB_TYPE: bevy_jobs::JobType = bevy_jobs::JobType::Compute;

    fn name(&self) -> String {
        "Building LODs".to_string()
    }

    fn perform(self, _: Context) -> AsyncReturn<Self::Outcome> {
        Box::pin(async move {
            Ok(LODBuilderJobOutcome {
                lods: build_lods(self.projected),
                layer_id: self.layer_id,
            })
        })
    }
}

/// The output of the `LODBuilderJob`, containing the generated LODs.
pub struct LODBuilderJobOutcome {
    pub lods: ProjectedFeatureCollectionWithLOD,
    pub layer_id: rgis_layer_id::LayerId,
}

fn calculate_epsilon(lod_level: u8) -> f64 {
    // TODO: make this configurable
    10.0_f64.powf(f64::from(lod_level) - 5.0)
}

fn calculate_cull_threshold(lod_level: u8) -> f64 {
    // TODO: make this configurable
    10.0_f64.powf(10.0 - f64::from(lod_level))
}

/// A trait for a single step in the LOD generation pipeline.
trait Pass {
    fn run(
        &self,
        feature: geo_features::Feature<ProjectedScalar>,
    ) -> Option<geo_features::Feature<ProjectedScalar>>;
}

/// A pass that removes features that are too small to be seen at a given zoom level.
struct CullPass {
    threshold: f64,
}

impl Pass for CullPass {
    fn run(
        &self,
        feature: geo_features::Feature<ProjectedScalar>,
    ) -> Option<geo_features::Feature<ProjectedScalar>> {
        if let Some(bounding_rect) = feature.bounding_rect {
            let threshold = geo_projected::ProjectedScalar::from(self.threshold);
            if bounding_rect.width() < threshold || bounding_rect.height() < threshold {
                return None;
            }
        }
        Some(feature)
    }
}

/// A pass that simplifies the geometry of a feature.
struct SimplifyPass {
    epsilon: f64,
}

impl Pass for SimplifyPass {
    fn run(
        &self,
        mut feature: geo_features::Feature<ProjectedScalar>,
    ) -> Option<geo_features::Feature<ProjectedScalar>> {
        if let Some(geometry) = &feature.geometry {
            // Convert to f64 geometry for simplification
            let geom_f64: geo::Geometry<f64> = geometry.map_coords(|c| (c.x.0, c.y.0).into());
            // Simplify
            let simplified_f64 = match geom_f64 {
                geo::Geometry::Polygon(p) => geo::Geometry::Polygon(p.simplify(&self.epsilon)),
                geo::Geometry::MultiPolygon(mp) => {
                    geo::Geometry::MultiPolygon(mp.simplify(&self.epsilon))
                }
                geo::Geometry::LineString(ls) => {
                    geo::Geometry::LineString(ls.simplify(&self.epsilon))
                }
                geo::Geometry::MultiLineString(mls) => {
                    geo::Geometry::MultiLineString(mls.simplify(&self.epsilon))
                }
                _ => geom_f64.clone(),
            };
            // Convert back to ProjectedScalar geometry
            let simplified_projected: geo::Geometry<ProjectedScalar> =
                simplified_f64.map_coords(|c| {
                    geo::Coord::from((
                        geo_projected::ProjectedScalar::from(c.x),
                        geo_projected::ProjectedScalar::from(c.y),
                    ))
                });
            feature.geometry = Some(simplified_projected);
        }
        Some(feature)
    }
}

/// A pass that validates the geometry of a feature, removing it if it's invalid.
struct ValidationPass;

impl Pass for ValidationPass {
    fn run(
        &self,
        feature: geo_features::Feature<ProjectedScalar>,
    ) -> Option<geo_features::Feature<ProjectedScalar>> {
        if let Some(geometry) = &feature.geometry {
            if is_geometry_valid(geometry) {
                Some(feature)
            } else {
                error!("Invalid geometry detected and removed: {:?}", geometry);
                None
            }
        } else {
            Some(feature)
        }
    }
}

fn is_geometry_valid(geometry: &geo::Geometry<ProjectedScalar>) -> bool {
    match geometry {
        geo::Geometry::Polygon(p) => p.exterior().0.len() >= 4 && p.exterior().is_closed(),
        geo::Geometry::MultiPolygon(mp) => {
            mp.0.iter()
                .all(|p| p.exterior().0.len() >= 4 && p.exterior().is_closed())
        }
        geo::Geometry::LineString(ls) => {
            if ls.0.len() < 2 {
                return false;
            }
            // Ensure there are at least two distinct points.
            let first = &ls.0[0];
            ls.0.iter().skip(1).any(|p| p != first)
        }
        geo::Geometry::MultiLineString(mls) => mls.0.iter().all(|ls| {
            if ls.0.len() < 2 {
                return false;
            }
            let first = &ls.0[0];
            ls.0.iter().skip(1).any(|p| p != first)
        }),
        _ => true,
    }
}

pub fn build_lods(
    projected: FeatureCollection<ProjectedScalar>,
) -> ProjectedFeatureCollectionWithLOD {
    let mut lods = BTreeMap::new();

    lods.insert(0, projected.clone());

    let original_coords_count = projected.coords_count();

    for lod_level in 1..=10 {
        let epsilon = calculate_epsilon(lod_level);
        let cull_threshold = calculate_cull_threshold(lod_level);

        let passes: Vec<Box<dyn Pass + Send + Sync>> = vec![
            Box::new(CullPass {
                threshold: cull_threshold,
            }),
            Box::new(SimplifyPass { epsilon }),
            Box::new(ValidationPass),
        ];

        let simplified_and_culled_features: Vec<_> = projected
            .features
            .iter()
            .filter_map(|feature| {
                let mut current_feature = Some(feature.clone());
                for pass in &passes {
                    current_feature = match current_feature {
                        Some(f) => pass.run(f),
                        None => break,
                    };
                }
                current_feature
            })
            .collect();

        let feature_collection = FeatureCollection {
            features: simplified_and_culled_features,
            ..projected.clone()
        };

        let simplified_coords_count = feature_collection.coords_count();
        let omitted_count = original_coords_count - simplified_coords_count;
        let percentage_omitted = (omitted_count as f64 / original_coords_count as f64) * 100.0;

        debug!(
            "LOD {}: omitted {} points ({:.2}%)",
            lod_level, omitted_count, percentage_omitted
        );

        lods.insert(lod_level, feature_collection);
    }

    ProjectedFeatureCollectionWithLOD { projected: lods }
}
