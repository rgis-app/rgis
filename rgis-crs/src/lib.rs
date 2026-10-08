use std::sync::Arc;

use bevy::prelude::*;
use rgis_primitives::Crs;

mod transform;
pub use transform::{lon_lat_to_crs, CrsTransformer, TransformError};

// --- Geodesy context (formerly rgis-geodesy) ---

/// Cached WGS 84 (EPSG:4326) operation handle, created once at startup.
#[derive(Resource)]
pub struct Wgs84OpHandle(pub geodesy::ctx::OpHandle);

/// Shared geodesy context for coordinate reference system operations.
///
/// This resource wraps `geodesy::ctx::Minimal` in an `Arc<RwLock>` because
/// it must be cloned into background jobs (which run on `AsyncComputeTaskPool`
/// threads). Use [`Self::clone_for_async`] to obtain a clone for background
/// work, and [`Self::read`] / [`Self::write`] for synchronous system access.
#[derive(Resource)]
pub struct GeodesyContext(Arc<std::sync::RwLock<geodesy::ctx::Minimal>>);

impl GeodesyContext {
    /// Clone the context handle for use in a background job.
    ///
    /// This is the only way to obtain a second handle, making the
    /// `Arc` cloning explicit rather than implicit via `Clone`.
    pub fn clone_for_async(&self) -> Self {
        GeodesyContext(Arc::clone(&self.0))
    }

    /// Acquire a read lock on the inner geodesy context.
    pub fn read(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, geodesy::ctx::Minimal>, GeodesyContextLockError>
    {
        self.0.read().map_err(|_| GeodesyContextLockError)
    }

    /// Acquire a write lock on the inner geodesy context.
    pub fn write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, geodesy::ctx::Minimal>, GeodesyContextLockError>
    {
        self.0.write().map_err(|_| GeodesyContextLockError)
    }
}

/// Error returned when the `GeodesyContext` lock is poisoned.
#[derive(Debug)]
pub struct GeodesyContextLockError;

impl std::fmt::Display for GeodesyContextLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GeodesyContext lock poisoned")
    }
}

impl std::error::Error for GeodesyContextLockError {}

pub fn epsg_code_to_geodesy_op_handle<C: geodesy::ctx::Context>(
    ctx: &mut C,
    source_crs: u16,
) -> Result<geodesy::ctx::OpHandle, Error> {
    let source =
        crs_definitions::from_code(source_crs).ok_or(Error::UnknownEpsgCode(source_crs))?;
    let source_geodesy_string = geodesy::authoring::parse_proj(source.proj4)?;
    let source_op_handle = ctx.op(&source_geodesy_string)?;
    Ok(source_op_handle)
}

pub fn proj_string_to_geodesy_op_handle<C: geodesy::ctx::Context>(
    ctx: &mut C,
    proj_string: &str,
) -> Result<geodesy::ctx::OpHandle, Error> {
    let geodesy_string = geodesy::authoring::parse_proj(proj_string)?;
    let op_handle = ctx.op(&geodesy_string)?;
    Ok(op_handle)
}

#[derive(Debug)]
pub enum Error {
    Geodesy(geodesy::Error),
    UnknownEpsgCode(u16),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Geodesy(err) => write!(f, "Geodesy error: {err}"),
            Error::UnknownEpsgCode(code) => write!(f, "Unknown EPSG code: {code}"),
        }
    }
}

impl From<geodesy::Error> for Error {
    fn from(err: geodesy::Error) -> Self {
        Error::Geodesy(err)
    }
}

impl std::error::Error for Error {}

// --- CRS plugin ---

#[derive(Resource, Clone)]
pub struct TargetCrs(pub Crs);

/// Configuration for the CRS plugin.
pub struct Plugin {
    /// The default target CRS EPSG code used on startup.
    pub default_crs: u16,
}

impl Default for Plugin {
    fn default() -> Self {
        Self { default_crs: 3857 }
    }
}

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.insert_resource(GeodesyContext(Default::default()))
            .insert_resource(DefaultTargetCrs(self.default_crs))
            .add_systems(Update, handle_crs_changed_events.in_set(rgis_primitives::RgisSet::LayerProcessing))
            .add_systems(Startup, insert_target_crs);
    }
}

/// Resource holding the configured default target CRS EPSG code.
#[derive(Resource)]
struct DefaultTargetCrs(u16);

fn insert_target_crs(
    mut commands: Commands,
    geodesy_ctx: Res<GeodesyContext>,
    default_target_crs: Res<DefaultTargetCrs>,
) -> Result {
    let default_crs = default_target_crs.0;
    let mut geodesy_ctx = match geodesy_ctx.write() {
        Ok(ctx) => ctx,
        Err(_) => {
            return Err("Failed to acquire geodesy context write lock".into());
        }
    };
    let op_handle = epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx, default_crs)?;
    let wgs84_op_handle = epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx, 4326)?;
    commands.insert_resource(TargetCrs(Crs {
        epsg_code: Some(default_crs),
        proj_string: None,
        op_handle,
    }));
    commands.insert_resource(Wgs84OpHandle(wgs84_op_handle));
    Ok(())
}

fn handle_crs_changed_events(
    mut change_crs_event_reader: MessageReader<rgis_events::ChangeCrsMessage>,
    mut commands: Commands,
    mut target_crs: ResMut<TargetCrs>,
) {
    if let Some(event) = change_crs_event_reader.read().last() {
        target_crs.0 = event.new.clone();
        commands.trigger(rgis_events::CrsChangedEvent {
            old: event.old.clone(),
            new: event.new.clone(),
        });
    }
}
