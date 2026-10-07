import { test, expect } from "./fixtures/app-fixture";

test.describe("text layer input", () => {
  test.beforeEach(async ({ appPage }) => {
    await appPage.openAddLayerWindow();
    await appPage.clickWidget("Text");
  });

  test("selecting Text source shows format options", async ({ appPage }) => {
    await appPage.expectScreenshot("text-tab-selected.png");
  });

  test("selecting GeoJSON format in text tab shows text area", async ({
    appPage,
  }) => {
    await appPage.clickWidget("GeoJSON");
    await appPage.expectScreenshot(
      "text-tab-geojson-textarea.png",
    );
  });

  test("selecting WKT format in text tab shows WKT hint", async ({
    appPage,
  }) => {
    await appPage.clickWidget("WKT");
    await appPage.expectScreenshot("text-tab-wkt-textarea.png");
  });

  // Regression test for #281: submitting used to panic on a fresh session.
  test("adding GeoJSON text creates a layer", async ({ appPage }) => {
    await appPage.clickWidget("GeoJSON");
    await appPage.clickWidget("Input text");
    await appPage.page.locator("canvas").focus();
    await appPage.page.keyboard.type(
      JSON.stringify({
        type: "Feature",
        properties: {},
        geometry: { type: "LineString", coordinates: [[0, 0], [10, 5]] },
      }),
    );
    await appPage.waitForNextFrame();
    await appPage.clickWidget("Add layer");
    await appPage.waitForIdle();

    const state = await appPage.getAppState();
    expect(state.open_windows).not.toContain("Add Layer");
    expect(state.layers).toHaveLength(1);
    expect(state.layers[0]).toMatchObject({
      name: "Inputted file",
      feature_count: 1,
      crs: { epsg: 4326 },
      bbox: { min_x: 0, min_y: 0, max_x: 10, max_y: 5 },
    });
  });
});
