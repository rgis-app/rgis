//! Smoketest suite runner for Luau scripts against a live DevMCP app.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tokio::runtime::Handle;

use crate::{DevMcp, ScriptArgs, ScriptEvalOptions, ScriptEvalOutcome, runtime};

const SUITE_RESULT_PATH: &str = "<suite>";

/// Configuration for a smoketest suite run.
#[derive(Debug, Clone, PartialEq)]
pub struct SuiteConfig {
    /// Directory containing `.luau` test scripts.
    pub suite_dir: PathBuf,
    /// Explicit script paths to run. Empty means discover all `.luau` files
    /// under `suite_dir` recursively in lexicographic order.
    pub scripts: Vec<PathBuf>,
    /// Wall-clock deadline for the entire suite.
    pub suite_timeout: Duration,
    /// Per-script timeout. `None` uses the script-eval default.
    pub script_timeout: Option<Duration>,
    /// Stop after the first failure.
    pub fail_fast: bool,
    /// Args passed to every script in the suite.
    pub args: ScriptArgs,
}

/// Outcome for an individual smoketest script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptStatus {
    /// Script completed successfully.
    Pass,
    /// Script failed or the suite hit a setup error.
    Fail,
    /// Script was skipped because the suite timed out or fail-fast triggered.
    Skip,
}

/// Result of a single script execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptResult {
    /// Forward-slash-normalized relative script path, or `<suite>` for suite-level failures.
    pub path: String,
    /// Final script status.
    pub status: ScriptStatus,
    /// Script runtime in milliseconds. Skipped scripts report `0`.
    pub elapsed_ms: u64,
    /// Failure or skip message when present.
    pub message: Option<String>,
    /// Logs emitted by the script.
    pub logs: Vec<String>,
}

impl ScriptResult {
    fn pass(path: String, elapsed_ms: u64, logs: Vec<String>) -> Self {
        Self {
            path,
            status: ScriptStatus::Pass,
            elapsed_ms,
            message: None,
            logs,
        }
    }

    fn fail(path: String, elapsed_ms: u64, message: String, logs: Vec<String>) -> Self {
        Self {
            path,
            status: ScriptStatus::Fail,
            elapsed_ms,
            message: Some(message),
            logs,
        }
    }

    fn skip(path: String, message: String) -> Self {
        Self {
            path,
            status: ScriptStatus::Skip,
            elapsed_ms: 0,
            message: Some(message),
            logs: Vec::new(),
        }
    }
}

/// Result of running a full suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteResult {
    /// Per-script results in execution order.
    pub results: Vec<ScriptResult>,
    /// Total suite runtime in milliseconds.
    pub elapsed_ms: u64,
}

impl SuiteResult {
    /// Returns `true` when every discovered script passed.
    pub fn success(&self) -> bool {
        self.failed() == 0 && self.skipped() == 0
    }

