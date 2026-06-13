--!strict

-- eguidev exposes an egui automation surface to Luau through the `script_eval`
-- MCP tool. Scripts run inside the instrumented app process, against the latest
-- captured frame, so one script can inspect widgets, queue input, wait for
-- state changes, apply fixtures, and capture screenshots without extra round
-- trips.
-- The script's final return value becomes JSON in the MCP result. Logs,
-- assertions, and returned images are recorded alongside it.
-- When the same script is run via `edev smoke`, the smoke runner ignores the
-- final return value and only uses thrown errors/assertions plus logs.
--
-- This file is the canonical scripting reference. `script_api` and
-- `edev --script-docs` return this file verbatim.
--
-- Model expectations:
-- - Write strict Luau. Inline `script_eval` scripts are strict even when the
--   host omits `--!strict`.
-- - Treat widget ids as the one canonical selector in the API.
-- - Prefer explicit ids when you care about meaning, reuse, or stable scripts.
-- - Explicit widget ids must be unique within a captured frame. Duplicate
--   explicit ids block automation until instrumentation is fixed.
-- - Generated ids are opaque tokens. They are best-effort stable within the
--   current app session, but they are not expected to survive restart.
-- - Widget discovery returns handles. Read current frame data through
--   `widget:state()`.
-- - Prefer `fixture()`, waits, and assertions over ad-hoc retries.
-- - High-level actions auto-settle unless you disable settling explicitly.
-- - Return `ImageRef` values from `screenshot()` when you want image blocks in
--   the MCP response.
-- - The sandbox has no filesystem access, no network access, and no module
--   imports.

export type Pos = {
    x: number,
    y: number,
}

export type Vec = {
    x: number,
    y: number,
}

export type Rect = {
    min: Pos,
    max: Pos,
}

export type Modifiers = {
    ctrl: boolean?,
    shift: boolean?,
    alt: boolean?,
    command: boolean?,
}

export type ImageRef = {
    type: "image_ref",
    id: string,
}

export type ScriptArgValue = string | number | boolean

export type ScriptArgs = { [string]: ScriptArgValue }

declare args: ScriptArgs

export type WidgetRole =
    "button"
    | "link"
    | "image"
    | "label"
    | "text_edit"
    | "slider"
    | "checkbox"
    | "combo_box"
    | "radio"
    | "drag_value"
    | "toggle"
    | "selectable"
    | "separator"
    | "spinner"
    | "scroll_area"
    | "menu_button"
    | "collapsing_header"
    | "window"
    | "progress_bar"
    | "color_picker"
    | "unknown"

--- Widget values match the widget role: booleans for checkboxes/toggles,
--- numbers for sliders/drag values/combo boxes (zero-based index), strings
--- for text edits.
export type WidgetValue = boolean | number | string

export type LayoutOverflow = boolean

export type WidgetLayout = {
    --- Size the widget requested before layout constraints were applied.
    desired_size: Vec,
    --- Size actually assigned to the widget by the layout engine.
    actual_size: Vec,
    --- Clip rectangle that bounds the visible area for this widget.
    clip_rect: Rect,
    --- Whether any part of the widget falls outside the clip rect.
    clipped: boolean,
    --- Whether the widget extends beyond its allocated layout slot.
    overflow: boolean,
    --- Rectangle available to the widget before it was laid out.
    available_rect: Rect,
    --- Fraction of the widget's area that is visible after clipping (0..1).
    visible_fraction: number,
}

export type ScrollAreaMeta = {
    offset: Vec,
    viewport_size: Vec,
    content_size: Vec,
    max_offset: Vec,
}

export type Range = {
    min: number,
    max: number,
}

-- Called on each poll with the latest resolved widget state. The predicate is
-- only called when the widget is present; when the widget is missing, the
-- predicate is skipped and treated as not satisfied. Use
-- `wait_for_widget_absent` / `wait_for_absent` for disappearance waits.
export type WidgetWaitPredicate = (widget: WidgetState) -> boolean

