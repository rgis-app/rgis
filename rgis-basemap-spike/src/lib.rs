//! Spike plugin: render an OpenStreetMap basemap via Galileo and display it
//! as a Bevy sprite that tracks the rgis 2D camera.
//!
//! Architecture:
//! - At startup, a dedicated OS worker thread is spawned. The worker owns:
//!     * a tokio multi-thread runtime
//!     * a Galileo `WgpuRenderer` (independent wgpu 29 context, never touched
//!       by the Bevy main thread)
//! - The main thread holds a `BasemapState` resource with the request/result
//!   channels and the Bevy `Image` handle backing the basemap sprite.
//! - Each `Update`, `refresh_basemap` does three things on the main thread,
//!   all non-blocking:
//!     1. Drains pending `RenderResponse`s from the worker (latest wins);
//!        if any arrived, copies the RGBA bytes into the existing `Image`
//!        asset and records the snapshot.
//!     2. Reads the rgis `Camera2d` transform; if it has zoomed past
//!        `SCALE_DELTA` or panned past the central region of the last
//!        rendered tile, sends a new `RenderRequest` (provided no request
//!        is already in flight and the debounce window has elapsed).
//!     3. Syncs the basemap sprite's transform to the latest snapshot so
//!        it sits at the correct world position and scale.
//! - The worker loop blocks on the request channel, drains any backlog so
//!   only the newest request is honored, then drives Galileo through tokio
//!   `block_on` (off the main thread, so this never hitches Bevy).

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::render::WgpuRenderer;
use galileo::{Map, MapView, Messenger};
use galileo_types::cartesian::{Point2, Size};

const TEXTURE_SIZE: u32 = 1024;
/// Re-render if `|new_scale / old_scale - 1|` exceeds this.
const SCALE_DELTA: f32 = 0.08;
/// Re-render if camera moves more than this fraction of the texture's
/// world-space half-width away from the rendered center.
const PAN_DELTA: f32 = 0.1;
/// Minimum spacing between successive request submissions.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(100);
/// Z position for the basemap sprite. rgis layer z values start at 0 and
/// grow positive, so we want negative. Bevy 2D's default OrthographicProjection
/// uses near=-1000, far=1000, and rgis parks the camera at z=999.9, so the
/// visible world-z range is roughly [-0.1, 1999.9]. Keep the basemap just
/// inside the front plane so rgis layers at z≥0 still draw on top.
const BASEMAP_Z: f32 = -0.05;

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, init_basemap);
        app.add_systems(Update, refresh_basemap);
    }
}

#[derive(Component)]
struct BasemapSprite;

#[derive(Resource)]
struct BasemapState {
    request_tx: Mutex<Sender<RenderRequest>>,
    result_rx: Mutex<Receiver<RenderResponse>>,
    image_handle: Handle<Image>,
    in_flight: bool,
    last_render: Option<RenderSnapshot>,
    last_request_at: Option<Instant>,
}

#[derive(Clone, Copy)]
struct RenderSnapshot {
    center_x: f32,
    center_y: f32,
    scale: f32,
}

struct RenderRequest {
    center_x: f64,
    center_y: f64,
    resolution: f64,
}

struct RenderResponse {
    rgba: Vec<u8>,
    snapshot: RenderSnapshot,
}

