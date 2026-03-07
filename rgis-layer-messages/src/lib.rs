use std::sync::Arc;
use bevy::prelude::*;
use geo_projected::UnprojectedScalar;

#[derive(Debug, Message)]
pub struct LayerCreatedMessage(pub rgis_primitives::LayerId);

#[derive(Message, Debug)]
pub struct ToggleLayerVisibilityMessage(pub rgis_primitives::LayerId);

#[derive(Event, Debug)]
pub struct LayerBecameHiddenEvent(pub rgis_primitives::LayerId);

#[derive(Event, Debug)]
pub struct LayerBecameVisibleEvent(pub rgis_primitives::LayerId);

/// After a `Layer`'s color is changed
#[derive(Clone, Copy, Message)]
pub enum LayerColorUpdatedMessage {
    Fill(rgis_primitives::LayerId),
    Stroke(rgis_primitives::LayerId),
}

#[derive(Message, Debug)]
pub struct LayerPointSizeUpdatedMessage(pub rgis_primitives::LayerId);

#[derive(Message)]
pub struct DeleteLayerMessage(pub rgis_primitives::LayerId);

pub enum MoveDirection {
    Up,
    Down,
}

#[derive(Message)]
pub struct MoveLayerMessage(pub rgis_primitives::LayerId, pub MoveDirection);

#[derive(Message)]
pub struct LayerZIndexUpdatedMessage(pub rgis_primitives::LayerId);

#[derive(Message)]
pub struct CreateLayerMessage {
    pub feature_collection: Arc<geo_features::FeatureCollection<UnprojectedScalar>>,
    pub name: String,
    pub source_crs: rgis_primitives::Crs,
}

#[derive(Message)]
pub struct LayerReprojectedMessage(pub rgis_primitives::LayerId);

#[derive(Debug, Message)]
pub struct LODChangedMessage(pub rgis_primitives::LayerId);

#[derive(Message, Debug)]
pub struct DuplicateLayerMessage(pub rgis_primitives::LayerId);

#[derive(Message)]
pub struct CreateRasterLayerMessage {
    pub raster: geo_raster::Raster,
    pub name: String,
    pub source_crs: rgis_primitives::Crs,
}

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LayerCreatedMessage>()
            .add_message::<ToggleLayerVisibilityMessage>()
            .add_message::<LayerColorUpdatedMessage>()
            .add_message::<LayerPointSizeUpdatedMessage>()
            .add_message::<DeleteLayerMessage>()
            .add_message::<MoveLayerMessage>()
            .add_message::<LayerZIndexUpdatedMessage>()
            .add_message::<LayerReprojectedMessage>()
            .add_message::<LODChangedMessage>()
            .add_message::<CreateLayerMessage>()
            .add_message::<DuplicateLayerMessage>()
            .add_message::<CreateRasterLayerMessage>();
    }
}
