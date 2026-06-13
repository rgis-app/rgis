fixture("basic.default")
local vp = root()

-- Test viewport state
local state = vp:state()
assert(state.pixels_per_point > 0, "pixels_per_point should be positive")
assert(vp.id == "root", "initial viewport should be root")

-- Test complex key combinations
vp:widget_get("basic.input.debug"):set_value(true)
vp:key("ctrl-shift-Z")
local key_event = vp:wait_for_widget("basic.input.key_event", function(widget)
    return string.find(widget.value_text, "Z", 1, true) ~= nil
end) :: any
assert(string.find(key_event.value_text, "ctrl=true", 1, true) ~= nil, "should have ctrl modifier")
assert(string.find(key_event.value_text, "shift=true", 1, true) ~= nil, "should have shift modifier")

-- Test text_measure
local measure = vp:widget_get("basic.submit"):text_measure() :: any
assert(measure.actual_size.x > 0, "actual_size.x should be positive")
assert(measure.actual_size.y > 0, "actual_size.y should be positive")
assert(measure.visible_text == "Submit")

-- Test sibling discovery via parent()/children()
local submit = vp:widget_get("basic.submit")
local siblings = {}
local parent = submit:parent()
if parent ~= nil then
    for _, child in ipairs(parent:children()) do
        if child.id ~= submit.id then
            table.insert(siblings, child.id)
        end
    end
end
assert(#siblings >= 1, "basic.submit should have siblings in basic.panel")
local found_name = false
for _, sibling_id in ipairs(siblings) do
    if sibling_id == "basic.name" then
        found_name = true
    end
end
assert(found_name, "basic.name should be a sibling of basic.submit")

-- Test widget_at_point
local submit_state = submit:state()
local center: Pos = {
    x = (submit_state.interact_rect.min.x + submit_state.interact_rect.max.x) / 2,
    y = (submit_state.interact_rect.min.y + submit_state.interact_rect.max.y) / 2,
}
local hits = vp:widget_at_point(center, true) :: any
assert(#hits >= 1, "should hit at least the submit button")
local found_submit = false
for _, entry in ipairs(hits) do
    if entry.id == "basic.submit" then
        found_submit = true
    end
end
assert(found_submit, "should find basic.submit in hits")

return {
    tested = true
}
