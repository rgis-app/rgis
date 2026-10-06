# Driving rgis programmatically

rgis can be scripted and inspected without looking at pixels, natively and
on the web. Scripts and the browser use the same command set, and both report
the same JSON app state.

* **Native:** `--script` runs steps after startup; `--dump-state` and
  `--screenshot` write the results, then rgis exits.
* **Web (wasm):** `dispatch(json)` runs the same steps; `get_app_state()`
  returns the same state JSON.
* **Tests:** `cargo test -p rgis-automation` runs realistic workflows headlessly
  and asserts on the state (`rgis-automation/tests/agent_workflows.rs`).

## Scripted native runs

```sh
cargo run -p rgis -- \
  --script '[{"cmd": "load_file", "path": "countries.geojson"},
             {"cmd": "change_crs", "epsg": 3857},
             {"cmd": "wait_idle"},
             {"cmd": "close_window", "title": "Welcome"}]' \
  --dump-state out.json \
  --screenshot out.png
```

Then check `out.json` for the layers, CRS, extents, and errors, and `out.png`
for the rendered window.

| Flag | Meaning |
| --- | --- |
| `--script <json>` | A JSON array of steps (or a single step object). |
| `--script @<file>` | Read the steps from a file. |
| `--screenshot <path.png>` | After the script, save a PNG of the window and exit. |
| `--dump-state <path.json>` | After the script, write the app state and exit. |
| `--exit-after-script` | Exit when the script is done, even without output flags. |
| `--window-size <W>x<H>` | Window size in logical pixels. Batch runs default to `1280x720` (the Playwright viewport) at scale factor 1, so screenshots are the same size on every display. |
| `--show-window` | Show the window during batch runs (see the macOS note below). |

A *batch run* is any run with `--screenshot`, `--dump-state`, or
`--exit-after-script`. Batch runs execute the script, take the screenshot,
write the state (in that order), and exit. Without `--script`, a batch run
just waits for startup work (such as `--url`) to settle.

Paths in scripts are relative to the current directory. Run through
`cargo run -p rgis` (or set `BEVY_ASSET_ROOT=rgis`) so rgis can find its
assets; otherwise the logo and point sprites are missing.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Every step ran. |
| 1 | A step failed at runtime (unknown layer, unknown EPSG code, unreadable file, `wait_idle` timeout, ...). The error is printed to stderr, the remaining steps are skipped, no screenshot is taken, and `--dump-state` is still written so you can see what happened. |
| 2 | The script couldn't be parsed or read (bad JSON, unknown command, bad parameters). Nothing ran. |

A file that fails to *load* (e.g. malformed GeoJSON) isn't a script error:
rgis handles it like a user would see it. Look for it in the state's
`messages`.

### The window, and the macOS occluded-window caveat

On macOS, a visible window that's fully covered by other windows stops
receiving redraw events, and Bevy stops updating the app until it's
uncovered. A scripted run left behind other windows would hang.

To avoid that, **batch runs hide the window by default**. Bevy keeps updating
an app whose windows are all invisible, and rendering and screenshots still
work, so batch runs behave the same whether or not anything covers them. Pass
`--show-window` to watch a batch run. The window is then kept on top of other
windows, but minimizing it or moving it to another Space will still pause it.

Interactive runs (`--script` without batch flags) use a normal visible window
and are subject to the caveat: keep the window visible until the script
finishes.

## Commands

Each step is an object with a `"cmd"` field plus parameters. Unknown commands,
unknown parameters, and out-of-range values are rejected before anything runs.
Steps run in order, at most one per frame, so each step sees the effects of the
previous one.

### Layers are referenced by name, index, or id

* `"layer": "World: Countries"`: exact name; an error if several layers share it.
* `"layer": 0`: draw-order index. `0` is the bottom layer and `-1` the top one,
  the most recently added. The side panel lists layers in the opposite order.
* `"layer": {"id": 3}`: the internal id from the state's `layers[].id`.

Loading is asynchronous. Add a `wait_idle` step before referring to a layer you
just loaded.

### Command reference

| Command | Parameters | Notes |
| --- | --- | --- |
| `load_file` | `path`, `format`?, `crs`?, `name`? | Native only. `format` (`geojson`, `shapefile`, `wkt`, `gpx`, `geotiff`) defaults to the file extension. `crs` is the source CRS, `4326`, `"EPSG:4326"`, or a PROJ string; default 4326 (GeoTIFFs use their embedded EPSG code). `name` defaults to the file name. |
| `load_url` | `url`, `format`?, `crs`?, `name`? | Fetches over HTTP(S). On the web the URL must be absolute. |
| `load_text` | `text`, `format`, `crs`?, `name`? | Inline GeoJSON, WKT, or GPX. |
| `change_crs` | `epsg` **or** `proj` | Changes the map's target CRS and reprojects every layer. |
| `toggle_visibility` | `layer`, `visible`? | With `visible`, sets the visibility instead of toggling. |
| `move_layer` | `layer`, `direction` | `"up"` (toward the top) or `"down"`. |
| `delete_layer` | `layer` | |
| `duplicate_layer` | `layer` | Vector layers only; the copy is added on top as `Copy of <name>`. |
| `rename_layer` | `layer`, `name` | |
| `set_fill_color` | `layer`, `color` | `"#rrggbb"`, `"#rrggbbaa"`, or `[r, g, b]` / `[r, g, b, a]` in sRGB, 0–1. |
| `set_stroke_color` | `layer`, `color` | |
| `set_point_size` | `layer`, `size` | |
| `center_on_layer` | `layer` | Flies the camera to the layer's extent. The layer must be loaded. |
| `pan` | `dx`?, `dy`? | Screen pixels. Positive `dx` moves the view east, positive `dy` north. |
| `zoom` | `factor` | Around the view center. `2` zooms in 2×, `0.5` zooms out. |
| `select_feature` | `layer`, `feature` | `feature` is the 0-based position in the source file. Opens the properties window, like clicking the feature. |
| `deselect` | | Clears the selection and closes the properties window. |
| `open_window` | `title` | `Add Layer`, `Change CRS`, `Logs`, or `Welcome`. |
| `close_window` | `title` | Any title in the state's `open_windows` except `Distances`. Fails if the window isn't open. |
| `set_animations` | `enabled` | Fades and highlight pulsing. |
| `wait_idle` | `max_frames`?, `timeout_secs`? | Waits until no background jobs (loading, reprojecting, mesh building), camera flights, or fades are running, for several consecutive frames. Defaults: 36000 frames, 60 s. Fails with what was still running if it times out. |
| `frames` | `n` | Lets `n` frames run. |
| `dump_state` | `path` | Native only. Writes the state mid-script. |
| `screenshot` | `path` | Native only. Saves a PNG mid-script. |

