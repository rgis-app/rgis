fixture("basic.default")
local vp = root()

-- Enable input diagnostics to observe key events and modifiers
vp:widget_get("basic.input.debug"):set_value(true)
assert_widget_exists("basic.input.key_event")

-- key with modifiers via combo string
vp:key("ctrl-A", { target = "basic.name" })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "A", 1, true) ~= nil
end)

-- key with shift modifier
vp:key("shift-B", { target = "basic.name" })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "B", 1, true) ~= nil
end)

-- key with alt modifier
vp:key("alt-C", { target = "basic.name" })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "C", 1, true) ~= nil
end)

-- key with repeat_count
vp:widget_get("basic.name"):type_text("", { clear = true })
vp:key("arrowright", { target = "basic.name", repeat_count = 3 })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "ArrowRight", 1, true) ~= nil
end)

-- type_text with clear
vp:widget_get("basic.name"):type_text("Fresh", { clear = true })
vp:wait_for_widget("basic.name", function(widget)
    return widget.value == "Fresh"
end)

-- type_text with enter (should trigger Enter key event)
vp:widget_get("basic.name"):type_text("WithEnter", { clear = true, enter = true })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "Enter", 1, true) ~= nil
end)

-- raw_key press and release with modifiers
vp:raw_key("tab", "press", { modifiers = { shift = true } })
vp:raw_key("tab", "release", { modifiers = { shift = true } })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "Tab", 1, true) ~= nil
end)

-- raw_text inserts raw text
vp:widget_get("basic.name"):focus()
vp:widget_get("basic.name"):type_text("", { clear = true })
vp:raw_text("raw")
vp:wait_for_widget("basic.name", function(widget)
    return string.find(widget.value_text, "raw", 1, true) ~= nil
end)

-- raw_pointer_button with secondary button
local submit = vp:widget_get("basic.submit")
local submit_state = submit:state()
local center: Pos = {
    x = (submit_state.interact_rect.min.x + submit_state.interact_rect.max.x) / 2,
    y = (submit_state.interact_rect.min.y + submit_state.interact_rect.max.y) / 2,
}
vp:raw_pointer_button(center, "secondary", "press")
vp:raw_pointer_button(center, "secondary", "release")

-- raw_pointer_button with middle button
vp:raw_pointer_button(center, "middle", "press")
vp:raw_pointer_button(center, "middle", "release")

-- raw_scroll with modifiers
vp:widget_get("basic.input.debug"):set_value(true)
vp:raw_scroll({ x = 0, y = -10 }, { modifiers = { ctrl = true } })
vp:wait_for_widget("basic.input.scroll_events", function(widget)
    return widget.value_text ~= "Scroll events: 0"
end)

-- viewport state reports expected fields
local state = vp:state()
assert(state.pixels_per_point > 0, "should report pixels_per_point")

return {
    input_state_ppp = state.pixels_per_point,
}
