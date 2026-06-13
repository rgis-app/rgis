fixture("basic.default")
local vp = root()

vp:widget_get("basic.link.docs"):hover()

vp:widget_get("basic.menu.actions"):click()
assert_widget_exists("basic.menu.actions.mark_ready")
vp:widget_get("basic.menu.actions.mark_ready"):click()
vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Menu ready clicked", 1, true) ~= nil
end)

vp:widget_get("basic.advanced"):set_value(true)
assert_widget_exists("basic.advanced.action")
vp:widget_get("basic.advanced.action"):click()
vp:wait_for_widget("basic.status", function(widget)
    return string.find(widget.value_text, "Advanced action clicked.", 1, true) ~= nil
end)

vp:widget_get("basic.scroll"):scroll_into_view()
vp:widget_get("basic.scroll"):scroll({ x = 0, y = -120 })
vp:widget_get("basic.scroll"):scroll_to({ align = "bottom" })
vp:wait_for_widget("basic.scroll.row.49", function(widget)
    return widget.visible
end)
vp:wait_for_settle()

return vp:widget_get("basic.status"):state()
