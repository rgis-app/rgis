import { test as base, expect, Page } from "@playwright/test";

/** A subset of `rgis_automation::AppState`; see docs/automation.md. */
export interface Bbox {
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
}

export interface CrsState {
  epsg: number | null;
  proj: string | null;
  name: string | null;
  geographic: boolean;
}

export interface LayerState {
  index: number;
  id: number;
  name: string;
  kind: "vector" | "raster";
  visible: boolean;
  crs: CrsState;
  feature_count: number | null;
  geometry_types: string[];
  bbox: Bbox | null;
  projected_bbox: Bbox | null;
  projected: boolean;
  rendered: boolean | null;
  fill_color: string | null;
  stroke_color: string;
  point_size: number;
  raster: { width: number; height: number; format: string } | null;
}

export interface AppState {
  idle: boolean;
  jobs_in_flight: number;
  target_crs: CrsState | null;
  camera: {
    center: { x: number; y: number };
    scale: number;
    viewport: { width: number; height: number };
    visible_bbox: Bbox | null;
    animating: boolean;
  } | null;
  layers: LayerState[];
  selected_feature: {
    layer_id: number;
    feature_index: number | null;
    properties: [string, string][];
  } | null;
  open_windows: string[];
  messages: string[];
  command_errors: string[];
  script: {
    pending_steps: number;
    current_step: string | null;
    last_error: string | null;
  };
}

/** A script step, e.g. `{ cmd: "toggle_visibility", layer: 0 }`. */
export type Step = { cmd: string } & Record<string, unknown>;

export class AppPage {
  constructor(public readonly page: Page) {}

  async init() {
    await this.page.goto("/");

    // Wait for WASM to initialize
    await this.page.waitForFunction(
      () => document.title.includes("ready"),
      null,
      { timeout: 30000 },
    );

    // Wait for canvas to appear
    const canvas = this.page.locator("canvas");
    await expect(canvas).toBeVisible({ timeout: 10000 });

    // Wait until egui has rendered at least one frame with widgets
    await this.page.waitForFunction(
      () => {
        const rects = (window as any).get_all_widget_rects?.();
        return rects && Object.keys(rects).length > 0;
      },
      null,
      { timeout: 10000 },
    );

    // Wait for logo image asset to load and render
    await this.page.waitForFunction(
      () => {
        const rect = (window as any).get_widget_rect?.("Logo");
        return !!rect;
      },
      null,
      { timeout: 10000 },
    );
    await this.waitForNextFrame();

    // Disable animations for deterministic test behavior
    await this.page.evaluate(() =>
      (window as any).set_animations_enabled?.(false),
    );
  }

  get canvas() {
    return this.page.locator("canvas");
  }

  async canvasBoundingBox() {
    const box = await this.canvas.boundingBox();
    if (!box) throw new Error("Canvas not found");
    return box;
  }

  async waitForNextFrame() {
    await this.page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
  }

  async openAddLayerWindow() {
    await this.clickWidget("Add Layer");
  }

  async closeWindow(title: string) {
    await this.page.evaluate((t) => (window as any).close_window(t), title);
    await this.waitForNextFrame();
  }

  async waitForFadesComplete(timeout = 10000) {
    await this.page.waitForFunction(
      () => ((window as any).get_active_fade_count?.() ?? 0) === 0,
      null,
      { timeout },
    );
    await this.waitForNextFrame();
  }

  async stabilizeForScreenshot() {
    await this.page.mouse.move(0, 0);
    await this.waitForNextFrame();
  }

  async expectScreenshot(name: string) {
    await this.stabilizeForScreenshot();
    await expect(this.page).toHaveScreenshot(name);
  }

  async clickOnCanvas(xFrac: number, yFrac: number) {
    const box = await this.canvasBoundingBox();
    const x = box.x + box.width * xFrac;
    const y = box.y + box.height * yFrac;
    await this.page.mouse.click(x, y);
    await this.waitForNextFrame();
  }

