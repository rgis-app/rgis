fixture("basic.empty")
local vp = root()
vp:widget_get("login.username"):type_text("alice", { clear = true })
vp:widget_get("login.password"):type_text("secret", { clear = true })
vp:widget_get("login.submit"):click()

local status = vp:wait_for_widget("login.status", function(widget)
    return string.find(widget.value_text, "Welcome", 1, true) ~= nil
end)
assert(status ~= nil, "login should succeed")

return status
