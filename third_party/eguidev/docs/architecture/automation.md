# Automation State Machine

This document describes the runtime state machine used by `eguidev` automation calls and
`edev` lifecycle orchestration.

## Layering

The automation stack now has three explicit layers:

1. App instrumentation: always-available `eguidev` APIs such as `DevMcp`,
   `FrameGuard`, `raw_input_hook`, `DevUiExt`, widget metadata types, and
   fixture registration/collection.
2. Optional embedded runtime: provided by the native-only
   `eguidev_runtime` crate, attached once through
   `eguidev_runtime::attach()`, and responsible for the in-process MCP server,
   script evaluation, screenshots, and async automation waits.
3. `edev` launcher: external process lifecycle and stable host tool surface
   (`start`, `stop`, `restart`, `status`, `script_eval`, `script_api`).

Default and release builds keep layer 1 only. Dev-capable builds add layer 2
behind one app-local feature boundary such as
`devtools = ["dep:eguidev_runtime"]`, and `edev` sits entirely outside the app
binary.

## Lifecycle and fixture flow (`edev`)

`edev mcp` exposes a fixed host tool list:
`start`, `stop`, `restart`, `status`, `script_eval`, and `script_api`.

Launcher lifecycle:

1. The launcher starts in `not_running`.
2. `start` transitions launcher state to `starting` unless the app is already running.
3. `restart` also transitions to `starting`, but first tears down any running app process.
4. App process spawn, MCP handshake, and a minimal `script_eval` readiness probe complete.
5. Successful startup transitions the launcher to `running`.
6. Failed startup transitions the launcher to `startup_failed` and records startup output.
7. `stop` leaves the launcher in `not_running`. Under the current locking model, a `stop`
   request issued while startup is in flight is serialized behind that transition rather than
   interrupting it.

Tool hosting:

- `script_eval` is proxied to the running app and is the only app-dependent host tool.
- `script_api` is served directly by `edev` from checked-in definitions, so it remains callable
  even while the app is stopped.
- `status` reports the current lifecycle state plus startup failure diagnostics when present.
- The host tool list is static for the lifetime of the launcher session; `edev` does not send
  `tools/list_changed` notifications.

Fixtures are applied by scripts via `fixture()`, which auto-settles after application.
For `eframe` apps, fixture requests should be drained from `App::update`.
Handling them in `logic` can apply the state change without producing the fresh
captured UI frame that automation expects afterward.

Renderer note:

- `eframe::Renderer::Glow` is currently the recommended backend for automation.
- Some `wgpu`-backed `eframe` integrations can stall idle-frame delivery under
  automation waits, screenshots, or fixture transitions.

Failure points:
- Build failure before app handshake.
- Script runtime not ready during the startup readiness probe.

## Input pipeline (`eguidev`)

1. Tool calls enqueue `InputAction` and `ViewportCommand` events into `ActionQueue`.
2. `raw_input_hook` drains queued actions for the current viewport and appends egui events.
3. Frame processing consumes injected egui events.
4. `end_frame` captures widget/input snapshots, applies viewport commands, and invokes the
   attached runtime hooks for frame waiters, screenshot capture, and fixture wakeups.

Keyboard delivery modes:
- Ambient (`key`, `input_key`): routed through normal focus state.
- Targeted (`key` with `target` option): resolve target, focus handshake, then delivery.

## Wait loops (`eguidev`)

### `wait_for_widget`

- Polls widget resolution + condition matching.
- Supports explicit poll interval and timeout.
- Condition may be a map (strict, typed keys) or a Luau predicate function.

### `wait_for_settle`

A single composite check combining:
- **InputSettled**: no pending input/command queue and input snapshot available.
- **RepaintIdle**: no repaint requested for viewport.

Both conditions must hold simultaneously. Returns a structured result with `matched` and
`elapsed_ms`.

### Auto-settle

All high-level actions (`click`, `type_text`, `key`, `hover`, `drag`, `scroll`, `paste`, etc.)
auto-settle after performing the action, ensuring the UI has fully processed queued work and
repainted. Disable with `settle: #{enabled: false}`.
