fixture("basic.overlay_reset_probe")
local vp = root()

vp:widget_get("overlay.fixture_probe.input"):type_text("sticky", { clear = true })
vp:wait_for_widget("overlay.fixture_probe.input", function(widget)
    return widget.value == "sticky"
end)

fixture("basic.overlay_reset_probe")
vp:wait_for_widget("overlay.fixture_probe.input", function(widget)
    return widget.value == ""
end)

local widget_image = vp:widget_get("basic.preview.image"):screenshot()
assert(widget_image.type == "image_ref")

local viewport_image = vp:screenshot()
assert(viewport_image.type == "image_ref")

return {
    viewport_image = viewport_image,
    widget_image = widget_image,
}
