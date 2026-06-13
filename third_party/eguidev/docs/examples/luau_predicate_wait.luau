local vp = root()

return vp:wait_for_widget("status", function(widget)
    return widget.label == "Ready"
end)