    /// Count passing scripts.
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == ScriptStatus::Pass)
            .count()
    }

    /// Count failing scripts.
    pub fn failed(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == ScriptStatus::Fail)
            .count()
    }

    /// Count skipped scripts.
    pub fn skipped(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == ScriptStatus::Skip)
            .count()
    }

    /// Render suite results as printable lines.
    pub fn render_lines(&self, verbose: bool) -> Vec<String> {
        let mut lines = Vec::new();
        for script in &self.results {
            if verbose {
                lines.extend(
                    script.logs.iter().map(|log| {
                        format!("LOG: {}", serde_json::to_string(log).unwrap_or_default())
                    }),
                );
            }
            match script.status {
                ScriptStatus::Pass => {
                    lines.push(format!("[PASS] {} ({}ms)", script.path, script.elapsed_ms));
                }
                ScriptStatus::Fail => {
                    let message = script
                        .message
                        .as_deref()
                        .unwrap_or("script failed without a message");
                    lines.push(format!(
                        "[FAIL] {} ({}ms): {}",
                        script.path, script.elapsed_ms, message
                    ));
                }
                ScriptStatus::Skip => {
                    lines.push(format!(
                        "[SKIP] {}: {}",
                        script.path,
                        script.message.as_deref().unwrap_or("skipped")
                    ));
                }
            }
        }
        if verbose {
            lines.push(format!(
                "smoketest summary: {} total, {} passed, {} failed, {} skipped in {}ms",
                self.results.len(),
                self.passed(),
                self.failed(),
                self.skipped(),
                self.elapsed_ms
            ));
        }
        lines
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Input passed to a caller-supplied per-script smoke executor.
pub struct ScriptRunRequest {
    /// Forward-slash-normalized relative script path used for diagnostics.
    pub path: String,
    /// Luau source code loaded from disk.
    pub source: String,
    /// Optional per-script timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Suite-wide args passed to the script.
    pub args: ScriptArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SuiteScript {
    display_path: String,
    source_path: PathBuf,
}

/// Run a smoketest suite against a live `DevMcp` instance.
pub fn run_suite(devmcp: &DevMcp, handle: &Handle, config: &SuiteConfig) -> SuiteResult {
    run_suite_with(config, |request| {
        Ok(runtime::eval_script(
            devmcp,
            handle.clone(),
            &request.source,
            request.timeout_ms,
            ScriptEvalOptions {
                source_name: Some(request.path),
                args: request.args,
            },
        ))
    })
}

/// Run a smoketest suite through a caller-supplied script executor.
pub fn run_suite_with<F>(config: &SuiteConfig, mut execute: F) -> SuiteResult
where
    F: FnMut(ScriptRunRequest) -> Result<ScriptEvalOutcome, String>,
{
    let suite_start = Instant::now();
    let scripts = match collect_suite_scripts(config) {
        Ok(paths) if !paths.is_empty() => paths,
        Ok(_) => {
            return suite_failure_result(
                suite_start.elapsed().as_millis() as u64,
                format!("no smoketests found under {}", config.suite_dir.display()),
            );
        }
        Err(error) => {
            return suite_failure_result(
                suite_start.elapsed().as_millis() as u64,
                format!(
                    "failed to discover smoketests under {}: {error}",
                    config.suite_dir.display()
                ),
            );
        }
    };

    let suite_deadline = suite_start
        .checked_add(config.suite_timeout)
        .unwrap_or_else(Instant::now);
    let mut results = Vec::with_capacity(scripts.len());

    for (index, script) in scripts.iter().enumerate() {
        if Instant::now() >= suite_deadline {
            append_skipped(
                &mut results,
                &scripts[index..],
                "suite deadline exceeded before test started",
            );
            break;
        }

        let relative_display = script.display_path.clone();
        let source_path = &script.source_path;
        let script_start = Instant::now();
        let source = match fs::read_to_string(source_path) {
            Ok(source) => source,
            Err(error) => {
                let elapsed_ms = script_start.elapsed().as_millis() as u64;
                let message = format!("failed to read script: {error}");
                results.push(ScriptResult::fail(
                    relative_display,
                    elapsed_ms,
                    message,
                    Vec::new(),
                ));
                if config.fail_fast {
                    append_skipped(
                        &mut results,
                        &scripts[index + 1..],
                        "fail-fast after earlier smoketest failure",
                    );
                    break;
                }
                continue;
            }
        };

        let elapsed_ms = script_start.elapsed().as_millis() as u64;
        let outcome = execute(ScriptRunRequest {
            path: relative_display.clone(),
            source,
            timeout_ms: config.script_timeout.map(duration_to_millis),
            args: config.args.clone(),
        });

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(message) => {
                results.push(ScriptResult::fail(
                    relative_display,
                    elapsed_ms,
                    message,
                    Vec::new(),
                ));
                if config.fail_fast {
                    append_skipped(
                        &mut results,
                        &scripts[index + 1..],
                        "fail-fast after earlier smoketest failure",
                    );
                    break;
                }
                continue;
            }
        };

        if outcome.success {
            results.push(ScriptResult::pass(
                relative_display,
                elapsed_ms,
                outcome.logs,
            ));
            continue;
        }

        let message = script_failure_summary(&outcome);
        results.push(ScriptResult::fail(
            relative_display,
            elapsed_ms,
            message,
            outcome.logs,
        ));
        if config.fail_fast {
            append_skipped(
                &mut results,
                &scripts[index + 1..],
                "fail-fast after earlier smoketest failure",
            );
            break;
        }
    }

    SuiteResult {
        results,
        elapsed_ms: suite_start.elapsed().as_millis() as u64,
    }
}

fn suite_failure_result(elapsed_ms: u64, message: String) -> SuiteResult {
    SuiteResult {
        results: vec![ScriptResult::fail(
            SUITE_RESULT_PATH.to_string(),
            elapsed_ms,
            message,
            Vec::new(),
        )],
        elapsed_ms,
    }
}

fn append_skipped(results: &mut Vec<ScriptResult>, paths: &[SuiteScript], reason: &str) {
    results.extend(
        paths
            .iter()
            .map(|path| ScriptResult::skip(path.display_path.clone(), reason.into())),
    );
}

fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn collect_suite_scripts(config: &SuiteConfig) -> io::Result<Vec<SuiteScript>> {
    if config.scripts.is_empty() {
        return collect_suite_paths(&config.suite_dir).map(|paths| {
            paths
                .into_iter()
                .map(|path| SuiteScript {
                    display_path: normalize_path(&path),
                    source_path: config.suite_dir.join(path),
                })
                .collect()
        });
    }

    config
        .scripts
        .iter()
        .map(|path| {
            let metadata = fs::metadata(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to access smoketest {}: {error}", path.display()),
                )
            })?;
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("smoketest path is not a file: {}", path.display()),
                ));
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("luau") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("smoketest path must end in .luau: {}", path.display()),
                ));
            }
            Ok(SuiteScript {
                display_path: normalize_path(path),
                source_path: path.clone(),
            })
        })
        .collect()
}

