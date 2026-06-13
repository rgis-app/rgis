fixture("basic.default")
local vp = root()

vp:widget_get("basic.input.debug"):set_value(true)
assert_widget_exists("basic.input.pointer")

local submit = vp:widget_get("basic.submit")
local submit_state = submit:state()
local center: Pos = {
    x = (submit_state.interact_rect.min.x + submit_state.interact_rect.max.x) / 2,
    y = (submit_state.interact_rect.min.y + submit_state.interact_rect.max.y) / 2,
}

vp:raw_pointer_move(center)
vp:wait_for_widget("basic.input.pointer", function(widget)
    return widget.value_text ~= "" and string.find(widget.value_text, "none", 1, true) == nil
end)

vp:raw_pointer_button(center, "primary", "press")
vp:raw_pointer_button(center, "primary", "release")
vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Saved Sky", 1, true) ~= nil
end)

vp:widget_get("basic.name"):focus()
vp:widget_get("basic.name"):type_text("", { clear = true })
vp:raw_text("++")
vp:wait_for_widget("basic.name", function(widget)
    return string.find(widget.value_text, "++", 1, true) ~= nil
end)

vp:key("enter", { target = "basic.name" })
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "Enter", 1, true) ~= nil
end)

vp:raw_key("escape", "press")
vp:raw_key("escape", "release")
vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "Escape", 1, true) ~= nil
end)

local state = vp:state()
assert(state.pixels_per_point > 0, "viewport state should expose pixels per point")

vp:raw_scroll({ x = 0, y = -24 })
vp:wait_for_settle()
vp:wait_for_widget("basic.input.scroll_events", function(widget)
    return widget.value_text ~= "Scroll events: 0"
end)

local final_status = vp:widget_get("basic.status"):state()

return {
    pixels_per_point = state.pixels_per_point,
    status = final_status.value_text,
}
