fixture("basic.default")
local vp = root()

-- Separator widgets exist
assert_widget_exists("basic.separator.primary")
assert_widget_exists("basic.separator.scroll")
assert_widget_exists("viewports.separator.primary")

local sep = vp:widget_get("basic.separator.primary")
assert(sep:state().role == "separator")

-- Spinner widget
assert_widget_exists("basic.spinner.loading")
local spinner = vp:widget_get("basic.spinner.loading")
local spinner_rect = spinner:state().rect
assert(spinner_rect.max.x > spinner_rect.min.x, "spinner should have width")

-- Image widget
assert_widget_exists("basic.preview.image")
local image = vp:widget_get("basic.preview.image")
assert(image:state().role == "image")

-- Link clicks update counter
local before_text = tostring(vp:widget_get("basic.links.count"):state().label)
assert(before_text == "Link clicks: 0")

vp:widget_get("basic.link.docs"):click()
vp:wait_for_widget("basic.links.count", function(widget)
    return string.find(widget.value_text, "Link clicks: 1", 1, true) ~= nil
end)

-- Menu action counter
local menu_before_text = tostring(vp:widget_get("basic.menu.count"):state().label)
assert(menu_before_text == "Menu actions: 0")

vp:widget_get("basic.menu.actions"):click()
assert_widget_exists("basic.menu.actions.reset_status")
vp:widget_get("basic.menu.actions.reset_status"):click()
vp:wait_for_widget("basic.menu.count", function(widget)
    return string.find(widget.value_text, "Menu actions: 1", 1, true) ~= nil
end)

-- Hover on a link
vp:widget_get("basic.link.reference"):hover()
vp:wait_for_settle()

-- Hover with position offset
vp:widget_get("basic.link.docs"):hover({ position = { x = 0.5, y = 0.5 } })
vp:wait_for_settle()

-- Collapsing section: verify advanced section toggle
vp:widget_get("basic.advanced"):set_value(false)
-- Advanced widgets should not be visible when collapsed (use include_invisible to check)
vp:widget_get("basic.advanced"):set_value(true)
assert_widget_exists("basic.advanced.action")
assert_widget_exists("basic.advanced.summary")
local summary_text = tostring(vp:widget_get("basic.advanced.summary"):state().label)
assert(summary_text == "This section is visible when expanded.")

-- Input diagnostics collapsing section
vp:widget_get("basic.input.debug"):set_value(true)
assert_widget_exists("basic.input.events")
assert_widget_exists("basic.input.modifiers")
assert_widget_exists("basic.input.raw_scroll")
assert_widget_exists("basic.input.smooth_scroll")

vp:widget_get("basic.input.debug"):set_value(false)

-- Slider value round-trip
vp:widget_get("basic.intensity"):set_value(99.5)
local slider = vp:widget_get("basic.intensity"):state()
-- Slider value is now a plain number
local slider_val = slider.value :: any
assert(slider_val ~= nil, "slider value should exist")
assert(slider_val >= 99 and slider_val <= 100, "slider should be near 99.5")

-- Checkbox round-trip
vp:widget_get("basic.enabled"):set_value(false)
vp:wait_for_widget("basic.enabled", function(widget)
    return widget.value == false
end)
vp:widget_get("basic.enabled"):set_value(true)
vp:wait_for_widget("basic.enabled", function(widget)
    return widget.value == true
end)

return {
    tests = "complete",
}
