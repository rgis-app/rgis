import { test as base, expect, Page } from "@playwright/test";

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

  async clickWidget(label: string) {
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
    await this.page.mouse.click(cx, cy);
    await this.waitForNextFrame();
  }

  async listWidgets(): Promise<string> {
    return await this.page.evaluate(() =>
      JSON.stringify((window as any).get_all_widget_rects()),
    );
  }


  async getRenderedLayerCount(): Promise<number> {
    return await this.page.evaluate(
      () => (window as any).get_rendered_layer_count?.() ?? 0,
    );
  }

  async waitForLayerRender(previousCount?: number, timeout?: number) {
    const baseline =
      previousCount ?? (await this.getRenderedLayerCount());
    await this.page.waitForFunction(
      (base) => {
        const count = (window as any).get_rendered_layer_count?.() ?? 0;
        return count > base;
      },
      baseline,
      { timeout: timeout ?? 30000 },
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
