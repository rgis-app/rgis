# Development Notes

## Documentation Comments in `.d.luau`

Good doc comments explain *why* and *how*, not *what* — the signature already says what.

- **Never recapitulate the signature.** Don't restate parameter names, types, or return
  types that are visible from the declaration itself. Instead, explain behavior that isn't
  obvious from the types alone: semantics, side effects, defaults, interactions between
  fields, and edge cases.
- **Never include trivial examples.** If the usage is obvious from the signature and the
  comment, an example adds nothing. Only include examples when they clarify a non-obvious
  interaction or a surprising calling convention.

## Smoketests

Run the default smoke wrapper from `xtask`:

```sh
cargo xtask smoke
```

That command runs the checked-in Luau smoketest suite through `edev smoke` and prints one line per
test by default.

Enable extra smoketest debugging output:

```sh
cargo xtask smoke --verbose
```

Verbose smoke runs keep diagnostics in terminal output. `edev smoke` no longer writes a persistent
artifact directory by default.

Run the separate `edev mcp` transport smoke:

```sh
cargo xtask smoke-edev
```

Enable verbose transport logging:

```sh
cargo xtask smoke-edev --verbose
```