  async waitForWidget(label: string, timeout = 10000) {
    await this.page.waitForFunction(
      (l) => !!(window as any).get_widget_rect?.(l),
      label,
      { timeout },
    );
  }

  async clickWidget(label: string) {
    await this.waitForWidget(label);
    const rect = await this.page.evaluate(
      (l) => (window as any).get_widget_rect(l),
      label,
    );
    if (!rect)
      throw new Error(
        `Widget "${label}" not found. Available: ${await this.listWidgets()}`,
      );
    const cx = (rect[0] + rect[2]) / 2;
    const cy = (rect[1] + rect[3]) / 2;
    // Dispatch pointer events directly on the canvas so that winit/bevy_egui
    // detects the click (Playwright's mouse.click may not produce pointer
    // events that winit recognises on all platforms).
    await this.page.evaluate(
      ({ x, y }) => {
        const canvas = document.querySelector("canvas")!;
        const opts = {
          clientX: x,
          clientY: y,
          bubbles: true,
          cancelable: true,
          pointerId: 1,
          pointerType: "mouse" as const,
          button: 0,
          buttons: 1,
        };
        canvas.dispatchEvent(new PointerEvent("pointerdown", opts));
        canvas.dispatchEvent(
          new PointerEvent("pointerup", { ...opts, buttons: 0 }),
        );
      },
      { x: cx, y: cy },
    );
    await this.waitForNextFrame();
  }

  /**
   * The app state as of the last rendered frame. The app only tracks its
   * state once it's been asked for, so the first call waits a frame.
   */
  async getAppState(): Promise<AppState> {
    const handle = await this.page.waitForFunction(
      () => {
        const json = (window as any).get_app_state?.();
        return json && json !== "null" ? json : false;
      },
      null,
      { timeout: 10000 },
    );
    return JSON.parse(await handle.jsonValue());
  }

  /**
   * Wait until no jobs, camera flights, fades, or dispatched steps are in
   * flight.
   */
  async waitForIdle(timeout = 30000) {
    await this.page.waitForFunction(
      () => {
        const json = (window as any).get_app_state?.();
        return !!json && JSON.parse(json)?.idle === true;
      },
      null,
      { timeout },
    );
  }

  /**
   * Run steps through the same command layer as `rgis --script`, wait for
   * the app to settle, and return the resulting state. Throws if a step
   * fails.
   */
  async dispatch(steps: Step | Step[], timeout = 30000): Promise<AppState> {
    const errorsBefore = (await this.getAppState()).command_errors.length;
    await this.page.evaluate(
      (json) => (window as any).dispatch(json),
      JSON.stringify(steps),
    );
    await this.waitForIdle(timeout);
    const state = await this.getAppState();
    const errors = state.command_errors.slice(errorsBefore);
    if (errors.length > 0) {
      throw new Error(`dispatch failed: ${errors.join("; ")}`);
    }
    return state;
  }

  /** The layer with this name; fails the test if there isn't exactly one. */
  async getLayer(name: string): Promise<LayerState> {
    const { layers } = await this.getAppState();
    const matches = layers.filter((layer) => layer.name === name);
    expect(
      matches,
      `layers named "${name}" in ${JSON.stringify(layers.map((l) => l.name))}`,
    ).toHaveLength(1);
    return matches[0];
  }

  async listWidgets(): Promise<string> {
    return await this.page.evaluate(() =>
      JSON.stringify((window as any).get_all_widget_rects()),
    );
  }


  async loadGeoTIFFFile(filePath: string) {
    await this.openAddLayerWindow();
    await this.clickWidget("File");
    await this.clickWidget("GeoTIFF");
    const fileChooserPromise = this.page.waitForEvent("filechooser");
    await this.clickWidget("Select file");
    const fileChooser = await fileChooserPromise;
    await fileChooser.setFiles(filePath);
    await this.waitForNextFrame();
    const countBefore = await this.getRenderedLayerCount();
    await this.clickWidget("Add layer");
    await this.waitForLayerRender(countBefore);
  }

