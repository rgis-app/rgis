use bevy::prelude::*;

pub mod log_buffer;
mod panels;
pub mod save_file;
mod systems;
pub mod widget_registry;
mod widgets;
mod windows;

pub struct Plugin;

/// Window state for displaying a message. `Some(message)` means visible.
type MessageWindowState = Option<String>;

/// Window state for managing a layer. `Some(layer_id)` means visible.
type ManageLayerWindowState = Option<rgis_primitives::LayerId>;

/// Data displayed in the feature properties window.
pub struct FeaturePropertiesWindowData {
    layer_id: rgis_primitives::LayerId,
    properties: Vec<(String, String)>,
}

/// Window state for feature properties. `Some(data)` means visible.
type FeaturePropertiesWindowState = Option<FeaturePropertiesWindowData>;

/// Window state for attribute table. `Some(layer_id)` means visible.
type AttributeTableWindowState = Option<rgis_primitives::LayerId>;

/// Whether the Change CRS window is visible.
#[derive(Resource, Default)]
pub struct ChangeCrsWindowVisible(pub bool);

/// Data displayed in the operation window.
struct OperationWindowData {
    operation: Box<dyn Send + Sync + rgis_geo_ops::Operation>,
    feature_collection: std::sync::Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
    source_crs: Option<rgis_primitives::Crs>,
    layer_name: String,
}

/// Window state for operations. `Some(data)` means visible.
type OperationWindowState = Option<OperationWindowData>;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_egui::EguiPlugin::default())
            .insert_resource(windows::add_layer::file::SelectedFile(None))
            .insert_resource(rgis_units::TopPanelHeight(0.))
            .insert_resource(rgis_units::BottomPanelHeight(0.))
            .insert_resource(rgis_units::SidePanelWidth(0.))
            .insert_resource(ChangeCrsWindowVisible::default())
            .insert_resource(ClearColor::default());

        systems::configure(app);
    }
}
