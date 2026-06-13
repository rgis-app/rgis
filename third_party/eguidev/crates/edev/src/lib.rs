//! Script-first MCP launcher and proxy for eguidev.

#[cfg(test)]
use std::path::PathBuf;
use std::{
    env,
    fmt::Display,
    future::Future,
    io::{self as std_io, IsTerminal},
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use eguidev::Anchor;
use eguidev_runtime::{
    ScriptErrorInfo, ScriptEvalOptions, ScriptEvalOutcome, ScriptEvalRequest, script_definitions,
    smoke::{ScriptRunRequest, SuiteResult, run_suite_with},
};
use instance_registry::InstanceRegistry;
use serde::Serialize;
use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::SyntaxSet,
    util::{LinesWithEndings, as_24_bit_terminal_escaped},
};
use tmcp::{
    Arguments, Error as McpError, Server, ServerCtx, ServerHandler,
    schema::{
        CallToolResult, ClientCapabilities, Cursor, Implementation, InitializeResult,
        ListToolsResult, TaskMetadata, Tool, ToolSchema,
    },
};
use tokio::{
    io::{self as tokio_io, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    runtime::Handle,
    sync::Mutex as AsyncMutex,
    task::{JoinHandle, block_in_place},
    time::sleep,
};

mod config;
mod instance_registry;

use config::{EdevCommand, FixtureConfig, LaunchConfig, McpConfig, SmokeConfig};

/// Tool names forwarded from edev to the app MCP server.
const PROXIED_TOOL_NAMES: &[&str] = &["script_eval"];
/// Timeout used for proxied request/response round-trips between edev and app MCP.
const APP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum attempts for restart when the app MCP transport closes mid-handshake.
const RESTART_MAX_ATTEMPTS: usize = 3;

/// Run the eguidev launcher on stdio.
pub async fn run() -> Result<(), EdevError> {
    match EdevCommand::from_env()? {
        EdevCommand::Help(help) => {
            print!("{help}");
            Ok(())
        }
        EdevCommand::Docs => {
            print!("{}", render_script_docs());
            Ok(())
        }
        EdevCommand::Mcp(config) => run_mcp(config).await,
        EdevCommand::Smoke(config) => run_smoke(config).await,
        EdevCommand::Fixture(config) => run_fixture(config).await,
    }
}

/// Run the long-lived `edev mcp` launcher server over stdio without starting the app eagerly.
async fn run_mcp(config: McpConfig) -> Result<(), EdevError> {
    let instance_registry = InstanceRegistry::register(&config.launch)?;
    let state = Arc::new(AsyncMutex::new(State::new(
        config.launch,
        instance_registry,
    )));
    let server_state = Arc::clone(&state);
    let server = Server::new(move || EdevServer {
        state: Arc::clone(&server_state),
    });
    let server_future = server.serve_stdio();
    tokio::pin!(server_future);
    let idle_future = wait_for_idle_shutdown(Arc::clone(&state), config.idle_shutdown_after);
    tokio::pin!(idle_future);
    let result = tokio::select! {
        result = &mut server_future => result.map_err(EdevError::Mcp),
        _ = shutdown_signal() => Ok(()),
        _ = &mut idle_future => Ok(()),
    };
    {
        let mut state_guard = state.lock().await;
        if let Err(error) = state_guard.shutdown().await {
            if result.is_ok() {
                return Err(error);
            }
            eprintln!("edev: shutdown failed: {error}");
        }
    }
    result
}

/// Run the checked-in smoke suite once and exit non-zero on any smoke failure.
async fn run_smoke(config: SmokeConfig) -> Result<(), EdevError> {
    let instance_registry = InstanceRegistry::register(&config.launch)?;
    let mut state = State::new(config.launch.clone(), instance_registry);
    let client = start_proxy_target(&mut state, "smoke runner could not reach the app").await?;

    let result = run_smoke_suite(client, &config).await;
    let shutdown_result = state.shutdown().await;
    match (result, shutdown_result) {
        (Ok(summary), Ok(())) => {
            for line in summary.render_lines(config.verbose_output) {
                println!("{line}");
            }
            if summary.success() {
                Ok(())
            } else {
                Err(EdevError::SmokeFailed("smoke suite failed".to_string()))
            }
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

/// Start the app, list or apply a fixture, then either exit or wait for ctrl-c.
async fn run_fixture(config: FixtureConfig) -> Result<(), EdevError> {
    let instance_registry = InstanceRegistry::register(&config.launch)?;
    let mut state = State::new(config.launch, instance_registry);
    let client = start_proxy_target(&mut state, "fixture command could not reach the app").await?;

    // Query registered fixtures.
    let fixtures =
        match eval_fixture_script(&client, "return fixtures()", "failed to query fixtures")
            .await
            .and_then(parse_fixture_list)
        {
            Ok(fixtures) => fixtures,
            Err(error) => {
                state.shutdown().await?;
                return Err(error);
            }
        };

    if fixtures.is_empty() {
        println!("No fixtures registered.");
        state.shutdown().await?;
        return Ok(());
    }

    let Some(name) = config.name else {
        // List-only mode.
        print_fixture_table(&fixtures);
        state.shutdown().await?;
        return Ok(());
    };

    // Validate the fixture name exists.
    if !fixtures.iter().any(|f| f.name == name) {
        eprintln!("error: unknown fixture \"{name}\"\n");
        print_fixture_table(&fixtures);
        state.shutdown().await?;
        return Err(EdevError::FixtureFailed(format!("unknown fixture: {name}")));
    }

    // Apply the fixture.
    let apply_script = format!("fixture_raw({:?})", name);
    if let Err(error) =
        eval_fixture_script(&client, &apply_script, "fixture application failed").await
    {
        state.shutdown().await?;
        return Err(error);
    }

    eprintln!("Fixture \"{name}\" applied. Press ctrl-c to stop.");
    shutdown_signal().await;
    state.shutdown().await?;
    Ok(())
}

/// Start the app and resolve the proxied client, shutting down on startup failures.
async fn start_proxy_target(
    state: &mut State,
    unavailable_message: &str,
) -> Result<Arc<AsyncMutex<tmcp::Client<()>>>, EdevError> {
    match state.restart().await? {
        LifecycleStartStatus::Running => {}
        LifecycleStartStatus::StartupFailed(output) => {
            state.shutdown().await?;
            return Err(EdevError::AppStart(output));
        }
    }

    match state.proxy_target() {
        Ok(client) => Ok(client),
        Err(error) => {
            let message = error.text().unwrap_or(unavailable_message).to_string();
            state.shutdown().await?;
            Err(EdevError::AppStart(message))
        }
    }
}

/// Call `script_eval` on the connected app and parse the outcome.
async fn call_script_eval(
    client: &Arc<AsyncMutex<tmcp::Client<()>>>,
    script: &str,
    timeout_ms: Option<u64>,
) -> Result<ScriptEvalOutcome, String> {
    let request = script_eval_request_value(ScriptEvalRequest {
        script: script.to_string(),
        timeout_ms: timeout_ms.or(Some(10_000)),
        options: None,
    });
    let result = {
        let mut client = client.lock().await;
        client
            .call_tool("script_eval".to_string(), request)
            .await
            .map_err(|error| error.to_string())?
    };
    parse_script_eval_outcome(&result)
}

/// Run a fixture-related script and convert script failures into fixture errors.
async fn eval_fixture_script(
    client: &Arc<AsyncMutex<tmcp::Client<()>>>,
    script: &str,
    fallback_message: &str,
) -> Result<ScriptEvalOutcome, EdevError> {
    match call_script_eval(client, script, None).await {
        Ok(outcome) if outcome.success => Ok(outcome),
        Ok(outcome) => Err(EdevError::FixtureFailed(script_eval_error_message(
            outcome.error.as_ref(),
            fallback_message,
        ))),
        Err(message) => Err(EdevError::FixtureFailed(message)),
    }
}

/// Decode the `fixtures()` result into the edev-side display shape.
fn parse_fixture_list(outcome: ScriptEvalOutcome) -> Result<Vec<FixtureInfo>, EdevError> {
    serde_json::from_value(
        outcome
            .value
            .ok_or_else(|| EdevError::FixtureFailed("fixtures() returned no value".to_string()))?,
    )
    .map_err(|error| EdevError::FixtureFailed(format!("failed to decode fixtures list: {error}")))
}

/// Prefer the runtime's script error text and fall back to a caller-provided message.
fn script_eval_error_message(error: Option<&ScriptErrorInfo>, fallback_message: &str) -> String {
    error
        .map(|error| error.message.as_str())
        .unwrap_or(fallback_message)
        .to_string()
}

/// Fixture metadata deserialized from a `fixtures()` script result.
#[derive(Debug, Clone, serde::Deserialize, Default)]
struct FixtureInfo {
    /// Fixture name used in `fixture("name")` calls.
    name: String,
    /// Human-readable description of the fixture baseline.
    #[serde(default)]
    description: String,
    /// Declarative readiness anchors reported by the app.
    #[serde(default)]
    anchors: Vec<Anchor>,
}

/// Print a formatted fixture table to stdout.
fn print_fixture_table(fixtures: &[FixtureInfo]) {
    let max_name = fixtures
        .iter()
        .map(|f| f.name.len())
        .max()
        .unwrap_or(0)
        .max(4);
    for f in fixtures {
        let readiness = if f.anchors.is_empty() {
            String::new()
        } else {
            format!(
                " [{} anchor{}]",
                f.anchors.len(),
                if f.anchors.len() == 1 { "" } else { "s" }
            )
        };
        if f.description.is_empty() {
            println!("  {}{}", f.name, readiness);
        } else {
            println!(
                "  {:width$}  {}{}",
                f.name,
                f.description,
                readiness,
                width = max_name
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// Errors returned by the edev launcher.
pub enum EdevError {
    /// Argument parsing error.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// MCP transport error.
    #[error("mcp error: {0}")]
    Mcp(#[from] McpError),
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std_io::Error),
    /// Application startup error.
    #[error("app start failed: {0}")]
    AppStart(String),
    /// Smoke suite failure.
    #[error("smoke failed: {0}")]
    SmokeFailed(String),
    /// Fixture operation failure.
    #[error("fixture failed: {0}")]
    FixtureFailed(String),
    /// Instance registry error.
    #[error("instance registry error: {0}")]
    InstanceRegistry(String),
}

/// Render the checked-in Luau API definitions, highlighting them for terminal output when possible.
fn render_script_docs() -> String {
    let definitions = script_definitions();
    if !std_io::stdout().is_terminal() || env::var_os("NO_COLOR").is_some() {
        return definitions.to_string();
    }
    highlight_script_definitions(definitions).unwrap_or_else(|| definitions.to_string())
}

/// Apply terminal syntax highlighting to checked-in Luau definitions when the output is a TTY.
fn highlight_script_definitions(definitions: &str) -> Option<String> {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let syntax = syntax_set
        .find_syntax_by_extension("luau")
        .or_else(|| syntax_set.find_syntax_by_extension("lua"))?;
    let theme_set = ThemeSet::load_defaults();
    let theme = [
        "base16-eighties.dark",
        "Solarized (dark)",
        "base16-ocean.dark",
        "Monokai Extended",
    ]
    .into_iter()
    .find_map(|name| theme_set.themes.get(name))
    .or_else(|| theme_set.themes.values().next())?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut rendered = String::new();
    for line in LinesWithEndings::from(definitions) {
        let ranges = highlighter.highlight_line(line, &syntax_set).ok()?;
        rendered.push_str(&as_24_bit_terminal_escaped(&ranges, false));
    }
    Some(rendered)
}

#[derive(Clone, Debug, Default)]
/// Lightweight logger for app and launcher messages.
struct LogState {
    /// Whether launcher lifecycle logs should be emitted.
    verbose: bool,
}

impl LogState {
    /// Build a logger that conditionally emits messages.
    fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    /// Record a single log line, preserving newlines when present.
    fn record_line(&self, line: &str) {
        if !self.verbose {
            return;
        }
        if line.ends_with('\n') || line.ends_with('\r') {
            eprint!("{line}");
        } else {
            eprintln!("{line}");
        }
    }
}

/// Mutable runtime state for the edev process manager.
struct State {
    /// Launcher configuration.
    config: LaunchConfig,
    /// Instance registry entry for this launcher.
    instance_registry: InstanceRegistry,
    /// Active app process, if running.
    app: Option<AppProcess>,
    /// Current lifecycle status.
    status: AppStatus,
    /// Timestamp of the last MCP interaction handled by this launcher.
    last_activity: Instant,
    /// Logger for launcher lifecycle messages.
    log_state: LogState,
}

/// Future returned by the app spawn helper.
type SpawnFuture<'a> = Pin<Box<dyn Future<Output = Result<AppProcess, AppStartError>> + Send + 'a>>;

/// Future returned by MCP server handlers.
impl State {
    /// Create a new runtime state from the provided configuration.
    fn new(config: LaunchConfig, instance_registry: InstanceRegistry) -> Self {
        let log_state = LogState::new(config.verbose);
        Self {
            config,
            instance_registry,
            app: None,
            status: AppStatus::NotRunning,
            last_activity: Instant::now(),
            log_state,
        }
    }

    /// Record an internal launcher log line into the shared log buffer.
    fn log_edev(&self, line: impl AsRef<str>) {
        let line = line.as_ref();
        self.log_state.record_line(&format!("edev: {line}"));
    }

    /// Mark launcher activity from user-visible tool execution.
    fn mark_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Stop the managed app process and reset launcher state without unregistering the launcher.
    async fn stop_app(&mut self) -> Result<StopStatus, EdevError> {
        if let Some(app) = self.app.take() {
            app.shutdown().await;
            self.instance_registry.clear_app()?;
            self.status = AppStatus::NotRunning;
            return Ok(StopStatus::Stopped);
        }
        if matches!(self.status, AppStatus::StartupFailed { .. }) {
            self.status = AppStatus::NotRunning;
            self.instance_registry.clear_app()?;
            return Ok(StopStatus::Stopped);
        }
        self.status = AppStatus::NotRunning;
        self.instance_registry.clear_app()?;
        Ok(StopStatus::AlreadyStopped)
    }

    /// Stop the app process and unregister the launcher.
    async fn shutdown(&mut self) -> Result<(), EdevError> {
        let _stopped = self.stop_app().await?;
        self.instance_registry.unregister()?;
        Ok(())
    }

    /// Start the app process unless it is already running.
    async fn start(&mut self) -> Result<StartStatus, EdevError> {
        match &self.status {
            AppStatus::Running => return Ok(StartStatus::AlreadyRunning),
            AppStatus::StartupFailed { output } => {
                return Ok(StartStatus::RestartRequired(output.clone()));
            }
            AppStatus::Starting => return Ok(StartStatus::AppStarting),
            AppStatus::NotRunning => {}
        }
        self.start_with(|config, log_state| Box::pin(spawn_app(config, log_state)))
            .await
    }

    /// Restart the app process using the default spawn behavior.
    async fn restart(&mut self) -> Result<LifecycleStartStatus, EdevError> {
        let mut attempt = 1;
        loop {
            let result = self
                .restart_with(|config, log_state| Box::pin(spawn_app(config, log_state)))
                .await;
            if restart_result_is_transport_closed(&result) && attempt < RESTART_MAX_ATTEMPTS {
                self.log_edev(format!(
                    "restart attempt {attempt} failed with closed transport; retrying"
                ));
                attempt += 1;
                continue;
            }
            return result;
        }
    }

    /// Resolve the current proxied app client, or return a lifecycle-specific tool error.
    fn proxy_target(&self) -> Result<Arc<AsyncMutex<tmcp::Client<()>>>, CallToolResult> {
        match &self.status {
            AppStatus::Running => {
                let Some(app) = &self.app else {
                    self.log_edev("proxy call failed: app not running");
                    return Err(tool_error(
                        ErrorKind::AppNotRunning,
                        "App process not running. Call start.",
                    ));
                };
                Ok(Arc::clone(&app.client))
            }
            AppStatus::Starting => {
                self.log_edev("proxy call failed: app starting");
                Err(tool_error(
                    ErrorKind::AppStarting,
                    "App is starting. Try again shortly.",
                ))
            }
            AppStatus::StartupFailed { output } => Err(tool_error_with_data(
                ErrorKind::RestartRequired,
                "App startup failed. Fix the issue and call restart.",
                &serde_json::json!({ "startup_output": output }),
            )),
            AppStatus::NotRunning => {
                self.log_edev("proxy call failed: app not running");
                Err(tool_error(
                    ErrorKind::AppNotRunning,
                    "App is not running. Call start.",
                ))
            }
        }
    }

    /// Proxy a tool call to the app if the app is running.
    #[cfg(test)]
    async fn proxy_call(&self, name: &str, arguments: Option<Arguments>) -> CallToolResult {
        let client = match self.proxy_target() {
            Ok(client) => client,
            Err(error) => return error,
        };
        call_proxy_tool(client, self.log_state.clone(), name.to_string(), arguments).await
    }

    /// Start the app process using a caller-provided spawn routine.
    async fn start_with<F>(&mut self, spawn: F) -> Result<StartStatus, EdevError>
    where
        F: for<'a> FnOnce(&'a LaunchConfig, LogState) -> SpawnFuture<'a>,
    {
        let status = self
            .spawn_with(LifecycleAction::Start, false, spawn)
            .await?;
        Ok(match status {
            LifecycleStartStatus::Running => StartStatus::Started,
            LifecycleStartStatus::StartupFailed(output) => StartStatus::StartupFailed(output),
        })
    }

    /// Restart the app process using a caller-provided spawn routine.
    async fn restart_with<F>(&mut self, spawn: F) -> Result<LifecycleStartStatus, EdevError>
    where
        F: for<'a> FnOnce(&'a LaunchConfig, LogState) -> SpawnFuture<'a>,
    {
        self.spawn_with(LifecycleAction::Restart, true, spawn).await
    }

    /// Spawn and attach an app process for either a start or restart transition.
    async fn spawn_with<F>(
        &mut self,
        action: LifecycleAction,
        replace_existing: bool,
        spawn: F,
    ) -> Result<LifecycleStartStatus, EdevError>
    where
        F: for<'a> FnOnce(&'a LaunchConfig, LogState) -> SpawnFuture<'a>,
    {
        self.status = AppStatus::Starting;
        if replace_existing {
            if let Some(app) = self.app.take() {
                app.shutdown().await;
            }
            self.instance_registry.clear_app()?;
        }
        self.log_edev(format!("{} requested", action.as_str()));
        match spawn(&self.config, self.log_state.clone()).await {
            Ok(app) => {
                if let Err(error) = self
                    .instance_registry
                    .set_app_process_group_id(app.process_group_id)
                {
                    app.shutdown().await;
                    self.status = AppStatus::NotRunning;
                    return Err(error);
                }
                if let Err(output) = probe_script_eval_ready(&app.client).await {
                    app.shutdown().await;
                    self.status = AppStatus::StartupFailed {
                        output: output.clone(),
                    };
                    self.instance_registry.clear_app()?;
                    self.log_edev(format!("{} failed during app startup", action.as_str()));
                    return Ok(LifecycleStartStatus::StartupFailed(output));
                }
                self.status = AppStatus::Running;
                self.app = Some(app);
                self.log_edev(format!("{} completed", action.as_str()));
                Ok(LifecycleStartStatus::Running)
            }
            Err(AppStartError::StartupFailed(output)) => {
                self.status = AppStatus::StartupFailed {
                    output: output.clone(),
                };
                self.log_edev(format!("{} failed during app startup", action.as_str()));
                Ok(LifecycleStartStatus::StartupFailed(output))
            }
            Err(AppStartError::Other(message)) => {
                self.status = AppStatus::NotRunning;
                self.log_edev(format!("{} failed: {message}", action.as_str()));
                Err(EdevError::AppStart(message))
            }
        }
    }

    /// Build the static host-side tool list.
    fn tools_list(&self) -> Vec<Tool> {
        vec![
            start_tool(),
            stop_tool(),
            restart_tool(),
            status_tool(),
            script_eval_tool(),
            script_api_tool(),
        ]
    }

    /// Build a structured status snapshot of the managed app lifecycle.
    fn status_report(&self) -> StatusReport {
        let (app_present, process_group_id) = self
            .app
            .as_ref()
            .map(|app| (true, app.process_group_id))
            .unwrap_or((false, None));
        let startup_output = match &self.status {
            AppStatus::StartupFailed { output } => Some(output.clone()),
            _ => None,
        };
        StatusReport {
            state: self.status.as_str(),
            app_present,
            process_group_id,
            startup_output,
        }
    }
}

/// Running app process and its connected MCP client.
struct AppProcess {
    /// Child process handle for `cargo run`.
    child: Option<Child>,
    /// Process group id for the running app process tree.
    process_group_id: Option<i32>,
    /// Connected MCP client speaking to the app over stdio.
    client: Arc<AsyncMutex<tmcp::Client<()>>>,
    /// Background task streaming stderr.
    stderr_task: Option<JoinHandle<()>>,
    /// Captured stderr output, primarily for startup errors.
    stderr_buffer: Arc<Mutex<Vec<u8>>>,
    /// Logger for process lifecycle messages.
    log_state: LogState,
}

impl AppProcess {
    /// Trigger immediate app termination without waiting for child process exit.
    fn start_termination(&mut self) {
        terminate_process_group(self.process_group_id.take(), &self.log_state);
        if let Some(child) = self.child.as_mut() {
            let _start_kill_result = child.start_kill();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }

    /// Terminate the app process and tear down stderr streaming.
    async fn shutdown(mut self) {
        self.start_termination();
        if let Some(mut child) = self.child.take() {
            let _wait_result = child.wait().await;
        }
        let _drain_result = drain_stderr(&self.stderr_buffer).await;
    }
}

impl Drop for AppProcess {
    fn drop(&mut self) {
        self.start_termination();
    }
}

#[derive(Debug)]
/// Current lifecycle state for the managed app.
enum AppStatus {
    /// The app is starting and MCP handshake has not completed.
    Starting,
    /// The app is running and MCP is connected.
    Running,
    /// The app is not running.
    NotRunning,
    /// The last startup attempt failed before the app became ready.
    StartupFailed {
        /// Captured startup output.
        output: String,
    },
}

impl AppStatus {
    /// Stable string identifier for lifecycle serialization.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::NotRunning => "not_running",
            Self::StartupFailed { .. } => "startup_failed",
        }
    }
}

#[derive(Debug)]
/// Errors emitted while starting the app process.
enum AppStartError {
    /// App startup failed before the MCP handshake completed.
    StartupFailed(String),
    /// Other startup failure.
    Other(String),
}

#[derive(Debug)]
/// Outcome of a completed start or restart path.
enum LifecycleStartStatus {
    /// Startup completed successfully.
    Running,
    /// Startup failed before the app became ready.
    StartupFailed(String),
}

#[derive(Debug)]
/// Outcome of a start attempt.
enum StartStatus {
    /// Start completed successfully.
    Started,
    /// The app was already running.
    AlreadyRunning,
    /// Another lifecycle action is currently starting the app.
    AppStarting,
    /// The previous startup failed and the caller must use restart.
    RestartRequired(String),
    /// Startup failed before the app became ready.
    StartupFailed(String),
}

#[derive(Debug)]
/// Outcome of a stop attempt.
enum StopStatus {
    /// A running app was stopped or a failed startup state was cleared.
    Stopped,
    /// No app was running.
    AlreadyStopped,
}

#[derive(Debug, Clone, Copy)]
/// Host-side lifecycle operation.
enum LifecycleAction {
    /// Start the app without replacing a running process.
    Start,
    /// Replace any existing app process with a fresh one.
    Restart,
}

impl LifecycleAction {
    /// Lowercase label for logging.
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Restart => "restart",
        }
    }
}

/// Spawn the app via `cargo run` and connect an MCP client over stdio.
async fn spawn_app(
    config: &LaunchConfig,
    log_state: LogState,
) -> Result<AppProcess, AppStartError> {
    log_state.record_line("edev: spawning app");
    let mut command = config.app_command();
    command.kill_on_drop(true);
    configure_child_process(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        log_state.record_line(&format!("edev: spawn failed: {error}"));
        AppStartError::Other(format!("Failed to spawn cargo run: {error}"))
    })?;
    log_state.record_line("edev: app process spawned");
    let process_group_id = process_group_id(&child);
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate_process_group(process_group_id, &log_state);
        AppStartError::Other("Failed to capture app stdout".to_string())
    })?;
    let stdin = child.stdin.take().ok_or_else(|| {
        terminate_process_group(process_group_id, &log_state);
        AppStartError::Other("Failed to capture app stdin".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_process_group(process_group_id, &log_state);
        AppStartError::Other("Failed to capture app stderr".to_string())
    })?;

    let stderr_buffer = Arc::new(Mutex::new(Vec::new()));
    let stderr_buffer_clone = Arc::clone(&stderr_buffer);
    let stderr_log_state = log_state.clone();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = match reader.read_line(&mut line).await {
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            if bytes == 0 {
                break;
            }
            if let Ok(mut buffer) = stderr_buffer_clone.lock() {
                buffer.extend_from_slice(line.as_bytes());
            }
            stderr_log_state.record_line(&line);
            let _write_result = tokio_io::stderr().write_all(line.as_bytes()).await;
        }
    });

    let mut client = tmcp::Client::new("edev", env!("CARGO_PKG_VERSION"))
        .with_request_timeout(APP_REQUEST_TIMEOUT);
    if let Err(error) = client.connect_stream(stdout, stdin).await {
        return Err(fail_startup_handshake(
            &mut child,
            process_group_id,
            &stderr_buffer,
            &log_state,
            "connect",
            &error,
        )
        .await);
    }
    if let Err(error) = client.init().await {
        return Err(fail_startup_handshake(
            &mut child,
            process_group_id,
            &stderr_buffer,
            &log_state,
            "init",
            &error,
        )
        .await);
    }
    log_state.record_line("edev: app MCP connected");

    Ok(AppProcess {
        child: Some(child),
        process_group_id,
        client: Arc::new(AsyncMutex::new(client)),
        stderr_task: Some(stderr_task),
        stderr_buffer,
        log_state,
    })
}