fn collect_suite_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_suite_paths_recursive(root, root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_suite_paths_recursive(
    root: &Path,
    current: &Path,
    paths: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_suite_paths_recursive(root, &path, paths)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("luau") {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|error| {
            io::Error::other(format!("failed to strip smoketest prefix: {error}"))
        })?;
        paths.push(relative.to_path_buf());
    }

    Ok(())
}

fn script_failure_summary(outcome: &ScriptEvalOutcome) -> String {
    let Some(error) = &outcome.error else {
        return "script failed without an error payload".to_string();
    };
    let location = error.location.as_ref().map(|location| {
        let column = location.column.unwrap_or(1);
        format!(":{}:{}", location.line, column)
    });
    match location {
        Some(location) => format!("{} at{}", error.message, location),
        None => error.message.clone(),
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use tokio::runtime::Builder;

    use super::{
        ScriptRunRequest, ScriptStatus, SuiteConfig, SuiteResult, collect_suite_paths,
        collect_suite_scripts, normalize_path, run_suite, run_suite_with,
    };
    use crate::{DevMcp, ScriptArgValue, ScriptArgs, runtime};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_root(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from("tmp")
            .join("smoke_tests")
            .join(format!("{name}_{id}"))
    }

    #[test]
    fn collect_suite_paths_sorts_recursively() {
        let root = test_root("collect_suite_paths_sorts_recursively");
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(root.join("nested")).expect("create suite dir");
        fs::write(root.join("20_second.luau"), "return true").expect("write second");
        fs::write(root.join("nested").join("10_first.luau"), "return true").expect("write first");

        let all = collect_suite_paths(&root).expect("all paths");
        assert_eq!(
            all,
            vec![
                PathBuf::from("20_second.luau"),
                PathBuf::from("nested/10_first.luau"),
            ]
        );

        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn collect_suite_scripts_uses_explicit_paths_in_given_order() {
        let root = test_root("collect_suite_scripts_uses_explicit_paths_in_given_order");
        drop(fs::remove_dir_all(&root));
        let suite_dir = root.join("suite");
        let external_dir = root.join("external");
        fs::create_dir_all(&suite_dir).expect("create suite dir");
        fs::create_dir_all(&external_dir).expect("create external dir");
        let suite_script = suite_dir.join("20_suite.luau");
        let external_script = external_dir.join("10_external.luau");
        fs::write(&suite_script, "return true").expect("write suite script");
        fs::write(&external_script, "return true").expect("write external script");

        let scripts = collect_suite_scripts(&SuiteConfig {
            suite_dir,
            scripts: vec![external_script.clone(), suite_script.clone()],
            suite_timeout: Duration::from_secs(10),
            script_timeout: None,
            fail_fast: false,
            args: ScriptArgs::default(),
        })
        .expect("collect scripts");
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].display_path, normalize_path(&external_script));
        assert_eq!(scripts[0].source_path, external_script);
        assert_eq!(scripts[1].display_path, normalize_path(&suite_script));
        assert_eq!(scripts[1].source_path, suite_script);

        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn run_suite_propagates_args_and_fail_fast() {
        let root = test_root("run_suite_propagates_args_and_fail_fast");
        let suite_dir = root.join("suite");
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&suite_dir).expect("create suite dir");
        fs::write(
            suite_dir.join("10_args.luau"),
            "assert(args.name == \"Sky\")\nassert(args.count == 4)\nreturn true",
        )
        .expect("write args script");
        fs::write(suite_dir.join("20_fail.luau"), "assert(false, \"boom\")").expect("write fail");
        fs::write(suite_dir.join("30_skip.luau"), "return true").expect("write skip");

        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let devmcp = runtime::attach_for_tests(DevMcp::new());
        let result = run_suite(
            &devmcp,
            runtime.handle(),
            &SuiteConfig {
                suite_dir,
                scripts: Vec::new(),
                suite_timeout: Duration::from_secs(10),
                script_timeout: None,
                fail_fast: true,
                args: ScriptArgs::from([
                    (
                        "name".to_string(),
                        ScriptArgValue::String("Sky".to_string()),
                    ),
                    ("count".to_string(), ScriptArgValue::Int(4)),
                ]),
            },
        );

        assert_eq!(result.passed(), 1);
        assert_eq!(result.failed(), 1);
        assert_eq!(result.skipped(), 1);
        assert_eq!(result.results[0].status, ScriptStatus::Pass);
        assert_eq!(result.results[1].status, ScriptStatus::Fail);
        assert_eq!(result.results[2].status, ScriptStatus::Skip);

        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn run_suite_with_callback_propagates_args_and_fail_fast() {
        let root = test_root("run_suite_with_callback_propagates_args_and_fail_fast");
        let suite_dir = root.join("suite");
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&suite_dir).expect("create suite dir");
        fs::write(suite_dir.join("10_args.luau"), "return true").expect("write args script");
        fs::write(suite_dir.join("20_fail.luau"), "return true").expect("write fail");
        fs::write(suite_dir.join("30_skip.luau"), "return true").expect("write skip");

        let result = run_suite_with(
            &SuiteConfig {
                suite_dir,
                scripts: Vec::new(),
                suite_timeout: Duration::from_secs(10),
                script_timeout: Some(Duration::from_secs(7)),
                fail_fast: true,
                args: ScriptArgs::from([
                    (
                        "name".to_string(),
                        ScriptArgValue::String("Sky".to_string()),
                    ),
                    ("count".to_string(), ScriptArgValue::Int(4)),
                ]),
            },
            |request: ScriptRunRequest| {
                assert_eq!(request.timeout_ms, Some(7_000));
                assert_eq!(
                    request.args.get("name"),
                    Some(&ScriptArgValue::String("Sky".to_string()))
                );
                assert_eq!(request.args.get("count"), Some(&ScriptArgValue::Int(4)));
                if request.path == "20_fail.luau" {
                    return Ok(serde_json::from_value(serde_json::json!({
                        "success": false,
                        "logs": ["boom"],
                        "assertions": [],
                        "timing": {
                            "compile_ms": 0,
                            "exec_ms": 0,
                            "total_ms": 0
                        },
                        "error": {
                            "type": "runtime",
                            "message": "boom"
                        }
                    }))
                    .expect("deserialize failure outcome"));
                }
                Ok(serde_json::from_value(serde_json::json!({
                    "success": true,
                    "logs": [],
                    "assertions": [],
                    "timing": {
                        "compile_ms": 0,
                        "exec_ms": 0,
                        "total_ms": 0
                    }
                }))
                .expect("deserialize success outcome"))
            },
        );

        assert_eq!(result.passed(), 1);
        assert_eq!(result.failed(), 1);
        assert_eq!(result.skipped(), 1);

        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn render_lines_emits_summary_and_logs_in_verbose_mode() {
        let result = SuiteResult {
            results: vec![
                super::ScriptResult::pass(
                    "10_pass.luau".to_string(),
                    12,
                    vec!["hello".to_string()],
                ),
                super::ScriptResult::fail(
                    "20_fail.luau".to_string(),
                    18,
                    "boom".to_string(),
                    Vec::new(),
                ),
            ],
            elapsed_ms: 30,
        };

        let lines = result.render_lines(true);
        assert!(lines.iter().any(|line| line == "LOG: \"hello\""));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("[FAIL] 20_fail.luau (18ms): boom"))
        );
        assert!(
            lines
                .last()
                .expect("summary line")
                .contains("smoketest summary: 2 total, 1 passed, 1 failed, 0 skipped in 30ms")
        );
    }
}
