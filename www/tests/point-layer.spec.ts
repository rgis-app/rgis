import { test, expect } from "./fixtures/app-fixture";

const POINT_GEOJSON = JSON.stringify({
  type: "Feature",
  properties: {},
  geometry: { type: "Point", coordinates: [10, 20] },
});

// Regression test for #284: a single point has no extent to fit, which used
// to set the camera scale to zero and hang the scale bar.
test("a single-point layer keeps the map usable", async ({ appPage }) => {
  test.setTimeout(60000);

  const initialScale = (await appPage.getAppState()).camera!.scale;

  let state = await appPage.dispatch([
    { cmd: "load_text", text: POINT_GEOJSON, format: "geojson", name: "Point" },
    { cmd: "wait_idle" },
    { cmd: "center_on_layer", layer: "Point" },
    { cmd: "wait_idle" },
  ]);
  const point = state.layers[0].projected_bbox!;
  expect(state.layers[0]).toMatchObject({ feature_count: 1, rendered: true });
  expect(state.camera!.scale).toBeCloseTo(initialScale);
  const visible = state.camera!.visible_bbox!;
  expect(point.min_x).toBeGreaterThan(visible.min_x);
  expect(point.max_x).toBeLessThan(visible.max_x);
  expect(point.min_y).toBeGreaterThan(visible.min_y);
  expect(point.max_y).toBeLessThan(visible.max_y);

  state = await appPage.dispatch({ cmd: "zoom", factor: 2 });
  expect(state.camera!.scale).toBeCloseTo(initialScale / 2);
});
