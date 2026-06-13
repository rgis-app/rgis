fixture("basic.default")
local vp = root()

-- Drag the root region over the status label
vp:widget_get("basic.drag"):drag_to(vp:widget_get("basic.status"), { settle = true })
wait_for_frames(5)

-- Check for overlaps
local issues = vp:check_layout()
local overlap_count = 0
for _, issue in ipairs(issues) do
    if issue.kind == "overlap" then
        overlap_count += 1
    end
end
assert(overlap_count > 0, "should find at least one overlap")

return {
    overlap_count = overlap_count,
}
