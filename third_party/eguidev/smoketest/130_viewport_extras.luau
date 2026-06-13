fixture("viewports.default")

local root_vp = root()

-- List viewports and find secondary
local vp_list = viewports()
assert(#vp_list >= 2, "should have root and secondary viewports")
local secondary: Viewport? = nil
for _, viewport in ipairs(vp_list) do
    if viewport.id ~= "root" then
        secondary = viewport
        break
    end
end
assert(secondary ~= nil, "secondary viewport should be present")
local secondary_vp = secondary :: Viewport

local root_info = root_vp:state()
assert(root_info.inner_size.x > 0 and root_info.inner_size.y > 0, "root viewport should be ready")

secondary_vp:focus()
secondary_vp:widget_get("viewports.scroll"):wait_for_scroll_ready()

-- scroll_to in secondary viewport
secondary_vp:widget_get("viewports.scroll"):scroll_to({
    align = "top",
    settle = false,
})
secondary_vp:wait_for_widget("viewports.row.0", function(widget)
    return widget.visible
end)
secondary_vp:wait_for_widget("viewports.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and math.abs(scroll.offset.y) < 0.75
end)

-- Scroll to bottom in secondary viewport
local open_status = root_vp:widget_get("viewports.open"):state()
assert(string.find(open_status.value_text, "true", 1, true) ~= nil,
    "secondary should be open")

-- viewports.scrolled fixture
fixture("viewports.scrolled")
secondary_vp:wait_for_widget("viewports.scroll", function(widget)
    local scroll = widget.scroll_state
    return scroll ~= nil and scroll.offset.y > 0
end)
assert(
    tostring(root_vp:widget_get("basic.status"):state().label)
        == "Fixture: secondary viewport scrolled"
)

return {
    viewport_id = secondary_vp.id,
    viewports_count = #vp_list,
}
