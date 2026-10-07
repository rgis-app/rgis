import { test, expect } from "./fixtures/app-fixture";

test("grid lines persist after zoom in", async ({ appPage }) => {
  await appPage.expectScreenshot("grid-before-zoom.png");

  await appPage.clickWidget("Zoom In");
  await appPage.page.waitForTimeout(500);
  await appPage.expectScreenshot("grid-after-zoom-in.png");
});

test("grid lines persist after zoom out", async ({ appPage }) => {
  await appPage.clickWidget("Zoom Out");
  await appPage.page.waitForTimeout(500);
  await appPage.expectScreenshot("grid-after-zoom-out.png");
});

// Regression test for #283: zooming far out used to generate a grid line and
// label every 180° across the whole view, freezing the app.
test("zooming far out doesn't freeze the app", async ({ appPage }) => {
  for (const epsg of [3857, 4326]) {
    const before = await appPage.dispatch([
      { cmd: "change_crs", epsg },
      { cmd: "wait_idle" },
    ]);
    const after = await appPage.dispatch({ cmd: "zoom", factor: 1e-12 });
    expect(after.camera!.scale / before.camera!.scale).toBeCloseTo(1e12, -6);
  }
});
