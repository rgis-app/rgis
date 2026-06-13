# Cleanup API Notes

These notes capture the public API snapshot (via ruskel) and the intended adjustments tracked
in the cleanup plan.

## Current public surface

- `eguidev` default build: inert `DevMcp`, `FrameGuard`, helper functions
  (`id`, `id_with_meta`, `id_label`, `track_response`,
  `track_response_with_meta`, `raw_input_hook`), `DevUiExt`, fixture types,
  and widget metadata/types.
- `eguidev` with `devtools`: adds `runtime::attach()`, the embedded MCP
  server, script evaluation/types, screenshots, and smoke helpers.
- `edev`: `run()` and `EdevError` remain the only public items.

## Intentions under cleanup

- Keep helper functions and `DevUiExt` consistent with the inert-by-default
  `DevMcp` model.
- Keep runtime attachment explicit and localized to one bootstrap boundary.
- Tool semantics updated: `widget_get` now errors on missing widgets, and `widget_list`
  respects visibility unless `include_invisible` is set.
- Duplicate explicit widget ids now block automation until instrumentation is fixed.
- Widget registry entries now include optional `value` data for stateful widgets to aid
  verification without screenshots.
- Widget registry entries now include optional `layout` data (clip rect, available rect,
  visible fraction) when captured via `DevUiExt` helpers.
- Script-facing widget discovery now returns `Widget` handles; live frame fields are read through
  `Widget:state()` as `WidgetState`.
- Widget registry entries now expose one canonical `id`. Explicit ids are
  preferred; generated ids may be opaque hex strings and are session-stable only.
- Widget role taxonomy now includes `Label` for text-only widgets.
- Tool errors now use `not_found`, `ambiguous`, `invalid_ref`, and `internal` codes with
  structured details when available.
