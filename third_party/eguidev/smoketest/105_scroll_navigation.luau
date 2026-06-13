fixture("basic.default")

local vp = root()
local TOLERANCE = 0.75

local function scroll_meta(id: string): ScrollAreaMeta
    local scroll = vp:widget_get(id):state().scroll_state
    assert(scroll ~= nil, id .. " should expose scroll_state")
    return scroll :: ScrollAreaMeta
end

local function approx_eq(actual: number, expected: number, label: string)
    assert(
        math.abs(actual - expected) <= TOLERANCE,
        string.format("%s (got %.2f, expected %.2f)", label, actual, expected)
    )
end

vp:widget_get("basic.scroll"):wait_for_scroll_ready()

local basic_initial = scroll_meta("basic.scroll")
local basic_max = basic_initial.max_offset.y
approx_eq(basic_initial.offset.y, 0, "basic.scroll should start at top")

vp:widget_get("basic.scroll"):scroll_to({ align = "bottom", settle = false })
vp:wait_for_widget("basic.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and math.abs(scroll.offset.y - basic_max) <= TOLERANCE
end)
vp:wait_for_settle()
local basic_bottom = scroll_meta("basic.scroll")
approx_eq(basic_bottom.offset.y, basic_max, "basic.scroll bottom align should reach max offset")
assert(not vp:widget_get("basic.scroll.row.0"):state().visible, "bottom align should hide the first row")

vp:widget_get("basic.scroll"):scroll_to({ align = "center", settle = false })
vp:wait_for_widget("basic.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and math.abs(scroll.offset.y - (basic_max * 0.5)) <= TOLERANCE
end)
vp:wait_for_settle()
local basic_center = scroll_meta("basic.scroll")
approx_eq(
    basic_center.offset.y,
    basic_max * 0.5,
    "basic.scroll center align should land at half the max offset"
)

vp:widget_get("basic.scroll"):scroll_to({ align = "top", settle = false })
vp:wait_for_widget("basic.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and math.abs(scroll.offset.y) <= TOLERANCE
end)
vp:wait_for_settle()
local basic_top = scroll_meta("basic.scroll")
approx_eq(basic_top.offset.y, 0, "basic.scroll top align should return to origin")

vp:widget_get("basic.scroll"):scroll_to({ offset = { x = 0, y = -50 } })
vp:wait_for_settle()
local basic_negative = scroll_meta("basic.scroll")
approx_eq(basic_negative.offset.y, 0, "basic.scroll should clamp negative offsets to zero")

vp:widget_get("basic.scroll"):scroll_to({ offset = { x = 0, y = basic_max + 500 } })
vp:wait_for_settle()
local basic_clamped = scroll_meta("basic.scroll")
approx_eq(basic_clamped.offset.y, basic_max, "basic.scroll should clamp oversized offsets to max")

local root_initial = scroll_meta("basic.root_scroll")
local root_max = root_initial.max_offset.y
approx_eq(root_initial.offset.y, 0, "basic.root_scroll should start at top")

fixture("basic.default")
local status_basic_before = scroll_meta("basic.scroll").offset.y
local status_root_before = scroll_meta("basic.root_scroll").offset.y
vp:widget_get("basic.status"):scroll_into_view()
vp:wait_for_settle()
local status_basic_after = scroll_meta("basic.scroll").offset.y
local status_root_after = scroll_meta("basic.root_scroll").offset.y
approx_eq(status_basic_after, status_basic_before, "scroll_into_view without scroll ancestors should be a no-op for basic.scroll")
approx_eq(status_root_after, status_root_before, "scroll_into_view without scroll ancestors should be a no-op for basic.root_scroll")

fixture("basic.default")
vp:widget_get("basic.scroll.row.0"):scroll_into_view()
local row0_again = vp:widget_get("basic.scroll.row.0"):wait_for_visible()
assert(row0_again.visible, "scroll_into_view on an already visible row should keep it visible")
vp:wait_for_settle()
approx_eq(
    scroll_meta("basic.scroll").offset.y,
    0,
    "scroll_into_view on an already visible first row should leave the inner scroll at zero"
)

return {
    basic_scroll_max = basic_max,
    root_scroll_max = root_max,
    root_scroll_offset = root_initial.offset.y,
}
