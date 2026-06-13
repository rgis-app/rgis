fixture("viewports.default")

local secondary: Viewport? = nil
for _, viewport in ipairs(viewports()) do
    if viewport.id ~= "root" then
        secondary = viewport
        break
    end
end
assert(secondary ~= nil, "secondary viewport should be present")
local secondary_vp = secondary :: Viewport

secondary_vp:focus()

-- exact inner size predicate
secondary_vp:set_inner_size({ x = 600, y = 400 })
local res_inner = secondary_vp:wait_for(function(viewport)
    return viewport.inner_size.x == 600 and viewport.inner_size.y == 400
end)
assert(res_inner.inner_size.x == 600 and res_inner.inner_size.y == 400)

-- We skip focused check because we are in no-focus mode.

return {
    tested = true
}
