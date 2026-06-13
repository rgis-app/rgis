# Luau guide

Quick reference for Luau syntax used in eguidev scripts.
For the canonical API surface and function-level behavior, see
`crates/eguidev_runtime/luau/eguidev.d.luau` or `script_api`.

`--!strict` is implicit for all scripts passed to `script_eval`. Write strict Luau, but omit the
hot comment unless you specifically want it in a checked-in file.

## Values and tables

```luau
local ok = true
local count = 3
local ratio = 0.25
local name = "Alice"
local nothing = nil
local items = { 1, 2, 3 }
local opts = { timeout_ms = 2000, key = "value" }
```

## Operators

```luau
-- Comparison
x == y    x ~= y    x > y    x < y    x >= y    x <= y

-- Logical
a and b    a or b    not a

-- Arithmetic
x + y    x - y    x * y    x / y    x % y
```

## Strings

```luau
local s = "hello"
#s
s:find("ell", 1, true) ~= nil
s .. " world"
string.upper(s)
```

## Tables and property access

```luau
local payload = { a = 1, nested = { c = 2 } }
payload.a
payload.nested.c

local vp = root()
local widget = vp:widget_get("submit")
local state = widget:state()
state.rect.min.x
state.value
```

## Nil checks

```luau
local state = root():widget_get("submit"):state()
if state.label ~= nil then
    log(state.label)
end
```

## Control flow

```luau
if x > 0 then
    log("positive")
elseif x < 0 then
    log("negative")
else
    log("zero")
end

for _, item in ipairs(items) do
    log(item)
end

while x < 10 do
    x += 1
end
```

## Functions

```luau
local function add(a: number, b: number): number
    return a + b
end

local sum = add(1, 2)
```

## Options pattern

Pass optional parameters as Luau tables. Unspecified keys use defaults.

```luau
local vp = root()
vp:widget_get("submit"):click({ timeout_ms = 1000 })
vp:wait_for_widget("status", function(widget)
    local value = widget ~= nil and widget.value or nil
    local text = tostring(value ~= nil and (value :: any).text or "")
    return string.find(text, "Done", 1, true) ~= nil
end, { timeout_ms = 1000 })
vp:wait_for_widget("status", function(widget)
    return widget ~= nil and widget.label == "Ready"
end, { timeout_ms = 1000 })
vp:wait_for_settle({ timeout_ms = 1000 })
```

## Assertions

Failed assertions stop evaluation and are also recorded in the `assertions` result array.

```luau
assert(x > 0)
assert(x > 0, "x must be positive")
assert(
    actual == expected,
    string.format("expected %s, got %s", tostring(expected), tostring(actual))
)
```

## Common idioms

```luau
local vp = root()
local toggle = vp:widget_get("toggle")
local toggle_state = toggle:state()
if toggle_state.value == nil or not ((toggle_state.value :: any).bool) then
    toggle:click()
end

local buttons = vp:widget_list({ role = "button" })
for _, button in ipairs(buttons) do
    log(button.id)
end

vp:widget_get("search.input"):hover()

vp:widget_get("search.input"):type_text("/tmp", {
    clear = true,
    enter = false,
    focus_timeout_ms = 1000,
})
vp:key("Enter", { target = "search.input" })
```
