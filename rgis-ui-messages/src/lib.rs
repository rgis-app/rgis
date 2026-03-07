use std::sync::Arc;
use bevy::prelude::*;

#[derive(Debug, Message)]
pub struct ShowManageLayerWindowMessage(pub rgis_primitives::LayerId);

#[derive(Default, Message)]
pub struct OpenChangeCrsWindowMessage;

#[derive(Default, Message)]
pub struct ShowAddLayerWindowMessage;

#[derive(Default, Message)]
pub struct HideAddLayerWindowMessage;

#[derive(Message)]
pub struct RenderTextMessage(pub String);

#[derive(Message)]
pub struct RenderFeaturePropertiesMessage {
    pub layer_id: rgis_primitives::LayerId,
    pub properties: geo_features::Properties,
}

/// Change the `Layer`'s color
#[derive(Message)]
pub enum UpdateLayerColorMessage {
    Fill(rgis_primitives::LayerId, bevy::prelude::Color),
    Stroke(rgis_primitives::LayerId, bevy::prelude::Color),
}

#[derive(Message, Debug)]
pub struct UpdateLayerPointSizeMessage(pub rgis_primitives::LayerId, pub f32);

#[derive(Message, Debug)]
pub struct UpdateLayerStrokeWidthMessage(pub rgis_primitives::LayerId, pub f32);

#[derive(Message)]
pub struct OpenOperationWindowMessage {
    pub operation: Box<dyn Send + Sync + rgis_geo_ops::Operation>,
    pub feature_collection: Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
}

#[derive(Message)]
pub struct PerformOperationMessage {
    pub operation: Box<dyn Send + Sync + rgis_geo_ops::Operation>,
    pub layer_id: rgis_primitives::LayerId,
}

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShowManageLayerWindowMessage>()
            .add_message::<OpenChangeCrsWindowMessage>()
            .add_message::<ShowAddLayerWindowMessage>()
            .add_message::<HideAddLayerWindowMessage>()
            .add_message::<RenderTextMessage>()
            .add_message::<RenderFeaturePropertiesMessage>()
            .add_message::<UpdateLayerColorMessage>()
            .add_message::<UpdateLayerPointSizeMessage>()
            .add_message::<UpdateLayerStrokeWidthMessage>()
            .add_message::<OpenOperationWindowMessage>()
            .add_message::<PerformOperationMessage>();
    }
}
