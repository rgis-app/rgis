//! Runs [`Command`]s against the world.
//!
//! Steps run from an exclusive system in `PreUpdate`, so the messages they
//! write are handled by the regular rgis systems in the same frame. At most
//! one command is applied per frame, so each step sees the effects of the
//! previous one (e.g. `delete_layer -1` twice deletes two layers).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use bevy::prelude::*;

use crate::command::{Command, CrsSpec, Format, LayerRef};
use crate::idle::{busy_reason, IdleTracker};
use crate::state::ScriptState;

/// Frames to let startup finish (window, egui context, target CRS) before
/// the first step runs.
const STARTUP_FRAMES: u32 = 2;
const DEFAULT_WAIT_IDLE_MAX_FRAMES: u32 = 36_000;
const DEFAULT_WAIT_IDLE_TIMEOUT_SECS: f64 = 60.;
#[cfg(not(target_arch = "wasm32"))]
const SCREENSHOT_MAX_FRAMES: u32 = 600;

/// Commands queued from outside the schedule (the wasm `dispatch` export).
static INBOX: Mutex<Vec<Command>> = Mutex::new(Vec::new());

/// Queue commands for the [`ScriptRunner`] of the running app. Used by the
/// wasm exports, which can't reach the `World` directly.
pub fn enqueue(commands: impl IntoIterator<Item = Command>) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.extend(commands);
    }
}

pub(crate) fn inbox_len() -> usize {
    INBOX.lock().map_or(0, |inbox| inbox.len())
}

fn take_inbox() -> Vec<Command> {
    INBOX
        .lock()
        .map(|mut inbox| std::mem::take(&mut *inbox))
        .unwrap_or_default()
}

/// Queue of script steps plus the step currently waiting.
#[derive(Resource, Default)]
pub struct ScriptRunner {
    queue: VecDeque<Command>,
    current: Option<(&'static str, Waiting)>,
    next_step: usize,
    completed: usize,
    last_error: Option<String>,
    failed: bool,
    exit_when_done: bool,
    exit_sent: bool,
    #[cfg(not(target_arch = "wasm32"))]
    dump_state_on_failure: Option<std::path::PathBuf>,
}

enum Waiting {
    Frames {
        remaining: u32,
    },
    Idle {
        frames: u32,
        max_frames: u32,
        started: Duration,
        timeout: Duration,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Screenshot {
        path: std::path::PathBuf,
        frames: u32,
        outcome: std::sync::Arc<Mutex<Option<Result<(), String>>>>,
    },
}

enum Poll {
    Pending,
    Done,
    Failed(String),
}

impl ScriptRunner {
    pub fn new(steps: impl IntoIterator<Item = Command>) -> Self {
        let mut runner = ScriptRunner::default();
        runner.push(steps);
        runner
    }

    /// Send `AppExit` once every step has run: success, or exit code 1 if a
    /// step failed. On failure, the state is still written to
    /// `dump_state_on_failure` so the caller can see what went wrong.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn exit_when_done(mut self, dump_state_on_failure: Option<std::path::PathBuf>) -> Self {
        self.exit_when_done = true;
        self.dump_state_on_failure = dump_state_on_failure;
        self
    }

    pub fn push(&mut self, steps: impl IntoIterator<Item = Command>) {
        self.queue.extend(steps);
    }

    /// True when no steps are queued or in progress.
    pub fn is_finished(&self) -> bool {
        self.queue.is_empty() && self.current.is_none()
    }

    /// The error from the most recent failed step, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn state(&self) -> ScriptState {
        ScriptState {
            pending_steps: self.queue.len() + usize::from(self.current.is_some()),
            current_step: self.current.as_ref().map(|(name, _)| name.to_string()),
            completed_steps: self.completed,
            last_error: self.last_error.clone(),
        }
    }

