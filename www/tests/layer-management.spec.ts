import { test, expect } from "./fixtures/app-fixture";

test.describe("layer management", () => {
  test.beforeEach(async ({ appPage }) => {
    test.setTimeout(60000);

    await appPage.addLibraryLayer("World", "Countries");

    // Close the Add Layer window
    await appPage.closeWindow("Add Layer");
    await appPage.waitForIdle();
  });

  test("loaded layer appears in side panel with collapsing header", async ({
    appPage,
  }) => {
    const state = await appPage.getAppState();
    expect(state.open_windows).not.toContain("Add Layer");
    expect(state.target_crs?.epsg).toBe(3857);
    expect(state.layers).toHaveLength(1);
    const layer = state.layers[0];
    expect(layer).toMatchObject({
      name: "World: Countries",
      kind: "vector",
      visible: true,
      projected: true,
      rendered: true,
      feature_count: 177,
      crs: { epsg: 4326 },
    });
    expect(layer.geometry_types).toEqual(
      expect.arrayContaining(["Polygon", "MultiPolygon"]),
    );
    expect(layer.bbox?.min_x).toBeCloseTo(-180);
    expect(layer.bbox?.max_x).toBeCloseTo(180);

    await appPage.expectScreenshot(
      "layer-loaded-in-side-panel.png",
    );
  });

  test("expanding layer shows layer details and buttons", async ({
    appPage,
  }) => {
    // Click the ▶ toggle arrow on the "World: Countries" collapsing header
    await appPage.clickWidget("World: Countries");
    await appPage.expectScreenshot("layer-details-expanded.png");
  });

  test("clicking Manage opens manage layer window", async ({ appPage }) => {
    // Expand the layer header
    await appPage.clickWidget("World: Countries");

    // Click "Manage..." button
    await appPage.clickWidget("Manage");
    expect((await appPage.getAppState()).open_windows).toContain(
      "Manage Layer",
    );
    await appPage.expectScreenshot("manage-layer-window.png");
  });

  test("layer visibility toggle hides the layer", async ({ appPage }) => {
    // Expand the layer header
    await appPage.clickWidget("World: Countries");

    // Click "Visible" checkbox
    await appPage.clickWidget("Toggle Visibility");
    await appPage.waitForNextFrame();
    expect((await appPage.getLayer("World: Countries")).visible).toBe(false);
    await appPage.expectScreenshot("layer-hidden.png");
  });

  test("zoom to extent centers the map on the layer", async ({ appPage }) => {
    // Expand the layer header
    await appPage.clickWidget("World: Countries");

    // Click "Zoom to Extent" button
    await appPage.clickWidget("Zoom to extent");
    await appPage.waitForNextFrame();
    await appPage.waitForIdle();

    // The whole layer fits in the window.
    const state = await appPage.getAppState();
    const extent = state.layers[0].projected_bbox!;
    const visible = state.camera!.visible_bbox!;
    expect(state.camera!.animating).toBe(false);
    expect(visible.min_x).toBeLessThanOrEqual(extent.min_x);
    expect(visible.max_x).toBeGreaterThanOrEqual(extent.max_x);
    expect(visible.min_y).toBeLessThanOrEqual(extent.min_y);
    expect(visible.max_y).toBeGreaterThanOrEqual(extent.max_y);

    await appPage.expectScreenshot("zoom-to-extent.png");
  });

  test("expanding Operations section shows available operations", async ({
    appPage,
  }) => {
    // Expand the layer header
    await appPage.clickWidget("World: Countries");

    // Click "Operations" collapsing header
    await appPage.clickWidget("Operations");
    await appPage.expectScreenshot("operations-expanded.png");
  });

  test("layer commands update the state", async ({ appPage }) => {
    // Six layer operations on the 177-feature Countries layer, on top of
    // loading it in `beforeEach`.
    test.setTimeout(120000);

    let state = await appPage.dispatch([
      { cmd: "duplicate_layer", layer: "World: Countries" },
      { cmd: "wait_idle" },
    ]);
    expect(state.layers.map((l) => l.name)).toEqual([
      "World: Countries",
      "Copy of World: Countries",
    ]);

    state = await appPage.dispatch([
      { cmd: "move_layer", layer: "Copy of World: Countries", direction: "down" },
      { cmd: "rename_layer", layer: 1, name: "Original" },
      { cmd: "toggle_visibility", layer: "Original", visible: false },
      { cmd: "set_fill_color", layer: 0, color: "#ff000080" },
    ]);
    expect(state.layers.map((l) => [l.name, l.visible, l.fill_color])).toEqual(
      [
        ["Copy of World: Countries", true, "#ff000080"],
        ["Original", false, expect.any(String)],
      ],
    );

    state = await appPage.dispatch({ cmd: "delete_layer", layer: "Original" });
    expect(state.layers.map((l) => l.name)).toEqual([
      "Copy of World: Countries",
    ]);

    await expect(
      appPage.dispatch({ cmd: "delete_layer", layer: "Original" }),
    ).rejects.toThrow('no layer matches "Original"');
  });
});
