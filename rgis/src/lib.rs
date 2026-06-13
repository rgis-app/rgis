use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_widget_rect(label: &str) -> JsValue {
    match rgis_ui::widget_registry::get(label) {
        Some(rect) => {
            let arr = js_sys::Array::new();
            for v in rect {
                arr.push(&JsValue::from_f64(f64::from(v)));
            }
            arr.into()
        }
        None => JsValue::NULL,
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_rendered_layer_count() -> u32 {
    rgis_renderer::RENDERED_LAYER_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_active_fade_count() -> u32 {
    rgis_renderer::ACTIVE_FADE_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_animations_enabled(enabled: bool) {
    rgis_renderer::ANIMATIONS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn close_window(title: &str) {
    rgis_ui::widget_registry::request_close(title);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_first_layer_fill_color(r: f32, g: f32, b: f32, a: f32) {
    rgis_ui::widget_registry::request_set_fill_color([r, g, b, a]);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_all_widget_rects() -> JsValue {
    let all = rgis_ui::widget_registry::get_all();
    let obj = js_sys::Object::new();
    for (label, rect) in &all {
        let arr = js_sys::Array::new();
        for v in rect {
            arr.push(&JsValue::from_f64(f64::from(*v)));
        }
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(label), &arr);
    }
    obj.into()
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct StartupUrl(String);

#[cfg(not(target_arch = "wasm32"))]
fn load_startup_url(
    url_res: Res<StartupUrl>,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
    mut load_file_writer: MessageWriter<rgis_events::LoadFileMessage>,
) {
    let mut geodesy_ctx = geodesy_ctx.write().unwrap();
    let Ok(op_handle) = rgis_crs::epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx, 4326) else {
        error!("Failed to create op handle for EPSG:4326");
        return;
    };
    load_file_writer.write(rgis_events::LoadFileMessage::FromNetwork {
        name: url_res.0.clone(),
        url: url_res.0.clone(),
        source_crs: rgis_primitives::Crs {
            epsg_code: Some(4326),
            proj_string: None,
            op_handle,
        },
    });
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn run() {
    #[cfg(not(target_arch = "wasm32"))]
    let cli_args = rgis_cli::run().ok();

    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "rgis".to_string(),
                    canvas: Some("#rgis".into()),
                    // Under eguidev automation, pin the window to Playwright's
                    // coordinate space (1280x720 logical, scale factor 1.0) so
                    // the two harnesses test the same layout.
                    #[cfg(all(feature = "eguidev", not(target_arch = "wasm32")))]
                    resolution: bevy::window::WindowResolution::new(1280, 720)
                        .with_scale_factor_override(1.0),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .disable::<bevy::log::LogPlugin>(),
    );
    app.add_plugins(bevy::log::LogPlugin {
        custom_layer: rgis_ui::log_buffer::create_log_layer,
        ..default()
    });
    app.add_plugins(MeshPickingPlugin);
    app.add_plugins(rgis_ui::Plugin);
    app.add_plugins(rgis_layers::Plugin);
    app.add_plugins(rgis_file_loader::Plugin);
    app.add_plugins(rgis_renderer::Plugin);
    app.add_plugins(rgis_grid::Plugin);
    app.add_plugins(rgis_mouse::Plugin);
    app.add_plugins(rgis_camera::Plugin::default());
    app.add_plugins(rgis_ui_messages::Plugin);
    app.add_plugins(rgis_events::RgisEventsPlugin);
    app.add_plugins(bevy_jobs::Plugin);
    app.add_plugins(rgis_transform::Plugin);
    app.add_plugins(rgis_settings::Plugin);
    app.add_plugins(rgis_crs::Plugin::default());

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(ref args) = cli_args {
        if let Some(ref url) = args.url {
            app.insert_resource(StartupUrl(url.clone()));
            app.add_systems(Startup, load_startup_url);
        }
    }

    // Establish explicit ordering between system sets to prevent race conditions.
    // For example, Transform must complete before Rendering so that a CRS change
    // doesn't despawn meshes that were just projected in the same frame.
    app.configure_sets(
        Update,
        (
            rgis_primitives::RgisSet::FileLoading,
            rgis_primitives::RgisSet::LayerProcessing,
            rgis_primitives::RgisSet::Transform,
            rgis_primitives::RgisSet::Rendering,
            rgis_primitives::RgisSet::Camera,
        )
            .chain(),
    );

    app.run();
}