## The state

`--dump-state`, `dump_state`, and `get_app_state()` all produce the same JSON,
described by `rgis_automation::AppState` (`rgis-automation/src/state.rs`):

```jsonc
{
  "version": 1,
  "frame": 105,
  "idle": true,              // nothing in flight; see wait_idle
  "jobs_in_flight": 0,
  "jobs": [],                // e.g. ["Projecting layer"]
  "target_crs": { "epsg": 3857, "proj": null, "name": "WGS 84 / Pseudo-Mercator", "geographic": false },
  "camera": {
    "center": { "x": -5897321.0, "y": -747450.4 },   // target-CRS coordinates
    "scale": 56978.9,                                 // target-CRS units per pixel
    "viewport": { "width": 1280.0, "height": 720.0 },
    "visible_bbox": { "min_x": -42363845.0, "min_y": -21259870.4, "max_x": 30569203.0, "max_y": 19764969.6 },
    "animating": false
  },
  "layers": [                // draw order: [0] is the bottom layer
    {
      "index": 0, "id": 1, "name": "countries.geojson",
      "kind": "vector", "visible": true,
      "crs": { "epsg": 4326, "proj": null, "name": "WGS 84", "geographic": true },
      "feature_count": 177,
      "geometry_types": ["Polygon", "MultiPolygon"],
      "bbox": { "min_x": -180.0, "min_y": -90.0, "max_x": 180.0, "max_y": 83.6 },        // source CRS
      "projected_bbox": { "min_x": -20037508.3, "min_y": -20006332.4, "max_x": 20037508.3, "max_y": 18397473.7 },  // target CRS
      "projected": true,
      "rendered": true,      // meshes spawned; null when there's no renderer
      "fill_color": "#1f77b4ff", "stroke_color": "#000000ff", "point_size": 5.0,
      "raster": null         // { "width", "height", "format" } for rasters
    }
  ],
  "selected_feature": null,  // { layer_id, layer_index, layer_name, feature_index, feature_id, properties }
  "open_windows": ["Welcome"],
  "messages": [],            // text shown in the message window, e.g. "Error loading file: ..."
  "command_errors": [],      // errors from script or dispatch steps
  "logs": [],                // recent WARN/ERROR log entries
  "script": { "pending_steps": 0, "current_step": null, "completed_steps": 4, "last_error": null },
  "animations_enabled": true
}
```

## On the web

The wasm build exports two functions, which `www/index.js` puts on `window`:

* `get_app_state(): string` returns the state JSON as of the last frame. The
  app only tracks its state once it's been asked for, so the first call returns
  `"null"`. Poll until you get an object.
* `dispatch(json: string)` queues one step or an array of steps, using the
  commands above. It throws if the JSON doesn't parse. Errors while running
  show up in the state's `command_errors` and `script.last_error`. `load_file`,
  `dump_state`, and `screenshot` aren't available on the web.

The older test hooks (`get_widget_rect`, `close_window`,
`set_first_layer_fill_color`, `set_animations_enabled`, ...) still work.
`set_first_layer_fill_color` now goes through the same command layer as
`set_fill_color` with `"layer": -1`.

In Playwright, `www/tests/fixtures/app-fixture.ts` wraps these:

```ts
const state = await appPage.dispatch([
  { cmd: "load_text", text: geojson, format: "geojson", name: "Square" },
  { cmd: "wait_idle" },
  { cmd: "change_crs", epsg: 4326 },
  { cmd: "wait_idle" },
]);
expect(state.target_crs?.epsg).toBe(4326);
expect((await appPage.getLayer("Square")).projected).toBe(true);
```

`appPage.dispatch` waits for the app to go idle and throws if a step failed;
`appPage.waitForIdle()` and `appPage.getAppState()` are available on their own.

## Headless tests

`rgis-automation/tests/agent_workflows.rs` builds an `App` from
`MinimalPlugins` plus every rgis plugin that doesn't need a GPU. It has no OS
window, just a `Window` component so the camera has a viewport size. Each test
runs a script through the same `ScriptRunner` as `--script` and asserts on
`AppState`. Everything that needs rendering (meshes, egui, screenshots) is
left to the Playwright suite and real `--script` runs. `rendered` is `null` in
these tests, and a `screenshot` step fails with a clear error instead of
hanging.

The GeoTIFF test needs the `geotiff-test-data` submodule
(`git submodule update --init`).
