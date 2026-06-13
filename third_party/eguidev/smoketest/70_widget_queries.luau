fixture("basic.default")
local vp = root()

-- widget_list with include_invisible shows at least as many as visible-only
local all_widgets = vp:widget_list({ include_invisible = true }) :: any
local visible_only = vp:widget_list() :: any
assert(#all_widgets >= #visible_only, "include_invisible should show at least as many")

-- widget_list filtered by role
local buttons = vp:widget_list({ role = "button" }) :: any
assert(#buttons >= 1, "should find at least one button")
for _, w in ipairs(buttons) do
    assert(w:state().role == "button")
end

-- widget_list with id_prefix (ensure scroll rows exist first)
assert_widget_exists("basic.scroll.row.0")
local scroll_rows = vp:widget_list({ id_prefix = "basic.scroll.row.", include_invisible = true }) :: any
assert(#scroll_rows >= 1, "should find scroll rows")
for _, w in ipairs(scroll_rows) do
    assert(string.find(w.id, "basic.scroll.row.", 1, true) == 1, "id should have prefix")
end

-- parent()/children() compose hierarchy queries directly
local submit = vp:widget_get("basic.submit")
local ancestors = { submit.id }
local parent = submit:parent()
while parent ~= nil do
    table.insert(ancestors, parent.id)
    parent = parent:parent()
end
assert(#ancestors >= 1, "ancestor walk should include at least the target itself")

-- children() on a scroll area should expose immediate rows
assert_widget_exists("basic.scroll")
local children = vp:widget_get("basic.scroll"):children()
assert(#children >= 1, "scroll area should have children")

local function collect_descendants(widget: Widget, out: { string })
    for _, child in ipairs(widget:children()) do
        table.insert(out, child.id)
        collect_descendants(child, out)
    end
end
local descendants = {}
collect_descendants(vp:widget_get("basic.scroll"), descendants)
assert(#descendants >= #children, "recursive child walk should include immediate children")

local siblings = {}
local submit_parent = submit:parent()
if submit_parent ~= nil then
    for _, child in ipairs(submit_parent:children()) do
        if child.id ~= submit.id then
            table.insert(siblings, child.id)
        end
    end
end
assert(#siblings >= 0, "sibling query should succeed")

-- widget_at_point on a known widget
local submit_state = submit:state()
local center: Pos = {
    x = (submit_state.interact_rect.min.x + submit_state.interact_rect.max.x) / 2,
    y = (submit_state.interact_rect.min.y + submit_state.interact_rect.max.y) / 2,
}
local hit = vp:widget_at_point(center) :: any
assert(hit[1] ~= nil, "should find a widget at button center")

-- text_measure on a label
local measure = vp:widget_get("basic.status"):text_measure() :: any
assert(measure.actual_size.x > 0, "text_measure should report positive width")
assert(measure.actual_size.y > 0, "text_measure should report positive height")

-- plain typed literals replace constructor helpers
local r: Rect = { min = { x = 10, y = 20 }, max = { x = 30, y = 40 } }
assert(r.min.x == 10)
assert(r.min.y == 20)
assert(r.max.x == 30)
assert(r.max.y == 40)

-- log function
log("widget queries test completed")
log({ key = "value", number = 42 })

return {
    all_count = #all_widgets,
    visible_count = #visible_only,
    button_count = #buttons,
    children_count = #children,
}
