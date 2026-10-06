Tiny hand-written fixtures for the headless workflow tests in
`../agent_workflows.rs`. They were written for rgis and are dedicated to the
public domain (CC0); use them for anything.

* `shapes.geojson`: three polygons with `name`/`rank` properties. Extent
  (-10, -5) to (30, 30) in EPSG:4326.
* `lines.geojson`: one line string, so a layer with no fill color.
* `antimeridian.geojson`: one polygon that crosses the 180° meridian.
* `malformed.geojson`: truncated GeoJSON, to check error reporting.

Raster tests use `geotiff-test-data` (a git submodule, MIT licensed).