-- Called on each poll with the latest viewport snapshot.
export type ViewportWaitPredicate = (viewport: ViewportState) -> boolean

-- Widgets are returned by `widget_get`, `widget_list`, and other discovery
-- functions. All interactions (click, type, drag, ...) are methods on Widget.
export type Widget = {
    id: string,
    viewport_id: string,
    viewport: (self: Widget) -> Viewport,
    click: (self: Widget, options: ActionOptions?) -> (),
    hover: (self: Widget, options: HoverOptions?) -> (),
    type_text: (self: Widget, text: string, options: TypeTextOptions?) -> (),
    focus: (self: Widget) -> (),
    --- Set the widget's value. Booleans for checkboxes/toggles, numbers for
    --- sliders/drag values/combo boxes (zero-based index), strings for text
    --- edits.
    set_value: (self: Widget, value: WidgetValue, options: ActionOptions?) -> (),
    drag: (self: Widget, to: Pos, options: ActionOptions?) -> (),
    drag_relative: (self: Widget, relative: Vec, from: Vec?) -> (),
    drag_to: (self: Widget, to: Widget, options: ActionOptions?) -> (),
    scroll: (self: Widget, delta: Vec, options: ActionOptions?) -> (),
    scroll_to: (self: Widget, options: ScrollToOptions?) -> (),
    --- Scroll ancestor scroll areas just enough to reveal this widget.
    --- No-op when the widget has no scroll-area ancestor.
    scroll_into_view: (self: Widget, options: ActionOptions?) -> (),
    state: (self: Widget) -> WidgetState,
    parent: (self: Widget) -> Widget?,
    children: (self: Widget) -> { Widget },
    text_measure: (self: Widget) -> TextMeasure,
    -- Checks this widget and its descendants for layout problems.
    check_layout: (self: Widget) -> { LayoutIssue },
    screenshot: (self: Widget) -> ImageRef,
    --- Show a persistent stroke overlay around this widget's interact_rect.
    --- The highlight remains visible until hide_highlight() is called. Calling
    --- show_highlight again on the same widget replaces the previous highlight.
    show_highlight: (self: Widget, color: string) -> HighlightResult,
    --- Remove the highlight for this widget. No-op if the widget is not
    --- currently highlighted.
    hide_highlight: (self: Widget) -> (),
    --- Show a debug overlay scoped to this widget's subtree.
    show_debug_overlay: (self: Widget, mode: OverlayDebugMode?, options: OverlayDebugOptions?) -> (),
    --- Hide the debug overlay for this widget's subtree.
    hide_debug_overlay: (self: Widget) -> (),
    --- Poll this widget until `predicate` returns true.
    --- The predicate receives the latest widget snapshot. When the widget is
    --- missing, the predicate is skipped (treated as false).
    --- Returns the terminal widget snapshot on success.
    --- Throws a timeout error if the predicate never matches.
    wait_for: (self: Widget, predicate: WidgetWaitPredicate) -> WidgetState,
    --- Poll until this widget both exists and is visible.
    --- Returns the terminal widget snapshot on success.
    --- Throws a timeout error if the widget never becomes visible.
    wait_for_visible: (self: Widget) -> WidgetState,
    --- Wait until this widget's scroll_state is initialized and stable across captures.
    --- Returns the terminal widget snapshot on success.
    wait_for_scroll_ready: (self: Widget) -> WidgetState,
    --- Poll until this widget is no longer present.
    --- Throws a timeout error if the widget does not disappear in time.
    wait_for_absent: (self: Widget) -> (),
}

export type WidgetState = {
    rect: Rect,
    interact_rect: Rect,
    role: WidgetRole,
    label: string?,
    value: WidgetValue?,
    --- String representation of value. Empty string when value is nil.
    --- Bool → "true"/"false", Float → decimal, Int → decimal, Text → verbatim.
    value_text: string,
    layout: WidgetLayout?,
    --- scroll_area only.
    scroll_state: ScrollAreaMeta?,
    --- slider, drag_value only.
    range: Range?,
    --- combo_box only.
    options: { string }?,
    --- button only when selected-aware metadata is recorded.
    selected: boolean?,
    --- checkbox only when indeterminate-aware metadata is recorded.
    indeterminate: boolean?,
    --- text_edit only.
    multiline: boolean?,
    --- text_edit only. True when input is masked.
    password: boolean?,
    enabled: boolean,
    visible: boolean,
    focused: boolean,
}

