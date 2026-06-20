# castr — Control UI Design Direction

**Date:** 2026-06-20
**Status:** Design-first reset

## Decision

The first attempted `castr sidebar`/`castr control` implementation is rejected. It tried
to imitate a Herdr-style control pane with tmux popups/splits and incremental row tweaks.
That direction produced poor UI and unsafe layout behavior.

The stable base remains:

- daemon lifecycle
- Lua kind detection
- pane labels in tmux borders
- typed pane snapshots exposed by the daemon

The control UI must be redesigned before more implementation.

## UX Principles

- Do not mutate the user's tmux layout just to show control UI.
- Do not fake a permanent Herdr sidebar with an ordinary tmux split.
- Do not put prose like `full · 2 hidden` in pane borders/status lines.
- Prefer one deliberate, polished full-screen control mode over many small tmux hacks.
- The UI should feel like a dashboard, not a debug list.

## Intended Surface

`castr control` should open a dedicated full-screen/overlay TUI mode that temporarily
takes over the terminal and returns cleanly to tmux on exit.

It should show a Herdr-inspired dashboard:

```text
castr

workspaces
┌ castr ────────────────────────────── working · 1 agent · 3 panes ┐
│ ~/Documents/phcurado/castr                                      │
└─────────────────────────────────────────────────────────────────┘

agents
┌ pi · castr ─────────────────────────────────────────── working ┐
│ cwd  ~/Documents/phcurado/castr                                │
│ pane %32   tab 2:castr                                         │
└────────────────────────────────────────────────────────────────┘

panes
┌ nvim · castr ───────────────────────────────────────── editing ┐
│ cwd  ~/Documents/phcurado/castr                                │
│ pane %31   tab 2:castr                                         │
└────────────────────────────────────────────────────────────────┘
```

## Interaction Model

- `j/k` or arrows: move between cards
- `enter`: jump to selected pane
- `/`: filter
- `tab`: switch section focus
- `z`: jump and zoom selected pane
- `q` / `esc`: close control UI and restore tmux

## Zoom Orientation

Zoom/fullscreen orientation needs a visual solution, not prose.

Candidates:

- border color/style change for zoomed windows
- a tiny symbolic badge only if necessary, e.g. `◱2`, not `full · 2 hidden`
- the control UI should show hidden panes clearly when invoked from a zoomed pane

No implementation until a small mockup is accepted.

## Extensibility

Lua owns semantics, not raw UI drawing at first:

- kind
- label
- state
- color/icon metadata
- grouping hints later if needed

Rust owns the initial TUI renderer. Add Lua formatting hooks only after concrete needs
appear, e.g. `format_control_card(pane)` or `register_section`, not a broad raw UI API.
