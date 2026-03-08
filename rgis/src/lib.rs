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

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn run() {
    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "rgis".to_string(),
                    canvas: Some("#rgis".into()),
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
    app.add_plugins(rgis_mouse::Plugin);
    app.add_plugins(rgis_camera::Plugin::default());
    app.add_plugins(rgis_ui_messages::Plugin);
    app.add_plugins(rgis_events::RgisEventsPlugin);
    app.add_plugins(bevy_jobs::Plugin);
    app.add_plugins(rgis_transform::Plugin);
    app.add_plugins(rgis_settings::Plugin);
    app.add_plugins(rgis_crs::Plugin::default());

    app.run();
}
