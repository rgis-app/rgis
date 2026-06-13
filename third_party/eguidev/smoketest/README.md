# Direct Luau Smoketests

This directory holds the checked-in Luau smoketest suite for `eguidev_demo`.

## Conventions

- Every smoketest file is a self-contained `.luau` script.
- Each script must establish its own starting state with `fixture(...)` before interacting with
  the UI. `fixture(...)` already waits for the fixture's readiness anchors, so setup-specific
  widget waits should only appear when the test is exercising an interaction after setup.
- Scripts should assert visible app behavior and public API results rather than internal details.
- `edev smoke` ignores a script's final return value; use assertions for pass/fail and `log(...)`
  for extra diagnostics.
- Keep files independent. Do not rely on state left behind by an earlier smoketest.

## Run

```sh
cargo run -p eguidev_demo --bin eguidev_demo -- --smoketests ./smoketest
```
