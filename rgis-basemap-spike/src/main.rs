//! Spike: render a Galileo OSM basemap into an offscreen texture, then display
//! the resulting RGBA bytes as a Bevy sprite.
//!
//! This is deliberately minimum-viable: one-shot render at startup, no camera
//! sync with rgis, no per-frame updates. The goal is to prove end-to-end that
//! Galileo can build inside the rgis workspace alongside Bevy 0.18 (which
//! pins wgpu 27, while Galileo pins wgpu 29 — two separate compilation
//! units, no device sharing).

use std::path::Path;
use std::time::Duration;

use bevy::prelude::*;
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::render::WgpuRenderer;
use galileo::{Map, MapView, Messenger};
use galileo_types::cartesian::Size;
use galileo_types::latlon;

const IMAGE_WIDTH: u32 = 1024;
const IMAGE_HEIGHT: u32 = 1024;
// Meters per pixel in EPSG:3857. ~150 m/px ≈ zoom level ~11 at the equator.
const RESOLUTION: f64 = 150.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "rgis-basemap-spike".into(),
                resolution: (IMAGE_WIDTH, IMAGE_HEIGHT).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(FramesUntilExit(3))
        .add_systems(Startup, setup)
        .add_systems(Update, exit_after_a_few_frames)
        .run();
}

#[derive(Resource)]
struct FramesUntilExit(u32);

fn exit_after_a_few_frames(
    mut frames: ResMut<FramesUntilExit>,
    mut exit: MessageWriter<AppExit>,
) {
    if frames.0 == 0 {
        exit.write(AppExit::Success);
    } else {
        frames.0 -= 1;
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn(Camera2d);

    info!("rendering Galileo basemap (this downloads OSM tiles on first run)…");
    let rgba = render_galileo_basemap();
    info!("Galileo basemap ready: {} bytes", rgba.len());

    // Dump a PNG so the spike has a verifiable artifact independent of Bevy's window.
    let png_path = Path::new("/tmp/spike-basemap.png");
    match image::RgbaImage::from_raw(IMAGE_WIDTH, IMAGE_HEIGHT, rgba.clone()) {
        Some(buf) => match buf.save(png_path) {
            Ok(()) => info!("wrote {}", png_path.display()),
            Err(e) => warn!("failed to write png: {e}"),
        },
        None => warn!("failed to wrap rgba bytes as ImageBuffer"),
    }

    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: IMAGE_WIDTH,
            height: IMAGE_HEIGHT,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        rgba,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );

    let handle = images.add(image);
    commands.spawn(Sprite {
        image: handle,
        ..default()
    });
}

/// Block on a fresh tokio runtime to drive Galileo's async tile fetch +
/// offscreen render. Returns the raw RGBA bytes of the rendered map.
fn render_galileo_basemap() -> Vec<u8> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let size = Size::new(IMAGE_WIDTH, IMAGE_HEIGHT);

        let mut osm = RasterTileLayerBuilder::new_osm()
            .with_file_cache_checked(".tile_cache")
            .build()
            .expect("build OSM layer");
        // Without this, first render returns transparent tiles.
        osm.set_fade_in_duration(Duration::default());

        // Center on Manhattan. `latlon!` yields a GeoPoint2d, which
        // `MapView::new` accepts as `&impl GeoPoint`.
        let center = latlon!(40.7580, -73.9855);
        let map_view = MapView::new(&center, RESOLUTION).with_size(size.cast());

        osm.load_tiles(&map_view).await;

        let map = Map::new(
            map_view,
            vec![Box::new(osm)],
            None::<Box<dyn Messenger>>,
        );

        let renderer = WgpuRenderer::new_with_texture_rt(size)
            .await
            .expect("create Galileo renderer");
        renderer.render(&map).expect("galileo render");
        renderer
            .get_image()
            .await
            .expect("galileo get_image")
    })
}