    /// Called once per frame: finish the waiting step if it's done, then
    /// start at most one new step.
    fn tick(&mut self, world: &mut World) {
        if let Some((_, waiting)) = &mut self.current {
            match poll(waiting, world) {
                Poll::Pending => return,
                Poll::Done => {
                    self.current = None;
                    self.completed += 1;
                }
                Poll::Failed(error) => {
                    self.fail(world, error);
                    return;
                }
            }
        }

        let Some(command) = self.queue.pop_front() else {
            self.finish(world);
            return;
        };
        self.next_step += 1;
        let name = command.name();
        match self.start(world, command) {
            // Polled from the next frame on.
            Ok(Some(waiting)) => self.current = Some((name, waiting)),
            Ok(None) => {
                self.completed += 1;
                // Give follow-up work a chance to show up before anyone
                // treats the app as idle.
                if let Some(mut tracker) = world.get_resource_mut::<IdleTracker>() {
                    tracker.reset();
                }
            }
            Err(error) => {
                let error = format!("step {} ({name}): {error}", self.next_step);
                self.fail(world, error);
            }
        }
    }

    /// Run `command`. Returns what to wait for, if anything.
    fn start(&self, world: &mut World, command: Command) -> Result<Option<Waiting>, String> {
        match command {
            Command::Frames { n } => Ok((n > 0).then_some(Waiting::Frames { remaining: n })),
            Command::WaitIdle {
                max_frames,
                timeout_secs,
            } => {
                let timeout = Duration::try_from_secs_f64(
                    timeout_secs.unwrap_or(DEFAULT_WAIT_IDLE_TIMEOUT_SECS),
                )
                .map_err(|e| format!("bad `timeout_secs`: {e}"))?;
                Ok(Some(Waiting::Idle {
                    frames: 0,
                    max_frames: max_frames.unwrap_or(DEFAULT_WAIT_IDLE_MAX_FRAMES),
                    started: real_elapsed(world),
                    timeout,
                }))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Command::DumpState { path } => {
                write_state(world, self.state(), std::path::Path::new(&path))?;
                Ok(None)
            }
            #[cfg(not(target_arch = "wasm32"))]
            Command::Screenshot { path } => start_screenshot(world, path.into()).map(Some),
            #[cfg(target_arch = "wasm32")]
            Command::DumpState { .. } | Command::Screenshot { .. } => Err(
                "only available in native builds; on the web, use get_app_state() and \
                 Playwright screenshots"
                    .into(),
            ),
            command => apply(world, command).map(|()| None),
        }
    }

    fn fail(&mut self, world: &mut World, error: String) {
        let skipped = self.queue.len();
        self.queue.clear();
        self.current = None;
        self.failed = true;
        self.last_error = Some(error.clone());
        if let Some(mut log) = world.get_resource_mut::<crate::AutomationLog>() {
            log.push_command_error(error.clone());
        }
        warn!("Script step failed: {error}");

        #[cfg(not(target_arch = "wasm32"))]
        {
            if skipped > 0 {
                eprintln!("rgis: {error} (skipped the remaining {skipped} step(s))");
            } else {
                eprintln!("rgis: {error}");
            }
            if let Some(path) = self.dump_state_on_failure.take() {
                if let Err(e) = write_state(world, self.state(), &path) {
                    eprintln!("rgis: couldn't write the state dump: {e}");
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = skipped;
    }

    fn finish(&mut self, world: &mut World) {
        if !self.exit_when_done || self.exit_sent {
            return;
        }
        self.exit_sent = true;
        world.write_message(if self.failed {
            AppExit::from_code(1)
        } else {
            AppExit::Success
        });
    }
}

fn poll(waiting: &mut Waiting, world: &mut World) -> Poll {
    match waiting {
        Waiting::Frames { remaining } => {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                Poll::Done
            } else {
                Poll::Pending
            }
        }
        Waiting::Idle {
            frames,
            max_frames,
            started,
            timeout,
        } => {
            *frames += 1;
            if world
                .get_resource::<IdleTracker>()
                .is_some_and(IdleTracker::is_idle)
            {
                return Poll::Done;
            }
            let elapsed = real_elapsed(world).saturating_sub(*started);
            if *frames > *max_frames || elapsed > *timeout {
                let reason = busy_reason(world)
                    .unwrap_or_else(|| "the app only just became quiet".to_string());
                return Poll::Failed(format!(
                    "wait_idle timed out after {frames} frames ({:.1}s): {reason}",
                    elapsed.as_secs_f64()
                ));
            }
            Poll::Pending
        }
        #[cfg(not(target_arch = "wasm32"))]
        Waiting::Screenshot {
            path,
            frames,
            outcome,
        } => {
            *frames += 1;
            match outcome.lock().ok().and_then(|mut outcome| outcome.take()) {
                Some(Ok(())) => {
                    info!("Saved screenshot to {}", path.display());
                    Poll::Done
                }
                Some(Err(e)) => Poll::Failed(format!(
                    "couldn't save the screenshot to {}: {e}",
                    path.display()
                )),
                None if *frames > SCREENSHOT_MAX_FRAMES => {
                    Poll::Failed(format!("no screenshot was captured after {frames} frames"))
                }
                None => Poll::Pending,
            }
        }
    }
}

fn real_elapsed(world: &World) -> Duration {
    world
        .get_resource::<Time<Real>>()
        .map_or(Duration::ZERO, Time::elapsed)
}

/// Exclusive system: feeds queued commands to the [`ScriptRunner`].
pub(crate) fn run_script(world: &mut World) {
    let inbox = take_inbox();
    if !world.contains_resource::<ScriptRunner>() {
        world.init_resource::<ScriptRunner>();
    }
    world.resource_scope(|world, mut runner: Mut<ScriptRunner>| {
        runner.push(inbox);
        let frame = world
            .get_resource::<bevy::diagnostic::FrameCount>()
            .map_or(u32::MAX, |frame| frame.0);
        if frame >= STARTUP_FRAMES {
            runner.tick(world);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn write_state(
    world: &mut World,
    script: ScriptState,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut state = crate::state::app_state(world);
    // The runner is out of the world while it runs a step, so fill in its
    // state here.
    state.script = script;
    let json = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("couldn't serialize the state: {e}"))?;
    create_parent_dir(path)?;
    std::fs::write(path, json + "\n")
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    info!("Wrote app state to {}", path.display());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn create_parent_dir(path: &std::path::Path) -> Result<(), String> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => std::fs::create_dir_all(parent)
            .map_err(|e| format!("couldn't create {}: {e}", parent.display())),
        _ => Ok(()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_screenshot(world: &mut World, path: std::path::PathBuf) -> Result<Waiting, String> {
    use bevy::render::view::screenshot::{CapturedScreenshots, Screenshot, ScreenshotCaptured};

    if !world.contains_resource::<CapturedScreenshots>() {
        return Err("screenshots need the renderer, which isn't running".into());
    }
    let is_png = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(format!("{} must end in .png", path.display()));
    }
    create_parent_dir(&path)?;

    let outcome = std::sync::Arc::new(Mutex::new(None));
    let observer_outcome = std::sync::Arc::clone(&outcome);
    let observer_path = path.clone();
    world
        .spawn(Screenshot::primary_window())
        .observe(move |captured: On<ScreenshotCaptured>| {
            let result = captured
                .image
                .clone()
                .try_into_dynamic()
                .map_err(|e| e.to_string())
                // Drop the alpha channel, like Bevy's `save_to_disk`.
                .and_then(|image| {
                    image
                        .to_rgb8()
                        .save(&observer_path)
                        .map_err(|e| e.to_string())
                });
            if let Ok(mut outcome) = observer_outcome.lock() {
                *outcome = Some(result);
            }
        });
    Ok(Waiting::Screenshot {
        path,
        frames: 0,
        outcome,
    })
}

/// Apply a command that takes effect immediately by writing the matching
/// rgis messages.
fn apply(world: &mut World, command: Command) -> Result<(), String> {
    match command {
        Command::LoadFile {
            path,
            format,
            crs,
            name,
        } => load_file(world, &path, format, crs, name),
        Command::LoadUrl {
            url,
            format,
            crs,
            name,
        } => {
            let format = format
                .or_else(|| Format::from_path(&url))
                .unwrap_or(Format::Geojson);
            let source_crs = build_crs(world, &crs.unwrap_or(CrsSpec::Epsg(4326)))?;
            let name = name.unwrap_or_else(|| {
                let path = url.split(['?', '#']).next().unwrap_or(&url);
                path.rsplit('/').next().unwrap_or(path).to_string()
            });
            send(
                world,
                rgis_events::LoadFileMessage::FromNetwork {
                    name,
                    url,
                    file_format: format.into(),
                    source_crs,
                },
            )
        }
        Command::LoadText {
            text,
            format,
            crs,
            name,
        } => {
            let file_format = geo_file_loader::FileFormat::from(format);
            if !file_format.is_plaintext() {
                return Err(format!(
                    "{} isn't a text format; use load_file or load_url",
                    file_format.display_name()
                ));
            }
            let source_crs = build_crs(world, &crs.unwrap_or(CrsSpec::Epsg(4326)))?;
            send(
                world,
                rgis_events::LoadFileMessage::FromBytes {
                    file_name: name.unwrap_or_else(|| "Inputted file".into()),
                    file_format,
                    bytes: bytes::Bytes::from(text),
                    source_crs,
                },
            )
        }
        Command::ChangeCrs { epsg, proj } => {
            let spec = match (epsg, proj) {
                (Some(code), None) => CrsSpec::Epsg(code),
                (None, Some(proj)) => CrsSpec::Proj(proj),
                _ => return Err("expected exactly one of `epsg` or `proj`".into()),
            };
            let new = build_crs(world, &spec)?;
            let old = world
                .get_resource::<rgis_crs::TargetCrs>()
                .ok_or("the target CRS isn't initialized yet")?
                .0
                .clone();
            send(world, rgis_events::ChangeCrsMessage { old, new })
        }
        Command::ToggleVisibility { layer, visible } => {
            let layer = resolve_layer(world, &layer)?;
            let currently_visible = world
                .get::<rgis_layers::LayerVisible>(layer.entity)
                .is_some_and(|visible| visible.0);
            if visible.is_some_and(|visible| visible == currently_visible) {
                return Ok(());
            }
            send(world, rgis_events::ToggleLayerVisibilityMessage(layer.id))
        }
        Command::MoveLayer { layer, direction } => {
            let layer = resolve_layer(world, &layer)?;
            send(
                world,
                rgis_events::MoveLayerMessage(layer.id, direction.into()),
            )
        }
        Command::DeleteLayer { layer } => {
            let layer = resolve_layer(world, &layer)?;
            send(world, rgis_events::DeleteLayerMessage(layer.id))
        }
        Command::DuplicateLayer { layer } => {
            let layer = resolve_layer(world, &layer)?;
            if !layer_data(world, &layer)?.is_vector() {
                return Err(format!(
                    "layer \"{}\" is a raster; only vector layers can be duplicated",
                    layer.name
                ));
            }
            send(world, rgis_events::DuplicateLayerMessage(layer.id))
        }
        Command::RenameLayer { layer, name } => {
            let layer = resolve_layer(world, &layer)?;
            send(world, rgis_ui_messages::RenameLayerMessage(layer.id, name))
        }
        Command::SetFillColor { layer, color } => {
            let layer = resolve_layer(world, &layer)?;
            send(
                world,
                rgis_ui_messages::UpdateLayerColorMessage::Fill(layer.id, color.0),
            )
        }
        Command::SetStrokeColor { layer, color } => {
            let layer = resolve_layer(world, &layer)?;
            send(
                world,
                rgis_ui_messages::UpdateLayerColorMessage::Stroke(layer.id, color.0),
            )
        }
        Command::SetPointSize { layer, size } => {
            let layer = resolve_layer(world, &layer)?;
            send(
                world,
                rgis_ui_messages::UpdateLayerPointSizeMessage(layer.id, size),
            )
        }
        Command::CenterOnLayer { layer } => {
            let layer = resolve_layer(world, &layer)?;
            if !layer_data(world, &layer)?.is_active() {
                return Err(format!(
                    "layer \"{}\" hasn't been projected yet; add a wait_idle step first",
                    layer.name
                ));
            }
            send(world, rgis_events::CenterCameraMessage(layer.id))
        }
        Command::Pan { dx, dy } => send(world, rgis_events::PanCameraMessage { x: dx, y: dy }),
        Command::Zoom { factor } => send(
            world,
            rgis_events::ZoomCameraMessage {
                amount: factor,
                coord: None,
            },
        ),
        Command::SelectFeature { layer, feature } => {
            let layer = resolve_layer(world, &layer)?;
            let fc = layer_data(world, &layer)?
                .unprojected_feature_collection()
                .ok_or_else(|| {
                    format!("layer \"{}\" is a raster and has no features", layer.name)
                })?;
            let selected = fc.features.get(feature).ok_or_else(|| {
                format!(
                    "layer \"{}\" has {} feature(s); index {feature} is out of range",
                    layer.name,
                    fc.features.len()
                )
            })?;
            let feature_id = selected.id;
            let properties = fc
                .properties
                .as_ref()
                .map(|properties| geo_features::properties_for_row(properties, feature));
            send(
                world,
                rgis_events::FeatureSelectedMessage(layer.id, feature_id),
            )?;
            // Clicking a feature also opens its properties window; do the same.
            send(
                world,
                rgis_ui_messages::RenderFeaturePropertiesMessage {
                    layer_id: layer.id,
                    properties,
                },
            )
        }
        Command::Deselect => {
            let properties_window_open = world
                .get_resource::<rgis_ui::OpenWindows>()
                .is_some_and(|windows| windows.contains("Layer Feature Properties"));
            if properties_window_open {
                rgis_ui::widget_registry::request_close("Layer Feature Properties");
            }
            send(world, rgis_events::FeaturesDeselectedMessage)
        }
        Command::OpenWindow { title } => rgis_ui::set_window_open(world, &title, true),
        Command::CloseWindow { title } => rgis_ui::set_window_open(world, &title, false),
        Command::SetAnimations { enabled } => {
            rgis_renderer::ANIMATIONS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        Command::WaitIdle { .. }
        | Command::Frames { .. }
        | Command::DumpState { .. }
        | Command::Screenshot { .. } => Err("internal error: not an immediate command".into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_file(
    world: &mut World,
    path: &str,
    format: Option<Format>,
    crs: Option<CrsSpec>,
    name: Option<String>,
) -> Result<(), String> {
    let format = format.or_else(|| Format::from_path(path)).ok_or_else(|| {
        format!(
            "can't tell the format of \"{path}\" from its extension; pass \"format\" \
             (geojson, shapefile, wkt, gpx, or geotiff)"
        )
    })?;
    let bytes = std::fs::read(path).map_err(|e| format!("couldn't read \"{path}\": {e}"))?;
    let name = name.unwrap_or_else(|| {
        std::path::Path::new(path).file_name().map_or_else(
            || path.to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    });
    let source_crs = build_crs(world, &crs.unwrap_or(CrsSpec::Epsg(4326)))?;
    send(
        world,
        rgis_events::LoadFileMessage::FromBytes {
            file_name: name,
            file_format: format.into(),
            bytes: bytes.into(),
            source_crs,
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn load_file(
    _world: &mut World,
    _path: &str,
    _format: Option<Format>,
    _crs: Option<CrsSpec>,
    _name: Option<String>,
) -> Result<(), String> {
    Err("there's no filesystem on the web; use load_url or load_text".into())
}

fn send<M: Message>(world: &mut World, message: M) -> Result<(), String> {
    world.write_message(message).map(|_| ()).ok_or_else(|| {
        format!(
            "{} isn't registered; is the plugin that handles it missing?",
            std::any::type_name::<M>()
        )
    })
}

fn build_crs(world: &World, spec: &CrsSpec) -> Result<rgis_primitives::Crs, String> {
    let geodesy_ctx = world
        .get_resource::<rgis_crs::GeodesyContext>()
        .ok_or("the CRS plugin isn't running")?;
    let mut geodesy_ctx = geodesy_ctx.write().map_err(|e| e.to_string())?;
    match spec {
        CrsSpec::Epsg(code) => rgis_crs::epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx, *code)
            .map(|op_handle| rgis_primitives::Crs {
                epsg_code: Some(*code),
                proj_string: None,
                op_handle,
            })
            .map_err(|e| e.to_string()),
        CrsSpec::Proj(proj) => rgis_crs::proj_string_to_geodesy_op_handle(&mut *geodesy_ctx, proj)
            .map(|op_handle| rgis_primitives::Crs {
                epsg_code: None,
                proj_string: Some(proj.clone()),
                op_handle,
            })
            .map_err(|e| format!("invalid PROJ string \"{proj}\": {e}")),
    }
}

struct ResolvedLayer {
    entity: Entity,
    id: rgis_primitives::LayerId,
    name: String,
}

fn resolve_layer(world: &World, layer: &LayerRef) -> Result<ResolvedLayer, String> {
    let order = world
        .get_resource::<rgis_layers::LayerOrder>()
        .ok_or("the layers plugin isn't running")?;
    let layers: Vec<ResolvedLayer> = order
        .iter_bottom_to_top()
        .filter_map(|entity| {
            Some(ResolvedLayer {
                entity,
                id: *world.get::<rgis_primitives::LayerId>(entity)?,
                name: world.get::<rgis_layers::LayerName>(entity)?.0.clone(),
            })
        })
        .collect();

    let found = match layer {
        LayerRef::Index(index) => {
            let len = i64::try_from(layers.len()).unwrap_or(i64::MAX);
            let index = if *index < 0 { len + index } else { *index };
            usize::try_from(index)
                .ok()
                .and_then(|index| layers.get(index))
        }
        LayerRef::Name(name) => {
            let mut matches = layers.iter().filter(|layer| &layer.name == name);
            let first = matches.next();
            if matches.next().is_some() {
                return Err(format!(
                    "several layers are named \"{name}\"; refer to one by index or {{\"id\": n}}"
                ));
            }
            first
        }
        LayerRef::Id(id) => layers.iter().find(|layer| layer.id.get() == *id),
    };

    match found {
        Some(found) => Ok(ResolvedLayer {
            entity: found.entity,
            id: found.id,
            name: found.name.clone(),
        }),
        None if layers.is_empty() => Err(format!(
            "no layer matches {layer} because there are no layers (if one is still loading, \
             add a wait_idle step first)"
        )),
        None => {
            let names: Vec<String> = layers
                .iter()
                .enumerate()
                .map(|(i, layer)| format!("{i}: \"{}\"", layer.name))
                .collect();
            Err(format!(
                "no layer matches {layer}; layers from bottom to top: [{}]",
                names.join(", ")
            ))
        }
    }
}

fn layer_data<'w>(
    world: &'w World,
    layer: &ResolvedLayer,
) -> Result<&'w rgis_layers::LayerData, String> {
    world
        .get::<rgis_layers::LayerData>(layer.entity)
        .ok_or_else(|| format!("layer \"{}\" has no data", layer.name))
}
