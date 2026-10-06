import init, { run, get_widget_rect, get_all_widget_rects, get_rendered_layer_count, get_active_fade_count, set_animations_enabled, close_window, set_first_layer_fill_color, get_app_state, dispatch } from "rgis-pkg/rgis.js";

async function fetchWasmWithProgress() {
    const bar = document.getElementById("loading-bar");
    const text = document.getElementById("loading-text");
    const response = await fetch("/rgis_bg.wasm");
    const contentLength = response.headers.get("Content-Length");
    const total = contentLength ? parseInt(contentLength, 10) : 0;
    const reader = response.body.getReader();
    const chunks = [];
    let received = 0;

    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.length;
        if (total) {
            const pct = Math.min(100, Math.round((received / total) * 100));
            bar.style.width = pct + "%";
            text.textContent = pct + "%";
        } else {
            // Indeterminate: pulse the bar
            bar.style.width = "60%";
            bar.style.animation = "pulse 1.5s ease-in-out infinite";
            text.textContent = (received / 1024 / 1024).toFixed(1) + " MB";
        }
    }

    bar.style.width = "100%";
    text.textContent = "Initializing…";

    const wasmBytes = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) {
        wasmBytes.set(chunk, offset);
        offset += chunk.length;
    }
    return wasmBytes.buffer;
}

async function main() {
    const wasmBuffer = await fetchWasmWithProgress();
    await init(wasmBuffer);
    document.getElementById("loading-overlay").style.display = "none";
    // Expose widget position functions for Playwright tests
    window.get_widget_rect = get_widget_rect;
    window.get_all_widget_rects = get_all_widget_rects;
    window.get_rendered_layer_count = get_rendered_layer_count;
    window.get_active_fade_count = get_active_fade_count;
    window.set_animations_enabled = set_animations_enabled;
    window.close_window = close_window;
    window.set_first_layer_fill_color = set_first_layer_fill_color;
    // App state as JSON and semantic commands; see docs/automation.md
    window.get_app_state = get_app_state;
    window.dispatch = dispatch;
    // Signal that WASM has loaded and initialized
    document.title = "rgis - ready";
    // Call the run function
    run();
}

main().catch(console.error);