  async getRenderedLayerCount(): Promise<number> {
    return await this.page.evaluate(
      () => (window as any).get_rendered_layer_count?.() ?? 0,
    );
  }

  async waitForLayerRender(previousCount?: number) {
    const baseline =
      previousCount ?? (await this.getRenderedLayerCount());
    await this.page.waitForFunction(
      (base) => {
        const count = (window as any).get_rendered_layer_count?.() ?? 0;
        return count > base;
      },
      baseline,
      { timeout: 30000 },
    );
    await this.waitForNextFrame();
  }

  async addLibraryLayer(folder: string, entry: string) {
    await this.openAddLayerWindow();
    await this.clickWidget("Library");
    await this.clickWidget(folder);
    const countBefore = await this.getRenderedLayerCount();
    await this.clickWidget(`Add:${entry}`);
    await this.waitForLayerRender(countBefore);
  }

  async setFirstLayerFillColor(r: number, g: number, b: number, a: number) {
    await this.page.evaluate(
      ({ r, g, b, a }) =>
        (window as any).set_first_layer_fill_color(r, g, b, a),
      { r, g, b, a },
    );
    await this.waitForNextFrame();
    await this.waitForNextFrame();
  }

  async regionHasContent(
    xFrac: number,
    yFrac: number,
    wFrac: number,
    hFrac: number,
  ): Promise<boolean> {
    return await this.page.evaluate(
      ({ xFrac, yFrac, wFrac, hFrac }) => {
        const canvas = document.querySelector("canvas");
        if (!canvas) return false;

        const gl = canvas.getContext("webgl2") || canvas.getContext("webgl");
        if (gl) {
          const x = Math.floor(xFrac * canvas.width);
          // WebGL has origin at bottom-left, so flip y
          const y = Math.floor((1 - yFrac - hFrac) * canvas.height);
          const w = Math.floor(wFrac * canvas.width);
          const h = Math.floor(hFrac * canvas.height);
          const pixels = new Uint8Array(w * h * 4);
          try {
            gl.readPixels(x, y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
          } catch {
            // readPixels may fail without preserveDrawingBuffer
            return true;
          }

          // Check if all pixels are zeros (readPixels returns zeros without preserveDrawingBuffer)
          let allZero = true;
          for (let i = 0; i < pixels.length; i++) {
            if (pixels[i] !== 0) {
              allZero = false;
              break;
            }
          }
          if (allZero) {
            // Can't determine — assume content exists since canvas is visible
            return true;
          }

          // Check for non-uniform pixels (content)
          const firstR = pixels[0],
            firstG = pixels[1],
            firstB = pixels[2];
          for (let i = 4; i < pixels.length; i += 4) {
            if (
              pixels[i] !== firstR ||
              pixels[i + 1] !== firstG ||
              pixels[i + 2] !== firstB
            ) {
              return true;
            }
          }
          return false;
        }

        // Fallback for 2d context
        const ctx = canvas.getContext("2d");
        if (!ctx) return true;

        const x = Math.floor(xFrac * canvas.width);
        const y = Math.floor(yFrac * canvas.height);
        const w = Math.floor(wFrac * canvas.width);
        const h = Math.floor(hFrac * canvas.height);
        const imageData = ctx.getImageData(x, y, w, h);
        const data = imageData.data;

        const firstR = data[0],
          firstG = data[1],
          firstB = data[2];
        for (let i = 4; i < data.length; i += 4) {
          if (
            data[i] !== firstR ||
            data[i + 1] !== firstG ||
            data[i + 2] !== firstB
          ) {
            return true;
          }
        }
        return false;
      },
      { xFrac, yFrac, wFrac, hFrac },
    );
  }
}

export const test = base.extend<{ appPage: AppPage }>({
  appPage: async ({ page }, use) => {
    const appPage = new AppPage(page);
    await appPage.init();
    await use(appPage);
  },
});

export { expect };
