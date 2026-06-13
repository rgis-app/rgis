fixture("basic.default")
local vp = root()

vp:widget_get("basic.submit"):click()
vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Saved", 1, true) ~= nil
end)

local drag_detail_before = vp:widget_get("basic.drag.detail"):state().value_text
local drag_widget = vp:widget_get("basic.drag")
local drag_state = drag_widget:state()
local drag_target: Pos = {
    x = drag_state.interact_rect.max.x,
    y = drag_state.interact_rect.max.y,
}

vp:widget_get("basic.drag"):drag(
    drag_target,
    {
        settle = false,
    }
)
vp:wait_for_widget("basic.drag.detail", function(widget)
    return widget.value_text ~= "" and widget.value_text ~= drag_detail_before
end)

vp:widget_get("basic.drag"):drag_relative({ x = 1, y = 1 }, { x = 0.2, y = 0.2 })
wait_for_frames(2)

vp:widget_get("basic.drag"):drag_to(vp:widget_get("basic.submit"), {
    settle = false,
})
wait_for_frames(2)

vp:widget_get("basic.scroll"):scroll_to({
    align = "bottom",
    settle = false,
})
vp:wait_for_widget("basic.scroll.row.49", function(widget)
    return widget.visible
end)

return {
    drag_detail = vp:widget_get("basic.drag.detail"):state().value_text,
}
