First, read `./skills/SKILL.md` for script-first development guidance.

You are developing the eguidev demo app, and you are using `edev` plus
`script_eval` to assist. Do not rely on removed direct MCP widget, action, or
fixture tools. Restart the app through `restart`, then perform setup,
interaction, waits, and verification inside Luau scripts.

## Core Functionality

- Try to make a crashing change to the app and restart. Does this act as
  expected? Are errors clear?
- Try to trigger various types of layout issue, including clipping, overflow,
  and overlapping widgets. Does the app behave as expected? Do you have the
  tools to troubleshoot?
- Try out all supported widget types. Do they behave as expected? Are there any
  missing features or issues?

## Script-First Setup

- Start most interactions by listing fixtures with `fixtures()` and applying an
  appropriate baseline via `fixture()` inside the script.
- Keep the smoke harness fixture-agnostic; scripts are responsible for choosing
  and applying fixtures explicitly.

## Input Injection

- Test the script-facing input surface: `click`, `hover`, `drag`,
  `drag_relative`, `drag_to`, `scroll`, `scroll_to`, `key`, `paste`, and
  `type_text`. Can you simulate complex interactions like drag-and-drop or
  keyboard shortcuts?
- Test key delivery with modifiers (ctrl, shift, alt, command). Do keyboard
  shortcuts work correctly?
- Test paste for clipboard simulation.

## Wait and Synchronization

- Test `wait_for_settle` - does the composite settle check (InputSettled +
  RepaintIdle) match expected flow semantics?
- Test `wait_for_widget` with various predicates: existence, visibility,
  enabled/focused state, text equality/substring checks, and disappearance.
  Do timeouts work? What happens when predicates are never met?

## Layout Diagnostics

- Test `check_layout` on a viewport
- Test `check_layout` on a widget subtree
- Verify the returned issues are useful for identifying what changed

## Widget Hierarchy

- Test `parent()` and `children()` traversal from `Widget` handles.
- Test `widget_at_point` - can you identify widgets by coordinate?

## Screenshot and Visual Tools

- Test `Widget:screenshot()` for individual widget capture (not just viewport).
- Test `show_highlight()` to visually mark specific widgets.
- Test all `show_debug_overlay` modes: `bounds`, `margins`, `clipping`,
  `overlaps`, `focus`, `layers`, `containers`.

## Widget Ids

- Test generated hex ids for session-stable lookup within one app run.
- Test duplicate explicit ids to confirm ambiguity errors stay clear.
- Test `id_prefix` filtering in `widget_list`.

## Error Handling

- Test invalid widget references - clear error messages?
- Test operations on non-existent ids.
- Test `Widget:set_value()` with wrong value types on different roles.
- Test out-of-bounds values (slider value outside range, combo index too high).

## Edge Cases

- Test with many widgets (add 100+ items) - does `widget_list` stay usable and
  deterministic?
- Test `include_invisible=true` in `widget_list` - are clipped widgets
  included?
- Test focus management with `focus()` - can you programmatically focus
  widgets?

When you are done, reset the code for `./crates/eguidev_demo` to the original
state, and report on:
1. What works well
2. What has issues or unclear behavior
3. What's missing or could be improved
4. Any API inconsistencies or documentation gaps
