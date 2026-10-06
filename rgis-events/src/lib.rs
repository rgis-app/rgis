use std::sync::Arc;
use bevy::prelude::*;
use geo_projected::{ProjectedCoord, UnprojectedScalar};

// ── Camera ──────────────────────────────────────────────────────────────────

// Magic number used to normalize the host's scroll value.
const ZOOM_FACTOR: f32 = 500.;

#[derive(Message, Debug)]
pub struct PanCameraMessage {
    // X offset for camera position. Positive is right, negative is left.
    pub x: f32,
    // Y offset for camera position. Positive is up, negative is down.
    pub y: f32,
}

impl PanCameraMessage {
    #[inline]
    pub fn up(amount: f32) -> Self {
        PanCameraMessage { x: 0., y: amount }
    }

    #[inline]
    pub fn right(amount: f32) -> Self {
        PanCameraMessage { x: amount, y: 0. }
    }

    #[inline]
    pub fn down(amount: f32) -> Self {
        PanCameraMessage { x: 0., y: -amount }
    }

    #[inline]
    pub fn left(amount: f32) -> Self {
        PanCameraMessage { x: -amount, y: 0. }
    }
}

#[derive(Message, Debug)]
pub struct ZoomCameraMessage {
    /// * `amount ∈ (1, ∞)` → zoom in
    /// * `amount ∈ [1]` → no change
    /// * `amount ∈ (0, 1)` → zoom out
    pub amount: f32,
    /// The point to zoom towards. When `None`, zooms around the camera center.
    pub coord: Option<ProjectedCoord>,
}

impl ZoomCameraMessage {
    #[inline]
    pub fn new(amount: f32, coord: ProjectedCoord) -> Self {
        ZoomCameraMessage {
            // Don't let amount be negative, so add `max`
            amount: (1. + amount / ZOOM_FACTOR).max(0.),
            coord: Some(coord),
        }
    }

    #[inline]
    pub fn new_center(amount: f32) -> Self {
        ZoomCameraMessage {
            amount: (1. + amount / ZOOM_FACTOR).max(0.),
            coord: None,
        }
    }
}

#[derive(Message, Debug)]
pub struct CenterCameraMessage(pub rgis_primitives::LayerId);

impl From<rgis_primitives::LayerId> for CenterCameraMessage {
    #[inline]
    fn from(layer_id: rgis_primitives::LayerId) -> Self {
        CenterCameraMessage(layer_id)
    }
}

#[derive(Message, Debug)]
pub struct CenterCameraOnFeatureMessage(pub rgis_primitives::LayerId, pub geo_features::FeatureId);

#[derive(Default, Message, Debug)]
pub struct RecalculateMousePositionMessage;

// ── Layer ───────────────────────────────────────────────────────────────────

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

#[derive(Message, Debug)]
pub struct DuplicateLayerMessage(pub rgis_primitives::LayerId);

#[derive(Message, Debug)]
pub struct DownloadLayerMessage {
    pub layer_id: rgis_primitives::LayerId,
    pub format: rgis_primitives::ExportFormat,
}

#[derive(Message)]
pub struct CreateRasterLayerMessage {
    pub raster: geo_raster::Raster,
    pub name: String,
    pub source_crs: rgis_primitives::Crs,
}

// ── CRS ─────────────────────────────────────────────────────────────────────

#[derive(Message)]
pub struct ChangeCrsMessage {
    pub old: rgis_primitives::Crs,
    pub new: rgis_primitives::Crs,
}

#[derive(Event)]
pub struct CrsChangedEvent {
    pub old: rgis_primitives::Crs,
    pub new: rgis_primitives::Crs,
}

// ── Renderer ────────────────────────────────────────────────────────────────

#[derive(Event)]
pub struct DespawnMeshesEvent(pub rgis_primitives::LayerId);

#[derive(Message)]
pub struct MeshesSpawnedMessage(pub rgis_primitives::LayerId);

impl From<rgis_primitives::LayerId> for MeshesSpawnedMessage {
    #[inline]
    fn from(layer_id: rgis_primitives::LayerId) -> Self {
        MeshesSpawnedMessage(layer_id)
    }
}

// ── Map ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Message, Debug)]
pub struct FeatureSelectedMessage(pub rgis_primitives::LayerId, pub geo_features::FeatureId);

#[derive(Default, Message, Debug)]
pub struct FeaturesDeselectedMessage;

// ── File Loader ─────────────────────────────────────────────────────────────

#[derive(Message, Debug)]
pub enum LoadFileMessage {
    FromNetwork {
        name: String,
        url: String,
        file_format: geo_file_loader::FileFormat,
        source_crs: rgis_primitives::Crs,
    },
    FromBytes {
        file_name: String,
        file_format: geo_file_loader::FileFormat,
        bytes: bytes::Bytes,
        source_crs: rgis_primitives::Crs,
    },
}

// ── Plugin ──────────────────────────────────────────────────────────────────

pub struct RgisEventsPlugin;

impl bevy::app::Plugin for RgisEventsPlugin {
    fn build(&self, app: &mut App) {
        // Camera
        app.add_message::<PanCameraMessage>()
            .add_message::<ZoomCameraMessage>()
            .add_message::<CenterCameraMessage>()
            .add_message::<CenterCameraOnFeatureMessage>()
            .add_message::<RecalculateMousePositionMessage>();

        // Layer
        app.add_message::<LayerCreatedMessage>()
            .add_message::<ToggleLayerVisibilityMessage>()
            .add_message::<LayerColorUpdatedMessage>()
            .add_message::<LayerPointSizeUpdatedMessage>()
            .add_message::<DeleteLayerMessage>()
            .add_message::<MoveLayerMessage>()
            .add_message::<LayerZIndexUpdatedMessage>()
            .add_message::<LayerReprojectedMessage>()
            .add_message::<CreateLayerMessage>()
            .add_message::<DuplicateLayerMessage>()
            .add_message::<CreateRasterLayerMessage>()
            .add_message::<DownloadLayerMessage>();

        // CRS
        app.add_message::<ChangeCrsMessage>();

        // Renderer
        app.add_message::<MeshesSpawnedMessage>();

        // Map
        app.add_message::<FeatureSelectedMessage>()
            .add_message::<FeaturesDeselectedMessage>();

        // File Loader
        app.add_message::<LoadFileMessage>();
    }
}
