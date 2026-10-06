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

/// The app state as JSON (see `rgis_automation::AppState`) as of the end of
/// the last frame. The state is only tracked once it's been asked for, so the
/// first call (and any call before the next frame) returns `"null"`; poll.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_app_state() -> String {
    match rgis_automation::latest_state_json() {
        Some(Ok(json)) => json,
        Some(Err(e)) => {
            error!("Couldn't serialize the app state: {e}");
            "null".into()
        }
        None => "null".into(),
    }
}

/// Queue one `{"cmd": ...}` step, or an array of them, to run on the next
/// frames. Throws if the JSON doesn't parse; errors while running show up in
/// `get_app_state().script.last_error` and `command_errors`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn dispatch(json: &str) -> Result<(), JsValue> {
    rgis_automation::dispatch_json(json)
        .map(|_| ())
        .map_err(|e| JsValue::from_str(&e))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn close_window(title: &str) {
    rgis_ui::widget_registry::request_close(title);
}

/// Set the fill color of the top layer. Kept for existing tests; equivalent
/// to dispatching `set_fill_color` with `"layer": -1`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_first_layer_fill_color(r: f32, g: f32, b: f32, a: f32) {
    rgis_automation::enqueue([rgis_automation::Command::SetFillColor {
        layer: rgis_automation::command::LayerRef::Index(-1),
        color: rgis_automation::command::ColorSpec(Color::linear_rgba(r, g, b, a)),
    }]);
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
        file_format: geo_file_loader::FileFormat::GeoJson,
        source_crs: rgis_primitives::Crs {
            epsg_code: Some(4326),
            proj_string: None,
            op_handle,
        },
    });
}

/// Build the script for `--script`, `--screenshot`, `--dump-state`, and
/// `--exit-after-script`, or `None` for a normal interactive run.
#[cfg(not(target_arch = "wasm32"))]
fn script_runner(
    args: &rgis_cli::CliArgs,
) -> Result<Option<rgis_automation::ScriptRunner>, String> {
    use rgis_automation::Command;

    let mut steps = match &args.script {
        Some(script) => {
            let json = match script.strip_prefix('@') {
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|e| format!("couldn't read script file {path}: {e}"))?,
                None => script.clone(),
            };
            rgis_automation::parse_script(&json).map_err(|e| format!("invalid --script: {e}"))?
        }
        // Still let startup work (e.g. --url) settle before writing outputs.
        None if args.is_batch() => vec![Command::WaitIdle {
            max_frames: None,
            timeout_secs: None,
        }],
        None => return Ok(None),
    };
    // Dump last, so the state shows every other step as completed.
    if let Some(path) = &args.screenshot {
        steps.push(Command::Screenshot {
            path: path.to_string_lossy().into_owned(),
        });
    }
    if let Some(path) = &args.dump_state {
        steps.push(Command::DumpState {
            path: path.to_string_lossy().into_owned(),
        });
    }

    let runner = rgis_automation::ScriptRunner::new(steps);
    Ok(Some(if args.is_batch() {
        runner.exit_when_done(args.dump_state.clone())
    } else {
        runner
    }))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn run() {
    #[cfg(not(target_arch = "wasm32"))]
    let cli_args = rgis_cli::run().ok();
    #[cfg(not(target_arch = "wasm32"))]
    let script_runner = match cli_args.as_ref().map(script_runner).transpose() {
        Ok(runner) => runner.flatten(),
        Err(e) => {
            eprintln!("rgis: {e}");
            std::process::exit(2);
        }
    };

    #[allow(unused_mut)]
    let mut primary_window = Window {
        title: "rgis".to_string(),
        canvas: Some("#rgis".into()),
        ..Default::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(args) = &cli_args {
        let batch = args.is_batch();
        if let Some((width, height)) = args.window_size.or(batch.then_some((1280, 720))) {
            primary_window.resolution = bevy::window::WindowResolution::new(width, height);
        }
        if batch {
            // Same pixel size on every display, like the Playwright viewport.
            primary_window.resolution.set_scale_factor_override(Some(1.0));
            // On macOS, a visible window that's covered by other windows stops
            // getting redraws, which stalls the app. Hidden windows keep
            // rendering (and can be screenshotted), so hide it unless asked;
            // when shown, keep it on top so it isn't covered.
            primary_window.visible = args.show_window;
            if args.show_window {
                primary_window.window_level = bevy::window::WindowLevel::AlwaysOnTop;
            }
        }
    }

    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(primary_window),
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
    app.add_plugins(rgis_automation::Plugin);

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(ref args) = cli_args {
        if let Some(ref url) = args.url {
            app.insert_resource(StartupUrl(url.clone()));
            app.add_systems(Startup, load_startup_url);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(runner) = script_runner {
        app.insert_resource(runner);
        // Keep updating while the window is unfocused (this is Bevy's
        // default, but scripted runs depend on it).
        app.insert_resource(bevy::winit::WinitSettings::continuous());
    }

    // Establish explicit ordering between system sets to prevent race conditions.
    // For example, Transform must complete before Rendering so that a CRS change
    // doesn't despawn meshes that were just projected in the same frame.
    rgis_primitives::RgisSet::configure(&mut app);

    let exit = app.run();
    #[cfg(not(target_arch = "wasm32"))]
    if let AppExit::Error(code) = exit {
        std::process::exit(i32::from(code.get()));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = exit;
}
