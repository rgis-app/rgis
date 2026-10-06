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

/// Titles of the egui windows drawn during the most recent UI pass.
///
/// Cleared at the start of every pass and filled in by each window's render
/// system, so readers outside the pass always see a complete frame.
#[derive(Resource, Default, Debug)]
pub struct OpenWindows(std::collections::BTreeSet<String>);

impl OpenWindows {
    pub fn record(&mut self, title: impl Into<String>) {
        self.0.insert(title.into());
    }

    pub fn contains(&self, title: &str) -> bool {
        self.0.contains(title)
    }

    /// Window titles in alphabetical order.
    pub fn titles(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

/// Windows that [`set_window_open`] can open.
pub const OPENABLE_WINDOWS: &[&str] = &["Add Layer", "Change CRS", "Logs", "Welcome"];

/// Open or close an egui window by title, as if the user had done it.
///
/// Closing works for any window listed in [`OpenWindows`] except "Distances",
/// which follows the measure tool. Opening is limited to [`OPENABLE_WINDOWS`];
/// the rest only make sense in response to another action (e.g. selecting a
/// feature opens "Layer Feature Properties").
pub fn set_window_open(world: &mut World, title: &str, open: bool) -> Result<(), String> {
    if open {
        match title {
            "Add Layer" => {
                world.write_message(rgis_ui_messages::ShowAddLayerWindowMessage);
            }
            "Change CRS" => world.insert_resource(ChangeCrsWindowVisible(true)),
            "Logs" => set_window_state::<windows::logs::Logs<'static>>(world, true)?,
            "Welcome" => set_window_state::<windows::welcome::Welcome<'static>>(world, true)?,
            _ => {
                return Err(format!(
                    "window \"{title}\" can't be opened directly; openable windows: {}",
                    OPENABLE_WINDOWS.join(", ")
                ))
            }
        }
        return Ok(());
    }

    let Some(open_windows) = world.get_resource::<OpenWindows>() else {
        return Err("no UI is running, so there are no windows to close".into());
    };
    if !open_windows.contains(title) {
        let titles: Vec<&str> = open_windows.titles().collect();
        return Err(format!(
            "window \"{title}\" is not open; open windows: [{}]",
            titles.join(", ")
        ));
    }
    match title {
        "Distances" => Err("the \"Distances\" window follows the measure tool and can't be closed directly".into()),
        "Logs" => set_window_state::<windows::logs::Logs<'static>>(world, false),
        _ => {
            widget_registry::request_close(title);
            Ok(())
        }
    }
}

fn set_window_state<W: bevy_egui_window::Window + 'static>(
    world: &mut World,
    open: bool,
) -> Result<(), String> {
    let Some(mut next_state) =
        world.get_resource_mut::<NextState<bevy_egui_window::WindowVisibility<W>>>()
    else {
        return Err("window state is not initialized".into());
    };
    next_state.set(if open {
        bevy_egui_window::WindowVisibility::Open
    } else {
        bevy_egui_window::WindowVisibility::Closed
    });
    Ok(())
}

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
            .init_resource::<OpenWindows>()
            .insert_resource(ClearColor::default());

        systems::configure(app);
    }
}
