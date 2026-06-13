# Automation Reliability Notes

## Summary

This document records the design decisions behind automation reliability in eguidev.
The design goal is deterministic scripting behavior with typed, diagnosable failures.

## Resolved failure modes

1. Fixture execution
- Fixtures are applied by scripts via `fixture()` with auto-settle.
- Restart is fixture-agnostic; it restarts the app and returns phase timing.

2. `wait_for_widget` predicate safety
- Waits evaluate explicit predicates over typed widget or viewport snapshots.
- Widget predicates receive `nil` while a widget is missing, so appearance and
  disappearance use the same API.
- Timeouts return typed `timeout` errors with structured wait diagnostics rather
  than soft `{ matched = false }` success results.

3. Keyboard target routing
- `key` accepts an optional `target` parameter to resolve + focus before delivery.
- `type_text` accepts `focus_timeout_ms` for explicit focus handshake.
- Targeted delivery emits typed routing failures:
  `target_not_focusable`, `focus_not_acquired`, `target_detached`.

4. Settle waits
- `Viewport:wait_for_settle()` uses a single composite check: InputSettled + RepaintIdle.
- All high-level actions auto-settle by default, ensuring the UI has processed all queued
  work and repainted before returning. Disable with `settle: #{enabled: false}`.

5. Deterministic click completion
- `click()` auto-settles by default, so the UI processes queued work and repaints
  before the action returns.
- Follow-up state checks are expressed as explicit waits after the action, which
  keeps action options data-shaped and timeout behavior consistent across the API.

6. Fixture reset contract and boundary cleanup
- Fixture apply boundaries clear transient DevMCP state (queued input/commands, queued widget
  value updates, scroll overrides, and overlay debug artifacts) to avoid cross-run leakage.
- Fixtures are baseline-reset by contract: each fixture must be independently invokable, isolated
  from prior app state, and safe to apply in any order.

## Intentional strict semantics

- Wait predicates are explicit; there is no secondary wait-condition DSL to interpret.
- Targeted key delivery fails fast instead of silently dropping delivery.
- Actions auto-settle; callers must explicitly opt out when needed.
