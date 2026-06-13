fixture("basic.default")
local vp = root()

local choice = vp:widget_get("basic.choice"):state()
assert(choice.role == "combo_box")
assert(choice.options ~= nil, "combo_box options should be present")
assert(#(choice.options :: { string }) == 3, "combo_box options should expose all choices")
assert((choice.options :: { string })[2] == "Beta", "combo_box options should keep order")
vp:widget_get("basic.choice"):set_value(2)
vp:wait_for_widget("basic.choice", function(widget)
    return widget.value == 2
end)

local drag_float = vp:widget_get("basic.drag.float"):state()
assert(drag_float.role == "drag_value")
assert(drag_float.range ~= nil, "ranged drag value should expose its range")
assert((drag_float.range :: Range).min == -10)
assert((drag_float.range :: Range).max == 10)

local drag_int = vp:widget_get("basic.drag.int"):state()
assert(drag_int.range ~= nil, "integer drag value should expose its range")
assert((drag_int.range :: Range).min == -5)
assert((drag_int.range :: Range).max == 20)

local toolbar = vp:widget_get("basic.toolbar.sync"):state()
assert(toolbar.role == "button")
assert(toolbar.selected == true, "selected-aware button should expose selected state")
vp:widget_get("basic.toolbar.sync"):click()
vp:wait_for_widget("basic.toolbar.sync", function(widget)
    return widget.selected == false
end)

local toggle = vp:widget_get("basic.toggle"):state()
assert(toggle.role == "toggle")
assert(toggle.value == false)
vp:widget_get("basic.toggle"):set_value(true)
vp:wait_for_widget("basic.toggle", function(widget)
    return widget.value == true
end)

local mixed = vp:widget_get("basic.mixed"):state()
assert(mixed.role == "checkbox")
assert(mixed.indeterminate == true, "indeterminate checkbox should expose third-state flag")

local name = vp:widget_get("basic.name"):state()
assert(name.multiline == false)
assert(name.password == false)

local notes = vp:widget_get("basic.notes"):state()
assert(notes.multiline == true)
assert(notes.password == false)

local password = vp:widget_get("basic.password"):state()
assert(password.role == "text_edit")
assert(password.multiline == false)
assert(password.password == true)

vp:widget_get("basic.mode.beta"):set_value(true)
vp:wait_for_widget("basic.mode.beta", function(widget)
    return widget.value == true
end)
vp:widget_get("basic.select.2"):set_value(true)
vp:wait_for_widget("basic.select.2", function(widget)
    return widget.value == true
end)

local progress = vp:widget_get("basic.progress.percent"):state()
assert(progress.role == "progress_bar")
assert(progress.label == "42%")

local detail_progress = vp:widget_get("basic.progress.detail"):state()
assert(detail_progress.role == "progress_bar")
assert(detail_progress.label == "Drag 1.5")

local color = vp:widget_get("basic.accent"):state()
assert(color.role == "color_picker")
assert(color.value == "#409CFFFF")
assert(color.value_text == "#409CFFFF")

vp:widget_get("basic.accent"):set_value("#112233FF")
vp:wait_for_widget("basic.accent", function(widget)
    return widget.value == "#112233FF"
end)
vp:wait_for_widget("basic.accent.value", function(widget)
    return widget.label == "Accent: #112233FF"
end)

local floating_window = vp:widget_get("basic.window.surface"):state()
assert(floating_window.role == "window")

local ok = pcall(function()
    vp:widget_get("basic.intensity"):set_value(200.5)
end)
assert(not ok, "out-of-range slider write should fail")

ok = pcall(function()
    vp:widget_get("basic.choice"):set_value(9)
end)
assert(not ok, "out-of-range combo-box write should fail")

ok = pcall(function()
    vp:widget_get("basic.drag.int"):set_value(99)
end)
assert(not ok, "out-of-range drag-value write should fail")

ok = pcall(function()
    vp:widget_get("basic.accent"):set_value("#1234")
end)
assert(not ok, "invalid color text should fail")

return {
    toolbar_selected = vp:widget_get("basic.toolbar.sync"):state().selected,
    accent = vp:widget_get("basic.accent"):state().value,
}
