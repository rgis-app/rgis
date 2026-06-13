fixture("basic.empty")
local vp = root()

vp:widget_get("basic.name"):focus()
vp:paste("Pasted from smoke")
vp:wait_for_settle()

vp:widget_get("basic.name"):type_text("Luau", { clear = true })
vp:widget_get("basic.enabled"):set_value(true)
vp:wait_for_widget("basic.enabled", function(widget)
    return widget.value == true
end)
vp:widget_get("basic.intensity"):set_value(73.25)

vp:widget_get("basic.submit"):click()
local status = vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Saved Luau", 1, true) ~= nil
end)
assert(status ~= nil, "submit should update status")

local links = vp:widget_list({ role = "link", id_prefix = "basic.link." }) :: any
assert(#links == 2)

local preview = vp:widget_get("basic.preview.image")
assert(preview:state().role == "image")
local final_status = status :: WidgetState
assert(final_status.value_text ~= "Fixture: empty")

return {
    link_count = #links,
    status = final_status.value_text,
}
