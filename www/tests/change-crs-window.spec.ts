import { test, expect } from "./fixtures/app-fixture";

// A 10°×10° square, loaded inline so these tests don't need the network.
const SQUARE_GEOJSON = JSON.stringify({
  type: "Feature",
  properties: { name: "Square" },
  geometry: {
    type: "Polygon",
    coordinates: [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]],
  },
});

test.describe("change CRS window", () => {
  test("CRS window shows input field and Set button", async ({ appPage }) => {
    // Close welcome window first
    await appPage.closeWindow("Welcome");

    // Click the CRS edit button in bottom panel
    await appPage.clickWidget("Edit CRS");

    const state = await appPage.getAppState();
    expect(state.open_windows).toContain("Change CRS");
    expect(state.open_windows).not.toContain("Welcome");
    expect(state.target_crs).toMatchObject({
      epsg: 3857,
      name: "WGS 84 / Pseudo-Mercator",
    });

    await appPage.expectScreenshot("change-crs-window.png");
  });

  test("open_window and close_window toggle the window", async ({
    appPage,
  }) => {
    let state = await appPage.dispatch([
      { cmd: "close_window", title: "Welcome" },
      { cmd: "open_window", title: "Change CRS" },
    ]);
    expect(state.open_windows).toEqual(["Change CRS"]);

    state = await appPage.dispatch({ cmd: "close_window", title: "Change CRS" });
    expect(state.open_windows).toEqual([]);
  });

  test("changing the CRS reprojects layers", async ({ appPage }) => {
    test.setTimeout(60000);

    let state = await appPage.dispatch([
      { cmd: "load_text", text: SQUARE_GEOJSON, format: "geojson", name: "Square" },
      { cmd: "wait_idle" },
    ]);
    const square = state.layers[0];
    expect(square.name).toBe("Square");
    expect(square.bbox).toEqual({ min_x: 0, min_y: 0, max_x: 10, max_y: 10 });
    // EPSG:3857 x for 10°E.
    expect(square.projected_bbox!.max_x).toBeCloseTo(1113194.9, 0);

    state = await appPage.dispatch([
      { cmd: "change_crs", epsg: 4326 },
      { cmd: "wait_idle" },
    ]);
    expect(state.target_crs).toMatchObject({ epsg: 4326, geographic: true });
    const projected = state.layers[0].projected_bbox!;
    expect(projected.min_x).toBeCloseTo(0, 6);
    expect(projected.max_x).toBeCloseTo(10, 6);
    expect(projected.max_y).toBeCloseTo(10, 6);

    await expect(
      appPage.dispatch({ cmd: "change_crs", epsg: 9999 }),
    ).rejects.toThrow("Unknown EPSG code: 9999");
    expect((await appPage.getAppState()).target_crs?.epsg).toBe(4326);
  });
});
