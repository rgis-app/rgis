local available = fixtures()
assert(#available >= 1, "demo should advertise fixtures")

local saw_basic_default = false
for _, fixture_info in ipairs(available) do
    if fixture_info.name == "basic.default" then
        saw_basic_default = true
        assert(#fixture_info.anchors >= 1, "basic.default should advertise readiness anchors")
    end
end
assert(saw_basic_default, "basic.default fixture should be present")

fixture("basic.default")
local vp = root()

assert_widget_exists("basic.submit")
vp:wait_for_widget("basic.submit", function(widget)
    return widget.visible
end)
vp:wait_for_widget("basic.submit", function(widget)
    return widget.enabled
end)

local submit = vp:widget_get("basic.submit")
local submit_state = submit:state()
assert(submit_state.role == "button")

local ancestor_count = 1
local current = submit:parent()
while current ~= nil do
    ancestor_count += 1
    current = current:parent()
end
assert(ancestor_count >= 1, "ancestor walk should include the submit button")

local hit_point: Pos = {
    x = submit_state.interact_rect.min.x + 2,
    y = submit_state.interact_rect.min.y + 2,
}
local hit = vp:widget_at_point(hit_point) :: any
assert(hit[1] ~= nil, "widget_at_point should find the submit area")

local measure = submit:text_measure() :: any
assert(measure.actual_size.x > 0, "text_measure should report width")

return {
    fixtures = #available,
    measured = measure.visible_text,
    tree_size = ancestor_count,
}
