import { test, expect } from "./fixtures/app-fixture";

// EPSG:3857 x at ±180°.
const WORLD_HALF_WIDTH = 20037508.342789244;

test.describe("antimeridian handling", () => {
  test("US states layer renders correctly with antimeridian fix", async ({
    appPage,
  }) => {
    test.setTimeout(60000);

    await appPage.addLibraryLayer("USA", "States");
    await appPage.closeWindow("Add Layer");
    await appPage.waitForIdle();

    // The source data runs west to -188.9° (the Aleutians). After the fix,
    // those parts are wrapped to the east side of 180° instead.
    const layer = await appPage.getLayer("USA: States");
    expect(layer).toMatchObject({
      feature_count: 52,
      projected: true,
      rendered: true,
    });
    expect(layer.bbox!.min_x).toBeGreaterThanOrEqual(-180);
    expect(layer.bbox!.max_x).toBeLessThanOrEqual(180);
    expect(layer.bbox!.max_x).toBeGreaterThan(170);
    expect(layer.projected_bbox!.min_x).toBeGreaterThanOrEqual(
      -WORLD_HALF_WIDTH - 1,
    );
    expect(layer.projected_bbox!.max_x).toBeLessThanOrEqual(
      WORLD_HALF_WIDTH + 1,
    );
    expect((await appPage.getAppState()).messages).toEqual([]);

    await appPage.expectScreenshot("us-states-antimeridian.png");
  });

  test("a polygon crossing 180° is split into two parts", async ({
    appPage,
  }) => {
    const box = JSON.stringify({
      type: "Feature",
      properties: {},
      geometry: {
        type: "Polygon",
        coordinates: [[[170, -10], [-170, -10], [-170, 10], [170, 10], [170, -10]]],
      },
    });
    const state = await appPage.dispatch([
      { cmd: "load_text", text: box, format: "geojson", name: "Dateline box" },
      { cmd: "wait_idle" },
    ]);
    const layer = state.layers[0];
    expect(layer.geometry_types).toEqual(["MultiPolygon"]);
    expect(layer.bbox!.min_x).toBe(-180);
    expect(layer.bbox!.max_x).toBe(180);
    expect(layer.projected_bbox!.min_x).toBeCloseTo(-WORLD_HALF_WIDTH, 0);
    expect(layer.projected_bbox!.max_x).toBeCloseTo(WORLD_HALF_WIDTH, 0);
  });
});
