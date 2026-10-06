# Driving rgis programmatically

Scripts (native) and `dispatch` (web) share one command set, and both report
the same JSON app state, so results can be checked without looking at pixels.

```sh
cargo run -p rgis -- \
  --script '[{"cmd": "load_file", "path": "countries.geojson"},
             {"cmd": "change_crs", "epsg": 3857},
             {"cmd": "wait_idle"}]' \
  --dump-state out.json --screenshot out.png
```

| Flag | Meaning |
| --- | --- |
| `--script <json \| @file>` | Steps to run after startup. |
| `--screenshot <path.png>` | Save a PNG of the window after the script, then exit. |
| `--dump-state <path.json>` | Write the app state after the script, then exit. |
| `--exit-after-script` | Exit when the script is done. |
| `--window-size <W>x<H>` | Defaults to `1280x720` at scale factor 1 for batch runs. |
| `--show-window` | Show the window during batch runs. |

Exit codes: `0` on success; `1` if a step failed at runtime (the error goes to
stderr and `--dump-state` is still written); `2` if the script doesn't parse.
A file that fails to load isn't a script error; it shows up in the state's
`messages`. Run via `cargo run -p rgis` so rgis can find its assets.

**macOS:** a visible window that's covered by other windows stops redrawing,
which stalls the app. Batch runs (`--screenshot`, `--dump-state`,
`--exit-after-script`) therefore hide the window, which still renders and can
be screenshotted. With `--show-window`, or a plain `--script` run, keep the
window uncovered until the script finishes.

## Commands

Each step is `{"cmd": "<name>", ...params}`. Unknown commands and bad
parameters are rejected before anything runs. Steps run one per frame. Loading
is asynchronous, so add `wait_idle` before using a layer you just loaded.

`layer` is a name, a draw-order index (`0` is the bottom layer, `-1` the top),
or `{"id": n}` from the state.

| Command | Parameters |
| --- | --- |
| `load_file` (native) | `path`, `format`?, `crs`? (default 4326), `name`? |
| `load_url` | `url`, `format`?, `crs`?, `name`? |
| `load_text` | `text`, `format`, `crs`?, `name`? |
| `change_crs` | `epsg` or `proj` |
| `toggle_visibility` | `layer`, `visible`? |
| `move_layer` | `layer`, `direction` (`up`/`down`) |
| `delete_layer`, `duplicate_layer` | `layer` |
| `rename_layer` | `layer`, `name` |
| `set_fill_color`, `set_stroke_color` | `layer`, `color` (`"#rrggbb[aa]"` or `[r, g, b, a?]` in 0–1) |
| `set_point_size` | `layer`, `size` |
| `center_on_layer` | `layer` |
| `pan` | `dx`?, `dy`? (screen pixels) |
| `zoom` | `factor` (>1 zooms in) |
| `select_feature` | `layer`, `feature` (0-based index in the file) |
| `deselect` | |
| `open_window`, `close_window` | `title` |
| `set_animations` | `enabled` |
| `wait_idle` | `max_frames`?, `timeout_secs`? (default 60 s) |
| `frames` | `n` |
| `dump_state`, `screenshot` (native) | `path` |

`format` is `geojson`, `shapefile`, `wkt`, `gpx`, or `geotiff`, inferred from
the extension when omitted. `crs` is `4326`, `"EPSG:4326"`, or a PROJ string.

## State

`rgis_automation::AppState` (`rgis-automation/src/state.rs` documents each
field) includes:
- layers in draw order: kind, visibility, CRS, feature count, geometry types, bbox in the source and target CRS, colors
- the target CRS and the camera
- open windows and the selected feature
- message-window text, command errors and recent warnings/errors
- in-flight jobs, and `idle`

## Web

`www/index.js` exposes two wasm exports on `window`:
- `get_app_state()` returns the state JSON. It returns `"null"` until a frame after the first call, so poll.
- `dispatch(json)` queues one step or an array of steps. It throws on bad JSON. Runtime errors land in `command_errors`.

The Playwright fixture wraps them as `appPage.getAppState()`,
`appPage.waitForIdle()`, and `appPage.dispatch(steps)`, which waits for idle
and throws if a step failed.

## Headless tests

`cargo test -p rgis-automation` runs the workflows in
`rgis-automation/tests/agent_workflows.rs` without a GPU or OS window and
asserts on `AppState`. Rendering, egui, and screenshots are covered by
Playwright. The GeoTIFF test needs `git submodule update --init`.
