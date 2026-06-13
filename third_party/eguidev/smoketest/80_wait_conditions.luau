fixture("basic.default")
local vp = root()

-- wait_for_widget with a presence predicate
local exists_result = vp:wait_for_widget("basic.submit", function(widget)
    return true
end)
assert(exists_result ~= nil)

-- wait_for_widget with a visibility predicate
local vis_result = vp:wait_for_widget("basic.submit", function(widget)
    return widget.visible
end)
assert(vis_result ~= nil and vis_result.visible)

-- explicit visible waits
local widget_visible = vp:wait_for_widget_visible("basic.submit")
assert(widget_visible.visible)
local handle_visible = vp:widget_get("basic.submit"):wait_for_visible()
assert(handle_visible.visible)

-- wait_for_widget with an enabled predicate
local enabled_result = vp:wait_for_widget("basic.submit", function(widget)
    return widget.enabled
end)
assert(enabled_result ~= nil and enabled_result.enabled)

-- wait_for_widget with a text contains predicate on the status widget
local contains_result = vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Waiting", 1, true) ~= nil
end)
assert(contains_result ~= nil)

-- wait_for_widget with an exact widget value predicate
local current_name = vp:widget_get("basic.name"):state().value
local equals_result = vp:wait_for_widget("basic.name", function(widget)
    return widget.value == current_name
end)
assert(equals_result ~= nil)

-- wait_for_widget with a label contains predicate
local label_result = vp:wait_for_widget("basic.submit", function(widget)
    return widget.label ~= nil and string.find(widget.label :: string, "Submit", 1, true) ~= nil
end)
assert(label_result ~= nil)

-- wait_for_widget with a label equals predicate
local exact_label = vp:wait_for_widget("basic.submit", function(widget)
    return widget.label == "Submit"
end)
assert(exact_label ~= nil)

-- wait_for_widget with predicate function
local pred_result = vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Waiting", 1, true) ~= nil
end)
assert(pred_result ~= nil)

-- wait_for_widget with a focus predicate after focusing
vp:widget_get("basic.name"):focus()
local focus_result = vp:wait_for_widget("basic.name", function(widget)
    return widget.focused
end)
assert(focus_result ~= nil and focus_result.focused)

-- wait_for_settle
vp:wait_for_settle()

-- assert_widget_exists
assert_widget_exists("basic.submit")
vp:wait_for_widget("basic.submit", function(widget)
    return widget.visible
end)
vp:wait_for_widget("basic.submit", function(widget)
    return widget.enabled
end)

-- assert_widget_value on the name field
vp:wait_for_widget("basic.name", function(widget)
    return widget.value == "Sky"
end)

return {
    conditions_tested = 13,
}