/// Finalize app startup after an MCP handshake failure.
async fn fail_startup_handshake(
    child: &mut Child,
    process_group_id: Option<i32>,
    stderr_buffer: &Arc<Mutex<Vec<u8>>>,
    log_state: &LogState,
    stage: &str,
    error: &McpError,
) -> AppStartError {
    log_state.record_line(&format!("edev: app {stage} failed: {error}"));
    terminate_process_group(process_group_id, log_state);
    let output = drain_stderr(stderr_buffer).await;
    let _wait_result = child.wait().await;
    AppStartError::StartupFailed(format_startup_output(error, &output))
}

/// Combine the handshake error with any captured stderr for diagnostics.
fn format_startup_output(error: impl Display, output: &str) -> String {
    let output = output.trim_end();
    if output.is_empty() {
        error.to_string()
    } else {
        format!("{error}\n{output}")
    }
}

#[cfg(unix)]
/// Wait for shutdown signals to terminate the app process cleanly.
async fn shutdown_signal() {
    use tokio::{
        signal,
        signal::unix::{SignalKind, signal as unix_signal},
    };

    let mut term = unix_signal(SignalKind::terminate()).ok();
    if let Some(term) = term.as_mut() {
        tokio::select! {
            _ = signal::ctrl_c() => {},
            _ = term.recv() => {},
        }
    } else {
        let _ctrl_c = signal::ctrl_c().await;
    }
}

