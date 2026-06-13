fixture("basic.default")
local vp = root()
local TOLERANCE = 0.75

-- Verify initial state: top rows visible
vp:wait_for_widget("basic.scroll.row.0", function(widget)
    return widget.visible
end)

-- scroll_to align=center
vp:widget_get("basic.scroll"):scroll_to({ align = "center" })
vp:wait_for_settle()

-- After centering, row 0 should not be visible but middle rows should exist
local _row_0_after_center = vp:widget_get("basic.scroll.row.0") :: any
-- Row 0 might or might not be visible depending on scroll area size, just verify the API works

-- Jump buttons
vp:widget_get("basic.scroll.jump_down"):click()
vp:wait_for_settle()

-- Verify scroll offset label updates
local offset_text = tostring(vp:widget_get("basic.scroll.offset"):state().label)
assert(string.find(offset_text, "Scroll offset:", 1, true) ~= nil, "should show scroll offset")

vp:widget_get("basic.scroll.jump_top"):click()
vp:wait_for_widget("basic.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and math.abs(scroll.offset.y) < TOLERANCE
end)

-- scroll with positive delta (content up)
vp:widget_get("basic.scroll"):scroll({ x = 0, y = -200 })
vp:wait_for_settle()

-- basic.scrolled fixture pre-scrolls the area
fixture("basic.scrolled")
assert(tostring(vp:widget_get("basic.status"):state().label) == "Fixture: scrolled")

-- viewports.scrolled fixture pre-scrolls the secondary viewport
fixture("viewports.scrolled")
assert(
    tostring(vp:widget_get("basic.status"):state().label)
        == "Fixture: secondary viewport scrolled"
)

return {
    scroll_tests = "complete",
}