export type ViewportState = {
    title: string?,
    outer_pos: Pos?,
    outer_size: Vec?,
    inner_size: Vec,
    focused: boolean,
    minimized: boolean?,
    maximized: boolean?,
    fullscreen: boolean?,
    frame_count: number,
    pixels_per_point: number,
    pointer_pos: Pos?,
}

export type Viewport = {
    id: string,
    state: (self: Viewport) -> ViewportState,
    widget_list: (self: Viewport, options: WidgetListOptions?) -> { Widget },
    widget_get: (self: Viewport, id: string) -> Widget,
    --- Returns the topmost widget at the given point. The widget's ancestors
    --- also occupy the point but are beneath it in the z-order. With
    --- `all_layers`, returns all overlapping widgets sorted topmost-first.
    widget_at_point: (self: Viewport, pos: Pos, all_layers: boolean?) -> { Widget },
    wait_for_widget: (
        self: Viewport,
        id: string,
        predicate: WidgetWaitPredicate
    ) -> WidgetState,
    --- Poll until the widget with the given id both exists and is visible.
    --- Returns the terminal widget snapshot on success.
    --- Throws a timeout error if the widget never becomes visible.
    wait_for_widget_visible: (self: Viewport, id: string) -> WidgetState,
    --- Poll until the widget with the given id is no longer present.
    --- Throws a timeout error if the widget does not disappear in time.
    wait_for_widget_absent: (self: Viewport, id: string) -> (),
    --- Wait until the viewport has processed pending actions and commands and
    --- observed a fresh frame.
    --- Throws a timeout error if the viewport does not settle in time.
    wait_for_settle: (self: Viewport) -> (),
    --- Wait until this viewport has produced a fresh captured snapshot.
    --- Throws a timeout error if no new capture arrives in time.
    wait_for_capture: (self: Viewport) -> (),
    --- Poll this viewport until `predicate` returns true.
    --- The predicate receives the latest viewport snapshot on each poll.
    --- Returns the terminal viewport snapshot that satisfied the predicate.
    --- Throws a timeout error if the predicate never matches.
    wait_for: (self: Viewport, predicate: ViewportWaitPredicate) -> ViewportState,
    --- Simulate a full key press-release cycle. The key argument is a combo
    --- string: optional modifiers joined by "-", then the key name.
    ---
    --- Modifiers (case-insensitive): ctrl, shift, alt, cmd (alias: command).
    --- Key names are case-insensitive for multi-character names (enter, ArrowUp,
    --- ESCAPE all work). Single characters are case-sensitive (a ≠ A).
    ---
    --- Categories of key names:
    ---   Navigation: up, down, left, right, home, end, pageup, pagedown
    ---   Editing:    enter, tab, space, backspace, delete, insert, escape
    ---   Letters:    a-z, A-Z (case-sensitive)
    ---   Digits:     0-9, num0-num9
    ---   Function:   f1-f35
    ---   Punctuation: minus(-), plus(+), equals(=), comma(,), period(.),
    ---               colon(:), semicolon(;), slash(/), backslash(\), pipe(|),
    ---               quote('), backtick(`), openbracket([), closebracket(])
    ---
    --- Examples: "enter", "ctrl-a", "shift-tab", "ctrl-shift-z", "cmd-s"
    key: (self: Viewport, key: string, options: KeyOptions?) -> (),
    --- Paste text into the focused widget via a clipboard paste event.
    paste: (self: Viewport, text: string, options: ActionOptions?) -> (),
    --- Inject a raw pointer move event (positions in egui points).
    raw_pointer_move: (self: Viewport, pos: Pos) -> (),
    --- Inject a raw pointer button event. For normal clicking, use
    --- Widget:click() instead.
    raw_pointer_button: (
        self: Viewport,
        pos: Pos,
        button: "primary" | "secondary" | "middle",
        action: RawInputAction,
        modifiers: Modifiers?
    ) -> (),
    --- Inject a single raw key event (press or release). No text event is
    --- generated and no press/release pairing is done. For normal key
    --- simulation, use Viewport:key() instead.
    raw_key: (
        self: Viewport,
        key: string,
        action: RawInputAction,
        modifiers: Modifiers?
    ) -> (),
    --- Inject a raw text input event (as if from an IME).
    raw_text: (self: Viewport, text: string) -> (),
    --- Inject a raw scroll event. Delta is in egui points.
    raw_scroll: (self: Viewport, delta: Vec, modifiers: Modifiers?) -> (),
    --- Request OS-level window focus. Only use this for testing focus events
    --- themselves; all other automation works without OS focus.
    focus: (self: Viewport) -> (),
    set_inner_size: (self: Viewport, size: Vec) -> (),
    set_resize_options: (self: Viewport, options: ResizeOptions) -> (),
    screenshot: (self: Viewport) -> ImageRef,
    -- Checks the current viewport for layout problems.
    check_layout: (self: Viewport) -> { LayoutIssue },
    --- Show a persistent highlight stroke around the given rect. The highlight
    --- remains visible until hide_highlight() is called. Calling show_highlight
    --- again with the same rect replaces the previous highlight; different rects
    --- coexist.
    show_highlight: (self: Viewport, rect: Rect, color: string) -> HighlightResult,
    --- Remove all highlights (both widget and rect-based).
    hide_highlight: (self: Viewport) -> (),
    -- Enable a persistent debug overlay that visualizes widget layout on this
    -- viewport. Painted every frame until hide_debug_overlay() is called.
    -- Always starts from default settings; pass all desired options each call.
    show_debug_overlay: (
        self: Viewport,
        mode: OverlayDebugMode?,
        options: OverlayDebugOptions?
    ) -> (),
    hide_debug_overlay: (self: Viewport) -> (),
}

-- High-level actions auto-settle by default. Use `configure()` to change
-- global timeout, poll interval, and settle behavior.
export type ActionOptions = {
    settle: boolean?,
}

-- `widget_list` returns the full filtered set for the current frame snapshot.
-- `id_prefix` matches the canonical widget id, whether that id was explicit in
-- instrumentation or generated by eguidev.
export type WidgetListOptions = {
    include_invisible: boolean?,
    role: WidgetRole?,
    id_prefix: string?,
}

export type HoverOptions = {
    settle: boolean?,
    position: Vec?,
    duration_ms: number?,
}

export type TypeTextOptions = {
    settle: boolean?,
    clear: boolean?,
    enter: boolean?,
}

-- scroll_to targets a scroll area widget. Use offset for explicit coordinates or align for
-- a coarse jump within the scroll area.
export type ScrollToOptions = {
    settle: boolean?,
    offset: Vec?,
    align: "top" | "center" | "bottom"?,
}

export type KeyOptions = {
    settle: boolean?,
    repeat_count: number?,
    target: string?,
}

--- "press" or "release" — used by raw input injection methods.
export type RawInputAction = "press" | "release"

-- Closed set of issue kinds returned by `check_layout()`.
export type LayoutIssueKind =
    "overlap"
    | "clipping"
    | "overflow"
    | "zero_size"
    | "text_truncation"
    | "offscreen"

-- Selects what the persistent debug overlay visualizes:
--
-- - "bounds"     — stroke around every widget rect.
-- - "margins"    — bounds + available_rect, showing the gap between layout
--                  slot and actual widget.
-- - "clipping"   — bounds + clipped_rect, revealing partial/full clipping.
-- - "overlaps"   — highlights intersection rects of sibling widgets that
--                  share a parent.
-- - "focus"      — bounds for focused widgets only.
-- - "layers"     — bounds with egui layer id in the label.
-- - "containers" — bounds for widgets that have children only, hiding leaves.
export type OverlayDebugMode =
    "bounds"
    | "margins"
    | "clipping"
    | "overlaps"
    | "focus"
    | "layers"
    | "containers"

-- Unset fields use defaults. Colors are hex "#RRGGBB" or "#RRGGBBAA".
export type OverlayDebugOptions = {
    -- Defaults to true.
    show_labels: boolean?,
    -- Append pixel dimensions to labels. No effect when show_labels is false.
    -- Defaults to true.
    show_sizes: boolean?,
    -- Label font size in logical points. Defaults to 10.
    label_font_size: number?,
    -- Defaults to semi-transparent green.
    bounds_color: string?,
    -- Used by "clipping" and "margins" modes. Defaults to semi-transparent red.
    clip_color: string?,
    -- Used by "overlaps" mode. Defaults to semi-transparent yellow.
    overlap_color: string?,
}

export type ResizeOptions = {
    min_size: Vec?,
    max_size: Vec?,
    increments: Vec?,
    resizable: boolean?,
}

export type HighlightResult = {
    -- The actual rect that was highlighted (useful when the widget target was
    -- used rather than an explicit rect).
    rect: Rect,
}

-- A single semantic layout problem reported by `check_layout()`.
export type LayoutIssue = {
    kind: LayoutIssueKind,
    widgets: { string },
    message: string,
    -- Overlaps use the intersection rect. Other issue kinds use the primary
    -- widget rect when one is useful.
    rect: Rect?,
}

export type TextMeasureLine = {
    --- The text content of this line.
    text: string,
    --- The rendered width of this line in logical points.
    width: number,
}

export type TextMeasure = {
    --- The full text string of the widget.
    text: string,
    --- The portion of text actually visible after layout and clipping.
    visible_text: string,
    --- The size the text would need if unconstrained.
    desired_size: Vec,
    --- The size the text actually occupies after layout.
    actual_size: Vec,
    --- Height of a single text line in logical points.
    line_height: number,
    --- Per-line breakdown of text content and rendered width.
    lines: { TextMeasureLine },
    --- Whether the text was truncated and an ellipsis character was inserted.
    ellipsis: boolean,
}

export type Fixture = {
    name: string,
    description: string,
    anchors: { Anchor },
}

export type AnchorCheck =
    "Visible"
    | "ScrollReady"
    | { Label: string }
    | { Value: WidgetValue }
    | { ScrollAt: { offset: Vec, tolerance: number } }

export type Anchor = {
    widget_id: string,
    viewport_id: string?,
    check: AnchorCheck,
}

export type ScriptOptions = {
    timeout_ms: number?,
    poll_interval_ms: number?,
    settle: boolean?,
}

-- Logged values are recorded in the script result payload.
declare function log(message: any): ()

--- Configure global script settings: timeout, poll interval, and auto-settle.
--- Unset fields keep their current values.
declare function configure(options: ScriptOptions): ()

--- Wait for at least `count` more frames to be observed.
--- Returns the ending frame count on success.
--- Throws a timeout error if the requested frames are not observed in time.
declare function wait_for_frames(count: number?): number
--- Wait until the root viewport has produced a fresh captured snapshot.
declare function wait_for_capture(): ()
declare function root(): Viewport
declare function viewports(): { Viewport }

--- Apply a registered fixture and wait until its readiness anchors match fresh captures.
--- Widget and viewport handles resolve fresh across fixture boundaries, so rebinding `root()`
--- after each fixture is usually unnecessary.
declare function fixture(name: string): ()
--- Apply a registered fixture without waiting for readiness.
declare function fixture_raw(name: string): ()
-- Fixtures are independent baseline setups and must be safe to invoke in any order.
declare function fixtures(): { Fixture }

-- Assertions record pass/fail metadata in the result payload and throw on failure.
declare function assert(condition: boolean, message: string?): ()
declare function assert_widget_exists(id: string): ()