#[cfg(not(unix))]
/// Wait for shutdown signals to terminate the app process cleanly.
async fn shutdown_signal() {
    let _ctrl_c = tokio::signal::ctrl_c().await;
}

/// Wait until launcher inactivity exceeds the configured timeout.
async fn wait_for_idle_shutdown(state: Arc<AsyncMutex<State>>, idle_after: Duration) {
    loop {
        let (sleep_for, should_shutdown) = {
            let state = state.lock().await;
            let elapsed = state.last_activity.elapsed();
            if elapsed >= idle_after {
                state.log_edev(format!("idle for {}s; shutting down", idle_after.as_secs()));
                (Duration::from_secs(0), true)
            } else {
                (idle_after - elapsed, false)
            }
        };
        if should_shutdown {
            return;
        }
        sleep(sleep_for).await;
    }
}

/// Drain buffered stderr output into a string.
async fn drain_stderr(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    let mut output = String::new();
    if let Ok(mut data) = buffer.lock() {
        output = String::from_utf8_lossy(&data).to_string();
        data.clear();
    }
    output
}

/// Fetch the process group id for a spawned child process.
fn process_group_id(child: &Child) -> Option<i32> {
    child.id().and_then(|id| i32::try_from(id).ok())
}

/// Configure child process behavior for cleanup and termination.
fn configure_child_process(command: &mut Command) {
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    #[cfg(all(unix, target_os = "linux"))]
    {
        let parent_pid = libc::pid_t::try_from(std::process::id()).ok();
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std_io::Error::last_os_error());
                }
                if let Some(parent_pid) = parent_pid
                    && libc::getppid() != parent_pid
                {
                    return Err(std_io::Error::other("parent process changed before exec"));
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// Terminate the process group for a running app process tree.
fn terminate_process_group(process_group_id: Option<i32>, log_state: &LogState) {
    #[cfg(unix)]
    {
        if let Some(pgid) = process_group_id {
            let result = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            if result != 0 {
                let error = std_io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    log_state.record_line(&format!(
                        "edev: failed to kill process group {pgid}: {error}"
                    ));
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (process_group_id, log_state);
    }
}

/// MCP server implementation that proxies tool calls to the app.
struct EdevServer {
    /// Shared runtime state for proxying and host-side lifecycle control.
    state: Arc<AsyncMutex<State>>,
}

/// Check whether a tool name should be proxied to the app.
fn is_proxied_tool(name: &str) -> bool {
    PROXIED_TOOL_NAMES.iter().copied().any(|tool| tool == name)
}

#[async_trait]
impl ServerHandler for EdevServer {
    async fn initialize(
        &self,
        _context: &ServerCtx,
        _protocol_version: String,
        _capabilities: ClientCapabilities,
        _client_info: Implementation,
    ) -> tmcp::Result<InitializeResult> {
        let version = env!("CARGO_PKG_VERSION").to_string();
        Ok(InitializeResult::new("edev")
            .with_version(version)
            .with_tools(false))
    }

    async fn list_tools(
        &self,
        _context: &ServerCtx,
        _cursor: Option<Cursor>,
    ) -> tmcp::Result<ListToolsResult> {
        let state = Arc::clone(&self.state);
        let state = state.lock().await;
        Ok(ListToolsResult::new().with_tools(state.tools_list()))
    }

    async fn call_tool(
        &self,
        _context: &ServerCtx,
        name: String,
        arguments: Option<Arguments>,
        _task: Option<TaskMetadata>,
    ) -> tmcp::Result<CallToolResult> {
        let state = Arc::clone(&self.state);
        {
            let mut state_guard = state.lock().await;
            state_guard.mark_activity();
        }
        if !is_host_tool(&name) && !is_proxied_tool(&name) {
            return Err(McpError::ToolNotFound(name));
        }
        match name.as_str() {
            "start" => {
                let mut state = state.lock().await;
                let start = Instant::now();
                match state.start().await {
                    Ok(StartStatus::Started) => Ok(lifecycle_success("started", start.elapsed())),
                    Ok(StartStatus::AlreadyRunning) => {
                        Ok(lifecycle_success("already_running", start.elapsed()))
                    }
                    Ok(StartStatus::AppStarting) => Ok(tool_error(
                        ErrorKind::AppStarting,
                        "App is starting. Try again shortly.",
                    )),
                    Ok(StartStatus::RestartRequired(output)) => Ok(tool_error_with_data(
                        ErrorKind::RestartRequired,
                        "App startup previously failed. Fix the issue and call restart.",
                        &serde_json::json!({ "startup_output": output }),
                    )),
                    Ok(StartStatus::StartupFailed(output)) => Ok(lifecycle_startup_failed(
                        "App startup failed. Fix the issue and call restart again.",
                        &output,
                        start.elapsed(),
                    )),
                    Err(error) => Ok(lifecycle_failed(
                        ErrorKind::StartFailed,
                        format!("Start failed: {error}"),
                        start.elapsed(),
                    )),
                }
            }
            "stop" => {
                let mut state = state.lock().await;
                let start = Instant::now();
                match state.stop_app().await {
                    Ok(StopStatus::Stopped) => Ok(lifecycle_success("stopped", start.elapsed())),
                    Ok(StopStatus::AlreadyStopped) => {
                        Ok(lifecycle_success("already_stopped", start.elapsed()))
                    }
                    Err(error) => Ok(lifecycle_failed(
                        ErrorKind::StopFailed,
                        format!("Stop failed: {error}"),
                        start.elapsed(),
                    )),
                }
            }
            "restart" => {
                let mut state = state.lock().await;
                let start = Instant::now();
                match state.restart().await {
                    Ok(LifecycleStartStatus::Running) => {
                        Ok(lifecycle_success("completed", start.elapsed()))
                    }
                    Ok(LifecycleStartStatus::StartupFailed(output)) => {
                        Ok(lifecycle_startup_failed(
                            "App startup failed. Fix the issue and call restart again.",
                            &output,
                            start.elapsed(),
                        ))
                    }
                    Err(error) => Ok(lifecycle_failed(
                        ErrorKind::RestartFailed,
                        format!("Restart failed: {error}"),
                        start.elapsed(),
                    )),
                }
            }
            "status" => {
                let state = state.lock().await;
                Ok(CallToolResult::new().with_structured_content(
                    serde_json::to_value(state.status_report()).expect("status report"),
                ))
            }
            "script_api" => Ok(CallToolResult::new().with_text_content(script_definitions())),
            _ => {
                let (client, log_state) = {
                    let state = state.lock().await;
                    match state.proxy_target() {
                        Ok(client) => (client, state.log_state.clone()),
                        Err(error) => return Ok(error),
                    }
                };
                Ok(call_proxy_tool(client, log_state, name, arguments).await)
            }
        }
    }
}

/// Proxy a tool call through the connected app client.
async fn call_proxy_tool(
    client: Arc<AsyncMutex<tmcp::Client<()>>>,
    log_state: LogState,
    name: String,
    arguments: Option<Arguments>,
) -> CallToolResult {
    let mut client = client.lock().await;
    match client.call_tool(name, arguments.unwrap_or_default()).await {
        Ok(result) => normalize_tool_result(result),
        Err(error) => {
            log_state.record_line(&format!("edev: proxy call failed: {error}"));
            tool_error(ErrorKind::ProxyFailed, error.to_string())
        }
    }
}

/// Execute the resolved smoke suite by calling `script_eval` for each discovered script.
async fn run_smoke_suite(
    client: Arc<AsyncMutex<tmcp::Client<()>>>,
    config: &SmokeConfig,
) -> Result<SuiteResult, EdevError> {
    Ok(run_suite_with(
        &config.suite,
        |request: ScriptRunRequest| {
            let payload = script_eval_request_value(ScriptEvalRequest {
                script: request.source,
                timeout_ms: request.timeout_ms,
                options: Some(ScriptEvalOptions {
                    source_name: Some(request.path),
                    args: request.args,
                }),
            });
            let result = block_in_place(|| {
                Handle::current().block_on(async {
                    let mut client = client.lock().await;
                    client
                        .call_tool("script_eval".to_string(), payload)
                        .await
                        .map_err(|error| error.to_string())
                })
            })?;
            parse_script_eval_outcome(&result)
        },
    ))
}

/// Decode a proxied `script_eval` tool result back into the checked-in outcome shape.
fn parse_script_eval_outcome(result: &CallToolResult) -> Result<ScriptEvalOutcome, String> {
    let payload = if let Some(structured) = &result.structured_content {
        structured.clone()
    } else {
        let Some(text) = result.text() else {
            return Err("script_eval response was missing JSON content".to_string());
        };
        serde_json::from_str(text)
            .map_err(|error| format!("failed to parse script_eval response: {error}"))?
    };
    serde_json::from_value(payload)
        .map_err(|error| format!("failed to decode script_eval outcome: {error}"))
}

/// Serialize a `script_eval` request into an MCP arguments object.
fn script_eval_request_value(request: ScriptEvalRequest) -> Arguments {
    Arguments::from_struct(request).expect("script eval request should serialize")
}

/// Probe the app's `script_eval` tool to confirm the script runtime is ready.
async fn probe_script_eval_ready(client: &Arc<AsyncMutex<tmcp::Client<()>>>) -> Result<(), String> {
    let request = script_eval_request_value(ScriptEvalRequest {
        script: "return true".to_string(),
        timeout_ms: Some(1_000),
        options: None,
    });
    let result = {
        let mut client = client.lock().await;
        client
            .call_tool("script_eval".to_string(), request)
            .await
            .map_err(|error| error.to_string())?
    };
    let outcome = parse_script_eval_outcome(&result)?;
    if outcome.success {
        Ok(())
    } else {
        let message = outcome
            .error
            .as_ref()
            .map(|error| error.message.as_str())
            .unwrap_or("script_eval readiness probe failed");
        Err(message.to_string())
    }
}

/// Return true when a tool is handled directly by the launcher.
fn is_host_tool(name: &str) -> bool {
    matches!(name, "start" | "stop" | "restart" | "status" | "script_api")
}

/// Tool definition for starting the app process.
fn start_tool() -> Tool {
    Tool::new("start", ToolSchema::empty())
        .with_description("Start the underlying app process if it is not already running.")
}

/// Tool definition for stopping the app process.
fn stop_tool() -> Tool {
    Tool::new("stop", ToolSchema::empty())
        .with_description("Stop the underlying app process if it is running.")
}

/// Tool definition for restarting the app process.
fn restart_tool() -> Tool {
    Tool::new("restart", ToolSchema::empty())
        .with_description("Restart the underlying app process.")
}

/// Tool definition for reporting launcher lifecycle state.
fn status_tool() -> Tool {
    Tool::new("status", ToolSchema::empty())
        .with_description("Report the current launcher and app lifecycle state.")
}

/// Tool definition for proxying `script_eval` through the launcher.
fn script_eval_tool() -> Tool {
    Tool::new(
        "script_eval",
        ToolSchema::from_json_schema::<ScriptEvalRequest>(),
    )
    .with_description(
        "Evaluate a Luau script with DevMCP helpers. Scripts are assumed to be strict.",
    )
}

/// Tool definition for exposing the checked-in Luau API definitions.
fn script_api_tool() -> Tool {
    Tool::new("script_api", ToolSchema::empty())
        .with_description("Return the checked-in Luau definitions for the full scripting API.")
}

#[allow(clippy::missing_docs_in_private_items)]
#[derive(Debug, Clone, Serialize)]
struct LifecycleReport {
    status: &'static str,
    elapsed_ms: u64,
}

#[allow(clippy::missing_docs_in_private_items)]
#[derive(Debug, Clone, Serialize)]
struct StatusReport {
    state: &'static str,
    app_present: bool,
    process_group_id: Option<i32>,
    startup_output: Option<String>,
}

/// Build a successful lifecycle tool result.
fn lifecycle_success(status: &'static str, elapsed: Duration) -> CallToolResult {
    CallToolResult::new().with_structured_content(serde_json::json!({
        "ok": true,
        "report": LifecycleReport {
            status,
            elapsed_ms: elapsed.as_millis() as u64,
        },
    }))
}

/// Build a lifecycle tool result for startup failure.
fn lifecycle_startup_failed(
    message: &'static str,
    output: &str,
    elapsed: Duration,
) -> CallToolResult {
    tool_error_with_data(
        ErrorKind::StartupFailed,
        message,
        &serde_json::json!({
            "startup_output": output,
            "report": LifecycleReport {
                status: "startup_failed",
                elapsed_ms: elapsed.as_millis() as u64,
            },
        }),
    )
}

/// Build a lifecycle tool result for non-startup failures.
fn lifecycle_failed(kind: ErrorKind, message: String, elapsed: Duration) -> CallToolResult {
    tool_error_with_data(
        kind,
        message,
        &serde_json::json!({
            "report": LifecycleReport {
                status: "failed",
                elapsed_ms: elapsed.as_millis() as u64,
            },
        }),
    )
}

#[derive(Debug, Clone, Copy)]
/// Error kinds returned in structured tool failures.
enum ErrorKind {
    /// App is starting and cannot accept tool calls.
    AppStarting,
    /// App process is not running.
    AppNotRunning,
    /// Restart is required to recover.
    RestartRequired,
    /// Start failed for a non-startup reason.
    StartFailed,
    /// Stop failed.
    StopFailed,
    /// Restart failed for a non-startup reason.
    RestartFailed,
    /// Startup failed before the app became ready.
    StartupFailed,
    /// Proxy call to the app failed.
    ProxyFailed,
}

/// Build a structured tool error result.
fn tool_error(kind: ErrorKind, message: impl Into<String>) -> CallToolResult {
    build_tool_error(kind, message.into(), None)
}

/// Build a structured tool error result with extra data.
fn tool_error_with_data(
    kind: ErrorKind,
    message: impl Into<String>,
    data: &serde_json::Value,
) -> CallToolResult {
    build_tool_error(kind, message.into(), Some(data))
}

/// Build a structured tool error result with optional data payload.
fn build_tool_error(
    kind: ErrorKind,
    message: String,
    data: Option<&serde_json::Value>,
) -> CallToolResult {
    let mut error = serde_json::json!({
        "kind": kind.as_str(),
        "message": &message,
    });
    if let Some(data) = data {
        error
            .as_object_mut()
            .expect("tool error payload should be an object")
            .insert("data".to_string(), data.clone());
    }
    CallToolResult::new()
        .mark_as_error()
        .with_text_content(message)
        .with_structured_content(serde_json::json!({ "error": error }))
}

/// Ensure tool error results include readable text content.
fn normalize_tool_result(result: CallToolResult) -> CallToolResult {
    if !matches!(result.is_error, Some(true)) || !result.content.is_empty() {
        return result;
    }
    let message = result
        .structured_content
        .as_ref()
        .and_then(extract_error_message)
        .map(str::to_string);
    match message {
        Some(message) => result.with_text_content(message),
        None => result,
    }
}

/// Extract an error message from a structured tool error payload.
fn extract_error_message(structured: &serde_json::Value) -> Option<&str> {
    structured
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .or_else(|| {
            structured
                .get("message")
                .and_then(|message| message.as_str())
        })
}

impl ErrorKind {
    /// Stable string identifier for error serialization.
    fn as_str(self) -> &'static str {
        match self {
            Self::AppStarting => "app_starting",
            Self::AppNotRunning => "app_not_running",
            Self::RestartRequired => "restart_required",
            Self::StartFailed => "start_failed",
            Self::StopFailed => "stop_failed",
            Self::RestartFailed => "restart_failed",
            Self::StartupFailed => "startup_failed",
            Self::ProxyFailed => "proxy_failed",
        }
    }
}

/// Return true when a restart result indicates transient transport closure.
fn restart_result_is_transport_closed(result: &Result<LifecycleStartStatus, EdevError>) -> bool {
    matches!(
        result,
        Err(EdevError::Mcp(
            McpError::TransportDisconnected | McpError::ConnectionClosed
        ))
    ) || matches!(
        result,
        Err(EdevError::Mcp(McpError::Transport(message)))
            if transport_message_is_closed(message)
    ) || matches!(
        result,
        Ok(LifecycleStartStatus::StartupFailed(output)) if transport_message_is_closed(output)
    )
}

/// Return true when an MCP transport error string indicates closed transport.
fn transport_message_is_closed(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    ["closed", "disconnect", "broken pipe", "eof"]
        .iter()
        .any(|fragment| message.contains(fragment))
}

#[cfg(test)]
fn test_tempdir() -> tempfile::TempDir {
    use std::fs;

    fs::create_dir_all("tmp").expect("create tmp");
    tempfile::Builder::new()
        .prefix("edev-test-")
        .tempdir_in("tmp")
        .expect("tempdir")
}

#[cfg(test)]
fn test_config(cwd: PathBuf) -> LaunchConfig {
    LaunchConfig {
        cwd,
        command: vec![
            "cargo".to_string(),
            "run".to_string(),
            "--dev-mcp".to_string(),
        ],
        env: Default::default(),
        verbose: false,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use eguidev_runtime::ScriptErrorInfo;
    use tempfile::TempDir;
    use tmcp::{
        Client, Server, ServerCtx, ServerHandle, ServerHandler,
        schema::{
            CallToolResult, ContentBlock, Cursor, InitializeResult, ListToolsResult, TaskMetadata,
        },
        testutils::{TestServerContext, make_duplex_pair},
    };
    use tokio::time::timeout;

    use super::*;

    fn make_state(tempdir: &TempDir) -> State {
        let config = test_config(tempdir.path().to_path_buf());
        let registry = InstanceRegistry::register(&config).expect("instance registry");
        State::new(config, registry)
    }

    fn successful_script_eval_result() -> CallToolResult {
        CallToolResult::new()
            .with_json_text(serde_json::json!({
                "success": true,
                "value": true,
                "logs": [],
                "assertions": [],
                "timing": {
                    "compile_ms": 0,
                    "exec_ms": 0,
                    "total_ms": 0
                }
            }))
            .expect("script eval json")
    }

    fn successful_outcome(value: &serde_json::Value) -> ScriptEvalOutcome {
        parse_script_eval_outcome(
            &CallToolResult::new()
                .with_json_text(serde_json::json!({
                    "success": true,
                    "value": value,
                    "logs": [],
                    "assertions": [],
                    "timing": {
                        "compile_ms": 0,
                        "exec_ms": 0,
                        "total_ms": 0
                    }
                }))
                .expect("script eval json"),
        )
        .expect("script eval outcome")
    }

    struct MockServer;

    #[async_trait]
    impl ServerHandler for MockServer {
        async fn initialize(
            &self,
            _context: &ServerCtx,
            _protocol_version: String,
            _capabilities: ClientCapabilities,
            _client_info: Implementation,
        ) -> tmcp::Result<InitializeResult> {
            Ok(InitializeResult::new("mock"))
        }

        async fn list_tools(
            &self,
            _context: &ServerCtx,
            _cursor: Option<Cursor>,
        ) -> tmcp::Result<ListToolsResult> {
            Ok(ListToolsResult::new())
        }

        async fn call_tool(
            &self,
            _context: &ServerCtx,
            name: String,
            _arguments: Option<Arguments>,
            _task: Option<TaskMetadata>,
        ) -> tmcp::Result<CallToolResult> {
            if name == "script_eval" {
                Ok(successful_script_eval_result())
            } else {
                Err(McpError::ToolNotFound(name))
            }
        }
    }

    struct FailingServer;

    #[async_trait]
    impl ServerHandler for FailingServer {
        async fn initialize(
            &self,
            _context: &ServerCtx,
            _protocol_version: String,
            _capabilities: ClientCapabilities,
            _client_info: Implementation,
        ) -> tmcp::Result<InitializeResult> {
            Ok(InitializeResult::new("mock"))
        }

        async fn list_tools(
            &self,
            _context: &ServerCtx,
            _cursor: Option<Cursor>,
        ) -> tmcp::Result<ListToolsResult> {
            Ok(ListToolsResult::new())
        }

        async fn call_tool(
            &self,
            _context: &ServerCtx,
            name: String,
            _arguments: Option<Arguments>,
            _task: Option<TaskMetadata>,
        ) -> tmcp::Result<CallToolResult> {
            if name == "script_eval" {
                Err(McpError::InternalError("boom".to_string()))
            } else {
                Err(McpError::ToolNotFound(name))
            }
        }
    }

    async fn make_mock_app() -> (AppProcess, ServerHandle) {
        let ((server_reader, server_writer), (client_reader, client_writer)) = {
            let (sr, sw, cr, cw) = make_duplex_pair();
            ((sr, sw), (cr, cw))
        };

        let server = Server::new(|| MockServer);
        let handle = ServerHandle::from_stream(server, server_reader, server_writer)
            .await
            .expect("server handle");

        let mut client = Client::new("test", "0.1.0");
        client
            .connect_stream(client_reader, client_writer)
            .await
            .expect("connect");
        client.init().await.expect("init");

        let app = AppProcess {
            child: None,
            process_group_id: None,
            client: Arc::new(AsyncMutex::new(client)),
            stderr_task: None,
            stderr_buffer: Arc::new(Mutex::new(Vec::new())),
            log_state: LogState::new(false),
        };

        (app, handle)
    }

    async fn make_failing_app() -> (AppProcess, ServerHandle) {
        let ((server_reader, server_writer), (client_reader, client_writer)) = {
            let (sr, sw, cr, cw) = make_duplex_pair();
            ((sr, sw), (cr, cw))
        };

        let server = Server::new(|| FailingServer);
        let handle = ServerHandle::from_stream(server, server_reader, server_writer)
            .await
            .expect("server handle");

        let mut client = Client::new("test", "0.1.0");
        client
            .connect_stream(client_reader, client_writer)
            .await
            .expect("connect");
        client.init().await.expect("init");

        let app = AppProcess {
            child: None,
            process_group_id: None,
            client: Arc::new(AsyncMutex::new(client)),
            stderr_task: None,
            stderr_buffer: Arc::new(Mutex::new(Vec::new())),
            log_state: LogState::new(false),
        };

        (app, handle)
    }

    #[tokio::test]
    async fn proxy_forwards_tool_calls() {
        let (app, _handle) = make_mock_app().await;
        let tempdir = test_tempdir();

        let mut state = make_state(&tempdir);
        state.status = AppStatus::Running;
        state.app = Some(app);

        let result = state
            .proxy_call(
                "script_eval",
                Some(script_eval_request_value(ScriptEvalRequest {
                    script: "return true".to_string(),
                    timeout_ms: Some(1_000),
                    options: None,
                })),
            )
            .await;
        let outcome = parse_script_eval_outcome(&result).expect("script eval outcome");
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn restart_updates_state_on_success() {
        let (app, _handle) = make_mock_app().await;
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);

        let status = state
            .restart_with(|_, _| Box::pin(async move { Ok(app) }))
            .await
            .expect("restart");

        assert!(matches!(status, LifecycleStartStatus::Running));
        assert!(matches!(state.status, AppStatus::Running));
        assert!(state.app.is_some());
    }

    #[tokio::test]
    async fn start_is_idempotent_when_running() {
        let (app, _handle) = make_mock_app().await;
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);
        state.status = AppStatus::Running;
        state.app = Some(app);

        let status = state.start().await.expect("start");
        assert!(matches!(status, StartStatus::AlreadyRunning));
    }

    #[tokio::test]
    async fn restart_reports_startup_failure() {
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);

        let status = state
            .restart_with(|_, _| {
                Box::pin(async { Err(AppStartError::StartupFailed("startup output".to_string())) })
            })
            .await
            .expect("restart");

        assert!(matches!(
            status,
            LifecycleStartStatus::StartupFailed(ref output) if output == "startup output"
        ));
        assert!(matches!(
            state.status,
            AppStatus::StartupFailed { ref output } if output == "startup output"
        ));
    }

    #[tokio::test]
    async fn restart_sets_not_running_on_spawn_error() {
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);

        let error = state
            .restart_with(|_, _| {
                Box::pin(async { Err(AppStartError::Other("spawn failed".to_string())) })
            })
            .await
            .expect_err("restart should fail");

        assert!(matches!(error, EdevError::AppStart(_)));
        assert!(matches!(state.status, AppStatus::NotRunning));
        assert!(state.app.is_none());
    }

    #[tokio::test]
    async fn shutdown_clears_running_app() {
        let (app, _handle) = make_mock_app().await;
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);
        state.status = AppStatus::Running;
        state.app = Some(app);

        state.shutdown().await.expect("shutdown");

        assert!(matches!(state.status, AppStatus::NotRunning));
        assert!(state.app.is_none());
    }

    #[tokio::test]
    async fn restart_reports_startup_failure_when_readiness_probe_fails() {
        let (app, _handle) = make_failing_app().await;
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);

        let status = state
            .restart_with(|_, _| Box::pin(async move { Ok(app) }))
            .await
            .expect("restart");

        assert!(matches!(status, LifecycleStartStatus::StartupFailed(_)));
        assert!(matches!(state.status, AppStatus::StartupFailed { .. }));
        assert!(state.app.is_none());
    }

    #[tokio::test]
    async fn start_reports_restart_required_after_prior_startup_failure() {
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);
        state.status = AppStatus::StartupFailed {
            output: "boom".to_string(),
        };

        let status = state.start().await.expect("start");
        assert!(matches!(status, StartStatus::RestartRequired(ref output) if output == "boom"));
    }

    #[test]
    fn tools_list_is_static_across_lifecycle_states() {
        let tempdir = test_tempdir();
        let state = make_state(&tempdir);
        let stopped_names = state
            .tools_list()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let mut running_state = make_state(&tempdir);
        running_state.status = AppStatus::Running;
        let running_names = running_state
            .tools_list()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            stopped_names,
            vec![
                "start".to_string(),
                "stop".to_string(),
                "restart".to_string(),
                "status".to_string(),
                "script_eval".to_string(),
                "script_api".to_string(),
            ]
        );
        assert_eq!(stopped_names, running_names);
    }

    #[test]
    fn proxied_tool_names_only_include_script_eval() {
        assert!(is_proxied_tool("script_eval"));
        assert!(!is_proxied_tool("script_api"));
        assert!(!is_proxied_tool("start"));
        assert!(!is_proxied_tool("restart"));
        assert!(!is_proxied_tool("widget_list"));
    }

    #[test]
    fn tool_error_reports_restart_failed_kind() {
        let result = tool_error(ErrorKind::RestartFailed, "restart failed");
        let payload = result.structured_content.expect("structured content");
        assert_eq!(payload["error"]["kind"], "restart_failed");
    }

    #[test]
    fn normalize_tool_result_adds_text_for_structured_errors() {
        let result = CallToolResult::error("INTERNAL", "Something broke");
        assert!(result.content.is_empty());
        let normalized = normalize_tool_result(result);
        assert!(!normalized.content.is_empty());
        match &normalized.content[0] {
            ContentBlock::Text(text) => assert_eq!(text.text, "Something broke"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn parse_fixture_list_decodes_registered_fixtures() {
        let fixtures = parse_fixture_list(successful_outcome(&serde_json::json!([
            {
                "name": "basic.default",
                "description": "baseline"
            }
        ])))
        .expect("fixtures");

        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].name, "basic.default");
        assert_eq!(fixtures[0].description, "baseline");
    }

    #[test]
    fn parse_fixture_list_rejects_missing_payload() {
        let outcome = successful_outcome(&serde_json::Value::Null);

        let error = parse_fixture_list(outcome).expect_err("missing payload should fail");
        assert!(matches!(
            error,
            EdevError::FixtureFailed(ref message) if message == "fixtures() returned no value"
        ));
    }

    #[test]
    fn script_eval_error_message_prefers_runtime_error() {
        let message = script_eval_error_message(
            Some(&ScriptErrorInfo {
                error_type: "runtime".to_string(),
                message: "script exploded".to_string(),
                location: None,
                backtrace: None,
                code: None,
                details: None,
            }),
            "fallback",
        );

        assert_eq!(message, "script exploded");
    }

    #[test]
    fn restart_retry_detector_matches_transport_closed_variants() {
        assert!(restart_result_is_transport_closed(&Err(EdevError::Mcp(
            McpError::TransportDisconnected,
        ))));
        assert!(restart_result_is_transport_closed(&Err(EdevError::Mcp(
            McpError::ConnectionClosed,
        ))));
        assert!(restart_result_is_transport_closed(&Err(EdevError::Mcp(
            McpError::Transport("transport closed".to_string()),
        ))));
    }

    #[test]
    fn restart_retry_detector_ignores_non_transport_errors() {
        assert!(!restart_result_is_transport_closed(&Err(EdevError::Mcp(
            McpError::InternalError("boom".to_string()),
        ))));
        assert!(!restart_result_is_transport_closed(&Err(
            EdevError::AppStart("spawn failed".to_string(),)
        )));
    }

    #[test]
    fn restart_result_detector_matches_startup_failed_transport_closed() {
        assert!(restart_result_is_transport_closed(&Ok(
            LifecycleStartStatus::StartupFailed("Transport disconnected unexpectedly".to_string()),
        )));
        assert!(!restart_result_is_transport_closed(&Ok(
            LifecycleStartStatus::StartupFailed("error[E0432]: unresolved import".to_string()),
        )));
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let (app, _handle) = make_mock_app().await;
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);
        state.status = AppStatus::Running;
        state.app = Some(app);

        let stopped = state.stop_app().await.expect("stop");
        let already_stopped = state.stop_app().await.expect("stop");

        assert!(matches!(stopped, StopStatus::Stopped));
        assert!(matches!(already_stopped, StopStatus::AlreadyStopped));
        assert!(matches!(state.status, AppStatus::NotRunning));
    }

    #[test]
    fn status_report_covers_all_lifecycle_states() {
        let tempdir = test_tempdir();
        let mut state = make_state(&tempdir);
        assert_eq!(state.status_report().state, "not_running");

        state.status = AppStatus::Starting;
        assert_eq!(state.status_report().state, "starting");

        state.status = AppStatus::Running;
        assert_eq!(state.status_report().state, "running");

        state.status = AppStatus::StartupFailed {
            output: "boom".to_string(),
        };
        let report = state.status_report();
        assert_eq!(report.state, "startup_failed");
        assert_eq!(report.startup_output.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn script_api_is_available_while_stopped() {
        let tempdir = test_tempdir();
        let state = Arc::new(AsyncMutex::new(make_state(&tempdir)));
        let server = EdevServer { state };
        let ctx = TestServerContext::new();

        let result = server
            .call_tool(ctx.ctx(), "script_api".to_string(), None, None)
            .await
            .expect("script_api");
        assert_eq!(result.text().expect("text"), script_definitions());
    }

    #[tokio::test]
    async fn script_eval_returns_call_start_when_app_is_stopped() {
        let tempdir = test_tempdir();
        let state = Arc::new(AsyncMutex::new(make_state(&tempdir)));
        let server = EdevServer { state };
        let ctx = TestServerContext::new();

        let result = server
            .call_tool(
                ctx.ctx(),
                "script_eval".to_string(),
                Some(script_eval_request_value(ScriptEvalRequest {
                    script: "return true".to_string(),
                    timeout_ms: Some(1_000),
                    options: None,
                })),
                None,
            )
            .await
            .expect("script_eval");
        let payload = result
            .structured_content
            .clone()
            .expect("structured content");
        assert_eq!(payload["error"]["kind"], "app_not_running");
        assert_eq!(
            result.text().expect("text"),
            "App is not running. Call start."
        );
    }

    #[tokio::test]
    async fn idle_shutdown_waits_for_inactivity() {
        let tempdir = test_tempdir();
        let state = Arc::new(AsyncMutex::new(make_state(&tempdir)));
        {
            let mut state_guard = state.lock().await;
            state_guard.mark_activity();
        }

        let waiter = tokio::spawn(wait_for_idle_shutdown(
            Arc::clone(&state),
            Duration::from_millis(80),
        ));

        sleep(Duration::from_millis(30)).await;
        {
            let mut state_guard = state.lock().await;
            state_guard.mark_activity();
        }
        sleep(Duration::from_millis(40)).await;
        assert!(!waiter.is_finished());

        sleep(Duration::from_millis(60)).await;
        assert!(waiter.is_finished());
    }

    #[tokio::test]
    async fn list_tools_does_not_delay_idle_shutdown() {
        let tempdir = test_tempdir();
        let state = Arc::new(AsyncMutex::new(make_state(&tempdir)));
        {
            let mut state_guard = state.lock().await;
            state_guard.last_activity = Instant::now() - Duration::from_millis(250);
        }

        let server = EdevServer {
            state: Arc::clone(&state),
        };
        let ctx = TestServerContext::new();
        let _result = server
            .list_tools(ctx.ctx(), None::<Cursor>)
            .await
            .expect("list tools");

        let wait_result = timeout(
            Duration::from_millis(20),
            wait_for_idle_shutdown(Arc::clone(&state), Duration::from_millis(100)),
        )
        .await;
        assert!(
            wait_result.is_ok(),
            "list_tools should not refresh idle activity"
        );
    }
}
