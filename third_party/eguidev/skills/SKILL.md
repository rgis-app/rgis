---
name: eguidev
description: AI-assisted egui development -- instrumentation, Luau scripting, smoketests, and iterative workflows.
---

# eguidev

eguidev instruments egui apps for AI-assisted development. Agents drive the app
through Luau scripts via `script_eval`; `edev` manages app lifecycle and hosts
the MCP tool surface.

Full documentation: https://github.com/cortesi/eguidev

**First action:** call the `script_api` MCP tool to load the Luau API
definitions before doing anything else. The returned `eguidev.d.luau` is the
canonical reference for all scripting types, functions, and conventions. You
cannot write correct scripts without it.


## Setup

The app needs three things: an `eguidev` dependency, widget instrumentation, and
a `.edev.toml` config. See the project README and `examples/edev.toml` for
setup details.

The MCP server exposes six tools: `start`, `stop`, `restart`, `status`,
`script_eval`, and `script_api`. Everything else happens inside Luau scripts.


## Instrumentation

Tag widgets with `dev_*` helpers from the `DevUiExt` trait to make them visible
to scripts. Each helper takes an explicit string id that becomes the canonical
selector. Use `container()` to annotate hierarchy.

The `eguidev` crate docs are the canonical Rust API reference. If `ruskel` is
installed, run `ruskel eguidev` to inspect the public API surface (widget
helpers, types, instrumentation functions). Use `ruskel eguidev --search <term>`
to find specific items.

For custom widgets, the happy path has two parts:

- publish state with `track_response_full(...)` or `id_with_meta(...)`
- consume queued `set_value(...)` updates with
  `take_widget_value_override(...)` before rendering the widget


## Scripting

Write strict Luau. `--!strict` is implicit for all `script_eval` scripts.
See `docs/luau.md` for Luau syntax quick reference.


## Complete Scripts

Each `script_eval` call must be a self-contained script that performs setup,
action, and verification in one evaluation. Do not drive the app step-by-step
across multiple `script_eval` calls.

A well-structured script has four phases:

```luau
-- Setup
fixture("basic.with_items")
local vp = root()

-- Act
vp:widget_get("item.0.delete"):click()

-- Verify
local remaining = vp:wait_for_widget("items.count", function(widget)
    return widget.value_text == "2"
end)
assert(remaining ~= nil, "items.count should update")

-- Report
return { remaining = remaining.value_text }
```

Use `log()` for intermediate diagnostics and return a summary at the end.
When running under `edev smoke`, the final return value is ignored; only
assertions/failures and logs affect the suite result.


## Lifecycle

- **NEVER** use `pkill`, `kill`, ctrl-c, or shell commands to manage the app.
- Use `start` to ensure the app is running, `restart` for a fresh process.
- Call `fixture()` at the start of scripts for a known in-app baseline.
- Fixtures auto-settle, so the UI is ready for actions after `fixture()`.
- For `eframe` apps, process fixture requests in `App::update`, not `logic`.
- Prefer `eframe::Renderer::Glow` for automation runs; `wgpu` can stall idle
  frames in some integrations.


## Inspection

Prefer programmatic inspection over screenshots:

- `widget_list`, `widget_get`, `state()`, `children()`, `parent()` for
  structure and values.
- `wait_for_widget` with predicates for state readiness -- widget existence
  does not imply state readiness.
- `check_layout()` for layout problems (clipping, overflow, overlap).
- `text_measure()` for text sizing and truncation.

Use `screenshot()` only when the question is genuinely visual: alignment,
clipping, rendering quality, image content. Returned `ImageRef` values produce
image blocks in the MCP response.


## Smoketests

Smoketests are self-contained `.luau` scripts in a suite directory. The
configured suite is discovered recursively and executed in lexicographic order
by relative path. Explicit script arguments to `edev smoke` run in the order
provided. Each script establishes its own state via `fixture()`, exercises the
UI, and asserts outcomes.

```sh
edev smoke
edev smoke --verbose
edev smoke smoketest/*.luau
edev smoke smoketest/10_basic.luau tmp/ad_hoc_probe.luau
```

Conventions:
- Every script must call `fixture()` before interacting with the UI.
- Assert visible app behavior, not internals.
- Keep scripts independent -- no reliance on state from earlier tests.
- Do not rely on the script's final `return` value; `edev smoke` ignores it.
- Smoketests double as regression tests and executable API documentation.


## Iterative Development Workflow

1. **Modify code** -- make changes to the app or its instrumentation.
2. **Lint** -- `cargo xtask tidy` to catch issues before running.
3. **Restart** -- call `restart` to pick up code changes.
4. **Inspect** -- use `script_eval` to explore widget state, verify layout,
   and exercise interactions. Start with `fixtures()` to discover baselines.
5. **Verify** -- write a complete script that sets up, acts, and asserts.
6. **Smoketest** -- run the suite to confirm nothing else broke.

When debugging:
- Use `widget_list({ role = "button" })` or `widget_list({ id_prefix = "settings" })`
  to discover what's available.
- Use `state()` on a widget handle to inspect current role, value, geometry,
  enabled/visible/focused state.
- Use `show_debug_overlay("bounds")` to visualize widget rects.
- Use `check_layout()` to find clipping, overflow, and overlap issues.
- Use `log()` liberally -- logs appear in the script result payload.
