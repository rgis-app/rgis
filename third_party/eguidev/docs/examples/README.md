# DevMCP demo app

This folder documents the demo app used to exercise the DevMCP scripting surface and the
matching `mcp.json` configurations under `docs/examples/mcp/`.

## Prerequisites

- Build or install the `edev` launcher (`cargo build -p edev` or
  `cargo install --path crates/edev`).
- The demo's dev-capable build uses the app-local `devtools` feature, which
  enables the optional `eguidev_runtime` dependency and calls
  `eguidev_runtime::attach()` when launched with `--dev-mcp`.
- This repository checks in a root `.edev.toml`, so `edev mcp` and `edev smoke`
  work without repeating the demo command line.

## eguidev_demo

One demo now covers both surfaces:

- The root viewport exercises instrumented text edits, slider, checkbox, menu, links, images, and
  the primary scroll area.
- A secondary viewport stays open by default and exercises multi-viewport enumeration, instrumented
  rows, scrolling, and a draggable region.

Run directly:

```sh
cargo run -p eguidev_demo --features devtools -- --dev-mcp
```

Run through `edev`:

```sh
cargo run -p edev -- mcp
```

MCP config example: `docs/examples/mcp/eguidev_demo.json`.

## Script behavior notes

- `Viewport:widget_list` omits clipped/hidden widgets unless `include_invisible` is `true`.
- `Viewport:widget_list` returns `Widget` handles for the current frame. Optional filters are
  `role` and `id_prefix`. Read live fields through `widget:state()`.
- `Viewport:widget_get` returns `not_found` when no widget matches and `ambiguous` when selectors
  conflict or match multiple widgets. Duplicate explicit ids are a harder error that block
  further automation until instrumentation is fixed.
- `drag` expects absolute positions in egui points; use `viewport:input_state().pixels_per_point`
  to convert from pixels when needed.
- `drag_relative` accepts normalized 0..1 coordinates within the widget rect.
- `Viewport:set_inner_size` is best-effort; window managers may clamp or ignore it. Verify via
  `viewport:wait_for()` or `viewport:state()` after a resize request.
- `Viewport:wait_for()` is frame-driven; for resizes, prefer explicit predicates over
  `ViewportState`, such as exact size checks or minimum-size guards.

## Smoke tests

Checked-in smoke coverage now has two layers:

- `edev smoke` runs the checked-in Luau smoketest suite.
- `edev smoke --verbose` keeps the same suite but also emits the extra suite summary and launcher
  output that are useful when debugging failures. Smoke diagnostics stay in terminal output rather
  than being written to a persistent artifact directory.
- `edev smoke smoketest/*.luau` or `edev smoke path/to/ad_hoc_probe.luau` runs explicit scripts in
  the order provided, so you can use normal shell expansion or quick one-off probes outside the
  configured suite.
- This repository also keeps a smaller transport-only smoke as a local maintenance task via
  `cargo xtask smoke-edev`.

The suite still requires a GUI-capable environment. The demo uses the native eframe windowing
stack, so this is not a headless smoke path.

Record manual verification runs here when needed.

- 2026-01-23: Launched the demo app via
  `cargo run -p eguidev_demo --features devtools -- --dev-mcp` using the glow renderer. The process was manually
  interrupted because this environment does not support interactive GUI verification.
- 2026-01-25: Used `script_eval` against the demo app to call
  `secondary:set_inner_size()` (640x480 then 900x700). `secondary:wait_for()` matched and
  `secondary:state()` reported updated inner and outer sizes.