fn init_basemap(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let (request_tx, request_rx) = mpsc::channel::<RenderRequest>();
    let (response_tx, response_rx) = mpsc::channel::<RenderResponse>();

    thread::Builder::new()
        .name("galileo-worker".into())
        .spawn(move || worker_loop(request_rx, response_tx))
        .expect("spawn galileo worker thread");

    let image_handle = images.add(blank_rgba_image());

    commands.spawn((
        Sprite {
            image: image_handle.clone(),
            custom_size: Some(Vec2::splat(TEXTURE_SIZE as f32)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, BASEMAP_Z),
        Visibility::Visible,
        Name::new("galileo basemap"),
        BasemapSprite,
    ));

    commands.insert_resource(BasemapState {
        request_tx: Mutex::new(request_tx),
        result_rx: Mutex::new(response_rx),
        image_handle,
        in_flight: false,
        last_render: None,
        last_request_at: None,
    });
}

fn refresh_basemap(
    mut state: ResMut<BasemapState>,
    camera_query: Query<&Transform, (With<Camera2d>, Without<BasemapSprite>)>,
    mut sprite_query: Query<&mut Transform, With<BasemapSprite>>,
    mut images: ResMut<Assets<Image>>,
) {
    // 1. Drain any worker results, keeping only the latest.
    let mut latest_response: Option<RenderResponse> = None;
    {
        let rx = state.result_rx.lock().expect("result_rx mutex poisoned");
        loop {
            match rx.try_recv() {
                Ok(response) => latest_response = Some(response),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("rgis_basemap_spike: worker channel disconnected");
                    break;
                }
            }
        }
    }
    if let Some(response) = latest_response {
        // Replace the asset in-place at the existing handle so Bevy's render
        // world re-extracts into the same texture slot. Handle swap (add + new
        // handle) works in debug but races with asset extraction in release.
        if let Err(e) = images.insert(&state.image_handle, rgba_to_image(response.rgba)) {
            warn!("rgis_basemap_spike: failed to insert basemap image: {e:?}");
        }
        state.last_render = Some(response.snapshot);
        state.in_flight = false;
    }

    // 2. Read camera state.
    let Ok(cam) = camera_query.single() else {
        return;
    };
    let cam_x = cam.translation.x;
    let cam_y = cam.translation.y;
    let cam_scale = cam.scale.x;
    if !cam_scale.is_normal() || !cam_x.is_finite() || !cam_y.is_finite() {
        return;
    }

    let needs_render = match state.last_render {
        None => true,
        Some(snap) => {
            let scale_changed = ((cam_scale / snap.scale) - 1.0).abs() > SCALE_DELTA;
            let texture_world_half = (TEXTURE_SIZE as f32 * snap.scale) * 0.5;
            let pan_threshold = texture_world_half * PAN_DELTA;
            let pan_changed = (cam_x - snap.center_x).abs() > pan_threshold
                || (cam_y - snap.center_y).abs() > pan_threshold;
            scale_changed || pan_changed
        }
    };

    let too_soon = state
        .last_request_at
        .map(|t| t.elapsed() < REFRESH_DEBOUNCE)
        .unwrap_or(false);

    if needs_render && !too_soon && !state.in_flight {
        let req = RenderRequest {
            center_x: cam_x as f64,
            center_y: cam_y as f64,
            resolution: cam_scale as f64,
        };
        let send_result = {
            let tx = state.request_tx.lock().expect("request_tx mutex poisoned");
            tx.send(req)
        };
        if send_result.is_ok() {
            state.in_flight = true;
            state.last_request_at = Some(Instant::now());
        } else {
            warn!("rgis_basemap_spike: worker thread is dead, can't send request");
        }
    }

    // 3. Sync sprite transform to whatever the latest render snapshot is.
    if let Some(snap) = state.last_render {
        if let Ok(mut sprite_t) = sprite_query.single_mut() {
            sprite_t.translation.x = snap.center_x;
            sprite_t.translation.y = snap.center_y;
            sprite_t.translation.z = BASEMAP_Z;
            sprite_t.scale = Vec3::new(snap.scale, snap.scale, 1.0);
        }
    }
}

/// Worker thread entry point. Owns the tokio runtime + Galileo renderer for
/// its entire lifetime so the main Bevy thread never touches them.
fn worker_loop(rx: Receiver<RenderRequest>, tx: Sender<RenderResponse>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("rgis_basemap_spike[worker]: tokio runtime build failed: {e}");
            return;
        }
    };

    let renderer = match runtime.block_on(async {
        WgpuRenderer::new_with_texture_rt(Size::new(TEXTURE_SIZE, TEXTURE_SIZE)).await
    }) {
        Some(r) => r,
        None => {
            error!("rgis_basemap_spike[worker]: WgpuRenderer init failed");
            return;
        }
    };

    while let Ok(first) = rx.recv() {
        // Coalesce: if the user kept moving the camera while we were busy
        // initializing or rendering, drain the queue and keep only the newest.
        let mut latest = first;
        while let Ok(newer) = rx.try_recv() {
            latest = newer;
        }

        let rgba = runtime.block_on(async {
            let mut osm = RasterTileLayerBuilder::new_osm()
                .with_file_cache_checked(".tile_cache")
                .build()
                .ok()?;
            osm.set_fade_in_duration(Duration::default());

            let center = Point2::<f64>::new(latest.center_x, latest.center_y);
            let map_view = MapView::new_projected(&center, latest.resolution)
                .with_size(Size::new(TEXTURE_SIZE as f64, TEXTURE_SIZE as f64));

            osm.load_tiles(&map_view).await;

            let map = Map::new(map_view, vec![Box::new(osm)], None::<Box<dyn Messenger>>);
            renderer.render(&map).ok()?;
            renderer.get_image().await.ok()
        });

        match rgba {
            Some(rgba) => {
                let response = RenderResponse {
                    rgba,
                    snapshot: RenderSnapshot {
                        center_x: latest.center_x as f32,
                        center_y: latest.center_y as f32,
                        scale: latest.resolution as f32,
                    },
                };
                if tx.send(response).is_err() {
                    break;
                }
            }
            None => {
                warn!("rgis_basemap_spike[worker]: render produced no bytes");
            }
        }
    }
}

fn blank_rgba_image() -> Image {
    let len = (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize;
    rgba_to_image(vec![0u8; len])
}

fn rgba_to_image(rgba: Vec<u8>) -> Image {
    Image::new(
        bevy::render::render_resource::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        rgba,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    )
}
