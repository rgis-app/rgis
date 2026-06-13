fixture("basic.default")
local vp = root()
local submit = vp:widget_get("basic.submit")

local layout = vp:check_layout()
assert(type(layout) == "table", "check_layout should return an issues array")

local submit_layout = submit:check_layout()
assert(#submit_layout == 0, "submit subtree should be layout-clean in the default fixture")

local highlighted = submit:show_highlight("#00ff00")
assert(highlighted.rect.max.x > highlighted.rect.min.x, "show_highlight should return a rect")
submit:hide_highlight()

vp:show_debug_overlay("bounds", { show_labels = true })
vp:hide_debug_overlay()

return {
    issues = #layout,
    submit_issues = #submit_layout,
}
